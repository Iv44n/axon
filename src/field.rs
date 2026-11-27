//! A line of text being edited: the field of a dialog (`.f-input`), of the overview's filter (`.ov-search`)
//! and of the palette (`.pal-input-row`). A state machine over `plat::Key` with a caret, a selection and a
//! longest length, and the painting of the three looks in the glass style of the reference.
//!
//! The model is pure: `Field::key` takes a key and says what happened (`Edit`), the clipboard is the app's
//! (`Edit::Paste` asks, `insert` takes the text), and positions are character indices, so it is tested
//! without a window or a font. The caret is solid, never blinking: a blinking caret would wake the process
//! (idle is 0 % CPU). Where the text starts when it is longer than the field is a function of the caret, not
//! state: the caret is kept in view by showing the text's end up to it.

use crate::{
    icons::{self, Icon},
    input::sym,
    overlay::{Ink, PX, PX_INPUT, Rect, Text, accent, dim, faint, fg, ring},
    paint::{Paint, WHITE},
    plat::Key,
};

/// What a key did to the field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Edit {
    /// Not a key of the field (Enter, Tab, Up, Down, Esc, a shortcut): it is the owner's.
    Ignored,
    /// A key of the field that changed nothing (Left at the start, a letter past the longest length).
    Same,
    /// The caret or the selection moved: paint again.
    Moved,
    /// The text changed.
    Changed,
    /// Wants the clipboard's text: hand it to `insert`.
    Paste,
    /// The selected text, to put on the clipboard.
    Copy(String),
    /// The selected text, to put on the clipboard; it is gone from the field.
    Cut(String),
}

