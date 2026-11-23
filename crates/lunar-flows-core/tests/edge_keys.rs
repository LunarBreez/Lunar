//! Correspondance flux de la matrice ↔ arêtes du graphe, sur tout le corpus.
//!
//! L'export des seuls flux affichés traduit « les arêtes visibles » en lignes
//! de matrice par [`edge_key_of_flow`]. Cette traduction n'est juste que si
//! la clé calculée depuis un flux est exactement celle de l'arête que le
//! graphe construit pour lui. Le test l'établit sur chaque capture et chaque
//! matrice de `fixtures/`, compteurs compris : un flux rangé sous la mauvaise
//! arête, ou une arête sans flux, ferait exporter trop ou trop peu.

#![cfg(feature = "pcap")]
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use lunar_flows_core::csv::merge_matrix_files;
use lunar_flows_core::graph::{GraphData, edge_key_of_flow};
use lunar_flows_core::matrix::FlowMatrix;
use lunar_flows_core::pcap::append_pcap_file;

/// Racine du corpus versionné.
fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

/// Fichiers de `dir`, récursivement, par chemin croissant.
fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(current) = pending.pop() {
        for entry in std::fs::read_dir(&current).expect("lecture du corpus") {
            let path = entry.expect("entrée du corpus").path();
            if path.is_dir() {
                pending.push(path);
            } else {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

/// Extension du fichier, en minuscules.
fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// (paquets, octets) par clé d'arête.
type Totals = BTreeMap<String, (u64, u64)>;

/// Vérifie la correspondance sur une matrice ; rend son nombre de flux.
fn assert_flows_match_edges(matrix: &FlowMatrix, source: &Path) -> usize {
    let mut from_flows = Totals::new();
    for (flow, entries) in &matrix.matrix {
        let totals = from_flows.entry(edge_key_of_flow(flow)).or_default();
        for (_, stats) in entries {
            totals.0 = totals.0.saturating_add(stats.count);
            totals.1 = totals.1.saturating_add(stats.total_bytes);
        }
    }
    let graph = GraphData::from_flow_matrix(matrix);
    let from_edges: Totals = graph
        .edges
        .iter()
        .map(|(key, edge)| (key.clone(), (edge.count, edge.total_bytes)))
        .collect();
    assert_eq!(
        from_flows.keys().collect::<Vec<_>>(),
        from_edges.keys().collect::<Vec<_>>(),
        "{} : clés d'arête des flux ≠ clés du graphe",
        source.display()
    );
    assert_eq!(
        from_flows,
        from_edges,
        "{} : compteurs par arête, flux ≠ graphe",
        source.display()
    );
    matrix.matrix.len()
}

/// Chaque capture du corpus que le parseur accepte : les flux de sa matrice
/// retrouvent leurs arêtes, tunnels, VLAN et trames sans IP compris.
#[test]
fn every_capture_of_the_corpus_maps_its_flows_to_its_edges() {
    let mut checked = 0;
    let mut flows = 0;
    for path in files_under(&fixtures()) {
        if !matches!(extension(&path).as_str(), "pcap" | "pcapng" | "cap") {
            continue;
        }
        let mut matrix = FlowMatrix::new();
        // Les captures d'un type de liaison non pris en charge sont refusées
        // à l'ouverture : elles n'ont ni matrice ni graphe à comparer.
        if append_pcap_file(&mut matrix, &path).is_err() {
            continue;
        }
        flows += assert_flows_match_edges(&matrix, &path);
        checked += 1;
    }
    assert!(checked >= 30, "seulement {checked} captures vérifiées");
    assert!(flows >= 500, "seulement {flows} flux vérifiés");
}

/// Chaque matrice CSV ou XLSX du corpus : même correspondance après
/// réimport, labels et colonnes de tunnel compris.
#[test]
fn every_matrix_file_of_the_corpus_maps_its_flows_to_its_edges() {
    let mut checked = 0;
    for path in files_under(&fixtures()) {
        if !matches!(extension(&path).as_str(), "csv" | "xlsx") {
            continue;
        }
        // Le corpus mêle matrices, fichiers de labels et relevés WinAudit :
        // seuls les fichiers qui se lisent comme une matrice non vide comptent.
        let Ok(matrix) = merge_matrix_files(std::slice::from_ref(&path)) else {
            continue;
        };
        if matrix.matrix.is_empty() {
            continue;
        }
        assert_flows_match_edges(&matrix, &path);
        checked += 1;
    }
    assert!(checked >= 6, "seulement {checked} matrices vérifiées");
}
