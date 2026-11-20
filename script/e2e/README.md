# Tests E2E du binaire LUNAR sous X11

Deux scripts pilotent le **vrai binaire release** `target/release/lunar` au
clavier et à la souris (XTest), sur un écran Xvfb qui leur est propre :

| Script | Rôle |
|---|---|
| `run-lunar-x11-e2e.sh` | Parcours opérateur complet. Verdict `LUNAR X11 E2E: PASS` ou `FAIL`. |
| `soak-lunar-memory.sh` | Mémoire sur la durée : cycles import → gravité → réinitialisation, VmRSS, tas vivant et threads relevés à chaque cycle. Verdict `LUNAR SOAK MÉMOIRE: PASS` ou `FAIL`. |

Fichiers associés :

- `lunar-x11-lib.sh` : fonctions communes (isolement, Xvfb, lancement,
  attente des journaux, sélecteurs de fichiers GTK, import, gravité,
  réinitialisation) et **la géométrie de la fenêtre, à un seul endroit**.
- `live-capture-steps.sh` : étapes de `--live-capture`, lues par le parcours.
- `lunar-x11-input.c` : pilote XTest, compilé par les scripts. Les commandes
  propres à gpui et aux dialogues rfd sont listées dans son en-tête :
  `paste-id` (collage servi par le pilote), `drop-id`
  (glisser-déposer XDND dont le pilote est la source), `type-id`,
  `wheel-id`, `move-id`, `double-click-id`, `pid-id`, `list-windows`, et
  `wait-dialog` qui reconnaît le sélecteur GTK par le `_NET_WM_PID`.

Ce ne sont pas des tests de l'interface avec un moteur simulé : le binaire
utilise le moteur `lunar-engine`, les sélecteurs de fichiers GTK (rfd), libpcap
et le rendu Vulkan, comme chez l'opérateur.

## Isolement

Les deux scripts appliquent les mêmes règles :

- **Réseau** : tout se déroule dans un espace de noms utilisateur + réseau
  (`unshare --user --map-root-user --net`) où seule `lo` existe. LUNAR n'a
  donc aucun accès au réseau, et la capture live y dispose de `CAP_NET_RAW`
  sans `sudo` ni `setcap`.
- **Écran** : un Xvfb dédié (`:121` pour le parcours, `:122` pour la mesure
  mémoire), lancé avec `-nolisten tcp -noreset -extension Composite`. Un
  écran déjà occupé (socket `/tmp/.X11-unix/XN` ou verrou `/tmp/.XN-lock`)
  est refusé : choisir `--display`. `DISPLAY`, `WAYLAND_DISPLAY`,
  `DBUS_SESSION_BUS_ADDRESS` et `XAUTHORITY` de l'utilisateur ne sont jamais
  hérités.
- **D-Bus** : un bus de session minimal (`dbus-run-session` avec une
  configuration sans répertoire de services) : ni portail, ni gvfs, ni at-spi
  ne sont activés. `GIO_USE_VFS=local`, `GSETTINGS_BACKEND=memory`,
  `NO_AT_BRIDGE=1`.
- **Données** : `HOME` et `XDG_*` dans le dossier d'artefacts ; les
  sélecteurs de fichiers s'ouvrent dans ce dossier.
- **Processus** : seuls les PID lancés par le script (Xvfb, LUNAR,
  échantillonneur réseau) sont arrêtés, par des pièges `EXIT`. Pas de
  `pkill` ni de recherche large de processus. Sans espace de noms PID, les
  PID restent ceux de l'hôte.
- **Rendu** : Vulkan logiciel lavapipe (`VK_ICD_FILENAMES`), `GDK_BACKEND=x11`,
  échelle 1. Quelques images par seconde, c'est normal.

## Prérequis Linux

En plus des dépendances de construction de LUNAR (voir le `README.md` racine) :

```bash
sudo apt-get install --yes \
  build-essential pkg-config libx11-dev libxtst-dev \
  xvfb x11-utils imagemagick dbus file unzip \
  iproute2 util-linux iputils-ping python3 \
  mesa-vulkan-drivers libgtk-3-0
```

- `cc` + en-têtes X11/XTest (`pkg-config x11 xtst`) : le pilote est compilé à
  chaque lancement.
- `Xvfb`, `xdpyinfo` ; ImageMagick (`import`, `convert`, `identify`) pour les
  captures et les contrôles d'image.
- `unshare`, `ip`, `ss`, `ping`, `dbus-run-session`, `python3`, `file`,
  `unzip`, `sha256sum`, `cmp`, `timeout`.
- lavapipe : `/usr/share/vulkan/icd.d/lvp_icd.x86_64.json`
  (`mesa-vulkan-drivers`).
