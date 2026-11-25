//! The kit's own screens: a dialog with a field (`AXON_DEMO_OVERLAY=modal`), the three looks of a field
//! (`field`), a palette-shaped picker (`list`), a context menu (`menu`) and a toast (`toast`). They exist to
//! see and try each primitive in the real window and on the design board, and they are the usage example
//! for the next screens: the palette is a `Picker` with real entries, the group dialog is a `Dialog` with
//! more rows, a window's menu is a `Menu` with its commands (ADR 0025).
// The kit's API is wider than what the app uses yet: the palette, the overview and the dialogs are built on it next (ADR 0025).
#![allow(dead_code)]

use std::time::Instant;

use crate::{
    field::{self, Edit, Field, H_BOX, H_ROW, H_SEARCH, Look},
    i18n::{Str, tr},
    icons::Icon,
    input::sym,
    list::{self, Entry, FOOT_H, Hint, List, Mark, Nav},
    menu::{self, Menu},
    overlay::{BTN_H, Cap, Click, Clicks, Cx, Cursor, Fade, Geo, Ink, Modal, Out, Overlay, PX_HINT, PX_TITLE, Rect, Stack, button, button_width, dim, fg, inside},
    paint::{Paint, WHITE},
    xkb::Key,
};

/// The tag the demo overlays are opened under, and the commands they run.
pub const TAG: u32 = 0x4f56_4552;
pub const CMD_CREATE: u32 = 100;
pub const CMD_UNDO: u32 = 101;

/// The names `AXON_DEMO_OVERLAY` takes.
pub const KINDS: [&str; 5] = ["field", "list", "menu", "toast", "modal"];

/// Opens the demo named `kind` on the stack. False if there is no such demo.
pub fn open(kind: &str, st: &mut Stack, g: Geo, now: Instant) -> bool {
    match kind {
        "field" => st.open(TAG, Box::new(Dialog::new(Shape::Fields))),
        "modal" => st.open(TAG, Box::new(Dialog::new(Shape::Group))),
        "list" => st.open(TAG, Box::new(Picker::new())),
        "menu" => st.open(TAG, Box::new(window_menu(g.w / 3, g.h / 4))),
        "toast" => st.toast(tr(Str::OvToastClosed), Some(tr(Str::OvUndo)), CMD_UNDO, now),
        _ => return false,
    }
    true
}

/// The reference's menu of a window: items with shortcuts, a heading, the groups with their colours and a
/// check, and two destructive ones, the last disabled.
pub fn window_menu(x: i32, y: i32) -> Menu {
    Menu::at(x, y)
        .item(tr(Str::OvMenuRename), "F2", 1, 0)
        .item(tr(Str::OvMenuDuplicate), "", 2, 0)
        .item(tr(Str::OvActSplit), "Alt+\\", 3, 0)
        .sep()
        .head(tr(Str::OvMenuGroup))
        .dot("arcton", 1, 10, menu::CHECKED)
        .dot("docs", 3, 11, 0)
        .item(tr(Str::OvMenuNewGroupWith), "Alt+G", 12, 0)
        .sep()
        .item(tr(Str::OvActClose), "Alt+W", 20, menu::DANGER)
        .item(tr(Str::OvMenuCloseOthers), "", 21, menu::DANGER | menu::DISABLED)
}

// ------------------------------------------------------------------ dialog --

/// What a `Dialog` shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// A card with the three looks of a field, one under the other.
    Fields,
    /// The reference's group dialog: a title and a hint, one field, a footer with two buttons.
    Group,
}

/// A modal card with fields and, in the group dialog, buttons: Tab moves between fields, Enter creates.
pub struct Dialog {
    modal: Modal,
    shape: Shape,
    fields: Vec<Field>,
    focus: usize,
    clicks: Clicks,
    /// The button under the pointer and the one pressed (0 cancel, 1 create).
    over: Option<usize>,
    armed: Option<usize>,
}

const GROUP_H: i32 = 196;
const FIELDS_H: i32 = 257;

impl Dialog {
    pub fn new(shape: Shape) -> Dialog {
        let (modal, fields) = match shape {
            Shape::Fields => (Modal::new(420, FIELDS_H), vec![Field::new(24, tr(Str::OvFieldNameHint)), Field::new(40, tr(Str::OvFieldSearch)), Field::new(80, tr(Str::OvSearch))]),
            Shape::Group => (Modal::new(470, GROUP_H), vec![Field::new(24, tr(Str::OvFieldNameHint))]),
        };
        Dialog { modal, shape, fields, focus: 0, clicks: Clicks::default(), over: None, armed: None }
    }

