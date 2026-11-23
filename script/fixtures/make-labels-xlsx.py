#!/usr/bin/env python3
"""Régénère fixtures/20261122_labels.xlsx, classeur de labels de test, et
fixtures/20261122_labels_erreur.xlsx, le même avec une cellule en erreur,
et fixtures/20261122_labels_matrice.xlsx, labels d'équipements présents
dans fixtures/20260703_NP_Matrice.csv (avec un doublon en désaccord et une
ligne sans label).

Deux classeurs hostiles les accompagnent : 20261122_classeur_cellule_lointaine.xlsx
(cellules A1 et XFD1048576 : la plage rectangulaire entre les deux pèserait
des centaines de gigaoctets) et 20261122_classeur_hors_feuille.xlsx (cellule
au-delà de la dernière ligne d'une feuille).

Cinq autres fixent les cas de saisie au tableur : 20261122_labels_numeriques.xlsx
(labels en cellules numériques, lus), 20261122_labels_mac_numerique.xlsx et
20261122_labels_ip_numerique.xlsx (adresse en cellule numérique, refusée),
20261122_labels_ligne_4.xlsx (tableau qui commence en ligne 4) et
20261122_labels_colonne_b.xlsx (tableau qui commence en colonne B, refusé).

Bibliothèque standard seule, sortie reproductible (dates et ordre figés) :
    python3 script/fixtures/make-labels-xlsx.py

Première feuille : en-tête mac, ip, label, puis des lignes qui exercent le
lecteur (MAC en écritures d'inventaire, ligne vide, équipement sans label,
virgules et point-virgules dans un label, cellule de formule, colonne de
plus). La deuxième feuille ne doit jamais être lue.
"""

import sys
import zipfile
from pathlib import Path
from xml.sax.saxutils import escape

ROOT = Path(__file__).resolve().parents[2]
TARGET = ROOT / "fixtures" / "20261122_labels.xlsx"

# None = ligne vide ; ("f", formule, valeur en cache) = cellule de formule ;
# ("e", code) = cellule en erreur ; ("n", nombre) = cellule numérique.
ROWS = [
    ["mac", "ip", "label"],
    ["AA-BB-CC-DD-EE-01", "10.0.0.1", "Automate ligne 1"],
    ["aabb.ccdd.ee02", "10.0.0.2", "Poste; supervision, salle A"],
    None,
    ["aa:bb:cc:dd:ee:03", "10.0.0.3", ""],
    ["aa:bb:cc:dd:ee:04", "2001:0db8:0000:0000:0000:0000:0000:0004", "Passerelle"],
    ["aa:bb:cc:dd:ee:05", "10.0.0.5", ("f", 'CONCATENATE("Ser","veur")', "Serveur")],
    ["aa:bb:cc:dd:ee:06", "10.0.0.6", "Imprimante", "étage 2"],
    ["aa:bb:cc:dd:ee:07", "10.0.0.7", "=HYPERLINK(\"http://exemple.invalid\")"],
]
BROKEN = [
    ["mac", "ip", "label"],
    ["aa:bb:cc:dd:ee:01", "10.0.0.1", "Automate ligne 1"],
    ["aa:bb:cc:dd:ee:02", "10.0.0.2", ("e", "#N/A")],
]
OBSERVED = [
    ["mac", "ip", "label"],
    ["E0-D5-5E-28-9B-D4", "192.168.1.181", "Poste de relevé"],
    ["44:15:24:20:a5:64", "192.168.1.254", "Passerelle"],
    ["e0:d5:5e:28:9b:d4", "192.168.1.181", "Autre nom"],
    ["aa:bb:cc:dd:ee:09", "10.0.0.9", ""],
]
# Labels saisis comme des nombres : lus tels qu'affichés.
NUMERIC_LABELS = [
    ["mac", "ip", "label"],
    ["aa:bb:cc:dd:ee:01", "10.0.0.1", ("n", "42")],
    ["aa:bb:cc:dd:ee:02", "10.0.0.2", ("n", "1.5")],
]
# MAC 00:11:22:33:44:55 saisie sans séparateur : le tableur en a fait le
# nombre 1122334455, zéros de tête perdus.
NUMERIC_MAC = [
    ["mac", "ip", "label"],
    ["aa:bb:cc:dd:ee:01", "10.0.0.1", "Automate ligne 1"],
    [("n", "1122334455"), "10.0.0.2", "Automate ligne 2"],
]
NUMERIC_IP = [
    ["mac", "ip", "label"],
    ["aa:bb:cc:dd:ee:01", ("n", "10"), "Automate ligne 1"],
]
# Tableau posé plus bas dans la feuille, sous trois lignes vides.
FROM_LINE_4 = [
    None,
    None,
    None,
    ["mac", "ip", "label"],
    ["aa:bb:cc:dd:ee:01", "10.0.0.1", "Automate ligne 1"],
    ["aa:bb:cc:dd:ee:02", "10.0.0.2", "Automate ligne 2"],
]
# Tableau décalé d'une colonne : la colonne A est vide.
FROM_COLUMN_B = [
    ["", "mac", "ip", "label"],
    ["", "aa:bb:cc:dd:ee:01", "10.0.0.1", "Automate ligne 1"],
    ["", "aa:bb:cc:dd:ee:02", "10.0.0.2", "Automate ligne 2"],
]
HIDDEN = [["mac", "ip", "label"], ["aa:bb:cc:dd:ee:ff", "10.9.9.9", "Feuille 2, jamais lue"]]


