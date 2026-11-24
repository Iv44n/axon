//! CPU renderer ([0011](../knowledge/decisions/0011-software-render-wl-shm.md)): paints the cell
//! grid straight into a premultiplied `Argb8888` framebuffer, the memory a `wl_shm` buffer maps.
//!
//! The framebuffer is the retained state: only damaged rows are repainted into it, so there is no
//! cell buffer and no copy. Cells, text coverage and overlays match shader.wgsl; the frame around
//! them (glass body, rounded corners, title bar and buttons) is `chrome.rs` and exists only here.
//! Driven by `wlapp.rs` (`AXON_BACKEND=wl`); see `knowledge/plan/software-renderer.md`.
#![allow(dead_code)]

use crate::cell::{
    Cell, Glyphs, MAX_TILES, T_BAR, T_BEAM, T_BOX, T_FIRST_GLYPH, T_STRIKE, T_UNDER, pack,
};
use crate::chrome::{self, REACH, TITLE_CLEAR, TITLE_H, Ui};
use crate::config::FontCfg;
use crate::font::{Fonts, GlyphTable, special_tiles};
use crate::view::Backend;

pub struct Soft {
    fonts: Fonts,
    glyphs: GlyphTable,
    /// Coverage bitmaps: tile `t` is `tiles[t * cw * ch..][..cw * ch]` (fixed stride, no atlas
    /// geometry). Grows as glyphs are baked; the lazy bake goes away with task 12.
    tiles: Vec<u8>,
    next_tile: u32,
    opacity: f32,
    /// Margin left, right and below the grid, in pixels.
    pub pad: u32,
    /// Where the grid starts: the title bar plus a margin.
    pub top: u32,
    ui: Ui,
    title: String,
    /// The title bar no longer shows `ui` and `title`.
    chrome_dirty: bool,
    /// Packed RGBA of the cursor overlay.
    pub cursor_color: u32,
    /// Framebuffer size and grid size of the last frame.
    w: usize,
    h: usize,
    rows: usize,
    cols: usize,
    /// The framebuffer no longer matches the glyph tiles or the geometry: repaint every row.
    stale: bool,
}

