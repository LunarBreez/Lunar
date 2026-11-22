//! Lecture des matrices de flux au format XLSX (même schéma LFMS que le CSV).
//!
//! Le classeur est converti en CSV en mémoire puis repassé au désérialiseur et
//! à la validation de [`crate::csv`] : CSV et XLSX appliquent ainsi exactement
//! le même schéma, la même validation stricte et les mêmes numéros de ligne.

use std::path::Path;

use std::collections::BTreeMap;

use calamine::{Data, DataType, Reader, Xlsx, open_workbook};
use packet_parser::LinkType;

use crate::lfms;
use crate::matrix::FlowMatrixRow;
use crate::{LunarCoreError, Result};

/// Vrai si le chemin porte l'extension `.xlsx` (insensible à la casse).
pub fn is_xlsx(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("xlsx"))
}

/// Erreur « CSV invalide » pour le classeur `path` : un XLSX refusé est
/// rapporté exactement comme un CSV refusé, le consommateur ne distingue pas.
fn invalid(path: &Path, message: impl Into<String>) -> LunarCoreError {
    LunarCoreError::InvalidCsv {
        path: path.to_path_buf(),
        message: message.into(),
    }
}

/// Convertit une cellule Excel en texte CSV. Les dates réellement typées par
/// Excel sont normalisées dans le format LFMS accepté par `last_seen` ; les
/// autres cellules gardent leur représentation textuelle.
fn cell_text(cell: &Data, header: &str) -> String {
    if header == "last_seen"
        && let Some(date) = cell.as_datetime()
    {
        return date.format("%Y-%m-%d %H:%M:%S%.6f UTC").to_string();
    }
    cell.to_string()
}

/// Dernière ligne lisible d'une feuille (index à partir de 0) : la limite
/// d'Excel, 1 048 576 lignes.
const MAX_ROW_INDEX: u32 = 1_048_575;

/// Colonnes lues par ligne. Une matrice LFMS en a dix-neuf, un fichier de
/// labels trois ; au-delà de cette borne le classeur est refusé.
const MAX_COLUMNS: u32 = 256;

/// Cellules gardées en mémoire pour une feuille, cellules vides entre deux
/// valeurs d'une même ligne comprises.
const MAX_CELLS: usize = 8_000_000;

/// Première feuille d'un classeur : ses lignes non vides, par numéro de
/// ligne croissant (index à partir de 0), chacune jusqu'à sa dernière
/// cellule renseignée.
///
/// Les cellules sont lues une à une et rangées par ligne, sans passer par la
/// plage rectangulaire de calamine : celle-ci réserve lignes × colonnes
/// cellules entre la première et la dernière cellule utilisée, si bien qu'un
/// classeur de quelques centaines d'octets déclarant `A1` et `XFD1048576`
/// demandait des centaines de gigaoctets. Ici la mémoire suit ce que le
/// fichier contient, dans les bornes ci-dessus.
struct Sheet {
    /// Lignes non vides, par index de ligne.
    rows: BTreeMap<u32, Vec<Data>>,
}

impl Sheet {
    /// Lignes non vides, dans l'ordre de la feuille.
    fn rows(&self) -> impl Iterator<Item = &[Data]> {
        self.rows.values().map(Vec::as_slice)
    }
}

