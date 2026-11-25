//! The group dialog ([0029](../knowledge/decisions/workspace/0029-group-dialog-menus-rename-toasts.md)), after the reference's
//! `#gdialog` and `openGroupDialog`: a name, one of the eight colours, the windows to put in the group, the
//! templates that open new windows with it, an error line, a summary, Cancel and Create; in edit mode (`Edit group`)
//! only the name and the colour. It is an overlay built from the kit (`Modal`, `Field`, `button`): a pure
//! function of its state and the window, painted over the grid while it is open, nothing kept between frames.
//!
//! The dialog checks its own form (`groups::check`: an empty name, a name another group has, nothing chosen)
//! and shows the reason in its error line; the app hears only of a valid submission, which the dialog leaves in
//! a cell it shares with it (`Submit`) before it answers `Out::Run(CMD_SUBMIT)`.
//!
//! Keyboard: Tab goes name, colours, windows, templates, Cancel, Create (the windows and templates are absent in edit
//! mode); arrows pick a colour or a template, move in the windows, Space ticks one, Enter submits (Esc closes: the
//! stack does that). The body scrolls when the card does not fit the window; the windows' list has its own wheel.
use std::{cell::RefCell, rc::Rc, time::Instant};

use crate::{
    field::{self, Edit, Field, H_BOX, Look},
    groups::{self, FormError, Template, fill},
    i18n::{Str, tr},
    input::sym,
    list::clipped,
    overlay::{BTN_H, CARD, Click, Clicks, Cx, Cursor, Geo, Ink, Modal, Out, Overlay, PX, PX_HINT, PX_SMALL, PX_TITLE, Rect, Text, accent, button, button_width, dim, fg, inside},
    paint::{BLACK, Paint, WHITE, seg},
    side::group_color,
    xkb::Key,
};

/// The tag the dialog is opened under on the overlay stack, and the command it runs when the form is valid.
pub const TAG: u32 = 0x4744_4c47;
pub const CMD_SUBMIT: u32 = 1;

/// A window the dialog offers to put in the group: its id (what the app knows it by), its name, its folder and
/// the group it is in now, if any (name and colour).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WinRow {
    pub id: u64,
    pub title: String,
    pub place: String,
    pub group: Option<(String, u8)>,
}

/// What the dialog is opened with.
pub struct Init {
    /// The id of the group being edited, or none for a new one.
    pub edit: Option<u32>,
    pub name: String,
    pub color: u8,
    pub wins: Vec<WinRow>,
    /// The windows ticked at first (their ids).
    pub picked: Vec<u64>,
    /// The names the other groups have.
    pub taken: Vec<String>,
}

/// A valid form: what the app does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Submit {
    pub edit: Option<u32>,
    pub name: String,
    pub color: u8,
    pub members: Vec<u64>,
    pub template: Template,
}

/// Where the dialog leaves its answer.
pub type Answer = Rc<RefCell<Option<Submit>>>;

/// Where the keyboard is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Zone {
    Name,
    Colors,
    Members,
    Templates,
    Cancel,
    Create,
}

impl Zone {
    const ALL: [Zone; 6] = [Zone::Name, Zone::Colors, Zone::Members, Zone::Templates, Zone::Cancel, Zone::Create];
}

/// What the pointer is over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hover {
    None,
    Name,
    Swatch(usize),
    Member(usize),
    Tpl(usize),
    Cancel,
    Create,
}

// -------------------------------------------------------------- measures --

/// The reference's measures, in pixels. The card is 470 wide; inside its 1 px border the head is 43 tall (18 above the
/// title, 4 under it), the body has 4 above its fields and 8 under them, the footer is 63 (a hairline, 14, a 34 px
/// button, 14). A field (`.field`) has 8 of padding around a 17 px label, a 7 px gap and its control.
const CARD_W: i32 = 470;
const HEAD_H: i32 = 43;
const FOOT_H: i32 = 63;
const BODY_TOP: i32 = 4;
const BODY_BOTTOM: i32 = 8;
/// The content's left edge from the card's: the body's 10 and the field's 10 of padding.
const SIDE: i32 = 20;
const FIELD_PAD: i32 = 8;
const LABEL_H: i32 = 17;
const LABEL_GAP: i32 = 7;
const NAME_H: i32 = FIELD_PAD + LABEL_H + LABEL_GAP + H_BOX + FIELD_PAD;
const ERR_H: i32 = 21;
const SWATCH: i32 = 24;
const SWATCH_GAP: i32 = 10;
const COLOR_H: i32 = FIELD_PAD + LABEL_H + LABEL_GAP + 30 + FIELD_PAD;
/// The windows' list: a box 150 tall with 4 of padding, rows of 30 with 2 between them, a gutter for its scrollbar.
const MEMBERS_BOX: i32 = 150;
/// In a window too short for the card the list gives up height before the body has to scroll, down to this (two rows).
const MEMBERS_MIN: i32 = 2 * BOX_PAD + 2 + 2 * ROW_PITCH - 2;
const ROW_H: i32 = 30;
const ROW_PITCH: i32 = 32;
const BOX_PAD: i32 = 4;
const GUTTER: i32 = 10;
const TPL_BOX: i32 = 75;
const TPL_GAP: i32 = 8;
const TPL_H: i32 = FIELD_PAD + LABEL_H + LABEL_GAP + TPL_BOX + FIELD_PAD;
/// The error line is 17 tall with 4 under it.
const RED: [f32; 3] = [1.0, 0.553, 0.553];

/// Where each part of the body starts, from the body's top before scrolling, and how tall it all is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Plan {
    name: i32,
    err: Option<i32>,
    color: i32,
    members: Option<i32>,
    tpls: Option<i32>,
    total: i32,
}

/// Where everything is for a card at `card` (window pixels), with the body scrolled.
struct Lay {
    /// The body's viewport.
    body: Rect,
    name: Rect,
    name_label: i32,
    err: Option<i32>,
    color_label: i32,
    swatch_y: i32,
    members_label: Option<i32>,
    members: Option<Rect>,
    tpl_label: Option<i32>,
    tpls: [Rect; 3],
    cancel: Rect,
    create: Rect,
    foot: i32,
    /// The left edge of the content.
    x: i32,
}

pub struct GroupDialog {
    modal: Modal,
    edit: Option<u32>,
    wins: Vec<WinRow>,
    taken: Vec<String>,
    name: Field,
    color: u8,
    picked: Vec<bool>,
    template: Template,
    error: Option<FormError>,
    zone: Zone,
    /// The row the keyboard is on in the windows' list.
    cursor: usize,
    /// How far the body and the windows' list are scrolled.
    scroll: i32,
    list_scroll: i32,
    hover: Hover,
    armed: Hover,
    clicks: Clicks,
    out: Answer,
}