#[derive(Clone, Debug, Default)]
pub struct Field {
    text: String,
    /// The caret, and where the selection started: both are character indices; equal when nothing is selected.
    caret: usize,
    anchor: usize,
    /// The longest text in characters (0: no limit).
    max: usize,
    placeholder: std::borrow::Cow<'static, str>,
    /// Which characters it takes (none: any): a number's box takes digits, a sign and a point.
    accept: Option<fn(char) -> bool>,
    /// The primary button is down in the field: a pointer move extends the selection.
    dragging: bool,
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl Field {
    pub fn new(max: usize, placeholder: &'static str) -> Field {
        Field { max, placeholder: placeholder.into(), ..Field::default() }
    }

    /// A field whose placeholder is made when it opens (the local user's name).
    pub fn with_placeholder(mut self, s: String) -> Field {
        self.placeholder = s.into();
        self
    }

    /// The field takes only the characters `accept` says yes to (typed or pasted).
    #[cfg(test)]
    pub fn only(mut self, accept: fn(char) -> bool) -> Field {
        self.accept = Some(accept);
        self
    }

    /// A field for a number: digits, a point or comma, a minus.
    #[cfg(test)]
    pub fn number(self) -> Field {
        self.only(|c| c.is_ascii_digit() || matches!(c, '.' | ',' | '-'))
    }

    /// The field starting with `s`, the caret after it.
    #[cfg(test)]
    pub fn with_text(mut self, s: &str) -> Field {
        self.set(s);
        self
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn placeholder(&self) -> &str {
        &self.placeholder
    }

    /// Replaces the text; the caret goes to the end and nothing is selected.
    pub fn set(&mut self, s: &str) {
        self.text.clear();
        self.caret = 0;
        self.anchor = 0;
        self.insert(s);
    }

    fn len(&self) -> usize {
        self.text.chars().count()
    }

    /// The byte offset of character `ci`.
    fn byte(&self, ci: usize) -> usize {
        self.text.char_indices().nth(ci).map_or(self.text.len(), |(b, _)| b)
    }

    /// The text before character `ci`.
    pub fn before(&self, ci: usize) -> &str {
        &self.text[..self.byte(ci)]
    }

    pub fn caret(&self) -> usize {
        self.caret
    }

    /// The selected characters `(from, to)`, if any.
    pub fn selection(&self) -> Option<(usize, usize)> {
        (self.caret != self.anchor).then(|| (self.caret.min(self.anchor), self.caret.max(self.anchor)))
    }

    pub fn selected(&self) -> &str {
        match self.selection() {
            Some((a, b)) => &self.text[self.byte(a)..self.byte(b)],
            None => "",
        }
    }

    pub fn select_all(&mut self) {
        (self.anchor, self.caret) = (0, self.len());
    }

    /// Selects the word under the caret (or the run of spaces and signs it is in).
    pub fn select_word(&mut self) {
        let cs: Vec<char> = self.text.chars().collect();
        let at = self.caret.min(cs.len());
        let kind = |i: usize| cs.get(i).map(|&c| is_word(c));
        // A word is the run of word characters around the caret; on a gap, the run of non-word ones.
        let want = kind(at).or_else(|| at.checked_sub(1).and_then(kind));
        let Some(want) = want else { return };
        let (mut a, mut b) = (at, at);
        while a > 0 && kind(a - 1) == Some(want) {
            a -= 1;
        }
        while b < cs.len() && kind(b) == Some(want) {
            b += 1;
        }
        (self.anchor, self.caret) = (a, b);
    }

    fn go(&mut self, to: usize, extend: bool) -> Edit {
        let before = (self.caret, self.anchor);
        self.caret = to;
        if !extend {
            self.anchor = to;
        }
        if (self.caret, self.anchor) == before { Edit::Same } else { Edit::Moved }
    }

    fn word_left(&self, from: usize) -> usize {
        let cs: Vec<char> = self.text.chars().collect();
        let mut i = from.min(cs.len());
        while i > 0 && !is_word(cs[i - 1]) {
            i -= 1;
        }
        while i > 0 && is_word(cs[i - 1]) {
            i -= 1;
        }
        i
    }

    fn word_right(&self, from: usize) -> usize {
        let cs: Vec<char> = self.text.chars().collect();
        let mut i = from.min(cs.len());
        while i < cs.len() && !is_word(cs[i]) {
            i += 1;
        }
        while i < cs.len() && is_word(cs[i]) {
            i += 1;
        }
        i
    }

    /// Removes characters `a..b` and puts the caret at `a`.
    fn cut(&mut self, a: usize, b: usize) {
        let (ba, bb) = (self.byte(a), self.byte(b));
        self.text.replace_range(ba..bb, "");
        self.caret = a;
        self.anchor = a;
    }

    /// Deletes the selection if there is one. True if it did.
    fn delete_selection(&mut self) -> bool {
        match self.selection() {
            Some((a, b)) => {
                self.cut(a, b);
                true
            }
            None => false,
        }
    }

    /// Types `s` over the selection: line breaks become a space, other control characters go, and what does
    /// not fit in the longest length is left out. True if the text changed.
    pub fn insert(&mut self, s: &str) -> bool {
        let before = self.text.clone();
        let mut clean = String::with_capacity(s.len());
        let mut last_break = false;
        for c in s.chars() {
            let brk = c == '\n' || c == '\r';
            if brk {
                if !last_break {
                    clean.push(' ');
                }
            } else if !c.is_control() && self.accept.is_none_or(|ok| ok(c)) {
                clean.push(c);
            }
            last_break = brk;
        }
        self.delete_selection();
        let room = if self.max == 0 { usize::MAX } else { self.max.saturating_sub(self.len()) };
        let take: String = clean.chars().take(room).collect();
        let at = self.byte(self.caret);
        self.text.insert_str(at, &take);
        self.caret += take.chars().count();
        self.anchor = self.caret;
        self.text != before
    }

    /// A key press. Returns what it did, or `Ignored` for the keys that are the owner's.
    pub fn key(&mut self, k: &Key) -> Edit {
        let (ctrl, alt, shift) = (k.mods.ctrl, k.mods.alt, k.mods.shift);
        let n = self.len();
        let letter = char::from_u32(k.sym).map(|c| c.to_ascii_lowercase());
        match k.sym {
            sym::LEFT | sym::KP_LEFT if !alt => {
                if let (Some((a, _)), false, false) = (self.selection(), shift, ctrl) {
                    return self.go(a, false);
                }
                self.go(if ctrl { self.word_left(self.caret) } else { self.caret.saturating_sub(1) }, shift)
            }
            sym::RIGHT | sym::KP_RIGHT if !alt => {
                if let (Some((_, b)), false, false) = (self.selection(), shift, ctrl) {
                    return self.go(b, false);
                }
                self.go(if ctrl { self.word_right(self.caret) } else { (self.caret + 1).min(n) }, shift)
            }
            sym::HOME | sym::KP_HOME => self.go(0, shift),
            sym::END | sym::KP_END => self.go(n, shift),
            sym::BACKSPACE if !alt => {
                if self.delete_selection() {
                    return Edit::Changed;
                }
                let from = if ctrl { self.word_left(self.caret) } else { self.caret.saturating_sub(1) };
                if from == self.caret {
                    return Edit::Same;
                }
                self.cut(from, self.caret);
                Edit::Changed
            }
            sym::DELETE | sym::KP_DELETE if !alt => {
                if self.delete_selection() {
                    return Edit::Changed;
                }
                let to = if ctrl { self.word_right(self.caret) } else { (self.caret + 1).min(n) };
                if to == self.caret {
                    return Edit::Same;
                }
                self.cut(self.caret, to);
                Edit::Changed
            }
            sym::INSERT if shift && !ctrl => Edit::Paste,
            _ if ctrl && !alt => match letter {
                Some('a') => {
                    let before = (self.caret, self.anchor);
                    self.select_all();
                    if (self.caret, self.anchor) == before { Edit::Same } else { Edit::Moved }
                }
                Some('v') => Edit::Paste,
                Some('c') if self.selection().is_some() => Edit::Copy(self.selected().to_string()),
                Some('x') if self.selection().is_some() => {
                    let text = self.selected().to_string();
                    self.delete_selection();
                    Edit::Cut(text)
                }
                Some('c' | 'x') => Edit::Same,
                _ => Edit::Ignored,
            },
            _ if !ctrl && !alt && !k.text.is_empty() && !k.text.chars().any(char::is_control) => {
                if self.insert(&k.text) { Edit::Changed } else { Edit::Same }
            }
            _ => Edit::Ignored,
        }
    }

    // ---------------------------------------------------------------- pointer --

    /// The character boundary nearest to `x` pixels from the text's start, `width` measuring a string. The
    /// widths of the characters are added up: kerning is not applied (nor is it in the drawing).
    pub fn index_at(&self, x: i32, width: &dyn Fn(&str) -> i32) -> usize {
        let mut acc = 0;
        for (i, c) in self.text.chars().enumerate() {
            let next = acc + width(c.encode_utf8(&mut [0; 4]));
            if x < (acc + next + 1) / 2 {
                return i;
            }
            acc = next;
        }
        self.len()
    }

    /// The primary button went down at `x` pixels from the text's start: places the caret (`clicks` 1; with
    /// `extend` the selection grows to it), selects the word (2) or everything (3 and more); a move then drags.
    pub fn press(&mut self, x: i32, width: &dyn Fn(&str) -> i32, clicks: u8, extend: bool) {
        let at = self.index_at(x, width);
        self.dragging = true;
        match clicks {
            0 | 1 => {
                self.go(at, extend);
            }
            2 => {
                self.go(at, false);
                self.select_word();
            }
            _ => self.select_all(),
        }
    }

    /// The pointer moved with the button down: the selection grows to `x`.
    pub fn drag(&mut self, x: i32, width: &dyn Fn(&str) -> i32) -> bool {
        if !self.dragging {
            return false;
        }
        let at = self.index_at(x, width);
        self.go(at, true) == Edit::Moved
    }

    pub fn release(&mut self) {
        self.dragging = false;
    }

    pub fn dragging(&self) -> bool {
        self.dragging
    }

    // ------------------------------------------------------------------ view --

    /// Where things are inside a field `inner` pixels wide, from the text's start: how far the text is
    /// shifted left to keep the caret in view, the caret's x and the selection's x range, after the shift.
    pub fn view(&self, inner: i32, width: &dyn Fn(&str) -> i32) -> View {
        let at = |ci: usize| width(self.before(ci));
        let caret = at(self.caret);
        let scroll = (caret + 2 - inner).max(0);
        View { scroll, caret: caret - scroll, sel: self.selection().map(|(a, b)| (at(a) - scroll, at(b) - scroll)) }
    }
}

/// What `Field::view` says: all x positions are from the text's origin, after the shift.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct View {
    pub scroll: i32,
    pub caret: i32,
    pub sel: Option<(i32, i32)>,
}

// ----------------------------------------------------------------- painting --

/// The three looks of a field in the reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    /// `.f-input`: a 36 px box, black 35 % fill, outline, an amber glow when it has the focus.
    Box,
    /// `.ov-search`: a 30 px box with a search icon, 220 px wide in the overview's header.
    Search,
    /// `.h-search`: the same box in the panel of servers, whose outline turns white (25 %), not amber, with the keyboard.
    Find,
    /// `.pal-input-row`: no box, a 16 px icon and 14.5 px text in a row with a hairline under it.
    Row,
}

