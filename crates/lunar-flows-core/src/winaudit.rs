//! Dérivation de labels depuis un relevé de configuration WinAudit.
//!
//! WinAudit collecte la configuration d'une machine et l'écrit dans un CSV à
//! rubriques numérotées. Ce relevé porte exactement ce qu'il faut pour
//! nommer un équipement dans LUNAR : le nom de la machine
//! (rubrique `300`) et, par adaptateur, son adresse MAC et ses adresses IP
//! (rubrique `2600`). Le nom du fichier, `AAAAMMJJ_CLASSIF_CONF_TRI-BI-NNNNNN.csv`,
//! donne en plus l'identifiant d'inventaire SALP de la machine.
//!
//! Le relevé décrit ce que la machine **déclare d'elle-même** ; la matrice
//! décrit ce qui a été **observé sur le fil**. Ce module ne fait que traduire
//! le premier en référentiel de labels : il ne rapproche rien, ne déduit rien,
//! et laisse le rapprochement à `FlowMatrix::get_label`.
//!
//! Les règles sont celles de l'outil `winaudit2labels`, pour qu'un même
//! relevé donne les mêmes labels par les deux chemins :
//!
//! - label « identifiant SALP + nom d'hôte » (le nom d'hôte seul si le nom de
//!   fichier ne porte pas d'identifiant) ;
//! - par interface, une ligne par adresse, plus une ligne de repli à IP vide :
//!   `FlowMatrix::get_label` cherche le couple exact, puis l'IP seule, puis la
//!   MAC seule — cette dernière est la seule qui nomme un flux sans couche 3,
//!   l'ARP par exemple ;
//! - MAC et adresses sous forme canonique ; boucle locale, adresses non
//!   spécifiées, multicast et diffusion, MAC nulles et de groupe écartées ;
//! - interfaces virtuelles (MAC localement administrée : ponts Docker, `veth`,
//!   `virbr`…) écartées par défaut, gardées sur demande ;
//! - deux relevés qui nomment différemment un même couple (MAC, IP) : le plus
//!   récent l'emporte (date et heure de la rubrique `300`, sinon date du nom
//!   de fichier), le premier lu à horodatage égal.
//!
//! Les deux outils partagent aussi leurs réglages par défaut : lancés sans
//! option, ils écrivent le même fichier.
//!
//! Deux propriétés du format guident l'implémentation :
//!
//! - l'encodage est de l'UTF-16 avec BOM, pas de l'UTF-8 ; le séparateur est
//!   détecté (virgule, point-virgule ou tabulation) ;
//! - les rubriques sont lues **par position**. Un lecteur qui le fait sans
//!   garde voit une ligne trop courte interrompre la conversion d'un lot
//!   entier. On ne reproduit pas ce défaut :
//!   une ligne inexploitable est comptée et ignorée, jamais fatale.

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

use crate::{LunarCoreError, Result};

/// Rubrique WinAudit décrivant la machine elle-même.
const SECTION_MACHINE: &str = "300";
/// Rubrique WinAudit décrivant les adaptateurs réseau.
const SECTION_NETWORK: &str = "2600";

/// Colonne du nom de machine dans la rubrique `300`.
const MACHINE_NAME: usize = 2;
/// Colonne de la date du relevé dans la rubrique `300`, par exemple
/// « 2026-09-18 16:11:55 +0200 ».
const MACHINE_DATE: usize = 20;
/// Colonne du nom de l'adaptateur dans la rubrique `2600` (sert aux
/// traces).
const ADAPTER_NAME: usize = 3;
/// Colonne des adresses IP de l'adaptateur dans la rubrique `2600`,
/// séparées par des blancs, index de zone `%iface` possible.
const ADAPTER_ADDRESSES: usize = 6;
/// Colonne du type d'adaptateur dans la rubrique `2600` (« Software
/// Loopback » est écarté).
const ADAPTER_TYPE: usize = 16;
/// Colonne de l'adresse MAC dans la rubrique `2600`, sous une forme
/// libre ramenée à la forme canonique.
const ADAPTER_MAC: usize = 17;

