//! Champ de saisie sur une ligne ou sur plusieurs (expression BPF brute).
//!
//! Adapté de l'exemple `input.rs` de gpui 0.2.2 : curseur, sélection au
//! clavier et à la souris (double clic : mot, triple clic : ligne),
//! presse-papiers, composition IME par `EntityInputHandler`, texte indicatif,
//! état désactivé, défilement horizontal d'une ligne trop longue, message de
//! validation. L'édition elle-même est dans [`buffer::EditBuffer`], testable
//! sans fenêtre.
//!
//! Style : fond `#12121e`, texte `#d0d0f0`, bordure `#2d2d50` puis
//! `#4a7cff` au focus, rayon 6, 13 px ; erreur en `#ff6060`.
//!
//! Raccourcis : flèches, Ctrl+flèches (mot), Maj (sélection), Début / Fin,
//! Ctrl+A, Ctrl+C / X / V (et Maj+Inser, Ctrl+Inser, Maj+Suppr), Ctrl+Retour
//! arrière (mot). Entrée valide un champ d'une ligne ; dans un champ de
//! plusieurs lignes, Entrée insère un retour et Ctrl+Entrée valide.
//!
//! Limites : pas de clignotement du curseur, pas d'annulation (Ctrl+Z), pas
//! de retour à la ligne automatique dans le champ de plusieurs lignes (les
//! lignes trop longues sont coupées à droite).

pub mod buffer;

use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, Element, ElementId, ElementInputHandler,
    Entity, EntityInputHandler, EventEmitter, FocusHandle, Focusable, GlobalElementId,
    InspectorElementId, IntoElement, KeyBinding, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point, Render, ShapedLine, SharedString,
    Style, TextRun, UTF16Selection, UnderlineStyle, Window, actions, div, fill, point, prelude::*,
    px, relative, size,
};

use crate::theme::ActiveTheme;
pub use buffer::EditBuffer;

actions!(
    lunar_text_input,
    [
        /// Efface le caractère à gauche.
        Backspace,
        /// Efface le caractère à droite.
        Delete,
        /// Efface le mot à gauche.
        DeleteWordLeft,
        /// Curseur à gauche.
        Left,
        /// Curseur à droite.
        Right,
        /// Ligne précédente (début du texte sur une ligne).
        Up,
        /// Ligne suivante (fin du texte sur une ligne).
        Down,
        /// Étend la sélection à gauche.
        SelectLeft,
        /// Étend la sélection à droite.
        SelectRight,
        /// Étend la sélection à la ligne précédente.
        SelectUp,
        /// Étend la sélection à la ligne suivante.
        SelectDown,
        /// Mot précédent.
        WordLeft,
        /// Mot suivant.
        WordRight,
        /// Étend la sélection au mot précédent.
        SelectWordLeft,
        /// Étend la sélection au mot suivant.
        SelectWordRight,
        /// Sélectionne tout.
        SelectAll,
        /// Début de ligne.
        Home,
        /// Fin de ligne.
        End,
        /// Étend la sélection au début de ligne.
        SelectHome,
        /// Étend la sélection à la fin de ligne.
        SelectEnd,
        /// Colle le presse-papiers.
        Paste,
        /// Coupe la sélection.
        Cut,
        /// Copie la sélection.
        Copy,
        /// Valide le champ.
        Submit,
        /// Insère un retour à la ligne (champ de plusieurs lignes).
        Newline,
        /// Palette de caractères de la plateforme.
        ShowCharacterPalette,
    ]
);

/// Contexte clavier des champs de saisie.
pub const TEXT_INPUT_CONTEXT: &str = "LunarTextInput";

