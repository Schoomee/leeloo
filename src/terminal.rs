//! Onglets terminal (VTE) : shell local et sessions distantes.

use adw::prelude::*;
use gtk::glib;
use leeloo::hyperlink;
use leeloo::palette::{self, Palette};
use leeloo::pane_nav::{Direction, PaneRect, neighbor};
use leeloo::process::{self, JobTracker, NOTIFY_MIN_DURATION};
use leeloo::workspace::{Layout, PaneSource, TabLayout};
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
    static NOTIFY_ON_FINISH: Cell<bool> = const { Cell::new(true) };
    static CONFIRM_CLOSE: Cell<bool> = const { Cell::new(true) };
    static COPY_ON_SELECT: Cell<bool> = const { Cell::new(true) };
    static APPEARANCE: Cell<Appearance> = const {
        Cell::new(Appearance { dark: true, opacity_percent: 100, palette: None })
    };
    static FONT_SCALE_PERCENT: Cell<u16> = const { Cell::new(100) };
    static TABS: RefCell<Vec<Rc<TabContext>>> = const { RefCell::new(Vec::new()) };
    static TERMINALS: RefCell<Vec<Entry>> = const { RefCell::new(Vec::new()) };
    static NEXT_SURFACE_ID: Cell<u64> = const { Cell::new(1) };
    static SURFACE_STYLE: RefCell<Option<gtk::CssProvider>> = const { RefCell::new(None) };
}

/// Active ou non la notification de fin de commande (tous les onglets).
pub fn set_notify_on_finish(enabled: bool) {
    NOTIFY_ON_FINISH.with(|flag| flag.set(enabled));
}

fn notify_on_finish() -> bool {
    NOTIFY_ON_FINISH.with(Cell::get)
}

/// Active ou non la confirmation de fermeture (onglet, volet, fenêtre).
pub fn set_confirm_close(enabled: bool) {
    CONFIRM_CLOSE.with(|flag| flag.set(enabled));
}

pub fn confirm_close_enabled() -> bool {
    CONFIRM_CLOSE.with(Cell::get)
}

/// Active ou non la copie automatique de la sélection (tous les onglets).
pub fn set_copy_on_select(enabled: bool) {
    COPY_ON_SELECT.with(|flag| flag.set(enabled));
}

fn copy_on_select() -> bool {
    COPY_ON_SELECT.with(Cell::get)
}

/// Zoom de la police (en %) de tous les terminaux ouverts et des suivants.
pub fn set_font_scale_percent(percent: u16) {
    FONT_SCALE_PERCENT.with(|current| current.set(percent));
    TERMINALS.with(|terminals| {
        for entry in terminals.borrow().iter() {
            if let Some(terminal) = entry.terminal.upgrade() {
                terminal.set_font_scale(f64::from(percent) / 100.0);
            }
        }
    });
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
        PaneSource::Local,
    );
}

/// Session distante : à la fin du processus, le code de sortie est affiché et
/// Entrée relance la commande.
pub fn open_session_tab(
    tab_view: &adw::TabView,
    title: &str,
    argv: Vec<String>,
    style: TabStyle,
    source: PaneSource,
) {
    open_tab(tab_view, title, argv, false, style, source);
}

/// Ce qu'il faut pour (re)lancer un volet : un volet issu d'une division
/// relance la même commande (nouvelle connexion pour une session SSH).
#[derive(Clone)]
struct PaneSpec {
    argv: Vec<String>,
    close_on_exit: bool,
    style: TabStyle,
    /// Référence enregistrée pour la restauration de session.
    source: PaneSource,
}

/// État partagé par les volets d'un onglet. Références faibles vers la page et
/// la racine : les terminaux gardent ce contexte, un lien fort ferait un cycle
/// qui empêcherait de libérer un volet fermé (et donc de terminer son shell).
struct TabContext {
    tab_view: adw::TabView,
    page: gtk::glib::WeakRef<adw::TabPage>,
    root: gtk::glib::WeakRef<gtk::Box>,
    /// Dernier titre posé automatiquement : un titre différent = onglet renommé.
    automatic_title: RefCell<String>,
    pane_count: Cell<usize>,
    /// Volets de l'onglet et leur origine (pour enregistrer la disposition).
    panes: RefCell<Vec<(gtk::glib::WeakRef<gtk::Box>, PaneSource)>>,
    /// Multi-exécution : la saisie du volet actif est envoyée à tous les volets
    /// de l'onglet. Jamais restauré au lancement.
    broadcast: Cell<bool>,
    /// Couleur d'identification de l'onglet (`#rrggbb` validé).
    color: String,
    /// État d'exécution de chaque volet (pour la confirmation de fermeture).
    runtimes: RefCell<Vec<(gtk::glib::WeakRef<vte4::Terminal>, Rc<PaneRuntime>)>>,
    broadcast_actions: RefCell<Vec<gtk::glib::WeakRef<gtk::gio::SimpleAction>>>,
}