/// Une ligne du fichier de labels : `mac,ip,label`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WinAuditLabel {
    /// MAC canonique de l'interface (`aa:bb:cc:dd:ee:ff`).
    pub mac: String,
    /// Vide pour la ligne de repli de l'interface : le label vaut alors pour
    /// la MAC seule.
    pub ip: String,
    /// Nom proposé : « identifiant SALP + nom d'hôte », ou le nom d'hôte
    /// seul.
    pub label: String,
    /// Relevé d'où vient le label. N'est pas écrit dans le fichier de labels :
    /// il sert à montrer à l'utilisateur la provenance d'un nom qu'il arbitre.
    pub source: PathBuf,
}

/// Un nom proposé pour un couple (MAC, IP), avec le relevé qui le porte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WinAuditCandidate {
    /// Nom proposé pour le couple.
    pub label: String,
    /// Relevé qui le propose.
    pub source: PathBuf,
}

/// Un couple (MAC, IP) nommé différemment par plusieurs relevés du lot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WinAuditConflict {
    /// MAC canonique du couple en désaccord.
    pub mac: String,
    /// IP du couple, vide pour la ligne de repli à MAC seule.
    pub ip: String,
    /// Le nom retenu : celui du relevé le plus récent, ou du premier lu à
    /// horodatage égal. C'est celui que porte le label.
    pub kept: WinAuditCandidate,
    /// Les autres noms, dans l'ordre où ils ont été écartés.
    pub dropped: Vec<WinAuditCandidate>,
}

/// Ce que la conversion a retenu, et ce qu'elle a écarté. Rendu à l'appelant
/// pour que l'humain voie le bilan avant d'appliquer quoi que ce soit.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WinAuditReport {
    /// Relevés lus jusqu'au bout et exploités (les rejetés sont à part).
    pub files_read: usize,
    /// Fichiers ignorés parce qu'illisibles ou d'un autre format.
    pub files_rejected: Vec<(PathBuf, String)>,
    /// Machines décrites, une par relevé lu : égal à `files_read`.
    pub machines: usize,
    /// Lignes de rubrique `2600` rencontrées, retenues ou non.
    pub adapters: usize,
    /// Lignes de labels produites, après arbitrage des conflits.
    pub labels: usize,
    /// Adaptateurs à MAC localement administrée (ponts Docker, `veth`,
    /// `virbr`…). Comptés qu'ils soient gardés ou non : le bilan dit ce que
    /// la conversion a retiré.
    pub virtual_adapters: usize,
    /// Parmi eux, ceux effectivement écartés : tous, sauf avec `keep_virtual`.
    pub skipped_virtual: usize,
    /// Adaptateurs de boucle locale (type « Software Loopback ») ou à MAC
    /// nulle.
    pub skipped_loopback: usize,
    /// Lignes `2600` sans MAC lisible : tronquées par le collecteur, ou
    /// adaptateur sans adresse matérielle.
    pub skipped_without_mac: usize,
    /// Adaptateurs à MAC de groupe (multicast, diffusion) : aucun équipement.
    pub skipped_group_mac: usize,
    /// Adresses écartées : boucle locale, non spécifiées, multicast, diffusion
    /// ou illisibles.
    pub skipped_addresses: usize,
    /// Relevés dont le nom ne porte pas d'identifiant SALP : leur label se
    /// réduit au nom d'hôte.
    pub files_without_identifier: Vec<PathBuf>,
    /// Relevés lus mais sans rubrique `2600` : collecte antérieure à
    /// l'inventaire réseau, aucun label possible. Signalés comme par
    /// `winaudit2labels`, pour qu'un lot sans label dise pourquoi.
    pub files_without_network_section: Vec<PathBuf>,
    /// Lignes trop courtes pour la rubrique annoncée : comptées, jamais fatales.
    pub malformed_rows: usize,
    /// Même couple (MAC, IP) vu avec deux noms différents : le relevé le plus
    /// récent est gardé, l'arbitrage revient à l'utilisateur.
    pub conflicts: Vec<String>,
    /// Les mêmes conflits, structurés pour un arbitrage relevé par relevé.
    pub conflict_details: Vec<WinAuditConflict>,
}

