mod azure_dialog;
mod rename_dialog;
mod sessions_panel;
mod style;
mod terminal;

use adw::prelude::*;
use leeloo::palette::{self, DEFAULT_PALETTE_ID, PALETTES};
use leeloo::settings::{
    DEFAULT_FONT_SCALE_PERCENT, MAX_OPACITY_PERCENT, MIN_OPACITY_PERCENT, MIN_SIDEBAR_WIDTH,
    Settings, Theme, clamp_opacity_percent, clamp_sidebar_width, step_font_scale_percent,
};
use leeloo::tab_nav::nth_tab_index;
use leeloo::workspace::{PaneSource, Workspace};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

const APP_ID: &str = "io.github.Schoomee.Leeloo";

fn main() -> gtk::glib::ExitCode {
    // Reprise de l'ancien dossier de configuration (avant le renommage en Leeloo).
    if let Err(error) = leeloo::private_file::migrate_legacy_config_dir() {
        eprintln!("Reprise de l'ancienne configuration impossible : {error}");
    }
    // Sous X11, la classe de fenêtre (WM_CLASS) vient du nom de programme : elle
    // doit valoir l'identifiant de l'application pour que le gestionnaire de
    // fenêtres retrouve le `.desktop` (StartupWMClass) et donc l'icône.
    gtk::glib::set_prgname(Some(APP_ID));
    let application = adw::Application::builder().application_id(APP_ID).build();
    application.connect_startup(|_| gtk::Window::set_default_icon_name(APP_ID));
    application.connect_activate(build_window);
    application.run()
}

fn build_window(application: &adw::Application) {
    let tab_view = adw::TabView::new();
    tab_view.set_vexpand(true);
    let tab_bar = adw::TabBar::builder()
        .view(&tab_view)
        .autohide(false)
        .build();

    let tabs = gtk::Box::new(gtk::Orientation::Vertical, 0);
    tabs.append(&tab_bar);
    tabs.append(&tab_view);

    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&tabs));

    // Volet redimensionnable à la souris : la poignée de GtkPaned gère le glissement.
    let paned = gtk::Paned::new(gtk::Orientation::Horizontal);
    paned.set_end_child(Some(&toasts));
    paned.set_resize_start_child(false);
    paned.set_resize_end_child(true);
    paned.set_shrink_start_child(false);
    paned.set_shrink_end_child(false);
    paned.set_wide_handle(true);

    let header = adw::HeaderBar::new();
    let sidebar_toggle = gtk::ToggleButton::new();
    sidebar_toggle.set_icon_name("sidebar-show-symbolic");
    sidebar_toggle.set_tooltip_text(Some("Afficher les sessions"));
    header.pack_start(&sidebar_toggle);
    let new_tab_button = gtk::Button::from_icon_name("tab-new-symbolic");
    new_tab_button.set_tooltip_text(Some("Nouveau terminal local"));
    header.pack_start(&new_tab_button);
    let azure_button = gtk::Button::with_label("Azure");
    azure_button.set_icon_name("network-server-symbolic");
    azure_button.set_tooltip_text(Some("Se connecter à une VM Azure (Entra ID)"));
    header.pack_start(&azure_button);

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&paned));

    let window = adw::ApplicationWindow::builder()
        .application(application)
        .title("Leeloo")
        .default_width(1200)
        .default_height(750)
        .content(&toolbar_view)
        .build();

    let (sessions_widget, sessions) = sessions_panel::build(&tab_view, &toasts, &window);
    install_tab_shortcuts(application, &window, &tab_view);
    let preferences = install_window_actions(
        application,
        &window,
        &header,
        &toasts,
        &tab_view,
        &SidebarParts {
            paned: &paned,
            widget: &sessions_widget,
            toggle: &sidebar_toggle,
        },
    );

    new_tab_button.connect_clicked({
        let tab_view = tab_view.clone();
        move |_| terminal::open_local_tab(&tab_view)
    });

    azure_button.connect_clicked({
        let window = window.clone();
        let tab_view = tab_view.clone();
        let toasts = toasts.clone();
        let sessions = sessions.clone();
        move |_| azure_dialog::present(&window, &tab_view, &toasts, &sessions)
    });

    // Ferme la fenêtre quand le dernier onglet est fermé.
    // Ferme l'onglet, après confirmation s'il a une commande ou une connexion
    // active (sinon immédiatement). Un refus doit être signalé à la vue.
    tab_view.connect_close_page({
        let window = window.clone();
        move |view, page| {
            let live = terminal::live_panes_in_page(page);
            if live == 0 || !terminal::confirm_close_enabled() {
                finish_tab_close(&window, view, page, true);
            } else {
                let (window_ref, view, page) = (window.clone(), view.clone(), page.clone());
                terminal::confirm_dialog(
                    Some(window.upcast_ref()),
                    "Fermer cet onglet ?",
                    &terminal::live_summary(live),
                    "Fermer",
                    move |accepted| finish_tab_close(&window_ref, &view, &page, accepted),
                );
            }
            gtk::glib::Propagation::Stop
        }
    });

    restore_or_open_first_tab(&tab_view, &sessions, &preferences, &toasts);
    // Fermeture de la fenêtre : confirmation si des commandes ou connexions sont
    // actives, puis enregistrement de la session.
    let confirmed = Rc::new(std::cell::Cell::new(false));
    window.connect_close_request({
        let tab_view = tab_view.clone();
        let sessions = sessions.clone();
        let preferences = preferences.clone();
        move |window| {
            let live = terminal::live_panes(&tab_view);
            if live > 0 && terminal::confirm_close_enabled() && !confirmed.get() {
                let (target, confirmed) = (window.clone(), confirmed.clone());
                terminal::confirm_dialog(
                    Some(window.upcast_ref()),
                    "Quitter Leeloo ?",
                    &terminal::live_summary(live),
                    "Quitter",
                    move |accepted| {
                        if accepted {
                            confirmed.set(true);
                            target.close();
                        }
                    },
                );
                return gtk::glib::Propagation::Stop;
            }
            save_workspace(&tab_view, &sessions, &preferences);
            gtk::glib::Propagation::Proceed
        }
    });
    window.present();
}

