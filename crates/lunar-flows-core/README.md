# lunar-flows-core

`lunar-flows-core` est la bibliothèque métier de LUNAR. Elle transforme des
paquets réseau parsés et des fichiers LFMS (LUNAR Flow Matrix Standard, voir
[docs/LFMS.md](../../docs/LFMS.md)) en matrices de flux et graphes réseau
déterministes, sans dépendre de l'interface.

Le crate se concentre sur le traitement passif de données hors ligne. La
capture live et l'interface utilisateur appartiennent à `lunar-engine` et à
l'application `lunar`.

## Capacités

| Module | Rôle |
| --- | --- |
| `matrix` | Agréger les métadonnées de paquets par flux, maintenir labels et provenances, importer et exporter les lignes LFMS. |
| `csv` | Valider strictement, lire, reconstruire et fusionner les fichiers CSV LFMS. |
| `graph` | Construire un graphe réseau et produire des mises à jour de nœuds et d'arêtes coalescibles pour un consommateur. |
| `link` | Projeter les couches de liaison typées vers l'identité LFMS et reconstruire les identités Ethernet, RAW, SLL et SLL2. |
| `packet` | Représenter les paquets capturés et aplatir les flux imbriqués en conservant leur relation de tunnel. |
| `pcap` | Lire (en option) les fichiers PCAP/PCAPNG et les convertir en matrice de flux. |
| `xlsx` | Lire (en option) une matrice exportée en classeur XLSX. |
| `lfms` | Lire et écrire le préambule `#LFMS` (version, DLT, contexte du relevé). |
| `winaudit` | Convertir des relevés de configuration WinAudit en labels. |
| `report` | Rapports de conversion (paquets lus, parsés, non supportés). |
| `error` | Fournir le type `Result` partagé et les erreurs structurées `LunarCoreError`. |

L'ensemble de features par défaut est vide. Le traitement des matrices, CSV,
graphes et paquets ne nécessite aucune bibliothèque native de capture.

| Feature | Par défaut | Effet |
| --- | --- | --- |
| `pcap` | Non | Active la lecture hors ligne de PCAP/PCAPNG via `libpcap` ou Npcap. |
| `xlsx` | Non | Active la lecture des matrices en classeur XLSX (calamine). |

## Utilisation

Le crate n'est pas publié : les autres crates du workspace le consomment en
dépendance `path`.

```toml
[dependencies]
lunar-flows-core = { path = "../lunar-flows-core" }
```

Le nom du paquet utilise des tirets ; Rust l'importe sous le nom
`lunar_flows_core`.

## Fusionner des matrices LFMS

Cet exemple n'utilise que l'ensemble de features par défaut :

```rust
use std::path::{Path, PathBuf};

fn main() -> lunar_flows_core::Result<()> {
    let entrees = vec![
        PathBuf::from("site-a.csv"),
        PathBuf::from("site-b.csv"),
    ];

    let lignes = lunar_flows_core::csv::merge_matrix_files_to_csv(
        &entrees,
        Path::new("fusion.csv"),
    )?;

    println!("{lignes} flux exportés");
    Ok(())
}
```

Chaque entrée est validée avant d'être fusionnée. Les compteurs sont
accumulés, `last_seen` conserve l'horodatage le plus récent, les labels sont
réappliqués et la colonne `origin` enregistre les fichiers sources associés à
chaque flux.

## Convertir des fichiers PCAP

Activer la feature `pcap`, puis appeler l'API de conversion par lots :

```rust
use std::path::{Path, PathBuf};

fn main() -> lunar_flows_core::Result<()> {
    let entrees = vec![PathBuf::from("capture.pcap")];

    let lignes = lunar_flows_core::pcap::convert_pcap_files_to_csv(
        &entrees,
        Path::new("matrice.csv"),
        |chemin, rapport| {
            eprintln!(
                "{} : {} paquets, {} parsés, {} non supportés",
                chemin.display(),
                rapport.packets,
                rapport.parse_ok,
                rapport.parse_errors,
            );
        },
    )?;

    println!("{lignes} flux exportés");
    Ok(())
}
```