    /// Already faded in (the design board's frames).
    pub fn shown(mut self) -> Dialog {
        self.modal.fade = Fade::full();
        self
    }

    /// Field `i` (to give it a text or a selection) and the focus (the design board's frames).
    #[cfg(test)]
    pub fn field_mut(&mut self, i: usize) -> &mut Field {
        &mut self.fields[i]
    }

    #[cfg(test)]
    pub fn focus(mut self, i: usize) -> Dialog {
        self.focus = i.min(self.fields.len() - 1);
        self
    }

    /// The footer button under the pointer, 0 cancel or 1 create (the design board's frames).
    #[cfg(test)]
    pub fn lit(mut self, b: usize) -> Dialog {
        self.over = Some(b);
        self
    }

    /// The fields' boxes in the card at `card`, with their look and label.
    fn field_rects(&self, (x, y, w, _): Rect) -> [(Rect, Look, Option<&'static str>); 3] {
        match self.shape {
            Shape::Group => {
                let r = (x + 21, y + 80, w - 42, H_BOX);
                [(r, Look::Box, Some(tr(Str::OvFieldName))), ((0, 0, 0, 0), Look::Box, None), ((0, 0, 0, 0), Look::Box, None)]
            }
            Shape::Fields => {
                let (fx, fw) = (x + 20, w - 40);
                let a = y + 20 + 22;
                let b = a + H_BOX + 18 + 22;
                let c = b + H_SEARCH + 18 + 22;
                [((fx, a, fw, H_BOX), Look::Box, Some("Box")), ((fx, b, fw, H_SEARCH), Look::Search, Some("Search")), ((fx, c, fw, H_ROW), Look::Row, Some("Row"))]
            }
        }
    }

    fn count(&self) -> usize {
        self.fields.len()
    }

    /// The footer buttons: cancel, then the primary one at the right edge.
    fn buttons(&self, (x, y, w, h): Rect, cx: &Cx) -> [Rect; 2] {
        let (cancel, create) = (tr(Str::Cancel), tr(Str::OvCreateGroup));
        let (cw, pw) = (button_width(cx.text, cancel), button_width(cx.text, create));
        let by = y + h - 1 - 14 - BTN_H;
        let right = x + w - 1 - 20;
        [(right - pw - 8 - cw, by, cw, BTN_H), (right - pw, by, pw, BTN_H)]
    }

    fn button_at(&self, card: Rect, cx: &Cx, x: f32, y: f32) -> Option<usize> {
        if self.shape != Shape::Group {
            return None;
        }
        self.buttons(card, cx).iter().position(|&b| inside(b, x, y))
    }
}

impl Overlay for Dialog {
    fn key(&mut self, k: &Key, _: &Cx) -> Out {
        match k.sym {
            sym::TAB => {
                let n = self.count();
                self.focus = (self.focus + if k.mods.shift { n - 1 } else { 1 }) % n;
                return Out::Redraw;
            }
            sym::RETURN | sym::KP_ENTER if self.shape == Shape::Group => return Out::Run(CMD_CREATE),
            _ => {}
        }
        match self.fields[self.focus].key(k) {
            Edit::Ignored | Edit::Same => Out::None,
            Edit::Moved | Edit::Changed => Out::Redraw,
            Edit::Paste => Out::Paste,
            Edit::Copy(t) | Edit::Cut(t) => Out::Copy(t),
        }
    }

    fn paste(&mut self, text: &str, _: &Cx) -> Out {
        if self.fields[self.focus].insert(text) { Out::Redraw } else { Out::None }
    }

    fn pointer(&mut self, x: f32, y: f32, cx: &Cx) -> Out {
        let card = self.modal.rect(cx.g);
        let over = self.button_at(card, cx, x, y);
        let mut out = if std::mem::replace(&mut self.over, over) != over { Out::Redraw } else { Out::None };
        if self.fields[self.focus].dragging() {
            let (r, look, _) = self.field_rects(card)[self.focus];
            if self.fields[self.focus].drag_at(x, r, look, cx.text) {
                out = Out::Redraw;
            }
        }
        out
    }

