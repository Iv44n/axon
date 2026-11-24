//! Palette of the `terminal.html` reference (ADR 0021) and ANSI -> RGB colour resolution.
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, Rgb};

const fn hex(v: u32) -> Rgb {
    Rgb { r: (v >> 16) as u8, g: (v >> 8) as u8, b: v as u8 }
}

/// The tokens of the `terminal.html` reference (ADR 0021): a near-black ground, a soft foreground,
/// a dim grey for secondary text, one amber accent and six clear hues.
pub const FG: Rgb = hex(0xd9d9de);
pub const BG: Rgb = hex(0x0b0b0c);
pub const CURSOR: Rgb = hex(0xf4f4f5);
pub const SELECTION_BG: Rgb = hex(0x4a3d27);
/// Secondary text of the chrome: the host in the title, the status bar, hints, settings labels.
pub const DIM: Rgb = hex(0x74747d);
/// Tertiary text: the duration of a command.
pub const FAINT: Rgb = hex(0x4a4a52);
/// The one accent: the prompt `$`, the sliders, the switch, the selected radio.
pub const ACCENT: Rgb = hex(0xffb454);
pub const GREEN: Rgb = hex(0x3ddc84);
pub const RED: Rgb = hex(0xff5d5d);
pub const MAGENTA: Rgb = hex(0xd86bdc);
pub const YELLOW: Rgb = hex(0xf6b73c);
pub const BLUE: Rgb = hex(0x6aa9ff);
pub const CYAN: Rgb = hex(0x4fd6d0);

/// The reference's hues in the ANSI slots programs use. Index 8 (gray) is a step lighter than `DIM` (`#74747d` is 4.25:1 on `BG`, fine for chrome text) so programs' grey text keeps 4.5:1.
const ANSI: [Rgb; 16] = [
    hex(0x2c2c30), RED, GREEN, YELLOW,
    BLUE, MAGENTA, CYAN, FG,
    hex(0x7e7e87), hex(0xff8a8a), hex(0x7aeaa6), hex(0xffd073),
    hex(0x9cc4ff), hex(0xe891ec), hex(0x82e8e3), hex(0xffffff),
];

fn dim(c: Rgb) -> Rgb {
    let f = |v: u8| (v as f32 * 0.66) as u8;
    Rgb { r: f(c.r), g: f(c.g), b: f(c.b) }
}

/// alacritty named colors: 0..15 ANSI, 256 fg, 257 bg, 258 cursor,
/// 259..=266 dim ANSI, 267 bright fg, 268 dim fg.
fn named(i: usize) -> Rgb {
    match i {
        0..=15 => ANSI[i],
        256 | 267 => FG,
        257 => BG,
        258 => CURSOR,
        259..=266 => dim(ANSI[i - 259]),
        268 => dim(FG),
        _ => FG,
    }
}

fn indexed(i: u8) -> Rgb {
    match i {
        0..=15 => ANSI[i as usize],
        16..=231 => {
            let n = i - 16;
            let lv = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            Rgb { r: lv(n / 36), g: lv((n / 6) % 6), b: lv(n % 6) }
        }
        _ => {
            let v = 8 + (i - 232) * 10;
            Rgb { r: v, g: v, b: v }
        }
    }
}

pub fn resolve(c: Color, colors: &Colors) -> Rgb {
    match c {
        Color::Spec(rgb) => rgb,
        Color::Named(n) => {
            let i = n as usize;
            colors[i].unwrap_or_else(|| named(i))
        }
        Color::Indexed(i) => colors[i as usize].unwrap_or_else(|| indexed(i)),
    }
}

/// `0..=1` channels of a colour, for the painters that blend in floating point.
pub fn unit(c: Rgb) -> [f32; 3] {
    [c.r as f32 / 255.0, c.g as f32 / 255.0, c.b as f32 / 255.0]
}

pub fn dimmed(c: Rgb) -> Rgb {
    dim(c)
}

/// Color by alacritty index (for OSC 4/10/11 replies).
pub fn by_index(i: usize) -> Rgb {
    if (16..256).contains(&i) { indexed(i as u8) } else { named(i) }
}
