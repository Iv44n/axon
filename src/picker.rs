//! The command palette's overlay (`Ctrl+K`, ADR 0027): a field in a row, a filtered list under it and a footer
//! with the keys, in the modal card of the reference's `.palette`. It does not know what its entries mean:
//! it is given a `List` (the app's: `commands::build`; the kit's demo: `overlay_demo::picker`) and answers
//! `Out::Run(id)` with the id of the entry chosen, which the app turns into a command.
//!
//! Type to filter, Up and Down move (wrapping), Enter or a click runs the entry, the wheel scrolls, Esc or
//! `Ctrl+K` again closes it, a press on the scrim closes it.
use std::time::Instant;

use crate::{
    field::{self, Edit, Field, H_ROW, Look},
    i18n::{Str, tr},
    icons::Icon,
    input::sym,
    list::{self, FOOT_H, Hint, List, Nav},
    overlay::{Cap, Click, Clicks, Cx, Cursor, Geo, Modal, Out, Overlay, Rect, inside},
    paint::Paint,
    xkb::Key,
};

/// The palette: a field in a row, a list under it, a footer with the keys.
pub struct Picker {
    modal: Modal,
    field: Field,
    list: List,
    clicks: Clicks,
    /// The row the button went down on.
    armed: Option<usize>,
}

/// The palette's measures: 580 wide, 470 tall at most, 60 from the top.
const PICKER_W: i32 = 580;
const PICKER_MAX_H: i32 = 470;
const PICKER_TOP: i32 = 60;
/// The room the message of an empty result takes: the list's padding and the message's own (28 around a line).
const EMPTY_H: i32 = 86;

impl Picker {
    /// A palette over `list`, which must already be filtered (by an empty query: everything under its headings).
    pub fn new(mut list: List) -> Picker {
        list.view = PICKER_MAX_H - 2 - H_ROW - FOOT_H;
        Picker { modal: Modal::new(PICKER_W, PICKER_MAX_H).at_top(PICKER_TOP), field: Field::new(0, tr(Str::OvSearch)), list, clicks: Clicks::default(), armed: None }
    }

    /// Already faded in, with `query` typed and row `sel` selected (the design board's frames).
    #[cfg(test)]
    pub fn state(mut self, query: &str, sel: usize) -> Picker {
        self.modal.fade = crate::overlay::Fade::full();
        self.field.set(query);
        self.list.filter(query);
        self.list.view = PICKER_MAX_H - 2 - H_ROW - FOOT_H;
        self.list.select(sel);
        self
    }

    /// The card, the input row, the list and the footer at window size `g`.
    fn geom(&self, g: Geo) -> (Rect, Rect, Rect, Rect) {
        let max_list = PICKER_MAX_H - 2 - H_ROW - FOOT_H;
        let want = if self.list.is_empty() { EMPTY_H } else { self.list.content_h() };
        let h = 2 + H_ROW + want.min(max_list) + FOOT_H;
        let (x, y, w, h) = Modal { h, ..self.modal }.rect(g);
        let input = (x + 1, y + 1, w - 2, H_ROW);
        let foot = (x + 1, y + h - 1 - FOOT_H, w - 2, FOOT_H);
        let list = (x + 1, y + 1 + H_ROW, w - 2, (h - 2 - H_ROW - FOOT_H).max(0));
        ((x, y, w, h), input, list, foot)
    }

    /// Tells the list how tall its viewport is now.
    fn sync(&mut self, g: Geo) {
        self.list.view = self.geom(g).2.3;
        let sel = self.list.selected();
        self.list.select(sel);
    }

    fn refilter(&mut self, g: Geo) {
        self.list.filter(self.field.text());
        self.sync(g);
    }
}