fn open_tab(
    tab_view: &adw::TabView,
    title: &str,
    argv: Vec<String>,
    close_on_exit: bool,
    style: TabStyle,
    source: PaneSource,
) {
    let (ctx, root, page) = new_tab_context(tab_view, title, title, style.color.as_deref());
    let mut style = style;
    style.color = Some(ctx.color.clone());
    let spec = PaneSpec {
        argv,
        close_on_exit,
        style,
        source,
    };
    let (pane, terminal) = build_pane(&ctx, &spec);
    root.append(&pane);
    tab_view.set_selected_page(&page);
    spawn_pane(&ctx, &terminal, &spec.argv);
}

/// Lance la commande d'un volet en suivant son processus.
fn spawn_pane(ctx: &TabContext, terminal: &vte4::Terminal, argv: &[String]) {
    let runtime = ctx
        .runtimes
        .borrow()
        .iter()
        .find(|(known, _)| known.upgrade().as_ref() == Some(terminal))
        .map(|(_, runtime)| runtime.clone());
    if let Some(runtime) = runtime {
        spawn_argv(terminal, argv, &runtime);
    }
}

/// Ce que fait le processus d'un volet : sert à décider s'il faut demander
/// confirmation avant de le fermer.
struct PaneRuntime {
    pid: Cell<Option<i32>>,
    /// Session distante terminée (en attente de « Entrée pour se reconnecter »).
    ended: Cell<bool>,
    /// Session SSH/Azure : la connexion elle-même est un travail en cours.
    remote: bool,
}

impl PaneRuntime {
    fn is_live(&self) -> bool {
        if self.ended.get() {
            return false;
        }
        if self.remote {
            return true;
        }
        self.pid.get().is_some_and(process::is_busy)
    }
}

/// Nombre de volets de l'onglet qui ont un travail ou une connexion actifs.
fn live_panes_of(ctx: &TabContext) -> usize {
    let mut runtimes = ctx.runtimes.borrow_mut();
    runtimes.retain(|(terminal, _)| terminal.upgrade().is_some());
    runtimes
        .iter()
        .filter(|(_, runtime)| runtime.is_live())
        .count()
}

/// Volets actifs dans la page donnée.
pub fn live_panes_in_page(page: &adw::TabPage) -> usize {
    context_for_page(page).map_or(0, |ctx| live_panes_of(&ctx))
}

/// Volets actifs dans tous les onglets.
pub fn live_panes(tab_view: &adw::TabView) -> usize {
    (0..tab_view.n_pages())
        .map(|index| live_panes_in_page(&tab_view.nth_page(index)))
        .sum()
}

