//! Fonts and glyph bookkeeping shared by the renderers: fallback loading, rasterizing one glyph
//! into a cell-sized bitmap, the flat code point -> tile table and the special tiles.
//! Everything here is scaffolding over `fontdue` (own font reader: task 12).
use std::cell::OnceCell;

use fontdue::{Font, FontSettings};

// ------------------------------------------------------------------ fonts --

/// Fallback font, parsed on first use. fontdue parses every glyph eagerly, so a CJK font costs
/// ~300 MB and ~600 ms; `ranges` (inclusive code points, empty = any) keeps it from being
/// consulted, and therefore loaded, for characters it cannot help with.
struct Fallback {
    path: &'static str,
    ranges: &'static [(u32, u32)],
    font: OnceCell<Option<Font>>,
}

const CJK: &[(u32, u32)] = &[
    (0x2E80, 0x9FFF),   // radicals, CJK symbols, kana, bopomofo, enclosed forms, ideographs
    (0xAC00, 0xD7FF),   // hangul syllables
    (0xF900, 0xFAFF),   // compatibility ideographs
    (0xFE30, 0xFE4F),   // compatibility forms
    (0xFF00, 0xFFEF),   // half/full-width forms
    (0x20000, 0x3FFFF), // ideograph extensions
];

const FALLBACKS: &[(&str, &[(u32, u32)])] = &[
    ("/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf", &[]),
    ("/usr/share/fonts/truetype/noto/NotoSansSymbols2-Regular.ttf", &[]),
    ("/usr/share/fonts/truetype/noto/NotoSansSymbols-Regular.ttf", &[]),
    ("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", CJK),
];

pub struct Fonts {
    primary: Font,
    fallbacks: Vec<Fallback>,
    pub px: f32,
    pub cell_w: u32,
    pub cell_h: u32,
    pub ascent: i32,
}

fn load_font(path: &str) -> Option<Font> {
    let bytes = std::fs::read(path).ok()?;
    Font::from_bytes(bytes, FontSettings::default()).ok()
}

impl Fonts {
    pub fn new(px: f32) -> Self {
        let home = std::env::var("HOME").unwrap_or_default();
        let mut candidates: Vec<String> = Vec::new();
        if let Ok(p) = std::env::var("AXON_FONT") {
            candidates.push(p);
        }
        for f in ["CascadiaMonoNF.ttf", "JetBrainsMono[wght].ttf", "CascadiaMono.ttf"] {
            candidates.push(format!("{home}/.local/share/fonts/{f}"));
        }
        for f in [
            "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf",
            "/usr/share/fonts/truetype/ubuntu/UbuntuMono-R.ttf",
        ] {
            candidates.push(f.to_string());
        }
        let primary = candidates
            .iter()
            .find_map(|p| load_font(p))
            .expect("no monospaced font found (set AXON_FONT=/path/to/font.ttf)");

        let fallbacks = FALLBACKS
            .iter()
            .map(|&(path, ranges)| Fallback { path, ranges, font: OnceCell::new() })
            .collect();

        let mut fonts = Fonts { primary, fallbacks, px, cell_w: 1, cell_h: 1, ascent: 0 };
        fonts.set_px(px);
        fonts
    }

    /// Recomputes the cell metrics for a new size; no font file is read again.
    pub fn set_px(&mut self, px: f32) {
        let lm = self.primary.horizontal_line_metrics(px).expect("font has no line metrics");
        self.px = px;
        self.cell_w = self.primary.metrics('M', px).advance_width.round().max(1.0) as u32;
        self.cell_h = (lm.ascent - lm.descent).ceil().max(1.0) as u32;
        self.ascent = lm.ascent.round() as i32;
    }

    /// First fallback font that has `c`, loading fallbacks lazily and in order.
    fn fallback_for(&self, c: char) -> Option<&Font> {
        let cp = c as u32;
        self.fallbacks.iter().find_map(|f| {
            if !f.ranges.is_empty() && !f.ranges.iter().any(|&(lo, hi)| (lo..=hi).contains(&cp)) {
                return None;
            }
            f.font.get_or_init(|| load_font(f.path)).as_ref().filter(|font| font.lookup_glyph_index(c) != 0)
        })
    }

