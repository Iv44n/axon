//! The small outline icons of the overlays (palette rows, menu checks, search fields, key caps), after the
//! `terminal.html` reference: each one is a few strokes on the reference's 16-unit grid, drawn as coverage
//! per pixel with a round stroke (the chip icons of `bar.rs` are drawn the same way). No images, no cache.
// The kit's API is wider than what the app uses yet: the palette, the overview and the dialogs are built on it next (ADR 0025).
#![allow(dead_code)]

use crate::{
    chrome::rounded_rect,
    paint::{Paint, seg},
};

/// What an icon can be. `Dot` is not here: a dot is a colour, not a shape (`list::Mark`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Search,
    Plus,
    Group,
    Split,
    Panel,
    Grid,
    Lines,
    Gear,
    Cross,
    Prompt,
    Term,
    Check,
    Up,
    Down,
    Enter,
}

impl Icon {
    pub const ALL: [Icon; 15] = [
        Icon::Search,
        Icon::Plus,
        Icon::Group,
        Icon::Split,
        Icon::Panel,
        Icon::Grid,
        Icon::Lines,
        Icon::Gear,
        Icon::Cross,
        Icon::Prompt,
        Icon::Term,
        Icon::Check,
        Icon::Up,
        Icon::Down,
        Icon::Enter,
    ];
}

/// Distance from `(px, py)` to the polyline through `pts`.
fn dist_poly(px: f32, py: f32, pts: &[(f32, f32)]) -> f32 {
    pts.windows(2).map(|w| seg(px, py, w[0], w[1])).fold(f32::MAX, f32::min)
}

/// Coverage of `icon` at `(x, y)` on the 16-unit grid, `u` pixels to the unit, with a stroke `t` units wide.
fn coverage(icon: Icon, x: f32, y: f32, u: f32, t: f32) -> f32 {
    // One unit of distance is `u` pixels: half the stroke plus half a pixel of antialiasing.
    let stroke = |d: f32| ((t / 2.0 + 0.5 / u - d.abs()) * u).clamp(0.0, 1.0);
    let poly = |pts: &[(f32, f32)]| stroke(dist_poly(x, y, pts));
    let circle = |cx: f32, cy: f32, r: f32| stroke((x - cx).hypot(y - cy) - r);
    let rect = |rx: f32, ry: f32, w: f32, h: f32, r: f32| stroke(rounded_rect(x - rx, y - ry, w, h, r));
    match icon {
        Icon::Search => circle(7.0, 7.0, 4.0).max(poly(&[(10.0, 10.0), (13.0, 13.0)])),
        Icon::Plus => poly(&[(8.0, 3.5), (8.0, 12.5)]).max(poly(&[(3.5, 8.0), (12.5, 8.0)])),
        Icon::Group => rect(2.5, 3.5, 11.0, 9.0, 2.0).max(poly(&[(8.0, 6.2), (8.0, 9.8)])).max(poly(&[(6.2, 8.0), (9.8, 8.0)])),
        Icon::Split => rect(2.5, 3.0, 11.0, 10.0, 2.0).max(poly(&[(8.0, 3.0), (8.0, 13.0)])),
        Icon::Panel => rect(2.5, 3.0, 11.0, 10.0, 2.0).max(poly(&[(6.5, 3.0), (6.5, 13.0)])),
        Icon::Grid => rect(2.5, 2.5, 4.6, 4.6, 1.2).max(rect(8.9, 2.5, 4.6, 4.6, 1.2)).max(rect(2.5, 8.9, 4.6, 4.6, 1.2)).max(rect(8.9, 8.9, 4.6, 4.6, 1.2)),
        Icon::Lines => poly(&[(3.5, 3.5), (12.5, 3.5)]).max(poly(&[(3.5, 6.5), (12.5, 6.5)])).max(poly(&[(3.5, 9.5), (9.5, 9.5)])).max(poly(&[(3.5, 12.5), (7.5, 12.5)])),
        Icon::Gear => poly(&[(2.5, 4.5), (8.5, 4.5)])
            .max(poly(&[(11.5, 4.5), (13.5, 4.5)]))
            .max(poly(&[(2.5, 11.5), (4.5, 11.5)]))
            .max(poly(&[(7.5, 11.5), (13.5, 11.5)]))
            .max(circle(10.0, 4.5, 1.5))
            .max(circle(6.0, 11.5, 1.5)),
        Icon::Cross => poly(&[(4.0, 4.0), (12.0, 12.0)]).max(poly(&[(12.0, 4.0), (4.0, 12.0)])),
        Icon::Prompt => poly(&[(4.5, 5.0), (8.0, 8.0), (4.5, 11.0)]).max(poly(&[(9.0, 11.5), (12.0, 11.5)])),
        Icon::Term => rect(2.0, 3.0, 12.0, 10.0, 2.0).max(poly(&[(5.0, 7.0), (7.0, 8.5), (5.0, 10.0)])).max(poly(&[(8.5, 10.5), (11.0, 10.5)])),
        Icon::Check => poly(&[(3.5, 8.5), (6.5, 11.5), (12.5, 4.5)]),
        Icon::Up => poly(&[(8.0, 12.5), (8.0, 3.5)]).max(poly(&[(4.5, 7.0), (8.0, 3.5), (11.5, 7.0)])),
        Icon::Down => poly(&[(8.0, 3.5), (8.0, 12.5)]).max(poly(&[(4.5, 9.0), (8.0, 12.5), (11.5, 9.0)])),
        Icon::Enter => poly(&[(12.5, 4.0), (12.5, 9.0), (4.0, 9.0)]).max(poly(&[(7.0, 6.0), (4.0, 9.0), (7.0, 12.0)])),
    }
}