/// Dialogue « Annuler / <action> » ; `decide` reçoit `true` après accord,
/// `false` sinon (annulation, Échap).
pub fn confirm_dialog(
    parent: Option<&gtk::Window>,
    heading: &str,
    body: &str,
    confirm_label: &str,
    decide: impl FnOnce(bool) + 'static,
) {
    let dialog = adw::AlertDialog::new(Some(heading), Some(body));
    dialog.add_responses(&[("cancel", "Annuler"), ("confirm", confirm_label)]);
    dialog.set_response_appearance("confirm", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    dialog.choose(parent, gtk::gio::Cancellable::NONE, move |response| {
        decide(response == "confirm");
    });
}

/// Texte « N connexion(s) ou commande(s) en cours ».
pub fn live_summary(count: usize) -> String {
    format!("{count} connexion(s) ou commande(s) en cours seront interrompues.")
}

/// Crée la page vide d'un onglet et son contexte. `automatic_title` est le
/// dernier titre posé par l'application : un titre différent = onglet renommé.
fn new_tab_context(
    tab_view: &adw::TabView,
    title: &str,
    automatic_title: &str,
    preferred_color: Option<&str>,
) -> (Rc<TabContext>, gtk::Box, adw::TabPage) {
    // Couleur de la session si elle en a une, sinon la première couleur
    // encore libre parmi les onglets ouverts.
    let color = match preferred_color.filter(|hex| palette::parse_hex(hex).is_some()) {
        Some(hex) => hex.to_owned(),
        None => {
            let used: Vec<String> = TABS.with(|tabs| {
                tabs.borrow()
                    .iter()
                    .filter(|known| known.page.upgrade().is_some())
                    .map(|known| known.color.clone())
                    .collect()
            });
            palette::next_tab_color(&used).to_owned()
        }
    };
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.set_hexpand(true);
    root.set_vexpand(true);
    let page = tab_view.append(&root);
    page.set_title(title);
    page.set_icon(Some(&color_dot_icon(&color)));
    let ctx = Rc::new(TabContext {
        tab_view: tab_view.clone(),
        page: page.downgrade(),
        root: root.downgrade(),
        automatic_title: RefCell::new(automatic_title.to_owned()),
        pane_count: Cell::new(1),
        panes: RefCell::new(Vec::new()),
        broadcast: Cell::new(false),
        color,
        runtimes: RefCell::new(Vec::new()),
        broadcast_actions: RefCell::new(Vec::new()),
    });
    TABS.with(|tabs| {
        let mut tabs = tabs.borrow_mut();
        tabs.retain(|known| known.page.upgrade().is_some());
        tabs.push(ctx.clone());
    });
    (ctx, root, page)
}

/// Construit un volet complet (terminal, recherche, menu, apparence) sans le
/// lancer ni l'insérer : utilisé pour le premier volet comme pour les divisions.
fn build_pane(ctx: &Rc<TabContext>, spec: &PaneSpec) -> (gtk::Box, vte4::Terminal) {
    let terminal = vte4::Terminal::new();
    terminal.set_scrollback_lines(10_000);
    terminal.set_hexpand(true);
    terminal.set_vexpand(true);

    terminal.set_clear_background(false);
    terminal.set_font_scale(f64::from(FONT_SCALE_PERCENT.with(Cell::get)) / 100.0);
    // Le parent direct du terminal doit rester ce conteneur : l'opacité s'y applique.
    let surface = gtk::Box::new(gtk::Orientation::Vertical, 0);
    surface.set_hexpand(true);
    surface.set_vexpand(true);
    surface.add_css_class(SURFACE_CLASS);
    let (search_bar, search_entry) = build_search(&terminal);
    surface.append(&search_bar);
    surface.append(&terminal);
    ctx.panes
        .borrow_mut()
        .push((surface.downgrade(), spec.source.clone()));
    if ctx.broadcast.get() {
        surface.add_css_class(BROADCAST_CLASS);
    }
    install_broadcast_input(&terminal, ctx);

    // Ctrl + molette : zoom de la police (action de fenêtre `win.zoom`).
    let zoom_scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    zoom_scroll.connect_scroll(|controller, _, delta_y| {
        if !controller
            .current_event_state()
            .contains(gtk::gdk::ModifierType::CONTROL_MASK)
            || delta_y == 0.0
        {
            return gtk::glib::Propagation::Proceed;
        }
        let steps: i32 = if delta_y < 0.0 { 1 } else { -1 };
        if let Some(widget) = controller.widget() {
            let _ = widget.activate_action("win.zoom", Some(&steps.to_variant()));
        }
        gtk::glib::Propagation::Stop
    });
    terminal.add_controller(zoom_scroll);
    install_hyperlinks(&terminal);

    install_clipboard_and_menu(&terminal, ctx, &surface, spec, &search_bar, &search_entry);
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
            style: spec.style.clone(),
        });
    });
    refresh_surface_css();
    apply_colors(
        &terminal,
        effective(APPEARANCE.with(Cell::get), &spec.style),
    );
    surface.set_opacity(surface_opacity(APPEARANCE.with(Cell::get)));
    // VTE recalcule ses couleurs par défaut depuis le CSS quand le widget est
    // réalisé ou que le thème change : on réapplique après chaque étape.
    terminal.connect_realize({
        let style = spec.style.clone();
        move |terminal| apply_colors(terminal, effective(APPEARANCE.with(Cell::get), &style))
    });
    terminal.connect_map({
        let style = spec.style.clone();
        move |terminal| apply_colors(terminal, effective(APPEARANCE.with(Cell::get), &style))
    });

    let runtime = Rc::new(PaneRuntime {
        pid: Cell::new(None),
        ended: Cell::new(false),
        remote: !spec.close_on_exit,
    });
    ctx.runtimes
        .borrow_mut()
        .push((terminal.downgrade(), runtime.clone()));
    watch_for_notifications(ctx, &terminal, &runtime);
    // Référence faible : le terminal possède ses propres contrôleurs.
    let spawn = gtk::glib::clone!(
        #[weak]
        terminal,
        #[strong(rename_to = argv)]
        spec.argv,
        #[strong]
        runtime,
        move || spawn_argv(&terminal, &argv, &runtime)
    );

    // Le titre de l'onglet suit celui du volet qui a le focus, tant que
    // l'utilisateur n'a pas renommé l'onglet.
    terminal.connect_window_title_changed({
        let ctx = ctx.clone();
        move |terminal| sync_tab_title(&ctx, terminal)
    });
    let focus = gtk::EventControllerFocus::new();
    focus.connect_enter({
        let ctx = ctx.clone();
        move |controller| {
            if let Some(terminal) = controller.widget().and_downcast::<vte4::Terminal>() {
                sync_tab_title(&ctx, &terminal);
            }
        }
    });
    terminal.add_controller(focus);

    terminal.connect_child_exited({
        let ctx = ctx.clone();
        let surface = surface.downgrade();
        let runtime = runtime.clone();
        let close_on_exit = spec.close_on_exit;
        move |terminal, status| {
            if close_on_exit {
                if let Some(surface) = surface.upgrade() {
                    close_pane_later(&ctx, &surface);
                }
                return;
            }
            runtime.ended.set(true);
            let message =
                format!("\r\n\x1b[33m[Session terminée (code {status}) — Entrée pour se reconnecter]\x1b[0m\r\n");
            terminal.feed(message.as_bytes());
        }
    });

    if !spec.close_on_exit {
        let key_controller = gtk::EventControllerKey::new();
        key_controller.set_propagation_phase(gtk::PropagationPhase::Capture);
        key_controller.connect_key_pressed(move |_, key, _, _| {
            if runtime.ended.get() && matches!(key, gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter)
            {
                runtime.ended.set(false);
                spawn();
                return gtk::glib::Propagation::Stop;
            }
            gtk::glib::Propagation::Proceed
        });
        terminal.add_controller(key_controller);
    }

    (surface, terminal)
}