- Espaces de noms utilisateur non privilégiés autorisés
  (`unshare --user --map-root-user --net true` doit réussir). Sur une
  distribution qui les restreint par AppArmor
  (`kernel.apparmor_restrict_unprivileged_userns=1`), il faut un profil pour
  `unshare` ou lever la restriction sur la machine de test.

## Parcours E2E

```bash
cargo build --release --locked -p lunar
cargo build --release --locked -p lunar-engine --example parity_csv
script/e2e/run-lunar-x11-e2e.sh --binary target/release/lunar \
  --artifacts /chemin/vers/artefacts-vides
# capture live sur lo en plus :
script/e2e/run-lunar-x11-e2e.sh --artifacts /chemin/vers/autres-artefacts --live-capture
```

Options : `--binary`, `--parity` (référence `parity_csv`), `--artifacts`
(absent ou vide), `--display :N` (défaut `:121`), `--live-capture`,
`--live-seconds N` (défaut 5), `--build` (compile les deux binaires avant).
Fixtures remplaçables par `LUNAR_E2E_PCAP`, `LUNAR_E2E_MATRIX`,
`LUNAR_E2E_LABELS`, `LUNAR_E2E_WINAUDIT` (défauts sous
`fixtures/`).

Toute vérification en échec arrête le script avec un code non nul, une
capture `echec.png` et `LUNAR X11 E2E: FAIL` en dernière ligne de
`summary.txt`.

### Ce qui est vérifié

Les actions passent d'abord par les raccourcis clavier et les actions de menu
(déterministes) ; seuls les boutons sans raccourci sont cliqués. Les preuves
sont les journaux de l'application, les fichiers exportés et une capture par
étape.

| Étape | Contrôle |
|---|---|
| Smoke test | `--version` puis `--lunar-smoke-test` : marqueurs `LUNAR_STARTUP_VALIDATION=BEGIN`, `LUNAR_SMOKE_FIRST_FRAME=OK`, `LUNAR_STARTUP_VALIDATION=OK`. |
| Lancement | Fenêtre `LUNAR` dont le `_NET_WM_PID` est celui du processus lancé, au moins 1240×720. |
| Configuration, filtre BPF | `Ctrl+,` et `Ctrl+F` ouvrent puis ferment les panneaux (`[Panneaux] config ouvert` / `fermé`, `[Configuration] config=…`). |
| Capture live (`--live-capture`) | Interface `lo` choisie dans la configuration et enregistrée (`[get_config_capture] app.config "lo"`), case ICMP du filtre puis « Appliquer » (`[set_filter] filter: icmp`), `Ctrl+P` (`Capture démarrée : true`), ping local et UDP sans écoute, journal de trames rempli (capture), `Ctrl+Shift+P` (`Capture arrêtée : false`), compteurs relus au retour dans l'analyse (`… reçus, … flux`), export CSV : `interface=lo`, un seul flux ICMP 127.0.0.1, au moins 2 paquets par ping, aucun UDP ; puis réinitialisation. |
| Import PCAP | Panneau d'import (`Ctrl+O`), sélecteur GTK, « Ouvrir » : `10 paquets lus = 10 intégrés + 0 tronqués + …`, `FIN traitement liste PCAP`, panneau refermé. |
| Parité CSV | `Ctrl+S` juste après l'import : export **identique à l'octet** à `parity_csv pcap` sur la même capture. |
| ForceAtlas2 | Bouton « Gravité » : `[Graphe] gravité coupée` puis `activée`. |
| Export PNG | PNG valide, largeur 2 × celle du graphe, hauteur 2 × celle du canevas (zone du graphe moins sa barre d'informations), plus de deux couleurs. |
| Labels, WinAudit, gestion | Import du fichier de labels (`import_label_file`), relevé WinAudit (`1 relevé(s) lu(s), 3 label(s) ajouté(s), 0 déjà présent(s), 1 conflit(s) à arbitrer`), arbitrage ouvert puis fermé par Échap, gestion des labels (`Ctrl+K`). |
| Exports CSV, labels, journaux | Matrice : `#LFMS version=…`, en-tête, label WinAudit sur les flux du poste ; labels : lignes WinAudit et « Passerelle » intact ; journaux : archive ZIP contenant des `.log`. |
| Reset + matrice | `Ctrl+Shift+R` confirmé (`Relevé réinitialisé`), matrice **déposée** sur le panneau d'import (glisser-déposer, aucun sélecteur ouvert) puis importée : `30 ligne(s) importée(s) -> 30 flux fusionné(s), 12 nœuds, 11 arêtes`. |
| Investigation | `Ctrl+I` (`[Investigation] snapshot chargé`), onglet Endpoints, label saisi au clavier sur la première ligne. |
| Synthèse | `Ctrl+S` dans la vue : HTML sans `<script>` ni attribut `on…=`, SHA-256 journalisé = SHA-256 du CSV écrit à côté = empreinte affichée, label présent, durée « inconnue », titre et générateur LUNAR. |
| Retour à l'analyse | `Ctrl+I` : `[Vue] analyse`, `[Barre d'état] Restauration au montage` (pas de remise à zéro), `[Graphe] Restauration au montage : 12 nœuds`. Puis export des labels contenant le label posé dans l'investigation. |
| Fin | Aucune panic dans les journaux, fermeture `Ctrl+Q` confirmée : code 0, `LUNAR fermé`. |

Contrôles de l'intégrité du binaire :

- **ressources embarquées** : chaque icône et image listée dans
  `crates/lunar/src/assets.rs`, et la police Liberation Sans, est présente à
  l'octet dans le binaire ;
- **icônes rendues** : la bande des boutons de la barre d'outils, sur la
  première image, compte au moins 20 couleurs (fond uni sinon) ;
- **aucune panic** (`panicked`, `fatal runtime error`) dans le journal et le
  smoke test ;
- **aucun réseau** : l'espace de noms ne contient que `lo` (vérifié au début,
  après les exports et à la fin) ; un échantillonneur relève chaque seconde
  les sockets de LUNAR (`/proc/PID/net/{tcp,udp,raw}{,6}` et `packet`,
  appariées par inode aux descripteurs du processus) et toute adresse hors
  boucle locale fait échouer le parcours. Sans `--live-capture`, LUNAR
  n'ouvre aucune socket ; avec, seule la socket de capture `packet` sur `lo`
  apparaît.

### Géométrie

Les positions cliquées sont dans le bloc « Géométrie de la fenêtre » de
`lunar-x11-lib.sh` : hauteurs des barres (menus 28, outils 40, journal de
trames 190, statut 22), largeurs des panneaux latéraux (configuration 400,
imports 480, filtre 520), ordonnées des boutons de panneau, comptées depuis le
haut, et abscisses depuis le bord droit de la fenêtre. Les boutons du bas des
panneaux qui défilent (« Sauvegarder », « Appliquer ») sont visés après un
défilement jusqu'au bout, depuis le bas de la zone. Une modification de mise
en page de ces vues se répercute là, et nulle part ailleurs.

### Sélecteurs de fichiers GTK

`Ctrl+L`, `Ctrl+A`, puis le chemin **collé** (`paste-id` : le pilote possède
la sélection CLIPBOARD et la sert lui-même), `Suppr` (retire une complétion
en ligne), `Entrée`. Une saisie touche par touche relance la complétion GTK à
chaque caractère et prend plus de dix secondes par chemin.

### Glisser-déposer

`drop-id` joue la source XDND (version 5) : le pointeur est amené au point
visé (gpui lit sa position), `XdndEnter` annonce `text/uri-list`,
`XdndPosition` déclenche la lecture de la liste d'URI `file://` que le pilote
sert, `XdndDrop` part après le `XdndStatus` de la fenêtre, et la commande
attend `XdndFinished`. gpui transmet le dépôt au panneau d'import ouvert.

