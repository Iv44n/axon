//! Fonts and glyph bookkeeping shared by the renderers: fallback loading, rasterizing one glyph
//! into a cell-sized bitmap, the flat code point -> tile table and the special tiles.
//! Everything here is scaffolding over `fontdue` (own font reader: task 12).
use std::{
    cell::OnceCell,
    path::{Path, PathBuf},
};

use fontdue::{Font, FontSettings};

use crate::config::{FontCfg, Smoothing};

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
    /// Where the horizontal features land at this size (hinting).
    vmap: Vmap,
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
    find_any(dirs, &[name])
}

/// A candidate file and the rank that orders it (`find_any`).
type Ranked = ((bool, bool, usize), PathBuf);

/// The font file of the first of `names` (families, in order of preference) that has one, as `find_family` ranks
/// them: the directories are walked once for all of them, not once for each.
fn find_any(dirs: Vec<PathBuf>, names: &[&str]) -> Option<PathBuf> {
    // The name asked for does not say "mono": a file that goes on with it ("Ubuntu Sans" and `UbuntuSansMono[wght]`) is
    // another family, whatever the length of its name.
    let wants: Vec<(String, bool)> = names.iter().map(|n| squash(n)).filter(|w| !w.is_empty()).map(|w| (w.clone(), !w.contains("mono"))).collect();
    let mut best: Vec<Option<Ranked>> = vec![None; wants.len()];
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
            if stem.contains("italic") || stem.contains("oblique") {
                continue;
            }
            for ((want, plain), best) in wants.iter().zip(&mut best) {
                if !stem.starts_with(want.as_str()) {
                    continue;
                }
                let rest = &stem[want.len()..];
                let rank = (STYLE_WORDS.iter().any(|s| rest.contains(s)), *plain && rest.contains("mono"), stem.len());
                if best.as_ref().is_none_or(|(r, _)| rank < *r) {
                    *best = Some((rank, path.clone()));
                }
            }
        }
    }
    best.into_iter().flatten().next().map(|(_, p)| p)
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
fn embolden(buf: &mut [u8], w: usize, weight: u16, sx: usize) {
    if weight >= 400 {
        // One pixel is `sx` samples across.
        let mut left = (weight - 400) as f32 / 300.0;
        while left > 0.0 {
            let k = left.min(1.0);
            for row in buf.chunks_exact_mut(w) {
                for x in (sx..w).rev() {
                    let prev = (row[x - sx] as f32 * k) as u8;
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

        let mut fonts = Fonts { primary, fallbacks, cfg: cfg.clone(), px: cfg.size, cell_w: 1, cell_h: 1, ascent: 0, dx: 0, vmap: Vmap::NONE };
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
        self.vmap = Vmap::of(&self.primary, px);
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

    /// Rasterizes a glyph into a `cell_w * (1|2)` x `cell_h` grey bitmap (what the wgpu path reads).
    pub fn raster(&self, c: char, bold: bool, wide: bool) -> Vec<u8> {
        let mut buf = Vec::new();
        self.raster_with(&mut buf, &self.grid(), c, bold, wide, Smoothing::Gray);
        buf.chunks_exact(3).map(|p| p[0]).collect()
    }

    /// The grid's metrics as `Small` takes them.
    pub fn grid(&self) -> Small {
        Small { px: self.px, cell_w: self.cell_w, cell_h: self.cell_h, ascent: self.ascent, dx: self.dx, vmap: self.vmap }
    }

    /// The cell metrics at `scale` times the current size (the prompt chips use 0.9): the same font
    /// and spacing rules as the grid, but the row is the font's own height, without `line_height`.
    pub fn small(&self, scale: f32) -> Small {
        let px = (self.px * scale).max(1.0);
        let lm = self.primary.horizontal_line_metrics(px).expect("font has no line metrics");
        let spacing = self.cfg.letter_spacing * px / self.cfg.size;
        let advance = self.primary.metrics('M', px).advance_width.round();
        let cell_w = (advance + spacing.round()).max(1.0) as u32;
        Small {
            px,
            cell_w,
            cell_h: (lm.ascent - lm.descent).ceil().max(1.0) as u32,
            ascent: lm.ascent.round() as i32,
            dx: (cell_w as i32 - advance as i32) / 2,
            vmap: Vmap::of(&self.primary, px),
        }
    }

    /// Like `raster` at the metrics `m`, with the configured smoothing, into `buf` (cleared and resized;
    /// nothing is allocated once it is big enough). Three bytes a pixel: the coverage of its red, green
    /// and blue subpixels (all equal for grey smoothing).
    pub fn raster_into(&self, buf: &mut Vec<u8>, m: &Small, c: char, bold: bool, wide: bool) {
        self.raster_with(buf, m, c, bold, wide, self.cfg.smoothing);
    }

    fn raster_with(&self, buf: &mut Vec<u8>, m: &Small, c: char, bold: bool, wide: bool, smoothing: Smoothing) {
        let w = (m.cell_w * if wide { 2 } else { 1 }) as usize;
        let h = m.cell_h as usize;
        buf.clear();
        buf.resize(w * h * 3, 0);

        let (font, fb) = if self.primary.lookup_glyph_index(c) != 0 {
            (&self.primary, false)
        } else if let Some(f) = self.fallback_for(c) {
            (f, true)
        } else {
            (&self.primary, false)
        };
        // The glyph is drawn `SS` times bigger than needed and averaged down: every output sample is the
        // exact area its box covers, whatever the box is (a pixel, a third of one for LCD, a row that
        // hinting stretched). `sx` samples across a pixel: 3 for LCD.
        let sx = if smoothing == Smoothing::Gray { 1 } else { 3 };
        let (g, big) = font.rasterize(c, m.px * SS as f32);
        let (gw, gh) = (g.width, g.height);
        let ss = SS as i32;
        // The glyph's left edge and the row heights of its bitmap, in 1/SS pixels from the cell origin / baseline.
        let advance = g.advance_width / SS as f32;
        let xoff = if fb { ((w as f32 - advance) / 2.0).round() as i32 } else { m.dx };
        let x0 = xoff * ss + g.xmin;
        let top = g.ymin + gh as i32;
        let hint = self.cfg.hinting && !fb;
        let mut cells = vec![0u8; w * sx * h];
        let mut strip = vec![0f32; gw];
        let mut acc = vec![0f32; w * sx];
        for y in 0..h {
            // This cell row covers heights [v, v + 1] above the baseline, which hinting maps from a
            // different span of the outline.
            let v = (m.ascent - y as i32 - 1) as f32;
            let (a, b) = if hint { (m.vmap.unmap_high(v), m.vmap.unmap_low(v + 1.0)) } else { (v, v + 1.0) };
            let (a, b) = (a * SS as f32, b * SS as f32);
            if b <= a || b <= g.ymin as f32 || a >= top as f32 {
                continue;
            }
            strip.fill(0.0);
            let (r0, r1) = ((top as f32 - b).max(0.0).floor() as usize, ((top as f32 - a).ceil() as usize).min(gh));
            for r in r0..r1 {
                // Bitmap row r covers heights [top - r - 1, top - r].
                let (lo, hi) = ((top - r as i32 - 1) as f32, (top - r as i32) as f32);
                let wgt = (hi.min(b) - lo.max(a)).max(0.0);
                if wgt > 0.0 {
                    for (acc, &cov) in strip.iter_mut().zip(&big[r * gw..][..gw]) {
                        *acc += cov as f32 * wgt;
                    }
                }
            }
            acc.fill(0.0);
            let norm = (b - a) * (SS as f32 / sx as f32);
            for (i, &sum) in strip.iter().enumerate() {
                let x = x0 + i as i32;
                if x >= 0 && x < (w * SS) as i32 {
                    acc[(x as usize * sx) / SS] += sum;
                }
            }
            for (cell, &sum) in cells[y * w * sx..][..w * sx].iter_mut().zip(&acc) {
                *cell = (sum / norm).round().min(255.0) as u8;
            }
        }
        let weight = if bold && self.cfg.bold { (self.cfg.weight + 300).min(900) } else { self.cfg.weight };
        embolden(&mut cells, w * sx, weight, sx);
        for y in 0..h {
            let src = &cells[y * w * sx..][..w * sx];
            let dst = &mut buf[y * w * 3..][..w * 3];
            if sx == 1 {
                for (x, &v) in src.iter().enumerate() {
                    dst[x * 3..][..3].fill(contrast(v));
                }
            } else {
                for x in 0..w {
                    for ch in 0..3 {
                        // FreeType's default LCD filter over the five nearest subpixels.
                        let at = (x * 3 + ch) as i32;
                        let mut sum = 0u32;
                        for (t, k) in LCD_FIR.iter().enumerate() {
                            let i = at + t as i32 - 2;
                            if i >= 0 && (i as usize) < src.len() {
                                sum += src[i as usize] as u32 * k;
                            }
                        }
                        let v = contrast((sum / 256).min(255) as u8);
                        dst[x * 3 + if smoothing == Smoothing::Bgr { 2 - ch } else { ch }] = v;
                    }
                }
            }
        }
    }
}

/// How many times bigger than the target size a glyph is rasterized before it is averaged down.
const SS: usize = 6;

/// FreeType's default LCD filter (`FT_LCD_FILTER_DEFAULT`), the weights of five neighbouring subpixels.
const LCD_FIR: [u32; 5] = [0x08, 0x4D, 0x56, 0x4D, 0x08];

/// Cell metrics at some size: what `Fonts` keeps for the grid, for text at another size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Small {
    pub px: f32,
    pub cell_w: u32,
    pub cell_h: u32,
    pub ascent: i32,
    dx: i32,
    vmap: Vmap,
}

/// Where the horizontal features of the font land at a size, and where they are snapped to: the
/// baseline, the x-height, the cap height, the ascender and the descender. Heights in pixels above
/// the baseline. This is what FreeType's "slight" hinting does (vertical only, so the shapes and
/// advances do not change), done by hand.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vmap {
    n: usize,
    from: [f32; Vmap::MAX],
    to: [f32; Vmap::MAX],
}

impl Vmap {
    const MAX: usize = 10;

    /// The identity: nothing is snapped.
    pub const NONE: Vmap = Vmap { n: 0, from: [0.0; Vmap::MAX], to: [0.0; Vmap::MAX] };

    /// Measured on flat and round letters of `font` at `px`: a round letter overshoots a flat one by a
    /// fraction of a pixel, and both land on the same whole row.
    pub fn of(font: &Font, px: f32) -> Vmap {
        let top = |c| font.metrics(c, px).bounds;
        let (bot, hi) = (|c| top(c).ymin, |c| top(c).ymin + top(c).height);
        // (lowest, highest) source height of each zone, snapped to one whole row.
        let zones = [
            (bot('p'), bot('p')),
            (bot('o').min(0.0), 0.0f32),
            (hi('x').min(hi('o')), hi('x').max(hi('o'))),
            (hi('H').min(hi('O')), hi('H').max(hi('O'))),
            (hi('l'), hi('l')),
        ];
        let mut v = Vmap::NONE;
        let mut last = f32::NEG_INFINITY;
        for (lo, up) in zones {
            // The flat edge decides the row; a zone that would cross or touch the previous one is skipped.
            // The row a zone lands on is its flat edge, rounded; the zone is widened to the grid the glyph is
            // drawn on (`SS` samples a pixel), so a sample row cut by the edge never leaks into the row above.
            let target = if up == 0.0 { 0.0 } else { lo.round() };
            let g = SS as f32;
            let (lo, up) = ((lo * g - 1e-3).floor() / g, (up * g + 1e-3).ceil() / g);
            if lo <= last || v.n + 2 > Vmap::MAX || !(lo.is_finite() && up.is_finite()) {
                continue;
            }
            v.from[v.n] = lo;
            v.to[v.n] = target;
            v.from[v.n + 1] = up;
            v.to[v.n + 1] = target;
            v.n += 2;
            last = up;
        }
        v
    }

    /// Hinted height of source height `y`.
    #[cfg(test)]
    fn map(&self, y: f32) -> f32 {
        if self.n == 0 {
            return y;
        }
        if y <= self.from[0] {
            return y + self.to[0] - self.from[0];
        }
        let n = self.n;
        if y >= self.from[n - 1] {
            return y + self.to[n - 1] - self.from[n - 1];
        }
        let i = (1..n).find(|&i| y < self.from[i]).unwrap_or(n - 1);
        let (f0, f1, t0, t1) = (self.from[i - 1], self.from[i], self.to[i - 1], self.to[i]);
        if f1 <= f0 { t0 } else { t0 + (y - f0) / (f1 - f0) * (t1 - t0) }
    }

    /// The highest source height that lands at or below hinted height `v`: the top of a collapsed zone
    /// when `v` is its row, so a round letter's overshoot is left out of the row above it.
    pub fn unmap_high(&self, v: f32) -> f32 {
        let n = self.n;
        if n == 0 {
            return v;
        }
        if v < self.to[0] {
            return self.from[0] + v - self.to[0];
        }
        let i = (0..n).rev().find(|&i| self.to[i] <= v).unwrap_or(0);
        if i == n - 1 {
            return self.from[i] + v - self.to[i];
        }
        let (t0, t1) = (self.to[i], self.to[i + 1]);
        self.from[i] + (v - t0) / (t1 - t0) * (self.from[i + 1] - self.from[i])
    }

    /// The lowest source height that lands at or above hinted height `v`.
    pub fn unmap_low(&self, v: f32) -> f32 {
        let n = self.n;
        if n == 0 {
            return v;
        }
        let Some(i) = (0..n).find(|&i| self.to[i] >= v) else {
            return self.from[n - 1] + v - self.to[n - 1];
        };
        if i == 0 {
            return self.from[0] + v - self.to[0];
        }
        let (t0, t1) = (self.to[i - 1], self.to[i]);
        self.from[i - 1] + (v - t0) / (t1 - t0) * (self.from[i] - self.from[i - 1])
    }
}

// -------------------------------------------------------------- ui font --

/// The proportional font of the interface around the terminal: the title, the panel of windows, the tabs, the
/// settings panel and the overlays (the reference's `--ui`, Geist), where the grid's monospace font reads as code.
/// The window holds it from its first frame (`Soft`); whoever else asks for it while it lives gets the same one.
/// It is read through `cmap::narrow`, so only the outlines of the characters an interface draws are parsed:
/// ~2 MB for an Inter-like font instead of ~8.5.
pub struct UiFont {
    font: Font,
}

/// The synthetic weight of the UI font's bold (a heading, a selected label).
pub const BOLD: u16 = 640;
/// Its medium: the reference's `font-weight: 500` (the title's name, a group's name, a card's title).
pub const MEDIUM: u16 = 500;

thread_local! {
    /// The font the holders of `UiFont::shared` have now: not a copy of it, only a way to find the one they hold.
    static UI_FONT: std::cell::RefCell<std::rc::Weak<UiFont>> = const { std::cell::RefCell::new(std::rc::Weak::new()) };
}

/// Gives the heap pages that nothing uses any more back to the system. A dropped font is thousands of small
/// allocations: freed, they stay in the process (the resident size does not fall) until this is asked.
pub fn release_memory() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        unsafe extern "C" {
            fn malloc_trim(pad: usize) -> std::ffi::c_int;
        }
        // SAFETY: `malloc_trim` is glibc's and takes no pointers; it only returns free pages of the heap.
        unsafe { malloc_trim(0) };
    }
}

