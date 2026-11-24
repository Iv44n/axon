//! The prompt chips: a row of rounded pills (time, user, directory, git branch...) on the line right
//! above the prompt the user is typing at, drawn with the grid's own font at 90 % of its size and
//! rasterized on the fly (no tiles of their own).
//!
//! The line is one the shell reserved (`view::prompt_row`); the chips are drawn over its blank cells.
//! A framebuffer that is repainted row by row must still equal a full repaint, so `Chips::frame`
//! repaints the row a chip row appears on, moves from or changes on, and draws the chips again only
//! on rows that were repainted: never twice over the same pixels.
//! Coordinates are window pixels, the origin the window's top-left corner (not the shadow's).
use alacritty_terminal::{Term, event::EventListener, vte::ansi::Rgb};

use crate::{
    cell::pack,
    palette,
    config::Chip,
    context::Context,
    history::{self, History, Run},
    i18n::{Str, tr},
    paint::{Paint, seg},
    soft::{Soft, Target},
    view::{self, View},
};

/// Chips are drawn at this fraction of the font size set in the settings: text, icon, padding and height
/// (a 14 px font gives 12.6 px chips).
pub const SCALE: f32 = 0.9;

/// The size, in pixels, of the parts of a chip. They are designed for 14 px text (`k = 1`: icon 14, padding
/// 9, icon gap 6, chip gap 8) and scale with it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dims {
    /// Width of a character cell of the chip text.
    pub cw: usize,
    /// Scale from the 14 px design.
    pub k: f32,
    /// Space inside a chip, left and right of its content.
    pub pad: usize,
    pub icon: usize,
    /// Space between the icon and the text.
    pub icon_gap: usize,
    /// Space between chips.
    pub gap: usize,
}

impl Dims {
    /// For chip text of `px` pixels whose character cell is `cw` wide.
    pub fn new(px: f32, cw: usize) -> Dims {
        let k = px / 14.0;
        let at = |v: f32| (v * k).round().max(1.0) as usize;
        Dims { cw, k, pad: at(9.0), icon: at(14.0), icon_gap: at(6.0), gap: at(8.0) }
    }

    /// A chip with `chars` characters of text.
    fn width(&self, chars: usize) -> usize {
        2 * self.pad + self.icon + self.icon_gap + chars * self.cw
    }
}

/// The colour of each chip, indexed by `Chip as usize`: the hues of the `terminal.html` reference.
fn colors() -> [[f32; 3]; Chip::COUNT] {
    [
        palette::unit(palette::RED),                // time
        palette::unit(palette::GREEN),              // user
        palette::unit(palette::CYAN),               // env
        palette::unit(palette::ACCENT),             // pyenv
        palette::unit(Rgb { r: 0xa1, g: 0xa1, b: 0xaa }), // host
        palette::unit(palette::MAGENTA),            // dir
        palette::unit(palette::YELLOW),             // branch
        palette::unit(Rgb { r: 0xff, g: 0x8a, b: 0x4c }), // changes
        palette::unit(palette::BLUE),               // kube
    ]
}

/// The chip's colour.
pub fn color(chip: Chip) -> [f32; 3] {
    colors()[chip as usize]
}

/// A chip that is shown: which one, where it starts, how wide it is and how many characters of its
/// text fit, in window pixels (the text is cut with `…` when `chars` is less than its length).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slot {
    pub chip: Chip,
    pub x: usize,
    pub w: usize,
    pub chars: usize,
}

/// The shortest a chip's text is cut to before the chip is dropped instead.
const MIN_CHARS: usize = 8;

