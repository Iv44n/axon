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
    /// The columns of the last build: a different width (a pane resized) rebuilds every row.
    cols: usize,
    /// Scratch for the rows being built; dead once they are handed to the backend.
    stage: Vec<Cell>,
    /// Rows the caller wants repainted at the next frame (something is drawn over them).
    wanted: Vec<usize>,
}

impl View {
    pub fn new() -> View {
        View {
            dirty: Vec::new(),
            last_cursor: (Point::default(), CursorShape::Block, true),
            last_sel: None,
            cols: 0,
            stage: Vec::new(),
            wanted: Vec::new(),
        }
    }

    /// Repaints `row` at the next frame even if the terminal did not change it.
    pub fn invalidate_row(&mut self, row: usize) {
        self.wanted.push(row);
    }

    /// Brings the backend up to date: only rows that changed since the last frame are rebuilt.
    pub fn build<L: EventListener>(&mut self, term: &mut Term<L>, focused: bool, out: &mut impl Backend) {
        let (rows, cols) = (term.screen_lines(), term.columns());
        let mut full = out.begin_frame(rows, cols);
        // The terminal was resized (a pane got another size): what is on screen is of the old grid.
        full |= self.cols != cols || self.dirty.len() != rows;
        self.cols = cols;
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
        for r in self.wanted.drain(..) {
            if let Some(f) = self.dirty.get_mut(r) {
                *f = true;
            }
        }

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

/// How many rows above the prompt's last line its reserved line may be (a prompt of several lines), and
/// how many rows a typed command may wrap over.
const PROMPT_LINES: i32 = 4;

/// Where the shell put the line the prompt chips go on, as a viewport row.
///
/// The shell's hook (`context.rs`) makes the prompt start with a line of its own holding one concealed
/// space (SGR 8) in its first column, and end with a concealed space right before where the user types.
/// So the prompt being typed at is the one whose end marker is on the cursor's row (or on an earlier
/// row of a command that wrapped), and the chips go on the reserved line above it. Output a program
/// printed since leaves no marker at the cursor. Only a hook older than the numbered prompts is drawn from this: the rest use `prompt_rows`.
pub fn prompt_row<L: EventListener>(term: &Term<L>) -> Option<usize> {
    let grid = term.grid();
    if grid.display_offset() != 0 {
        return None;
    }
    let cols = term.columns();
    let marker = |line: i32, col: usize| {
        let c = &grid[Line(line)][Column(col)];
        c.c == ' ' && c.flags.contains(Flags::HIDDEN)
    };
    let mut row = grid.cursor.point.line.0;
    let mut end = None;
    for _ in 0..=PROMPT_LINES {
        if row < 0 || row >= term.screen_lines() as i32 {
            return None;
        }
        if (0..cols).any(|c| marker(row, c)) {
            end = Some(row);
            break;
        }
        // Up a row only when the one above wraps into this one.
        if row == 0 || !grid[Line(row - 1)][Column(cols - 1)].flags.contains(Flags::WRAPLINE) {
            return None;
        }
        row -= 1;
    }
    let end = end?;
    (1..=PROMPT_LINES).find_map(|d| (end - d >= 0 && marker(end - d, 0)).then_some((end - d) as usize))
}

/// The reserved lines of every prompt on screen, as `(viewport row, prompt number)`, oldest first: a
/// hidden space in the first column whose colour is the number (`context.rs`, `history.rs`). Unlike
/// `prompt_row` it finds the prompts of commands that have run, in the scrollback too.
pub fn prompt_rows<L: EventListener>(term: &Term<L>, out: &mut Vec<(usize, u32)>) {
    out.clear();
    let grid = term.grid();
    let offset = grid.display_offset() as i32;
    for row in 0..term.screen_lines() {
        let c = &grid[Line(row as i32 - offset)][Column(0)];
        if let (' ', true, Color::Spec(rgb)) = (c.c, c.flags.contains(Flags::HIDDEN), c.fg) {
            out.push((row, (rgb.r as u32) << 16 | (rgb.g as u32) << 8 | rgb.b as u32));
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

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::{
        event::VoidListener,
        term::Config,
        vte::ansi::{Processor, StdSyncHandler},
    };

    use crate::session::Size;

    const MARK: &str = "\x1b[8m \x1b[0m";

    fn term(rows: usize, text: &str) -> Term<VoidListener> {
        let mut t = Term::new(Config::default(), &Size { cols: 20, rows }, VoidListener);
        Processor::<StdSyncHandler>::new().advance(&mut t, text.as_bytes());
        t
    }

    /// What the hook makes a prompt print: the reserved line, the prompt, the end marker.
    fn prompt(ps1: &str) -> String {
        format!("{MARK}\r\n{ps1}{MARK}")
    }

    #[test]
    fn every_numbered_prompt_on_screen_is_found_with_its_number() {
        let numbered = |n: u32| format!("\x1b[8;38;2;{};{};{}m \x1b[0m\r\nme$ {MARK}", n >> 16, n >> 8 & 255, n & 255);
        let t = term(12, &format!("{}ls\r\nout\r\n{}\r\nout\r\n{}", numbered(1), numbered(258), numbered(70000)));
        let mut rows = Vec::new();
        prompt_rows(&t, &mut rows);
        assert_eq!(rows, [(0, 1), (3, 258), (6, 70000)]);
        // The plain markers of an older hook, ordinary hidden spaces and coloured text are not numbered prompts.
        for text in [format!("{MARK}\r\nme$ {MARK}"), "\x1b[38;2;0;0;5m \x1b[0m".to_string(), "\x1b[8;38;2;0;0;5mx\x1b[0m".to_string()] {
            prompt_rows(&term(5, &text), &mut rows);
            assert!(rows.is_empty(), "{text:?}");
        }
        // Scrolled back, the numbers follow the rows.
        let mut t = term(5, &format!("{}\r\n\r\n\r\n\r\n\r\n\r\n{}", numbered(1), numbered(2)));
        prompt_rows(&t, &mut rows);
        assert_eq!(rows.iter().map(|r| r.1).collect::<Vec<_>>(), [2]);
        t.scroll_display(alacritty_terminal::grid::Scroll::Delta(4));
        prompt_rows(&t, &mut rows);
        assert!(rows.iter().any(|r| r.1 == 1), "{rows:?}");
    }

    #[test]
    fn chips_go_on_the_reserved_line_above_the_prompt_being_typed_at() {
        let t = term(10, &format!("old output\r\n{}", prompt("me$ ")));
        assert_eq!(prompt_row(&t), Some(1));
        // Text typed after the marker does not move it.
        let t = term(10, &format!("{}ls -la", prompt("me$ ")));
        assert_eq!(prompt_row(&t), Some(0));
    }

    #[test]
    fn a_prompt_of_several_lines_and_a_wrapped_command_still_find_it() {
        let t = term(10, &format!("{MARK}\r\n~/code/project  main\r\nme$ {MARK}"));
        assert_eq!(prompt_row(&t), Some(0));
        // A command longer than a row wraps onto the next ones: the marker is on the first.
        let t = term(10, &format!("x\r\n{}{}", prompt("me$ "), "a".repeat(35)));
        assert_eq!(prompt_row(&t), Some(1));
    }

    #[test]
    fn no_chips_while_a_program_runs_or_without_the_hook() {
        // The command ran and printed: the cursor is past the output, no marker on its row.
        let t = term(10, &format!("{}ls\r\nfile-a\r\nfile-b\r\n", prompt("me$ ")));
        assert_eq!(prompt_row(&t), None);
        // A prompt without the markers (the hook is not installed).
        assert_eq!(prompt_row(&term(10, "out\r\nme$ ")), None);
        // The end marker without a reserved line above it.
        assert_eq!(prompt_row(&term(10, &format!("me$ {MARK}"))), None);
        // Ordinary spaces are not markers.
        assert_eq!(prompt_row(&term(10, " \r\nme$  ")), None);
    }

    #[test]
    fn chips_need_the_live_screen_and_a_row_to_stand_on() {
        // The prompt on the very first row has no room above it for the reserved line.
        assert_eq!(prompt_row(&term(10, &format!("me$ {MARK}"))), None);
        // Scrolled back into the history: the prompt is not the one being typed at.
        let mut t = term(5, &format!("{}\r\n\r\n\r\n\r\n\r\n\r\n{}", prompt("a$ "), prompt("b$ ")));
        assert!(prompt_row(&t).is_some());
        t.scroll_display(alacritty_terminal::grid::Scroll::Delta(2));
        assert_eq!(prompt_row(&t), None);
    }
}
