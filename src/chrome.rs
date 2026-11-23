//! Window chrome on the CPU renderer: geometry, hit testing and the glass look.
//!
//! Nothing here is stored. Every pixel of the frame background is a pure function of its position,
//! the window size and a few bits of UI state (`Ui`), so there is no texture, no mask and no cache:
//! a rounded rectangle (signed distance) of one continuous translucent black glass, a faint uniform
//! outline, the three window buttons and the title floating on it, and a soft shadow in a margin
//! around the window (Mutter draws none for client-decorated windows).
//!
//! Continuous on purpose, like the integrated title bars of Ghostty, WezTerm and Warp: the title
//! bar is the same surface as the grid (no lighter pane, no separator), and nothing on the edge
//! changes along it, so no line gathers in the corners (`knowledge/decisions/0015`). Black and
//! white; the only colour on screen is what programs ask for.
//!
//! Coordinates: `pixel` works in buffer pixels (window plus shadow margin, see `margins`); `hit`
//! works in window pixels, so it can be negative over the margin.

/// Default alpha of the glass body (`AXON_OPACITY`). There is no compositor blur on GNOME, so what
/// is behind shows through sharp: at 0.80 the text of a window behind was readable through ours.
pub const OPACITY: f32 = 0.88;
/// Corner radius of the window, in pixels (0 when maximized).
pub const RADIUS: f32 = 14.0;
/// The shadow: how far it spreads, and how far down it is cast. The margin around the window is
/// exactly what it covers: `SHADOW_BLUR` left and right, `BLUR - DY` above and `BLUR + DY` below.
pub const SHADOW_BLUR: usize = 26;
pub const SHADOW_DY: usize = 8;
const SHADOW_ALPHA: f32 = 0.55;
/// The glass tint (premultiplied by the opacity) and the white of the 1 px outline: the same all
/// around the window, so it reads as the glass's edge, not as a line of its own.
const GLASS: f32 = 0.035;
const OUTLINE: f32 = 0.08;
/// Height of the title bar: where the buttons and the title sit and the window can be dragged.
/// Not drawn: it is the same glass as the rest. The terminal grid starts below it.
pub const TITLE_H: usize = 38;
/// Pixels from an edge beyond which the background no longer varies along a row.
pub const REACH: usize = 32;
/// Left edge of the area the title text must stay clear of; the right side mirrors it.
pub const TITLE_CLEAR: usize = 90;

// Pixels are sampled at their centre (`x + 0.5`), so the button centres sit on `n + 0.5`: a 13 px
// disc then ends exactly on pixel borders and a 1 px stroke through the centre fills one row.
const BTN_R: f32 = 6.5;
const BTN_X0: f32 = 20.5;
const BTN_DX: f32 = 20.0;
const BTN_CY: f32 = 19.5;
/// Grab zone inside a window edge, the one outside it (over the shadow), and the longer zone that
/// turns an edge into a corner.
const EDGE: f32 = 4.0;
pub const EDGE_OUT: usize = 8;
const CORNER: f32 = 18.0;

/// `xdg_toplevel.resize_edge` bits.
pub const EDGE_TOP: u32 = 1;
pub const EDGE_BOTTOM: u32 = 2;
pub const EDGE_LEFT: u32 = 4;
pub const EDGE_RIGHT: u32 = 8;

/// What the pointer is over.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Hit {
    #[default]
    Client,
    Close,
    Minimize,
    Maximize,
    Title,
    /// A resize edge or corner: `EDGE_*` bits.
    Edge(u32),
}

impl Hit {
    /// 1..=3 for the buttons, 0 for everything else.
    pub fn button(self) -> u8 {
        match self {
            Hit::Close => 1,
            Hit::Minimize => 2,
            Hit::Maximize => 3,
            _ => 0,
        }
    }
}

/// The bits of state the look depends on; repaint the title bar when one changes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Ui {
    /// Button under the pointer, `Hit::button` (0 = none).
    pub hover: u8,
    /// The primary button is held down on `hover`.
    pub down: bool,
    pub focused: bool,
    /// Maximized, fullscreen or tiled: square corners, no rim, no shadow, no resize edges.
    pub maximized: bool,
}

impl Ui {
    pub const IDLE: Ui = Ui { hover: 0, down: false, focused: true, maximized: false };
}

fn button_center(i: u8) -> (f32, f32) {
    (BTN_X0 + (i - 1) as f32 * BTN_DX, BTN_CY)
}

/// Space the shadow needs around the window: (left and right, top, bottom). None when square.
pub fn margins(maximized: bool) -> (usize, usize, usize) {
    if maximized { (0, 0, 0) } else { (SHADOW_BLUR, SHADOW_BLUR - SHADOW_DY, SHADOW_BLUR + SHADOW_DY) }
}

