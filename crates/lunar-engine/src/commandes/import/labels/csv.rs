//! Lecture et parsing des fichiers de labels CSV : lignes brutes, détection
//! d'en-tête, normalisation des champs mac/ip/label vers les formes produites
//! par le parser de paquets.
//!
//! Un classeur `.xlsx` est lu par le même chemin : sa première feuille est
//! rendue en texte CSV (cf. [`lunar_flows_core::xlsx::first_sheet_as_csv`])
//! avant tout contrôle.

use std::{io::ErrorKind, net::IpAddr};

use crate::{
    errors::{CaptureStateError, label::LabelError},
    setup::labels::{canonical_mac, clean_csv_field, label_row_is_blank, parse_label_fields},
};

/// Ligne de labels lue d'un fichier, champs normalisés vers les formes du
/// parser de paquets (cf. `read_label_rows_report`) ; `raw` garde la
/// ligne d'origine pour les messages d'erreur.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LabelRow {
    /// Numéro de ligne dans le fichier (à partir de 1), pour les erreurs.
    pub(super) line: usize,
    /// Ligne telle que lue, champs joints par des virgules, avant normalisation.
    pub(super) raw: String,
    /// MAC canonique (minuscules, `:`) ; gardée brute en minuscules si non
    /// reconnue, pour que la validation la rapporte.
    pub(super) mac: String,
    /// IP canonique (IPv6 compressée) ; gardée brute si non parseable.
    pub(super) ip: String,
    /// Label, préfixe anti-injection de formule retiré.
    pub(super) label: String,
}

impl LabelRow {
    /// Forme `(mac, ip, label)` attendue par le store de labels ; le numéro
    /// de ligne et la forme brute sont abandonnés.
    pub(super) fn into_tuple(self) -> (String, String, String) {
        (self.mac, self.ip, self.label)
    }
}

/// Erreur « fichier de labels introuvable », pour un CSV comme un classeur.
fn label_file_not_found(csv_path: &str) -> CaptureStateError {
    std::io::Error::new(
        ErrorKind::NotFound,
        format!("fichier de labels introuvable : {csv_path}"),
    )
    .into()
}

/// Lit le fichier en mémoire et rend son texte CSV avec son séparateur de
/// colonnes. Retire le BOM UTF-8 d'Excel ; un chemin inexistant est une
/// erreur explicite, pas un fichier vide. Un classeur `.xlsx` donne sa
/// première feuille, toujours séparée par des virgules.
fn read_label_file(csv_path: &str) -> Result<(String, u8), CaptureStateError> {
    let path = std::path::Path::new(csv_path);
    if lunar_flows_core::xlsx::is_xlsx(path) {
        if !path.exists() {
            return Err(label_file_not_found(csv_path));
        }
        // Séparateur imposé, pas détecté : un label qui contient des
        // points-virgules ne doit pas faire découper la feuille autrement.
        let csv_data =
            lunar_flows_core::xlsx::first_sheet_as_csv(path, LABEL_COLUMNS, ADDRESS_COLUMNS)?;
        return Ok((csv_data, DELIMITERS[0]));
    }
    read_label_text(csv_path).map(|csv_data| {
        let delimiter = detect_delimiter(&csv_data);
        (csv_data, delimiter)
    })
}

/// Texte d'un fichier de labels CSV, BOM retiré.
fn read_label_text(csv_path: &str) -> Result<String, CaptureStateError> {
    match std::fs::read_to_string(csv_path) {
        // Excel écrit un BOM en tête des « CSV UTF-8 » : sans ce retrait il
        // collait à la première MAC, qui devenait invalide.
        Ok(csv_data) => Ok(csv_data
            .strip_prefix('\u{feff}')
            .map(str::to_owned)
            .unwrap_or(csv_data)),
        // Un chemin inexistant était lu comme un fichier vide : l'import
        // vidait alors le store de labels sans un mot. Erreur explicite.
        Err(error) if error.kind() == ErrorKind::NotFound => Err(label_file_not_found(csv_path)),
        Err(error) => Err(error.into()),
    }
}

/// Colonnes d'un fichier de labels : mac, ip, label.
const LABEL_COLUMNS: usize = 3;

