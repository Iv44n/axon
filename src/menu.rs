//! The context menu, after the reference's `.menu` (`openMenu`): items with a label and a shortcut, a
//! checkmark or a colour dot, a danger colour, a disabled look, separators and headings; opened at the
//! pointer and kept inside the window; Up and Down move (skipping what cannot be chosen), Enter or a click
//! chooses, Esc or a press outside closes. It is an overlay (`overlay::Overlay`): the app opens it on the
//! stack and learns the item's command id from `Reply::ran`.
//!
//! The geometry is pure (`size`, `place`, `item_at`) and tested with a fake font; the strings live in one
//! pool with spans and the items are one flat array of small structs.
// The kit's API is wider than what the app uses yet: the palette, the overview and the dialogs are built on it next (ADR 0025).
#![allow(dead_code)]

use std::time::Instant;

use crate::{
    icons::{self, Icon},
    input::sym,
    overlay::{Cap, Click, Cx, Cursor, Fade, Geo, Ink, Out, Overlay, POP, PX_HEAD, PX_MENU, Rect, SHADOW_POP, Text, accent, card, faint, fg, inside, kbd, kbd_width},
    paint::{Paint, WHITE},
    side::group_color,
    xkb::Key,
};

/// Flags of an item: the red of a destructive action, an item that cannot be chosen, a mark before it.
pub const DANGER: u8 = 1;
pub const DISABLED: u8 = 2;
pub const CHECKED: u8 = 4;

/// The reference's measures, in pixels: the menu's padding and least width, a row, a separator, a heading;
/// the row's own padding, the column of the checkmark, the gap between parts, the colour dot.
const PAD: i32 = 5;
const MIN_W: i32 = 230;
const ITEM_H: i32 = 29;
const SEP_H: i32 = 11;
const HEAD_H: i32 = 23;
const ROW_PAD: i32 = 9;
const MARK_W: i32 = 14;
const GAP: i32 = 9;
const DOT: i32 = 8;
/// The room kept to the window's edges.
const EDGE: i32 = 8;
const RADIUS: f32 = 12.0;
/// The danger colour (`#ff8d8d`).
const RED: [f32; 3] = [1.0, 0.553, 0.553];
const RISE: f32 = 6.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Item,
    Sep,
    Head,
}

#[derive(Clone, Copy, Debug)]
struct Item {
    kind: Kind,
    /// Ranges of the text pool.
    label: (u32, u32),
    keys: (u32, u32),
    flags: u8,
    /// A group's colour, 0 to 7, or `NO_DOT`.
    dot: u8,
    id: u32,
}

const NO_DOT: u8 = 255;

pub struct Menu {
    pool: String,
    items: Vec<Item>,
    /// Where it was asked for (the pointer), before it is moved to fit the window.
    at: (i32, i32),
    /// The item the pointer or the keyboard is on, and the one the button went down on.
    sel: Option<usize>,
    armed: Option<usize>,
    fade: Fade,
}

impl Menu {
    /// An empty menu to open at window pixel `(x, y)`.
    pub fn at(x: i32, y: i32) -> Menu {
        Menu { pool: String::new(), items: Vec::new(), at: (x, y), sel: None, armed: None, fade: Fade::rising(0.12) }
    }

    fn span(&mut self, s: &str) -> (u32, u32) {
        let start = self.pool.len() as u32;
        self.pool.push_str(s);
        (start, s.len() as u32)
    }

    fn text(&self, (start, len): (u32, u32)) -> &str {
        &self.pool[start as usize..(start + len) as usize]
    }

    fn push(mut self, kind: Kind, label: &str, keys: &str, flags: u8, dot: u8, id: u32) -> Menu {
        let (label, keys) = (self.span(label), self.span(keys));
        self.items.push(Item { kind, label, keys, flags, dot, id });
        self
    }

    /// An item: `label`, its `shortcut` as one key cap (may be empty), the command `id` it runs, and flags.
    pub fn item(self, label: &str, shortcut: &str, id: u32, flags: u8) -> Menu {
        self.push(Kind::Item, label, shortcut, flags, NO_DOT, id)
    }

    /// An item with a group's colour dot before its label (`color` 0 to 7: `side::group_color`).
    pub fn dot(self, label: &str, color: u8, id: u32, flags: u8) -> Menu {
        self.push(Kind::Item, label, "", flags, color % 8, id)
    }