/// Termine (ou refuse) la fermeture d'un onglet ; le dernier onglet ferme la
/// fenêtre.
fn finish_tab_close(
    window: &adw::ApplicationWindow,
    view: &adw::TabView,
    page: &adw::TabPage,
    confirmed: bool,
) {
    view.close_page_finish(page, confirmed);
    if confirmed && view.n_pages() == 0 {
        window.close();
    }
}

/// Rouvre les onglets de la dernière session (si le réglage est actif), sinon
/// ouvre un terminal local.
fn restore_or_open_first_tab(
    tab_view: &adw::TabView,
    sessions: &sessions_panel::SessionsHandle,
    preferences: &Preferences,
    toasts: &adw::ToastOverlay,
) {
    if preferences.settings.borrow().restore_session
        && let Some(path) = Workspace::default_path()
    {
        match Workspace::load(&path) {
            Ok(workspace) => {
                sessions.restore_collapsed(&workspace.collapsed_folders);
                let resolve = |source: &PaneSource| sessions.resolve_pane(source);
                let mut selected_page = None;
                for (index, tab) in workspace.tabs.iter().enumerate() {
                    if terminal::restore_tab(tab_view, tab, &resolve) && index == workspace.selected
                    {
                        selected_page = Some(tab_view.nth_page(tab_view.n_pages() - 1));
                    }
                }
                if let Some(page) = selected_page.or_else(|| tab_view.selected_page()) {
                    tab_view.set_selected_page(&page);
                    terminal::focus_selected(tab_view);
                }
            }
            Err(error) => {
                toasts.add_toast(adw::Toast::new(&format!("Restauration ignorée : {error}")))
            }
        }
    }
    if tab_view.n_pages() == 0 {
        terminal::open_local_tab(tab_view);
    }
}

/// Enregistre les onglets ouverts à la fermeture. Réglage coupé : rien n'est
/// écrit, et un ancien fichier est supprimé pour ne pas ressurgir plus tard.
fn save_workspace(
    tab_view: &adw::TabView,
    sessions: &sessions_panel::SessionsHandle,
    preferences: &Preferences,
) {
    let Some(path) = Workspace::default_path() else {
        return;
    };
    if !preferences.settings.borrow().restore_session {
        let _ = std::fs::remove_file(&path);
        return;
    }
    let (tabs, selected) = terminal::capture_workspace(tab_view);
    let workspace = Workspace {
        tabs,
        selected,
        collapsed_folders: sessions.collapsed_folders(),
    };
    if let Err(error) = workspace.save(&path) {
        eprintln!("Sauvegarde de la session impossible : {error}");
    }
}

