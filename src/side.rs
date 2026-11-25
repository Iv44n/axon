//! The list of windows, after the `terminal (1).html` reference, in either of its two places: a panel
//! beside the grid (a header with a `+`, one 28 px row per window) or tabs in the title bar. A row or
//! tab has an icon, the title, a dot when it printed while hidden and a cross on hover; the selected
//! one is lit. Pure layout and hit testing in window pixels, and the painting of a box the caller has
//! cleared (`Soft::clear_rect`). The text is the grid's own font, small: a proportional UI font would
//! cost 5 MB of RSS for something that is always on screen.
use alacritty_terminal::vte::ansi::Rgb;

use crate::{
    cell::pack,
    chrome::TITLE_H,
    icons::{self, Icon},
    dnd::{Grab, Target},
    paint::{Paint, WHITE, seg},
    palette,
    soft::Soft,
};

/// The header's height, a row's height, the gap between rows and the padding around the list.
pub const HEAD_H: i32 = 34;
pub const ROW_H: i32 = 30;
pub const GAP: i32 = 1;
pub const PAD: i32 = 8;
/// A group's header row, the space above a group, and how far its windows are indented.
const GROUP_H: i32 = 26;
const GROUP_GAP: i32 = 10;
const INDENT: i32 = 8;
/// A tab at the top is a little lower than a row of the panel.
const TAB_H: i32 = 28;
/// The `+` and the cross are square buttons this big.
const BTN: i32 = 24;
const CLOSE: i32 = 18;
/// A pane's row under its window (`.prow`): 26 px tall, indented 22 px (24 inside a group, where the group's
/// bar runs 16 px to its left) and with 4 px to the right of its cross.
const PANE_H: i32 = 26;
const PANE_IND: i32 = 22;
const PANE_IND_GROUPED: i32 = 24;
const PANE_RIGHT: i32 = 4;
/// A tab at the top: its natural width, its least, and the gap between tabs.
const TAB_W: i32 = 168;
const TAB_MIN: i32 = 96;
const TAB_GAP: i32 = 4;
/// The `+` after the tabs is as big as the one in the panel's header, but tall as a tab.
const TAB_NEW: i32 = 28;
/// A group at the top (`.tgroup`): a container 2 px round its chip and its tabs, 2 px between them, and the
/// chip (`.gchip`: 24 px tall, 9 px of padding, an 8 px dot, 6 px gaps, at most 180 px wide).
const GROUP_PAD: i32 = 2;
const GROUP_INNER: i32 = 2;
const CHIP_H: i32 = 24;
const CHIP_PAD: i32 = 9;
const CHIP_DOT: i32 = 8;
const CHIP_GAP: i32 = 6;
const CHIP_MAX: i32 = 180;
/// What a chip takes besides its texts: the padding, the dot and the two gaps.
const CHIP_FIXED: i32 = 2 * CHIP_PAD + CHIP_DOT + 2 * CHIP_GAP;
/// How far a window's row or a tab is faded while it is being dragged (`.dragging { opacity: .4 }`).
const DRAGGED: f32 = 0.4;
/// The marks of a drop (`drop-before`, `drop-after`): a line this thick; and the ring round a group's header.
const MARK: i32 = 2;

/// The colours a group can have (`GROUP_COLORS` of the reference): the accent and the six hues, and a grey.
pub fn group_color(i: u8) -> [f32; 3] {
    match i % 8 {
        0 => palette::unit(palette::ACCENT),
        1 => palette::unit(palette::MAGENTA),
        2 => palette::unit(palette::CYAN),
        3 => palette::unit(palette::GREEN),
        4 => palette::unit(palette::BLUE),
        5 => palette::unit(palette::RED),
        6 => palette::unit(palette::YELLOW),
        _ => [0.631, 0.631, 0.667],
    }
}

type Rect = (i32, i32, i32, i32);

/// What the pointer is over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    /// The window's row or tab: go to it.
    Row(usize),
    /// The cross of a row: close it.
    Close(usize),
    /// The `+`: a new window.
    New,
    /// The group button beside it (the panel's header only): the dialog for a new group.
    NewGroup,
    /// A group's header: fold or unfold it.
    Head(usize),
    /// A pane's row under its window: go to the window and focus the pane `(window, pane)`.
    Pane(usize, usize),
    /// The cross of a pane's row: close the pane.
    ClosePane(usize, usize),
}

/// What the panel lists, top to bottom: a group's header, a window (`grouped`: it is in a group) and, under
/// a window with several panes, one row for each of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entry {
    Head(usize),
    Row { win: usize, grouped: bool },
    Pane { win: usize, idx: usize, grouped: bool },
}

/// A group as the panel or the tabs show it. `busy`: it is folded and a window in it runs a command, which its
/// dot says by turning (`.gchip .spin`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupItem {
    /// The group's `Group::id`: what a command names it by, as places change.
    pub id: u32,
    pub name: String,
    pub color: u8,
    pub collapsed: bool,
    pub count: usize,
    pub busy: bool,
}

/// The entries for windows whose groups are `win_group` and that have `panes` panes each, and groups whose
/// folded state is `collapsed`: each group's header and its windows (a folded group lists only `active`, the
/// window on screen, if it is one of its own), in the order of the groups, then the windows that are in none. A
/// window with more than one pane is followed by its panes.
pub fn entries(win_group: &[Option<usize>], collapsed: &[bool], panes: &[usize], active: Option<usize>) -> Vec<Entry> {
    let mut out = Vec::with_capacity(win_group.len() + collapsed.len());
    let window = |out: &mut Vec<Entry>, win: usize, grouped: bool| {
        out.push(Entry::Row { win, grouped });
        let n = panes.get(win).copied().unwrap_or(1);
        if n > 1 {
            out.extend((0..n).map(|idx| Entry::Pane { win, idx, grouped }));
        }
    };
    for (g, &folded) in collapsed.iter().enumerate() {
        out.push(Entry::Head(g));
        for (win, _) in win_group.iter().enumerate().filter(|(win, w)| **w == Some(g) && (!folded || Some(*win) == active)) {
            window(&mut out, win, true);
        }
    }
    for (win, _) in win_group.iter().enumerate().filter(|(_, w)| w.is_none()) {
        window(&mut out, win, false);
    }
    out
}

/// What a pane's row says about its shell: nothing special, a command running, or the last one failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Status {
    #[default]
    Idle,
    Running,
    Failed,
}

impl Status {
    /// What a window says about its panes (`Tab.status`): running if any pane runs a command, else failed if
    /// any last command failed, else nothing special.
    pub fn of(panes: impl IntoIterator<Item = Status>) -> Status {
        panes.into_iter().fold(Status::Idle, |a, s| match (a, s) {
            (Status::Running, _) | (_, Status::Running) => Status::Running,
            (Status::Failed, _) | (_, Status::Failed) => Status::Failed,
            _ => Status::Idle,
        })
    }
}

/// How long the spinner of a running command takes to turn once (`@keyframes spin`, .8 s), and how often it is
/// painted while one is on screen: 25 frames a second. Nothing is scheduled when nothing runs.
pub const SPIN_MS: u128 = 800;
pub const SPIN_EVERY_MS: u64 = 40;

/// Where in its turn the spinner is `ms` milliseconds after some fixed moment: 0 to 1.
pub fn spin_phase(ms: u128) -> f32 {
    (ms % SPIN_MS) as f32 / SPIN_MS as f32
}

/// A pane as its row shows it: the text (`Pane 2`, plus what a program set as its title), whether it is the
/// one with the focus in the window on screen, and its status.
pub struct PaneItem<'a> {
    pub title: &'a str,
    pub active: bool,
    pub status: Status,
}

/// A window as the list shows it. `badge` is the number of panes written in a tab at the top (0: none); `status`
/// is what its panes say together (`Status::of`).
pub struct Item<'a> {
    pub title: &'a str,
    pub selected: bool,
    pub unread: bool,
    pub badge: usize,
    pub status: Status,
}

/// Where a text is in `Items::text`: its start and its length in bytes.
#[derive(Clone, Copy)]
struct Span(u32, u32);

struct WinRow {
    title: Span,
    selected: bool,
    unread: bool,
    badge: usize,
    status: Status,
    /// Where its panes begin in `Items::panes` (they follow one another, window after window).
    first: u32,
}

struct PaneRow {
    title: Span,
    active: bool,
    status: Status,
}

/// What the list shows of the windows and of the panes under them, flat: one pool of text, the windows in the order of
/// the list, and the panes of all of them in the same order. The painters read an `Item` or a `PaneItem` from it
/// (`win`, `pane`) with no copy; the app keeps one and fills it again for each paint (`clear`, `push_win`,
/// `push_pane`), which allocates nothing once its three arrays have grown to fit.
#[derive(Default)]
pub struct Items {
    text: String,
    wins: Vec<WinRow>,
    panes: Vec<PaneRow>,
}

impl Items {
    /// Forgets what was listed, keeping the room.
    pub fn clear(&mut self) {
        self.text.clear();
        self.wins.clear();
        self.panes.clear();
    }

    /// How many windows are listed.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.wins.len()
    }

    /// Adds the next window; the panes added after it (`push_pane`) are its own.
    pub fn push_win(&mut self, title: &str, selected: bool, unread: bool, badge: usize, status: Status) {
        let title = self.put(|t| t.push_str(title));
        self.wins.push(WinRow { title, selected, unread, badge, status, first: self.panes.len() as u32 });
    }

    /// Adds a pane to the window added last; `label` writes its text.
    pub fn push_pane(&mut self, active: bool, status: Status, label: impl FnOnce(&mut String)) {
        let title = self.put(label);
        self.panes.push(PaneRow { title, active, status });
    }

    /// Window `i` as the painters read it.
    pub fn win(&self, i: usize) -> Item<'_> {
        let w = &self.wins[i];
        Item { title: self.str(w.title), selected: w.selected, unread: w.unread, badge: w.badge, status: w.status }
    }

    /// Pane `idx` of window `win`.
    pub fn pane(&self, win: usize, idx: usize) -> PaneItem<'_> {
        let p = &self.panes[self.wins[win].first as usize + idx];
        PaneItem { title: self.str(p.title), active: p.active, status: p.status }
    }

    /// What `write` appends to the pool, and where it went.
    fn put(&mut self, write: impl FnOnce(&mut String)) -> Span {
        let start = self.text.len();
        write(&mut self.text);
        Span(start as u32, (self.text.len() - start) as u32)
    }

    fn str(&self, s: Span) -> &str {
        &self.text[s.0 as usize..(s.0 + s.1) as usize]
    }
}

/// What the tabs at the top list, left to right: like `entries`, but no window lists its panes (a tab says how
/// many it has).
pub fn top_entries(win_group: &[Option<usize>], collapsed: &[bool], active: usize) -> Vec<Entry> {
    entries(win_group, collapsed, &[], Some(active))
}

/// How many digits `n` is written with.
fn digits(n: usize) -> i32 {
    n.checked_ilog10().map_or(1, |l| l as i32 + 1)
}

/// How many characters of group `g`'s name its chip shows: all, or what a 180 px chip has room for.
fn chip_chars(g: &GroupItem, cw: i32) -> i32 {
    let room = (CHIP_MAX - CHIP_FIXED) / cw.max(1) - digits(g.count);
    (g.name.chars().count() as i32).min(room.max(1))
}

/// The width of group `g`'s chip when a character is `cw` px wide: the dot, the name and the count.
fn chip_w(g: &GroupItem, cw: i32) -> i32 {
    CHIP_FIXED + (chip_chars(g, cw) + digits(g.count)) * cw.max(1)
}

/// What `Layout::Top` has put somewhere: an entry's box, or the container round a group's chip and tabs.
#[derive(Clone, Copy)]
enum Placed {
    Entry(usize, Rect),
    Group(usize, Rect),
}

/// A drop's mark, in window pixels: a line where the dragged window or group would go in (before or after
/// something), or a ring round a group's header or chip (the window would go into the group).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Line(Rect),
    Ring(Rect, usize),
}

/// Where the list is: a panel at `(x0, y0)`, `w` wide, listing `entries`, or the tabs between `x0` and `x1`
/// in the title bar (`entries` as `top_entries` makes them; `cw` is how wide a character of a chip is).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout<'a> {
    Side { x0: i32, y0: i32, w: i32, entries: &'a [Entry] },
    Top { x0: i32, x1: i32, entries: &'a [Entry], groups: &'a [GroupItem], cw: i32 },
}

