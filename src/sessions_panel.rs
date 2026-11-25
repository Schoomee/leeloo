//! Panneau latéral : liste des sessions par dossier, édition, import.

use crate::terminal;
use adw::prelude::*;
use leeloo::palette::{self, PALETTES, SESSION_COLORS};
use leeloo::session::{AzureVmSession, Session, SessionKind, SshSession};
use leeloo::ssh_config;
use leeloo::store::{FOLDER_SEPARATOR, FolderBackup, IMPORT_FOLDER, SessionStore, folder_name};
use leeloo::workspace::PaneSource;
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;

/// Action sur un dossier identifié par son chemin.
type FolderAction = fn(&Rc<Panel>, &str);

const INDENT: i32 = 18;

fn tab_style(session: &Session) -> terminal::TabStyle {
    terminal::TabStyle {
        color: session.color.clone(),
        palette: session.palette.as_deref().and_then(palette::by_id),
    }
}

fn folder_icon(collapsed: bool) -> &'static str {
    if collapsed {
        "folder-symbolic"
    } else {
        "folder-open-symbolic"
    }
}
const AZURE_FOLDER: &str = "Azure";

/// Durée du déroulement d'un dossier (ignorée si les animations sont coupées).
const FOLDER_ANIMATION_MS: u32 = 220;

/// Ligne affichée, avec de quoi la révéler ou la masquer en douceur.
struct RowEntry {
    revealer: gtk::Revealer,
    /// Dossiers qui contiennent la ligne, du plus externe au plus interne.
    ancestors: Vec<String>,
    /// Pour une ligne de dossier : son chemin et l'icône ouvert/fermé.
    folder: Option<(String, gtk::Image)>,
}

struct Panel {
    list: gtk::Box,
    rows: RefCell<Vec<RowEntry>>,
    /// Recherche rapide : texte saisi et sessions retenues (`None` = pas de filtre).
    filter: RefCell<String>,
    matches: RefCell<Option<HashSet<u64>>>,
    /// Première session affichée (Entrée dans le champ la lance).
    first_match: Cell<Option<u64>>,
    /// Dossiers repliés (état de la session en cours).
    collapsed: RefCell<HashSet<String>>,
    store: RefCell<SessionStore>,
    path: Option<PathBuf>,
    tab_view: adw::TabView,
    toasts: adw::ToastOverlay,
    window: adw::ApplicationWindow,
}

/// Construit le panneau ; `toasts` reçoit les messages d'état.
pub fn build(
    tab_view: &adw::TabView,
    toasts: &adw::ToastOverlay,
    window: &adw::ApplicationWindow,
) -> (gtk::Widget, SessionsHandle) {
    let path = SessionStore::default_path();
    let (store, load_error) = match path.as_deref().map(SessionStore::load) {
        Some(Ok(store)) => (store, None),
        Some(Err(error)) => (SessionStore::new(), Some(error.to_string())),
        None => (SessionStore::new(), None),
    };

    // Chaque ligne a sa propre liste dans un `Revealer` : le dossier se déroule
    // en animant la hauteur des lignes, sans reconstruire la liste.
    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    list.set_margin_top(6);
    list.set_margin_bottom(6);
    list.set_margin_start(6);
    list.set_margin_end(6);

    let panel = Rc::new(Panel {
        list: list.clone(),
        rows: RefCell::new(Vec::new()),
        filter: RefCell::new(String::new()),
        matches: RefCell::new(None),
        first_match: Cell::new(None),
        collapsed: RefCell::new(HashSet::new()),
        store: RefCell::new(store),
        // Fichier illisible : on n'écrase rien tant que l'utilisateur ne le corrige pas.
        path: if load_error.is_some() { None } else { path },
        tab_view: tab_view.clone(),
        toasts: toasts.clone(),
        window: window.clone(),
    });
    if let Some(message) = load_error {
        panel.toast(&format!("{message} — sauvegarde désactivée"));
    }

    let add_button = gtk::Button::from_icon_name("list-add-symbolic");
    add_button.set_tooltip_text(Some("Nouvelle session"));
    add_button.connect_clicked({
        let panel = panel.clone();
        move |_| panel.edit(None, None)
    });
    let folder_button = gtk::Button::from_icon_name("folder-new-symbolic");
    folder_button.set_tooltip_text(Some("Nouveau dossier"));
    folder_button.connect_clicked({
        let panel = panel.clone();
        move |_| panel.prompt_new_folder(None)
    });
    let import_button = gtk::Button::from_icon_name("document-open-symbolic");
    import_button.set_tooltip_text(Some("Importer ~/.ssh/config"));
    import_button.connect_clicked({
        let panel = panel.clone();
        move |_| panel.import_ssh_config()
    });
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    actions.set_margin_top(6);
    actions.set_margin_bottom(6);
    actions.set_margin_start(6);
    actions.append(&add_button);
    actions.append(&folder_button);
    actions.append(&import_button);

    let scrolled = gtk::ScrolledWindow::builder()
        .child(&list)
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
    // Fond opaque : le volet n'est plus dans un conteneur libadwaita qui le peint.
    container.add_css_class("leeloo-sidebar");
    let search_entry = gtk::SearchEntry::builder()
        .placeholder_text("Rechercher une session (Ctrl+Maj+K)")
        .margin_start(6)
        .margin_end(6)
        .margin_bottom(6)
        .build();
    search_entry.connect_search_changed({
        let panel = panel.clone();
        move |entry| panel.set_filter(&entry.text())
    });
    // Entrée lance la première session affichée.
    search_entry.connect_activate({
        let panel = panel.clone();
        move |_| {
            if let Some(id) = panel.first_match.get() {
                panel.connect(id);
            }
        }
    });
    let search_action = gtk::gio::SimpleAction::new("search-sessions", None);
    search_action.connect_activate({
        let container = container.downgrade();
        let entry = search_entry.downgrade();
        move |_, _| {
            if let (Some(container), Some(entry)) = (container.upgrade(), entry.upgrade()) {
                container.set_visible(true);
                entry.grab_focus();
            }
        }
    });
    window.add_action(&search_action);
    if let Some(application) = window.application() {
        application.set_accels_for_action("win.search-sessions", &["<Control><Shift>k"]);
    }
    container.append(&actions);
    container.append(&search_entry);
    container.append(&scrolled);

    panel.refresh();
    (container.upcast(), SessionsHandle(panel))
}