/// Installe les raccourcis des champs ; appelé par [`crate::widgets::init`].
pub(crate) fn bind_keys(cx: &mut App) {
    let c = Some(TEXT_INPUT_CONTEXT);
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, c),
        KeyBinding::new("shift-backspace", Backspace, c),
        KeyBinding::new("ctrl-backspace", DeleteWordLeft, c),
        KeyBinding::new("delete", Delete, c),
        KeyBinding::new("left", Left, c),
        KeyBinding::new("right", Right, c),
        KeyBinding::new("up", Up, c),
        KeyBinding::new("down", Down, c),
        KeyBinding::new("shift-left", SelectLeft, c),
        KeyBinding::new("shift-right", SelectRight, c),
        KeyBinding::new("shift-up", SelectUp, c),
        KeyBinding::new("shift-down", SelectDown, c),
        KeyBinding::new("ctrl-left", WordLeft, c),
        KeyBinding::new("ctrl-right", WordRight, c),
        KeyBinding::new("ctrl-shift-left", SelectWordLeft, c),
        KeyBinding::new("ctrl-shift-right", SelectWordRight, c),
        KeyBinding::new("secondary-a", SelectAll, c),
        KeyBinding::new("home", Home, c),
        KeyBinding::new("end", End, c),
        KeyBinding::new("shift-home", SelectHome, c),
        KeyBinding::new("shift-end", SelectEnd, c),
        KeyBinding::new("secondary-v", Paste, c),
        KeyBinding::new("shift-insert", Paste, c),
        KeyBinding::new("secondary-c", Copy, c),
        KeyBinding::new("ctrl-insert", Copy, c),
        KeyBinding::new("secondary-x", Cut, c),
        KeyBinding::new("shift-delete", Cut, c),
        KeyBinding::new("enter", Submit, Some("LunarTextInput && !multiline")),
        KeyBinding::new("enter", Newline, Some("LunarTextInput && multiline")),
        KeyBinding::new("secondary-enter", Submit, c),
        KeyBinding::new("ctrl-cmd-space", ShowCharacterPalette, c),
    ]);
}

/// Événements d'un champ, à suivre par `cx.subscribe`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextInputEvent {
    /// Le texte a changé par une saisie de l'utilisateur (pas par
    /// [`TextInput::set_text`]).
    Changed(SharedString),
    /// Entrée (ou Ctrl+Entrée sur plusieurs lignes) : le texte est validé.
    Submitted(SharedString),
}

/// Vérification du texte : `Some(message)` signale une erreur.
pub type Validator = Box<dyn Fn(&str) -> Option<SharedString>>;

/// Mise en page du dernier rendu, pour la souris et les flèches verticales.
struct InputLayout {
    /// Lignes affichées avec leur décalage de début dans le texte.
    lines: Vec<(usize, ShapedLine)>,
    /// Rectangle du texte.
    bounds: Bounds<Pixels>,
    /// Hauteur de ligne.
    line_height: Pixels,
}

/// Champ de saisie, détenu dans une `Entity`.
///
/// ```ignore
/// let bpf = cx.new(|cx| TextInput::multiline(3, cx).placeholder("tcp port 502").mono());
/// cx.subscribe(&bpf, |this, _, event: &TextInputEvent, cx| { … }).detach();
/// ```
pub struct TextInput {
    /// Focus du champ.
    focus_handle: FocusHandle,
    /// Texte et sélection.
    buffer: EditBuffer,
    /// Texte indicatif affiché quand le champ est vide.
    placeholder: SharedString,
    /// Champ désactivé : ni focus ni saisie.
    disabled: bool,
    /// Message d'erreur affiché sous le champ (bordure rouge).
    error: Option<SharedString>,
    /// Vérification appelée à chaque changement.
    validator: Option<Validator>,
    /// Police à chasse fixe.
    mono: bool,
    /// Nombre minimal de lignes affichées (1 pour un champ d'une ligne).
    rows: usize,
    /// Marges réduites (édition dans une cellule de tableau).
    compact: bool,
    /// Mise en page du dernier rendu.
    layout: Option<InputLayout>,
    /// Défilement horizontal d'un champ d'une ligne.
    scroll_x: Pixels,
    /// Sélection à la souris en cours.
    selecting: bool,
}

