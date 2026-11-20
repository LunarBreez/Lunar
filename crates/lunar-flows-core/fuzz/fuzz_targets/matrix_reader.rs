//! Cible de fuzzing : import d'une matrice LFMS (CSV) hostile.
#![no_main]

use libfuzzer_sys::fuzz_target;
use lunar_flows_core::{csv::merge_matrix_files, graph::GraphData};
use std::path::PathBuf;

/// Fichier de travail propre au processus libFuzzer, retiré après chaque
/// entrée. L'extension `.csv` aiguille vers le lecteur CSV.
struct FuzzMatrix(PathBuf);

impl FuzzMatrix {
    fn for_current_process() -> Self {
        Self(std::env::temp_dir().join(format!("lunar-matrix-fuzz-{}.csv", std::process::id())))
    }
}

impl Drop for FuzzMatrix {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

// Chaîne d'import complète d'une matrice : préambule `#LFMS` (versions, type
// de liaison, contexte percent-encodé, période), lignes CSV validées champ par
// champ, reconstruction des flux et de leurs tunnels, fusion, labels. Un
// fichier hostile doit produire une erreur typée, jamais un panic, une boucle
// ou un débordement. Une matrice acceptée passe ensuite par ses deux
// consommateurs : les lignes à plat de l'export et le graphe.
fuzz_target!(|data: &[u8]| {
    let file = FuzzMatrix::for_current_process();
    if std::fs::write(&file.0, data).is_err() {
        return;
    }

    if let Ok(matrix) = merge_matrix_files(std::slice::from_ref(&file.0)) {
        let _ = matrix.to_flat_vec();
        let _ = GraphData::from_flow_matrix(&matrix);
    }
});
