# LUNAR

<p align="center">
  <img src="crates/lunar/assets/images/logo.svg" alt="Logo de LUNAR" width="128">
</p>

Relevé de matrices de flux réseau, avec une interface native
[GPUI](https://crates.io/crates/gpui) 0.2.2. LUNAR couvre six fonctions :

- capture en direct ;
- import de PCAP et de matrices ;
- graphe des flux ;
- labels (CSV, relevés WinAudit, conflits, arbitrage) ;
- investigation et synthèse ;
- projets et exports.

Ce dépôt est **expérimental**. La CI publie des binaires Linux et Windows, sans
paquet ni signature de code (voir « Intégration continue et versions » et « Hors
périmètre »).

Pour les bugs, les questions et les demandes d'évolution, ouvrez une
[issue](https://github.com/LunarBreez/Lunar/issues). Pour une vulnérabilité,
pas d'issue publique : suivez la [politique de sécurité](SECURITY.md)
(signalement privé).

## Construire et lancer

Prérequis Linux :

- la toolchain Rust 1.97.1, installée par `rust-toolchain.toml` ;
- libpcap ;
- les en-têtes X11/xkbcommon et Vulkan exigés par gpui (`libxkbcommon-x11-dev`,
  `libvulkan1`, un pilote Vulkan) ;
- GTK 3 pour les dialogues de fichiers.

```bash
cargo build --release --locked -p lunar
./target/release/lunar
```

- `--version`, `--help` : répondent sans ouvrir de fenêtre.
- `--lunar-smoke-test` : ouvre la fenêtre jusqu'à la première image, écrit
  `LUNAR_STARTUP_VALIDATION=OK` et rend la main (code 0).

### Capture en direct

La capture exige `CAP_NET_RAW`. Appliquez ce droit **après** le dernier build,
car un nouveau binaire perd ses capabilities :

```bash
sudo setcap cap_net_raw=eip ./target/release/lunar
```

Sans ce droit, l'application démarre normalement et l'import de PCAP marche.
Seul le démarrage d'une capture est refusé, avec un message explicite.

### Affichage

- **Wayland.** Sous une session Wayland, LUNAR passe par XWayland : le backend
  Wayland de gpui 0.2.2 se fige sur un graphe lourd
  (`docs/gpui-plateforme.md`).
- **Xvfb.** Sur un écran Xvfb sans gestionnaire de fenêtres, deux
  contournements sont nécessaires (`docs/gpui-plateforme.md`) :
  - le réveil X11, intégré à LUNAR ;
  - `Xvfb -extension Composite`, à passer au lancement.

### Fichiers de l'application

L'identifiant d'application est `fr.lunar`. LUNAR range sous cet identifiant :

- la configuration ;
- les journaux (`lunar.log`, `DR_LUNAR_<date>.log`) ;
- la sauvegarde automatique ;
- `session.lock`.

## Organisation

| Chemin | Rôle |
|---|---|
| `crates/lunar-flows-core` | Cœur métier : matrices de flux LFMS, CSV et XLSX, graphe réseau, conversion PCAP, relevés WinAudit. |
| `crates/lunar-engine` | Moteur sans interface : capture, imports, labels, projets `.lunar`, exports, derrière la façade `Lunar`. |
| `crates/lunar-ui-logic` | Logique d'interface sans fenêtre, avec ses tests : style et placement du graphe, ForceAtlas2, investigation, synthèse HTML, BPF, menus. |
| `crates/lunar` | Interface : thème, composants, coquille (`app/`), vue du graphe (`graph_view/`). |
| `crates/lunar-flows-core/fuzz` | Cibles de fuzzing (cargo-fuzz) des lecteurs de fichiers non fiables ; workspace à part, hors du `Cargo.lock` du produit. |
| `fixtures/` | Captures, matrices, labels et relevés WinAudit de test. |
| `windows/npcap-sdk` | Bibliothèques d'import du SDK Npcap, pour l'édition des liens sous Windows. |
| `docs/` | Format de matrice LFMS (`LFMS.md`), notes de plateforme gpui (`gpui-plateforme.md`). |
| `script/` | `build-windows-check.sh`, `ci/`, `e2e/`, `fuzz/`. |

## Vérifier

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
script/build-windows-check.sh          # clippy et .exe Windows (cargo-xwin)
script/fuzz/smoke.sh                   # fuzzing borné des lecteurs de fichiers
script/ci/tests/test-check-binary-is-passive.sh
script/ci/check-binary-is-passive.sh target/release/lunar
```

Le fuzzing (cargo-fuzz, toolchain nightly) est décrit dans
`crates/lunar-flows-core/fuzz/README.md`. Le harnais E2E et la mesure mémoire
longue durée sont décrits dans `script/e2e/README.md`.

Faites les essais graphiques **uniquement sur un écran Xvfb isolé**, jamais
sur la session de l'opérateur (`env -u DISPLAY -u WAYLAND_DISPLAY -u
DBUS_SESSION_BUS_ADDRESS -u XAUTHORITY`).

## Particularités

- **Formats.**
  - Les projets sont des archives `.lunar`. Leur manifeste et la synthèse HTML
    portent la version de LUNAR.
  - Les matrices suivent le format LFMS (« LUNAR Flow Matrix Standard »,
    préambule `#LFMS`, voir `docs/LFMS.md`).
  - Les interfaces du poste de relevé reçoivent le label « pc lunar ».
- **Plateformes.** Linux, avec X11 ou XWayland. Les binaires Windows sont
  construits par la CI. `script/build-windows-check.sh` en fait une compilation
  de contrôle depuis Linux : profil `windows-check`, console visible, shaders
  compilés au lancement. macOS n'est pas visé.
- **Menus.** La barre de menus est dessinée dans la fenêtre, car gpui n'a pas de
  menu natif sous Linux. Chaque action a son raccourci clavier.
- **Icônes.** Tabler Icons 3.50.0 (MIT, `crates/lunar/assets/icons/SOURCE.md`).
- **Graphe.**
  - À l'écran, les libellés d'arêtes restent horizontaux, car gpui ne tourne
    pas le texte. L'export PNG les tourne.
  - Le placement initial ne dépend pas de l'ordre des données : la même capture
    donne toujours le même dessin.
  - L'export PNG utilise la police Liberation Sans (SIL OFL 1.1, métriques
    d'Arial).
- **Import XLSX.** Il passe par le même lecteur que le CSV et subit les mêmes
  contrôles : type de liaison et neutralisation des formules.
- **Sauvegarde automatique.** Elle démarre et s'arrête explicitement. Une
  fermeture propre retire `session.lock`.
- **Ergonomie.**
  - Retrait d'un fichier et statut par fichier dans le panneau d'import.
  - Bouton de rafraîchissement des interfaces.
  - Message explicite pour un champ numérique invalide.
  - Confirmation avant de vider les labels.
  - Notification de succès de chaque export.
  - Préréglages BPF : « IPv4 uniquement », « Trafic web (80/443) », « SYN TCP
    uniquement ».
- **Changelog.** L'entrée « Changelog » du menu Aide affiche les notes de la
  dernière version, tirées de `CHANGELOG.md`.

## Intégration continue et versions

- **`.github/workflows/ci.yml`**, à chaque push sur `main` et sur chaque pull
  request :
  - Linux (Ubuntu 22.04) : formatage, clippy `-D warnings` et tests ;
  - Windows : clippy, compilation et `--version` sans Npcap ;
  - dépendances : `cargo deny check` (avis RustSec, licences, sources), selon
    `deny.toml` ;
  - fuzzing borné des lecteurs de fichiers non fiables (capture PCAP, matrice
    CSV, relevé WinAudit), 90 s par cible ;
  - garde-fous de release : le contrôle de passivité est éprouvé sur des
    binaires fabriqués pour émettre, qu'il doit refuser.
- **`.github/workflows/release.yml`**, au push d'un tag `vX.Y.Z` égal à la
  version du workspace :
  - binaires release Linux et Windows, avec un smoke test pour chacun : sur
    Linux, première image sur Xvfb avec lavapipe ; sur Windows, `--version` ;
  - contrôle de passivité du binaire Linux : aucune primitive d'émission
    libpcap importée, aucun client HTTP embarqué
    (`script/ci/check-binary-is-passive.sh`) ; le binaire Windows n'est pas
    couvert ;
  - archives `lunar-X.Y.Z-x86_64-linux-gnu.tar.gz` et
    `lunar-X.Y.Z-x86_64-windows.zip`, `SHA256SUMS` ;
  - SBOM CycloneDX 1.5 de chaque cible (`lunar-X.Y.Z-<cible>.cdx.json`) ;
  - attestation de provenance GitHub et signature Sigstore (cosign, sans clé)
    de chaque archive et de chaque SBOM, produites par le job qui l'a
    construit ;
  - vérification de `SHA256SUMS`, des signatures et des attestations avant
    toute publication ;
  - release créée en brouillon, rendue publique une fois tous les fichiers en
    place (environnement `production`, où une validation manuelle peut être
    exigée).
- **Release à blanc** (lancement manuel de `release.yml`) : toute la chaîne
  tourne sur des fichiers étiquetés `X.Y.Z-dev.<commit>` : compilation, smoke
  tests, SBOM, attestation, signature, puis vérification de chaque signature
  et de chaque attestation. Seule la publication est sautée ; le jeu complet
  reste dans l'artefact `release-files` du run. Les signatures d'essai sont
  inscrites au journal public Sigstore, liées à la branche et non à un tag.
- **Actions tierces** : elles sont épinglées par SHA. Dependabot propose leurs
  mises à jour (`.github/dependabot.yml`).

Pour publier une version :

```bash
git tag -a v0.1.0 -m "LUNAR 0.1.0" && git push origin v0.1.0
```

Pour vérifier une archive :

```bash
sha256sum -c --ignore-missing SHA256SUMS
gh attestation verify lunar-0.1.0-x86_64-linux-gnu.tar.gz --repo LunarBreez/Lunar
cosign verify-blob lunar-0.1.0-x86_64-linux-gnu.tar.gz \
  --bundle lunar-0.1.0-x86_64-linux-gnu.tar.gz.sigstore.json \
  --certificate-identity "https://github.com/LunarBreez/Lunar/.github/workflows/release.yml@refs/tags/v0.1.0" \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
```

## Dépendances vendorisées

Les sources de toutes les dépendances sont commitées dans `vendor/`, aux
versions des `Cargo.lock` du produit et du crate de fuzzing, pour les deux
cibles livrées : Linux x86_64 (Ubuntu 22.04) et Windows 10/11 x86_64 (MSVC).
Les crates qu'aucune de ces cibles ne demande (macOS, WebAssembly, Windows GNU
ou 32 bits…) sont réduits à leur `Cargo.toml` : ni leurs sources ni leurs
binaires précompilés n'entrent dans le dépôt. `.cargo/config.toml` y redirige crates.io : développement, CI et
release compilent depuis le dépôt, sans rien télécharger, et une mise à jour de
dépendance se relit comme n'importe quel diff.

```sh
cargo build --release --locked --offline      # un clone suffit, sans réseau
script/vendor.sh --update -p packet_parser    # mettre à jour une dépendance
script/vendor.sh --update --workspace         # après retouche d'un Cargo.toml
script/vendor.sh --check                      # vendor/ couvre-t-il les Cargo.lock ?
script/vendor.sh --verify                     # vendor/ identique à crates.io ? (CI)
```

`cargo update` seul ne voit plus crates.io : passer par `script/vendor.sh
--update`, qui réaligne aussi le `Cargo.lock` du fuzzing. La CI régénère
`vendor/` depuis crates.io et refuse tout écart avec ce qui est commité. Les
sources vendorisées ne sont pas modifiées à la main.

## Hors périmètre

- paquets `.deb` et NSIS ;
- signature de code (Authenticode, signature hors ligne) ;
- build reproductible.

Le `Cargo.lock` est commité. libc 0.2.186, tokio 1.53.3 et mio 1.2.1 y sont
figés volontairement : `xattr` 0.2.3, tiré par gpui, ne compile plus avec
libc ≥ 0.2.191.

## Licence

AGPL-3.0-only (`LICENSE.md`). gpui est sous Apache-2.0.