impl EventEmitter<TextInputEvent> for TextInput {}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl TextInput {
    /// Champ d'une ligne.
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle().tab_stop(true),
            buffer: EditBuffer::new(false),
            placeholder: SharedString::default(),
            disabled: false,
            error: None,
            validator: None,
            mono: false,
            rows: 1,
            compact: false,
            layout: None,
            scroll_x: px(0.),
            selecting: false,
        }
    }

    /// Champ de plusieurs lignes, `rows` lignes au moins.
    pub fn multiline(rows: usize, cx: &mut Context<Self>) -> Self {
        Self {
            buffer: EditBuffer::new(true),
            rows: rows.max(1),
            ..Self::new(cx)
        }
    }

    /// Texte indicatif.
    pub fn placeholder(mut self, text: impl Into<SharedString>) -> Self {
        self.placeholder = text.into();
        self
    }

    /// Texte initial.
    pub fn with_text(mut self, text: &str) -> Self {
        self.buffer.set_text(text);
        self.revalidate();
        self
    }

    /// Police à chasse fixe (adresses, expressions BPF).
    pub fn mono(mut self) -> Self {
        self.mono = true;
        self
    }

    /// Marges réduites, pour une cellule de tableau.
    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }

    /// Désactive le champ.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Vérification appelée à chaque changement ; son message s'affiche
    /// sous le champ.
    pub fn validator(mut self, validator: impl Fn(&str) -> Option<SharedString> + 'static) -> Self {
        self.validator = Some(Box::new(validator));
        self.revalidate();
        self
    }

    /// Texte courant.
    pub fn text(&self) -> &str {
        self.buffer.text()
    }

    /// Tampon d'édition (lecture seule).
    pub fn buffer(&self) -> &EditBuffer {
        &self.buffer
    }

    /// Remplace le texte par programme ; n'émet pas `Changed`.
    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.buffer.set_text(text);
        self.scroll_x = px(0.);
        self.revalidate();
        cx.notify();
    }

    /// Change le texte indicatif.
    pub fn set_placeholder(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.placeholder = text.into();
        cx.notify();
    }

    /// Active ou désactive le champ.
    pub fn set_disabled(&mut self, disabled: bool, cx: &mut Context<Self>) {
        self.disabled = disabled;
        cx.notify();
    }

    /// Champ désactivé.
    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    /// Force un message d'erreur (`None` l'efface).
    pub fn set_error(&mut self, error: Option<SharedString>, cx: &mut Context<Self>) {
        self.error = error;
        cx.notify();
    }

    /// Message d'erreur courant.
    pub fn error(&self) -> Option<&SharedString> {
        self.error.as_ref()
    }

    /// Donne le focus au champ.
    pub fn focus(&self, window: &mut Window) {
        if !self.disabled {
            window.focus(&self.focus_handle);
        }
    }

    /// Relance la vérification.
    fn revalidate(&mut self) {
        if let Some(validator) = self.validator.as_ref() {
            self.error = validator(self.buffer.text());
        }
    }

    /// Après une saisie : vérification, événement, nouveau rendu.
    fn edited(&mut self, cx: &mut Context<Self>) {
        self.revalidate();
        cx.emit(TextInputEvent::Changed(
            self.buffer.text().to_string().into(),
        ));
        cx.notify();
    }

    /// Applique une opération d'édition et signale le changement.
    fn edit(&mut self, cx: &mut Context<Self>, op: impl FnOnce(&mut EditBuffer) -> bool) {
        if self.disabled {
            return;
        }
        if op(&mut self.buffer) {
            self.edited(cx);
        } else {
            cx.notify();
        }
    }

    /// Applique un déplacement du curseur.
    fn motion(&mut self, cx: &mut Context<Self>, op: impl FnOnce(&mut EditBuffer)) {
        op(&mut self.buffer);
        cx.notify();
    }

    /// Décalage visé par une flèche verticale, à abscisse constante.
    fn vertical_target(&self, down: bool) -> usize {
        let text_len = self.buffer.text().len();
        let cursor = self.buffer.cursor();
        let fallback = if down { text_len } else { 0 };
        let Some(layout) = self.layout.as_ref() else {
            return fallback;
        };
        let current: Vec<_> = self.buffer.lines().collect();
        let stale = layout.lines.len() != current.len()
            || layout
                .lines
                .iter()
                .zip(&current)
                .any(|((start, line), (cur_start, cur))| start != cur_start || line.text != *cur);
        if stale || !self.buffer.is_multiline() {
            return fallback;
        }
        let line_ix = self.buffer.line_index(cursor);
        let target_ix = if down {
            line_ix + 1
        } else if line_ix == 0 {
            return 0;
        } else {
            line_ix - 1
        };
        let (Some((start, line)), Some((target_start, target_line))) =
            (layout.lines.get(line_ix), layout.lines.get(target_ix))
        else {
            return fallback;
        };
        let x = line.x_for_index(cursor - start);
        target_start + target_line.closest_index_for_x(x)
    }

    /// Décalage du texte sous un point de la fenêtre.
    fn index_for_position(&self, position: Point<Pixels>) -> usize {
        let Some(layout) = self.layout.as_ref() else {
            return 0;
        };
        if self.buffer.text().is_empty() || layout.lines.is_empty() {
            return 0;
        }
        let rel_y = position.y - layout.bounds.top();
        let line_ix = if rel_y < px(0.) {
            0
        } else {
            ((rel_y / layout.line_height).floor() as usize).min(layout.lines.len() - 1)
        };
        let Some((start, line)) = layout.lines.get(line_ix) else {
            return 0;
        };
        let x = position.x - layout.bounds.left() + self.scroll_x;
        start + line.closest_index_for_x(x)
    }

    /// Bouton de souris enfoncé dans le champ.
    fn on_mouse_down(&mut self, event: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        let index = self.index_for_position(event.position);
        match event.click_count {
            2 => {
                let word = self.buffer.word_range_at(index);
                self.buffer.select_range(word);
            }
            n if n >= 3 => {
                let start = self.buffer.line_start(index);
                let end = self.buffer.line_end(index);
                self.buffer.select_range(start..end);
            }
            _ => {
                self.selecting = true;
                if event.modifiers.shift {
                    self.buffer.select_to(index);
                } else {
                    self.buffer.move_to(index);
                }
            }
        }
        cx.notify();
    }

    /// Bouton de souris relâché.
    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.selecting = false;
    }

    /// Souris déplacée : étend la sélection en cours.
    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.selecting && event.pressed_button == Some(MouseButton::Left) {
            let index = self.index_for_position(event.position);
            self.buffer.select_to(index);
            cx.notify();
        }
    }

    /// Copie la sélection dans le presse-papiers.
    fn copy_selection(&self, cx: &mut Context<Self>) {
        if !self.buffer.selection().is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.buffer.selected_text().to_string(),
            ));
        }
    }
}