/// What is at `(x, y)` in a `w` x `h` window, in window pixels (negative over the shadow margin).
pub fn hit(x: f32, y: f32, w: u32, h: u32, maximized: bool) -> Hit {
    let (w, h) = (w as f32, h as f32);
    if !maximized {
        let mut edges = 0;
        let out = EDGE_OUT as f32;
        if x < -out || y < -out || x >= w + out || y >= h + out {
            return Hit::Client; // over the shadow, beyond the grab zone: not ours (input region)
        }
        if y < EDGE {
            edges |= EDGE_TOP;
        } else if y >= h - EDGE {
            edges |= EDGE_BOTTOM;
        }
        if x < EDGE {
            edges |= EDGE_LEFT;
        } else if x >= w - EDGE {
            edges |= EDGE_RIGHT;
        }
        // An edge strip close to a corner resizes both ways.
        if edges & (EDGE_TOP | EDGE_BOTTOM) != 0 && edges & (EDGE_LEFT | EDGE_RIGHT) == 0 {
            if x < CORNER {
                edges |= EDGE_LEFT;
            } else if x >= w - CORNER {
                edges |= EDGE_RIGHT;
            }
        } else if edges & (EDGE_LEFT | EDGE_RIGHT) != 0 && edges & (EDGE_TOP | EDGE_BOTTOM) == 0 {
            if y < CORNER {
                edges |= EDGE_TOP;
            } else if y >= h - CORNER {
                edges |= EDGE_BOTTOM;
            }
        }
        if edges != 0 {
            return Hit::Edge(edges);
        }
    }
    for (i, hit) in [(1, Hit::Close), (2, Hit::Minimize), (3, Hit::Maximize)] {
        let (cx, cy) = button_center(i);
        if (x - cx).hypot(y - cy) <= BTN_R + 3.0 {
            return hit;
        }
    }
    if y < TITLE_H as f32 { Hit::Title } else { Hit::Client }
}

// ------------------------------------------------------------------ look --

/// Premultiplied colour, 0..1.
#[derive(Clone, Copy)]
struct P {
    r: f32,
    g: f32,
    b: f32,
    a: f32,
}

impl P {
    fn white(a: f32) -> P {
        P { r: a, g: a, b: a, a }
    }

    fn black(a: f32) -> P {
        P { r: 0.0, g: 0.0, b: 0.0, a }
    }

    /// `self` painted over `dst`.
    fn over(self, dst: P) -> P {
        let k = 1.0 - self.a;
        P { r: self.r + dst.r * k, g: self.g + dst.g * k, b: self.b + dst.b * k, a: self.a + dst.a * k }
    }

    fn scale(self, k: f32) -> P {
        P { r: self.r * k, g: self.g * k, b: self.b * k, a: self.a * k }
    }
}

