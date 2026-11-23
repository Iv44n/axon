//! CPU renderer ([0011](../knowledge/decisions/0011-software-render-wl-shm.md)): paints the cell
//! grid straight into a premultiplied `Argb8888` framebuffer, the memory a `wl_shm` buffer maps.
//!
//! The framebuffer is the retained state: only damaged rows are repainted into it, so there is no
//! cell buffer and no copy. The look is shader.wgsl on the CPU (squared text coverage, overlays,
//! gradient and neon border background); keep the two in sync until the GPU path is deleted.
//! Not wired into the app yet: stage B of `knowledge/plan/software-renderer.md` does that.
#![allow(dead_code)]

use crate::cell::{
    Cell, Glyphs, MAX_TILES, T_BAR, T_BEAM, T_BOX, T_FIRST_GLYPH, T_STRIKE, T_UNDER, pack,
};
use crate::font::{Fonts, GlyphTable, special_tiles};

/// Distance from a window edge beyond which the neon glow is too faint to show in 8 bits
/// (`exp(-48/7) * 0.16` is 0.0002); inside it every pixel is computed.
const GLOW_REACH: f32 = 48.0;

pub struct Soft {
    fonts: Fonts,
    glyphs: GlyphTable,
    /// Coverage bitmaps: tile `t` is `tiles[t * cw * ch..][..cw * ch]` (fixed stride, no atlas
    /// geometry). Grows as glyphs are baked; the lazy bake goes away with task 12.
    tiles: Vec<u8>,
    next_tile: u32,
    opacity: f32,
    /// Margin around the grid, in pixels.
    pub pad: u32,
    /// Packed RGBA of the cursor overlay.
    pub cursor_color: u32,
    /// Framebuffer size and grid size of the last frame.
    w: usize,
    h: usize,
    rows: usize,
    cols: usize,
    /// Scratch for the rows being painted; dead once they are.
    stage: Vec<Cell>,
    /// The framebuffer no longer matches the glyph tiles or the geometry: repaint every row.
    stale: bool,
}