/// Notifications de bureau, quand l'utilisateur ne regarde pas l'onglet :
/// fin d'une commande longue d'un shell local (suivi du premier plan, une fois
/// par seconde) et cloche du terminal (`\a`), seul signal possible à distance.
fn watch_for_notifications(
    ctx: &Rc<TabContext>,
    terminal: &vte4::Terminal,
    runtime: &Rc<PaneRuntime>,
) {
    let user_is_watching = |ctx: &TabContext, terminal: &vte4::Terminal| {
        let window_active = terminal
            .root()
            .and_downcast::<gtk::Window>()
            .is_some_and(|window| window.is_active());
        let tab_selected = ctx.page.upgrade().is_some_and(|page| page.is_selected());
        (window_active, tab_selected)
    };

    if !runtime.remote {
        let started = std::time::Instant::now();
        let tracker = RefCell::new(JobTracker::default());
        let weak_terminal = terminal.downgrade();
        let (ctx, runtime) = (ctx.clone(), runtime.clone());
        gtk::glib::timeout_add_seconds_local(1, move || {
            let Some(terminal) = weak_terminal.upgrade() else {
                return gtk::glib::ControlFlow::Break;
            };
            let busy = runtime.pid.get().is_some_and(process::is_busy);
            let finished = tracker.borrow_mut().observe(busy, started.elapsed());
            if let Some(duration) = finished
                && notify_on_finish()
            {
                let (window_active, tab_selected) = user_is_watching(&ctx, &terminal);
                if process::should_notify(
                    duration,
                    NOTIFY_MIN_DURATION,
                    window_active,
                    tab_selected,
                ) {
                    send_notification(
                        &ctx,
                        "Commande terminée",
                        &format!(
                            "{} — {}",
                            tab_title(&ctx),
                            process::format_duration(duration)
                        ),
                    );
                }
            }
            gtk::glib::ControlFlow::Continue
        });
    }

    let last_bell: Cell<Option<std::time::Instant>> = Cell::new(None);
    terminal.connect_bell({
        let ctx = ctx.clone();
        move |terminal| {
            let recent = last_bell
                .get()
                .is_some_and(|at| at.elapsed() < std::time::Duration::from_secs(5));
            let (window_active, tab_selected) = user_is_watching(&ctx, terminal);
            if recent || !notify_on_finish() || (window_active && tab_selected) {
                return;
            }
            last_bell.set(Some(std::time::Instant::now()));
            send_notification(&ctx, "Signal du terminal", &tab_title(&ctx));
        }
    });
}

/// Titre de l'onglet, borné et sans caractère de contrôle (il vient du programme
/// qui tourne dans le terminal).
fn tab_title(ctx: &TabContext) -> String {
    ctx.page
        .upgrade()
        .map(|page| page.title().to_string())
        .unwrap_or_default()
        .chars()
        .filter(|character| !character.is_control())
        .take(80)
        .collect()
}

fn send_notification(ctx: &TabContext, heading: &str, body: &str) {
    let Some(application) = gtk::gio::Application::default() else {
        return;
    };
    let notification = gtk::gio::Notification::new(heading);
    notification.set_body(Some(body));
    notification.set_icon(&color_dot_icon(&ctx.color));
    application.send_notification(None, &notification);
}

/// Pastille de couleur de l'onglet. `color` est un `#rrggbb` validé : sans
/// risque dans le SVG.
fn color_dot_icon(color: &str) -> gtk::gio::BytesIcon {
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"16\" height=\"16\">\
         <circle cx=\"8\" cy=\"8\" r=\"6\" fill=\"{color}\"/></svg>"
    );
    gtk::gio::BytesIcon::new(&glib::Bytes::from_owned(svg.into_bytes()))
}

const BROADCAST_CLASS: &str = "leeloo-broadcast";

/// Multi-exécution : ce que l'utilisateur tape (ou colle) dans le volet qui a le
/// focus est aussi envoyé aux autres volets de l'onglet. `feed_child` n'émet pas
/// `commit`, donc la diffusion ne se répercute pas en boucle.
fn install_broadcast_input(terminal: &vte4::Terminal, ctx: &Rc<TabContext>) {
    terminal.connect_commit({
        let ctx = ctx.clone();
        move |terminal, text, _| {
            if !ctx.broadcast.get() || !terminal.is_focus() {
                return;
            }
            let others: Vec<vte4::Terminal> = ctx
                .panes
                .borrow()
                .iter()
                .filter_map(|(pane, _)| pane.upgrade())
                .filter_map(|pane| first_terminal(pane.upcast_ref()))
                .filter(|other| other != terminal)
                .collect();
            for other in others {
                other.feed_child(text.as_bytes());
            }
        }
    });
}

/// Active ou coupe la multi-exécution de l'onglet : cadre rouge sur chaque
/// volet, pastille sur l'onglet, case cochée dans tous les menus.
fn set_broadcast(ctx: &TabContext, enabled: bool) {
    ctx.broadcast.set(enabled);
    ctx.broadcast_actions.borrow_mut().retain(|weak| {
        let Some(action) = weak.upgrade() else {
            return false;
        };
        action.set_state(&enabled.to_variant());
        true
    });
    for (pane, _) in ctx.panes.borrow().iter() {
        if let Some(pane) = pane.upgrade() {
            if enabled {
                pane.add_css_class(BROADCAST_CLASS);
            } else {
                pane.remove_css_class(BROADCAST_CLASS);
            }
        }
    }
    if let Some(page) = ctx.page.upgrade() {
        if enabled {
            page.set_indicator_icon(Some(&gtk::gio::ThemedIcon::new(
                "network-transmit-symbolic",
            )));
            page.set_indicator_tooltip("Saisie diffusée à tous les volets");
        } else {
            page.set_indicator_icon(None::<&gtk::gio::Icon>);
            page.set_indicator_tooltip("");
        }
    }
}