/// Charge la première feuille du classeur.
fn first_sheet(path: &Path) -> Result<Sheet> {
    let mut workbook: Xlsx<_> = open_workbook(path)
        .map_err(|error| invalid(path, format!("ouverture XLSX impossible: {error}")))?;
    let name = workbook
        .sheet_names()
        .into_iter()
        .next()
        .ok_or_else(|| invalid(path, "le classeur XLSX ne contient aucune feuille"))?;
    let unreadable =
        |error: &dyn std::fmt::Display| invalid(path, format!("lecture XLSX impossible: {error}"));
    let mut cells = workbook
        .worksheet_cells_reader(&name)
        .map_err(|error| unreadable(&error))?;

    let mut rows: BTreeMap<u32, Vec<Data>> = BTreeMap::new();
    let mut kept = 0usize;
    while let Some(cell) = cells.next_cell().map_err(|error| unreadable(&error))? {
        let (row, column) = cell.get_position();
        let value = Data::from(cell.get_value().clone());
        if value.is_empty() {
            continue;
        }
        if row > MAX_ROW_INDEX {
            return Err(invalid(
                path,
                format!(
                    "cellule en ligne {} : au-delà de la dernière ligne d'une feuille",
                    u64::from(row) + 1
                ),
            ));
        }
        if column >= MAX_COLUMNS {
            return Err(invalid(
                path,
                format!(
                    "ligne {} : cellule en colonne {}, au-delà des {MAX_COLUMNS} colonnes lues",
                    row + 1,
                    u64::from(column) + 1
                ),
            ));
        }
        let column = column as usize;
        let cells_of_row = rows.entry(row).or_default();
        if column >= cells_of_row.len() {
            kept += column + 1 - cells_of_row.len();
            if kept > MAX_CELLS {
                return Err(invalid(
                    path,
                    format!("feuille trop grande : plus de {MAX_CELLS} cellules"),
                ));
            }
            cells_of_row.resize(column + 1, Data::Empty);
        }
        cells_of_row[column] = value;
    }
    Ok(Sheet { rows })
}

/// Première feuille du classeur rendue en texte CSV (virgules, champs
/// protégés), pour repasser par un lecteur CSV existant : c'est ainsi qu'un
/// fichier de labels `.xlsx` suit exactement les contrôles de son équivalent
/// `.csv`.
///
/// La ligne `n` du texte est la ligne `n` de la feuille : les lignes vides
/// sont gardées (vides) et un saut de ligne dans une cellule devient une
/// espace, si bien que les numéros de ligne des erreurs désignent la feuille.
/// Une cellule de formule donne sa dernière valeur calculée, jamais la
/// formule : rien n'est évalué ici. Une cellule en erreur (`#N/A`,
/// `#DIV/0!`…) fait refuser le classeur, avec sa ligne.
///
/// Une ligne non vide porte au moins `min_columns` champs : dans une feuille,
/// une cellule laissée vide en fin de ligne (label à compléter) n'est pas
/// une colonne manquante, contrairement à une ligne CSV trop courte.
///
/// Les `text_columns` premières colonnes portent des identifiants (adresses)
/// et doivent être des cellules de texte : un tableur qui a pris
/// `001122334455` pour un nombre en a retiré les zéros de tête, et
/// `123456789E03` est devenu `123456789000`, une autre adresse tout aussi
/// plausible. Une telle cellule fait refuser le classeur, avec sa ligne, sa
/// colonne et la correction à faire, plutôt que de lire une valeur réécrite.
/// Au-delà, une cellule numérique est lue telle qu'affichée.
pub fn first_sheet_as_csv(path: &Path, min_columns: usize, text_columns: usize) -> Result<String> {
    let range = first_sheet(path)?;
    let failed = |error: &dyn std::fmt::Display| {
        invalid(path, format!("conversion XLSX impossible: {error}"))
    };
    let mut csv_data = Vec::new();
    {
        let mut writer = csv::WriterBuilder::new()
            .flexible(true)
            .from_writer(&mut csv_data);
        // Seules les lignes non vides sont en mémoire : les lignes vides qui
        // les précèdent sont restituées pour garder la numérotation.
        let mut next_line = 1usize;
        for (index, row) in &range.rows {
            let line = *index as usize + 1;
            for _ in next_line..line {
                writer.write_record([""]).map_err(|error| failed(&error))?;
            }
            next_line = line + 1;
            if let Some(error) = row.iter().find(|cell| cell.is_error()) {
                return Err(invalid(
                    path,
                    format!("ligne {line} invalide: cellule en erreur ({error})"),
                ));
            }
            if let Some((column, cell)) = row
                .iter()
                .enumerate()
                .take(text_columns)
                .find(|(_, cell)| !cell.is_empty() && !cell.is_string())
            {
                return Err(invalid(
                    path,
                    format!(
                        "ligne {line} invalide: la cellule de la colonne {} n'est pas du \
                         texte (valeur lue : {cell}). Un tableur réécrit une adresse saisie \
                         comme un nombre (zéros de tête retirés) : mettre la colonne au \
                         format Texte, puis ressaisir l'adresse",
                        column + 1
                    ),
                ));
            }
            let used = row
                .iter()
                .rposition(|cell| !cell.is_empty())
                .map_or(0, |last| last + 1);
            let padding = min_columns.saturating_sub(used);
            let record = row[..used]
                .iter()
                .map(|cell| cell.to_string().replace(['\r', '\n'], " "))
                .chain(std::iter::repeat_n(String::new(), padding));
            writer
                .write_record(record)
                .map_err(|error| failed(&error))?;
        }
        writer.flush().map_err(|error| failed(&error))?;
    }
    String::from_utf8(csv_data).map_err(|error| failed(&error))
}