/// Déclare des écouteurs de déplacement qui délèguent au tampon.
macro_rules! motions {
    ($($fn_name:ident: $action:ident => $method:ident),* $(,)?) => {
        impl TextInput {
            $(
                #[doc = concat!("Action [`", stringify!($action), "`].")]
                fn $fn_name(&mut self, _: &$action, _: &mut Window, cx: &mut Context<Self>) {
                    self.motion(cx, |b| b.$method());
                }
            )*
        }
    };
}

motions! {
    on_left: Left => left,
    on_right: Right => right,
    on_select_left: SelectLeft => select_left,
    on_select_right: SelectRight => select_right,
    on_word_left: WordLeft => word_left,
    on_word_right: WordRight => word_right,
    on_select_word_left: SelectWordLeft => select_word_left,
    on_select_word_right: SelectWordRight => select_word_right,
    on_select_all: SelectAll => select_all,
    on_home: Home => home,
    on_end: End => end,
    on_select_home: SelectHome => select_home,
    on_select_end: SelectEnd => select_end,
}

impl TextInput {
    /// Action [`Up`].
    fn on_up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        let target = self.vertical_target(false);
        self.motion(cx, |b| b.move_to(target));
    }

    /// Action [`Down`].
    fn on_down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        let target = self.vertical_target(true);
        self.motion(cx, |b| b.move_to(target));
    }

    /// Action [`SelectUp`].
    fn on_select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        let target = self.vertical_target(false);
        self.motion(cx, |b| b.select_to(target));
    }

    /// Action [`SelectDown`].
    fn on_select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        let target = self.vertical_target(true);
        self.motion(cx, |b| b.select_to(target));
    }

    /// Action [`Backspace`].
    fn on_backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, EditBuffer::backspace);
    }

    /// Action [`Delete`].
    fn on_delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, EditBuffer::delete);
    }

    /// Action [`DeleteWordLeft`].
    fn on_delete_word_left(&mut self, _: &DeleteWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, EditBuffer::delete_word_left);
    }

    /// Action [`Newline`].
    fn on_newline(&mut self, _: &Newline, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, |b| b.insert("\n"));
    }

    /// Action [`Paste`].
    fn on_paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.edit(cx, |b| b.insert(&text));
        }
    }

    /// Action [`Copy`].
    fn on_copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        self.copy_selection(cx);
    }

    /// Action [`Cut`].
    fn on_cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        if !self.buffer.selection().is_empty() {
            self.copy_selection(cx);
            self.edit(cx, |b| b.insert(""));
        }
    }

    /// Action [`Submit`].
    fn on_submit(&mut self, _: &Submit, _: &mut Window, cx: &mut Context<Self>) {
        if !self.disabled {
            cx.emit(TextInputEvent::Submitted(
                self.buffer.text().to_string().into(),
            ));
        }
    }

    /// Action [`ShowCharacterPalette`].
    fn on_character_palette(
        &mut self,
        _: &ShowCharacterPalette,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        window.show_character_palette();
    }
}