/// Colonnes d'adresses en tête d'un fichier de labels : mac, ip. Dans un
/// classeur elles doivent être du texte, un tableur réécrivant les adresses
/// qu'il prend pour des nombres.
const ADDRESS_COLUMNS: usize = 2;

/// Séparateurs de colonnes acceptés. La virgule, forme de référence de
/// l'application (export de labels, store), est en tête : elle gagne à égalité.
const DELIMITERS: &[u8; 3] = b",;\t";

/// Détecte le séparateur de colonnes sur la première ligne non vide : un
/// tableur en locale française exporte en `;`, certains inventaires en
/// tabulation, et ces fichiers étaient refusés comme « format invalide »
/// alors que leur contenu est exploitable. Le séparateur le plus
/// fréquent sur la ligne gagne ; à égalité, le premier de [`DELIMITERS`].
pub(super) fn detect_delimiter(csv_data: &str) -> u8 {
    let Some(line) = csv_data.lines().find(|line| !line.trim().is_empty()) else {
        return DELIMITERS[0];
    };
    DELIMITERS
        .iter()
        .copied()
        .fold((DELIMITERS[0], 0usize), |best, candidate| {
            let count = line.bytes().filter(|byte| *byte == candidate).count();
            if count > best.1 {
                (candidate, count)
            } else {
                best
            }
        })
        .0
}

/// Numéro de ligne d'un enregistrement selon le parser CSV, ou `fallback`
/// (index + 1) s'il n'en porte pas.
fn label_record_line(record: &csv::StringRecord, fallback: usize) -> usize {
    record
        .position()
        .map(|position| position.line() as usize)
        .unwrap_or(fallback)
}

/// Numéro de ligne d'une erreur du parser CSV, ou `fallback` si elle
/// n'est pas localisée.
fn label_error_line(error: &csv::Error, fallback: usize) -> usize {
    error
        .position()
        .map(|position| position.line() as usize)
        .unwrap_or(fallback)
}

/// Rejoint les champs d'un enregistrement par des virgules : forme brute
/// des messages d'erreur, quel que soit le séparateur du fichier.
fn label_record_to_display(record: &csv::StringRecord) -> String {
    record.iter().collect::<Vec<_>>().join(",")
}

/// Vrai si la ligne est l'en-tête d'une matrice de flux exportée
/// (`FlowMatrixRow`) : l'utilisateur s'est trompé de fichier.
fn is_matrix_export_header(record: &csv::StringRecord) -> bool {
    let mut fields = record.iter().map(str::trim);
    fields.next() == Some("mac_source") && fields.next() == Some("mac_destination")
}

/// Résultat du classement d'une ligne CSV lue avec succès par le parser.
enum LabelRecordOutcome {
    /// Ligne entièrement vide (champs blancs) : ignorée sans erreur.
    Empty,
    /// Au moins trois champs : (n° de ligne, enregistrement).
    Valid(usize, csv::StringRecord),
    /// Moins de trois champs : (n° de ligne, ligne affichable) pour le
    /// rapport d'erreur.
    Invalid(usize, String),
}

/// Classe une ligne CSV déjà parsée : vide, valide, invalide (moins de 3
/// champs), ou erreur bloquante immédiate (mauvais type de fichier).
/// Extrait de `read_label_records` pour garder sa complexité cognitive sous
/// le seuil de 15 (16 avec ce classement inline).
fn classify_label_record(
    index: usize,
    record: csv::StringRecord,
    fallback_line: usize,
) -> Result<LabelRecordOutcome, CaptureStateError> {
    let line = label_record_line(&record, fallback_line);
    if record.iter().all(|field| clean_csv_field(field).is_empty()) {
        return Ok(LabelRecordOutcome::Empty);
    }
    // Mauvais type de fichier : une matrice de flux exportée commence par son
    // en-tête `mac_source,mac_destination,…`. Sans cette détection, chacune
    // de ses lignes devenait une « erreur MAC/IP » et l'utilisateur recevait
    // des centaines d'erreurs au lieu d'un diagnostic clair.
    if index == 0 && is_matrix_export_header(&record) {
        return Err(LabelError::InvalidRowsFormat {
            invalid_lines: vec![(
                line,
                "ce fichier est une matrice de flux exportée (colonnes \
                 mac_source, mac_destination…) : importez-le via « Ouvrir \
                 un fichier CSV ». Un fichier de labels contient mac, ip, label."
                    .to_string(),
            )],
        }
        .into());
    }
    if record.len() < LABEL_COLUMNS {
        Ok(LabelRecordOutcome::Invalid(
            line,
            label_record_to_display(&record),
        ))
    } else {
        Ok(LabelRecordOutcome::Valid(line, record))
    }
}

