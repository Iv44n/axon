//! Neon palette and ANSI -> RGB color resolution.
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, Rgb};

const fn hex(v: u32) -> Rgb {
    Rgb { r: (v >> 16) as u8, g: (v >> 8) as u8, b: v as u8 }
}

pub const FG: Rgb = hex(0xcfd9f5);
pub const BG: Rgb = hex(0x070a13);
pub const CURSOR: Rgb = hex(0x19e3ff);
pub const SELECTION_BG: Rgb = hex(0x143a56);

const ANSI: [Rgb; 16] = [
    hex(0x11162a), hex(0xff4d6d), hex(0x38f2a0), hex(0xffd36b),
    hex(0x5b8cff), hex(0xc77dff), hex(0x19e3ff), hex(0xc9d6f2),
    hex(0x4a5578), hex(0xff7a93), hex(0x7dffc4), hex(0xffe39a),
    hex(0x8fb0ff), hex(0xdba6ff), hex(0x7df2ff), hex(0xf2f6ff),
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
