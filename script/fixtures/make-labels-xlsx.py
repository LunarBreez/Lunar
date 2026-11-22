#!/usr/bin/env python3
"""Régénère fixtures/20261122_labels.xlsx, classeur de labels de test, et
fixtures/20261122_labels_erreur.xlsx, le même avec une cellule en erreur.

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
# ("e", code) = cellule en erreur.
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


def write(target, rows):
    files = dict(FILES)
    files["xl/worksheets/sheet1.xml"] = sheet_xml(rows)
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


if __name__ == "__main__":
    sys.exit(main())
