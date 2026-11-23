//! Window chrome on the CPU renderer: geometry, hit testing and the glass look.
//!
//! Nothing here is stored. Every pixel of the frame background is a pure function of its position,
//! the window size and a few bits of UI state (`Ui`), so there is no texture, no mask and no cache:
//! a rounded rectangle (signed distance), a translucent black body, a white sheen from the top, a
//! 1 px light rim that fades toward the bottom, and the three window buttons in the title bar.
//! Black and white on purpose; the only colour on screen is what programs ask for.

/// Corner radius of the window, in pixels (0 when maximized).
pub const RADIUS: f32 = 14.0;
/// Height of the title bar. The terminal grid starts below it.
pub const TITLE_H: usize = 38;
/// Pixels from an edge beyond which the background no longer varies along a row.
pub const REACH: usize = 32;
/// Left edge of the area the title text must stay clear of; the right side mirrors it.
pub const TITLE_CLEAR: usize = 90;

const BTN_R: f32 = 6.5;
const BTN_X0: f32 = 20.0;
const BTN_DX: f32 = 20.0;
/// Grab zone around a window edge, and the longer zone that turns it into a corner.
const EDGE: f32 = 6.0;
const CORNER: f32 = 16.0;

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
    pub maximized: bool,
}

impl Ui {
    pub const IDLE: Ui = Ui { hover: 0, down: false, focused: true, maximized: false };
}

fn button_center(i: u8) -> (f32, f32) {
    (BTN_X0 + (i - 1) as f32 * BTN_DX, TITLE_H as f32 * 0.5)
}

/// What is at `(x, y)` in a `w` x `h` window.
pub fn hit(x: f32, y: f32, w: u32, h: u32, maximized: bool) -> Hit {
    let (w, h) = (w as f32, h as f32);
    if !maximized {
        let mut edges = 0;
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

/// Coverage of a 1.4 px line at distance `d`.
fn line(d: f32) -> f32 {
    (1.2 - d).clamp(0.0, 1.0)
}

/// The glyph on button `i` (close, minimize, maximize) at `(px, py)`.
fn glyph(i: u8, px: f32, py: f32) -> f32 {
    let (cx, cy) = button_center(i);
    let k = 2.7;
    match i {
        1 => line(seg(px, py, (cx - k, cy - k), (cx + k, cy + k))).max(line(seg(px, py, (cx - k, cy + k), (cx + k, cy - k)))),
        2 => line(seg(px, py, (cx - k - 0.4, cy), (cx + k + 0.4, cy))),
        _ => line(seg(px, py, (cx - k - 0.4, cy), (cx + k + 0.4, cy))).max(line(seg(px, py, (cx, cy - k - 0.4), (cx, cy + k + 0.4)))),
    }
}

/// The three buttons over `base`, at pixel centre `(px, py)`.
fn buttons(mut base: P, px: f32, py: f32, ui: &Ui) -> P {
    if px > BTN_X0 + 2.0 * BTN_DX + BTN_R + 2.0 || (py - TITLE_H as f32 * 0.5).abs() > BTN_R + 2.0 {
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

/// The frame background at pixel `(x, y)` of a `w` x `h` window, premultiplied `Argb8888`.
/// `opacity` is the alpha of the glass body.
pub fn pixel(x: usize, y: usize, w: usize, h: usize, opacity: f32, ui: &Ui) -> u32 {
    let (wf, hf) = (w as f32, h as f32);
    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
    let r = if ui.maximized { 0.0 } else { RADIUS };

    // Signed distance to the rounded rectangle: negative inside.
    let qx = (px - wf * 0.5).abs() - (wf * 0.5 - r);
    let qy = (py - hf * 0.5).abs() - (hf * 0.5 - r);
    let d = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r;
    let cov = (0.5 - d).clamp(0.0, 1.0);
    if cov == 0.0 {
        return 0;
    }
    let inside = (-d).max(0.0);
    let v = (py / hf).clamp(0.0, 1.0);

    // Body: near-black glass.
    let a0 = opacity.clamp(0.0, 1.0);
    let mut p = P { r: 0.035 * a0, g: 0.035 * a0, b: 0.035 * a0, a: a0 };
    // Sheen from the top, light catching the glass.
    let sheen = (1.0 - py / 200.0).clamp(0.0, 1.0);
    p = P::white(0.10 * sheen * sheen).over(p);
    // Title bar: a slightly lighter pane with a hairline under it.
    if y < TITLE_H {
        p = P::white(0.045).over(p);
        if y == TITLE_H - 1 {
            p = P::white(0.09).over(p);
        }
    }
    // Light gathering at the edges, stronger near the top.
    p = P::white(0.075 * (-inside / 5.0).exp() * (1.2 - v)).over(p);
    // The rim: 1 px, bright on top and fading toward the bottom.
    if !ui.maximized {
        let ring = (1.5 - inside).clamp(0.0, 1.0);
        p = P::white(ring * (0.46 - 0.30 * v)).over(p);
    }
    p = buttons(p, px, py, ui);

    let p = p.scale(cov);
    let q = |c: f32| ((c * 255.0 + 0.5) as u32).min(255);
    q(p.a) << 24 | q(p.r) << 16 | q(p.g) << 8 | q(p.b)
}

#[cfg(test)]
mod tests {
    use super::*;

    const UI: Ui = Ui::IDLE;

    #[test]
    fn corners_are_transparent_and_the_body_is_not() {
        assert_eq!(pixel(0, 0, 800, 600, 0.8, &UI) >> 24, 0);
        assert_eq!(pixel(799, 599, 800, 600, 0.8, &UI) >> 24, 0);
        assert!(pixel(400, 300, 800, 600, 0.8, &UI) >> 24 > 150);
    }

    #[test]
    fn maximized_windows_have_square_corners() {
        let ui = Ui { maximized: true, ..UI };
        assert!(pixel(0, 0, 800, 600, 0.8, &ui) >> 24 > 150);
    }

    #[test]
    fn the_body_is_constant_along_a_row_away_from_the_edges() {
        for y in [REACH, 100, 300, 600 - REACH - 1] {
            assert_eq!(pixel(REACH, y, 800, 600, 0.8, &UI), pixel(400, y, 800, 600, 0.8, &UI), "row {y}");
        }
    }

    #[test]
    fn hover_lights_the_button_and_draws_the_glyph() {
        let (cx, cy) = button_center(1);
        let blue = |x: f32, ui: &Ui| pixel(x as usize, cy as usize, 800, 600, 0.8, ui) & 255;
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
    }
}
