//! Contrôle de parité : traite des PCAP ou des matrices par la façade
//! [`lunar_engine::Lunar`] et exporte le CSV : référence pour comparer à
//! l'octet l'export de l'interface (parcours E2E).
//!
//! Usage : `parity_csv pcap|matrix <sortie.csv> <entrées>…`

use lunar_engine::events::CaptureEvent;
use lunar_engine::ipc::Channel;
use lunar_engine::{AppPaths, Lunar};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(mode), Some(output)) = (args.first(), args.get(1)) else {
        eprintln!("usage : parity_csv pcap|matrix <sortie.csv> <entrées>…");
        return ExitCode::from(2);
    };
    let inputs = args[2..].to_vec();
    let root = std::env::temp_dir().join(format!("lunar-parity-{}", std::process::id()));
    let result = run(mode, output, inputs, &root);
    let _ = std::fs::remove_dir_all(&root);
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("parity_csv : {message}");
            ExitCode::FAILURE
        }
    }
}

/// Importe `inputs` dans un moteur neuf, isolé sous `root`, puis exporte.
fn run(
    mode: &str,
    output: &str,
    inputs: Vec<String>,
    root: &std::path::Path,
) -> Result<(), String> {
    let lunar = Lunar::new(AppPaths::under(root)).map_err(|e| format!("{e:?}"))?;
    // Les erreurs remontent par le `Result` ; les événements sont ignorés.
    let channel = Channel::new(|_: CaptureEvent| {});
    match mode {
        "pcap" => lunar.convert_from_pcap_list(inputs, channel),
        "matrix" => lunar.import_matrix_files(inputs, channel),
        other => return Err(format!("mode inconnu : {other}")),
    }
    .map_err(|e| format!("{e:?}"))?;
    lunar
        .export_csv(output.to_string())
        .map_err(|e| format!("{e:?}"))
}