/// Titre de l'onglet = titre du terminal, si l'onglet n'a pas été renommé et
/// que le terminal est le seul volet ou celui qui a le focus.
fn sync_tab_title(ctx: &TabContext, terminal: &vte4::Terminal) {
    if ctx.pane_count.get() > 1 && !terminal.is_focus() {
        return;
    }
    let (Some(page), Some(window_title)) = (ctx.page.upgrade(), terminal.window_title()) else {
        return;
    };
    let mut last = ctx.automatic_title.borrow_mut();
    if page.title() == *last {
        page.set_title(&window_title);
        *last = window_title.to_string();
    }
}

/// Remplace `old` par `new` à la même place dans son parent (`Paned` ou `Box`).
fn replace_in_parent(old: &gtk::Widget, new: &gtk::Widget) {
    let Some(parent) = old.parent() else {
        return;
    };
    if let Some(paned) = parent.downcast_ref::<gtk::Paned>() {
        if paned.start_child().as_ref() == Some(old) {
            paned.set_start_child(Some(new));
        } else {
            paned.set_end_child(Some(new));
        }
    } else if let Some(container) = parent.downcast_ref::<gtk::Box>() {
        container.insert_child_after(new, Some(old));
        container.remove(old);
    }
}

fn new_paned(orientation: gtk::Orientation) -> gtk::Paned {
    let paned = gtk::Paned::new(orientation);
    paned.set_resize_start_child(true);
    paned.set_resize_end_child(true);
    paned.set_shrink_start_child(false);
    paned.set_shrink_end_child(false);
    paned.set_hexpand(true);
    paned.set_vexpand(true);
    paned
}

/// Divise `pane` : le nouveau volet relance la même commande à sa droite
/// (`Horizontal`) ou en dessous (`Vertical`) et prend le focus.
fn split_pane(
    ctx: &Rc<TabContext>,
    pane: &gtk::Box,
    orientation: gtk::Orientation,
    spec: &PaneSpec,
) {
    if pane.parent().is_none() {
        return;
    }
    let (new_pane, new_terminal) = build_pane(ctx, spec);
    let paned = new_paned(orientation);
    replace_in_parent(pane.upcast_ref(), paned.upcast_ref());
    paned.set_start_child(Some(pane));
    paned.set_end_child(Some(&new_pane));
    ctx.pane_count.set(ctx.pane_count.get() + 1);
    spawn_pane(ctx, &new_terminal, &spec.argv);
    new_terminal.grab_focus();
}

/// Fermeture demandée par l'utilisateur : confirmation si le volet a un travail
/// actif. Le dernier volet ferme l'onglet, dont la fermeture demande déjà
/// confirmation (pas de double question).
fn request_close_pane(ctx: &Rc<TabContext>, pane: &gtk::Box) {
    let live = pane_runtime(ctx, pane).is_some_and(|runtime| runtime.is_live());
    if !live || ctx.pane_count.get() <= 1 || !confirm_close_enabled() {
        close_pane_later(ctx, pane);
        return;
    }
    let (ctx, target) = (ctx.clone(), pane.clone());
    confirm_dialog(
        pane.root().and_downcast::<gtk::Window>().as_ref(),
        "Fermer ce volet ?",
        &live_summary(1),
        "Fermer",
        move |accepted| {
            if accepted {
                close_pane_later(&ctx, &target);
            }
        },
    );
}

fn pane_runtime(ctx: &TabContext, pane: &gtk::Box) -> Option<Rc<PaneRuntime>> {
    let terminal = first_terminal(pane.upcast_ref())?;
    ctx.runtimes
        .borrow()
        .iter()
        .find(|(known, _)| known.upgrade().as_ref() == Some(&terminal))
        .map(|(_, runtime)| runtime.clone())
}

/// Fermeture différée : appelée depuis un signal du terminal qu'on détruit.
fn close_pane_later(ctx: &Rc<TabContext>, pane: &gtk::Box) {
    let ctx = ctx.clone();
    let pane = pane.clone();
    gtk::glib::idle_add_local_once(move || close_pane(&ctx, &pane));
}

