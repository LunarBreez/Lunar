//! Export de synthèse depuis la vue d'investigation : une page HTML inerte,
//! lisible sans LUNAR, et la matrice CSV qu'elle résume, écrite ici juste à côté. La page porte
//! l'empreinte SHA-256 de ce CSV, pour que son destinataire vérifie qu'elle
//! correspond bien au relevé.
//!
//! La page elle-même est composée par l'interface, à partir du graphe renvoyé
//! ici : matrice et graphe sont lus sous le même verrou, si bien que la
//! synthèse et le CSV décrivent exactement le même état, même pendant une
//! capture.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};

use crate::{
    commandes::flow_matrix::SurveyPeriodView,
    errors::{CaptureStateError, export::ExportError},
    state::{
        flow_matrix::{FlowMatrix, FlowMatrixRow},
        graph::GraphData,
    },
};
use lunar_flows_core::lfms::{SurveyContext, link_type_name};

/// Ce que l'interface reçoit pour composer la page : le graphe de l'instant
/// de l'export et l'identité du CSV écrit à côté. Le graphe est sérialisé
/// comme par `get_graph_snapshot`.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SynthesisExport {
    /// Graphe complet à l'instant de l'export (nœuds = endpoints, arêtes = flux).
    pub graph: GraphData,
    /// Nom du CSV seul, sans dossier : la page ne divulgue pas
    /// l'arborescence du poste de relevé.
    pub csv_file_name: String,
    /// SHA-256 du CSV tel qu'écrit sur le disque, en hexadécimal minuscule.
    pub csv_sha256: String,
    /// Taille du CSV écrit, en octets.
    pub csv_bytes: u64,
    /// Type de liaison du relevé (`ETHERNET`…), comme dans le préambule LFMS.
    pub link_type: Option<String>,
    /// Site du relevé, s'il a été renseigné.
    pub site: Option<String>,
    /// Capteur (poste de relevé), s'il a été renseigné.
    pub sensor: Option<String>,
    /// Interface écoutée, si elle a été renseignée.
    pub interface: Option<String>,
    /// Version de LUNAR qui a produit le relevé exporté.
    pub app_version: String,
    /// Période observée du relevé, écrite aussi dans le préambule du CSV.
    pub period: SurveyPeriodView,
}

/// Empreinte et taille du CSV écrit.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct WrittenCsv {
    /// Chemin du CSV écrit.
    pub path: PathBuf,
    /// SHA-256 du contenu écrit, en hexadécimal minuscule.
    pub sha256: String,
    /// Taille du CSV, en octets.
    pub bytes: u64,
}

/// Chemin du CSV associé à la page : même nom, extension `.csv`. Refuse une
/// cible qui n'est pas une page HTML, et un CSV existant que l'utilisateur
/// n'a pas accepté d'écraser : le sélecteur de fichiers ne confirme que
/// l'écrasement de la page, il ne voit pas son voisin.
pub(crate) fn sibling_csv_path(html_path: &Path) -> Result<PathBuf, ExportError> {
    if html_path.as_os_str().is_empty() {
        return Err(ExportError::EmptyPath);
    }
    let is_html = html_path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("html") || extension.eq_ignore_ascii_case("htm")
        });
    if !is_html {
        return Err(ExportError::NotHtml);
    }
    let csv_path = html_path.with_extension("csv");
    // Page déjà présente : son écrasement a été confirmé dans le sélecteur,
    // la paire est remplacée d'un bloc. Page absente : un CSV homonyme est
    // un autre export, on ne l'écrase pas en silence.
    if csv_path.exists() && !html_path.exists() {
        return Err(ExportError::CsvAlreadyExists(
            csv_path.to_string_lossy().into_owned(),
        ));
    }
    Ok(csv_path)
}

/// Écrit `rows` au format LFMS (celui de l'export CSV) à côté de la page,
/// puis relit le fichier pour en calculer l'empreinte : elle porte sur les
/// octets réellement écrits, pas sur une sérialisation recalculée.
pub(crate) fn write_matrix_with_fingerprint(
    rows: &[FlowMatrixRow],
    link_type: Option<packet_parser::LinkType>,
    context: &SurveyContext,
    period: Option<lunar_flows_core::lfms::ObservedPeriod>,
    filter: Option<&str>,
    html_path: &Path,
) -> Result<WrittenCsv, ExportError> {
    let csv_path = sibling_csv_path(html_path)?;
    FlowMatrix::write_rows_to_csv_with_filter(
        rows,
        link_type,
        context,
        period,
        filter,
        &csv_path.to_string_lossy(),
    )?;
    let content = std::fs::read(&csv_path)?;
    let digest = Sha256::digest(&content);
    let sha256 = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(WrittenCsv {
        path: csv_path,
        sha256,
        bytes: content.len() as u64,
    })
}

