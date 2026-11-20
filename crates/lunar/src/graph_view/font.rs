//! Police des labels : Liberation Sans Regular embarquée (métriques d'Arial,
//! la police des labels Sigma), licence SIL OFL 1.1 dans
//! `assets/fonts/LICENSE-OFL-1.1.txt`, provenance dans
//! `assets/fonts/SOURCE.md`.
//!
//! À l'écran, gpui met en forme le texte avec cette police (enregistrée par
//! [`register`]). L'export PNG, hors gpui, la lit avec `ttf-parser` :
//! largeurs par chasse des glyphes (sans crénage, à moins d'un pixel près
//! sur un label) et contours vectoriels remplis par tiny-skia.

use std::borrow::Cow;
use std::sync::{Once, OnceLock};

use super::scene::TextMeasure;

/// Fichier de la police.
pub static LABEL_FONT_BYTES: &[u8] =
    include_bytes!("../../assets/fonts/LiberationSans-Regular.ttf");

/// Famille de la police, telle que la déclare son fichier.
pub const LABEL_FONT_FAMILY: &str = "Liberation Sans";

/// Police analysée.
pub struct LabelFont {
    /// Fichier analysé.
    face: ttf_parser::Face<'static>,
    /// Unités par em.
    units_per_em: f32,
    /// Chasse des caractères ASCII, en em.
    ascii: [f32; 128],
    /// Chasse du glyphe de remplacement, en em.
    fallback: f32,
}

impl LabelFont {
    /// Analyse la police embarquée.
    fn parse() -> Option<Self> {
        let face = ttf_parser::Face::parse(LABEL_FONT_BYTES, 0)
            .map_err(|err| log::error!("police des labels illisible : {err}"))
            .ok()?;
        let units_per_em = f32::from(face.units_per_em().max(1));
        let em = |c: char| -> Option<f32> {
            let glyph = face.glyph_index(c)?;
            Some(f32::from(face.glyph_hor_advance(glyph)?) / units_per_em)
        };
        let fallback = em('\u{FFFD}').or_else(|| em('?')).unwrap_or(0.5);
        let mut ascii = [fallback; 128];
        for (i, slot) in ascii.iter_mut().enumerate() {
            if let Some(w) = char::from_u32(i as u32).and_then(em) {
                *slot = w;
            }
        }
        Some(Self {
            face,
            units_per_em,
            ascii,
            fallback,
        })
    }

    /// Chasse de `c`, en em.
    pub fn advance(&self, c: char) -> f32 {
        if let Some(&w) = self.ascii.get(c as usize) {
            return w;
        }
        self.face
            .glyph_index(c)
            .and_then(|g| self.face.glyph_hor_advance(g))
            .map_or(self.fallback, |a| f32::from(a) / self.units_per_em)
    }

    /// Largeur de `text` à `size` pixels.
    pub fn width(&self, text: &str, size: f32) -> f32 {
        text.chars().map(|c| self.advance(c)).sum::<f32>() * size
    }

    /// Trace le contour de `c` dans `builder`, en unités de la police (y vers
    /// le haut) ; retourne faux sans glyphe ou sans contour (espace).
    pub fn outline(&self, c: char, builder: &mut dyn ttf_parser::OutlineBuilder) -> bool {
        self.face
            .glyph_index(c)
            .and_then(|g| self.face.outline_glyph(g, builder))
            .is_some()
    }

    /// Unités par em.
    pub fn units_per_em(&self) -> f32 {
        self.units_per_em
    }
}

/// Police des labels, analysée au premier usage.
pub fn label_font() -> Option<&'static LabelFont> {
    static FONT: OnceLock<Option<LabelFont>> = OnceLock::new();
    FONT.get_or_init(LabelFont::parse).as_ref()
}

/// Enregistre la police auprès de gpui (une fois par processus).
pub fn register(cx: &gpui::App) {
    static REGISTERED: Once = Once::new();
    REGISTERED.call_once(|| {
        if let Err(err) = cx
            .text_system()
            .add_fonts(vec![Cow::Borrowed(LABEL_FONT_BYTES)])
        {
            log::warn!("police des labels non enregistrée : {err:#}");
        }
    });
}

/// Mesure par la police embarquée (export PNG, tests).
#[derive(Clone, Copy, Debug, Default)]
pub struct FontMeasure;

impl TextMeasure for FontMeasure {
    fn width(&mut self, text: &str, font_size: f32) -> f32 {
        label_font().map_or(text.chars().count() as f32 * font_size * 0.55, |f| {
            f.width(text, font_size)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metriques_d_arial() {
        let font = label_font().expect("police embarquée");
        // Chasses d'Arial (unités sur 2048) : « a » 1139, espace 569, « W » 1933.
        assert!((font.advance('a') - 1139.0 / 2048.0).abs() < 1e-4);
        assert!((font.advance(' ') - 569.0 / 2048.0).abs() < 1e-4);
        assert!((font.advance('W') - 1933.0 / 2048.0).abs() < 1e-4);
        assert!(font.advance('…') > 0.0);
        assert_eq!(FontMeasure.width("", 13.0), 0.0);
    }
}
