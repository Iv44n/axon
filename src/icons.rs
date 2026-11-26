//! The small outline icons of the overlays (palette rows, menu checks, search fields, key caps), after the
//! `terminal.html` reference: each one is a few strokes on the reference's 16-unit grid, drawn as coverage
//! per pixel with a round stroke (the chip icons of `bar.rs` are drawn the same way). No images, no cache.

use crate::{
    agent::Agent,
    chrome::rounded_rect,
    palette,
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
    Check,
    Up,
    Down,
    Enter,
}

impl Icon {
    /// Every icon (the test that draws each one).
    #[cfg(test)]
    pub const ALL: [Icon; 14] = [
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

/// The colour of an agent's mark: Claude's orange, ChatGPT's own black-and-white read as the text colour on the glass,
/// Google's blue.
fn agent_color(a: Agent) -> [f32; 3] {
    palette::unit(match a {
        Agent::Claude => palette::CLAUDE,
        Agent::ChatGpt => palette::FG,
        Agent::Google => palette::BLUE,
    })
}

/// Coverage of the mark of `a` at `(x, y)`, measured from the mark's centre, for a mark `r` pixels from its centre to its edge.
/// Each is a few round strokes like the outline icons: Claude's burst of rays (long and short in turn), ChatGPT's knot
/// (two triangles whose crossing leaves a hexagon open in the middle), Antigravity's arch (an A).
fn agent_coverage(a: Agent, x: f32, y: f32, r: f32) -> f32 {
    use std::f32::consts::TAU;
    // Half the stroke in pixels: thin enough for the rays to tell apart and the knot's middle to stay open at 12 px, never
    // under a pixel and a quarter across.
    let half = (r * 0.12).max(0.62);
    let stroke = |d: f32| (half + 0.5 - d).clamp(0.0, 1.0);
    let at = |k: f32, of: f32, len: f32| ((k / of * TAU - TAU / 4.0).cos() * r * len, (k / of * TAU - TAU / 4.0).sin() * r * len);
    match a {
        Agent::Claude => {
            const LEN: [f32; 10] = [1.0, 0.62, 0.9, 0.66, 0.96, 0.62, 1.0, 0.7, 0.88, 0.64];
            LEN.iter().enumerate().map(|(k, &l)| stroke(seg(x, y, (0.0, 0.0), at(k as f32, 10.0, l)))).fold(0.0, f32::max)
        }
        Agent::ChatGpt => {
            let v = |k: usize| at(k as f32, 6.0, 0.86);
            (0..6).map(|k| stroke(seg(x, y, v(k), v((k + 2) % 6)))).fold(0.0, f32::max)
        }
        Agent::Google => {
            let (apex, foot_l, foot_r) = ((0.0, -r * 0.86), (-r * 0.82, r * 0.82), (r * 0.82, r * 0.82));
            let bar = (r * 0.3, r * 0.34);
            stroke(seg(x, y, foot_l, apex)).max(stroke(seg(x, y, apex, foot_r))).max(stroke(seg(x, y, (-bar.0, bar.1), bar)))
        }
    }
}

/// Paints the mark of `agent` centred on `(cx, cy)`, `size` pixels across, at alpha `alpha`.
pub fn agent(p: &mut Paint, agent: Agent, cx: f32, cy: f32, size: f32, alpha: f32) {
    let r = size / 2.0;
    let (x0, y0) = ((cx - r - 1.0).floor() as i32, (cy - r - 1.0).floor() as i32);
    let (x1, y1) = ((cx + r + 1.0).ceil() as i32 + 1, (cy + r + 1.0).ceil() as i32 + 1);
    p.shape(x0, y0, x1, y1, agent_color(agent), alpha, &|px, py| agent_coverage(agent, px - cx, py - cy, r));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_agent_mark_draws_something_inside_its_box_and_nothing_outside() {
        for a in Agent::ALL {
            let (w, h) = (40usize, 40usize);
            let mut fb = vec![0u32; w * h];
            let mut p = Paint { fb: &mut fb, w, h, ox: 0, oy: 0, fade: 1.0 };
            agent(&mut p, a, 20.0, 20.0, 14.0, 1.0);
            let inked = fb.iter().filter(|&&px| px >> 24 != 0).count();
            assert!(inked > 20, "{a:?} inked {inked} pixels");
            for (i, &px) in fb.iter().enumerate() {
                let (x, y) = ((i % w) as i32, (i / w) as i32);
                if px >> 24 != 0 {
                    assert!((12..=28).contains(&x) && (12..=28).contains(&y), "{a:?} drew at {x},{y}");
                }
            }
        }
    }

    #[test]
    fn the_three_marks_are_not_the_same_picture() {
        let draw = |a: Agent| {
            let mut fb = vec![0u32; 30 * 30];
            let mut p = Paint { fb: &mut fb, w: 30, h: 30, ox: 0, oy: 0, fade: 1.0 };
            agent(&mut p, a, 15.0, 15.0, 14.0, 1.0);
            fb
        };
        let (c, g, o) = (draw(Agent::Claude), draw(Agent::ChatGpt), draw(Agent::Google));
        assert!(c != g && g != o && c != o);
    }

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
