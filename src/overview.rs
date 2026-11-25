//! The overview of windows (`Alt+O`, [0028](../knowledge/decisions/workspace/0028-overview.md)), after the reference's
//! "Vista general de ventanas": over the workspace (the panel of windows beside it stays live), a header with the
//! count, a filter, two buttons and a cross; under it every window as a card, in a section per group (a tinted
//! box with the group's dot and name) and one for the windows in none; at the end a dashed tile that makes a group.
//! A card shows the last rows of the window's terminal, its status, name, panes, number and directory.
//!
//! An overlay of the kit (`overlay.rs`): a pure function of its state and the window size. The state is a
//! snapshot of the windows (`cards`, `groups`) that the app brings up to date every frame while it is open
//! (`Stack::with`), so nothing here is kept longer than a frame's worth of truth; what only the terminal has (the
//! rows of text, the grid's font) comes at draw time through `overlay::Grid`, read under the terminal's lock and
//! dropped at once. Layout is computed on demand from the width and the counts: no stored rectangles.
use std::{any::Any, borrow::Cow, cell::RefCell, fmt::Write, time::Instant};

use crate::{
    chrome::{self, rounded_rect},
    field::{self, Edit, Field, H_SEARCH, Look},
    i18n::{Str, tr},
    icons::{self, Icon},
    input::sym,
    list::clipped,
    overlay::{self, Cap, Click, Clicks, Cx, Cursor, Fade, Geo, Grid, Ink, Out, Overlay, PX, PX_HINT, PX_KBD, PX_SMALL, PX_TITLE, Rect, Text, accent, dim, faint, fg, fill, inside, kbd, kbd_width},
    paint::{Paint, WHITE},
    palette,
    side::{Status, group_color},
    plat::Key,
};

/// The tag the app opens the overview under.
pub const TAG: u32 = 0x4f56_5700;

// ---------------------------------------------------------------- measures --
// All from the reference's CSS (`.ov-*`, `.card`), in pixels.

/// The header: 14 above the 30 px filter, 12 below and a hairline.
pub const HEAD_H: i32 = 14 + H_SEARCH + 12 + 1;
/// The sides of the header and of the body.
const PAD_X: i32 = 20;
/// Above the first section and below the last thing.
const BODY_TOP: i32 = 16;
const BODY_BOTTOM: i32 = 24;
/// The filter's width, a button's height, and the gap between the header's parts.
const SEARCH_W: i32 = 220;
const BTN_H: i32 = 28;
const HEAD_GAP: i32 = 10;
/// The grid of cards: gap, least column width.
const GAP: i32 = 12;
const MIN_COL: i32 = 210;
/// A section: border 1, padding 10 around, a 24 px head, 10 under it, 12 below the grid; 18 between sections.
const SEC_PAD: i32 = 10;
const SEC_HEAD: i32 = 24;
const SEC_HEAD_GAP: i32 = 10;
const SEC_FIXED: i32 = 1 + SEC_PAD + SEC_HEAD + SEC_HEAD_GAP + 12 + 1;
const SEC_GAP: i32 = 18;
/// A card: the preview, the foot (hairline, title row) and the directory line, inside a 1 px border.
pub const CARD_H: i32 = 1 + PREV_H + FOOT_H + META_H + 1;
const PREV_H: i32 = 118;
const FOOT_H: i32 = 32;
const META_H: i32 = 26;
/// The dashed tile that makes a group.
const TILE_H: i32 = 88;
/// The box that says a section has no window (`.ov-empty`: 6 px of margin, 18 of padding round a line of text, a 1 px border).
const EMPTY_BOX: i32 = 1 + 18 + 20 + 18 + 1;
const EMPTY_H: i32 = 6 + EMPTY_BOX + 6;
/// The preview's text is the grid's font at this fraction of its size (10.5 px of the reference's 14) and its
/// padding; the lines fade out towards the top (`mask-image`: transparent to opaque over the first 38 %).
pub const PREV_SCALE: f32 = 0.75;
const PREV_PAD_X: i32 = 12;
const PREV_PAD_Y: i32 = 10;
const PREV_FADE: f32 = 0.38;

/// The card's body (`#101012`) and the preview's text (`#b4b4bb`).
const CARD_BG: [f32; 3] = [0.0627, 0.0627, 0.0706];
const PREV_INK: [f32; 3] = [0.706, 0.706, 0.733];

// ---------------------------------------------------------------- the data --

/// A window as a card shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Card {
    /// The window's number (`Win::id`), what every command says it by.
    pub id: u32,
    /// The group it is in (an index into `Overview::groups`).
    pub group: Option<u8>,
    pub title: String,
    /// Its directory and, when known, the branch: `~/lab/axon · main`.
    pub meta: String,
    pub status: Status,
    /// It printed while out of sight.
    pub unread: bool,
    pub panes: u16,
    /// The window on screen.
    pub current: bool,
    /// The filter is found in the text of its terminal (the app looks; the card's own words are looked at here).
    pub text_hit: bool,
}

/// A group as a section shows it: the colour is one of `side::group_color`'s eight.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Group {
    pub name: String,
    pub color: u8,
}

/// What the overview asks of the app (`Ran::id`): a window or group number in the low 24 bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmd {
    /// Go to the window and close the overview.
    Go(u32),
    /// Close the window.
    Close(u32),
    /// A new window that does not take the screen.
    NewWindow,
    /// A new group, with this window ticked in its dialog (`None`: none, and the dialog starts with the template "one window").
    NewGroup(Option<u32>),
    /// A new window in the group.
    AddTo(u8),
    /// The dialog that edits the group (`data-act="edit"` of the section's chip).
    EditGroup(u8),
    /// A card was dropped on a section: the window and the group are in `Overview::dropped`.
    Move,
}

const MASK: u32 = 0x00ff_ffff;

impl Cmd {
    pub fn encode(self) -> u32 {
        match self {
            Cmd::Go(id) => 1 << 24 | id & MASK,
            Cmd::Close(id) => 2 << 24 | id & MASK,
            Cmd::NewWindow => 3 << 24,
            Cmd::NewGroup(id) => 4 << 24 | id.map_or(MASK, |i| i & MASK),
            Cmd::AddTo(g) => 5 << 24 | g as u32,
            Cmd::EditGroup(g) => 6 << 24 | g as u32,
            Cmd::Move => 7 << 24,
        }
    }

    pub fn decode(n: u32) -> Option<Cmd> {
        let arg = n & MASK;
        Some(match n >> 24 {
            1 => Cmd::Go(arg),
            2 => Cmd::Close(arg),
            3 => Cmd::NewWindow,
            4 => Cmd::NewGroup((arg != MASK).then_some(arg)),
            5 => Cmd::AddTo(arg as u8),
            6 => Cmd::EditGroup(arg as u8),
            7 => Cmd::Move,
            _ => return None,
        })
    }
}

// ----------------------------------------------------------- pure geometry --

/// How many columns of cards a grid `w` wide has (`repeat(auto-fill, minmax(210px, 1fr))`).
pub fn columns(w: i32) -> i32 {
    ((w + GAP) / (MIN_COL + GAP)).max(1)
}

/// How wide each of `cols` columns is in a grid `w` wide.
pub fn column_width(w: i32, cols: i32) -> i32 {
    ((w - GAP * (cols - 1)) / cols.max(1)).max(0)
}

/// The room a section's grid has in an area `w` wide: the body's padding, the section's border and padding.
pub fn grid_width(w: i32) -> i32 {
    w - 2 * PAD_X - 2 - 2 * SEC_PAD
}

/// Where the arrow keys move the selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
    First,
    Last,
}

/// The card the selection moves to from flat position `at` of the shown cards, which sit in sections of `counts`
/// cards (in order, `cols` to a row). Left and right walk the cards in reading order; up and down keep the column
/// (or the last card of a shorter row) and go on into the neighbouring section.
pub fn step(counts: &[usize], cols: usize, at: usize, d: Dir) -> usize {
    let total: usize = counts.iter().sum();
    if total == 0 {
        return 0;
    }
    let at = at.min(total - 1);
    let (mut s, mut base) = (0, 0);
    while at >= base + counts[s] {
        base += counts[s];
        s += 1;
    }
    let (cols, j) = (cols.max(1), at - base);
    let (row, col) = (j / cols, j % cols);
    let first_of = |from: usize| -> usize { counts[..from].iter().sum() };
    match d {
        Dir::First => 0,
        Dir::Last => total - 1,
        Dir::Left => at.saturating_sub(1),
        Dir::Right => (at + 1).min(total - 1),
        Dir::Up => {
            if row > 0 {
                return at - cols;
            }
            match (0..s).rev().find(|&k| counts[k] > 0) {
                Some(k) => first_of(k) + (((counts[k] - 1) / cols) * cols + col).min(counts[k] - 1),
                None => at,
            }
        }
        Dir::Down => {
            if row < (counts[s] - 1) / cols {
                return base + (j + cols).min(counts[s] - 1);
            }
            match (s + 1..counts.len()).find(|&k| counts[k] > 0) {
                Some(k) => first_of(k) + col.min(counts[k] - 1),
                None => at,
            }
        }
    }
}

/// The scrollbar's thumb, `(top, height)` from the viewport's top, for `content` pixels of content in a viewport
/// of `view` pixels scrolled `scroll` (as `List::thumb_in`).
pub fn thumb(content: i32, view: i32, scroll: i32) -> Option<(i32, i32)> {
    if content <= view || view <= 0 {
        return None;
    }
    let h = (view as i64 * view as i64 / content as i64).clamp(20, view as i64) as i32;
    let scroll = scroll.clamp(0, content - view);
    Some(((scroll as i64 * (view - h) as i64 / (content - view) as i64) as i32, h))
}

/// Whether `hay` has `needle` in it, ignoring case; `needle` is already lower case. No allocation.
pub fn contains_ci(hay: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    hay.char_indices().any(|(i, _)| {
        let mut h = hay[i..].chars().flat_map(char::to_lowercase);
        needle.chars().all(|n| h.next() == Some(n))
    })
}

/// The parts of the header, laid out for an area: the count's x (when it fits), the filter, the two buttons (when
/// they fit; `labels`: with their words, else as 28 px squares) and the cross. What does not fit is dropped in
/// the order the reference would have wrapped it: the count, the words, the buttons; the filter shrinks last.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Head {
    pub count: Option<i32>,
    pub search: Rect,
    pub group: Option<Rect>,
    pub window: Option<Rect>,
    pub close: Rect,
    pub labels: bool,
}

/// The width of a ghost button with its words (`2` borders, `9` + 14 px icon + `6` before, `11` after).
pub fn ghost_width(label_w: i32) -> i32 {
    2 + 9 + 14 + 6 + label_w + 11
}