impl<'a> Layout<'a> {
    /// Calls `f` with the box of each entry in turn until it says to stop (a tab at the top, a row of the panel).
    fn each(self, f: &mut dyn FnMut(usize, Rect) -> bool) {
        match self {
            Layout::Side { x0, y0, w, entries } => {
                let mut y = y0 + HEAD_H;
                let mut prev: Option<Entry> = None;
                for (i, e) in entries.iter().enumerate() {
                    // The space above a group, and above the windows that are in none (after groups).
                    let gap = match (e, prev) {
                        (Entry::Head(_), _) => GROUP_GAP,
                        (Entry::Row { grouped: false, .. }, Some(Entry::Head(_) | Entry::Row { grouped: true, .. } | Entry::Pane { grouped: true, .. })) => GROUP_GAP,
                        _ => 0,
                    };
                    y += gap;
                    let (h, ind) = match e {
                        Entry::Head(_) => (GROUP_H, 0),
                        Entry::Row { grouped, .. } => (ROW_H, if *grouped { INDENT } else { 0 }),
                        Entry::Pane { grouped, .. } => (PANE_H, if *grouped { PANE_IND_GROUPED } else { PANE_IND }),
                    };
                    if f(i, (x0 + PAD + ind, y, w - 2 * PAD - ind, h)) {
                        return;
                    }
                    y += h + GAP;
                    prev = Some(*e);
                }
            }
            Layout::Top { .. } => {
                self.top_walk(&mut |p| match p {
                    Placed::Entry(k, r) => f(k, r),
                    Placed::Group(..) => false,
                });
            }
        }
    }

    /// Lays the tabs out and calls `f` with what lands where, a group's chip and tabs and then its container,
    /// until it says to stop. Returns where the next thing would start. Every tab is as wide as the bar allows
    /// once the chips, the containers' padding, the gaps and the `+` have their room: `TAB_W` down to `TAB_MIN`.
    fn top_walk(self, f: &mut dyn FnMut(Placed) -> bool) -> i32 {
        let Layout::Top { x0, x1, entries, groups, cw } = self else { return 0 };
        let (mut tabs, mut fixed, mut units) = (0, TAB_NEW + TAB_GAP, 0);
        for e in entries {
            match e {
                Entry::Head(g) => {
                    units += 1;
                    fixed += 2 * GROUP_PAD + chip_w(&groups[*g], cw);
                }
                Entry::Row { grouped, .. } => {
                    tabs += 1;
                    if *grouped {
                        fixed += GROUP_INNER;
                    } else {
                        units += 1;
                    }
                }
                Entry::Pane { .. } => {}
            }
        }
        let w = ((x1 - x0 - fixed - units * TAB_GAP) / tabs.max(1)).clamp(TAB_MIN, TAB_W);
        let y = (TITLE_H as i32 - 1 - TAB_H) / 2;
        let (mut x, mut i) = (x0, 0);
        while i < entries.len() {
            match entries[i] {
                Entry::Head(g) => {
                    let cwid = chip_w(&groups[g], cw);
                    let mut gx = x + GROUP_PAD;
                    if f(Placed::Entry(i, (gx, y + (TAB_H - CHIP_H) / 2, cwid, CHIP_H))) {
                        return x;
                    }
                    gx += cwid;
                    let mut j = i + 1;
                    while let Some(Entry::Row { grouped: true, .. }) = entries.get(j) {
                        gx += GROUP_INNER;
                        if f(Placed::Entry(j, (gx, y, w, TAB_H))) {
                            return x;
                        }
                        gx += w;
                        j += 1;
                    }
                    let total = gx + GROUP_PAD - x;
                    if f(Placed::Group(g, (x, y - GROUP_PAD, total, TAB_H + 2 * GROUP_PAD))) {
                        return x;
                    }
                    x += total + TAB_GAP;
                    i = j;
                }
                Entry::Row { .. } => {
                    if f(Placed::Entry(i, (x, y, w, TAB_H))) {
                        return x;
                    }
                    x += w + TAB_GAP;
                    i += 1;
                }
                Entry::Pane { .. } => i += 1,
            }
        }
        x
    }

    /// The box of entry `k` (a tab, at the top).
    fn rect(self, k: usize) -> Rect {
        let mut found = (0, 0, 0, 0);
        self.each(&mut |i, r| {
            found = r;
            i == k
        });
        if k < self.count() { found } else { (0, 0, 0, 0) }
    }

    /// How many entries there are.
    fn count(self) -> usize {
        match self {
            Layout::Side { entries, .. } | Layout::Top { entries, .. } => entries.len(),
        }
    }

    /// What is listed, in order.
    fn entries(self) -> &'a [Entry] {
        match self {
            Layout::Side { entries, .. } | Layout::Top { entries, .. } => entries,
        }
    }

    /// What entry `k` is: a window's index, or a group's header.
    fn entry(self, k: usize) -> Entry {
        match self {
            Layout::Side { entries, .. } | Layout::Top { entries, .. } => entries[k],
        }
    }

    /// The box of the group button in the panel's header: the right-most, 6 px from the edge. The tabs at the top have none.
    fn group_box(self) -> Option<Rect> {
        match self {
            Layout::Side { x0, y0, w, .. } => Some((x0 + w - 6 - BTN, y0 + (HEAD_H - BTN) / 2 + 2, BTN, BTN)),
            Layout::Top { .. } => None,
        }
    }

    /// The box of the `+`: left of the group button with 2 px between them in the panel; after the last tab at the top,
    /// or, when the tabs do not all fit, at the end of the bar.
    fn new_box(self) -> Rect {
        match self {
            Layout::Side { x0, y0, w, .. } => (x0 + w - 6 - 2 * BTN - 2, y0 + (HEAD_H - BTN) / 2 + 2, BTN, BTN),
            Layout::Top { x0, x1, .. } => {
                let end = self.top_walk(&mut |_| false);
                let at = if end + TAB_NEW <= x1 { end } else { (x1 - TAB_NEW).max(x0) };
                (at, (TITLE_H as i32 - 1 - TAB_H) / 2, TAB_NEW, TAB_H)
            }
        }
    }

    /// Whether the tab or chip in `r` is seen: the tabs that do not fit before the `+` are left out.
    fn shown(self, r: Rect) -> bool {
        match self {
            Layout::Side { .. } => true,
            Layout::Top { .. } => r.0 + r.2 + TAB_GAP <= self.new_box().0,
        }
    }

    /// What is at window pixel `(px, py)`.
    pub fn hit(self, px: f32, py: f32) -> Option<Part> {
        if self.group_box().is_some_and(|b| inside(b, px, py)) {
            return Some(Part::NewGroup);
        }
        if inside(self.new_box(), px, py) {
            return Some(Part::New);
        }
        let mut found = None;
        self.each(&mut |k, b| {
            if !inside(b, px, py) {
                return false;
            }
            if self.shown(b) {
                found = Some(match self.entry(k) {
                    Entry::Head(g) => Part::Head(g),
                    Entry::Row { win, .. } if inside(close_box(b), px, py) => Part::Close(win),
                    Entry::Row { win, .. } => Part::Row(win),
                    Entry::Pane { win, idx, .. } if inside(pane_close_box(b), px, py) => Part::ClosePane(win, idx),
                    Entry::Pane { win, idx, .. } => Part::Pane(win, idx),
                });
            }
            true
        });
        found
    }

    /// The box of what `grab` is: the window's row or tab, or the group's header or chip.
    pub fn grab_box(self, grab: Grab) -> Option<Rect> {
        let mut found = None;
        self.each(&mut |k, r| {
            let hit = match (grab, self.entry(k)) {
                (Grab::Win(w), Entry::Row { win, .. }) => w == win,
                (Grab::Group(g), Entry::Head(h)) => g == h,
                _ => false,
            };
            if hit {
                found = Some(r);
            }
            hit
        });
        found
    }

    /// The box round window `win`'s row and the rows of its panes (the row alone, at the top).
    fn block(self, win: usize) -> Option<Rect> {
        let mut found: Option<Rect> = None;
        self.each(&mut |k, r| match self.entry(k) {
            Entry::Row { win: w, .. } if w == win => {
                found = Some(r);
                false
            }
            Entry::Pane { win: w, .. } if w == win => {
                if let Some(b) = found.as_mut() {
                    b.3 = r.1 + r.3 - b.1;
                }
                false
            }
            _ => found.is_some(),
        });
        found
    }

    /// The box round group `g`'s header and everything listed under it (its windows, their panes); at the top,
    /// the container.
    fn group_block(self, g: usize) -> Option<Rect> {
        match self {
            Layout::Side { entries, .. } => {
                let mut found: Option<Rect> = None;
                self.each(&mut |k, r| {
                    let Some(b) = found.as_mut() else {
                        if entries[k] == Entry::Head(g) {
                            found = Some(r);
                        }
                        return false;
                    };
                    // The group ends at the next header or at the first window that is in none.
                    if matches!(entries[k], Entry::Head(_) | Entry::Row { grouped: false, .. }) {
                        return true;
                    }
                    b.3 = r.1 + r.3 - b.1;
                    false
                });
                found
            }
            Layout::Top { .. } => {
                let mut found = None;
                self.top_walk(&mut |p| match p {
                    Placed::Group(h, r) if h == g => {
                        found = Some(r);
                        true
                    }
                    _ => false,
                });
                found
            }
        }
    }

    /// The group that entry `k` is under: the last header before it, for a window in a group.
    fn group_at(self, k: usize) -> Option<usize> {
        let entries = match self {
            Layout::Side { entries, .. } | Layout::Top { entries, .. } => entries,
        };
        match entries[k] {
            Entry::Head(g) => Some(g),
            Entry::Row { grouped: true, .. } | Entry::Pane { grouped: true, .. } => entries[..k].iter().rev().find_map(|e| if let Entry::Head(g) = e { Some(*g) } else { None }),
            _ => None,
        }
    }

    /// Where dropping `grab` with the pointer at window pixel `(px, py)` would put it, or `None` when it would
    /// not go anywhere (the pointer is off the list, or over what is being dragged). A window goes next to the
    /// window under the pointer, before or after it by which half the pointer is in, or into the group of the
    /// header under it, or, on the empty part of the list, out of every group; a group goes before or after
    /// the group of what is under the pointer (a loose window: after the last one).
    pub fn target(self, grab: Grab, px: f32, py: f32) -> Option<Target> {
        let (region, along) = match self {
            Layout::Side { x0, y0, w, .. } => ((x0, y0 + HEAD_H, w, i32::MAX / 2), py),
            Layout::Top { x0, x1, .. } => ((x0, 0, x1 - x0, TITLE_H as i32 - 1), px),
        };
        if !inside(region, px, py) {
            return None;
        }
        // The entry that holds the pointer: the first whose far edge is past it, so the gaps belong to the
        // entry after them.
        let mut owner = None;
        self.each(&mut |k, r| {
            let far = if matches!(self, Layout::Side { .. }) { r.1 + r.3 } else { r.0 + r.2 };
            if along < far as f32 && self.shown(r) {
                owner = Some((k, r));
            }
            owner.is_some()
        });
        let Some((k, r)) = owner else {
            // Past the last entry: the empty part of the list.
            return matches!(grab, Grab::Win(_)).then_some(Target::End);
        };
        let after = |r: Rect| if matches!(self, Layout::Side { .. }) { py > (r.1 + r.3 / 2) as f32 } else { px > (r.0 + r.2 / 2) as f32 };
        let entry = self.entry(k);
        match (grab, entry) {
            (Grab::Win(_), Entry::Head(g)) => Some(Target::Into(g)),
            (Grab::Win(w), Entry::Row { win, .. } | Entry::Pane { win, .. }) => {
                (win != w).then(|| Target::Near { win, after: after(self.block(win).unwrap_or(r)) })
            }
            (Grab::Group(h), Entry::Head(g)) => (g != h).then(|| Target::Beside { group: g, after: after(r) }),
            (Grab::Group(h), Entry::Row { win, .. } | Entry::Pane { win, .. }) => match self.group_at(k) {
                Some(g) => (g != h).then(|| Target::Beside { group: g, after: after(self.block(win).unwrap_or(r)) }),
                // A window in none: the group goes after the last one.
                None => {
                    let last = self.last_group()?;
                    (last != h).then_some(Target::Beside { group: last, after: true })
                }
            },
        }
    }

    /// The group listed last.
    fn last_group(self) -> Option<usize> {
        let entries = match self {
            Layout::Side { entries, .. } | Layout::Top { entries, .. } => entries,
        };
        entries.iter().rev().find_map(|e| if let Entry::Head(g) = e { Some(*g) } else { None })
    }

    /// The mark that tells where a drop on `to` would go: a line before or after the window's row or tab, or
    /// the group's block, a ring round a header or chip, or a line at the end of the list.
    pub fn mark(self, to: Target) -> Option<Mark> {
        let side = matches!(self, Layout::Side { .. });
        // A line along `r`'s near or far edge, outside it.
        let line = |r: Rect, after: bool| -> Mark {
            Mark::Line(match (side, after) {
                (true, false) => (r.0, r.1 - MARK, r.2, MARK),
                (true, true) => (r.0, r.1 + r.3, r.2, MARK),
                (false, false) => (r.0 - MARK, r.1, MARK, r.3),
                (false, true) => (r.0 + r.2, r.1, MARK, r.3),
            })
        };
        match to {
            Target::Near { win, after } => Some(line(self.block(win)?, after)),
            Target::Into(g) => {
                let mut found = None;
                self.each(&mut |k, r| {
                    if self.entry(k) == Entry::Head(g) {
                        found = Some(Mark::Ring(r, g));
                    }
                    found.is_some()
                });
                found
            }
            Target::Beside { group, after } => {
                let b = self.group_block(group)?;
                // In the panel the line is as wide as a header, not as a window's row.
                let b = if let Layout::Side { x0, w, .. } = self { (x0 + PAD, b.1, w - 2 * PAD, b.3) } else { b };
                Some(line(b, after))
            }
            Target::End => {
                let mut last: Option<Rect> = None;
                self.each(&mut |_, r| {
                    last = Some(r);
                    false
                });
                match self {
                    Layout::Side { x0, y0, w, .. } => {
                        let y = last.map_or(y0 + HEAD_H, |r| r.1 + r.3 + GAP);
                        Some(Mark::Line((x0 + PAD, y + 3, w - 2 * PAD, MARK)))
                    }
                    Layout::Top { .. } => {
                        let n = self.new_box();
                        Some(Mark::Line((n.0 - TAB_GAP + 1, n.1, MARK, n.3)))
                    }
                }
            }
        }
    }

    /// The groups' containers and their group, at the top.
    fn containers(self, f: &mut dyn FnMut(usize, Rect)) {
        self.top_walk(&mut |p| {
            if let Placed::Group(g, r) = p {
                f(g, r);
            }
            false
        });
    }
}

