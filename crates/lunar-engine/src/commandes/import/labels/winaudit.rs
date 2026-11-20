//! Import de relevés de configuration WinAudit comme source de labels.
//!
//! La conversion est celle du cœur,
//! [`lunar_flows_core::winaudit::labels_from_surveys`] : un même lot de
//! relevés donne toujours les mêmes labels.
//!
//! Contrairement à l'import CSV, qui remplace le store, un import de relevés
//! **ajoute** ses labels : WinAudit ne couvre que les postes et serveurs, et
//! les équipements industriels nommés à la main ne doivent pas disparaître.
//! Un couple (MAC, IP) déjà nommé autrement n'est jamais écrasé : le label en
//! place reste appliqué et le conflit part à l'arbitrage, que l'utilisateur
//! tranche. Même règle quand deux relevés du lot nomment différemment le même
//! couple.

use crate::ipc::Channel;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use log::info;
use lunar_flows_core::winaudit::{
    WinAuditConflict, WinAuditLabel, WinAuditOptions, labels_from_surveys,
};

use crate::{
    errors::CaptureStateError,
    events::CaptureEvent,
    state::{
        capture::CaptureState,
        flow_matrix::FlowMatrix,
        graph::GraphData,
        labels_list::{LabelStore, PcInfoLabel},
    },
};

use super::super::event_channel;
use super::commands::{resync_labels_and_notify, send_label_import_started};
use super::conflicts::{LabelChoice, LabelConflictReport, LabelConflictStore};
use super::validation::normalize_label_key;

/// Provenance affichée à l'arbitrage pour le label déjà présent dans le store.
const EXISTING_SOURCE: &str = "référentiel actuel";

/// Bilan d'un import de relevés WinAudit, rendu au front.
#[derive(Debug, Clone, serde::Serialize)]
pub struct WinAuditImportReport {
    /// Relevés lus avec succès.
    pub files_read: usize,
    /// Relevés écartés : (fichier, raison).
    pub files_rejected: Vec<(String, String)>,
    /// Machines distinctes rencontrées dans les relevés.
    pub machines: usize,
    /// Adaptateurs réseau rencontrés, avant tout filtrage.
    pub adapters: usize,
    /// Adaptateurs à MAC localement administrée, gardés ou non.
    pub virtual_adapters: usize,
    /// Adaptateurs virtuels effectivement écartés (tous, sauf si
    /// `exclude_virtual` est faux).
    pub skipped_virtual: usize,
    /// Adaptateurs de boucle locale écartés.
    pub skipped_loopback: usize,
    /// Adaptateurs sans adresse MAC écartés.
    pub skipped_without_mac: usize,
    /// Adaptateurs à MAC de groupe (multicast, diffusion).
    pub skipped_group_mac: usize,
    /// Adresses écartées : boucle locale, non spécifiées, multicast,
    /// diffusion ou illisibles.
    pub skipped_addresses: usize,
    /// Relevés sans identifiant SALP dans leur nom : label = nom d'hôte seul.
    pub files_without_identifier: Vec<String>,
    /// Relevés lus sans rubrique `2600` (collecte antérieure à l'inventaire
    /// réseau) : aucun label possible.
    pub files_without_network_section: Vec<String>,
    /// Lignes trop courtes pour leur rubrique : comptées, jamais fatales.
    pub malformed_rows: usize,
    /// Labels tirés des relevés, un par couple (MAC, IP), lignes de repli à
    /// IP vide comprises.
    pub labels_found: usize,
    /// Couples jusqu'ici sans label, ajoutés au référentiel.
    pub added: usize,
    /// Couples déjà nommés à l'identique.
    pub unchanged: usize,
    /// Couples refusés par la normalisation (MAC ou IP invalide).
    pub invalid: Vec<String>,
    /// Nœuds du graphe renommés par l'import.
    pub applied: usize,
    /// Conflits nés de cet import.
    pub new_conflicts: usize,
    /// Tous les conflits en attente d'arbitrage, ceux d'avant compris.
    pub conflicts: Vec<LabelConflictReport>,
}

