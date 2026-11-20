//! Commande d'export des labels en CSV.

use std::sync::{Arc, Mutex};

use crate::{
    errors::{CaptureStateError, export::ExportError},
    state::flow_matrix::FlowMatrix,
};

/// Exporte les labels courants (matrice de flux) vers un fichier CSV
/// `mac,ip,label`. Les labels partiels édités à la main pendant la capture
/// sont complétés à partir des couples (MAC, IP) observés dans la matrice.
pub fn export_label_file(
    state: &Arc<Mutex<FlowMatrix>>,
    path: String,
) -> Result<(), CaptureStateError> {
    if path.trim().is_empty() {
        return Err(CaptureStateError::Export(ExportError::EmptyPath));
    }

    // Verrou court : snapshot des labels résolus seulement. L'écriture
    // disque se fait hors verrou pour ne pas bloquer le pipeline de capture.
    let rows = state.lock()?.export_labels();

    FlowMatrix::write_label_rows_to_csv(&rows, &path)?;
    Ok(())
}

/// Lignes du fichier « à compléter » : d'abord les équipements
/// réels sans label, label vide, par paquets décroissants ; puis les labels
/// en place. L'import remplaçant tout le référentiel, réimporter le fichier
/// complété garde donc les labels existants ; les lignes restées vides sont
/// ignorées à l'import.
pub(crate) fn rows_to_complete(matrix: &FlowMatrix) -> (Vec<(String, String, String)>, usize) {
    let unlabeled = matrix.unlabeled_endpoints();
    let count = unlabeled.len();
    let mut rows: Vec<(String, String, String)> = unlabeled
        .into_iter()
        .map(|endpoint| (endpoint.mac, endpoint.ip, String::new()))
        .collect();
    rows.extend(matrix.export_labels());
    (rows, count)
}

/// Exporte le fichier de labels « à compléter » et rend le nombre
/// d'équipements sans label qu'il contient.
pub fn export_labels_to_complete(
    state: &Arc<Mutex<FlowMatrix>>,
    path: String,
) -> Result<usize, CaptureStateError> {
    if path.trim().is_empty() {
        return Err(CaptureStateError::Export(ExportError::EmptyPath));
    }
    // Verrou court : lignes calculées sous verrou, écriture disque hors verrou.
    let (rows, count) = {
        let matrix = state.lock()?;
        rows_to_complete(&matrix)
    };
    FlowMatrix::write_label_rows_to_csv(&rows, &path)?;
    log::info!("Labels à compléter : {count} équipement(s) sans label exporté(s) vers {path}");
    Ok(count)
}
