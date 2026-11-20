//! Cible de fuzzing : import d'une capture PCAP/PCAPNG hostile.
#![no_main]

use libfuzzer_sys::fuzz_target;
use lunar_flows_core::{graph::GraphData, matrix::FlowMatrix, pcap::append_pcap_file};
use std::path::PathBuf;

/// Fichier de travail propre au processus libFuzzer, retiré après chaque
/// entrée : libpcap ne lit une capture que depuis un chemin.
struct FuzzCapture(PathBuf);

impl FuzzCapture {
    fn for_current_process() -> Self {
        Self(std::env::temp_dir().join(format!("lunar-pcap-fuzz-{}.pcap", std::process::id())))
    }
}

impl Drop for FuzzCapture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

// Même chaîne que l'import de l'application : ouverture libpcap, contrôle du
// type de liaison, lecture exhaustive, `packet_parser`, agrégation dans
// `FlowMatrix`. Un fichier refusé est un résultat normal ; seuls un panic, un
// crash, une erreur de sanitizer ou un blocage intéressent le fuzzer. La
// matrice, même partielle après une erreur de lecture, passe ensuite par ses
// deux consommateurs : les lignes à plat de l'export et le graphe.
fuzz_target!(|data: &[u8]| {
    let capture = FuzzCapture::for_current_process();
    if std::fs::write(&capture.0, data).is_err() {
        return;
    }

    let mut matrix = FlowMatrix::new();
    let _ = append_pcap_file(&mut matrix, &capture.0);
    let _ = matrix.to_flat_vec();
    let _ = GraphData::from_flow_matrix(&matrix);
});