/// Importe un ou plusieurs relevés WinAudit et **ajoute** leurs labels au
/// store. Un label déjà présent n'est jamais écrasé : tout désaccord, avec le
/// référentiel ou entre relevés, est rendu à l'arbitrage.
#[allow(clippy::too_many_arguments)]
pub fn import_winaudit_surveys(
    paths: Vec<String>,
    exclude_virtual: bool,
    label_store: &Arc<Mutex<LabelStore>>,
    pcinfo: &Arc<Mutex<PcInfoLabel>>,
    matrix: &Arc<Mutex<FlowMatrix>>,
    graph: &Arc<Mutex<GraphData>>,
    capture_state: &Arc<Mutex<CaptureState>>,
    conflict_store: &Arc<Mutex<LabelConflictStore>>,
    on_event: Channel<CaptureEvent>,
) -> Result<WinAuditImportReport, CaptureStateError> {
    let on_event = event_channel(capture_state, on_event)?;
    let inputs: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();

    // Conversion d'abord : une erreur (aucun relevé, chemin absent) laisse le
    // store intact. Un relevé illisible, lui, est rapporté sans bloquer le lot.
    let (labels, survey) = labels_from_surveys(
        &inputs,
        WinAuditOptions {
            keep_virtual: !exclude_virtual,
        },
    )?;

    send_label_import_started(&on_event);

    let merge = merge_survey_labels(label_store.lock()?.get(), &labels, &survey.conflict_details);
    let added = merge.to_add.len();

    let applied = if added == 0 {
        0
    } else {
        {
            let mut store = label_store.lock()?;
            for row in merge.to_add {
                store.add(row);
            }
        }
        resync_labels_and_notify(label_store, pcinfo, matrix, graph, &on_event)?
    };

    let new_conflicts = merge.conflicts.len();
    let conflicts = {
        let mut store = conflict_store.lock()?;
        merge_pending(&mut store.conflicts, merge.conflicts);
        store.conflicts.clone()
    };

    // Des labels ont été ajoutés : relevé modifié, marqué hors des
    // verrous de données.
    if added > 0 {
        capture_state.lock()?.mark_dirty();
    }

    info!(
        "[import_winaudit_surveys] {} relevé(s) lu(s), {} label(s) ajouté(s), {} déjà présent(s), {} conflit(s) à arbitrer, {} nœud(s) mis à jour dans le graphe",
        survey.files_read, added, merge.unchanged, new_conflicts, applied
    );

    Ok(WinAuditImportReport {
        files_read: survey.files_read,
        files_rejected: survey
            .files_rejected
            .iter()
            .map(|(path, reason)| (path.display().to_string(), reason.clone()))
            .collect(),
        machines: survey.machines,
        adapters: survey.adapters,
        virtual_adapters: survey.virtual_adapters,
        skipped_virtual: survey.skipped_virtual,
        skipped_loopback: survey.skipped_loopback,
        skipped_without_mac: survey.skipped_without_mac,
        skipped_group_mac: survey.skipped_group_mac,
        skipped_addresses: survey.skipped_addresses,
        files_without_identifier: survey
            .files_without_identifier
            .iter()
            .map(|path| path.display().to_string())
            .collect(),
        files_without_network_section: survey
            .files_without_network_section
            .iter()
            .map(|path| path.display().to_string())
            .collect(),
        malformed_rows: survey.malformed_rows,
        labels_found: survey.labels,
        added,
        unchanged: merge.unchanged,
        invalid: merge.invalid,
        applied,
        new_conflicts,
        conflicts,
    })
}

/// Ce que l'import fait des labels tirés des relevés, au vu du store actuel.
#[derive(Debug, Default)]
struct SurveyMerge {
    /// Couples jusqu'ici sans label : à ajouter au store.
    to_add: Vec<(String, String, String)>,
    /// Déjà nommés à l'identique : rien à faire.
    unchanged: usize,
    /// Refusés par la normalisation.
    invalid: Vec<String>,
    /// Désaccords à arbitrer, avec la provenance de chaque nom.
    conflicts: Vec<LabelConflictReport>,
}

