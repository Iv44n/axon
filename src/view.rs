//! Grid -> cells: reads the terminal and tells a backend which rows to paint.
//! Backend-agnostic: the GPU renderer and the CPU renderer both implement `Backend`.
use alacritty_terminal::{
    Term,
    event::EventListener,
    grid::Dimensions,
    index::{Column, Line, Point},
    selection::SelectionRange,
    term::{TermDamage, TermMode, cell::Flags},
    vte::ansi::{Color, CursorShape, NamedColor},
};

use crate::cell::{Cell, F_BAR, F_BEAM, F_BOX, F_STRIKE, F_UNDER, Glyphs, T_BLANK, pack};
use crate::palette;

/// What the grid builder needs from a renderer, besides the glyph tiles.
pub trait Backend: Glyphs {
    /// Starts a frame for a `rows` x `cols` grid. Returns true when every row has to be written:
    /// first frame, the grid size changed, or the glyph tiles were reassigned.
    fn begin_frame(&mut self, rows: usize, cols: usize) -> bool;
    /// Receives `count` consecutive rows starting at `first`: `count * cols` cells, row-major.
    fn put_rows(&mut self, first: usize, count: usize, cells: &[Cell]);
}

/// Frame-to-frame state that `Term`'s damage does not cover, and the scratch rows are built in.
pub struct View {
    dirty: Vec<bool>,
    last_cursor: (Point, CursorShape, bool),
    last_sel: Option<SelectionRange>,
    /// Scratch for the rows being built; dead once they are handed to the backend.
    stage: Vec<Cell>,
}

impl View {
    pub fn new() -> View {
        View {
            dirty: Vec::new(),
            last_cursor: (Point::default(), CursorShape::Block, true),
            last_sel: None,
            stage: Vec::new(),
        }
    }

    /// Brings the backend up to date: only rows that changed since the last frame are rebuilt.
    pub fn build<L: EventListener>(&mut self, term: &mut Term<L>, focused: bool, out: &mut impl Backend) {
        let (rows, cols) = (term.screen_lines(), term.columns());
        let mut full = out.begin_frame(rows, cols);
        self.dirty.clear();
        self.dirty.resize(rows, false);

        // Terminal damage must be read and reset on every frame.
        match term.damage() {
            TermDamage::Full => full = true,
            TermDamage::Partial(lines) => {
                for d in lines {
                    if let Some(f) = self.dirty.get_mut(d.line) {
                        *f = true;
                    }
                }
            }
        }
        term.reset_damage();

        // The cursor shape and the selection are not part of `Term`'s damage: compare them
        // with the previous frame and rebuild the rows they touch, old and new.
        let content = term.renderable_content();
        let offset = content.display_offset;
        let cur = content.cursor;
        let shown = content.mode.contains(TermMode::SHOW_CURSOR) && offset == 0;
        let shape = if !focused { CursorShape::HollowBlock } else { cur.shape };
        let cursor = (cur.point, shape, shown);
        let selection = content.selection;

        // Scrolled back in history, every row moves: rebuild everything (rare).
        full |= offset != 0;
        if full {
            self.dirty.fill(true);
        } else {
            let dirty = &mut self.dirty;
            let mut mark = |from: i32, to: i32| {
                for r in from.max(0)..=to.min(rows as i32 - 1) {
                    dirty[r as usize] = true;
                }
            };
            if cursor != self.last_cursor {
                mark(self.last_cursor.0.line.0, self.last_cursor.0.line.0);
                mark(cur.point.line.0, cur.point.line.0);
            }
            if selection != self.last_sel {
                for s in [self.last_sel, selection].into_iter().flatten() {
                    mark(s.start.line.0, s.end.line.0);
                }
            }
        }
        self.last_cursor = cursor;
        self.last_sel = selection;

        // Write each run of consecutive damaged rows with one upload.
        let mut row = 0;
        while row < rows {
            if !self.dirty[row] {
                row += 1;
                continue;
            }
            let first = row;
            while row < rows && self.dirty[row] {
                row += 1;
            }
            self.stage.clear();
            self.stage.resize((row - first) * cols, Cell::BLANK);
            build_rows(out, term, first, &mut self.stage, focused);
            out.put_rows(first, row - first, &self.stage);
        }
    }
}