/// The heights of the looks.
pub const H_BOX: i32 = 36;
pub const H_SEARCH: i32 = 30;
pub const H_ROW: i32 = 49;

/// Where a field's text goes inside its rectangle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    /// The text's left edge and the width it may use (window pixels).
    pub tx: i32,
    pub inner: i32,
    pub px: f32,
    pub cy: i32,
}

pub fn place((x, y, w, h): Rect, look: Look) -> Place {
    match look {
        Look::Box => Place { tx: x + 1 + 12, inner: w - 2 - 24, px: PX, cy: y + h / 2 },
        Look::Search | Look::Find => Place { tx: x + 1 + 10 + 14 + 8, inner: w - 2 - 10 - 14 - 8 - 10, px: PX, cy: y + h / 2 },
        Look::Row => Place { tx: x + 16 + 16 + 10, inner: w - 16 - 16 - 10 - 16, px: PX_INPUT, cy: y + (h - 1) / 2 },
    }
}

impl Field {
    /// The primary button went down at window pixel `x` in the field painted at `r`.
    pub fn press_at(&mut self, x: f32, r: Rect, look: Look, t: &Text, clicks: u8, extend: bool) {
        let pl = place(r, look);
        let width = |s: &str| t.width(s, pl.px);
        let shift = self.view(pl.inner, &width).scroll;
        self.press(x as i32 - pl.tx + shift, &width, clicks, extend);
    }

