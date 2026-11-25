//! Window chrome on the CPU renderer: geometry, hit testing and the glass look.
//!
//! Nothing here is stored. Every pixel of the frame background is a pure function of its position,
//! the window size and a few bits of UI state (`Ui`), so there is no texture, no mask and no cache:
//! a rounded rectangle (signed distance) of one continuous translucent black glass, a faint uniform
//! outline, the three window buttons and the title floating on it, and a soft shadow in a margin
//! around the window (Mutter draws none for client-decorated windows).
//!
//! One glass for the whole window (`knowledge/decisions/0015`), now with the structure of the
//! `terminal.html` reference (ADR 0021): a hairline under the title bar and above the status bar,
//! a faint top-light on the title bar, grey window buttons that take their traffic-light colour
//! when the pointer is over them, and a sliders icon for the settings. Nothing on the edge changes
//! along it, so no line gathers in the corners.
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
/// The glass tint (premultiplied by the opacity: `#0b0b0c`) and the white of the 1 px outline: the
/// same all around the window, so it reads as the glass's edge, not as a line of its own.
const GLASS: f32 = 0.043;
const OUTLINE: f32 = 0.07;
/// White of the hairlines (under the title bar, above the status bar), of the title bar's top light
/// (at its top, fading to nothing), of the 1 px highlight inside the top edge and of the status bar.
const LINE: f32 = 0.07;
const TITLE_LIGHT: f32 = 0.035;
const TOP_LIGHT: f32 = 0.05;
const STATUS_LIGHT: f32 = 0.02;
/// Height of the title bar (its last row is the hairline): where the buttons and the title sit and
/// the window can be dragged. The terminal grid starts below it.
pub const TITLE_H: usize = 42;
/// Height of the status bar at the bottom of the window (its first row is the hairline).
pub const STATUS_H: usize = 28;
/// Pixels from an edge beyond which the background no longer varies along a row.
pub const REACH: usize = 32;
/// Left edge of the area the title text must stay clear of; the right side mirrors it.
pub const TITLE_CLEAR: usize = 118;

// Pixels are sampled at their centre (`x + 0.5`), so the button centres sit on `n + 0.5`: a 13 px
// disc then ends exactly on pixel borders and a 1 px stroke through the centre fills one row.
const BTN_R: f32 = 6.0;
const BTN_X0: f32 = 20.0;
const BTN_DX: f32 = 20.0;
const BTN_CY: f32 = 21.0;
/// The settings button is a 28 px square whose centre is this far from the right edge of the window.
const GEAR_FROM_RIGHT: f32 = 28.0;
const GEAR_HALF: f32 = 14.0;
/// The button that shows and hides the panel of windows: a 28 px square right of the window buttons.
const SIDE_CX: f32 = 90.0;
/// Resting grey of the window buttons and the colours they take under the pointer (`#3a3a3e`,
/// close `#ff5f57`, minimize `#febc2e`, maximize `#28c840`).
const BTN_REST: [f32; 3] = [0.227, 0.227, 0.243];
const BTN_HOT: [[f32; 3]; 3] = [[1.0, 0.373, 0.341], [0.996, 0.737, 0.18], [0.157, 0.784, 0.251]];
/// The icon of the settings button: dim, and the foreground colour under the pointer.
const ICON_DIM: [f32; 3] = [0.455, 0.455, 0.490];
const ICON_FG: [f32; 3] = [0.851, 0.851, 0.871];
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
    /// The gear at the right end of the title bar: the settings panel.
    Settings,
    /// The button right of the window buttons: shows or hides the panel of windows.
    SideBtn,
    Title,
    /// The panel of windows (the app finds out what in it; `side.rs`).
    Side,
    /// The settings panel docked beside the grid (the panel finds out what in it; `settings.rs`).
    Prefs,
    /// A resize edge or corner: `EDGE_*` bits.
    Edge(u32),
}