pub fn head((ax, ay, aw, _): Rect, title_w: i32, count_w: i32, group_w: i32, window_w: i32) -> Head {
    let close = (ax + aw - PAD_X - BTN_H, ay + 15, BTN_H, BTN_H);
    let end = close.0 - HEAD_GAP;
    let title_end = ax + PAD_X + title_w;
    for (with_count, labels, buttons) in [(true, true, true), (false, true, true), (false, false, true), (false, false, false)] {
        let (gw, ww) = if labels { (ghost_width(group_w), ghost_width(window_w)) } else { (BTN_H, BTN_H) };
        let search_x = title_end + HEAD_GAP + if with_count { count_w + HEAD_GAP } else { 0 };
        let need = SEARCH_W + if buttons { HEAD_GAP + gw + HEAD_GAP + ww } else { 0 };
        if search_x + need <= end {
            let window = (end - ww, ay + 15, ww, BTN_H);
            let group = (window.0 - HEAD_GAP - gw, ay + 15, gw, BTN_H);
            return Head {
                count: with_count.then_some(title_end + HEAD_GAP),
                search: (search_x, ay + 14, SEARCH_W, H_SEARCH),
                group: buttons.then_some(group),
                window: buttons.then_some(window),
                close,
                labels,
            };
        }
    }
    let x = title_end + HEAD_GAP;
    Head { count: None, search: (x, ay + 14, (end - x).max(40), H_SEARCH), group: None, window: None, close, labels: false }
}

// ------------------------------------------------------------- the overlay --

/// What the pointer is over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hit {
    Field,
    NewGroup,
    NewWindow,
    Close,
    /// A card (by window number), or its cross.
    Card(u32),
    CardClose(u32),
    /// A group's "new window" button, and its chip (the group's dot and name: it edits the group).
    Add(usize),
    Chip(usize),
    Tile,
}

/// What `Overview::layout` walks through, in content coordinates (`y` from the top of the scrolled body).
enum Item<'a> {
    Sec(&'a Sec, Rect),
    Card(usize, Rect),
    Tile(Rect),
    /// The dashed box of a section with no window in it.
    Empty(Rect),
    Nothing(i32),
}

struct Sec {
    group: Option<usize>,
}

/// How far the pointer goes with a card pressed before it is a drag and not a click (the reference's `dragstart`).
const DRAG_PX: f32 = 5.0;
/// How opaque the card being dragged is (`.card.dragging`).
const DRAGGED: f32 = 0.4;

/// A card with the button down: which, where the pointer went down, and whether it has gone far enough to be a drag.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Drag {
    id: u32,
    from: (f32, f32),
    moved: bool,
}

/// Where dropping a card would put it: into a section (a group, or `None` for the windows in none), or on the tile that
/// makes a group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Drop {
    Section(Option<usize>),
    Tile,
}

pub struct Overview {
    fade: Fade,
    pub cards: Vec<Card>,
    pub groups: Vec<Group>,
    /// The workspace in window pixels: where the overview is (the app says it every frame).
    pub area: Rect,
    field: Field,
    /// The filter, trimmed and in lower case.
    pub lower: String,
    /// Scratch for the app: the text of a pane while it looks for the filter in it.
    pub text: String,
    sel: Option<u32>,
    scroll: i32,
    over: Option<Hit>,
    armed: Option<Hit>,
    drag: Option<Drag>,
    drop: Option<Drop>,
    /// What a `Cmd::Move` asks for: the window (its number) and the group (an index into `groups`, `None` for none).
    pub dropped: Option<(u32, Option<usize>)>,
    clicks: Clicks,
    scratch: RefCell<String>,
    rows: RefCell<String>,
}

impl Overview {
    pub fn new(cards: Vec<Card>, groups: Vec<Group>, area: Rect) -> Overview {
        let mut o = Overview {
            fade: Fade::rising(0.14),
            cards,
            groups,
            area,
            field: Field::new(40, tr(Str::OvFieldSearch)),
            lower: String::new(),
            text: String::new(),
            sel: None,
            scroll: 0,
            over: None,
            armed: None,
            drag: None,
            drop: None,
            dropped: None,
            clicks: Clicks::default(),
            scratch: RefCell::default(),
            rows: RefCell::default(),
        };
        o.settle();
        o
    }

    /// Already faded in (the design board's frames).
    #[cfg(test)]
    pub fn shown(mut self) -> Overview {
        self.fade = Fade::full();
        self
    }

    /// With a filter typed, a card selected (by number), the pointer over a card and the body scrolled (the
    /// design board's frames).
    #[cfg(test)]
    pub fn state(mut self, filter: &str, sel: Option<u32>, over: Option<u32>, scroll: i32) -> Overview {
        self.field.set(filter);
        self.lower = filter.trim().to_lowercase();
        self.sel = sel.or(self.sel);
        self.over = over.map(Hit::Card);
        // `i32::MIN`: scrolled to the selected card; a large number: to the end.
        self.scroll = if scroll == i32::MIN { 0 } else { scroll };
        if scroll == i32::MIN
            && let Some(ci) = self.sel_card()
        {
            self.reveal(ci);
        }
        self.settle();
        self
    }

    /// The pointer on a card's cross.
    #[cfg(test)]
    pub fn over_close(mut self, id: u32) -> Overview {
        self.over = Some(Hit::CardClose(id));
        self
    }

    /// Card `id` being carried over a group's section (`Some(Some(g))`), the windows in none (`Some(None)`) or the tile (`None`) (the
    /// design board's frames).
    #[cfg(test)]
    pub fn carrying(mut self, id: u32, over: Option<Option<usize>>) -> Overview {
        self.drag = Some(Drag { id, from: (0.0, 0.0), moved: true });
        self.drop = Some(over.map_or(Drop::Tile, Drop::Section));
        self
    }

    /// The pointer on one of the header's buttons or the tile (0 new group, 1 new window, 2 the cross, 3 the tile).
    #[cfg(test)]
    pub fn over_button(mut self, n: usize) -> Overview {
        self.over = Some([Hit::NewGroup, Hit::NewWindow, Hit::Close, Hit::Tile][n]);
        self
    }

    /// The selection is on a card that is there; with none chosen, on the window on screen. The scroll stays in
    /// range. Called after the cards change.
    pub fn settle(&mut self) {
        if self.sel.is_none_or(|id| !self.cards.iter().any(|c| c.id == id)) {
            self.sel = self.cards.iter().find(|c| c.current).or(self.cards.first()).map(|c| c.id);
        }
        let max = self.max_scroll();
        self.scroll = self.scroll.clamp(0, max);
    }

    /// The window the selection is on.
    #[cfg(test)]
    pub fn selected_id(&self) -> Option<u32> {
        self.sel_card().map(|ci| self.cards[ci].id)
    }

    // ------------------------------------------------------------ the filter --

    /// Whether the filter lets card `c` show: its words, its group's name, or its terminal's text.
    fn passes(&self, c: &Card) -> bool {
        let q = self.lower.as_str();
        q.is_empty()
            || c.text_hit
            || contains_ci(&c.title, q)
            || contains_ci(&c.meta, q)
            || c.group.and_then(|g| self.groups.get(g as usize)).is_some_and(|g| contains_ci(&g.name, q))
    }