/// Ferme un volet ; le voisin récupère la place et le focus. Le dernier volet
/// ferme l'onglet.
fn close_pane(ctx: &Rc<TabContext>, pane: &gtk::Box) {
    // Déjà détaché (onglet en cours de fermeture) : rien à faire.
    if pane.root().is_none() {
        return;
    }
    let Some(parent) = pane.parent() else {
        return;
    };
    let Some(paned) = parent.downcast_ref::<gtk::Paned>() else {
        if let Some(page) = ctx.page.upgrade() {
            ctx.tab_view.close_page(&page);
        }
        return;
    };
    let pane_widget: &gtk::Widget = pane.upcast_ref();
    let sibling = if paned.start_child().as_ref() == Some(pane_widget) {
        paned.end_child()
    } else {
        paned.start_child()
    };
    let Some(sibling) = sibling else {
        return;
    };
    paned.set_start_child(None::<&gtk::Widget>);
    paned.set_end_child(None::<&gtk::Widget>);
    replace_in_parent(paned.upcast_ref(), &sibling);
    ctx.pane_count.set(ctx.pane_count.get().saturating_sub(1));
    ctx.panes
        .borrow_mut()
        .retain(|(weak, _)| weak.upgrade().is_some_and(|known| &known != pane));
    if let Some(terminal) = first_terminal(&sibling) {
        terminal.grab_focus();
    }
}

fn first_terminal(widget: &gtk::Widget) -> Option<vte4::Terminal> {
    if let Some(terminal) = widget.downcast_ref::<vte4::Terminal>() {
        return Some(terminal.clone());
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Some(terminal) = first_terminal(&current) {
            return Some(terminal);
        }
        child = current.next_sibling();
    }
    None
}

/// Conteneurs de volet de l'arbre, dans l'ordre d'affichage.
fn collect_panes(widget: &gtk::Widget, panes: &mut Vec<gtk::Widget>) {
    if widget.has_css_class(SURFACE_CLASS) {
        panes.push(widget.clone());
        return;
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        collect_panes(&current, panes);
        child = current.next_sibling();
    }
}

/// Déplace le focus vers le volet voisin de `pane` dans `direction`.
fn focus_neighbor(ctx: &TabContext, pane: &gtk::Box, direction: Direction) {
    let Some(root) = ctx.root.upgrade() else {
        return;
    };
    let mut panes = Vec::new();
    collect_panes(root.upcast_ref(), &mut panes);
    let rects: Option<Vec<PaneRect>> = panes
        .iter()
        .map(|widget| {
            widget.compute_bounds(&root).map(|bounds| PaneRect {
                x: f64::from(bounds.x()),
                y: f64::from(bounds.y()),
                width: f64::from(bounds.width()),
                height: f64::from(bounds.height()),
            })
        })
        .collect();
    let pane_widget: &gtk::Widget = pane.upcast_ref();
    let (Some(rects), Some(current)) = (rects, panes.iter().position(|w| w == pane_widget)) else {
        return;
    };
    if let Some(target) = neighbor(&rects, current, direction)
        && let Some(terminal) = first_terminal(&panes[target])
    {
        terminal.grab_focus();
    }
}

/// Commande d'un volet à restaurer, reconstruite et validée par l'appelant.
pub struct Launch {
    pub title: String,
    pub argv: Vec<String>,
    pub close_on_exit: bool,
    pub style: TabStyle,
}

/// Lancement d'un shell local (même règle que `open_local_tab`).
pub fn local_launch() -> Launch {
    Launch {
        title: "Terminal local".to_owned(),
        argv: vec![std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_owned())],
        close_on_exit: true,
        style: TabStyle::default(),
    }
}

/// Disposition résolue : chaque volet a sa commande, ceux qu'on ne sait plus
/// lancer (session supprimée, VM invalide) ont disparu.
enum Resolved {
    Pane(Launch, PaneSource),
    Split {
        vertical: bool,
        first: Box<Resolved>,
        second: Box<Resolved>,
    },
}

fn resolve_layout(
    layout: &Layout,
    resolve: &dyn Fn(&PaneSource) -> Option<Launch>,
) -> Option<Resolved> {
    match layout {
        Layout::Pane { source } => {
            resolve(source).map(|launch| Resolved::Pane(launch, source.clone()))
        }
        Layout::Split {
            vertical,
            first,
            second,
        } => match (
            resolve_layout(first, resolve),
            resolve_layout(second, resolve),
        ) {
            (Some(first), Some(second)) => Some(Resolved::Split {
                vertical: *vertical,
                first: Box::new(first),
                second: Box::new(second),
            }),
            (only, None) | (None, only) => only,
        },
    }
}

fn first_title(resolved: &Resolved) -> &str {
    match resolved {
        Resolved::Pane(launch, _) => &launch.title,
        Resolved::Split { first, .. } => first_title(first),
    }
}

fn first_color(resolved: &Resolved) -> Option<&str> {
    match resolved {
        Resolved::Pane(launch, _) => launch.style.color.as_deref(),
        Resolved::Split { first, .. } => first_color(first),
    }
}

fn pane_total(resolved: &Resolved) -> usize {
    match resolved {
        Resolved::Pane(..) => 1,
        Resolved::Split { first, second, .. } => pane_total(first) + pane_total(second),
    }
}

fn build_resolved(
    ctx: &Rc<TabContext>,
    resolved: Resolved,
    started: &mut Vec<(vte4::Terminal, Vec<String>)>,
) -> gtk::Widget {
    match resolved {
        Resolved::Pane(launch, source) => {
            let mut style = launch.style;
            style.color.get_or_insert_with(|| ctx.color.clone());
            let spec = PaneSpec {
                argv: launch.argv,
                close_on_exit: launch.close_on_exit,
                style,
                source,
            };
            let (pane, terminal) = build_pane(ctx, &spec);
            started.push((terminal, spec.argv));
            pane.upcast()
        }
        Resolved::Split {
            vertical,
            first,
            second,
        } => {
            let first = build_resolved(ctx, *first, started);
            let second = build_resolved(ctx, *second, started);
            let paned = new_paned(if vertical {
                gtk::Orientation::Vertical
            } else {
                gtk::Orientation::Horizontal
            });
            paned.set_start_child(Some(&first));
            paned.set_end_child(Some(&second));
            paned.upcast()
        }
    }
}