impl EntityInputHandler for TextInput {
    fn text_for_range(
        &mut self,
        range_utf16: std::ops::Range<usize>,
        actual_range: &mut Option<std::ops::Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.buffer.range_from_utf16(&range_utf16);
        actual_range.replace(self.buffer.range_to_utf16(&range));
        self.buffer.text().get(range).map(str::to_string)
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.buffer.range_to_utf16(&self.buffer.selection()),
            reversed: self.buffer.is_reversed(),
        })
    }

    fn marked_text_range(
        &self,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<std::ops::Range<usize>> {
        self.buffer
            .marked()
            .map(|range| self.buffer.range_to_utf16(&range))
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.buffer.unmark();
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<std::ops::Range<usize>>,
        new_text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16.map(|r| self.buffer.range_from_utf16(&r));
        self.edit(cx, |b| b.replace(range, new_text));
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<std::ops::Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<std::ops::Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16.map(|r| self.buffer.range_from_utf16(&r));
        // Sélection relative au texte inséré : conversion UTF-16 → UTF-8
        // dans `new_text`, pas dans tout le contenu.
        let relative = new_selected_range_utf16.map(|r| {
            let to_utf8 = |utf16: usize| {
                let mut units = 0;
                let mut bytes = 0;
                for c in new_text.chars() {
                    if units >= utf16 {
                        break;
                    }
                    units += c.len_utf16();
                    bytes += c.len_utf8();
                }
                bytes
            };
            to_utf8(r.start)..to_utf8(r.end)
        });
        self.edit(cx, |b| b.replace_and_mark(range, new_text, relative));
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: std::ops::Range<usize>,
        _element_bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let layout = self.layout.as_ref()?;
        let range = self.buffer.range_from_utf16(&range_utf16);
        let line_ix = self.buffer.line_index(range.start);
        let (start, line) = layout.lines.get(line_ix)?;
        let top = layout.bounds.top() + layout.line_height * line_ix as f32;
        let left = layout.bounds.left() - self.scroll_x;
        let end_in_line = range.end.min(start + line.len());
        Some(Bounds::from_corners(
            point(left + line.x_for_index(range.start - start), top),
            point(
                left + line.x_for_index(end_in_line.saturating_sub(*start)),
                top + layout.line_height,
            ),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let index = self.index_for_position(point);
        Some(self.buffer.offset_to_utf16(index))
    }
}

impl Render for TextInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let focused = !self.disabled && self.focus_handle.is_focused(window);
        let border = if self.error.is_some() {
            theme.danger_text
        } else if focused {
            theme.focus_ring
        } else {
            theme.border
        };
        let hover_border = theme.border_strong;
        let font = if self.mono {
            theme.font_mono.clone()
        } else {
            theme.font_family.clone()
        };
        let multiline = self.buffer.is_multiline();
        let context = if multiline {
            "LunarTextInput multiline"
        } else {
            "LunarTextInput"
        };

        let field = div()
            .id("champ")
            .flex()
            .w_full()
            .px(if self.compact { px(6.) } else { px(10.) })
            .py(if self.compact { px(1.) } else { px(6.) })
            .rounded(if self.compact {
                theme.radius_sm
            } else {
                theme.radius_md
            })
            .border_1()
            .border_color(border)
            .bg(theme.bg_field)
            .text_color(theme.text_field)
            .text_size(theme.text_md)
            .line_height(px(18.))
            .font_family(font)
            .when(!focused && self.error.is_none() && !self.disabled, |d| {
                d.hover(move |s| s.border_color(hover_border))
            })
            .child(TextElement { input: cx.entity() });

        let field = if self.disabled {
            field
                .opacity(theme.disabled_opacity)
                .cursor(CursorStyle::OperationNotAllowed)
        } else {
            field
                .key_context(context)
                .track_focus(&self.focus_handle)
                .tab_index(0)
                .cursor(CursorStyle::IBeam)
                .on_action(cx.listener(Self::on_backspace))
                .on_action(cx.listener(Self::on_delete))
                .on_action(cx.listener(Self::on_delete_word_left))
                .on_action(cx.listener(Self::on_left))
                .on_action(cx.listener(Self::on_right))
                .on_action(cx.listener(Self::on_up))
                .on_action(cx.listener(Self::on_down))
                .on_action(cx.listener(Self::on_select_left))
                .on_action(cx.listener(Self::on_select_right))
                .on_action(cx.listener(Self::on_select_up))
                .on_action(cx.listener(Self::on_select_down))
                .on_action(cx.listener(Self::on_word_left))
                .on_action(cx.listener(Self::on_word_right))
                .on_action(cx.listener(Self::on_select_word_left))
                .on_action(cx.listener(Self::on_select_word_right))
                .on_action(cx.listener(Self::on_select_all))
                .on_action(cx.listener(Self::on_home))
                .on_action(cx.listener(Self::on_end))
                .on_action(cx.listener(Self::on_select_home))
                .on_action(cx.listener(Self::on_select_end))
                .on_action(cx.listener(Self::on_paste))
                .on_action(cx.listener(Self::on_copy))
                .on_action(cx.listener(Self::on_cut))
                .on_action(cx.listener(Self::on_submit))
                .on_action(cx.listener(Self::on_newline))
                .on_action(cx.listener(Self::on_character_palette))
                .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
                .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
                .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
                .on_mouse_move(cx.listener(Self::on_mouse_move))
        };

        div()
            .flex()
            .flex_col()
            .gap(theme.space_xs)
            .w_full()
            .child(field)
            .when_some(self.error.clone(), |d, error| {
                d.child(
                    div()
                        .text_size(theme.text_xs)
                        .text_color(theme.danger_text)
                        .child(error),
                )
            })
    }
}

