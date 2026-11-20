//! Listes de fichiers du panneau d'import (mode captures et matrices) et
//! suivi fichier par fichier de l'import en cours : statut, bilan et durée
//! tirés des événements du moteur, libellé de progression de l'overlay.
//!
//! La déduplication et le routage du glisser-déposer sont ceux de
//! [`crate::file_types`].
//!
//! - Un fichier se retire de sa liste un par un, ou toute la liste par
//!   « Effacer ».
//! - Chaque fichier affiche son statut (en attente, en cours, lu avec son
//!   bilan, annulé, en échec) et sa durée.
//! - Les nombres du libellé sont groupés par milliers avec une espace
//!   insécable, comme `toLocaleString()` sous une locale française.

use std::time::{Duration, Instant};

use lunar_engine::events::CaptureEvent;

use crate::file_types::{DroppedPathsRouting, append_unique_paths};
use crate::import_progress::{ImportProgress, progress_file_name_of};

/// Séparateur des milliers (espace insécable).
pub const THOUSANDS_SEPARATOR: char = '\u{a0}';

/// Nature d'un fichier à importer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImportKind {
    /// Capture PCAP, PCAPNG ou CAP (`convert_from_pcap_list`).
    Captures,
    /// Matrice de flux CSV ou XLSX (`import_matrix_files`).
    Matrices,
}

/// Les deux listes du panneau, dans l'ordre d'ajout, sans doublon.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportFiles {
    /// Captures à convertir.
    captures: Vec<String>,
    /// Matrices à fusionner.
    matrices: Vec<String>,
}

impl ImportFiles {
    /// Listes vides.
    pub fn new() -> Self {
        Self::default()
    }

    /// Fichiers de la liste `kind`.
    pub fn list(&self, kind: ImportKind) -> &[String] {
        match kind {
            ImportKind::Captures => &self.captures,
            ImportKind::Matrices => &self.matrices,
        }
    }

    /// Liste `kind`, modifiable.
    fn list_mut(&mut self, kind: ImportKind) -> &mut Vec<String> {
        match kind {
            ImportKind::Captures => &mut self.captures,
            ImportKind::Matrices => &mut self.matrices,
        }
    }

    /// Ajoute des chemins à la liste `kind` en écartant ceux déjà listés
    /// Rend vrai si la liste a changé.
    pub fn add(&mut self, kind: ImportKind, paths: &[impl AsRef<str>]) -> bool {
        let list = self.list_mut(kind);
        let merged = append_unique_paths(list, paths);
        let changed = merged != *list;
        *list = merged;
        changed
    }

    /// Ajoute les fichiers déposés, déjà routés par
    /// [`crate::file_types::route_dropped_paths`]. Rend vrai si une liste a
    /// changé ; un dépôt de labels (`Csv`) ou ignoré ne change rien.
    pub fn add_dropped(&mut self, routing: &DroppedPathsRouting) -> bool {
        match routing {
            DroppedPathsRouting::Files { pcaps, matrices } => {
                let captures = self.add(ImportKind::Captures, pcaps);
                let matrices = self.add(ImportKind::Matrices, matrices);
                captures || matrices
            }
            DroppedPathsRouting::Csv { .. } | DroppedPathsRouting::Ignored => false,
        }
    }

    /// Retire le fichier de rang `index` de la liste `kind`. Rend le chemin
    /// retiré, `None` hors de la liste.
    pub fn remove(&mut self, kind: ImportKind, index: usize) -> Option<String> {
        let list = self.list_mut(kind);
        (index < list.len()).then(|| list.remove(index))
    }

    /// Vide la liste `kind` (« Effacer »).
    pub fn clear(&mut self, kind: ImportKind) {
        self.list_mut(kind).clear();
    }
}

