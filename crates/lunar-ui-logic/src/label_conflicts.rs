//! Arbitrage des conflits de labels : candidats d'un conflit tels que le
//! dialogue les présente, choix par défaut, arbitrages envoyés au moteur et
//! mises à jour du graphe qui en découlent.
//!
//! Un conflit né d'un import CSV désigne ses candidats par numéro de ligne ;
//! un conflit né d'un import de relevés WinAudit, par leur provenance
//! (« relevé poste-01.csv », « référentiel actuel ») — n° de ligne 0 côté
//! moteur.

use std::collections::HashMap;

use lunar_engine::commandes::import::labels::conflicts::{LabelConflictReport, LabelResolution};
use lunar_flows_core::graph::{GraphData, GraphUpdate};

/// Un label candidat d'un conflit, avec ce qui le propose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictCandidate {
    /// Label proposé (peut être vide).
    pub label: String,
    /// Lignes du fichier CSV qui le portent.
    pub lines: Vec<usize>,
    /// Provenances hors CSV (relevé WinAudit, référentiel actuel), sans
    /// doublon.
    pub sources: Vec<String>,
}

/// Candidats d'un conflit, un par label distinct, dans l'ordre de lecture
/// (le label gardé d'abord).
pub fn conflict_candidates(conflict: &LabelConflictReport) -> Vec<ConflictCandidate> {
    let mut candidates: Vec<ConflictCandidate> = Vec::new();
    for (line, label, origin) in std::iter::once(&conflict.kept).chain(&conflict.dropped) {
        let index = match candidates.iter().position(|c| &c.label == label) {
            Some(index) => index,
            None => {
                candidates.push(ConflictCandidate {
                    label: label.clone(),
                    lines: Vec::new(),
                    sources: Vec::new(),
                });
                candidates.len() - 1
            }
        };
        let candidate = &mut candidates[index];
        if *line > 0 {
            candidate.lines.push(*line);
        } else if !candidate.sources.contains(origin) {
            candidate.sources.push(origin.clone());
        }
    }
    candidates
}

/// D'où vient un candidat : « lignes 3, 7 », « relevé a.csv · relevé
/// b.csv »….
pub fn candidate_origin(candidate: &ConflictCandidate) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !candidate.lines.is_empty() {
        let lines: Vec<String> = candidate.lines.iter().map(usize::to_string).collect();
        parts.push(format!(
            "ligne{} {}",
            if candidate.lines.len() > 1 { "s" } else { "" },
            lines.join(", ")
        ));
    }
    parts.extend(candidate.sources.iter().cloned());
    parts.join(" · ")
}

/// Texte d'un candidat dans le dialogue : le label, ou « (vide) ».
pub fn candidate_text(candidate: &ConflictCandidate) -> &str {
    if candidate.label.is_empty() {
        "(vide)"
    } else {
        &candidate.label
    }
}

/// Clé d'un conflit telle que l'affiche sa carte : `mac / ip`, `-` pour un
/// champ vide.
pub fn conflict_key_text(conflict: &LabelConflictReport) -> String {
    format!(
        "{} / {}",
        dash_if_empty(&conflict.mac),
        dash_if_empty(&conflict.ip)
    )
}

/// `-` pour un champ vide (`value || "-"` des gabarits Vue).
pub fn dash_if_empty(value: &str) -> &str {
    if value.is_empty() { "-" } else { value }
}

/// Choix initiaux : le label gardé de chaque conflit.
pub fn default_choices(conflicts: &[LabelConflictReport]) -> Vec<String> {
    conflicts
        .iter()
        .map(|conflict| conflict.kept.1.clone())
        .collect()
}

/// Arbitrages à envoyer au moteur en un seul appel (transactionnel) : un
/// par conflit, avec le label choisi (le label gardé si aucun choix).
pub fn resolutions(conflicts: &[LabelConflictReport], choices: &[String]) -> Vec<LabelResolution> {
    conflicts
        .iter()
        .enumerate()
        .map(|(index, conflict)| LabelResolution {
            mac: conflict.mac.clone(),
            ip: conflict.ip.clone(),
            label: choices
                .get(index)
                .cloned()
                .unwrap_or_else(|| conflict.kept.1.clone()),
        })
        .collect()
}