/// Accès au magasin de sessions depuis les autres fenêtres (sélecteur Azure).
#[derive(Clone)]
pub struct SessionsHandle(Rc<Panel>);

impl SessionsHandle {
    /// Commande d'un volet à restaurer ; `None` si la session n'existe plus ou
    /// n'est plus valide. La ligne de commande est toujours reconstruite et
    /// validée ici, jamais lue du disque.
    pub fn resolve_pane(&self, source: &PaneSource) -> Option<terminal::Launch> {
        match source {
            PaneSource::Local => Some(terminal::local_launch()),
            PaneSource::Session { id } => {
                let store = self.0.store.borrow();
                let session = store.get(*id)?;
                Some(terminal::Launch {
                    title: session.name.clone(),
                    argv: session.argv().ok()?,
                    close_on_exit: false,
                    style: tab_style(session),
                })
            }
            PaneSource::Azure { vm } => Some(terminal::Launch {
                title: vm.vm_name.clone(),
                argv: vm.checked_argv().ok()?,
                close_on_exit: false,
                style: terminal::TabStyle::default(),
            }),
        }
    }

    /// Dossiers repliés, pour la restauration de session.
    pub fn collapsed_folders(&self) -> Vec<String> {
        let mut folders: Vec<String> = self.0.collapsed.borrow().iter().cloned().collect();
        folders.sort();
        folders
    }

    pub fn restore_collapsed(&self, folders: &[String]) {
        *self.0.collapsed.borrow_mut() = folders.iter().cloned().collect();
        self.0.refresh();
    }

    /// Enregistre une VM Azure dans le dossier « Azure » ; `Ok(false)` si elle
    /// l'est déjà.
    pub fn add_azure_vm(&self, name: &str, vm: AzureVmSession) -> Result<bool, String> {
        let added = self
            .0
            .store
            .borrow_mut()
            .add_azure_vm(name, Some(AZURE_FOLDER), vm)
            .map_err(|error| error.to_string())?
            .is_some();
        if added {
            self.0.save();
            self.0.refresh();
        }
        Ok(added)
    }
}

impl Panel {
    fn toast(&self, message: &str) {
        self.toasts.add_toast(adw::Toast::new(message));
    }

    fn save(&self) {
        let Some(path) = &self.path else {
            return;
        };
        if let Err(error) = self.store.borrow().save(path) {
            self.toast(&format!("Sauvegarde impossible : {error}"));
        }
    }

    /// Applique la recherche rapide et reconstruit la liste.
    fn set_filter(self: &Rc<Self>, text: &str) {
        let text = text.trim().to_owned();
        if *self.filter.borrow() == text {
            return;
        }
        *self.filter.borrow_mut() = text;
        self.refresh();
    }

    /// Une recherche en cours déroule tout : le repli ne s'applique pas.
    fn is_collapsed(&self, path: &str) -> bool {
        self.matches.borrow().is_none() && self.collapsed.borrow().contains(path)
    }