/// Bilan d'un fichier lu, tiré de son événement `Finished`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FileReport {
    /// Paquets (ou lignes) lus.
    pub read: usize,
    /// Paquets (ou lignes) intégrés.
    pub integrated: usize,
    /// Rejets du parseur (tronqués, DLT non supportés, malformés).
    pub rejected: usize,
}

/// Événement du moteur réduit à ce que le suivi par fichier exploite, daté
/// à sa réception.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileEvent {
    /// Progression dans un fichier (`ImportProgress`).
    Progress {
        /// Chemin émis par le moteur.
        file_name: String,
        /// Rang du fichier, à partir de 1.
        file_index: usize,
        /// Réception.
        at: Instant,
    },
    /// Fichier lu (`Finished`).
    Finished {
        /// Chemin émis par le moteur.
        file_name: String,
        /// Bilan.
        report: FileReport,
        /// Réception.
        at: Instant,
    },
}

/// Réduit un événement du moteur pour le suivi par fichier ; `None` pour
/// les événements qui ne concernent pas un fichier.
pub fn file_event(event: &CaptureEvent, at: Instant) -> Option<FileEvent> {
    match event {
        CaptureEvent::ImportProgress {
            file_name,
            file_index,
            ..
        } => Some(FileEvent::Progress {
            file_name: file_name.clone(),
            file_index: *file_index,
            at,
        }),
        CaptureEvent::Finished {
            file_name,
            packet_total_count,
            integrated_count,
            rejected_truncated_count,
            rejected_unsupported_link_type_count,
            rejected_malformed_count,
            ..
        } => Some(FileEvent::Finished {
            file_name: file_name.clone(),
            report: FileReport {
                read: *packet_total_count,
                integrated: *integrated_count,
                rejected: rejected_truncated_count
                    + rejected_unsupported_link_type_count
                    + rejected_malformed_count,
            },
            at,
        }),
        _ => None,
    }
}

/// Statut d'un fichier dans l'import en cours ou terminé.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileState {
    /// Pas encore ouvert.
    Pending,
    /// En cours de lecture.
    Running,
    /// Lu, avec son bilan.
    Done(FileReport),
    /// Interrompu par l'annulation de l'import.
    Cancelled,
    /// Interrompu par une erreur.
    Failed,
}

/// Fichier de l'import, son statut et ses instants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRun {
    /// Chemin, tel que listé.
    pub path: String,
    /// Statut.
    pub state: FileState,
    /// Début de la lecture.
    started: Option<Instant>,
    /// Dernier événement reçu pour ce fichier.
    last: Option<Instant>,
    /// Fin de la lecture (fichier suivant commencé, ou import terminé).
    ended: Option<Instant>,
}

impl FileRun {
    /// Durée de lecture : figée une fois le fichier terminé, courante sinon ;
    /// `None` avant son début.
    pub fn elapsed(&self, now: Instant) -> Option<Duration> {
        let started = self.started?;
        Some(self.ended.unwrap_or(now).saturating_duration_since(started))
    }
}

/// Issue d'un import, connue au retour de la commande.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOutcome {
    /// Import réussi.
    Success,
    /// Import annulé par l'opérateur.
    Cancelled,
    /// Import en échec.
    Failed,
}

/// Suivi fichier par fichier d'un import.
///
/// Un fichier commence quand le précédent cède la place (premier événement
/// d'un autre fichier) ou, pour le premier, au lancement ; il se termine à
/// son dernier événement avant de céder la place, ou à `Finished` s'il est
/// toujours le fichier courant. Pour les captures, c'est le temps de lecture
/// et d'analyse ; pour les matrices (lues toutes, puis fusionnées), celui de
/// la lecture, la fusion comptant pour le dernier fichier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportRun {
    /// Nature des fichiers.
    kind: ImportKind,
    /// Fichiers, dans l'ordre de la liste.
    files: Vec<FileRun>,
    /// Rang du fichier courant.
    current: Option<usize>,
    /// Issue, une fois connue.
    outcome: Option<RunOutcome>,
}