    /// The shown cards of section `s` (`groups.len()` is the windows in no group), with their index.
    fn in_section(&self, s: usize) -> impl Iterator<Item = (usize, &Card)> + '_ {
        let group = (s < self.groups.len()).then_some(s);
        self.cards.iter().enumerate().filter(move |(_, c)| c.group.map(usize::from) == group && self.passes(c))
    }

    /// The shown cards in the order the sections list them.
    fn order(&self) -> Vec<usize> {
        (0..=self.groups.len()).flat_map(|s| self.in_section(s).map(|(i, _)| i)).collect()
    }

    fn counts(&self) -> Vec<usize> {
        (0..=self.groups.len()).map(|s| self.in_section(s).count()).collect()
    }

    /// The selected card's index, when it is shown.
    fn sel_card(&self) -> Option<usize> {
        let id = self.sel?;
        self.cards.iter().position(|c| c.id == id).filter(|&i| self.passes(&self.cards[i]))
    }

    // ---------------------------------------------------------------- layout --

    fn grid(&self) -> (i32, i32) {
        let w = grid_width(self.area.2);
        let cols = columns(w);
        (cols, column_width(w, cols))
    }

    fn view_h(&self) -> i32 {
        (self.area.3 - HEAD_H).max(0)
    }

    /// Walks the body top to bottom in content coordinates (the body's top is 0 and `scroll` is not applied):
    /// each section with its box, each card in it with its box, the tile or the message that ends it. Returns the
    /// content's height.
    fn layout(&self, mut f: impl FnMut(Item)) -> i32 {
        let (cols, cw) = self.grid();
        let (x0, sw) = (self.area.0 + PAD_X, self.area.2 - 2 * PAD_X);
        let mut y = BODY_TOP;
        for s in 0..=self.groups.len() {
            let n = self.in_section(s).count();
            // With groups and nothing in none, "No group" is still there while nothing is filtered (an empty one says so).
            let empty_loose = n == 0 && s == self.groups.len() && s > 0 && self.lower.is_empty();
            if n == 0 && !empty_loose {
                continue;
            }
            let rows = (n as i32 + cols - 1) / cols;
            let h = SEC_FIXED + if n == 0 { EMPTY_H } else { rows * CARD_H + (rows - 1) * GAP };
            let sec = Sec { group: (s < self.groups.len()).then_some(s) };
            f(Item::Sec(&sec, (x0, y, sw, h)));
            if empty_loose {
                f(Item::Empty((x0 + 1 + SEC_PAD, y + 1 + SEC_PAD + SEC_HEAD + SEC_HEAD_GAP + 6, sw - 2 - 2 * SEC_PAD, EMPTY_BOX)));
            }
            for (j, (ci, _)) in self.in_section(s).enumerate() {
                let (col, row) = (j as i32 % cols, j as i32 / cols);
                let r = (x0 + 1 + SEC_PAD + col * (cw + GAP), y + 1 + SEC_PAD + SEC_HEAD + SEC_HEAD_GAP + row * (CARD_H + GAP), cw, CARD_H);
                f(Item::Card(ci, r));
            }
            y += h + SEC_GAP;
        }
        if self.lower.is_empty() {
            f(Item::Tile((x0, y, sw, TILE_H)));
            y += TILE_H;
        } else if y == BODY_TOP {
            f(Item::Nothing(y + 40));
            y += 40 + 18 + 40;
        }
        y + BODY_BOTTOM
    }

    fn max_scroll(&self) -> i32 {
        (self.layout(|_| {}) - self.view_h()).max(0)
    }

    /// Scrolls so that card `ci` (and its section's head when it is in the first row) is in view.
    fn reveal(&mut self, ci: usize) {
        let mut at = None;
        let mut sec_top = 0;
        self.layout(|it| match it {
            Item::Sec(_, r) => sec_top = r.1,
            Item::Card(i, r) if i == ci => at = Some((if r.1 == sec_top + 1 + SEC_PAD + SEC_HEAD + SEC_HEAD_GAP { sec_top } else { r.1 }, r.1 + r.3)),
            _ => {}
        });
        if let Some((top, bottom)) = at {
            let view = self.view_h();
            if top - 8 < self.scroll {
                self.scroll = (top - 8).max(0);
            } else if bottom + 8 > self.scroll + view {
                self.scroll = bottom + 8 - view;
            }
            self.scroll = self.scroll.clamp(0, self.max_scroll());
        }
    }

    fn head(&self, t: &Text) -> Head {
        let mut buf = self.scratch.borrow_mut();
        buf.clear();
        self.count_text(&mut buf);
        head(self.area, t.width(tr(Str::Windows), PX_TITLE) + 2, t.width(&buf, PX_HINT), t.width(tr(Str::OvBtnNewGroup), PX), t.width(tr(Str::OvActNewWindow), PX))
    }

    /// `6 windows · 2 groups`: panes count as windows, as in the reference.
    fn count_text(&self, out: &mut String) {
        let n: usize = self.cards.iter().map(|c| c.panes.max(1) as usize).sum();
        let g = self.groups.len();
        let _ = write!(out, "{n} {} \u{b7} {g} {}", if n == 1 { tr(Str::OvWindow1) } else { tr(Str::OvWindowsN) }, if g == 1 { tr(Str::OvGroup1) } else { tr(Str::OvGroupsN) });
    }

    /// How many windows a section lists (its panes count), whatever the filter.
    fn section_count(&self, group: Option<usize>) -> usize {
        self.cards.iter().filter(|c| c.group.map(usize::from) == group).map(|c| c.panes.max(1) as usize).sum()
    }

    /// The box of a card's cross and of a group's add button, in content coordinates.
    fn close_box((x, y, w, _): Rect) -> Rect {
        (x + w - 1 - 10 - 18, y + 1 + PREV_H + 1 + 9 + 9 - 9, 18, 18)
    }

    fn add_box(&self, t: &Text, (x, y, w, _): Rect) -> Rect {
        let bw = 2 + 7 + 14 + 6 + t.width(tr(Str::OvBtnAdd), PX_HINT) + 9;
        (x + w - 1 - SEC_PAD - bw, y + 1 + SEC_PAD, bw, SEC_HEAD)
    }

    /// A group's chip in the head of its section `r`: 9 px, the dot (8), 6 px, the name (cut to what the add button leaves)
    /// and 9 px, 24 px tall; and the name as it is drawn.
    fn chip<'a>(&self, t: &Text, r: Rect, g: &'a Group) -> (Rect, Cow<'a, str>) {
        let (x, y, w, _) = r;
        let at = x + 1 + SEC_PAD;
        let limit = self.add_box(t, r).0 - 10;
        let name = t.fit(&g.name, PX, (limit - at - 9 - 8 - 6 - 9).clamp(24, w / 2));
        ((at, y + 1 + SEC_PAD, 9 + 8 + 6 + t.width(&name, PX) + 9, SEC_HEAD), name)
    }

    /// What is under window pixel `(x, y)`.
    fn hit_at(&self, t: &Text, x: f32, y: f32) -> Option<Hit> {
        if !inside(self.area, x, y) {
            return None;
        }
        let ay = self.area.1;
        if y < (ay + HEAD_H) as f32 {
            let h = self.head(t);
            return if inside(h.close, x, y) {
                Some(Hit::Close)
            } else if h.group.is_some_and(|r| inside(r, x, y)) {
                Some(Hit::NewGroup)
            } else if h.window.is_some_and(|r| inside(r, x, y)) {
                Some(Hit::NewWindow)
            } else if inside(h.search, x, y) {
                Some(Hit::Field)
            } else {
                None
            };
        }
        let yc = y - (ay + HEAD_H - self.scroll) as f32;
        let (mut hit, many) = (None, self.cards.len() > 1);
        self.layout(|it| match it {
            Item::Card(ci, r) if inside(r, x, yc) => {
                let id = self.cards[ci].id;
                hit = Some(if many && inside(Self::close_box(r), x, yc) { Hit::CardClose(id) } else { Hit::Card(id) });
            }
            Item::Sec(s, r) => {
                if let Some(g) = s.group.filter(|_| inside(self.add_box(t, r), x, yc)) {
                    hit = Some(Hit::Add(g));
                } else if let Some((g, group)) = s.group.and_then(|g| Some((g, self.groups.get(g)?)))
                    && inside(self.chip(t, r, group).0, x, yc)
                {
                    hit = Some(Hit::Chip(g));
                }
            }
            Item::Tile(r) if inside(r, x, yc) => hit = Some(Hit::Tile),
            _ => {}
        });
        hit
    }

    /// Where a drop at window pixel `(x, y)` would go: the section or the tile under it.
    fn drop_at(&self, x: f32, y: f32) -> Option<Drop> {
        if !inside(self.area, x, y) || y < (self.area.1 + HEAD_H) as f32 {
            return None;
        }
        let yc = y - (self.area.1 + HEAD_H - self.scroll) as f32;
        let mut to = None;
        self.layout(|it| match it {
            Item::Sec(s, r) if inside(r, x, yc) => to = Some(Drop::Section(s.group)),
            Item::Tile(r) if inside(r, x, yc) => to = Some(Drop::Tile),
            _ => {}
        });
        to
    }

    /// Whether card `id` is the one being dragged (`moved` is the drag that is not a click any more).
    fn dragged(&self, id: u32) -> bool {
        self.drag.is_some_and(|d| d.moved && d.id == id)
    }

    fn lift(&self, id: u32) -> i32 {
        if matches!(self.over, Some(Hit::Card(i) | Hit::CardClose(i)) if i == id) { -1 } else { 0 }
    }

    /// Moves the selection, keeping it in view.
    fn go(&mut self, d: Dir) -> Out {
        let order = self.order();
        if order.is_empty() {
            return Out::None;
        }
        let at = self.sel_card().and_then(|ci| order.iter().position(|&o| o == ci));
        let to = match at {
            None => if matches!(d, Dir::Up | Dir::Left | Dir::Last) { order.len() - 1 } else { 0 },
            Some(a) => step(&self.counts(), self.grid().0 as usize, a, d),
        };
        let ci = order[to];
        self.sel = Some(self.cards[ci].id);
        self.reveal(ci);
        Out::Redraw
    }

    /// The card Enter opens: the selected one, else the first shown.
    fn chosen(&self) -> Option<u32> {
        self.sel_card().or_else(|| self.order().first().copied()).map(|ci| self.cards[ci].id)
    }
}

impl Overlay for Overview {
    fn key(&mut self, k: &Key, _: &Cx) -> Out {
        // A card being dragged: Esc puts it back, and nothing else is heard until it is dropped.
        if self.drag.is_some_and(|d| d.moved) {
            if k.sym == sym::ESCAPE {
                (self.drag, self.drop, self.armed) = (None, None, None);
                return Out::Redraw;
            }
            return Out::None;
        }
        let m = k.mods;
        if m.alt && !m.ctrl {
            let letter = char::from_u32(k.sym).map(|c| c.to_ascii_lowercase());
            return match (letter, k.sym) {
                (Some('o'), _) => Out::Close,
                (Some('n'), _) => Out::Act(Cmd::NewWindow.encode()),
                (_, d @ 0x31..=0x39) => self.cards.get((d - 0x31) as usize).map_or(Out::None, |c| Out::Run(Cmd::Go(c.id).encode())),
                _ => Out::None,
            };
        }
        let editing = !self.field.is_empty();
        match k.sym {
            sym::UP | sym::KP_UP => return self.go(Dir::Up),
            sym::DOWN | sym::KP_DOWN => return self.go(Dir::Down),
            sym::LEFT | sym::KP_LEFT if !editing && !m.ctrl && !m.shift => return self.go(Dir::Left),
            sym::RIGHT | sym::KP_RIGHT if !editing && !m.ctrl && !m.shift => return self.go(Dir::Right),
            sym::HOME | sym::KP_HOME if m.ctrl => return self.go(Dir::First),
            sym::END | sym::KP_END if m.ctrl => return self.go(Dir::Last),
            sym::TAB | sym::ISO_LEFT_TAB => {
                let back = m.shift || k.sym == sym::ISO_LEFT_TAB;
                let order = self.order();
                let at = self.sel_card().and_then(|ci| order.iter().position(|&o| o == ci));
                return self.go(match (back, at) {
                    (false, Some(a)) if a + 1 == order.len() => Dir::First,
                    (true, Some(0)) => Dir::Last,
                    (false, _) => Dir::Right,
                    (true, _) => Dir::Left,
                });
            }
            sym::PAGE_UP | sym::KP_PAGE_UP | sym::PAGE_DOWN | sym::KP_PAGE_DOWN => {
                let page = (self.view_h() - 40).max(40);
                let down = matches!(k.sym, sym::PAGE_DOWN | sym::KP_PAGE_DOWN);
                let was = self.scroll;
                self.scroll = (self.scroll + if down { page } else { -page }).clamp(0, self.max_scroll());
                return if was != self.scroll { Out::Redraw } else { Out::None };
            }
            sym::RETURN | sym::KP_ENTER => return self.chosen().map_or(Out::None, |id| Out::Run(Cmd::Go(id).encode())),
            _ => {}
        }
        match self.field.key(k) {
            Edit::Ignored | Edit::Same => Out::None,
            Edit::Moved => Out::Redraw,
            Edit::Changed => {
                self.lower = self.field.text().trim().to_lowercase();
                self.settle();
                Out::Redraw
            }
            Edit::Paste => Out::Paste,
            Edit::Copy(t) => Out::Copy(t),
            Edit::Cut(t) => {
                self.lower = self.field.text().trim().to_lowercase();
                self.settle();
                Out::Copy(t)
            }
        }
    }

    fn paste(&mut self, text: &str, _: &Cx) -> Out {
        if self.field.insert(&text.replace(['\n', '\r'], " ")) {
            self.lower = self.field.text().trim().to_lowercase();
            self.settle();
            Out::Redraw
        } else {
            Out::None
        }
    }

    fn pointer(&mut self, x: f32, y: f32, cx: &Cx) -> Out {
        if self.field.dragging() {
            let r = self.head(cx.text).search;
            return if self.field.drag_at(x, r, Look::Search, cx.text) { Out::Redraw } else { Out::None };
        }
        if let Some(d) = self.drag.as_mut() {
            d.moved |= (x - d.from.0).hypot(y - d.from.1) >= DRAG_PX;
            if d.moved {
                // The pointer lights nothing while a card is carried: the sections it could go into say so.
                let (was, over) = (self.drop, self.over.take());
                self.drop = self.drop_at(x, y);
                return if was != self.drop || over.is_some() { Out::Redraw } else { Out::None };
            }
        }
        let over = self.hit_at(cx.text, x, y);
        if std::mem::replace(&mut self.over, over) != over { Out::Redraw } else { Out::None }
    }