impl Soft {
    pub fn new(cfg: &FontCfg, opacity: f32) -> Self {
        let font_px = cfg.size;
        let mut s = Soft {
            fonts: Fonts::new(cfg),
            glyphs: GlyphTable::new(),
            tiles: Vec::new(),
            next_tile: T_FIRST_GLYPH,
            opacity,
            pad: (font_px * 0.9).round() as u32,
            top: TITLE_H as u32 + (font_px * 0.45).round() as u32,
            ui: Ui::IDLE,
            title: String::new(),
            chrome_dirty: true,
            cursor_color: pack(255, 255, 255, 255),
            w: 0,
            h: 0,
            rows: 0,
            cols: 0,
            stale: true,
        };
        s.bake_specials();
        s
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

    /// Changes the glass alpha (reload).
    pub fn set_opacity(&mut self, opacity: f32) {
        if opacity != self.opacity {
            self.opacity = opacity;
            self.stale = true;
        }
    }

    fn reset_tiles(&mut self, px: f32) {
        self.glyphs.clear();
        self.next_tile = T_FIRST_GLYPH;
        self.pad = (px * 0.9).round() as u32;
        self.top = TITLE_H as u32 + (px * 0.45).round() as u32;
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
        if ui.maximized != self.ui.maximized {
            self.stale = true;
        } else {
            self.chrome_dirty = true;
        }
        self.ui = ui;
    }

    pub fn set_title(&mut self, title: &str) {
        if title != self.title {
            self.title.clear();
            self.title.push_str(title);
            self.chrome_dirty = true;
        }
    }

    fn tile_len(&self) -> usize {
        (self.fonts.cell_w * self.fonts.cell_h) as usize
    }

    fn bake_specials(&mut self) {
        let f = &self.fonts;
        for data in special_tiles(f.cell_w, f.cell_h, f.px, f.ascent) {
            self.tiles.extend_from_slice(&data);
        }
    }

    /// Starts a frame on a `w` x `h` framebuffer holding a grid of `rows` x `cols` cells. Returns
    /// true when every row has to be written: first frame, the sizes changed, or the tiles were
    /// reassigned. The margins around the grid and the title bar are painted here.
    pub fn begin_frame(&mut self, fb: &mut [u32], w: usize, h: usize, rows: usize, cols: usize) -> bool {
        let full = self.stale || (self.w, self.h, self.rows, self.cols) != (w, h, rows, cols);
        let (ox, oy) = self.origin();
        if full {
            self.stale = false;
            (self.w, self.h, self.rows, self.cols) = (w, h, rows, cols);
            let (top, bottom) = (oy + self.top as usize, oy + self.top as usize + rows * self.fonts.cell_h as usize);
            self.paint_bg(fb, 0, top, 0, w);
            self.paint_bg(fb, bottom, h, 0, w);
            // Beside the grid rows: the shadow, which row repaints leave alone.
            self.paint_bg(fb, top, bottom, 0, ox);
            self.paint_bg(fb, top, bottom, w - ox, w);
            self.paint_title(fb);
        } else if self.chrome_dirty && fb.len() >= self.w * self.h {
            self.paint_bg(fb, oy, oy + TITLE_H, ox, self.w - ox);
            self.paint_title(fb);
        }
        self.chrome_dirty = false;
        full
    }

    /// Paints `count` consecutive rows starting at `first` from `cells` (`count * cols` cells).
    pub fn paint_rows(&mut self, fb: &mut [u32], first: usize, count: usize, cells: &[Cell]) {
        if first + count > self.rows || fb.len() < self.w * self.h {
            return;
        }
        let (ox, oy) = self.origin();
        let (top, ch) = (oy + self.top as usize, self.fonts.cell_h as usize);
        for (n, cells) in cells.chunks_exact(self.cols).enumerate() {
            let y0 = top + (first + n) * ch;
            self.paint_bg(fb, y0, y0 + ch, ox, self.w - ox);
            self.paint_cells(fb, y0, cells);
        }
    }

    // ------------------------------------------------------------ painting --

    /// Where the window starts in the framebuffer: the shadow margin (`chrome::margins`).
    fn origin(&self) -> (usize, usize) {
        let (ml, mt, _) = chrome::margins(self.ui.maximized);
        (ml, mt)
    }

    fn paint_cells(&self, fb: &mut [u32], y0: usize, cells: &[Cell]) {
        let (cw, ch) = (self.fonts.cell_w as usize, self.fonts.cell_h as usize);
        let len = cw * ch;
        let cursor = unpack(self.cursor_color);
        let left = self.origin().0 + self.pad as usize;
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
                    let i = y * cw + x;
                    let mut px = text_px(fg, bg, glyph[i] as u32);
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
                        *dst = over(px, *dst);
                    }
                }
            }
        }
    }

    /// Background of pixels `x0..x1` of rows `y0..y1`: the glass frame and its shadow
    /// (`chrome::pixel`). Row repaints pass the window's own columns only: the shadow beside a
    /// row does not change between full repaints.
    fn paint_bg(&self, fb: &mut [u32], y0: usize, y1: usize, x0: usize, x1: usize) {
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

    /// The title, centred between the buttons and the right margin, over the title bar background.
    fn paint_title(&mut self, fb: &mut [u32]) {
        let (cw, ch, w) = (self.fonts.cell_w as usize, self.fonts.cell_h as usize, self.w);
        let (ox, oy) = self.origin();
        let ww = w - 2 * ox;
        let room = ww.saturating_sub(2 * TITLE_CLEAR) / cw;
        let title = std::mem::take(&mut self.title);
        let count = title.chars().count().min(room);
        let y0 = oy + (TITLE_H - ch) / 2;
        let x0 = ox + (ww - count * cw) / 2;
        let strength = if self.ui.focused { 190 } else { 100 };
        let len = cw * ch;
        for (n, c) in title.chars().take(count).enumerate() {
            let c = if n + 1 == count && title.chars().count() > count { '\u{2026}' } else { c };
            let tile = self.glyph(c, false, false) as usize;
            for y in 0..ch {
                for x in 0..cw {
                    let cov = self.tiles[tile * len + y * cw + x] as u32 * strength / 255;
                    if cov != 0 {
                        let dst = &mut fb[(y0 + y) * w + x0 + n * cw + x];
                        *dst = argb(overlay(from_argb(*dst), [235, 235, 235, 255], cov));
                    }
                }
            }
        }
        self.title = title;
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
        let bmp = self.fonts.raster(c, bold, wide);
        let (cw, ch) = (self.fonts.cell_w as usize, self.fonts.cell_h as usize);
        if wide {
            for half in 0..2 {
                for y in 0..ch {
                    let from = y * cw * 2 + half * cw;
                    self.tiles.extend_from_slice(&bmp[from..from + cw]);
                }
            }
        } else {
            self.tiles.extend_from_slice(&bmp);
        }
        debug_assert_eq!(self.tiles.len(), self.next_tile as usize * cw * ch);
        self.glyphs.set(c, style, tile);
        tile
    }
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
fn text_px(fg: [u32; 4], bg: [u32; 4], c: u32) -> [u32; 4] {
    let a = bg[3] + div255((255 - bg[3]) * c);
    let m = |i: usize| (fg[i] * c * 255 + bg[i] * bg[3] * (255 - c) + 32512) / 65025;
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
    fn begin_frame(&mut self, rows: usize, cols: usize) -> bool {
        self.soft.begin_frame(self.fb, self.w, self.h, rows, cols)
    }

    fn put_rows(&mut self, first: usize, count: usize, cells: &[Cell]) {
        self.soft.paint_rows(self.fb, first, count, cells);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
