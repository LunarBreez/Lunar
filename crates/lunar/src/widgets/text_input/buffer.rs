//! Tampon d'édition sans fenêtre : texte, sélection, composition (IME).
//!
//! Toutes les positions sont des décalages en octets UTF-8, toujours sur une
//! frontière de caractère ; les déplacements d'un caractère suivent les
//! graphèmes (un « é » décomposé s'efface d'un coup). Les conversions UTF-16
//! servent à l'interface de saisie de la plateforme (`EntityInputHandler`).

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

/// Caractère de mot : lettre, chiffre ou souligné.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Texte édité et sélection courante.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EditBuffer {
    /// Contenu.
    text: String,
    /// Sélection ; vide, c'est la position du curseur.
    selection: Range<usize>,
    /// Le curseur est au début de la sélection (sélection vers la gauche).
    reversed: bool,
    /// Texte en cours de composition par l'IME.
    marked: Option<Range<usize>>,
    /// Les retours à la ligne sont permis.
    multiline: bool,
}

impl EditBuffer {
    /// Tampon vide, sur une ou plusieurs lignes.
    pub fn new(multiline: bool) -> Self {
        Self {
            multiline,
            ..Self::default()
        }
    }

    /// Contenu.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Les retours à la ligne sont permis.
    pub fn is_multiline(&self) -> bool {
        self.multiline
    }

    /// Sélection courante.
    pub fn selection(&self) -> Range<usize> {
        self.selection.clone()
    }

    /// Le curseur est au début de la sélection.
    pub fn is_reversed(&self) -> bool {
        self.reversed
    }

    /// Plage en cours de composition par l'IME.
    pub fn marked(&self) -> Option<Range<usize>> {
        self.marked.clone()
    }

    /// Position du curseur.
    pub fn cursor(&self) -> usize {
        if self.reversed {
            self.selection.start
        } else {
            self.selection.end
        }
    }

    /// Texte sélectionné.
    pub fn selected_text(&self) -> &str {
        &self.text[self.selection.clone()]
    }

    /// Adapte un texte au mode : sur une seule ligne, les retours à la ligne
    /// deviennent des espaces ; sur plusieurs, `\r\n` et `\r` deviennent `\n`.
    pub fn sanitize(&self, text: &str) -> String {
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        if self.multiline {
            text
        } else {
            text.replace('\n', " ")
        }
    }

    /// Remplace tout le contenu ; le curseur va à la fin.
    pub fn set_text(&mut self, text: &str) {
        self.text = self.sanitize(text);
        let end = self.text.len();
        self.selection = end..end;
        self.reversed = false;
        self.marked = None;
    }

    /// Ramène un décalage dans le texte, sur une frontière de caractère.
    fn clamp(&self, offset: usize) -> usize {
        let mut offset = offset.min(self.text.len());
        while !self.text.is_char_boundary(offset) {
            offset -= 1;
        }
        offset
    }

    /// Place le curseur, sans sélection.
    pub fn move_to(&mut self, offset: usize) {
        let offset = self.clamp(offset);
        self.selection = offset..offset;
        self.reversed = false;
    }

    /// Étend la sélection jusqu'à `offset` en gardant son ancre.
    pub fn select_to(&mut self, offset: usize) {
        let offset = self.clamp(offset);
        if self.reversed {
            self.selection.start = offset;
        } else {
            self.selection.end = offset;
        }
        if self.selection.end < self.selection.start {
            self.reversed = !self.reversed;
            self.selection = self.selection.end..self.selection.start;
        }
    }

    /// Sélectionne une plage, curseur à la fin.
    pub fn select_range(&mut self, range: Range<usize>) {
        let start = self.clamp(range.start);
        let end = self.clamp(range.end);
        self.selection = start.min(end)..start.max(end);
        self.reversed = false;
    }

    /// Sélectionne tout.
    pub fn select_all(&mut self) {
        self.select_range(0..self.text.len());
    }

    /// Frontière de graphème précédente.
    pub fn previous_boundary(&self, offset: usize) -> usize {
        self.text
            .grapheme_indices(true)
            .rev()
            .find_map(|(ix, _)| (ix < offset).then_some(ix))
            .unwrap_or(0)
    }

    /// Frontière de graphème suivante.
    pub fn next_boundary(&self, offset: usize) -> usize {
        self.text
            .grapheme_indices(true)
            .find_map(|(ix, _)| (ix > offset).then_some(ix))
            .unwrap_or(self.text.len())
    }

