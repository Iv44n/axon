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
/// A tab at the top: its natural width, its least, and the gap between tabs.
const TAB_W: i32 = 168;
const TAB_MIN: i32 = 96;
const TAB_GAP: i32 = 4;
/// The `+` after the tabs is as big as the one in the panel's header, but tall as a tab.
const TAB_NEW: i32 = 28;

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
    /// A group's header: fold or unfold it.
    Head(usize),
}

/// What the panel lists, top to bottom: a group's header, and a window (`grouped`: it is in a group).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entry {
    Head(usize),
    Row { win: usize, grouped: bool },
}

/// A group as the panel shows it.
pub struct GroupItem {
    pub name: String,
    pub color: u8,
    pub collapsed: bool,
    pub count: usize,
}

/// The entries for windows whose groups are `win_group` and groups whose folded state is `collapsed`: each
/// group's header and its windows (none when it is folded), in the order of the groups, then the windows
/// that are in none.
pub fn entries(win_group: &[Option<usize>], collapsed: &[bool]) -> Vec<Entry> {
    let mut out = Vec::with_capacity(win_group.len() + collapsed.len());
    for (g, &folded) in collapsed.iter().enumerate() {
        out.push(Entry::Head(g));
        if !folded {
            out.extend(win_group.iter().enumerate().filter(|(_, w)| **w == Some(g)).map(|(win, _)| Entry::Row { win, grouped: true }));
        }
    }
    out.extend(win_group.iter().enumerate().filter(|(_, w)| w.is_none()).map(|(win, _)| Entry::Row { win, grouped: false }));
    out
}

/// A window as the list shows it.
pub struct Item {
    pub title: String,
    pub selected: bool,
    pub unread: bool,
}

/// Where the list is: a panel at `(x0, y0)`, `w` wide, listing `entries`, or the tabs between `x0` and `x1`
/// in the title bar, one per window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout<'a> {
    Side { x0: i32, y0: i32, w: i32, entries: &'a [Entry] },
    Top { x0: i32, x1: i32, n: usize },
}

impl Layout<'_> {
    /// The box of entry `k` (a tab, at the top).
    fn rect(self, k: usize) -> Rect {
        match self {
            Layout::Side { x0, y0, w, entries } => {
                let mut y = y0 + HEAD_H;
                let mut prev: Option<Entry> = None;
                for (i, e) in entries.iter().enumerate() {
                    // The space above a group, and above the windows that are in none (after groups).
                    let gap = match (e, prev) {
                        (Entry::Head(_), _) => GROUP_GAP,
                        (Entry::Row { grouped: false, .. }, Some(Entry::Head(_) | Entry::Row { grouped: true, .. })) => GROUP_GAP,
                        _ => 0,
                    };
                    y += gap;
                    let (h, ind) = match e {
                        Entry::Head(_) => (GROUP_H, 0),
                        Entry::Row { grouped, .. } => (ROW_H, if *grouped { INDENT } else { 0 }),
                    };
                    if i == k {
                        return (x0 + PAD + ind, y, w - 2 * PAD - ind, h);
                    }
                    y += h + GAP;
                    prev = Some(*e);
                }
                (0, 0, 0, 0)
            }
            Layout::Top { x0, x1, n } => {
                let w = Self::tab_w(x0, x1, n);
                (x0 + k as i32 * (w + TAB_GAP), (TITLE_H as i32 - 1 - TAB_H) / 2, w, TAB_H)
            }
        }
    }

    /// How many entries there are.
    fn count(self) -> usize {
        match self {
            Layout::Side { entries, .. } => entries.len(),
            Layout::Top { n, .. } => n,
        }
    }

    /// What entry `k` is: a window's index, or a group's header.
    fn entry(self, k: usize) -> Entry {
        match self {
            Layout::Side { entries, .. } => entries[k],
            Layout::Top { .. } => Entry::Row { win: k, grouped: false },
        }
    }

    /// The width of a tab when `n` share `x0..x1` with the `+`: `TAB_W`, or less down to `TAB_MIN`.
    fn tab_w(x0: i32, x1: i32, n: usize) -> i32 {
        let room = x1 - x0 - TAB_NEW - TAB_GAP;
        (room / n.max(1) as i32 - TAB_GAP).clamp(TAB_MIN, TAB_W)
    }

    /// The box of the `+`.
    fn new_box(self) -> Rect {
        match self {
            Layout::Side { x0, y0, w, .. } => (x0 + w - 6 - BTN, y0 + (HEAD_H - BTN) / 2 + 2, BTN, BTN),
            Layout::Top { n, .. } => {
                let last = self.rect(n.saturating_sub(1));
                (if n == 0 { last.0 } else { last.0 + last.2 + TAB_GAP }, last.1, TAB_NEW, TAB_H)
            }
        }
    }

    /// What is at window pixel `(px, py)`.
    pub fn hit(self, px: f32, py: f32) -> Option<Part> {
        if inside(self.new_box(), px, py) {
            return Some(Part::New);
        }
        (0..self.count()).find_map(|k| {
            let b = self.rect(k);
            if !inside(b, px, py) {
                return None;
            }
            Some(match self.entry(k) {
                Entry::Head(g) => Part::Head(g),
                Entry::Row { win, .. } if inside(close_box(b), px, py) => Part::Close(win),
                Entry::Row { win, .. } => Part::Row(win),
            })
        })
    }
}

