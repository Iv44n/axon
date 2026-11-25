//! The panel of windows beside the grid, after the `terminal (1).html` reference: a header with a `+`,
//! then one 28 px row per window (an icon, the title, a dot when it printed while hidden, a cross on
//! hover), the selected one lit. Pure layout and hit testing in window pixels, and the painting of a
//! panel box (the box is cleared by the caller, `Soft::clear_rect`). The text is the grid's own font,
//! small: a proportional UI font would cost 5 MB of RSS for a panel that is always there.
use crate::{
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

/// What the pointer is over in the panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    /// The window's row: go to it.
    Row(usize),
    /// The cross of a row: close it.
    Close(usize),
    /// The `+` of the header: a new window.
    New,
}

/// A window as the panel shows it.
pub struct Item {
    pub title: String,
    pub selected: bool,
    pub unread: bool,
}

/// The `+` button's box `(x, y, w, h)` for a panel `w` pixels wide whose top is `y0`.
fn new_box(x0: i32, y0: i32, w: i32) -> (i32, i32, i32, i32) {
    (x0 + w - 6 - BTN, y0 + (HEAD_H - BTN) / 2 + 2, BTN, BTN)
}

/// The row `i`'s box `(x, y, w, h)`.
fn row_box(x0: i32, y0: i32, w: i32, i: usize) -> (i32, i32, i32, i32) {
    (x0 + PAD, y0 + HEAD_H + i as i32 * (ROW_H + GAP), w - 2 * PAD, ROW_H)
}

/// The cross's box in a row box.
fn close_box((x, y, w, h): (i32, i32, i32, i32)) -> (i32, i32, i32, i32) {
    (x + w - 5 - CLOSE, y + (h - CLOSE) / 2, CLOSE, CLOSE)
}

fn inside((x, y, w, h): (i32, i32, i32, i32), px: f32, py: f32) -> bool {
    px >= x as f32 && px < (x + w) as f32 && py >= y as f32 && py < (y + h) as f32
}