/// Distance from `(px, py)` to the segment `a`-`b`.
fn seg(px: f32, py: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let t = (((px - a.0) * dx + (py - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    (px - a.0 - dx * t).hypot(py - a.1 - dy * t)
}

/// Coverage of a line about 1.2 px wide at distance `d` from its axis: a stroke through a pixel
/// centre fills that pixel and leaves a faint halo on its neighbours.
fn line(d: f32) -> f32 {
    (1.1 - d).clamp(0.0, 1.0)
}

/// The glyph on button `i` (close, minimize, maximize) at `(px, py)`.
fn glyph(i: u8, px: f32, py: f32) -> f32 {
    let (cx, cy) = button_center(i);
    let k = 2.7;
    match i {
        1 => line(seg(px, py, (cx - k, cy - k), (cx + k, cy + k))).max(line(seg(px, py, (cx - k, cy + k), (cx + k, cy - k)))),
        2 => line(seg(px, py, (cx - 3.0, cy), (cx + 3.0, cy))),
        _ => line(seg(px, py, (cx - 3.0, cy), (cx + 3.0, cy))).max(line(seg(px, py, (cx, cy - 3.0), (cx, cy + 3.0)))),
    }
}

/// The three buttons over `base`, at pixel centre `(px, py)`.
fn buttons(mut base: P, px: f32, py: f32, ui: &Ui) -> P {
    if px > BTN_X0 + 2.0 * BTN_DX + BTN_R + 2.0 || (py - BTN_CY).abs() > BTN_R + 2.0 {
        return base;
    }
    for i in 1..=3u8 {
        let (cx, cy) = button_center(i);
        let d = (px - cx).hypot(py - cy) - BTN_R;
        if d > 0.5 {
            continue;
        }
        let cov = (0.5 - d).clamp(0.0, 1.0);
        let hovered = ui.hover == i;
        // Resting discs are light gray glass; a hovered group shows its glyphs and the one
        // under the pointer turns near white.
        let alpha = match (ui.focused, ui.hover != 0, hovered, ui.down && hovered) {
            (_, _, true, true) => 0.62,
            (_, _, true, false) => 0.92,
            (_, true, false, _) => 0.46,
            (true, false, false, _) => 0.34,
            (false, false, false, _) => 0.17,
        };
        base = P::white(alpha * cov).over(base);
        // A faint dark ring inside the disc gives it an edge.
        let ring = (1.0 - (-d).clamp(0.0, 1.0)) * cov;
        base = P::black(ring * 0.20).over(base);
        if ui.hover != 0 {
            base = P::black(glyph(i, px, py) * 0.78).over(base);
        }
    }
    base
}

/// Signed distance from `(px, py)` to a `w` x `h` rectangle at the origin with corner radius `r`:
/// negative inside.
fn rounded_rect(px: f32, py: f32, w: f32, h: f32, r: f32) -> f32 {
    let qx = (px - w * 0.5).abs() - (w * 0.5 - r);
    let qy = (py - h * 0.5).abs() - (h * 0.5 - r);
    qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r
}

/// Alpha of the shadow at window pixel `(px, py)`: soft, cast `SHADOW_DY` down, already at about
/// half strength under the window's edge so it starts where the glass ends, and zero inside the
/// margin so the buffer edge never cuts it.
fn shadow(px: f32, py: f32, w: f32, h: f32) -> f32 {
    let (blur, dy) = (SHADOW_BLUR as f32, SHADOW_DY as f32);
    let d = rounded_rect(px, py - dy, w, h, RADIUS + 4.0);
    let t = ((d + 8.0) / (blur + 8.0)).clamp(0.0, 1.0);
    SHADOW_ALPHA * (1.0 - t) * (1.0 - t) * (1.0 - t)
}

/// The frame at buffer pixel `(x, y)` of a `w` x `h` buffer (window plus `margins`), premultiplied
/// `Argb8888`. `opacity` is the alpha of the glass body.
pub fn pixel(x: usize, y: usize, w: usize, h: usize, opacity: f32, ui: &Ui) -> u32 {
    let (ml, mt, mb) = margins(ui.maximized);
    let (wf, hf) = ((w - 2 * ml) as f32, (h - mt - mb) as f32);
    // Window pixel, sampled at its centre.
    let (px, py) = ((x as isize - ml as isize) as f32 + 0.5, (y as isize - mt as isize) as f32 + 0.5);
    let r = if ui.maximized { 0.0 } else { RADIUS };

    let d = rounded_rect(px, py, wf, hf, r);
    let cov = (0.5 - d).clamp(0.0, 1.0);
    let shade = if ui.maximized || cov == 1.0 { 0.0 } else { shadow(px, py, wf, hf) * (1.0 - cov) };
    let q = |c: f32| ((c * 255.0 + 0.5) as u32).min(255);
    if cov == 0.0 {
        return q(shade) << 24;
    }

    // One glass everywhere: the title bar is not a pane of its own.
    let a0 = opacity.clamp(0.0, 1.0);
    let mut p = P { r: GLASS * a0, g: GLASS * a0, b: GLASS * a0, a: a0 };
    // The outline: the outermost pixel, the same everywhere (none when square).
    if !ui.maximized {
        p = P::white(OUTLINE * (1.5 + d).clamp(0.0, 1.0)).over(p);
    }
    p = buttons(p, px, py, ui);

    // The window over its own shadow (only its anti-aliased edge lets any through).
    let p = p.scale(cov);
    let a = p.a + shade;
    q(a) << 24 | q(p.r) << 16 | q(p.g) << 8 | q(p.b)
}

#[cfg(test)]
mod tests {
    use super::*;

    const UI: Ui = Ui::IDLE;

    /// An 800 x 600 window: its buffer, and buffer pixel `(x, y)` of window pixel `(x, y)`.
    const WIN: (usize, usize) = (800, 600);
    fn buf(ui: &Ui) -> (usize, usize, usize, usize) {
        let (ml, mt, mb) = margins(ui.maximized);
        (WIN.0 + 2 * ml, WIN.1 + mt + mb, ml, mt)
    }
    fn at(x: usize, y: usize, ui: &Ui) -> u32 {
        let (w, h, ml, mt) = buf(ui);
        pixel(x + ml, y + mt, w, h, 0.8, ui)
    }

    #[test]
    fn corners_are_clear_and_the_body_is_not() {
        let (w, h, ..) = buf(&UI);
        // The buffer corners are beyond the shadow; the window corners only show a little of it.
        assert_eq!(pixel(0, 0, w, h, 0.8, &UI) >> 24, 0);
        assert_eq!(pixel(w - 1, h - 1, w, h, 0.8, &UI) >> 24, 0);
        assert!(at(0, 0, &UI) >> 24 < 60);
        assert!(at(400, 300, &UI) >> 24 > 150);
    }

    #[test]
    fn the_shadow_is_black_falls_downward_and_fades_out_inside_the_margin() {
        let (w, h, ml, mt) = buf(&UI);
        let below = pixel(ml + 400, mt + WIN.1 + 4, w, h, 0.8, &UI);
        let above = pixel(ml + 400, mt - 4, w, h, 0.8, &UI);
        assert_eq!(below & 0x00ff_ffff, 0, "black");
        assert!(below >> 24 > above >> 24, "cast downward");
        for (x, y) in [(ml + 400, 0), (ml + 400, h - 1), (0, mt + 300), (w - 1, mt + 300)] {
            assert_eq!(pixel(x, y, w, h, 0.8, &UI) >> 24, 0, "edge of the buffer at ({x}, {y})");
        }
    }

    #[test]
    fn the_glass_is_continuous_and_the_outline_uniform() {
        // The title bar is the same glass as the grid, top to bottom.
        let body = at(400, 300, &UI);
        for y in [2, 12, TITLE_H - 1, TITLE_H, 100, WIN.1 - 2] {
            assert_eq!(at(400, y, &UI), body, "row {y}");
        }
        // The outline is the same on all four sides, and only one pixel deep.
        let edge = at(400, 0, &UI);
        assert_ne!(edge, body);
        assert_eq!(at(400, WIN.1 - 1, &UI), edge);
        assert_eq!(at(0, 300, &UI), edge);
        assert_eq!(at(WIN.0 - 1, 300, &UI), edge);
        assert_eq!(at(1, 300, &UI), body);
    }

    #[test]
    fn maximized_windows_have_square_corners_and_no_margin() {
        let ui = Ui { maximized: true, ..UI };
        assert_eq!(margins(true), (0, 0, 0));
        assert!(pixel(0, 0, 800, 600, 0.8, &ui) >> 24 > 150);
    }

    #[test]
    fn rows_are_constant_away_from_the_side_edges() {
        // What soft.rs relies on to fill the middle of a row with one value, margin rows included.
        let (w, h, ml, mt) = buf(&UI);
        for y in [2, mt + REACH, mt + 300, mt + WIN.1 - REACH - 1, h - 3] {
            assert_eq!(pixel(ml + REACH, y, w, h, 0.8, &UI), pixel(w / 2, y, w, h, 0.8, &UI), "row {y}");
        }
    }

    #[test]
    fn hover_lights_the_button_and_draws_the_glyph() {
        let (cx, cy) = button_center(1);
        let blue = |x: f32, ui: &Ui| at(x as usize, cy as usize, ui) & 255;
        let hover = Ui { hover: 1, ..UI };
        // Off the glyph, the disc is lighter under the pointer; at the centre the glyph makes it darker.
        assert!(blue(cx - 4.0, &hover) > blue(cx - 4.0, &UI));
        assert!(blue(cx, &hover) < blue(cx, &UI));
    }

    #[test]
    fn hit_testing_finds_buttons_title_edges_and_client() {
        let h = |x, y| hit(x, y, 800, 600, false);
        assert_eq!(h(20.0, 19.0), Hit::Close);
        assert_eq!(h(40.0, 19.0), Hit::Minimize);
        assert_eq!(h(60.0, 19.0), Hit::Maximize);
        assert_eq!(h(400.0, 19.0), Hit::Title);
        assert_eq!(h(400.0, 300.0), Hit::Client);
        assert_eq!(h(2.0, 300.0), Hit::Edge(EDGE_LEFT));
        assert_eq!(h(400.0, 598.0), Hit::Edge(EDGE_BOTTOM));
        assert_eq!(h(2.0, 2.0), Hit::Edge(EDGE_TOP | EDGE_LEFT));
        assert_eq!(h(790.0, 3.0), Hit::Edge(EDGE_TOP | EDGE_RIGHT));
        assert_eq!(h(797.0, 590.0), Hit::Edge(EDGE_RIGHT | EDGE_BOTTOM));
        assert_eq!(hit(2.0, 300.0, 800, 600, true), Hit::Client);
        // Over the shadow: a grab zone just outside the edge, nothing beyond it.
        assert_eq!(h(-5.0, 300.0), Hit::Edge(EDGE_LEFT));
        assert_eq!(h(400.0, 605.0), Hit::Edge(EDGE_BOTTOM));
        assert_eq!(h(-5.0, -5.0), Hit::Edge(EDGE_TOP | EDGE_LEFT));
        assert_eq!(h(-20.0, 300.0), Hit::Client);
    }
}