    fn button(&mut self, c: Click, cx: &Cx) -> Out {
        let hit = self.hit_at(cx.text, c.x, c.y);
        if c.pressed {
            self.armed = hit;
            return match hit {
                Some(Hit::Field) => {
                    let r = self.head(cx.text).search;
                    let clicks = self.clicks.press(c.time);
                    self.field.press_at(c.x, r, Look::Search, cx.text, clicks, false);
                    Out::Redraw
                }
                Some(Hit::Card(id) | Hit::CardClose(id)) => {
                    self.sel = Some(id);
                    // A card can be carried to a section (a cross cannot).
                    self.drag = matches!(hit, Some(Hit::Card(_))).then_some(Drag { id, from: (c.x, c.y), moved: false });
                    Out::Redraw
                }
                Some(_) => Out::Redraw,
                None => Out::None,
            };
        }
        self.field.release();
        let armed = self.armed.take();
        // A card carried and let go: into the section or onto the tile under the pointer, or nowhere.
        if let Some(d) = self.drag.take().filter(|d| d.moved) {
            let to = self.drop.take().or_else(|| self.drop_at(c.x, c.y));
            return match to {
                Some(Drop::Section(g)) => {
                    self.dropped = Some((d.id, g));
                    Out::Act(Cmd::Move.encode())
                }
                Some(Drop::Tile) => Out::Act(Cmd::NewGroup(Some(d.id)).encode()),
                None => Out::Redraw,
            };
        }
        self.drag = None;
        let Some(hit) = hit.filter(|&h| armed == Some(h)) else { return Out::Redraw };
        match hit {
            Hit::Card(id) => Out::Run(Cmd::Go(id).encode()),
            Hit::CardClose(id) => Out::Act(Cmd::Close(id).encode()),
            // The dialog opens empty (`openGroupDialog({})`): no window ticked, so it starts with the template "one window".
            Hit::NewGroup | Hit::Tile => Out::Act(Cmd::NewGroup(None).encode()),
            Hit::NewWindow => Out::Act(Cmd::NewWindow.encode()),
            Hit::Close => Out::Close,
            Hit::Add(g) => Out::Act(Cmd::AddTo(g as u8).encode()),
            Hit::Chip(g) => Out::Act(Cmd::EditGroup(g as u8).encode()),
            Hit::Field => Out::Redraw,
        }
    }

    fn scroll(&mut self, dy: f32, _: f32, _: f32, _: &Cx) -> Out {
        let was = self.scroll;
        self.scroll = (self.scroll + dy.round() as i32).clamp(0, self.max_scroll());
        if was != self.scroll { Out::Redraw } else { Out::None }
    }

    fn contains(&self, x: f32, y: f32, _: &Cx) -> bool {
        inside(self.area, x, y)
    }

    fn cursor(&self, x: f32, y: f32, cx: &Cx) -> Cursor {
        match self.hit_at(cx.text, x, y) {
            Some(Hit::Field) => Cursor::Text,
            Some(_) => Cursor::Pointer,
            None => Cursor::Default,
        }
    }

    fn step(&mut self, now: Instant) -> bool {
        self.fade.step(now)
    }

    fn handles_escape(&self) -> bool {
        !self.field.is_empty() || self.drag.is_some_and(|d| d.moved)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }

    fn draw(&self, p: &mut Paint, cx: &Cx) {
        let t = cx.text;
        p.fade = self.fade.value();
        let (ax, ay, aw, ah) = self.area;
        if aw < 120 || ah < HEAD_H + 60 {
            return;
        }
        backdrop(p, self.area, cx.g);
        // The header.
        let h = self.head(t);
        let cy = ay + 14 + H_SEARCH / 2;
        t.draw(p, ax + PAD_X, cy, tr(Str::Windows), Ink::new(PX_TITLE, fg()).bold());
        if let Some(x) = h.count {
            let mut buf = self.scratch.borrow_mut();
            buf.clear();
            self.count_text(&mut buf);
            t.draw(p, x, cy + 1, &buf, Ink::new(PX_HINT, dim()));
        }
        field::paint(p, t, &self.field, h.search, Look::Search, true);
        let lit = |hit| self.over == Some(hit);
        if let (Some(g), Some(w)) = (h.group, h.window) {
            ghost(p, t, g, Icon::Group, h.labels.then(|| tr(Str::OvBtnNewGroup)), PX, lit(Hit::NewGroup), 8.0);
            ghost(p, t, w, Icon::Plus, h.labels.then(|| tr(Str::OvActNewWindow)), PX, lit(Hit::NewWindow), 8.0);
        }
        let c = h.close;
        if lit(Hit::Close) {
            p.rrect(c.0, c.1, c.2, c.3, 8.0, WHITE, 0.07);
        }
        icons::paint(p, Icon::Cross, (c.0 + c.2 / 2) as f32, (c.1 + c.3 / 2) as f32, 14.0, 1.5, if lit(Hit::Close) { fg() } else { dim() }, 1.0);
        fill(p, ax, ay + HEAD_H - 1, aw, 1, WHITE, 0.07);

        // The body, scrolled and clipped to its viewport.
        let (vy0, vy1) = (ay + HEAD_H, ay + ah);
        let off = vy0 - self.scroll;
        let mut q = clipped(p, vy0, vy1);
        let q = &mut q;
        let selected = self.sel_card().map(|i| self.cards[i].id);
        let content = self.layout(|it| match it {
            Item::Sec(s, (x, y, w, hh)) => {
                if y + off + hh < vy0 || y + off > vy1 {
                    return;
                }
                self.paint_section(q, t, s, (x, y + off, w, hh));
            }
            Item::Card(ci, r) => {
                if r.1 + off + r.3 < vy0 || r.1 + off > vy1 {
                    return;
                }
                let c = &self.cards[ci];
                let lift = self.lift(c.id);
                let gc = c.group.and_then(|g| self.groups.get(g as usize)).map(|g| g.color);
                let state = CardState { over: lift != 0, close_lit: self.over == Some(Hit::CardClose(c.id)), selected: selected == Some(c.id), closable: self.cards.len() > 1, hue: gc };
                let shown = q.fade;
                if self.dragged(c.id) {
                    q.fade = shown * DRAGGED;
                }
                paint_card(q, t, (r.0, r.1 + off + lift, r.2, r.3), c, c.current, &state, ci);
                q.fade = shown;
            }
            Item::Tile((x, y, w, hh)) => {
                if y + off + hh >= vy0 && y + off <= vy1 {
                    paint_tile(q, t, (x, y + off, w, hh), self.over == Some(Hit::Tile) || self.drop == Some(Drop::Tile));
                }
            }
            Item::Empty((x, y, w, hh)) => {
                if y + off + hh >= vy0 && y + off <= vy1 {
                    let msg = tr(Str::OvLooseEmpty);
                    dashed(q, (x, y + off, w, hh), 10, WHITE, 0.07);
                    t.draw(q, x + (w - t.width(msg, PX)) / 2, y + off + hh / 2, msg, Ink::new(PX, faint()));
                }
            }
            Item::Nothing(y) => {
                let mut buf = self.scratch.borrow_mut();
                buf.clear();
                let _ = write!(buf, "{}{}{}", tr(Str::OvNoneBefore), self.field.text().trim(), tr(Str::OvNoneAfter));
                let msg = t.fit(&buf, PX, aw - 2 * PAD_X);
                t.draw(q, ax + (aw - t.width(&msg, PX)) / 2, y + off + 9, &msg, Ink::new(PX, dim()));
            }
        });
        if let Some((ty, th)) = thumb(content, vy1 - vy0, self.scroll) {
            q.rrect(ax + aw - 8, vy0 + ty, 5, th, 2.5, WHITE, 0.16);
        }
    }

    fn draw_grid(&self, p: &mut Paint, grid: &mut dyn Grid, _: &Cx) {
        let (ax, ay, aw, ah) = self.area;
        if aw < 120 || ah < HEAD_H + 60 {
            return;
        }
        let (vy0, vy1) = (ay + HEAD_H, ay + ah);
        let off = vy0 - self.scroll;
        let fade = p.fade;
        let mut q = clipped(p, vy0, vy1);
        let mut rows = self.rows.borrow_mut();
        let (cw, lh) = grid.metrics(PREV_SCALE);
        let pitch = lh.max((10.5 * 1.55_f32).round() as i32);
        self.layout(|it| {
            let Item::Card(ci, r) = it else { return };
            let c = &self.cards[ci];
            let (x, y) = (r.0, r.1 + off + self.lift(c.id));
            if y + PREV_H < vy0 || y > vy1 {
                return;
            }
            let fade = if self.dragged(c.id) { fade * DRAGGED } else { fade };
            let (px, inner) = (x + 1 + PREV_PAD_X, r.2 - 2 - 2 * PREV_PAD_X);
            let (top, bottom) = (y + 1, y + 1 + PREV_H);
            let lines = ((PREV_H - PREV_PAD_Y) / pitch + 1).max(1) as usize;
            rows.clear();
            let running = grid.tail(c.id, lines, &mut rows);
            for (k, line) in rows.lines().rev().enumerate() {
                let cy = bottom - PREV_PAD_Y - pitch * k as i32 - pitch / 2;
                // A line cut by the preview's top, or by the body's edges, is not drawn.
                if cy - pitch / 2 < top || cy - lh / 2 < vy0 || cy - lh / 2 + lh > vy1 {
                    continue;
                }
                if line.is_empty() {
                    continue;
                }
                let mask = (((cy - top) as f32) / (PREV_FADE * PREV_H as f32)).clamp(0.0, 1.0) * fade;
                let base = if k == 0 { if running { accent() } else { fg() } } else { PREV_INK };
                let ink = [0, 1, 2].map(|i| CARD_BG[i] + (base[i] - CARD_BG[i]) * mask);
                let text = overlay::fit(line, inner, |s| s.chars().count() as i32 * cw);
                grid.text(&mut q, px, cy, &text, PREV_SCALE, ink);
            }
        });
        let _ = ax;
    }
}

// ----------------------------------------------------------------- painting --

/// How opaque the overview's backdrop is. The reference's is `--bg` at 94 % over a 14 px blur, which smears the
/// terminal under it into a tint; without a blur the same 6 % would leave its text legible, so the backdrop is
/// nearly solid (the desktop shows through the window's glass by under 2 %).
const BACKDROP: f32 = 0.985;

/// The overview's own background over the workspace, with the window's rounded corners when the workspace
/// reaches its bottom edge.
fn backdrop(p: &mut Paint, (x, y, w, h): Rect, g: Geo) {
    let bg = palette::unit(palette::BG);
    let r = if g.square { 0 } else { chrome::RADIUS as i32 };
    if y + h >= g.h && r > 0 {
        let edge = y + h - r;
        fill(p, x, y, w, edge - y, bg, BACKDROP);
        let (ww, wh, rf) = (g.w as f32, g.h as f32, r as f32);
        p.shape(x, edge, x + w, y + h, bg, BACKDROP, &|px, py| if py < wh - rf { 1.0 } else { (0.5 - rounded_rect(px, py, ww, wh, rf)).clamp(0.0, 1.0) });
    } else {
        fill(p, x, y, w, h, bg, BACKDROP);
    }
}

/// A rounded rectangle: the straight rows are filled, only the corner rows go through coverage (a whole-area
/// `Paint::rrect` costs milliseconds at this size).
fn slab(p: &mut Paint, (x, y, w, h): Rect, r: i32, c: [f32; 3], a: f32) {
    let r = r.min(h / 2).min(w / 2);
    if w <= 0 || h <= 0 {
        return;
    }
    let (fx, fy, fw, fh) = (x as f32, y as f32, w as f32, h as f32);
    let cov = |px: f32, py: f32| (0.5 - rounded_rect(px - fx, py - fy, fw, fh, r as f32)).clamp(0.0, 1.0);
    fill(p, x, y + r, w, h - 2 * r, c, a);
    p.shape(x, y, x + w, y + r, c, a, &cov);
    p.shape(x, y + h - r, x + w, y + h, c, a, &cov);
}