/// Lit et classe toutes les lignes du fichier (séparateur détecté, pas
/// d'en-tête imposé, nombre de champs libre). Retourne les lignes non
/// vides avec leur numéro ; une seule ligne invalide ou illisible met
/// tout le fichier en erreur `InvalidRowsFormat`, lignes fautives listées.
fn read_label_records(
    csv_path: &str,
) -> Result<Vec<(usize, csv::StringRecord)>, CaptureStateError> {
    let (file, delimiter) = read_label_file(csv_path)?;
    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(false)
        .flexible(true)
        .trim(csv::Trim::All)
        .from_reader(file.as_bytes());

    let mut records = Vec::new();
    let mut invalid_lines = Vec::new();

    for (index, result) in rdr.records().enumerate() {
        let fallback_line = index + 1;
        match result {
            Ok(record) => match classify_label_record(index, record, fallback_line)? {
                LabelRecordOutcome::Empty => continue,
                LabelRecordOutcome::Valid(line, record) => records.push((line, record)),
                LabelRecordOutcome::Invalid(line, display) => invalid_lines.push((line, display)),
            },
            Err(error) => {
                invalid_lines.push((label_error_line(&error, fallback_line), error.to_string()))
            }
        }
    }

    if invalid_lines.is_empty() {
        Ok(records)
    } else {
        Err(LabelError::InvalidRowsFormat { invalid_lines }.into())
    }
}

/// Lignes de labels d'un fichier et nombre de lignes sans label ignorées.
pub(super) struct LabelRowsRead {
    /// Lignes exploitables, en-tête éventuel compris.
    pub rows: Vec<LabelRow>,
    /// Lignes désignant un équipement sans lui donner de label, ignorées.
    pub without_label: usize,
}

#[cfg(test)]
pub(super) fn read_label_rows(csv_path: &str) -> Result<Vec<LabelRow>, CaptureStateError> {
    Ok(read_label_rows_report(csv_path)?.rows)
}

/// Lit un fichier de labels et normalise chaque ligne vers les formes du
/// parser de paquets ; les lignes sans label sont comptées plutôt que
/// gardées. Une erreur de format est bloquante, cf.
/// `read_label_records`.
pub(super) fn read_label_rows_report(csv_path: &str) -> Result<LabelRowsRead, CaptureStateError> {
    let mut without_label = 0;
    let rows = read_label_records(csv_path)?
        .into_iter()
        .filter_map(|(line, record)| {
            if label_row_is_blank(record.iter()) {
                without_label += 1;
                return None;
            }
            let raw = label_record_to_display(&record);
            parse_label_fields(record.iter()).map(|(mac, ip, label)| LabelRow {
                line,
                raw,
                // Normalisation vers les formes produites par le parser de
                // paquets : MAC canonique (minuscules, `:` rétablis depuis
                // les écritures `aa-bb-…`, `aabb.ccdd.eeff` ou
                // `aabbccddeeff` des inventaires), IP canonique (IPv6
                // compressée). Sans elle, un label « AA:BB:… » ou « ::0001 »
                // ne correspondait jamais, sans erreur visible. La
                // forme brute reste dans `raw` pour les messages d'erreur ;
                // une MAC non reconnue est gardée telle quelle (en
                // minuscules) pour être rapportée par la validation.
                mac: canonical_mac(&mac).unwrap_or_else(|| mac.to_ascii_lowercase()),
                ip: ip
                    .parse::<IpAddr>()
                    .map(|addr| addr.to_string())
                    .unwrap_or(ip),
                // Retire le préfixe anti-injection de formule posé à
                // l'export : aller-retour sans altération.
                label: crate::state::flow_matrix::unescape_formula_cell(&label),
            })
        })
        .collect();
    Ok(LabelRowsRead {
        rows,
        without_label,
    })
}