/// What is at window pixel `(px, py)` of a panel at `(x0, y0)`, `w` wide, with `n` windows.
pub fn hit(x0: i32, y0: i32, w: i32, n: usize, px: f32, py: f32) -> Option<Part> {
    if inside(new_box(x0, y0, w), px, py) {
        return Some(Part::New);
    }
    (0..n).find_map(|i| {
        let b = row_box(x0, y0, w, i);
        inside(b, px, py).then(|| if inside(close_box(b), px, py) { Part::Close(i) } else { Part::Row(i) })
    })
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

/// Paints the panel whose box is `(x0, y0)` (window pixels), `w` wide and `h` tall. `hover` is what the
/// pointer is over; `grid_on_right` says which side the hairline goes on.
#[allow(clippy::too_many_arguments)]
pub fn paint(soft: &Soft, fb: &mut [u32], (x0, y0, w, h): (i32, i32, i32, i32), grid_on_right: bool, title: &str, items: &[Item], hover: Option<Part>, scratch: &mut Vec<u8>) {
    let (fw, fh) = soft.fb_size();
    let (ox, oy) = soft.origin();
    if fb.len() < fw * fh {
        return;
    }
    let bottom = y0 + h;
    {
        let mut p = Paint { fb: &mut *fb, w: fw, h: fh, ox: ox as i32, oy: oy as i32, fade: 1.0 };
        // The hairline on the side of the grid.
        let hx = if grid_on_right { x0 + w - 1 } else { x0 };
        for y in y0..bottom {
            p.put(hx, y, WHITE, 0.07);
        }
        // The `+`.
        let b = new_box(x0, y0, w);
        if hover == Some(Part::New) {
            p.rrect(b.0, b.1, b.2, b.3, 8.0, WHITE, 0.07);
        }
        let hot = if hover == Some(Part::New) { palette::unit(palette::FG) } else { palette::unit(palette::DIM) };
        let (cx, cy) = (b.0 as f32 + b.2 as f32 / 2.0, b.1 as f32 + b.3 as f32 / 2.0);
        p.shape(b.0, b.1, b.0 + b.2, b.1 + b.3, hot, 1.0, &|px, py| {
            let d = seg(px, py, (cx - 4.5, cy), (cx + 4.5, cy)).min(seg(px, py, (cx, cy - 4.5), (cx, cy + 4.5)));
            (1.2 - d).clamp(0.0, 1.0)
        });
        for (i, it) in items.iter().enumerate() {
            let (x, y, rw, rh) = row_box(x0, y0, w, i);
            if y + rh > bottom {
                break;
            }
            let over = matches!(hover, Some(Part::Row(j) | Part::Close(j)) if j == i);
            if it.selected {
                p.rrect(x, y, rw, rh, 8.0, WHITE, 0.075);
            } else if over {
                p.rrect(x, y, rw, rh, 8.0, WHITE, 0.045);
            }
            // The terminal icon: a chevron and a dash.
            let lit = it.selected || over;
            let ic = palette::unit(if lit { palette::DIM } else { palette::FAINT });
            let (ix, iy) = ((x + 9) as f32 + 6.5, y as f32 + rh as f32 / 2.0);
            p.shape(x + 8, y + 5, x + 24, y + rh - 5, ic, 1.0, &|px, py| {
                let d = seg(px, py, (ix - 4.5, iy - 3.5), (ix - 1.0, iy)).min(seg(px, py, (ix - 1.0, iy), (ix - 4.5, iy + 3.5))).min(seg(px, py, (ix + 1.0, iy + 3.5), (ix + 5.0, iy + 3.5)));
                (1.1 - d).clamp(0.0, 1.0)
            });
            // The dot of a window that printed while it was hidden.
            let tail = if over { close_box((x, y, rw, rh)).0 as f32 - 6.0 } else { (x + rw) as f32 - 12.0 };
            if it.unread && !it.selected {
                p.disc(tail, y as f32 + rh as f32 / 2.0, 3.0, palette::unit(palette::CYAN), 1.0);
            }
            // The cross, on hover.
            if over {
                let c = close_box((x, y, rw, rh));
                let hot = hover == Some(Part::Close(i));
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
        }
    }
    // Text, in the grid's own font at 0.85 of its size.
    let m = soft.small(0.85);
    let pack = |c: alacritty_terminal::vte::ansi::Rgb| crate::cell::pack(c.r, c.g, c.b, 255);
    let head = fit(title, ((w - 24) / m.cell_w as i32).max(1) as usize);
    soft.paint_text(fb, ox + (x0 + 16) as usize, oy + (y0 + HEAD_H / 2 + 2) as usize, head.chars(), pack(palette::FAINT), false, &m, scratch);
    for (i, it) in items.iter().enumerate() {
        let (x, y, rw, rh) = row_box(x0, y0, w, i);
        if y + rh > bottom {
            break;
        }
        let over = matches!(hover, Some(Part::Row(j) | Part::Close(j)) if j == i);
        let room = (rw - 9 - 13 - 7 - 5 - if over { CLOSE + 6 } else { 18 }).max(0);
        let label = fit(&it.title, (room / m.cell_w as i32).max(1) as usize);
        let fg = if it.selected || over { palette::FG } else { palette::DIM };
        soft.paint_text(fb, ox + (x + 9 + 13 + 7) as usize, oy + (y + rh / 2) as usize, label.chars(), pack(fg), false, &m, scratch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_and_the_plus_are_found_where_they_are_drawn() {
        let (x0, y0, w) = (0, 42, 220);
        let n = row_box(x0, y0, w, 1);
        assert_eq!(hit(x0, y0, w, 3, (n.0 + 10) as f32, (n.1 + 5) as f32), Some(Part::Row(1)));
        let c = close_box(n);
        assert_eq!(hit(x0, y0, w, 3, (c.0 + 3) as f32, (c.1 + 3) as f32), Some(Part::Close(1)));
        let b = new_box(x0, y0, w);
        assert_eq!(hit(x0, y0, w, 3, (b.0 + 3) as f32, (b.1 + 3) as f32), Some(Part::New));
        // The gap between rows, the margin, and rows that are not there.
        assert_eq!(hit(x0, y0, w, 3, 100.0, (n.1 - 1) as f32), None);
        assert_eq!(hit(x0, y0, w, 3, 2.0, (n.1 + 5) as f32), None);
        assert_eq!(hit(x0, y0, w, 1, (n.0 + 10) as f32, (n.1 + 5) as f32), None);
    }

    #[test]
    fn titles_are_cut_with_an_ellipsis() {
        assert_eq!(fit("short", 10), "short");
        assert_eq!(fit("a very long title", 8), "a very …");
        assert_eq!(fit("abc", 1), "…");
    }
}