impl ImportRun {
    /// Import de `paths` lancé à `at` : le premier fichier est en cours.
    pub fn start(kind: ImportKind, paths: &[String], at: Instant) -> Self {
        let mut files: Vec<FileRun> = paths
            .iter()
            .map(|path| FileRun {
                path: path.clone(),
                state: FileState::Pending,
                started: None,
                last: None,
                ended: None,
            })
            .collect();
        let current = files.first_mut().map(|first| {
            first.state = FileState::Running;
            first.started = Some(at);
            0
        });
        Self {
            kind,
            files,
            current,
            outcome: None,
        }
    }

    /// Nature des fichiers.
    pub fn kind(&self) -> ImportKind {
        self.kind
    }

    /// Fichiers suivis.
    pub fn files(&self) -> &[FileRun] {
        &self.files
    }

    /// Issue, `None` tant que l'import tourne.
    pub fn outcome(&self) -> Option<RunOutcome> {
        self.outcome
    }

    /// Rang du fichier désigné par un événement : chemin identique d'abord,
    /// rang annoncé ensuite (le moteur écarte les doublons, ce qui décale
    /// les rangs mais jamais les chemins).
    fn position(&self, file_name: &str, file_index: Option<usize>) -> Option<usize> {
        self.files
            .iter()
            .position(|file| file.path == file_name)
            .or_else(|| {
                file_index
                    .and_then(|index| index.checked_sub(1))
                    .filter(|index| *index < self.files.len())
            })
    }

    /// Fait de `index` le fichier courant à `at` : le précédent se termine à
    /// son dernier événement.
    fn make_current(&mut self, index: usize, at: Instant) {
        if self.current == Some(index) {
            return;
        }
        let mut start = at;
        if let Some(previous) = self.current.and_then(|i| self.files.get_mut(i)) {
            let end = previous.last.unwrap_or(at);
            previous.ended = Some(end);
            start = end;
        }
        if let Some(file) = self.files.get_mut(index) {
            if file.started.is_none() {
                file.started = Some(start);
            }
            if file.state == FileState::Pending {
                file.state = FileState::Running;
            }
            file.ended = None;
        }
        self.current = Some(index);
    }

    /// Applique un événement du moteur. Ignoré une fois l'issue connue, ou
    /// s'il ne désigne aucun fichier de la liste.
    pub fn apply(&mut self, event: &FileEvent) {
        if self.outcome.is_some() {
            return;
        }
        match event {
            FileEvent::Progress {
                file_name,
                file_index,
                at,
            } => {
                let Some(index) = self.position(file_name, Some(*file_index)) else {
                    return;
                };
                self.make_current(index, *at);
                if let Some(file) = self.files.get_mut(index) {
                    file.last = Some(*at);
                }
            }
            FileEvent::Finished {
                file_name,
                report,
                at,
            } => {
                let Some(index) = self.position(file_name, None) else {
                    return;
                };
                let is_current = self.current == Some(index);
                if let Some(file) = self.files.get_mut(index) {
                    if file.started.is_none() {
                        file.started = Some(*at);
                    }
                    file.state = FileState::Done(*report);
                    if is_current || file.ended.is_none() {
                        file.last = Some(*at);
                        if is_current {
                            file.ended = Some(*at);
                        }
                    }
                }
            }
        }
    }

    /// Issue de l'import, au retour de la commande : le fichier en cours
    /// s'arrête (annulé ou en échec), les durées se figent.
    pub fn finish(&mut self, outcome: RunOutcome, at: Instant) {
        if self.outcome.is_some() {
            return;
        }
        self.outcome = Some(outcome);
        for file in &mut self.files {
            if file.started.is_some() && file.ended.is_none() {
                file.ended = Some(at);
            }
            if file.state == FileState::Running {
                file.state = match outcome {
                    // Réussite sans `Finished` pour ce fichier (doublon
                    // écarté par le moteur) : rien à signaler.
                    RunOutcome::Success => FileState::Pending,
                    RunOutcome::Cancelled => FileState::Cancelled,
                    RunOutcome::Failed => FileState::Failed,
                };
            }
        }
        self.current = None;
    }
}