    fn refresh(self: &Rc<Self>) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        self.rows.borrow_mut().clear();
        let store = self.store.borrow();
        *self.matches.borrow_mut() = store.search(&self.filter.borrow());
        self.first_match.set(None);
        self.render_level(&store, None, 0, &mut Vec::new());
        if self.matches.borrow().is_some() && self.rows.borrow().is_empty() {
            let empty = gtk::Label::new(Some("Aucune session ne correspond"));
            empty.add_css_class("dim-label");
            empty.set_margin_top(12);
            self.list.append(&empty);
        }
    }

    /// Ajoute une ligne dans son `Revealer` ; elle naît révélée ou masquée selon
    /// l'état de ses dossiers parents, sans animation.
    fn push_row(
        &self,
        row: &adw::ActionRow,
        ancestors: &[String],
        folder: Option<(String, gtk::Image)>,
    ) {
        let single = gtk::ListBox::new();
        single.add_css_class("navigation-sidebar");
        single.add_css_class("leeloo-row-list");
        single.set_selection_mode(gtk::SelectionMode::None);
        single.append(row);
        let hidden = ancestors.iter().any(|path| self.is_collapsed(path));
        let revealer = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SlideDown)
            .transition_duration(FOLDER_ANIMATION_MS)
            .reveal_child(!hidden)
            .child(&single)
            .build();
        self.list.append(&revealer);
        self.rows.borrow_mut().push(RowEntry {
            revealer,
            ancestors: ancestors.to_vec(),
            folder,
        });
    }

    /// Aligne l'affichage sur l'état replié : les lignes concernées se
    /// déroulent ou s'enroulent, les icônes de dossier suivent.
    fn apply_collapsed(&self) {
        for entry in self.rows.borrow().iter() {
            let hidden = entry.ancestors.iter().any(|path| self.is_collapsed(path));
            entry.revealer.set_reveal_child(!hidden);
            if let Some((path, icon)) = &entry.folder {
                icon.set_icon_name(Some(folder_icon(self.is_collapsed(path))));
            }
        }
    }

    /// Sous-dossiers (chacun suivi de son contenu s'il n'est pas replié), puis
    /// les sessions de ce niveau.
    fn render_level(
        self: &Rc<Self>,
        store: &SessionStore,
        parent: Option<&str>,
        depth: i32,
        ancestors: &mut Vec<String>,
    ) {
        for folder in store.child_folders(parent) {
            // Recherche : seuls les dossiers qui contiennent un résultat restent.
            if self.matches.borrow().as_ref().is_some_and(|ids| {
                !store
                    .sessions_under(&folder)
                    .any(|session| ids.contains(&session.id))
            }) {
                continue;
            }
            let collapsed = self.is_collapsed(&folder);
            let (row, icon) = self.folder_row(store, &folder, depth, collapsed);
            self.push_row(&row, ancestors, Some((folder.clone(), icon)));
            ancestors.push(folder.clone());
            self.render_level(store, Some(&folder), depth + 1, ancestors);
            ancestors.pop();
        }
        for session in store.in_folder(parent) {
            if self
                .matches
                .borrow()
                .as_ref()
                .is_some_and(|ids| !ids.contains(&session.id))
            {
                continue;
            }
            if self.first_match.get().is_none() {
                self.first_match.set(Some(session.id));
            }
            let row = self.session_row(session);
            row.set_margin_start(depth * INDENT);
            self.push_row(&row, ancestors, None);
        }
    }

    fn folder_row(
        self: &Rc<Self>,
        store: &SessionStore,
        path: &str,
        depth: i32,
        collapsed: bool,
    ) -> (adw::ActionRow, gtk::Image) {
        let count = store.sessions_in_subtree(path);
        let row = adw::ActionRow::builder()
            .title(gtk::glib::markup_escape_text(folder_name(path)))
            .activatable(true)
            .build();
        // Le nombre de sessions tient sur la même ligne : ligne de dossier fine.
        let count_label = gtk::Label::new(Some(&count.to_string()));
        count_label.add_css_class("dim-label");
        count_label.add_css_class("caption");
        count_label.set_tooltip_text(Some(&format!("{count} session(s)")));
        row.add_suffix(&count_label);
        row.set_margin_start(depth * INDENT);
        row.add_css_class("leeloo-folder");
        let icon = gtk::Image::from_icon_name(folder_icon(collapsed));
        row.add_prefix(&icon);

        let icon_button = |icon: &str, tooltip: &str| {
            let button = gtk::Button::from_icon_name(icon);
            button.set_valign(gtk::Align::Center);
            button.add_css_class("flat");
            button.set_tooltip_text(Some(tooltip));
            button
        };
        let new_folder = icon_button("folder-new-symbolic", "Nouveau sous-dossier");
        new_folder.connect_clicked({
            let panel = self.clone();
            let path = path.to_owned();
            move |_| panel.prompt_new_folder(Some(&path))
        });
        let new_session = icon_button("list-add-symbolic", "Nouvelle session dans ce dossier");
        new_session.connect_clicked({
            let panel = self.clone();
            let path = path.to_owned();
            move |_| panel.edit(None, Some(path.clone()))
        });
        row.add_suffix(&new_folder);
        row.add_suffix(&new_session);
        row.add_suffix(&self.folder_menu(path));

        row.connect_activated({
            let panel = self.clone();
            let path = path.to_owned();
            move |_| {
                {
                    let mut collapsed = panel.collapsed.borrow_mut();
                    if !collapsed.remove(&path) {
                        collapsed.insert(path.clone());
                    }
                }
                panel.apply_collapsed();
            }
        });
        (row, icon)
    }

    /// Menu « … » d'un dossier : renommer, déplacer, supprimer.
    fn folder_menu(self: &Rc<Self>, path: &str) -> gtk::MenuButton {
        let popover = gtk::Popover::new();
        let items = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let entries: [(&str, FolderAction); 3] = [
            ("Renommer…", Panel::prompt_rename_folder),
            ("Déplacer…", Panel::prompt_move_folder),
            ("Supprimer", Panel::confirm_delete_folder),
        ];
        for (label, action) in entries {
            let button = gtk::Button::with_label(label);
            button.add_css_class("flat");
            if label == "Supprimer" {
                button.add_css_class("destructive-action");
            }
            button.connect_clicked({
                let panel = self.clone();
                let popover = popover.clone();
                let path = path.to_owned();
                move |_| {
                    popover.popdown();
                    let panel = panel.clone();
                    let path = path.clone();
                    // Différé : le menu se ferme avant l'ouverture du dialogue.
                    gtk::glib::idle_add_local_once(move || action(&panel, &path));
                }
            });
            items.append(&button);
        }
        popover.set_child(Some(&items));
        let menu_button = gtk::MenuButton::new();
        menu_button.set_icon_name("view-more-symbolic");
        menu_button.set_valign(gtk::Align::Center);
        menu_button.add_css_class("flat");
        menu_button.set_tooltip_text(Some("Renommer, déplacer, supprimer"));
        menu_button.set_popover(Some(&popover));
        menu_button
    }

    /// Les dossiers repliés suivent un renommage ou un déplacement.
    fn rekey_collapsed(&self, old: &str, new: &str) {
        let mut collapsed = self.collapsed.borrow_mut();
        let moved: Vec<String> = collapsed
            .iter()
            .filter(|path| {
                path.as_str() == old || path.starts_with(&format!("{old}{FOLDER_SEPARATOR}"))
            })
            .cloned()
            .collect();
        for path in moved {
            collapsed.remove(&path);
            collapsed.insert(format!("{new}{}", &path[old.len()..]));
        }
    }

    fn apply_folder_change(
        self: &Rc<Self>,
        old: &str,
        result: Result<String, impl std::fmt::Display>,
    ) {
        match result {
            Ok(new) => {
                self.rekey_collapsed(old, &new);
                self.save();
                self.refresh();
            }
            Err(error) => self.toast(&format!("Opération impossible : {error}")),
        }
    }

    fn prompt_rename_folder(self: &Rc<Self>, path: &str) {
        let entry = gtk::Entry::builder()
            .text(folder_name(path))
            .max_length(64)
            .activates_default(true)
            .build();
        entry.select_region(0, -1);
        let dialog = adw::AlertDialog::new(Some("Renommer le dossier"), None);
        dialog.set_extra_child(Some(&entry));
        dialog.add_responses(&[("cancel", "Annuler"), ("rename", "Renommer")]);
        dialog.set_response_appearance("rename", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("rename"));
        dialog.set_close_response("cancel");
        let panel = self.clone();
        let path = path.to_owned();
        dialog.choose(Some(&self.window), gtk::gio::Cancellable::NONE, {
            let entry = entry.clone();
            move |response| {
                if response == "rename" {
                    let result = panel.store.borrow_mut().rename_folder(&path, &entry.text());
                    panel.apply_folder_change(&path, result);
                }
            }
        });
        entry.grab_focus();
    }

    fn prompt_move_folder(self: &Rc<Self>, path: &str) {
        let destinations = self.store.borrow().move_destinations(path);
        let labels: Vec<String> = std::iter::once("(racine)".to_owned())
            .chain(
                destinations
                    .iter()
                    .map(|destination| destination.replace(FOLDER_SEPARATOR, " › ")),
            )
            .collect();
        let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let dropdown = gtk::DropDown::new(
            Some(gtk::StringList::new(&label_refs)),
            gtk::Expression::NONE,
        );

        let dialog = adw::AlertDialog::new(
            Some("Déplacer le dossier"),
            Some(&format!(
                "Choisir le dossier de destination de « {} ».",
                folder_name(path)
            )),
        );
        dialog.set_extra_child(Some(&dropdown));
        dialog.add_responses(&[("cancel", "Annuler"), ("move", "Déplacer")]);
        dialog.set_response_appearance("move", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("move"));
        dialog.set_close_response("cancel");
        let panel = self.clone();
        let path = path.to_owned();
        dialog.choose(Some(&self.window), gtk::gio::Cancellable::NONE, {
            let dropdown = dropdown.clone();
            move |response| {
                if response != "move" {
                    return;
                }
                let target = usize::try_from(dropdown.selected())
                    .ok()
                    .and_then(|index| index.checked_sub(1))
                    .and_then(|index| destinations.get(index))
                    .cloned();
                let result = panel
                    .store
                    .borrow_mut()
                    .move_folder(&path, target.as_deref());
                panel.apply_folder_change(&path, result);
            }
        });
    }

    /// Suppression d'un dossier : sans confirmation s'il est vide, sinon le
    /// dialogue indique combien de sessions disparaîtront.
    fn confirm_delete_folder(self: &Rc<Self>, path: &str) {
        let count = self.store.borrow().sessions_in_subtree(path);
        if count == 0 {
            self.delete_folder(path);
            return;
        }
        let label = path.replace(FOLDER_SEPARATOR, " › ");
        let dialog = adw::AlertDialog::new(
            Some("Supprimer le dossier ?"),
            Some(&format!(
                "« {label} », ses sous-dossiers et {count} session(s) seront supprimés (annulable quelques secondes)."
            )),
        );
        dialog.add_responses(&[("cancel", "Annuler"), ("delete", "Supprimer")]);
        dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        let panel = self.clone();
        let path = path.to_owned();
        dialog.choose(
            Some(&self.window),
            gtk::gio::Cancellable::NONE,
            move |response| {
                if response == "delete" {
                    panel.delete_folder(&path);
                }
            },
        );
    }

    fn delete_folder(self: &Rc<Self>, path: &str) {
        let result = self.store.borrow_mut().take_folder(path);
        match result {
            Ok(backup) => {
                let prefix = format!("{path}{FOLDER_SEPARATOR}");
                // Dossiers repliés retirés, rendus à l'annulation.
                let removed_collapsed: Vec<String> = {
                    let mut collapsed = self.collapsed.borrow_mut();
                    let removed = collapsed
                        .iter()
                        .filter(|known| known.as_str() == path || known.starts_with(&prefix))
                        .cloned()
                        .collect();
                    collapsed.retain(|known| known != path && !known.starts_with(&prefix));
                    removed
                };
                self.save();
                let panel = self.clone();
                gtk::glib::idle_add_local_once(move || {
                    panel.refresh();
                    panel.offer_undo(backup, removed_collapsed);
                });
            }
            Err(error) => self.toast(&format!("Suppression impossible : {error}")),
        }
    }

    /// Message « Dossier supprimé » avec un bouton « Annuler » (10 s).
    fn offer_undo(self: &Rc<Self>, backup: FolderBackup, collapsed: Vec<String>) {
        let sessions = backup.session_count();
        let label = backup.path().replace(FOLDER_SEPARATOR, " › ");
        let title = if sessions > 0 {
            format!("Dossier « {label} » supprimé ({sessions} session(s))")
        } else {
            format!("Dossier « {label} » supprimé")
        };
        let toast = adw::Toast::builder()
            .title(gtk::glib::markup_escape_text(&title))
            .button_label("Annuler")
            .timeout(10)
            .build();
        // Annulation unique : le bouton consomme la sauvegarde.
        let backup = RefCell::new(Some(backup));
        let collapsed = RefCell::new(Some(collapsed));
        toast.connect_button_clicked({
            let panel = self.clone();
            move |_| {
                let (Some(backup), Some(collapsed)) =
                    (backup.borrow_mut().take(), collapsed.borrow_mut().take())
                else {
                    return;
                };
                panel.store.borrow_mut().restore_folder(backup);
                panel.collapsed.borrow_mut().extend(collapsed);
                panel.save();
                panel.refresh();
            }
        });
        self.toasts.add_toast(toast);
    }

    /// Demande le nom d'un dossier ; « Azure/Dev » crée les deux niveaux.
    fn prompt_new_folder(self: &Rc<Self>, parent: Option<&str>) {
        let entry = gtk::Entry::builder()
            .placeholder_text("Nom (ex. Dev, ou Azure/Dev)")
            .max_length(128)
            .activates_default(true)
            .build();
        let heading = match parent {
            Some(parent) => format!(
                "Nouveau sous-dossier de « {} »",
                parent.replace(FOLDER_SEPARATOR, " › ")
            ),
            None => "Nouveau dossier".to_owned(),
        };
        let dialog = adw::AlertDialog::new(Some(&heading), None);
        dialog.set_extra_child(Some(&entry));
        dialog.add_responses(&[("cancel", "Annuler"), ("create", "Créer")]);
        dialog.set_response_appearance("create", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("create"));
        dialog.set_close_response("cancel");

        let panel = self.clone();
        let parent = parent.map(str::to_owned);
        dialog.choose(Some(&self.window), gtk::gio::Cancellable::NONE, {
            let entry = entry.clone();
            move |response| {
                if response != "create" {
                    return;
                }
                let result = panel
                    .store
                    .borrow_mut()
                    .create_folder(parent.as_deref(), &entry.text());
                match result {
                    Ok(path) => {
                        // Le dossier parent doit rester visible.
                        panel.collapsed.borrow_mut().retain(|collapsed| {
                            !path.starts_with(&format!("{collapsed}{FOLDER_SEPARATOR}"))
                        });
                        panel.save();
                        panel.refresh();
                    }
                    Err(error) => panel.toast(&format!("Création impossible : {error}")),
                }
            }
        });
        entry.grab_focus();
    }

    fn session_row(self: &Rc<Self>, session: &Session) -> adw::ActionRow {
        let row = adw::ActionRow::builder()
            .title(gtk::glib::markup_escape_text(&session.name))
            .activatable(true)
            .build();
        // Hôte ou VM en infobulle : la ligne reste fine (une seule ligne).
        row.set_tooltip_text(Some(&session.summary()));
        row.add_css_class("leeloo-session");
        let id = session.id;
        // `#rrggbb` validé par le modèle : sans risque dans le balisage Pango.
        if let Some(color) = session
            .color
            .as_deref()
            .filter(|color| palette::parse_hex(color).is_some())
        {
            let dot = gtk::Label::new(None);
            dot.set_markup(&format!(
                "<span foreground=\"{color}\" size=\"x-large\">●</span>"
            ));
            row.add_prefix(&dot);
        }

        let edit_button = gtk::Button::from_icon_name("document-edit-symbolic");
        edit_button.set_valign(gtk::Align::Center);
        edit_button.add_css_class("flat");
        edit_button.set_tooltip_text(Some("Modifier"));
        edit_button.connect_clicked({
            let panel = self.clone();
            move |_| panel.edit(Some(id), None)
        });
        let delete_button = gtk::Button::from_icon_name("user-trash-symbolic");
        delete_button.set_valign(gtk::Align::Center);
        delete_button.add_css_class("flat");
        delete_button.set_tooltip_text(Some("Supprimer"));
        delete_button.connect_clicked({
            let panel = self.clone();
            move |_| panel.confirm_delete(id)
        });
        row.add_suffix(&edit_button);
        row.add_suffix(&delete_button);
        row.connect_activated({
            let panel = self.clone();
            move |_| panel.connect(id)
        });
        row
    }

    fn connect(&self, id: u64) {
        let session = self.store.borrow().get(id).cloned();
        let Some(session) = session else {
            return;
        };
        match session.argv() {
            Ok(argv) => {
                terminal::open_session_tab(
                    &self.tab_view,
                    &session.name,
                    argv,
                    tab_style(&session),
                    PaneSource::Session { id },
                );
            }
            Err(error) => self.toast(&format!("Session invalide : {error}")),
        }
    }

    fn import_ssh_config(self: &Rc<Self>) {
        match ssh_config::read_default_aliases() {
            Ok(aliases) => {
                let added = self
                    .store
                    .borrow_mut()
                    .import_ssh_aliases(&aliases, Some(IMPORT_FOLDER));
                self.save();
                self.refresh();
                self.toast(&format!("{added} session(s) importée(s)"));
            }
            Err(error) => self.toast(&format!("Lecture de ~/.ssh/config impossible : {error}")),
        }
    }

    fn confirm_delete(self: &Rc<Self>, id: u64) {
        let name = self
            .store
            .borrow()
            .get(id)
            .map(|session| session.name.clone())
            .unwrap_or_default();
        let dialog = adw::AlertDialog::new(
            Some("Supprimer la session ?"),
            Some(&format!("« {name} » sera supprimée.")),
        );
        dialog.add_responses(&[("cancel", "Annuler"), ("delete", "Supprimer")]);
        dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        let panel = self.clone();
        dialog.choose(
            Some(&self.window),
            gtk::gio::Cancellable::NONE,
            move |response| {
                if response == "delete" {
                    panel.store.borrow_mut().remove(id);
                    panel.save();
                    panel.refresh();
                }
            },
        );
    }

    /// Dialogue de création (`None`) ou d'édition d'une session.
    fn edit(self: &Rc<Self>, id: Option<u64>, default_folder: Option<String>) {
        let existing = id.and_then(|id| self.store.borrow().get(id).cloned());
        let folders = self.store.borrow().folders();
        let form = Form::new(existing.as_ref(), folders, default_folder.as_deref());

        let dialog = adw::Dialog::builder()
            .title(if existing.is_some() {
                "Modifier la session"
            } else {
                "Nouvelle session"
            })
            .content_width(480)
            .child(&form.widget)
            .build();

        form.save_button.connect_clicked({
            let panel = self.clone();
            let form = form.clone();
            let dialog = dialog.clone();
            move |_| match form.build_session(id) {
                Ok(session) => {
                    let result = {
                        let mut store = panel.store.borrow_mut();
                        if session.id == 0 {
                            store.add_session(session).map(|_| ())
                        } else {
                            store.update(session).map(|_| ())
                        }
                    };
                    match result {
                        Ok(()) => {
                            panel.save();
                            panel.refresh();
                            dialog.close();
                        }
                        Err(error) => form.show_error(&error.to_string()),
                    }
                }
                Err(message) => form.show_error(&message),
            }
        });
        dialog.present(Some(&self.window));
    }
}

