//! Black-and-white glass palette and ANSI -> RGB color resolution.
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, Rgb};

const fn hex(v: u32) -> Rgb {
    Rgb { r: (v >> 16) as u8, g: (v >> 8) as u8, b: v as u8 }
}

pub const FG: Rgb = hex(0xf2f2f2);
pub const BG: Rgb = hex(0x0a0a0a);
pub const CURSOR: Rgb = hex(0xffffff);
pub const SELECTION_BG: Rgb = hex(0x4a4a4a);

/// Black and white first; the eight hues are muted so programs that ask for colour stay readable
/// without breaking the monochrome glass. Index 8 (gray) keeps 4.5:1 on `BG`.
const ANSI: [Rgb; 16] = [
    hex(0x2c2c2e), hex(0xe5736f), hex(0x8fd19e), hex(0xe3c98a),
    hex(0x7aa7e0), hex(0xc4a0d8), hex(0x86c9d0), hex(0xd0d0d0),
    hex(0x8e8e93), hex(0xff8f8a), hex(0xa9e5b5), hex(0xf2dca4),
    hex(0x9cc2f2), hex(0xdcbdea), hex(0xa5dfe4), hex(0xffffff),
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

pub fn dimmed(c: Rgb) -> Rgb {
    dim(c)
}

/// Color by alacritty index (for OSC 4/10/11 replies).
pub fn by_index(i: usize) -> Rgb {
    if (16..256).contains(&i) { indexed(i as u8) } else { named(i) }
}