/// Sous-titre du dialogue d'arbitrage.
pub fn arbitration_subtitle(count: usize) -> String {
    format!(
        "{count} clé(s) (MAC, IP) reçoivent plusieurs labels. Choisis le label à conserver pour chacune."
    )
}

/// Ligne de journal d'un arbitrage appliqué.
pub fn arbitration_log(count: usize) -> String {
    format!("Arbitrage : {count} conflit(s) résolu(s)")
}

/// Mises à jour du graphe entre deux instantanés : un `NodeUpdated` par
/// nœud dont le label a changé, dans l'ordre des identifiants.
///
/// L'arbitrage (`resolve_label_conflicts`) modifie le graphe du moteur mais
/// ne pousse ses mises à jour que sur le canal d'une capture en cours ; hors
/// capture, le graphe affiché garderait l'ancien label. La comparaison des
/// instantanés pris avant et après rend ces mises à jour à
/// l'interface sans toucher au moteur.
pub fn label_updates_between(before: &GraphData, after: &GraphData) -> Vec<GraphUpdate> {
    let previous: HashMap<&str, &Option<String>> = before
        .nodes
        .values()
        .map(|node| (node.id.as_str(), &node.label))
        .collect();
    let mut changed: Vec<_> = after
        .nodes
        .values()
        .filter(|node| previous.get(node.id.as_str()) != Some(&&node.label))
        .collect();
    changed.sort_by(|a, b| a.id.cmp(&b.id));
    changed
        .into_iter()
        .map(|node| GraphUpdate::NodeUpdated(node.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choice(line: usize, label: &str, origin: &str) -> (usize, String, String) {
        (line, label.to_owned(), origin.to_owned())
    }

    fn conflict(
        mac: &str,
        ip: &str,
        kept: (usize, String, String),
        dropped: Vec<(usize, String, String)>,
    ) -> LabelConflictReport {
        LabelConflictReport {
            mac: mac.to_owned(),
            ip: ip.to_owned(),
            kept,
            dropped,
        }
    }

    fn candidate(label: &str, lines: &[usize], sources: &[&str]) -> ConflictCandidate {
        ConflictCandidate {
            label: label.to_owned(),
            lines: lines.to_vec(),
            sources: sources.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    // --- Candidats et arbitrages ---

    #[test]
    fn conflit_csv_candidats_regroupes_par_label_avec_leurs_lignes() {
        let c = conflict(
            "aa:bb:cc:dd:ee:ff",
            "10.0.0.1",
            choice(3, "poste-compta", "aa:bb:cc:dd:ee:ff,10.0.0.1,poste-compta"),
            vec![
                choice(7, "compta-2", "aa:bb:cc:dd:ee:ff,10.0.0.1,compta-2"),
                choice(9, "poste-compta", "aa:bb:cc:dd:ee:ff,10.0.0.1,poste-compta"),
            ],
        );
        let candidates = conflict_candidates(&c);
        assert_eq!(
            candidates,
            [
                candidate("poste-compta", &[3, 9], &[]),
                candidate("compta-2", &[7], &[]),
            ]
        );
        let origins: Vec<String> = candidates.iter().map(candidate_origin).collect();
        assert_eq!(origins, ["lignes 3, 9", "ligne 7"]);
    }

    #[test]
    fn conflit_winaudit_la_provenance_remplace_le_numero_de_ligne() {
        let c = conflict(
            "38:f3:ab:cf:ba:01",
            "10.0.0.1",
            choice(0, "poste-compta", "référentiel actuel"),
            vec![
                choice(0, "nom-inventaire", "relevé a.csv"),
                choice(0, "poste-compta", "relevé b.csv"),
            ],
        );
        let candidates = conflict_candidates(&c);
        assert_eq!(
            candidates,
            [
                candidate("poste-compta", &[], &["référentiel actuel", "relevé b.csv"]),
                candidate("nom-inventaire", &[], &["relevé a.csv"]),
            ]
        );
        let origins: Vec<String> = candidates.iter().map(candidate_origin).collect();
        assert_eq!(
            origins,
            ["référentiel actuel · relevé b.csv", "relevé a.csv"]
        );
    }

    #[test]
    fn conflit_mixte_un_csv_en_attente_rejoint_par_un_releve() {
        let c = conflict(
            "38:f3:ab:cf:ba:01",
            "10.0.0.1",
            choice(3, "poste-compta", "38:f3:ab:cf:ba:01,10.0.0.1,poste-compta"),
            vec![
                choice(0, "poste-compta", "référentiel actuel"),
                choice(0, "poste-01", "relevé a.csv"),
            ],
        );
        let origins: Vec<String> = conflict_candidates(&c)
            .iter()
            .map(candidate_origin)
            .collect();
        assert_eq!(origins, ["ligne 3 · référentiel actuel", "relevé a.csv"]);
    }

    // --- Dialogue d'arbitrage ---

    #[test]
    fn provenance_repetee_comptee_une_fois() {
        let c = conflict(
            "aa:bb:cc:dd:ee:ff",
            "",
            choice(0, "a", "relevé x.csv"),
            vec![choice(0, "a", "relevé x.csv")],
        );
        assert_eq!(
            conflict_candidates(&c),
            [candidate("a", &[], &["relevé x.csv"])]
        );
    }

    #[test]
    fn label_vide_et_cle_partielle_affiches() {
        let c = conflict(
            "aa:bb:cc:dd:ee:ff",
            "",
            choice(2, "", "aa:bb:cc:dd:ee:ff,,"),
            vec![],
        );
        let candidates = conflict_candidates(&c);
        assert_eq!(candidate_text(&candidates[0]), "(vide)");
        assert_eq!(conflict_key_text(&c), "aa:bb:cc:dd:ee:ff / -");
        assert_eq!(dash_if_empty("x"), "x");
    }

    #[test]
    fn choix_par_defaut_le_label_garde_puis_arbitrages() {
        let conflicts = vec![
            conflict(
                "aa:bb:cc:dd:ee:01",
                "10.0.0.1",
                choice(1, "un", ""),
                vec![choice(2, "deux", "")],
            ),
            conflict(
                "aa:bb:cc:dd:ee:02",
                "",
                choice(0, "trois", "référentiel actuel"),
                vec![choice(0, "quatre", "relevé a.csv")],
            ),
        ];
        let mut choices = default_choices(&conflicts);
        assert_eq!(choices, ["un", "trois"]);
        choices[1] = "quatre".to_owned();
        let sent: Vec<(String, String, String)> = resolutions(&conflicts, &choices)
            .into_iter()
            .map(|r| (r.mac, r.ip, r.label))
            .collect();
        assert_eq!(
            sent,
            [
                ("aa:bb:cc:dd:ee:01".into(), "10.0.0.1".into(), "un".into()),
                ("aa:bb:cc:dd:ee:02".into(), String::new(), "quatre".into()),
            ]
        );
        // Choix manquant : le label gardé.
        assert_eq!(resolutions(&conflicts, &[])[1].label, "trois");
    }

    #[test]
    fn textes_du_dialogue() {
        assert_eq!(
            arbitration_subtitle(2),
            "2 clé(s) (MAC, IP) reçoivent plusieurs labels. Choisis le label à conserver pour chacune."
        );
        assert_eq!(arbitration_log(3), "Arbitrage : 3 conflit(s) résolu(s)");
    }

    // --- Mises à jour du graphe après arbitrage ---

    /// Graphe de deux postes IPv4.
    fn graph() -> GraphData {
        let mut graph = GraphData::new();
        for (mac, ip) in [
            ("aa:bb:cc:dd:ee:01", "10.0.0.1"),
            ("aa:bb:cc:dd:ee:02", "10.0.0.2"),
        ] {
            let node = lunar_flows_core::graph::Node::new(
                ip,
                ip.to_owned(),
                mac.to_owned(),
                "#ffffff",
                ip.to_owned(),
                None,
                None,
            );
            graph.nodes.insert(ip.to_owned(), node);
        }
        graph
    }

    #[test]
    fn seuls_les_noeuds_relabellises_sont_rendus() {
        let before = graph();
        let mut after = graph();
        assert!(label_updates_between(&before, &after).is_empty());
        let direct = after.update_node_label("aa:bb:cc:dd:ee:01", "10.0.0.1", "poste".into());
        assert_eq!(direct.len(), 1);
        let updates = label_updates_between(&before, &after);
        assert_eq!(updates.len(), 1);
        match &updates[0] {
            GraphUpdate::NodeUpdated(node) => {
                assert_eq!(node.ip, "10.0.0.1");
                assert_eq!(node.label.as_deref(), Some("poste"));
            }
            other => panic!("mise à jour inattendue : {other:?}"),
        }
    }
}
