//! Relevé WinAudit → fichier de labels.
//!
//! Les fixtures sont écrites dans le même encodage que le collecteur réel —
//! UTF-16LE avec BOM — et avec les mêmes positions de colonnes que les
//! relevés réels, sans quoi ces tests ne prouveraient rien du format attendu.

#![allow(clippy::expect_used)]

use std::path::{Path, PathBuf};

use lunar_flows_core::winaudit::{
    WinAuditOptions, labels_from_surveys, salp_identifier, write_label_csv,
};

/// Rubrique `2600` : une ligne d'adaptateur, aux positions réelles.
fn network_row(adapter: &str, dns_host: &str, addresses: &str, kind: &str, mac: &str) -> String {
    let mut fields = vec![String::new(); 20];
    fields[0] = "2600".to_string();
    fields[3] = adapter.to_string();
    fields[4] = dns_host.to_string();
    fields[6] = addresses.to_string();
    fields[16] = kind.to_string();
    fields[17] = mac.to_string();
    fields[18] = "up".to_string();
    fields.join(",")
}

/// Rubrique `300` : la machine elle-même.
fn machine_row(name: &str) -> String {
    let mut fields = vec![String::new(); 21];
    fields[0] = "300".to_string();
    fields[2] = name.to_string();
    fields[7] = "Ubuntu 22.04.5 LTS".to_string();
    fields.join(",")
}

/// Rubrique `300` datée : la date du relevé est en colonne 21 de WinAudit.
fn machine_row_at(name: &str, date: &str) -> String {
    let mut fields: Vec<String> = machine_row(name).split(',').map(str::to_string).collect();
    fields[20] = date.to_string();
    fields.join(",")
}

fn utf16le_with_bom(text: &str) -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xFE];
    for unit in text.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    bytes
}

struct Fixtures {
    dir: PathBuf,
}

impl Fixtures {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("lunar-winaudit-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("création du répertoire de fixtures");
        Self { dir }
    }

    fn write(&self, file_name: &str, rows: &[String]) -> PathBuf {
        let path = self.dir.join(file_name);
        let text = format!("{}\n", rows.join("\n"));
        std::fs::write(&path, utf16le_with_bom(&text)).expect("écriture de la fixture");
        path
    }

    fn write_raw(&self, file_name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.dir.join(file_name);
        std::fs::write(&path, bytes).expect("écriture de la fixture");
        path
    }

    fn path(&self) -> &Path {
        &self.dir
    }
}

impl Drop for Fixtures {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn un_releve_donne_un_label_par_adresse_de_la_machine() {
    let fixtures = Fixtures::new("nominal");
    let survey = fixtures.write(
        "20260921_NP_CONF_XXX-YY-000001.csv",
        &[
            machine_row("poste-atelier-01"),
            network_row(
                "enp0s31f6",
                "poste-atelier-01",
                "192.168.1.16 fe80::13d7:5125:5bff:ec96",
                "Ethernet 802.3",
                "38:F3:AB:CF:BA:92",
            ),
        ],
    );

    let (labels, report) =
        labels_from_surveys(&[survey], WinAuditOptions::default()).expect("conversion du relevé");

    assert_eq!(report.files_read, 1);
    assert_eq!(report.machines, 1);
    assert_eq!(report.adapters, 1);
    assert_eq!(
        labels.len(),
        3,
        "une ligne par adresse portée par la MAC, plus la ligne de repli"
    );