### Preuves à conserver

Dans le dossier d'artefacts :

- `summary.txt` (verdict en dernière ligne), `checks.txt` (un `PASS` par
  contrôle, ou le `FAIL`) ;
- `runtime.log` (sortie de LUNAR), `app-logs/` (`lunar.log`,
  `DR_LUNAR_*.log` tels qu'écrits sur disque), `startup-smoke*.log`,
  `version.txt`, `xvfb.log` ;
- les captures `NN-*.png` de chaque étape, `dialogue-NN-avant.png` pour
  chaque sélecteur, `echec.png` en cas d'échec ;
- les exports : `matrice-pcap.csv` et sa référence
  `matrice-pcap-parity_csv.csv`, `graphe-export.png`, `matrice-export.csv`,
  `labels-export.csv`, `logs-export.zip` (+ `logs-export-contenu.txt`),
  `synthese-investigation.html` et le CSV écrit à côté,
  `labels-apres-investigation.csv`, et avec `--live-capture`
  `capture-live.csv`, `capture-live-controle.txt`, `live-ping.txt` ;
- le réseau : `reseau-*-interfaces.txt`, `reseau-*-ss.txt`,
  `reseau-releves.txt` (sockets relevées pendant la session),
  `ressources-embarquees.txt`.

## Mesure mémoire

```bash
script/e2e/soak-lunar-memory.sh --binary target/release/lunar \
  --artifacts /chemin/vers/soak-vide --cycles 20
```

Chaque cycle : la matrice (`20260703_NP_Matrice_1000.csv` par défaut, 232
nœuds et 931 arêtes, ou une capture avec `--input pcap`) est **déposée** sur
le panneau d'import (`--via drop`, défaut) puis importée, la gravité tourne
`--gravity-seconds` (15 s par défaut), le relevé est réinitialisé.
`memoire.csv` reçoit une ligne par phase (`import`, `gravite`,
`reinitialisation`) : `cycle,phase,secondes,vmrss_ko,vmhwm_ko,threads,tas_vivant_ko`.
VmRSS, VmHWM et threads viennent de `/proc/PID/status` ; le tas vivant
(octets alloués et non libérés, `mallinfo2`) de la ligne `[Mémoire]` que
LUNAR journalise après chaque réinitialisation.

Verdict, sur les relevés pris **après la réinitialisation** (état vide,
comparable d'un cycle à l'autre), les `--warmup` premiers cycles (5) écartés :

- croissance de VmRSS entre le premier et le dernier relevé ≤
  `--tolerance-mb` (48 Mo) ;
- pente de VmRSS (régression linéaire) ≤ `--max-slope-mb` (1,5 Mo par
  cycle) ;
- pente du tas vivant ≤ `--max-heap-slope-mb` (0,5 Mo par cycle) ;
- threads : au plus `--max-thread-growth` (2) de plus qu'au premier relevé.

`summary.txt` donne la courbe (par cycle : VmRSS après réinitialisation, tas
vivant, pic pendant la gravité, threads, barre proportionnelle), la
croissance, les pentes, le VmHWM et le verdict ; captures du premier et du
dernier cycle, `runtime.log`, `memoire.csv` et `verdict.txt` à côté.

### Ce que la mesure a appris

Mesures des 19 et 20/11/2026 (matrice de 1 000 flux, gravité 15 s, Xvfb + lavapipe) :

| Variante | VmRSS après chauffe | Tas vivant | Verdict |
|---|---|---|---|
| Avant correctif, sélecteur GTK à chaque cycle (20 cycles) | 229,3 → 282,0 Mo, +3,68 Mo/cycle | non relevé | FAIL |
| `malloc_trim` après réinitialisation, sélecteur GTK (20 cycles) | 209,7 → 241,6 Mo, +2,30 Mo/cycle | ≈ +1,6 Mo/cycle (relevé sur 8 cycles) | FAIL |
| `malloc_trim`, glisser-déposer (24 cycles, défaut) | 163,8 → 180,1 Mo, +0,91 Mo/cycle (+0,4 Mo/cycle sur les cycles 17 à 24) | 96,7 → 99,2 Mo, +0,18 Mo/cycle | PASS |

- **Arènes glibc.** Le moteur seul (import → réinitialisation en boucle) et
  le modèle du graphe avec ForceAtlas2, sur un fil, restent stables. Dans
  l'application, le travail passe d'un fil à l'autre (exécuteur de fond de
  gpui, fil ForceAtlas2, rendu) et chaque fil remplit sa propre arène : la
  mémoire libérée à la réinitialisation restait résidente, arène après
  arène. LUNAR appelle donc `malloc_trim(0)` après « Relevé réinitialisé »
  (`crates/lunar/src/app/memory.rs`) : 5 à 9 Mo rendus à chaque
  réinitialisation. Le reste de la pente vient des pages encore partagées
  avec des allocations vivantes dans les arènes nouvellement touchées
  (`/proc/PID/smaps` : seules les zones anonymes des arènes grossissent,
  le tas vivant ne bouge pas) ; il est borné par le nombre de fils (une
  arène par fil) et la pente baisse au fil des cycles.