impl GroupDialog {
    /// A dialog opened with `init`; the answer goes to `out`.
    pub fn new(init: Init, out: Answer) -> GroupDialog {
        let picked = init.wins.iter().map(|w| init.picked.contains(&w.id)).collect();
        // A new group with no window chosen starts with "one window" ticked, as the reference does.
        let template = if init.edit.is_none() && !init.wins.iter().any(|w| init.picked.contains(&w.id)) { Template::One } else { Template::None };
        let mut name = Field::new(groups::NAME_MAX, tr(Str::OvFieldNameHint));
        name.set(&init.name);
        name.select_all();
        GroupDialog {
            modal: Modal::new(CARD_W, 0),
            edit: init.edit,
            wins: init.wins,
            taken: init.taken,
            name,
            color: init.color % groups::COLORS as u8,
            picked,
            template,
            error: None,
            zone: Zone::Name,
            cursor: 0,
            scroll: 0,
            list_scroll: 0,
            hover: Hover::None,
            armed: Hover::None,
            clicks: Clicks::default(),
            out,
        }
    }

    // ----------------------------------------------------------- the form --

    pub fn is_edit(&self) -> bool {
        self.edit.is_some()
    }

    #[cfg(test)]
    pub fn error(&self) -> Option<FormError> {
        self.error
    }

    #[cfg(test)]
    pub fn name(&self) -> &str {
        self.name.text()
    }

    #[cfg(test)]
    pub fn color(&self) -> u8 {
        self.color
    }

    #[cfg(test)]
    pub fn template(&self) -> Template {
        self.template
    }

    /// The ids of the windows ticked.
    pub fn members(&self) -> Vec<u64> {
        self.wins.iter().zip(&self.picked).filter(|(_, on)| **on).map(|(w, _)| w.id).collect()
    }

    /// How many windows the group will have: the ones ticked and the ones the template opens.
    pub fn total(&self) -> usize {
        self.picked.iter().filter(|&&on| on).count() + self.template.count()
    }