/// Entier groupé par milliers (`1234567` → `1 234 567`, espace insécable).
pub fn format_count(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3 * 2);
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(THOUSANDS_SEPARATOR);
        }
        out.push(digit);
    }
    out
}

/// Durée lisible : `0,4 s`, `12,3 s`, `2 min 05 s`.
pub fn format_duration(elapsed: Duration) -> String {
    let tenths = (elapsed.as_millis() + 50) / 100;
    if tenths < 600 {
        format!("{},{} s", tenths / 10, tenths % 10)
    } else {
        let secs = elapsed.as_secs();
        format!("{} min {:02} s", secs / 60, secs % 60)
    }
}

/// Libellé sous la barre de progression :
/// `Fichier 1/3 · capture.pcap · 1 200/4 500`.
pub fn progress_label(progress: &ImportProgress) -> String {
    format!(
        "Fichier {}/{} · {} · {}/{}",
        progress.file_index,
        progress.files_total,
        progress_file_name_of(Some(progress)),
        format_count(progress.current),
        format_count(progress.total)
    )
}

/// Statut d'un fichier tel qu'affiché à droite de sa ligne ; vide pour un
/// fichier en attente hors import.
pub fn file_status_text(file: &FileRun, running: bool, now: Instant) -> String {
    let elapsed = file.elapsed(now).map(format_duration);
    match file.state {
        FileState::Pending if running => "en attente".to_owned(),
        FileState::Pending => String::new(),
        FileState::Running => match elapsed {
            Some(elapsed) => format!("en cours · {elapsed}"),
            None => "en cours".to_owned(),
        },
        FileState::Done(report) => {
            let mut text = format!("{} lus", format_count(report.read as u64));
            match report.rejected {
                0 => {}
                1 => text.push_str(", 1 rejeté"),
                n => text.push_str(&format!(", {} rejetés", format_count(n as u64))),
            }
            if let Some(elapsed) = elapsed {
                text.push_str(" · ");
                text.push_str(&elapsed);
            }
            text
        }
        FileState::Cancelled => "annulé".to_owned(),
        FileState::Failed => "en échec".to_owned(),
    }
}