    /// A line between groups of items.
    pub fn sep(self) -> Menu {
        self.push(Kind::Sep, "", "", 0, NO_DOT, 0)
    }

    /// A small heading over the items that follow.
    pub fn head(self, label: &str) -> Menu {
        self.push(Kind::Head, label, "", 0, NO_DOT, 0)
    }

    /// Selects the first item that can be chosen, as when the menu is opened from the keyboard.
    pub fn focus_first(mut self) -> Menu {
        let first = self.enabled().next();
        self.sel = first;
        self
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn selected(&self) -> Option<usize> {
        self.sel
    }

    /// Already faded in, with item `i` lit (the design board's frames).
    #[cfg(test)]
    pub fn shown(mut self, i: Option<usize>) -> Menu {
        self.fade = Fade::full();
        self.sel = i;
        self
    }

    /// The items that can be chosen: not a separator, not a heading, not disabled.
    fn enabled(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.items.len()).filter(|&i| self.can(i))
    }

    fn can(&self, i: usize) -> bool {
        let it = &self.items[i];
        it.kind == Kind::Item && it.flags & DISABLED == 0
    }

    // ---------------------------------------------------------------- geometry --

    fn row_h(it: &Item) -> i32 {
        match it.kind {
            Kind::Item => ITEM_H,
            Kind::Sep => SEP_H,
            Kind::Head => HEAD_H,
        }
    }

    /// The least width that shows an item whole: its padding, the mark's column, the dot, the label and the cap.
    fn item_w(&self, it: &Item, width: &dyn Fn(&str, f32) -> i32) -> i32 {
        let label = width(self.text(it.label), if it.kind == Kind::Head { PX_HEAD } else { PX_MENU });
        let dot = if it.dot == NO_DOT { 0 } else { DOT + GAP };
        let keys = if it.keys.1 == 0 { 0 } else { GAP + 2 + 2 * 5 + width(self.text(it.keys), 10.5) };
        match it.kind {
            Kind::Item => ROW_PAD + MARK_W + GAP + dot + label + keys + ROW_PAD,
            Kind::Head => 2 * ROW_PAD + label,
            Kind::Sep => 0,
        }
    }

    /// The menu's size: `MIN_W` wide or what its widest item needs, tall as its rows, both with the padding
    /// and the 1 px border.
    pub fn size(&self, width: &dyn Fn(&str, f32) -> i32) -> (i32, i32) {
        let w = self.items.iter().map(|it| self.item_w(it, width)).max().unwrap_or(0) + 2 * PAD + 2;
        (w.max(MIN_W), self.items.iter().map(Menu::row_h).sum::<i32>() + 2 * PAD + 2)
    }

    /// Where a `w` x `h` menu asked for at `(x, y)` goes in the window: kept `EDGE` px inside it (the reference's
    /// `max(8, min(x, width - w - 8))`).
    pub fn place((x, y): (i32, i32), (w, h): (i32, i32), g: Geo) -> (i32, i32) {
        (EDGE.max(x.min(g.w - w - EDGE)), EDGE.max(y.min(g.h - h - EDGE)))
    }

    pub fn rect(&self, width: &dyn Fn(&str, f32) -> i32, g: Geo) -> Rect {
        let (w, h) = self.size(width);
        let (x, y) = Menu::place(self.at, (w, h), g);
        (x, y, w, h)
    }

    /// The top of item `i` in a menu at `rect`, and its height.
    fn row(&self, rect: Rect, i: usize) -> (i32, i32) {
        let top = rect.1 + 1 + PAD + self.items[..i].iter().map(Menu::row_h).sum::<i32>();
        (top, Menu::row_h(&self.items[i]))
    }

    /// The item that can be chosen at window pixel `(x, y)` in a menu at `rect`.
    pub fn item_at(&self, rect: Rect, x: f32, y: f32) -> Option<usize> {
        if x < (rect.0 + 1 + PAD) as f32 || x >= (rect.0 + rect.2 - 1 - PAD) as f32 {
            return None;
        }
        self.enabled().find(|&i| {
            let (top, h) = self.row(rect, i);
            y >= top as f32 && y < (top + h) as f32
        })
    }

