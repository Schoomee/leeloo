//! Onglets terminal (VTE) : shell local et sessions distantes.

use adw::prelude::*;
use leeloo::palette::{self, Palette};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use vte4::prelude::*;

/// Apparence appliquée à tous les terminaux.
#[derive(Clone, Copy)]
pub struct Appearance {
    pub dark: bool,
    pub opacity_percent: u8,
    /// `None` : couleurs par défaut du thème.
    pub palette: Option<&'static Palette>,
}

/// Réglages propres à une session (couleur d'identification, palette).
#[derive(Clone, Default)]
pub struct TabStyle {
    /// `#rrggbb` validé par le modèle de session.
    pub color: Option<String>,
    pub palette: Option<&'static Palette>,
}

struct Entry {
    terminal: gtk::glib::WeakRef<vte4::Terminal>,
    surface_id: u64,
    style: TabStyle,
}

thread_local! {
    static COPY_ON_SELECT: Cell<bool> = const { Cell::new(true) };
    static APPEARANCE: Cell<Appearance> = const {
        Cell::new(Appearance { dark: true, opacity_percent: 100, palette: None })
    };
    static TERMINALS: RefCell<Vec<Entry>> = const { RefCell::new(Vec::new()) };
    static NEXT_SURFACE_ID: Cell<u64> = const { Cell::new(1) };
    static SURFACE_STYLE: RefCell<Option<gtk::CssProvider>> = const { RefCell::new(None) };
}

/// Active ou non la copie automatique de la sélection (tous les onglets).
pub fn set_copy_on_select(enabled: bool) {
    COPY_ON_SELECT.with(|flag| flag.set(enabled));
}

fn copy_on_select() -> bool {
    COPY_ON_SELECT.with(Cell::get)
}

const SURFACE_CLASS: &str = "leeloo-terminal-surface";

/// Apparence effective d'un onglet : la palette de la session prime.
fn effective(appearance: Appearance, style: &TabStyle) -> Appearance {
    Appearance {
        palette: style.palette.or(appearance.palette),
        ..appearance
    }
}

/// Change l'apparence de tous les terminaux ouverts et des suivants.
pub fn set_appearance(appearance: Appearance) {
    APPEARANCE.with(|current| current.set(appearance));
    TERMINALS.with(|terminals| {
        terminals
            .borrow_mut()
            .retain(|entry| match entry.terminal.upgrade() {
                Some(terminal) => {
                    apply_colors(&terminal, effective(appearance, &entry.style));
                    if let Some(surface) = terminal.parent() {
                        surface.set_opacity(surface_opacity(appearance));
                    }
                    true
                }
                None => false,
            });
    });
    refresh_surface_css();
}

/// Le conteneur de l'onglet peint un fond opaque ; la translucidité vient de
/// l'opacité de groupe du conteneur (`set_opacity`), qui s'applique à tout ce
/// qu'il contient quel que soit le rendu interne de VTE. Le texte est donc
/// lui aussi légèrement translucide. Une règle par onglet gère la palette et le
/// bandeau de couleur propres à la session.
fn refresh_surface_css() {
    let appearance = APPEARANCE.with(Cell::get);
    let background_rule = |palette: Option<&'static Palette>| {
        let colors = palette::resolve(palette, appearance.dark);
        let (red, green, blue) = palette::parse_hex(colors.background).unwrap_or((0, 0, 0));
        format!("background-color: rgb({red}, {green}, {blue});")
    };
    let mut css = format!(
        ".{SURFACE_CLASS} {{ {} }}\n",
        background_rule(appearance.palette)
    );
    TERMINALS.with(|terminals| {
        for entry in terminals.borrow().iter() {
            let mut rules = String::new();
            if entry.style.palette.is_some() {
                rules.push_str(&background_rule(entry.style.palette));
            }
            // Défense en profondeur : la couleur finit dans du CSS.
            if let Some(color) = entry
                .style
                .color
                .as_deref()
                .filter(|color| palette::parse_hex(color).is_some())
            {
                rules.push_str(&format!(" border-top: 4px solid {color};"));
            }
            if !rules.is_empty() {
                css.push_str(&format!(
                    ".{SURFACE_CLASS}.leeloo-surface-{} {{ {rules} }}\n",
                    entry.surface_id
                ));
            }
        }
    });
    SURFACE_STYLE.with(|style| {
        let mut style = style.borrow_mut();
        match style.as_ref() {
            Some(provider) => provider.load_from_string(&css),
            None => *style = Some(crate::style::install_css(&css)),
        }
    });
}

