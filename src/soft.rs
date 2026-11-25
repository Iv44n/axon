//! CPU renderer ([0011](../knowledge/decisions/core/0011-software-render-wl-shm.md)): paints the cell
//! grid straight into a premultiplied `Argb8888` framebuffer, the memory a `wl_shm` buffer maps.
//!
//! The framebuffer is the retained state: only damaged rows are repainted into it, so there is no
//! cell buffer and no copy. Cells, text coverage and overlays match shader.wgsl; the frame around
//! them (glass body, rounded corners, title bar and buttons) is `chrome.rs` and exists only here.
//! Driven by `wlapp.rs` (`AXON_BACKEND=wl`); see `knowledge/plan/software-renderer.md`.

use crate::cell::{
    Cell, Glyphs, MAX_TILES, T_BAR, T_BEAM, T_BOX, T_FIRST_GLYPH, T_STRIKE, T_UNDER, pack,
};
use crate::chrome::{self, REACH, STATUS_H, TITLE_FROM_RIGHT, TITLE_H, TITLE_X0, Ui};
use alacritty_terminal::vte::ansi::Rgb;
use crate::{bar, config::Chip, i18n::{Str, tr}, palette, paint::{Paint, WHITE, seg}, panes::{self, Rect}};
use crate::config::FontCfg;
use crate::font::{Fonts, GlyphTable, UiFont, special_tiles};
use crate::view::{Backend, Fold};

/// Where one pane of the window is and how its frame looks: its box in window pixels (the outer edge of
/// its border), whether it has the focus, and whether it has a frame at all (a window with one pane has none).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slot {
    pub rect: Rect,
    pub focus: bool,
    pub framed: bool,
}

/// The space between the text and the edges of its pane: beside it, above it and below it.
pub const PAD_X: u32 = 22;
const PAD_TOP: u32 = 16;
const PAD_BOTTOM: u32 = 14;

/// What a pane keeps between its lines and the composer (`view::Fold`, ADR 0037): the reference's `.screen` bottom
/// padding (10 px), the composer's `border-top` (a hairline of white at 5 %) and its top padding (11 px).
const COMPOSER_GAP: i32 = 22;
const COMPOSER_LINE_Y: i32 = 10;
const COMPOSER_LINE_A: f32 = 0.05;
/// Between the chips and the prompt below them (`.c-box .meta { margin-bottom: .45em }`), in ems of the grid's font.
const CHIPS_GAP_EM: f32 = 0.45;

/// The alpha an unfocused pane's screen is drawn at: the reference's `opacity: .72`.
pub const UNFOCUSED: u32 = 184;

/// A panel beside the grid: how wide it is and whether it sits at the window's left edge (else the right one).
/// Zero wide is no panel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Dock {
    pub w: u32,
    pub left: bool,
}

pub struct Soft {
    fonts: Fonts,
    /// The interface's proportional font (`ui_text`), held for the life of the window: the title, the panel of
    /// windows and the tabs are always on screen. None when no UI font is installed: the grid's font stands in.
    ui_font: Option<std::rc::Rc<UiFont>>,
    glyphs: GlyphTable,
    /// Coverage bitmaps: tile `t` is `tiles[t * 3 * cw * ch..][..3 * cw * ch]` (fixed stride, no atlas
    /// geometry), three bytes a pixel: the coverage of its red, green and blue stripes (all equal
    /// for grey text and for the special tiles). Grows as glyphs are baked; the lazy bake goes away with task 12.
    tiles: Vec<u8>,
    next_tile: u32,
    opacity: f32,
    /// Margin left, right and below the grid, in pixels.
    pub pad: u32,
    /// Where the grid starts: the title bar plus a margin.
    pub top: u32,
    /// Space below the grid: a margin, or the status bar and a margin, and the composer's gaps (`set_metrics`).
    pub bottom: u32,
    /// The panel of windows and the settings panel beside the grid. On a shared edge the windows come first
    /// (outermost); the reference puts the settings on the other side, so they seldom share one.
    side: Dock,
    prefs: Dock,
    /// The last frame repainted everything: whatever is drawn beside the grid has to be drawn again.
    pub last_full: bool,
    /// The last frame repainted the title bar: the tabs drawn in it have to be drawn again.
    pub title_painted: bool,
    /// The windows are tabs in the title bar, which then has no title text.
    tabs_top: bool,
    ui: Ui,
    title: String,
    place: String,
    /// What the status bar says: the shell, the git branch (may be empty) and the grid size.
    status: Status,
    /// The title bar no longer shows `ui` and `title`.
    chrome_dirty: bool,
    /// The status bar no longer shows `ui` and `status`.
    status_dirty: bool,
    /// One glyph's coverage between calls of `paint_text` (the title and the status bar).
    scratch: Vec<u8>,
    /// Packed RGBA of the cursor overlay.
    pub cursor_color: u32,
    /// Framebuffer size and grid size of the last frame.
    w: usize,
    h: usize,
    rows: usize,
    cols: usize,
    /// The framebuffer no longer matches the glyph tiles or the geometry: repaint every row.
    stale: bool,
    /// A box, in window pixels, that the glass is painted over again at the next frame (`restore`): what an overlay
    /// painted there goes away, and the rows and the status bar are painted over the glass as they are.
    restore: Option<Rect>,
    /// Which grid rows were repainted in the last frame: whatever is drawn over a row (the prompt
    /// chips) has to be drawn again when the row was.
    touched: Vec<bool>,
    /// The panes as the framebuffer shows them (`set_layout`); empty means one pane filling the workspace.
    slots: Vec<Slot>,
    /// The pane being built (`select`): the geometry every row and chip of the next calls follows.
    cur: usize,
    /// How each pane's lines were laid on its rows at its last frame (`view::Fold`): where its rows are drawn, and
    /// so what the pointer is over.
    folds: Vec<Fold>,
    /// The frame being built repaints everything (decided when pane 0 began it).
    full: bool,
    /// The grid size of pane 0 at the last full frame.
    shape0: (usize, usize),
    /// Multiplies the alpha of cells and text painted next: 255 normally, `UNFOCUSED` for a pane without focus.
    dim: u32,
}

impl Soft {
    pub fn new(cfg: &FontCfg, opacity: f32) -> Self {
        let font_px = cfg.size;
        let mut s = Soft {
            fonts: Fonts::new(cfg),
            ui_font: UiFont::shared(),
            glyphs: GlyphTable::new(),
            tiles: Vec::new(),
            next_tile: T_FIRST_GLYPH,
            opacity,
            pad: PAD_X,
            top: 0,
            bottom: 0,
            side: Dock::default(),
            prefs: Dock::default(),
            last_full: true,
            title_painted: true,
            tabs_top: false,
            ui: Ui::IDLE,
            title: String::new(),
            place: String::new(),
            status: Status::default(),
            chrome_dirty: true,
            status_dirty: true,
            scratch: Vec::new(),
            cursor_color: pack(255, 255, 255, 255),
            w: 0,
            h: 0,
            rows: 0,
            cols: 0,
            stale: true,
            restore: None,
            touched: Vec::new(),
            slots: Vec::new(),
            cur: 0,
            folds: Vec::new(),
            full: true,
            shape0: (0, 0),
            dim: 255,
        };
        s.set_metrics(font_px);
        s.bake_specials();
        s
    }

    /// Leaves room beside the grid for the panel of windows and the settings panel.
    pub fn set_docks(&mut self, side: Dock, prefs: Dock) {
        if (side, prefs) != (self.side, self.prefs) {
            (self.side, self.prefs) = (side, prefs);
            self.stale = true;
        }
    }

    /// Whether the title bar holds the tabs of the windows instead of the window's title.
    pub fn set_tabs_top(&mut self, on: bool) {
        if on != self.tabs_top {
            self.tabs_top = on;
            self.chrome_dirty = true;
        }
    }