/// Vrai si la ligne est l'en-tête de colonnes d'un fichier de labels.
///
/// Une ligne d'en-tête ne peut être que la première du fichier et n'est
/// reconnue que si elle utilise une paire de colonnes explicitement supportée
/// (insensible à la casse). L'ancienne heuristique « champs non parseables »
/// avalait silencieusement une première ligne de données invalide.
pub(super) fn is_header_row(row: &LabelRow) -> bool {
    (row.mac.eq_ignore_ascii_case("mac") && row.ip.eq_ignore_ascii_case("ip"))
        || (row.mac.eq_ignore_ascii_case("adresse_mac")
            && row.ip.eq_ignore_ascii_case("adresse_ip"))
}

#[cfg(test)]
pub(super) fn verif_label_rows_format(file: String) -> Result<(), CaptureStateError> {
    read_label_records(&file)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::super::test_support::TempDir;
    use super::*;
    use std::fs;

    /// Un classeur de labels est lu comme le CSV de mêmes cellules : mêmes
    /// lignes, mêmes numéros, mêmes normalisations, mêmes lignes sans label.
    /// Seule la première feuille compte, et une formule donne sa valeur.
    #[test]
    fn xlsx_label_file_reads_like_the_equivalent_csv() {
        let xlsx = crate::test_support::fixtures().join("20261122_labels.xlsx");
        let from_xlsx = read_label_rows_report(&xlsx.to_string_lossy()).unwrap();

        let dir = TempDir::new("labels_xlsx_equivalent");
        let csv = dir.path().join("labels.csv");
        fs::write(
            &csv,
            "mac,ip,label\n\
             AA-BB-CC-DD-EE-01,10.0.0.1,Automate ligne 1\n\
             aabb.ccdd.ee02,10.0.0.2,\"Poste; supervision, salle A\"\n\
             ,,\n\
             aa:bb:cc:dd:ee:03,10.0.0.3,\n\
             aa:bb:cc:dd:ee:04,2001:0db8:0000:0000:0000:0000:0000:0004,Passerelle\n\
             aa:bb:cc:dd:ee:05,10.0.0.5,Serveur\n\
             aa:bb:cc:dd:ee:06,10.0.0.6,Imprimante,étage 2\n\
             aa:bb:cc:dd:ee:07,10.0.0.7,\"=HYPERLINK(\"\"http://exemple.invalid\"\")\"\n",
        )
        .unwrap();
        let from_csv = read_label_rows_report(&csv.to_string_lossy()).unwrap();

        let fields = |rows: &[LabelRow]| -> Vec<(usize, String, String, String)> {
            rows.iter()
                .map(|row| (row.line, row.mac.clone(), row.ip.clone(), row.label.clone()))
                .collect()
        };
        assert_eq!(fields(&from_xlsx.rows), fields(&from_csv.rows));
        assert_eq!(from_xlsx.without_label, 1);
        assert_eq!(from_csv.without_label, 1);

        let rows = from_xlsx.rows;
        assert!(is_header_row(&rows[0]), "en-tête mac, ip, label reconnu");
        assert_eq!(rows.len(), 7);
        assert_eq!(
            rows[1].mac, "aa:bb:cc:dd:ee:01",
            "MAC d'inventaire normalisée"
        );
        assert_eq!(rows[2].label, "Poste; supervision, salle A");
        assert_eq!((rows[3].line, rows[3].ip.as_str()), (6, "2001:db8::4"));
        assert_eq!(
            rows[4].label, "Serveur",
            "valeur de la formule, pas son texte"
        );
        assert_eq!(
            rows[5].label, "Imprimante étage 2",
            "colonne de plus jointe au label"
        );
        assert!(
            rows.iter().all(|row| !row.label.contains("Feuille 2")),
            "la deuxième feuille n'est pas lue"
        );
        super::super::validation::verif_mac_ip_format_rows(&rows[1..]).unwrap();
    }

    /// Un classeur refusé l'est avec un message qui situe la faute : cellule
    /// en erreur, fichier qui n'est pas un classeur, chemin absent.
    #[test]
    fn xlsx_label_file_errors_are_explicit() {
        let broken = crate::test_support::fixtures().join("20261122_labels_erreur.xlsx");
        let message = read_label_rows(&broken.to_string_lossy())
            .unwrap_err()
            .to_string();
        assert!(
            message.contains("ligne 3 invalide: cellule en erreur (#N/A)"),
            "{message}"
        );

        let dir = TempDir::new("labels_xlsx_errors");
        let fake = dir.path().join("labels.xlsx");
        fs::write(&fake, "mac,ip,label\naa:bb:cc:dd:ee:01,10.0.0.1,PC\n").unwrap();
        let message = read_label_rows(&fake.to_string_lossy())
            .unwrap_err()
            .to_string();
        assert!(message.contains("ouverture XLSX impossible"), "{message}");

        let missing = dir.path().join("absent.xlsx");
        let message = read_label_rows(&missing.to_string_lossy())
            .unwrap_err()
            .to_string();
        assert!(
            message.contains("fichier de labels introuvable"),
            "{message}"
        );
    }

    /// Saisie au tableur : une adresse devenue un nombre est refusée avec sa
    /// ligne et la correction à faire, un label numérique est lu tel
    /// qu'affiché.
    #[test]
    fn xlsx_numeric_address_is_refused_and_numeric_label_is_read() {
        let fixtures = crate::test_support::fixtures();
        let message = read_label_rows(
            &fixtures
                .join("20261122_labels_mac_numerique.xlsx")
                .to_string_lossy(),
        )
        .unwrap_err()
        .to_string();
        assert!(
            message.contains("ligne 3 invalide: la cellule de la colonne 1 n'est pas du texte"),
            "{message}"
        );
        assert!(message.contains("1122334455"), "{message}");
        assert!(message.contains("format Texte"), "{message}");

        let message = read_label_rows(
            &fixtures
                .join("20261122_labels_ip_numerique.xlsx")
                .to_string_lossy(),
        )
        .unwrap_err()
        .to_string();
        assert!(
            message.contains("ligne 2 invalide: la cellule de la colonne 2 n'est pas du texte"),
            "{message}"
        );

        let rows = read_label_rows(
            &fixtures
                .join("20261122_labels_numeriques.xlsx")
                .to_string_lossy(),
        )
        .unwrap();
        let labels: Vec<&str> = rows.iter().skip(1).map(|row| row.label.as_str()).collect();
        assert_eq!(labels, ["42", "1.5"]);
    }

    /// Tableau posé en ligne 4 : l'en-tête est reconnu et les numéros de
    /// ligne restent ceux de la feuille. Tableau décalé en colonne B : rien
    /// n'est recalé, chaque ligne est rapportée avec son numéro de feuille
    /// (la MAC lue à la place de l'IP) et aucun label n'est accepté.
    #[test]
    fn xlsx_table_away_from_a1_keeps_sheet_lines_and_a_shifted_one_is_refused() {
        use super::super::validation::verif_mac_ip_format_rows;
        let fixtures = crate::test_support::fixtures();

        let rows = read_label_rows(
            &fixtures
                .join("20261122_labels_ligne_4.xlsx")
                .to_string_lossy(),
        )
        .unwrap();
        assert!(is_header_row(&rows[0]), "en-tête reconnu en ligne 4");
        let lines: Vec<usize> = rows.iter().map(|row| row.line).collect();
        assert_eq!(lines, [4, 5, 6]);
        verif_mac_ip_format_rows(&rows).unwrap();

        let shifted = read_label_rows(
            &fixtures
                .join("20261122_labels_colonne_b.xlsx")
                .to_string_lossy(),
        )
        .unwrap();
        assert!(!is_header_row(&shifted[0]), "en-tête décalé non reconnu");
        match verif_mac_ip_format_rows(&shifted) {
            Err(CaptureStateError::Label(LabelError::InvalidMacIpFormat {
                invalid_mac,
                invalid_ip,
            })) => {
                assert!(invalid_mac.is_empty());
                let reported: Vec<(usize, &str)> = invalid_ip
                    .iter()
                    .map(|(line, value, _)| (*line, value.as_str()))
                    .collect();
                assert_eq!(
                    reported,
                    [
                        (1, "mac"),
                        (2, "aa:bb:cc:dd:ee:01"),
                        (3, "aa:bb:cc:dd:ee:02")
                    ]
                );
            }
            other => panic!("refus attendu sur les IP, obtenu : {other:?}"),
        }
    }

    /// Chemins avec espaces, Unicode et caractères spéciaux shell :
    /// l'import de labels lit le fichier tel quel (`std::fs`, jamais de
    /// shell ni de concaténation) et les labels Unicode traversent intacts.
    #[test]
    fn label_paths_with_spaces_unicode_and_special_characters_import() {
        let dir = TempDir::new("lunar_test_label_weird_names");
        for name in [
            "labels de l'été (v2).csv",
            "étiquettes 中文 📡.csv",
            "site \"principal\" `usine`.csv",
        ] {
            let path = dir.path().join(name);
            fs::write(
                &path,
                "aa:bb:cc:dd:ee:ff,192.168.1.10,Automate n°1 (atelier été)\n",
            )
            .unwrap();

            let rows = read_label_rows(path.to_str().unwrap())
                .unwrap_or_else(|e| panic!("import de {name}: {e:?}"));
            assert_eq!(rows.len(), 1, "{name}");
            assert_eq!(rows[0].label, "Automate n°1 (atelier été)", "{name}");
        }
    }

    /// Une matrice de flux exportée importée comme fichier de labels doit
    /// produire UNE erreur claire (« mauvais type de fichier »), pas une
    /// erreur MAC/IP par ligne du fichier.
    #[test]
    fn matrix_export_imported_as_labels_gives_one_clear_error() {
        let dir = TempDir::new("lunar_test_label_wrong_kind");
        let path = dir.path().join("matrice.csv");
        fs::write(
            &path,
            "mac_source,mac_destination,vlan_id,protocol_data_link,ip_source\n\
             aa:bb:cc:dd:ee:ff,ff:ff:ff:ff:ff:ff,,ARP,172.24.8.29\n",
        )
        .unwrap();

        let err = read_label_rows(path.to_str().unwrap()).unwrap_err();
        let CaptureStateError::Label(LabelError::InvalidRowsFormat { invalid_lines }) = err else {
            panic!("erreur InvalidRowsFormat attendue, reçu {err:?}");
        };
        assert_eq!(
            invalid_lines.len(),
            1,
            "une seule erreur, pas une par ligne"
        );
        assert!(invalid_lines[0].1.contains("matrice de flux"));
    }

    /// Un export de tableur français sépare les colonnes par `;` : il est
    /// lu comme un fichier `,`, y compris quand le label contient une
    /// virgule, et les MAC Windows/Cisco sont normalisées.
    #[test]
    fn semicolon_separated_file_is_imported() {
        let dir = TempDir::new("lunar_test_label_semicolon");
        let path = dir.path().join("labels.csv");
        fs::write(
            &path,
            "mac;ip;label\n\
             AA-BB-CC-DD-EE-FF;192.168.1.10;Automate, atelier 1\n\
             aabb.ccdd.eeff;192.168.1.11;Superviseur\n",
        )
        .unwrap();

        let rows = read_label_rows(path.to_str().unwrap()).unwrap();
        assert!(is_header_row(&rows[0]), "en-tête reconnu avec `;`");
        assert_eq!(
            rows[1..]
                .iter()
                .cloned()
                .map(LabelRow::into_tuple)
                .collect::<Vec<_>>(),
            vec![
                (
                    "aa:bb:cc:dd:ee:ff".to_string(),
                    "192.168.1.10".to_string(),
                    "Automate, atelier 1".to_string()
                ),
                (
                    "aa:bb:cc:dd:ee:ff".to_string(),
                    "192.168.1.11".to_string(),
                    "Superviseur".to_string()
                ),
            ]
        );
    }

    /// Tabulation et BOM UTF-8 (Excel « CSV UTF-8 ») sont acceptés ;
    /// le BOM ne colle plus à la première MAC.
    #[test]
    fn tab_separated_file_with_bom_is_imported() {
        let dir = TempDir::new("lunar_test_label_tab_bom");
        let path = dir.path().join("labels.tsv");
        fs::write(&path, "\u{feff}aa:bb:cc:dd:ee:ff\t192.168.1.10\tAutomate\n").unwrap();

        let rows = read_label_rows(path.to_str().unwrap()).unwrap();
        assert_eq!(
            rows.into_iter()
                .map(LabelRow::into_tuple)
                .collect::<Vec<_>>(),
            vec![(
                "aa:bb:cc:dd:ee:ff".to_string(),
                "192.168.1.10".to_string(),
                "Automate".to_string()
            )]
        );
    }

    /// Le séparateur est celui qui domine la première ligne non vide ; à
    /// égalité ou en l'absence de séparateur, la virgule (forme de référence).
    #[test]
    fn delimiter_detection_prefers_the_dominant_separator_then_comma() {
        assert_eq!(detect_delimiter("mac;ip;label, suite\n"), b';');
        assert_eq!(detect_delimiter("mac,ip,label; suite\n"), b',');
        assert_eq!(
            detect_delimiter("mac,ip,label;a;b\n"),
            b',',
            "égalité → virgule"
        );
        assert_eq!(
            detect_delimiter("\n\n  \nmac\tip\tlabel\n"),
            b'\t',
            "lignes vides ignorées"
        );
        assert_eq!(detect_delimiter(""), b',');
        assert_eq!(detect_delimiter("pas-de-separateur\n"), b',');
    }

    /// Un fichier `,` dont un label contient `;` reste lu en `,` : le `;`
    /// traverse dans le label (pas de régression sur la forme de référence).
    #[test]
    fn comma_file_with_semicolon_inside_label_is_unchanged() {
        let dir = TempDir::new("lunar_test_label_comma_semicolon");
        let path = dir.path().join("labels.csv");
        fs::write(&path, "aa:bb:cc:dd:ee:ff,192.168.1.10,Automate; zone A\n").unwrap();

        let rows = read_label_rows(path.to_str().unwrap()).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].label, "Automate; zone A");
    }

    #[test]
    fn valid_csv_format_returns_ok() {
        let dir = TempDir::new("lunar_test_valid_csv_format");
        let file_path = dir.path().join("labels.csv");
        fs::write(&file_path, "aa:bb:cc:dd:ee:ff,192.168.1.1,mon-pc\n").unwrap();

        let result = verif_label_rows_format(file_path.to_str().unwrap().to_string());

        assert!(result.is_ok());
    }

    /// Un chemin inexistant est une erreur explicite : il était lu comme un
    /// fichier vide et l'import vidait le store de labels.
    #[test]
    fn missing_file_is_an_explicit_error() {
        let result = read_label_rows("/chemin/qui/n/existe/pas/labels.csv");
        match result.unwrap_err() {
            CaptureStateError::Io(e) => {
                assert_eq!(e.kind(), std::io::ErrorKind::NotFound);
                assert!(e.to_string().contains("introuvable"), "{e}");
            }
            error => panic!("erreur inattendue: {error:?}"),
        }
    }

    /// L'en-tête n'est reconnu que s'il dit littéralement `mac,ip` : une
    /// première ligne de données invalide n'est plus avalée en silence.
    #[test]
    fn header_detection_is_strict() {
        let header = LabelRow {
            line: 1,
            raw: "MAC,IP,Label".into(),
            mac: "MAC".into(),
            ip: "IP".into(),
            label: "Label".into(),
        };
        assert!(is_header_row(&header));

        let invalid_data = LabelRow {
            line: 1,
            raw: "pas-une-mac,pas-une-ip,label".into(),
            mac: "pas-une-mac".into(),
            ip: "pas-une-ip".into(),
            label: "label".into(),
        };
        assert!(
            !is_header_row(&invalid_data),
            "une ligne invalide n'est pas un en-tête"
        );
    }

    /// MAC/IP non canoniques (casse, forme IPv6 étendue) sont normalisées à
    /// l'import vers les formes du parser : le label s'applique.
    #[test]
    fn labels_are_normalized_to_parser_forms() {
        let dir = TempDir::new("lunar_test_label_normalization");
        let file_path = dir.path().join("labels.csv");
        fs::write(
            &file_path,
            "AA:BB:CC:DD:EE:FF,192.168.1.1,pc-majuscules\n00:11:22:33:44:55,2001:0db8:0000:0000:0000:0000:0000:0001,ipv6-étendue\n",
        )
        .unwrap();

        let rows = read_label_rows(file_path.to_str().unwrap()).unwrap();

        assert_eq!(rows[0].mac, "aa:bb:cc:dd:ee:ff", "MAC en minuscules");
        assert_eq!(rows[1].ip, "2001:db8::1", "IPv6 compressée canonique");
    }

    /// Les inventaires exportés écrivent souvent la MAC sans séparateur
    /// (`aabbccddeeff`), avec des tirets (Windows) ou des points (Cisco) :
    /// ces écritures sont ramenées à la forme du parser au lieu d'être
    /// rejetées comme « MAC invalide ».
    #[test]
    fn mac_without_or_with_other_separators_is_canonicalized() {
        let dir = TempDir::new("lunar_test_label_mac_separators");
        let file_path = dir.path().join("labels.csv");
        fs::write(
            &file_path,
            "aabbccddeeff,192.168.1.1,sans-separateur\n\
             AABBCCDDEEFF,192.168.1.2,sans-separateur-majuscules\n\
             AA-BB-CC-DD-EE-FF,192.168.1.3,tirets-windows\n\
             aabb.ccdd.eeff,192.168.1.4,points-cisco\n",
        )
        .unwrap();

        let rows = read_label_rows(file_path.to_str().unwrap()).unwrap();

        assert_eq!(rows.len(), 4);
        for row in &rows {
            assert_eq!(
                row.mac, "aa:bb:cc:dd:ee:ff",
                "ligne {} : {}",
                row.line, row.raw
            );
        }
        assert_eq!(
            rows[0].raw, "aabbccddeeff,192.168.1.1,sans-separateur",
            "forme brute conservée"
        );
    }

    #[test]
    fn missing_column_returns_invalid_file_format_error() {
        let dir = TempDir::new("lunar_test_missing_column");
        let file_path = dir.path().join("labels.csv");
        fs::write(&file_path, "192.168.1.1,mon-pc\n").unwrap();

        let result = verif_label_rows_format(file_path.to_str().unwrap().to_string());

        match result.unwrap_err() {
            CaptureStateError::Label(LabelError::InvalidRowsFormat { invalid_lines }) => {
                assert_eq!(invalid_lines, vec![(1, "192.168.1.1,mon-pc".to_string())]);
            }
            error => panic!("erreur inattendue: {error:?}"),
        }
    }

    #[test]
    fn extra_column_is_merged_into_label() {
        let dir = TempDir::new("lunar_test_extra_column");
        let file_path = dir.path().join("labels.csv");
        fs::write(
            &file_path,
            "aa:bb:cc:dd:ee:ff,192.168.1.1,mon-pc,réseau-2\n",
        )
        .unwrap();

        let result = verif_label_rows_format(file_path.to_str().unwrap().to_string());
        assert!(result.is_ok());

        let rows = read_label_rows(file_path.to_str().unwrap()).unwrap();
        assert_eq!(
            rows.into_iter()
                .map(LabelRow::into_tuple)
                .collect::<Vec<_>>(),
            vec![(
                "aa:bb:cc:dd:ee:ff".to_string(),
                "192.168.1.1".to_string(),
                "mon-pc réseau-2".to_string()
            )]
        );
    }

    #[test]
    fn empty_file_returns_ok() {
        let dir = TempDir::new("lunar_test_empty_file");
        let file_path = dir.path().join("labels.csv");
        fs::write(&file_path, "").unwrap();

        let result = verif_label_rows_format(file_path.to_str().unwrap().to_string());

        assert!(result.is_ok());
    }
}