    /// The zones the keyboard can be in: the windows and the templates are not there when editing.
    fn zones(&self) -> impl Iterator<Item = Zone> + '_ {
        Zone::ALL.into_iter().filter(|z| !(self.is_edit() && matches!(z, Zone::Members | Zone::Templates)))
    }

    fn step_zone(&mut self, d: isize) {
        let zs: Vec<Zone> = self.zones().collect();
        let i = zs.iter().position(|&z| z == self.zone).unwrap_or(0) as isize;
        self.zone = zs[(i + d).rem_euclid(zs.len() as isize) as usize];
    }

    /// Checks the form; a valid one is left in the shared cell. The error line says what is wrong.
    fn submit(&mut self) -> Out {
        let taken: Vec<&str> = self.taken.iter().map(String::as_str).collect();
        let members = self.members();
        match groups::check(self.name.text(), &taken, members.len(), self.template.count(), self.is_edit()) {
            Ok(name) => {
                *self.out.borrow_mut() = Some(Submit { edit: self.edit, name, color: self.color, members: if self.is_edit() { Vec::new() } else { members }, template: if self.is_edit() { Template::None } else { self.template } });
                Out::Run(CMD_SUBMIT)
            }
            Err(e) => {
                self.error = Some(e);
                if e != FormError::NothingChosen {
                    self.zone = Zone::Name;
                }
                Out::Redraw
            }
        }
    }

    fn toggle(&mut self, i: usize) {
        if let Some(on) = self.picked.get_mut(i) {
            *on = !*on;
            self.error = None;
        }
    }

    fn pick_template(&mut self, t: Template) {
        self.template = t;
        self.error = None;
    }

    // ------------------------------------------------------------ geometry --

    /// The windows' box height in a card `card_h` tall: 150, or less when the card was cut to fit the window.
    fn bh(&self, card_h: i32) -> i32 {
        if self.is_edit() { MEMBERS_BOX } else { (MEMBERS_BOX - (self.natural_h() - card_h).max(0)).clamp(MEMBERS_MIN, MEMBERS_BOX) }
    }

    fn plan(&self, bh: i32) -> Plan {
        let mut y = BODY_TOP;
        let name = y;
        y += NAME_H;
        let err = self.error.map(|_| y);
        if err.is_some() {
            y += ERR_H;
        }
        let color = y;
        y += COLOR_H;
        let (members, tpls) = if self.is_edit() {
            (None, None)
        } else {
            let m = y;
            y += FIELD_PAD + LABEL_H + LABEL_GAP + bh + FIELD_PAD;
            let t = y;
            y += TPL_H;
            (Some(m), Some(t))
        };
        Plan { name, err, color, members, tpls, total: y + BODY_BOTTOM }
    }

    /// The card's height with everything shown.
    fn natural_h(&self) -> i32 {
        2 + HEAD_H + self.plan(MEMBERS_BOX).total + FOOT_H
    }

    fn frame(&self) -> Modal {
        Modal { h: self.natural_h(), ..self.modal }
    }

    /// The body's viewport height in a card `card_h` tall, and how far the body can scroll.
    fn view_h(card_h: i32) -> i32 {
        (card_h - 2 - HEAD_H - FOOT_H).max(0)
    }

    fn max_scroll(&self, card_h: i32) -> i32 {
        (self.plan(self.bh(card_h)).total - GroupDialog::view_h(card_h)).max(0)
    }

    /// How tall the windows' rows are together, and how far the list can scroll.
    fn list_content(&self) -> i32 {
        (self.wins.len() as i32 * ROW_PITCH - 2).max(0)
    }

    /// The rows' viewport in a box `bh` tall, how far they can scroll, and whether they do not all fit.
    fn list_view(bh: i32) -> i32 {
        bh - 2 - 2 * BOX_PAD
    }

    fn list_max(&self, bh: i32) -> i32 {
        (self.list_content() - GroupDialog::list_view(bh)).max(0)
    }

    fn overflows(&self, bh: i32) -> bool {
        self.list_content() > GroupDialog::list_view(bh)
    }

    fn lay(&self, card: Rect, t: &Text) -> Lay {
        let bh = self.bh(card.3);
        let pl = self.plan(bh);
        let (x, y, w, h) = card;
        let (cx, cw) = (x + 1 + SIDE, w - 2 - 2 * SIDE);
        let scroll = self.scroll.clamp(0, self.max_scroll(h));
        let top = y + 1 + HEAD_H - scroll;
        let body = (x + 1, y + 1 + HEAD_H, w - 2, GroupDialog::view_h(h));
        let tw = (cw - 2 * TPL_GAP) / 3;
        let tpl_y = pl.tpls.map_or(0, |o| top + o + FIELD_PAD + LABEL_H + LABEL_GAP);
        let tpls = [0, 1, 2].map(|i| (cx + i * (tw + TPL_GAP), tpl_y, if i == 2 { cw - 2 * (tw + TPL_GAP) } else { tw }, TPL_BOX));
        let foot = y + h - 1 - FOOT_H;
        let (cancel_l, create_l) = (tr(Str::Cancel), if self.is_edit() { tr(Str::GroupSave) } else { tr(Str::OvCreateGroup) });
        let (cwid, pwid) = (button_width(t, cancel_l), button_width(t, create_l));
        let by = foot + 1 + 14;
        let right = x + w - 1 - SIDE;
        Lay {
            body,
            name: (cx, top + pl.name + FIELD_PAD + LABEL_H + LABEL_GAP, cw, H_BOX),
            name_label: top + pl.name + FIELD_PAD + LABEL_H / 2,
            err: pl.err.map(|o| top + o + LABEL_H / 2),
            color_label: top + pl.color + FIELD_PAD + LABEL_H / 2,
            swatch_y: top + pl.color + FIELD_PAD + LABEL_H + LABEL_GAP + 2,
            members_label: pl.members.map(|o| top + o + FIELD_PAD + LABEL_H / 2),
            members: pl.members.map(|o| (cx, top + o + FIELD_PAD + LABEL_H + LABEL_GAP, cw, bh)),
            tpl_label: pl.tpls.map(|o| top + o + FIELD_PAD + LABEL_H / 2),
            tpls,
            cancel: (right - pwid - 8 - cwid, by, cwid, BTN_H),
            create: (right - pwid, by, pwid, BTN_H),
            foot,
            x: cx,
        }
    }

    /// The swatch `i`'s box.
    fn swatch(l: &Lay, i: usize) -> Rect {
        (l.x + 2 + i as i32 * (SWATCH + SWATCH_GAP), l.swatch_y, SWATCH, SWATCH)
    }

    /// The first row's top and the rows' width in the windows' box `m`, with the list scrolled.
    fn rows_at(&self, m: Rect) -> (i32, i32) {
        let ls = self.list_scroll.clamp(0, self.list_max(m.3));
        (m.1 + 1 + BOX_PAD - ls, m.2 - 2 - 2 * BOX_PAD - if self.overflows(m.3) { GUTTER } else { 0 })
    }

    fn member_rect(&self, m: Rect, i: usize) -> Rect {
        let (y0, w) = self.rows_at(m);
        (m.0 + 1 + BOX_PAD, y0 + i as i32 * ROW_PITCH, w, ROW_H)
    }

    /// What is under window pixel `(x, y)`.
    fn hit(&self, l: &Lay, x: f32, y: f32) -> Hover {
        if inside(l.cancel, x, y) {
            return Hover::Cancel;
        }
        if inside(l.create, x, y) {
            return Hover::Create;
        }
        if !inside(l.body, x, y) {
            return Hover::None;
        }
        if inside(l.name, x, y) {
            return Hover::Name;
        }
        if let Some(i) = (0..groups::COLORS).find(|&i| inside(GroupDialog::swatch(l, i), x, y)) {
            return Hover::Swatch(i);
        }
        if let Some(m) = l.members.filter(|&m| inside(m, x, y)) {
            let inner = (m.0 + 1, m.1 + 1, m.2 - 2, m.3 - 2);
            if let Some(i) = (0..self.wins.len()).find(|&i| inside(self.member_rect(m, i), x, y) && inside(inner, x, y)) {
                return Hover::Member(i);
            }
        }
        if self.edit.is_none()
            && let Some(i) = (0..3).find(|&i| inside(l.tpls[i], x, y))
        {
            return Hover::Tpl(i);
        }
        Hover::None
    }

    /// Scrolls the body so that the part from `off` to `off + h` (from the body's top) is in view.
    fn reveal(&mut self, off: i32, h: i32, card_h: i32) {
        let view = GroupDialog::view_h(card_h);
        if off < self.scroll {
            self.scroll = off;
        } else if off + h > self.scroll + view {
            self.scroll = off + h - view;
        }
        self.scroll = self.scroll.clamp(0, self.max_scroll(card_h));
    }

    /// After the keyboard moved: brings the zone it is in into view.
    fn follow(&mut self, g: Geo) {
        let card_h = self.frame().rect(g).3;
        let bh = self.bh(card_h);
        let pl = self.plan(bh);
        match self.zone {
            Zone::Name => self.reveal(pl.name - BODY_TOP, NAME_H + BODY_TOP, card_h),
            Zone::Colors => self.reveal(pl.color, COLOR_H, card_h),
            Zone::Members => {
                if let Some(o) = pl.members {
                    self.reveal(o, FIELD_PAD + LABEL_H + LABEL_GAP + bh + FIELD_PAD, card_h);
                }
                // And the row the cursor is on, in its own list.
                let (top, bottom) = (self.cursor as i32 * ROW_PITCH, self.cursor as i32 * ROW_PITCH + ROW_H);
                let view = GroupDialog::list_view(bh);
                if top < self.list_scroll {
                    self.list_scroll = top;
                } else if bottom > self.list_scroll + view {
                    self.list_scroll = bottom - view;
                }
                self.list_scroll = self.list_scroll.clamp(0, self.list_max(bh));
            }
            Zone::Templates => {
                if let Some(o) = pl.tpls {
                    self.reveal(o, TPL_H + BODY_BOTTOM, card_h);
                }
            }
            Zone::Cancel | Zone::Create => {}
        }
    }

    // --------------------------------------------- states for the design board --

    /// Already faded in (the design board's frames).
    #[cfg(test)]
    pub fn shown(mut self) -> GroupDialog {
        self.modal.fade = crate::overlay::Fade::full();
        self
    }

    /// With the form's error showing, as after pressing Create.
    #[cfg(test)]
    pub fn with_error(mut self, e: FormError) -> GroupDialog {
        self.error = Some(e);
        self
    }

    #[cfg(test)]
    pub fn at(mut self, z: Zone) -> GroupDialog {
        self.zone = z;
        self
    }

    #[cfg(test)]
    pub fn over(mut self, h: Hover) -> GroupDialog {
        self.hover = h;
        self
    }

    #[cfg(test)]
    pub fn with_template(mut self, t: Template) -> GroupDialog {
        self.template = t;
        self
    }

    #[cfg(test)]
    pub fn with_color(mut self, c: u8) -> GroupDialog {
        self.color = c;
        self
    }

    #[cfg(test)]
    pub fn with_name(mut self, s: &str) -> GroupDialog {
        self.name.set(s);
        self
    }

    /// The windows' list scrolled by `px` (clamped to its end).
    #[cfg(test)]
    pub fn scrolled_list(mut self, px: i32) -> GroupDialog {
        self.list_scroll = px.max(0);
        self
    }
}

/// `s` broken on spaces into at most two lines that fit `max` pixels (`width` measures); the second is cut with
/// `…` when it still does not fit. The templates' hints are two lines at most.
pub fn wrap2(s: &str, max: i32, width: &dyn Fn(&str) -> i32) -> [String; 2] {
    if width(s) <= max {
        return [s.to_string(), String::new()];
    }
    let mut first = String::new();
    let mut rest = s;
    for (i, c) in s.char_indices() {
        if c == ' ' && width(&s[..i]) <= max {
            (first, rest) = (s[..i].to_string(), s[i + 1..].trim_start());
        } else if width(&s[..i]) > max {
            break;
        }
    }
    if first.is_empty() {
        return [crate::overlay::fit(s, max, width).into_owned(), String::new()];
    }
    [first, crate::overlay::fit(rest, max, width).into_owned()]
}