fn surface_opacity(appearance: Appearance) -> f64 {
    f64::from(appearance.opacity_percent) / 100.0
}

/// Couleurs du texte et palette ANSI ; le fond par défaut n'est pas peint par VTE.
fn apply_colors(terminal: &vte4::Terminal, appearance: Appearance) {
    let colors = palette::resolve(appearance.palette, appearance.dark);
    let parse = |hex: &str| gtk::gdk::RGBA::parse(hex).ok();
    let (Some(foreground), Some(background)) = (parse(colors.foreground), parse(colors.background))
    else {
        return;
    };
    let ansi: Vec<gtk::gdk::RGBA> = colors
        .ansi
        .map(|list| list.iter().filter_map(|hex| parse(hex)).collect())
        .unwrap_or_default();
    let ansi_refs: Vec<&gtk::gdk::RGBA> = ansi.iter().collect();
    // Une palette incomplète est ignorée : VTE exige 0, 8, 16, 232 ou 256 entrées.
    let ansi_refs: &[&gtk::gdk::RGBA] = if ansi_refs.len() == 16 {
        &ansi_refs
    } else {
        &[]
    };
    terminal.set_colors(Some(&foreground), Some(&background), ansi_refs);
}

/// Shell local : l'onglet se ferme quand le shell se termine.
pub fn open_local_tab(tab_view: &adw::TabView) {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_owned());
    open_tab(
        tab_view,
        "Terminal local",
        vec![shell],
        true,
        TabStyle::default(),
    );
}

/// Session distante : à la fin du processus, le code de sortie est affiché et
/// Entrée relance la commande.
pub fn open_session_tab(tab_view: &adw::TabView, title: &str, argv: Vec<String>, style: TabStyle) {
    open_tab(tab_view, title, argv, false, style);
}

fn open_tab(
    tab_view: &adw::TabView,
    title: &str,
    argv: Vec<String>,
    close_on_exit: bool,
    style: TabStyle,
) {
    let terminal = vte4::Terminal::new();
    terminal.set_scrollback_lines(10_000);
    terminal.set_hexpand(true);
    terminal.set_vexpand(true);

    terminal.set_clear_background(false);
    let surface = gtk::Box::new(gtk::Orientation::Vertical, 0);
    surface.add_css_class(SURFACE_CLASS);
    let (search_bar, search_entry) = build_search(&terminal);
    surface.append(&search_bar);
    surface.append(&terminal);

    let page = tab_view.append(&surface);
    page.set_title(title);
    tab_view.set_selected_page(&page);
    install_clipboard_and_menu(&terminal, tab_view, &page, &search_bar, &search_entry);
    let surface_id = NEXT_SURFACE_ID.with(|next| {
        let id = next.get();
        next.set(id + 1);
        id
    });
    surface.add_css_class(&format!("leeloo-surface-{surface_id}"));
    TERMINALS.with(|terminals| {
        terminals.borrow_mut().push(Entry {
            terminal: terminal.downgrade(),
            surface_id,
            style: style.clone(),
        });
    });
    refresh_surface_css();
    apply_colors(&terminal, effective(APPEARANCE.with(Cell::get), &style));
    surface.set_opacity(surface_opacity(APPEARANCE.with(Cell::get)));
    // VTE recalcule ses couleurs par défaut depuis le CSS quand le widget est
    // réalisé ou que le thème change : on réapplique après chaque étape.
    terminal.connect_realize({
        let style = style.clone();
        move |terminal| apply_colors(terminal, effective(APPEARANCE.with(Cell::get), &style))
    });
    terminal.connect_map({
        let style = style.clone();
        move |terminal| apply_colors(terminal, effective(APPEARANCE.with(Cell::get), &style))
    });

    let ended = Rc::new(Cell::new(false));
    let spawn = {
        let terminal = terminal.clone();
        let argv = argv.clone();
        move || spawn_argv(&terminal, &argv)
    };

    // Le titre suit celui du terminal tant que l'utilisateur n'a pas renommé
    // l'onglet : un titre différent du dernier titre automatique = renommé.
    let automatic_title = Rc::new(RefCell::new(title.to_owned()));
    terminal.connect_window_title_changed({
        let page = page.clone();
        move |terminal| {
            let Some(window_title) = terminal.window_title() else {
                return;
            };
            let mut last = automatic_title.borrow_mut();
            if page.title() == *last {
                page.set_title(&window_title);
                *last = window_title.to_string();
            }
        }
    });

    terminal.connect_child_exited({
        let tab_view = tab_view.clone();
        let page = page.clone();
        let ended = ended.clone();
        move |terminal, status| {
            if close_on_exit {
                tab_view.close_page(&page);
                return;
            }
            ended.set(true);
            let message =
                format!("\r\n\x1b[33m[Session terminée (code {status}) — Entrée pour se reconnecter]\x1b[0m\r\n");
            terminal.feed(message.as_bytes());
        }
    });

    if !close_on_exit {
        let key_controller = gtk::EventControllerKey::new();
        key_controller.set_propagation_phase(gtk::PropagationPhase::Capture);
        let spawn = spawn.clone();
        key_controller.connect_key_pressed(move |_, key, _, _| {
            if ended.get() && matches!(key, gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter) {
                ended.set(false);
                spawn();
                return gtk::glib::Propagation::Stop;
            }
            gtk::glib::Propagation::Proceed
        });
        terminal.add_controller(key_controller);
    }

    spawn();
}

