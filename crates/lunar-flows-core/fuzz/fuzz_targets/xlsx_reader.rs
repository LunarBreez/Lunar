//! Cible de fuzzing : classeur XLSX hostile, matrice ou fichier de labels.
#![no_main]

use libfuzzer_sys::fuzz_target;
use lunar_flows_core::{csv::merge_matrix_files, graph::GraphData, xlsx::first_sheet_as_csv};
use std::path::PathBuf;

/// Fichier de travail propre au processus libFuzzer, retiré après chaque
/// entrée. L'extension `.xlsx` aiguille vers le lecteur de classeurs.
struct FuzzWorkbook(PathBuf);

impl FuzzWorkbook {
    fn for_current_process() -> Self {
        Self(std::env::temp_dir().join(format!("lunar-xlsx-fuzz-{}.xlsx", std::process::id())))
    }
}

impl Drop for FuzzWorkbook {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Défaut amont connu, toléré par la cible : calamine 0.38.0 calcule la
/// colonne et la ligne d'une référence de cellule sans contrôle de
/// débordement (`AAAAAAAAAA2`). cargo-fuzz compile avec ces contrôles, d'où
/// un panic ; le binaire livré est compilé sans, la valeur aberrante qui en
/// sort est alors refusée par les bornes de lunar-flows-core. Tout autre
/// panic, de calamine ou de LUNAR, reste un crash.
fn is_known_calamine_overflow(info: &std::panic::PanicHookInfo<'_>) -> bool {
    let in_calamine = info
        .location()
        .is_some_and(|location| location.file().contains("/calamine-0.38.0/src/xlsx/mod.rs"));
    let message = info
        .payload()
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| info.payload().downcast_ref::<String>().map(String::as_str))
        .unwrap_or_default();
    in_calamine && message.starts_with("attempt to multiply with overflow")
}

/// Remplace, une fois, le crochet de panic de libFuzzer (qui interrompt le
/// processus) par un crochet qui laisse se dérouler le seul défaut connu.
fn tolerate_known_calamine_overflow() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let abort_on_panic = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if !is_known_calamine_overflow(info) {
                abort_on_panic(info);
            }
        }));
    });
}

/// Sépare, dans une entrée « feuille », le XML de la feuille de celui de la
/// table des chaînes partagées (facultative).
const SHARED_STRINGS_MARKER: &[u8] = b"\n<!--LUNAR-SHARED-STRINGS-->\n";

const CONTENT_TYPES: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/sharedStrings.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/></Types>"#;

const ROOT_RELS: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#;

const WORKBOOK: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Feuille" sheetId="1" r:id="rId1"/></sheets></workbook>"#;

const WORKBOOK_RELS: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/></Relationships>"#;

/// CRC-32 (IEEE) d'un membre de l'archive.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