#[derive(Clone)]
struct Form {
    widget: adw::ToolbarView,
    save_button: gtk::Button,
    error_label: gtk::Label,
    kind: adw::ComboRow,
    name: adw::EntryRow,
    folder: adw::ComboRow,
    folder_paths: Vec<String>,
    host: adw::EntryRow,
    user: adw::EntryRow,
    port: adw::EntryRow,
    identity_file: adw::EntryRow,
    jump_host: adw::EntryRow,
    subscription: adw::EntryRow,
    resource_group: adw::EntryRow,
    vm_name: adw::EntryRow,
    prefer_private_ip: adw::SwitchRow,
    color: adw::ComboRow,
    palette: adw::ComboRow,
}

fn entry(title: &str) -> adw::EntryRow {
    adw::EntryRow::builder().title(title).build()
}

impl Form {
    fn new(
        existing: Option<&Session>,
        folder_paths: Vec<String>,
        default_folder: Option<&str>,
    ) -> Self {
        let kind = adw::ComboRow::builder()
            .title("Type")
            .model(&gtk::StringList::new(&["SSH", "VM Azure (Entra ID)"]))
            .build();
        let form = Self {
            widget: adw::ToolbarView::new(),
            save_button: gtk::Button::with_label("Enregistrer"),
            error_label: gtk::Label::new(None),
            kind,
            name: entry("Nom"),
            folder: adw::ComboRow::builder()
                .title("Dossier")
                .model(&gtk::StringList::new(
                    &std::iter::once("(racine)".to_owned())
                        .chain(
                            folder_paths
                                .iter()
                                .map(|path| path.replace(FOLDER_SEPARATOR, " › ")),
                        )
                        .collect::<Vec<_>>()
                        .iter()
                        .map(String::as_str)
                        .collect::<Vec<_>>(),
                ))
                .build(),
            folder_paths: folder_paths.clone(),
            host: entry("Hôte ou alias"),
            user: entry("Utilisateur (optionnel)"),
            port: entry("Port (optionnel)"),
            identity_file: entry("Fichier de clé (optionnel)"),
            jump_host: entry("Rebond ProxyJump (optionnel)"),
            subscription: entry("Subscription (optionnel)"),
            resource_group: entry("Resource group"),
            vm_name: entry("Nom de la VM"),
            prefer_private_ip: adw::SwitchRow::builder()
                .title("Préférer l'IP privée")
                .build(),
            color: adw::ComboRow::builder()
                .title("Couleur d'identification")
                .subtitle("Pastille dans la liste et bandeau en haut du terminal")
                .model(&gtk::StringList::new(
                    &std::iter::once("Aucune")
                        .chain(SESSION_COLORS.iter().map(|(name, _)| *name))
                        .collect::<Vec<_>>(),
                ))
                .build(),
            palette: adw::ComboRow::builder()
                .title("Palette du terminal")
                .model(&gtk::StringList::new(
                    &std::iter::once("Celle de l'application")
                        .chain(PALETTES.iter().map(|palette| palette.name))
                        .collect::<Vec<_>>(),
                ))
                .build(),
        };

        let general = adw::PreferencesGroup::new();
        general.add(&form.kind);
        general.add(&form.name);
        general.add(&form.folder);
        general.add(&form.color);
        general.add(&form.palette);
        let ssh_group = adw::PreferencesGroup::builder().title("SSH").build();
        for row in [
            &form.host,
            &form.user,
            &form.port,
            &form.identity_file,
            &form.jump_host,
        ] {
            ssh_group.add(row);
        }
        let azure_group = adw::PreferencesGroup::builder().title("VM Azure").build();
        for row in [&form.subscription, &form.resource_group, &form.vm_name] {
            azure_group.add(row);
        }
        azure_group.add(&form.prefer_private_ip);

        form.error_label.add_css_class("error");
        form.error_label.set_wrap(true);
        form.error_label.set_visible(false);

        let page = adw::PreferencesPage::new();
        page.add(&general);
        page.add(&ssh_group);
        page.add(&azure_group);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        body.append(&page);
        body.append(&form.error_label);
        let scrolled = gtk::ScrolledWindow::builder()
            .child(&body)
            .propagate_natural_height(true)
            .max_content_height(640)
            .build();

        let header = adw::HeaderBar::new();
        form.save_button.add_css_class("suggested-action");
        header.pack_end(&form.save_button);
        form.widget.add_top_bar(&header);
        form.widget.set_content(Some(&scrolled));

        let update_visibility = {
            let kind = form.kind.clone();
            let ssh_group = ssh_group.clone();
            let azure_group = azure_group.clone();
            move || {
                let is_ssh = kind.selected() == 0;
                ssh_group.set_visible(is_ssh);
                azure_group.set_visible(!is_ssh);
            }
        };
        form.kind
            .connect_selected_notify(move |_| update_visibility());

        if let Some(session) = existing {
            form.fill(session);
        } else if let Some(default_folder) = default_folder {
            form.select_folder(Some(default_folder));
        }
        form.kind.notify("selected");
        form
    }

