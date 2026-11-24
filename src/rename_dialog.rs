//! Dialogue de renommage d'un onglet.

use adw::prelude::*;
use leeloo::tab_title::{MAX_TAB_TITLE_CHARS, normalize_tab_title};

/// Demande un nouveau nom ; un nom vide est ignoré (l'onglet garde son titre).
pub fn prompt(parent: &impl IsA<gtk::Widget>, page: &adw::TabPage) {
    let entry = gtk::Entry::builder()
        .text(page.title().as_str())
        .max_length(MAX_TAB_TITLE_CHARS as i32)
        .activates_default(true)
        .build();
    entry.select_region(0, -1);

    let dialog = adw::AlertDialog::new(Some("Renommer l'onglet"), None);
    dialog.set_extra_child(Some(&entry));
    dialog.add_responses(&[("cancel", "Annuler"), ("rename", "Renommer")]);
    dialog.set_response_appearance("rename", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("rename"));
    dialog.set_close_response("cancel");

    let page = page.clone();
    dialog.choose(Some(parent), gtk::gio::Cancellable::NONE, {
        let entry = entry.clone();
        move |response| {
            if response == "rename"
                && let Some(title) = normalize_tab_title(&entry.text())
            {
                page.set_title(&title);
            }
        }
    });
    entry.grab_focus();
}