impl Overlay for Picker {
    fn key(&mut self, k: &Key, cx: &Cx) -> Out {
        // The key that opened the palette closes it (the reference's `Ctrl+K` toggles).
        if k.mods.ctrl && !k.mods.alt && !k.mods.shift && matches!(k.sym, 0x6b | 0x4b) {
            return Out::Close;
        }
        self.sync(cx.g);
        // The list has the arrows, the page keys and Enter; Home and End are the caret's unless Ctrl is held.
        if !matches!(k.sym, sym::HOME | sym::END | sym::KP_HOME | sym::KP_END) || k.mods.ctrl {
            match self.list.key(k) {
                Nav::Chose(e) => return Out::Run(self.list.id(e)),
                Nav::Moved => return Out::Redraw,
                Nav::Same => return Out::None,
                Nav::Ignored => {}
            }
        }
        match self.field.key(k) {
            Edit::Ignored | Edit::Same => Out::None,
            Edit::Moved => Out::Redraw,
            Edit::Changed => {
                self.refilter(cx.g);
                Out::Redraw
            }
            Edit::Paste => Out::Paste,
            Edit::Copy(t) => Out::Copy(t),
            Edit::Cut(t) => {
                self.refilter(cx.g);
                Out::Copy(t)
            }
        }
    }

    fn paste(&mut self, text: &str, cx: &Cx) -> Out {
        if self.field.insert(text) {
            self.refilter(cx.g);
            Out::Redraw
        } else {
            Out::None
        }
    }

    fn pointer(&mut self, x: f32, y: f32, cx: &Cx) -> Out {
        self.sync(cx.g);
        let (_, input, list, _) = self.geom(cx.g);
        if self.field.dragging() {
            return if self.field.drag_at(x, input, Look::Row, cx.text) { Out::Redraw } else { Out::None };
        }
        if inside(list, x, y) && self.list.hover(y as i32 - list.1) { Out::Redraw } else { Out::None }
    }

    fn button(&mut self, c: Click, cx: &Cx) -> Out {
        self.sync(cx.g);
        let (_, input, list, _) = self.geom(cx.g);
        if !c.pressed {
            self.field.release();
            let row = inside(list, c.x, c.y).then(|| self.list.row_at(c.y as i32 - list.1)).flatten();
            return match (self.armed.take(), row) {
                (Some(a), Some(b)) if a == b => self.list.index(a).map_or(Out::None, |e| Out::Run(self.list.id(e))),
                _ => Out::None,
            };
        }
        if inside(input, c.x, c.y) {
            let clicks = self.clicks.press(c.time);
            self.field.press_at(c.x, input, Look::Row, cx.text, clicks, false);
            return Out::Redraw;
        }
        if inside(list, c.x, c.y) {
            self.armed = self.list.row_at(c.y as i32 - list.1);
            if let Some(a) = self.armed {
                self.list.select(a);
                return Out::Redraw;
            }
        }
        Out::None
    }

    fn scroll(&mut self, dy: f32, _: f32, _: f32, cx: &Cx) -> Out {
        self.sync(cx.g);
        if self.list.scroll_by(dy) { Out::Redraw } else { Out::None }
    }

    fn contains(&self, x: f32, y: f32, cx: &Cx) -> bool {
        inside(self.geom(cx.g).0, x, y)
    }

    fn cursor(&self, x: f32, y: f32, cx: &Cx) -> Cursor {
        let (_, input, list, _) = self.geom(cx.g);
        if inside(input, x, y) {
            Cursor::Text
        } else if inside(list, x, y) && self.list.row_at(y as i32 - list.1).is_some() {
            Cursor::Pointer
        } else {
            Cursor::Default
        }
    }

    fn step(&mut self, now: Instant) -> bool {
        self.modal.fade.step(now)
    }

