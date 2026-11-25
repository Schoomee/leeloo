# Leeloo

[![CI](https://github.com/Schoomee/leeloo/actions/workflows/ci.yml/badge.svg)](https://github.com/Schoomee/leeloo/actions/workflows/ci.yml)

Client de terminal pour Ubuntu, inspiré de MobaXterm : un terminal local, des
sessions SSH organisées en dossiers et un sélecteur de VM Azure avec connexion
Entra ID (`az ssh vm`). Application native GTK4 / libadwaita / VTE, écrite en
Rust.

> Le nom vient du « multipass » du *Cinquième Élément* : un seul laissez-passer
> pour toutes les portes.

## Fonctions

- **Terminal local** : onglets, 10 000 lignes d'historique, recherche
  (Ctrl+Maj+F), copie à la sélection, menu contextuel (clic droit), liens
  `http(s)://` et `mailto:` (y compris OSC 8) ouverts par Ctrl+clic.
- **Sessions SSH** : création, édition, suppression, import des alias de
  `~/.ssh/config`. Les connexions utilisent le vrai `ssh` (agent, `ProxyJump`,
  `~/.ssh/config` fonctionnent tels quels).
- **Multi-exécution** : Ctrl+Maj+B (ou menu clic droit) diffuse ce que vous tapez
  ou collez dans le volet actif à tous les volets de l'onglet. Un cadre rouge
  entoure chaque volet et une pastille s'affiche sur l'onglet tant que c'est
  actif ; relancer Leeloo la désactive toujours.
- **Couleur par onglet** : chaque onglet a sa couleur (pastille sur l'onglet et
  bandeau en haut du terminal), la première libre parmi 8 ; une session avec
  couleur d'identification garde la sienne.
- **Notification de fin de commande** : quand une commande de plus de 10 s se
  termine dans un shell local et que vous ne regardez pas l'onglet (fenêtre
  inactive ou autre onglet), une notification de bureau s'affiche. La cloche du
  terminal (`printf '\a'`) notifie aussi, ce qui permet de signaler la fin d'une
  commande distante (`long_commande; printf '\a'`). Désactivable dans le menu
  (« Notifier en fin de commande »).
- **Confirmation de fermeture** : fermer un onglet, un volet ou la fenêtre demande
  confirmation quand une commande tourne au premier plan (`sleep`, `vim`,
  `make`…) ou qu'une session SSH/Azure est ouverte. Un shell au repos ou une
  session terminée se ferment sans question. Désactivable dans le menu
  principal (« Confirmer avant de fermer »).
- **Restauration de session** : à la fermeture, les onglets, leurs divisions, le
  titre des onglets renommés et les dossiers repliés sont enregistrés, puis
  rouverts au lancement (réglable dans le menu principal). Les sessions SSH et
  Azure se reconnectent à l'ouverture ; une session supprimée entre-temps est
  ignorée. Les terminaux locaux repartent dans le dossier personnel (le
  répertoire courant n'est pas conservé).
- **Recherche rapide** : le champ en haut du volet (Ctrl+Maj+K) filtre les
  sessions par mots (nom, hôte, utilisateur, VM, dossier) ; les dossiers sans
  résultat disparaissent, les autres se déroulent. Entrée lance la première.
- **Volets (splits)** : un onglet peut être divisé à volonté (menu clic droit ou
  raccourcis). Le nouveau volet relance la même commande : pour une session
  SSH ou Azure, cela ouvre une seconde connexion. Quand un volet se termine
  (`exit`), il se ferme et le voisin récupère la place ; le dernier ferme
  l'onglet.
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

Télécharger le `.deb` et `SHA256SUMS` depuis la page
[Releases](https://github.com/Schoomee/leeloo/releases), vérifier la somme de
contrôle, puis installer :

```bash
sha256sum -c SHA256SUMS --ignore-missing
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
| Ctrl+Maj+K                    | Rechercher une session                   |
| Ctrl+Maj+B                    | Diffuser la saisie aux volets de l'onglet |
| Ctrl+Maj+O / Ctrl+Maj+E       | Diviser le volet à droite / en bas       |
| Alt+Maj+flèches               | Passer au volet voisin                   |
| Ctrl++ / Ctrl+- / Ctrl+0      | Zoom de la police : agrandir / réduire / normal (aussi Ctrl+molette) |
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
| `settings.json` | thème, opacité, zoom de la police, palette, copie à la sélection, volet (largeur), confirmation de fermeture, notifications |
| `azure.json`    | favoris et connexions récentes du sélecteur Azure              |
| `workspace.json`| onglets, divisions et dossiers repliés de la dernière session  |

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
- **Liens** : seuls `http`, `https` et `mailto` s'ouvrent (jamais `file:`),
  et uniquement par Ctrl+clic.
- **Restauration** : `workspace.json` ne contient que des références (terminal
  local, identifiant de session, VM), jamais de ligne de commande : elle est
  reconstruite et validée au lancement. Le nombre d'onglets, de volets et la
  profondeur sont bornés à la lecture.
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

### CI et publication

- `.github/workflows/ci.yml` : à chaque push sur `main` et chaque pull request,
  format, lint (clippy), tests, puis construction du `.deb` (disponible comme
  artefact du workflow).
- `.github/workflows/release.yml` : publie une release GitHub avec le `.deb` et
  `SHA256SUMS` quand un tag `vX.Y.Z` est poussé. Le tag doit correspondre à la
  `version` de `Cargo.toml`.

```bash
git tag v0.1.0 && git push origin v0.1.0
```

Les actions GitHub utilisées sont officielles et épinglées par SHA.

### Organisation

- `src/lib.rs` : logique pure, sans GTK et testée (`session`, `store`,
  `settings`, `palette`, `pane_nav`, `process`, `workspace`, `azure`, `azure_history`, `ssh_config`,
  `environment`, `hyperlink`, `tab_title`, `tab_nav`, `private_file`).
- `src/main.rs`, `terminal.rs`, `sessions_panel.rs`, `azure_dialog.rs`,
  `rename_dialog.rs`, `style.rs` : interface GTK, volontairement fine.
- `assets/` : fichier `.desktop` et icône (identifiant `io.github.Schoomee.Leeloo`).

Les tests couvrent la logique pure. L'interface graphique n'a pas de test
automatisé : elle se vérifie à la main.

## Limites connues

- Pas de panneau SFTP, de tunnels SSH graphiques ni de multi-exécution.
- La taille des volets divisés et le répertoire courant des shells ne sont pas
  restaurés.
- La suppression d'un dossier supprime ses sessions ; elle peut être annulée
  pendant 10 s (bouton « Annuler » du message), pas ensuite.
- L'opacité s'applique au terminal entier, texte compris, et demande un
  compositeur actif.

## Licence

[MIT](LICENSE), © 2026 Schoomee. Les dépendances Rust sont toutes sous licences
permissives (MIT, Apache-2.0, Unlicense). GTK4, libadwaita et VTE sont sous
LGPL et liées dynamiquement.