/// Fenêtre transparente (le terminal translucide laisse voir le bureau). Le
/// volet des sessions, la barre d'en-tête et la barre d'onglets gardent un fond
/// opaque : seul le terminal doit être translucide.
const APP_CSS: &str = "\
window.leeloo-translucent { background: transparent; }\n\
window.leeloo-translucent toolbarview > .top-bar { background-color: @headerbar_bg_color; }\n\
window.leeloo-translucent tabbar { background-color: @headerbar_bg_color; }\n\
window.leeloo-translucent paned > separator { background-color: @window_bg_color; }\n\
.leeloo-sidebar { background-color: @window_bg_color; }\n\
.leeloo-broadcast { outline: 2px solid @error_color; outline-offset: -2px; }\n\
list.leeloo-row-list { padding: 0; }\n\
row.leeloo-folder { margin-top: 2px; margin-bottom: 2px; border-radius: 8px; min-height: 0; \
border: 1px solid alpha(@window_fg_color, 0.12); \
background-image: linear-gradient(to bottom, alpha(@window_fg_color, 0.10), alpha(@window_fg_color, 0.02)); \
box-shadow: inset 0 1px 0 alpha(white, 0.14), 0 2px 4px alpha(black, 0.35); \
transition: box-shadow 150ms ease, background-image 150ms ease; }\n\
row.leeloo-folder > box.header { min-height: 32px; padding-top: 0; padding-bottom: 0; }\n\
row.leeloo-session { min-height: 0; margin-top: 1px; margin-bottom: 1px; }\n\
row.leeloo-session > box.header { min-height: 30px; padding-top: 0; padding-bottom: 0; }\n\
row.leeloo-folder:hover { \
background-image: linear-gradient(to bottom, alpha(@window_fg_color, 0.16), alpha(@window_fg_color, 0.05)); \
box-shadow: inset 0 1px 0 alpha(white, 0.20), 0 4px 8px alpha(black, 0.40); }\n\
tabbar tab { border-radius: 8px; \
border: 1px solid alpha(@window_fg_color, 0.12); \
background-image: linear-gradient(to bottom, alpha(@window_fg_color, 0.08), alpha(@window_fg_color, 0.01)); \
box-shadow: inset 0 1px 0 alpha(white, 0.12), 0 2px 3px alpha(black, 0.30); \
transition: box-shadow 150ms ease, background-image 150ms ease; }\n\
tabbar tab:hover { \
background-image: linear-gradient(to bottom, alpha(@window_fg_color, 0.14), alpha(@window_fg_color, 0.04)); \
box-shadow: inset 0 1px 0 alpha(white, 0.18), 0 3px 6px alpha(black, 0.36); }\n\
tabbar tab:selected { \
background-image: linear-gradient(to bottom, alpha(@window_fg_color, 0.20), alpha(@window_fg_color, 0.07)); \
box-shadow: inset 0 1px 0 alpha(white, 0.24), 0 4px 8px alpha(black, 0.42); }\n\
row.leeloo-folder:active { \
box-shadow: inset 0 2px 4px alpha(black, 0.40); \
background-image: linear-gradient(to bottom, alpha(@window_fg_color, 0.02), alpha(@window_fg_color, 0.08)); }";

/// Éléments du volet des sessions manipulés par les préférences.
struct SidebarParts<'a> {
    paned: &'a gtk::Paned,
    widget: &'a gtk::Widget,
    toggle: &'a gtk::ToggleButton,
}

struct Preferences {
    settings: RefCell<Settings>,
    path: Option<PathBuf>,
    toasts: adw::ToastOverlay,
    window: adw::ApplicationWindow,
}

impl Preferences {
    fn persist(&self) {
        if let Some(path) = &self.path
            && let Err(error) = self.settings.borrow().save(path)
        {
            self.toasts.add_toast(adw::Toast::new(&format!(
                "Sauvegarde des préférences impossible : {error}"
            )));
        }
    }