    /// The pointer moved to window pixel `x` with the button held down.
    pub fn drag_at(&mut self, x: f32, r: Rect, look: Look, t: &Text) -> bool {
        let pl = place(r, look);
        let width = |s: &str| t.width(s, pl.px);
        let shift = self.view(pl.inner, &width).scroll;
        self.drag(x as i32 - pl.tx + shift, &width)
    }
}

/// Paints the field `f` in rectangle `r` (the box, or the row). `focused`: the caret shows, the border and
/// the icon light up.
pub fn paint(p: &mut Paint, t: &Text, f: &Field, r: Rect, look: Look, focused: bool) {
    let (x, y, w, h) = r;
    let pl = place(r, look);
    let line = 0.07;
    match look {
        Look::Box => {
            p.rrect(x, y, w, h, 9.0, [0.0; 3], 0.35);
            if focused {
                ring(p, r, 9.0, 3.0, accent(), 0.16);
                p.outline(x, y, w, h, 9.0, accent(), 0.60);
            } else {
                p.outline(x, y, w, h, 9.0, WHITE, line);
            }
        }
        Look::Search | Look::Find => {
            p.rrect(x, y, w, h, 9.0, [0.0; 3], 0.30);
            let (c, a) = match (look, focused) {
                (Look::Search, true) => (accent(), 0.60),
                (Look::Find, true) => (WHITE, 0.25),
                _ => (WHITE, line),
            };
            p.outline(x, y, w, h, 9.0, c, a);
            icons::paint(p, Icon::Search, (x + 1 + 10) as f32 + 7.0, pl.cy as f32, 14.0, 1.6, if focused { fg() } else { dim() }, 1.0);
        }
        Look::Row => {
            icons::paint(p, Icon::Search, (x + 16) as f32 + 8.0, pl.cy as f32, 16.0, 1.6, dim(), 1.0);
            p.shape(x, y + h - 1, x + w, y + h, WHITE, line, &|_, _| 1.0);
        }
    }
    let width = |s: &str| t.width(s, pl.px);
    let v = f.view(pl.inner, &width);
    let (x0, x1) = (pl.tx, pl.tx + pl.inner);
    let ch = (pl.px * 1.25).round() as i32;
    if let Some((a, b)) = v.sel.filter(|_| focused) {
        let (a, b) = (a.max(0), b.min(pl.inner));
        if b > a {
            p.rrect(pl.tx + a, pl.cy - ch / 2, b - a, ch, 2.0, accent(), 0.30);
        }
    }
    if f.is_empty() {
        t.draw(p, pl.tx, pl.cy, f.placeholder(), Ink::new(pl.px, faint()).clip(x0, x1));
    } else {
        t.draw(p, pl.tx - v.scroll, pl.cy, f.text(), Ink::new(pl.px, fg()).clip(x0, x1));
    }
    if focused {
        let cx = pl.tx + v.caret;
        p.shape(cx, pl.cy - ch / 2, cx + 1, pl.cy - ch / 2 + ch, fg(), 1.0, &|_, _| 1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plat::Mods;

    fn key(sym: u32) -> Key {
        Key { sym, ..Key::default() }
    }

    fn with(sym: u32, shift: bool, ctrl: bool, alt: bool) -> Key {
        Key { sym, mods: Mods { shift, ctrl, alt }, ..Key::default() }
    }

    fn typed(f: &mut Field, s: &str) {
        for c in s.chars() {
            f.key(&Key { sym: c as u32, text: c.to_string(), ..Key::default() });
        }
    }

    fn field(s: &str) -> Field {
        Field::new(0, "type here").with_text(s)
    }

    /// Eight pixels a character.
    fn w8(s: &str) -> i32 {
        s.chars().count() as i32 * 8
    }

    #[test]
    fn typing_puts_text_at_the_caret_and_a_new_field_is_empty_with_its_placeholder() {
        let mut f = Field::new(0, "p. ej. arcton");
        assert!(f.is_empty() && f.placeholder() == "p. ej. arcton");
        typed(&mut f, "helo");
        assert_eq!((f.text(), f.caret()), ("helo", 4));
        f.key(&key(sym::LEFT));
        f.key(&key(sym::LEFT));
        typed(&mut f, "l");
        assert_eq!((f.text(), f.caret()), ("hello", 3));
        assert_eq!(f.selection(), None);
    }

    #[test]
    fn the_arrows_home_and_end_move_the_caret_and_stop_at_the_ends() {
        let mut f = field("abc");
        assert_eq!(f.key(&key(sym::RIGHT)), Edit::Same, "already at the end");
        assert_eq!(f.key(&key(sym::LEFT)), Edit::Moved);
        assert_eq!(f.caret(), 2);
        assert_eq!(f.key(&key(sym::HOME)), Edit::Moved);
        assert_eq!(f.key(&key(sym::LEFT)), Edit::Same, "already at the start");
        assert_eq!(f.key(&key(sym::END)), Edit::Moved);
        assert_eq!(f.caret(), 3);
    }

    #[test]
    fn shift_extends_the_selection_and_an_arrow_without_it_collapses_to_the_side() {
        let mut f = field("hello");
        f.key(&with(sym::LEFT, true, false, false));
        f.key(&with(sym::LEFT, true, false, false));
        assert_eq!((f.selection(), f.selected()), (Some((3, 5)), "lo"));
        f.key(&with(sym::HOME, true, false, false));
        assert_eq!(f.selected(), "hello");
        // Left collapses to the start of the selection, Right to its end.
        f.key(&key(sym::LEFT));
        assert_eq!((f.caret(), f.selection()), (0, None));
        f.key(&with(sym::END, true, false, false));
        f.key(&key(sym::RIGHT));
        assert_eq!((f.caret(), f.selection()), (5, None));
    }

    #[test]
    fn typing_over_a_selection_replaces_it_and_backspace_deletes_it() {
        let mut f = field("hello world");
        f.key(&with(sym::HOME, true, false, false));
        typed(&mut f, "x");
        assert_eq!(f.text(), "x");
        let mut f = field("hello world");
        f.key(&with(sym::LEFT, true, true, false));
        assert_eq!(f.selected(), "world");
        assert_eq!(f.key(&key(sym::BACKSPACE)), Edit::Changed);
        assert_eq!((f.text(), f.caret()), ("hello ", 6));
    }

    #[test]
    fn backspace_and_delete_remove_one_character_and_ctrl_removes_a_word() {
        let mut f = field("one two three");
        f.key(&key(sym::BACKSPACE));
        assert_eq!(f.text(), "one two thre");
        f.key(&with(sym::BACKSPACE, false, true, false));
        assert_eq!(f.text(), "one two ");
        f.key(&with(sym::BACKSPACE, false, true, false));
        assert_eq!(f.text(), "one ");
        f.key(&key(sym::HOME));
        assert_eq!(f.key(&key(sym::BACKSPACE)), Edit::Same, "nothing before the caret");
        f.key(&key(sym::DELETE));
        assert_eq!(f.text(), "ne ");
        f.key(&with(sym::DELETE, false, true, false));
        assert_eq!(f.text(), " ");
        f.key(&key(sym::END));
        assert_eq!(f.key(&key(sym::DELETE)), Edit::Same);
    }

    #[test]
    fn ctrl_arrows_jump_over_words_and_punctuation() {
        let mut f = field("foo.bar baz");
        f.key(&with(sym::LEFT, false, true, false));
        assert_eq!(f.caret(), 8);
        f.key(&with(sym::LEFT, false, true, false));
        assert_eq!(f.caret(), 4);
        f.key(&with(sym::RIGHT, false, true, false));
        assert_eq!(f.caret(), 7);
        f.key(&with(sym::HOME, false, false, false));
        f.key(&with(sym::RIGHT, false, true, false));
        assert_eq!(f.caret(), 3);
    }

    #[test]
    fn ctrl_a_selects_everything_and_ctrl_v_asks_for_the_clipboard() {
        let mut f = field("abc");
        assert_eq!(f.key(&with(0x61, false, true, false)), Edit::Moved);
        assert_eq!(f.selected(), "abc");
        assert_eq!(f.key(&with(0x61, false, true, false)), Edit::Same);
        assert_eq!(f.key(&with(0x76, false, true, false)), Edit::Paste);
        assert_eq!(f.key(&with(0x56, true, true, false)), Edit::Paste, "Ctrl+Shift+V too");
        assert_eq!(f.key(&with(sym::INSERT, true, false, false)), Edit::Paste);
        // The app hands the text in: it replaces the selection.
        assert!(f.insert("xyz"));
        assert_eq!((f.text(), f.caret()), ("xyz", 3));
    }

    #[test]
    fn copy_and_cut_give_the_selected_text_and_do_nothing_without_one() {
        let mut f = field("hello world");
        assert_eq!(f.key(&with(0x63, false, true, false)), Edit::Same);
        f.key(&with(sym::LEFT, true, true, false));
        assert_eq!(f.key(&with(0x63, false, true, false)), Edit::Copy("world".into()));
        assert_eq!(f.text(), "hello world");
        assert_eq!(f.key(&with(0x78, false, true, false)), Edit::Cut("world".into()));
        assert_eq!(f.text(), "hello ");
    }

    #[test]
    fn a_pasted_text_is_one_line_and_clean() {
        let mut f = Field::new(0, "");
        f.insert("ls -la\r\ncd /tmp\n\n\x1b[31mred\x07");
        assert_eq!(f.text(), "ls -la cd /tmp [31mred");
        let mut f = Field::new(0, "");
        assert!(!f.insert("\x07\x1b"), "only control characters: nothing to insert");
    }

    #[test]
    fn a_number_field_takes_only_digits_a_point_and_a_minus() {
        let mut f = Field::new(10, "").number();
        typed(&mut f, "-1a2.5x");
        assert_eq!(f.text(), "-12.5");
        assert!(f.insert(",3 4"), "a paste is filtered the same way");
        assert_eq!(f.text(), "-12.5,34");
        assert_eq!(f.key(&Key { sym: 0x7a, text: "z".into(), ..Key::default() }), Edit::Same);
        let any = Field::new(0, "").only(|c| c.is_ascii_uppercase());
        let mut any = any;
        typed(&mut any, "aBcD");
        assert_eq!(any.text(), "BD");
    }

    #[test]
    fn the_longest_length_cuts_typing_and_pastes() {
        let mut f = Field::new(5, "");
        typed(&mut f, "abcdefg");
        assert_eq!(f.text(), "abcde");
        assert_eq!(f.key(&Key { sym: 0x7a, text: "z".into(), ..Key::default() }), Edit::Same);
        f.key(&with(sym::HOME, false, false, false));
        f.key(&with(sym::RIGHT, true, false, false));
        assert!(f.insert("XYZ"), "the selected character leaves room for one");
        assert_eq!(f.text(), "Xbcde", "five characters at most");
        // Counted in characters, not bytes.
        let mut f = Field::new(3, "");
        f.insert("ñandú");
        assert_eq!(f.text(), "ñan");
    }

    #[test]
    fn keys_that_are_not_the_fields_are_left_to_the_owner() {
        let mut f = field("abc");
        for s in [sym::RETURN, sym::TAB, sym::UP, sym::DOWN, sym::ESCAPE, sym::PAGE_UP, sym::F1] {
            assert_eq!(f.key(&key(s)), Edit::Ignored, "{s:#x}");
        }
        // A shortcut with Alt or Ctrl is not typing.
        assert_eq!(f.key(&Key { sym: 0x6e, text: "n".into(), mods: Mods { alt: true, ..Mods::default() } }), Edit::Ignored);
        assert_eq!(f.key(&Key { sym: 0x6b, text: "\x0b".into(), mods: Mods { ctrl: true, ..Mods::default() } }), Edit::Ignored);
        // Enter and Tab type a control character in their text: not into the field.
        assert_eq!(f.key(&Key { sym: sym::RETURN, text: "\r".into(), ..Key::default() }), Edit::Ignored);
        assert_eq!(f.text(), "abc");
    }

    #[test]
    fn unicode_is_edited_by_character() {
        let mut f = field("añb😀");
        assert_eq!(f.caret(), 4);
        f.key(&key(sym::BACKSPACE));
        assert_eq!(f.text(), "añb");
        f.key(&key(sym::LEFT));
        f.key(&key(sym::LEFT));
        typed(&mut f, "é");
        assert_eq!(f.text(), "aéñb");
        f.key(&key(sym::DELETE));
        assert_eq!(f.text(), "aéb");
    }

    #[test]
    fn set_replaces_the_text_with_the_caret_at_the_end() {
        let mut f = field("old");
        f.key(&with(sym::HOME, true, false, false));
        f.set("new text");
        assert_eq!((f.text(), f.caret(), f.selection()), ("new text", 8, None));
    }

    #[test]
    fn a_double_click_selects_the_word_and_a_triple_click_everything() {
        let mut f = field("foo bar.baz");
        // x = 36 is in the middle of "bar".
        f.press(36, &w8, 2, false);
        assert_eq!(f.selected(), "bar");
        f.press(36, &w8, 3, false);
        assert_eq!(f.selected(), "foo bar.baz");
        f.press(2, &w8, 2, false);
        assert_eq!(f.selected(), "foo");
        // On the gap between words: the run of spaces and signs.
        f.press(26, &w8, 2, false);
        assert_eq!(f.selected(), " ");
    }

    #[test]
    fn a_click_puts_the_caret_between_the_characters_nearest_to_it() {
        let mut f = field("hello");
        f.press(0, &w8, 1, false);
        assert_eq!(f.caret(), 0);
        f.press(11, &w8, 1, false);
        assert_eq!(f.caret(), 1, "11 is nearer 8 than 16");
        f.press(13, &w8, 1, false);
        assert_eq!(f.caret(), 2);
        f.press(500, &w8, 1, false);
        assert_eq!(f.caret(), 5, "past the end");
        f.press(-30, &w8, 1, false);
        assert_eq!(f.caret(), 0, "before the start");
    }

    #[test]
    fn dragging_selects_from_where_the_press_was_and_shift_click_extends() {
        let mut f = field("hello world");
        f.press(16, &w8, 1, false);
        assert!(f.dragging());
        assert!(f.drag(60, &w8));
        assert_eq!(f.selected(), "llo wo");
        assert!(f.drag(0, &w8));
        assert_eq!(f.selected(), "he");
        f.release();
        assert!(!f.dragging());
        assert!(!f.drag(40, &w8), "a move with the button up does nothing");
        f.press(88, &w8, 1, true);
        assert_eq!(f.selection(), Some((2, 11)), "the anchor stayed where the last press left it");
    }

    #[test]
    fn a_long_text_shows_its_end_up_to_the_caret() {
        let f = field("0123456789");
        // 80 px of text in a field 40 px wide: the caret is at 80, the text shifts by 42 to show it at 38.
        let v = f.view(40, &w8);
        assert_eq!((v.scroll, v.caret), (42, 38));
        let mut f = f;
        f.key(&key(sym::HOME));
        assert_eq!(f.view(40, &w8), View { scroll: 0, caret: 0, sel: None });
        f.key(&with(sym::RIGHT, true, false, false));
        f.key(&with(sym::RIGHT, true, false, false));
        assert_eq!(f.view(40, &w8).sel, Some((0, 16)));
        // A short text is not shifted.
        assert_eq!(field("ab").view(100, &w8).scroll, 0);
    }

    #[test]
    fn the_text_sits_where_each_look_puts_it() {
        let r = (100, 50, 300, H_BOX);
        let b = place(r, Look::Box);
        assert_eq!((b.tx, b.inner, b.cy), (113, 300 - 26, 50 + H_BOX / 2));
        let s = place((10, 10, 220, H_SEARCH), Look::Search);
        assert_eq!((s.tx, s.cy), (10 + 33, 10 + H_SEARCH / 2));
        let row = place((0, 0, 580, H_ROW), Look::Row);
        assert_eq!(row.tx, 42);
        assert!(row.px > b.px, "the palette's input is bigger");
    }

    #[test]
    fn every_look_paints_the_placeholder_the_text_the_selection_and_the_caret() {
        let t = Text::load();
        let render = |f: &Field, look: Look, focused: bool| {
            let h = match look {
                Look::Box => H_BOX,
                Look::Search | Look::Find => H_SEARCH,
                Look::Row => H_ROW,
            };
            let mut fb = vec![0u32; 320 * 60];
            let mut p = Paint { fb: &mut fb, w: 320, h: 60, ox: 0, oy: 0, fade: 1.0 };
            paint(&mut p, &t, f, (10, 5, 300, h), look, focused);
            fb
        };
        for look in [Look::Box, Look::Search, Look::Find, Look::Row] {
            let empty = Field::new(0, "Search windows");
            let (idle, focused) = (render(&empty, look, false), render(&empty, look, true));
            assert!(idle.iter().any(|&p| p >> 24 != 0), "{look:?}: the placeholder is drawn");
            let diff = idle.iter().zip(&focused).filter(|(a, b)| a != b).count();
            assert!(diff > 10, "{look:?}: focus changes the picture ({diff})");
            let typed = field("hello");
            assert_ne!(render(&typed, look, true), focused, "{look:?}: text is drawn");
            let mut sel = field("hello");
            sel.select_all();
            assert_ne!(render(&sel, look, true), render(&typed, look, true), "{look:?}: a selection is drawn");
        }
    }

    #[test]
    fn the_caret_is_solid_the_same_picture_on_every_frame() {
        let t = Text::load();
        let f = field("hello");
        let frame = || {
            let mut fb = vec![0u32; 320 * 60];
            let mut p = Paint { fb: &mut fb, w: 320, h: 60, ox: 0, oy: 0, fade: 1.0 };
            paint(&mut p, &t, &f, (10, 5, 300, H_BOX), Look::Box, true);
            fb
        };
        assert_eq!(frame(), frame(), "no blinking: nothing depends on time");
    }

    #[test]
    fn text_longer_than_the_field_stays_inside_it() {
        let t = Text::load();
        let f = field(&"long text ".repeat(20));
        let mut fb = vec![0u32; 320 * 60];
        let mut p = Paint { fb: &mut fb, w: 320, h: 60, ox: 0, oy: 0, fade: 1.0 };
        paint(&mut p, &t, &f, (10, 5, 200, H_BOX), Look::Box, true);
        // Right of the box (x = 210) nothing was drawn, and the caret is inside it.
        for y in 0..60 {
            for x in 215..320 {
                assert_eq!(fb[y * 320 + x], 0, "{x},{y}");
            }
        }
    }
}