impl Hit {
    /// 1..=3 for the window buttons, 4 for the gear, 5 for the panel button, 0 for everything else.
    pub fn button(self) -> u8 {
        match self {
            Hit::Close => 1,
            Hit::Minimize => 2,
            Hit::Maximize => 3,
            Hit::Settings => 4,
            Hit::SideBtn => 5,
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
    /// The status bar is shown: its hairline and its faint tint belong to the frame.
    pub status: bool,
    /// The panel button: 0 absent (the windows are tabs at the top), 1 the panel is hidden, 2 it is shown.
    pub side: u8,
    /// The settings panel is open: the gear stays lit, as the reference's `aria-expanded`.
    pub prefs: bool,
}

impl Ui {
    pub const IDLE: Ui = Ui { hover: 0, down: false, focused: true, maximized: false, status: true, side: 1, prefs: false };
}

fn button_center(i: u8) -> (f32, f32) {
    (BTN_X0 + (i - 1) as f32 * BTN_DX, BTN_CY)
}

/// Space the shadow needs around the window: (left and right, top, bottom). None when square.
pub fn margins(maximized: bool) -> (usize, usize, usize) {
    if maximized { (0, 0, 0) } else { (SHADOW_BLUR, SHADOW_BLUR - SHADOW_DY, SHADOW_BLUR + SHADOW_DY) }
}

/// What is at `(x, y)` in a `w` x `h` window, in window pixels (negative over the shadow margin).
/// The panel button counts when it is there (`Ui::side != 0`: `side_btn`).
pub fn hit(x: f32, y: f32, w: u32, h: u32, maximized: bool, side_btn: bool) -> Hit {
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
    if side_btn && (x - SIDE_CX).abs() <= GEAR_HALF && (y - BTN_CY).abs() <= GEAR_HALF {
        return Hit::SideBtn;
    }
    if (x - (w - GEAR_FROM_RIGHT)).abs() <= GEAR_HALF && (y - BTN_CY).abs() <= GEAR_HALF {
        return Hit::Settings;
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

/// Coverage of a 1.3 px stroke at distance `d` from its axis (the lines of the window glyphs).
fn line(d: f32) -> f32 {
    (0.65 + 0.5 - d).clamp(0.0, 1.0)
}

/// The glyph on button `i` (close, minimize, maximize) at `(px, py)`: a cross, a dash, a square.
fn glyph(i: u8, px: f32, py: f32) -> f32 {
    let (cx, cy) = button_center(i);
    let k = 2.2;
    match i {
        1 => line(seg(px, py, (cx - k, cy - k), (cx + k, cy + k))).max(line(seg(px, py, (cx - k, cy + k), (cx + k, cy - k)))),
        2 => line(seg(px, py, (cx - k, cy), (cx + k, cy))),
        _ => {
            let q = 1.75;
            let d = (px - cx).abs().max((py - cy).abs()) - q;
            line(d)
        }
    }
}

/// A colour `c` (0..1) at opacity `a`, premultiplied.
fn tint(c: [f32; 3], a: f32) -> P {
    P { r: c[0] * a, g: c[1] * a, b: c[2] * a, a }
}

/// The three buttons over `base`, at pixel centre `(px, py)`.
fn buttons(mut base: P, px: f32, py: f32, ui: &Ui) -> P {
    // Hovering one of the three colours all three and shows their glyphs; the sliders are their own thing.
    let group = (1..=3).contains(&ui.hover);
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
        let pressed = ui.down && ui.hover == i;
        let (colour, a) = if group { (BTN_HOT[i as usize - 1], if pressed { 0.72 } else { 1.0 }) } else { (BTN_REST, if ui.focused { 1.0 } else { 0.55 }) };
        base = tint(colour, a * cov).over(base);
        if group {
            base = P::black(glyph(i, px, py) * 0.65).over(base);
        }
    }
    base
}

/// Coverage of a ring of radius `r` and stroke `t` around `(cx, cy)`.
fn ring(px: f32, py: f32, cx: f32, cy: f32, r: f32, t: f32) -> f32 {
    (t / 2.0 + 0.5 - ((px - cx).hypot(py - cy) - r).abs()).clamp(0.0, 1.0)
}

/// Coverage of the settings icon (two sliders, on a 16 px grid) at offset `(dx, dy)` from its centre.
fn sliders(dx: f32, dy: f32) -> f32 {
    let (x, y) = (dx + 8.0, dy + 8.0);
    let l = |a: (f32, f32), b: (f32, f32)| (0.75 + 0.5 - seg(x, y, a, b)).clamp(0.0, 1.0);
    l((2.5, 4.5), (8.5, 4.5))
        .max(l((11.5, 4.5), (13.5, 4.5)))
        .max(l((2.5, 11.5), (4.5, 11.5)))
        .max(l((7.5, 11.5), (13.5, 11.5)))
        .max(ring(x, y, 10.0, 4.5, 1.5, 1.5))
        .max(ring(x, y, 6.0, 11.5, 1.5, 1.5))
}

/// The settings button over `base` at window pixel `(px, py)`: a dim sliders icon that turns to the
/// foreground colour on a rounded highlight when the pointer is over it or the settings panel is open.
fn gear_button(base: P, px: f32, py: f32, wf: f32, ui: &Ui) -> P {
    let (dx, dy) = (px - (wf - GEAR_FROM_RIGHT), py - BTN_CY);
    if dx.abs() > GEAR_HALF + 1.0 || dy.abs() > GEAR_HALF + 1.0 {
        return base;
    }
    let (hovered, on) = (ui.hover == 4, ui.prefs);
    let mut p = base;
    if hovered || on {
        let d = rounded_rect(dx + GEAR_HALF, dy + GEAR_HALF, 2.0 * GEAR_HALF, 2.0 * GEAR_HALF, 8.0);
        p = P::white(if hovered && ui.down { 0.12 } else { 0.07 } * (0.5 - d).clamp(0.0, 1.0)).over(p);
    }
    let (colour, a) = match (hovered || on, ui.focused) {
        (true, _) => (ICON_FG, 1.0),
        (false, true) => (ICON_DIM, 1.0),
        (false, false) => (ICON_DIM, 0.55),
    };
    tint(colour, a * sliders(dx, dy)).over(p)
}

/// Coverage of the panel icon (a rounded frame with a vertical divider, on a 16 px grid) at offset `(dx, dy)`
/// from its centre.
fn sidebar(dx: f32, dy: f32) -> f32 {
    let (x, y) = (dx + 8.0, dy + 8.0);
    let frame = (0.75 + 0.5 - rounded_rect(x - 2.5, y - 3.0, 11.0, 10.0, 2.0).abs()).clamp(0.0, 1.0);
    frame.max((0.75 + 0.5 - seg(x, y, (6.5, 3.0), (6.5, 13.0))).clamp(0.0, 1.0))
}

/// The panel button over `base`: dim, and lit on a rounded highlight under the pointer and while the panel is shown.
fn side_button(base: P, px: f32, py: f32, ui: &Ui) -> P {
    let (dx, dy) = (px - SIDE_CX, py - BTN_CY);
    if ui.side == 0 || dx.abs() > GEAR_HALF + 1.0 || dy.abs() > GEAR_HALF + 1.0 {
        return base;
    }
    let (hovered, on) = (ui.hover == 5, ui.side == 2);
    let mut p = base;
    if hovered || on {
        let d = rounded_rect(dx + GEAR_HALF, dy + GEAR_HALF, 2.0 * GEAR_HALF, 2.0 * GEAR_HALF, 8.0);
        p = P::white(if hovered && ui.down { 0.12 } else { 0.07 } * (0.5 - d).clamp(0.0, 1.0)).over(p);
    }
    let (colour, a) = match (hovered || on, ui.focused) {
        (true, _) => (ICON_FG, 1.0),
        (false, true) => (ICON_DIM, 1.0),
        (false, false) => (ICON_DIM, 0.55),
    };
    tint(colour, a * sidebar(dx, dy)).over(p)
}

/// Signed distance from `(px, py)` to a `w` x `h` rectangle at the origin with corner radius `r`:
/// negative inside.
pub fn rounded_rect(px: f32, py: f32, w: f32, h: f32, r: f32) -> f32 {
    let qx = (px - w * 0.5).abs() - (w * 0.5 - r);
    let qy = (py - h * 0.5).abs() - (h * 0.5 - r);
    qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r
}

/// Most rectangles `blur_strips` returns: two per corner row and the middle.
pub const BLUR_STRIPS: usize = RADIUS as usize * 2 + 1;

/// The window's rounded outline at `(x, y, w, h)` as rectangles, for the compositor's blur region: one
/// strip per row of each rounded corner (the first and last `radius` rows) and one for the middle.
/// Returns them and how many there are.
pub fn blur_strips(x: i32, y: i32, w: i32, h: i32, radius: i32) -> ([[i32; 4]; BLUR_STRIPS], usize) {
    let mut out = [[0; 4]; BLUR_STRIPS];
    let r = radius.clamp(0, RADIUS as i32).min(h / 2).min(w / 2);
    let mut n = 0;
    for i in 0..r {
        // Row i from the edge: the corner circle leaves this much out on each side.
        let dy = (r - i) as f32 - 0.5;
        let inset = r - ((r * r) as f32 - dy * dy).max(0.0).sqrt().round() as i32;
        out[n] = [x + inset, y + i, w - 2 * inset, 1];
        out[n + 1] = [x + inset, y + h - 1 - i, w - 2 * inset, 1];
        n += 2;
    }
    out[n] = [x, y + r, w, h - 2 * r];
    (out, n + 1)
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

    // One glass; the title bar and the status bar only add light and a hairline to it.
    let a0 = opacity.clamp(0.0, 1.0);
    let mut p = P { r: GLASS * a0, g: GLASS * a0, b: GLASS * a0, a: a0 };
    // The outline: the outermost pixel, the same everywhere (none when square).
    if !ui.maximized {
        p = P::white(OUTLINE * (1.5 + d).clamp(0.0, 1.0)).over(p);
        if (1.0..2.0).contains(&py) && d <= -1.0 {
            p = P::white(TOP_LIGHT).over(p); // the 1 px light inside the top edge
        }
    }
    if py < TITLE_H as f32 {
        // A top light that fades out down the title bar, and the hairline that ends it.
        p = P::white(TITLE_LIGHT * (1.0 - py / TITLE_H as f32)).over(p);
        if py >= TITLE_H as f32 - 1.0 {
            p = P::white(LINE).over(p);
        }
    }
    if ui.status && py >= hf - STATUS_H as f32 {
        p = P::white(STATUS_LIGHT).over(p);
        if py < hf - STATUS_H as f32 + 1.0 {
            p = P::white(LINE).over(p);
        }
    }
    p = buttons(p, px, py, ui);
    p = gear_button(p, px, py, wf, ui);
    p = side_button(p, px, py, ui);

    // The window over its own shadow (only its anti-aliased edge lets any through).
    let p = p.scale(cov);
    let a = p.a + shade;
    q(a) << 24 | q(p.r) << 16 | q(p.g) << 8 | q(p.b)
}

#[cfg(test)]
mod tests {
    #[test]
    fn blur_strips_follow_the_rounded_outline() {
        let (x, y, w, h) = (26, 18, 300, 200);
        let (rects, n) = super::blur_strips(x, y, w, h, super::RADIUS as i32);
        assert_eq!(n, super::BLUR_STRIPS);
        let area: i32 = rects[..n].iter().map(|r| r[2] * r[3]).sum();
        // A rounded rectangle has (4 - pi) r^2 less area than the plain one.
        let full = w * h;
        let lost = (full - area) as f32;
        let expect = (4.0 - std::f32::consts::PI) * super::RADIUS * super::RADIUS;
        assert!((lost - expect).abs() < expect * 0.12, "lost {lost}, expected about {expect}");
        for r in &rects[..n] {
            assert!(r[0] >= x && r[1] >= y && r[0] + r[2] <= x + w && r[1] + r[3] <= y + h, "{r:?} leaves the window");
            assert!(r[2] > 0 && r[3] > 0);
            assert_eq!(r[0] - x, x + w - (r[0] + r[2]), "{r:?} is not symmetric");
        }
        // The top row is inset the most; the strips are exactly the window when square (maximized).
        assert!(rects[0][0] > rects[n - 3][0]);
        let (sq, n) = super::blur_strips(0, 0, 100, 50, 0);
        assert_eq!((n, sq[0]), (1, [0, 0, 100, 50]));
        // A window smaller than its corners does not make a negative strip.
        let (tiny, n) = super::blur_strips(0, 0, 10, 6, super::RADIUS as i32);
        assert!(tiny[..n].iter().all(|r| r[2] > 0 && r[3] >= 0));
    }

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
    fn the_glass_is_one_surface_with_hairlines_between_the_bars_and_the_grid() {
        // Away from the bars the glass is the same top to bottom.
        let body = at(400, 300, &UI);
        for y in [TITLE_H, 100, WIN.1 - STATUS_H - 2] {
            assert_eq!(at(400, y, &UI), body, "row {y}");
        }
        // The hairline closes the title bar and opens the status bar; both are lighter than the glass.
        let lum = |p: u32| (p >> 8) & 255;
        assert!(lum(at(400, TITLE_H - 1, &UI)) > lum(at(400, TITLE_H - 3, &UI)));
        assert!(lum(at(400, WIN.1 - STATUS_H, &UI)) > lum(at(400, WIN.1 - STATUS_H + 2, &UI)));
        assert!(lum(at(400, WIN.1 - STATUS_H + 2, &UI)) > lum(body), "the status bar is tinted");
        // The title bar's top light fades out downward.
        assert!(lum(at(400, 3, &UI)) > lum(at(400, TITLE_H - 3, &UI)));
        // Without a status bar the bottom is plain glass.
        let none = Ui { status: false, ..UI };
        assert_eq!(at(400, WIN.1 - STATUS_H, &none), at(400, 300, &none));
        // The outline is the same on all four sides, and only one pixel deep.
        let edge = at(400, 200, &UI);
        let side = at(0, 300, &UI);
        assert_ne!(side, body);
        assert_eq!(at(WIN.0 - 1, 300, &UI), side);
        assert_eq!(at(1, 300, &UI), body);
        assert_eq!(at(400, WIN.1 - 1, &none), side);
        let _ = edge;
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
        // Resting discs are grey; under the pointer the close button turns red (blue falls) and its
        // glyph is darker than the disc at the centre.
        let green = |x: f32, ui: &Ui| at(x as usize, cy as usize, ui) >> 8 & 255;
        assert!(green(cx - 4.0, &hover) > 0 && blue(cx - 4.0, &hover) > blue(cx - 4.0, &UI) / 2);
        assert!(green(cx - 4.0, &hover) > green(cx - 4.0, &UI), "red is a bright colour");
        assert!(green(cx, &hover) < green(cx - 4.0, &hover), "the glyph is darker than the disc");
    }

    #[test]
    fn hit_testing_finds_buttons_title_edges_and_client() {
        let h = |x, y| hit(x, y, 800, 600, false, false);
        assert_eq!(h(20.0, 21.0), Hit::Close);
        assert_eq!(h(40.0, 21.0), Hit::Minimize);
        assert_eq!(h(60.0, 21.0), Hit::Maximize);
        assert_eq!(h(400.0, 21.0), Hit::Title);
        assert_eq!(h(800.0 - 28.0, 21.0), Hit::Settings);
        assert_eq!(h(800.0 - 50.0, 21.0), Hit::Title);
        assert_eq!(Hit::Settings.button(), 4);
        assert_eq!(h(400.0, 300.0), Hit::Client);
        assert_eq!(h(2.0, 300.0), Hit::Edge(EDGE_LEFT));
        assert_eq!(h(400.0, 598.0), Hit::Edge(EDGE_BOTTOM));
        assert_eq!(h(2.0, 2.0), Hit::Edge(EDGE_TOP | EDGE_LEFT));
        assert_eq!(h(790.0, 3.0), Hit::Edge(EDGE_TOP | EDGE_RIGHT));
        assert_eq!(h(797.0, 590.0), Hit::Edge(EDGE_RIGHT | EDGE_BOTTOM));
        assert_eq!(hit(2.0, 300.0, 800, 600, true, false), Hit::Client);
        // The panel button is there only when asked for; without it that spot is the title bar.
        assert_eq!(hit(SIDE_CX, 21.0, 800, 600, false, true), Hit::SideBtn);
        assert_eq!(hit(SIDE_CX, 21.0, 800, 600, false, false), Hit::Title);
        assert_eq!(Hit::SideBtn.button(), 5);
        // Over the shadow: a grab zone just outside the edge, nothing beyond it.
        assert_eq!(h(-5.0, 300.0), Hit::Edge(EDGE_LEFT));
        assert_eq!(h(400.0, 605.0), Hit::Edge(EDGE_BOTTOM));
        assert_eq!(h(-5.0, -5.0), Hit::Edge(EDGE_TOP | EDGE_LEFT));
        assert_eq!(h(-20.0, 300.0), Hit::Client);
    }

    #[test]
    fn the_settings_button_is_drawn_lights_up_on_hover_and_leaves_the_window_buttons_alone() {
        let lum = |ui: &Ui, x: usize, y: usize| {
            let (ml, mt, mb) = margins(ui.maximized);
            let (bw, bh) = (WIN.0 + 2 * ml, WIN.1 + mt + mb);
            pixel(ml + x, mt + y, bw, bh, 0.88, ui) >> 8 & 255
        };
        // The icon's first slider line runs along y = 16.5 + 4.5 - 8 + ... : find its ink by scanning the button.
        let (cx, cy) = (WIN.0 - 28, 21usize);
        let ink = |ui: &Ui| (cx - 8..cx + 8).flat_map(|x| (cy - 8..cy + 8).map(move |y| (x, y))).map(|(x, y)| lum(ui, x, y)).max().unwrap();
        let off = lum(&UI, 700, 21);
        assert!(ink(&UI) > off + 40, "the icon is lighter than the glass");
        let hover = Ui { hover: 4, ..UI };
        assert!(ink(&hover) > ink(&UI), "hover brightens it");
        assert!(lum(&hover, cx + 12, cy) > lum(&UI, cx + 12, cy), "and lights a rounded highlight around it");
        // Hovering it does not colour the three discs.
        for x in 14..70 {
            assert_eq!(lum(&hover, x, 21), lum(&UI, x, 21), "x = {x}");
        }
        // While the settings panel is open it stays lit, without the pointer: the reference's `aria-expanded`.
        let open = Ui { prefs: true, ..UI };
        assert!(ink(&open) > ink(&UI));
        assert_eq!(ink(&open), ink(&hover));
        assert!(lum(&open, cx + 12, cy) > lum(&UI, cx + 12, cy));
        assert_eq!(Hit::Prefs.button(), 0, "the panel itself is not a button");
    }
}