/// Where the chips go: `chips` in order from `left`, skipping the ones with no text. When they do not
/// all fit before `right`, the longest texts are cut (down to `MIN_CHARS`) and, if that is not
/// enough, the chips at the end are dropped.
pub fn layout(ctx: &Context, chips: &[Chip], d: &Dims, left: usize, right: usize) -> Vec<Slot> {
    let mut shown: Vec<(Chip, usize, usize)> = chips
        .iter()
        .filter_map(|&c| Some((c, ctx.get(c).chars().count(), 0)).filter(|t| t.1 > 0))
        .map(|(c, n, _)| (c, n, n))
        .collect();
    let width = |shown: &[(Chip, usize, usize)]| {
        shown.iter().map(|&(_, _, n)| d.width(n)).sum::<usize>() + shown.len().saturating_sub(1) * d.gap
    };
    let room = right.saturating_sub(left);
    while width(&shown) > room {
        // Cut the longest text by one character; when none can be cut further, drop the last chip.
        match shown.iter_mut().filter(|t| t.2 > MIN_CHARS).max_by_key(|t| t.2) {
            Some(t) => t.2 -= 1,
            None => {
                shown.pop();
            }
        }
        if shown.is_empty() {
            break;
        }
    }
    let mut x = left;
    shown
        .into_iter()
        .map(|(chip, _, chars)| {
            let w = d.width(chars);
            let slot = Slot { chip, x, w, chars };
            x += w + d.gap;
            slot
        })
        .collect()
}

/// Draws the chips on grid row `row`, which has just been repainted (blank). A chip is `SCALE` of the
/// row's height, centred in it, so it never touches the lines above and below. `scratch` is the glyph
/// coverage buffer the text is rasterized through.
pub fn paint(soft: &Soft, fb: &mut [u32], ctx: &Context, chips: &[Chip], row: usize, run: Option<&Run>, scratch: &mut Vec<u8>) {
    let (_, ch) = soft.cell();
    let (ww, _) = soft.window();
    let pad = soft.pad as usize;
    let wy = soft.top as usize + row * ch as usize; // the row's top, in window pixels
    paint_line(soft, fb, ctx, chips, run, (wy, ch as usize), (pad, ww.saturating_sub(pad)), (1.0, 1.0), scratch);
}

/// Draws the chips on a line `ch` pixels tall whose top is window pixel row `wy`, between the columns
/// `left` and `right`, for a font `scale` times the grid's; `fade` multiplies the shapes' opacity. `run`,
/// how the prompt's command ended, is drawn at the right end (a check or a cross, the status when it is
/// not 0, and the time); the chips get what is left.
#[allow(clippy::too_many_arguments)]
pub fn paint_line(soft: &Soft, fb: &mut [u32], ctx: &Context, chips: &[Chip], run: Option<&Run>, (wy, ch): (usize, usize), (left, right): (usize, usize), (scale, fade): (f32, f32), scratch: &mut Vec<u8>) {
    let (w, h) = soft.fb_size();
    let (ox, oy) = soft.origin();
    if fb.len() < w * h || ch < 8 {
        return;
    }
    let small = soft.small(scale * SCALE);
    let d = Dims::new(small.px, small.cell_w as usize);
    let chip_h = ((ch as f32 * SCALE).round() as usize).clamp(6, ch);
    let cy = wy as f32 + ch as f32 / 2.0;
    let mut right = right;
    if let Some(run) = run {
        right = paint_run(soft, fb, run, (cy, right), (scale, fade), scratch).saturating_sub(d.gap);
    }
    for slot in layout(ctx, chips, &d, left, right) {
        let color = color(slot.chip);
        let mut p = Paint { fb: &mut *fb, w, h, ox: ox as i32, oy: oy as i32, fade };
        let (x, y, cwid, chei) = (slot.x as i32, (wy + (ch - chip_h) / 2) as i32, slot.w as i32, chip_h as i32);
        let r = (8.0 * d.k).min(chip_h as f32 / 2.0);
        p.rrect(x, y, cwid, chei, r, color, 0.10);
        p.outline(x, y, cwid, chei, r, color, 0.58);
        icon(&mut p, slot.chip, (slot.x + d.pad) as f32 + d.icon as f32 / 2.0, cy, d.k, color);

        // The text: one cell per character, in the chip's colour.
        let fg = pack((color[0] * 255.0) as u8, (color[1] * 255.0) as u8, (color[2] * 255.0) as u8, 255);
        let text = ctx.get(slot.chip);
        let cut = text.chars().count() > slot.chars;
        let chars = text.chars().take(slot.chars).enumerate().map(|(i, c)| if cut && i + 1 == slot.chars { '\u{2026}' } else { c });
        if fade > 0.5 {
            soft.paint_text(fb, ox + slot.x + d.pad + d.icon + d.icon_gap, oy + cy.round() as usize, chars, fg, false, &small, scratch);
        }
    }
}