    fn fill(&self, session: &Session) {
        self.name.set_text(&session.name);
        self.select_folder(session.folder.as_deref());
        match &session.kind {
            SessionKind::Ssh(ssh) => {
                self.kind.set_selected(0);
                self.host.set_text(&ssh.host);
                self.user.set_text(ssh.user.as_deref().unwrap_or(""));
                self.port
                    .set_text(&ssh.port.map(|p| p.to_string()).unwrap_or_default());
                self.identity_file
                    .set_text(ssh.identity_file.as_deref().unwrap_or(""));
                self.jump_host
                    .set_text(ssh.jump_host.as_deref().unwrap_or(""));
            }
            SessionKind::AzureVm(vm) => {
                self.kind.set_selected(1);
                self.subscription
                    .set_text(vm.subscription.as_deref().unwrap_or(""));
                self.resource_group.set_text(&vm.resource_group);
                self.vm_name.set_text(&vm.vm_name);
                self.prefer_private_ip.set_active(vm.prefer_private_ip);
            }
        }
    }

    fn select_folder(&self, folder: Option<&str>) {
        let index = folder
            .and_then(|folder| self.folder_paths.iter().position(|path| path == folder))
            .map_or(0, |index| index as u32 + 1);
        self.folder.set_selected(index);
    }

