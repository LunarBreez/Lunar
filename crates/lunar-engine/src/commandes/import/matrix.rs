//! Import des matrices de flux CSV ou XLSX (schéma de
//! `FlowMatrix::export_to_csv`) : lecture, fusion multi-fichiers avec
//! provenance, reconstruction de la matrice et du graphe.

use log::{error, info, warn};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::ipc::Channel;

use lunar_flows_core::csv::{apply_row_labels, merge_rows};

use crate::{
    errors::{CaptureStateError, import::PcapImportError},
    events::CaptureEvent,
    state::{
        capture::CaptureState,
        flow_matrix::{FlowMatrix, FlowMatrixRow, unescape_formula_cell},
        graph::GraphData,
        labels_list::LabelStore,
    },
};

use super::{event_channel, labels::copy_labels_to_matrix};

/// Lignes lues, groupées par fichier source (chemin tel que fourni par l'appelant).
type MatrixRowsByFile = Vec<(String, Vec<FlowMatrixRow>)>;
/// Résultat d'une lecture multi-fichiers : DLT commun à tous les fichiers et lignes par fichier.
type MatrixBatch = (packet_parser::LinkType, MatrixRowsByFile);

/// Vrai si la matrice de flux courante n'a aucune ligne. Sert à activer les
/// exports ; seule erreur possible : verrou empoisonné.
pub fn is_matrix_empty(state: &Arc<Mutex<FlowMatrix>>) -> Result<bool, CaptureStateError> {
    Ok(state.lock()?.matrix.is_empty())
}

/// Lit une matrice de flux (format de `FlowMatrix::export_to_csv`),
/// entièrement validé avant de toucher à l'état — lecture et validation
/// stricte. La production passe par
/// `read_matrix_rows_per_file` ; ce raccourci ne sert plus qu'aux tests.
#[cfg(test)]
fn read_matrix_rows(path: &str) -> Result<Vec<FlowMatrixRow>, CaptureStateError> {
    Ok(read_matrix_file(Path::new(path))?.1)
}

/// Lit un fichier de matrice selon son extension, CSV ou XLSX, entièrement
/// par le cœur partagé : `lunar_flows_core::csv::read_matrix_rows` aiguille
/// un classeur vers `lunar_flows_core::xlsx`. Retourne le DLT du préambule et
/// les lignes validées.
///
/// Classeurs et CSV suivent les mêmes règles : le refus d'un DLT sans
/// projection LFMS (`ensure_reimportable`) et le retrait du préfixe
/// anti-formule s'appliquent aussi aux classeurs. Le classeur est ouvert deux
/// fois (type de liaison, puis lignes), le cœur n'exposant pas les deux d'un
/// coup.
fn read_matrix_file(
    path: &Path,
) -> Result<(packet_parser::LinkType, Vec<FlowMatrixRow>), CaptureStateError> {
    let link_type = if lunar_flows_core::xlsx::is_xlsx(path) {
        lunar_flows_core::xlsx::matrix_file_link_type(path)?
    } else {
        lunar_flows_core::lfms::matrix_file_link_type(path)?
    };
    Ok((link_type, lunar_flows_core::csv::read_matrix_rows(path)?))
}

/// Lit chaque fichier de matrice et retourne ses lignes groupées par fichier,
/// pour la comptabilité par fichier (événements `Finished`). La lecture et
/// l'héritage de la colonne `origin` viennent du cœur partagé. Le callback est
/// appelé après validation complète de chaque fichier pour publier sa
/// progression sans exposer de lignes partielles.
fn read_matrix_rows_per_file(
    incoming_file_paths: &[String],
    mut on_file_read: impl FnMut(usize, &str, usize),
) -> Result<MatrixBatch, CaptureStateError> {
    if incoming_file_paths.is_empty() {
        return Err(
            PcapImportError::MissingInput("l'import de matrice CSV/XLSX".to_string()).into(),
        );
    }

    let mut files = Vec::with_capacity(incoming_file_paths.len());
    let mut common_link_type = None;
    for (index, path) in incoming_file_paths.iter().enumerate() {
        let path_buf = PathBuf::from(path);
        let origin = lunar_flows_core::csv::origin_name_from_path(&path_buf);
        let (link_type, mut rows) = read_matrix_file(&path_buf)?;
        match common_link_type {
            None => common_link_type = Some(link_type),
            Some(expected) if expected == link_type => {}
            Some(expected) => {
                return Err(lunar_flows_core::LunarCoreError::MixedLinkTypes {
                    path: path_buf,
                    found: lunar_flows_core::lfms::link_type_name(link_type),
                    expected: lunar_flows_core::lfms::link_type_name(expected),
                }
                .into());
            }
        }
        for row in &mut rows {
            if row.origin.trim().is_empty() {
                row.origin = origin.clone();
            }
        }
        on_file_read(index, path, rows.len());
        files.push((path.clone(), rows));
    }
    let link_type = common_link_type
        .ok_or_else(|| PcapImportError::MissingInput("l'import de matrice CSV/XLSX".to_string()))?;
    Ok((link_type, files))
}