/// Confronte les labels des relevés au store. Les clés sont normalisées
/// comme à l'import CSV, pour tomber sur celles de la matrice.
fn merge_survey_labels(
    existing: &[(String, String, String)],
    labels: &[WinAuditLabel],
    batch_conflicts: &[WinAuditConflict],
) -> SurveyMerge {
    let current: HashMap<(&str, &str), &str> = existing
        .iter()
        .map(|(mac, ip, label)| ((mac.as_str(), ip.as_str()), label.as_str()))
        .collect();
    let mut merge = SurveyMerge::default();
    // Clés ajoutées par ce lot, avec la provenance du nom retenu : deux
    // écritures d'une même adresse (IPv6 étendue et compressée) ne font
    // qu'une clé une fois normalisées.
    let mut added: HashMap<(String, String), (String, String)> = HashMap::new();
    let mut conflicts: BTreeMap<(String, String), LabelConflictReport> = BTreeMap::new();

    for label in labels {
        let Ok((mac, ip)) = normalize_label_key(&label.mac, &label.ip) else {
            merge.invalid.push(key_text(&label.mac, &label.ip));
            continue;
        };
        let candidate = choice(&label.label, survey_source(&label.source));

        if let Some(existing_label) = current.get(&(mac.as_str(), ip.as_str())) {
            if *existing_label == label.label {
                merge.unchanged += 1;
            } else {
                let kept = choice(existing_label, EXISTING_SOURCE.to_string());
                push_candidate(&mut conflicts, mac, ip, kept, candidate);
            }
            continue;
        }

        match added.get(&(mac.clone(), ip.clone())) {
            Some((kept_label, _)) if *kept_label == label.label => merge.unchanged += 1,
            Some((kept_label, kept_source)) => {
                let kept = choice(kept_label, kept_source.clone());
                push_candidate(&mut conflicts, mac, ip, kept, candidate);
            }
            None => {
                added.insert(
                    (mac.clone(), ip.clone()),
                    (label.label.clone(), candidate.2),
                );
                merge.to_add.push((mac, ip, label.label.clone()));
            }
        }
    }

    // Désaccords entre relevés du lot : le nom retenu par le cœur, celui du
    // relevé le plus récent, a été traité plus haut (ajouté, déjà présent, ou
    // déjà en conflit avec le store) ; les autres rejoignent l'arbitrage à
    // côté du nom appliqué.
    for conflict in batch_conflicts {
        let Ok((mac, ip)) = normalize_label_key(&conflict.mac, &conflict.ip) else {
            continue;
        };
        let kept = match current.get(&(mac.as_str(), ip.as_str())) {
            Some(existing_label) => choice(existing_label, EXISTING_SOURCE.to_string()),
            None => choice(&conflict.kept.label, survey_source(&conflict.kept.source)),
        };
        for dropped in &conflict.dropped {
            let candidate = choice(&dropped.label, survey_source(&dropped.source));
            push_candidate(
                &mut conflicts,
                mac.clone(),
                ip.clone(),
                kept.clone(),
                candidate,
            );
        }
    }

    merge.conflicts = conflicts.into_values().collect();
    merge
}

/// Ajoute `candidate` au conflit de la clé, créé avec `kept` s'il n'existe
/// pas encore. Un même candidat (label et provenance) n'est listé qu'une fois.
fn push_candidate(
    conflicts: &mut BTreeMap<(String, String), LabelConflictReport>,
    mac: String,
    ip: String,
    kept: LabelChoice,
    candidate: LabelChoice,
) {
    let report = conflicts
        .entry((mac.clone(), ip.clone()))
        .or_insert_with(|| LabelConflictReport {
            mac,
            ip,
            kept,
            dropped: Vec::new(),
        });
    if report.kept != candidate && !report.dropped.contains(&candidate) {
        report.dropped.push(candidate);
    }
}