    fn button(&mut self, c: Click, cx: &Cx) -> Out {
        let card = self.modal.rect(cx.g);
        if !c.pressed {
            self.fields[self.focus].release();
            let (armed, on) = (self.armed.take(), self.button_at(card, cx, c.x, c.y));
            return match (armed, on) {
                (Some(0), Some(0)) => Out::Close,
                (Some(1), Some(1)) => Out::Run(CMD_CREATE),
                _ => Out::None,
            };
        }
        if let Some(b) = self.button_at(card, cx, c.x, c.y) {
            self.armed = Some(b);
            return Out::Redraw;
        }
        let rects = self.field_rects(card);
        for (i, &(r, look, _)) in rects.iter().enumerate().take(self.count()) {
            if inside(r, c.x, c.y) {
                self.focus = i;
                let clicks = self.clicks.press(c.time);
                self.fields[i].press_at(c.x, r, look, cx.text, clicks, false);
                return Out::Redraw;
            }
        }
        Out::None
    }

    fn contains(&self, x: f32, y: f32, cx: &Cx) -> bool {
        self.modal.contains(cx.g, x, y)
    }

    fn cursor(&self, x: f32, y: f32, cx: &Cx) -> Cursor {
        let card = self.modal.rect(cx.g);
        if self.button_at(card, cx, x, y).is_some() {
            return Cursor::Pointer;
        }
        let over_field = (0..self.count()).any(|i| inside(self.field_rects(card)[i].0, x, y));
        if over_field { Cursor::Text } else { Cursor::Default }
    }

    fn step(&mut self, now: Instant) -> bool {
        self.modal.fade.step(now)
    }

    fn draw(&self, p: &mut Paint, cx: &Cx) {
        let t = cx.text;
        let (x, y, w, h) = self.modal.paint(p, cx.g);
        let card = (x, y, w, h);
        let rects = self.field_rects(card);
        if self.shape == Shape::Group {
            let cy = y + 1 + 18 + 10;
            t.draw(p, x + 20, cy, tr(Str::OvDialogTitle), Ink::new(PX_TITLE, fg()).bold());
            let hint = tr(Str::OvDialogHint);
            t.draw(p, x + w - 20 - t.width(hint, PX_HINT), cy + 1, hint, Ink::new(PX_HINT, dim()));
        }
        for (i, &(r, look, label)) in rects.iter().enumerate().take(self.count()) {
            if let Some(label) = label {
                t.draw(p, r.0, r.1 - 7 - 8, label, Ink::new(PX_HINT, dim()));
            }
            field::paint(p, t, &self.fields[i], r, look, i == self.focus);
        }
        if self.shape == Shape::Group {
            p.shape(x + 1, y + h - 1 - 63, x + w - 1, y + h - 1 - 62, WHITE, 0.07, &|_, _| 1.0);
            let [cancel, create] = self.buttons(card, cx);
            button(p, t, cancel, tr(Str::Cancel), false, self.over == Some(0));
            button(p, t, create, tr(Str::OvCreateGroup), true, self.over == Some(1));
        }
    }
}

// ------------------------------------------------------------------ picker --

/// The palette's shape: a field in a row, a list under it, a footer with the keys. Type to filter, Up and
/// Down move, Enter or a click runs the entry, the wheel scrolls.
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
/// The room the message of an empty result takes.
const EMPTY_H: i32 = 74;

impl Picker {
    pub fn new() -> Picker {
        let mut list = List::default();
        let (actions, windows, groups) = (tr(Str::OvGroupActions), tr(Str::Windows), tr(Str::OvGroupGroups));
        let act = |label: &'static str, keys: &'static str, icon: Icon, id: u32| Entry { label, keys, group: actions, mark: Mark::Icon(icon), id, ..Entry::default() };
        list.push(act(tr(Str::OvActNewWindow), "Alt+N", Icon::Plus, 1));
        list.push(act(tr(Str::OvActNewGroup), "Alt+G", Icon::Group, 2));
        list.push(act(tr(Str::OvActSplit), "Alt+\\", Icon::Split, 3));
        list.push(act(tr(Str::OvActOverview), "Alt+O", Icon::Grid, 4));
        list.push(act(tr(Str::OvActPanel), "Alt+B", Icon::Panel, 5));
        list.push(act(tr(Str::SettingsTitle), "Ctrl+,", Icon::Gear, 6));
        list.push(act(tr(Str::OvActClose), "Alt+W", Icon::Cross, 7));
        list.push(Entry { label: "arcton", desc: "~/lab/apps/arcton", group: windows, mark: Mark::Icon(Icon::Prompt), keys: "Alt+1", id: 8, ..Entry::default() });
        list.push(Entry { label: "servidor", desc: "~/servidor · arcton", keywords: "arcton", group: windows, mark: Mark::Icon(Icon::Prompt), keys: "Alt+2", id: 9 });
        list.push(Entry { label: "arcton", desc: tr(Str::OvThreeWindows), keywords: "grupo group", group: groups, mark: Mark::Dot(1), id: 10, ..Entry::default() });
        list.filter("");
        list.view = PICKER_MAX_H - 2 - H_ROW - FOOT_H;
        Picker { modal: Modal::new(PICKER_W, PICKER_MAX_H).at_top(PICKER_TOP), field: Field::new(0, tr(Str::OvSearch)), list, clicks: Clicks::default(), armed: None }
    }