// La production passe par `read_matrix_rows_per_file` (comptabilité par
// fichier) ; cette version aplatie ne sert plus qu'aux tests.
#[cfg(test)]
fn read_matrix_rows_from_files(
    incoming_file_paths: &[String],
) -> Result<Vec<FlowMatrixRow>, CaptureStateError> {
    Ok(
        read_matrix_rows_per_file(incoming_file_paths, |_, _, _| {})?
            .1
            .into_iter()
            .flat_map(|(_, rows)| rows)
            .collect(),
    )
}

/// Contexte de relevé commun à des fichiers de matrice.
///
/// Le contexte du préambule `#LFMS` décrit **un** point de mesure. Fusionner
/// des relevés qui n'ont pas le même — deux switchs, deux armoires — ne peut
/// donc pas produire un contexte unique honnête : on retourne alors un
/// contexte vide plutôt que d'élire arbitrairement celui du premier fichier
/// et d'étiqueter tout le reste avec.
///
/// Attribuer chaque flux à sa provenance demande le contexte **par ligne**
/// (généralisation d'`origin`), qui n'est pas encore livré. Jusque-là, un
/// contexte vide est la seule réponse qui ne ment pas.
fn common_survey_context(paths: &[String]) -> lunar_flows_core::lfms::SurveyContext {
    let mut common: Option<lunar_flows_core::lfms::SurveyContext> = None;
    for path in paths {
        let context = lunar_flows_core::lfms::read_preamble(std::path::Path::new(path))
            .ok()
            .flatten()
            .map(|preamble| preamble.context)
            .unwrap_or_default();
        match &common {
            None => common = Some(context),
            Some(expected) if *expected == context => {}
            Some(_) => {
                warn!(
                    "[import_matrix_files] contextes de relevé divergents entre fichiers : \
                     le relevé fusionné reste non qualifié"
                );
                return lunar_flows_core::lfms::SurveyContext::default();
            }
        }
    }
    common.unwrap_or_default()
}

/// Reconstruit `matrice` et `graph` à partir de lignes déjà validées (les deux
/// sont vidés d'abord) : labels du store puis ceux des fichiers, fusion des
/// flux par le cœur, puis une arête de graphe par ligne avec ses labels et
/// ses identifiants d'encapsulation.
fn rebuild_matrix_and_graph_from_rows(
    rows: &[FlowMatrixRow],
    link_type: packet_parser::LinkType,
    label_store: &LabelStore,
    matrice: &mut FlowMatrix,
    graph: &mut GraphData,
) -> Result<(), CaptureStateError> {
    matrice.clear();
    graph.clear();
    matrice.link_type = Some(link_type);

    // Labels du store courant, puis ceux portés par les fichiers (prioritaires
    // à clé égale puisque appliqués après) ; fusion des flux tunnel par tunnel
    // et provenance accumulée par le cœur partagé.
    copy_labels_to_matrix(label_store, matrice)?;
    apply_row_labels(matrice, rows);
    merge_rows(matrice, rows, link_type)?;

    for row in rows {
        let (flow, tunnel_rows) = row
            .to_flow_and_rows(link_type)
            .map_err(lunar_flows_core::LunarCoreError::InvalidMatrixRow)?;
        let encap_ids: Vec<u64> = tunnel_rows.iter().filter_map(|(id, _)| *id).collect();

        let source_label = matrice.get_label(&row.mac_source, &row.ip_source);
        let destination_label = matrice.get_label(&row.mac_destination, &row.ip_destination);
        graph.add_packet_flow(
            &flow,
            source_label,
            destination_label,
            row.count,
            row.total_bytes,
            &encap_ids,
        );
    }

    Ok(())
}

/// Variante à un seul fichier de `import_matrix_files`, mêmes garanties et
/// mêmes erreurs. Exposée par la façade.
pub fn import_matrix_file(
    incoming_file_path: String,
    matrice: &Arc<Mutex<FlowMatrix>>,
    graph: &Arc<Mutex<GraphData>>,
    label_store: &Arc<Mutex<LabelStore>>,
    capture_state: &Arc<Mutex<CaptureState>>,
    on_event: Channel<CaptureEvent>,
) -> Result<(), CaptureStateError> {
    import_matrix_files(
        vec![incoming_file_path],
        matrice,
        graph,
        label_store,
        capture_state,
        on_event,
    )
}

