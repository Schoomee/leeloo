//! CSS global de l'application.

/// Ajoute un fournisseur CSS à l'affichage courant et le retourne pour pouvoir
/// le recharger (`load_from_string`).
pub fn install_css(css: &str) -> gtk::CssProvider {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(css);
    #[allow(deprecated)] // API usuelle de libadwaita pour un style global.
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
    provider
}