/// Where the top left corner of the ghost of a dragged row of `size` goes when the pointer is at `at` and
/// grabbed it `off` from its corner: it follows the pointer, kept inside `region`.
pub fn ghost_at((rx, ry, rw, rh): Rect, (w, h): (i32, i32), at: (f32, f32), off: (f32, f32)) -> (i32, i32) {
    let x = (at.0 - off.0).round() as i32;
    let y = (at.1 - off.1).round() as i32;
    (x.clamp(rx, (rx + rw - w).max(rx)), y.clamp(ry, (ry + rh - h).max(ry)))
}

/// The cross's box in a row box.
fn close_box((x, y, w, h): Rect) -> Rect {
    (x + w - 5 - CLOSE, y + (h - CLOSE) / 2, CLOSE, CLOSE)
}

/// The cross's box in a pane's row box.
fn pane_close_box((x, y, w, h): Rect) -> Rect {
    (x + w - PANE_RIGHT - CLOSE, y + (h - CLOSE) / 2, CLOSE, CLOSE)
}

fn inside((x, y, w, h): Rect, px: f32, py: f32) -> bool {
    px >= x as f32 && px < (x + w) as f32 && py >= y as f32 && py < (y + h) as f32
}

/// `text` cut to `max` characters, with `…` where it was: its characters, for `Soft::paint_text`, with no string made.
pub fn fit(text: &str, max: usize) -> impl Iterator<Item = char> + '_ {
    let long = text.chars().count() > max;
    text.chars().take(if long { max.saturating_sub(1) } else { usize::MAX }).chain(long.then_some('…'))
}

/// A count written in decimal, a character at a time, with no string made: it is painted again with every list.
fn dec(n: usize) -> impl Iterator<Item = char> {
    let width = digits(n) as u32;
    (0..width).rev().map(move |i| char::from(b'0' + (n / 10usize.pow(i) % 10) as u8))
}

fn plus(p: &mut Paint, b: Rect, hot: bool) {
    if hot {
        p.rrect(b.0, b.1, b.2, b.3, 8.0, WHITE, 0.07);
    }
    let c = palette::unit(if hot { palette::FG } else { palette::DIM });
    let (cx, cy) = (b.0 as f32 + b.2 as f32 / 2.0, b.1 as f32 + b.3 as f32 / 2.0);
    p.shape(b.0, b.1, b.0 + b.2, b.1 + b.3, c, 1.0, &|px, py| {
        let d = seg(px, py, (cx - 4.5, cy), (cx + 4.5, cy)).min(seg(px, py, (cx, cy - 4.5), (cx, cy + 4.5)));
        (1.2 - d).clamp(0.0, 1.0)
    });
}

/// The new-group button: a rounded box with a plus in it, lit under the pointer.
fn group_button(p: &mut Paint, b: Rect, hot: bool) {
    if hot {
        p.rrect(b.0, b.1, b.2, b.3, 8.0, WHITE, 0.07);
    }
    let c = palette::unit(if hot { palette::FG } else { palette::DIM });
    icons::paint(p, Icon::Group, b.0 as f32 + b.2 as f32 / 2.0, b.1 as f32 + b.3 as f32 / 2.0, 14.0, 1.5, c, 1.0);
}

/// How a window's row or tab sits in its group: not in one, with the group's bar on its left (the panel), or
/// tinted with the group's colour when it is the one on screen (the tabs).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Grouping {
    No,
    Bar(u8),
    Tint(u8),
}

/// What a row, a tab or a header is painted with besides its own data: whether the pointer is over it and on its
/// cross, where its group is, how wide a character of the pane count is, where the spinners are in their turn,
/// and how opaque it is (1, or `DRAGGED` for the row being dragged and for its ghost).
#[derive(Clone, Copy)]
struct Ink {
    over: bool,
    cross_hot: bool,
    group: Grouping,
    bcw: i32,
    phase: f32,
    fade: f32,
}

/// `c` as it looks over the glass at opacity `k`.
fn faded(c: Rgb, k: f32) -> Rgb {
    let mix = |a: u8, bg: u8| (bg as f32 + (a as f32 - bg as f32) * k + 0.5) as u8;
    Rgb { r: mix(c.r, palette::BG.r), g: mix(c.g, palette::BG.g), b: mix(c.b, palette::BG.b) }
}

/// A ring `d` px across that turns: its track at `track` of the opacity of `c`, and a quarter of it in `c`, whose
/// middle is at the top when `phase` is 0 and goes round clockwise as it grows to 1 (`.spin`: a 1.6 px border
/// with only its top coloured, turning).
fn spinner(p: &mut Paint, cx: f32, cy: f32, d: f32, c: [f32; 3], track: f32, phase: f32) {
    use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI, TAU};
    let thick = 1.6;
    let r = d / 2.0 - thick / 2.0;
    let mid = -FRAC_PI_2 + phase * TAU;
    let o = d / 2.0 + 1.0;
    p.shape((cx - o) as i32, (cy - o) as i32, (cx + o) as i32 + 1, (cy + o) as i32 + 1, c, 1.0, &|px, py| {
        let (dx, dy) = (px - cx, py - cy);
        let ring = (thick / 2.0 + 0.5 - (dx.hypot(dy) - r).abs()).clamp(0.0, 1.0);
        if ring <= 0.0 {
            return 0.0;
        }
        let a = (dy.atan2(dx) - mid + PI).rem_euclid(TAU) - PI;
        let arc = ((FRAC_PI_4 - a.abs()) * r + 0.5).clamp(0.0, 1.0);
        ring * (track + (1.0 - track) * arc)
    });
}

/// The status icon of a row, centred on `(cx, cy)`: the prompt (a chevron and a dash, in `idle`) when nothing
/// special is going on, a turning ring while a command runs, a red dot when the last one failed.
fn status_icon(p: &mut Paint, cx: f32, cy: f32, st: Status, idle: [f32; 3], phase: f32) {
    match st {
        Status::Idle => {
            let (x, y) = (cx as i32, cy as i32);
            p.shape(x - 8, y - 8, x + 9, y + 9, idle, 1.0, &|px, py| {
                let d = seg(px, py, (cx - 4.5, cy - 3.5), (cx - 1.0, cy)).min(seg(px, py, (cx - 1.0, cy), (cx - 4.5, cy + 3.5))).min(seg(px, py, (cx + 1.0, cy + 3.5), (cx + 5.0, cy + 3.5)));
                (1.1 - d).clamp(0.0, 1.0)
            });
        }
        Status::Running => spinner(p, cx, cy, 11.0, palette::unit(palette::ACCENT), 0.28, phase),
        Status::Failed => p.disc(cx, cy, 3.5, palette::unit(palette::RED), 1.0),
    }
}

/// The width of the box that says how many panes a tab has (10 px of padding round the digits), or 0.
fn badge_w(panes: usize, cw: i32) -> i32 {
    if panes < 2 { 0 } else { digits(panes) * cw + 10 }
}

/// Where the pane count's box is in a tab's box `b`: just left of the cross, 16 px tall.
fn badge_box(b: Rect, w: i32) -> Rect {
    (close_box(b).0 - 7 - w, b.1 + (b.3 - 16) / 2, w, 16)
}

/// A row's or tab's background: the colour and the opacity it is lit with (the window on screen, or the pointer over it).
fn row_bg(it: &Item, over: bool, group: Grouping) -> ([f32; 3], f32) {
    match (it.selected, group) {
        (true, Grouping::Tint(g)) => (group_color(g), 0.16),
        (true, _) => (WHITE, 0.075),
        (false, _) if over => (WHITE, 0.045),
        _ => (WHITE, 0.0),
    }
}

/// The shapes of one row or tab in box `b`: its background, status icon, unread dot, pane count and cross. In a
/// group in the panel it has the group's bar on its left edge, and square corners there.
fn row_shapes(p: &mut Paint, b: Rect, it: &Item, k: Ink) {
    let (x, y, rw, rh) = b;
    let Ink { over, cross_hot, group, bcw, phase, fade } = k;
    p.fade = fade;
    let (bg_c, bg) = row_bg(it, over, group);
    if bg > 0.0 {
        if let Grouping::Bar(_) = group {
            // Round only on the right: the left edge is the group's bar.
            let (fx, fy, fw, fh) = ((x - 8) as f32, y as f32, (rw + 8) as f32, rh as f32);
            p.shape(x, y, x + rw, y + rh, bg_c, bg, &|px, py| (0.5 - crate::chrome::rounded_rect(px - fx, py - fy, fw, fh, 8.0)).clamp(0.0, 1.0));
        } else {
            p.rrect(x, y, rw, rh, 8.0, bg_c, bg);
        }
    }
    if let Grouping::Bar(g) = group {
        for yy in y..y + rh {
            for xx in x..x + 2 {
                p.put(xx, yy, group_color(g), 0.5);
            }
        }
    }
    let ic = palette::unit(if it.selected || over { palette::DIM } else { palette::FAINT });
    status_icon(p, (x + 9) as f32 + 6.5, y as f32 + rh as f32 / 2.0, it.status, ic, phase);
    // The number of panes, in a box before the cross (a tab at the top: the panel lists them instead).
    let bw = badge_w(it.badge, bcw);
    if bw > 0 {
        let bb = badge_box(b, bw);
        p.rrect(bb.0, bb.1, bb.2, bb.3, 5.0, WHITE, 0.07);
    }
    // The dot of a window that printed while it was hidden.
    if it.unread && !it.selected {
        let tail = if bw > 0 {
            (badge_box(b, bw).0 - 7) as f32 - 3.0
        } else if over {
            close_box(b).0 as f32 - 6.0
        } else {
            (x + rw) as f32 - 12.0
        };
        p.disc(tail, y as f32 + rh as f32 / 2.0, 3.0, palette::unit(palette::CYAN), 1.0);
    }
    // The cross, on hover.
    if over {
        cross(p, close_box(b), cross_hot);
    }
    p.fade = 1.0;
}

/// A cross in its box `c`, lit when the pointer is on it.
fn cross(p: &mut Paint, c: Rect, hot: bool) {
    if hot {
        p.rrect(c.0, c.1, c.2, c.3, 5.0, WHITE, 0.12);
    }
    let (cx, cy) = (c.0 as f32 + c.2 as f32 / 2.0, c.1 as f32 + c.3 as f32 / 2.0);
    let col = palette::unit(if hot { palette::FG } else { palette::DIM });
    p.shape(c.0, c.1, c.0 + c.2, c.1 + c.3, col, 1.0, &|px, py| {
        let d = seg(px, py, (cx - 3.0, cy - 3.0), (cx + 3.0, cy + 3.0)).min(seg(px, py, (cx + 3.0, cy - 3.0), (cx - 3.0, cy + 3.0)));
        (1.0 - d).clamp(0.0, 1.0)
    });
}

