//! The cell format every renderer reads, and the tile contract between the grid and the glyphs.

/// Special tiles; glyph tiles start at `T_FIRST_GLYPH`, so tile 0 is never a real glyph.
pub const T_BLANK: u32 = 0;
pub const T_BEAM: u32 = 1;
pub const T_BAR: u32 = 2;
pub const T_BOX: u32 = 3;
pub const T_UNDER: u32 = 4;
pub const T_STRIKE: u32 = 5;
pub const T_FIRST_GLYPH: u32 = 6;
/// The tile index lives in the low 16 bits of `Cell::tf`.
pub const MAX_TILES: u32 = 1 << 16;

// Flags in the high 16 bits of `Cell::tf`; the renderer draws them (shader.wgsl, soft.rs).
pub const F_UNDER: u32 = 1 << 16;
pub const F_STRIKE: u32 = 1 << 17;
/// Cursor shape: a 2-bit field at bits 18-19.
pub const F_BEAM: u32 = 1 << 18;
pub const F_BAR: u32 = 2 << 18;
pub const F_BOX: u32 = 3 << 18;

/// One terminal cell as a renderer reads it. A cell buffer is a flat array indexed by
/// `row * cols + col`: the position is the index, so it is not stored.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Cell {
    /// Tile in the low 16 bits, `F_*` flags above.
    pub tf: u32,
    /// Packed RGBA (`pack`).
    pub fg: u32,
    pub bg: u32,
}

impl Cell {
    /// Nothing to draw: blank tile, no flags, transparent background.
    pub const BLANK: Cell = Cell { tf: 0, fg: 0, bg: 0 };
}

pub fn pack(r: u8, g: u8, b: u8, a: u8) -> u32 {
    r as u32 | (g as u32) << 8 | (b as u32) << 16 | (a as u32) << 24
}

/// What the grid builder needs from a renderer: the tile of a glyph (the right half of a wide
/// glyph is the next tile).
pub trait Glyphs {
    fn glyph(&mut self, c: char, bold: bool, wide: bool) -> u32;
}