    /// The room taken beside the grid at the left and at the right, in pixels: what the panels add up to.
    pub fn inset(&self) -> (u32, u32) {
        let at = |left: bool| [self.side, self.prefs].iter().filter(|d| d.left == left).map(|d| d.w).sum();
        (at(true), at(false))
    }

    /// The panel of windows' box in framebuffer pixels (`x0, y0, x1, y1`): beside the grid, under the title
    /// bar and above the status bar. None when there is no panel.
    pub fn side_rect(&self) -> Option<(usize, usize, usize, usize)> {
        let (ww, wh) = self.window();
        self.side_box((ww as u32, wh as u32)).map(|b| self.to_rect(b))
    }

    /// The settings panel's box, in the same terms: next to the grid on its edge, inside the windows' panel.
    pub fn prefs_rect(&self) -> Option<(usize, usize, usize, usize)> {
        let (ww, wh) = self.window();
        self.prefs_box((ww as u32, wh as u32)).map(|b| self.to_rect(b))
    }

    /// The panel of windows' box `(x, y, w, h)` in window pixels, for a window `ww` x `wh` (the app knows the
    /// size before the next frame tells this renderer).
    pub fn side_box(&self, win: (u32, u32)) -> Option<(i32, i32, i32, i32)> {
        self.dock_box(self.side, 0, win)
    }

    /// The settings panel's box in window pixels.
    pub fn prefs_box(&self, win: (u32, u32)) -> Option<(i32, i32, i32, i32)> {
        let outer = if self.side.left == self.prefs.left { self.side.w } else { 0 };
        self.dock_box(self.prefs, outer, win)
    }

    fn dock_box(&self, d: Dock, outer: u32, (ww, wh): (u32, u32)) -> Option<(i32, i32, i32, i32)> {
        if d.w == 0 {
            return None;
        }
        let x = if d.left { outer as i32 } else { ww as i32 - outer as i32 - d.w as i32 };
        let h = wh as i32 - TITLE_H as i32 - if self.ui.status { STATUS_H as i32 } else { 0 };
        Some((x, TITLE_H as i32, d.w as i32, h))
    }

    /// A box in window pixels as framebuffer pixels `(x0, y0, x1, y1)`.
    fn to_rect(&self, (x, y, w, h): (i32, i32, i32, i32)) -> (usize, usize, usize, usize) {
        let (ox, oy) = self.origin();
        let (x0, y0) = ((ox as i32 + x).max(0) as usize, (oy as i32 + y).max(0) as usize);
        (x0, y0, x0 + w.max(0) as usize, y0 + h.max(0) as usize)
    }

    /// Paints the glass frame's background over `rect` (`side_rect`): the panel starts from a clean slate.
    pub fn clear_rect(&self, fb: &mut [u32], (x0, y0, x1, y1): (usize, usize, usize, usize)) {
        if fb.len() >= self.w * self.h {
            self.paint_bg(fb, y0, y1, x0, x1);
        }
    }

    pub fn cell(&self) -> (u32, u32) {
        (self.fonts.cell_w, self.fonts.cell_h)
    }

    /// Rebuilds the glyph tiles for a new size (scale / font size change).
    pub fn set_font_px(&mut self, px: f32) {
        self.fonts.set_px(px);
        self.reset_tiles(px);
    }

    /// Loads the fonts again from a new configuration (reload).
    pub fn set_fonts(&mut self, cfg: &FontCfg) {
        self.fonts = Fonts::new(cfg);
        self.reset_tiles(cfg.size);
    }

    /// Applies a configuration: the font file is read again only when the family changed.
    pub fn update_fonts(&mut self, cfg: &FontCfg) {
        if self.fonts.family() == cfg.family.as_deref() {
            self.fonts.reconfigure(cfg);
            self.reset_tiles(cfg.size);
        } else {
            self.set_fonts(cfg);
        }
    }

    /// The next frame repaints everything (an overlay was drawn over the grid, or taken away).
    pub fn invalidate(&mut self) {
        self.stale = true;
    }

    /// Takes away what was drawn over window pixels `r` (a toast, the way it moved or faded) without painting the
    /// whole window again: the next frame paints the glass over `r` and the status bar over it, and the caller
    /// invalidates the rows `rows_under` says. False, and nothing done, when `r` is not a part of the frame that
    /// can be painted alone: it reaches the title bar, a panel, or a pane's frame, which are painted whole or not
    /// at all; the caller then repaints everything (`invalidate`).
    pub fn restore(&mut self, r: Rect) -> bool {
        let (ww, wh) = self.window();
        let ws = self.workspace(ww as u32, wh as u32);
        // Past the window's bottom edge there is only the margin of its shadow, which nothing paints over.
        let r = Rect::new(r.x, r.y, r.w, r.y2().min(wh as i32) - r.y);
        if r.w <= 0 || r.h <= 0 || r.x < ws.x || r.x2() > ws.x2() || r.y < ws.y {
            return false;
        }
        let framed = self.slots.iter().any(|s| s.framed && s.rect.x < r.x2() && r.x < s.rect.x2() && s.rect.y < r.y2() && r.y < s.rect.y2());
        if framed {
            return false;
        }
        self.restore = Some(self.restore.map_or(r, |o| {
            let (x, y) = (o.x.min(r.x), o.y.min(r.y));
            Rect::new(x, y, o.x2().max(r.x2()) - x, o.y2().max(r.y2()) - y)
        }));
        if self.ui.status && r.y2() > wh as i32 - STATUS_H as i32 {
            self.status_dirty = true;
        }
        true
    }

    /// The rows of pane `i`'s text that window pixels `y0..y1` touch, as its last frame laid them out.
    pub fn rows_under(&self, i: usize, y0: i32, y1: i32) -> std::ops::Range<usize> {
        let slot = self.slot(i);
        let g = self.grid_of(&slot);
        let f = self.fold_of(i, g.rows);
        let bottom = self.band(&f, g.rows - 1).1;
        let (dy0, dy1) = (y0 - g.y, y1 - g.y);
        if dy1 <= 0 || dy0 >= bottom || dy1 <= dy0 {
            return 0..0;
        }
        self.row_at_dy(&f, dy0)..self.row_at_dy(&f, dy1 - 1) + 1
    }

    /// The row of pane `i` at window pixel row `y`, as its last frame laid them out: above the grid the first,
    /// below it the last (a selection dragged past the text goes on in it).
    pub fn row_at(&self, i: usize, y: f32) -> usize {
        let slot = self.slot(i);
        let g = self.grid_of(&slot);
        self.row_at_dy(&self.fold_of(i, g.rows), y.floor() as i32 - g.y)
    }

    /// Pane `i`'s layout of lines on rows at its last frame; the rows as the terminal has them before it had one.
    fn fold_of(&self, i: usize, rows: usize) -> Fold {
        self.folds.get(i).copied().filter(|f| f.rows == rows).unwrap_or(Fold::flat(rows, 0))
    }

    /// Between the chips' row and the prompt, in pixels.
    fn chips_gap(&self) -> i32 {
        (CHIPS_GAP_EM * self.fonts.px).round() as i32
    }

    /// Where row `r`'s cells start, below the top of the grid: a row of the composer lies below its gap, and below
    /// the chips' gap too when it follows the chips' row.
    fn cells_top(&self, f: &Fold, r: usize) -> i32 {
        let mut y = r as i32 * self.fonts.cell_h as i32;
        if f.docked() && r >= f.split {
            y += COMPOSER_GAP;
            if f.chips && r > f.split {
                y += self.chips_gap();
            }
        }
        y
    }