/// Événement d'une ouverture de projet, rapporté au fichier `.lunar` :
/// l'import de matrice que compose `open_project` annonce le CSV extrait
/// dans un dossier temporaire, que le badge d'import de la barre de statut
/// afficherait sinon.
pub fn relabel_import_event(event: CaptureEvent, file_name: &str) -> CaptureEvent {
    match event {
        CaptureEvent::Finished {
            packet_total_count,
            integrated_count,
            rejected_truncated_count,
            rejected_unsupported_link_type_count,
            rejected_malformed_count,
            matrix_total_count,
            ..
        } => CaptureEvent::Finished {
            file_name: file_name.to_owned(),
            packet_total_count,
            integrated_count,
            rejected_truncated_count,
            rejected_unsupported_link_type_count,
            rejected_malformed_count,
            matrix_total_count,
        },
        CaptureEvent::ImportProgress {
            file_index,
            files_total,
            current,
            total,
            ..
        } => CaptureEvent::ImportProgress {
            file_name: file_name.to_owned(),
            file_index,
            files_total,
            current,
            total,
        },
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_types::{ImportMode, route_dropped_paths};

    /// Liste de chemins possédés.
    fn owned(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|p| (*p).to_owned()).collect()
    }

    /// Instant de référence décalé de `ms` millisecondes.
    fn at(t0: Instant, ms: u64) -> Instant {
        t0 + Duration::from_millis(ms)
    }

    fn progress(name: &str, index: usize, t: Instant) -> FileEvent {
        FileEvent::Progress {
            file_name: name.to_owned(),
            file_index: index,
            at: t,
        }
    }

    fn finished(name: &str, read: usize, t: Instant) -> FileEvent {
        FileEvent::Finished {
            file_name: name.to_owned(),
            report: FileReport {
                read,
                integrated: read,
                rejected: 0,
            },
            at: t,
        }
    }

    // --- Listes ---

    #[test]
    fn ajout_deduplique_et_signale_le_changement() {
        let mut files = ImportFiles::new();
        assert!(files.add(ImportKind::Captures, &["/a.pcap", "/b.pcap", "/a.pcap"]));
        assert!(!files.add(ImportKind::Captures, &["/b.pcap"]), "déjà listé");
        assert_eq!(files.list(ImportKind::Captures), ["/a.pcap", "/b.pcap"]);
        assert!(files.list(ImportKind::Matrices).is_empty());
    }

    #[test]
    fn depot_reparti_dans_les_deux_listes() {
        let mut files = ImportFiles::new();
        let routing = route_dropped_paths(&["/m.csv", "/c.pcapng", "/x.txt"], ImportMode::Pcap);
        assert!(files.add_dropped(&routing));
        assert_eq!(files.list(ImportKind::Captures), ["/c.pcapng"]);
        assert_eq!(files.list(ImportKind::Matrices), ["/m.csv"]);
        assert!(!files.add_dropped(&routing), "redéposer ne change rien");
        assert!(!files.add_dropped(&DroppedPathsRouting::Ignored));
        assert!(!files.add_dropped(&DroppedPathsRouting::Csv {
            path: "/l.csv".to_owned()
        }));
    }

    #[test]
    fn retrait_et_effacement() {
        let mut files = ImportFiles::new();
        files.add(ImportKind::Matrices, &["/1.csv", "/2.xlsx", "/3.csv"]);
        assert_eq!(
            files.remove(ImportKind::Matrices, 1).as_deref(),
            Some("/2.xlsx")
        );
        assert_eq!(files.remove(ImportKind::Matrices, 5), None);
        assert_eq!(files.list(ImportKind::Matrices), ["/1.csv", "/3.csv"]);
        files.clear(ImportKind::Matrices);
        assert!(files.list(ImportKind::Matrices).is_empty());
    }

    // --- Suivi par fichier ---

    #[test]
    fn captures_duree_de_chaque_fichier() {
        let t0 = Instant::now();
        let paths = owned(&["/a.pcap", "/b.pcap"]);
        let mut run = ImportRun::start(ImportKind::Captures, &paths, t0);
        assert_eq!(run.files()[0].state, FileState::Running);
        assert_eq!(run.files()[1].state, FileState::Pending);

        run.apply(&progress("/a.pcap", 1, at(t0, 100)));
        run.apply(&finished("/a.pcap", 10, at(t0, 400)));
        run.apply(&progress("/b.pcap", 2, at(t0, 450)));
        run.apply(&finished("/b.pcap", 20, at(t0, 1000)));
        run.finish(RunOutcome::Success, at(t0, 1100));

        let [a, b] = run.files() else {
            panic!("deux fichiers attendus")
        };
        assert!(matches!(a.state, FileState::Done(r) if r.read == 10));
        assert_eq!(a.elapsed(at(t0, 5000)), Some(Duration::from_millis(400)));
        assert!(matches!(b.state, FileState::Done(r) if r.read == 20));
        // Le second commence quand le premier cède la place (à son Finished).
        assert_eq!(b.elapsed(at(t0, 5000)), Some(Duration::from_millis(600)));
        assert_eq!(run.outcome(), Some(RunOutcome::Success));
    }

    #[test]
    fn matrices_lues_puis_fusionnees() {
        let t0 = Instant::now();
        let paths = owned(&["/1.csv", "/2.csv"]);
        let mut run = ImportRun::start(ImportKind::Matrices, &paths, t0);
        run.apply(&progress("/1.csv", 1, at(t0, 200)));
        run.apply(&progress("/2.csv", 2, at(t0, 500)));
        run.apply(&finished("/1.csv", 30, at(t0, 900)));
        run.apply(&finished("/2.csv", 40, at(t0, 900)));
        run.finish(RunOutcome::Success, at(t0, 950));
        let files = run.files();
        assert_eq!(
            files[0].elapsed(at(t0, 2000)),
            Some(Duration::from_millis(200))
        );
        assert_eq!(
            files[1].elapsed(at(t0, 2000)),
            Some(Duration::from_millis(700))
        );
        assert!(matches!(files[0].state, FileState::Done(r) if r.read == 30));
    }

    #[test]
    fn annulation_marque_le_fichier_courant() {
        let t0 = Instant::now();
        let paths = owned(&["/a.pcap", "/b.pcap", "/c.pcap"]);
        let mut run = ImportRun::start(ImportKind::Captures, &paths, t0);
        run.apply(&finished("/a.pcap", 5, at(t0, 100)));
        run.apply(&progress("/b.pcap", 2, at(t0, 150)));
        run.finish(RunOutcome::Cancelled, at(t0, 300));
        let states: Vec<FileState> = run.files().iter().map(|f| f.state).collect();
        assert!(matches!(states[0], FileState::Done(_)));
        assert_eq!(states[1], FileState::Cancelled);
        assert_eq!(states[2], FileState::Pending);
        assert_eq!(
            run.files()[1].elapsed(at(t0, 9999)),
            Some(Duration::from_millis(200))
        );
        assert_eq!(run.files()[2].elapsed(at(t0, 9999)), None);
        // Un retardataire après l'issue est ignoré.
        run.apply(&finished("/b.pcap", 1, at(t0, 400)));
        assert_eq!(run.files()[1].state, FileState::Cancelled);
    }

    #[test]
    fn echec_et_rang_de_secours() {
        let t0 = Instant::now();
        let paths = owned(&["/a.pcap", "/b.pcap"]);
        let mut run = ImportRun::start(ImportKind::Captures, &paths, t0);
        // Chemin inconnu : le rang annoncé désigne le fichier.
        run.apply(&progress("/autre/b.pcap", 2, at(t0, 50)));
        assert_eq!(run.files()[1].state, FileState::Running);
        // Inconnu et hors rang : ignoré.
        run.apply(&progress("/z.pcap", 9, at(t0, 60)));
        run.apply(&finished("/z.pcap", 1, at(t0, 70)));
        run.finish(RunOutcome::Failed, at(t0, 80));
        assert_eq!(run.files()[1].state, FileState::Failed);
        assert_eq!(run.kind(), ImportKind::Captures);
    }

    #[test]
    fn reduction_des_evenements_du_moteur() {
        let t0 = Instant::now();
        let finished_event = CaptureEvent::Finished {
            file_name: "/a.pcap".into(),
            packet_total_count: 100,
            integrated_count: 95,
            rejected_truncated_count: 3,
            rejected_unsupported_link_type_count: 1,
            rejected_malformed_count: 1,
            matrix_total_count: 12,
        };
        assert_eq!(
            file_event(&finished_event, t0),
            Some(FileEvent::Finished {
                file_name: "/a.pcap".into(),
                report: FileReport {
                    read: 100,
                    integrated: 95,
                    rejected: 5
                },
                at: t0,
            })
        );
        let progress_event = CaptureEvent::ImportProgress {
            file_name: "/a.pcap".into(),
            file_index: 1,
            files_total: 2,
            current: 3,
            total: 4,
        };
        assert_eq!(
            file_event(&progress_event, t0),
            Some(progress("/a.pcap", 1, t0))
        );
        let started = CaptureEvent::Stopped {
            session_id: 1,
            reason: "fin".into(),
        };
        assert_eq!(file_event(&started, t0), None);
    }

    // --- Libellés ---

    #[test]
    fn milliers_separes() {
        assert_eq!(format_count(0), "0");
        assert_eq!(format_count(999), "999");
        assert_eq!(format_count(1000), "1\u{a0}000");
        assert_eq!(format_count(1_234_567), "1\u{a0}234\u{a0}567");
        assert_eq!(format_count(12_345), "12\u{a0}345");
    }

    #[test]
    fn durees() {
        assert_eq!(format_duration(Duration::ZERO), "0,0 s");
        assert_eq!(format_duration(Duration::from_millis(420)), "0,4 s");
        assert_eq!(format_duration(Duration::from_millis(450)), "0,5 s");
        assert_eq!(format_duration(Duration::from_millis(12_300)), "12,3 s");
        assert_eq!(format_duration(Duration::from_secs(125)), "2 min 05 s");
    }

    #[test]
    fn libelle_de_progression() {
        let progress = ImportProgress {
            file_name: "/captures/reseau.pcap".to_owned(),
            file_index: 2,
            files_total: 3,
            current: 1200,
            total: 45_000,
        };
        assert_eq!(
            progress_label(&progress),
            "Fichier 2/3 · reseau.pcap · 1\u{a0}200/45\u{a0}000"
        );
    }

    #[test]
    fn statuts_affiches() {
        let t0 = Instant::now();
        let paths = owned(&["/a.pcap", "/b.pcap"]);
        let mut run = ImportRun::start(ImportKind::Captures, &paths, t0);
        let now = at(t0, 1500);
        assert_eq!(
            file_status_text(&run.files()[0], true, now),
            "en cours · 1,5 s"
        );
        assert_eq!(file_status_text(&run.files()[1], true, now), "en attente");
        assert_eq!(file_status_text(&run.files()[1], false, now), "");
        run.apply(&FileEvent::Finished {
            file_name: "/a.pcap".into(),
            report: FileReport {
                read: 2000,
                integrated: 1997,
                rejected: 3,
            },
            at: at(t0, 2000),
        });
        assert_eq!(
            file_status_text(&run.files()[0], true, now),
            "2\u{a0}000 lus, 3 rejetés · 2,0 s"
        );
        run.finish(RunOutcome::Failed, at(t0, 2100));
        assert_eq!(file_status_text(&run.files()[1], false, now), "");
    }

    #[test]
    fn projet_rapporte_au_fichier_lunar() {
        let event = CaptureEvent::Finished {
            file_name: "/tmp/lunar-projet-123/matrice.csv".into(),
            packet_total_count: 30,
            integrated_count: 30,
            rejected_truncated_count: 0,
            rejected_unsupported_link_type_count: 0,
            rejected_malformed_count: 0,
            matrix_total_count: 30,
        };
        match relabel_import_event(event, "/releves/audit.lunar") {
            CaptureEvent::Finished {
                file_name,
                packet_total_count,
                matrix_total_count,
                ..
            } => {
                assert_eq!(file_name, "/releves/audit.lunar");
                assert_eq!((packet_total_count, matrix_total_count), (30, 30));
            }
            other => panic!("Finished attendu : {other:?}"),
        }
        let progress = CaptureEvent::ImportProgress {
            file_name: "/tmp/x/matrice.csv".into(),
            file_index: 1,
            files_total: 1,
            current: 30,
            total: 30,
        };
        assert!(matches!(
            relabel_import_event(progress, "/p.lunar"),
            CaptureEvent::ImportProgress { file_name, .. } if file_name == "/p.lunar"
        ));
        let other = CaptureEvent::Stopped {
            session_id: 2,
            reason: "x".into(),
        };
        assert!(matches!(
            relabel_import_event(other, "/p.lunar"),
            CaptureEvent::Stopped { session_id: 2, .. }
        ));
    }
}