/// The status at the right end of a prompt's line: a check or a cross, the exit code when it is not 0, and
/// how long the command ran, ending at window column `right` and centred on window row `cy`. Returns
/// where it starts.
fn paint_run(soft: &Soft, fb: &mut [u32], run: &Run, (cy, right): (f32, usize), (scale, fade): (f32, f32), scratch: &mut Vec<u8>) -> usize {
    let (w, h) = soft.fb_size();
    let (ox, oy) = soft.origin();
    let m = soft.small(scale * 0.8);
    let cw = m.cell_w as usize;
    let ok = run.status == 0;
    let (code, time) = (if ok { String::new() } else { run.status.to_string() }, history::duration_text(run.ms));
    let size = m.px.round().max(8.0) as usize; // the mark is 1 em
    let gap = (m.px * 0.3).round() as usize;
    let sep = (m.px * 0.6).round() as usize;
    let code_w = if ok { 0 } else { gap + code.chars().count() * cw };
    let total = size + code_w + sep + time.chars().count() * cw;
    let x0 = right.saturating_sub(total);
    let hue = palette::unit(if ok { palette::GREEN } else { palette::RED });
    {
        let mut p = Paint { fb: &mut *fb, w, h, ox: ox as i32, oy: oy as i32, fade };
        let (mx, my, k) = (x0 as f32 + size as f32 / 2.0, cy, size as f32 / 16.0);
        let t = 1.8 * k;
        p.shape(x0 as i32 - 1, (cy - size as f32) as i32, (x0 + size) as i32 + 2, (cy + size as f32) as i32 + 1, hue, 1.0, &|px, py| {
            let (gx, gy) = ((px - mx) / k + 8.0, (py - my) / k + 8.0);
            let d = if ok {
                dist_poly(gx, gy, &[(3.5, 8.5), (6.5, 11.5), (12.5, 4.5)], false)
            } else {
                dist_poly(gx, gy, &[(4.0, 4.0), (12.0, 12.0)], false).min(dist_poly(gx, gy, &[(12.0, 4.0), (4.0, 12.0)], false))
            };
            (t / 2.0 + 0.5 - d * k).clamp(0.0, 1.0)
        });
    }
    if fade > 0.5 {
        let hue8 = |c: Rgb| pack(c.r, c.g, c.b, 255);
        let cyi = oy + cy.round() as usize;
        let mut x = ox + x0 + size;
        if !ok {
            soft.paint_text(fb, x + gap, cyi, code.chars(), hue8(if ok { palette::GREEN } else { palette::RED }), false, &m, scratch);
            x += code_w;
        }
        soft.paint_text(fb, x + sep, cyi, time.chars(), hue8(palette::FAINT), false, &m, scratch);
    }
    x0
}