/// Paints `icon` centred on `(cx, cy)`, `size` pixels square (the 16-unit grid scaled to it), in colour `c`
/// at alpha `a`; `t` is the stroke in grid units (the reference draws 1.5, thicker for a menu's check).
#[allow(clippy::too_many_arguments)]
pub fn paint(p: &mut Paint, icon: Icon, cx: f32, cy: f32, size: f32, t: f32, c: [f32; 3], a: f32) {
    let u = size / 16.0;
    let r = size / 2.0 + 1.0;
    let (x0, y0) = ((cx - r).floor() as i32, (cy - r).floor() as i32);
    let (x1, y1) = ((cx + r).ceil() as i32 + 1, (cy + r).ceil() as i32 + 1);
    p.shape(x0, y0, x1, y1, c, a, &|px, py| coverage(icon, (px - cx) / u + 8.0, (py - cy) / u + 8.0, u, t));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_draws_something_inside_its_box_and_nothing_outside() {
        for icon in Icon::ALL {
            let (w, h) = (40usize, 40usize);
            let mut fb = vec![0u32; w * h];
            let mut p = Paint { fb: &mut fb, w, h, ox: 0, oy: 0, fade: 1.0 };
            paint(&mut p, icon, 20.0, 20.0, 16.0, 1.5, [1.0; 3], 1.0);
            let inked = fb.iter().filter(|&&px| px >> 24 != 0).count();
            assert!(inked > 12, "{icon:?} inked {inked} pixels");
            // Nothing beyond one pixel around the 16 px box.
            for (i, &px) in fb.iter().enumerate() {
                let (x, y) = ((i % w) as i32, (i / w) as i32);
                if px >> 24 != 0 {
                    assert!((11..=29).contains(&x) && (11..=29).contains(&y), "{icon:?} drew at {x},{y}");
                }
            }
        }
    }

    #[test]
    fn a_larger_icon_covers_more_pixels() {
        let ink = |size: f32| {
            let mut fb = vec![0u32; 60 * 60];
            let mut p = Paint { fb: &mut fb, w: 60, h: 60, ox: 0, oy: 0, fade: 1.0 };
            paint(&mut p, Icon::Grid, 30.0, 30.0, size, 1.5, [1.0; 3], 1.0);
            fb.iter().filter(|&&px| px >> 24 != 0).count()
        };
        assert!(ink(24.0) > ink(12.0));
    }
}
