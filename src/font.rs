//! Fonts and glyph bookkeeping shared by the renderers: fallback loading, rasterizing one glyph
//! into a cell-sized bitmap, the flat code point -> tile table and the special tiles.
//! Everything here is scaffolding over `fontdue` (own font reader: task 12).
use std::{
    cell::OnceCell,
    path::{Path, PathBuf},
};

use fontdue::{Font, FontSettings};

use crate::config::FontCfg;

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
    cfg: FontCfg,
    pub px: f32,
    pub cell_w: u32,
    pub cell_h: u32,
    pub ascent: i32,
    /// Pixels to shift primary-font glyphs right so that they stay centred in a cell wider or
    /// narrower than their advance (`letter_spacing`).
    dx: i32,
}

fn load_font(path: &Path) -> Option<Font> {
    let bytes = std::fs::read(path).ok()?;
    Font::from_bytes(bytes, FontSettings::default()).ok()
}

/// Exponent applied to glyph coverage when a tile is baked. Light text is mixed with the dark
/// background in sRGB, which makes a half-covered edge pixel look thinner than it should; raising
/// coverage to a power below 1 gives those pixels back their weight, so stems read as solid lines
/// instead of dotted ones. The same curve serves both renderers (it lives in the bitmap).
const COVERAGE_GAMMA: f32 = 0.70;

fn contrast(c: u8) -> u8 {
    match c {
        0 | 255 => c,
        _ => ((c as f32 / 255.0).powf(COVERAGE_GAMMA) * 255.0 + 0.5) as u8,
    }
}

/// Words in a file name that mark a style other than the plain one.
const STYLE_WORDS: &[&str] = &[
    "bold", "light", "thin", "medium", "black", "extra", "semi", "demi", "condensed", "italic", "oblique",
];

/// Fonts directories searched by family name.
fn font_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    vec![
        home.join(".local/share/fonts"),
        home.join(".fonts"),
        PathBuf::from("/usr/local/share/fonts"),
        PathBuf::from("/usr/share/fonts"),
    ]
}

/// Letters and digits only, lower case: "JetBrains Mono" and "JetBrainsMono[wght]" compare by prefix.
fn squash(s: &str) -> String {
    s.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect()
}

/// The font file that best matches a family name: its file name starts with the name, is not an
/// italic or oblique, and is the plain style (no Bold, Light...) with the shortest name. Looks
/// through the font directories, up to six levels down.
fn find_family(name: &str) -> Option<PathBuf> {
    find_in(font_dirs(), name)
}

// ------------------------------------------------------- reading font files --
// Just enough of the sfnt container to list fonts for the settings screen: two tables, read
// with seeks, so scanning a directory of fonts does not load them (own font reader: task 12).

fn be16(b: &[u8], at: usize) -> Option<usize> {
    Some(u16::from_be_bytes(b.get(at..at + 2)?.try_into().ok()?) as usize)
}

fn be32(b: &[u8], at: usize) -> Option<usize> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?) as usize)
}

/// `(offset, length)` of table `tag` of a TrueType or OpenType file (not a collection).
fn table(f: &mut std::fs::File, tag: &[u8; 4]) -> Option<(u64, usize)> {
    use std::io::Read;
    let mut head = [0u8; 12];
    f.read_exact(&mut head).ok()?;
    if !matches!(&head[..4], [0, 1, 0, 0] | b"OTTO" | b"true") {
        return None;
    }
    let n = be16(&head, 4)?.min(64);
    let mut dir = vec![0u8; n * 16];
    f.read_exact(&mut dir).ok()?;
    dir.chunks_exact(16).find(|e| &e[..4] == tag).map(|e| (be32(e, 8).unwrap_or(0) as u64, be32(e, 12).unwrap_or(0)))
}

/// Whether the font says it is monospaced (`post.isFixedPitch`).
fn is_monospaced(path: &Path) -> bool {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(path) else { return false };
    let Some((off, len)) = table(&mut f, b"post") else { return false };
    let mut b = [0u8; 4];
    len >= 16 && f.seek(SeekFrom::Start(off + 12)).is_ok() && f.read_exact(&mut b).is_ok() && b != [0; 4]
}

