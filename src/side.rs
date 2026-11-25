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
pub const ROW_H: i32 = 28;
pub const GAP: i32 = 2;
pub const PAD: i32 = 8;
/// The `+` and the cross are square buttons this big.
const BTN: i32 = 24;
const CLOSE: i32 = 18;
/// A tab at the top: its natural width, its least, and the gap between tabs.
const TAB_W: i32 = 168;
const TAB_MIN: i32 = 96;
const TAB_GAP: i32 = 4;
/// The `+` after the tabs is as big as the one in the panel's header, but tall as a tab.
const TAB_NEW: i32 = 28;

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
}

/// A window as the list shows it.
pub struct Item {
    pub title: String,
    pub selected: bool,
    pub unread: bool,
}

/// Where the list is: a panel at `(x0, y0)`, `w` wide, or the tabs between `x0` and `x1` in the title bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    Side { x0: i32, y0: i32, w: i32 },
    Top { x0: i32, x1: i32, n: usize },
}

impl Layout {
    /// The box of row or tab `i`.
    fn row(self, i: usize) -> Rect {
        match self {
            Layout::Side { x0, y0, w } => (x0 + PAD, y0 + HEAD_H + i as i32 * (ROW_H + GAP), w - 2 * PAD, ROW_H),
            Layout::Top { x0, x1, n } => {
                let w = Self::tab_w(x0, x1, n);
                (x0 + i as i32 * (w + TAB_GAP), (TITLE_H as i32 - 1 - ROW_H) / 2, w, ROW_H)
            }
        }
    }

    /// The width of a tab when `n` share `x0..x1` with the `+`: `TAB_W`, or less down to `TAB_MIN`.
    fn tab_w(x0: i32, x1: i32, n: usize) -> i32 {
        let room = x1 - x0 - TAB_NEW - TAB_GAP;
        (room / n.max(1) as i32 - TAB_GAP).clamp(TAB_MIN, TAB_W)
    }

    /// The box of the `+`.
    fn new_box(self, n: usize) -> Rect {
        match self {
            Layout::Side { x0, y0, w } => (x0 + w - 6 - BTN, y0 + (HEAD_H - BTN) / 2 + 2, BTN, BTN),
            Layout::Top { .. } => {
                let last = self.row(n.saturating_sub(1));
                (if n == 0 { last.0 } else { last.0 + last.2 + TAB_GAP }, last.1, TAB_NEW, ROW_H)
            }
        }
    }