/// The shapes of a pane's row in box `b`: background (lit when it is the focused pane of the window on
/// screen), the group's bar 16 px to its left, the status icon and the cross on hover.
fn pane_shapes(p: &mut Paint, b: Rect, it: &PaneItem, over: bool, cross_hot: bool, group: Option<u8>, phase: f32) {
    let (x, y, rw, rh) = b;
    let bg = if it.active { 0.07 } else if over { 0.045 } else { 0.0 };
    if bg > 0.0 {
        p.rrect(x, y, rw, rh, 7.0, WHITE, bg);
    }
    if let Some(g) = group {
        for yy in y..y + rh {
            for xx in x - 16..x - 14 {
                p.put(xx, yy, group_color(g), 0.5);
            }
        }
    }
    status_icon(p, (x + 8) as f32 + 6.5, y as f32 + rh as f32 / 2.0, it.status, palette::unit(palette::FAINT), phase);
    if over {
        let c = pane_close_box(b);
        if cross_hot {
            p.rrect(c.0, c.1, c.2, c.3, 5.0, WHITE, 0.12);
        }
        let (cx, cy) = (c.0 as f32 + c.2 as f32 / 2.0, c.1 as f32 + c.3 as f32 / 2.0);
        let col = palette::unit(if cross_hot { palette::FG } else { palette::DIM });
        p.shape(c.0, c.1, c.0 + c.2, c.1 + c.3, col, 1.0, &|px, py| {
            let d = seg(px, py, (cx - 3.0, cy - 3.0), (cx + 3.0, cy + 3.0)).min(seg(px, py, (cx + 3.0, cy - 3.0), (cx - 3.0, cy + 3.0)));
            (1.0 - d).clamp(0.0, 1.0)
        });
    }
}

/// The dot of a group's header or chip: its colour, or, folded with a window running a command, a ring in its
/// colour that turns. `cx` is where the dot's middle is.
fn group_dot(p: &mut Paint, cx: f32, cy: f32, g: &GroupItem, phase: f32) {
    let c = group_color(g.color);
    if g.busy {
        spinner(p, cx, cy, 10.0, c, 0.30, phase);
    } else {
        p.disc(cx, cy, 3.5, c, 1.0);
    }
}

/// The shapes of a group's header in box `b`: a tinted background under the pointer, the colour's dot and
/// a chevron that points down when the group is open and right when it is folded.
fn head_shapes(p: &mut Paint, b: Rect, g: &GroupItem, over: bool, phase: f32, fade: f32) {
    let (x, y, w, h) = b;
    let c = group_color(g.color);
    p.fade = fade;
    if over {
        p.rrect(x, y, w, h, 7.0, c, 0.14);
    }
    group_dot(p, (x + 6) as f32 + 3.5, y as f32 + h as f32 / 2.0, g, phase);
    let (cx, cy) = ((x + w - 8) as f32 - 4.0, y as f32 + h as f32 / 2.0);
    let col = palette::unit(palette::FAINT);
    p.shape(x + w - 18, y + 3, x + w - 2, y + h - 3, col, 1.0, &|px, py| {
        let d = if g.collapsed {
            seg(px, py, (cx - 1.5, cy - 3.5), (cx + 2.0, cy)).min(seg(px, py, (cx + 2.0, cy), (cx - 1.5, cy + 3.5)))
        } else {
            seg(px, py, (cx - 3.5, cy - 1.5), (cx, cy + 2.0)).min(seg(px, py, (cx, cy + 2.0), (cx + 3.5, cy - 1.5)))
        };
        (1.0 - d).clamp(0.0, 1.0)
    });
    p.fade = 1.0;
}

/// The shapes of a group's chip in the tabs, in box `b`: a tint of the group's colour under the pointer, and
/// its dot (or the spinner).
fn chip_shapes(p: &mut Paint, b: Rect, g: &GroupItem, over: bool, phase: f32, fade: f32) {
    let (x, y, w, h) = b;
    p.fade = fade;
    if over {
        p.rrect(x, y, w, h, 8.0, group_color(g.color), 0.14);
    }
    group_dot(p, (x + CHIP_PAD) as f32 + CHIP_DOT as f32 / 2.0, y as f32 + h as f32 / 2.0, g, phase);
    p.fade = 1.0;
}

/// The title of one row or tab, in the grid's font at 0.85 of its size.
fn row_text(soft: &Soft, fb: &mut [u32], b: Rect, it: &Item, over: bool, fade: f32, scratch: &mut Vec<u8>) {
    let (ox, oy) = soft.origin();
    let m = soft.small(0.85);
    let (x, y, rw, rh) = b;
    let bm = soft.small(0.72);
    let bw = badge_w(it.badge, bm.cell_w as i32);
    let room = (rw - 9 - 13 - 7 - 5 - if over { CLOSE + 6 } else { 18 } - if bw > 0 { bw + 7 } else { 0 }).max(0);
    let label = fit(it.title, (room / m.cell_w as i32).max(1) as usize);
    let c: Rgb = faded(if it.selected || over { palette::FG } else { palette::DIM }, fade);
    soft.paint_text(fb, ox + (x + 9 + 13 + 7) as usize, oy + (y + rh / 2) as usize, label, pack(c.r, c.g, c.b, 255), false, &m, scratch);
    if bw > 0 {
        let bb = badge_box(b, bw);
        let d = faded(palette::DIM, fade);
        soft.paint_text(fb, ox + (bb.0 + 5) as usize, oy + (bb.1 + bb.3 / 2) as usize, dec(it.badge), pack(d.r, d.g, d.b, 255), false, &bm, scratch);
    }
}

/// The text of a pane's row.
fn pane_text(soft: &Soft, fb: &mut [u32], b: Rect, it: &PaneItem, over: bool, scratch: &mut Vec<u8>) {
    let (ox, oy) = soft.origin();
    let m = soft.small(0.82);
    let (x, y, rw, rh) = b;
    let room = (rw - 8 - 13 - 7 - PANE_RIGHT - CLOSE - 7).max(0);
    let label = fit(it.title, (room / m.cell_w as i32).max(1) as usize);
    let c: Rgb = if it.active || over { palette::FG } else { palette::DIM };
    soft.paint_text(fb, ox + (x + 8 + 13 + 7) as usize, oy + (y + rh / 2) as usize, label, pack(c.r, c.g, c.b, 255), false, &m, scratch);
}

/// A group's name and, at the right end, how many windows it has.
fn head_text(soft: &Soft, fb: &mut [u32], b: Rect, g: &GroupItem, fade: f32, scratch: &mut Vec<u8>) {
    let (ox, oy) = soft.origin();
    let m = soft.small(0.85);
    let (x, y, w, h) = b;
    let cw = m.cell_w as i32;
    let count_w = digits(g.count) * cw;
    let room = (w - 6 - 7 - 6 - 8 - count_w - 22).max(0);
    let name = fit(&g.name, (room / cw).max(1) as usize);
    let (fg, faint) = (faded(palette::FG, fade), faded(palette::FAINT, fade));
    let cy = oy + (y + h / 2) as usize;
    soft.paint_text(fb, ox + (x + 6 + 7 + 6) as usize, cy, name, pack(fg.r, fg.g, fg.b, 255), true, &m, scratch);
    soft.paint_text(fb, ox + (x + w - 22 - count_w) as usize, cy, dec(g.count), pack(faint.r, faint.g, faint.b, 255), false, &m, scratch);
}

/// A group chip's name (medium weight, dim, bright under the pointer) and its count, in the tabs.
fn chip_text(soft: &Soft, fb: &mut [u32], b: Rect, g: &GroupItem, over: bool, fade: f32, scratch: &mut Vec<u8>) {
    let (ox, oy) = soft.origin();
    let m = soft.small(0.85);
    let cw = m.cell_w as i32;
    let (x, y, _, h) = b;
    let chars = chip_chars(g, cw);
    let name = fit(&g.name, chars as usize);
    let (fg, faint) = (faded(if over { palette::FG } else { palette::DIM }, fade), faded(palette::FAINT, fade));
    let cy = oy + (y + h / 2) as usize;
    let nx = x + CHIP_PAD + CHIP_DOT + CHIP_GAP;
    soft.paint_text(fb, ox + nx as usize, cy, name, pack(fg.r, fg.g, fg.b, 255), true, &m, scratch);
    soft.paint_text(fb, ox + (nx + chars * cw + CHIP_GAP) as usize, cy, dec(g.count), pack(faint.r, faint.g, faint.b, 255), false, &m, scratch);
}

fn over_row(hover: Option<Part>, i: usize) -> bool {
    matches!(hover, Some(Part::Row(j) | Part::Close(j)) if j == i)
}

fn over_pane(hover: Option<Part>, win: usize, idx: usize) -> bool {
    matches!(hover, Some(Part::Pane(w, i) | Part::ClosePane(w, i)) if (w, i) == (win, idx))
}

/// A drag in progress, as the painter needs it: what is dragged (it stays where it was, at `DRAGGED` opacity),
/// where a drop would put it (a mark there) and where its ghost is: the top left corner, in window pixels.
#[derive(Clone, Copy, Debug)]
pub struct Dragging {
    pub grab: Grab,
    pub target: Option<Target>,
    pub at: (i32, i32),
}

/// What the list is painted with besides its data: what the pointer is over, the drag in progress, and where
/// the spinners are in their turn (`spin_phase`).
#[derive(Clone, Copy)]
pub struct Look {
    pub hover: Option<Part>,
    pub drag: Option<Dragging>,
    pub phase: f32,
}

/// Whether `e` is what is being dragged.
fn is_grab(e: Entry, drag: Option<Dragging>) -> bool {
    match (drag.map(|d| d.grab), e) {
        (Some(Grab::Win(w)), Entry::Row { win, .. }) => w == win,
        (Some(Grab::Group(g)), Entry::Head(h)) => g == h,
        _ => false,
    }
}

/// The mark of a drop in `p`: a line in the accent colour, or a 2 px ring round a header in its group's colour
/// (`.drop-before`, `.drop-into`); `r` is the radius of the header's corners.
fn paint_mark(p: &mut Paint, m: Mark, groups: &[GroupItem], r: f32) {
    match m {
        Mark::Line((x, y, w, h)) => p.shape(x, y, x + w, y + h, palette::unit(palette::ACCENT), 1.0, &|_, _| 1.0),
        Mark::Ring((x, y, w, h), g) => {
            let c = groups.get(g).map_or(palette::unit(palette::ACCENT), |g| group_color(g.color));
            let (fx, fy, fw, fh) = (x as f32, y as f32, w as f32, h as f32);
            let t = MARK as f32;
            p.shape(x - MARK - 1, y - MARK - 1, x + w + MARK + 1, y + h + MARK + 1, c, 1.0, &|px, py| {
                let d = crate::chrome::rounded_rect(px - fx, py - fy, fw, fh, r);
                (d + 0.5).clamp(0.0, 1.0) * (t + 0.5 - d).clamp(0.0, 1.0)
            });
        }
    }
}

/// The glass behind a ghost, so the rows it passes over do not show through its text.
fn ghost_backing(p: &mut Paint, (x, y, w, h): Rect) {
    p.rrect(x, y, w, h, 8.0, palette::unit(palette::BG), 0.96);
}