/// Écrit la matrice courante en CSV à côté de `html_path` et renvoie de quoi
/// composer la page de synthèse. La page n'est pas écrite ici : l'interface
/// l'écrit ensuite au chemin choisi, comme l'export PNG du graphe.
pub fn export_synthesis_matrix(
    matrix: &Arc<Mutex<FlowMatrix>>,
    graph: &Arc<Mutex<GraphData>>,
    html_path: String,
) -> Result<SynthesisExport, CaptureStateError> {
    // Même ordre de verrouillage que le pipeline et les imports (matrice puis
    // graphe) : les deux copies datent du même instant. L'écriture disque se
    // fait hors verrou.
    let (rows, link_type, context, period, filter, graph_snapshot) = {
        let matrix_guard = matrix.lock()?;
        let graph_guard = graph.lock()?;
        (
            matrix_guard.to_flat_vec(),
            matrix_guard.link_type,
            matrix_guard.context.clone(),
            matrix_guard.period,
            matrix_guard.export_filter(None),
            graph_guard.get_all_graph_data(),
        )
    };

    // Un relevé issu d'un export partiel le reste : le CSV joint à la
    // synthèse le dit dans son préambule, comme tout autre export.
    let written = write_matrix_with_fingerprint(
        &rows,
        link_type,
        &context,
        period.known(),
        filter.as_deref(),
        Path::new(&html_path),
    )?;
    log::info!(
        "Synthèse : matrice écrite dans {} (SHA-256 {})",
        written.path.display(),
        written.sha256
    );

    Ok(SynthesisExport {
        graph: graph_snapshot,
        csv_file_name: written
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        csv_sha256: written.sha256,
        csv_bytes: written.bytes,
        link_type: link_type.map(link_type_name),
        site: context.site,
        sensor: context.sensor,
        interface: context.interface,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        period: period.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use lunar_flows_core::csv::merge_matrix_files;

    fn temp_dir(label: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("lunar_synthesis_{label}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dossier temporaire");
        dir
    }

    /// Matrice de référence du corpus (226 lignes, tunnels compris).
    fn sample_rows() -> Vec<FlowMatrixRow> {
        let path = crate::test_support::fixtures()
            .join("pcaps/import/ultimate_ethernet_sample/ultimate_ethernet_sample.csv");
        let rows = merge_matrix_files(std::slice::from_ref(&path))
            .expect("import de la matrice de référence")
            .to_flat_vec();
        assert!(!rows.is_empty());
        rows
    }

    #[test]
    fn fingerprint_is_the_sha256_of_the_bytes_written_next_to_the_page() {
        let dir = temp_dir("fingerprint");
        let html = dir.join("20261012_DR_Synthese.html");
        let rows = sample_rows();
        let context = SurveyContext {
            site: Some("Site d'essai".to_string()),
            sensor: None,
            interface: Some("eth0".to_string()),
        };

        let written = write_matrix_with_fingerprint(
            &rows,
            Some(packet_parser::LinkType::ETHERNET),
            &context,
            None,
            None,
            &html,
        )
        .expect("export");

        assert_eq!(written.path, dir.join("20261012_DR_Synthese.csv"));
        let content = std::fs::read(&written.path).expect("relecture du CSV");
        let expected = Sha256::digest(&content)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(written.sha256, expected);
        assert_eq!(written.sha256.len(), 64);
        assert_eq!(written.bytes, content.len() as u64);
        // Le CSV est un export LFMS ordinaire : contexte dans le préambule,
        // réimportable sans perte.
        let text = String::from_utf8(content).expect("CSV UTF-8");
        assert!(
            text.starts_with("#LFMS version=2 dlt=ETHERNET site=Site%20d%27essai interface=eth0\n"),
            "préambule LFMS absent ou sans contexte"
        );
        let reimported = merge_matrix_files(std::slice::from_ref(&written.path))
            .expect("réimport du CSV de synthèse");
        assert_eq!(reimported.to_flat_vec().len(), rows.len());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn same_matrix_gives_the_same_fingerprint() {
        let dir = temp_dir("stable");
        let rows = sample_rows();
        let context = SurveyContext::default();
        let first =
            write_matrix_with_fingerprint(&rows, None, &context, None, None, &dir.join("a.html"))
                .expect("premier export");
        let second =
            write_matrix_with_fingerprint(&rows, None, &context, None, None, &dir.join("b.html"))
                .expect("second export");
        assert_eq!(first.sha256, second.sha256);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn target_must_be_an_html_page() {
        assert!(matches!(
            sibling_csv_path(Path::new("")),
            Err(ExportError::EmptyPath)
        ));
        assert!(matches!(
            sibling_csv_path(Path::new("/tmp/synthese.csv")),
            Err(ExportError::NotHtml)
        ));
        assert!(matches!(
            sibling_csv_path(Path::new("/tmp/synthese")),
            Err(ExportError::NotHtml)
        ));
        assert_eq!(
            sibling_csv_path(Path::new("/tmp/lunar_absent_dir/Synthese.HTM")).expect("htm"),
            PathBuf::from("/tmp/lunar_absent_dir/Synthese.csv")
        );
    }

    #[test]
    fn an_unrelated_csv_with_the_same_name_is_not_overwritten() {
        let dir = temp_dir("sibling");
        let html = dir.join("releve.html");
        let csv = dir.join("releve.csv");
        std::fs::write(&csv, "export précédent").expect("CSV existant");

        // Page absente : le CSV homonyme appartient à un autre export.
        assert!(matches!(
            sibling_csv_path(&html),
            Err(ExportError::CsvAlreadyExists(_))
        ));
        assert_eq!(
            std::fs::read_to_string(&csv).expect("CSV intact"),
            "export précédent"
        );

        // Page présente : l'utilisateur a confirmé l'écrasement de la paire.
        std::fs::write(&html, "<html></html>").expect("page existante");
        assert_eq!(sibling_csv_path(&html).expect("paire"), csv);
        let _ = std::fs::remove_dir_all(dir);
    }
}
