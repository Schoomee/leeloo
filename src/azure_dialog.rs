//! Sélecteur de VM Azure : liste Resource Graph, recherche, connexion `az ssh vm`.

use crate::sessions_panel::SessionsHandle;
use crate::terminal;
use adw::prelude::*;
use leeloo::azure::{self, Query, VmRow};
use leeloo::azure_history::{Group, History, VmKey};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

/// Exécute `argv` sans shell, sans bloquer l'interface ; retourne stdout.
async fn run_command(argv: Vec<String>) -> Result<String, String> {
    let arguments: Vec<&std::ffi::OsStr> = argv.iter().map(std::ffi::OsStr::new).collect();
    let flags = gtk::gio::SubprocessFlags::STDOUT_PIPE | gtk::gio::SubprocessFlags::STDERR_PIPE;
    let subprocess = gtk::gio::Subprocess::newv(&arguments, flags)
        .map_err(|error| format!("lancement de « {} » impossible : {error}", argv[0]))?;
    let (stdout, stderr) = subprocess
        .communicate_utf8_future(None::<String>)
        .await
        .map_err(|error| error.to_string())?;
    if subprocess.is_successful() {
        Ok(stdout.map(|text| text.to_string()).unwrap_or_default())
    } else {
        let message = stderr
            .map(|text| text.trim().to_owned())
            .unwrap_or_default();
        Err(if message.is_empty() {
            "la commande a échoué".to_owned()
        } else {
            message
        })
    }
}

struct Dialog {
    dialog: adw::Dialog,
    list: gtk::ListBox,
    search: gtk::SearchEntry,
    all_subscriptions: gtk::CheckButton,
    private_ip: gtk::CheckButton,
    status: gtk::Label,
    spinner: gtk::Spinner,
    rows: RefCell<Vec<VmRow>>,
    groups: RefCell<Vec<Group>>,
    history: RefCell<History>,
    history_path: Option<PathBuf>,
    sessions: SessionsHandle,
    star_group: gtk::SizeGroup,
    query: RefCell<Query>,
    generation: Cell<u64>,
    size_groups: Vec<gtk::SizeGroup>,
    tab_view: adw::TabView,
    toasts: adw::ToastOverlay,
    window: adw::ApplicationWindow,
}

const COLUMNS: [&str; 6] = [
    "Resource group",
    "VM",
    "État",
    "Application",
    "Components",
    "Middlewares",
];