/// Ajoute les conflits de cet import à ceux qui attendaient déjà l'arbitrage
/// (un import CSV précédent, par exemple) : une clé déjà en attente reçoit
/// les nouveaux candidats au lieu d'être dédoublée.
fn merge_pending(pending: &mut Vec<LabelConflictReport>, incoming: Vec<LabelConflictReport>) {
    for conflict in incoming {
        match pending
            .iter_mut()
            .find(|p| p.mac == conflict.mac && p.ip == conflict.ip)
        {
            Some(existing) => {
                for candidate in std::iter::once(conflict.kept).chain(conflict.dropped) {
                    if existing.kept != candidate && !existing.dropped.contains(&candidate) {
                        existing.dropped.push(candidate);
                    }
                }
            }
            None => pending.push(conflict),
        }
    }
}

/// Un candidat d'arbitrage qui ne vient pas d'un fichier CSV : pas de numéro
/// de ligne (0), la provenance occupe le troisième champ.
fn choice(label: &str, source: String) -> LabelChoice {
    (0, label.to_string(), source)
}

/// Provenance affichée à l'arbitrage pour un label tiré d'un relevé :
/// « relevé `nom de fichier` » (chemin complet si le chemin n'a pas de nom).
fn survey_source(path: &Path) -> String {
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    format!("relevé {name}")
}