    /// What is at window pixel `(px, py)` with `n` windows.
    pub fn hit(self, n: usize, px: f32, py: f32) -> Option<Part> {
        if inside(self.new_box(n), px, py) {
            return Some(Part::New);
        }
        (0..n).find_map(|i| {
            let b = self.row(i);
            inside(b, px, py).then(|| if inside(close_box(b), px, py) { Part::Close(i) } else { Part::Row(i) })
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

/// The shapes of one row or tab in box `b`: its background, icon, unread dot and cross.
fn row_shapes(p: &mut Paint, b: Rect, it: &Item, over: bool, cross_hot: bool) {
    let (x, y, rw, rh) = b;
    if it.selected {
        p.rrect(x, y, rw, rh, 8.0, WHITE, 0.075);
    } else if over {
        p.rrect(x, y, rw, rh, 8.0, WHITE, 0.045);
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

fn over_row(hover: Option<Part>, i: usize) -> bool {
    matches!(hover, Some(Part::Row(j) | Part::Close(j)) if j == i)
}

/// Paints the panel whose box is `(x0, y0)` (window pixels), `w` wide and `h` tall. `hover` is what the
/// pointer is over; `grid_on_right` says which side the hairline goes on.
#[allow(clippy::too_many_arguments)]
pub fn paint(soft: &Soft, fb: &mut [u32], (x0, y0, w, h): Rect, grid_on_right: bool, title: &str, items: &[Item], hover: Option<Part>, scratch: &mut Vec<u8>) {
    let (fw, fh) = soft.fb_size();
    let (ox, oy) = soft.origin();
    if fb.len() < fw * fh {
        return;
    }
    let lay = Layout::Side { x0, y0, w };
    let bottom = y0 + h;
    {
        let mut p = Paint { fb: &mut *fb, w: fw, h: fh, ox: ox as i32, oy: oy as i32, fade: 1.0 };
        // The hairline on the side of the grid.
        let hx = if grid_on_right { x0 + w - 1 } else { x0 };
        for y in y0..bottom {
            p.put(hx, y, WHITE, 0.07);
        }
        plus(&mut p, lay.new_box(items.len()), hover == Some(Part::New));
        for (i, it) in items.iter().enumerate() {
            let b = lay.row(i);
            if b.1 + b.3 > bottom {
                break;
            }
            row_shapes(&mut p, b, it, over_row(hover, i), hover == Some(Part::Close(i)));
        }
    }
    let m = soft.small(0.85);
    let head = fit(title, ((w - 24) / m.cell_w as i32).max(1) as usize);
    let f = palette::FAINT;
    soft.paint_text(fb, ox + (x0 + 16) as usize, oy + (y0 + HEAD_H / 2 + 2) as usize, head.chars(), pack(f.r, f.g, f.b, 255), false, &m, scratch);
    for (i, it) in items.iter().enumerate() {
        let b = lay.row(i);
        if b.1 + b.3 > bottom {
            break;
        }
        row_text(soft, fb, b, it, over_row(hover, i), scratch);
    }
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
            row_shapes(&mut p, lay.row(i), it, over_row(hover, i), hover == Some(Part::Close(i)));
        }
        plus(&mut p, lay.new_box(items.len()), hover == Some(Part::New));
    }
    for (i, it) in items.iter().enumerate() {
        row_text(soft, fb, lay.row(i), it, over_row(hover, i), scratch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_and_the_plus_are_found_where_they_are_drawn() {
        let lay = Layout::Side { x0: 0, y0: 42, w: 220 };
        let n = lay.row(1);
        assert_eq!(lay.hit(3, (n.0 + 10) as f32, (n.1 + 5) as f32), Some(Part::Row(1)));
        let c = close_box(n);
        assert_eq!(lay.hit(3, (c.0 + 3) as f32, (c.1 + 3) as f32), Some(Part::Close(1)));
        let b = lay.new_box(3);
        assert_eq!(lay.hit(3, (b.0 + 3) as f32, (b.1 + 3) as f32), Some(Part::New));
        // The gap between rows, the margin, and rows that are not there.
        assert_eq!(lay.hit(3, 100.0, (n.1 - 1) as f32), None);
        assert_eq!(lay.hit(3, 2.0, (n.1 + 5) as f32), None);
        assert_eq!(lay.hit(1, (n.0 + 10) as f32, (n.1 + 5) as f32), None);
    }

    #[test]
    fn tabs_share_the_bar_and_shrink_down_to_a_least_width() {
        let wide = Layout::Top { x0: 118, x1: 900, n: 3 };
        assert_eq!(wide.row(0).2, TAB_W);
        let (a, b) = (wide.row(0), wide.row(1));
        assert_eq!(b.0, a.0 + a.2 + TAB_GAP);
        // The `+` follows the last tab; everything fits the bar.
        let plus = wide.new_box(3);
        assert_eq!(plus.0, wide.row(2).0 + wide.row(2).2 + TAB_GAP);
        assert!(plus.0 + plus.2 <= 900);
        // Eight tabs in a narrow bar: narrower, not narrower than the least.
        let tight = Layout::Top { x0: 118, x1: 700, n: 8 };
        assert!(tight.row(0).2 < TAB_W && tight.row(0).2 >= TAB_MIN);
        // Tabs are as tall as rows and sit inside the title bar, above its hairline.
        assert!(a.1 >= 0 && a.1 + a.3 <= TITLE_H as i32 - 1);
        assert_eq!(wide.hit(3, (a.0 + 20) as f32, (a.1 + 10) as f32), Some(Part::Row(0)));
        assert_eq!(wide.hit(3, (a.0 + 20) as f32, 1.0), None);
    }

    #[test]
    fn titles_are_cut_with_an_ellipsis() {
        assert_eq!(fit("short", 10), "short");
        assert_eq!(fit("a very long title", 8), "a very …");
        assert_eq!(fit("abc", 1), "…");
    }
}