    /// Début du mot à gauche de `offset` (saute d'abord les séparateurs).
    pub fn previous_word_start(&self, offset: usize) -> usize {
        let mut chars = self.text[..self.clamp(offset)]
            .char_indices()
            .rev()
            .peekable();
        let mut position = self.clamp(offset);
        while let Some(&(ix, c)) = chars.peek() {
            if is_word_char(c) {
                break;
            }
            position = ix;
            chars.next();
        }
        while let Some(&(ix, c)) = chars.peek() {
            if !is_word_char(c) {
                break;
            }
            position = ix;
            chars.next();
        }
        position
    }

    /// Fin du mot à droite de `offset` (saute d'abord les séparateurs).
    pub fn next_word_end(&self, offset: usize) -> usize {
        let start = self.clamp(offset);
        let mut chars = self.text[start..].char_indices().peekable();
        let mut position = start;
        while let Some(&(ix, c)) = chars.peek() {
            if is_word_char(c) {
                break;
            }
            position = start + ix + c.len_utf8();
            chars.next();
        }
        while let Some(&(ix, c)) = chars.peek() {
            if !is_word_char(c) {
                break;
            }
            position = start + ix + c.len_utf8();
            chars.next();
        }
        position
    }

    /// Mot (ou suite de séparateurs) sous `offset`, pour le double clic.
    pub fn word_range_at(&self, offset: usize) -> Range<usize> {
        let offset = self.clamp(offset);
        let Some(c) = self.text[offset..].chars().next() else {
            return offset..offset;
        };
        let same_class = |other: char| {
            if is_word_char(c) {
                is_word_char(other)
            } else if c.is_whitespace() {
                other.is_whitespace() && other != '\n'
            } else {
                false
            }
        };
        let start = self.text[..offset]
            .char_indices()
            .rev()
            .take_while(|(_, other)| same_class(*other))
            .last()
            .map_or(offset, |(ix, _)| ix);
        let end = self.text[offset..]
            .char_indices()
            .take_while(|(ix, other)| *ix == 0 || same_class(*other))
            .last()
            .map_or(offset, |(ix, other)| offset + ix + other.len_utf8());
        start..end
    }

    /// Début de la ligne contenant `offset`.
    pub fn line_start(&self, offset: usize) -> usize {
        let offset = self.clamp(offset);
        self.text[..offset].rfind('\n').map_or(0, |ix| ix + 1)
    }

    /// Fin de la ligne contenant `offset` (avant le `\n`).
    pub fn line_end(&self, offset: usize) -> usize {
        let offset = self.clamp(offset);
        self.text[offset..]
            .find('\n')
            .map_or(self.text.len(), |ix| offset + ix)
    }

    /// Lignes du texte avec leur décalage de début.
    pub fn lines(&self) -> impl Iterator<Item = (usize, &str)> {
        let mut start = 0;
        self.text.split('\n').map(move |line| {
            let line_start = start;
            start += line.len() + 1;
            (line_start, line)
        })
    }

    /// Indice de la ligne contenant `offset`.
    pub fn line_index(&self, offset: usize) -> usize {
        self.text[..self.clamp(offset)].matches('\n').count()
    }

    /// Flèche gauche.
    pub fn left(&mut self) {
        if self.selection.is_empty() {
            self.move_to(self.previous_boundary(self.cursor()));
        } else {
            self.move_to(self.selection.start);
        }
    }

    /// Flèche droite.
    pub fn right(&mut self) {
        if self.selection.is_empty() {
            self.move_to(self.next_boundary(self.cursor()));
        } else {
            self.move_to(self.selection.end);
        }
    }

    /// Maj + flèche gauche.
    pub fn select_left(&mut self) {
        self.select_to(self.previous_boundary(self.cursor()));
    }

    /// Maj + flèche droite.
    pub fn select_right(&mut self) {
        self.select_to(self.next_boundary(self.cursor()));
    }

    /// Ctrl + flèche gauche.
    pub fn word_left(&mut self) {
        self.move_to(self.previous_word_start(self.cursor()));
    }

    /// Ctrl + flèche droite.
    pub fn word_right(&mut self) {
        self.move_to(self.next_word_end(self.cursor()));
    }

    /// Ctrl + Maj + flèche gauche.
    pub fn select_word_left(&mut self) {
        self.select_to(self.previous_word_start(self.cursor()));
    }

    /// Ctrl + Maj + flèche droite.
    pub fn select_word_right(&mut self) {
        self.select_to(self.next_word_end(self.cursor()));
    }

    /// Début : début de ligne.
    pub fn home(&mut self) {
        self.move_to(self.line_start(self.cursor()));
    }

    /// Fin : fin de ligne.
    pub fn end(&mut self) {
        self.move_to(self.line_end(self.cursor()));
    }

    /// Maj + Début.
    pub fn select_home(&mut self) {
        self.select_to(self.line_start(self.cursor()));
    }

    /// Maj + Fin.
    pub fn select_end(&mut self) {
        self.select_to(self.line_end(self.cursor()));
    }

