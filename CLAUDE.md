# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Leeloo : client de terminal pour Ubuntu 24.04 (GTK4 / libadwaita / VTE, Rust édition 2024) : terminal local, sessions SSH en dossiers, sélecteur de VM Azure (Entra ID via `az ssh vm`). Code, commentaires et interface en français.

## Commandes

Prérequis : `sudo apt install build-essential pkg-config libgtk-4-dev libadwaita-1-dev libvte-2.91-gtk4-dev`

```bash
cargo run                                    # lancer l'application
cargo fmt --check                            # format (CI)
cargo clippy --all-targets -- -D warnings    # lint (CI)
cargo test                                   # tests (CI)
cargo test store::                           # un module ; ou `cargo test <nom_du_test>`
cargo deb                                    # paquet .deb dans target/debian/ (cargo install cargo-deb)
```

Lancer fmt, clippy et test avant tout commit : ce sont les contrôles de `.github/workflows/ci.yml`.

Release : pousser un tag `vX.Y.Z` qui doit être égal à la `version` de `Cargo.toml` (`release.yml` publie le `.deb` et `SHA256SUMS`). Les actions GitHub sont épinglées par SHA.

## Architecture

- **`src/lib.rs`** : logique pure, sans GTK, seule partie testée (`session`, `store`, `settings`, `palette`, `azure`, `azure_history`, `ssh_config`, `environment`, `hyperlink`, `pane_nav`, `process`, `workspace`, `tab_title`, `tab_nav`, `private_file`). Toute nouvelle logique va ici, avec ses tests.
- **Binaire GTK** (`main.rs`, `terminal.rs`, `sessions_panel.rs`, `azure_dialog.rs`, `rename_dialog.rs`, `style.rs`) : interface volontairement fine, sans test automatisé ; elle s'appelle depuis la lib et se vérifie à la main.
- **Persistance** : `~/.config/leeloo/{sessions,settings,azure,workspace}.json`, toujours via `private_file` (écriture atomique, dossier 0700, fichiers 0600). Un fichier illisible n'est jamais écrasé : la sauvegarde est désactivée jusqu'à correction. Migration automatique de l'ancien `~/.config/moba-linux` sans écraser un dossier `leeloo`.
- **Lancement de `ssh` / `az`** : `session.rs` construit des vecteurs d'arguments, jamais `sh -c`. Hôtes, utilisateurs, noms de VM et de groupes sont validés (pas de `-` initial, pas d'espace) ; les couleurs sont limitées à `#rrggbb` (elles finissent dans du CSS). Le modèle de session n'a aucun champ secret.
- **`environment.rs`** : fixe `TERM`/`COLORTERM` et retire les variables du terminal parent (Ghostty, Kitty…) pour les processus des onglets.
- **Azure** : liste via Azure Resource Graph (≈1 s, limitée à 1000 VM Linux) et non `az vm list --show-details` (≈1 min). Favoris et récents dans `azure_history`.
- **Volets** : un onglet = un `TabContext` (page et racine en `WeakRef`) dont la racine contient un arbre de `gtk::Paned` ; chaque volet est un `gtk::Box` (barre de recherche + `vte4::Terminal`) créé par `build_pane`, y compris pour les divisions. Le parent direct du terminal doit rester ce conteneur (l'opacité s'y applique). Les fermetures attachées au terminal ou à ses contrôleurs doivent capturer le terminal en `#[weak]` (`glib::clone!`), sinon le volet fermé n'est jamais libéré et son shell continue de tourner.
- **Restauration de session** : `workspace.rs` (pur) décrit onglets et divisions par *références* (`PaneSource` : local, id de session, VM Azure), jamais par argv ; `terminal::restore_tab` / `capture_workspace` convertissent depuis et vers l'arbre GTK, `SessionsHandle::resolve_pane` reconstruit et valide la commande. Enregistré à `close-request`, relu dans `restore_or_open_first_tab`.
- **Confirmation de fermeture** : `PaneRuntime` (pid, `ended`, `remote`) suit chaque volet ; `process::foreground_job_running` (pur) lit `/proc/<pid>/stat` du shell local. Toute fermeture d'onglet passe par `close-page` de `AdwTabView` (main.rs), qui doit toujours finir par `close_page_finish` (y compris en cas de refus) ; la fenêtre confirme dans `close-request`.
- **Couleur et notifications par onglet** : `TabContext.color` (première couleur libre de `palette::SESSION_COLORS`, ou celle de la session) alimente la pastille (`BytesIcon` SVG) et le bandeau des volets. `watch_for_notifications` interroge `process::is_busy` chaque seconde (`JobTracker`, logique pure) pour les shells locaux ; les sessions distantes n'ont que la cloche du terminal.