    fn width_fn<'a>(cx: &'a Cx) -> impl Fn(&str, f32) -> i32 + 'a {
        move |s, px| cx.text.width(s, px)
    }

    fn step_sel(&mut self, dir: i32) -> bool {
        let n = self.items.len();
        let before = self.sel;
        let mut i = match (self.sel, dir > 0) {
            (Some(i), _) => i,
            (None, true) => n - 1,
            (None, false) => 0,
        };
        for _ in 0..n {
            i = (i as i32 + dir).rem_euclid(n as i32) as usize;
            if self.can(i) {
                self.sel = Some(i);
                break;
            }
        }
        self.sel != before
    }
}

impl Overlay for Menu {
    fn key(&mut self, k: &Key, _: &Cx) -> Out {
        if self.items.is_empty() {
            return Out::None;
        }
        let moved = match k.sym {
            sym::DOWN | sym::KP_DOWN => self.step_sel(1),
            sym::UP | sym::KP_UP => self.step_sel(-1),
            sym::HOME | sym::KP_HOME => {
                let to = self.enabled().next();
                std::mem::replace(&mut self.sel, to) != to
            }
            sym::END | sym::KP_END => {
                let to = self.enabled().last();
                std::mem::replace(&mut self.sel, to) != to
            }
            sym::RETURN | sym::KP_ENTER | sym::SPACE => {
                return match self.sel.filter(|&i| self.can(i)) {
                    Some(i) => Out::Run(self.items[i].id),
                    None => Out::None,
                };
            }
            _ => false,
        };
        if moved { Out::Redraw } else { Out::None }
    }

    fn pointer(&mut self, x: f32, y: f32, cx: &Cx) -> Out {
        let r = self.rect(&Menu::width_fn(cx), cx.g);
        let to = self.item_at(r, x, y);
        if std::mem::replace(&mut self.sel, to) != to { Out::Redraw } else { Out::None }
    }

    fn button(&mut self, c: Click, cx: &Cx) -> Out {
        let r = self.rect(&Menu::width_fn(cx), cx.g);
        let on = self.item_at(r, c.x, c.y);
        if c.pressed {
            self.armed = on;
            self.sel = on;
            return Out::Redraw;
        }
        match (self.armed.take(), on) {
            (Some(a), Some(b)) if a == b => Out::Run(self.items[a].id),
            _ => Out::None,
        }
    }

    fn contains(&self, x: f32, y: f32, cx: &Cx) -> bool {
        inside(self.rect(&Menu::width_fn(cx), cx.g), x, y)
    }

    fn cursor(&self, x: f32, y: f32, cx: &Cx) -> Cursor {
        if self.item_at(self.rect(&Menu::width_fn(cx), cx.g), x, y).is_some() { Cursor::Pointer } else { Cursor::Default }
    }

    fn step(&mut self, now: Instant) -> bool {
        self.fade.step(now)
    }

    fn draw(&self, p: &mut Paint, cx: &Cx) {
        let e = self.fade.value();
        p.fade = e;
        let t = cx.text;
        let (x, y0, w, h) = self.rect(&Menu::width_fn(cx), cx.g);
        let rise = ((1.0 - e) * RISE).round() as i32;
        let rect = (x, y0 + rise, w, h);
        card(p, rect, RADIUS, POP, 0.10, SHADOW_POP);
        for (i, it) in self.items.iter().enumerate() {
            let (top, rh) = self.row(rect, i);
            match it.kind {
                Kind::Sep => {
                    p.shape(x + 1 + PAD + 4, top + 5, x + w - 1 - PAD - 4, top + 6, WHITE, 0.07, &|_, _| 1.0);
                }
                Kind::Head => t.draw(p, x + 1 + PAD + ROW_PAD, top + 6 + 7, self.text(it.label), Ink::new(PX_HEAD, faint())),
                Kind::Item => paint_item(p, t, self, i, (x + 1 + PAD, top, w - 2 - 2 * PAD, rh)),
            }
        }
    }
}