    /// Already faded in, with `query` typed and row `sel` selected (the design board's frames).
    pub fn state(mut self, query: &str, sel: usize) -> Picker {
        self.modal.fade = Fade::full();
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
    use crate::{overlay::Text, xkb::Mods};

    const G: Geo = Geo { w: 900, h: 620, square: false };

    fn typed(s: &str) -> Key {
        Key { sym: s.chars().next().unwrap() as u32, text: s.into(), ..Key::default() }
    }

    fn key(sym: u32) -> Key {
        Key { sym, ..Key::default() }
    }

    #[test]
    fn every_demo_kind_opens_and_an_unknown_one_does_not() {
        for kind in KINDS {
            let mut st = Stack::default();
            assert!(open(kind, &mut st, G, Instant::now()), "{kind}");
            assert!(!st.is_empty());
            assert_eq!(st.is_open(), kind != "toast", "a toast does not take the keyboard");
        }
        let mut st = Stack::default();
        assert!(!open("nope", &mut st, G, Instant::now()));
        assert!(st.is_empty());
    }

    #[test]
    fn typing_in_the_picker_filters_the_list_and_enter_runs_the_first_match() {
        let text = Text::load();
        let cx = Cx { g: G, text: &text };
        let mut p = Picker::new();
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
        let mut p = Picker::new();
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
        let few = Picker::new().state("grupo", 0);
        let (card, input, list, foot) = few.geom(G);
        assert_eq!(card.1, 60);
        assert_eq!(card.2, 580);
        assert_eq!(input.3, H_ROW);
        assert_eq!(list.1, input.1 + H_ROW);
        assert_eq!(foot.1 + foot.3, card.1 + card.3 - 1);
        assert_eq!(list.1 + list.3, foot.1, "the list fills what is between the input and the footer");
        let many = Picker::new().state("", 0);
        let big = many.geom(G).0;
        assert!(big.3 > card.3 && big.3 <= 470, "{} vs {}", big.3, card.3);
        // The card is the input row, the rows of the list and the footer, inside a border.
        assert_eq!(card.3, 2 + H_ROW + few.list.content_h() + FOOT_H);
        // No match: room for the message.
        let none = Picker::new().state("zzzz", 0);
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
        let mut p = Picker::new().state("", 0);
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
        let mut p = Picker::new();
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
        let mut p = Picker::new().state("", 0);
        assert_eq!(p.scroll(48.0, 400.0, 300.0, &cx), Out::Redraw, "ten rows with headings do not fit 383 px");
        assert!(p.list.scroll() > 0);
        let mut p = Picker::new().state("arcton", 0);
        let (_, input, _, _) = p.geom(G);
        let click = |x: f32, time| Click { x, y: (input.1 + 20) as f32, pressed: true, time };
        p.button(click(input.0 as f32 + 42.0 + 1.0, 100), &cx);
        assert_eq!(p.field.caret(), 0, "at the start of the text");
        p.button(Click { pressed: false, ..click(0.0, 120) }, &cx);
        p.button(click(input.0 as f32 + 42.0 + 1.0, 200), &cx);
        assert_eq!(p.field.selected(), "arcton", "a double click selects the word");
    }

    #[test]
    fn the_dialog_moves_focus_with_tab_types_in_the_focused_field_and_enter_creates() {
        let text = Text::load();
        let cx = Cx { g: G, text: &text };
        let mut d = Dialog::new(Shape::Fields);
        assert_eq!(d.key(&typed("a"), &cx), Out::Redraw);
        assert_eq!(d.key(&key(sym::TAB), &cx), Out::Redraw);
        assert_eq!(d.key(&typed("b"), &cx), Out::Redraw);
        assert_eq!((d.fields[0].text(), d.fields[1].text()), ("a", "b"));
        d.key(&Key { sym: sym::TAB, mods: Mods { shift: true, ..Mods::default() }, ..Key::default() }, &cx);
        assert_eq!(d.focus, 0);
        d.key(&Key { sym: sym::TAB, mods: Mods { shift: true, ..Mods::default() }, ..Key::default() }, &cx);
        assert_eq!(d.focus, 2, "backwards wraps");
        assert_eq!(d.key(&key(sym::RETURN), &cx), Out::None, "the field gallery has nothing to create");
        let mut g = Dialog::new(Shape::Group);
        g.key(&typed("x"), &cx);
        assert_eq!(g.key(&key(sym::RETURN), &cx), Out::Run(CMD_CREATE));
        assert_eq!(g.key(&key(sym::TAB), &cx), Out::Redraw, "one field: Tab stays on it");
    }

    #[test]
    fn the_dialogs_buttons_are_found_where_they_are_drawn_and_run_on_release() {
        let text = Text::load();
        let cx = Cx { g: G, text: &text };
        let mut d = Dialog::new(Shape::Group);
        let card = d.modal.rect(G);
        let [cancel, create] = d.buttons(card, &cx);
        assert!(create.0 > cancel.0 + cancel.2, "cancel first, then the primary one");
        assert_eq!(create.0 + create.2, card.0 + card.2 - 1 - 20);
        assert_eq!(create.1 + create.3, card.1 + card.3 - 1 - 14);
        let at = |r: Rect| ((r.0 + r.2 / 2) as f32, (r.1 + r.3 / 2) as f32);
        let click = |(x, y): (f32, f32), pressed| Click { x, y, pressed, time: 0 };
        d.pointer(at(create).0, at(create).1, &cx);
        assert_eq!(d.over, Some(1));
        assert_eq!(d.cursor(at(create).0, at(create).1, &cx), Cursor::Pointer);
        assert_eq!(d.button(click(at(create), true), &cx), Out::Redraw);
        assert_eq!(d.button(click(at(create), false), &cx), Out::Run(CMD_CREATE));
        assert_eq!(d.button(click(at(cancel), true), &cx), Out::Redraw);
        assert_eq!(d.button(click(at(cancel), false), &cx), Out::Close);
        // Pressed on one, released on the other: nothing.
        d.button(click(at(cancel), true), &cx);
        assert_eq!(d.button(click(at(create), false), &cx), Out::None);
        // The field takes the click, with the caret where it was clicked.
        let (r, ..) = d.field_rects(card)[0];
        d.fields[0].set("hello");
        d.button(click(((r.0 + 13) as f32, (r.1 + 18) as f32), true), &cx);
        assert_eq!(d.fields[0].caret(), 0);
        assert_eq!(d.cursor((r.0 + 20) as f32, (r.1 + 18) as f32, &cx), Cursor::Text);
        // Dragging along the text selects it; letting go ends the drag.
        assert_eq!(d.pointer((r.0 + 13 + 200) as f32, (r.1 + 18) as f32, &cx), Out::Redraw);
        assert_eq!(d.fields[0].selected(), "hello");
        d.button(click((0.0, 0.0), false), &cx);
        assert!(!d.fields[0].dragging());
    }

    #[test]
    fn the_demo_screens_paint_over_a_frame_and_fade_in() {
        let text = Text::load();
        let cx = Cx { g: G, text: &text };
        let render = |o: &dyn Overlay| {
            let mut fb = vec![0u32; 900 * 620];
            let mut p = Paint { fb: &mut fb, w: 900, h: 620, ox: 0, oy: 0, fade: 1.0 };
            o.draw(&mut p, &cx);
            fb
        };
        let screens: [Box<dyn Overlay>; 3] = [Box::new(Dialog::new(Shape::Group).shown()), Box::new(Dialog::new(Shape::Fields).shown()), Box::new(Picker::new().state("", 0))];
        for s in &screens {
            let fb = render(s.as_ref());
            assert!(fb.iter().filter(|&&px| px >> 24 != 0).count() > 100_000, "a scrim and a card");
            assert_eq!(fb[10 * 900 + 450], 0, "the title bar is left alone");
        }
        // Not yet shown: invisible on the first frame.
        let fresh = render(&Dialog::new(Shape::Group));
        assert!(fresh.iter().all(|&px| px == 0));
    }
}