    /// Applique thème, opacité et copie à la sélection.
    fn apply(&self) {
        let settings = self.settings.borrow();
        let style_manager = adw::StyleManager::default();
        style_manager.set_color_scheme(match settings.theme {
            Theme::System => adw::ColorScheme::Default,
            Theme::Light => adw::ColorScheme::ForceLight,
            Theme::Dark => adw::ColorScheme::ForceDark,
        });
        if settings.opacity_percent < MAX_OPACITY_PERCENT {
            self.window.add_css_class("leeloo-translucent");
        } else {
            self.window.remove_css_class("leeloo-translucent");
        }
        terminal::set_copy_on_select(settings.copy_on_select);
        terminal::set_confirm_close(settings.confirm_close);
        terminal::set_notify_on_finish(settings.notify_on_finish);
        terminal::set_font_scale_percent(settings.font_scale_percent);
        terminal::set_appearance(terminal::Appearance {
            dark: style_manager.is_dark(),
            opacity_percent: settings.opacity_percent,
            palette: palette::by_id(&settings.palette),
        });
    }
}

/// Délai avant d'écrire la largeur sur disque, pendant un glissement.
const WIDTH_SAVE_DELAY: std::time::Duration = std::time::Duration::from_millis(400);

/// Volet des sessions : visibilité (bouton, F9, menu) et largeur réglable à la
/// souris, toutes deux restaurées au lancement.
fn install_sidebar(
    application: &adw::Application,
    window: &adw::ApplicationWindow,
    preferences: &Rc<Preferences>,
    sidebar: &SidebarParts,
) {
    use gtk::gio;

    let (visible, width) = {
        let settings = preferences.settings.borrow();
        (settings.sidebar_visible, settings.sidebar_width)
    };
    sidebar.widget.set_size_request(MIN_SIDEBAR_WIDTH, -1);
    sidebar.paned.set_start_child(Some(sidebar.widget));
    sidebar.paned.set_position(width);

    // Le bouton et l'action pilotent la visibilité du volet ; GtkPaned masque
    // alors sa poignée et donne toute la place au terminal.
    sidebar.toggle.set_active(visible);
    sidebar
        .toggle
        .bind_property("active", sidebar.widget, "visible")
        .bidirectional()
        .sync_create()
        .build();
    window.add_action(&gio::PropertyAction::new(
        "toggle-sidebar",
        sidebar.widget,
        "visible",
    ));
    application.set_accels_for_action("win.toggle-sidebar", &["F9"]);
    sidebar.widget.connect_visible_notify({
        let preferences = preferences.clone();
        move |widget| {
            preferences.settings.borrow_mut().sidebar_visible = widget.is_visible();
            preferences.persist();
        }
    });

    // Largeur : bornée, mémorisée, écrite sur disque une fois le glissement fini.
    let generation = Rc::new(std::cell::Cell::new(0_u64));
    sidebar.paned.connect_position_notify({
        let preferences = preferences.clone();
        let widget = sidebar.widget.clone();
        move |paned| {
            if !widget.is_visible() {
                return;
            }
            let position = paned.position();
            let clamped = clamp_sidebar_width(position);
            if clamped != position {
                paned.set_position(clamped);
                return;
            }
            if preferences.settings.borrow().sidebar_width == clamped {
                return;
            }
            preferences.settings.borrow_mut().sidebar_width = clamped;
            let current = generation.get() + 1;
            generation.set(current);
            let preferences = preferences.clone();
            let generation = generation.clone();
            gtk::glib::timeout_add_local_once(WIDTH_SAVE_DELAY, move || {
                if generation.get() == current {
                    preferences.persist();
                }
            });
        }
    });
}