/// Archive ZIP valide, membres stockés sans compression. Fabriquée ici pour
/// que les mutations du fuzzer portent sur le XML des feuilles : une archive
/// compressée mutée à l'aveugle est presque toujours refusée dès son
/// conteneur, sans atteindre les lecteurs de cellules.
fn stored_zip(members: &[(&str, &[u8])]) -> Vec<u8> {
    let mut archive = Vec::new();
    let mut directory = Vec::new();
    for (name, content) in members {
        let (Ok(offset), Ok(size), Ok(name_len)) = (
            u32::try_from(archive.len()),
            u32::try_from(content.len()),
            u16::try_from(name.len()),
        ) else {
            break;
        };
        let crc = crc32(content);
        // En-tête local : signature, version 2.0, pas d'option, stocké.
        archive.extend_from_slice(&[0x50, 0x4b, 0x03, 0x04, 20, 0, 0, 0, 0, 0, 0, 0, 0x21, 0]);
        for field in [crc, size, size] {
            archive.extend_from_slice(&field.to_le_bytes());
        }
        archive.extend_from_slice(&name_len.to_le_bytes());
        archive.extend_from_slice(&[0, 0]);
        archive.extend_from_slice(name.as_bytes());
        archive.extend_from_slice(content);

        directory.extend_from_slice(&[
            0x50, 0x4b, 0x01, 0x02, 20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0x21, 0,
        ]);
        for field in [crc, size, size] {
            directory.extend_from_slice(&field.to_le_bytes());
        }
        directory.extend_from_slice(&name_len.to_le_bytes());
        // Champ extra, commentaire, disque, attributs internes et externes.
        directory.extend_from_slice(&[0; 12]);
        directory.extend_from_slice(&offset.to_le_bytes());
        directory.extend_from_slice(name.as_bytes());
    }
    let (Ok(directory_offset), Ok(directory_size), Ok(count)) = (
        u32::try_from(archive.len()),
        u32::try_from(directory.len()),
        u16::try_from(members.len()),
    ) else {
        return archive;
    };
    archive.extend_from_slice(&directory);
    archive.extend_from_slice(&[0x50, 0x4b, 0x05, 0x06, 0, 0, 0, 0]);
    archive.extend_from_slice(&count.to_le_bytes());
    archive.extend_from_slice(&count.to_le_bytes());
    archive.extend_from_slice(&directory_size.to_le_bytes());
    archive.extend_from_slice(&directory_offset.to_le_bytes());
    archive.extend_from_slice(&[0, 0]);
    archive
}

/// Classeur à une feuille dont l'entrée fournit le XML (et, après le
/// marqueur, celui des chaînes partagées).
fn workbook_around(sheet_and_strings: &[u8]) -> Vec<u8> {
    let split = sheet_and_strings
        .windows(SHARED_STRINGS_MARKER.len())
        .position(|window| window == SHARED_STRINGS_MARKER);
    let (sheet, strings) = match split {
        Some(at) => (
            &sheet_and_strings[..at],
            Some(&sheet_and_strings[at + SHARED_STRINGS_MARKER.len()..]),
        ),
        None => (sheet_and_strings, None),
    };
    let mut members = vec![
        ("[Content_Types].xml", CONTENT_TYPES),
        ("_rels/.rels", ROOT_RELS),
        ("xl/workbook.xml", WORKBOOK),
        ("xl/_rels/workbook.xml.rels", WORKBOOK_RELS),
        ("xl/worksheets/sheet1.xml", sheet),
    ];
    if let Some(strings) = strings {
        members.push(("xl/sharedStrings.xml", strings));
    }
    stored_zip(&members)
}

// Deux formes d'entrée, choisies par le premier octet. Pair : le reste est le
// classeur tel quel, pour le conteneur ZIP et l'ouverture par calamine.
// Impair : le reste est le XML de la première feuille, emballé ici dans une
// archive valide, pour les lecteurs de cellules. Le classeur passe par ses
// deux usages : matrice de flux (préambule, validation, fusion, lignes à plat
// et graphe) et fichier de labels (première feuille rendue en CSV). Un
// classeur hostile doit produire une erreur typée, jamais un panic, une
// boucle ou un débordement.
fuzz_target!(|data: &[u8]| {
    let Some((mode, body)) = data.split_first() else {
        return;
    };
    let workbook = if mode % 2 == 0 {
        body.to_vec()
    } else {
        workbook_around(body)
    };
    let file = FuzzWorkbook::for_current_process();
    if std::fs::write(&file.0, workbook).is_err() {
        return;
    }

    tolerate_known_calamine_overflow();
    let _ = std::panic::catch_unwind(|| {
        if let Ok(matrix) = merge_matrix_files(std::slice::from_ref(&file.0)) {
            let _ = matrix.to_flat_vec();
            let _ = GraphData::from_flow_matrix(&matrix);
        }
    });
    let _ = std::panic::catch_unwind(|| {
        let _ = first_sheet_as_csv(&file.0, 3, 2);
    });
});
