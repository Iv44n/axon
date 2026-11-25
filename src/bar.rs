//! The prompt chips: the time, the user, the directory, the git branch... as quiet coloured text with an icon
//! (the reference's composer: no box, 80 % opaque) on the line right above the prompt the user is typing at,
//! drawn with the grid's own font at 84 % of its size and rasterized on the fly (no tiles of their own).
//! The prompts of commands that have run keep no chips: their reserved line is the space between two
//! commands, and how a command ended (a check or a cross, the status, the time) is shown at the right end of
//! its line when it failed or the pointer is over it, as the reference's blocks do.
//!
//! The line is one the shell reserved (`view::live_prompt`), the first of the composer (`view::Fold`); the chips are
//! drawn over its blank cells. While nothing is typed at that prompt, the composer says what it is for (the
//! reference's placeholder, in the faint colour after the cursor).
//! A framebuffer that is repainted row by row must still equal a full repaint, so `Chips::frame`
//! repaints the row something appears on, moves from or changes on, and draws again only on rows that
//! were repainted: never twice over the same pixels.
//! Coordinates are window pixels, the origin the window's top-left corner (not the shadow's).
use alacritty_terminal::{Term, event::EventListener, grid::Dimensions, vte::ansi::Rgb};

use crate::{
    cell::{Cell, Glyphs, pack},
    i18n::{Str, tr},
    palette,
    config::Chip,
    context::Context,
    history::{self, History, Run},
    paint::{Paint, seg},
    soft::{Soft, Target},
    view::{self, View},
};

/// Chips are drawn at this fraction of the font size set in the settings (`.pill { font-size: .84em }`).
pub const SCALE: f32 = 0.84;

/// How opaque a chip is (`.c-box .meta .pill { opacity: .8 }`).
const ALPHA: f32 = 0.8;

/// The size, in pixels, of the parts of a chip, for chip text of a given size: the icon is 1.05 em of it, 0.45 em
/// from the text, and the chips are 1.1 em of the grid's font apart.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dims {
    /// Width of a character cell of the chip text.
    pub cw: usize,
    /// Scale of the icon from its 14 px design.
    pub k: f32,
    pub icon: usize,
    /// Space between the icon and the text.
    pub icon_gap: usize,
    /// Space between chips.
    pub gap: usize,
}

impl Dims {
    /// For chip text of `px` pixels whose character cell is `cw` wide.
    pub fn new(px: f32, cw: usize) -> Dims {
        let at = |v: f32| v.round().max(1.0) as usize;
        let icon = 1.05 * px;
        Dims { cw, k: icon / 14.0, icon: at(icon), icon_gap: at(0.45 * px), gap: at(1.1 * px / SCALE) }
    }