    /// Retour arrière ; renvoie vrai si le texte a changé.
    pub fn backspace(&mut self) -> bool {
        if self.selection.is_empty() {
            if self.cursor() == 0 {
                return false;
            }
            self.select_to(self.previous_boundary(self.cursor()));
        }
        self.replace(None, "")
    }

    /// Suppr ; renvoie vrai si le texte a changé.
    pub fn delete(&mut self) -> bool {
        if self.selection.is_empty() {
            if self.cursor() == self.text.len() {
                return false;
            }
            self.select_to(self.next_boundary(self.cursor()));
        }
        self.replace(None, "")
    }

    /// Ctrl + retour arrière : efface le mot à gauche.
    pub fn delete_word_left(&mut self) -> bool {
        if self.selection.is_empty() {
            if self.cursor() == 0 {
                return false;
            }
            self.select_to(self.previous_word_start(self.cursor()));
        }
        self.replace(None, "")
    }

    /// Remplace la sélection par `text` (frappe, collage).
    pub fn insert(&mut self, text: &str) -> bool {
        self.replace(None, text)
    }

    /// Plage visée par un remplacement : celle donnée, sinon la composition
    /// en cours, sinon la sélection.
    fn target_range(&self, range: Option<Range<usize>>) -> Range<usize> {
        let range = range
            .or_else(|| self.marked.clone())
            .unwrap_or_else(|| self.selection.clone());
        let start = self.clamp(range.start);
        let end = self.clamp(range.end);
        start.min(end)..start.max(end)
    }

    /// Remplace une plage (ou la composition, ou la sélection) par
    /// `new_text` et termine la composition ; renvoie vrai si le texte a
    /// changé.
    pub fn replace(&mut self, range: Option<Range<usize>>, new_text: &str) -> bool {
        let range = self.target_range(range);
        let new_text = self.sanitize(new_text);
        let before = self.text.len();
        let changed = new_text != self.text[range.clone()];
        self.text.replace_range(range.clone(), &new_text);
        let cursor = range.start + new_text.len();
        self.selection = cursor..cursor;
        self.reversed = false;
        self.marked = None;
        changed || before != self.text.len()
    }

    /// Remplace une plage par un texte en composition (IME) et le marque ;
    /// `new_selection` est en octets UTF-8, relative au texte inséré.
    pub fn replace_and_mark(
        &mut self,
        range: Option<Range<usize>>,
        new_text: &str,
        new_selection: Option<Range<usize>>,
    ) -> bool {
        let range = self.target_range(range);
        let new_text = self.sanitize(new_text);
        let changed = new_text != self.text[range.clone()];
        self.text.replace_range(range.clone(), &new_text);
        self.marked = (!new_text.is_empty()).then(|| range.start..range.start + new_text.len());
        // L'exemple `input.rs` de gpui 0.2.2 ajoute `range.end` à la fin de la
        // sélection : la sélection est relative au texte inséré, donc à
        // `range.start` pour les deux bornes.
        let selection = new_selection
            .map(|selection| range.start + selection.start..range.start + selection.end)
            .unwrap_or_else(|| {
                let cursor = range.start + new_text.len();
                cursor..cursor
            });
        self.select_range(selection);
        changed
    }

    /// Termine la composition sans changer le texte.
    pub fn unmark(&mut self) {
        self.marked = None;
    }

    /// Décalage UTF-8 vers UTF-16.
    pub fn offset_to_utf16(&self, offset: usize) -> usize {
        let mut utf16 = 0;
        let mut utf8 = 0;
        for c in self.text.chars() {
            if utf8 >= offset {
                break;
            }
            utf8 += c.len_utf8();
            utf16 += c.len_utf16();
        }
        utf16
    }