/// Turns the spinners a step without painting the list again: each ring is wiped back to the glass and what lay on it
/// (a group's container at the top, the row's lit background), and drawn at `look.phase`. The text is not painted
/// again, which is most of what painting the list costs. What is under a ring is flat (the ring sits 9 px or more from
/// the corners of its row), so this is the same as painting everything again, which `design.rs` checks. The list must
/// be as the last full paint left it (same pointer, same selection, nothing dragged). `bottom` is where the panel
/// ends; at the top, what is not shown (`Layout::shown`) is left out.
#[allow(clippy::too_many_arguments)]
pub fn paint_spin(soft: &Soft, fb: &mut [u32], lay: Layout, bottom: i32, entries: &[Entry], groups: &[GroupItem], items: &Items, look: Look) {
    let (fw, fh) = soft.fb_size();
    let (ox, oy) = soft.origin();
    if fb.len() < fw * fh {
        return;
    }
    let panel = matches!(lay, Layout::Side { .. });
    for (k, e) in entries.iter().enumerate() {
        let b = lay.rect(k);
        if (panel && b.1 + b.3 > bottom) || !lay.shown(b) {
            continue;
        }
        let mid = b.1 as f32 + b.3 as f32 / 2.0;
        // At the top a group's chip and tabs lie in its container, tinted with its colour.
        let tint = match *e {
            _ if panel => None,
            Entry::Head(g) => Some(group_color(groups[g].color)),
            Entry::Row { grouped: true, .. } => Some(group_color(group_of(entries, k, groups))),
            _ => None,
        };
        // Where the ring is, what lies on the glass under it besides the tint, and how big it is.
        let (cx, lit, d) = match *e {
            Entry::Row { win, grouped } if items.win(win).status == Status::Running => {
                let g = match (grouped, panel) {
                    (false, _) => Grouping::No,
                    (true, true) => Grouping::Bar(group_of(entries, k, groups)),
                    (true, false) => Grouping::Tint(group_of(entries, k, groups)),
                };
                ((b.0 + 9) as f32 + 6.5, row_bg(&items.win(win), over_row(look.hover, win), g), 11.0)
            }
            Entry::Pane { win, idx, .. } if items.pane(win, idx).status == Status::Running => {
                let it = items.pane(win, idx);
                ((b.0 + 8) as f32 + 6.5, (WHITE, if it.active { 0.07 } else if over_pane(look.hover, win, idx) { 0.045 } else { 0.0 }), 11.0)
            }
            Entry::Head(g) if groups[g].busy => {
                let cx = if panel { (b.0 + 6) as f32 + 3.5 } else { (b.0 + CHIP_PAD) as f32 + CHIP_DOT as f32 / 2.0 };
                (cx, (group_color(groups[g].color), if look.hover == Some(Part::Head(g)) { 0.14 } else { 0.0 }), 10.0)
            }
            _ => continue,
        };
        let (x, y) = (cx as i32 - 7, mid as i32 - 7);
        soft.clear_rect(fb, ((ox as i32 + x) as usize, (oy as i32 + y) as usize, (ox as i32 + x + 15) as usize, (oy as i32 + y + 15) as usize));
        let mut p = Paint { fb: &mut *fb, w: fw, h: fh, ox: ox as i32, oy: oy as i32, fade: 1.0 };
        if let Some(c) = tint {
            p.shape(x, y, x + 15, y + 15, c, 0.06, &|_, _| 1.0);
        }
        if lit.1 > 0.0 {
            p.shape(x, y, x + 15, y + 15, lit.0, lit.1, &|_, _| 1.0);
        }
        match *e {
            Entry::Head(g) => group_dot(&mut p, cx, mid, &groups[g], look.phase),
            _ => spinner(&mut p, cx, mid, d, palette::unit(palette::ACCENT), 0.28, look.phase),
        }
    }
}

/// The top layer of a drag, over the finished list: the ghost of what is dragged (its row or header or chip at
/// `DRAGGED` opacity on a little glass, where the pointer took it), and the mark of where a drop would put it.
#[allow(clippy::too_many_arguments)]
fn paint_drag(soft: &Soft, fb: &mut [u32], lay: Layout, entries: &[Entry], groups: &[GroupItem], items: &Items, look: Look, scratch: &mut Vec<u8>) {
    let Some(d) = look.drag else { return };
    let (fw, fh) = soft.fb_size();
    let (ox, oy) = soft.origin();
    let panel = matches!(lay, Layout::Side { .. });
    let bcw = soft.small(0.72).cell_w as i32;
    let ghost = lay.grab_box(d.grab).map(|r| (d.at.0, d.at.1, r.2, r.3));
    if let Some(gb) = ghost {
        let mut p = Paint { fb: &mut *fb, w: fw, h: fh, ox: ox as i32, oy: oy as i32, fade: 1.0 };
        ghost_backing(&mut p, gb);
        match d.grab {
            Grab::Win(win) => {
                let k = entries.iter().position(|e| matches!(e, Entry::Row { win: w, .. } if *w == win)).unwrap_or(0);
                let group = match entries.get(k) {
                    Some(Entry::Row { grouped: true, .. }) if panel => Grouping::Bar(group_of(entries, k, groups)),
                    Some(Entry::Row { grouped: true, .. }) => Grouping::Tint(group_of(entries, k, groups)),
                    _ => Grouping::No,
                };
                row_shapes(&mut p, gb, &items.win(win), Ink { over: false, cross_hot: false, group, bcw, phase: look.phase, fade: DRAGGED });
            }
            Grab::Group(g) if panel => head_shapes(&mut p, gb, &groups[g], false, look.phase, DRAGGED),
            Grab::Group(g) => chip_shapes(&mut p, gb, &groups[g], false, look.phase, DRAGGED),
        }
        match d.grab {
            Grab::Win(win) => row_text(soft, fb, gb, &items.win(win), false, DRAGGED, scratch),
            Grab::Group(g) if panel => head_text(soft, fb, gb, &groups[g], DRAGGED, scratch),
            Grab::Group(g) => chip_text(soft, fb, gb, &groups[g], false, DRAGGED, scratch),
        }
    }
    if let Some(m) = d.target.and_then(|t| lay.mark(t)) {
        let mut p = Paint { fb: &mut *fb, w: fw, h: fh, ox: ox as i32, oy: oy as i32, fade: 1.0 };
        paint_mark(&mut p, m, groups, if panel { 7.0 } else { 8.0 });
    }
}

/// Paints the panel whose box is `(x0, y0)` (window pixels), `w` wide and `h` tall: the header with its `+`,
/// then `entries`. `look` says what the pointer is over, what is being dragged and how far the spinners have
/// turned; `grid_on_right` says which side the hairline is on.
#[allow(clippy::too_many_arguments)]
pub fn paint(soft: &Soft, fb: &mut [u32], (x0, y0, w, h): Rect, grid_on_right: bool, title: &str, entries: &[Entry], groups: &[GroupItem], items: &Items, look: Look, scratch: &mut Vec<u8>) {
    let (fw, fh) = soft.fb_size();
    let (ox, oy) = soft.origin();
    if fb.len() < fw * fh {
        return;
    }
    let lay = Layout::Side { x0, y0, w, entries };
    let bottom = y0 + h;
    let bcw = soft.small(0.72).cell_w as i32;
    // While something is dragged the pointer lights nothing: the marks say where it would go.
    let hover = if look.drag.is_some() { None } else { look.hover };
    let ink = |over, cross_hot, group, fade| Ink { over, cross_hot, group, bcw, phase: look.phase, fade };
    let fade_of = |e: Entry| if is_grab(e, look.drag) { DRAGGED } else { 1.0 };
    {
        let mut p = Paint { fb: &mut *fb, w: fw, h: fh, ox: ox as i32, oy: oy as i32, fade: 1.0 };
        // The hairline on the side of the grid.
        let hx = if grid_on_right { x0 + w - 1 } else { x0 };
        for y in y0..bottom {
            p.put(hx, y, WHITE, 0.07);
        }
        plus(&mut p, lay.new_box(), hover == Some(Part::New));
        if let Some(b) = lay.group_box() {
            group_button(&mut p, b, hover == Some(Part::NewGroup));
        }
        for (k, e) in entries.iter().enumerate() {
            let b = lay.rect(k);
            if b.1 + b.3 > bottom {
                break;
            }
            match *e {
                Entry::Head(g) => head_shapes(&mut p, b, &groups[g], hover == Some(Part::Head(g)), look.phase, fade_of(*e)),
                Entry::Row { win, grouped } => {
                    let group = if grouped { Grouping::Bar(group_of(entries, k, groups)) } else { Grouping::No };
                    row_shapes(&mut p, b, &items.win(win), ink(over_row(hover, win), hover == Some(Part::Close(win)), group, fade_of(*e)));
                }
                Entry::Pane { win, idx, grouped } => {
                    pane_shapes(&mut p, b, &items.pane(win, idx), over_pane(hover, win, idx), hover == Some(Part::ClosePane(win, idx)), grouped.then_some(group_of(entries, k, groups)), look.phase)
                }
            }
        }
    }
    let m = soft.small(0.85);
    let head = fit(title, ((w - 24) / m.cell_w as i32).max(1) as usize);
    let f = palette::FAINT;
    soft.paint_text(fb, ox + (x0 + 16) as usize, oy + (y0 + HEAD_H / 2 + 2) as usize, head, pack(f.r, f.g, f.b, 255), false, &m, scratch);
    for (k, e) in entries.iter().enumerate() {
        let b = lay.rect(k);
        if b.1 + b.3 > bottom {
            break;
        }
        match *e {
            Entry::Head(g) => head_text(soft, fb, b, &groups[g], fade_of(*e), scratch),
            Entry::Row { win, .. } => row_text(soft, fb, b, &items.win(win), over_row(hover, win), fade_of(*e), scratch),
            Entry::Pane { win, idx, .. } => pane_text(soft, fb, b, &items.pane(win, idx), over_pane(hover, win, idx), scratch),
        }
    }
    paint_drag(soft, fb, lay, entries, groups, items, look, scratch);
}

/// The colour of the group that the window entry `k` belongs to: that of the last header before it.
fn group_of(entries: &[Entry], k: usize, groups: &[GroupItem]) -> u8 {
    entries[..k].iter().rev().find_map(|e| if let Entry::Head(g) = e { Some(groups[*g].color) } else { None }).unwrap_or(0)
}

/// How wide a character of a group's chip is at the top: the grid's font at 0.85 of its size.
pub fn chip_cw(soft: &Soft) -> i32 {
    soft.small(0.85).cell_w as i32
}

/// Paints the tabs in the title bar between window pixels `x0` and `x1`, over a cleared bar: each group in a
/// container tinted with its colour, with its chip and tabs, the loose windows' tabs, the `+`.
#[allow(clippy::too_many_arguments)]
pub fn paint_top(soft: &Soft, fb: &mut [u32], (x0, x1): (i32, i32), entries: &[Entry], groups: &[GroupItem], items: &Items, look: Look, scratch: &mut Vec<u8>) {
    let (fw, fh) = soft.fb_size();
    let (ox, oy) = soft.origin();
    if fb.len() < fw * fh {
        return;
    }
    let lay = Layout::Top { x0, x1, entries, groups, cw: chip_cw(soft) };
    let bcw = soft.small(0.72).cell_w as i32;
    let hover = if look.drag.is_some() { None } else { look.hover };
    let ink = |over, cross_hot, group, fade| Ink { over, cross_hot, group, bcw, phase: look.phase, fade };
    let fade_of = |e: Entry| if is_grab(e, look.drag) { DRAGGED } else { 1.0 };
    {
        let mut p = Paint { fb: &mut *fb, w: fw, h: fh, ox: ox as i32, oy: oy as i32, fade: 1.0 };
        lay.containers(&mut |g, r| {
            if lay.shown(r) {
                p.rrect(r.0, r.1, r.2, r.3, 11.0, group_color(groups[g].color), 0.06);
            }
        });
        for (k, e) in entries.iter().enumerate() {
            let b = lay.rect(k);
            if !lay.shown(b) {
                continue;
            }
            match *e {
                Entry::Head(g) => chip_shapes(&mut p, b, &groups[g], hover == Some(Part::Head(g)), look.phase, fade_of(*e)),
                Entry::Row { win, grouped } => {
                    let group = if grouped { Grouping::Tint(group_of(entries, k, groups)) } else { Grouping::No };
                    row_shapes(&mut p, b, &items.win(win), ink(over_row(hover, win), hover == Some(Part::Close(win)), group, fade_of(*e)));
                }
                Entry::Pane { .. } => {}
            }
        }
        plus(&mut p, lay.new_box(), hover == Some(Part::New));
    }
    for (k, e) in entries.iter().enumerate() {
        let b = lay.rect(k);
        if !lay.shown(b) {
            continue;
        }
        match *e {
            Entry::Head(g) => chip_text(soft, fb, b, &groups[g], hover == Some(Part::Head(g)), fade_of(*e), scratch),
            Entry::Row { win, .. } => row_text(soft, fb, b, &items.win(win), over_row(hover, win), fade_of(*e), scratch),
            Entry::Pane { .. } => {}
        }
    }
    paint_drag(soft, fb, lay, entries, groups, items, look, scratch);
}

// ------------------------------------------------------------------ rename --

/// A window's name being edited in its row (the reference's `.t-edit`): the text, the caret and the selection
/// (characters). The row is painted with no title and `paint_rename` puts the field over it.
pub struct Edit<'a> {
    pub text: &'a str,
    pub caret: usize,
    pub sel: Option<(usize, usize)>,
}

/// The field's height, and how far its text is from its left edge (1 px of border and 6 of padding).
const EDIT_H: i32 = 22;
pub const EDIT_PAD: i32 = 7;

