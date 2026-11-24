# Leeloo

Client de terminal pour Ubuntu, inspiré de MobaXterm : un terminal local, des
sessions SSH organisées en dossiers et un sélecteur de VM Azure avec connexion
Entra ID (`az ssh vm`). Application native GTK4 / libadwaita / VTE, écrite en
Rust.

> Le nom vient du « multipass » du *Cinquième Élément* : un seul laissez-passer
> pour toutes les portes.

## Fonctions

- **Terminal local** : onglets, 10 000 lignes d'historique, recherche
  (Ctrl+Maj+F), copie à la sélection, menu contextuel (clic droit).
- **Sessions SSH** : création, édition, suppression, import des alias de
  `~/.ssh/config`. Les connexions utilisent le vrai `ssh` (agent, `ProxyJump`,
  `~/.ssh/config` fonctionnent tels quels).
- **Dossiers imbriqués** (`Azure › Dev`) : création, renommage, déplacement,
  suppression, repli.
- **Personnalisation par session** : couleur d'identification (pastille dans la
  liste, bandeau en haut du terminal) et palette de terminal propre.
- **VM Azure (Entra ID)** : liste triée par resource group, filtre par mot ou
  par tag (`Application=Gitlab`), colonnes Application / Components /
  Middlewares, favoris, VM récentes, démarrage d'une VM arrêtée, lien vers le
  portail Azure, enregistrement comme session.
- **Apparence** : thème système / clair / sombre, 7 palettes de terminal
  (Tango, Solarized clair et sombre, Dracula, Nord, Gruvbox, One Dark), opacité
  du terminal de 30 à 100 %.
- **Interface** : volet des sessions redimensionnable et masquable (F9),
  renommage des onglets, raccourcis d'onglets.
- **Fin de session** : le code de sortie s'affiche dans l'onglet, Entrée
  relance la connexion.

## Installation

Cible : Ubuntu 24.04 (GTK 4.14, libadwaita 1.5, VTE 0.76).

```bash
sudo apt install ./leeloo_0.1.0-1_amd64.deb
```

Le paquet dépend de `openssh-client` et des bibliothèques GTK4 / libadwaita /
VTE. `azure-cli` est suggéré, il n'est nécessaire que pour les VM Azure.

### Prérequis pour les VM Azure

- Azure CLI installé et connecté : `az login`.
- Extensions : `az extension add --name ssh` et
  `az extension add --name resource-graph`.
- Côté VM : connexion Entra ID activée (extension *AADSSHLoginForLinux*) et un
  rôle Azure `Virtual Machine Administrator Login` ou
  `Virtual Machine User Login` sur la VM.

## Utilisation

Lancer **Leeloo** depuis le menu des applications, ou `leeloo` dans un
terminal.

### Raccourcis

| Raccourci                     | Action                                   |
| ----------------------------- | ---------------------------------------- |
| Ctrl+Maj+T                    | Nouveau terminal local                   |
| Ctrl+Maj+W                    | Fermer l'onglet                          |
| Ctrl+Maj+R                    | Renommer l'onglet                        |
| Ctrl+PageDown / Ctrl+Tab      | Onglet suivant                           |
| Ctrl+PageUp / Ctrl+Maj+Tab    | Onglet précédent                         |
| Alt+1 … Alt+9                 | Aller à l'onglet n°N                     |
| Ctrl+Maj+C / Ctrl+Maj+V       | Copier / coller                          |
| Ctrl+Maj+F                    | Rechercher dans le terminal              |
| F9                            | Afficher / masquer le volet des sessions |

Ctrl+T et Ctrl+W ne sont volontairement pas utilisés : ils appartiennent à
readline et à vim.

### Filtre du sélecteur Azure

Un seul champ : un mot cherche dans le nom, le groupe, la région, l'état et les
tags affichés ; `CLE=VALEUR` filtre sur un tag (valeur exacte, casse ignorée).
Les deux se combinent :

```text
Application=Gitlab Environment=dev
```

La liste vient d'Azure Resource Graph (environ 1 s), limitée à 1000 VM Linux.

## Configuration

Les fichiers sont dans `~/.config/leeloo/` (dossier en 0700, fichiers en 0600) :

| Fichier         | Contenu                                                        |
| --------------- | -------------------------------------------------------------- |
| `sessions.json` | sessions, dossiers, couleurs, palettes                         |
| `settings.json` | thème, opacité, palette, copie à la sélection, volet (largeur) |
| `azure.json`    | favoris et connexions récentes du sélecteur Azure              |

Un ancien dossier `~/.config/moba-linux` est repris automatiquement au premier
lancement, sans jamais écraser un dossier `leeloo` existant.

Un fichier illisible n'est jamais écrasé : un message s'affiche et la
sauvegarde est désactivée jusqu'à correction.

## Sécurité

- **Aucun secret stocké** : pas de champ mot de passe. L'authentification passe
  par ssh-agent, les clés (chemin du fichier uniquement) ou la session
  `az login`.
- **Pas d'interpréteur intermédiaire** : `ssh` et `az` sont lancés avec un
  vecteur d'arguments, jamais `sh -c`.
- **Valeurs validées** : hôtes, utilisateurs, noms de VM et de groupes ne
  peuvent pas commencer par `-` (injection d'option) ni contenir d'espaces. Les
  couleurs sont limitées au format `#rrggbb` (elles finissent dans du CSS).
- **Environnement des onglets** : `TERM=xterm-256color`, `COLORTERM=truecolor` ;
  les variables du terminal parent (Ghostty, Kitty…) sont retirées pour éviter
  les erreurs « cannot initialize terminal type » sur les machines distantes.

## Développement

Prérequis (Ubuntu 24.04) :

```bash
sudo apt install build-essential pkg-config \
  libgtk-4-dev libadwaita-1-dev libvte-2.91-gtk4-dev
```

Une chaîne Rust stable récente est nécessaire (par exemple
`mise use -g rust@stable`).

```bash
cargo run                                    # lancer l'application
cargo fmt --check                            # format
cargo clippy --all-targets -- -D warnings    # lint
cargo test                                   # tests
cargo install cargo-deb && cargo deb         # construire le paquet .deb
```

Le paquet est produit dans `target/debian/`.

### Organisation

- `src/lib.rs` : logique pure, sans GTK et testée (`session`, `store`,
  `settings`, `palette`, `azure`, `azure_history`, `ssh_config`, `environment`,
  `tab_title`, `tab_nav`, `private_file`).
- `src/main.rs`, `terminal.rs`, `sessions_panel.rs`, `azure_dialog.rs`,
  `rename_dialog.rs`, `style.rs` : interface GTK, volontairement fine.
- `assets/` : fichier `.desktop` et icône (identifiant `io.github.Schoomee.Leeloo`).

Les tests couvrent la logique pure. L'interface graphique n'a pas de test
automatisé : elle se vérifie à la main.

## Limites connues

- Pas de panneau SFTP, de tunnels SSH graphiques ni de multi-exécution.
- Le renommage d'un onglet n'est pas conservé à sa fermeture.
- L'état replié des dossiers n'est pas conservé au redémarrage.
- La suppression d'un dossier supprime ses sessions, sans corbeille.
- L'opacité s'applique au terminal entier, texte compris, et demande un
  compositeur actif.

## Licence

[MIT](LICENSE), © 2026 Schoomee. Les dépendances Rust sont toutes sous licences
permissives (MIT, Apache-2.0, Unlicense). GTK4, libadwaita et VTE sont sous
LGPL et liées dynamiquement.