    fn draw(&self, p: &mut Paint, cx: &Cx) {
        let t = cx.text;
        // The card is painted at the size the contents want, risen a little while it fades in.
        let m = Modal { h: self.geom(cx.g).0.3, ..self.modal };
        let (_, y, ..) = m.paint(p, cx.g);
        let (card, input, list, foot) = self.geom(cx.g);
        let dy = y - card.1;
        let moved = |r: Rect| (r.0, r.1 + dy, r.2, r.3);
        field::paint(p, t, &self.field, moved(input), Look::Row, true);
        list::paint(p, t, &self.list, moved(list), tr(Str::OvNoMatches));
        let hints: [Hint; 3] = [(&[Cap::Icon(Icon::Up), Cap::Icon(Icon::Down)], tr(Str::OvKeyMove)), (&[Cap::Icon(Icon::Enter)], tr(Str::OvKeyRun)), (&[Cap::Text("Esc")], tr(Str::OvKeyClose))];
        list::paint_footer(p, t, moved(foot), &hints);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{commands, overlay::Text, overlay_demo, xkb::Mods};

    const G: Geo = Geo { w: 900, h: 620, square: false };

    /// The kit's sample palette: ten entries, the first seven actions.
    fn sample() -> Picker {
        overlay_demo::picker()
    }

    fn typed(s: &str) -> Key {
        Key { sym: s.chars().next().unwrap() as u32, text: s.into(), ..Key::default() }
    }

    fn key(sym: u32) -> Key {
        Key { sym, ..Key::default() }
    }

    #[test]
    fn typing_in_the_picker_filters_the_list_and_enter_runs_the_first_match() {
        let text = Text::load();
        let cx = Cx { g: G, text: &text };
        let mut p = sample();
        assert_eq!(p.list.len(), 10);
        for c in "gru".chars() {
            assert_eq!(p.key(&typed(&c.to_string()), &cx), Out::Redraw);
        }
        assert_eq!(p.field.text(), "gru");
        assert!(p.list.len() < 10 && p.list.len() >= 2);
        let first = p.list.id(p.list.chosen().unwrap());
        assert_eq!(p.key(&key(sym::RETURN), &cx), Out::Run(first));
        // Backspace widens it again; Down moves the selection, Enter runs another entry.
        p.key(&key(sym::BACKSPACE), &cx);
        p.key(&key(sym::BACKSPACE), &cx);
        p.key(&key(sym::BACKSPACE), &cx);
        assert_eq!(p.list.len(), 10);
        assert_eq!(p.key(&key(sym::DOWN), &cx), Out::Redraw);
        assert_eq!(p.key(&key(sym::RETURN), &cx), Out::Run(2));
        // Home and End are the caret's, with Ctrl the list's.
        p.field.set("abc");
        p.list.filter("");
        assert_eq!(p.key(&key(sym::HOME), &cx), Out::Redraw);
        assert_eq!(p.field.caret(), 0);
        assert_eq!(p.list.selected(), 0);
        let ctrl_end = Key { sym: sym::END, mods: Mods { ctrl: true, ..Mods::default() }, ..Key::default() };
        assert_eq!(p.key(&ctrl_end, &cx), Out::Redraw);
        assert_eq!(p.list.selected(), 9);
        // A key nobody wants is not taken by anything but the stack.
        assert_eq!(p.key(&key(sym::TAB), &cx), Out::None);
    }

    #[test]
    fn pasting_into_the_picker_filters_and_copy_goes_to_the_clipboard() {
        let text = Text::load();
        let cx = Cx { g: G, text: &text };
        let mut p = sample();
        let ctrl = |c: char| Key { sym: c as u32, mods: Mods { ctrl: true, ..Mods::default() }, ..Key::default() };
        assert_eq!(p.key(&ctrl('v'), &cx), Out::Paste);
        // «arcton» is data (a window, a group), not a string of the interface: it is in both languages.
        assert_eq!(p.paste("arcton", &cx), Out::Redraw);
        assert_eq!(p.list.len(), 3);
        p.key(&ctrl('a'), &cx);
        assert_eq!(p.key(&ctrl('c'), &cx), Out::Copy("arcton".into()));
        assert_eq!(p.key(&ctrl('x'), &cx), Out::Copy("arcton".into()));
        assert_eq!(p.list.len(), 10, "cutting the text shows everything again");
    }

    #[test]
    fn the_picker_is_sized_by_its_rows_up_to_470_and_hangs_60_from_the_top() {
        let few = sample().state("grupo", 0);
        let (card, input, list, foot) = few.geom(G);
        assert_eq!(card.1, 60);
        assert_eq!(card.2, 580);
        assert_eq!(input.3, H_ROW);
        assert_eq!(list.1, input.1 + H_ROW);
        assert_eq!(foot.1 + foot.3, card.1 + card.3 - 1);
        assert_eq!(list.1 + list.3, foot.1, "the list fills what is between the input and the footer");
        let many = sample().state("", 0);
        let big = many.geom(G).0;
        assert!(big.3 > card.3 && big.3 <= 470, "{} vs {}", big.3, card.3);
        // The card is the input row, the rows of the list and the footer, inside a border.
        assert_eq!(card.3, 2 + H_ROW + few.list.content_h() + FOOT_H);
        // No match: room for the message.
        let none = sample().state("zzzz", 0);
        assert_eq!(none.geom(G).2.3, EMPTY_H);
        // A short window shrinks the list, not the input or the footer.
        let short = Geo { h: 300, ..G };
        let (c, i, l, f) = many.geom(short);
        assert!(c.1 + c.3 <= 300 - 16);
        assert_eq!((i.3, f.3), (H_ROW, FOOT_H));
        assert!(l.3 > 0);
    }

    #[test]
    fn hovering_a_row_selects_it_and_a_click_on_it_runs_it_on_release() {
        let text = Text::load();
        let cx = Cx { g: G, text: &text };
        let mut p = sample().state("", 0);
        let (_, _, list, _) = p.geom(G);
        // The first entry of the second row after the heading: y = list top + 6 + 28 + 40 + 5.
        let (x, y) = (400.0, (list.1 + 6 + 28 + 40 + 5) as f32);
        assert_eq!(p.pointer(x, y, &cx), Out::Redraw);
        assert_eq!(p.list.selected(), 1);
        let click = |pressed| Click { x, y, pressed, time: 0 };
        assert_eq!(p.button(click(true), &cx), Out::Redraw);
        assert_eq!(p.button(click(false), &cx), Out::Run(2));
        // Pressed on a row, released on another: nothing.
        p.button(click(true), &cx);
        let up = Click { y: y + 45.0, pressed: false, ..click(false) };
        assert_eq!(p.button(up, &cx), Out::None);
        assert_eq!(p.cursor(x, y, &cx), Cursor::Pointer);
        assert_eq!(p.cursor(x, (list.1 - 20) as f32, &cx), Cursor::Text);
        assert_eq!(p.cursor(x, 5.0, &cx), Cursor::Default);
    }

    #[test]
    fn a_click_on_a_row_of_a_picker_nobody_typed_in_does_not_scroll_it() {
        let text = Text::load();
        let cx = Cx { g: G, text: &text };
        let mut p = sample();
        let (_, _, list, _) = p.geom(G);
        let (x, y) = (400.0, (list.1 + 6 + 28 + 40 + 5) as f32);
        let press = Click { x, y, pressed: true, time: 0 };
        p.button(press, &cx);
        assert_eq!((p.list.selected(), p.list.scroll()), (1, 0));
    }

    #[test]
    fn the_wheel_scrolls_the_picker_and_a_click_in_the_input_places_the_caret() {
        let text = Text::load();
        let cx = Cx { g: G, text: &text };
        let mut p = sample().state("", 0);
        assert_eq!(p.scroll(48.0, 400.0, 300.0, &cx), Out::Redraw, "ten rows with headings do not fit 383 px");
        assert!(p.list.scroll() > 0);
        let mut p = sample().state("arcton", 0);
        let (_, input, _, _) = p.geom(G);
        let click = |x: f32, time| Click { x, y: (input.1 + 20) as f32, pressed: true, time };
        p.button(click(input.0 as f32 + 42.0 + 1.0, 100), &cx);
        assert_eq!(p.field.caret(), 0, "at the start of the text");
        p.button(Click { pressed: false, ..click(0.0, 120) }, &cx);
        p.button(click(input.0 as f32 + 42.0 + 1.0, 200), &cx);
        assert_eq!(p.field.selected(), "arcton", "a double click selects the word");
    }

    /// The real palette: the commands' own entries for a window of three in a group and one apart, the loose one on screen.
    fn real() -> Picker {
        let (wins, groups) = commands::sample();
        Picker::new(commands::build(&commands::State { wins: &wins, groups: &groups, cur: 3, panes: 1, overview: false }))
    }

    fn ctrl(c: char) -> Key {
        Key { sym: c as u32, mods: Mods { ctrl: true, ..Mods::default() }, ..Key::default() }
    }

    #[test]
    fn ctrl_k_closes_the_palette_and_no_other_chord_does() {
        let text = Text::load();
        let cx = Cx { g: G, text: &text };
        let mut p = real();
        assert_eq!(p.key(&ctrl('k'), &cx), Out::Close);
        assert_eq!(p.key(&ctrl('K'), &cx), Out::Close, "with Caps Lock or Shift held by the layout");
        let chord = |sym: char, mods: Mods| Key { sym: sym as u32, mods, ..Key::default() };
        assert_ne!(p.key(&chord('k', Mods { ctrl: true, shift: true, alt: false }), &cx), Out::Close);
        assert_ne!(p.key(&chord('k', Mods { ctrl: true, alt: true, shift: false }), &cx), Out::Close);
        // A plain k is a letter of the query.
        assert_eq!(p.key(&typed("k"), &cx), Out::Redraw);
        assert_eq!(p.field.text(), "k");
    }

    #[test]
    fn enter_runs_the_command_of_the_selected_row_and_typing_narrows_to_it() {
        let text = Text::load();
        let cx = Cx { g: G, text: &text };
        let mut p = real();
        let run = |p: &mut Picker| match p.key(&key(sym::RETURN), &cx) {
            Out::Run(id) => commands::Cmd::from_id(id),
            other => panic!("{other:?}"),
        };
        assert_eq!(run(&mut p), Some(commands::Cmd::NewWindow), "the first row is the first action");
        // Down twice: the third row. Up from the first wraps to the last (the group's fold command).
        p.key(&key(sym::DOWN), &cx);
        p.key(&key(sym::DOWN), &cx);
        assert_eq!(run(&mut p), Some(commands::Cmd::NewPane));
        let mut p = real();
        assert_eq!(p.key(&key(sym::UP), &cx), Out::Redraw);
        assert_eq!(run(&mut p), Some(commands::Cmd::Fold(0)));
        // Typing a window's name leaves it first.
        let mut p = real();
        for c in "servidor".chars() {
            p.key(&typed(&c.to_string()), &cx);
        }
        assert_eq!(run(&mut p), Some(commands::Cmd::Window(1)));
        // Nothing matches: Enter has nothing to run, and nothing happens.
        let mut p = real();
        for c in "zzzz".chars() {
            p.key(&typed(&c.to_string()), &cx);
        }
        assert!(p.list.is_empty());
        assert_eq!(p.key(&key(sym::RETURN), &cx), Out::None);
    }

    #[test]
    fn the_list_scrolls_to_the_selection_at_the_end_and_back_to_the_top_when_it_wraps() {
        let text = Text::load();
        let cx = Cx { g: G, text: &text };
        let mut p = real();
        assert_eq!(p.list.scroll(), 0);
        assert_eq!(p.key(&Key { sym: sym::END, ..ctrl('x') }, &cx), Out::Redraw, "Ctrl+End is the list's");
        assert_eq!(p.list.selected(), p.list.len() - 1);
        assert!(p.list.scroll() > 0, "more rows than fit: the last one is brought into view");
        // Down from the last wraps to the first and the list goes back to the top.
        assert_eq!(p.key(&key(sym::DOWN), &cx), Out::Redraw);
        assert_eq!((p.list.selected(), p.list.scroll()), (0, 0));
    }

    #[test]
    fn a_click_on_a_row_of_the_real_palette_runs_it_on_release() {
        let text = Text::load();
        let cx = Cx { g: G, text: &text };
        let mut p = real();
        let (_, _, list, _) = p.geom(G);
        // The first row after the "Actions" heading.
        let (x, y) = (300.0, (list.1 + 6 + 28 + 20) as f32);
        let click = |pressed| Click { x, y, pressed, time: 0 };
        assert_eq!(p.button(click(true), &cx), Out::Redraw);
        assert_eq!(p.button(click(false), &cx), Out::Run(commands::Cmd::NewWindow.id()));
    }
}