/// Raccourcis d'onglets. Ctrl+T / Ctrl+W sont volontairement évités : ils
/// appartiennent à readline et à vim. On suit la convention des terminaux
/// GNOME : Ctrl+Maj+T / Ctrl+Maj+W, Ctrl+PageUp/PageDown, Ctrl+Tab, Alt+1…9.
fn install_tab_shortcuts(
    application: &adw::Application,
    window: &adw::ApplicationWindow,
    tab_view: &adw::TabView,
) {
    use gtk::gio;

    let close_tab = gio::SimpleAction::new("close-tab", None);
    close_tab.connect_activate({
        let tab_view = tab_view.clone();
        move |_, _| {
            if let Some(page) = tab_view.selected_page() {
                tab_view.close_page(&page);
            }
        }
    });
    window.add_action(&close_tab);

    let next_tab = gio::SimpleAction::new("next-tab", None);
    next_tab.connect_activate({
        let tab_view = tab_view.clone();
        move |_, _| {
            // Fait le tour : après le dernier onglet, retour au premier.
            if !tab_view.select_next_page() {
                tab_view.set_selected_page(&tab_view.nth_page(0));
            }
        }
    });
    window.add_action(&next_tab);

    let previous_tab = gio::SimpleAction::new("previous-tab", None);
    previous_tab.connect_activate({
        let tab_view = tab_view.clone();
        move |_, _| {
            if !tab_view.select_previous_page() {
                tab_view.set_selected_page(&tab_view.nth_page(tab_view.n_pages() - 1));
            }
        }
    });
    window.add_action(&previous_tab);

    let select_tab = gio::SimpleAction::new("select-tab", Some(gtk::glib::VariantTy::INT32));
    select_tab.connect_activate({
        let tab_view = tab_view.clone();
        move |_, parameter| {
            let Some(requested) = parameter.and_then(|value| value.get::<i32>()) else {
                return;
            };
            if let Some(index) = nth_tab_index(requested, tab_view.n_pages()) {
                tab_view.set_selected_page(&tab_view.nth_page(index));
            }
        }
    });
    window.add_action(&select_tab);

    application.set_accels_for_action("win.new-local-tab", &["<Control><Shift>t"]);
    application.set_accels_for_action("win.close-tab", &["<Control><Shift>w"]);
    application.set_accels_for_action("win.next-tab", &["<Control>Page_Down", "<Control>Tab"]);
    application.set_accels_for_action(
        "win.previous-tab",
        &["<Control>Page_Up", "<Control><Shift>Tab"],
    );
    for digit in 1..=9 {
        application.set_accels_for_action(
            &format!("win.select-tab({digit})"),
            &[&format!("<Alt>{digit}")],
        );
    }
}