fn paint_item(p: &mut Paint, t: &Text, m: &Menu, i: usize, (x, y, w, h): Rect) {
    let it = &m.items[i];
    let a = if it.flags & DISABLED != 0 { 0.4 } else { 1.0 };
    let cy = y + h / 2;
    if m.sel == Some(i) {
        p.rrect(x, y, w, h, 7.0, WHITE, 0.08);
    }
    let mut tx = x + ROW_PAD;
    if it.flags & CHECKED != 0 {
        icons::paint(p, Icon::Check, (tx + MARK_W / 2) as f32, cy as f32, 13.0, 2.0, accent(), a);
    }
    tx += MARK_W + GAP;
    if it.dot != NO_DOT {
        p.disc((tx + DOT / 2) as f32, cy as f32, DOT as f32 / 2.0, group_color(it.dot), a);
        tx += DOT + GAP;
    }
    let colour = if it.flags & DANGER != 0 { RED } else { fg() };
    t.draw(p, tx, cy, m.text(it.label), Ink::new(PX_MENU, colour).alpha(a));
    let keys = m.text(it.keys);
    if !keys.is_empty() {
        let kw = kbd_width(t, Cap::Text(keys));
        kbd(p, t, x + w - ROW_PAD - kw, cy, Cap::Text(keys), a);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay::Text;
    use crate::xkb::Mods;

    const G: Geo = Geo { w: 900, h: 620, square: false };

    /// Seven pixels a character at any size.
    fn w7(s: &str, _: f32) -> i32 {
        s.chars().count() as i32 * 7
    }

    fn key(sym: u32) -> Key {
        Key { sym, ..Key::default() }
    }

    /// The reference's menu of a window.
    fn tab_menu(x: i32, y: i32) -> Menu {
        Menu::at(x, y)
            .item("Rename", "F2", 1, 0)
            .item("Duplicate", "", 2, 0)
            .item("Split into columns", "Alt+\\", 3, 0)
            .sep()
            .head("Group")
            .dot("arcton", 1, 10, CHECKED)
            .dot("docs", 3, 11, 0)
            .item("New group with this window…", "Alt+G", 12, 0)
            .sep()
            .item("Close window", "Alt+W", 20, DANGER)
            .item("Close the others", "", 21, DANGER | DISABLED)
    }

    #[test]
    fn the_menu_is_as_wide_as_its_widest_item_and_never_under_230() {
        let m = tab_menu(0, 0);
        let (w, h) = m.size(&w7);
        // Rows: 8 items of 29, two separators of 11, one heading of 23, plus the padding and the border.
        assert_eq!(h, 8 * 29 + 2 * 11 + 23 + 2 * 5 + 2, "289, as the reference's menu of a window");
        // The widest is «New group with this window…» with its cap: 27 characters, 5 in the cap.
        assert_eq!(w, 9 + 14 + 9 + 27 * 7 + 9 + (5 * 7 + 12) + 9 + 2 * 5 + 2);
        assert_eq!(Menu::at(0, 0).item("Go", "", 1, 0).size(&w7), (MIN_W, 29 + 12), "short labels: the least width");
        let wide = Menu::at(0, 0).item("A rather long label for an item", "Ctrl+Shift+K", 1, 0);
        let (w, _) = wide.size(&w7);
        let label = 31 * 7;
        let cap = 12 * 7 + 12;
        assert_eq!(w, 9 + 14 + 9 + label + 9 + cap + 9 + 2 * 5 + 2);
        assert_eq!(Menu::at(0, 0).size(&w7), (MIN_W, 12));
    }

    #[test]
    fn the_menu_opens_at_the_pointer_and_is_kept_inside_the_window() {
        let (w, h) = (230, 300);
        assert_eq!(Menu::place((100, 100), (w, h), G), (100, 100));
        assert_eq!(Menu::place((850, 100), (w, h), G), (900 - 230 - 8, 100), "moved left to fit");
        assert_eq!(Menu::place((100, 600), (w, h), G), (100, 620 - 300 - 8), "and up");
        assert_eq!(Menu::place((-20, -20), (w, h), G), (8, 8), "never closer than 8 px to an edge");
        // Bigger than the window: the top-left corner wins, as in the reference.
        assert_eq!(Menu::place((50, 50), (w, 900), G), (50, 8));
    }

    #[test]
    fn only_items_that_can_be_chosen_are_found_under_the_pointer() {
        let m = tab_menu(100, 100);
        let r = (100, 100, 230, m.size(&w7).1);
        // The first row starts after the border and the padding.
        assert_eq!(m.item_at(r, 150.0, 106.0), Some(0));
        assert_eq!(m.item_at(r, 150.0, 106.0 + 29.0), Some(1));
        // The separator (row 3) and the heading (row 4) are not items.
        let (sep_top, _) = m.row(r, 3);
        assert_eq!(m.item_at(r, 150.0, (sep_top + 5) as f32), None);
        let (head_top, _) = m.row(r, 4);
        assert_eq!(m.item_at(r, 150.0, (head_top + 5) as f32), None);
        // The disabled last item is not found; the one before it is.
        let (close_top, _) = m.row(r, 9);
        assert_eq!(m.item_at(r, 150.0, (close_top + 5) as f32), Some(9));
        let (off_top, _) = m.row(r, 10);
        assert_eq!(m.item_at(r, 150.0, (off_top + 5) as f32), None);
        // The padding and the border at the sides are not items.
        assert_eq!(m.item_at(r, 102.0, 110.0), None);
        assert_eq!(m.item_at(r, 150.0, 99.0), None);
    }

    #[test]
    fn up_and_down_skip_separators_headings_and_disabled_items_and_wrap() {
        let text = Text::load();
        let cx = Cx { g: G, text: &text };
        let mut m = tab_menu(10, 10);
        assert_eq!(m.selected(), None, "opened by the pointer: nothing is lit");
        assert_eq!(m.key(&key(sym::DOWN), &cx), Out::Redraw);
        assert_eq!(m.selected(), Some(0));
        m.key(&key(sym::DOWN), &cx);
        m.key(&key(sym::DOWN), &cx);
        assert_eq!(m.selected(), Some(2));
        m.key(&key(sym::DOWN), &cx);
        assert_eq!(m.selected(), Some(5), "past the separator and the heading");
        for _ in 0..3 {
            m.key(&key(sym::DOWN), &cx);
        }
        assert_eq!(m.selected(), Some(9), "past the second separator to the last one that can be chosen");
        m.key(&key(sym::DOWN), &cx);
        assert_eq!(m.selected(), Some(0), "wraps round; the disabled item is skipped");
        m.key(&key(sym::UP), &cx);
        assert_eq!(m.selected(), Some(9));
        assert_eq!(m.key(&key(sym::HOME), &cx), Out::Redraw);
        assert_eq!(m.selected(), Some(0));
        assert_eq!(m.key(&key(sym::HOME), &cx), Out::None, "already there");
        m.key(&key(sym::END), &cx);
        assert_eq!(m.selected(), Some(9));
        // Up with nothing selected starts from the end.
        let mut m = tab_menu(10, 10);
        m.key(&key(sym::UP), &cx);
        assert_eq!(m.selected(), Some(9));
    }

    #[test]
    fn enter_and_space_run_the_selected_item_and_nothing_without_one() {
        let text = Text::load();
        let cx = Cx { g: G, text: &text };
        let mut m = tab_menu(10, 10);
        assert_eq!(m.key(&key(sym::RETURN), &cx), Out::None);
        m.key(&key(sym::DOWN), &cx);
        m.key(&key(sym::DOWN), &cx);
        assert_eq!(m.key(&key(sym::RETURN), &cx), Out::Run(2));
        assert_eq!(m.key(&key(sym::SPACE), &cx), Out::Run(2));
        assert_eq!(m.key(&key(0x61), &cx), Out::None, "a letter does nothing");
        assert_eq!(m.key(&Key { sym: sym::DOWN, mods: Mods { shift: true, ..Mods::default() }, ..Key::default() }, &cx), Out::Redraw);
        // Opened from the keyboard: the first item is lit.
        assert_eq!(tab_menu(0, 0).focus_first().selected(), Some(0));
    }

    #[test]
    fn the_pointer_lights_the_item_under_it_and_a_click_runs_it_on_release() {
        let text = Text::load();
        let cx = Cx { g: G, text: &text };
        let mut m = tab_menu(100, 100);
        let r = m.rect(&|s, px| text.width(s, px), G);
        let tops: Vec<f32> = (0..m.len()).map(|i| (m.row(r, i).0 + 10) as f32).collect();
        let row = |i: usize| (r.0 as f32 + 60.0, tops[i]);
        let (x, y) = row(1);
        assert_eq!(m.pointer(x, y, &cx), Out::Redraw);
        assert_eq!(m.selected(), Some(1));
        assert_eq!(m.pointer(x + 3.0, y, &cx), Out::None, "same item: nothing to repaint");
        let (sx, sy) = (r.0 as f32 + 60.0, (m.row(r, 3).0 + 5) as f32);
        m.pointer(sx, sy, &cx);
        assert_eq!(m.selected(), None, "over a separator: nothing is lit");
        // Press and release on one item runs it; released elsewhere does nothing.
        let click = |x, y, pressed| Click { x, y, pressed, time: 0 };
        let (x0, y0) = row(0);
        assert_eq!(m.button(click(x0, y0, true), &cx), Out::Redraw);
        assert_eq!(m.button(click(x0, y0, false), &cx), Out::Run(1));
        m.button(click(x0, y0, true), &cx);
        assert_eq!(m.button(click(x, y, false), &cx), Out::None, "dragged to another item");
        // A press on the separator arms nothing.
        m.button(click(sx, sy, true), &cx);
        assert_eq!(m.button(click(sx, sy, false), &cx), Out::None);
        // The disabled item cannot be clicked.
        let (dx, dy) = row(10);
        m.button(click(dx, dy, true), &cx);
        assert_eq!(m.button(click(dx, dy, false), &cx), Out::None);
        assert_eq!(m.cursor(x0, y0, &cx), Cursor::Pointer);
        assert_eq!(m.cursor(dx, dy, &cx), Cursor::Default);
        assert!(m.contains(x0, y0, &cx) && !m.contains(5.0, 5.0, &cx));
    }

    #[test]
    fn it_paints_every_part_and_the_danger_item_in_red() {
        let text = Text::load();
        let cx = Cx { g: G, text: &text };
        let mut m = tab_menu(100, 100);
        m.fade = Fade::full();
        let render = |m: &Menu| {
            let mut fb = vec![0u32; 900 * 620];
            let mut p = Paint { fb: &mut fb, w: 900, h: 620, ox: 0, oy: 0, fade: 1.0 };
            m.draw(&mut p, &cx);
            fb
        };
        let plain = render(&m);
        let r = m.rect(&|s, px| text.width(s, px), G);
        // Nothing outside the menu and its shadow.
        assert_eq!(plain[10 * 900 + 10], 0);
        // The danger row has red text (r high, g and b low-ish), the others have none.
        let red = |fb: &[u32], i: usize| {
            let (top, h) = m.row(r, i);
            (top..top + h).flat_map(|y| (r.0..r.0 + r.2).map(move |x| (y * 900 + x) as usize)).filter(|&k| {
                let px = fb[k];
                (px >> 16 & 255) > 200 && (px >> 8 & 255) < 170 && (px & 255) < 170 && px >> 24 != 0
            }).count()
        };
        assert!(red(&plain, 9) > 20, "Close window is red");
        assert_eq!(red(&plain, 0), 0);
        // Lighting an item changes the picture.
        m.sel = Some(0);
        assert_ne!(render(&m), plain);
        // The checkmark is amber.
        let amber = |px: u32| (px >> 16 & 255) > 200 && (px >> 8 & 255) > 140 && (px & 255) < 120 && px >> 24 != 0;
        let (top, h) = m.row(r, 5);
        let check = (top..top + h).flat_map(|y| (r.0..r.0 + 40).map(move |x| (y * 900 + x) as usize)).filter(|&k| amber(plain[k])).count();
        assert!(check > 5, "the checked item has an amber mark ({check})");
        // The disabled item is dimmer than the enabled one of the same colour.
        let lum = |i: usize| {
            let (top, h) = m.row(r, i);
            (top..top + h).flat_map(|y| (r.0..r.0 + r.2).map(move |x| (y * 900 + x) as usize)).map(|k| plain[k] >> 8 & 255).max().unwrap_or(0)
        };
        assert!(lum(10) < lum(9), "{} {}", lum(10), lum(9));
    }

    #[test]
    fn it_fades_in_while_rising_and_then_rests() {
        let mut m = tab_menu(10, 10);
        let t0 = Instant::now();
        assert!(m.step(t0));
        assert!(m.step(t0 + std::time::Duration::from_millis(60)));
        assert!(!m.step(t0 + std::time::Duration::from_millis(300)));
    }
}