    /// The pixels row `r` owns, below the top of the grid: from where the row above it ends (so a gap belongs to the
    /// row below it, the composer's hairline to its first row) to where its cells end, or for the last row to the end
    /// of the space the grid keeps for the gaps. The rows tile that space whatever the layout, so a frame that lays
    /// the lines out anew and repaints every row leaves nothing of the old layout.
    fn band(&self, f: &Fold, r: usize) -> (i32, i32) {
        let ch = self.fonts.cell_h as i32;
        let top = if r == 0 { 0 } else { self.cells_top(f, r - 1) + ch };
        let bottom = if r + 1 >= f.rows { f.rows as i32 * ch + COMPOSER_GAP + self.chips_gap() } else { self.cells_top(f, r) + ch };
        (top, bottom)
    }

    /// The row whose band holds pixel row `dy` below the top of the grid, clamped to the rows.
    fn row_at_dy(&self, f: &Fold, dy: i32) -> usize {
        let ch = (self.fonts.cell_h as i32).max(1);
        let last = f.rows.saturating_sub(1);
        if dy < f.split as i32 * ch || !f.docked() {
            return ((dy.max(0) / ch) as usize).min(last);
        }
        (f.split..f.rows).find(|&r| dy < self.band(f, r).1).unwrap_or(last)
    }

    /// The top of row `row`'s cells of the pane being built, in window pixels.
    pub fn row_top(&self, row: usize) -> usize {
        let g = self.grid_of(&self.slot(self.cur));
        let f = self.fold_of(self.cur, g.rows);
        (g.y + self.cells_top(&f, row)).max(0) as usize
    }

    /// Changes the glass alpha (reload).
    pub fn set_opacity(&mut self, opacity: f32) {
        if opacity != self.opacity {
            self.opacity = opacity;
            self.stale = true;
        }
    }

    /// The margins around the grid for a font of `px` pixels.
    /// The margins round the grid: the reference's `.screen` and `.block` paddings added up (10 + 12 px beside the
    /// text, 14 + 2 above it), whatever the font size, as CSS has them. Below it, the composer's bottom padding, and
    /// the space its gaps take (`band`), kept whether it is shown or not: the rows do not change when it comes or goes.
    fn set_metrics(&mut self, _px: f32) {
        self.pad = PAD_X;
        self.top = TITLE_H as u32 + PAD_TOP;
        let below = if self.ui.status { STATUS_H as u32 + PAD_TOP } else { PAD_BOTTOM };
        self.bottom = below + (COMPOSER_GAP + self.chips_gap()) as u32;
    }

    fn reset_tiles(&mut self, px: f32) {
        self.glyphs.clear();
        self.next_tile = T_FIRST_GLYPH;
        self.set_metrics(px);
        self.tiles.clear();
        self.stale = true;
        self.bake_specials();
    }

    /// Window state that changes the look. Square corners (maximized) change the whole frame; the
    /// rest only the title bar.
    pub fn set_ui(&mut self, ui: Ui) {
        if ui == self.ui {
            return;
        }
        if ui.maximized != self.ui.maximized || ui.status != self.ui.status {
            self.stale = true;
        } else {
            self.chrome_dirty = true;
            self.status_dirty = true;
        }
        let status_changed = ui.status != self.ui.status;
        self.ui = ui;
        if status_changed {
            self.set_metrics(self.fonts.px);
        }
    }

    /// What the status bar says; it is repainted when this changes.
    pub fn set_status(&mut self, shell: &str, branch: &str, cols: usize, rows: usize) {
        let new = Status { shell: shell.to_string(), branch: branch.to_string(), cols, rows };
        if new != self.status {
            self.status = new;
            self.status_dirty = true;
        }
    }

    /// The title bar's text: the window's name (bold) and, after it, dim, where it is (its directory).
    pub fn set_title(&mut self, name: &str, place: &str) {
        if (name, place) != (self.title.as_str(), self.place.as_str()) {
            self.title.clear();
            self.title.push_str(name);
            self.place.clear();
            self.place.push_str(place);
            self.chrome_dirty = true;
        }
    }

    fn bake_specials(&mut self) {
        let f = &self.fonts;
        for data in special_tiles(f.cell_w, f.cell_h, f.px, f.ascent) {
            self.tiles.extend(data.iter().flat_map(|&c| [c, c, c]));
        }
    }

    /// Starts the frame of the pane selected (`select`, 0 first) on a `w` x `h` framebuffer holding a grid of
    /// `rows` x `cols` cells. Returns true when every row has to be written: first frame, the window size
    /// changed, the panes moved, or the tiles were reassigned. Pane 0 begins the frame and paints what is not
    /// a pane (the glass, the title bar, the status bar, the gaps); the others find that decided. A framed
    /// pane has its border painted here, on a full frame: a row never touches it.
    pub fn begin_frame(&mut self, fb: &mut [u32], w: usize, h: usize, rows: usize, cols: usize, fold: &Fold) -> bool {
        if self.cur == 0 {
            // The title and the status bar are not a pane's screen: never dimmed.
            let dim = std::mem::replace(&mut self.dim, 255);
            let full = self.stale || (self.w, self.h, self.shape0) != (w, h, (rows, cols));
            let (ox, oy) = self.origin();
            if full {
                self.stale = false;
                self.restore = None;
                (self.w, self.h, self.shape0) = (w, h, (rows, cols));
                self.paint_bg(fb, 0, h, 0, w);
                self.paint_title(fb);
                self.paint_status(fb);
            } else if fb.len() >= self.w * self.h {
                if let Some(r) = self.restore.take() {
                    self.paint_bg(fb, oy + r.y as usize, oy + r.y2() as usize, ox + r.x as usize, ox + r.x2() as usize);
                }
                if self.chrome_dirty {
                    self.paint_bg(fb, oy, oy + TITLE_H, ox, self.w - ox);
                    self.paint_title(fb);
                }
                if self.status_dirty && self.ui.status {
                    let (_, wh) = self.window();
                    self.paint_bg(fb, oy + wh - STATUS_H, oy + wh, ox, self.w - ox);
                    self.paint_status(fb);
                }
            }
            self.title_painted = full || self.chrome_dirty;
            self.chrome_dirty = false;
            self.status_dirty = false;
            self.last_full = full;
            self.full = full;
            self.dim = dim;
        }
        (self.rows, self.cols) = (rows, cols);
        if self.folds.len() <= self.cur {
            self.folds.resize(self.cur + 1, Fold::default());
        }
        self.folds[self.cur] = *fold;
        self.touched.clear();
        self.touched.resize(rows, self.full);
        if self.full && fb.len() >= self.w * self.h {
            let slot = self.slot(self.cur);
            if slot.framed {
                self.paint_frame(fb, &slot);
            }
        }
        self.full
    }

    /// Paints `count` consecutive rows of the pane selected, starting at `first`, from `cells` (`count * cols` cells).
    pub fn paint_rows(&mut self, fb: &mut [u32], first: usize, count: usize, cells: &[Cell]) {
        if first + count > self.rows || fb.len() < self.w * self.h {
            return;
        }
        let slot = self.slot(self.cur);
        let g = self.grid_of(&slot);
        let f = self.fold_of(self.cur, self.rows);
        // A row's background reaches the pane's edges; a frame's border and the pixel inside it are left alone.
        let edge = if slot.framed { 2 } else { 0 };
        let (ox, oy) = self.origin();
        let (x0, x1) = (ox + (slot.rect.x + edge).max(0) as usize, ox + (slot.rect.x2() - edge).max(0) as usize);
        let top = oy as i32 + g.y;
        for (n, cells) in cells.chunks_exact(self.cols).enumerate() {
            let r = first + n;
            if let Some(t) = self.touched.get_mut(r) {
                *t = true;
            }
            let (b0, b1) = self.band(&f, r);
            self.paint_bg(fb, (top + b0).max(0) as usize, (top + b1).max(0) as usize, x0, x1);
            if f.docked() && r == f.split {
                self.paint_composer_line(fb, &slot, g.y + b0 + COMPOSER_LINE_Y);
            }
            self.paint_cells_at(fb, ox + g.x.max(0) as usize, (top + self.cells_top(&f, r)).max(0) as usize, cells);
        }
    }