pub fn present(
    window: &adw::ApplicationWindow,
    tab_view: &adw::TabView,
    toasts: &adw::ToastOverlay,
    sessions: &SessionsHandle,
) {
    let history_path = History::default_path();
    let (history, history_error) = match history_path.as_deref().map(History::load) {
        Some(Ok(history)) => (history, None),
        Some(Err(error)) => (History::default(), Some(error)),
        None => (History::default(), None),
    };
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk::SelectionMode::Single);

    let search = gtk::SearchEntry::builder()
        .placeholder_text("Filtrer : mot, ou Application=Gitlab Environment=dev")
        .hexpand(true)
        .build();
    let all_subscriptions = gtk::CheckButton::with_label("Toutes les subscriptions");
    let private_ip = gtk::CheckButton::with_label("IP privée");
    let status = gtk::Label::builder().xalign(0.0).wrap(true).build();
    let spinner = gtk::Spinner::new();

    let refresh_button = gtk::Button::from_icon_name("view-refresh-symbolic");
    refresh_button.set_tooltip_text(Some("Recharger"));

    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    controls.set_margin_start(12);
    controls.set_margin_end(12);
    controls.set_margin_top(8);
    controls.append(&search);
    controls.append(&all_subscriptions);
    controls.append(&private_ip);

    let size_groups: Vec<gtk::SizeGroup> = COLUMNS
        .iter()
        .map(|_| gtk::SizeGroup::new(gtk::SizeGroupMode::Horizontal))
        .collect();
    let header_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    header_row.set_margin_start(24);
    header_row.set_margin_end(24);
    header_row.set_margin_top(8);
    let star_group = gtk::SizeGroup::new(gtk::SizeGroupMode::Horizontal);
    let star_spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    star_group.add_widget(&star_spacer);
    header_row.append(&star_spacer);
    for (title, group) in COLUMNS.iter().zip(&size_groups) {
        let label = gtk::Label::builder().label(*title).xalign(0.0).build();
        label.add_css_class("heading");
        group.add_widget(&label);
        header_row.append(&label);
    }

    let scrolled = gtk::ScrolledWindow::builder()
        .child(&list)
        .vexpand(true)
        .hexpand(true)
        .margin_start(12)
        .margin_end(12)
        .margin_bottom(12)
        .build();

    let status_bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    status_bar.set_margin_start(12);
    status_bar.set_margin_end(12);
    status_bar.set_margin_bottom(8);
    status_bar.append(&spinner);
    status_bar.append(&status);

    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    body.append(&controls);
    body.append(&header_row);
    body.append(&scrolled);
    body.append(&status_bar);

    let header = adw::HeaderBar::new();
    header.pack_start(&refresh_button);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&body));

    let dialog = adw::Dialog::builder()
        .title("VM Azure (Entra ID)")
        .content_width(1100)
        .content_height(600)
        .child(&toolbar)
        .build();

    let state = Rc::new(Dialog {
        dialog: dialog.clone(),
        list: list.clone(),
        search: search.clone(),
        all_subscriptions: all_subscriptions.clone(),
        private_ip,
        status,
        spinner,
        rows: RefCell::new(Vec::new()),
        groups: RefCell::new(Vec::new()),
        history: RefCell::new(history),
        // Historique illisible : on n'écrase pas le fichier.
        history_path: if history_error.is_some() {
            None
        } else {
            history_path
        },
        sessions: sessions.clone(),
        star_group,
        query: RefCell::new(Query::default()),
        generation: Cell::new(0),
        size_groups,
        tab_view: tab_view.clone(),
        toasts: toasts.clone(),
        window: window.clone(),
    });

    list.set_filter_func({
        let state = state.clone();
        move |row| {
            let rows = state.rows.borrow();
            usize::try_from(row.index())
                .ok()
                .and_then(|index| rows.get(index))
                .is_some_and(|vm| state.query.borrow().matches(vm))
        }
    });
    list.set_header_func({
        let state = state.clone();
        move |row, before| {
            let group_of = |row: &gtk::ListBoxRow| {
                usize::try_from(row.index())
                    .ok()
                    .and_then(|index| state.groups.borrow().get(index).copied())
            };
            let group = group_of(row);
            if before.is_some_and(|before| group_of(before) == group) {
                row.set_header(None::<&gtk::Widget>);
                return;
            }
            let title = match group {
                Some(Group::Favorites) => "Favoris",
                Some(Group::Recent) => "Récentes",
                _ => "Toutes les VM",
            };
            let label = gtk::Label::builder()
                .label(title)
                .xalign(0.0)
                .margin_top(8)
                .margin_bottom(4)
                .margin_start(12)
                .build();
            label.add_css_class("heading");
            row.set_header(Some(&label));
        }
    });
    search.connect_search_changed({
        let state = state.clone();
        move |entry| {
            *state.query.borrow_mut() = Query::parse(&entry.text());
            state.list.invalidate_filter();
        }
    });
    list.connect_row_activated({
        let state = state.clone();
        move |_, row| {
            let vm = usize::try_from(row.index())
                .ok()
                .and_then(|index| state.rows.borrow().get(index).cloned());
            if let Some(vm) = vm {
                state.connect(vm);
            }
        }
    });
    refresh_button.connect_clicked({
        let state = state.clone();
        move |_| state.reload()
    });
    all_subscriptions.connect_toggled({
        let state = state.clone();
        move |_| state.reload()
    });

    dialog.present(Some(window));
    search.grab_focus();
    if let Some(error) = history_error {
        state.set_idle(&format!("Favoris ignorés : {error}"));
    }
    state.reload();
}

impl Dialog {
    fn set_busy(&self, message: &str) {
        self.spinner.set_spinning(true);
        self.status.set_text(message);
    }

    fn set_idle(&self, message: &str) {
        self.spinner.set_spinning(false);
        self.status.set_text(message);
    }