    fn show_error(&self, message: &str) {
        self.error_label.set_text(message);
        self.error_label.set_visible(true);
    }

    /// Session issue du formulaire ; `id == 0` pour une nouvelle session.
    fn build_session(&self, id: Option<u64>) -> Result<Session, String> {
        let optional = |row: &adw::EntryRow| {
            let text = row.text();
            let trimmed = text.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        };
        let kind = if self.kind.selected() == 0 {
            let port = match optional(&self.port) {
                Some(text) => Some(
                    text.parse::<u16>()
                        .map_err(|_| "Port invalide".to_owned())?,
                ),
                None => None,
            };
            SessionKind::Ssh(SshSession {
                host: self.host.text().trim().to_owned(),
                user: optional(&self.user),
                port,
                identity_file: optional(&self.identity_file),
                jump_host: optional(&self.jump_host),
            })
        } else {
            SessionKind::AzureVm(AzureVmSession {
                subscription: optional(&self.subscription),
                resource_group: self.resource_group.text().trim().to_owned(),
                vm_name: self.vm_name.text().trim().to_owned(),
                prefer_private_ip: self.prefer_private_ip.is_active(),
            })
        };
        Ok(Session {
            id: id.unwrap_or(0),
            name: self.name.text().to_string(),
            folder: usize::try_from(self.folder.selected())
                .ok()
                .and_then(|index| index.checked_sub(1))
                .and_then(|index| self.folder_paths.get(index))
                .cloned(),
            color: usize::try_from(self.color.selected())
                .ok()
                .and_then(|index| index.checked_sub(1))
                .and_then(|index| SESSION_COLORS.get(index))
                .map(|(_, hex)| (*hex).to_owned()),
            palette: usize::try_from(self.palette.selected())
                .ok()
                .and_then(|index| index.checked_sub(1))
                .and_then(|index| PALETTES.get(index))
                .map(|palette| palette.id.to_owned()),
            kind,
        })
    }
}