/// Families tried in order: the reference's own (Geist), the two that look most like it (Inter, and GNOME's Adwaita
/// Sans, drawn from it), then the desktop's and other common good ones. Not Cantarell: its file is CFF2, which
/// `fontdue` parses but draws nothing from.
const UI_FAMILIES: [&str; 7] = ["Geist", "Inter", "Adwaita Sans", "Ubuntu Sans", "Noto Sans", "Open Sans", "DejaVu Sans"];

/// The UI font in `path`, with its character map narrowed to what an interface draws (`cmap::narrow`), or none
/// when it cannot be read or draws nothing (an outline format `fontdue` does not draw).
fn load_ui_font(path: &Path) -> Option<Font> {
    let bytes = std::fs::read(path).ok()?;
    let bytes = crate::cmap::narrow(&bytes, crate::cmap::ui_char).unwrap_or(bytes);
    let font = Font::from_bytes(bytes, FontSettings { load_substitutions: false, ..FontSettings::default() }).ok()?;
    font.rasterize('H', 12.0).1.iter().any(|&c| c != 0).then_some(font)
}

impl UiFont {
    /// The font a holder already has, else the first of `UI_FAMILIES` installed (a regular, upright file) read from its
    /// file, or none. The settings panel and the overlays hold the same one: it is parsed once, and the font
    /// directories are walked only when it is not.
    pub fn shared() -> Option<std::rc::Rc<UiFont>> {
        if let Some(f) = UI_FONT.with(|w| w.borrow().upgrade()) {
            return Some(f);
        }
        let font = find_any(font_dirs(), &UI_FAMILIES).and_then(|p| load_ui_font(&p)).map(|font| std::rc::Rc::new(UiFont { font }));
        // The file's bytes, their narrowed copy and the parser's scratch are free now: give their pages back.
        release_memory();
        let font = font?;
        UI_FONT.with(|w| *w.borrow_mut() = std::rc::Rc::downgrade(&font));
        Some(font)
    }