    fn reload(self: &Rc<Self>) {
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        self.set_busy("Chargement des VM…");
        let state = self.clone();
        gtk::glib::spawn_future_local(async move {
            let result = state.fetch().await;
            // Une recharge plus récente a été lancée : on ignore ce résultat.
            if state.generation.get() != generation {
                return;
            }
            match result {
                Ok(list) => {
                    state.fill(list.rows);
                    let total = state.rows.borrow().len();
                    state.set_idle(&if list.truncated {
                        format!("{total} VM (liste tronquée à {})", azure::GRAPH_PAGE_SIZE)
                    } else {
                        format!("{total} VM")
                    });
                }
                Err(message) => {
                    state.fill(Vec::new());
                    state.set_idle(&format!("Erreur : {message}"));
                }
            }
        });
    }

    async fn fetch(&self) -> Result<azure::VmList, String> {
        let subscription = if self.all_subscriptions.is_active() {
            None
        } else {
            let id = run_command(azure::current_subscription_argv())
                .await
                .map_err(|error| format!("{error} (lancer « az login » ?)"))?;
            Some(id.trim().to_owned())
        };
        let output = run_command(azure::graph_query_argv(subscription.as_deref())).await?;
        azure::parse_graph_output(&output).map_err(|error| error.to_string())
    }

    /// Remplit la liste ; l'ordre (favoris, récentes, reste) vient de l'historique.
    fn fill(self: &Rc<Self>, rows: Vec<VmRow>) {
        self.list.remove_all();
        let ordered = self.history.borrow().order(rows);
        let (groups, rows): (Vec<Group>, Vec<VmRow>) = ordered.into_iter().unzip();
        for vm in &rows {
            self.list.append(&self.build_row(vm));
        }
        *self.groups.borrow_mut() = groups;
        *self.rows.borrow_mut() = rows;
        *self.query.borrow_mut() = Query::parse(&self.search.text());
        self.list.invalidate_filter();
        self.list.invalidate_headers();
    }

    fn save_history(&self) {
        if let Some(path) = &self.history_path
            && let Err(error) = self.history.borrow().save(path)
        {
            self.set_idle(&format!("Sauvegarde de l'historique impossible : {error}"));
        }
    }

    fn build_row(self: &Rc<Self>, vm: &VmRow) -> gtk::ListBoxRow {
        let cells = [
            vm.resource_group.as_str(),
            vm.name.as_str(),
            vm.power.as_deref().unwrap_or("-"),
            vm.tag("Application").unwrap_or("-"),
            vm.tag("Components").unwrap_or("-"),
            vm.tag("Middlewares").unwrap_or("-"),
        ];
        let container = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        container.set_margin_start(12);
        container.set_margin_end(12);
        container.set_margin_top(6);
        container.set_margin_bottom(6);
        let key = VmKey::from_row(vm);
        let star = gtk::Button::from_icon_name(if self.history.borrow().is_favorite(&key) {
            "starred-symbolic"
        } else {
            "non-starred-symbolic"
        });
        star.add_css_class("flat");
        star.set_valign(gtk::Align::Center);
        star.set_tooltip_text(Some("Ajouter aux favoris / retirer"));
        self.star_group.add_widget(&star);
        star.connect_clicked({
            let state = self.clone();
            move |_| {
                state.history.borrow_mut().toggle_favorite(key.clone());
                state.save_history();
                // Reconstruction différée : on ne détruit pas le bouton pendant son clic.
                let state = state.clone();
                gtk::glib::idle_add_local_once(move || {
                    let rows = state.rows.borrow().clone();
                    state.fill(rows);
                });
            }
        });
        container.append(&star);
        for (index, (text, group)) in cells.iter().zip(&self.size_groups).enumerate() {
            let label = gtk::Label::builder()
                .label(*text)
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .max_width_chars(34)
                .build();
            if index == 2 && !vm.is_running() {
                label.add_css_class("warning");
            }
            group.add_widget(&label);
            container.append(&label);
        }
        let link_button = gtk::Button::from_icon_name("external-link-symbolic");
        link_button.add_css_class("flat");
        link_button.set_valign(gtk::Align::Center);
        link_button.set_tooltip_text(Some("Ouvrir dans le portail Azure"));
        link_button.connect_clicked({
            let url = vm.portal_url();
            let window = self.window.clone();
            let toasts = self.toasts.clone();
            move |_| {
                gtk::UriLauncher::new(&url).launch(Some(&window), gtk::gio::Cancellable::NONE, {
                    let toasts = toasts.clone();
                    move |result| {
                        if let Err(error) = result {
                            toasts.add_toast(adw::Toast::new(&format!(
                                "Ouverture du navigateur impossible : {error}"
                            )));
                        }
                    }
                });
            }
        });
        container.append(&link_button);
        let save_button = gtk::Button::from_icon_name("list-add-symbolic");
        save_button.add_css_class("flat");
        save_button.set_valign(gtk::Align::Center);
        save_button.set_tooltip_text(Some("Enregistrer comme session"));
        save_button.connect_clicked({
            let state = self.clone();
            let vm = vm.clone();
            move |_| {
                let session = vm.to_session(state.private_ip.is_active());
                let message = match state.sessions.add_azure_vm(&vm.name, session) {
                    Ok(true) => format!("Session « {} » enregistrée (dossier Azure)", vm.name),
                    Ok(false) => format!("« {} » est déjà enregistrée", vm.name),
                    Err(error) => format!("Enregistrement impossible : {error}"),
                };
                state.set_idle(&message);
            }
        });
        container.append(&save_button);
        let row = gtk::ListBoxRow::new();
        row.set_child(Some(&container));
        row
    }