/// The cross's box in a row box.
fn close_box((x, y, w, h): Rect) -> Rect {
    (x + w - 5 - CLOSE, y + (h - CLOSE) / 2, CLOSE, CLOSE)
}

fn inside((x, y, w, h): Rect, px: f32, py: f32) -> bool {
    px >= x as f32 && px < (x + w) as f32 && py >= y as f32 && py < (y + h) as f32
}

/// `text` cut to `max` characters, with `…` where it was.
pub fn fit(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut s: String = text.chars().take(max.saturating_sub(1)).collect();
    s.push('…');
    s
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

/// The shapes of one row or tab in box `b`: its background, icon, unread dot and cross. `group` is the colour
/// of the group it is in: a bar on its left edge, which then has square corners.
fn row_shapes(p: &mut Paint, b: Rect, it: &Item, over: bool, cross_hot: bool, group: Option<u8>) {
    let (x, y, rw, rh) = b;
    let bg = if it.selected { 0.075 } else if over { 0.045 } else { 0.0 };
    if bg > 0.0 {
        if group.is_some() {
            // Round only on the right: the left edge is the group's bar.
            let (fx, fy, fw, fh) = ((x - 8) as f32, y as f32, (rw + 8) as f32, rh as f32);
            p.shape(x, y, x + rw, y + rh, WHITE, bg, &|px, py| (0.5 - crate::chrome::rounded_rect(px - fx, py - fy, fw, fh, 8.0)).clamp(0.0, 1.0));
        } else {
            p.rrect(x, y, rw, rh, 8.0, WHITE, bg);
        }
    }
    if let Some(g) = group {
        for yy in y..y + rh {
            for xx in x..x + 2 {
                p.put(xx, yy, group_color(g), 0.5);
            }
        }
    }
    // The terminal icon: a chevron and a dash.
    let ic = palette::unit(if it.selected || over { palette::DIM } else { palette::FAINT });
    let (ix, iy) = ((x + 9) as f32 + 6.5, y as f32 + rh as f32 / 2.0);
    p.shape(x + 8, y + 5, x + 24, y + rh - 5, ic, 1.0, &|px, py| {
        let d = seg(px, py, (ix - 4.5, iy - 3.5), (ix - 1.0, iy)).min(seg(px, py, (ix - 1.0, iy), (ix - 4.5, iy + 3.5))).min(seg(px, py, (ix + 1.0, iy + 3.5), (ix + 5.0, iy + 3.5)));
        (1.1 - d).clamp(0.0, 1.0)
    });
    // The dot of a window that printed while it was hidden.
    if it.unread && !it.selected {
        let tail = if over { close_box(b).0 as f32 - 6.0 } else { (x + rw) as f32 - 12.0 };
        p.disc(tail, y as f32 + rh as f32 / 2.0, 3.0, palette::unit(palette::CYAN), 1.0);
    }
    // The cross, on hover.
    if over {
        let c = close_box(b);
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

/// The shapes of a group's header in box `b`: a tinted background under the pointer, the colour's dot and
/// a chevron that points down when the group is open and right when it is folded.
fn head_shapes(p: &mut Paint, b: Rect, g: &GroupItem, over: bool) {
    let (x, y, w, h) = b;
    let c = group_color(g.color);
    if over {
        p.rrect(x, y, w, h, 7.0, c, 0.14);
    }
    p.disc((x + 6) as f32 + 3.5, y as f32 + h as f32 / 2.0, 3.5, c, 1.0);
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
}

/// The title of one row or tab, in the grid's font at 0.85 of its size.
fn row_text(soft: &Soft, fb: &mut [u32], b: Rect, it: &Item, over: bool, scratch: &mut Vec<u8>) {
    let (ox, oy) = soft.origin();
    let m = soft.small(0.85);
    let (x, y, rw, rh) = b;
    let room = (rw - 9 - 13 - 7 - 5 - if over { CLOSE + 6 } else { 18 }).max(0);
    let label = fit(&it.title, (room / m.cell_w as i32).max(1) as usize);
    let c: Rgb = if it.selected || over { palette::FG } else { palette::DIM };
    soft.paint_text(fb, ox + (x + 9 + 13 + 7) as usize, oy + (y + rh / 2) as usize, label.chars(), pack(c.r, c.g, c.b, 255), false, &m, scratch);
}

/// A group's name and, at the right end, how many windows it has.
fn head_text(soft: &Soft, fb: &mut [u32], b: Rect, g: &GroupItem, scratch: &mut Vec<u8>) {
    let (ox, oy) = soft.origin();
    let m = soft.small(0.85);
    let (x, y, w, h) = b;
    let count = g.count.to_string();
    let cw = m.cell_w as i32;
    let count_w = count.chars().count() as i32 * cw;
    let room = (w - 6 - 7 - 6 - 8 - count_w - 22).max(0);
    let name = fit(&g.name, (room / cw).max(1) as usize);
    let (fg, faint) = (palette::FG, palette::FAINT);
    let cy = oy + (y + h / 2) as usize;
    soft.paint_text(fb, ox + (x + 6 + 7 + 6) as usize, cy, name.chars(), pack(fg.r, fg.g, fg.b, 255), true, &m, scratch);
    soft.paint_text(fb, ox + (x + w - 22 - count_w) as usize, cy, count.chars(), pack(faint.r, faint.g, faint.b, 255), false, &m, scratch);
}

fn over_row(hover: Option<Part>, i: usize) -> bool {
    matches!(hover, Some(Part::Row(j) | Part::Close(j)) if j == i)
}

/// Paints the panel whose box is `(x0, y0)` (window pixels), `w` wide and `h` tall: the header with its `+`,
/// then `entries`. `hover` is what the pointer is over; `grid_on_right` says which side the hairline is on.
#[allow(clippy::too_many_arguments)]
pub fn paint(soft: &Soft, fb: &mut [u32], (x0, y0, w, h): Rect, grid_on_right: bool, title: &str, entries: &[Entry], groups: &[GroupItem], items: &[Item], hover: Option<Part>, scratch: &mut Vec<u8>) {
    let (fw, fh) = soft.fb_size();
    let (ox, oy) = soft.origin();
    if fb.len() < fw * fh {
        return;
    }
    let lay = Layout::Side { x0, y0, w, entries };
    let bottom = y0 + h;
    {
        let mut p = Paint { fb: &mut *fb, w: fw, h: fh, ox: ox as i32, oy: oy as i32, fade: 1.0 };
        // The hairline on the side of the grid.
        let hx = if grid_on_right { x0 + w - 1 } else { x0 };
        for y in y0..bottom {
            p.put(hx, y, WHITE, 0.07);
        }
        plus(&mut p, lay.new_box(), hover == Some(Part::New));
        for (k, e) in entries.iter().enumerate() {
            let b = lay.rect(k);
            if b.1 + b.3 > bottom {
                break;
            }
            match *e {
                Entry::Head(g) => head_shapes(&mut p, b, &groups[g], hover == Some(Part::Head(g))),
                Entry::Row { win, grouped } => row_shapes(&mut p, b, &items[win], over_row(hover, win), hover == Some(Part::Close(win)), grouped.then_some(group_of(entries, k, groups))),
            }
        }
    }
    let m = soft.small(0.85);
    let head = fit(title, ((w - 24) / m.cell_w as i32).max(1) as usize);
    let f = palette::FAINT;
    soft.paint_text(fb, ox + (x0 + 16) as usize, oy + (y0 + HEAD_H / 2 + 2) as usize, head.chars(), pack(f.r, f.g, f.b, 255), false, &m, scratch);
    for (k, e) in entries.iter().enumerate() {
        let b = lay.rect(k);
        if b.1 + b.3 > bottom {
            break;
        }
        match *e {
            Entry::Head(g) => head_text(soft, fb, b, &groups[g], scratch),
            Entry::Row { win, .. } => row_text(soft, fb, b, &items[win], over_row(hover, win), scratch),
        }
    }
}

/// The colour of the group that the window entry `k` belongs to: that of the last header before it.
fn group_of(entries: &[Entry], k: usize, groups: &[GroupItem]) -> u8 {
    entries[..k].iter().rev().find_map(|e| if let Entry::Head(g) = e { Some(groups[*g].color) } else { None }).unwrap_or(0)
}

/// Paints the tabs in the title bar between window pixels `x0` and `x1`, over a cleared bar.
pub fn paint_top(soft: &Soft, fb: &mut [u32], (x0, x1): (i32, i32), items: &[Item], hover: Option<Part>, scratch: &mut Vec<u8>) {
    let (fw, fh) = soft.fb_size();
    let (ox, oy) = soft.origin();
    if fb.len() < fw * fh {
        return;
    }
    let lay = Layout::Top { x0, x1, n: items.len() };
    {
        let mut p = Paint { fb: &mut *fb, w: fw, h: fh, ox: ox as i32, oy: oy as i32, fade: 1.0 };
        for (i, it) in items.iter().enumerate() {
            row_shapes(&mut p, lay.rect(i), it, over_row(hover, i), hover == Some(Part::Close(i)), None);
        }
        plus(&mut p, lay.new_box(), hover == Some(Part::New));
    }
    for (i, it) in items.iter().enumerate() {
        row_text(soft, fb, lay.rect(i), it, over_row(hover, i), scratch);
    }
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
        let e = entries(&wg, &[false, true]);
        assert_eq!(
            e,
            [Entry::Head(0), Entry::Row { win: 0, grouped: true }, Entry::Row { win: 2, grouped: true }, Entry::Head(1), Entry::Row { win: 3, grouped: false }]
        );
        assert_eq!(entries(&[None, None], &[]), rows(2));
        assert_eq!(entries(&[], &[false]), [Entry::Head(0)], "an empty group still has its header");
    }

    #[test]
    fn a_groups_header_windows_and_the_windows_in_none_are_spaced_like_the_reference() {
        let wg = [Some(0), Some(0), None];
        let e = entries(&wg, &[false]);
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

    #[test]
    fn tabs_share_the_bar_and_shrink_down_to_a_least_width() {
        let wide = Layout::Top { x0: 118, x1: 900, n: 3 };
        assert_eq!(wide.rect(0).2, TAB_W);
        let (a, b) = (wide.rect(0), wide.rect(1));
        assert_eq!(b.0, a.0 + a.2 + TAB_GAP);
        // The `+` follows the last tab; everything fits the bar.
        let plus = wide.new_box();
        assert_eq!(plus.0, wide.rect(2).0 + wide.rect(2).2 + TAB_GAP);
        assert!(plus.0 + plus.2 <= 900);
        // Eight tabs in a narrow bar: narrower, not narrower than the least.
        let tight = Layout::Top { x0: 118, x1: 700, n: 8 };
        assert!(tight.rect(0).2 < TAB_W && tight.rect(0).2 >= TAB_MIN);
        // Tabs sit inside the title bar, above its hairline.
        assert!(a.1 >= 0 && a.1 + a.3 <= TITLE_H as i32 - 1);
        assert_eq!(wide.hit((a.0 + 20) as f32, (a.1 + 10) as f32), Some(Part::Row(0)));
        assert_eq!(wide.hit((a.0 + 20) as f32, 1.0), None);
    }

    #[test]
    fn titles_are_cut_with_an_ellipsis() {
        assert_eq!(fit("short", 10), "short");
        assert_eq!(fit("a very long title", 8), "a very …");
        assert_eq!(fit("abc", 1), "…");
    }
}