impl Overlay for GroupDialog {
    fn key(&mut self, k: &Key, cx: &Cx) -> Out {
        match k.sym {
            sym::TAB | sym::ISO_LEFT_TAB => {
                self.step_zone(if k.mods.shift || k.sym == sym::ISO_LEFT_TAB { -1 } else { 1 });
                self.follow(cx.g);
                return Out::Redraw;
            }
            sym::RETURN | sym::KP_ENTER => return if self.zone == Zone::Cancel { Out::Close } else { self.submit() },
            _ => {}
        }
        let next = |i: usize, n: usize, d: isize| (i as isize + d).rem_euclid(n.max(1) as isize) as usize;
        let d = match k.sym {
            sym::LEFT | sym::UP | sym::KP_LEFT | sym::KP_UP => -1,
            sym::RIGHT | sym::DOWN | sym::KP_RIGHT | sym::KP_DOWN => 1,
            _ => 0,
        };
        match self.zone {
            Zone::Name => match self.name.key(k) {
                Edit::Ignored | Edit::Same => Out::None,
                Edit::Moved => Out::Redraw,
                Edit::Changed => {
                    self.error = None;
                    Out::Redraw
                }
                Edit::Paste => Out::Paste,
                Edit::Copy(t) => Out::Copy(t),
                Edit::Cut(t) => {
                    self.error = None;
                    Out::Copy(t)
                }
            },
            Zone::Colors if d != 0 => {
                self.color = next(self.color as usize, groups::COLORS, d) as u8;
                Out::Redraw
            }
            Zone::Templates if d != 0 => {
                let i = Template::ALL.iter().position(|&t| t == self.template).unwrap_or(0);
                self.pick_template(Template::ALL[next(i, 3, d)]);
                Out::Redraw
            }
            Zone::Members if d != 0 || matches!(k.sym, sym::HOME | sym::END | sym::KP_HOME | sym::KP_END) => {
                let last = self.wins.len().saturating_sub(1);
                self.cursor = match k.sym {
                    sym::HOME | sym::KP_HOME => 0,
                    sym::END | sym::KP_END => last,
                    _ => (self.cursor as isize + d).clamp(0, last as isize) as usize,
                };
                self.follow(cx.g);
                Out::Redraw
            }
            Zone::Members if k.sym == sym::SPACE => {
                self.toggle(self.cursor);
                Out::Redraw
            }
            Zone::Cancel if k.sym == sym::SPACE => Out::Close,
            Zone::Create if k.sym == sym::SPACE => self.submit(),
            _ => Out::None,
        }
    }

    fn paste(&mut self, text: &str, _: &Cx) -> Out {
        if self.zone == Zone::Name && self.name.insert(text) {
            self.error = None;
            Out::Redraw
        } else {
            Out::None
        }
    }

    fn pointer(&mut self, x: f32, y: f32, cx: &Cx) -> Out {
        let card = self.frame().rect(cx.g);
        let l = self.lay(card, cx.text);
        let over = self.hit(&l, x, y);
        let mut out = if std::mem::replace(&mut self.hover, over) != over { Out::Redraw } else { Out::None };
        if self.name.dragging() && self.name.drag_at(x, l.name, Look::Box, cx.text) {
            out = Out::Redraw;
        }
        out
    }

    fn button(&mut self, c: Click, cx: &Cx) -> Out {
        let card = self.frame().rect(cx.g);
        let l = self.lay(card, cx.text);
        let on = self.hit(&l, c.x, c.y);
        if !c.pressed {
            self.name.release();
            let armed = std::mem::replace(&mut self.armed, Hover::None);
            return match (armed, on) {
                (Hover::Cancel, Hover::Cancel) => Out::Close,
                (Hover::Create, Hover::Create) => self.submit(),
                _ => Out::None,
            };
        }
        match on {
            Hover::Name => {
                self.zone = Zone::Name;
                let clicks = self.clicks.press(c.time);
                self.name.press_at(c.x, l.name, Look::Box, cx.text, clicks, false);
            }
            Hover::Swatch(i) => {
                self.color = i as u8;
                self.zone = Zone::Colors;
            }
            Hover::Member(i) => {
                self.cursor = i;
                self.zone = Zone::Members;
                self.toggle(i);
            }
            Hover::Tpl(i) => {
                self.zone = Zone::Templates;
                self.pick_template(Template::ALL[i]);
            }
            Hover::Cancel | Hover::Create => {
                self.armed = on;
                self.zone = if on == Hover::Cancel { Zone::Cancel } else { Zone::Create };
            }
            Hover::None => return Out::None,
        }
        Out::Redraw
    }

    fn scroll(&mut self, dy: f32, x: f32, y: f32, cx: &Cx) -> Out {
        let card = self.frame().rect(cx.g);
        let l = self.lay(card, cx.text);
        let before = (self.scroll, self.list_scroll);
        if let Some(m) = l.members.filter(|&m| inside(m, x, y) && self.overflows(m.3)) {
            self.list_scroll = (self.list_scroll + dy.round() as i32).clamp(0, self.list_max(m.3));
        } else {
            self.scroll = (self.scroll + dy.round() as i32).clamp(0, self.max_scroll(card.3));
        }
        if before != (self.scroll, self.list_scroll) { Out::Redraw } else { Out::None }
    }

    fn contains(&self, x: f32, y: f32, cx: &Cx) -> bool {
        self.frame().contains(cx.g, x, y)
    }

    fn cursor(&self, x: f32, y: f32, cx: &Cx) -> Cursor {
        let card = self.frame().rect(cx.g);
        match self.hit(&self.lay(card, cx.text), x, y) {
            Hover::None => Cursor::Default,
            Hover::Name => Cursor::Text,
            _ => Cursor::Pointer,
        }
    }

    fn step(&mut self, now: Instant) -> bool {
        self.modal.fade.step(now)
    }

    fn draw(&self, p: &mut Paint, cx: &Cx) {
        let t = cx.text;
        let (x, y, w, h) = self.frame().paint(p, cx.g);
        // The card as drawn rises while it fades in: everything is laid out on that.
        let l = self.lay((x, y, w, h), t);
        // The head: the title, and the hint at the right.
        let cy = y + 1 + 18 + 10;
        let (title, hint) = if self.is_edit() { (tr(Str::GroupEdit), tr(Str::GroupEditHint)) } else { (tr(Str::OvDialogTitle), tr(Str::OvDialogHint)) };
        t.draw(p, l.x, cy, title, Ink::new(PX_TITLE, fg()).bold());
        t.draw(p, x + w - 1 - SIDE - t.width(hint, PX_HINT), cy + 1, hint, Ink::new(PX_HINT, dim()));
        {
            let mut b = clipped(p, l.body.1, l.body.1 + l.body.3);
            self.draw_body(&mut b, t, &l);
        }
        // The footer: a hairline, the summary and the buttons.
        p.shape(x + 1, l.foot, x + w - 1, l.foot + 1, WHITE, 0.07, &|_, _| 1.0);
        if !self.is_edit() {
            let n = self.total();
            let sum = fill("{} {}", &[&n.to_string(), tr(if n == 1 { Str::WinOne } else { Str::WinMany })]);
            t.draw(p, l.x, l.cancel.1 + BTN_H / 2, &sum, Ink::new(PX_HINT, dim()));
        }
        button(p, t, l.cancel, tr(Str::Cancel), false, self.hover == Hover::Cancel || self.zone == Zone::Cancel);
        button(p, t, l.create, if self.is_edit() { tr(Str::GroupSave) } else { tr(Str::OvCreateGroup) }, true, self.hover == Hover::Create || self.zone == Zone::Create);
    }
}