Les paquets non supportés sont comptés puis ignorés. L'ouverture ou la
lecture d'une capture corrompue ou tronquée est une erreur fatale : les API
par lots ne retournent jamais une matrice silencieusement partielle.

### Dépendance native pour `pcap`

La feature s'appuie sur la bibliothèque de capture de la plateforme :

- Debian/Ubuntu : installer `libpcap-dev`.
- Fedora : installer `libpcap-devel`.
- macOS : utiliser la `libpcap` système.
- Windows : installer Npcap et configurer son SDK/bibliothèque d'import pour
  le build consommateur.

Les types de paquets sont pris en charge sous Linux, macOS et Windows.

## Sortie LFMS

Une matrice exportée contient une ligne par flux distinct :

```text
mac_source,mac_destination,vlan_id,protocol_data_link,ip_source,ip_source_type,label_source,ip_destination,ip_destination_type,label_destination,port_source,port_destination,protocol_transport,application_protocol,count,total_bytes,last_seen,encap_id,origin
```

Deux colonnes d'extension préservent le contexte d'audit :

- `encap_id` relie les flux de tunnels externes à leurs flux enfants
  décapsulés. Un identifiant hexadécimal nu de 16 caractères signifie que
  chaque paquet de la ligne a utilisé un seul tunnel ; `id:n|id:n` enregistre
  les décomptes de paquets par tunnel.
- `origin` est vide pour les paquets et les imports PCAP. Lors de la fusion
  de matrices, elle contient les noms des fichiers sources, triés et
  dédupliqués, séparés par `|`.

Il n'y a volontairement pas de colonne `link_details`. Pour les captures
Linux « cooked », l'adresse source et le protocole transporté font partie de
l'identité de conversation LFMS. La direction du paquet, le type ARPHRD, la
longueur d'adresse déclarée, les bits réservés et l'index d'interface SLL2
décrivent le point d'observation : les événements paquets les conservent,
mais ils ne scindent pas les lignes de la matrice et n'empêchent pas la
fusion de matrices issues de sondes différentes sur le même réseau. La
colonne `origin` garde les noms des fichiers contributeurs.

## Garanties sur les données

- Les imports CSV rejettent les lignes malformées, les adresses IP invalides
  et les horodatages invalides, avec le numéro de la ligne fautive.
- Les exports CSV ont un ordre de lignes déterministe et utilisent un
  remplacement atomique pour ne jamais laisser un fichier final tronqué après
  une panne d'entrée/sortie.
- Les labels commençant par un caractère de formule de tableur sont échappés
  à l'export et restaurés au réimport.
- Les identifiants de tunnels sont déterministes quels que soient la
  direction, le build et la version de Rust.
- L'export CSV et le réimport préservent les identités LFMS Ethernet, RAW,
  SLL et SLL2, la comptabilité de paquets par tunnel, les labels et les
  provenances des flux.

Les briques de plus bas niveau restent disponibles quand les fonctions par
lots sont trop grossières : `FlowMatrix`, `FlowMatrixRow`, `GraphData`,
`GraphUpdateBatch`, `CapturedPacketOwned`, et les fonctions des modules `csv`
et `pcap`.

## Développement

Le workspace requiert Rust 1.97.1 et utilise l'édition 2024. Depuis la racine
du dépôt :

```sh
cargo fmt -- --check
cargo clippy -p lunar-flows-core --all-targets --all-features -- -D warnings
cargo test -p lunar-flows-core --all-features
cargo doc -p lunar-flows-core --all-features --no-deps
```

Installer la dépendance native ci-dessus avant de compiler ou tester avec
`--all-features`. Les tests lisent leurs fichiers d'entrée dans `fixtures/`, à
la racine du dépôt.

## Licence

Sous licence [AGPL-3.0-only](../../LICENSE.md).