    /// A chip with `chars` characters of text.
    fn width(&self, chars: usize) -> usize {
        self.icon + self.icon_gap + chars * self.cw
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

/// Draws the chips on grid row `row` of the pane being built, which has just been repainted (blank), between
/// where its text starts and where it may reach. `scratch` is the glyph coverage buffer the text is rasterized
/// through. An unfocused pane's chips are as dim as its text.
pub fn paint(soft: &Soft, fb: &mut [u32], ctx: &Context, chips: &[Chip], row: usize, scratch: &mut Vec<u8>) {
    let (_, ch) = soft.cell();
    let (left, _, right) = soft.content_box();
    paint_line(soft, fb, ctx, chips, (soft.row_top(row), ch as usize), (left, right), soft.dim_alpha(), scratch);
}

/// The placeholder after the cursor at column `col` of row `row` (`.ph`: "Type a command", faint), in the grid's
/// font, as far as the row reaches. `cells` is scratch.
fn paint_hint(soft: &mut Soft, fb: &mut [u32], (row, col): (usize, usize), cols: usize, cells: &mut Vec<Cell>) {
    let faint = pack(palette::FAINT.r, palette::FAINT.g, palette::FAINT.b, 255);
    cells.clear();
    for c in tr(Str::TypeACommand).chars().take(cols.saturating_sub(col + 1)) {
        let tf = soft.glyph(c, false, false);
        cells.push(Cell { tf, fg: faint, bg: 0 });
    }
    let (cw, _) = soft.cell();
    let (left, _, _) = soft.content_box();
    let (ox, oy) = soft.origin();
    soft.paint_cells_at(fb, ox + left + (col + 1) * cw as usize, oy + soft.row_top(row), cells);
}

/// Draws the chips on a line `ch` pixels tall whose top is window pixel row `wy`, between the columns `left` and
/// `right`; `fade` multiplies their opacity.
#[allow(clippy::too_many_arguments)]
pub fn paint_line(soft: &Soft, fb: &mut [u32], ctx: &Context, chips: &[Chip], (wy, ch): (usize, usize), (left, right): (usize, usize), fade: f32, scratch: &mut Vec<u8>) {
    let (w, h) = soft.fb_size();
    let (ox, oy) = soft.origin();
    if fb.len() < w * h || ch < 8 {
        return;
    }
    let small = soft.small(SCALE);
    let d = Dims::new(small.px, small.cell_w as usize);
    let cy = wy as f32 + ch as f32 / 2.0;
    for slot in layout(ctx, chips, &d, left, right) {
        let color = color(slot.chip);
        let mut p = Paint { fb: &mut *fb, w, h, ox: ox as i32, oy: oy as i32, fade: fade * ALPHA };
        icon(&mut p, slot.chip, slot.x as f32 + d.icon as f32 / 2.0, cy, d.k, color);

        // The text: one cell per character, in the chip's colour as seen through its opacity.
        let fg = pack((color[0] * 255.0) as u8, (color[1] * 255.0) as u8, (color[2] * 255.0) as u8, (255.0 * ALPHA * fade) as u8);
        let text = ctx.get(slot.chip);
        let cut = text.chars().count() > slot.chars;
        let chars = text.chars().take(slot.chars).enumerate().map(|(i, c)| if cut && i + 1 == slot.chars { '\u{2026}' } else { c });
        soft.paint_text(fb, ox + slot.x + d.icon + d.icon_gap, oy + cy.round() as usize, chars, fg, false, &small, scratch);
    }
}

/// How wide the status of `run` is (`paint_run`), in pixels, for the grid's font.
fn run_width(soft: &Soft, run: &Run) -> usize {
    let m = soft.small(RUN_SCALE);
    let cw = m.cell_w as usize;
    let (size, gap, sep) = run_parts(m.px);
    let code_w = if run.status == 0 { 0 } else { gap + digits(run.status) * cw };
    size + code_w + sep + history::duration_text(run.ms).chars().count() * cw
}

/// The status is drawn at this fraction of the grid's font (`.b-info { font-size: .78em }`).
const RUN_SCALE: f32 = 0.78;

/// The parts of a status at `px`: the mark (1 em), the gap before the status code (0.3 em) and before the time (0.6 em).
fn run_parts(px: f32) -> (usize, usize, usize) {
    (px.round().max(8.0) as usize, (px * 0.3).round() as usize, (px * 0.6).round() as usize)
}

/// How many characters `n` is written with, its sign included.
fn digits(n: i32) -> usize {
    n.unsigned_abs().checked_ilog10().map_or(1, |l| l as usize + 1) + (n < 0) as usize
}

/// The status at the right end of a command's line: a check or a cross, the exit code when it is not 0, and how
/// long the command ran, ending at window column `right` and centred on window row `cy`.
fn paint_run(soft: &Soft, fb: &mut [u32], run: &Run, (cy, right): (f32, usize), fade: f32, scratch: &mut Vec<u8>) {
    let (w, h) = soft.fb_size();
    let (ox, oy) = soft.origin();
    let m = soft.small(RUN_SCALE);
    let cw = m.cell_w as usize;
    let ok = run.status == 0;
    let (code, time) = (if ok { String::new() } else { run.status.to_string() }, history::duration_text(run.ms));
    let (size, gap, sep) = run_parts(m.px);
    let code_w = if ok { 0 } else { gap + code.chars().count() * cw };
    let x0 = right.saturating_sub(run_width(soft, run));
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
    let hue8 = |c: Rgb| pack(c.r, c.g, c.b, (255.0 * fade) as u8);
    let cyi = oy + cy.round() as usize;
    let mut x = ox + x0 + size;
    if !ok {
        soft.paint_text(fb, x + gap, cyi, code.chars(), hue8(palette::RED), false, &m, scratch);
        x += code_w;
    }
    soft.paint_text(fb, x + sep, cyi, time.chars(), hue8(palette::FAINT), false, &m, scratch);
}

/// What is drawn on a prompt's lines.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Show {
    /// The chips, on its reserved line: the prompt being typed at.
    Chips,
    /// How its command ended, at the right end of the command's line (the row after the reserved one).
    Run,
    /// The placeholder, on the row of the prompt being typed at while nothing is typed (`row` is that row, `n` the
    /// cursor's column).
    Hint,
}

/// A prompt as it was drawn: its reserved row, which prompt, the revision of its record, and what was drawn.
#[derive(Clone, Copy, PartialEq)]
struct Drawn {
    row: usize,
    n: u32,
    rev: u32,
    show: Show,
}

impl Drawn {
    /// The row it is drawn on.
    fn line(&self) -> usize {
        match self.show {
            Show::Chips | Show::Hint => self.row,
            Show::Run => self.row + 1,
        }
    }
}

/// What was drawn over the rows in the last frame.
#[derive(Default)]
pub struct Chips {
    last: Vec<Drawn>,
    now: Vec<Drawn>,
    rows: Vec<(usize, u32)>,
    scratch: Vec<u8>,
    /// The placeholder's cells, built when it is drawn.
    cells: Vec<Cell>,
}

impl Chips {
    /// The reserved row of the prompt whose block row `y` is in (a block runs from a prompt to the next), as the
    /// last frame found the prompts; none above the first. The pointer moving within a block changes nothing drawn.
    pub fn block_of(&self, y: usize) -> Option<usize> {
        self.rows.iter().rev().find(|&&(row, _)| row <= y).map(|&(row, _)| row)
    }