/// Rend un couple (MAC, IP) lisible dans la liste des clés invalides du
/// bilan, un champ vide étant nommé explicitement.
fn key_text(mac: &str, ip: &str) -> String {
    format!(
        "{} / {}",
        if mac.is_empty() { "(sans MAC)" } else { mac },
        if ip.is_empty() { "(sans IP)" } else { ip }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use lunar_flows_core::winaudit::WinAuditCandidate;

    fn from_survey(mac: &str, ip: &str, label: &str, source: &str) -> WinAuditLabel {
        WinAuditLabel {
            mac: mac.to_string(),
            ip: ip.to_string(),
            label: label.to_string(),
            source: PathBuf::from(source),
        }
    }

    fn stored(mac: &str, ip: &str, label: &str) -> (String, String, String) {
        (mac.to_string(), ip.to_string(), label.to_string())
    }

    fn candidate(label: &str, source: &str) -> WinAuditCandidate {
        WinAuditCandidate {
            label: label.to_string(),
            source: PathBuf::from(source),
        }
    }

    #[test]
    fn un_couple_encore_sans_label_est_ajoute() {
        let merge = merge_survey_labels(
            &[stored("aa:bb:cc:dd:ee:ff", "10.0.0.9", "automate-ligne-2")],
            &[from_survey(
                "38:f3:ab:cf:ba:01",
                "10.0.0.1",
                "poste-01",
                "/r/poste-01.csv",
            )],
            &[],
        );

        assert_eq!(
            merge.to_add,
            vec![stored("38:f3:ab:cf:ba:01", "10.0.0.1", "poste-01")]
        );
        assert_eq!(merge.unchanged, 0);
        assert!(merge.conflicts.is_empty());
    }

    #[test]
    fn un_couple_deja_nomme_a_l_identique_ne_change_rien() {
        let merge = merge_survey_labels(
            &[stored("38:f3:ab:cf:ba:01", "10.0.0.1", "poste-01")],
            &[from_survey(
                "38:f3:ab:cf:ba:01",
                "10.0.0.1",
                "poste-01",
                "/r/poste-01.csv",
            )],
            &[],
        );

        assert!(merge.to_add.is_empty());
        assert_eq!(merge.unchanged, 1);
        assert!(merge.conflicts.is_empty());
    }

    #[test]
    fn un_label_en_place_n_est_jamais_ecrase_le_desaccord_part_a_l_arbitrage() {
        let merge = merge_survey_labels(
            &[stored("38:f3:ab:cf:ba:01", "10.0.0.1", "poste-compta")],
            &[from_survey(
                "38:f3:ab:cf:ba:01",
                "10.0.0.1",
                "poste-01",
                "/r/poste-01.csv",
            )],
            &[],
        );

        assert!(merge.to_add.is_empty(), "le label en place reste appliqué");
        assert_eq!(merge.conflicts.len(), 1);
        let conflict = &merge.conflicts[0];
        assert_eq!(
            conflict.kept,
            (0, "poste-compta".to_string(), EXISTING_SOURCE.to_string())
        );
        assert_eq!(
            conflict.dropped,
            vec![(0, "poste-01".to_string(), "relevé poste-01.csv".to_string())]
        );
    }

    #[test]
    fn les_cles_sont_normalisees_comme_a_l_import_csv() {
        // MAC en capitales, IPv6 écrite en long : mêmes clés que le store une
        // fois normalisées, donc pas de doublon ni de faux conflit.
        let merge = merge_survey_labels(
            &[stored(
                "38:f3:ab:cf:ba:92",
                "fe80::13d7:5125:5bff:ec96",
                "poste-atelier-01",
            )],
            &[from_survey(
                "38:F3:AB:CF:BA:92",
                "FE80:0000:0000:0000:13D7:5125:5BFF:EC96",
                "poste-atelier-01",
                "/r/a.csv",
            )],
            &[],
        );

        assert_eq!(merge.unchanged, 1);
        assert!(merge.to_add.is_empty());
    }

    #[test]
    fn une_adresse_que_la_normalisation_refuse_est_comptee_pas_appliquee() {
        let merge = merge_survey_labels(
            &[],
            &[from_survey(
                "38:f3:ab:cf:ba:01",
                "fe80::1%eth0",
                "poste-01",
                "/r/a.csv",
            )],
            &[],
        );

        assert!(merge.to_add.is_empty());
        assert_eq!(
            merge.invalid,
            vec!["38:f3:ab:cf:ba:01 / fe80::1%eth0".to_string()]
        );
    }

    #[test]
    fn deux_releves_en_desaccord_vont_a_l_arbitrage_avec_leur_provenance() {
        // Le cœur a gardé le nom du relevé le plus récent ; l'autre attend
        // l'arbitrage.
        let merge = merge_survey_labels(
            &[],
            &[from_survey(
                "38:f3:ab:cf:ba:01",
                "10.0.0.1",
                "nom-inventaire",
                "/r/a.csv",
            )],
            &[WinAuditConflict {
                mac: "38:f3:ab:cf:ba:01".to_string(),
                ip: "10.0.0.1".to_string(),
                kept: candidate("nom-inventaire", "/r/a.csv"),
                dropped: vec![candidate("nom-renomme", "/r/b.csv")],
            }],
        );

        assert_eq!(
            merge.to_add,
            vec![stored("38:f3:ab:cf:ba:01", "10.0.0.1", "nom-inventaire")]
        );
        assert_eq!(merge.conflicts.len(), 1);
        let conflict = &merge.conflicts[0];
        assert_eq!(conflict.kept.2, "relevé a.csv");
        assert_eq!(
            conflict.dropped,
            vec![(0, "nom-renomme".to_string(), "relevé b.csv".to_string())]
        );
    }

    #[test]
    fn desaccord_avec_le_referentiel_et_entre_releves_forme_un_seul_conflit() {
        let merge = merge_survey_labels(
            &[stored("38:f3:ab:cf:ba:01", "10.0.0.1", "poste-compta")],
            &[from_survey(
                "38:f3:ab:cf:ba:01",
                "10.0.0.1",
                "nom-inventaire",
                "/r/a.csv",
            )],
            &[WinAuditConflict {
                mac: "38:f3:ab:cf:ba:01".to_string(),
                ip: "10.0.0.1".to_string(),
                kept: candidate("nom-inventaire", "/r/a.csv"),
                dropped: vec![candidate("nom-renomme", "/r/b.csv")],
            }],
        );

        assert!(merge.to_add.is_empty());
        assert_eq!(merge.conflicts.len(), 1, "une clé, un seul arbitrage");
        let conflict = &merge.conflicts[0];
        assert_eq!(conflict.kept.1, "poste-compta");
        let labels: Vec<&str> = conflict.dropped.iter().map(|c| c.1.as_str()).collect();
        assert_eq!(labels, vec!["nom-inventaire", "nom-renomme"]);
    }

    #[test]
    fn un_conflit_deja_en_attente_recoit_les_nouveaux_candidats() {
        let mut pending = vec![LabelConflictReport {
            mac: "38:f3:ab:cf:ba:01".to_string(),
            ip: "10.0.0.1".to_string(),
            kept: (
                3,
                "poste-compta".to_string(),
                "38:f3:ab:cf:ba:01,10.0.0.1,poste-compta".to_string(),
            ),
            dropped: vec![(
                7,
                "compta-2".to_string(),
                "38:f3:ab:cf:ba:01,10.0.0.1,compta-2".to_string(),
            )],
        }];
        let incoming = vec![
            LabelConflictReport {
                mac: "38:f3:ab:cf:ba:01".to_string(),
                ip: "10.0.0.1".to_string(),
                kept: choice("poste-compta", EXISTING_SOURCE.to_string()),
                dropped: vec![choice("poste-01", "relevé a.csv".to_string())],
            },
            LabelConflictReport {
                mac: "38:f3:ab:cf:ba:02".to_string(),
                ip: "10.0.0.2".to_string(),
                kept: choice("poste-02", EXISTING_SOURCE.to_string()),
                dropped: vec![choice("poste-deux", "relevé b.csv".to_string())],
            },
        ];

        merge_pending(&mut pending, incoming);

        assert_eq!(
            pending.len(),
            2,
            "la clé déjà en attente n'est pas dédoublée"
        );
        let labels: Vec<&str> = pending[0].dropped.iter().map(|c| c.1.as_str()).collect();
        assert_eq!(labels, vec!["compta-2", "poste-compta", "poste-01"]);
    }

    /// De bout en bout sur un vrai relevé UTF-16 : la conversion du cœur puis
    /// la confrontation au store, comme la commande les enchaîne.
    #[test]
    fn un_releve_utf16_rejoint_les_cles_du_store() {
        use super::super::super::test_support::TempDir;

        let dir = TempDir::new("lunar_test_winaudit_import");
        let mut machine = vec![String::new(); 21];
        machine[0] = "300".to_string();
        machine[2] = "poste-atelier-01".to_string();
        let mut adapter = vec![String::new(); 20];
        adapter[0] = "2600".to_string();
        adapter[3] = "enp0s31f6".to_string();
        adapter[6] = "192.168.1.16 FE80::13D7:5125:5BFF:EC96".to_string();
        adapter[16] = "Ethernet 802.3".to_string();
        adapter[17] = "38:F3:AB:CF:BA:92".to_string();
        let text = format!("{}\n{}\n", machine.join(","), adapter.join(","));
        let mut bytes = vec![0xFF, 0xFE];
        for unit in text.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        // Nom de relevé conventionnel : l'identifiant SALP préfixe le label.
        let survey_path = dir.path().join("20260921_NP_CONF_SSF-WI-100206.csv");
        std::fs::write(&survey_path, bytes).unwrap();

        let (labels, survey) =
            labels_from_surveys(&[survey_path], WinAuditOptions::default()).unwrap();
        let merge = merge_survey_labels(
            &[stored(
                "38:f3:ab:cf:ba:92",
                "192.168.1.16",
                "SSF-WI-100206 poste-atelier-01",
            )],
            &labels,
            &survey.conflict_details,
        );

        assert_eq!(merge.unchanged, 1, "l'IPv4 déjà nommée à l'identique");
        assert_eq!(
            merge.to_add,
            vec![
                // Ligne de repli à IP vide : nomme les flux sans couche 3.
                stored("38:f3:ab:cf:ba:92", "", "SSF-WI-100206 poste-atelier-01"),
                stored(
                    "38:f3:ab:cf:ba:92",
                    "fe80::13d7:5125:5bff:ec96",
                    "SSF-WI-100206 poste-atelier-01"
                ),
            ],
            "la ligne de repli et l'IPv6, sous sa forme normalisée"
        );
    }
}
