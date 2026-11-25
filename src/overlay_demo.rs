//! The kit's own screens: a dialog with a field (`AXON_DEMO_OVERLAY=modal`), the three looks of a field
//! (`field`), a palette-shaped picker (`list`), a context menu (`menu`) and a toast (`toast`). They exist to
//! see and try each primitive in the real window and on the design board, and they are the usage example
//! for new screens: the palette is the `Picker` with real entries (`commands::build`, ADR 0027), the group dialog
//! is a `Dialog` with more rows (`group_dialog.rs`, ADR 0029), a window's menu is a `Menu` with its commands
//! (`ctxmenu.rs`).

use std::time::Instant;

use crate::{
    field::{self, Edit, Field, H_BOX, H_ROW, H_SEARCH, Look},
    i18n::{Str, tr},
    icons::Icon,
    input::sym,
    list::{Entry, List, Mark},
    menu::{self, Menu},
    overlay::{BTN_H, Click, Clicks, Cx, Cursor, Geo, Ink, Modal, Out, Overlay, PX_HINT, PX_TITLE, Rect, Stack, button, button_width, dim, fg, inside},
    paint::{Paint, WHITE},
    picker::Picker,
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
        "list" => st.open(TAG, Box::new(picker())),
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
    #[cfg(test)]
    pub fn shown(mut self) -> Dialog {
        self.modal.fade = crate::overlay::Fade::full();
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

/// The kit's sample palette (`AXON_DEMO_OVERLAY=list`): the reference's shape with ten entries, seven actions, two
/// windows and a group. The real palette is `picker::Picker` over `commands::build`.
pub fn picker() -> Picker {
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
    Picker::new(list)
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
        let screens: [Box<dyn Overlay>; 3] = [Box::new(Dialog::new(Shape::Group).shown()), Box::new(Dialog::new(Shape::Fields).shown()), Box::new(picker().state("", 0))];
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