/// The field in a row's box `b`: where the title was, up to where the cross is (`flex: 1`).
fn edit_box((x, y, w, h): Rect) -> Rect {
    let left = 9 + 13 + 7;
    (x + left, y + (h - EDIT_H) / 2, (w - left - 5 - CLOSE - 7).max(2 * EDIT_PAD), EDIT_H)
}

/// How many characters of `cw` pixels fit in a field `w` pixels wide: the text's own padding on the left and a
/// hairline of room on the right, where a caret after the last character sits.
pub fn edit_cols(w: i32, cw: i32) -> usize {
    ((w - EDIT_PAD - 2) / cw.max(1)).max(1) as usize
}

/// The first character shown when the caret is at `caret` and `cols` fit: the text scrolls just far enough to
/// keep the caret in view (a function of the caret, nothing is stored).
pub fn edit_first(caret: usize, cols: usize) -> usize {
    (caret + 1).saturating_sub(cols)
}

impl Layout<'_> {
    /// The field of window `win`'s row or tab, when it is listed.
    pub fn edit_rect(self, win: usize) -> Option<Rect> {
        let k = self.entries().iter().position(|e| matches!(e, Entry::Row { win: w, .. } if *w == win))?;
        let r = self.rect(k);
        self.shown(r).then(|| edit_box(r))
    }
}