/// Lance `argv` (recherche dans PATH, aucun shell intermédiaire) avec
/// l'environnement courant.
fn spawn_argv(terminal: &vte4::Terminal, argv: &[String]) {
    let argv_refs: Vec<&str> = argv.iter().map(String::as_str).collect();
    let environment = leeloo::environment::terminal_environment(std::env::vars());
    let environment_refs: Vec<&str> = environment.iter().map(String::as_str).collect();
    let command_name = argv.first().cloned().unwrap_or_default();

    terminal.spawn_async(
        vte4::PtyFlags::DEFAULT,
        None,
        &argv_refs,
        &environment_refs,
        gtk::glib::SpawnFlags::SEARCH_PATH,
        || {},
        -1,
        gtk::gio::Cancellable::NONE,
        move |result| {
            if let Err(error) = result {
                eprintln!("Échec du lancement de « {command_name} » : {error}");
            }
        },
    );
}

/// Copie à la sélection, raccourcis Ctrl+Maj+C/V et menu contextuel (clic droit).
fn install_clipboard_and_menu(
    terminal: &vte4::Terminal,
    tab_view: &adw::TabView,
    page: &adw::TabPage,
    search_bar: &gtk::SearchBar,
    search_entry: &gtk::SearchEntry,
) {
    use gtk::gio;

    let actions = gio::SimpleActionGroup::new();
    let add_action = |name: &str, callback: Box<dyn Fn()>| {
        let action = gio::SimpleAction::new(name, None);
        action.connect_activate(move |_, _| callback());
        actions.add_action(&action);
        action
    };
    let copy = add_action("copy", {
        let terminal = terminal.clone();
        Box::new(move || terminal.copy_clipboard_format(vte4::Format::Text))
    });
    add_action("paste", {
        let terminal = terminal.clone();
        Box::new(move || terminal.paste_clipboard())
    });
    add_action("find", {
        let search_bar = search_bar.clone();
        let search_entry = search_entry.clone();
        Box::new(move || {
            search_bar.set_search_mode(true);
            search_entry.grab_focus();
        })
    });
    add_action("select-all", {
        let terminal = terminal.clone();
        Box::new(move || terminal.select_all())
    });
    add_action("close", {
        let tab_view = tab_view.clone();
        let page = page.clone();
        Box::new(move || tab_view.close_page(&page))
    });
    copy.set_enabled(false);
    terminal.insert_action_group("term", Some(&actions));

    terminal.connect_selection_changed(move |terminal| {
        let has_selection = terminal.has_selection();
        copy.set_enabled(has_selection);
        if has_selection && copy_on_select() {
            terminal.copy_clipboard_format(vte4::Format::Text);
        }
    });

    let shortcuts = gtk::ShortcutController::new();
    for (trigger, action) in [
        ("<Control><Shift>c", "term.copy"),
        ("<Control><Shift>v", "term.paste"),
        ("<Control><Shift>f", "term.find"),
    ] {
        if let Some(trigger) = gtk::ShortcutTrigger::parse_string(trigger) {
            shortcuts.add_shortcut(gtk::Shortcut::new(
                Some(trigger),
                Some(gtk::NamedAction::new(action)),
            ));
        }
    }
    terminal.add_controller(shortcuts);

    let menu = gio::Menu::new();
    let clipboard_section = gio::Menu::new();
    clipboard_section.append(Some("Copier"), Some("term.copy"));
    clipboard_section.append(Some("Coller"), Some("term.paste"));
    clipboard_section.append(Some("Tout sélectionner"), Some("term.select-all"));
    clipboard_section.append(Some("Rechercher…"), Some("term.find"));
    menu.append_section(None, &clipboard_section);
    let options_section = gio::Menu::new();
    options_section.append(Some("Copier à la sélection"), Some("win.copy-on-select"));
    menu.append_section(None, &options_section);
    let tabs_section = gio::Menu::new();
    tabs_section.append(Some("Renommer l'onglet…"), Some("win.rename-tab"));
    tabs_section.append(Some("Nouveau terminal local"), Some("win.new-local-tab"));
    tabs_section.append(Some("Fermer l'onglet"), Some("win.close-tab"));
    menu.append_section(None, &tabs_section);

    let popover = gtk::PopoverMenu::from_model(Some(&menu));
    popover.set_parent(terminal);
    popover.set_has_arrow(false);
    popover.set_halign(gtk::Align::Start);
    terminal.connect_destroy({
        let popover = popover.clone();
        move |_| popover.unparent()
    });

    let right_click = gtk::GestureClick::builder().button(3).build();
    right_click.connect_pressed(move |_, _, x, y| {
        popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover.popup();
    });
    terminal.add_controller(right_click);
}

