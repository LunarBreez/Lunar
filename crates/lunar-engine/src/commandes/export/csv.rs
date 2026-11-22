//! Commande d'export de la matrice de flux en CSV.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use crate::{
    errors::{CaptureStateError, export::ExportError},
    state::flow_matrix::FlowMatrix,
};

/// Exporte la matrice de flux courante vers `path` (format de
/// `FlowMatrix::export_to_csv`, réimportable par `import_matrix_files`).
pub fn export_csv(state: &Arc<Mutex<FlowMatrix>>, path: String) -> Result<(), CaptureStateError> {
    if path.trim().is_empty() {
        return Err(CaptureStateError::Export(ExportError::EmptyPath));
    }

    // Verrou court : snapshot des lignes, du DLT, du contexte et de la
    // période seulement. L'écriture disque se fait hors verrou pour ne pas
    // bloquer le pipeline de capture (le processing thread verrouille la
    // matrice à chaque paquet).
    let (rows, link_type, context, period, filter) = {
        let matrix = state.lock()?;
        (
            matrix.to_flat_vec(),
            matrix.link_type,
            matrix.context.clone(),
            matrix.period.known(),
            matrix.export_filter(None),
        )
    };

    // Un export CSV porte le contexte et la période du
    // relevé dans son préambule `#LFMS` : le fichier reste qualifié une fois
    // sorti de l'application. Un relevé issu d'un export partiel y redit
    // son filtre : exporter tout ce qu'on a ne le rend pas complet.
    FlowMatrix::write_rows_to_csv_with_filter(
        &rows,
        link_type,
        &context,
        period,
        filter.as_deref(),
        &path,
    )?;
    Ok(())
}

/// Exporte vers `path` les seuls flux des arêtes `edge_keys` du graphe (clés
/// de [`lunar_flows_core::graph::GraphData::edges`]), c'est-à-dire les flux
/// affichés après filtrage. `description` décrit le filtre en clair ; elle part dans
/// le préambule `#LFMS` (`filter=`), pour que le fichier dise lui-même qu'il
/// ne porte pas tout le relevé. Retourne le nombre de lignes écrites.
///
/// Refusé sans description : un export partiel anonyme serait pris pour un
/// relevé complet.
pub fn export_csv_filtered(
    state: &Arc<Mutex<FlowMatrix>>,
    path: String,
    edge_keys: &HashSet<String>,
    description: &str,
) -> Result<usize, CaptureStateError> {
    if path.trim().is_empty() {
        return Err(CaptureStateError::Export(ExportError::EmptyPath));
    }
    if description.trim().is_empty() {
        return Err(CaptureStateError::Export(ExportError::MissingFilter));
    }

    // Même discipline que `export_csv` : snapshot sous verrou court,
    // écriture hors verrou.
    let (rows, link_type, context, period, filter) = {
        let matrix = state.lock()?;
        (
            matrix.to_flat_vec_where(|flow| {
                edge_keys.contains(&lunar_flows_core::graph::edge_key_of_flow(flow))
            }),
            matrix.link_type,
            matrix.context.clone(),
            matrix.period.known(),
            matrix.export_filter(Some(description)),
        )
    };

    FlowMatrix::write_rows_to_csv_with_filter(
        &rows,
        link_type,
        &context,
        period,
        filter.as_deref(),
        &path,
    )?;
    Ok(rows.len())
}