    /// The hairline over the composer (`.composer { border-top }`), across the pane at window pixel row `y`: as far
    /// as a row's background (`paint_rows`), and short of the window's own edge.
    fn paint_composer_line(&self, fb: &mut [u32], slot: &Slot, y: i32) {
        let (ox, oy) = self.origin();
        let (ww, _) = self.window();
        let (x0, x1) = if slot.framed {
            (slot.rect.x + 2, slot.rect.x2() - 2)
        } else {
            (slot.rect.x.max(1), slot.rect.x2().min(ww as i32 - 1))
        };
        let mut p = Paint { fb, w: self.w, h: self.h, ox: ox as i32, oy: oy as i32, fade: 1.0 };
        for x in x0..x1 {
            p.put(x, y, WHITE, COMPOSER_LINE_A);
        }
    }

    // --------------------------------------------------------------- panes --

    /// The workspace in window pixels, for a window `ww` x `wh`: between the title bar and the status bar and
    /// beside the panel of windows. One pane fills it; several are laid out in it (`panes::rect`).
    pub fn workspace(&self, ww: u32, wh: u32) -> Rect {
        let (il, ir) = (self.inset().0 as i32, self.inset().1 as i32);
        let status = if self.ui.status { STATUS_H as i32 } else { 0 };
        Rect::new(il, TITLE_H as i32, ww as i32 - il - ir, wh as i32 - TITLE_H as i32 - status)
    }

    /// The space between a pane's box and its text, `(left, top, right, bottom)`: for the only pane, the
    /// window's own margins (which `grid_size` has always used); for one of several, its 1 px border and the same
    /// margins inside it (the reference pads a split pane's screen as it pads the only one). Below, the space for the
    /// composer's gaps is kept too (`band`, `set_metrics`).
    pub fn margins(&self, framed: bool) -> (i32, i32, i32, i32) {
        if framed {
            let (x, top, bottom) = (PAD_X as i32 + 1, PAD_TOP as i32 + 1, PAD_BOTTOM as i32 + 1);
            (x, top, x, bottom + COMPOSER_GAP + self.chips_gap())
        } else {
            let (pad, gap) = (self.pad as i32, self.top as i32 - TITLE_H as i32);
            let below = self.bottom as i32 - if self.ui.status { STATUS_H as i32 } else { 0 };
            (pad, gap, pad, below)
        }
    }

    /// The text grid of a pane whose box is `rect`.
    pub fn grid(&self, rect: Rect, framed: bool) -> panes::Grid {
        panes::grid(rect, self.margins(framed), self.fonts.cell_w, self.fonts.cell_h)
    }

    /// The least a framed pane may be, in pixels: `cols` x `rows` cells and its margins.
    pub fn min_pane(&self, cols: usize, rows: usize) -> (i32, i32) {
        let (ml, mt, mr, mb) = self.margins(true);
        (ml + mr + (cols as u32 * self.fonts.cell_w) as i32, mt + mb + (rows as u32 * self.fonts.cell_h) as i32)
    }

    /// Tells the renderer where the panes are. When that differs from what the framebuffer shows, the next
    /// frame repaints everything (the gaps, the borders and the focus ring are only painted then).
    pub fn set_layout(&mut self, slots: &[Slot]) {
        if self.slots.as_slice() != slots {
            self.slots.clear();
            self.slots.extend_from_slice(slots);
            self.stale = true;
        }
    }

    /// The pane `i` of the layout; with none set, the one pane that fills the workspace.
    fn slot(&self, i: usize) -> Slot {
        self.slots.get(i).copied().unwrap_or_else(|| {
            let (ww, wh) = self.window();
            Slot { rect: self.workspace(ww as u32, wh as u32), focus: true, framed: false }
        })
    }

    fn grid_of(&self, slot: &Slot) -> panes::Grid {
        self.grid(slot.rect, slot.framed)
    }

    /// Makes pane `i` the one whose rows, chips and geometry the next calls are about.
    pub fn select(&mut self, i: usize) {
        self.cur = i;
    }

    /// Alpha (of 255) of the cells and text painted next: `UNFOCUSED` for a pane without focus, 255 otherwise.
    pub fn set_dim(&mut self, alpha: u32) {
        self.dim = alpha.min(255);
    }

    /// The same as a fraction, for shapes.
    pub fn dim_alpha(&self) -> f32 {
        self.dim as f32 / 255.0
    }

    /// The selected pane's text area in window pixels: where column 0 starts, the top of row 0, and the right
    /// end of the usable width (what the prompt chips are laid out in).
    pub fn content_box(&self) -> (usize, usize, usize) {
        let slot = self.slot(self.cur);
        let g = self.grid_of(&slot);
        let right = slot.rect.x2() - self.margins(slot.framed).2;
        (g.x.max(0) as usize, g.y.max(0) as usize, right.max(0) as usize)
    }

    /// The border of a framed pane: a hairline, or the accent at 55 % with a ring of 18 % just outside when
    /// it has the focus (the reference's `.is-focus-pane`).
    fn paint_frame(&self, fb: &mut [u32], slot: &Slot) {
        let (ox, oy) = self.origin();
        let mut p = Paint { fb, w: self.w, h: self.h, ox: ox as i32, oy: oy as i32, fade: 1.0 };
        let r = slot.rect;
        if slot.focus {
            let accent = palette::unit(palette::ACCENT);
            p.outline(r.x, r.y, r.w, r.h, panes::RADIUS, accent, 0.55);
            p.outline(r.x - 1, r.y - 1, r.w + 2, r.h + 2, panes::RADIUS + 1.0, accent, 0.18);
        } else {
            p.outline(r.x, r.y, r.w, r.h, panes::RADIUS, WHITE, 0.07);
        }
    }

    /// The grid rows of pane `i` that the cross at its top right is drawn over: they are repainted before the
    /// cross is, every frame, so it is never drawn twice over the same pixels.
    pub fn close_rows(&self, i: usize) -> std::ops::Range<usize> {
        let slot = self.slot(i);
        if !slot.framed {
            return 0..0;
        }
        let b = panes::close_box(slot.rect);
        self.rows_under(i, b.y, b.y2())
    }

    /// Puts the glass back under the part of pane `i`'s cross that lies in its top margin, above the rows: the rows
    /// under the rest are repainted (`close_rows`), that part never is. Called where the cross was, when it goes.
    pub fn clear_pane_close(&self, fb: &mut [u32], i: usize) {
        let slot = self.slot(i);
        if !slot.framed || fb.len() < self.w * self.h {
            return;
        }
        let (b, g) = (panes::close_box(slot.rect), self.grid_of(&slot));
        let (ox, oy) = self.origin();
        let (y0, y1) = (b.y.max(0) as usize, b.y2().min(g.y).max(0) as usize);
        if y1 > y0 {
            self.paint_bg(fb, oy + y0, oy + y1, ox + b.x.max(0) as usize, ox + b.x2().max(0) as usize);
        }
    }

    /// The cross that closes pane `i`, over the rows `close_rows` repainted (and the margin above them, put back
    /// first); `hot` when the pointer is on it.
    pub fn paint_pane_close(&self, fb: &mut [u32], i: usize, hot: bool) {
        let slot = self.slot(i);
        if !slot.framed || fb.len() < self.w * self.h {
            return;
        }
        self.clear_pane_close(fb, i);
        let b = panes::close_box(slot.rect);
        let (ox, oy) = self.origin();
        let mut p = Paint { fb, w: self.w, h: self.h, ox: ox as i32, oy: oy as i32, fade: 1.0 };
        p.rrect(b.x, b.y, b.w, b.h, 6.0, WHITE, if hot { 0.16 } else { 0.08 });
        let (cx, cy) = (b.x as f32 + b.w as f32 / 2.0, b.y as f32 + b.h as f32 / 2.0);
        let fg = palette::unit(palette::FG);
        p.shape(b.x, b.y, b.x2(), b.y2(), fg, 1.0, &|px, py| {
            let d = seg(px, py, (cx - 2.5, cy - 2.5), (cx + 2.5, cy + 2.5)).min(seg(px, py, (cx + 2.5, cy - 2.5), (cx - 2.5, cy + 2.5)));
            (1.2 - d).clamp(0.0, 1.0)
        });
    }