    // La MAC est normalisée en minuscules, comme dans le fichier de labels.
    assert!(labels.iter().all(|entry| entry.mac == "38:f3:ab:cf:ba:92"));
    // Label de `winaudit2labels` : identifiant SALP du nom de fichier, puis
    // nom d'hôte de la rubrique 300.
    assert!(
        labels
            .iter()
            .all(|entry| entry.label == "XXX-YY-000001 poste-atelier-01")
    );
    assert!(report.files_without_identifier.is_empty());
    let addresses: Vec<&str> = labels.iter().map(|entry| entry.ip.as_str()).collect();
    assert!(addresses.contains(&"192.168.1.16"));
    assert!(addresses.contains(&"fe80::13d7:5125:5bff:ec96"));
    // Ligne de repli à IP vide : la seule qui nomme un flux sans couche 3.
    assert!(addresses.contains(&""));
}

#[test]
fn les_adaptateurs_virtuels_sont_ecartes_par_defaut_mais_comptes() {
    let fixtures = Fixtures::new("virtuels");
    let survey = fixtures.write(
        "poste.csv",
        &[
            machine_row("poste-dev"),
            // MAC universelle : la vraie carte du poste.
            network_row(
                "wlp0s20f3",
                "poste-dev",
                "192.168.1.16",
                "Wireless 802.11",
                "64:6e:e0:ea:fa:83",
            ),
            // Bit « localement administré » posé : pont Docker, MAC retirée au
            // prochain redémarrage.
            network_row(
                "br-18f56d54f78e",
                "poste-dev",
                "172.19.0.1",
                "Ethernet 802.3",
                "26:4f:b1:67:12:bb",
            ),
        ],
    );

    // Par défaut, comme `winaudit2labels` : l'interface virtuelle
    // est écartée, et le bilan le dit.
    let (labels, report) =
        labels_from_surveys(std::slice::from_ref(&survey), WinAuditOptions::default())
            .expect("conversion du relevé");
    assert_eq!(report.adapters, 2);
    assert_eq!(report.virtual_adapters, 1, "compté pour être visible");
    assert_eq!(report.skipped_virtual, 1);
    assert_eq!(
        labels.len(),
        2,
        "l'adresse et la ligne de repli de la vraie carte"
    );
    assert!(labels.iter().all(|entry| entry.mac == "64:6e:e0:ea:fa:83"));

    // L'utilisateur peut tout garder.
    let (tout, report) = labels_from_surveys(&[survey], WinAuditOptions { keep_virtual: true })
        .expect("conversion du relevé");
    assert_eq!(report.virtual_adapters, 1);
    assert_eq!(report.skipped_virtual, 0);
    assert_eq!(
        tout.len(),
        4,
        "adresse et ligne de repli des deux adaptateurs"
    );
}

#[test]
fn le_bouclage_ne_produit_jamais_de_label() {
    let fixtures = Fixtures::new("bouclage");
    let survey = fixtures.write(
        "poste.csv",
        &[
            machine_row("poste-dev"),
            network_row(
                "lo",
                "poste-dev",
                "127.0.0.1 ::1",
                "Software Loopback",
                "00:00:00:00:00:00",
            ),
        ],
    );

    let (labels, report) =
        labels_from_surveys(&[survey], WinAuditOptions::default()).expect("conversion du relevé");
    assert_eq!(report.skipped_loopback, 1);
    assert!(labels.is_empty());
}

#[test]
fn une_ligne_tronquee_est_comptee_sans_interrompre_le_fichier() {
    // Un lecteur sans garde verrait une ligne trop courte interrompre la
    // conversion du dossier entier. Ici, elle est comptée et le reste du
    // relevé est lu.
    let fixtures = Fixtures::new("tronquee");
    let survey = fixtures.write(
        "poste.csv",
        &[
            machine_row("poste-atelier-02"),
            "2600,,,enp0s31f6".to_string(),
            network_row(
                "wlp0s20f3",
                "poste-atelier-02",
                "192.168.1.17",
                "Wireless 802.11",
                "64:6e:e0:ea:fa:84",
            ),
        ],
    );

    let (labels, report) = labels_from_surveys(&[survey], WinAuditOptions::default())
        .expect("une ligne tronquée n'est pas fatale");
    assert_eq!(report.skipped_without_mac, 1);
    assert_eq!(labels.len(), 2, "l'adresse et la ligne de repli");
    assert!(labels.iter().all(|entry| entry.label == "poste-atelier-02"));
}

#[test]
fn un_fichier_illisible_ne_perd_pas_le_lot() {
    let fixtures = Fixtures::new("lot");
    fixtures.write(
        "a.csv",
        &[
            machine_row("poste-a"),
            network_row(
                "eth0",
                "poste-a",
                "10.0.0.1",
                "Ethernet 802.3",
                "38:f3:ab:cf:ba:01",
            ),
        ],
    );
    // Ni UTF-16, ni CSV WinAudit : rejeté seul.
    fixtures.write_raw("b.csv", &[0xFF, 0xFE, 0x00]);
    fixtures.write(
        "c.csv",
        &[
            machine_row("poste-c"),
            network_row(
                "eth0",
                "poste-c",
                "10.0.0.3",
                "Ethernet 802.3",
                "38:f3:ab:cf:ba:03",
            ),
        ],
    );

    let (labels, report) =
        labels_from_surveys(&[fixtures.path().to_path_buf()], WinAuditOptions::default())
            .expect("le répertoire est parcouru");

    assert_eq!(report.files_read, 2);
    assert_eq!(report.files_rejected.len(), 1);
    assert_eq!(
        labels.len(),
        4,
        "les deux relevés lisibles sont conservés, adresse et ligne de repli"
    );
}

#[test]
fn deux_noms_pour_la_meme_adresse_sont_signales_et_arbitres_par_l_utilisateur() {
    let fixtures = Fixtures::new("conflit");
    fixtures.write(
        "a.csv",
        &[
            machine_row("nom-inventaire"),
            network_row(
                "eth0",
                "nom-inventaire",
                "10.0.0.1",
                "Ethernet 802.3",
                "38:f3:ab:cf:ba:01",
            ),
        ],
    );
    fixtures.write(
        "b.csv",
        &[
            machine_row("nom-renomme"),
            network_row(
                "eth0",
                "nom-renomme",
                "10.0.0.1",
                "Ethernet 802.3",
                "38:f3:ab:cf:ba:01",
            ),
        ],
    );

    let (labels, report) =
        labels_from_surveys(&[fixtures.path().to_path_buf()], WinAuditOptions::default())
            .expect("conversion du lot");

    // Aucune date, ni dans la rubrique 300 ni dans le nom de fichier :
    // horodatages égaux, l'adresse et la ligne de repli portent chacune le
    // premier nom lu.
    assert_eq!(labels.len(), 2);
    assert!(
        labels.iter().all(|entry| entry.label == "nom-inventaire"),
        "le premier lu est gardé"
    );
    assert_eq!(
        report.conflicts.len(),
        2,
        "sur l'adresse et sur la ligne de repli"
    );
    assert!(report.conflicts[0].contains("nom-renomme"));

    // Chaque nom garde sa provenance : c'est ce que l'arbitrage montre.
    assert!(
        labels
            .iter()
            .all(|entry| entry.source == fixtures.path().join("a.csv"))
    );
    assert_eq!(report.conflict_details.len(), 2);
    let conflict = report
        .conflict_details
        .iter()
        .find(|conflict| conflict.ip == "10.0.0.1")
        .expect("conflit sur l'adresse");
    assert_eq!(conflict.mac, "38:f3:ab:cf:ba:01");
    assert_eq!(conflict.kept.label, "nom-inventaire");
    assert_eq!(conflict.kept.source, fixtures.path().join("a.csv"));
    assert_eq!(conflict.dropped.len(), 1);
    assert_eq!(conflict.dropped[0].label, "nom-renomme");
    assert_eq!(conflict.dropped[0].source, fixtures.path().join("b.csv"));
}

#[test]
fn le_releve_le_plus_recent_l_emporte_comme_dans_winaudit2labels() {
    // Lus dans l'ordre des chemins : a, b, puis c. C'est la date de la
    // rubrique 300 qui départage, pas l'ordre de lecture.
    let fixtures = Fixtures::new("recent");
    for (file_name, name, date) in [
        ("a.csv", "nom-du-21", "2026-09-21 08:00:00 +0200"),
        ("b.csv", "nom-du-19", "2026-09-19 08:00:00 +0200"),
        ("c.csv", "nom-du-22", "2026-09-22 08:00:00 +0200"),
    ] {
        fixtures.write(
            file_name,
            &[
                machine_row_at(name, date),
                network_row(
                    "eth0",
                    name,
                    "10.0.0.1",
                    "Ethernet 802.3",
                    "38:f3:ab:cf:ba:01",
                ),
            ],
        );
    }

    let (labels, report) =
        labels_from_surveys(&[fixtures.path().to_path_buf()], WinAuditOptions::default())
            .expect("conversion du lot");
    assert!(
        labels.iter().all(|entry| entry.label == "nom-du-22"),
        "{labels:?}"
    );
    let conflict = report
        .conflict_details
        .iter()
        .find(|conflict| conflict.ip == "10.0.0.1")
        .expect("conflit sur l'adresse");
    assert_eq!(conflict.kept.label, "nom-du-22");
    assert_eq!(conflict.kept.source, fixtures.path().join("c.csv"));
    let dropped: Vec<&str> = conflict
        .dropped
        .iter()
        .map(|candidate| candidate.label.as_str())
        .collect();
    assert_eq!(dropped, vec!["nom-du-19", "nom-du-21"]);
}

#[test]
fn sans_date_dans_la_rubrique_300_la_date_du_nom_de_fichier_departage() {
    let fixtures = Fixtures::new("date-du-nom");
    // Rubrique 300 sans date : horodatage 20260920000000, tiré du nom.
    fixtures.write(
        "20260920_NP_CONF_TST-PC-000010.csv",
        &[
            machine_row("poste"),
            network_row(
                "eth0",
                "poste",
                "10.0.0.1",
                "Ethernet 802.3",
                "38:f3:ab:cf:ba:01",
            ),
        ],
    );
    // Lu ensuite (sous-dossier), daté du 19 par sa rubrique 300 : plus ancien.
    std::fs::create_dir_all(fixtures.path().join("z")).expect("création du sous-dossier");
    fixtures.write(
        "z/20260918_NP_CONF_TST-PC-000011.csv",
        &[
            machine_row_at("poste", "2026-09-19 23:59:59 +0200"),
            network_row(
                "eth0",
                "poste",
                "10.0.0.1",
                "Ethernet 802.3",
                "38:f3:ab:cf:ba:01",
            ),
        ],
    );

    let (labels, _) =
        labels_from_surveys(&[fixtures.path().to_path_buf()], WinAuditOptions::default())
            .expect("conversion du lot");
    assert!(
        labels
            .iter()
            .all(|entry| entry.label == "TST-PC-000010 poste"),
        "{labels:?}"
    );
}

#[test]
fn le_fichier_produit_est_celui_des_labels_et_ne_varie_pas() {
    let fixtures = Fixtures::new("sortie");
    let survey = fixtures.write(
        "20260924_NP_CONF_SSF-WI-100206.csv",
        &[
            machine_row("poste-atelier-03"),
            network_row(
                "eth0",
                "poste-atelier-03",
                "10.0.0.5",
                "Ethernet 802.3",
                "38:f3:ab:cf:ba:05",
            ),
        ],
    );

    let (labels, _) =
        labels_from_surveys(&[survey], WinAuditOptions::default()).expect("conversion du relevé");
    let output = fixtures.dir.join("labels.csv");
    let written = write_label_csv(&labels, &output).expect("écriture du fichier de labels");

    assert_eq!(written, labels.len());
    let content = std::fs::read_to_string(&output).expect("relecture");
    assert_eq!(
        content,
        "mac,ip,label\n\
         38:f3:ab:cf:ba:05,,SSF-WI-100206 poste-atelier-03\n\
         38:f3:ab:cf:ba:05,10.0.0.5,SSF-WI-100206 poste-atelier-03\n"
    );
}

#[test]
fn l_identifiant_salp_vient_du_nom_du_releve() {
    assert_eq!(
        salp_identifier("20260921_NP_CONF_SSF-WI-100206.csv"),
        "SSF-WI-100206"
    );
    assert_eq!(
        salp_identifier("20260921_DR_CONF_ACD-KT-100001.CSV"),
        "ACD-KT-100001"
    );
    // Hors convention : pas d'identifiant, le label se réduit au nom d'hôte.
    assert_eq!(salp_identifier("poste.csv"), "");
    assert_eq!(
        salp_identifier("20260921_NP_CONF_E2E-PC-000001.csv"),
        "",
        "trigramme en lettres seulement"
    );
    assert_eq!(
        salp_identifier("20260921_NP_CONF_SSF-WIN-100206.csv"),
        "",
        "bigramme de deux lettres"
    );
    assert_eq!(
        salp_identifier("20260921_NP_CONF_SSF-WI-10020.csv"),
        "",
        "numéro à six chiffres"
    );
}

#[test]
fn sans_nom_d_hote_le_label_se_reduit_a_l_identifiant_et_sans_rien_le_releve_est_rejete() {
    let fixtures = Fixtures::new("sans-hote");
    let survey = fixtures.write(
        "20260921_NP_CONF_SSF-WI-100207.csv",
        &[network_row(
            "eth0",
            "",
            "10.0.0.7",
            "Ethernet 802.3",
            "38:f3:ab:cf:ba:07",
        )],
    );
    let (labels, _) =
        labels_from_surveys(&[survey], WinAuditOptions::default()).expect("conversion du relevé");
    assert_eq!(labels.len(), 2);
    assert!(labels.iter().all(|entry| entry.label == "SSF-WI-100207"));

    // Ni identifiant ni nom d'hôte : aucun label n'est inventé.
    let anonymous = fixtures.write(
        "anonyme.csv",
        &[network_row(
            "eth0",
            "",
            "10.0.0.8",
            "Ethernet 802.3",
            "38:f3:ab:cf:ba:08",
        )],
    );
    let (labels, report) = labels_from_surveys(&[anonymous], WinAuditOptions::default())
        .expect("le lot n'est pas interrompu");
    assert!(labels.is_empty());
    assert_eq!(report.files_rejected.len(), 1);
}

#[test]
fn un_releve_sans_rubrique_2600_est_lu_et_le_bilan_dit_pourquoi_il_ne_donne_rien() {
    // Collecte antérieure à l'inventaire réseau : la machine est décrite
    // (rubrique 300), ses interfaces non. Ce n'est pas un fichier illisible,
    // mais sans ce signalement le lot rendrait « 0 label » sans explication.
    let fixtures = Fixtures::new("sans-2600");
    let ancien = fixtures.write(
        "20260901_NP_CONF_TST-PC-000002.csv",
        &[machine_row("poste-ancien")],
    );
    let recent = fixtures.write(
        "20260924_NP_CONF_TST-PC-000003.csv",
        &[
            machine_row("poste-recent"),
            network_row(
                "eth0",
                "poste-recent",
                "10.0.0.10",
                "Ethernet 802.3",
                "38:f3:ab:cf:ba:10",
            ),
        ],
    );

    let (labels, report) =
        labels_from_surveys(&[ancien.clone(), recent], WinAuditOptions::default())
            .expect("conversion du lot");

    assert_eq!(report.files_read, 2);
    assert_eq!(report.machines, 2);
    assert!(report.files_rejected.is_empty());
    assert_eq!(report.files_without_network_section, vec![ancien]);
    assert_eq!(
        labels.len(),
        2,
        "le relevé récent donne son adresse et sa ligne de repli"
    );
    assert!(
        labels
            .iter()
            .all(|entry| entry.label == "TST-PC-000003 poste-recent")
    );
}

#[test]
fn mac_et_adresses_prennent_la_forme_canonique() {
    // Relevé au point-virgule, MAC Windows à tirets, IPv6 en capitales avec
    // index de zone ; adresse non spécifiée et multicast à écarter. « Local
    // Area Connection » (Windows XP en anglais) reste une interface physique.
    let fixtures = Fixtures::new("canonique");
    let survey = fixtures.write(
        "poste-win.csv",
        &[
            machine_row("poste-win").replace(',', ";"),
            network_row(
                "Local Area Connection",
                "poste-win",
                "192.168.1.30 FE80:0000::0001%12 0.0.0.0 224.0.0.251",
                "Ethernet 802.3",
                "38-F3-AB-CF-BA-30",
            )
            .replace(',', ";"),
        ],
    );

    let (labels, report) =
        labels_from_surveys(&[survey], WinAuditOptions::default()).expect("conversion du relevé");
    let keys: Vec<(&str, &str)> = labels
        .iter()
        .map(|entry| (entry.mac.as_str(), entry.ip.as_str()))
        .collect();
    assert_eq!(
        keys,
        vec![
            ("38:f3:ab:cf:ba:30", ""),
            ("38:f3:ab:cf:ba:30", "192.168.1.30"),
            ("38:f3:ab:cf:ba:30", "fe80::1"),
        ]
    );
    assert_eq!(report.skipped_addresses, 2, "0.0.0.0 et le multicast");
    assert!(labels.iter().all(|entry| entry.label == "poste-win"));
    assert_eq!(report.files_without_identifier.len(), 1);
}

#[test]
fn une_mac_de_groupe_ne_nomme_aucun_equipement() {
    let fixtures = Fixtures::new("groupe");
    let survey = fixtures.write(
        "poste.csv",
        &[
            machine_row("poste-g"),
            network_row(
                "eth0",
                "poste-g",
                "10.0.0.9",
                "Ethernet 802.3",
                "01:00:5e:00:00:fb",
            ),
        ],
    );

    let (labels, report) =
        labels_from_surveys(&[survey], WinAuditOptions::default()).expect("conversion du relevé");
    assert!(labels.is_empty());
    assert_eq!(report.skipped_group_mac, 1);
}

#[test]
fn un_repertoire_est_parcouru_avec_ses_sous_dossiers() {
    // Les collectes rangent leurs relevés par sous-dossier.
    let fixtures = Fixtures::new("recursif");
    std::fs::create_dir_all(fixtures.path().join("collecte-2").join("site-b"))
        .expect("création des sous-dossiers");
    for (file_name, name, mac) in [
        ("a.csv", "poste-a", "38:f3:ab:cf:ba:0a"),
        ("collecte-2/site-b/b.csv", "poste-b", "38:f3:ab:cf:ba:0b"),
    ] {
        fixtures.write(
            file_name,
            &[
                machine_row(name),
                network_row("eth0", name, "10.0.1.1", "Ethernet 802.3", mac),
            ],
        );
    }

    let (labels, report) =
        labels_from_surveys(&[fixtures.path().to_path_buf()], WinAuditOptions::default())
            .expect("le répertoire est parcouru");
    assert_eq!(report.files_read, 2);
    assert!(labels.iter().any(|entry| entry.label == "poste-b"));
}

/// Parité avec l'outil `winaudit2labels` :
/// `attendu-winaudit2labels.csv` est sa sortie sur le même corpus, lancée
/// **sans option**. Les deux outils partagent leurs réglages par défaut :
/// interfaces virtuelles écartées d'après la MAC, relevé le plus récent
/// gardé. Si une règle dérive d'un côté, un même relevé ne donnerait plus le
/// même label.
#[test]
fn la_sortie_est_celle_de_winaudit2labels_sur_le_corpus_de_parite() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/winaudit/parite-winaudit2labels");
    let (labels, report) =
        labels_from_surveys(&[corpus.join("releves")], WinAuditOptions::default())
            .expect("conversion du corpus");
    assert_eq!(report.files_read, 7);
    assert!(report.files_rejected.is_empty());

    let fixtures = Fixtures::new("parite");
    let output = fixtures.path().join("labels.csv");
    write_label_csv(&labels, &output).expect("écriture du fichier de labels");

    assert_eq!(
        std::fs::read_to_string(&output).expect("relecture"),
        std::fs::read_to_string(corpus.join("attendu-winaudit2labels.csv"))
            .expect("sortie de référence de winaudit2labels"),
    );
}