/// A 1 px outline of a rounded rectangle: the corners by coverage, the straight edges filled.
fn frame(p: &mut Paint, (x, y, w, h): Rect, r: i32, c: [f32; 3], a: f32) {
    let r = r.min(h / 2).min(w / 2);
    let (fx, fy, fw, fh) = (x as f32, y as f32, w as f32, h as f32);
    let line = |px: f32, py: f32| {
        let d = rounded_rect(px - fx, py - fy, fw, fh, r as f32);
        (0.5 - d).clamp(0.0, 1.0) * (d + 1.5).clamp(0.0, 1.0)
    };
    for (cx, cy) in [(x, y), (x + w - r, y), (x, y + h - r), (x + w - r, y + h - r)] {
        p.shape(cx, cy, cx + r, cy + r, c, a, &line);
    }
    fill(p, x + r, y, w - 2 * r, 1, c, a);
    fill(p, x + r, y + h - 1, w - 2 * r, 1, c, a);
    fill(p, x, y + r, 1, h - 2 * r, c, a);
    fill(p, x + w - 1, y + r, 1, h - 2 * r, c, a);
}

/// The dashed outline of the new-group tile: dashes 6 px long with 4 px between along the straight edges, the
/// corners drawn solid.
fn dashed(p: &mut Paint, (x, y, w, h): Rect, r: i32, c: [f32; 3], a: f32) {
    let (fx, fy, fw, fh) = (x as f32, y as f32, w as f32, h as f32);
    let line = |px: f32, py: f32| {
        let d = rounded_rect(px - fx, py - fy, fw, fh, r as f32);
        (0.5 - d).clamp(0.0, 1.0) * (d + 1.5).clamp(0.0, 1.0)
    };
    for (cx, cy) in [(x, y), (x + w - r, y), (x, y + h - r), (x + w - r, y + h - r)] {
        p.shape(cx, cy, cx + r, cy + r, c, a, &line);
    }
    let mut at = x + r;
    while at < x + w - r {
        let len = 6.min(x + w - r - at);
        fill(p, at, y, len, 1, c, a);
        fill(p, at, y + h - 1, len, 1, c, a);
        at += 10;
    }
    let mut at = y + r;
    while at < y + h - r {
        let len = 6.min(y + h - r - at);
        fill(p, x, at, 1, len, c, a);
        fill(p, x + w - 1, at, 1, len, c, a);
        at += 10;
    }
}

/// A ghost button (`.ghost-btn`): a 7 % outline, an icon and, when `label` is given, its words; lit under the
/// pointer. Without words it is just the icon, centred.
#[allow(clippy::too_many_arguments)]
fn ghost(p: &mut Paint, t: &Text, (x, y, w, h): Rect, icon: Icon, label: Option<&str>, px: f32, lit: bool, r: f32) {
    if lit {
        p.rrect(x, y, w, h, r, WHITE, 0.07);
    }
    p.outline(x, y, w, h, r, WHITE, 0.07);
    let ink = if lit { fg() } else { dim() };
    let cy = y + h / 2;
    match label {
        Some(l) => {
            icons::paint(p, icon, (x + 1 + 9) as f32 + 7.0, cy as f32, 14.0, 1.5, ink, 1.0);
            t.draw(p, x + 1 + 9 + 14 + 6, cy, l, Ink::new(px, ink));
        }
        None => icons::paint(p, icon, (x + w / 2) as f32, cy as f32, 14.0, 1.5, ink, 1.0),
    }
}

impl Overview {
    /// A section's box, its tint, and its head: the group's dot and name, how many windows, the add button.
    fn paint_section(&self, p: &mut Paint, t: &Text, s: &Sec, r: Rect) {
        let (x, y, w, _) = r;
        let group = s.group.and_then(|g| self.groups.get(g));
        if let Some(g) = group {
            slab(p, r, 14, group_color(g.color), 0.05);
        }
        // A card carried over it (`.ov-sec.drop-over`): the colour at 10 % and a dashed border at 70 %; the windows in none take the accent.
        if self.drop == Some(Drop::Section(s.group)) {
            let c = group.map_or_else(accent, |g| group_color(g.color));
            slab(p, r, 14, c, if group.is_some() { 0.05 } else { 0.10 });
            dashed(p, r, 14, c, 0.7);
        }
        let cy = y + 1 + SEC_PAD + SEC_HEAD / 2;
        let mut at = x + 1 + SEC_PAD;
        // What the head may use: up to the add button, or the section's edge.
        let add = s.group.map(|_| self.add_box(t, r));
        let limit = add.map_or(x + w - 1 - SEC_PAD, |b| b.0 - 10);
        match group {
            Some(g) => {
                // The chip: 9 px, the dot, 6 px, the name, 9 px; lit with the group's colour under the pointer.
                let (chip, name) = self.chip(t, r, g);
                let lit = s.group.is_some_and(|i| self.over == Some(Hit::Chip(i)));
                if lit {
                    slab(p, chip, 8, group_color(g.color), 0.14);
                }
                p.disc((at + 9) as f32 + 4.0, cy as f32, 4.0, group_color(g.color), 1.0);
                t.draw(p, at + 9 + 8 + 6, cy, &name, Ink::new(PX, if lit { fg() } else { dim() }).medium());
                at += chip.2;
            }
            None => {
                let l = tr(Str::OvLoose);
                t.draw(p, at, cy, l, Ink::new(PX, dim()).bold());
                at += t.width(l, PX);
            }
        }
        let n = self.section_count(s.group);
        let mut buf = self.scratch.borrow_mut();
        buf.clear();
        let _ = write!(buf, "{n} {}", if n == 1 { tr(Str::OvWindow1) } else { tr(Str::OvWindowsN) });
        if at + 10 + t.width(&buf, PX_HINT) <= limit {
            t.draw(p, at + 10, cy + 1, &buf, Ink::new(PX_HINT, dim()));
        }
        if let (Some(g), Some(b)) = (s.group, add) {
            ghost(p, t, b, Icon::Plus, Some(tr(Str::OvBtnAdd)), PX_HINT, self.over == Some(Hit::Add(g)), 8.0);
        }
    }
}

/// How a card looks: pointer on it (lifted, brighter outline, with its cross), on its cross, the keyboard on it,
/// whether it can be closed, and its group's colour.
struct CardState {
    over: bool,
    close_lit: bool,
    selected: bool,
    closable: bool,
    hue: Option<u8>,
}

/// One card at `r` (window pixels, the lift already in): body and outline, the foot with the status, the title,
/// the unread dot, the pane count, the number and the cross, and the directory under it. The preview's text is
/// painted by `draw_grid`, over the empty body.
fn paint_card(p: &mut Paint, t: &Text, r: Rect, c: &Card, active: bool, s: &CardState, number: usize) {
    let (x, y, w, _) = r;
    slab(p, r, 12, CARD_BG, 1.0);
    if active {
        frame(p, (x - 1, y - 1, w + 2, r.3 + 2), 13, accent(), 0.25);
        frame(p, r, 12, accent(), 0.70);
    } else {
        frame(p, r, 12, WHITE, if s.selected { 0.34 } else if s.over { 0.18 } else { 0.07 });
        if s.selected {
            frame(p, (x - 1, y - 1, w + 2, r.3 + 2), 13, WHITE, 0.08);
        }
    }
    let fy = y + 1 + PREV_H;
    fill(p, x + 1, fy, w - 2, 1, WHITE, 0.07);
    let cy = fy + 1 + 9 + 9;
    status_icon(p, c.status, (x + 19) as f32, cy as f32, active);
    // From the right: the cross (always room for it), the number, the pane count, the unread dot.
    let close = (x + w - 1 - 10 - 18, cy - 9, 18, 18);
    if s.over && s.closable {
        if s.close_lit {
            p.rrect(close.0, close.1, close.2, close.3, 5.0, WHITE, 0.12);
        }
        icons::paint(p, Icon::Cross, (close.0 + 9) as f32, cy as f32, 12.0, 1.6, if s.close_lit { fg() } else { dim() }, 1.0);
    }
    let mut right = close.0 - 8;
    let mut buf = [0u8; 20];
    if number < 9 {
        let n = overlay::dec(number + 1, &mut buf);
        let kw = kbd_width(t, Cap::Text(n));
        kbd(p, t, right - kw, cy, Cap::Text(n), 1.0);
        right -= kw + 8;
    }
    if c.panes > 1 {
        let n = overlay::dec(c.panes as usize, &mut buf);
        let bw = t.width(n, PX_KBD) + 10;
        p.rrect(right - bw, cy - 8, bw, 16, 5.0, WHITE, 0.07);
        t.draw(p, right - bw + 5, cy, n, Ink::new(PX_KBD, dim()));
        right -= bw + 8;
    }
    if c.unread && !c.current {
        p.disc((right - 3) as f32, cy as f32, 3.0, palette::unit(palette::CYAN), 1.0);
        right -= 6 + 8;
    }
    let title = t.fit(&c.title, PX, (right - (x + 35)).max(20));
    t.draw(p, x + 35, cy, &title, Ink::new(PX, fg()).medium());
    if !c.meta.is_empty() {
        let meta = t.fit(&c.meta, PX_SMALL, w - 2 - 24);
        t.draw(p, x + 12, fy + FOOT_H + 8, &meta, Ink::new(PX_SMALL, dim()));
    }
    // A window in a group that is not on screen shows its group's colour as a thin bar at the foot's left: none
    // in the reference, so none here (the section's tint says it).
    let _ = s.hue;
}

/// What a window's shell is doing, as the reference's `.t-ico`: the prompt icon (amber on the active card), a
/// ring with a bright quarter for a command that runs (the reference spins it; a still frame costs no timer),
/// a red dot for a failure.
fn status_icon(p: &mut Paint, st: Status, cx: f32, cy: f32, active: bool) {
    match st {
        Status::Idle => icons::paint(p, Icon::Prompt, cx, cy, 14.0, 1.5, if active { accent() } else { dim() }, 1.0),
        Status::Running => {
            let a = accent();
            p.shape(cx as i32 - 8, cy as i32 - 8, cx as i32 + 9, cy as i32 + 9, a, 1.0, &|px, py| {
                let (dx, dy) = (px - cx, py - cy);
                let ring = (0.9 - (dx.hypot(dy) - 5.0).abs()).clamp(0.0, 1.0);
                if dx >= 0.0 && dy <= 0.0 { ring } else { ring * 0.28 }
            });
        }
        Status::Failed => p.disc(cx, cy, 3.5, palette::unit(palette::RED), 1.0),
    }
}

