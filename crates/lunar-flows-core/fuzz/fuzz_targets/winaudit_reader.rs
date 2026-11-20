//! Cible de fuzzing : conversion de relevés WinAudit hostiles en labels.
#![no_main]

use libfuzzer_sys::fuzz_target;
use lunar_flows_core::winaudit::{WinAuditOptions, labels_from_surveys};
use std::path::PathBuf;

/// Noms donnés à la même entrée. Les deux premiers suivent la convention
/// `AAAAMMJJ_CLASSIF_CONF_TRI-BI-NNNNNN.csv` avec deux identifiants et deux
/// dates différents ; le troisième ne porte pas d'identifiant.
const SURVEY_NAMES: [&str; 3] = [
    "20260924_NP_CONF_TST-PC-000001.csv",
    "20260925_NP_CONF_TST-PC-000002.csv",
    "poste-sans-id.csv",
];

/// Répertoire de travail propre au processus libFuzzer, retiré après chaque
/// entrée.
struct FuzzSurveys(PathBuf);

impl FuzzSurveys {
    fn for_current_process() -> Self {
        Self(std::env::temp_dir().join(format!("lunar-winaudit-fuzz-{}", std::process::id())))
    }
}

impl Drop for FuzzSurveys {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// Même chaîne que l'import de labels de l'application : parcours du
// répertoire, décodage (UTF-16 avec BOM, UTF-8), détection du séparateur,
// rubriques lues par position, filtrage des adaptateurs, arbitrage entre
// relevés. La même entrée est écrite sous trois noms : les relevés décrivent
// alors les mêmes interfaces sous des labels différents, ce qui exerce aussi
// l'arbitrage des conflits. Les deux réglages de `keep_virtual` sont joués.
// Un relevé illisible est un résultat normal ; seuls un panic, un crash ou un
// blocage intéressent le fuzzer.
fuzz_target!(|data: &[u8]| {
    let surveys = FuzzSurveys::for_current_process();
    if std::fs::create_dir_all(&surveys.0).is_err() {
        return;
    }
    for name in SURVEY_NAMES {
        if std::fs::write(surveys.0.join(name), data).is_err() {
            return;
        }
    }

    let inputs = std::slice::from_ref(&surveys.0);
    for keep_virtual in [false, true] {
        let _ = labels_from_surveys(inputs, WinAuditOptions { keep_virtual });
    }
});