/// The family name from the `name` table (typographic family, else family), as it is written in
/// the font: "JetBrains Mono", "DejaVu Sans Mono".
fn family_name(path: &Path) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let (off, len) = table(&mut f, b"name")?;
    let mut t = vec![0u8; len.min(1 << 16)];
    f.seek(SeekFrom::Start(off)).ok()?;
    f.read_exact(&mut t).ok()?;
    let (count, strings) = (be16(&t, 2)?, be16(&t, 4)?);
    let mut best: Option<(u8, String)> = None;
    for i in 0..count {
        let r = 6 + i * 12;
        let (platform, id, n, o) = (be16(&t, r)?, be16(&t, r + 6)?, be16(&t, r + 8)?, be16(&t, r + 10)?);
        let rank = match (id, platform) {
            (16, 3) => 4,
            (1, 3) => 3,
            (16, 1) => 2,
            (1, 1) => 1,
            _ => continue,
        };
        let raw = t.get(strings + o..strings + o + n)?;
        let text = if platform == 3 {
            String::from_utf16(&raw.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect::<Vec<_>>()).ok()?
        } else {
            raw.iter().map(|&b| b as char).collect()
        };
        if best.as_ref().is_none_or(|(r, _)| rank > *r) && !text.is_empty() {
            best = Some((rank, text));
        }
    }
    best.map(|(_, n)| n)
}

/// The monospaced families installed: `(name, file)`, one file each (the plain style), by name.
pub fn monospaced_families() -> Vec<(String, PathBuf)> {
    list_monospaced(font_dirs())
}

fn list_monospaced(dirs: Vec<PathBuf>) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf, usize)> = Vec::new();
    let mut stack: Vec<(PathBuf, u8)> = dirs.into_iter().map(|d| (d, 0)).collect();
    while let Some((dir, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let path = e.path();
            if path.is_dir() {
                if depth < 6 {
                    stack.push((path, depth + 1));
                }
                continue;
            }
            let ext = path.extension().and_then(|x| x.to_str()).map(str::to_ascii_lowercase);
            if !matches!(ext.as_deref(), Some("ttf" | "otf")) {
                continue;
            }
            let stem = squash(path.file_stem().and_then(|x| x.to_str()).unwrap_or(""));
            if STYLE_WORDS.iter().any(|w| stem.contains(w)) || !is_monospaced(&path) {
                continue;
            }
            let Some(name) = family_name(&path) else { continue };
            // Several files can share a family (Regular, NL, Mono variants): keep the shortest name.
            match out.iter_mut().find(|(n, ..)| *n == name) {
                Some(old) if stem.len() < old.2 => *old = (name, path, stem.len()),
                Some(_) => {}
                None => out.push((name, path, stem.len())),
            }
        }
    }
    out.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    out.into_iter().map(|(n, p, _)| (n, p)).collect()
}

/// How `family` is written in the settings file for the font at `path`: the family name when
/// that finds the same file again, else the path.
pub fn family_setting(name: &str, path: &Path) -> String {
    match find_family(name) {
        Some(found) if found == path => name.to_string(),
        _ => path.display().to_string(),
    }
}

fn find_in(dirs: Vec<PathBuf>, name: &str) -> Option<PathBuf> {
    let want = squash(name);
    if want.is_empty() {
        return None;
    }
    let mut best: Option<((bool, usize), PathBuf)> = None;
    let mut stack: Vec<(PathBuf, u8)> = dirs.into_iter().map(|d| (d, 0)).collect();
    while let Some((dir, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let path = e.path();
            if path.is_dir() {
                if depth < 6 {
                    stack.push((path, depth + 1));
                }
                continue;
            }
            let ext = path.extension().and_then(|x| x.to_str()).map(str::to_ascii_lowercase);
            if !matches!(ext.as_deref(), Some("ttf" | "otf")) {
                continue;
            }
            let stem = squash(path.file_stem().and_then(|x| x.to_str()).unwrap_or(""));
            if !stem.starts_with(&want) || stem.contains("italic") || stem.contains("oblique") {
                continue;
            }
            let rank = (STYLE_WORDS.iter().any(|s| stem[want.len()..].contains(s)), stem.len());
            if best.as_ref().is_none_or(|(r, _)| rank < *r) {
                best = Some((rank, path));
            }
        }
    }
    best.map(|(_, p)| p)
}