/// Rouvre un onglet enregistré. Retourne `false` (sans rien créer) si plus
/// aucun de ses volets ne peut être relancé.
pub fn restore_tab(
    tab_view: &adw::TabView,
    tab: &TabLayout,
    resolve: &dyn Fn(&PaneSource) -> Option<Launch>,
) -> bool {
    let Some(resolved) = resolve_layout(&tab.layout, resolve) else {
        return false;
    };
    let automatic = first_title(&resolved).to_owned();
    let title = tab.title.clone().unwrap_or_else(|| automatic.clone());
    let (ctx, root, _page) = new_tab_context(tab_view, &title, &automatic, first_color(&resolved));
    ctx.pane_count.set(pane_total(&resolved));
    let mut started = Vec::new();
    let widget = build_resolved(&ctx, resolved, &mut started);
    root.append(&widget);
    for (terminal, argv) in &started {
        spawn_pane(&ctx, terminal, argv);
    }
    true
}

/// Disposition des onglets ouverts et indice de l'onglet sélectionné.
pub fn capture_workspace(tab_view: &adw::TabView) -> (Vec<TabLayout>, usize) {
    let selected_page = tab_view.selected_page();
    let mut tabs = Vec::new();
    let mut selected = 0;
    for index in 0..tab_view.n_pages() {
        let page = tab_view.nth_page(index);
        let Some(ctx) = context_for_page(&page) else {
            continue;
        };
        let Some(layout) = ctx
            .root
            .upgrade()
            .and_then(|root| root.first_child())
            .and_then(|child| capture_layout(&ctx, &child))
        else {
            continue;
        };
        if selected_page.as_ref() == Some(&page) {
            selected = tabs.len();
        }
        let renamed = page.title() != *ctx.automatic_title.borrow();
        tabs.push(TabLayout {
            title: renamed.then(|| page.title().to_string()),
            layout,
        });
    }
    (tabs, selected)
}

fn context_for_page(page: &adw::TabPage) -> Option<Rc<TabContext>> {
    TABS.with(|tabs| {
        tabs.borrow()
            .iter()
            .find(|ctx| ctx.page.upgrade().as_ref() == Some(page))
            .cloned()
    })
}

fn capture_layout(ctx: &TabContext, widget: &gtk::Widget) -> Option<Layout> {
    if let Some(paned) = widget.downcast_ref::<gtk::Paned>() {
        return Some(Layout::Split {
            vertical: paned.orientation() == gtk::Orientation::Vertical,
            first: Box::new(capture_layout(ctx, &paned.start_child()?)?),
            second: Box::new(capture_layout(ctx, &paned.end_child()?)?),
        });
    }
    let source = ctx
        .panes
        .borrow()
        .iter()
        .find(|(pane, _)| {
            pane.upgrade()
                .is_some_and(|pane| pane.upcast_ref::<gtk::Widget>() == widget)
        })
        .map(|(_, source)| source.clone())?;
    Some(Layout::Pane { source })
}

/// Donne le focus au premier terminal de l'onglet sélectionné.
pub fn focus_selected(tab_view: &adw::TabView) {
    if let Some(terminal) = tab_view.selected_page().and_then(|page| {
        context_for_page(&page)
            .and_then(|ctx| ctx.root.upgrade())
            .and_then(|root| first_terminal(root.upcast_ref()))
    }) {
        terminal.grab_focus();
    }
}

/// Lance `argv` (recherche dans PATH, aucun shell intermédiaire) avec
/// l'environnement courant.
fn spawn_argv(terminal: &vte4::Terminal, argv: &[String], runtime: &Rc<PaneRuntime>) {
    let argv_refs: Vec<&str> = argv.iter().map(String::as_str).collect();
    let environment = leeloo::environment::terminal_environment(std::env::vars());
    let environment_refs: Vec<&str> = environment.iter().map(String::as_str).collect();
    let command_name = argv.first().cloned().unwrap_or_default();
    let runtime = runtime.clone();

    terminal.spawn_async(
        vte4::PtyFlags::DEFAULT,
        None,
        &argv_refs,
        &environment_refs,
        gtk::glib::SpawnFlags::SEARCH_PATH,
        || {},
        -1,
        gtk::gio::Cancellable::NONE,
        move |result| match result {
            Ok(pid) => runtime.pid.set(Some(pid.0)),
            Err(error) => eprintln!("Échec du lancement de « {command_name} » : {error}"),
        },
    );
}