    /// Width in pixels of `s` at `px`.
    pub fn width(&self, s: &str, px: f32) -> i32 {
        self.pen(s, px).last().unwrap_or(0.0).round() as i32
    }

    /// The pen position before each character of `s`, and after the last, in pixels.
    fn pen<'a>(&'a self, s: &'a str, px: f32) -> impl Iterator<Item = f32> + 'a {
        let mut x = 0.0f32;
        std::iter::once(0.0).chain(s.chars().map(move |c| {
            x += self.font.metrics(c, px).advance_width;
            x
        }))
    }

    /// Height of a capital letter at `px`, to centre text on its middle.
    pub fn cap_height(&self, px: f32) -> f32 {
        let b = self.font.metrics('H', px).bounds;
        b.ymin + b.height
    }

    /// Calls `put(x, y, coverage)` for every inked pixel of `s` at `px`, its pen starting at `(x, baseline)`.
    /// A `weight` above 400 thickens the strokes by synthesis (the font file's variable axis is not read): 500 is
    /// the reference's medium labels, `BOLD` its headings.
    pub fn draw(&self, s: &str, px: f32, x: i32, baseline: i32, weight: u16, mut put: impl FnMut(i32, i32, u8)) {
        for (c, pen) in s.chars().zip(self.pen(s, px)) {
            if c == ' ' {
                continue;
            }
            let (g, mut bmp) = self.font.rasterize(c, px);
            let mut w = g.width;
            if weight > 400 && w > 0 {
                // One extra column on the right, then the smear.
                let mut wide = vec![0u8; (w + 1) * g.height];
                for (r, row) in bmp.chunks(w).enumerate() {
                    wide[r * (w + 1)..][..w].copy_from_slice(row);
                }
                embolden(&mut wide, w + 1, weight, 1);
                (bmp, w) = (wide, w + 1);
            }
            let gx = x + pen.round() as i32 + g.xmin;
            for (r, row) in bmp.chunks(w.max(1)).enumerate().take(g.height) {
                let gy = baseline - g.ymin - g.height as i32 + r as i32;
                for (col, &cov) in row.iter().enumerate() {
                    if cov != 0 {
                        put(gx + col as i32, gy, contrast(cov));
                    }
                }
            }
        }
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

    /// Rows of the glyph's bitmap (grey) as the coverage of each row's strongest pixel, top to bottom.
    fn row_peaks(f: &Fonts, c: char) -> Vec<u8> {
        let w = f.cell_w as usize;
        f.raster(c, false, false).chunks(w).map(|r| *r.iter().max().unwrap()).collect()
    }

    #[test]
    fn hinting_puts_flat_tops_and_the_baseline_on_whole_rows() {
        for px in [13.0, 14.0, 15.0, 16.0, 18.0] {
            let hinted = fonts(|c| c.size = px);
            let plain = fonts(|c| {
                c.size = px;
                c.hinting = false;
            });
            for ch in ['x', 'H', 'z'] {
                let (h, p) = (row_peaks(&hinted, ch), row_peaks(&plain, ch));
                let first = |v: &[u8]| v.iter().position(|&m| m > 0).unwrap();
                let last = |v: &[u8]| v.iter().rposition(|&m| m > 0).unwrap();
                // The top and bottom rows of a flat letter are as strong as the middle of a stem...
                assert!(h[first(&h)] >= 200 && h[last(&h)] >= 200, "{ch} at {px}: top {} bottom {}", h[first(&h)], h[last(&h)]);
                // ...and hinting never makes a letter taller or shorter by more than a row.
                assert!((last(&h) - first(&h)).abs_diff(last(&p) - first(&p)) <= 1, "{ch} at {px}");
            }
        }
    }

    #[test]
    fn hinting_leaves_the_ink_about_the_same() {
        let sum = |f: &Fonts, c| f.raster(c, false, false).iter().map(|&v| v as u32).sum::<u32>() as f32;
        let hinted = fonts(|_| {});
        let plain = fonts(|c| c.hinting = false);
        for c in ['a', 'e', 'g', 'W', '8', 'f'] {
            let (a, b) = (sum(&hinted, c), sum(&plain, c));
            assert!((a / b - 1.0).abs() < 0.12, "{c}: {a} against {b}");
        }
    }

    #[test]
    fn lcd_text_has_a_coverage_per_stripe_and_grey_text_one() {
        let at = |s| {
            let f = fonts(|c| c.smoothing = s);
            let mut buf = Vec::new();
            f.raster_into(&mut buf, &f.grid(), 'l', false, false);
            buf
        };
        let (gray, rgb, bgr) = (at(Smoothing::Gray), at(Smoothing::Rgb), at(Smoothing::Bgr));
        assert!(gray.chunks(3).all(|p| p[0] == p[1] && p[1] == p[2]), "grey is one value");
        assert!(rgb.chunks(3).any(|p| p[0] != p[2]), "LCD differs across the stripes of an edge");
        // The same ink: BGR is RGB with the stripes read the other way round.
        assert!(rgb.chunks(3).zip(bgr.chunks(3)).all(|(a, b)| a[0] == b[2] && a[1] == b[1] && a[2] == b[0]));
        // And it has as much ink as grey text, give or take the filter.
        let ink = |v: &[u8]| v.iter().map(|&c| c as f32).sum::<f32>();
        assert!((ink(&rgb) / ink(&gray) - 1.0).abs() < 0.1, "{} against {}", ink(&rgb), ink(&gray));
    }

    #[test]
    fn the_vertical_map_is_monotone_and_inverts() {
        let f = fonts(|_| {});
        let v = f.grid().vmap;
        let mut last = f32::NEG_INFINITY;
        for i in -40..160 {
            let y = i as f32 / 10.0;
            let m = v.map(y);
            assert!(m >= last - 1e-4, "map goes down at {y}");
            last = m;
        }
        // The rows of a hinted size land on whole pixels: the baseline, the x-height, the cap height.
        for y in [0.0, 8.37, 10.95] {
            let m = v.map(y);
            assert!((m - m.round()).abs() < 1e-3, "{y} maps to {m}");
        }
        for target in [-2.0f32, 0.5, 3.0, 7.7, 9.5] {
            assert!((v.map(v.unmap_low(target)) - target).abs() < 1e-3 || (v.map(v.unmap_high(target)) - target).abs() < 1e-3);
        }
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
        // A family and its monospaced sibling, whose names are as long: the proportional one, unless the mono one is asked for.
        let ub = dir.join("ubuntu");
        std::fs::create_dir_all(&ub).unwrap();
        for f in ["UbuntuSansMono[wght].ttf", "UbuntuSans[wdth,wght].ttf"] {
            std::fs::write(ub.join(f), b"").unwrap();
        }
        let found = |n: &str| find_in(vec![ub.clone()], n).map(|p| p.file_name().unwrap().to_string_lossy().into_owned());
        assert_eq!(found("Ubuntu Sans").as_deref(), Some("UbuntuSans[wdth,wght].ttf"));
        assert_eq!(found("Ubuntu Sans Mono").as_deref(), Some("UbuntuSansMono[wght].ttf"));
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