    // ------------------------------------------------------------ painting --

    /// Where the window starts in the framebuffer: the shadow margin (`chrome::margins`).
    pub fn origin(&self) -> (usize, usize) {
        let (ml, mt, _) = chrome::margins(self.ui.maximized);
        (ml, mt)
    }

    /// Cells in a row starting at pixel `(left, y0)` of the framebuffer, not tied to the grid's columns.
    pub fn paint_cells_at(&self, fb: &mut [u32], left: usize, y0: usize, cells: &[Cell]) {
        let (cw, ch) = (self.fonts.cell_w as usize, self.fonts.cell_h as usize);
        let len = cw * ch * 3;
        let cursor = unpack(self.cursor_color);
        let dim = self.dim;
        for (col, cell) in cells.iter().enumerate() {
            if cell.tf == 0 && cell.bg >> 24 == 0 {
                continue; // nothing to draw: the background is already there
            }
            let x0 = left + col * cw;
            if x0 + cw > self.w || y0 + ch > self.h {
                continue;
            }
            let tile = |t: u32| &self.tiles[t as usize * len..][..len];
            let glyph = tile(cell.tf & 0xffff);
            let flags = cell.tf >> 16;
            let (fg, bg) = (unpack(cell.fg), unpack(cell.bg));
            let shape = (flags >> 2) & 3;
            let shape_tile = match shape {
                1 => T_BEAM,
                2 => T_BAR,
                3 => T_BOX,
                _ => 0,
            };
            for y in 0..ch {
                let row = &mut fb[(y0 + y) * self.w + x0..][..cw];
                for (x, dst) in row.iter_mut().enumerate() {
                    let i = (y * cw + x) * 3;
                    let mut px = text_px3(fg, bg, [glyph[i] as u32, glyph[i + 1] as u32, glyph[i + 2] as u32]);
                    if flags & 1 != 0 {
                        px = overlay(px, fg, tile(T_UNDER)[i] as u32);
                    }
                    if flags & 2 != 0 {
                        px = overlay(px, fg, tile(T_STRIKE)[i] as u32);
                    }
                    if shape != 0 {
                        px = overlay(px, cursor, tile(shape_tile)[i] as u32);
                    }
                    if px[3] != 0 {
                        *dst = over(dimmed(px, dim), *dst);
                    }
                }
            }
        }
    }

    /// The metrics of text at `scale` times the grid's size (see `Fonts::small`).
    pub fn small(&self, scale: f32) -> crate::font::Small {
        self.fonts.small(scale)
    }

    /// `text` in the colour `fg` at the metrics `m`, from pixel `left`, vertically centred on `cy`, bold or not,
    /// over what is already in `fb`; the alpha of `fg` is its opacity. `scratch` holds one glyph's coverage between calls.
    pub fn paint_text(&self, fb: &mut [u32], left: usize, cy: usize, text: impl Iterator<Item = char>, fg: u32, bold: bool, m: &crate::font::Small, scratch: &mut Vec<u8>) {
        let (cw, ch) = (m.cell_w as usize, m.cell_h as usize);
        let Some(y0) = cy.checked_sub(ch / 2) else { return };
        let fg = unpack(fg);
        // The colour's own alpha (straight, not premultiplied) is an opacity, like the pane's dimming.
        let dim = div255(self.dim * fg[3]);
        let fg = [fg[0], fg[1], fg[2], 255];
        for (i, c) in text.enumerate() {
            let x0 = left + i * cw;
            if x0 + cw > self.w || y0 + ch > self.h {
                break;
            }
            self.fonts.raster_into(scratch, m, c, bold, false);
            for y in 0..ch {
                let row = &mut fb[(y0 + y) * self.w + x0..][..cw];
                for (x, dst) in row.iter_mut().enumerate() {
                    let i = (y * cw + x) * 3;
                    let cov = [scratch[i] as u32, scratch[i + 1] as u32, scratch[i + 2] as u32];
                    if cov != [0; 3] {
                        *dst = over(dimmed(text_px3(fg, [0; 4], cov), dim), *dst);
                    }
                }
            }
        }
    }

    /// Interface text (the title, the panel of windows, the tabs): `s` with its left edge at window pixel `x` and
    /// the middle of its capitals at `cy`, `px` high, in `c` at alpha `a`, of `weight` (400, `font::MEDIUM`,
    /// `font::BOLD`). The UI font, or with none installed the grid's font at that size (no alpha, bold for `BOLD`).
    #[allow(clippy::too_many_arguments)]
    pub fn ui_text(&self, fb: &mut [u32], x: i32, cy: i32, s: &str, px: f32, c: Rgb, a: f32, weight: u16, scratch: &mut Vec<u8>) {
        let (ox, oy) = self.origin();
        if fb.len() < self.w * self.h {
            return;
        }
        match &self.ui_font {
            Some(f) => {
                let mut p = Paint { fb, w: self.w, h: self.h, ox: ox as i32, oy: oy as i32, fade: 1.0 };
                let base = cy + (f.cap_height(px) / 2.0).round() as i32;
                let rgb = [c.r as f32 / 255.0, c.g as f32 / 255.0, c.b as f32 / 255.0];
                f.draw(s, px, x, base, weight, |gx, gy, cov| p.put(gx, gy, rgb, a * cov as f32 / 255.0));
            }
            None => {
                let m = self.fonts.small(px / self.fonts.px);
                let (Ok(x), Ok(cy)) = (usize::try_from(ox as i32 + x), usize::try_from(oy as i32 + cy)) else { return };
                self.paint_text(fb, x, cy, s.chars(), pack(c.r, c.g, c.b, 255), weight > crate::font::MEDIUM, &m, scratch);
            }
        }
    }

    /// How wide `ui_text` draws `s` at `px`.
    pub fn ui_width(&self, s: &str, px: f32) -> i32 {
        match &self.ui_font {
            Some(f) => f.width(s, px),
            None => s.chars().count() as i32 * self.fonts.small(px / self.fonts.px).cell_w as i32,
        }
    }