/// Élément de texte du champ : mise en forme, sélection, curseur, saisie.
struct TextElement {
    /// Champ dessiné.
    input: Entity<TextInput>,
}

/// Résultat de la préparation du dessin.
struct PrepaintState {
    /// Lignes mises en forme, avec leur décalage de début.
    lines: Vec<(usize, ShapedLine)>,
    /// Texte indicatif mis en forme (champ vide).
    placeholder: Option<ShapedLine>,
    /// Rectangles de sélection.
    selections: Vec<PaintQuad>,
    /// Curseur.
    cursor: Option<PaintQuad>,
    /// Défilement horizontal retenu.
    scroll_x: Pixels,
    /// Hauteur de ligne.
    line_height: Pixels,
}

impl IntoElement for TextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let input = self.input.read(cx);
        let rows = if input.buffer.is_multiline() {
            input.rows.max(input.buffer.lines().count())
        } else {
            1
        };
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = (window.line_height() * rows as f32).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let theme = cx.theme();
        let selection_color = theme.focus_ring.opacity(0.35);
        let cursor_color = theme.text;
        let placeholder_color = theme.text_placeholder;
        let input = self.input.read(cx);
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line_height = window.line_height();
        let base = TextRun {
            len: 0,
            font: style.font(),
            color: style.color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let marked = input.buffer.marked();
        let shape = |text: &str, start: usize, window: &mut Window| {
            let end = start + text.len();
            let mut runs = Vec::new();
            match marked.as_ref() {
                Some(m) if m.start < end && m.end > start => {
                    let m_start = m.start.max(start) - start;
                    let m_end = m.end.min(end) - start;
                    runs.push(TextRun {
                        len: m_start,
                        ..base.clone()
                    });
                    runs.push(TextRun {
                        len: m_end - m_start,
                        underline: Some(UnderlineStyle {
                            color: Some(base.color),
                            thickness: px(1.),
                            wavy: false,
                        }),
                        ..base.clone()
                    });
                    runs.push(TextRun {
                        len: text.len() - m_end,
                        ..base.clone()
                    });
                }
                _ => runs.push(TextRun {
                    len: text.len(),
                    ..base.clone()
                }),
            }
            runs.retain(|run| run.len > 0);
            window.text_system().shape_line(
                SharedString::from(text.to_string()),
                font_size,
                &runs,
                None,
            )
        };

        let text_lines: Vec<(usize, String)> = input
            .buffer
            .lines()
            .map(|(start, line)| (start, line.to_string()))
            .collect();
        let selection = input.buffer.selection();
        let cursor = input.buffer.cursor();
        let multiline = input.buffer.is_multiline();
        let previous_scroll = input.scroll_x;
        let placeholder_text = input.placeholder.clone();
        let empty = input.buffer.text().is_empty();

        let lines: Vec<(usize, ShapedLine)> = text_lines
            .iter()
            .map(|(start, line)| (*start, shape(line, *start, window)))
            .collect();
        let placeholder = (empty && !placeholder_text.is_empty()).then(|| {
            window.text_system().shape_line(
                placeholder_text.clone(),
                font_size,
                &[TextRun {
                    len: placeholder_text.len(),
                    color: placeholder_color,
                    ..base.clone()
                }],
                None,
            )
        });

        // Position du curseur et défilement horizontal (une seule ligne).
        let cursor_line = lines
            .iter()
            .rposition(|(start, _)| *start <= cursor)
            .unwrap_or(0);
        let cursor_x = lines
            .get(cursor_line)
            .map_or(px(0.), |(start, line)| line.x_for_index(cursor - start));
        let mut scroll_x = previous_scroll;
        if multiline {
            scroll_x = px(0.);
        } else {
            let width = bounds.size.width;
            let margin = px(2.);
            if cursor_x - scroll_x > width - margin {
                scroll_x = cursor_x - width + margin;
            }
            if cursor_x < scroll_x {
                scroll_x = cursor_x;
            }
            let line_width = lines.first().map_or(px(0.), |(_, line)| line.width);
            let max_scroll = line_width - width + margin;
            if scroll_x > max_scroll {
                scroll_x = max_scroll;
            }
            if scroll_x < px(0.) {
                scroll_x = px(0.);
            }
        }

        let origin_x = bounds.left() - scroll_x;
        let mut selections = Vec::new();
        if !selection.is_empty() {
            for (ix, (start, line)) in lines.iter().enumerate() {
                let end = start + line.len();
                if selection.start > end || selection.end < *start {
                    continue;
                }
                let from = selection.start.max(*start) - start;
                let to = selection.end.min(end) - start;
                let mut right = origin_x + line.x_for_index(to);
                if selection.end > end {
                    // La sélection franchit le retour à la ligne.
                    right += px(6.);
                }
                let top = bounds.top() + line_height * ix as f32;
                selections.push(fill(
                    Bounds::from_corners(
                        point(origin_x + line.x_for_index(from), top),
                        point(right, top + line_height),
                    ),
                    selection_color,
                ));
            }
        }
        let cursor = selection.is_empty().then(|| {
            fill(
                Bounds::new(
                    point(
                        origin_x + cursor_x,
                        bounds.top() + line_height * cursor_line as f32,
                    ),
                    size(px(1.5), line_height),
                ),
                cursor_color,
            )
        });
        PrepaintState {
            lines,
            placeholder,
            selections,
            cursor,
            scroll_x,
            line_height,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let (focus_handle, disabled) = {
            let input = self.input.read(cx);
            (input.focus_handle.clone(), input.disabled)
        };
        if !disabled {
            window.handle_input(
                &focus_handle,
                ElementInputHandler::new(bounds, self.input.clone()),
                cx,
            );
        }
        let line_height = prepaint.line_height;
        let origin_x = bounds.left() - prepaint.scroll_x;
        let lines = std::mem::take(&mut prepaint.lines);
        let selections = std::mem::take(&mut prepaint.selections);
        let placeholder = prepaint.placeholder.take();
        let cursor = prepaint.cursor.take();
        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
            for selection in selections {
                window.paint_quad(selection);
            }
            if let Some(placeholder) = placeholder.as_ref() {
                let _ = placeholder.paint(bounds.origin, line_height, window, cx);
            }
            for (ix, (_, line)) in lines.iter().enumerate() {
                let origin = point(origin_x, bounds.top() + line_height * ix as f32);
                let _ = line.paint(origin, line_height, window, cx);
            }
            if !disabled
                && focus_handle.is_focused(window)
                && let Some(cursor) = cursor
            {
                window.paint_quad(cursor);
            }
        });
        let scroll_x = prepaint.scroll_x;
        self.input.update(cx, |input, _| {
            input.scroll_x = scroll_x;
            input.layout = Some(InputLayout {
                lines,
                bounds,
                line_height,
            });
        });
    }
}