/// Réglages de la conversion.
#[derive(Debug, Default, Clone, Copy)]
pub struct WinAuditOptions {
    /// Garder les adaptateurs à MAC localement administrée (ponts Docker,
    /// `veth`, `virbr`…).
    ///
    /// Faux par défaut, comme dans `winaudit2labels` : ces
    /// interfaces n'apparaissent pas sur le réseau observé et leur MAC change
    /// d'un démarrage à l'autre. Sur les relevés réels d'une forge de 14 VM,
    /// elles faisaient 61 des 111 lignes. Le bilan les compte, gardées ou non.
    pub keep_virtual: bool,
}

/// Décode un relevé WinAudit en texte.
///
/// Le format impose l'UTF-16 avec BOM ; l'UTF-8 est accepté en plus pour les
/// relevés déjà reconvertis par un outil tiers.
fn decode_survey(path: &Path, bytes: &[u8]) -> Result<String> {
    let invalid = |message: &str| LunarCoreError::InvalidCsv {
        path: path.to_path_buf(),
        message: message.to_string(),
    };

    match bytes {
        [0xFF, 0xFE, rest @ ..] => decode_utf16(rest, u16::from_le_bytes)
            .ok_or_else(|| invalid("UTF-16LE invalide (BOM FF FE)")),
        [0xFE, 0xFF, rest @ ..] => decode_utf16(rest, u16::from_be_bytes)
            .ok_or_else(|| invalid("UTF-16BE invalide (BOM FE FF)")),
        [0xEF, 0xBB, 0xBF, rest @ ..] => {
            String::from_utf8(rest.to_vec()).map_err(|_| invalid("UTF-8 invalide (BOM EF BB BF)"))
        }
        _ => String::from_utf8(bytes.to_vec()).map_err(|_| {
            invalid("encodage non reconnu : un relevé WinAudit est en UTF-16 avec BOM")
        }),
    }
}

/// Décode des unités UTF-16 lues deux octets par deux, `to_unit` fixant
/// le boutisme. `None` si la longueur est impaire ou si une paire de
/// substituts est invalide.
fn decode_utf16(bytes: &[u8], to_unit: fn([u8; 2]) -> u16) -> Option<String> {
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| to_unit([pair[0], pair[1]]))
        .collect();
    String::from_utf16(&units).ok()
}

/// Identifiant d'inventaire SALP tiré du nom d'un relevé
/// `AAAAMMJJ_CLASSIF_CONF_TRI-BI-NNNNNN.csv` : le dernier segment séparé par
/// `_`, de la forme lettres-deux lettres-six chiffres. Vide si le nom ne suit
/// pas cette convention.
///
/// Même expression que l'outil `winaudit2labels` : un même relevé doit
/// donner le même label par les deux chemins.
pub fn salp_identifier(file_name: &str) -> String {
    let stem = file_name
        .rsplit_once('.')
        .map_or(file_name, |(stem, _)| stem);
    stem.rsplit('_')
        .next()
        .filter(|candidate| {
            let parts: Vec<&str> = candidate.split('-').collect();
            parts.len() == 3
                && !parts[0].is_empty()
                && parts[0].chars().all(|c| c.is_ascii_alphabetic())
                && parts[1].len() == 2
                && parts[1].chars().all(|c| c.is_ascii_alphabetic())
                && parts[2].len() == 6
                && parts[2].chars().all(|c| c.is_ascii_digit())
        })
        .unwrap_or_default()
        .to_string()
}

/// Date `AAAAMMJJ` en tête du nom d'un relevé
/// `AAAAMMJJ_CLASSIF_CONF_TRI-BI-NNNNNN.csv`, vide si le nom ne commence pas
/// par huit chiffres — même règle que `winaudit2labels`.
fn survey_file_date(file_name: &str) -> String {
    let stem = file_name
        .rsplit_once('.')
        .map_or(file_name, |(stem, _)| stem);
    stem.split('_')
        .next()
        .filter(|date| date.len() == 8 && date.chars().all(|c| c.is_ascii_digit()))
        .unwrap_or_default()
        .to_string()
}