- **Sélecteur GTK (rfd).** Ouvrir le sélecteur et le refermer, sans aucun
  import, ajoute environ 1,6 Mo au tas vivant à chaque fois, alors
  qu'ouvrir et fermer le panneau d'import seul n'ajoute rien. Cause
  probable : rfd 0.16 détruit le `GtkFileChooserNative`
  (`gtk_native_dialog_destroy`) sans jamais lâcher sa référence. C'est
  hors de LUNAR ; la mesure dépose donc le fichier pour suivre LUNAR seul,
  et `--via dialog` reste disponible pour suivre ce coût.
- **Reste du tas vivant** (+0,18 Mo/cycle, par paliers de 0,7 à 1 Mo) :
  caches de rendu de gpui et textures de lavapipe, sous la tolérance.

## Limites

- Sans gestionnaire de fenêtres : la fenêtre garde sa taille par défaut
  (1280×800), les positions mesurées valent pour elle (contrôle de taille
  minimale au lancement).
- Le glisser-déposer n'est essayé que sur le panneau d'import ouvert ; le
  menu contextuel du graphe, les infobulles et les projets (`.lunar`) ne
  sont pas parcourus.
- La mesure mémoire tourne sous lavapipe : le rendu Vulkan d'un vrai GPU
  (mémoire du pilote) n'y est pas représenté.
- La capture live n'est essayée que sur `lo` dans l'espace de noms ; une
  interface réelle demande `setcap cap_net_raw` sur le binaire, hors de ces
  scripts.