/// Actions `win.*`, menu principal (thème, opacité, copie à la sélection) et
/// préférences persistées.
fn install_window_actions(
    application: &adw::Application,
    window: &adw::ApplicationWindow,
    header: &adw::HeaderBar,
    toasts: &adw::ToastOverlay,
    tab_view: &adw::TabView,
    sidebar: &SidebarParts,
) -> Rc<Preferences> {
    use gtk::gio;

    let settings_path = Settings::default_path();
    let settings = match settings_path.as_deref().map(Settings::load) {
        Some(Ok(settings)) => settings,
        Some(Err(error)) => {
            toasts.add_toast(adw::Toast::new(&format!("Préférences ignorées : {error}")));
            Settings::default()
        }
        None => Settings::default(),
    };
    let preferences = Rc::new(Preferences {
        settings: RefCell::new(settings),
        path: settings_path,
        toasts: toasts.clone(),
        window: window.clone(),
    });

    style::install_css(APP_CSS);

    install_sidebar(application, window, &preferences, sidebar);

    let copy_on_select = gio::SimpleAction::new_stateful(
        "copy-on-select",
        None,
        &preferences.settings.borrow().copy_on_select.to_variant(),
    );
    copy_on_select.connect_activate({
        let preferences = preferences.clone();
        move |action, _| {
            let enabled = !preferences.settings.borrow().copy_on_select;
            preferences.settings.borrow_mut().copy_on_select = enabled;
            action.set_state(&enabled.to_variant());
            preferences.apply();
            preferences.persist();
        }
    });
    window.add_action(&copy_on_select);

    let restore_session = gio::SimpleAction::new_stateful(
        "restore-session",
        None,
        &preferences.settings.borrow().restore_session.to_variant(),
    );
    restore_session.connect_activate({
        let preferences = preferences.clone();
        move |action, _| {
            let enabled = !preferences.settings.borrow().restore_session;
            preferences.settings.borrow_mut().restore_session = enabled;
            action.set_state(&enabled.to_variant());
            preferences.persist();
        }
    });
    window.add_action(&restore_session);

    let confirm_close = gio::SimpleAction::new_stateful(
        "confirm-close",
        None,
        &preferences.settings.borrow().confirm_close.to_variant(),
    );
    confirm_close.connect_activate({
        let preferences = preferences.clone();
        move |action, _| {
            let enabled = !preferences.settings.borrow().confirm_close;
            preferences.settings.borrow_mut().confirm_close = enabled;
            action.set_state(&enabled.to_variant());
            preferences.apply();
            preferences.persist();
        }
    });
    window.add_action(&confirm_close);

    let notify_on_finish = gio::SimpleAction::new_stateful(
        "notify-on-finish",
        None,
        &preferences.settings.borrow().notify_on_finish.to_variant(),
    );
    notify_on_finish.connect_activate({
        let preferences = preferences.clone();
        move |action, _| {
            let enabled = !preferences.settings.borrow().notify_on_finish;
            preferences.settings.borrow_mut().notify_on_finish = enabled;
            action.set_state(&enabled.to_variant());
            preferences.apply();
            preferences.persist();
        }
    });
    window.add_action(&notify_on_finish);

    let theme_action = gio::SimpleAction::new_stateful(
        "theme",
        Some(gtk::glib::VariantTy::STRING),
        &preferences.settings.borrow().theme.as_str().to_variant(),
    );
    theme_action.connect_change_state({
        let preferences = preferences.clone();
        move |action, value| {
            let Some(theme) = value
                .and_then(|value| value.str())
                .and_then(Theme::from_name)
            else {
                return;
            };
            preferences.settings.borrow_mut().theme = theme;
            action.set_state(&theme.as_str().to_variant());
            preferences.apply();
            preferences.persist();
        }
    });
    window.add_action(&theme_action);

    let palette_action = gio::SimpleAction::new_stateful(
        "palette",
        Some(gtk::glib::VariantTy::STRING),
        &preferences.settings.borrow().palette.to_variant(),
    );
    palette_action.connect_change_state({
        let preferences = preferences.clone();
        move |action, value| {
            let Some(id) = value.and_then(|value| value.str()) else {
                return;
            };
            if id != DEFAULT_PALETTE_ID && palette::by_id(id).is_none() {
                return;
            }
            preferences.settings.borrow_mut().palette = id.to_owned();
            action.set_state(&id.to_variant());
            preferences.apply();
            preferences.persist();
        }
    });
    window.add_action(&palette_action);

    // Zoom de la police : -1 / +1 cran, 0 = taille normale.
    let zoom_action = gio::SimpleAction::new("zoom", Some(gtk::glib::VariantTy::INT32));
    zoom_action.connect_activate({
        let preferences = preferences.clone();
        move |_, parameter| {
            let Some(steps) = parameter.and_then(|value| value.get::<i32>()) else {
                return;
            };
            let current = preferences.settings.borrow().font_scale_percent;
            let next = if steps == 0 {
                DEFAULT_FONT_SCALE_PERCENT
            } else {
                step_font_scale_percent(current, steps)
            };
            if next != current {
                preferences.settings.borrow_mut().font_scale_percent = next;
                preferences.apply();
                preferences.persist();
            }
        }
    });
    window.add_action(&zoom_action);
    application.set_accels_for_action(
        "win.zoom(1)",
        &["<Control>plus", "<Control>equal", "<Control>KP_Add"],
    );
    application.set_accels_for_action("win.zoom(-1)", &["<Control>minus", "<Control>KP_Subtract"]);
    application.set_accels_for_action("win.zoom(0)", &["<Control>0", "<Control>KP_0"]);

    // Renommage : onglet sélectionné (menu du terminal, Ctrl+Maj+R) ou onglet
    // visé par le clic droit sur sa languette.
    let rename_selected = gio::SimpleAction::new("rename-tab", None);
    rename_selected.connect_activate({
        let tab_view = tab_view.clone();
        let window = window.clone();
        move |_, _| {
            if let Some(page) = tab_view.selected_page() {
                rename_dialog::prompt(&window, &page);
            }
        }
    });
    window.add_action(&rename_selected);
    application.set_accels_for_action("win.rename-tab", &["<Control><Shift>r"]);

    let clicked_page: Rc<RefCell<Option<adw::TabPage>>> = Rc::new(RefCell::new(None));
    tab_view.connect_setup_menu({
        let clicked_page = clicked_page.clone();
        move |_, page| *clicked_page.borrow_mut() = page.cloned()
    });
    let tab_actions = gio::SimpleActionGroup::new();
    let rename_clicked = gio::SimpleAction::new("rename", None);
    rename_clicked.connect_activate({
        let window = window.clone();
        move |_, _| {
            if let Some(page) = clicked_page.borrow().as_ref() {
                rename_dialog::prompt(&window, page);
            }
        }
    });
    tab_actions.add_action(&rename_clicked);
    tab_view.insert_action_group("tab", Some(&tab_actions));
    let tab_menu = gio::Menu::new();
    tab_menu.append(Some("Renommer l'onglet…"), Some("tab.rename"));
    tab_view.set_menu_model(Some(&tab_menu));

    let new_local_tab = gio::SimpleAction::new("new-local-tab", None);
    new_local_tab.connect_activate({
        let tab_view = tab_view.clone();
        move |_, _| terminal::open_local_tab(&tab_view)
    });
    window.add_action(&new_local_tab);

    // Menu principal.
    let menu = gio::Menu::new();
    let theme_section = gio::Menu::new();
    for (label, name) in [
        ("Système", Theme::System),
        ("Clair", Theme::Light),
        ("Sombre", Theme::Dark),
    ] {
        let item = gio::MenuItem::new(Some(label), None);
        item.set_action_and_target_value(Some("win.theme"), Some(&name.as_str().to_variant()));
        theme_section.append_item(&item);
    }
    menu.append_section(Some("Thème"), &theme_section);
    let palette_menu = gio::Menu::new();
    let default_item = gio::MenuItem::new(Some("Défaut (suit le thème)"), None);
    default_item
        .set_action_and_target_value(Some("win.palette"), Some(&DEFAULT_PALETTE_ID.to_variant()));
    palette_menu.append_item(&default_item);
    for palette in PALETTES {
        let item = gio::MenuItem::new(Some(palette.name), None);
        item.set_action_and_target_value(Some("win.palette"), Some(&palette.id.to_variant()));
        palette_menu.append_item(&item);
    }
    menu.append_submenu(Some("Palette du terminal"), &palette_menu);
    let opacity_section = gio::Menu::new();
    let opacity_item = gio::MenuItem::new(Some("Opacité"), None);
    opacity_item.set_attribute_value("custom", Some(&"opacity".to_variant()));
    opacity_section.append_item(&opacity_item);
    menu.append_section(Some("Opacité du fond"), &opacity_section);
    let zoom_section = gio::Menu::new();
    zoom_section.append(Some("Agrandir la police (Ctrl++)"), Some("win.zoom(1)"));
    zoom_section.append(Some("Réduire la police (Ctrl+-)"), Some("win.zoom(-1)"));
    zoom_section.append(Some("Taille normale (Ctrl+0)"), Some("win.zoom(0)"));
    menu.append_section(Some("Zoom de la police"), &zoom_section);
    let options_section = gio::Menu::new();
    options_section.append(Some("Copier à la sélection"), Some("win.copy-on-select"));
    options_section.append(
        Some("Restaurer les onglets au lancement"),
        Some("win.restore-session"),
    );
    options_section.append(Some("Confirmer avant de fermer"), Some("win.confirm-close"));
    options_section.append(
        Some("Notifier en fin de commande"),
        Some("win.notify-on-finish"),
    );
    options_section.append(
        Some("Panneau des sessions (F9)"),
        Some("win.toggle-sidebar"),
    );
    menu.append_section(None, &options_section);

    let popover = gtk::PopoverMenu::from_model(Some(&menu));
    let opacity_scale = gtk::Scale::with_range(
        gtk::Orientation::Horizontal,
        f64::from(MIN_OPACITY_PERCENT),
        f64::from(MAX_OPACITY_PERCENT),
        5.0,
    );
    opacity_scale.set_width_request(220);
    opacity_scale.set_value(f64::from(preferences.settings.borrow().opacity_percent));
    opacity_scale.set_draw_value(true);
    opacity_scale.set_value_pos(gtk::PositionType::Right);
    opacity_scale.set_margin_start(12);
    opacity_scale.set_margin_end(12);
    opacity_scale.connect_value_changed({
        let preferences = preferences.clone();
        move |scale| {
            // Valeur bornée 30..=100 : la conversion ne peut pas déborder.
            let percent = clamp_opacity_percent(scale.value().round() as u8);
            preferences.settings.borrow_mut().opacity_percent = percent;
            preferences.apply();
        }
    });
    popover.add_child(&opacity_scale, "opacity");
    // Écriture disque une seule fois, à la fermeture du menu.
    popover.connect_closed({
        let preferences = preferences.clone();
        move |_| preferences.persist()
    });

    let menu_button = gtk::MenuButton::new();
    menu_button.set_icon_name("open-menu-symbolic");
    menu_button.set_tooltip_text(Some("Apparence et préférences"));
    menu_button.set_popover(Some(&popover));
    header.pack_end(&menu_button);

    // Le thème « système » peut changer pendant l'exécution.
    adw::StyleManager::default().connect_dark_notify({
        let preferences = preferences.clone();
        move |_| preferences.apply()
    });
    preferences.apply();
    preferences
}