/// Séparateur d'un relevé : le plus fréquent de `,`, `;` et tabulation sur la
/// première ligne non vide, virgule par défaut — règle de `winaudit2labels`.
fn detect_delimiter(text: &str) -> u8 {
    let header = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("");
    b",;\t"
        .iter()
        .copied()
        .max_by_key(|&delimiter| header.matches(char::from(delimiter)).count())
        .filter(|&delimiter| header.contains(char::from(delimiter)))
        .unwrap_or(b',')
}

/// MAC canonique : 12 chiffres hexadécimaux en minuscules, séparés
/// par `:`. Accepte `:`, `-`, `.`, l'espace ou aucun séparateur.
fn canonical_mac(value: &str) -> Option<String> {
    let digits: Vec<char> = value
        .chars()
        .filter(|c| !matches!(c, ':' | '-' | '.' | ' '))
        .collect();
    if digits.len() != 12 || !digits.iter().all(char::is_ascii_hexdigit) {
        return None;
    }
    let mut mac = String::with_capacity(17);
    for (index, pair) in digits.chunks(2).enumerate() {
        if index > 0 {
            mac.push(':');
        }
        mac.extend(pair.iter().map(char::to_ascii_lowercase));
    }
    Some(mac)
}

/// Une MAC est-elle celle d'une interface virtuelle éphémère ?
///
/// Le bit « localement administré » (0x02 du premier octet) est posé par
/// Docker, `veth`, `virbr` et consorts, qui tirent une MAC au hasard à chaque
/// création. Ces interfaces n'apparaissent pas sur le réseau observé et leur
/// MAC change au prochain relevé : en faire des labels pollue le référentiel.
fn is_locally_administered(mac: &str) -> bool {
    first_octet(mac).is_some_and(|octet| octet & 0x02 != 0)
}

/// MAC de groupe (multicast, diffusion) : elle ne désigne aucun équipement.
fn is_group_mac(mac: &str) -> bool {
    first_octet(mac).is_some_and(|octet| octet & 0x01 != 0)
}

/// Premier octet d'une MAC `xx:…`, porteur des bits de groupe et
/// d'administration locale ; `None` s'il n'est pas hexadécimal.
fn first_octet(mac: &str) -> Option<u8> {
    let head = mac.split(':').next()?;
    u8::from_str_radix(head, 16).ok()
}

/// MAC nulle `00:00:00:00:00:00` : adaptateur sans adresse matérielle
/// (boucle locale, tunnels), traité comme la boucle locale.
fn is_null_mac(mac: &str) -> bool {
    mac == "00:00:00:00:00:00"
}

/// Adresse utilisable comme clé de label : ni boucle locale (`127.0.0.1`
/// désignerait toutes les machines à la fois), ni non spécifiée, ni multicast,
/// ni diffusion limitée.
fn is_usable_address(address: &IpAddr) -> bool {
    !address.is_loopback()
        && !address.is_unspecified()
        && !address.is_multicast()
        && *address != IpAddr::from([255, 255, 255, 255])
}