/// The settings screen's live preview in the box `(x, y, w, h)` (window pixels): the chips, when they are
/// on, over a prompt line, in the font, weight and spacing being edited. The font is scaled down when
/// its lines would not fit the box.
#[allow(clippy::too_many_arguments)]
pub fn paint_preview(soft: &Soft, fb: &mut [u32], (x, y, bw, bh): (i32, i32, i32, i32), input: &crate::config::InputCfg, ctx: &Context, fade: f32) {
    let (w, h) = soft.fb_size();
    let (ox, oy) = soft.origin();
    if fb.len() < w * h {
        return;
    }
    let lines = if input.inline { 2 } else { 1 };
    let cell_h = soft.cell().1 as f32;
    let scale = ((bh as f32 - 20.0) / lines as f32 / cell_h).clamp(0.3, 1.0);
    let m = soft.small(scale);
    let ch = m.cell_h as usize;
    {
        let mut p = Paint { fb: &mut *fb, w, h, ox: ox as i32, oy: oy as i32, fade };
        p.rrect(x, y, bw, bh, 10.0, [0.0; 3], 0.40);
        p.outline(x, y, bw, bh, 10.0, [1.0; 3], 0.07);
    }
    let content = lines * ch + if input.inline { 4 } else { 0 };
    let top = y as usize + (bh as usize).saturating_sub(content) / 2;
    let (left, right) = (x as usize + 12, (x + bw - 12) as usize);
    let mut scratch = Vec::new();
    if input.inline {
        paint_line(soft, fb, ctx, &input.chips, None, (top, ch), (left, right), (scale, fade), &mut scratch);
    }
    if fade <= 0.5 {
        return;
    }
    let cy = oy + top + if input.inline { ch + 4 } else { 0 } + ch / 2;
    let fit = (right - left) / m.cell_w as usize;
    let pack = |c: alacritty_terminal::vte::ansi::Rgb| pack(c.r, c.g, c.b, 255);
    let mut x = ox + left;
    let mut put = |s: &str, c: u32, bold: bool, x: &mut usize, used: &mut usize| {
        let n = s.chars().count().min(fit.saturating_sub(*used));
        soft.paint_text(fb, *x, cy, s.chars().take(n), c, bold, &m, &mut scratch);
        *x += n * m.cell_w as usize;
        *used += n;
    };
    let mut used = 0;
    if input.inline {
        put("$", pack(palette::ACCENT), true, &mut x, &mut used);
        put(&format!(" {}", tr(Str::PreviewCommand)), pack(palette::FG), false, &mut x, &mut used);
    } else {
        put(ctx.get(Chip::Host), pack(palette::GREEN), true, &mut x, &mut used);
        put(":", pack(palette::FG), false, &mut x, &mut used);
        put(ctx.get(Chip::Dir), pack(palette::BLUE), true, &mut x, &mut used);
        put(&format!("$ {}", tr(Str::PreviewCommand)), pack(palette::FG), false, &mut x, &mut used);
    }
}

/// A prompt's line as it was drawn: its row, which prompt and the revision of its record.
#[derive(Clone, Copy, PartialEq)]
struct Drawn {
    row: usize,
    n: u32,
    rev: u32,
}

/// The rows the chips were drawn on in the last frame.
#[derive(Default)]
pub struct Chips {
    last: Vec<Drawn>,
    now: Vec<Drawn>,
    rows: Vec<(usize, u32)>,
    scratch: Vec<u8>,
}

impl Chips {
    /// One frame: the grid, and the chips over the reserved line of every prompt on screen when `on`,
    /// each from its own record (`history`): what it said when it was typed at and how its command ended.
    /// `changed`: the settings changed since the last frame.
    #[allow(clippy::too_many_arguments)]
    pub fn frame<L: EventListener>(
        &mut self,
        on: bool,
        changed: bool,
        history: &History,
        chips: &[Chip],
        term: &mut Term<L>,
        view: &mut View,
        soft: &mut Soft,
        fb: &mut [u32],
        (w, h): (usize, usize),
        focused: bool,
    ) {
        self.now.clear();
        if on {
            view::prompt_rows(term, &mut self.rows);
            // A shell whose hook predates the numbered prompts marks only the one being typed at.
            if self.rows.is_empty() {
                if let (Some(row), Some(latest)) = (view::prompt_row(term), history.latest()) {
                    self.rows.push((row, latest.n));
                }
            }
            for &(row, n) in &self.rows {
                self.now.push(Drawn { row, n, rev: history.get(n).map_or(0, |r| r.rev) });
            }
        }
        // Whatever was drawn over a row must go when it changes or leaves; where chips arrive or change,
        // the row is rebuilt so they are drawn on fresh pixels.
        for d in &self.last {
            if changed || !self.now.contains(d) {
                view.invalidate_row(d.row);
            }
        }
        for d in &self.now {
            if changed || !self.last.contains(d) {
                view.invalidate_row(d.row);
            }
        }
        std::mem::swap(&mut self.last, &mut self.now);
        view.build(term, focused, &mut Target { soft: &mut *soft, fb: &mut *fb, w, h });
        for d in &self.last {
            if let (true, Some(rec)) = (soft.row_repainted(d.row), history.get(d.n)) {
                paint(soft, fb, &rec.ctx, chips, d.row, rec.run.as_ref(), &mut self.scratch);
            }
        }
    }
}