/// Options PCRE2 de la recherche : sans distinction de casse, UTF-8, multi-lignes.
const SEARCH_REGEX_FLAGS: u32 = 0x0008 | 0x0400 | 0x0008_0000;

/// Barre de recherche (Ctrl+Maj+F) : Entrée = suivant, boutons précédent/suivant,
/// Échap = fermer et rendre le focus au terminal.
fn build_search(terminal: &vte4::Terminal) -> (gtk::SearchBar, gtk::SearchEntry) {
    let entry = gtk::SearchEntry::builder()
        .placeholder_text("Rechercher dans le terminal")
        .hexpand(true)
        .build();
    let previous = gtk::Button::from_icon_name("go-up-symbolic");
    previous.set_tooltip_text(Some("Occurrence précédente"));
    let next = gtk::Button::from_icon_name("go-down-symbolic");
    next.set_tooltip_text(Some("Occurrence suivante (Entrée)"));

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.append(&entry);
    row.append(&previous);
    row.append(&next);
    let bar = gtk::SearchBar::new();
    bar.set_child(Some(&row));
    bar.connect_entry(&entry);

    terminal.search_set_wrap_around(true);
    entry.connect_search_changed({
        let terminal = terminal.clone();
        move |entry| {
            set_search_text(&terminal, &entry.text());
            terminal.search_find_next();
        }
    });
    entry.connect_activate({
        let terminal = terminal.clone();
        move |_| {
            terminal.search_find_next();
        }
    });
    next.connect_clicked({
        let terminal = terminal.clone();
        move |_| {
            terminal.search_find_next();
        }
    });
    previous.connect_clicked({
        let terminal = terminal.clone();
        move |_| {
            terminal.search_find_previous();
        }
    });
    bar.connect_search_mode_enabled_notify({
        let terminal = terminal.clone();
        move |bar| {
            if !bar.is_search_mode() {
                terminal.search_set_regex(None, 0);
                terminal.grab_focus();
            }
        }
    });
    (bar, entry)
}

/// Le texte saisi est cherché tel quel (caractères spéciaux échappés).
fn set_search_text(terminal: &vte4::Terminal, text: &str) {
    if text.is_empty() {
        terminal.search_set_regex(None, 0);
        return;
    }
    let pattern = gtk::glib::Regex::escape_string(text);
    match vte4::Regex::for_search(pattern.as_str(), SEARCH_REGEX_FLAGS) {
        Ok(regex) => terminal.search_set_regex(Some(&regex), 0),
        Err(_) => terminal.search_set_regex(None, 0),
    }
}