/// Cellule `index` d'une ligne, sans blancs de bord ; `None` si elle
/// est absente ou vide. Une ligne trop courte n'est donc jamais une
/// erreur, contrairement au consommateur de WinAudit.
fn cell(row: &csv::StringRecord, index: usize) -> Option<&str> {
    row.get(index)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

/// Un adaptateur retenu dans le relevé.
#[derive(Debug)]
struct Adapter {
    /// MAC canonique.
    mac: String,
    /// Adresses utilisables, sous forme canonique, sans index de zone.
    addresses: Vec<String>,
}

/// Ce qu'un relevé apporte au référentiel.
#[derive(Debug)]
struct Survey {
    /// Label de la machine : « identifiant SALP + nom d'hôte », ou le nom
    /// d'hôte seul.
    label: String,
    /// Adaptateurs retenus après filtrage (boucle locale, MAC nulle ou de
    /// groupe, virtuelles écartées).
    adapters: Vec<Adapter>,
    /// Horodatage triable `AAAAMMJJhhmmss`, qui départage deux relevés en
    /// désaccord : le plus récent l'emporte.
    timestamp: String,
}

/// Lit un relevé et en extrait son label et ses adaptateurs retenus.
///
/// Le label est « identifiant SALP + nom d'hôte », l'identifiant venant du nom
/// du fichier et le nom d'hôte de la rubrique `300` — le label par défaut de
/// `winaudit2labels`. Sans identifiant dans le nom, il reste le nom d'hôte.
///
/// L'horodatage suit aussi `winaudit2labels` : les chiffres des 19 premiers
/// caractères de la date de la rubrique `300` (« 2026-09-18 16:11:55 +0200 »
/// donne « 20260918161155 »). Sans heure exploitable, la date du nom de
/// fichier, complétée de zéros.
fn read_survey(
    path: &Path,
    options: WinAuditOptions,
    report: &mut WinAuditReport,
) -> Result<Survey> {
    let bytes = std::fs::read(path)?;
    let text = decode_survey(path, &bytes)?;
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut date = survey_file_date(&file_name);
    let mut timestamp = String::new();

    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        // Les rubriques n'ont pas toutes le même nombre de colonnes.
        .flexible(true)
        .delimiter(detect_delimiter(&text))
        .from_reader(text.as_bytes());

    let mut machine_name: Option<String> = None;
    let mut adapters: Vec<Adapter> = Vec::new();
    let mut saw_machine = false;
    let mut saw_network = false;

    for record in reader.records() {
        let row = match record {
            Ok(row) => row,
            // Ligne illisible (guillemets déséquilibrés) : comptée, le reste
            // du fichier continue d'être lu.
            Err(_) => {
                report.malformed_rows += 1;
                continue;
            }
        };

        match row.get(0).map(str::trim) {
            Some(SECTION_MACHINE) => {
                saw_machine = true;
                // Comme `winaudit2labels` : la date vient de la ligne qui
                // donne le nom de machine, ou de la dernière lue sans nom.
                if machine_name.is_none() {
                    timestamp = row
                        .get(MACHINE_DATE)
                        .map(str::trim)
                        .unwrap_or_default()
                        .chars()
                        .take(19)
                        .filter(char::is_ascii_digit)
                        .collect();
                    if date.is_empty() {
                        date = timestamp.chars().take(8).collect();
                    }
                }
                match cell(&row, MACHINE_NAME) {
                    Some(name) => machine_name.get_or_insert_with(|| name.to_string()),
                    None => {
                        report.malformed_rows += 1;
                        continue;
                    }
                };
            }
            Some(SECTION_NETWORK) => {
                saw_network = true;
                report.adapters += 1;
                let Some(mac) = cell(&row, ADAPTER_MAC).and_then(canonical_mac) else {
                    // Ligne tronquée par le collecteur, ou adaptateur sans
                    // adresse matérielle lisible : aucun label possible.
                    report.skipped_without_mac += 1;
                    continue;
                };

                let adapter_type = cell(&row, ADAPTER_TYPE).unwrap_or_default();
                let adapter_name = cell(&row, ADAPTER_NAME).unwrap_or_default();

                if is_null_mac(&mac) || adapter_type.eq_ignore_ascii_case("Software Loopback") {
                    report.skipped_loopback += 1;
                    continue;
                }
                if is_group_mac(&mac) {
                    report.skipped_group_mac += 1;
                    continue;
                }
                if is_locally_administered(&mac) {
                    report.virtual_adapters += 1;
                    if !options.keep_virtual {
                        log::debug!(
                            "{} : adaptateur {adapter_name} écarté, MAC {mac} localement administrée",
                            path.display()
                        );
                        report.skipped_virtual += 1;
                        continue;
                    }
                }

                let mut addresses = Vec::new();
                for written in cell(&row, ADAPTER_ADDRESSES)
                    .unwrap_or_default()
                    .split_whitespace()
                {
                    // `fe80::1%eth0` : l'index de zone n'appartient pas à
                    // l'adresse. Forme canonique, celle du parser de paquets.
                    let bare = written.split('%').next().unwrap_or(written);
                    match bare.parse::<IpAddr>() {
                        Ok(address) if is_usable_address(&address) => {
                            addresses.push(address.to_string());
                        }
                        _ => report.skipped_addresses += 1,
                    }
                }

                adapters.push(Adapter { mac, addresses });
            }
            _ => {}
        }
    }

    if !saw_machine && !saw_network {
        return Err(LunarCoreError::InvalidCsv {
            path: path.to_path_buf(),
            message: "aucune rubrique WinAudit reconnue (300 ou 2600)".to_string(),
        });
    }

    let identifier = salp_identifier(&file_name);
    let label = format!("{identifier} {}", machine_name.unwrap_or_default())
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    if label.is_empty() {
        return Err(LunarCoreError::InvalidCsv {
            path: path.to_path_buf(),
            message:
                "relevé sans identifiant SALP ni nom de machine : aucun label ne peut en être tiré"
                    .to_string(),
        });
    }
    if identifier.is_empty() {
        report.files_without_identifier.push(path.to_path_buf());
    }
    if !saw_network {
        report
            .files_without_network_section
            .push(path.to_path_buf());
    }

    if timestamp.len() < 14 {
        timestamp = format!("{date:0<14}");
    }

    report.machines += 1;
    Ok(Survey {
        label,
        adapters,
        timestamp,
    })
}

/// Convertit un ou plusieurs relevés WinAudit en lignes de labels.
///
/// Les entrées peuvent être des fichiers ou des répertoires ; un répertoire
/// est parcouru récursivement (les collectes rangent leurs relevés par
/// sous-dossier), ses `.csv` seulement. Les relevés sont lus dans l'ordre de
/// leur chemin, comme par `winaudit2labels`. Un fichier illisible est
/// rapporté et n'interrompt pas le lot.
pub fn labels_from_surveys(
    inputs: &[PathBuf],
    options: WinAuditOptions,
) -> Result<(Vec<WinAuditLabel>, WinAuditReport)> {
    if inputs.is_empty() {
        return Err(LunarCoreError::MissingInput);
    }

    let mut files: Vec<PathBuf> = Vec::new();
    for input in inputs {
        if !input.exists() {
            return Err(LunarCoreError::MissingInputFile(input.clone()));
        }
        if input.is_dir() {
            let mut to_visit = vec![input.clone()];
            while let Some(dir) = to_visit.pop() {
                for entry in std::fs::read_dir(&dir)? {
                    let path = entry?.path();
                    if path.is_dir() {
                        to_visit.push(path);
                    } else if path
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("csv"))
                    {
                        files.push(path);
                    }
                }
            }
        } else {
            files.push(input.clone());
        }
    }
    files.sort();

    let mut report = WinAuditReport::default();
    let mut surveys: Vec<(Survey, &PathBuf)> = Vec::new();
    for file in &files {
        match read_survey(file, options, &mut report) {
            Ok(survey) => {
                report.files_read += 1;
                surveys.push((survey, file));
            }
            Err(error) => report
                .files_rejected
                .push((file.clone(), error.to_string())),
        }
    }

    // Ordre stable : deux exécutions sur le même lot donnent le même fichier.
    let mut seen = Seen::default();
    for (survey, source) in surveys {
        let proposal = Proposal {
            label: &survey.label,
            timestamp: &survey.timestamp,
            source,
        };
        for adapter in survey.adapters {
            for address in adapter.addresses {
                seen.insert(&mut report, adapter.mac.clone(), address, &proposal);
            }
            // Ligne de repli à IP vide, pour chaque interface : seule à nommer
            // les flux sans couche 3 (ARP), comme dans `winaudit2labels`.
            seen.insert(&mut report, adapter.mac, String::new(), &proposal);
        }
    }

    let labels: Vec<WinAuditLabel> = seen
        .labels
        .into_iter()
        .map(|((mac, ip), kept)| WinAuditLabel {
            mac,
            ip,
            label: kept.label,
            source: kept.source,
        })
        .collect();
    report.labels = labels.len();
    report.conflict_details = seen.conflicts.into_values().collect();

    Ok((labels, report))
}

