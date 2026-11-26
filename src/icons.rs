//! The small outline icons of the overlays (palette rows, menu checks, search fields, key caps), after the
//! `terminal.html` reference: each one is a few strokes on the reference's 16-unit grid, drawn as coverage
//! per pixel with a round stroke (the chip icons of `bar.rs` are drawn the same way). No images, no cache.

use crate::{
    agent::{Agent, State},
    chrome::rounded_rect,
    palette,
    paint::{Paint, WHITE, seg},
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

// ------------------------------------------------------------------------------------------------- agents' marks --
//
// The marks of the agents are their own published ones, not strokes: each outline was flattened once into a polygon (a
// curve cut into short lines, then every point within 1/250 of the mark of the line through its neighbours dropped). The
// points are in units of the mark's larger side, centred on 0, so all of them lie within ±0.5; `fill` scales them to
// the size asked for. About 200 points in all, read only when a mark is painted. No images, no cache.

/// Claude's burst: rays of uneven length and width from a common middle.
const CLAUDE: [(f32, f32); 90] = [
    (-0.304, 0.165), (-0.106, 0.055), (-0.106, 0.040), (-0.455, 0.025), (-0.478, 0.020), (-0.499, -0.010),
    (-0.497, -0.025), (-0.477, -0.038), (-0.104, -0.009), (-0.102, -0.016), (-0.110, -0.024), (-0.316, -0.160),
    (-0.401, -0.221), (-0.416, -0.241), (-0.422, -0.283), (-0.396, -0.313), (-0.350, -0.308), (-0.115, -0.128),
    (-0.108, -0.132), (-0.169, -0.250), (-0.257, -0.397), (-0.268, -0.453), (-0.238, -0.495), (-0.220, -0.500),
    (-0.178, -0.494), (-0.162, -0.480), (-0.095, -0.327), (-0.009, -0.163), (0.003, -0.119), (0.010, -0.119),
    (0.039, -0.427), (0.054, -0.465), (0.084, -0.485), (0.111, -0.474), (0.130, -0.445), (0.076, -0.147),
    (0.084, -0.147), (0.094, -0.158), (0.205, -0.298), (0.270, -0.370), (0.293, -0.388), (0.336, -0.388),
    (0.368, -0.341), (0.353, -0.292), (0.219, -0.118), (0.187, -0.061), (0.198, -0.057), (0.458, -0.107),
    (0.493, -0.091), (0.496, -0.075), (0.483, -0.041), (0.160, 0.033), (0.225, 0.042), (0.448, 0.054), (0.481, 0.074),
    (0.499, 0.101), (0.496, 0.121), (0.446, 0.147), (0.164, 0.080), (0.156, 0.084), (0.390, 0.301), (0.395, 0.325),
    (0.382, 0.345), (0.368, 0.343), (0.156, 0.175), (0.272, 0.356), (0.277, 0.401), (0.269, 0.415), (0.244, 0.425),
    (0.216, 0.419), (0.098, 0.249), (0.051, 0.167), (0.046, 0.171), (0.018, 0.473), (0.004, 0.488), (-0.025, 0.500),
    (-0.050, 0.480), (-0.064, 0.450), (-0.002, 0.137), (-0.009, 0.137), (-0.160, 0.343), (-0.231, 0.419),
    (-0.248, 0.426), (-0.278, 0.411), (-0.275, 0.383), (-0.061, 0.102), (-0.327, 0.274), (-0.374, 0.280),
    (-0.395, 0.260), (-0.393, 0.230),
];
/// Codex's cloud: a round shape of lobes.
const CODEX: [(f32, f32); 48] = [
    (-0.163, -0.481), (-0.093, -0.499), (-0.026, -0.497), (0.052, -0.471), (0.114, -0.426), (0.192, -0.434),
    (0.276, -0.415), (0.325, -0.389), (0.368, -0.351), (0.402, -0.305), (0.424, -0.252), (0.434, -0.180),
    (0.427, -0.113), (0.473, -0.047), (0.497, 0.025), (0.498, 0.096), (0.481, 0.162), (0.454, 0.212), (0.418, 0.253),
    (0.374, 0.286), (0.312, 0.313), (0.283, 0.378), (0.239, 0.432), (0.191, 0.467), (0.138, 0.490), (0.079, 0.500),
    (0.015, 0.495), (-0.052, 0.472), (-0.115, 0.426), (-0.199, 0.433), (-0.280, 0.414), (-0.331, 0.385),
    (-0.368, 0.351), (-0.403, 0.304), (-0.424, 0.253), (-0.435, 0.182), (-0.427, 0.115), (-0.472, 0.052),
    (-0.497, -0.020), (-0.499, -0.084), (-0.488, -0.145), (-0.463, -0.198), (-0.429, -0.243), (-0.373, -0.286),
    (-0.313, -0.312), (-0.290, -0.367), (-0.255, -0.416), (-0.213, -0.454),
];
/// The prompt in the cloud: its `>`...
const CODEX_CHEVRON: [(f32, f32); 10] = [
    (0.030, 0.106), (0.008, 0.116), (-0.003, 0.140), (0.004, 0.163), (0.027, 0.176), (0.239, 0.177), (0.259, 0.167),
    (0.269, 0.148), (0.263, 0.121), (0.240, 0.107),
];
/// ...and its `_`, a bar with round ends.
const CODEX_BAR: [(f32, f32); 14] = [
    (-0.197, -0.154), (-0.213, -0.168), (-0.235, -0.170), (-0.253, -0.159), (-0.262, -0.139), (-0.258, -0.119),
    (-0.187, 0.005), (-0.262, 0.133), (-0.256, 0.162), (-0.231, 0.177), (-0.213, 0.174), (-0.198, 0.162),
    (-0.111, 0.009), (-0.115, -0.011),
];
/// Antigravity's arch: a curve that rises to a round top and whose feet turn outwards; it is a little lower than wide.
const ANTIGRAVITY: [(f32, f32); 40] = [
    (0.406, 0.440), (0.440, 0.456), (0.480, 0.457), (0.498, 0.442), (0.497, 0.415), (0.429, 0.331), (0.365, 0.217),
    (0.316, 0.084), (0.215, -0.255), (0.163, -0.361), (0.131, -0.403), (0.094, -0.434), (0.051, -0.454),
    (0.002, -0.461), (-0.048, -0.454), (-0.091, -0.434), (-0.128, -0.403), (-0.160, -0.361), (-0.187, -0.311),
    (-0.233, -0.192), (-0.313, 0.084), (-0.362, 0.217), (-0.426, 0.331), (-0.490, 0.406), (-0.500, 0.428),
    (-0.498, 0.445), (-0.479, 0.459), (-0.437, 0.457), (-0.414, 0.447), (-0.365, 0.411), (-0.277, 0.315),
    (-0.138, 0.099), (-0.092, 0.060), (-0.065, 0.046), (0.002, 0.035), (0.068, 0.046), (0.096, 0.060), (0.142, 0.099),
    (0.306, 0.348), (0.368, 0.411),
];

/// How much lower than wide Antigravity's arch is: its colours (`palette::ANTIGRAVITY`) are spread over that box.
const ANTIGRAVITY_H: f32 = 0.9215;

/// Scanlines a pixel row of a mark is sampled with: across a scanline the coverage is exact, down a row it is the mean
/// of these.
const SUB: usize = 8;
/// Edges a scanline of a mark may cross (Claude's burst crosses about 26 at most).
const CROSS: usize = 48;
/// The widest mark one row of coverage holds, in pixels.
const ROW: usize = 64;

/// Fills `contours` (polygons in mark units, as the tables above) scaled to `size` pixels and centred on `(cx, cy)`,
/// with the non-zero rule, at `alpha`, leaving out the disc `cut` (centre and radius in pixels) when there is one;
/// each pixel takes the colour `ink` gives for its centre, in mark units. The
/// coverage is raised to the power 0.8, as a font is emboldened at small sizes: the burst's rays are about a pixel
/// wide at 14 px and would read thin and dark on the glass. Nothing is allocated: one row of coverage on the stack.
#[allow(clippy::too_many_arguments)]
fn fill(p: &mut Paint, contours: &[&[(f32, f32)]], cx: f32, cy: f32, size: f32, alpha: f32, cut: Option<(f32, f32, f32)>, ink: &dyn Fn(f32, f32) -> [f32; 3]) {
    let half = size / 2.0;
    let (x0, y0) = ((cx - half).floor() as i32 - 1, (cy - half).floor() as i32 - 1);
    let (x1, y1) = ((cx + half).ceil() as i32 + 1, (cy + half).ceil() as i32 + 1);
    let w = ((x1 - x0) as usize).min(ROW);
    for py in y0..y1 {
        let mut row = [0f32; ROW];
        for s in 0..SUB {
            let v = (py as f32 + (s as f32 + 0.5) / SUB as f32 - cy) / size;
            // Where the scanline crosses the outline, in pixels from `x0`, and which way the edge goes.
            let mut cross = [(0f32, 0i8); CROSS];
            let mut n = 0;
            for c in contours {
                for (k, &a) in c.iter().enumerate() {
                    let b = c[(k + 1) % c.len()];
                    if (a.1 <= v) != (b.1 <= v) && n < CROSS {
                        let x = a.0 + (v - a.1) * (b.0 - a.0) / (b.1 - a.1);
                        cross[n] = (cx + x * size - x0 as f32, if b.1 > a.1 { 1 } else { -1 });
                        n += 1;
                    }
                }
            }
            // In order along the scanline: a few dozen at most, so by insertion (the library's sort, made for this
            // type, would add more code to the binary than all of this).
            let cross = &mut cross[..n];
            for k in 1..n {
                let mut m = k;
                while m > 0 && cross[m - 1].0 > cross[m].0 {
                    cross.swap(m - 1, m);
                    m -= 1;
                }
            }
            let mut wind = 0;
            for k in 0..n.saturating_sub(1) {
                wind += cross[k].1 as i32;
                if wind != 0 {
                    span(&mut row[..w], cross[k].0, cross[k + 1].0, 1.0 / SUB as f32);
                }
            }
        }
        for (i, &c) in row[..w].iter().enumerate() {
            if c > 0.0 {
                let x = x0 + i as i32;
                let (fx, fy) = (x as f32 + 0.5, py as f32 + 0.5);
                let kept = cut.map_or(1.0, |(bx, by, r)| ((fx - bx).hypot(fy - by) - r).clamp(0.0, 1.0));
                p.put(x, py, ink((fx - cx) / size, (fy - cy) / size), alpha * c.min(1.0).powf(0.8) * kept);
            }
        }
    }
}

/// Adds to each pixel of `row` `k` times the part of it that `a..b` (pixels from the row's start) covers.
fn span(row: &mut [f32], a: f32, b: f32, k: f32) {
    let (a, b) = (a.max(0.0), b.min(row.len() as f32));
    if a >= b {
        return;
    }
    for (i, px) in row.iter_mut().enumerate().take(b.ceil() as usize).skip(a as usize) {
        let over = b.min(i as f32 + 1.0) - a.max(i as f32);
        if over > 0.0 {
            *px += over * k;
        }
    }
}

/// `a` to `b` by `t`.
fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// The colour of Codex's cloud at height `v` (mark units): its gradient from the top down, lilac to deep blue.
fn codex_ink(v: f32) -> [f32; 3] {
    let [top, mid, foot] = palette::CODEX.map(palette::unit);
    let t = (v + 0.5).clamp(0.0, 1.0) * 2.0;
    if t < 1.0 { mix(top, mid, t) } else { mix(mid, foot, t - 1.0) }
}

/// The colour of Antigravity's arch at `(u, v)` (mark units): its field of Google's colours, read between the
/// nearest four of `palette::ANTIGRAVITY` (yellow and green at the top left, red at the top right, blue below).
fn antigravity_ink(u: f32, v: f32) -> [f32; 3] {
    let last = (palette::ANTIGRAVITY.len() - 1) as f32;
    let fx = ((u + 0.5) * last).clamp(0.0, last);
    let fy = ((v / ANTIGRAVITY_H + 0.5) * last).clamp(0.0, last);
    let (i, j) = ((fx as usize).min(palette::ANTIGRAVITY.len() - 2), (fy as usize).min(palette::ANTIGRAVITY.len() - 2));
    let at = |j: usize, i: usize| palette::unit(palette::ANTIGRAVITY[j][i]);
    let (tx, ty) = (fx - i as f32, fy - j as f32);
    mix(mix(at(j, i), at(j, i + 1), tx), mix(at(j + 1, i), at(j + 1, i + 1), tx), ty)
}

/// How much of its alpha an agent's mark keeps while the agent works: it is busy, the user can look elsewhere.
pub const WORKING_ALPHA: f32 = 0.4;

/// The dot of an agent that waits for the user, for a mark `size` pixels across: its centre from the mark's (down and
/// to the right), its radius, and the gap cut out of the mark around it so it stands apart from the mark's own colour.
fn waiting_dot(size: f32) -> (f32, f32, f32) {
    (size * 0.34, size * 0.17, (size * 0.09).max(1.2))
}

/// Paints the mark of `agent` centred on `(cx, cy)`, `size` pixels across its larger side, at alpha `alpha`: Claude's
/// burst in its orange, Codex's cloud in its gradient with the prompt cut out in white, Antigravity's arch in its colours.
/// `state` is what the agent's title says it does: while it works the mark is faint (`WORKING_ALPHA`); while it waits
/// for the user an amber dot sits at its lower right, in a gap cut out of the mark; unknown, the mark is as it is.
#[allow(clippy::too_many_arguments)]
pub fn agent(p: &mut Paint, agent: Agent, state: State, cx: f32, cy: f32, size: f32, alpha: f32) {
    let (off, r, gap) = waiting_dot(size);
    let (dot, alpha) = match state {
        State::Waiting => (Some((cx + off, cy + off)), alpha),
        State::Working => (None, alpha * WORKING_ALPHA),
        State::Unknown => (None, alpha),
    };
    let cut = dot.map(|(x, y)| (x, y, r + gap));
    match agent {
        Agent::Claude => {
            let c = palette::unit(palette::CLAUDE);
            fill(p, &[&CLAUDE], cx, cy, size, alpha, cut, &|_, _| c);
        }
        Agent::Codex => {
            fill(p, &[&CODEX], cx, cy, size, alpha, cut, &|_, v| codex_ink(v));
            fill(p, &[&CODEX_CHEVRON, &CODEX_BAR], cx, cy, size, alpha, cut, &|_, _| WHITE);
        }
        Agent::Antigravity => fill(p, &[&ANTIGRAVITY], cx, cy, size, alpha, cut, &antigravity_ink),
    }
    if let Some((x, y)) = dot {
        p.disc(x, y, r, palette::unit(palette::ACCENT), alpha);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_agent_mark_draws_something_inside_its_box_and_nothing_outside() {
        for (a, state) in Agent::ALL.into_iter().flat_map(|a| [State::Unknown, State::Working, State::Waiting].map(|s| (a, s))) {
            let (w, h) = (40usize, 40usize);
            let mut fb = vec![0u32; w * h];
            let mut p = Paint { fb: &mut fb, w, h, ox: 0, oy: 0, fade: 1.0 };
            agent(&mut p, a, state, 20.0, 20.0, 14.0, 1.0);
            let inked = fb.iter().filter(|&&px| px >> 24 != 0).count();
            assert!(inked > 20, "{a:?} {state:?} inked {inked} pixels");
            for (i, &px) in fb.iter().enumerate() {
                let (x, y) = ((i % w) as i32, (i / w) as i32);
                if px >> 24 != 0 {
                    assert!((12..=28).contains(&x) && (12..=28).contains(&y), "{a:?} {state:?} drew at {x},{y}");
                }
            }
        }
    }

    #[test]
    fn the_three_marks_are_not_the_same_picture() {
        let draw = |a: Agent| {
            let mut fb = vec![0u32; 30 * 30];
            let mut p = Paint { fb: &mut fb, w: 30, h: 30, ox: 0, oy: 0, fade: 1.0 };
            agent(&mut p, a, State::Unknown, 15.0, 15.0, 14.0, 1.0);
            fb
        };
        let (c, g, o) = (draw(Agent::Claude), draw(Agent::Codex), draw(Agent::Antigravity));
        assert!(c != g && g != o && c != o);
    }

    #[test]
    fn every_mark_lies_within_half_a_unit_of_its_middle() {
        for (name, pts) in [("claude", &CLAUDE[..]), ("codex", &CODEX), ("chevron", &CODEX_CHEVRON), ("bar", &CODEX_BAR), ("antigravity", &ANTIGRAVITY)] {
            assert!(pts.len() >= 3, "{name}");
            for &(x, y) in pts {
                assert!(x.abs() <= 0.5 && y.abs() <= 0.5, "{name}: {x},{y}");
            }
        }
        // The larger side is the unit: some point reaches the edge.
        assert!(CLAUDE.iter().any(|&(x, y)| x.abs().max(y.abs()) > 0.49));
        assert!(ANTIGRAVITY.iter().all(|&(_, y)| y.abs() <= ANTIGRAVITY_H / 2.0 + 0.001));
    }

    #[test]
    fn a_span_covers_the_part_of_each_pixel_it_crosses() {
        let mut row = [0.0f32; 4];
        span(&mut row, 0.25, 2.5, 1.0);
        assert_eq!(row, [0.75, 1.0, 0.5, 0.0]);
        // Outside the row is cut off, not wrapped.
        span(&mut row, -3.0, 0.5, 1.0);
        span(&mut row, 3.5, 9.0, 1.0);
        assert_eq!(row, [1.25, 1.0, 0.5, 0.5]);
    }

    /// The pixel at `(x, y)` of `fb`, `w` wide: alpha, red, green, blue (0 to 255, premultiplied).
    fn argb(fb: &[u32], w: usize, x: usize, y: usize) -> [u32; 4] {
        let px = fb[y * w + x];
        [px >> 24, px >> 16 & 255, px >> 8 & 255, px & 255]
    }

    #[test]
    fn the_marks_are_solid_where_they_are_filled() {
        // Claude's burst: its middle is covered, its corners (between the rays) are not.
        let mut fb = vec![0u32; 40 * 40];
        let mut p = Paint { fb: &mut fb, w: 40, h: 40, ox: 0, oy: 0, fade: 1.0 };
        agent(&mut p, Agent::Claude, State::Unknown, 20.0, 20.0, 30.0, 1.0);
        assert_eq!(argb(&fb, 40, 20, 20)[0], 255);
        assert_eq!(argb(&fb, 40, 7, 7)[0], 0);
    }

    #[test]
    fn a_mark_is_faint_while_its_agent_works_and_has_a_dot_while_it_waits() {
        let draw = |state: State| {
            let mut fb = vec![0u32; 40 * 40];
            let mut p = Paint { fb: &mut fb, w: 40, h: 40, ox: 0, oy: 0, fade: 1.0 };
            agent(&mut p, Agent::Claude, state, 20.0, 20.0, 30.0, 1.0);
            fb
        };
        let (still, works, waits) = (draw(State::Unknown), draw(State::Working), draw(State::Waiting));
        // The middle of the burst: solid, then faint.
        assert_eq!(argb(&still, 40, 20, 20)[0], 255);
        assert_eq!(argb(&works, 40, 20, 20)[0], (255.0 * WORKING_ALPHA).round() as u32);
        assert_eq!(argb(&waits, 40, 20, 20)[0], 255);
        // The dot: amber and solid at its centre, and a ring of nothing between it and the mark.
        let (off, r, gap) = waiting_dot(30.0);
        let (dx, dy) = ((20.0 + off) as usize, (20.0 + off) as usize);
        let amber = palette::ACCENT;
        assert_eq!(argb(&waits, 40, dx, dy), [255, amber.r as u32, amber.g as u32, amber.b as u32]);
        let in_gap = (0..40 * 40).filter(|&i| {
            let (x, y) = ((i % 40) as f32 + 0.5, (i / 40) as f32 + 0.5);
            let d = (x - 20.0 - off).hypot(y - 20.0 - off);
            d > r + 0.75 && d < r + gap - 0.75
        });
        for i in in_gap {
            assert_eq!(waits[i], 0, "pixel {} in the gap", i);
        }
    }

    #[test]
    fn codex_has_its_prompt_in_white_on_its_blue_cloud() {
        let (w, size) = (64usize, 60.0);
        let mut fb = vec![0u32; w * w];
        let mut p = Paint { fb: &mut fb, w, h: w, ox: 0, oy: 0, fade: 1.0 };
        agent(&mut p, Agent::Codex, State::Unknown, 32.0, 32.0, size, 1.0);
        // The middle of the `_` (mark units (0.15, 0.13)) is white; the cloud above the prompt is blue.
        let bar = argb(&fb, w, (32.0 + 0.15 * size) as usize, (32.0 + 0.13 * size) as usize);
        assert!(bar[1] > 240 && bar[2] > 240 && bar[3] > 240, "{bar:?}");
        let cloud = argb(&fb, w, 32, (32.0 - 0.35 * size) as usize);
        assert!(cloud[0] == 255 && cloud[3] > cloud[1] + 40, "{cloud:?}");
    }

    #[test]
    fn antigravity_is_warm_at_the_top_and_blue_at_the_feet() {
        let (w, size) = (64usize, 60.0);
        let mut fb = vec![0u32; w * w];
        let mut p = Paint { fb: &mut fb, w, h: w, ox: 0, oy: 0, fade: 1.0 };
        agent(&mut p, Agent::Antigravity, State::Unknown, 32.0, 32.0, size, 1.0);
        let top = argb(&fb, w, 32, (32.0 - 0.40 * size) as usize);
        assert!(top[0] == 255 && top[1] > top[3], "{top:?}");
        let foot = (0..w).map(|x| argb(&fb, w, x, (32.0 + 0.43 * size) as usize)).find(|c| c[0] == 255).expect("a foot");
        assert!(foot[3] > foot[1] + 80, "{foot:?}");
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