/// Copie à la sélection, raccourcis Ctrl+Maj+C/V et menu contextuel (clic droit).
/// Détecte les URL (texte brut et liens OSC 8) ; Ctrl+clic les ouvre.
fn install_hyperlinks(terminal: &vte4::Terminal) {
    // PCRE2_UTF | PCRE2_MULTILINE (ce dernier est exigé par VTE).
    const REGEX_FLAGS: u32 = 0x0008_0000 | 0x0000_0400;
    terminal.set_allow_hyperlink(true);
    if let Ok(regex) = vte4::Regex::for_match(hyperlink::URL_PATTERN, REGEX_FLAGS) {
        let tag = terminal.match_add_regex(&regex, 0);
        terminal.match_set_cursor_name(tag, "pointer");
    }

    let click = gtk::GestureClick::builder().button(1).build();
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    // Référence faible : le terminal possède ce contrôleur.
    click.connect_pressed(gtk::glib::clone!(
        #[weak]
        terminal,
        move |gesture, _, x, y| {
            if !gesture
                .current_event_state()
                .contains(gtk::gdk::ModifierType::CONTROL_MASK)
            {
                return;
            }
            let uri = terminal
                .check_hyperlink_at(x, y)
                .or_else(|| terminal.check_match_at(x, y).0);
            let Some(uri) = uri else {
                return;
            };
            // Ni journalisé ni affiché : une URL peut contenir un jeton.
            let Some(uri) = hyperlink::openable_uri(&uri) else {
                return;
            };
            gesture.set_state(gtk::EventSequenceState::Claimed);
            let window = terminal.root().and_downcast::<gtk::Window>();
            gtk::UriLauncher::new(uri).launch(window.as_ref(), gtk::gio::Cancellable::NONE, |_| {});
        }
    ));
    terminal.add_controller(click);
}

fn install_clipboard_and_menu(
    terminal: &vte4::Terminal,
    ctx: &Rc<TabContext>,
    surface: &gtk::Box,
    spec: &PaneSpec,
    search_bar: &gtk::SearchBar,
    search_entry: &gtk::SearchEntry,
) {
    use gtk::gio;
    use gtk::glib::clone;

    let actions = gio::SimpleActionGroup::new();
    let add_action = |name: &str, callback: Box<dyn Fn()>| {
        let action = gio::SimpleAction::new(name, None);
        action.connect_activate(move |_, _| callback());
        actions.add_action(&action);
        action
    };
    // Références faibles vers le terminal et le volet : leurs propres
    // contrôleurs et groupes d'actions portent ces fermetures.
    let copy = add_action(
        "copy",
        Box::new(clone!(
            #[weak]
            terminal,
            move || terminal.copy_clipboard_format(vte4::Format::Text)
        )),
    );
    add_action(
        "paste",
        Box::new(clone!(
            #[weak]
            terminal,
            move || terminal.paste_clipboard()
        )),
    );
    // Faibles aussi : les rappels de la recherche gardent le terminal.
    add_action(
        "find",
        Box::new(clone!(
            #[weak]
            search_bar,
            #[weak]
            search_entry,
            move || {
                search_bar.set_search_mode(true);
                search_entry.grab_focus();
            }
        )),
    );
    add_action(
        "select-all",
        Box::new(clone!(
            #[weak]
            terminal,
            move || terminal.select_all()
        )),
    );
    add_action(
        "close",
        Box::new(clone!(
            #[strong]
            ctx,
            #[weak]
            surface,
            move || request_close_pane(&ctx, &surface)
        )),
    );
    let broadcast =
        gio::SimpleAction::new_stateful("broadcast", None, &ctx.broadcast.get().to_variant());
    broadcast.connect_activate({
        let ctx = ctx.clone();
        move |_, _| set_broadcast(&ctx, !ctx.broadcast.get())
    });
    actions.add_action(&broadcast);
    ctx.broadcast_actions
        .borrow_mut()
        .push(broadcast.downgrade());
    for (name, orientation) in [
        ("split-right", gtk::Orientation::Horizontal),
        ("split-down", gtk::Orientation::Vertical),
    ] {
        add_action(
            name,
            Box::new(clone!(
                #[strong]
                ctx,
                #[weak]
                surface,
                #[strong]
                spec,
                move || split_pane(&ctx, &surface, orientation, &spec)
            )),
        );
    }
    for (name, direction) in [
        ("focus-left", Direction::Left),
        ("focus-right", Direction::Right),
        ("focus-up", Direction::Up),
        ("focus-down", Direction::Down),
    ] {
        add_action(
            name,
            Box::new(clone!(
                #[strong]
                ctx,
                #[weak]
                surface,
                move || focus_neighbor(&ctx, &surface, direction)
            )),
        );
    }
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
        ("<Control><Shift>o", "term.split-right"),
        ("<Control><Shift>e", "term.split-down"),
        ("<Control><Shift>b", "term.broadcast"),
        ("<Alt><Shift>Left", "term.focus-left"),
        ("<Alt><Shift>Right", "term.focus-right"),
        ("<Alt><Shift>Up", "term.focus-up"),
        ("<Alt><Shift>Down", "term.focus-down"),
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
    let split_section = gio::Menu::new();
    split_section.append(Some("Diviser à droite"), Some("term.split-right"));
    split_section.append(Some("Diviser en bas"), Some("term.split-down"));
    split_section.append(
        Some("Diffuser la saisie aux volets (Ctrl+Maj+B)"),
        Some("term.broadcast"),
    );
    split_section.append(Some("Fermer le volet"), Some("term.close"));
    menu.append_section(None, &split_section);
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