/// Le nom qu'un relevé propose pour ses couples (MAC, IP).
struct Proposal<'a> {
    /// Nom proposé par le relevé.
    label: &'a str,
    /// Horodatage `AAAAMMJJhhmmss` du relevé, critère de départage.
    timestamp: &'a str,
    /// Chemin du relevé.
    source: &'a Path,
}

impl Proposal<'_> {
    /// Copie possédée de la proposition, quand elle devient le nom retenu
    /// d'un couple.
    fn kept(&self) -> Kept {
        Kept {
            label: self.label.to_string(),
            timestamp: self.timestamp.to_string(),
            source: self.source.to_path_buf(),
        }
    }

    /// Vue publique de la proposition, sans horodatage, pour le bilan des
    /// conflits.
    fn candidate(&self) -> WinAuditCandidate {
        WinAuditCandidate {
            label: self.label.to_string(),
            source: self.source.to_path_buf(),
        }
    }
}

/// Le nom retenu pour un couple (MAC, IP), et d'où il vient.
struct Kept {
    /// Nom retenu.
    label: String,
    /// Horodatage du relevé retenu, comparé à celui de toute proposition
    /// suivante pour le même couple.
    timestamp: String,
    /// Relevé d'où vient le nom retenu.
    source: PathBuf,
}