/// The dashed "new group" tile (`.ov-newgroup`): a plus and a bold line, a hint under it; amber and tinted when lit.
fn paint_tile(p: &mut Paint, t: &Text, r: Rect, lit: bool) {
    let (x, y, w, h) = r;
    let ink = if lit { accent() } else { dim() };
    if lit {
        slab(p, r, 14, accent(), 0.06);
        dashed(p, r, 14, accent(), 0.7);
    } else {
        dashed(p, r, 14, WHITE, 0.14);
    }
    let title = tr(Str::OvBtnNewGroup);
    let tw = 14 + 6 + t.width(title, PX);
    let tx = x + (w - tw) / 2;
    let cy = y + 1 + (h - 2) / 4;
    icons::paint(p, Icon::Plus, (tx + 7) as f32, cy as f32, 14.0, 1.5, ink, 1.0);
    t.draw(p, tx + 14 + 6, cy, title, Ink::new(PX, ink).bold());
    let hint = tr(Str::OvTileHint);
    t.draw(p, x + (w - t.width(hint, PX_HINT)) / 2, y + 1 + (h - 2) * 3 / 4, hint, Ink::new(PX_HINT, ink).alpha(0.8));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{overlay::NoGrid, plat::Mods};

    const G: Geo = Geo { w: 900, h: 620, square: false };
    /// The workspace of the board's window with the panel of windows at its left.
    const AREA: Rect = (220, 42, 680, 550);

    fn card(id: u32, title: &str, group: Option<u8>) -> Card {
        Card { id, group, title: title.into(), meta: "~/lab/axon".into(), panes: 1, ..Card::default() }
    }

    fn sample() -> Overview {
        let cards = vec![card(1, "arcton", Some(0)), card(2, "servidor", Some(0)), card(3, "pruebas", Some(0)), card(4, "notas", Some(1)), Card { current: true, ..card(5, "~", None) }];
        let groups = vec![Group { name: "arcton".into(), color: 1 }, Group { name: "docs".into(), color: 3 }];
        Overview::new(cards, groups, AREA)
    }

    fn key(sym: u32) -> Key {
        Key { sym, ..Key::default() }
    }

    fn typed(s: &str) -> Key {
        Key { sym: s.chars().next().unwrap() as u32, text: s.into(), ..Key::default() }
    }

    fn alt(c: char) -> Key {
        Key { sym: c as u32, mods: Mods { alt: true, ..Mods::default() }, ..Key::default() }
    }

    #[test]
    fn columns_follow_the_reference_auto_fill_of_210_px_with_12_between() {
        // The reference's captures: 827.6 wide gives 3 columns of 267.9, 607.6 gives 2 of 297.8.
        assert_eq!(columns(827), 3);
        assert_eq!(column_width(827, 3), 267);
        assert_eq!(columns(607), 2);
        assert_eq!(column_width(607, 2), 297);
        // The edges: one column below 432 (two of 210 and the gap), two from there, a column never narrower than 210.
        assert_eq!(columns(100), 1);
        assert_eq!(columns(431), 1);
        assert_eq!(columns(432), 2);
        assert_eq!(columns(653), 2);
        assert_eq!(columns(654), 3);
        for w in 210..2000 {
            let c = columns(w);
            assert!(c == 1 || column_width(w, c) >= MIN_COL, "{w}");
            assert!(c * column_width(w, c) + (c - 1) * GAP <= w.max(MIN_COL));
        }
        // The grid is the area less the body's 2 x 20, the section's border and its 2 x 10 of padding.
        assert_eq!(grid_width(899), 899 - 40 - 2 - 20);
    }

    #[test]
    fn a_card_is_178_px_high_and_a_row_of_cards_makes_the_reference_section() {
        assert_eq!(CARD_H, 178);
        assert_eq!(HEAD_H, 57, "14 + 30 + 12 + the hairline");
        // One row: 1 + 10 + 24 + 10 + 178 + 12 + 1 = 236 (the reference measured 236.3), two rows add a card and a gap.
        let mut heights = Vec::new();
        let o = Overview::new(vec![card(1, "a", Some(0)), card(2, "b", Some(0)), card(3, "c", Some(0))], vec![Group { name: "g".into(), color: 1 }], (0, 42, 900, 550));
        o.layout(|it| {
            if let Item::Sec(s, r) = it
                && s.group.is_some()
            {
                heights.push(r.3);
            }
        });
        assert_eq!(heights, [236], "three cards in the three columns of a wide body");
        let narrow = Overview { area: (0, 42, 600, 550), ..o };
        narrow.layout(|it| {
            if let Item::Sec(s, r) = it
                && s.group.is_some()
            {
                heights = vec![r.3];
            }
        });
        assert_eq!(heights, [236 + 178 + 12], "the third card goes to a second row");
    }

    #[test]
    fn cards_sit_in_their_sections_in_a_grid_and_the_tile_ends_the_list() {
        let o = sample();
        let (mut secs, mut cards, mut tile) = (Vec::new(), Vec::new(), None);
        let total = o.layout(|it| match it {
            Item::Sec(s, r) => secs.push((s.group, r)),
            Item::Card(ci, r) => cards.push((o.cards[ci].id, r)),
            Item::Tile(r) => tile = Some(r),
            Item::Empty(_) => panic!("a window is in none"),
            Item::Nothing(_) => panic!("there are windows"),
        });
        assert_eq!(secs.iter().map(|s| s.0).collect::<Vec<_>>(), [Some(0), Some(1), None]);
        assert_eq!(cards.iter().map(|c| c.0).collect::<Vec<_>>(), [1, 2, 3, 4, 5]);
        // Two columns in 680 px: the first two cards share a row, the third is under the first.
        assert_eq!(cards[0].1.1, cards[1].1.1);
        assert_eq!(cards[2].1.1, cards[0].1.1 + CARD_H + GAP);
        assert_eq!(cards[2].1.0, cards[0].1.0);
        assert_eq!(cards[1].1.0, cards[0].1.0 + cards[0].1.2 + GAP);
        // Sections are 18 px apart and the tile follows the last; the body ends 24 px below it.
        assert_eq!(secs[1].1.1, secs[0].1.1 + secs[0].1.3 + SEC_GAP);
        let tile = tile.unwrap();
        assert_eq!(tile.1, secs[2].1.1 + secs[2].1.3 + SEC_GAP);
        assert_eq!(total, tile.1 + TILE_H + BODY_BOTTOM);
        // The cards are inside their sections' padding.
        assert_eq!(cards[0].1.0, secs[0].1.0 + 1 + SEC_PAD);
        assert!(cards[2].1.1 + CARD_H < secs[0].1.1 + secs[0].1.3 - 12);
    }

    #[test]
    fn the_filter_narrows_the_cards_by_name_directory_group_and_terminal_text() {
        let mut o = sample();
        o.cards[1].meta = "~/servicios · main".into();
        let shown = |o: &Overview| o.order().iter().map(|&i| o.cards[i].id).collect::<Vec<_>>();
        assert_eq!(shown(&o), [1, 2, 3, 4, 5]);
        let text = Text::load();
        let cx = Cx { g: G, text: &text };
        for c in "SERV".chars() {
            assert_eq!(o.key(&typed(&c.to_string()), &cx), Out::Redraw);
        }
        assert_eq!(shown(&o), [2], "the name and the directory, case ignored");
        o.field.set("");
        o.lower = "docs".into();
        assert_eq!(shown(&o), [4], "the group's name");
        o.lower = "hmr".into();
        assert!(shown(&o).is_empty());
        o.cards[2].text_hit = true;
        assert_eq!(shown(&o), [3], "the app found it in the terminal's text");
        // With a filter there is no tile; with nothing found, the message and no sections.
        o.cards[2].text_hit = false;
        let mut seen = (0, 0, 0);
        o.layout(|it| match it {
            Item::Tile(_) => seen.0 += 1,
            Item::Nothing(_) => seen.1 += 1,
            Item::Sec(..) => seen.2 += 1,
            Item::Card(..) | Item::Empty(_) => {}
        });
        assert_eq!(seen, (0, 1, 0));
        // The tile is back when the filter is.
        o.lower.clear();
        let mut tile = 0;
        o.layout(|it| tile += matches!(it, Item::Tile(_)) as i32);
        assert_eq!(tile, 1);
    }

    #[test]
    fn with_groups_and_no_loose_window_the_section_of_none_stays_and_says_it_is_empty() {
        // Every window is in a group: the reference still lists "No group" (an empty box under its head), after the groups.
        let groups = vec![Group { name: "a".into(), color: 1 }, Group { name: "b".into(), color: 3 }];
        let mut o = Overview::new(vec![card(1, "x", Some(0)), card(2, "y", Some(1))], groups, AREA);
        let (mut secs, mut empty) = (Vec::new(), Vec::new());
        let total = o.layout(|it| match it {
            Item::Sec(s, r) => secs.push((s.group, r)),
            Item::Empty(r) => empty.push(r),
            _ => {}
        });
        assert_eq!(secs.iter().map(|s| s.0).collect::<Vec<_>>(), [Some(0), Some(1), None]);
        let (none, e) = (secs[2].1, empty[0]);
        assert_eq!(empty.len(), 1);
        assert_eq!(none.3, SEC_FIXED + EMPTY_H, "a head, the box with its margins and the padding");
        // The box is inside the section's padding, under the head, 6 px of margin round it.
        assert_eq!((e.0, e.1, e.2, e.3), (none.0 + 1 + SEC_PAD, none.1 + 1 + SEC_PAD + SEC_HEAD + SEC_HEAD_GAP + 6, none.2 - 2 - 2 * SEC_PAD, EMPTY_BOX));
        assert!(total > none.1 + none.3, "the tile follows it");
        // Nothing is shown for it while a filter is, nor without groups (there is nothing to be apart from).
        o.lower = "x".into();
        let mut n = 0;
        o.layout(|it| n += matches!(it, Item::Empty(_)) as i32);
        assert_eq!(n, 0);
        let none_at_all = Overview::new(vec![], vec![], AREA);
        none_at_all.layout(|it| assert!(!matches!(it, Item::Empty(_))));
    }

    #[test]
    fn contains_ci_ignores_case_and_accents_of_the_pattern_only_as_lowercase() {
        assert!(contains_ci("Servidor ARCTON", "arcton"));
        assert!(contains_ci("~/Lab/Árbol", "árbol"));
        assert!(contains_ci("anything", ""));
        assert!(!contains_ci("abc", "abd"));
        assert!(!contains_ci("", "a"));
        assert!(contains_ci("x [HMR] y", "[hmr]"));
    }

    #[test]
    fn the_arrows_move_between_cards_by_rows_and_through_the_sections() {
        // Sections of 3, 1 and 2 cards in two columns: 0 1 / 2 ; 3 ; 4 5.
        let c = [3, 1, 2];
        assert_eq!(step(&c, 2, 0, Dir::Right), 1);
        assert_eq!(step(&c, 2, 1, Dir::Right), 2, "reading order goes on to the next row");
        assert_eq!(step(&c, 2, 5, Dir::Right), 5, "and stops at the last card");
        assert_eq!(step(&c, 2, 0, Dir::Left), 0);
        assert_eq!(step(&c, 2, 0, Dir::Down), 2);
        assert_eq!(step(&c, 2, 1, Dir::Down), 2, "a shorter last row: the card that is there");
        assert_eq!(step(&c, 2, 2, Dir::Down), 3, "into the next section");
        assert_eq!(step(&c, 2, 3, Dir::Down), 4);
        assert_eq!(step(&c, 2, 5, Dir::Down), 5, "nothing below the last row");
        assert_eq!(step(&c, 2, 5, Dir::Up), 3, "up from the second column into a one-card section");
        assert_eq!(step(&c, 2, 3, Dir::Up), 2, "the last row of the section above, the same column or the last card");
        assert_eq!(step(&c, 2, 1, Dir::Up), 1, "nothing above the first row");
        assert_eq!(step(&c, 2, 4, Dir::Up), 3);
        assert_eq!((step(&c, 2, 3, Dir::First), step(&c, 2, 3, Dir::Last)), (0, 5));
        // A section with nothing shown is skipped; one column is a list; no cards is position 0.
        assert_eq!(step(&[2, 0, 2], 3, 1, Dir::Down), 3);
        assert_eq!(step(&[2, 0, 2], 3, 2, Dir::Up), 0);
        assert_eq!(step(&[4], 1, 1, Dir::Down), 2);
        assert_eq!(step(&[], 2, 0, Dir::Down), 0);
        assert_eq!(step(&[2], 2, 9, Dir::Left), 0, "a position past the end is clamped");
    }

    #[test]
    fn arrow_keys_select_cards_scroll_them_into_view_and_enter_goes_to_the_selected_one() {
        let text = Text::load();
        let cx = Cx { g: G, text: &text };
        let mut o = sample();
        assert_eq!(o.selected_id(), Some(5), "the window on screen is selected when it opens");
        assert_eq!(o.key(&key(sym::UP), &cx), Out::Redraw);
        assert_eq!(o.selected_id(), Some(4));
        o.key(&key(sym::UP), &cx);
        o.key(&key(sym::UP), &cx);
        assert_eq!(o.selected_id(), Some(1), "back to the first card");
        assert!(o.scroll <= 8, "the first row's section head is in view");
        assert_eq!(o.key(&key(sym::RETURN), &cx), Out::Run(Cmd::Go(1).encode()));
        o.key(&key(sym::END), &cx);
        assert_eq!(o.selected_id(), Some(1), "End is the filter's, without Ctrl");
        let ctrl_end = Key { sym: sym::END, mods: Mods { ctrl: true, ..Mods::default() }, ..Key::default() };
        o.key(&ctrl_end, &cx);
        assert_eq!(o.selected_id(), Some(5));
        assert!(o.scroll > 0, "the last card is below the fold of a 550 px workspace and was scrolled to");
        let card_bottom = {
            let mut b = 0;
            o.layout(|it| {
                if let Item::Card(ci, r) = it
                    && o.cards[ci].id == 5
                {
                    b = r.1 + r.3;
                }
            });
            b
        };
        assert!(card_bottom - o.scroll <= o.view_h(), "the selected card ends inside the viewport");
        // Left and Right move too when the filter is empty, and are the caret's when it is not.
        o.key(&key(sym::LEFT), &cx);
        assert_eq!(o.selected_id(), Some(4));
        o.key(&typed("x"), &cx);
        o.key(&key(sym::LEFT), &cx);
        assert_eq!(o.field.caret(), 0);
        // Tab walks the cards and wraps.
        let mut o = sample();
        o.key(&key(sym::TAB), &cx);
        assert_eq!(o.selected_id(), Some(1), "after the last, the first");
        let back = Key { sym: sym::TAB, mods: Mods { shift: true, ..Mods::default() }, ..Key::default() };
        o.key(&back, &cx);
        assert_eq!(o.selected_id(), Some(5), "before the first, the last");
        o.key(&back, &cx);
        assert_eq!(o.selected_id(), Some(4));
    }

    #[test]
    fn the_filter_keeps_the_selection_on_a_shown_card_and_enter_opens_the_first_when_it_is_gone() {
        let text = Text::load();
        let cx = Cx { g: G, text: &text };
        let mut o = sample();
        for c in "pru".chars() {
            o.key(&typed(&c.to_string()), &cx);
        }
        assert_eq!(o.order().len(), 1);
        assert_eq!(o.selected_id(), None, "the selected window (5) is filtered out");
        assert_eq!(o.key(&key(sym::RETURN), &cx), Out::Run(Cmd::Go(3).encode()), "Enter takes the first shown");
        // No match: nothing to open.
        for c in "zz".chars() {
            o.key(&typed(&c.to_string()), &cx);
        }
        assert_eq!(o.key(&key(sym::RETURN), &cx), Out::None);
        assert_eq!(o.key(&key(sym::DOWN), &cx), Out::None);
    }

    #[test]
    fn escape_clears_the_filter_first_and_closes_after_that() {
        let text = Text::load();
        let cx = Cx { g: G, text: &text };
        let mut o = sample();
        assert!(!o.handles_escape(), "nothing typed: the stack closes it");
        o.key(&typed("a"), &cx);
        assert!(o.handles_escape(), "the filter has text: Esc is the filter's");
        o.field.set("");
        o.lower.clear();
        assert!(!o.handles_escape());
        // Alt+O closes it, Alt+N opens a window and stays, Alt+digit goes to that window.
        assert_eq!(o.key(&alt('o'), &cx), Out::Close);
        assert_eq!(o.key(&alt('n'), &cx), Out::Act(Cmd::NewWindow.encode()));
        assert_eq!(o.key(&alt('2'), &cx), Out::Run(Cmd::Go(2).encode()));
        assert_eq!(o.key(&alt('9'), &cx), Out::None, "there is no ninth window");
        assert_eq!(o.key(&alt('z'), &cx), Out::None, "other chords are swallowed");
    }

    #[test]
    fn commands_survive_the_trip_through_a_u32() {
        for c in [Cmd::Go(7), Cmd::Go(MASK - 1), Cmd::Close(0), Cmd::NewWindow, Cmd::NewGroup(None), Cmd::NewGroup(Some(3)), Cmd::AddTo(5), Cmd::EditGroup(2)] {
            assert_eq!(Cmd::decode(c.encode()), Some(c), "{c:?}");
        }
        assert_eq!(Cmd::decode(0), None);
        assert_eq!(Cmd::decode(9 << 24), None);
    }

    #[test]
    fn the_header_drops_what_does_not_fit_and_keeps_the_filter_and_the_cross() {
        let (tw, cw, gw, ww) = (68, 120, 76, 88);
        let wide = head((0, 42, 900, 550), tw, cw, gw, ww);
        assert!(wide.count.is_some() && wide.labels && wide.group.is_some() && wide.window.is_some());
        assert_eq!(wide.search.2, SEARCH_W);
        assert_eq!(wide.close.0 + wide.close.2, 900 - PAD_X);
        let (g, w) = (wide.group.unwrap(), wide.window.unwrap());
        assert_eq!(w.0 + w.2 + HEAD_GAP, wide.close.0, "10 px between the buttons and the cross");
        assert_eq!(g.0 + g.2 + HEAD_GAP, w.0);
        assert_eq!((g.2, w.2), (ghost_width(gw), ghost_width(ww)));
        assert!(wide.search.0 + wide.search.2 < g.0, "the filter is left of the buttons");
        // 680: the count goes first (the reference wraps it under the title).
        let mid = head((220, 42, 680, 550), tw, cw, gw, ww);
        assert!(mid.count.is_none() && mid.labels && mid.group.is_some());
        // 460: the buttons lose their words and become squares.
        let small = head((0, 42, 460, 550), tw, cw, gw, ww);
        assert!(!small.labels && small.group.is_some_and(|r| r.2 == BTN_H));
        // 308 (the workspace beside both panels): only the filter, shrunk, and the cross.
        let tiny = head((0, 42, 308, 550), tw, cw, gw, ww);
        assert!(tiny.group.is_none() && tiny.window.is_none());
        assert!(tiny.search.2 >= 40 && tiny.search.0 + tiny.search.2 <= tiny.close.0, "{tiny:?}");
        // Everything is inside the header's 57 rows.
        for h in [wide, mid, small, tiny] {
            assert!(h.search.1 >= 42 && h.search.1 + h.search.3 <= 42 + HEAD_H);
            assert!(h.close.1 >= 42 && h.close.1 + h.close.3 <= 42 + HEAD_H);
        }
    }

    #[test]
    fn the_scrollbar_thumb_is_in_proportion_and_ends_at_the_track() {
        assert_eq!(thumb(400, 500, 0), None);
        let (top, h) = thumb(1000, 500, 0).unwrap();
        assert_eq!((top, h), (0, 250));
        assert_eq!(thumb(1000, 500, 500), Some((250, 250)));
        assert_eq!(thumb(1000, 500, 9999).map(|t| t.0 + t.1), Some(500), "scrolling past the end is the end");
        assert_eq!(thumb(1_000_000, 500, 0).unwrap().1, 20, "never smaller than a grip");
    }

    #[test]
    fn clicks_go_where_the_things_are_drawn_and_run_on_release() {
        let text = Text::load();
        let cx = Cx { g: G, text: &text };
        let mut o = sample().shown();
        let mut card1 = None;
        o.layout(|it| {
            if let Item::Card(ci, r) = it
                && o.cards[ci].id == 2
            {
                card1 = Some(r);
            }
        });
        let r = card1.unwrap();
        let (x, y) = ((r.0 + 40) as f32, (r.1 + 30 + AREA.1 + HEAD_H - o.scroll) as f32);
        let click = |x, y, pressed| Click { x, y, pressed, time: 0 };
        assert_eq!(o.pointer(x, y, &cx), Out::Redraw);
        assert_eq!(o.over, Some(Hit::Card(2)));
        assert_eq!(o.cursor(x, y, &cx), Cursor::Pointer);
        assert_eq!(o.button(click(x, y, true), &cx), Out::Redraw);
        assert_eq!(o.selected_id(), Some(2), "a press selects the card");
        assert_eq!(o.button(click(x, y, false), &cx), Out::Run(Cmd::Go(2).encode()));
        // Pressed on a card, released on nothing: nothing runs.
        o.button(click(x, y, true), &cx);
        assert_eq!(o.button(click(x, 5.0, false), &cx), Out::Redraw);
        // The cross of a hovered card closes the window without closing the overview.
        let close = Overview::close_box(r);
        let (cx_, cy_) = ((close.0 + 9) as f32, (close.1 + 9 + AREA.1 + HEAD_H - o.scroll) as f32);
        o.pointer(cx_, cy_, &cx);
        assert_eq!(o.over, Some(Hit::CardClose(2)));
        o.button(click(cx_, cy_, true), &cx);
        assert_eq!(o.button(click(cx_, cy_, false), &cx), Out::Act(Cmd::Close(2).encode()));
        // The only window has no cross.
        let one = Overview::new(vec![Card { current: true, ..card(1, "a", None) }], vec![], AREA);
        let mut r1 = (0, 0, 0, 0);
        one.layout(|it| {
            if let Item::Card(_, r) = it {
                r1 = r;
            }
        });
        let close = Overview::close_box(r1);
        assert_eq!(one.hit_at(&text, (close.0 + 9) as f32, (close.1 + 9 + AREA.1 + HEAD_H) as f32), Some(Hit::Card(1)));
        // The header's buttons.
        let h = o.head(&text);
        let mid = |r: Rect| ((r.0 + r.2 / 2) as f32, (r.1 + r.3 / 2) as f32);
        let go = |o: &mut Overview, at: (f32, f32)| {
            o.button(click(at.0, at.1, true), &cx);
            o.button(click(at.0, at.1, false), &cx)
        };
        assert_eq!(go(&mut o, mid(h.window.unwrap())), Out::Act(Cmd::NewWindow.encode()));
        assert_eq!(go(&mut o, mid(h.close)), Out::Close);
        o.sel = Some(4);
        assert_eq!(go(&mut o, mid(h.group.unwrap())), Out::Act(Cmd::NewGroup(None).encode()), "the dialog opens empty, not around the selected window");
        assert_eq!(o.cursor(mid(h.search).0, mid(h.search).1, &cx), Cursor::Text);
        // A group's own add button.
        let mut sec0 = None;
        o.layout(|it| {
            if let Item::Sec(s, r) = it
                && s.group == Some(0)
            {
                sec0 = Some(r);
            }
        });
        let b = o.add_box(&text, sec0.unwrap());
        let at = (mid(b).0, mid(b).1 + (AREA.1 + HEAD_H - o.scroll) as f32);
        assert_eq!(go(&mut o, at), Out::Act(Cmd::AddTo(0).encode()));
        // Its chip opens the dialog that edits the group, and is lit under the pointer.
        let (chip, _) = o.chip(&text, sec0.unwrap(), &o.groups[0]);
        assert_eq!((chip.2 > 9 + 8 + 6 + 9, chip.3), (true, SEC_HEAD));
        let at = (mid(chip).0, mid(chip).1 + (AREA.1 + HEAD_H - o.scroll) as f32);
        o.pointer(at.0, at.1, &cx);
        assert_eq!((o.over, o.cursor(at.0, at.1, &cx)), (Some(Hit::Chip(0)), Cursor::Pointer));
        assert_eq!(go(&mut o, at), Out::Act(Cmd::EditGroup(0).encode()));
        assert!(chip.0 + chip.2 < b.0, "the chip ends before the add button");
        // Outside the workspace is not the overview's.
        assert!(o.contains(500.0, 300.0, &cx) && !o.contains(100.0, 300.0, &cx));
        assert_eq!(o.hit_at(&text, 100.0, 300.0), None);
    }

    #[test]
    fn a_card_carried_to_a_section_or_the_tile_goes_there_and_esc_puts_it_back() {
        let text = Text::load();
        let cx = Cx { g: G, text: &text };
        // A body tall enough to show every section, the card and the tile at once (no scrolling in the way).
        let mut o = Overview { area: (AREA.0, AREA.1, AREA.2, 1400), ..sample().shown() };
        let click = |x, y, pressed| Click { x, y, pressed, time: 0 };
        // Card 5 (the window in none), the section of "docs" (group 1) and the tile, as window pixels at the scroll `o` has.
        let find = |o: &Overview| {
            let (mut card, mut docs, mut tile, mut none) = ((0, 0, 0, 0), (0, 0, 0, 0), (0, 0, 0, 0), (0, 0, 0, 0));
            o.layout(|it| match it {
                Item::Card(ci, r) if o.cards[ci].id == 5 => card = r,
                Item::Sec(s, r) if s.group == Some(1) => docs = r,
                Item::Sec(s, r) if s.group.is_none() => none = r,
                Item::Tile(r) => tile = r,
                _ => {}
            });
            let at = |r: Rect| ((r.0 + 40) as f32, (r.1 + 6 + AREA.1 + HEAD_H - o.scroll) as f32);
            (at(card), at(docs), at(tile), at(none))
        };
        // A press and a release in the same place is a click, as it was.
        let (card, ..) = find(&o);
        o.button(click(card.0, card.1, true), &cx);
        assert_eq!(o.drag.map(|d| (d.id, d.moved)), Some((5, false)));
        assert_eq!(o.pointer(card.0 + 2.0, card.1 + 1.0, &cx), Out::Redraw, "the lift only");
        assert_eq!(o.drag.map(|d| d.moved), Some(false), "under 5 px it is not a drag");
        assert_eq!(o.button(click(card.0, card.1, false), &cx), Out::Run(Cmd::Go(5).encode()));
        assert!(o.drag.is_none());
        // Carried over the "docs" section and let go there: the window and the group are left for the app.
        let (card, docs, ..) = find(&o);
        o.button(click(card.0, card.1, true), &cx);
        o.pointer(docs.0, docs.1, &cx);
        assert_eq!((o.drag.map(|d| d.moved), o.drop, o.over), (Some(true), Some(Drop::Section(Some(1))), None), "the pointer lights nothing while it carries");
        assert!(o.dragged(5) && !o.dragged(4));
        assert_eq!(o.button(click(docs.0, docs.1, false), &cx), Out::Act(Cmd::Move.encode()));
        assert_eq!((o.dropped.take(), o.drag, o.drop), (Some((5, Some(1))), None, None));
        // Onto the section of none (its own): a move to none; the cursor of a card is still a hand.
        let (card, _, _, none) = find(&o);
        o.button(click(card.0, card.1, true), &cx);
        o.pointer(none.0, none.1, &cx);
        assert_eq!(o.drop, Some(Drop::Section(None)));
        assert_eq!(o.button(click(none.0, none.1, false), &cx), Out::Act(Cmd::Move.encode()));
        assert_eq!(o.dropped.take(), Some((5, None)));
        // Onto the tile: the dialog, with this window ticked.
        let (card, _, tile, _) = find(&o);
        o.button(click(card.0, card.1, true), &cx);
        o.pointer(tile.0, tile.1, &cx);
        assert_eq!(o.drop, Some(Drop::Tile));
        assert_eq!(o.button(click(tile.0, tile.1, false), &cx), Out::Act(Cmd::NewGroup(Some(5)).encode()));
        // Esc puts it back: nothing is dropped, the overview stays, and keys are not heard while it is carried.
        let (card, docs, ..) = find(&o);
        o.button(click(card.0, card.1, true), &cx);
        o.pointer(docs.0, docs.1, &cx);
        assert!(o.handles_escape());
        assert_eq!(o.key(&typed("x"), &cx), Out::None);
        assert_eq!(o.key(&key(sym::ESCAPE), &cx), Out::Redraw);
        assert_eq!((o.drag, o.drop, o.handles_escape()), (None, None, false));
        assert_eq!(o.button(click(docs.0, docs.1, false), &cx), Out::Redraw);
        assert_eq!(o.dropped, None);
        // Let go over the header, or off the sections: nowhere.
        o.button(click(card.0, card.1, true), &cx);
        o.pointer(card.0, AREA.1 as f32 + 10.0, &cx);
        assert_eq!(o.drop, None);
        assert_eq!(o.button(click(card.0, AREA.1 as f32 + 10.0, false), &cx), Out::Redraw);
        assert_eq!(o.dropped, None);
        // A cross is not carried.
        let r = Overview::close_box(find_card(&o, 2));
        let (cx_, cy_) = ((r.0 + 9) as f32, (r.1 + 9 + AREA.1 + HEAD_H - o.scroll) as f32);
        o.pointer(cx_, cy_, &cx);
        o.button(click(cx_, cy_, true), &cx);
        assert_eq!(o.drag, None);
    }

    fn find_card(o: &Overview, id: u32) -> Rect {
        let mut r = (0, 0, 0, 0);
        o.layout(|it| {
            if let Item::Card(ci, b) = it
                && o.cards[ci].id == id
            {
                r = b;
            }
        });
        r
    }

    #[test]
    fn the_wheel_scrolls_the_body_inside_its_range() {
        let text = Text::load();
        let cx = Cx { g: G, text: &text };
        let mut o = sample().shown();
        let max = o.max_scroll();
        assert!(max > 100, "{max}");
        assert_eq!(o.scroll(60.0, 400.0, 300.0, &cx), Out::Redraw);
        assert_eq!(o.scroll, 60);
        assert_eq!(o.scroll(-9999.0, 400.0, 300.0, &cx), Out::Redraw);
        assert_eq!(o.scroll, 0);
        assert_eq!(o.scroll(-5.0, 400.0, 300.0, &cx), Out::None);
        o.scroll(99999.0, 400.0, 300.0, &cx);
        assert_eq!(o.scroll, max);
        // Fewer cards: the position is kept inside the shorter content.
        o.cards.truncate(1);
        o.settle();
        assert_eq!(o.scroll, o.max_scroll());
    }

    fn painted(o: &Overview) -> Vec<u32> {
        let text = Text::load();
        let cx = Cx { g: G, text: &text };
        let mut fb = vec![0u32; 900 * 620];
        let mut p = Paint { fb: &mut fb, w: 900, h: 620, ox: 0, oy: 0, fade: 1.0 };
        o.draw(&mut p, &cx);
        o.draw_grid(&mut p, &mut NoGrid, &cx);
        fb
    }

    #[test]
    fn the_overview_covers_its_workspace_only_and_fades_in() {
        let fb = painted(&sample().shown());
        let at = |x: usize, y: usize| fb[y * 900 + x];
        assert_eq!(at(100, 300), 0, "the panel of windows is not painted over");
        assert_eq!(at(500, 10), 0, "nor the title bar");
        assert_eq!(at(500, 610), 0, "nor the status bar below the workspace");
        assert!(at(235, 70) >> 24 >= 0xf8, "the backdrop is nearly solid");
        let covered = fb.iter().filter(|&&px| px >> 24 != 0).count();
        assert!(covered > 600 * 500, "{covered}");
        // Not yet shown: invisible on the first frame.
        assert!(painted(&sample()).iter().all(|&px| px == 0));
    }

    #[test]
    fn the_cards_the_filter_and_the_states_each_paint_differently() {
        let base = painted(&sample().shown());
        let differs = |o: &Overview| painted(o).iter().zip(&base).filter(|(a, b)| a != b).count();
        assert!(differs(&sample().shown().state("serv", None, None, 0)) > 20_000, "a filter removes cards");
        assert!(differs(&sample().shown().state("", Some(2), None, 0)) > 100, "the selection has an outline");
        assert!(differs(&sample().shown().state("", None, Some(1), 0)) > 100, "the pointer lifts and lights a card");
        assert!(differs(&sample().shown().over_close(1)) > 50);
        assert!(differs(&sample().shown().over_button(0)) > 50);
        assert!(differs(&sample().shown().over_button(3)) == 0, "the tile is below the fold unscrolled");
        let max = sample().max_scroll();
        assert!(differs(&sample().shown().state("", None, None, max)) > 20_000);
        assert!(differs(&sample().shown().state("", None, None, max).over_button(3)) > 20_000);
        assert!(differs(&sample().shown().state("zzzz", None, None, 0)) > 20_000, "the message");
    }

    #[test]
    fn a_narrow_or_short_workspace_paints_without_overflowing_the_buffer() {
        let text = Text::load();
        let cx = Cx { g: G, text: &text };
        for area in [(220, 42, 308, 550), (0, 42, 900, 550), (600, 42, 300, 200), (0, 42, 120, 120), (0, 42, 50, 50)] {
            let mut o = sample().shown();
            o.area = area;
            o.settle();
            let mut fb = vec![0u32; 900 * 620];
            let mut p = Paint { fb: &mut fb, w: 900, h: 620, ox: 0, oy: 0, fade: 1.0 };
            o.draw(&mut p, &cx);
            for scroll in [0, 80, 100_000] {
                o.scroll = scroll;
                o.settle();
                o.draw(&mut p, &cx);
            }
        }
    }

    #[test]
    fn the_count_reads_in_both_forms() {
        let o = sample();
        let mut s = String::new();
        o.count_text(&mut s);
        assert!(s.starts_with("5 ") && s.contains('2'), "{s}");
        let mut one = Overview::new(vec![card(1, "a", None)], vec![], AREA);
        s.clear();
        one.count_text(&mut s);
        assert!(s.starts_with("1 ") && s.contains(" 0 "), "{s}");
        // Panes count as windows, in the section and in the total.
        one.cards[0].panes = 3;
        assert_eq!(one.section_count(None), 3);
    }
}