impl GroupDialog {
    fn draw_body(&self, p: &mut Paint, t: &Text, l: &Lay) {
        let label = |p: &mut Paint, cy: i32, s: &str| t.draw(p, l.x, cy, s, Ink::new(PX_HINT, dim()));
        // The name.
        label(p, l.name_label, tr(Str::OvFieldName));
        field::paint(p, t, &self.name, l.name, Look::Box, self.zone == Zone::Name);
        if let (Some(cy), Some(e)) = (l.err, self.error) {
            let s = tr(match e {
                FormError::NameEmpty => Str::ErrNameEmpty,
                FormError::NameTaken => Str::ErrNameTaken,
                FormError::NothingChosen => Str::ErrNothing,
            });
            t.draw(p, l.x, cy, s, Ink::new(PX_HINT, RED));
        }
        // The colours.
        label(p, l.color_label, tr(Str::GroupColor));
        for i in 0..groups::COLORS {
            let (sx, sy, _, _) = GroupDialog::swatch(l, i);
            let (cx, cy) = ((sx + SWATCH / 2) as f32, (sy + SWATCH / 2) as f32);
            let c = group_color(i as u8);
            if i == self.color as usize {
                // A ring of the card's colour, then one of the group's: `0 0 0 2px #141416, 0 0 0 4px colour`.
                p.disc(cx, cy, 16.0, c, 1.0);
                p.disc(cx, cy, 14.0, CARD, 1.0);
                p.disc(cx, cy, 12.0, c, 1.0);
            } else {
                p.disc(cx, cy, if self.hover == Hover::Swatch(i) { 13.2 } else { 12.0 }, c, 1.0);
                p.ring(cx, cy, 11.5, 1.0, WHITE, 0.20);
            }
        }
        // The windows.
        if let (Some(cy), Some(m)) = (l.members_label, l.members) {
            label(p, cy, tr(Str::GroupWindows));
            self.draw_members(p, t, m);
        }
        // The templates.
        if let Some(cy) = l.tpl_label {
            label(p, cy, tr(Str::GroupNewWindows));
            let items = [(tr(Str::TplNone), tr(Str::TplNoneHint)), (tr(Str::TplOne), tr(Str::TplOneHint)), (tr(Str::TplDev), tr(Str::TplDevHint))];
            for (i, (title, hint)) in items.into_iter().enumerate() {
                let (x, y, w, h) = l.tpls[i];
                let on = self.template == Template::ALL[i];
                if on {
                    p.rrect(x, y, w, h, 10.0, accent(), 0.09);
                    p.outline(x, y, w, h, 10.0, accent(), 0.55);
                } else {
                    p.rrect(x, y, w, h, 10.0, WHITE, 0.025);
                    p.outline(x, y, w, h, 10.0, WHITE, if self.hover == Hover::Tpl(i) { 0.16 } else { 0.07 });
                }
                let room = w - 2 - 20;
                t.draw(p, x + 11, y + 20, title, Ink::new(PX, fg()).bold());
                let lines = wrap2(hint, room, &|s| t.width(s, PX_SMALL));
                for (n, line) in lines.iter().filter(|s| !s.is_empty()).enumerate() {
                    t.draw(p, x + 11, y + 40 + n as i32 * 16, line, Ink::new(PX_SMALL, dim()));
                }
            }
        }
    }