    /// Rasterizes a glyph into a `cell_w * (1|2)` x `cell_h` bitmap.
    pub fn raster(&self, c: char, bold: bool, wide: bool) -> Vec<u8> {
        let w = (self.cell_w * if wide { 2 } else { 1 }) as i32;
        let h = self.cell_h as i32;
        let mut buf = vec![0u8; (w * h) as usize];

        let (font, fb) = if self.primary.lookup_glyph_index(c) != 0 {
            (&self.primary, false)
        } else if let Some(f) = self.fallback_for(c) {
            (f, true)
        } else {
            (&self.primary, false)
        };
        let (m, bmp) = font.rasterize(c, self.px);
        let xoff = if fb { ((w as f32 - m.advance_width) / 2.0).round() as i32 } else { 0 };
        for r in 0..m.height as i32 {
            let y = self.ascent - m.ymin - m.height as i32 + r;
            if y < 0 || y >= h {
                continue;
            }
            for col in 0..m.width as i32 {
                let x = m.xmin + col + xoff;
                if x < 0 || x >= w {
                    continue;
                }
                buf[(y * w + x) as usize] = bmp[(r * m.width as i32 + col) as usize];
            }
        }
        if bold {
            for y in 0..h {
                for x in (1..w).rev() {
                    let i = (y * w + x) as usize;
                    buf[i] = buf[i].max(buf[i - 1]);
                }
            }
        }
        buf
    }
}

// ------------------------------------------------------------ glyph table --

/// Tile of each baked (code point, style): a two-level flat table, no hashing. Code point `cp`
/// lives in page `cp >> 8`, slot `(cp & 255) << 2 | style`. A tile of 0 means "not baked":
/// glyph tiles start at `T_FIRST_GLYPH`, so 0 is never a real glyph.
pub struct GlyphTable {
    dir: Vec<u16>,
    pages: Vec<[u32; 1024]>,
}

impl GlyphTable {
    pub fn new() -> Self {
        GlyphTable { dir: vec![0; 0x1100], pages: Vec::new() }
    }

    #[inline]
    pub fn get(&self, c: char, style: u8) -> u32 {
        let cp = c as usize;
        match self.dir[cp >> 8] {
            0 => 0,
            p => self.pages[p as usize - 1][(cp & 255) << 2 | style as usize],
        }
    }

    pub fn set(&mut self, c: char, style: u8, tile: u32) {
        let cp = c as usize;
        if self.dir[cp >> 8] == 0 {
            self.pages.push([0; 1024]);
            self.dir[cp >> 8] = self.pages.len() as u16;
        }
        let page = self.dir[cp >> 8] as usize - 1;
        self.pages[page][(cp & 255) << 2 | style as usize] = tile;
    }

    pub fn clear(&mut self) {
        self.dir.fill(0);
        self.pages.clear();
    }
}

// ----------------------------------------------------------- special tiles --

/// Bitmaps of the special tiles `T_BLANK..=T_STRIKE` (`cell_w * cell_h` bytes each, 0 or 255):
/// the cursor shapes and the underline and strikeout lines.
pub fn special_tiles(cell_w: u32, cell_h: u32, px: f32, ascent: i32) -> [Vec<u8>; 6] {
    let (w, h) = (cell_w as usize, cell_h as usize);
    let t = ((px / 14.0).round() as usize).max(1);
    let base = ascent as usize;
    let mk = |f: &dyn Fn(usize, usize) -> bool| -> Vec<u8> {
        let mut v = vec![0u8; w * h];
        for y in 0..h {
            for x in 0..w {
                if f(x, y) {
                    v[y * w + x] = 255;
                }
            }
        }
        v
    };
    [
        mk(&|_, _| false),
        mk(&|x, _| x < t.max(2)),
        mk(&|_, y| y + t.max(2) >= h),
        mk(&|x, y| x < t || y < t || x + t >= w || y + t >= h),
        mk(&|_, y| y >= (base + 1).min(h - t) && y < (base + 1).min(h - t) + t),
        mk(&|_, y| y >= h * 55 / 100 && y < h * 55 / 100 + t),
    ]
}