pub fn expand_home(p: &str) -> PathBuf {
    match (p.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => Path::new(&home).join(rest),
        _ => PathBuf::from(p),
    }
}

/// Changes the weight of a drawn glyph from the regular 400. Heavier smears the coverage one pixel
/// to the right, by a fraction when the weight is between steps (700 is exactly one pixel, 900
/// is two); lighter raises coverage to a power, which thins the soft edges first.
fn embolden(buf: &mut [u8], w: usize, weight: u16) {
    if weight >= 400 {
        let mut left = (weight - 400) as f32 / 300.0;
        while left > 0.0 {
            let k = left.min(1.0);
            for row in buf.chunks_exact_mut(w) {
                for x in (1..w).rev() {
                    let prev = (row[x - 1] as f32 * k) as u8;
                    row[x] = row[x].max(prev);
                }
            }
            left -= 1.0;
        }
    } else {
        let power = 1.0 + (400 - weight) as f32 / 150.0;
        for c in buf.iter_mut() {
            *c = ((*c as f32 / 255.0).powf(power) * 255.0 + 0.5) as u8;
        }
    }
}

impl Fonts {
    /// Loads the configured family (a path, or a name searched in the font directories), then the
    /// built-in candidates; panics only when no monospaced font exists at all.
    pub fn new(cfg: &FontCfg) -> Self {
        let home = std::env::var("HOME").unwrap_or_default();
        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Some(fam) = &cfg.family {
            if fam.contains('/') || fam.starts_with('~') {
                candidates.push(expand_home(fam));
            } else if let Some(p) = find_family(fam) {
                candidates.push(p);
            }
            if candidates.is_empty() || load_font(&candidates[0]).is_none() {
                eprintln!("axon: config: font {fam:?} not found or not readable, using the default font");
            }
        }
        for f in ["JetBrainsMono[wght].ttf", "CascadiaMonoNF.ttf", "CascadiaMono.ttf"] {
            candidates.push(format!("{home}/.local/share/fonts/{f}").into());
        }
        for f in [
            "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf",
            "/usr/share/fonts/truetype/ubuntu/UbuntuMono-R.ttf",
        ] {
            candidates.push(f.into());
        }
        let primary = candidates
            .iter()
            .find_map(|p| load_font(p))
            .expect("no monospaced font found (set font.family in ~/.config/axon/config.toml)");

        let fallbacks = FALLBACKS
            .iter()
            .map(|&(path, ranges)| Fallback { path, ranges, font: OnceCell::new() })
            .collect();

        let mut fonts = Fonts { primary, fallbacks, cfg: cfg.clone(), px: cfg.size, cell_w: 1, cell_h: 1, ascent: 0, dx: 0 };
        fonts.set_px(cfg.size);
        fonts
    }

    /// Applies new settings that keep the same font file (everything but `family`); the tiles that
    /// were rasterized with the old ones must be thrown away by the caller.
    pub fn reconfigure(&mut self, cfg: &FontCfg) {
        self.cfg = cfg.clone();
        self.set_px(cfg.size);
    }

    /// The family the fonts were loaded for.
    pub fn family(&self) -> Option<&str> {
        self.cfg.family.as_deref()
    }

    /// Recomputes the cell metrics for a new size; no font file is read again.
    pub fn set_px(&mut self, px: f32) {
        let lm = self.primary.horizontal_line_metrics(px).expect("font has no line metrics");
        self.px = px;
        // The spacing is set at the configured size and follows zooming proportionally.
        let spacing = self.cfg.letter_spacing * px / self.cfg.size;
        let advance = self.primary.metrics('M', px).advance_width.round();
        self.cell_w = (advance + spacing.round()).max(1.0) as u32;
        self.dx = (self.cell_w as i32 - advance as i32) / 2;
        let natural = (lm.ascent - lm.descent).ceil().max(1.0);
        self.cell_h = (natural * self.cfg.line_height).ceil().max(1.0) as u32;
        self.ascent = lm.ascent.round() as i32 + (self.cell_h as i32 - natural as i32) / 2;
    }