    /// The part of `s` that `ui_text` shows in `max` pixels at `px`, and whether it was cut (then an ellipsis goes
    /// after it, `ui_cut_text`). No string is made: the prefix is found by halving.
    pub fn ui_cut<'a>(&self, s: &'a str, px: f32, max: i32) -> (&'a str, bool) {
        if self.ui_width(s, px) <= max {
            return (s, false);
        }
        let ell = self.ui_width("\u{2026}", px);
        let prefix = |k: usize| s.char_indices().nth(k).map_or(s, |(b, _)| &s[..b]);
        let (mut lo, mut hi) = (0, s.chars().count());
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            if self.ui_width(prefix(mid), px) + ell <= max {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        (prefix(lo).trim_end(), true)
    }

    /// `ui_text` of `s` cut to `max` pixels with an ellipsis (`ui_cut`). Returns how wide it was drawn.
    #[allow(clippy::too_many_arguments)]
    pub fn ui_cut_text(&self, fb: &mut [u32], x: i32, cy: i32, s: &str, max: i32, px: f32, c: Rgb, a: f32, weight: u16, scratch: &mut Vec<u8>) -> i32 {
        let (shown, cut) = self.ui_cut(s, px, max);
        self.ui_text(fb, x, cy, shown, px, c, a, weight, scratch);
        let w = self.ui_width(shown, px);
        if !cut {
            return w;
        }
        self.ui_text(fb, x + w, cy, "\u{2026}", px, c, a, weight, scratch);
        w + self.ui_width("\u{2026}", px)
    }

    /// Whether grid row `row` was repainted in the last frame (all of them are after a full repaint).
    pub fn row_repainted(&self, row: usize) -> bool {
        self.touched.get(row).copied().unwrap_or(false)
    }

    /// The framebuffer size of the last frame.
    pub fn fb_size(&self) -> (usize, usize) {
        (self.w, self.h)
    }

    /// The window's size inside the framebuffer (without the shadow margin) of the last frame.
    pub fn window(&self) -> (usize, usize) {
        let (ml, mt, mb) = chrome::margins(self.ui.maximized);
        (self.w.saturating_sub(2 * ml), self.h.saturating_sub(mt + mb))
    }

    /// Background of pixels `x0..x1` of rows `y0..y1`: the glass frame and its shadow
    /// (`chrome::pixel`). Row repaints pass the window's own columns only: the shadow beside a
    /// row does not change between full repaints.
    fn paint_bg(&self, fb: &mut [u32], y0: usize, y1: usize, x0: usize, x1: usize) {
        // Nothing to paint in an empty or inverted span (a pane narrower than its frame): `clamp` would abort.
        if x0 >= x1 {
            return;
        }
        let (w, h) = (self.w, self.h);
        let (ml, mt, mb) = chrome::margins(self.ui.maximized);
        let (wh, reach) = ((h - mt - mb) as isize, REACH as isize);
        // Columns between the side bands: there a row can be one value.
        let (b0, b1) = ((ml + REACH).clamp(x0, x1), w.saturating_sub(ml + REACH).clamp(x0, x1));
        for y in y0..y1.min(h) {
            let row = &mut fb[y * w..][..w];
            let wy = y as isize - mt as isize;
            // Away from the title bar, the bottom edge and the corners a row is one value between
            // the side bands (chrome tests), in the margin rows above and below as well.
            if (wy < 0 || wy >= wh || (wy >= reach && wy + reach < wh)) && b1 > b0 {
                for x in (x0..b0).chain(b1..x1) {
                    row[x] = self.bg_px(x, y);
                }
                row[b0..b1].fill(self.bg_px(w / 2, y));
            } else {
                for x in x0..x1 {
                    row[x] = self.bg_px(x, y);
                }
            }
        }
    }

    fn bg_px(&self, x: usize, y: usize) -> u32 {
        chrome::pixel(x, y, self.w, self.h, self.opacity, &self.ui)
    }

    /// The title, centred in the part of the bar between the panel button and the gear (`.win-title`: `flex: 1` between them, so
    /// its middle is 34 px right of the window's), over the title bar background. A `user@host:` prefix is dim, the rest the
    /// foreground colour.
    fn paint_title(&mut self, fb: &mut [u32]) {
        if self.tabs_top {
            return;
        }
        // `.win-title`: 12.5 px, the name medium in the text colour, then (0.6 em on) the place, dim; centred in the
        // part of the bar between the panel button and the gear. What does not fit is cut from the end, the place first.
        const PX: f32 = 12.5;
        let (ox, _) = self.origin();
        let span = (self.w as i32 - 2 * ox as i32 - TITLE_X0 as i32 - TITLE_FROM_RIGHT as i32).max(0);
        let (name, place) = (std::mem::take(&mut self.title), std::mem::take(&mut self.place));
        let mut scratch = std::mem::take(&mut self.scratch);
        let gap = if name.is_empty() || place.is_empty() { 0 } else { (PX * 0.6).round() as i32 };
        let (name_w, place_w) = (self.ui_width(&name, PX), self.ui_width(&place, PX));
        let total = name_w + gap + place_w;
        let x0 = TITLE_X0 as i32 + ((span - total) / 2).max(0);
        let cy = (TITLE_H as i32 - 1) / 2;
        let a = if self.ui.focused { 1.0 } else { 0.55 };
        let drawn = self.ui_cut_text(fb, x0, cy, &name, span, PX, palette::FG, a, crate::font::MEDIUM, &mut scratch);
        let room = span - drawn - gap;
        if !place.is_empty() && room >= self.ui_width("\u{2026}", PX) * 3 {
            self.ui_cut_text(fb, x0 + drawn + gap, cy, &place, room, PX, palette::DIM, a, 400, &mut scratch);
        }
        (self.title, self.place, self.scratch) = (name, place, scratch);
    }

    /// The status bar: shell, encoding and branch on the left; key hints and the grid size on the
    /// right. Hints that do not fit are dropped from the left.
    fn paint_status(&mut self, fb: &mut [u32]) {
        if !self.ui.status {
            return;
        }
        let (ox, oy) = self.origin();
        let (ww, wh) = self.window();
        let (w, h) = (self.w, self.h);
        let top = wh - STATUS_H; // the hairline row, in window pixels
        let m = self.small(0.82);
        let (cw, tw) = (m.cell_w as usize, m.cell_h as usize);
        let cy = top + 1 + (STATUS_H - 1) / 2;
        let rgb = |c: Rgb| [c.r as f32 / 255.0, c.g as f32 / 255.0, c.b as f32 / 255.0];
        let pack = |c: Rgb| crate::cell::pack(c.r, c.g, c.b, 255);
        let (dim, fg) = (pack(palette::DIM), pack(palette::FG));
        let mut scratch = std::mem::take(&mut self.scratch);
        let status = std::mem::take(&mut self.status);
        let text = |this: &Self, fb: &mut [u32], x: usize, s: &str, colour: u32, scratch: &mut Vec<u8>| {
            this.paint_text(fb, ox + x, oy + cy, s.chars(), colour, false, &m, scratch);
            x + s.chars().count() * cw
        };

        // Left: a live dot, the shell, the encoding, the branch.
        let mut x = 14usize;
        {
            let mut p = Paint { fb: &mut *fb, w, h, ox: ox as i32, oy: oy as i32, fade: 1.0 };
            p.disc(x as f32 + 3.0, cy as f32, 6.0, rgb(palette::GREEN), 0.14);
            p.disc(x as f32 + 3.0, cy as f32, 3.0, rgb(palette::GREEN), 1.0);
        }
        x = text(self, fb, x + 12, &status.shell, dim, &mut scratch) + 16;
        x = text(self, fb, x, "UTF-8", dim, &mut scratch) + 16;
        if !status.branch.is_empty() {
            let mut p = Paint { fb: &mut *fb, w, h, ox: ox as i32, oy: oy as i32, fade: 1.0 };
            bar::icon(&mut p, Chip::Branch, x as f32 + 6.5, cy as f32, 0.9, rgb(palette::DIM));
            x = text(self, fb, x + 19, &status.branch, dim, &mut scratch) + 16;
        }

        // Right: the grid size, then as many hints as fit.
        let size = format!("{}\u{d7}{}", status.cols, status.rows);
        let size_w = size.chars().count() * cw;
        let right = ww.saturating_sub(14);
        text(self, fb, right.saturating_sub(size_w), &size, dim, &mut scratch);
        let hints: [(&[&str], &str); 4] = [(&["\u{2191}", "\u{2193}"], tr(Str::HintHistory)), (&["Tab"], tr(Str::HintComplete)), (&["Ctrl", "L"], tr(Str::HintClear)), (&["Ctrl", ","], tr(Str::HintSettings))];
        let cap_w = |k: &str| k.chars().count() * cw + 10;
        let hint_w = |(keys, label): &(&[&str], &str)| keys.iter().map(|k| cap_w(k) + 6).sum::<usize>() + label.chars().count() * cw;
        let mut room = right.saturating_sub(size_w + 16).saturating_sub(x + 16);
        let mut shown = 0;
        for hint in hints.iter().rev() {
            let need = hint_w(hint) + 16;
            if need > room {
                break;
            }
            room -= need;
            shown += 1;
        }
        let mut hx = right.saturating_sub(size_w + 16);
        for (keys, label) in hints.iter().rev().take(shown) {
            hx -= hint_w(&(keys, label));
            let mut at = hx;
            for k in keys.iter() {
                let (bw, bh) = (cap_w(k), tw + 3);
                {
                    let mut p = Paint { fb: &mut *fb, w, h, ox: ox as i32, oy: oy as i32, fade: 1.0 };
                    let by = (cy + 1) as i32 - bh as i32 / 2;
                    p.rrect(at as i32, by, bw as i32, bh as i32, 4.0, [1.0; 3], 0.04);
                    p.outline(at as i32, by, bw as i32, bh as i32, 4.0, [1.0; 3], 0.07);
                    for dx in 3..bw as i32 - 3 {
                        p.put(at as i32 + dx, by + bh as i32 - 2, [1.0; 3], 0.07); // the thicker bottom edge
                    }
                }
                text(self, fb, at + 5, k, fg, &mut scratch);
                at += bw + 6;
            }
            text(self, fb, at, label, dim, &mut scratch);
            hx -= 16;
        }
        self.scratch = scratch;
        self.status = status;
    }
}