    fn draw_members(&self, p: &mut Paint, t: &Text, m: Rect) {
        let (x, y, w, h) = m;
        p.rrect(x, y, w, h, 10.0, BLACK, 0.25);
        p.outline(x, y, w, h, 10.0, WHITE, 0.07);
        let mut b = clipped(p, y + 1, y + h - 1);
        for (i, row) in self.wins.iter().enumerate() {
            let r = self.member_rect(m, i);
            if r.1 + r.3 < y || r.1 > y + h {
                continue;
            }
            let (rx, ry, rw, rh) = r;
            let cy = ry + rh / 2;
            let lit = self.hover == Hover::Member(i) || (self.zone == Zone::Members && self.cursor == i);
            if lit {
                b.rrect(rx, ry, rw, rh, 7.0, WHITE, 0.05);
            }
            // The check box: a hairline square, filled with the accent when ticked.
            let on = self.picked[i];
            let (bx, by) = (rx + 8, cy - 8);
            if on {
                b.rrect(bx, by, 16, 16, 5.0, accent(), 1.0);
                let (mx, my) = (bx as f32 + 8.0, cy as f32);
                let dark = [0.102, 0.075, 0.031];
                b.shape(bx, by, bx + 16, by + 16, dark, 1.0, &|px, py| (1.1 - seg(px, py, (mx - 3.6, my + 0.2), (mx - 1.0, my + 2.8)).min(seg(px, py, (mx - 1.0, my + 2.8), (mx + 3.8, my - 2.6)))).clamp(0.0, 1.0));
            } else {
                b.outline(bx, by, 16, 16, 5.0, [0.353, 0.353, 0.384], 1.0);
            }
            // The badge of the group it is in now, at the right end.
            let mut right = rx + rw - 8;
            if let Some((g, c)) = &row.group {
                let name = t.fit(g, 11.0, 90);
                let bw = t.width(&name, 11.0) + 14;
                let gc = group_color(*c);
                b.rrect(right - bw, cy - 9, bw, 18, 6.0, gc, 0.22);
                let text = [0, 1, 2].map(|i| gc[i] * 0.8 + 0.2);
                t.draw(&mut b, right - bw + 7, cy, &name, Ink::new(11.0, text));
                right -= bw + 10;
            }
            let tx = rx + 8 + 16 + 10;
            let title = t.fit(&row.title, PX, (right - tx).max(0));
            t.draw(&mut b, tx, cy, &title, Ink::new(PX, fg()));
            let mx = tx + t.width(&title, PX) + 10;
            if right - mx > 24 {
                let meta = t.fit(&row.place, PX_SMALL, right - mx);
                t.draw(&mut b, mx, cy, &meta, Ink::new(PX_SMALL, dim()));
            }
        }
        // The scrollbar: a thin thumb at the right, when the rows do not fit.
        if self.overflows(h) {
            let (view, content) = (GroupDialog::list_view(h), self.list_content());
            let th = (view * view / content).max(24);
            let ty = y + 1 + BOX_PAD + (view - th) * self.list_scroll.clamp(0, self.list_max(h)) / self.list_max(h).max(1);
            b.rrect(x + w - 1 - 7, ty, 4, th, 2.0, [0.165, 0.165, 0.180], 1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xkb::Mods;

    const G: Geo = Geo { w: 1000, h: 640, square: false };

    fn rows(n: usize) -> Vec<WinRow> {
        (0..n).map(|i| WinRow { id: 10 + i as u64, title: format!("win{i}"), place: "~/lab/apps/axon".into(), group: (i == 1).then(|| ("old".to_string(), 1)) }).collect()
    }

    fn init(n: usize, picked: &[u64], taken: &[&str]) -> Init {
        Init { edit: None, name: "axon".into(), color: 0, wins: rows(n), picked: picked.to_vec(), taken: taken.iter().map(|s| s.to_string()).collect() }
    }

    fn dialog(n: usize, picked: &[u64], taken: &[&str]) -> (GroupDialog, Answer) {
        let out = Answer::default();
        (GroupDialog::new(init(n, picked, taken), out.clone()), out)
    }

    fn edit_dialog() -> (GroupDialog, Answer) {
        let out = Answer::default();
        (GroupDialog::new(Init { edit: Some(5), name: "docs".into(), color: 3, wins: rows(4), picked: vec![], taken: vec!["api".into()] }, out.clone()), out)
    }

    fn key(sym: u32) -> Key {
        Key { sym, ..Key::default() }
    }

    /// What the keyboard says for Shift+Tab: the layout turns it into its own key.
    fn shift_tab() -> Key {
        Key { sym: sym::ISO_LEFT_TAB, mods: Mods { shift: true, ..Mods::default() }, ..Key::default() }
    }

    fn typed(s: &str) -> Key {
        Key { sym: s.chars().next().unwrap() as u32, text: s.into(), ..Key::default() }
    }

    fn click(x: i32, y: i32, pressed: bool) -> Click {
        Click { x: x as f32, y: y as f32, pressed, time: 1000 }
    }

    /// The dialog's layout in the real window, for finding where to click.
    fn lay(d: &GroupDialog, t: &Text) -> Lay {
        d.lay(d.frame().rect(G), t)
    }

    #[test]
    fn the_cards_height_is_the_references_with_and_without_the_windows_and_the_error_line() {
        let (d, _) = dialog(5, &[10], &[]);
        // 1 + 43 + (4 + 76 + 70 + 190 + 115 + 8) + 63 + 1: the reference's 570.6 px dialog.
        assert_eq!(d.natural_h(), 2 + 43 + 463 + 63);
        assert_eq!(d.natural_h(), 571);
        let (e, _) = edit_dialog();
        assert_eq!(e.natural_h(), 266, "the reference's 265.6 px: only the name and the colour");
        assert_eq!(d.with_error(FormError::NameEmpty).natural_h(), 571 + 21);
    }

    #[test]
    fn a_card_taller_than_the_window_scrolls_its_body_and_keeps_the_head_and_the_footer() {
        let (d, _) = dialog(5, &[10], &[]);
        let card = d.frame().rect(Geo { w: 1000, h: 480, square: false });
        assert!(card.3 < d.natural_h(), "{card:?}");
        let max = d.max_scroll(card.3);
        assert_eq!(max, d.plan(d.bh(card.3)).total - (card.3 - 2 - 43 - 63));
        assert_eq!(d.bh(card.3), MEMBERS_MIN, "the list gave up all it could first");
        assert!(max > 0);
        // A window that holds it: nothing to scroll.
        assert_eq!(d.max_scroll(d.natural_h()), 0);
        // Tab to the buttons and back to the templates: the body follows the keyboard.
        let t = Text::load();
        let g = Geo { w: 1000, h: 480, square: false };
        let cx = Cx { g, text: &t };
        let mut d = d;
        for _ in 0..3 {
            d.key(&key(sym::TAB), &cx);
        }
        assert_eq!(d.zone, Zone::Templates);
        assert_eq!(d.scroll, d.max_scroll(card.3), "the templates are at the bottom of the body");
        d.key(&shift_tab(), &cx);
        d.key(&shift_tab(), &cx);
        d.key(&shift_tab(), &cx);
        assert_eq!((d.zone, d.scroll), (Zone::Name, 0), "and the name brings the top back");
    }

    #[test]
    fn a_window_a_little_too_short_makes_the_windows_list_shorter_before_the_body_has_to_scroll() {
        let (d, _) = dialog(9, &[10], &[]);
        // The design board's window: 620 high, 556 for the card under the title bar and above the margin.
        let card = d.frame().rect(Geo { w: 900, h: 620, square: false });
        assert_eq!(card.3, 620 - 16 - 48);
        assert_eq!(d.bh(card.3), 150 - (571 - 556), "the list gives the missing 15 px");
        assert_eq!(d.max_scroll(card.3), 0, "and the body has nothing to scroll");
        assert_eq!(d.bh(d.natural_h()), 150, "with room it is the reference's 150");
        // The edit dialog has no list.
        let (e, _) = edit_dialog();
        assert_eq!(e.bh(100), 150);
        // With the error line (21 more) it gives 36; the rows are found where they are drawn.
        let d = d.with_error(FormError::NameEmpty);
        assert_eq!(d.bh(card.3), 150 - (592 - 556));
        let t = Text::load();
        let l = d.lay(card, &t);
        let m = l.members.unwrap();
        assert_eq!(m.3, d.bh(card.3));
        assert!(inside(l.body, (m.0 + 20) as f32, (m.1 + 10) as f32));
        assert!(m.1 + m.3 <= l.body.1 + l.body.3, "the whole box is in the body's viewport");
    }

    #[test]
    fn a_new_group_opens_with_the_name_selected_and_the_window_ticked_or_one_new_window() {
        let (d, _) = dialog(3, &[11], &[]);
        assert_eq!((d.name(), d.members(), d.template()), ("axon", vec![11], Template::None));
        assert_eq!(d.name.selection(), Some((0, 4)), "the name is selected, to be typed over");
        assert_eq!(d.zone, Zone::Name);
        // No window to put in it: one new window is ticked, as the reference.
        let (d, _) = dialog(3, &[], &[]);
        assert_eq!((d.members(), d.template(), d.total()), (vec![], Template::One, 1));
        // An edit has none of that.
        let (e, _) = edit_dialog();
        assert_eq!((e.template(), e.color(), e.name()), (Template::None, 3, "docs"));
    }

    #[test]
    fn enter_with_an_empty_name_says_so_and_typing_takes_it_back() {
        let t = Text::load();
        let cx = Cx { g: G, text: &t };
        let (mut d, out) = dialog(3, &[10], &[]);
        d.key(&key(sym::BACKSPACE), &cx);
        assert_eq!(d.name(), "");
        assert_eq!(d.key(&key(sym::RETURN), &cx), Out::Redraw);
        assert_eq!(d.error(), Some(FormError::NameEmpty));
        assert!(out.borrow().is_none(), "nothing was submitted");
        assert!(d.natural_h() > 571, "the error line takes room");
        assert_eq!(d.key(&typed("a"), &cx), Out::Redraw);
        assert_eq!(d.error(), None, "the line goes when the name changes");
    }

    #[test]
    fn a_name_another_group_has_is_refused_in_any_case_and_an_empty_group_too() {
        let t = Text::load();
        let cx = Cx { g: G, text: &t };
        let (mut d, out) = dialog(3, &[10], &["AXON"]);
        assert_eq!(d.key(&key(sym::RETURN), &cx), Out::Redraw);
        assert_eq!(d.error(), Some(FormError::NameTaken));
        assert_eq!(d.zone, Zone::Name, "the name is where to fix it");
        // Nothing chosen and no template: the other error, and the name keeps the keyboard where it was.
        let (mut d, _) = dialog(3, &[10], &[]);
        d.key(&key(sym::TAB), &cx);
        d.key(&key(sym::TAB), &cx);
        assert_eq!(d.zone, Zone::Members);
        d.key(&key(sym::SPACE), &cx);
        assert!(d.members().is_empty());
        d.template = Template::None;
        assert_eq!(d.key(&key(sym::RETURN), &cx), Out::Redraw);
        assert_eq!(d.error(), Some(FormError::NothingChosen));
        assert_eq!(d.zone, Zone::Members);
        d.key(&key(sym::SPACE), &cx);
        assert_eq!(d.error(), None, "ticking a window takes the line back");
        assert!(out.borrow().is_none());
    }

    #[test]
    fn a_valid_form_is_left_in_the_shared_cell_and_runs_the_command() {
        let t = Text::load();
        let cx = Cx { g: G, text: &t };
        let (mut d, out) = dialog(4, &[10, 12], &["docs"]);
        // Colour: Tab, then Right twice; template: two Tabs on.
        d.key(&key(sym::TAB), &cx);
        d.key(&key(sym::RIGHT), &cx);
        d.key(&key(sym::RIGHT), &cx);
        d.key(&key(sym::TAB), &cx);
        d.key(&key(sym::TAB), &cx);
        assert_eq!(d.zone, Zone::Templates);
        d.key(&key(sym::RIGHT), &cx);
        d.key(&key(sym::RIGHT), &cx);
        assert_eq!(d.template(), Template::Dev);
        assert_eq!(d.total(), 2 + 3, "two windows ticked, three of the template");
        assert_eq!(d.key(&key(sym::RETURN), &cx), Out::Run(CMD_SUBMIT));
        assert_eq!(out.borrow().clone(), Some(Submit { edit: None, name: "axon".into(), color: 2, members: vec![10, 12], template: Template::Dev }));
    }

    #[test]
    fn tab_goes_round_the_zones_and_skips_the_windows_and_templates_when_editing() {
        let t = Text::load();
        let cx = Cx { g: G, text: &t };
        let (mut d, _) = dialog(3, &[10], &[]);
        let mut seen = vec![d.zone];
        for _ in 0..6 {
            d.key(&key(sym::TAB), &cx);
            seen.push(d.zone);
        }
        assert_eq!(seen, [Zone::Name, Zone::Colors, Zone::Members, Zone::Templates, Zone::Cancel, Zone::Create, Zone::Name]);
        d.key(&shift_tab(), &cx);
        assert_eq!(d.zone, Zone::Create, "Shift+Tab goes back");
        let (mut e, _) = edit_dialog();
        let mut seen = vec![e.zone];
        for _ in 0..4 {
            e.key(&key(sym::TAB), &cx);
            seen.push(e.zone);
        }
        assert_eq!(seen, [Zone::Name, Zone::Colors, Zone::Cancel, Zone::Create, Zone::Name]);
    }

    #[test]
    fn enter_on_cancel_closes_and_enter_elsewhere_submits_and_space_works_the_buttons_too() {
        let t = Text::load();
        let cx = Cx { g: G, text: &t };
        let (mut d, _) = dialog(2, &[10], &[]);
        d.zone = Zone::Cancel;
        assert_eq!(d.key(&key(sym::RETURN), &cx), Out::Close);
        assert_eq!(d.key(&key(sym::SPACE), &cx), Out::Close);
        d.zone = Zone::Create;
        assert_eq!(d.key(&key(sym::SPACE), &cx), Out::Run(CMD_SUBMIT));
        d.zone = Zone::Colors;
        assert_eq!(d.key(&key(sym::RETURN), &cx), Out::Run(CMD_SUBMIT));
    }

    #[test]
    fn the_colours_wrap_and_the_windows_list_keeps_its_cursor_inside_and_scrolls_with_it() {
        let t = Text::load();
        let cx = Cx { g: G, text: &t };
        let (mut d, _) = dialog(9, &[10], &[]);
        d.zone = Zone::Colors;
        d.key(&key(sym::LEFT), &cx);
        assert_eq!(d.color(), 7, "left of the first is the last");
        d.key(&key(sym::RIGHT), &cx);
        assert_eq!(d.color(), 0);
        d.zone = Zone::Members;
        assert!(d.overflows(MEMBERS_BOX), "nine rows do not fit 4 and a bit");
        d.key(&key(sym::UP), &cx);
        assert_eq!(d.cursor, 0, "no wrapping in the list");
        for _ in 0..20 {
            d.key(&key(sym::DOWN), &cx);
        }
        assert_eq!(d.cursor, 8);
        assert_eq!(d.list_scroll, d.list_max(MEMBERS_BOX), "the last row is in view");
        d.key(&key(sym::HOME), &cx);
        assert_eq!((d.cursor, d.list_scroll), (0, 0));
        d.key(&key(sym::END), &cx);
        assert_eq!(d.cursor, 8);
        // Space ticks the cursor's row and again unticks it.
        d.key(&key(sym::SPACE), &cx);
        assert!(d.members().contains(&18));
        d.key(&key(sym::SPACE), &cx);
        assert!(!d.members().contains(&18));
    }

    #[test]
    fn clicking_ticks_a_window_picks_a_colour_and_a_template_and_the_buttons_act_on_release() {
        let t = Text::load();
        let cx = Cx { g: G, text: &t };
        let (mut d, out) = dialog(4, &[10], &[]);
        let l = lay(&d, &t);
        // A window's row.
        let m = l.members.unwrap();
        let r = d.member_rect(m, 2);
        assert_eq!(d.button(click(r.0 + 40, r.1 + 5, true), &cx), Out::Redraw);
        assert!(d.members().contains(&12));
        assert_eq!(d.zone, Zone::Members);
        // A colour.
        let s = GroupDialog::swatch(&l, 4);
        d.button(click(s.0 + 5, s.1 + 5, true), &cx);
        assert_eq!(d.color(), 4);
        // A template.
        let tp = l.tpls[2];
        d.button(click(tp.0 + 5, tp.1 + 5, true), &cx);
        assert_eq!(d.template(), Template::Dev);
        // Cancel: the press only arms it; the release over it closes.
        let c = l.cancel;
        assert_eq!(d.button(click(c.0 + 5, c.1 + 5, true), &cx), Out::Redraw);
        assert_eq!(d.button(click(c.0 + 5, c.1 + 5, false), &cx), Out::Close);
        // Released somewhere else: nothing.
        d.button(click(c.0 + 5, c.1 + 5, true), &cx);
        assert_eq!(d.button(click(l.create.0 + 5, l.create.1 + 5, false), &cx), Out::None);
        // Create runs the submit, the same as Enter.
        let c = l.create;
        d.button(click(c.0 + 5, c.1 + 5, true), &cx);
        assert_eq!(d.button(click(c.0 + 5, c.1 + 5, false), &cx), Out::Run(CMD_SUBMIT));
        let s = out.borrow().clone().unwrap();
        assert_eq!((s.members, s.color, s.template), (vec![10, 12], 4, Template::Dev));
    }

    #[test]
    fn a_click_in_the_name_puts_the_caret_there_and_what_is_outside_the_card_is_not_the_dialogs() {
        let t = Text::load();
        let cx = Cx { g: G, text: &t };
        let (mut d, _) = dialog(2, &[10], &[]);
        let l = lay(&d, &t);
        d.zone = Zone::Colors;
        d.button(click(l.name.0 + 200, l.name.1 + 18, true), &cx);
        assert_eq!(d.zone, Zone::Name);
        assert_eq!(d.name.caret(), 4, "past the end of 'axon'");
        assert_eq!(d.name.selection(), None);
        d.button(click(l.name.0 + 200, l.name.1 + 18, false), &cx);
        assert!(!d.name.dragging());
        let card = d.frame().rect(G);
        assert!(d.contains((card.0 + 5) as f32, (card.1 + 5) as f32, &cx));
        assert!(!d.contains(5.0, 100.0, &cx), "the scrim");
        assert_eq!(d.cursor(l.name.0 as f32 + 5.0, l.name.1 as f32 + 5.0, &cx), Cursor::Text);
        let s = GroupDialog::swatch(&l, 0);
        assert_eq!(d.cursor((s.0 + 3) as f32, (s.1 + 3) as f32, &cx), Cursor::Pointer);
        assert_eq!(d.cursor((card.0 + 3) as f32, (card.1 + 3) as f32, &cx), Cursor::Default);
    }

    #[test]
    fn the_wheel_scrolls_the_windows_list_over_it_and_the_body_elsewhere() {
        let t = Text::load();
        let g = Geo { w: 1000, h: 480, square: false };
        let cx = Cx { g, text: &t };
        let (mut d, _) = dialog(9, &[10], &[]);
        let l = d.lay(d.frame().rect(g), &t);
        let m = l.members.unwrap();
        assert_eq!(d.scroll(48.0, (m.0 + 20) as f32, (m.1 + 20) as f32, &cx), Out::Redraw);
        assert_eq!((d.list_scroll, d.scroll), (48, 0));
        assert_eq!(d.scroll(10_000.0, (m.0 + 20) as f32, (m.1 + 20) as f32, &cx), Out::Redraw);
        assert_eq!(d.list_scroll, d.list_max(m.3), "clamped to the end");
        assert_eq!(d.scroll(10.0, (m.0 + 20) as f32, (m.1 + 20) as f32, &cx), Out::None);
        // Over the name: the body scrolls (the card is shorter than the window here).
        assert_eq!(d.scroll(30.0, (l.name.0 + 5) as f32, (l.name.1 + 5) as f32, &cx), Out::Redraw);
        assert_eq!(d.scroll, 30);
        assert_eq!(d.scroll(-100.0, (l.name.0 + 5) as f32, (l.name.1 + 5) as f32, &cx), Out::Redraw);
        assert_eq!(d.scroll, 0);
    }

    #[test]
    fn editing_a_group_keeps_the_members_out_and_submits_only_the_name_and_the_colour() {
        let t = Text::load();
        let cx = Cx { g: G, text: &t };
        let (mut e, out) = edit_dialog();
        e.name.set("api");
        assert_eq!(e.key(&key(sym::RETURN), &cx), Out::Redraw, "api is another group's");
        assert_eq!(e.error(), Some(FormError::NameTaken));
        e.name.set("docs 2");
        e.key(&typed("x"), &cx);
        assert_eq!(e.key(&key(sym::RETURN), &cx), Out::Run(CMD_SUBMIT));
        assert_eq!(out.borrow().clone(), Some(Submit { edit: Some(5), name: "docs 2x".into(), color: 3, members: vec![], template: Template::None }));
        // Its own name is not taken (the app leaves it out of `taken`).
        let (mut e, _) = edit_dialog();
        assert_eq!(e.key(&key(sym::RETURN), &cx), Out::Run(CMD_SUBMIT));
        assert!(e.lay(e.frame().rect(G), &t).members.is_none());
    }

    #[test]
    fn a_hint_is_broken_in_two_lines_at_a_space_and_never_overflows() {
        let w = |s: &str| s.chars().count() as i32 * 6;
        assert_eq!(wrap2("An empty terminal", 120, &w), ["An empty terminal", ""]);
        assert_eq!(wrap2("Only the chosen windows", 120, &w), ["Only the chosen", "windows"]);
        assert_eq!(wrap2("Solo las ventanas elegidas", 120, &w), ["Solo las ventanas", "elegidas"]);
        let [a, b] = wrap2("one two three four five six seven eight nine", 100, &w);
        assert!(w(&a) <= 100 && w(&b) <= 100, "{a:?} {b:?}");
        assert!(b.ends_with('…'));
        // A single long word has no space to break at: it is cut.
        let [a, b] = wrap2("supercalifragilisticexpialidocious", 60, &w);
        assert!(w(&a) <= 60 && b.is_empty(), "{a:?}");
    }

    #[test]
    fn the_dialog_paints_in_both_modes_and_fades_in_from_nothing() {
        let t = Text::load();
        let cx = Cx { g: G, text: &t };
        for (d, _) in [dialog(9, &[10], &[]), edit_dialog()] {
            let mut d = d;
            let mut fb = vec![0u32; 1000 * 640];
            let mut p = Paint { fb: &mut fb, w: 1000, h: 640, ox: 0, oy: 0, fade: 1.0 };
            d.draw(&mut p, &cx);
            assert!(fb.iter().all(|&px| px == 0), "the first frame is the invisible one");
            let t0 = Instant::now();
            assert!(d.step(t0));
            assert!(d.step(t0 + std::time::Duration::from_millis(60)));
            assert!(!d.step(t0 + std::time::Duration::from_millis(500)), "at rest nothing asks for a frame");
            let mut p = Paint { fb: &mut fb, w: 1000, h: 640, ox: 0, oy: 0, fade: 1.0 };
            d.draw(&mut p, &cx);
            assert!(fb.iter().filter(|&&px| px != 0).count() > 20_000);
        }
    }
}