/// The chip's icon: a 14 px design scaled by `k`, centred on `(cx, cy)`.
pub fn icon(p: &mut Paint, chip: Chip, cx: f32, cy: f32, k: f32, c: [f32; 3]) {
    let r = 9.0 * k;
    let (x0, y0, x1, y1) = ((cx - r).floor() as i32, (cy - r).floor() as i32, (cx + r).ceil() as i32 + 1, (cy + r).ceil() as i32 + 1);
    p.shape(x0, y0, x1, y1, c, 0.95, &|px, py| icon_coverage(chip, cx + (px - cx) / k, cy + (py - cy) / k, cx, cy));
}

/// Distance from `(px, py)` to the polyline through `pts` (closed: back to the first point).
fn dist_poly(px: f32, py: f32, pts: &[(f32, f32)], closed: bool) -> f32 {
    let n = pts.len();
    let segs = if closed { n } else { n - 1 };
    (0..segs).map(|i| seg(px, py, pts[i], pts[(i + 1) % n])).fold(f32::MAX, f32::min)
}

/// Scale from the 16-unit grid the icons are drawn on (the reference's SVGs) to the 14 px design.
const GRID: f32 = 14.0 / 16.0;

/// Coverage of the icon at `(px, py)` in the 14 px design around `(cx, cy)`: the outline icons of the
/// `terminal.html` reference, drawn from their 16-unit paths with a 1.5-unit round stroke.
fn icon_coverage(chip: Chip, px: f32, py: f32, cx: f32, cy: f32) -> f32 {
    // Back to the 16-unit grid, centred on (8, 8).
    let (x, y) = ((px - cx) / GRID + 8.0, (py - cy) / GRID + 8.0);
    let t = 1.5;
    let stroke = |d: f32| ((t / 2.0 + 0.5 / GRID - d.abs()) * GRID).clamp(0.0, 1.0);
    let poly = |pts: &[(f32, f32)], closed: bool| stroke(dist_poly(x, y, pts, closed));
    let circle = |ox: f32, oy: f32, r: f32| stroke((x - ox).hypot(y - oy) - r);
    match chip {
        Chip::Time => circle(8.0, 8.0, 5.8).max(poly(&[(8.0, 4.8), (8.0, 8.0), (10.2, 9.4)], false)),
        Chip::User => {
            // The shoulders: the upper half of an ellipse 10.4 wide and 8 tall resting on y = 13.5.
            let (u, v) = ((x - 8.0) / 5.2, (y - 13.5) / 4.0);
            let shoulders = if y <= 13.5 { stroke(((u * u + v * v).sqrt() - 1.0) * 4.2) } else { 0.0 };
            circle(8.0, 5.5, 2.6).max(shoulders)
        }
        Chip::Host => {
            let d = crate::chrome::rounded_rect(x - 2.0, y - 3.0, 12.0, 8.0, 1.4);
            stroke(d).max(poly(&[(6.0, 13.5), (10.0, 13.5)], false)).max(poly(&[(8.0, 11.0), (8.0, 13.5)], false))
        }
        Chip::Env => poly(&[(8.0, 2.0), (13.0, 5.0), (13.0, 11.0), (8.0, 14.0), (3.0, 11.0), (3.0, 5.0)], true),
        Chip::Pyenv => poly(&[(5.5, 5.0), (3.0, 8.0), (5.5, 11.0)], false).max(poly(&[(10.5, 5.0), (13.0, 8.0), (10.5, 11.0)], false)),
        Chip::Dir => poly(
            &[(2.0, 4.5), (3.4, 3.1), (6.0, 3.1), (7.4, 4.7), (11.6, 4.7), (13.0, 6.1), (13.0, 11.5), (11.6, 12.9), (3.4, 12.9), (2.0, 11.5)],
            true,
        ),
        Chip::Branch => circle(4.5, 3.5, 1.6)
            .max(circle(4.5, 12.5, 1.6))
            .max(circle(11.5, 5.5, 1.6))
            .max(poly(&[(4.5, 5.1), (4.5, 10.9)], false))
            .max(poly(&[(11.5, 7.1), (11.0, 8.8), (9.6, 9.9), (7.6, 10.5), (5.3, 11.1)], false)),
        Chip::Changes => poly(&[(3.0, 13.0), (3.7, 10.0), (10.5, 3.2), (11.2, 2.9), (12.5, 3.5), (12.8, 4.6), (12.5, 5.2), (5.7, 12.0)], true),
        Chip::Kube => circle(8.0, 8.0, 5.6)
            .max(circle(8.0, 8.0, 1.4))
            .max(poly(&[(8.0, 2.4), (8.0, 5.6)], false))
            .max(poly(&[(8.0, 10.4), (8.0, 13.6)], false))
            .max(poly(&[(2.4, 8.0), (5.6, 8.0)], false))
            .max(poly(&[(10.4, 8.0), (13.6, 8.0)], false)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The 14 px design: icon 14, padding 9, icon gap 6, chip gap 8, 9 px characters.
    const D: Dims = Dims { cw: 9, k: 1.0, pad: 9, icon: 14, icon_gap: 6, gap: 8 };

    fn ctx(texts: &[(Chip, &str)]) -> Context {
        let mut c = Context::default();
        for (chip, t) in texts {
            c.text[*chip as usize] = t.to_string();
        }
        c
    }

    #[test]
    fn chips_without_text_are_skipped_and_the_order_is_the_lists() {
        let c = ctx(&[(Chip::Time, "03:48 pm"), (Chip::Dir, "~/Desktop"), (Chip::Branch, "main")]);
        let slots = layout(&c, &[Chip::Dir, Chip::Kube, Chip::Time, Chip::Branch], &D, 20, 2000);
        assert_eq!(slots.iter().map(|s| s.chip).collect::<Vec<_>>(), [Chip::Dir, Chip::Time, Chip::Branch]);
        assert_eq!(slots[0].x, 20);
        // Each chip starts a gap after the previous one ends; its width is padding, icon, gap and text.
        assert_eq!(slots[0].w, 2 * 9 + 14 + 6 + 9 * 9);
        assert_eq!(slots[1].x, slots[0].x + slots[0].w + 8);
    }

    #[test]
    fn a_crowded_bar_cuts_the_longest_texts_before_dropping_chips() {
        let long = "x".repeat(30);
        let c = ctx(&[(Chip::Time, "03:48 pm"), (Chip::Dir, &long), (Chip::Branch, "main")]);
        let wide = layout(&c, &Chip::ALL, &D, 20, 2000);
        assert!(wide.iter().all(|s| s.chars == if s.chip == Chip::Dir { 30 } else { c.get(s.chip).chars().count() }));
        let end = |v: &[Slot]| v.last().map(|s| s.x + s.w).unwrap();
        // Less room than the full text: only the long one is cut, and the result fits.
        let tight = layout(&c, &Chip::ALL, &D, 20, end(&wide) - 40);
        assert_eq!(tight.len(), 3);
        assert!(end(&tight) <= end(&wide) - 40);
        assert_eq!(tight.iter().find(|s| s.chip == Chip::Dir).unwrap().chars, 30 - 5);
        assert_eq!(tight.iter().find(|s| s.chip == Chip::Branch).unwrap().chars, 4, "short texts are never cut");
    }

    #[test]
    fn when_cutting_is_not_enough_the_last_chips_are_dropped() {
        let c = ctx(&[(Chip::Time, "03:48 pm"), (Chip::Dir, "~/Desktop"), (Chip::Branch, "main")]);
        let all = layout(&c, &Chip::ALL, &D, 20, 2000);
        let two = layout(&c, &Chip::ALL, &D, 20, all[1].x + all[1].w);
        assert_eq!(two.iter().map(|s| s.chip).collect::<Vec<_>>(), [Chip::Time, Chip::Dir]);
        assert!(layout(&c, &Chip::ALL, &D, 20, 30).is_empty());
        assert!(layout(&Context::default(), &Chip::ALL, &D, 20, 2000).is_empty());
    }

    #[test]
    fn every_icon_draws_something_inside_its_box_and_nothing_outside() {
        for chip in Chip::ALL {
            let (w, h) = (40usize, 40usize);
            let mut fb = vec![0u32; w * h];
            icon(&mut Paint { fb: &mut fb, w, h, ox: 0, oy: 0, fade: 1.0 }, chip, 20.0, 20.0, 1.0, [1.0, 1.0, 1.0]);
            let lit: Vec<usize> = (0..w * h).filter(|&i| fb[i] >> 24 != 0).collect();
            assert!(lit.len() > 12, "{chip:?} drew {} pixels", lit.len());
            for i in lit {
                let (x, y) = ((i % w) as i32, (i / w) as i32);
                assert!((12..=28).contains(&x) && (12..=28).contains(&y), "{chip:?} at {x},{y}");
            }
        }
    }

    #[test]
    fn chips_are_a_tenth_smaller_than_the_font_they_follow() {
        // 14 px text gives 12.6 px chips; everything in them follows, icon and spacing included.
        let big = Dims::new(14.0, 9);
        assert_eq!(big, D);
        let small = Dims::new(14.0 * SCALE, 8);
        assert_eq!((small.icon, small.pad, small.icon_gap, small.gap), (13, 8, 5, 7));
        // A bigger font gives bigger chips, in proportion.
        let large = Dims::new(20.0 * SCALE, 10);
        assert!(large.icon > small.icon && large.pad > small.pad);
        // The width of a chip is made of its parts.
        assert_eq!(small.width(4), 2 * 8 + 13 + 5 + 4 * 8);
    }

    #[test]
    fn an_icon_shrinks_with_the_scale_and_stays_in_its_box() {
        for chip in Chip::ALL {
            let ink = |k: f32| {
                let (w, h) = (40usize, 40usize);
                let mut fb = vec![0u32; w * h];
                icon(&mut Paint { fb: &mut fb, w, h, ox: 0, oy: 0, fade: 1.0 }, chip, 20.0, 20.0, k, [1.0, 1.0, 1.0]);
                let lit: Vec<usize> = (0..w * h).filter(|&i| fb[i] >> 24 != 0).collect();
                let (lo, hi) = lit.iter().fold((99i32, 0i32), |(lo, hi), &i| (lo.min((i % w) as i32), hi.max((i % w) as i32)));
                (lit.len(), hi - lo)
            };
            let (n1, w1) = ink(1.0);
            let (n8, w8) = ink(0.9);
            assert!(n8 < n1 && n8 > 8, "{chip:?}: {n8} pixels at 0.9, {n1} at 1.0");
            assert!(w8 <= w1, "{chip:?} is not narrower: {w8} vs {w1}");
        }
    }
}