    /// First fallback font that has `c`, loading fallbacks lazily and in order.
    fn fallback_for(&self, c: char) -> Option<&Font> {
        let cp = c as u32;
        self.fallbacks.iter().find_map(|f| {
            if !f.ranges.is_empty() && !f.ranges.iter().any(|&(lo, hi)| (lo..=hi).contains(&cp)) {
                return None;
            }
            f.font.get_or_init(|| load_font(Path::new(f.path))).as_ref().filter(|font| font.lookup_glyph_index(c) != 0)
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
        let xoff = if fb { ((w as f32 - m.advance_width) / 2.0).round() as i32 } else { self.dx };
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
        let weight = if bold && self.cfg.bold { (self.cfg.weight + 300).min(900) } else { self.cfg.weight };
        embolden(&mut buf, w as usize, weight);
        for c in buf.iter_mut() {
            *c = contrast(*c);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn fonts(f: impl FnOnce(&mut FontCfg)) -> Fonts {
        let mut cfg = FontCfg::default();
        f(&mut cfg);
        Fonts::new(&cfg)
    }

    fn ink(f: &Fonts, c: char, bold: bool) -> u32 {
        f.raster(c, bold, false).iter().map(|&b| b as u32).sum()
    }

    #[test]
    fn heavier_weights_draw_more_ink_and_the_default_is_unchanged() {
        let ink_at = |w: u16| ink(&fonts(|c| c.weight = w), 'n', false);
        let (thin, light, regular, medium, bold, black) =
            (ink_at(100), ink_at(300), ink_at(400), ink_at(500), ink_at(700), ink_at(900));
        assert!(thin < light && light < regular && regular < medium && medium < bold && bold < black,
            "{thin} {light} {regular} {medium} {bold} {black}");
        // Bold text is the regular weight plus 300: the one-pixel smear it always had.
        let f = fonts(|_| {});
        assert_eq!(f.raster('n', true, false), fonts(|c| c.weight = 700).raster('n', false, false));
    }

    #[test]
    fn bold_can_be_turned_off() {
        let f = fonts(|c| c.bold = false);
        assert_eq!(f.raster('n', true, false), f.raster('n', false, false));
    }

    #[test]
    fn letter_spacing_widens_the_cell_and_keeps_the_glyph_centred() {
        let base = fonts(|_| {});
        let wide = fonts(|c| c.letter_spacing = 4.0);
        assert_eq!(wide.cell_w, base.cell_w + 4);
        assert_eq!(wide.cell_h, base.cell_h);
        // The same pixels, moved right by half the extra width.
        let (a, b) = (base.raster('l', false, false), wide.raster('l', false, false));
        let (bw, ww) = (base.cell_w as usize, wide.cell_w as usize);
        for y in 0..base.cell_h as usize {
            for x in 0..bw {
                assert_eq!(a[y * bw + x], b[y * ww + x + 2], "({x}, {y})");
            }
        }
        assert_eq!(fonts(|c| c.letter_spacing = -1.0).cell_w, base.cell_w - 1);
    }

    #[test]
    fn spacing_follows_zoom() {
        let mut f = fonts(|c| c.letter_spacing = 2.0);
        let at15 = f.cell_w;
        f.set_px(30.0);
        let natural = fonts(|c| c.size = 30.0).cell_w;
        assert_eq!(f.cell_w, natural + 4, "at 15 px it was {at15}");
    }

    #[test]
    fn line_height_grows_the_row_and_centres_the_text() {
        let base = fonts(|_| {});
        let tall = fonts(|c| c.line_height = 1.5);
        assert!(tall.cell_h > base.cell_h);
        assert_eq!(tall.ascent - base.ascent, (tall.cell_h as i32 - base.cell_h as i32) / 2);
        assert_eq!(tall.cell_w, base.cell_w);
    }

    #[test]
    fn family_names_match_file_names_loosely() {
        let dir = std::env::temp_dir().join(format!("axon-fonts-{}", std::process::id()));
        let sub = dir.join("deep/er");
        std::fs::create_dir_all(&sub).unwrap();
        for f in ["JetBrainsMono-Bold.ttf", "JetBrainsMono-Italic[wght].ttf", "JetBrainsMonoNL-Regular.ttf", "Other.ttf"] {
            std::fs::write(dir.join(f), b"").unwrap();
        }
        std::fs::write(sub.join("JetBrainsMono[wght].ttf"), b"").unwrap();
        std::fs::write(sub.join("notes.txt"), b"").unwrap();
        let name = |n: &str| find_in(vec![dir.clone()], n).map(|p| p.file_name().unwrap().to_string_lossy().into_owned());
        assert_eq!(name("JetBrains Mono").as_deref(), Some("JetBrainsMono[wght].ttf"));
        assert_eq!(name("jetbrainsmono").as_deref(), Some("JetBrainsMono[wght].ttf"));
        assert_eq!(name("other").as_deref(), Some("Other.ttf"));
        assert_eq!(name("Nonexistent"), None);
        assert_eq!(name(""), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A minimal sfnt with a `post` and a `name` table, for the readers above.
    fn sfnt(fixed: bool, family: &str) -> Vec<u8> {
        let utf16: Vec<u8> = family.encode_utf16().flat_map(|u| u.to_be_bytes()).collect();
        let mut name = vec![0, 0, 0, 1, 0, 18, 0, 3, 0, 1, 4, 9, 0, 1, 0];
        name.pop();
        name.extend_from_slice(&[0, utf16.len() as u8, 0, 0]);
        name.extend_from_slice(&utf16);
        let mut post = vec![0u8; 16];
        post[12..16].copy_from_slice(&(fixed as u32).to_be_bytes());
        let mut out = vec![0, 1, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0];
        let base = 12 + 2 * 16;
        out.extend_from_slice(b"name");
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&(base as u32).to_be_bytes());
        out.extend_from_slice(&(name.len() as u32).to_be_bytes());
        out.extend_from_slice(b"post");
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&((base + name.len()) as u32).to_be_bytes());
        out.extend_from_slice(&(post.len() as u32).to_be_bytes());
        out.extend_from_slice(&name);
        out.extend_from_slice(&post);
        out
    }

    #[test]
    fn the_font_lister_reads_monospace_flag_and_family_name() {
        let dir = std::env::temp_dir().join(format!("axon-list-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("Fixed-Regular.ttf"), sfnt(true, "My Fixed Mono")).unwrap();
        std::fs::write(dir.join("FixedNL-Regular.ttf"), sfnt(true, "My Fixed Mono")).unwrap();
        std::fs::write(dir.join("Fixed-Bold.ttf"), sfnt(true, "My Fixed Mono")).unwrap();
        std::fs::write(dir.join("Prop-Regular.ttf"), sfnt(false, "Proportional")).unwrap();
        std::fs::write(dir.join("sub/Another.otf"), sfnt(true, "Another")).unwrap();
        std::fs::write(dir.join("Broken.ttf"), b"not a font").unwrap();
        let found = list_monospaced(vec![dir.clone()]);
        let names: Vec<&str> = found.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["Another", "My Fixed Mono"]);
        // One file per family: the shortest name, not the NL or Bold ones.
        assert!(found[1].1.ends_with("Fixed-Regular.ttf"));
        assert_eq!(family_name(&dir.join("Prop-Regular.ttf")).as_deref(), Some("Proportional"));
        assert!(!is_monospaced(&dir.join("Prop-Regular.ttf")));
        assert!(!is_monospaced(&dir.join("Broken.ttf")));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn real_system_fonts_are_listed_when_there_are_any() {
        // Whatever is installed: every entry must be a readable monospaced font with a name.
        for (name, path) in monospaced_families() {
            assert!(!name.is_empty() && is_monospaced(&path), "{name} {}", path.display());
        }
    }
}
