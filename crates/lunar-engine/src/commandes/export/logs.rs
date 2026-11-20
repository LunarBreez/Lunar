//! Commande d'export des fichiers de logs de l'application.

use chrono::{DateTime, Local};
use std::fs::{self, File};
use std::io::copy;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

use crate::errors::CaptureStateError;
use crate::errors::export::ExportError;

/// Exporte les fichiers de logs de l'application vers une archive ZIP dont le
/// chemin est donné par l'utilisateur.
///
/// Le dossier source est `log_dir` ([`crate::paths::AppPaths::log_dir`]) :
/// l'interface qui installe le journal doit y écrire pour que l'export
/// trouve ses fichiers.
///
/// `destination` désigne le fichier ZIP à créer, pas un dossier : l'ancienne
/// implémentation traitait systématiquement `destination` comme un
/// répertoire (`create_dir_all` + copie fichier par fichier), si bien que
/// choisir `lunar.log` dans le dialogue de sauvegarde créait un **dossier**
/// `lunar.log/` au lieu du fichier annoncé.
///
/// # Paramètres
///
/// - `log_dir`: Dossier des journaux à archiver.
/// - `destination`: Chemin du fichier ZIP à créer, fourni par le frontend.
///
/// # Retour
///
/// - `Ok(String)` : Message de succès si l'archive a été créée correctement.
/// - `Err(ExportError)` : Erreur en cas d’échec (log introuvable, erreur de
///   lecture/écriture, erreur d'archivage, etc.).
///
/// # Erreurs possibles
///
/// - [`ExportError::LogNotFound`] : Le dossier source de logs n’existe pas.
/// - [`ExportError::Io`] : Une erreur d’entrée/sortie s’est produite lors de la copie ou de la lecture des fichiers.
/// - [`ExportError::Zip`] : Une erreur s'est produite lors de l'écriture de l'archive ZIP.
///
pub fn export_logs(log_dir: &Path, destination: String) -> Result<String, CaptureStateError> {
    if !log_dir.exists() {
        return Err(CaptureStateError::Export(ExportError::LogNotFound));
    }

    let destination = PathBuf::from(destination);
    if let Some(parent) = destination.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }

    // Écriture dans un fichier temporaire puis renommage atomique, pour ne
    // jamais laisser une archive partielle au chemin final en cas d'échec.
    //
    // Le temporaire porte le PID et un compteur : `with_extension("part")`
    // était déterministe, donc deux exports simultanés vers la même
    // destination écrivaient dans le même fichier et se corrompaient
    // mutuellement.
    let tmp_destination = unique_part_path(&destination);
    write_logs_zip(log_dir, &tmp_destination).inspect_err(|_| {
        let _ = fs::remove_file(&tmp_destination);
    })?;
    fs::rename(&tmp_destination, &destination).inspect_err(|_| {
        let _ = fs::remove_file(&tmp_destination);
    })?;
    // Le renommage rend le nom atomique, pas le contenu : sans cette
    // synchronisation du répertoire, une coupure d'alimentation juste après
    // peut laisser l'entrée de répertoire sans les données. Le
    // contenu du fichier, lui, est déjà synchronisé par `write_logs_zip`.
    sync_parent_dir(&destination);

    Ok("Logs exportés avec succès".to_string())
}

/// Chemin temporaire unique voisin de la destination — voisin pour que le
/// renommage final reste sur le même système de fichiers.
fn unique_part_path(destination: &Path) -> PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);

    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let stem = destination
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("lunar-support");
    destination.with_file_name(format!(".{stem}.{}.{n}.part", std::process::id()))
}