/// Fills `cells` with the rows starting at `first`: row-major, `term.columns()` cells per row.
/// The slice arrives zeroed (blank), so only non-blank cells are written.
fn build_rows<L: EventListener>(g: &mut impl Glyphs, term: &Term<L>, first: usize, cells: &mut [Cell], focused: bool) {
    let content = term.renderable_content();
    let colors = content.colors;
    let selection = content.selection;
    let offset = content.display_offset as i32;
    let cur = content.cursor;
    let show_cursor = content.mode.contains(TermMode::SHOW_CURSOR) && offset == 0;
    let shape = if !focused { CursorShape::HollowBlock } else { cur.shape };
    let cursor_color = palette::CURSOR;
    let dark = palette::BG;
    let default_bg = palette::resolve(Color::Named(NamedColor::Background), colors);
    let transparent = pack(0, 0, 0, 0);
    let cols = term.columns();

    for (n, out) in cells.chunks_exact_mut(cols).enumerate() {
        let line = Line((first + n) as i32 - offset);
        let row = &term.grid()[line];
        let mut right_half: Option<u32> = None;

        for (col, out) in out.iter_mut().enumerate() {
            let cell = &row[Column(col)];
            let point = Point::new(line, Column(col));
            let flags = cell.flags;

            let mut fg_c = cell.fg;
            if flags.contains(Flags::BOLD) {
                if let Color::Named(n) = fg_c {
                    if (n as usize) < 8 {
                        fg_c = Color::Indexed(n as u8 + 8);
                    }
                }
            }
            let mut fg = palette::resolve(fg_c, colors);
            let mut bg = palette::resolve(cell.bg, colors);
            // A cell painted in the terminal's own background colour (themes that set it explicitly,
            // `\e[49m`'s look-alikes) is the background: glass, not an opaque block on the glass.
            let mut bg_default = matches!(cell.bg, Color::Named(NamedColor::Background)) || bg == default_bg;
            if flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
                bg_default = false;
            }
            if flags.contains(Flags::DIM) {
                fg = palette::dimmed(fg);
            }
            if selection.is_some_and(|s| s.contains(point)) {
                bg = palette::SELECTION_BG;
                bg_default = false;
            }
            let is_cursor = show_cursor && point == cur.point;
            if is_cursor && shape == CursorShape::Block {
                fg = dark;
                bg = cursor_color;
                bg_default = false;
            }

            // Right half of a wide character: the next tile.
            let tile = if flags.contains(Flags::WIDE_CHAR_SPACER) {
                right_half.take().unwrap_or(T_BLANK)
            } else if flags.contains(Flags::HIDDEN) || flags.contains(Flags::LEADING_WIDE_CHAR_SPACER) {
                T_BLANK
            } else {
                let wide = flags.contains(Flags::WIDE_CHAR);
                let t = g.glyph(cell.c, flags.contains(Flags::BOLD), wide);
                if wide && t != T_BLANK {
                    right_half = Some(t + 1);
                }
                t
            };

            // Underline, strikeout and the non-block cursor are flags the shader draws.
            let mut tf = tile;
            if flags.intersects(Flags::ALL_UNDERLINES) {
                tf |= F_UNDER;
            }
            if flags.contains(Flags::STRIKEOUT) {
                tf |= F_STRIKE;
            }
            if is_cursor {
                tf |= match shape {
                    CursorShape::Beam => F_BEAM,
                    CursorShape::Underline => F_BAR,
                    CursorShape::HollowBlock => F_BOX,
                    _ => 0,
                };
            }
            *out = Cell {
                tf,
                fg: pack(fg.r, fg.g, fg.b, 255),
                bg: if bg_default { transparent } else { pack(bg.r, bg.g, bg.b, 255) },
            };
        }
    }
}