/// Remplace le relevé courant par la fusion des matrices CSV/XLSX données.
/// Appelée par le panneau d'import et par l'ouverture
/// de projet. Transactionnelle : labels, matrice et graphe sont construits
/// en local et ne remplacent l'état partagé qu'en cas de succès complet,
/// puis `Started`, `GraphSnapshot` et `Finished` sont émis sur le channel.
/// Erreurs : liste vide (`MissingInput`), capture en cours (phase
/// indisponible), fichier illisible ou invalide, DLT différents entre
/// fichiers (`MixedLinkTypes`) — l'état courant est alors intact.
pub fn import_matrix_files(
    incoming_file_paths: Vec<String>,
    matrice: &Arc<Mutex<FlowMatrix>>,
    graph: &Arc<Mutex<GraphData>>,
    label_store: &Arc<Mutex<LabelStore>>,
    capture_state: &Arc<Mutex<CaptureState>>,
    on_event: Channel<CaptureEvent>,
) -> Result<(), CaptureStateError> {
    // Refus avant la réservation, la sélection du channel, la lecture d'un
    // label et tout événement : une liste vide ne doit jamais vider la
    // session courante.
    if incoming_file_paths.is_empty() {
        return Err(
            PcapImportError::MissingInput("l'import de matrice CSV/XLSX".to_string()).into(),
        );
    }

    // Import et capture sont mutuellement exclusifs : l'import remplacerait
    // la matrice et le graphe pendant que le pipeline les alimente. La phase
    // `Importing` est réservée atomiquement et détenue jusqu'à la fin de la
    // commande, swap inclus.
    // Même garantie que pour les PCAP : une matrice fusionnée deux fois
    // doublerait ses compteurs et dupliquerait sa provenance.
    let incoming_file_paths = super::dedupe_paths_by_identity(&incoming_file_paths);

    let import_guard =
        crate::state::capture::ImportGuard::acquire(capture_state, "import de matrice CSV/XLSX")?;
    let on_event = event_channel(capture_state, on_event)?;

    info!(
        "[import_matrix_files] COMMAND CALLED avec {} fichier(s): {:?}",
        incoming_file_paths.len(),
        incoming_file_paths
    );

    // Un fichier invalide ne doit pas effacer la matrice courante. Les
    // fichiers doivent porter le même DLT (préambule #LFMS, Ethernet
    // implicite pour un export antérieur) : fusion inter-DLT refusée.
    let files_total = incoming_file_paths.len();
    let (link_type, files) =
        read_matrix_rows_per_file(&incoming_file_paths, |index, path, line_count| {
            if let Err(e) = on_event.send(CaptureEvent::ImportProgress {
                file_name: path.to_string(),
                file_index: index + 1,
                files_total,
                current: line_count,
                total: line_count,
            }) {
                error!("Erreur lors de l'envoi de ImportProgress: {:?}", e);
            }
        })?;

    // Même principe que l'import PCAP (`send_started_event`) : un import de
    // matrice CSV/XLSX est aussi une session, avec sa propre version de contrat
    // d'événements — sans `Started`, le frontend ne pourrait ni afficher le
    // DLT ni détecter une dérive de version pour cette voie d'import.
    if let Err(e) = on_event.send(CaptureEvent::Started {
        session_id: 0,
        device: String::new(),
        buffer_size: 0,
        chan_capacity: 0,
        timeout: 0,
        snaplen: 0,
        link_type: lunar_flows_core::lfms::link_type_name(link_type),
        protocol_version: crate::events::CAPTURE_EVENT_PROTOCOL_VERSION,
    }) {
        error!("Erreur lors de l'envoi de Started: {:?}", e);
    }

    let line_counts: Vec<(String, usize)> = files
        .iter()
        .map(|(path, rows)| (path.clone(), rows.len()))
        .collect();
    let survey_period = lunar_flows_core::csv::survey_period_of_files(&files)?;
    let rows: Vec<FlowMatrixRow> = files.into_iter().flat_map(|(_, rows)| rows).collect();

    // Snapshot transactionnel local : labels, matrice et graphe sont bâtis
    // sans toucher à l'état partagé. Ils seront remplacés ensemble seulement
    // si la réservation qui a lu ces fichiers est encore courante.
    let mut new_label_store = LabelStore {
        rows: label_store.lock()?.get().clone(),
    };

    // Les labels portés par les fichiers entrent dans le store — source de
    // vérité unique, fichier prioritaire à clé égale : ils survivent
    // aux resynchronisations déclenchées par les mutations ultérieures.
    for row in &rows {
        for (mac, ip, label) in [
            (&row.mac_source, &row.ip_source, &row.label_source),
            (
                &row.mac_destination,
                &row.ip_destination,
                &row.label_destination,
            ),
        ] {
            if let Some(label) = label.as_ref().filter(|l| !l.is_empty()) {
                new_label_store.set(mac, ip, &unescape_formula_cell(label));
            }
        }
    }

    let mut new_matrix = FlowMatrix::new();
    let mut new_graph = GraphData::new();
    // Le contexte du relevé vient des fichiers importés, jamais de la session
    // précédente : `new_matrix` part d'un contexte vide, donc importer
    // ne peut pas rhabiller un relevé avec la qualification de l'ancien.
    new_matrix.context = common_survey_context(&incoming_file_paths);
    rebuild_matrix_and_graph_from_rows(
        &rows,
        link_type,
        &new_label_store,
        &mut new_matrix,
        &mut new_graph,
    )?;
    // Même principe pour la période : elle vient des fichiers
    // (réunion de celles qu'ils déclarent), et devient inconnue si l'un
    // d'eux porte des lignes sans la déclarer. Posée APRÈS la reconstruction,
    // qui vide la matrice (`clear` remet la période à zéro).
    new_matrix.period = survey_period;

    import_guard.verify_current("commit de l'import de matrice")?;

    // Le commit est certain à partir d'ici (le swap est une pure
    // affectation) : relevé marqué modifié avant de prendre les verrous de
    // données, `CaptureState` ne s'imbrique jamais avec eux.
    capture_state.lock()?.mark_dirty();

    // Même ordre de verrouillage que convert_from_pcap_list et net_capture
    // (matrice -> graph -> label_store) pour éviter un interblocage ABBA.
    let mut matrice_guard = matrice.lock()?;
    let mut graph_guard = graph.lock()?;
    let mut label_store_guard = label_store.lock()?;
    *matrice_guard = new_matrix;
    *graph_guard = new_graph;
    *label_store_guard = new_label_store;

    info!(
        "[import_matrix_files] {} fichier(s), {} ligne(s) importée(s) -> {} flux fusionné(s), {} nœuds, {} arêtes",
        incoming_file_paths.len(),
        rows.len(),
        matrice_guard.row_count(),
        graph_guard.nodes.len(),
        graph_guard.edges.len()
    );

    let snapshot = graph_guard.get_all_graph_data();
    if let Err(e) = on_event.send(CaptureEvent::GraphSnapshot {
        graph_data: snapshot,
    }) {
        error!("Erreur lors de l'envoi de GraphSnapshot: {:?}", e);
    }

    // Mêmes événements que l'import PCAP, pour que la barre de statut mette à
    // jour ses compteurs : un `Finished` par fichier (lignes lues, total de
    // flux fusionnés), puis un `Stats` final qui fixe les totaux affichés
    // (📥 = lignes lues au total, 📊 = flux de la matrice fusionnée).
    let matrix_total_count = matrice_guard.row_count();
    for (path, line_count) in &line_counts {
        if let Err(e) = on_event.send(CaptureEvent::Finished {
            file_name: path.clone(),
            packet_total_count: *line_count,
            // Une matrice CSV/XLSX est validée ligne à ligne avant import : une
            // ligne invalide est fatale, donc tout ce qui est lu est
            // intégré — les trois catégories de rejet sont structurellement
            // nulles pour ce chemin.
            integrated_count: *line_count,
            rejected_truncated_count: 0,
            rejected_unsupported_link_type_count: 0,
            rejected_malformed_count: 0,
            matrix_total_count,
        }) {
            error!("Erreur lors de l'envoi de Finished: {:?}", e);
        }
    }
    if let Err(e) = on_event.send(CaptureEvent::Stats {
        session_id: 0,
        received: rows.len() as u32,
        dropped: 0,
        if_dropped: 0,
        app_dropped: 0,
        rejected_truncated: 0,
        rejected_unsupported_link_type: 0,
        rejected_malformed: 0,
        processed: matrix_total_count as u32,
    }) {
        error!("Erreur lors de l'envoi de Stats: {:?}", e);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::pcap::handle_pcap_file;
    use super::super::test_support::{TempDir, tunnel_pcap};
    use super::*;
    use std::fs;
    use std::io::Write;

    /// Lettre(s) de colonne d'un tableur (`0` → `A`, `26` → `AA`).
    fn column_letters(mut index: usize) -> String {
        let mut letters = Vec::new();
        loop {
            letters.push(b'A' + (index % 26) as u8);
            if index < 26 {
                break;
            }
            index = index / 26 - 1;
        }
        letters.reverse();
        String::from_utf8(letters).unwrap()
    }

    /// Échappement XML minimal du texte d'une cellule.
    fn xml_text(value: &str) -> String {
        value
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    }

    /// Écrit un classeur XLSX minimal (une feuille, cellules en chaînes
    /// en ligne). Remplace la `calamine::Range` construite en mémoire par le
    /// test d'origine : le lecteur du cœur ne prend qu'un chemin de fichier.
    fn write_minimal_xlsx(path: &Path, rows: &[Vec<String>]) {
        let mut sheet = String::from(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>"#,
        );
        for (row_index, row) in rows.iter().enumerate() {
            sheet.push_str(&format!(r#"<row r="{}">"#, row_index + 1));
            for (column, value) in row.iter().enumerate() {
                sheet.push_str(&format!(
                    r#"<c r="{}{}" t="inlineStr"><is><t xml:space="preserve">{}</t></is></c>"#,
                    column_letters(column),
                    row_index + 1,
                    xml_text(value)
                ));
            }
            sheet.push_str("</row>");
        }
        sheet.push_str("</sheetData></worksheet>");

        let entries = [
            (
                "[Content_Types].xml",
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#.to_string(),
            ),
            (
                "_rels/.rels",
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#.to_string(),
            ),
            (
                "xl/workbook.xml",
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Feuil1" sheetId="1" r:id="rId1"/></sheets></workbook>"#.to_string(),
            ),
            (
                "xl/_rels/workbook.xml.rels",
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#.to_string(),
            ),
            ("xl/worksheets/sheet1.xml", sheet),
        ];
        let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
        for (name, content) in entries {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn new_matrix_is_empty() {
        let matrix = FlowMatrix::new();
        assert!(matrix.matrix.is_empty());
    }

    #[test]
    fn xlsx_range_accepts_an_extra_column_in_the_middle() {
        let source_path = crate::test_support::fixtures().join("20260703_NP_Matrice.csv");
        let expected = lunar_flows_core::csv::read_matrix_rows(&source_path)
            .unwrap()
            .into_iter()
            .next()
            .unwrap();

        // Réutilise la sérialisation canonique pour obtenir les en-têtes et
        // insère une vraie colonne cliente au milieu de la feuille.
        let mut serialized = Vec::new();
        {
            let mut writer = csv::Writer::from_writer(&mut serialized);
            writer.serialize(&expected).unwrap();
            writer.flush().unwrap();
        }
        let mut reader = csv::Reader::from_reader(serialized.as_slice());
        let mut headers: Vec<String> = reader
            .headers()
            .unwrap()
            .iter()
            .map(str::to_string)
            .collect();
        let mut values: Vec<String> = reader
            .records()
            .next()
            .unwrap()
            .unwrap()
            .iter()
            .map(str::to_string)
            .collect();
        headers.insert(6, "commentaire_client".to_string());
        values.insert(6, "ajouté dans Excel".to_string());

        // Le lecteur XLSX est celui du cœur : la feuille passe par un vrai
        // classeur sur le disque, lu par le même chemin que l'import.
        let dir = TempDir::new("lunar_test_matrix_xlsx_extra_column");
        let workbook = dir.path().join("client.xlsx");
        write_minimal_xlsx(&workbook, &[headers, values]);

        let (link_type, rows) = read_matrix_file(&workbook).unwrap();
        assert_eq!(link_type, packet_parser::LinkType::ETHERNET);
        assert_eq!(rows, vec![expected]);
    }

    /// Reproduit la construction de `import_matrix_file` (labels puis flux).
    fn build_matrix_and_graph(rows: &[FlowMatrixRow]) -> (FlowMatrix, GraphData) {
        build_matrix_and_graph_for_link_type(rows, packet_parser::LinkType::ETHERNET)
    }

    fn build_matrix_and_graph_for_link_type(
        rows: &[FlowMatrixRow],
        link_type: packet_parser::LinkType,
    ) -> (FlowMatrix, GraphData) {
        let mut matrix = FlowMatrix::new();
        let mut graph = GraphData::new();
        let label_store = LabelStore::new();
        rebuild_matrix_and_graph_from_rows(rows, link_type, &label_store, &mut matrix, &mut graph)
            .unwrap();

        (matrix, graph)
    }

    /// L'import transmet le DLT du préambule à la matrice ET au
    /// graphe : un CSV SLL ne doit jamais être reconstruit comme Ethernet.
    #[test]
    fn sll_matrix_rebuild_keeps_typed_keys_and_graph() {
        let rows = read_matrix_rows(
            crate::test_support::fixtures()
                .join("20260703_NP_Matrice.csv")
                .to_str()
                .unwrap(),
        )
        .unwrap();
        let mut row = rows
            .into_iter()
            .find(|row| {
                !row.ip_source.is_empty()
                    && !row.ip_destination.is_empty()
                    && lunar_flows_core::link::cooked_protocol_from_text(&row.protocol_data_link)
                        .is_some()
            })
            .expect("la fixture contient un flux IP");
        row.mac_destination.clear();
        row.vlan_id = None;

        let (matrix, graph) =
            build_matrix_and_graph_for_link_type(&[row], packet_parser::LinkType::LINUX_SLL);

        assert_eq!(matrix.link_type, Some(packet_parser::LinkType::LINUX_SLL));
        assert!(
            matrix
                .matrix
                .keys()
                .all(|flow| { flow.data_link.link_type() == packet_parser::LinkType::LINUX_SLL })
        );
        assert!(!graph.nodes.is_empty());
        assert!(!graph.edges.is_empty());
    }

    #[test]
    fn import_matrix_rows_merges_duplicate_flows() {
        let mut rows = read_matrix_rows(
            crate::test_support::fixtures()
                .join("20260703_NP_Matrice.csv")
                .to_str()
                .unwrap(),
        )
        .unwrap();

        let mut duplicate = rows[0].clone();
        duplicate.count = 7;
        duplicate.total_bytes = 1234;
        duplicate.last_seen = "2099-01-01 00:00:00".to_string();
        rows.push(duplicate);

        let (flow, _tunnel_rows) = rows[0]
            .to_flow_and_rows(packet_parser::LinkType::ETHERNET)
            .unwrap();
        let (matrix, graph) = build_matrix_and_graph(&rows);
        let entries = matrix.matrix.get(&flow).unwrap();
        let merged_count: u64 = entries.iter().map(|(_, stats)| stats.count).sum();
        let merged_bytes: u64 = entries.iter().map(|(_, stats)| stats.total_bytes).sum();

        assert_eq!(
            matrix.row_count(),
            30,
            "le flux dupliqué doit être fusionné"
        );
        assert_eq!(merged_count, rows[0].count + 7);
        assert_eq!(merged_bytes, rows[0].total_bytes.saturating_add(1234));
        assert!(
            graph
                .edges
                .values()
                .any(|edge| edge.count >= rows[0].count + 7),
            "le graphe doit recevoir le trafic fusionné"
        );
    }

    /// Deux fichiers portant le même flux : la colonne `origin` de la ligne
    /// fusionnée doit contenir les deux noms de fichiers (triés, joints par `|`).
    #[test]
    fn import_matrix_records_origin_files_per_row() {
        let source = crate::test_support::fixtures().join("20260703_NP_Matrice.csv");
        let dir = TempDir::new("lunar_test_matrix_origin");
        let file_a = dir.path().join("site-a.csv");
        let file_b = dir.path().join("site-b.csv");
        fs::copy(&source, &file_a).unwrap();
        fs::copy(&source, &file_b).unwrap();

        let rows = read_matrix_rows_from_files(&[
            file_a.to_str().unwrap().to_string(),
            file_b.to_str().unwrap().to_string(),
        ])
        .unwrap();
        let (matrix, _graph) = build_matrix_and_graph(&rows);

        // Deux copies du même fichier : chaque flux est vu dans les deux, donc
        // fusionné (pas dupliqué) et sa colonne `origin` porte les deux noms.
        assert_eq!(matrix.row_count(), 30, "flux fusionnés, pas dupliqués");
        let exported = matrix.to_flat_vec();
        assert!(
            exported.iter().all(|r| r.origin == "site-a.csv|site-b.csv"),
            "chaque ligne doit porter ses deux fichiers d'origine: {:?}",
            exported
                .iter()
                .map(|r| r.origin.clone())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn matrix_read_reports_each_validated_file() {
        let source = crate::test_support::fixtures().join("20260703_NP_Matrice.csv");
        let dir = TempDir::new("lunar_test_matrix_progress");
        let file_a = dir.path().join("site-a.csv");
        let file_b = dir.path().join("site-b.csv");
        fs::copy(&source, &file_a).unwrap();
        fs::copy(&source, &file_b).unwrap();
        let paths = vec![
            file_a.to_str().unwrap().to_string(),
            file_b.to_str().unwrap().to_string(),
        ];
        let mut progress = Vec::new();

        let (_link_type, files) = read_matrix_rows_per_file(&paths, |index, path, rows| {
            progress.push((index, path.to_string(), rows));
        })
        .unwrap();

        assert_eq!(files.len(), 2);
        assert_eq!(progress.len(), 2);
        assert_eq!(progress[0].0, 0);
        assert_eq!(progress[0].1, paths[0]);
        assert_eq!(progress[0].2, files[0].1.len());
        assert_eq!(progress[1].0, 1);
        assert_eq!(progress[1].1, paths[1]);
        assert_eq!(progress[1].2, files[1].1.len());
    }

    /// Une matrice déjà fusionnée (colonne `origin` renseignée) réimportée sous
    /// un nouveau nom conserve sa provenance d'origine plutôt que de l'écraser.
    #[test]
    fn reimport_preserves_existing_origin_column() {
        let source = crate::test_support::fixtures().join("20260703_NP_Matrice.csv");
        let dir = TempDir::new("lunar_test_matrix_origin_preserve");

        // Étape 1 : import sous "brut.csv" (origin = brut.csv) puis export.
        let brut = dir.path().join("brut.csv");
        fs::copy(&source, &brut).unwrap();
        let rows = read_matrix_rows_from_files(&[brut.to_str().unwrap().to_string()]).unwrap();
        let (matrix, _graph) = build_matrix_and_graph(&rows);
        let merged = dir.path().join("fusion.csv");
        FlowMatrix::write_rows_to_csv(
            &matrix.to_flat_vec(),
            matrix.link_type,
            merged.to_str().unwrap(),
        )
        .unwrap();

        // Étape 2 : réimport de fusion.csv -> l'origine "brut.csv" est préservée.
        let rows = read_matrix_rows_from_files(&[merged.to_str().unwrap().to_string()]).unwrap();
        let (matrix, _graph) = build_matrix_and_graph(&rows);
        let exported = matrix.to_flat_vec();
        assert!(
            exported.iter().all(|r| r.origin == "brut.csv"),
            "l'origine héritée doit être conservée, pas remplacée par fusion.csv: {:?}",
            exported
                .iter()
                .map(|r| r.origin.clone())
                .collect::<Vec<_>>()
        );
    }

    /// Deux fichiers dont la colonne `origin` est **déjà remplie** (matrices
    /// issues de fusions antérieures) : les provenances des deux colonnes sont
    /// fusionnées, et le nom des fichiers physiques importés n'est PAS ajouté.
    #[test]
    fn reimport_merges_existing_origin_columns() {
        let source = crate::test_support::fixtures().join("20260703_NP_Matrice.csv");
        let dir = TempDir::new("lunar_test_matrix_origin_merge");

        // Prépare deux matrices déjà « étiquetées » par des origines distinctes,
        // en réexportant le même fichier source importé sous deux noms.
        let export_with_origin = |raw_name: &str, out_name: &str| {
            let raw = dir.path().join(raw_name);
            fs::copy(&source, &raw).unwrap();
            let rows = read_matrix_rows_from_files(&[raw.to_str().unwrap().to_string()]).unwrap();
            let (matrix, _graph) = build_matrix_and_graph(&rows);
            let out = dir.path().join(out_name);
            FlowMatrix::write_rows_to_csv(
                &matrix.to_flat_vec(),
                matrix.link_type,
                out.to_str().unwrap(),
            )
            .unwrap();
            out.to_str().unwrap().to_string()
        };
        let fusion_a = export_with_origin("a-raw.csv", "fusion-a.csv");
        let fusion_b = export_with_origin("b-raw.csv", "fusion-b.csv");

        // Réimport des deux matrices déjà renseignées (origin = "a-raw.csv" et
        // "b-raw.csv") : chaque flux partagé doit porter les DEUX provenances,
        // sans trace de "fusion-a.csv"/"fusion-b.csv".
        let rows = read_matrix_rows_from_files(&[fusion_a, fusion_b]).unwrap();
        let (matrix, _graph) = build_matrix_and_graph(&rows);
        let exported = matrix.to_flat_vec();
        assert!(
            exported.iter().all(|r| r.origin == "a-raw.csv|b-raw.csv"),
            "les colonnes origin doivent être fusionnées, sans les fichiers de fusion: {:?}",
            exported
                .iter()
                .map(|r| r.origin.clone())
                .collect::<Vec<_>>()
        );
    }

    /// Aller-retour complet sur le pcap réel : import PCAP -> export CSV ->
    /// réimport -> réexport. Les lignes (flux + ventilation par tunnel)
    /// doivent survivre au cycle à l'identique.
    #[test]
    fn pcap_matrix_survives_csv_roundtrip() {
        let pcap_path = tunnel_pcap();
        let mut matrix = FlowMatrix::new();
        let mut graph = GraphData::new();
        let on_event = Channel::new(|_| {});
        handle_pcap_file(
            pcap_path.to_str().unwrap(),
            1,
            1,
            &mut matrix,
            &mut graph,
            &on_event,
            &mut None,
            &std::sync::atomic::AtomicBool::new(false),
        )
        .unwrap();

        let dir = TempDir::new("lunar_test_matrix_roundtrip");
        let csv_path = dir.path().join("matrice.csv");
        FlowMatrix::write_rows_to_csv(
            &matrix.to_flat_vec(),
            matrix.link_type,
            csv_path.to_str().unwrap(),
        )
        .unwrap();

        let rows = read_matrix_rows(csv_path.to_str().unwrap()).unwrap();
        assert_eq!(rows.len(), matrix.row_count(), "une ligne CSV par flux");

        let (reimported, _graph) = build_matrix_and_graph(&rows);
        let normalize = |m: &FlowMatrix| {
            let mut v: Vec<String> = m
                .to_flat_vec()
                .into_iter()
                .map(|r| {
                    format!(
                        "{}|{}|{:?}|{:?}|{}|{}|{}",
                        r.mac_source,
                        r.mac_destination,
                        r.port_source,
                        r.port_destination,
                        r.count,
                        r.total_bytes,
                        r.encap_id
                    )
                })
                .collect();
            v.sort();
            v
        };
        assert_eq!(
            normalize(&matrix),
            normalize(&reimported),
            "compteurs et ventilation par tunnel identiques après réimport"
        );
    }

    /// Rejoue la chaîne de `import_matrix_file` sur le fichier réel de
    /// `fixtures/` : lecture CSV, reconstruction des flux, remplissage de la
    /// matrice et du graphe.
    #[test]
    fn import_real_matrix_file_rebuilds_matrix_and_graph() {
        let matrix_path = crate::test_support::fixtures()
            .join("20260703_NP_Matrice.csv")
            .to_str()
            .unwrap()
            .to_string();

        let rows = read_matrix_rows(&matrix_path).unwrap();
        assert_eq!(
            rows.len(),
            30,
            "30 lignes de données (l'en-tête est écarté)"
        );

        let (matrix, graph) = build_matrix_and_graph(&rows);

        assert_eq!(matrix.row_count(), 30, "un flux par ligne du fichier");
        assert!(!graph.nodes.is_empty(), "le graphe doit contenir des nœuds");
        assert!(
            !graph.edges.is_empty(),
            "le graphe doit contenir des arêtes"
        );

        // Les labels du fichier sont réappliqués sur les nœuds du graphe.
        let labelled = graph
            .nodes
            .values()
            .filter(|n| n.label.as_deref() == Some("pc lunar"))
            .count();
        assert!(
            labelled > 0,
            "au moins un nœud doit porter le label du fichier"
        );

        // Le CSV survit à un aller-retour export -> import (mêmes flux).
        let reexported = matrix.to_flat_vec();
        assert_eq!(reexported.len(), 30);
    }

    /// Même chaîne sur la matrice générée de 1000 lignes : vérifie la tenue
    /// en charge du parseur et la construction d'un graphe complet.
    #[test]
    fn import_1000_row_matrix_builds_full_graph() {
        let matrix_path = crate::test_support::fixtures()
            .join("20260703_NP_Matrice_1000.csv")
            .to_str()
            .unwrap()
            .to_string();

        let rows = read_matrix_rows(&matrix_path).unwrap();
        assert_eq!(rows.len(), 1000);

        let (matrix, graph) = build_matrix_and_graph(&rows);

        assert_eq!(matrix.row_count(), 1000, "1000 flux distincts");
        assert_eq!(graph.nodes.len(), 232, "un nœud par adresse IP distincte");
        assert!(graph.edges.len() > 100, "arêtes: {}", graph.edges.len());

        // Les poids de trafic du fichier sont reportés sur les arêtes.
        assert!(
            graph
                .edges
                .values()
                .all(|e| e.count > 0 && e.total_bytes > 0),
            "chaque arête doit porter son trafic cumulé"
        );

        let labelled = graph.nodes.values().filter(|n| n.label.is_some()).count();
        assert!(labelled >= 20, "nœuds labellisés: {labelled}");
    }

    /// Même garantie que la voie PCAP : l'appel avec `[]` échoue avant tout
    /// événement, lecture de labels ou remplacement partagé. Joué par la
    /// commande, avec l'erreur sous sa forme sérialisée et un canal qui
    /// enregistre tout.
    #[test]
    fn matrix_command_refuses_empty_list_without_touching_session() {
        let source = crate::test_support::fixtures().join("20260703_NP_Matrice.csv");
        let rows = read_matrix_rows(source.to_str().unwrap()).unwrap();
        let (seeded_matrix, seeded_graph) = build_matrix_and_graph(&rows);
        let mut seeded_labels = LabelStore::new();
        seeded_labels.add((
            "aa:bb:cc:dd:ee:ff".to_string(),
            "198.51.100.9".to_string(),
            "label sentinelle".to_string(),
        ));
        let mut seeded_capture_state = CaptureState::new();
        seeded_capture_state.session_id = 73;
        seeded_capture_state.filter = Some("udp".to_string());
        seeded_capture_state.config.device_name = "matrice-sentinelle".to_string();

        let matrix = Arc::new(Mutex::new(seeded_matrix));
        let graph = Arc::new(Mutex::new(seeded_graph));
        let labels = Arc::new(Mutex::new(seeded_labels));
        let capture_state = Arc::new(Mutex::new(seeded_capture_state));
        let before_matrix = matrix.lock().unwrap().to_flat_vec();
        let before_graph = serde_json::to_value(&*graph.lock().unwrap()).unwrap();
        let before_labels = labels.lock().unwrap().rows.clone();
        let before_config = serde_json::to_value(&capture_state.lock().unwrap().config).unwrap();
        let (on_event, events) = crate::ipc::test_support::json_recorder();

        let error = serde_json::to_value(
            import_matrix_files(vec![], &matrix, &graph, &labels, &capture_state, on_event)
                .expect_err("la liste de matrices vide doit être refusée"),
        )
        .unwrap();

        assert_eq!(error["kind"], "import");
        assert_eq!(error["message"]["kind"], "missingInput");
        assert!(events.lock().unwrap().is_empty(), "aucun événement émis");
        assert_eq!(matrix.lock().unwrap().to_flat_vec(), before_matrix);
        assert_eq!(
            serde_json::to_value(&*graph.lock().unwrap()).unwrap(),
            before_graph
        );
        assert_eq!(labels.lock().unwrap().rows, before_labels);
        let locked_state = capture_state.lock().unwrap();
        assert_eq!(
            locked_state.phase,
            crate::state::capture::capture_status::CapturePhase::Idle
        );
        assert_eq!(locked_state.session_id, 73);
        assert_eq!(locked_state.filter.as_deref(), Some("udp"));
        assert_eq!(
            serde_json::to_value(&locked_state.config).unwrap(),
            before_config
        );
    }

    /// Le contexte d'un relevé importé vient du fichier, et de lui seul.
    /// Un import ne doit jamais rhabiller les données entrantes avec
    /// la qualification de la session précédente.
    #[test]
    fn survey_context_comes_from_the_imported_file() {
        let dir = TempDir::new("import_context");
        let path = dir.path().join("relevé switch 7.csv");
        let context = lunar_flows_core::lfms::SurveyContext {
            site: Some("Usine Nord".to_string()),
            sensor: Some("PC de relevé".to_string()),
            interface: Some("eth0".to_string()),
        };
        FlowMatrix::write_rows_to_csv_with_context(
            &[],
            Some(packet_parser::LinkType::ETHERNET),
            &context,
            None,
            &path.to_string_lossy(),
        )
        .unwrap();

        let read = common_survey_context(&[path.to_string_lossy().into_owned()]);
        assert_eq!(read, context, "le contexte du préambule doit être rétabli");
    }

    /// Fusionner deux relevés de points de mesure différents ne peut pas
    /// produire un contexte unique honnête : le résultat reste non qualifié
    /// plutôt que d'hériter arbitrairement du premier fichier.
    ///
    /// Attribuer chaque flux à sa provenance demande le contexte par ligne,
    /// pas encore livré.
    #[test]
    fn merging_two_measurement_points_yields_no_context() {
        let dir = TempDir::new("import_context_conflict");
        let mut paths = Vec::new();
        for (index, site) in ["Armoire B3", "Armoire C1"].iter().enumerate() {
            let path = dir.path().join(format!("relevé {index}.csv"));
            let context = lunar_flows_core::lfms::SurveyContext {
                site: Some((*site).to_string()),
                sensor: Some("PC de relevé".to_string()),
                interface: Some("eth0".to_string()),
            };
            FlowMatrix::write_rows_to_csv_with_context(
                &[],
                Some(packet_parser::LinkType::ETHERNET),
                &context,
                None,
                &path.to_string_lossy(),
            )
            .unwrap();
            paths.push(path.to_string_lossy().into_owned());
        }

        assert!(
            common_survey_context(&paths).is_empty(),
            "des contextes divergents doivent donner un relevé non qualifié, \
             pas celui du premier fichier"
        );
    }

    /// Deux fichiers du même point de mesure gardent leur contexte : la règle
    /// ci-dessus ne doit pas punir une fusion légitime (un même relevé
    /// découpé en plusieurs exports).
    #[test]
    fn merging_the_same_measurement_point_keeps_the_context() {
        let dir = TempDir::new("import_context_same");
        let context = lunar_flows_core::lfms::SurveyContext {
            site: Some("Armoire B3".to_string()),
            sensor: Some("PC de relevé".to_string()),
            interface: Some("eth0".to_string()),
        };
        let mut paths = Vec::new();
        for index in 0..2 {
            let path = dir.path().join(format!("part {index}.csv"));
            FlowMatrix::write_rows_to_csv_with_context(
                &[],
                Some(packet_parser::LinkType::ETHERNET),
                &context,
                None,
                &path.to_string_lossy(),
            )
            .unwrap();
            paths.push(path.to_string_lossy().into_owned());
        }

        assert_eq!(common_survey_context(&paths), context);
    }
}