    /// Décalage UTF-16 vers UTF-8.
    pub fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf8 = 0;
        let mut utf16 = 0;
        for c in self.text.chars() {
            if utf16 >= offset {
                break;
            }
            utf16 += c.len_utf16();
            utf8 += c.len_utf8();
        }
        utf8
    }

    /// Plage UTF-8 vers UTF-16.
    pub fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    /// Plage UTF-16 vers UTF-8.
    pub fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range.start)..self.offset_from_utf16(range.end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffer(text: &str) -> EditBuffer {
        let mut b = EditBuffer::new(false);
        b.set_text(text);
        b
    }

    #[test]
    fn frappe_et_retour_arriere() {
        let mut b = buffer("");
        assert!(b.insert("tcp"));
        assert!(b.insert(" port 80"));
        assert_eq!(b.text(), "tcp port 80");
        assert!(b.backspace());
        assert_eq!(b.text(), "tcp port 8");
        assert_eq!(b.cursor(), 10);
        b.move_to(0);
        assert!(!b.backspace(), "rien à effacer en début de texte");
        b.move_to(b.text().len());
        assert!(!b.delete(), "rien à supprimer en fin de texte");
    }

    #[test]
    fn graphemes_effaces_d_un_coup() {
        // « é » décomposé : e + accent aigu combinant.
        let mut b = buffer("caf\u{65}\u{301}");
        assert!(b.backspace());
        assert_eq!(b.text(), "caf");
        let mut b = buffer("a👍b");
        b.move_to(1);
        b.right();
        assert_eq!(b.cursor(), 1 + '👍'.len_utf8());
    }

    #[test]
    fn selection_inversee_puis_retablie() {
        let mut b = buffer("abcdef");
        b.move_to(3);
        b.select_left();
        b.select_left();
        assert_eq!(b.selection(), 1..3);
        assert!(b.is_reversed());
        assert_eq!(b.cursor(), 1);
        b.select_right();
        b.select_right();
        b.select_right();
        assert_eq!(b.selection(), 3..4);
        assert!(!b.is_reversed());
        assert_eq!(b.selected_text(), "d");
    }

    #[test]
    fn fleches_reduisent_la_selection() {
        let mut b = buffer("abcdef");
        b.select_range(1..4);
        b.left();
        assert_eq!(b.selection(), 1..1);
        b.select_range(1..4);
        b.right();
        assert_eq!(b.selection(), 4..4);
    }

    #[test]
    fn deplacements_par_mot() {
        let mut b = buffer("host 192.168.1.1 and port_53");
        b.move_to(b.text().len());
        b.word_left();
        assert_eq!(b.cursor(), b.text().find("port_53").unwrap_or(0));
        b.word_left();
        assert_eq!(b.cursor(), b.text().find("and").unwrap_or(0));
        b.move_to(0);
        b.word_right();
        assert_eq!(b.cursor(), 4);
        b.word_right();
        assert_eq!(b.cursor(), 8, "s'arrête après « 192 »");
        assert!(b.delete_word_left());
        assert_eq!(b.text(), "host .168.1.1 and port_53");
    }

    #[test]
    fn mot_sous_le_curseur() {
        let b = buffer("src host  10.0.0.1");
        assert_eq!(b.word_range_at(5), 4..8);
        assert_eq!(b.word_range_at(9), 8..10, "suite d'espaces");
        assert_eq!(b.word_range_at(12), 12..13, "séparateur seul");
        assert_eq!(b.word_range_at(b.text().len()), 18..18);
    }

    #[test]
    fn une_ligne_remplace_les_retours() {
        let mut b = buffer("");
        b.insert("tcp\r\nand\nudp");
        assert_eq!(b.text(), "tcp and udp");
    }

    #[test]
    fn plusieurs_lignes_debut_fin() {
        let mut b = EditBuffer::new(true);
        b.set_text("tcp\r\nand udp\nor icmp");
        assert_eq!(b.text(), "tcp\nand udp\nor icmp");
        b.move_to(6);
        assert_eq!(b.line_index(6), 1);
        b.home();
        assert_eq!(b.cursor(), 4);
        b.end();
        assert_eq!(b.cursor(), 11);
        b.select_home();
        assert_eq!(b.selected_text(), "and udp");
        let lines: Vec<_> = b.lines().collect();
        assert_eq!(lines, vec![(0, "tcp"), (4, "and udp"), (12, "or icmp")]);
    }

    #[test]
    fn conversions_utf16() {
        let b = buffer("é👍x");
        // é : 2 octets, 1 unité ; 👍 : 4 octets, 2 unités.
        assert_eq!(b.offset_to_utf16(2), 1);
        assert_eq!(b.offset_to_utf16(6), 3);
        assert_eq!(b.offset_from_utf16(3), 6);
        assert_eq!(b.range_from_utf16(&(1..3)), 2..6);
        assert_eq!(b.range_to_utf16(&(0..7)), 0..4);
    }

    #[test]
    fn composition_ime() {
        let mut b = buffer("ab");
        b.move_to(1);
        b.replace_and_mark(None, "ñ", Some(2..2));
        assert_eq!(b.text(), "añb");
        assert_eq!(b.marked(), Some(1..3));
        assert_eq!(b.selection(), 3..3);
        // La validation remplace la composition.
        b.replace(None, "ñ");
        assert_eq!(b.text(), "añb");
        assert_eq!(b.marked(), None);
        assert_eq!(b.cursor(), 3);
    }

    #[test]
    fn decalages_hors_bornes_ramenes() {
        let mut b = buffer("é");
        b.move_to(1);
        assert_eq!(b.cursor(), 0, "milieu de caractère ramené au début");
        b.move_to(99);
        assert_eq!(b.cursor(), 2);
    }
}