def column(index):
    return chr(ord("A") + index)


def sheet_xml(rows):
    out = ['<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
           '<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>']
    for number, row in enumerate(rows, start=1):
        if row is None:
            continue
        out.append(f'<row r="{number}">')
        for index, cell in enumerate(row):
            ref = f"{column(index)}{number}"
            if isinstance(cell, tuple) and cell[0] == "e":
                out.append(f'<c r="{ref}" t="e"><v>{escape(cell[1])}</v></c>')
            elif isinstance(cell, tuple) and cell[0] == "n":
                out.append(f'<c r="{ref}"><v>{escape(cell[1])}</v></c>')
            elif isinstance(cell, tuple):
                _, formula, cached = cell
                out.append(f'<c r="{ref}" t="str"><f>{escape(formula)}</f><v>{escape(cached)}</v></c>')
            elif cell != "":
                out.append(f'<c r="{ref}" t="inlineStr"><is><t xml:space="preserve">{escape(cell)}</t></is></c>')
        out.append("</row>")
    out.append("</sheetData></worksheet>")
    return "".join(out)


FILES = {
    "[Content_Types].xml": (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
        '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
        '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
        '<Default Extension="xml" ContentType="application/xml"/>'
        '<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>'
        '<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>'
        '<Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>'
        "</Types>"
    ),
    "_rels/.rels": (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
        '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
        '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>'
        "</Relationships>"
    ),
    "xl/workbook.xml": (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
        '<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" '
        'xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">'
        '<sheets><sheet name="Labels" sheetId="1" r:id="rId1"/><sheet name="Notes" sheetId="2" r:id="rId2"/></sheets>'
        "</workbook>"
    ),
    "xl/_rels/workbook.xml.rels": (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
        '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
        '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>'
        '<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/>'
        "</Relationships>"
    ),
    "xl/worksheets/sheet2.xml": sheet_xml(HIDDEN),
}


def raw_sheet(cells):
    """Feuille à partir de couples (référence, texte), une ligne par cellule."""
    out = ['<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
           '<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>']
    for ref, text in cells:
        row = "".join(c for c in ref if c.isdigit())
        out.append(f'<row r="{row}"><c r="{ref}" t="inlineStr"><is><t>{escape(text)}</t></is></c></row>')
    out.append("</sheetData></worksheet>")
    return "".join(out)


def write(target, rows):
    files = dict(FILES)
    files["xl/worksheets/sheet1.xml"] = rows if isinstance(rows, str) else sheet_xml(rows)
    with zipfile.ZipFile(target, "w", zipfile.ZIP_DEFLATED) as archive:
        for name, content in files.items():
            info = zipfile.ZipInfo(name, date_time=(2026, 11, 22, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o644 << 16
            archive.writestr(info, content)
    print(target)


def main():
    write(TARGET, ROWS)
    write(TARGET.with_name("20261122_labels_erreur.xlsx"), BROKEN)
    write(TARGET.with_name("20261122_labels_matrice.xlsx"), OBSERVED)
    write(TARGET.with_name("20261122_labels_numeriques.xlsx"), NUMERIC_LABELS)
    write(TARGET.with_name("20261122_labels_mac_numerique.xlsx"), NUMERIC_MAC)
    write(TARGET.with_name("20261122_labels_ip_numerique.xlsx"), NUMERIC_IP)
    write(TARGET.with_name("20261122_labels_ligne_4.xlsx"), FROM_LINE_4)
    write(TARGET.with_name("20261122_labels_colonne_b.xlsx"), FROM_COLUMN_B)
    write(
        TARGET.with_name("20261122_classeur_cellule_lointaine.xlsx"),
        raw_sheet([("A1", "mac"), ("XFD1048576", "loin")]),
    )
    write(
        TARGET.with_name("20261122_classeur_hors_feuille.xlsx"),
        raw_sheet([("A1", "mac"), ("A1048577", "trop bas")]),
    )


if __name__ == "__main__":
    sys.exit(main())