impl Glyphs for Soft {
    fn glyph(&mut self, c: char, bold: bool, wide: bool) -> u32 {
        use crate::cell::T_BLANK;
        if c == ' ' || c == '\0' {
            return T_BLANK;
        }
        let style = bold as u8 | (wide as u8) << 1;
        let baked = self.glyphs.get(c, style);
        if baked != 0 {
            return baked;
        }
        let need = if wide { 2 } else { 1 };
        if self.next_tile + need > MAX_TILES {
            return T_BLANK; // out of tiles
        }
        let tile = self.next_tile;
        self.next_tile += need;
        let mut bmp = Vec::new();
        self.fonts.raster_into(&mut bmp, &self.fonts.grid(), c, bold, wide);
        let (cw, ch) = (self.fonts.cell_w as usize, self.fonts.cell_h as usize);
        if wide {
            for half in 0..2 {
                for y in 0..ch {
                    let from = (y * cw * 2 + half * cw) * 3;
                    self.tiles.extend_from_slice(&bmp[from..from + cw * 3]);
                }
            }
        } else {
            self.tiles.extend_from_slice(&bmp);
        }
        debug_assert_eq!(self.tiles.len(), self.next_tile as usize * cw * ch * 3);
        self.glyphs.set(c, style, tile);
        tile
    }
}

/// What the status bar says.
#[derive(Default, PartialEq)]
struct Status {
    shell: String,
    branch: String,
    cols: usize,
    rows: usize,
}

// ------------------------------------------------------------ pixel maths --
// Pixels are `[r, g, b, a]` in 0..=255, premultiplied unless said otherwise.

fn unpack(c: u32) -> [u32; 4] {
    [c & 255, (c >> 8) & 255, (c >> 16) & 255, c >> 24]
}

fn argb(p: [u32; 4]) -> u32 {
    p[3] << 24 | p[0] << 16 | p[1] << 8 | p[2]
}

fn from_argb(c: u32) -> [u32; 4] {
    [(c >> 16) & 255, (c >> 8) & 255, c & 255, c >> 24]
}

/// `x / 255` rounded to nearest, for `x <= 255 * 255`.
fn div255(x: u32) -> u32 {
    (x + 127) / 255
}

/// shader.wgsl `fs_cell` before the overlays: the glyph (`fg` at coverage `c`) over the cell
/// background, premultiplied. Plain source-over, so a cell with no background (alpha 0) keeps the
/// glyph at full strength: `fg * c`, alpha `c`.
#[cfg(test)]
fn text_px(fg: [u32; 4], bg: [u32; 4], c: u32) -> [u32; 4] {
    let a = bg[3] + div255((255 - bg[3]) * c);
    let m = |i: usize| (fg[i] * c * 255 + bg[i] * bg[3] * (255 - c) + 32512) / 65025;
    [m(0), m(1), m(2), a]
}

/// `text_px` with a coverage for each of the red, green and blue stripes of the pixel (LCD text). The
/// alpha follows the strongest one, so no colour channel can exceed it; with three equal coverages the
/// result is exactly `text_px`.
fn text_px3(fg: [u32; 4], bg: [u32; 4], c: [u32; 3]) -> [u32; 4] {
    let a = bg[3] + div255((255 - bg[3]) * c[0].max(c[1]).max(c[2]));
    let m = |i: usize| (fg[i] * c[i] * 255 + bg[i] * bg[3] * (255 - c[i]) + 32512) / 65025;
    [m(0), m(1), m(2), a]
}

/// shader.wgsl `overlay`: `color` at coverage `c` over the premultiplied `dst`.
fn overlay(dst: [u32; 4], color: [u32; 4], c: u32) -> [u32; 4] {
    if c == 0 {
        return dst;
    }
    let m = |i: usize| (div255(color[i] * c) + div255(dst[i] * (255 - c))).min(255);
    [m(0), m(1), m(2), (c + div255(dst[3] * (255 - c))).min(255)]
}

/// `px` with its alpha (and so its colour: premultiplied) scaled by `dim` / 255: what CSS `opacity` does to a layer.
fn dimmed(px: [u32; 4], dim: u32) -> [u32; 4] {
    if dim >= 255 {
        return px;
    }
    px.map(|c| div255(c * dim))
}

/// Blend state of the shader's pipeline: `src + dst * (1 - src.a)`, both premultiplied.
fn over(src: [u32; 4], dst: u32) -> u32 {
    if src[3] == 255 {
        return argb(src);
    }
    let d = from_argb(dst);
    let m = |i: usize| (src[i] + div255(d[i] * (255 - src[3]))).min(255);
    argb([m(0), m(1), m(2), m(3)])
}

/// A `Soft` painting into one framebuffer for one frame: what `View::build` drives.
pub struct Target<'a> {
    pub soft: &'a mut Soft,
    pub fb: &'a mut [u32],
    pub w: usize,
    pub h: usize,
}

impl Glyphs for Target<'_> {
    fn glyph(&mut self, c: char, bold: bool, wide: bool) -> u32 {
        self.soft.glyph(c, bold, wide)
    }
}