/// Type de liaison annoncé par le préambule `#LFMS` du classeur (Ethernet
/// implicite pour un export antérieur au préambule).
fn range_link_type(path: &Path, range: &Sheet) -> Result<(LinkType, usize)> {
    let first = range
        .rows()
        .next()
        .ok_or_else(|| invalid(path, "la première feuille XLSX est vide"))?;
    let first_cell = first.first().map(ToString::to_string).unwrap_or_default();
    match lfms::parse_preamble(&first_cell).map_err(|message| invalid(path, message))? {
        Some(preamble) => Ok((preamble.link_type.unwrap_or(LinkType::ETHERNET), 1)),
        None => Ok((LinkType::ETHERNET, 0)),
    }
}

/// Type de liaison d'un classeur, sans désérialiser ses lignes.
pub fn matrix_file_link_type(path: &Path) -> Result<LinkType> {
    let range = first_sheet(path)?;
    let (link_type, _) = range_link_type(path, &range)?;
    lfms::ensure_reimportable(path, link_type)?;
    Ok(link_type)
}

/// Lit toutes les lignes d'une matrice XLSX, entièrement validée : la première
/// ligne invalide interrompt la lecture avec son numéro de ligne (numérotation
/// de la feuille, préambule compris).
pub fn read_matrix_rows(path: &Path) -> Result<Vec<FlowMatrixRow>> {
    let range = first_sheet(path)?;
    let (link_type, header_index) = range_link_type(path, &range)?;
    lfms::ensure_reimportable(path, link_type)?;

    let non_empty_rows: Vec<&[Data]> = range.rows().collect();
    let header_row = non_empty_rows
        .get(header_index)
        .ok_or_else(|| invalid(path, "en-tête de matrice XLSX absent"))?;
    let headers: Vec<String> = header_row
        .iter()
        .map(|cell| cell.to_string().trim().to_string())
        .collect();

    let mut csv_data = Vec::new();
    {
        let mut writer = csv::WriterBuilder::new().from_writer(&mut csv_data);
        writer
            .write_record(&headers)
            .map_err(|error| invalid(path, format!("conversion XLSX impossible: {error}")))?;
        for row in non_empty_rows.iter().skip(header_index + 1) {
            let record = headers.iter().enumerate().map(|(column, header)| {
                row.get(column)
                    .map(|cell| cell_text(cell, header))
                    .unwrap_or_default()
            });
            writer
                .write_record(record)
                .map_err(|error| invalid(path, format!("conversion XLSX impossible: {error}")))?;
        }
        writer
            .flush()
            .map_err(|error| invalid(path, format!("conversion XLSX impossible: {error}")))?;
    }

    // Numéro de ligne réel dans la feuille : la première ligne de données suit
    // l'en-tête, lui-même précédé du préambule s'il est présent.
    let first_data_line = header_index + 2;
    let mut reader = csv::ReaderBuilder::new()
        .trim(csv::Trim::All)
        .from_reader(csv_data.as_slice());
    let mut rows = Vec::new();
    for (index, result) in reader.deserialize::<FlowMatrixRow>().enumerate() {
        let row = result.map_err(|error| {
            invalid(
                path,
                format!("ligne {} invalide: {error}", index + first_data_line),
            )
        })?;
        row.validate(link_type).map_err(|error| {
            invalid(
                path,
                format!("ligne {} invalide: {error}", index + first_data_line),
            )
        })?;
        rows.push(row);
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures")
            .join(name)
    }

    /// Le test central : XLSX et CSV décrivent le MÊME relevé. La fixture
    /// `.xlsx` est générée depuis le `.csv` par
    /// `script/matrix/generate-xlsx-fixture.py` — si les deux lectures
    /// divergeaient, un opérateur obtiendrait deux matrices différentes pour
    /// un même contenu selon le format qu'il a sous la main.
    #[test]
    fn xlsx_and_csv_yield_identical_rows() {
        let from_csv =
            crate::csv::read_matrix_rows(&fixture("20260703_NP_Matrice.csv")).expect("lecture CSV");
        let from_xlsx =
            read_matrix_rows(&fixture("20260703_NP_Matrice.xlsx")).expect("lecture XLSX");

        assert!(!from_csv.is_empty(), "la fixture porte des lignes");
        assert_eq!(
            from_xlsx.len(),
            from_csv.len(),
            "même nombre de lignes dans les deux formats"
        );
        assert_eq!(
            from_xlsx, from_csv,
            "chaque champ doit être identique entre XLSX et CSV"
        );
    }

    /// L'aiguillage de `crate::csv::read_matrix_rows` reconnaît le classeur :
    /// un appelant n'a pas à savoir quel format il tient.
    #[test]
    fn the_csv_entry_point_dispatches_to_xlsx() {
        let rows = crate::csv::read_matrix_rows(&fixture("20260703_NP_Matrice.xlsx"))
            .expect("aiguillage vers le lecteur XLSX");
        assert!(!rows.is_empty());
    }

    /// La feuille de labels de test rendue en CSV : une ligne de texte par
    /// ligne de feuille (la ligne vide reste), valeurs des formules et non
    /// leur texte, deuxième feuille ignorée.
    #[test]
    fn first_sheet_as_csv_keeps_sheet_line_numbers_and_cached_values() {
        let text = first_sheet_as_csv(&fixture("20261122_labels.xlsx"), 3, 2).expect("conversion");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 9, "{text}");
        assert_eq!(lines[0], "mac,ip,label");
        assert_eq!(
            lines[2],
            "aabb.ccdd.ee02,10.0.0.2,\"Poste; supervision, salle A\""
        );
        assert_eq!(lines[3], "\"\"", "ligne vide de la feuille gardée");
        assert_eq!(
            lines[4], "aa:bb:cc:dd:ee:03,10.0.0.3,",
            "label vide : colonne gardée"
        );
        assert_eq!(lines[6], "aa:bb:cc:dd:ee:05,10.0.0.5,Serveur");
        assert!(!text.contains("CONCATENATE"), "la formule n'est pas lue");
        assert!(
            !text.contains("Feuille 2"),
            "seule la première feuille est lue"
        );
    }

    /// Un classeur minuscule qui déclare deux cellules aux deux bouts d'une
    /// feuille est refusé sur sa colonne, sans réserver la plage qui les
    /// sépare ; même refus, typé, pour une cellule hors feuille. Les deux
    /// lecteurs (matrice, labels) passent par la même garde.
    #[test]
    fn far_apart_cells_are_refused_without_allocating_the_range_between() {
        let far = fixture("20261122_classeur_cellule_lointaine.xlsx");
        for message in [
            first_sheet_as_csv(&far, 3, 2)
                .expect_err("labels")
                .to_string(),
            read_matrix_rows(&far).expect_err("matrice").to_string(),
            matrix_file_link_type(&far).expect_err("DLT").to_string(),
        ] {
            assert!(
                message.contains(
                    "ligne 1048576 : cellule en colonne 16384, au-delà des 256 colonnes lues"
                ),
                "{message}"
            );
        }

        let below = fixture("20261122_classeur_hors_feuille.xlsx");
        let message = first_sheet_as_csv(&below, 3, 2)
            .expect_err("hors feuille")
            .to_string();
        assert!(
            message.contains("cellule en ligne 1048577 : au-delà de la dernière ligne"),
            "{message}"
        );
    }

    /// Une adresse en cellule numérique est refusée avec sa ligne, sa colonne
    /// et la correction à faire ; hors des colonnes d'adresses, un nombre est
    /// lu tel qu'affiché.
    #[test]
    fn numeric_cells_are_refused_in_text_columns_and_read_elsewhere() {
        let mac = first_sheet_as_csv(&fixture("20261122_labels_mac_numerique.xlsx"), 3, 2)
            .expect_err("MAC numérique")
            .to_string();
        assert!(
            mac.contains(
                "ligne 3 invalide: la cellule de la colonne 1 n'est pas du texte \
                 (valeur lue : 1122334455)"
            ),
            "{mac}"
        );
        assert!(mac.contains("format Texte"), "{mac}");

        let ip = first_sheet_as_csv(&fixture("20261122_labels_ip_numerique.xlsx"), 3, 2)
            .expect_err("IP numérique")
            .to_string();
        assert!(
            ip.contains("ligne 2 invalide: la cellule de la colonne 2 n'est pas du texte"),
            "{ip}"
        );

        let text = first_sheet_as_csv(&fixture("20261122_labels_numeriques.xlsx"), 3, 2)
            .expect("labels numériques");
        assert_eq!(
            text,
            "mac,ip,label\naa:bb:cc:dd:ee:01,10.0.0.1,42\naa:bb:cc:dd:ee:02,10.0.0.2,1.5\n"
        );
        // Sans colonne imposée en texte, la même feuille est lue.
        let unchecked = first_sheet_as_csv(&fixture("20261122_labels_mac_numerique.xlsx"), 3, 0)
            .expect("aucune colonne de texte imposée");
        assert!(unchecked.contains("1122334455,10.0.0.2,Automate ligne 2"));
    }

    /// Un tableau posé plus bas ou plus à droite garde les positions de la
    /// feuille : lignes vides restituées au-dessus, colonne vide restituée à
    /// gauche. Rien n'est recalé en silence.
    #[test]
    fn a_table_away_from_a1_keeps_its_sheet_positions() {
        let lower = first_sheet_as_csv(&fixture("20261122_labels_ligne_4.xlsx"), 3, 2)
            .expect("tableau en ligne 4");
        let lines: Vec<&str> = lower.lines().collect();
        assert_eq!(lines.len(), 6, "{lower}");
        assert_eq!(&lines[..3], ["\"\"", "\"\"", "\"\""]);
        assert_eq!(lines[3], "mac,ip,label");
        assert_eq!(lines[5], "aa:bb:cc:dd:ee:02,10.0.0.2,Automate ligne 2");

        let shifted = first_sheet_as_csv(&fixture("20261122_labels_colonne_b.xlsx"), 3, 2)
            .expect("tableau en colonne B");
        assert_eq!(
            shifted.lines().next(),
            Some(",mac,ip,label"),
            "colonne A vide gardée"
        );
    }

    #[test]
    fn is_xlsx_ignores_case_and_other_extensions() {
        assert!(is_xlsx(Path::new("releve.xlsx")));
        assert!(is_xlsx(Path::new("RELEVE.XLSX")));
        assert!(!is_xlsx(Path::new("releve.csv")));
        assert!(!is_xlsx(Path::new("releve")));
    }

    /// Le type de liaison se lit sans désérialiser les lignes, et il vient du
    /// préambule `#LFMS` comme pour le CSV.
    #[test]
    fn link_type_comes_from_the_preamble() {
        let from_xlsx =
            matrix_file_link_type(&fixture("20260703_NP_Matrice.xlsx")).expect("DLT du classeur");
        let from_csv =
            lfms::matrix_file_link_type(&fixture("20260703_NP_Matrice.csv")).expect("DLT du CSV");
        assert_eq!(from_xlsx, from_csv);
    }

    /// Un fichier qui n'est pas un classeur échoue proprement, avec son
    /// chemin dans le message — jamais un panic ni une matrice vide silencieuse.
    #[test]
    fn a_non_workbook_file_fails_with_its_path() {
        let error = read_matrix_rows(&fixture("20260703_NP_Matrice.csv"))
            .expect_err("un CSV n'est pas un classeur XLSX");
        assert!(
            error.to_string().contains("20260703_NP_Matrice.csv"),
            "le message doit nommer le fichier, obtenu : {error}"
        );
    }
}