    fn connect(self: &Rc<Self>, vm: VmRow) {
        if vm.is_running() {
            self.open_tab(&vm);
            return;
        }
        let state_text = vm.power.as_deref().unwrap_or("état inconnu");
        let alert = adw::AlertDialog::new(
            Some("VM non démarrée"),
            Some(&format!("« {} » : {state_text}. La démarrer ?", vm.name)),
        );
        alert.add_responses(&[("cancel", "Annuler"), ("start", "Démarrer et se connecter")]);
        alert.set_response_appearance("start", adw::ResponseAppearance::Suggested);
        alert.set_default_response(Some("cancel"));
        alert.set_close_response("cancel");
        let state = self.clone();
        alert.choose(
            Some(&self.dialog),
            gtk::gio::Cancellable::NONE,
            move |response| {
                if response == "start" {
                    state.start_then_connect(vm);
                }
            },
        );
    }

    fn start_then_connect(self: &Rc<Self>, vm: VmRow) {
        self.set_busy(&format!("Démarrage de {}…", vm.name));
        let state = self.clone();
        gtk::glib::spawn_future_local(async move {
            match run_command(azure::start_vm_argv(&vm)).await {
                Ok(_) => {
                    state.set_idle("");
                    state.open_tab(&vm);
                }
                Err(message) => state.set_idle(&format!("Démarrage impossible : {message}")),
            }
        });
    }

    fn open_tab(&self, vm: &VmRow) {
        let session = vm.to_session(self.private_ip.is_active());
        match (azure_session_argv(&session), &vm.name) {
            (Ok(argv), name) => {
                terminal::open_session_tab(
                    &self.tab_view,
                    name,
                    argv,
                    terminal::TabStyle::default(),
                    leeloo::workspace::PaneSource::Azure { vm: session },
                );
                self.history
                    .borrow_mut()
                    .record_connection(VmKey::from_row(vm));
                self.save_history();
                self.dialog.close();
            }
            (Err(error), _) => self
                .toasts
                .add_toast(adw::Toast::new(&format!("VM invalide : {error}"))),
        }
        let _ = &self.window;
    }
}

fn azure_session_argv(
    session: &leeloo::session::AzureVmSession,
) -> Result<Vec<String>, leeloo::session::ValidationError> {
    leeloo::session::Session {
        id: 0,
        name: session.vm_name.clone(),
        folder: None,
        color: None,
        palette: None,
        kind: leeloo::session::SessionKind::AzureVm(session.clone()),
    }
    .argv()
}