/// `fsync` du répertoire contenant `path`, pour que la création du fichier
/// survive à une coupure.
///
/// Silencieux en cas d'échec, et sans effet sous Windows : on n'y ouvre pas
/// un répertoire comme un fichier. L'export a réussi à ce stade — échouer
/// ici priverait l'utilisateur d'une archive pourtant écrite.
fn sync_parent_dir(path: &Path) {
    #[cfg(unix)]
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
        && let Ok(dir) = File::open(parent)
    {
        let _ = dir.sync_all();
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// Archive dans `destination` tous les fichiers de `log_dir` (sans
/// récursion), en Deflate, plus le manifest de support `MANIFEST_NAME`
/// (version, système, période couverte par les dates de modification). Le
/// fichier est synchronisé sur disque avant retour : l'appelant le renomme
/// ensuite vers le nom final.
fn write_logs_zip(log_dir: &Path, destination: &Path) -> Result<(), CaptureStateError> {
    let file = File::create(destination)?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    let mut archived: Vec<String> = Vec::new();
    let mut oldest: Option<SystemTime> = None;
    let mut newest: Option<SystemTime> = None;

    for entry in fs::read_dir(log_dir)? {
        let entry = entry?;
        let src_path = entry.path();
        if !src_path.is_file() {
            continue;
        }
        // Un fichier issu de read_dir a toujours un nom ; sinon on l'ignore
        // plutôt que de paniquer.
        let Some(file_name) = src_path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if let Ok(modified) = entry.metadata().and_then(|m| m.modified()) {
            oldest = Some(oldest.map_or(modified, |o: SystemTime| o.min(modified)));
            newest = Some(newest.map_or(modified, |n: SystemTime| n.max(modified)));
        }
        zip.start_file(file_name, options)
            .map_err(ExportError::Zip)?;
        let mut src_file = File::open(&src_path)?;
        copy(&mut src_file, &mut zip)?;
        archived.push(file_name.to_string());
    }

    // Manifest de support : sans lui, une archive reçue au support ne
    // dit ni de quelle version ni de quel système elle provient, et rien
    // n'indique si la période couverte correspond à l'incident signalé.
    zip.start_file(MANIFEST_NAME, options)
        .map_err(ExportError::Zip)?;
    copy(
        &mut support_manifest(&archived, oldest, newest).as_bytes(),
        &mut zip,
    )?;

    // `finish` rend le fichier ; il faut encore forcer son écriture sur le
    // disque avant le renommage, sinon l'archive publiée sous son nom final
    // peut être tronquée après une coupure — précisément ce que le motif
    // temporaire + renommage prétend éviter.
    let file = zip.finish().map_err(ExportError::Zip)?;
    file.sync_all()?;
    Ok(())
}

/// Nom du manifest dans l'archive. Préfixé pour rester en tête d'un listing
/// trié, avant les fichiers de logs eux-mêmes.
const MANIFEST_NAME: &str = "00-lunar-support-manifest.txt";

/// Contenu du manifest : ce qu'il faut pour situer l'archive sans l'ouvrir.
fn support_manifest(
    archived: &[String],
    oldest: Option<SystemTime>,
    newest: Option<SystemTime>,
) -> String {
    let format_time = |t: Option<SystemTime>| {
        t.map(|t| {
            DateTime::<Local>::from(t)
                .format("%Y-%m-%d %H:%M:%S %z")
                .to_string()
        })
        .unwrap_or_else(|| "inconnue".to_string())
    };

    format!(
        "LUNAR — archive de support\n\
         \n\
         version applicative : {}\n\
         système             : {} {} ({})\n\
         exportée le         : {}\n\
         période couverte    : {} → {}\n\
         fichiers archivés   : {}\n\
         \n\
         Ces journaux peuvent contenir le nom de la machine, ses interfaces\n\
         réseau et le chemin complet des fichiers importés.\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::env::consts::FAMILY,
        Local::now().format("%Y-%m-%d %H:%M:%S %z"),
        format_time(oldest),
        format_time(newest),
        archived.len(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    fn unique_temp_dir(label: &str) -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "lunar_export_logs_test_{label}_{}_{n}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn write_logs_zip_produces_a_single_archive_file_not_a_directory() {
        let log_dir = unique_temp_dir("src");
        fs::write(log_dir.join("lunar.log"), b"hello").unwrap();
        fs::write(log_dir.join("lunar.log.1"), b"older").unwrap();
        // Un sous-dossier ne doit pas être archivé : seuls les fichiers le sont.
        fs::create_dir_all(log_dir.join("subdir")).unwrap();

        let out_dir = unique_temp_dir("out");
        let destination = out_dir.join("lunar-logs.zip");

        write_logs_zip(&log_dir, &destination).unwrap();

        assert!(
            destination.is_file(),
            "la destination doit être un fichier, pas un dossier"
        );

        let file = File::open(&destination).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let mut names: Vec<_> = (0..archive.len())
            .map(|i| archive.by_index(i).unwrap().name().unwrap().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, vec![MANIFEST_NAME, "lunar.log", "lunar.log.1"]);

        fs::remove_dir_all(&log_dir).ok();
        fs::remove_dir_all(&out_dir).ok();
    }

    #[test]
    fn archive_carries_a_support_manifest_situating_it() {
        let log_dir = unique_temp_dir("manifest_src");
        fs::write(log_dir.join("lunar.log"), b"hello").unwrap();

        let out_dir = unique_temp_dir("manifest_out");
        let destination = out_dir.join("lunar-support.zip");
        write_logs_zip(&log_dir, &destination).unwrap();

        let file = File::open(&destination).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let mut manifest = String::new();
        std::io::Read::read_to_string(&mut archive.by_name(MANIFEST_NAME).unwrap(), &mut manifest)
            .unwrap();

        // Situer l'archive : version, système, et le fait que son contenu
        // n'est pas anodin. Sans quoi elle n'est qu'un tas de fichiers.
        assert!(
            manifest.contains(env!("CARGO_PKG_VERSION")),
            "manifest sans version applicative : {manifest}"
        );
        assert!(
            manifest.contains(std::env::consts::OS),
            "manifest sans système : {manifest}"
        );
        assert!(
            manifest.contains("fichiers archivés   : 1"),
            "manifest sans décompte exact : {manifest}"
        );
        assert!(
            manifest.contains("nom de la machine"),
            "manifest sans avertissement sur les données sensibles : {manifest}"
        );

        fs::remove_dir_all(&log_dir).ok();
        fs::remove_dir_all(&out_dir).ok();
    }

    #[test]
    fn concurrent_exports_do_not_share_a_temporary_path() {
        // `with_extension("part")` renvoyait le même chemin pour tout le
        // monde : deux exports simultanés vers la même destination
        // s'écrasaient l'un l'autre en cours d'écriture.
        let destination = PathBuf::from("/tmp/lunar-support.zip");
        let first = unique_part_path(&destination);
        let second = unique_part_path(&destination);

        assert_ne!(first, second, "deux temporaires identiques : {first:?}");
        assert_eq!(
            first.parent(),
            destination.parent(),
            "le temporaire doit rester dans le dossier de destination, \
             sinon le renommage traverse deux systèmes de fichiers"
        );
        for path in [&first, &second] {
            let name = path.file_name().unwrap().to_str().unwrap();
            assert!(name.ends_with(".part"), "suffixe inattendu : {name}");
            assert!(
                name.contains(&std::process::id().to_string()),
                "le temporaire doit porter le PID : {name}"
            );
        }
    }

    #[test]
    fn manifest_reports_unknown_period_when_no_file_carries_a_date() {
        let manifest = support_manifest(&[], None, None);
        assert!(
            manifest.contains("période couverte    : inconnue → inconnue"),
            "période absente mal rendue : {manifest}"
        );
        assert!(manifest.contains("fichiers archivés   : 0"));
    }
}