impl Kept {
    /// Vue publique du nom retenu, pour le champ `kept` d'un conflit.
    fn candidate(&self) -> WinAuditCandidate {
        WinAuditCandidate {
            label: self.label.clone(),
            source: self.source.clone(),
        }
    }
}

/// Labels retenus par couple (MAC, IP), et conflits entre relevés du lot.
#[derive(Default)]
struct Seen {
    /// Nom retenu par couple (MAC, IP) ; `BTreeMap` pour une sortie triée
    /// et déterministe.
    labels: BTreeMap<(String, String), Kept>,
    /// Conflits par couple, créés à la première divergence et complétés
    /// à chaque nom écarté ensuite.
    conflicts: BTreeMap<(String, String), WinAuditConflict>,
}

impl Seen {
    /// Retient `proposal` pour le couple, ou arbitre le désaccord comme
    /// `winaudit2labels` : le relevé le plus récent l'emporte, le premier lu
    /// à horodatage égal. Le nom écarté rejoint le conflit du couple.
    fn insert(
        &mut self,
        report: &mut WinAuditReport,
        mac: String,
        ip: String,
        proposal: &Proposal,
    ) {
        use std::collections::btree_map::Entry;

        let key = (mac, ip);
        let kept = match self.labels.entry(key.clone()) {
            Entry::Vacant(slot) => {
                slot.insert(proposal.kept());
                return;
            }
            Entry::Occupied(slot) => slot.into_mut(),
        };
        if kept.label == proposal.label {
            return;
        }

        let conflict = self
            .conflicts
            .entry(key.clone())
            .or_insert_with(|| WinAuditConflict {
                mac: key.0.clone(),
                ip: key.1.clone(),
                kept: kept.candidate(),
                dropped: Vec::new(),
            });
        let reason = if proposal.timestamp > kept.timestamp.as_str() {
            let previous = std::mem::replace(&mut conflict.kept, proposal.candidate());
            conflict.dropped.push(previous);
            *kept = proposal.kept();
            "relevé plus récent"
        } else {
            conflict.dropped.push(proposal.candidate());
            if proposal.timestamp == kept.timestamp {
                "même horodatage : le premier lu est gardé"
            } else {
                "relevé plus ancien"
            }
        };
        let dropped = conflict.dropped.last().map_or("", |c| c.label.as_str());
        report.conflicts.push(format!(
            "{} / {} : « {} » gardé, « {dropped} » écarté ({reason})",
            key.0,
            if key.1.is_empty() {
                "(sans IP)"
            } else {
                &key.1
            },
            kept.label,
        ));
    }
}

/// Écrit les labels au format du fichier de labels : `mac,ip,label`, en-tête
/// comprise.
pub fn write_label_csv(labels: &[WinAuditLabel], output: &Path) -> Result<usize> {
    let invalid = |error: csv::Error| LunarCoreError::InvalidCsv {
        path: output.to_path_buf(),
        message: error.to_string(),
    };

    let mut writer = csv::Writer::from_path(output).map_err(invalid)?;
    writer
        .write_record(["mac", "ip", "label"])
        .map_err(invalid)?;
    for label in labels {
        writer
            .write_record([&label.mac, &label.ip, &label.label])
            .map_err(invalid)?;
    }
    writer.flush()?;
    Ok(labels.len())
}