    /// One frame: the grid, with the composer apart (`view::Fold`), and over it, when `on`, the chips of the prompt
    /// being typed at (with the placeholder while nothing is typed) and the status of each command on screen that
    /// failed or that the pointer is over (`hover`: the row under it). Each prompt finds its record in `history`.
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
        hover: Option<usize>,
    ) {
        self.now.clear();
        let fold = view::fold(term, true);
        if on {
            view::prompt_rows(term, &fold, &mut self.rows);
            // A shell whose hook predates the numbered prompts marks only the one being typed at.
            if self.rows.is_empty()
                && let (Some(row), Some(latest)) = (view::live_prompt(term).and_then(|l| fold.row(l)), history.latest())
            {
                self.rows.push((row, latest.n));
            }
            let live = history.latest().filter(|_| !history.running()).map(|r| r.n);
            let hovered = hover.and_then(|y| self.block_of(y));
            for &(row, n) in &self.rows {
                let Some(rec) = history.get(n) else { continue };
                // A command's line is above the composer: the status never lands in it.
                let show = if Some(n) == live {
                    Show::Chips
                } else if rec.run.is_some_and(|r| (r.status != 0 && r.status != 130) || hovered == Some(row)) && row + 1 < fold.split {
                    Show::Run
                } else {
                    continue;
                };
                self.now.push(Drawn { row, n, rev: rec.rev, show });
            }
            let cursor = term.grid().cursor.point;
            if fold.chips && view::input_empty(term)
                && let Some(row) = fold.row(cursor.line.0)
            {
                self.now.push(Drawn { row, n: cursor.column.0 as u32, rev: 0, show: Show::Hint });
            }
        }
        // Whatever was drawn over a row must go when it changes or leaves; where something arrives or changes,
        // the row is rebuilt so it is drawn on fresh pixels.
        for d in &self.last {
            if changed || !self.now.contains(d) {
                view.invalidate_row(d.line());
            }
        }
        for d in &self.now {
            if changed || !self.last.contains(d) {
                view.invalidate_row(d.line());
            }
        }
        std::mem::swap(&mut self.last, &mut self.now);
        view.build_folded(term, focused, fold, &mut Target { soft: &mut *soft, fb: &mut *fb, w, h });
        let (ch, cw) = (soft.cell().1 as usize, soft.cell().0 as usize);
        let (left, _, right) = soft.content_box();
        for d in &self.last {
            if !soft.row_repainted(d.line()) {
                continue;
            }
            if d.show == Show::Hint {
                paint_hint(soft, fb, (d.row, d.n as usize), term.columns(), &mut self.cells);
                continue;
            }
            let Some(rec) = history.get(d.n) else { continue };
            match (d.show, rec.run) {
                (Show::Chips, _) => paint(soft, fb, &rec.ctx, chips, d.row, &mut self.scratch),
                (Show::Run, Some(run)) => {
                    // At the right end of the command's line, if its text leaves room there (a long command keeps it).
                    let used = left + cw * fold.line(d.line()).map_or(0, |l| used_cols(term, l));
                    let gap = cw * 2;
                    if right >= used + gap + run_width(soft, &run) {
                        let cy = soft.row_top(d.line()) as f32 + ch as f32 / 2.0;
                        paint_run(soft, fb, &run, (cy, right), soft.dim_alpha(), &mut self.scratch);
                    }
                }
                _ => {}
            }
        }
    }
}

/// How many columns of line `line` hold text: up to the last cell that is not blank.
fn used_cols<L: EventListener>(term: &Term<L>, line: i32) -> usize {
    use alacritty_terminal::index::{Column, Line};
    let grid = term.grid();
    let line = Line(line);
    let cols = term.columns();
    (0..cols).rev().find(|&c| !matches!(grid[line][Column(c)].c, ' ' | '\0')).map_or(0, |c| c + 1)
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

    /// Icon 14, icon gap 6, chip gap 8, 9 px characters.
    const D: Dims = Dims { cw: 9, k: 1.0, icon: 14, icon_gap: 6, gap: 8 };

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
        // Each chip starts a gap after the previous one ends; its width is icon, gap and text (no box, no padding).
        assert_eq!(slots[0].w, 14 + 6 + 9 * 9);
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
    fn chips_follow_the_reference_s_proportions() {
        // 14 px text gives 11.76 px chips (`.84em`): a 12 px icon (1.05 em) 5 px from the text (.45 em), and 15 px
        // between chips (1.1 em of the grid's font).
        let d = Dims::new(14.0 * SCALE, 7);
        assert_eq!((d.icon, d.icon_gap, d.gap), (12, 5, 15));
        assert!((d.k * 14.0 - 1.05 * 14.0 * SCALE).abs() < 1e-3, "the icon is drawn at its own size");
        // A bigger font gives bigger chips, in proportion.
        let large = Dims::new(20.0 * SCALE, 10);
        assert!(large.icon > d.icon && large.gap > d.gap && large.icon_gap >= d.icon_gap);
        // The width of a chip is made of its parts.
        assert_eq!(d.width(4), 12 + 5 + 4 * 7);
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
