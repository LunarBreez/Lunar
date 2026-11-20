//! Tableau en lecture seule de triplets `(MAC, IP, label)` filtrés par une
//! recherche : contenu importé et labels de la matrice.

use gpui::{SharedString, px};
use lunar_ui_logic::label_conflicts::dash_if_empty;
use lunar_ui_logic::label_management::filter_triplets;

use crate::widgets::table::{Column, TableDelegate};

/// Lignes et filtre d'un tableau de triplets.
pub struct TripletRows {
    /// Colonnes « Adresse MAC », « Adresse IP », « Label ».
    columns: Vec<Column>,
    /// Toutes les lignes.
    rows: Vec<(String, String, String)>,
    /// Indices des lignes qui passent la recherche.
    visible: Vec<usize>,
    /// Recherche courante.
    search: String,
}

impl TripletRows {
    /// Tableau vide.
    pub fn new() -> Self {
        Self {
            columns: vec![
                Column::new("Adresse MAC", px(170.)).mono().sortable(),
                Column::flex("Adresse IP", 1.).mono().sortable(),
                Column::flex("Label", 1.).sortable(),
            ],
            rows: Vec::new(),
            visible: Vec::new(),
            search: String::new(),
        }
    }

    /// Remplace les lignes (filtre conservé).
    pub fn set_rows(&mut self, rows: Vec<(String, String, String)>) {
        self.rows = rows;
        self.refilter();
    }

    /// Change la recherche.
    pub fn set_search(&mut self, search: &str) {
        self.search = search.to_owned();
        self.refilter();
    }

    /// Nombre total de lignes, filtre ignoré.
    pub fn total(&self) -> usize {
        self.rows.len()
    }

    /// Recalcule les lignes visibles.
    fn refilter(&mut self) {
        self.visible = filter_triplets(&self.rows, &self.search);
    }

    /// Ligne visible `row`.
    fn row(&self, row: usize) -> Option<&(String, String, String)> {
        self.visible
            .get(row)
            .and_then(|index| self.rows.get(*index))
    }
}

impl Default for TripletRows {
    fn default() -> Self {
        Self::new()
    }
}

impl TableDelegate for TripletRows {
    fn columns(&self) -> &[Column] {
        &self.columns
    }

    fn row_count(&self) -> usize {
        self.visible.len()
    }

    fn cell_text(&self, row: usize, column: usize) -> SharedString {
        let Some((mac, ip, label)) = self.row(row) else {
            return SharedString::default();
        };
        let value = match column {
            0 => mac,
            1 => ip,
            _ => label,
        };
        dash_if_empty(value).to_owned().into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn triplet(mac: &str, ip: &str, label: &str) -> (String, String, String) {
        (mac.to_owned(), ip.to_owned(), label.to_owned())
    }

    #[test]
    fn filtre_et_tirets_des_champs_vides() {
        let mut rows = TripletRows::new();
        rows.set_rows(vec![
            triplet("aa:bb:cc:dd:ee:01", "10.0.0.1", "Automate"),
            triplet("aa:bb:cc:dd:ee:02", "", "IHM"),
        ]);
        assert_eq!(rows.row_count(), 2);
        assert_eq!(rows.cell_text(1, 1).as_ref(), "-");
        rows.set_search("ihm");
        assert_eq!(rows.row_count(), 1);
        assert_eq!(rows.total(), 2);
        assert_eq!(rows.cell_text(0, 2).as_ref(), "IHM");
        assert_eq!(rows.cell_text(5, 0).as_ref(), "");
        rows.set_rows(vec![triplet("x", "y", "z")]);
        assert_eq!(rows.row_count(), 0, "le filtre reste appliqué");
    }
}