impl Backend for Target<'_> {
    fn begin_frame(&mut self, rows: usize, cols: usize, fold: &Fold) -> bool {
        self.soft.begin_frame(self.fb, self.w, self.h, rows, cols, fold)
    }

    fn put_rows(&mut self, first: usize, count: usize, cells: &[Cell]) {
        self.soft.paint_rows(self.fb, first, count, cells);
    }

    fn docks(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_docks_sit_at_their_edges_and_the_windows_panel_is_the_outermost() {
        let mut s = Soft::new(&FontCfg::sized(15.0), 0.88);
        s.set_ui(Ui { status: true, ..Ui::IDLE });
        let (win, body) = ((1000, 640), 640 - TITLE_H as i32 - STATUS_H as i32);
        assert_eq!((s.side_box(win), s.prefs_box(win), s.inset()), (None, None, (0, 0)));
        // The reference's usual: the windows at the left, the settings at the right.
        s.set_docks(Dock { w: 220, left: true }, Dock { w: 372, left: false });
        assert_eq!(s.inset(), (220, 372));
        assert_eq!(s.side_box(win), Some((0, TITLE_H as i32, 220, body)));
        assert_eq!(s.prefs_box(win), Some((628, TITLE_H as i32, 372, body)));
        // The windows at the right: the settings go to the left, and the room adds up the other way round.
        s.set_docks(Dock { w: 220, left: false }, Dock { w: 372, left: true });
        assert_eq!(s.inset(), (372, 220));
        assert_eq!((s.side_box(win).unwrap().0, s.prefs_box(win).unwrap().0), (780, 0));
        // Two panels on one edge stack: the windows outermost, the settings next to the grid.
        s.set_docks(Dock { w: 200, left: false }, Dock { w: 300, left: false });
        assert_eq!((s.side_box(win).unwrap().0, s.prefs_box(win).unwrap().0, s.inset()), (800, 500, (0, 500)));
        s.set_docks(Dock { w: 200, left: true }, Dock { w: 300, left: true });
        assert_eq!((s.side_box(win).unwrap().0, s.prefs_box(win).unwrap().0, s.inset()), (0, 200, (500, 0)));
        // The status bar's 28 px belong to the grid column when it is off.
        s.set_ui(Ui { status: false, ..Ui::IDLE });
        assert_eq!(s.prefs_box(win).map(|b| b.3), Some(body + STATUS_H as i32));
        // Nothing: no box.
        s.set_docks(Dock::default(), Dock::default());
        assert_eq!((s.side_box(win), s.prefs_box(win)), (None, None));
    }

    #[test]
    fn the_title_is_centred_between_the_panel_button_and_the_gear() {
        // The reference's `.win-title` is `flex: 1` from 114 (the panel button ends at 104, 10 px of gap) to 46 px short of
        // the right edge (the gear, 28 px, 8 of padding and 10 of gap): its middle is 34 px right of the window's.
        let mut s = Soft::new(&FontCfg::sized(15.0), 0.88);
        let (w, h) = (1000, 700);
        let ink = |s: &mut Soft| {
            let mut fb = vec![0u32; w * h];
            s.begin_frame(&mut fb, w, h, 10, 40, &Fold::default());
            let (ox, oy) = s.origin();
            // The bar's text rows, between the buttons' columns and the gear's.
            let mut xs = (usize::MAX, 0);
            for y in oy + 4..oy + TITLE_H - 4 {
                for x in ox + 100..w - ox - 40 {
                    if fb[y * w + x] & 0xff > 120 {
                        xs = (xs.0.min(x - ox), xs.1.max(x - ox));
                    }
                }
            }
            xs
        };
        s.set_title("arcton", "~/lab/apps/arcton");
        let (a, z) = ink(&mut s);
        let ww = s.window().0;
        let want = (TITLE_X0 + (ww - TITLE_FROM_RIGHT)) as i32 / 2;
        let mid = ((a + z) / 2) as i32;
        assert!((mid - want).abs() <= 8, "the text's middle is {mid}, the part's {want}");
        assert!(a >= TITLE_X0 && z < ww - TITLE_FROM_RIGHT, "{a}..{z} stays inside the part");
        // A title too long for it is cut to it, not run under the buttons.
        s.set_title(&"x".repeat(200), "");
        let (a, z) = ink(&mut s);
        assert!(a >= TITLE_X0 && z < ww - TITLE_FROM_RIGHT, "{a}..{z} stays inside the part");
    }

    #[test]
    fn changing_the_docks_asks_for_a_whole_frame_and_the_same_docks_do_not() {
        let mut s = Soft::new(&FontCfg::sized(15.0), 0.88);
        let (w, h) = (1000, 700);
        let mut fb = vec![0u32; w * h];
        s.set_docks(Dock { w: 100, left: false }, Dock::default());
        assert!(s.begin_frame(&mut fb, w, h, 10, 40, &Fold::default()), "the first frame is whole");
        assert!(!s.begin_frame(&mut fb, w, h, 10, 40, &Fold::default()));
        s.set_docks(Dock { w: 100, left: false }, Dock::default());
        assert!(!s.begin_frame(&mut fb, w, h, 10, 40, &Fold::default()), "the same docks change nothing");
        s.set_docks(Dock { w: 100, left: false }, Dock { w: 372, left: true });
        assert!(s.begin_frame(&mut fb, w, h, 10, 40, &Fold::default()), "a panel that opens repaints everything");
        assert!(s.prefs_rect().is_some_and(|(x0, y0, x1, y1)| x1 - x0 == 372 && y1 > y0));
    }

    #[test]
    fn rows_are_painted_without_aborting_when_the_panels_leave_the_grid_no_width() {
        // A 500 px window with the windows' panel (220) and the settings (310): the workspace is 30 px short.
        let mut s = Soft::new(&FontCfg::sized(15.0), 0.88);
        let (w, h) = (500, 400);
        let mut fb = vec![0u32; w * h];
        s.set_docks(Dock { w: 220, left: true }, Dock { w: 310, left: false });
        assert!(s.workspace(w as u32, h as u32).w < 0);
        s.begin_frame(&mut fb, w, h, 4, 10, &Fold::default());
        s.paint_rows(&mut fb, 0, 2, &[bytemuck::Zeroable::zeroed(); 20]);
        // A framed pane narrower than its border is the same.
        s.set_docks(Dock::default(), Dock::default());
        s.set_layout(&[Slot { rect: Rect::new(10, 40, 3, 100), focus: true, framed: true }]);
        s.begin_frame(&mut fb, w, h, 4, 10, &Fold::default());
        s.paint_rows(&mut fb, 0, 2, &[bytemuck::Zeroable::zeroed(); 20]);
    }

    #[test]
    fn three_equal_coverages_are_the_grey_result() {
        for c in [0, 1, 77, 128, 254, 255] {
            for bg in [[0, 0, 0, 0], [10, 20, 30, 255], [10, 20, 30, 224]] {
                assert_eq!(text_px3([200, 100, 50, 255], bg, [c; 3]), text_px([200, 100, 50, 255], bg, c));
            }
        }
    }

    #[test]
    fn stripes_with_different_coverage_colour_the_edge_and_never_exceed_the_alpha() {
        // Only the red stripe covered, over glass: red at its own strength, and alpha at least that.
        let p = text_px3([255, 255, 255, 255], [0, 0, 0, 0], [200, 0, 0]);
        assert_eq!((p[0], p[1], p[2]), (200, 0, 0));
        for c in [[0, 90, 255], [255, 0, 3], [17, 130, 255]] {
            let p = text_px3([255, 255, 255, 255], [0, 0, 0, 0], c);
            assert!(p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3], "{p:?}");
        }
    }

    #[test]
    fn full_coverage_is_the_foreground() {
        assert_eq!(text_px([200, 100, 50, 255], [0, 0, 0, 0], 255), [200, 100, 50, 255]);
    }

    #[test]
    fn partial_coverage_over_transparent_is_plain_source_over() {
        // Half coverage of white is half white at half alpha, not a quarter: edges keep their weight.
        let p = text_px([255, 255, 255, 255], [0, 0, 0, 0], 128);
        assert_eq!(p, [128, 128, 128, 128]);
    }

    #[test]
    fn no_coverage_over_transparent_is_nothing() {
        assert_eq!(text_px([200, 100, 50, 255], [0, 0, 0, 0], 0)[3], 0);
    }

    #[test]
    fn opaque_background_ignores_coverage_for_alpha() {
        let p = text_px([255, 255, 255, 255], [10, 20, 30, 255], 0);
        assert_eq!(p, [10, 20, 30, 255]);
    }

    #[test]
    fn full_overlay_replaces_and_none_keeps() {
        let dst = [10, 20, 30, 255];
        assert_eq!(overlay(dst, [200, 100, 50, 255], 255), [200, 100, 50, 255]);
        assert_eq!(overlay(dst, [200, 100, 50, 255], 0), dst);
    }

    #[test]
    fn over_an_opaque_source_replaces_and_transparent_keeps() {
        assert_eq!(over([1, 2, 3, 255], 0xff_ff_ff_ff), 0xff_01_02_03);
        let dst = 0xed_0a_14_1e;
        assert_eq!(over([0, 0, 0, 0], dst), dst);
    }
}