impl Soft {
    pub fn new(font_px: f32, opacity: f32) -> Self {
        let mut s = Soft {
            fonts: Fonts::new(font_px),
            glyphs: GlyphTable::new(),
            tiles: Vec::new(),
            next_tile: T_FIRST_GLYPH,
            opacity,
            pad: (font_px * 0.9).round() as u32,
            cursor_color: pack(255, 255, 255, 255),
            w: 0,
            h: 0,
            rows: 0,
            cols: 0,
            stage: Vec::new(),
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
        self.glyphs.clear();
        self.next_tile = T_FIRST_GLYPH;
        self.pad = (px * 0.9).round() as u32;
        self.tiles.clear();
        self.stale = true;
        self.bake_specials();
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
    /// reassigned. The margins around the grid are painted here.
    pub fn begin_frame(&mut self, fb: &mut [u32], w: usize, h: usize, rows: usize, cols: usize) -> bool {
        let full = self.stale || (self.w, self.h, self.rows, self.cols) != (w, h, rows, cols);
        if full {
            self.stale = false;
            (self.w, self.h, self.rows, self.cols) = (w, h, rows, cols);
            let (pad, ch) = (self.pad as usize, self.fonts.cell_h as usize);
            self.paint_bg(fb, 0, pad);
            self.paint_bg(fb, pad + rows * ch, h);
        }
        full
    }

    /// Paints `count` consecutive rows starting at `first`. `fill` receives `count * cols` cells
    /// (blank) and must set every cell that is not blank.
    pub fn write_rows(
        &mut self,
        fb: &mut [u32],
        first: usize,
        count: usize,
        fill: impl FnOnce(&mut Soft, &mut [Cell]),
    ) {
        if first + count > self.rows || fb.len() < self.w * self.h {
            return;
        }
        let mut stage = std::mem::take(&mut self.stage);
        stage.clear();
        stage.resize(count * self.cols, Cell::BLANK);
        fill(self, &mut stage);
        let (pad, ch) = (self.pad as usize, self.fonts.cell_h as usize);
        for (n, cells) in stage.chunks_exact(self.cols).enumerate() {
            let y0 = pad + (first + n) * ch;
            self.paint_bg(fb, y0, y0 + ch);
            self.paint_cells(fb, y0, cells);
        }
        self.stage = stage;
    }

    // ------------------------------------------------------------ painting --

    fn paint_cells(&self, fb: &mut [u32], y0: usize, cells: &[Cell]) {
        let (cw, ch) = (self.fonts.cell_w as usize, self.fonts.cell_h as usize);
        let len = cw * ch;
        let cursor = unpack(self.cursor_color);
        for (col, cell) in cells.iter().enumerate() {
            if cell.tf == 0 && cell.bg >> 24 == 0 {
                continue; // nothing to draw: the background is already there
            }
            let x0 = self.pad as usize + col * cw;
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

    /// Background of pixel rows `y0..y1`: vertical gradient plus the neon border glow.
    fn paint_bg(&self, fb: &mut [u32], y0: usize, y1: usize) {
        let (w, h) = (self.w, self.h);
        let reach = GLOW_REACH.ceil() as usize;
        for y in y0..y1.min(h) {
            let row = &mut fb[y * w..][..w];
            let py = y as f32 + 0.5;
            if py.min(h as f32 - py) >= GLOW_REACH && w > 2 * reach {
                for x in 0..reach {
                    row[x] = self.bg_px(x, y);
                }
                row[reach..w - reach].fill(self.bg_px(w / 2, y)); // no glow: constant along the row
                for x in w - reach..w {
                    row[x] = self.bg_px(x, y);
                }
            } else {
                for (x, px) in row.iter_mut().enumerate() {
                    *px = self.bg_px(x, y);
                }
            }
        }
    }

    /// shader.wgsl `fs_bg`, for the pixel whose centre is `(x + 0.5, y + 0.5)`.
    fn bg_px(&self, x: usize, y: usize) -> u32 {
        let (w, h) = (self.w as f32, self.h as f32);
        let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
        let (u, v) = (px / w, py / h);
        let d = px.min(w - px).min(py.min(h - py));
        let glow = if d < GLOW_REACH { (-d / 7.0).exp() * 0.16 + if d < 1.0 { 0.55 } else { 0.0 } } else { 0.0 };
        let mix = |a: f32, b: f32, t: f32| a + (b - a) * t;
        let top = [0.020, 0.030, 0.060];
        let bot = [0.040, 0.020, 0.075];
        let acc_a = [0.10, 0.89, 1.0];
        let acc_b = [0.78, 0.49, 1.0];
        let a = (self.opacity + glow * 0.5).clamp(0.0, 1.0);
        let ch = |i: usize| {
            let c = mix(top[i], bot[i], v) + mix(acc_a[i], acc_b[i], u) * glow;
            ((c * a * 255.0 + 0.5) as u32).min(255)
        };
        argb([ch(0), ch(1), ch(2), (a * 255.0 + 0.5) as u32])
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

/// shader.wgsl `fs_cell` before the overlays: the glyph at coverage `c` over the cell background,
/// premultiplied. Text on a transparent background gets squared coverage, the look of the shader.
fn text_px(fg: [u32; 4], bg: [u32; 4], c: u32) -> [u32; 4] {
    let a = bg[3] + div255((255 - bg[3]) * c);
    let m = |i: usize| ((bg[i] * (255 - c) + fg[i] * c) * a + 32512) / 65025;
    [m(0), m(1), m(2), a]
}

/// shader.wgsl `overlay`: `color` at coverage `c`, with squared coverage, over premultiplied `dst`.
fn overlay(dst: [u32; 4], color: [u32; 4], c: u32) -> [u32; 4] {
    if c == 0 {
        return dst;
    }
    let m = |i: usize| ((color[i] * c * c + 32512) / 65025 + div255(dst[i] * (255 - c))).min(255);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_coverage_is_the_foreground() {
        assert_eq!(text_px([200, 100, 50, 255], [0, 0, 0, 0], 255), [200, 100, 50, 255]);
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
