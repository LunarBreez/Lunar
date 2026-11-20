//! Données d'essai de la vue d'investigation, sans interface d'import.
//!
//! - `seed <dossier de données> <matrice.csv>` : importe la matrice dans un
//!   moteur neuf et l'enregistre comme autosave d'une session interrompue
//!   (`autosave/autosave.lunar` et `autosave/session.lock` sous le dossier
//!   de données de l'application, `$XDG_DATA_HOME/fr.lunar`) : au
//!   lancement suivant, LUNAR propose de la récupérer.
//! - `verify <projet.lunar> <page.html> [--label MAC IP LABEL]…` : rouvre le
//!   projet dans un moteur neuf, rejoue les labels posés dans la vue, exporte
//!   la synthèse sous le même nom dans un dossier temporaire et la compose
//!   avec [`render_synthesis_html`] à l'instant lu dans la page (et le fuseau
//!   du poste) ; la page et son CSV doivent être identiques à l'octet à ceux
//!   que la vue a écrits.
//!
//! Usage : `cargo run --release -p lunar --example seed_investigation -- …`

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use lunar::app::investigation::support::{SystemZone, synthesis_export};
use lunar_engine::events::CaptureEvent;
use lunar_engine::ipc::Channel;
use lunar_engine::{AppPaths, Lunar};
use lunar_ui_logic::investigation::dates::parse_js_date;
use lunar_ui_logic::investigation::render_synthesis_html;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = std::env::temp_dir().join(format!("lunar-seed-{}", std::process::id()));
    let result = match args.first().map(String::as_str) {
        Some("seed") if args.len() == 3 => seed(Path::new(&args[1]), &args[2], &root),
        Some("verify") if args.len() >= 3 => verify(&args[1], Path::new(&args[2]), &args[3..], &root),
        _ => Err("usage : seed_investigation seed <dossier de données> <matrice.csv>\n        seed_investigation verify <projet.lunar> <page.html> [--label MAC IP LABEL]…".into()),
    };
    let _ = std::fs::remove_dir_all(&root);
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("seed_investigation : {message}");
            ExitCode::FAILURE
        }
    }
}

/// Moteur neuf, isolé sous `root`.
fn engine(root: &Path) -> Result<Lunar, String> {
    Lunar::new(AppPaths::under(root)).map_err(|e| format!("moteur : {e}"))
}

/// Canal dont les événements sont ignorés.
fn quiet() -> Channel<CaptureEvent> {
    Channel::new(|_: CaptureEvent| {})
}

/// Pose l'autosave d'une session interrompue sous `data_dir`.
fn seed(data_dir: &Path, matrix: &str, root: &Path) -> Result<(), String> {
    let lunar = engine(root)?;
    lunar
        .import_matrix_file(matrix.to_string(), quiet())
        .map_err(|e| format!("import : {e}"))?;
    let dir = data_dir.join("autosave");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let project = dir.join("autosave.lunar");
    lunar
        .save_project(project.to_string_lossy().into_owned())
        .map_err(|e| format!("enregistrement : {e}"))?;
    std::fs::write(dir.join("session.lock"), b"").map_err(|e| format!("sentinelle : {e}"))?;
    let graph = lunar.get_graph_snapshot().map_err(|e| format!("{e}"))?;
    println!(
        "autosave posé : {} ({} nœuds, {} arêtes)",
        project.display(),
        graph.nodes.len(),
        graph.edges.len()
    );
    Ok(())
}

/// Instant d'export écrit dans la page (`<code>…Z</code>` de l'en-tête).
fn generated_at(html: &str) -> Option<lunar_ui_logic::investigation::EpochMillis> {
    let start = html.find("(<code>")? + "(<code>".len();
    let end = start + html[start..].find("</code>")?;
    parse_js_date(&html[start..end])
}

/// Recompose la page et compare.
fn verify(project: &str, page: &Path, rest: &[String], root: &Path) -> Result<(), String> {
    let lunar = engine(root)?;
    lunar
        .open_project(project.to_string(), quiet())
        .map_err(|e| format!("ouverture : {e}"))?;
    let mut rest = rest.iter();
    while let Some(flag) = rest.next() {
        let (Some(mac), Some(ip), Some(label)) = (rest.next(), rest.next(), rest.next()) else {
            return Err(format!("{flag} : MAC, IP et label attendus"));
        };
        if flag != "--label" {
            return Err(format!("option inconnue : {flag}"));
        }
        lunar
            .add_label(mac.clone(), ip.clone(), label.clone())
            .map_err(|e| format!("label : {e}"))?;
    }
    let written = std::fs::read(page).map_err(|e| format!("{}: {e}", page.display()))?;
    let written = String::from_utf8(written).map_err(|e| format!("page non UTF-8 : {e}"))?;
    let at = generated_at(&written).ok_or("instant d'export introuvable dans la page")?;
    let name = page.file_name().ok_or("nom de page vide")?;
    let out_dir = root.join("verification");
    std::fs::create_dir_all(&out_dir).map_err(|e| format!("{e}"))?;
    let html_path: PathBuf = out_dir.join(name);
    let export = lunar
        .export_synthesis_matrix(html_path.to_string_lossy().into_owned())
        .map_err(|e| format!("export : {e}"))?;
    let expected = render_synthesis_html(&synthesis_export(export), at, &SystemZone);
    let csv_written =
        std::fs::read(page.with_extension("csv")).map_err(|e| format!("CSV de la vue : {e}"))?;
    let csv_expected =
        std::fs::read(html_path.with_extension("csv")).map_err(|e| format!("{e}"))?;
    if csv_written != csv_expected {
        return Err("le CSV écrit par la vue diffère de celui du moteur".into());
    }
    if expected.as_bytes() != written.as_bytes() {
        let at = expected
            .bytes()
            .zip(written.bytes())
            .position(|(a, b)| a != b)
            .unwrap_or(expected.len().min(written.len()));
        return Err(format!(
            "page différente à l'octet {at} (attendu {} octets, écrit {})",
            expected.len(),
            written.len()
        ));
    }
    println!(
        "page identique à render_synthesis_html : {} octets, CSV {} octets",
        written.len(),
        csv_written.len()
    );
    Ok(())
}