/// Paints the field over window `win`'s row or tab (painted with an empty title): a box with a hairline, the text in
/// the grid's font, the selection, a solid caret, and the row's cross, which stays while the field has the focus.
pub fn paint_rename(soft: &Soft, fb: &mut [u32], lay: Layout, win: usize, ed: &Edit, hover: Option<Part>, scratch: &mut Vec<u8>) {
    let (fw, fh) = soft.fb_size();
    let (ox, oy) = soft.origin();
    let Some(b) = lay.edit_rect(win) else { return };
    if fb.len() < fw * fh {
        return;
    }
    let m = soft.small(0.85);
    let cw = m.cell_w as i32;
    let cols = edit_cols(b.2, cw);
    let first = edit_first(ed.caret, cols);
    let cy = b.1 + b.3 / 2;
    let ch = (m.cell_h as i32).min(b.3 - 6);
    {
        let mut p = Paint { fb: &mut *fb, w: fw, h: fh, ox: ox as i32, oy: oy as i32, fade: 1.0 };
        p.rrect(b.0, b.1, b.2, b.3, 6.0, [0.0; 3], 0.45);
        p.outline(b.0, b.1, b.2, b.3, 6.0, WHITE, 0.25);
        if let Some((a, z)) = ed.sel {
            let (a, z) = (a.saturating_sub(first).min(cols), z.saturating_sub(first).min(cols));
            if z > a {
                p.rrect(b.0 + EDIT_PAD + a as i32 * cw, cy - ch / 2, (z - a) as i32 * cw, ch, 2.0, palette::unit(palette::ACCENT), 0.30);
            }
        }
        let x = b.0 + EDIT_PAD + (ed.caret - first.min(ed.caret)) as i32 * cw;
        p.shape(x, cy - ch / 2, x + 1, cy - ch / 2 + ch, palette::unit(palette::FG), 1.0, &|_, _| 1.0);
        if !over_row(hover, win) {
            if let Some(k) = lay.entries().iter().position(|e| matches!(e, Entry::Row { win: w, .. } if *w == win)) {
                cross(&mut p, close_box(lay.rect(k)), false);
            }
        }
    }
    let shown: String = ed.text.chars().skip(first).take(cols).collect();
    let c = palette::FG;
    soft.paint_text(fb, ox + (b.0 + EDIT_PAD) as usize, oy + cy as usize, shown.chars(), pack(c.r, c.g, c.b, 255), false, &m, scratch);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(n: usize) -> Vec<Entry> {
        (0..n).map(|win| Entry::Row { win, grouped: false }).collect()
    }

    #[test]
    fn rows_and_the_plus_are_found_where_they_are_drawn() {
        let e = rows(3);
        let lay = Layout::Side { x0: 0, y0: 42, w: 220, entries: &e };
        let n = lay.rect(1);
        assert_eq!(lay.hit((n.0 + 10) as f32, (n.1 + 5) as f32), Some(Part::Row(1)));
        let c = close_box(n);
        assert_eq!(lay.hit((c.0 + 3) as f32, (c.1 + 3) as f32), Some(Part::Close(1)));
        let b = lay.new_box();
        assert_eq!(lay.hit((b.0 + 3) as f32, (b.1 + 3) as f32), Some(Part::New));
        // The group button is the right-most of the two, 2 px from the `+`, the same size; the tabs have none.
        let g = lay.group_box().unwrap();
        assert_eq!(lay.hit((g.0 + 3) as f32, (g.1 + 3) as f32), Some(Part::NewGroup));
        assert_eq!((g.0 - (b.0 + b.2), g.1, g.2, g.3), (2, b.1, b.2, b.3));
        assert_eq!(g.0 + g.2, 220 - 6, "6 px from the panel's edge");
        assert!(Layout::Top { x0: 118, x1: 900, entries: &e, groups: &[], cw: 8 }.group_box().is_none());
        // The margin, and rows that are not there.
        assert_eq!(lay.hit(2.0, (n.1 + 5) as f32), None);
        let few = rows(1);
        let lay = Layout::Side { x0: 0, y0: 42, w: 220, entries: &few };
        assert_eq!(lay.hit((n.0 + 10) as f32, (n.1 + 5) as f32), None);
    }

    #[test]
    fn groups_list_their_windows_after_the_header_and_the_rest_last() {
        // Windows 0 and 2 in group 0, 1 and 4 in group 1 (folded), 3 in none.
        let wg = [Some(0), Some(1), Some(0), None, Some(1)];
        let e = entries(&wg, &[false, true], &[1; 5], None);
        assert_eq!(
            e,
            [Entry::Head(0), Entry::Row { win: 0, grouped: true }, Entry::Row { win: 2, grouped: true }, Entry::Head(1), Entry::Row { win: 3, grouped: false }]
        );
        assert_eq!(entries(&[None, None], &[], &[1, 1], None), rows(2));
        assert_eq!(entries(&[], &[false], &[], None), [Entry::Head(0)], "an empty group still has its header");
    }

    #[test]
    fn a_window_with_several_panes_is_followed_by_a_row_for_each() {
        // Window 0 (in group 0) has three panes, window 1 (in none) two, window 2 (in none) one.
        let e = entries(&[Some(0), None, None], &[false], &[3, 2, 1], None);
        assert_eq!(
            e,
            [
                Entry::Head(0),
                Entry::Row { win: 0, grouped: true },
                Entry::Pane { win: 0, idx: 0, grouped: true },
                Entry::Pane { win: 0, idx: 1, grouped: true },
                Entry::Pane { win: 0, idx: 2, grouped: true },
                Entry::Row { win: 1, grouped: false },
                Entry::Pane { win: 1, idx: 0, grouped: false },
                Entry::Pane { win: 1, idx: 1, grouped: false },
                Entry::Row { win: 2, grouped: false },
            ]
        );
        // A folded group hides its windows and so their panes.
        assert_eq!(entries(&[Some(0), None], &[true], &[4, 1], None), [Entry::Head(0), Entry::Row { win: 1, grouped: false }]);
        // ... except the window on screen, with its panes.
        assert_eq!(
            entries(&[Some(0), Some(0), None], &[true], &[2, 1, 1], Some(0)),
            [Entry::Head(0), Entry::Row { win: 0, grouped: true }, Entry::Pane { win: 0, idx: 0, grouped: true }, Entry::Pane { win: 0, idx: 1, grouped: true }, Entry::Row { win: 2, grouped: false }]
        );
    }

    #[test]
    fn pane_rows_are_indented_and_spaced_like_the_reference_and_found_where_drawn() {
        let e = entries(&[Some(0), None], &[false], &[2, 2], None);
        let lay = Layout::Side { x0: 0, y0: 42, w: 220, entries: &e };
        // [Head, Row 0, Pane 0.0, Pane 0.1, Row 1, Pane 1.0, Pane 1.1]
        let (win, p0, p1, free, q0) = (lay.rect(1), lay.rect(2), lay.rect(3), lay.rect(4), lay.rect(5));
        assert_eq!((p0.3, p1.3), (PANE_H, PANE_H));
        assert_eq!(p0.1, win.1 + ROW_H + GAP);
        assert_eq!(p1.1, p0.1 + PANE_H + GAP);
        // In a group the row is 24 px in from the panel's padding, 16 px right of the window's bar; a loose
        // window's panes are 22 px in.
        assert_eq!(p0.0, PAD + PANE_IND_GROUPED);
        assert_eq!(p0.0 - win.0, 16);
        assert_eq!(q0.0, PAD + PANE_IND);
        // The window that is in no group is set apart from the group's last pane by the group gap.
        assert_eq!(free.1, p1.1 + PANE_H + GAP + GROUP_GAP);
        // Rows, crosses and windows are told apart.
        assert_eq!(lay.hit((p0.0 + 30) as f32, (p0.1 + 5) as f32), Some(Part::Pane(0, 0)));
        assert_eq!(lay.hit((p1.0 + 30) as f32, (p1.1 + 5) as f32), Some(Part::Pane(0, 1)));
        let c = pane_close_box(q0);
        assert_eq!(lay.hit((c.0 + 3) as f32, (c.1 + 3) as f32), Some(Part::ClosePane(1, 0)));
        assert_eq!(lay.hit((free.0 + 30) as f32, (free.1 + 5) as f32), Some(Part::Row(1)));
    }

    #[test]
    fn a_groups_header_windows_and_the_windows_in_none_are_spaced_like_the_reference() {
        let wg = [Some(0), Some(0), None];
        let e = entries(&wg, &[false], &[1, 1, 1], None);
        let lay = Layout::Side { x0: 0, y0: 42, w: 220, entries: &e };
        let (head, a, b, free) = (lay.rect(0), lay.rect(1), lay.rect(2), lay.rect(3));
        assert_eq!(head.1, 42 + HEAD_H + GROUP_GAP);
        assert_eq!((head.3, a.3), (GROUP_H, ROW_H));
        assert_eq!(a.1, head.1 + GROUP_H + GAP);
        assert_eq!(b.1, a.1 + ROW_H + GAP);
        assert_eq!(free.1, b.1 + ROW_H + GAP + GROUP_GAP);
        // The group's windows are indented, the others are not.
        assert_eq!((a.0 - head.0, a.2 - head.2), (INDENT, -INDENT));
        assert_eq!(free.0, head.0);
        // A header is found as a header, its windows as rows.
        assert_eq!(lay.hit((head.0 + 30) as f32, (head.1 + 5) as f32), Some(Part::Head(0)));
        assert_eq!(lay.hit((a.0 + 30) as f32, (a.1 + 5) as f32), Some(Part::Row(0)));
    }

    fn group(name: &str, count: usize) -> GroupItem {
        GroupItem { id: 0, name: name.into(), color: 1, collapsed: false, count, busy: false }
    }

    #[test]
    fn tabs_share_the_bar_and_shrink_down_to_a_least_width() {
        let e = rows(3);
        let wide = Layout::Top { x0: 118, x1: 900, entries: &e, groups: &[], cw: 8 };
        assert_eq!(wide.rect(0).2, TAB_W);
        let (a, b) = (wide.rect(0), wide.rect(1));
        assert_eq!(b.0, a.0 + a.2 + TAB_GAP);
        // The `+` follows the last tab; everything fits the bar.
        let plus = wide.new_box();
        assert_eq!(plus.0, wide.rect(2).0 + wide.rect(2).2 + TAB_GAP);
        assert!(plus.0 + plus.2 <= 900);
        // Eight tabs in a narrow bar: narrower, not narrower than the least.
        let e8 = rows(8);
        let tight = Layout::Top { x0: 118, x1: 700, entries: &e8, groups: &[], cw: 8 };
        assert!(tight.rect(0).2 < TAB_W && tight.rect(0).2 >= TAB_MIN);
        // Tabs sit inside the title bar, above its hairline.
        assert!(a.1 >= 0 && a.1 + a.3 <= TITLE_H as i32 - 1);
        assert_eq!(wide.hit((a.0 + 20) as f32, (a.1 + 10) as f32), Some(Part::Row(0)));
        assert_eq!(wide.hit((a.0 + 20) as f32, 1.0), None);
    }

    #[test]
    fn the_rename_field_sits_where_the_title_was_and_stops_before_the_cross() {
        let e = entries(&[Some(0), None], &[false], &[1, 1], None);
        let lay = Layout::Side { x0: 0, y0: 42, w: 220, entries: &e };
        // [Head 0, Row 0 (grouped), Row 1]: window 1 is entry 2.
        let (row, f) = (lay.rect(2), lay.edit_rect(1).unwrap());
        assert_eq!((f.0 - row.0, f.3), (9 + 13 + 7, 22), "after the icon, 22 px tall as the reference's input");
        assert_eq!(f.1 - row.1, (ROW_H - 22) / 2, "centred in the row");
        assert!(f.0 + f.2 <= close_box(row).0, "it ends before the cross");
        assert_eq!(f.2, 220 - 2 * PAD - 29 - 30, "the reference's flex: 1 between the icon and the cross");
        // A window the panel does not list (folded away) has no field; nor has one past the tabs.
        let folded = entries(&[Some(0), None], &[true], &[1, 1], None);
        assert!(Layout::Side { x0: 0, y0: 42, w: 220, entries: &folded }.edit_rect(0).is_none());
        let tabs = rows(2);
        let top = Layout::Top { x0: 118, x1: 900, entries: &tabs, groups: &[], cw: 8 };
        assert!(top.edit_rect(1).is_some() && top.edit_rect(2).is_none());
        let t = top.rect(1);
        assert_eq!(top.edit_rect(1).unwrap().0, t.0 + 29);
    }

    #[test]
    fn the_rename_text_scrolls_just_far_enough_to_keep_the_caret_in_view() {
        // A field 120 px wide with 8 px characters: (120 - 7 - 2) / 8 = 13 columns.
        assert_eq!(edit_cols(120, 8), 13);
        assert_eq!(edit_cols(78, 7), 9, "a name of eight letters fits a narrow tab with its caret after it, with no scrolling");
        assert_eq!(edit_first(8, 9), 0);
        assert_eq!(edit_cols(5, 8), 1, "never none");
        assert_eq!(edit_first(0, 13), 0);
        assert_eq!(edit_first(12, 13), 0, "the caret at the last column: no scroll yet");
        assert_eq!(edit_first(13, 13), 1, "one past: scrolled by one");
        assert_eq!(edit_first(20, 13), 8);
        // The caret is always inside the window it asks for.
        for caret in 0..60 {
            let first = edit_first(caret, 13);
            assert!(caret >= first && caret - first < 13, "{caret}");
        }
    }

    #[test]
    fn a_folded_group_at_the_top_shows_only_the_window_on_screen_if_it_is_its_own() {
        let wg = [Some(0), Some(0), Some(1), None];
        // Group 0 is open, group 1 is folded and its window is not the one on screen.
        let open = top_entries(&wg, &[false, true], 3);
        assert_eq!(open, [Entry::Head(0), Entry::Row { win: 0, grouped: true }, Entry::Row { win: 1, grouped: true }, Entry::Head(1), Entry::Row { win: 3, grouped: false }]);
        // The window on screen is in the folded group: it stays, the other windows of that group go.
        let wg = [Some(0), Some(0), None];
        assert_eq!(top_entries(&wg, &[true], 1), [Entry::Head(0), Entry::Row { win: 1, grouped: true }, Entry::Row { win: 2, grouped: false }]);
        assert_eq!(top_entries(&wg, &[true], 2), [Entry::Head(0), Entry::Row { win: 2, grouped: false }]);
        // No panes are listed: a tab says how many there are.
        assert_eq!(top_entries(&[None, None], &[], 0), rows(2));
    }

    #[test]
    fn a_group_at_the_top_is_a_container_with_a_chip_and_its_tabs_like_the_reference() {
        // A group of two windows and one loose window.
        let wg = [Some(0), Some(0), None];
        let e = top_entries(&wg, &[false], 0);
        let groups = [group("arcton", 3)];
        let lay = Layout::Top { x0: 118, x1: 900, entries: &e, groups: &groups, cw: 8 };
        let (chip, a, b, loose) = (lay.rect(0), lay.rect(1), lay.rect(2), lay.rect(3));
        // 9 px of padding, an 8 px dot, 6 px gaps, the name and the count: 9 + 8 + 6 + 6 * 8 + 6 + 8 + 9.
        assert_eq!((chip.2, chip.3), (CHIP_FIXED + 7 * 8, CHIP_H));
        // The container has 2 px of padding round the chip and the tabs, 2 px between them.
        let mut boxes = Vec::new();
        lay.containers(&mut |g, r| boxes.push((g, r)));
        let (g, c) = boxes[0];
        assert_eq!(g, 0);
        assert_eq!((c.0 + GROUP_PAD, c.1 + GROUP_PAD, c.3), (chip.0, a.1, TAB_H + 2 * GROUP_PAD));
        assert_eq!(a.0, chip.0 + chip.2 + GROUP_INNER);
        assert_eq!(b.0, a.0 + a.2 + GROUP_INNER);
        assert_eq!(c.0 + c.2, b.0 + b.2 + GROUP_PAD);
        // The chip is centred on the tabs; the loose tab is 4 px after the container.
        assert_eq!(chip.1 + chip.3 / 2, a.1 + a.3 / 2);
        assert_eq!(loose.0, c.0 + c.2 + TAB_GAP);
        // Everything is found where it is drawn: the chip is the group's header, the tabs its windows.
        assert_eq!(lay.hit((chip.0 + 20) as f32, (chip.1 + 5) as f32), Some(Part::Head(0)));
        assert_eq!(lay.hit((a.0 + 20) as f32, (a.1 + 5) as f32), Some(Part::Row(0)));
        assert_eq!(lay.hit((b.0 + 20) as f32, (b.1 + 5) as f32), Some(Part::Row(1)));
        assert_eq!(lay.hit((loose.0 + 20) as f32, (loose.1 + 5) as f32), Some(Part::Row(2)));
        // The padding of the container is nothing; the `+` follows the loose tab.
        assert_eq!(lay.hit((c.0 + 1) as f32, (c.1 + 1) as f32), None);
        assert_eq!(lay.new_box().0, loose.0 + loose.2 + TAB_GAP);
    }

    #[test]
    fn the_tabs_make_room_for_the_chips_and_the_containers() {
        let wg = [Some(0), Some(0), Some(1), None];
        let e = top_entries(&wg, &[false, false], 0);
        let groups = [group("arcton", 3), group("docs", 1)];
        // [Head 0, Row 0, Row 1, Head 1, Row 2, Row 3]
        for x1 in [500, 700, 900, 1400] {
            let lay = Layout::Top { x0: 118, x1, entries: &e, groups: &groups, cw: 8 };
            let plus = lay.new_box();
            let last = lay.rect(e.len() - 1);
            assert!(plus.0 + plus.2 <= x1, "{x1}: the + fits");
            if last.0 + last.2 + TAB_GAP <= plus.0 {
                assert_eq!(plus.0, last.0 + last.2 + TAB_GAP, "{x1}");
            }
            // Every tab is the same width, between the least and the natural.
            let w = lay.rect(1).2;
            assert!((TAB_MIN..=TAB_W).contains(&w), "{x1}");
            assert_eq!([lay.rect(2).2, lay.rect(4).2, lay.rect(5).2], [w; 3], "{x1}");
        }
        // As the bar narrows the tabs do: with room for them all they are the natural width.
        let narrow = Layout::Top { x0: 118, x1: 500, entries: &e, groups: &groups, cw: 8 };
        let roomy = Layout::Top { x0: 118, x1: 1400, entries: &e, groups: &groups, cw: 8 };
        assert!(narrow.rect(1).2 < roomy.rect(1).2);
        assert_eq!(roomy.rect(1).2, TAB_W);
    }

    #[test]
    fn what_does_not_fit_the_bar_is_left_out_and_the_plus_stays_in_reach() {
        let e = rows(20);
        let lay = Layout::Top { x0: 118, x1: 600, entries: &e, groups: &[], cw: 8 };
        let plus = lay.new_box();
        assert_eq!(plus.0 + plus.2, 600, "the + is pinned to the end of the bar");
        // The tabs after the last one that fits are not found, nor drawn.
        let seen = (0..e.len()).filter(|&k| lay.shown(lay.rect(k))).count();
        assert!(seen > 0 && seen < 20);
        let off = lay.rect(19);
        assert_eq!(lay.hit((off.0 + 10) as f32, (off.1 + 5) as f32), None);
        assert_eq!(lay.hit((plus.0 + 5) as f32, (plus.1 + 5) as f32), Some(Part::New));
        let last = lay.rect(seen - 1);
        assert!(last.0 + last.2 + TAB_GAP <= plus.0);
    }

    #[test]
    fn long_names_make_the_chip_at_most_180_px() {
        let long = group("a very very long group name indeed", 12);
        assert_eq!(chip_w(&long, 8), CHIP_FIXED + (chip_chars(&long, 8) + 2) * 8);
        assert!(chip_w(&long, 8) <= CHIP_MAX);
        assert!(chip_w(&long, 8) > CHIP_MAX - 8, "it takes what it is given");
        assert_eq!(chip_w(&group("x", 1), 8), CHIP_FIXED + 2 * 8);
        // A wide font still shows a character of the name.
        assert!(chip_chars(&long, 40) >= 1);
    }

    #[test]
    fn a_status_is_the_busiest_of_the_panes() {
        use Status::*;
        assert_eq!(Status::of([]), Idle);
        assert_eq!(Status::of([Idle, Idle]), Idle);
        assert_eq!(Status::of([Idle, Failed]), Failed);
        assert_eq!(Status::of([Failed, Running, Idle]), Running, "a command running says more than a failed one");
        assert_eq!(Status::of([Running]), Running);
    }

    #[test]
    fn the_spinner_turns_once_in_800_ms() {
        assert_eq!(spin_phase(0), 0.0);
        assert_eq!(spin_phase(400), 0.5);
        assert_eq!(spin_phase(800), 0.0);
        assert!((spin_phase(1_000_200) - 0.25).abs() < 1e-6);
    }

    // ------------------------------------------------------------ dragging --

    /// The panel's rows for a group of two windows (0, 1, the second with two panes), a folded group (window 2,
    /// hidden) and two loose windows (3, 4).
    fn panel() -> Vec<Entry> {
        entries(&[Some(0), Some(0), Some(1), None, None], &[false, true], &[1, 2, 1, 1, 1], None)
    }

    #[test]
    fn a_window_over_a_row_goes_before_or_after_it_by_the_half_the_pointer_is_in() {
        let e = panel();
        let lay = Layout::Side { x0: 0, y0: 42, w: 220, entries: &e };
        // [Head 0, Row 0, Row 1, Pane 1.0, Pane 1.1, Head 1, Row 3, Row 4]
        let r3 = lay.rect(6);
        let (x, up) = ((r3.0 + 20) as f32, (r3.1 + 3) as f32);
        assert_eq!(lay.target(Grab::Win(4), x, up), Some(Target::Near { win: 3, after: false }));
        assert_eq!(lay.target(Grab::Win(4), x, (r3.1 + r3.3 - 3) as f32), Some(Target::Near { win: 3, after: true }));
        // Over itself: nowhere.
        assert_eq!(lay.target(Grab::Win(3), x, up), None);
        // A window of several panes is one block: over its pane rows it is the block's half that counts.
        let (row1, p0, p1) = (lay.rect(2), lay.rect(3), lay.rect(4));
        // The block is 30 + 26 + 26 px: the first pane's row is in its upper half, the second's in its lower.
        assert_eq!(lay.target(Grab::Win(4), x, (p0.1 + 2) as f32), Some(Target::Near { win: 1, after: false }));
        assert_eq!(lay.target(Grab::Win(4), x, (p1.1 + p1.3 - 2) as f32), Some(Target::Near { win: 1, after: true }));
        assert_eq!(lay.target(Grab::Win(4), x, (row1.1 + 2) as f32), Some(Target::Near { win: 1, after: false }));
        // The gap between two rows belongs to the one after it, so there is always a target.
        let gap = (r3.1 + r3.3) as f32;
        assert_eq!(lay.target(Grab::Win(0), x, gap + 0.5), Some(Target::Near { win: 4, after: false }));
    }

    #[test]
    fn a_window_over_a_header_goes_into_the_group_and_on_the_empty_part_out_of_all() {
        let e = panel();
        let lay = Layout::Side { x0: 0, y0: 42, w: 220, entries: &e };
        let (h0, h1) = (lay.rect(0), lay.rect(5));
        assert_eq!(lay.target(Grab::Win(3), (h0.0 + 30) as f32, (h0.1 + 5) as f32), Some(Target::Into(0)));
        assert_eq!(lay.target(Grab::Win(3), (h1.0 + 30) as f32, (h1.1 + 5) as f32), Some(Target::Into(1)));
        // Below the last row: out of every group.
        let last = lay.rect(7);
        assert_eq!(lay.target(Grab::Win(0), 30.0, (last.1 + last.3 + 40) as f32), Some(Target::End));
        // Off the panel: nothing; in its header: nothing.
        assert_eq!(lay.target(Grab::Win(0), 300.0, (h0.1 + 5) as f32), None);
        assert_eq!(lay.target(Grab::Win(0), 30.0, 50.0), None);
        // A group is not dropped on the empty part.
        assert_eq!(lay.target(Grab::Group(0), 30.0, (last.1 + last.3 + 40) as f32), None);
    }

    #[test]
    fn a_group_over_another_goes_before_or_after_it_and_over_a_loose_window_after_the_last() {
        let e = panel();
        let lay = Layout::Side { x0: 0, y0: 42, w: 220, entries: &e };
        let (h0, row0, h1, loose) = (lay.rect(0), lay.rect(1), lay.rect(5), lay.rect(6));
        let x = (h1.0 + 30) as f32;
        assert_eq!(lay.target(Grab::Group(0), x, (h1.1 + 2) as f32), Some(Target::Beside { group: 1, after: false }));
        assert_eq!(lay.target(Grab::Group(0), x, (h1.1 + h1.3 - 2) as f32), Some(Target::Beside { group: 1, after: true }));
        // Over a window of group 0: the group 0's block, by the half of that window.
        assert_eq!(lay.target(Grab::Group(1), (row0.0 + 30) as f32, (row0.1 + 2) as f32), Some(Target::Beside { group: 0, after: false }));
        assert_eq!(lay.target(Grab::Group(1), (row0.0 + 30) as f32, (row0.1 + row0.3 - 2) as f32), Some(Target::Beside { group: 0, after: true }));
        // Over itself: nowhere. Over a loose window: after the last group, which is where it already is for the last.
        assert_eq!(lay.target(Grab::Group(0), (h0.0 + 30) as f32, (h0.1 + 5) as f32), None);
        assert_eq!(lay.target(Grab::Group(0), (loose.0 + 30) as f32, (loose.1 + 5) as f32), Some(Target::Beside { group: 1, after: true }));
        assert_eq!(lay.target(Grab::Group(1), (loose.0 + 30) as f32, (loose.1 + 5) as f32), None);
    }

    #[test]
    fn at_the_top_a_window_goes_by_the_half_of_the_tab_and_into_a_chip() {
        let wg = [Some(0), Some(0), None, None];
        let e = top_entries(&wg, &[false], 0);
        let groups = [group("arcton", 2)];
        let lay = Layout::Top { x0: 118, x1: 900, entries: &e, groups: &groups, cw: 8 };
        // [Head 0, Row 0, Row 1, Row 2, Row 3]
        let (chip, a, loose) = (lay.rect(0), lay.rect(1), lay.rect(4));
        let y = (a.1 + 10) as f32;
        assert_eq!(lay.target(Grab::Win(3), (chip.0 + 10) as f32, y), Some(Target::Into(0)));
        assert_eq!(lay.target(Grab::Win(3), (a.0 + 10) as f32, y), Some(Target::Near { win: 0, after: false }));
        assert_eq!(lay.target(Grab::Win(3), (a.0 + a.2 - 10) as f32, y), Some(Target::Near { win: 0, after: true }));
        assert_eq!(lay.target(Grab::Win(0), (a.0 + 10) as f32, y), None, "over itself");
        // Over the window at the far end, then the empty part, the `+` included: out of every group.
        assert_eq!(lay.target(Grab::Win(0), (loose.0 + 10) as f32, y), Some(Target::Near { win: 3, after: false }));
        let plus = lay.new_box();
        assert_eq!(lay.target(Grab::Win(0), (plus.0 + 5) as f32, y), Some(Target::End));
        assert_eq!(lay.target(Grab::Win(0), 890.0, y), Some(Target::End));
        // A group over a loose tab: after the last group, where the only one is already; off the bar: nothing.
        assert_eq!(lay.target(Grab::Group(0), (loose.0 + 10) as f32, y), None);
        assert_eq!(lay.target(Grab::Win(3), (a.0 + 10) as f32, 60.0), None, "below the bar");
        assert_eq!(lay.target(Grab::Win(3), 50.0, y), None, "left of the tabs");
        assert_eq!(lay.target(Grab::Win(3), 920.0, y), None, "past the bar");
    }

    #[test]
    fn marks_sit_on_the_edge_the_drop_would_use() {
        let e = panel();
        let lay = Layout::Side { x0: 0, y0: 42, w: 220, entries: &e };
        let (row, loose, h1) = (lay.rect(6), lay.rect(7), lay.rect(5));
        // A line 2 px thick along the row's top or bottom edge, outside it.
        assert_eq!(lay.mark(Target::Near { win: 3, after: false }), Some(Mark::Line((row.0, row.1 - MARK, row.2, MARK))));
        assert_eq!(lay.mark(Target::Near { win: 3, after: true }), Some(Mark::Line((row.0, row.1 + row.3, row.2, MARK))));
        // A window with panes: after its last pane's row.
        let (w1, p1) = (lay.rect(2), lay.rect(4));
        assert_eq!(lay.mark(Target::Near { win: 1, after: true }), Some(Mark::Line((w1.0, p1.1 + p1.3, w1.2, MARK))));
        // A ring round the header; its colour is the group's.
        assert_eq!(lay.mark(Target::Into(1)), Some(Mark::Ring(h1, 1)));
        // A group: its block, from the header to its last window's last pane, as wide as the header.
        let h0 = lay.rect(0);
        assert_eq!(lay.mark(Target::Beside { group: 0, after: false }), Some(Mark::Line((h0.0, h0.1 - MARK, h0.2, MARK))));
        assert_eq!(lay.mark(Target::Beside { group: 0, after: true }), Some(Mark::Line((h0.0, p1.1 + p1.3, h0.2, MARK))));
        // The end of the list is below the last row.
        let Some(Mark::Line((_, y, _, _))) = lay.mark(Target::End) else { panic!() };
        assert!(y > loose.1 + loose.3);
        // At the top the lines are vertical, along the tab's side; the ring is round the chip.
        let wg = [Some(0), None];
        let te = top_entries(&wg, &[false], 0);
        let groups = [group("g", 1)];
        let top = Layout::Top { x0: 118, x1: 900, entries: &te, groups: &groups, cw: 8 };
        let (chip, tab, free) = (top.rect(0), top.rect(1), top.rect(2));
        assert_eq!(top.mark(Target::Near { win: 1, after: false }), Some(Mark::Line((free.0 - MARK, free.1, MARK, free.3))));
        assert_eq!(top.mark(Target::Near { win: 0, after: true }), Some(Mark::Line((tab.0 + tab.2, tab.1, MARK, tab.3))));
        assert_eq!(top.mark(Target::Into(0)), Some(Mark::Ring(chip, 0)));
        let mut c = (0, (0, 0, 0, 0));
        top.containers(&mut |g, r| c = (g, r));
        assert_eq!(top.mark(Target::Beside { group: 0, after: false }), Some(Mark::Line((c.1.0 - MARK, c.1.1, MARK, c.1.3))));
        // A window that is not listed has no mark.
        assert_eq!(top.mark(Target::Near { win: 9, after: true }), None);
    }

    #[test]
    fn a_drag_from_a_press_to_a_drop_does_what_its_mark_promised() {
        // Group 0 holds windows 0 and 1; 2 and 3 are loose. Drag window 3 over group 0's header: a ring, and
        // the drop puts it first in the group.
        let wg = [Some(0), Some(0), None, None];
        let e = entries(&wg, &[false], &[1; 4], None);
        let lay = Layout::Side { x0: 0, y0: 42, w: 220, entries: &e };
        let head = lay.rect(0);
        let to = lay.target(Grab::Win(3), (head.0 + 30) as f32, (head.1 + 5) as f32).unwrap();
        assert_eq!(lay.mark(to), Some(Mark::Ring(head, 0)));
        let a = crate::dnd::arrange(&wg, 1, Grab::Win(3), to).unwrap();
        assert_eq!((a.order.as_slice(), a.group.as_slice()), (&[3, 0, 1, 2][..], &[Some(0), Some(0), Some(0), None][..]));
        // What the panel lists after it: the new order, the window now under its header.
        let wg2 = a.group;
        let e2 = entries(&wg2, &[false], &[1; 4], None);
        assert_eq!(e2, [Entry::Head(0), Entry::Row { win: 0, grouped: true }, Entry::Row { win: 1, grouped: true }, Entry::Row { win: 2, grouped: true }, Entry::Row { win: 3, grouped: false }]);

        // At the top: drag the last tab before the second one; the mark is the line before it, and the window
        // takes its group.
        let te = top_entries(&wg, &[false], 0);
        let groups = [group("g", 2)];
        let top = Layout::Top { x0: 118, x1: 900, entries: &te, groups: &groups, cw: 8 };
        let tab = top.rect(2);
        let to = top.target(Grab::Win(3), (tab.0 + 5) as f32, (tab.1 + 5) as f32).unwrap();
        assert_eq!(to, Target::Near { win: 1, after: false });
        assert_eq!(top.mark(to), Some(Mark::Line((tab.0 - MARK, tab.1, MARK, tab.3))));
        let a = crate::dnd::arrange(&wg, 1, Grab::Win(3), to).unwrap();
        assert_eq!((a.order.as_slice(), a.group.as_slice()), (&[0, 3, 1, 2][..], &[Some(0), Some(0), Some(0), None][..]));
    }

    #[test]
    fn the_ghost_follows_the_pointer_inside_the_list() {
        let region = (10, 40, 200, 300);
        // Grabbed 20 px in and 5 down from the row's corner: the corner follows.
        assert_eq!(ghost_at(region, (100, 30), (100.0, 100.0), (20.0, 5.0)), (80, 95));
        // Kept inside, on every side.
        assert_eq!(ghost_at(region, (100, 30), (0.0, 0.0), (20.0, 5.0)), (10, 40));
        assert_eq!(ghost_at(region, (100, 30), (500.0, 900.0), (20.0, 5.0)), (110, 310));
        // A box as big as the region: it cannot move.
        assert_eq!(ghost_at((10, 40, 100, 30), (100, 30), (50.0, 90.0), (0.0, 0.0)), (10, 40));
    }

    #[test]
    fn what_is_grabbed_is_found_by_its_box() {
        let e = panel();
        let lay = Layout::Side { x0: 0, y0: 42, w: 220, entries: &e };
        assert_eq!(lay.grab_box(Grab::Win(3)), Some(lay.rect(6)));
        assert_eq!(lay.grab_box(Grab::Group(1)), Some(lay.rect(5)));
        // A window in a folded group is not listed.
        assert_eq!(lay.grab_box(Grab::Win(2)), None);
        assert_eq!(lay.grab_box(Grab::Group(7)), None);
    }

    #[test]
    fn titles_are_cut_with_an_ellipsis() {
        let cut = |t: &str, n| fit(t, n).collect::<String>();
        assert_eq!(cut("short", 10), "short");
        assert_eq!(cut("a very long title", 8), "a very …");
        assert_eq!(cut("abc", 1), "…");
        assert_eq!(cut("abc", 0), "…");
        assert_eq!(cut("abc", 3), "abc");
    }

    #[test]
    fn counts_are_written_without_a_string() {
        for n in [0, 1, 9, 10, 99, 100, 12345, usize::MAX] {
            assert_eq!(dec(n).collect::<String>(), n.to_string());
            assert_eq!(digits(n), n.to_string().len() as i32);
        }
    }

    #[test]
    fn the_items_are_read_back_as_they_were_pushed() {
        let mut it = Items::default();
        for round in 0..2 {
            // Filled again after a clear: the second round reuses what the first allocated.
            it.clear();
            it.push_win("one", true, false, 2, Status::Running);
            it.push_pane(true, Status::Running, |t| t.push_str("Pane 1"));
            it.push_pane(false, Status::Idle, |t| t.push_str("Pane 2 \u{b7} vim"));
            it.push_win("two", false, true, 0, Status::Idle);
            it.push_win("three", false, false, 3, Status::Failed);
            it.push_pane(false, Status::Failed, |t| t.push_str("Pane 1"));
            assert_eq!(it.len(), 3, "round {round}");
            let one = it.win(0);
            assert_eq!((one.title, one.selected, one.unread, one.badge, one.status), ("one", true, false, 2, Status::Running));
            assert_eq!((it.pane(0, 1).title, it.pane(0, 1).active, it.pane(0, 1).status), ("Pane 2 \u{b7} vim", false, Status::Idle));
            assert_eq!(it.win(1).title, "two");
            assert_eq!((it.pane(2, 0).title, it.pane(2, 0).status), ("Pane 1", Status::Failed), "a pane belongs to the window before it");
        }
    }
}
