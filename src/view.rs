//! Grid -> cells: reads the terminal and tells a backend which rows to paint, and on which rows its lines go (`Fold`).
//! Backend-agnostic: the GPU renderer and the CPU renderer both implement `Backend`.
use alacritty_terminal::{
    Term,
    event::EventListener,
    grid::{Dimensions, GridCell},
    index::{Column, Line, Point},
    selection::SelectionRange,
    term::{TermDamage, TermMode, cell::Flags},
    vte::ansi::{Color, CursorShape, NamedColor},
};

use crate::cell::{Cell, F_BAR, F_BEAM, F_BOX, F_STRIKE, F_UNDER, Glyphs, T_BLANK, pack};
use crate::palette;

/// What the grid builder needs from a renderer, besides the glyph tiles.
pub trait Backend: Glyphs {
    /// Starts a frame for a `rows` x `cols` grid whose lines are laid out as `fold` says. Returns true when every
    /// row has to be written: first frame, the grid size changed, or the glyph tiles were reassigned.
    fn begin_frame(&mut self, rows: usize, cols: usize, fold: &Fold) -> bool;
    /// Receives `count` consecutive rows starting at `first`: `count * cols` cells, row-major.
    fn put_rows(&mut self, first: usize, count: usize, cells: &[Cell]);
    /// Whether it draws the composer apart (`Fold`); one that does not gets the rows as the terminal has them.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    fn docks(&self) -> bool {
        false
    }
}

/// How the terminal's lines are laid on a pane's rows (ADR 0037). The composer, the prompt being typed at (from its
/// reserved line, where the chips go) or else the line the cursor is on, and whatever is below either, takes the
/// last rows, which the backend draws apart below a hairline; the lines above it end on the row right over it. So
/// the prompt sits at the bottom from the first one on and what commands print rises above it, as the reference's
/// composer and blocks; scrolled back, the rows above scroll and the composer stays. A cleared screen shows nothing
/// above the prompt (the cleared lines are the history, shown when scrolled to). The alternate screen (a
/// full-screen program) and a backend without a composer get the rows as the terminal has them (`flat`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Fold {
    /// Rows of the pane.
    pub rows: usize,
    /// The composer's first row; `rows` when there is no composer.
    pub split: usize,
    /// The line on row 0 (history lines are negative): the rows above `split` show `top + row`.
    pub top: i32,
    /// Lines above this one are shown blank: the first line in view (`-display_offset`).
    pub floor: i32,
    /// The composer's first line: the rows from `split` on show `live + row - split`.
    pub live: i32,
    /// The composer starts with the prompt's reserved line (the chips' row).
    pub chips: bool,
}

impl Fold {
    /// The rows as the terminal has them, scrolled back `offset` lines.
    pub fn flat(rows: usize, offset: usize) -> Fold {
        let top = -(offset as i32);
        Fold { rows, split: rows, top, floor: top, live: rows as i32, chips: false }
    }

    /// There is a composer.
    pub fn docked(&self) -> bool {
        self.split < self.rows
    }

    /// The line row `row` shows; None for a blank row (above the first line in view, or past the last row).
    pub fn line(&self, row: usize) -> Option<i32> {
        if row >= self.rows {
            None
        } else if row >= self.split {
            Some(self.live + (row - self.split) as i32)
        } else {
            let l = self.top + row as i32;
            (l >= self.floor).then_some(l)
        }
    }

    /// The row line `line` is shown on, if it is shown.
    pub fn row(&self, line: i32) -> Option<usize> {
        if self.docked() && line >= self.live {
            let r = self.split as i64 + (line - self.live) as i64;
            return (r < self.rows as i64).then_some(r as usize);
        }
        let r = line as i64 - self.top as i64;
        (line >= self.floor && r >= 0 && r < self.split as i64).then_some(r as usize)
    }
}

/// Lays the lines of `term` on its rows (`Fold`); `dock` when the backend draws a composer.
pub fn fold<L: EventListener>(term: &Term<L>, dock: bool) -> Fold {
    let grid = term.grid();
    let (rows, offset) = (term.screen_lines(), grid.display_offset());
    if !dock || rows < 2 || term.mode().contains(TermMode::ALT_SCREEN) {
        return Fold::flat(rows, offset);
    }
    let cols = term.columns();
    let (n, first) = (rows as i32, -(grid.history_size() as i32));
    let cursor = grid.cursor.point.line.0;
    // The composer's first line: the reserved line of the prompt being typed at, else the first of the lines the
    // cursor's wraps from (a command typed at a prompt without the hook, a long line a program is writing).
    let (mut live, mut chips) = match live_prompt(term) {
        Some(l) => (l, true),
        None => {
            let mut l = cursor;
            while l > first && cursor - l < n - 1 && grid[Line(l - 1)][Column(cols - 1)].flags.contains(Flags::WRAPLINE) {
                l -= 1;
            }
            (l, false)
        }
    };
    // Its last: the cursor's, or the last below it that holds anything (completions listed under the prompt).
    let last = (cursor + 1..n).rev().find(|&l| !grid[Line(l)].is_clear()).unwrap_or(cursor);
    if last - live + 1 > n {
        (live, chips) = (last - n + 1, false);
    }
    let split = (n - (last - live + 1)) as usize;
    let floor = -(offset as i32);
    // The lines above end right over the composer; scrolled back, they start at the first line in view.
    Fold { rows, split, top: floor.min(live - split as i32), floor, live, chips }
}

/// Frame-to-frame state that `Term`'s damage does not cover, and the scratch rows are built in.
pub struct View {
    dirty: Vec<bool>,
    last_cursor: (Point, CursorShape, bool),
    last_sel: Option<SelectionRange>,
    /// The columns of the last build: a different width (a pane resized) rebuilds every row.
    cols: usize,
    /// The cells of the rows the last `build_folded` rebuilt, one after the other: what `put` hands to the backend.
    stage: Vec<Cell>,
    /// Rows the caller wants repainted at the next frame (something is drawn over them).
    wanted: Vec<usize>,
    /// How the last frame laid the lines on the rows: another layout repaints every row.
    fold: Fold,
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
            fold: Fold::default(),
        }
    }

    /// How the last frame laid the terminal's lines on the rows: what the pointer is over.
    pub fn fold(&self) -> Fold {
        self.fold
    }

    /// Repaints `row` at the next frame even if the terminal did not change it.
    pub fn invalidate_row(&mut self, row: usize) {
        self.wanted.push(row);
    }

    /// Brings the backend up to date: only rows that changed since the last frame are rebuilt.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub fn build<L: EventListener>(&mut self, term: &mut Term<L>, focused: bool, out: &mut impl Backend) {
        let fold = fold(term, out.docks());
        self.build_folded(term, focused, fold, out);
        self.put(out);
    }

    /// The part of `build` that reads the terminal, with the lines laid out as `fold` (from `fold`) says: what changed
    /// since the last frame, and the cells of every row to repaint. `put` hands them to the backend afterwards, so a
    /// caller holding the terminal's lock can release it first: painting the rows is most of a frame, and the PTY
    /// thread cannot parse while the lock is held.
    pub fn build_folded<L: EventListener>(&mut self, term: &mut Term<L>, focused: bool, fold: Fold, out: &mut impl Backend) {
        let (rows, cols) = (term.screen_lines(), term.columns());
        let mut full = out.begin_frame(rows, cols, &fold);
        // The terminal was resized (a pane got another size): what is on screen is of the old grid. Lines that
        // moved to other rows (the composer grew, the prompt moved down) are a new picture too.
        full |= self.cols != cols || self.dirty.len() != rows || fold != self.fold;
        self.fold = fold;
        self.cols = cols;
        self.dirty.clear();
        self.dirty.resize(rows, false);

        // Terminal damage must be read and reset on every frame.
        match term.damage() {
            TermDamage::Full => full = true,
            TermDamage::Partial(lines) => {
                // Damage is in lines of the screen; scrolled back, every row is rebuilt anyway.
                for d in lines {
                    if let Some(f) = fold.row(d.line as i32).and_then(|r| self.dirty.get_mut(r)) {
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
        let shown = content.mode.contains(TermMode::SHOW_CURSOR) && shows_cursor(&fold, offset, cur.point.line.0);
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
                for r in (from..=to).filter_map(|l| fold.row(l)) {
                    dirty[r] = true;
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

        // The damaged rows' cells, run after run.
        self.stage.clear();
        for (first, count) in runs(&self.dirty) {
            let at = self.stage.len();
            self.stage.resize(at + count * cols, Cell::BLANK);
            build_rows(out, term, &fold, first, &mut self.stage[at..], focused);
        }
    }

    /// Hands the rows the last `build_folded` built to the backend: each run of consecutive rows in one call.
    pub fn put(&self, out: &mut impl Backend) {
        let mut cells = self.stage.as_slice();
        for (first, count) in runs(&self.dirty) {
            let (run, rest) = cells.split_at(count * self.cols);
            out.put_rows(first, count, run);
            cells = rest;
        }
    }
}

/// The runs of consecutive true rows of `dirty`, as `(first, count)`.
fn runs(dirty: &[bool]) -> impl Iterator<Item = (usize, usize)> + '_ {
    let mut row = 0;
    std::iter::from_fn(move || {
        let first = row + dirty.get(row..)?.iter().position(|&d| d)?;
        let count = dirty[first..].iter().position(|&d| !d).unwrap_or(dirty.len() - first);
        row = first + count;
        Some((first, count))
    })
}

/// How many rows above the prompt's last line its reserved line may be (a prompt of several lines), and
/// how many rows a typed command may wrap over.
const PROMPT_LINES: i32 = 4;

/// Whether the cell at `(line, col)` is one of the shell hook's concealed spaces.
fn is_marker(grid: &alacritty_terminal::grid::Grid<alacritty_terminal::term::cell::Cell>, line: i32, col: usize) -> bool {
    let c = &grid[Line(line)][Column(col)];
    c.c == ' ' && c.flags.contains(Flags::HIDDEN)
}

/// Where the prompt being typed at ends: the concealed space the shell's hook (`context.rs`) puts right before where the
/// user types, on the cursor's row or on an earlier row of a command that wrapped. What is typed starts in the cell after
/// it. Output a program printed since leaves no marker at the cursor, so there is none then.
pub fn prompt_end<L: EventListener>(term: &Term<L>) -> Option<Point> {
    let grid = term.grid();
    let first = -(grid.history_size() as i32);
    let cols = term.columns();
    let mut row = grid.cursor.point.line.0;
    for _ in 0..=PROMPT_LINES {
        if row < first {
            return None;
        }
        if let Some(col) = (0..cols).rev().find(|&c| is_marker(grid, row, c)) {
            return Some(Point::new(Line(row), Column(col)));
        }
        // Up a row only when the one above wraps into this one.
        if row == first || !grid[Line(row - 1)][Column(cols - 1)].flags.contains(Flags::WRAPLINE) {
            return None;
        }
        row -= 1;
    }
    None
}

/// The line (of the terminal: the history's are negative) the shell put the prompt being typed at's chips on.
///
/// The shell's hook (`context.rs`) makes the prompt start with a line of its own holding one concealed
/// space (SGR 8) in its first column, and end with a concealed space right before where the user types.
/// So the prompt being typed at is the one whose end marker is on the cursor's row (or on an earlier
/// row of a command that wrapped: `prompt_end`), and the chips go on the reserved line above it. Whether the view is
/// scrolled back does not matter: the composer (`Fold`) shows this prompt either way.
pub fn live_prompt<L: EventListener>(term: &Term<L>) -> Option<i32> {
    let grid = term.grid();
    let first = -(grid.history_size() as i32);
    let end = prompt_end(term)?.line.0;
    (1..=PROMPT_LINES).map(|d| end - d).find(|&l| l >= first && is_marker(grid, l, 0))
}

/// Nothing is typed at the prompt yet: the cursor is right after the prompt's end marker (`live_prompt`) and its
/// line is empty from there on. The composer then says what it is for (`bar.rs`).
pub fn input_empty<L: EventListener>(term: &Term<L>) -> bool {
    let grid = term.grid();
    let (cur, cols) = (grid.cursor.point, term.columns());
    let row = &grid[cur.line];
    let col = cur.column.0;
    let after_marker = col > 0 && col < cols && {
        let m = &row[Column(col - 1)];
        m.c == ' ' && m.flags.contains(Flags::HIDDEN)
    };
    after_marker && !term.mode().contains(TermMode::ALT_SCREEN) && (col..cols).all(|c| row[Column(c)].is_empty())
}

/// Whether the cursor is drawn: on the screen in view, or in the composer while the rows above are scrolled back.
fn shows_cursor(fold: &Fold, offset: usize, line: i32) -> bool {
    offset == 0 || (fold.docked() && line >= fold.live && fold.row(line).is_some())
}

/// The reserved lines of every prompt on the rows, as `(row, prompt number)` with the rows laid out as `fold`
/// says, oldest first: a hidden space in the first column whose colour is the number (`context.rs`,
/// `history.rs`). Unlike `live_prompt` it finds the prompts of commands that have run, in the scrollback too.
pub fn prompt_rows<L: EventListener>(term: &Term<L>, fold: &Fold, out: &mut Vec<(usize, u32)>) {
    out.clear();
    let grid = term.grid();
    for row in 0..term.screen_lines() {
        let Some(line) = fold.line(row) else { continue };
        let c = &grid[Line(line)][Column(0)];
        if let (' ', true, Color::Spec(rgb)) = (c.c, c.flags.contains(Flags::HIDDEN), c.fg) {
            out.push((row, (rgb.r as u32) << 16 | (rgb.g as u32) << 8 | rgb.b as u32));
        }
    }
}

/// Fills `cells` with the rows starting at `first`, the lines laid out as `fold` says: row-major, `term.columns()`
/// cells per row. The slice arrives zeroed (blank), so only non-blank cells are written.
fn build_rows<L: EventListener>(g: &mut impl Glyphs, term: &Term<L>, fold: &Fold, first: usize, cells: &mut [Cell], focused: bool) {
    let content = term.renderable_content();
    let colors = content.colors;
    let selection = content.selection;
    let cur = content.cursor;
    let show_cursor = content.mode.contains(TermMode::SHOW_CURSOR) && shows_cursor(fold, content.display_offset, cur.point.line.0);
    let shape = if !focused { CursorShape::HollowBlock } else { cur.shape };
    let cursor_color = palette::CURSOR;
    let dark = palette::BG;
    let default_bg = palette::resolve(Color::Named(NamedColor::Background), colors);
    let transparent = pack(0, 0, 0, 0);
    let cols = term.columns();

    for (n, out) in cells.chunks_exact_mut(cols).enumerate() {
        let Some(line) = fold.line(first + n) else { continue };
        let line = Line(line);
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

    #[test]
    fn rows_to_repaint_go_out_in_runs_of_consecutive_rows() {
        let r = |d: &[bool]| runs(d).collect::<Vec<_>>();
        assert_eq!(r(&[]), []);
        assert_eq!(r(&[false, false]), []);
        assert_eq!(r(&[true, true, true]), [(0, 3)]);
        assert_eq!(r(&[false, true, true, false, true]), [(1, 2), (4, 1)]);
        assert_eq!(r(&[true, false, false, true, true]), [(0, 1), (3, 2)]);
    }

    fn term(rows: usize, text: &str) -> Term<VoidListener> {
        let mut t = Term::new(Config::default(), &Size { cols: 20, rows }, VoidListener);
        Processor::<StdSyncHandler>::new().advance(&mut t, text.as_bytes());
        t
    }

    fn flat(t: &Term<VoidListener>) -> Fold {
        Fold::flat(t.screen_lines(), t.grid().display_offset())
    }

    /// The lines the rows show, `None` for a blank one.
    fn shown(f: &Fold) -> Vec<Option<i32>> {
        (0..f.rows).map(|r| f.line(r)).collect()
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
        prompt_rows(&t, &flat(&t), &mut rows);
        assert_eq!(rows, [(0, 1), (3, 258), (6, 70000)]);
        // The plain markers of an older hook, ordinary hidden spaces and coloured text are not numbered prompts.
        for text in [format!("{MARK}\r\nme$ {MARK}"), "\x1b[38;2;0;0;5m \x1b[0m".to_string(), "\x1b[8;38;2;0;0;5mx\x1b[0m".to_string()] {
            let t = term(5, &text);
            prompt_rows(&t, &flat(&t), &mut rows);
            assert!(rows.is_empty(), "{text:?}");
        }
        // Scrolled back, the numbers follow the rows.
        let mut t = term(5, &format!("{}\r\n\r\n\r\n\r\n\r\n\r\n{}", numbered(1), numbered(2)));
        prompt_rows(&t, &flat(&t), &mut rows);
        assert_eq!(rows.iter().map(|r| r.1).collect::<Vec<_>>(), [2]);
        t.scroll_display(alacritty_terminal::grid::Scroll::Delta(4));
        prompt_rows(&t, &flat(&t), &mut rows);
        assert!(rows.iter().any(|r| r.1 == 1), "{rows:?}");
    }

    #[test]
    fn chips_go_on_the_reserved_line_above_the_prompt_being_typed_at() {
        let t = term(10, &format!("old output\r\n{}", prompt("me$ ")));
        assert_eq!(live_prompt(&t), Some(1));
        // Text typed after the marker does not move it.
        let t = term(10, &format!("{}ls -la", prompt("me$ ")));
        assert_eq!(live_prompt(&t), Some(0));
    }

    #[test]
    fn a_prompt_of_several_lines_and_a_wrapped_command_still_find_it() {
        let t = term(10, &format!("{MARK}\r\n~/code/project  main\r\nme$ {MARK}"));
        assert_eq!(live_prompt(&t), Some(0));
        // A command longer than a row wraps onto the next ones: the marker is on the first.
        let t = term(10, &format!("x\r\n{}{}", prompt("me$ "), "a".repeat(35)));
        assert_eq!(live_prompt(&t), Some(1));
    }

    #[test]
    fn no_chips_while_a_program_runs_or_without_the_hook() {
        // The command ran and printed: the cursor is past the output, no marker on its row.
        let t = term(10, &format!("{}ls\r\nfile-a\r\nfile-b\r\n", prompt("me$ ")));
        assert_eq!(live_prompt(&t), None);
        // A prompt without the markers (the hook is not installed).
        assert_eq!(live_prompt(&term(10, "out\r\nme$ ")), None);
        // The end marker without a reserved line above it.
        assert_eq!(live_prompt(&term(10, &format!("me$ {MARK}"))), None);
        // Ordinary spaces are not markers.
        assert_eq!(live_prompt(&term(10, " \r\nme$  ")), None);
    }

    #[test]
    fn chips_need_a_row_to_stand_on() {
        // The prompt on the very first row has no room above it for the reserved line.
        assert_eq!(live_prompt(&term(10, &format!("me$ {MARK}"))), None);
        // Scrolled back into the history: the prompt being typed at is still the live one, which the composer shows.
        let mut t = term(5, &format!("{}\r\n\r\n\r\n\r\n\r\n\r\n{}", prompt("a$ "), prompt("b$ ")));
        let live = live_prompt(&t);
        assert_eq!(live, Some(3));
        t.scroll_display(alacritty_terminal::grid::Scroll::Delta(2));
        assert_eq!(live_prompt(&t), live);
    }

    #[test]
    fn the_first_prompt_sits_at_the_bottom_with_nothing_above() {
        let t = term(6, &prompt("$ "));
        let f = fold(&t, true);
        assert_eq!((f.split, f.live, f.chips), (4, 0, true), "the reserved line and the prompt are the composer");
        assert_eq!(shown(&f), [None, None, None, None, Some(0), Some(1)]);
        // A backend without a composer, and the alternate screen, get the rows as they are.
        assert_eq!(fold(&t, false), Fold::flat(6, 0));
        let alt = term(6, &format!("{}\x1b[?1049h", prompt("$ ")));
        assert_eq!(fold(&alt, true), Fold::flat(6, 0));
    }

    #[test]
    fn output_rises_over_the_composer_and_a_running_command_docks_its_cursor_line() {
        // A command ran and printed two lines; the next prompt is the composer, everything else ends right above it.
        let t = term(8, &format!("{}ls\r\na\r\nb\r\n{}", prompt("$ "), prompt("$ ")));
        let f = fold(&t, true);
        assert_eq!((f.split, f.live), (6, 4));
        assert_eq!(shown(&f), [None, None, Some(0), Some(1), Some(2), Some(3), Some(4), Some(5)]);
        assert_eq!((f.row(1), f.row(5), f.row(6)), (Some(3), Some(7), None));
        // While a command runs (no marker at the cursor) the composer is the line the cursor is on.
        let t = term(8, &format!("{}make\r\nbuilding", prompt("$ ")));
        let f = fold(&t, true);
        assert_eq!((f.split, f.live, f.chips), (7, 2, false));
        assert_eq!(shown(&f), [None, None, None, None, None, Some(0), Some(1), Some(2)]);
        // A full screen folds nothing away: the rows are the lines, the composer the last two.
        let t = term(4, &format!("1\r\n2\r\n3\r\n4\r\n{}", prompt("$ ")));
        let f = fold(&t, true);
        assert_eq!((f.split, f.top), (2, 0));
        assert_eq!(shown(&f), [Some(0), Some(1), Some(2), Some(3)]);
    }

    #[test]
    fn a_wrapped_command_and_a_list_below_the_prompt_are_the_composer() {
        let t = term(8, &format!("x\r\n{}{}", prompt("$ "), "a".repeat(35)));
        let f = fold(&t, true);
        assert_eq!((f.split, f.live), (5, 1), "the reserved line, the prompt and the row its command wrapped onto");
        // Completions listed below the prompt (zsh) are kept with it.
        let t = term(8, &format!("{}\x1b7\r\n\r\nfoo  bar\x1b8", prompt("$ ")));
        let f = fold(&t, true);
        assert_eq!((f.split, f.live), (4, 0));
        assert_eq!(f.line(7), Some(3));
    }

    #[test]
    fn scrolled_back_the_rows_above_scroll_and_the_composer_stays() {
        let mut t = term(4, &format!("1\r\n2\r\n3\r\n4\r\n5\r\n{}", prompt("$ ")));
        t.scroll_display(alacritty_terminal::grid::Scroll::Delta(2));
        let f = fold(&t, true);
        assert_eq!(shown(&f), [Some(-2), Some(-1), Some(2), Some(3)]);
        assert!(shows_cursor(&f, 2, 3) && !shows_cursor(&f, 2, 1));
        // A cleared screen: what was on it is history, shown only when scrolled to (from the composer up).
        let mut t = term(4, &format!("1\r\n2\r\n3\x1b[H\x1b[2J{}", prompt("$ ")));
        let f = fold(&t, true);
        assert!(t.grid().history_size() > 0);
        assert_eq!(shown(&f), [None, None, Some(0), Some(1)]);
        t.scroll_display(alacritty_terminal::grid::Scroll::Delta(1));
        assert_eq!(shown(&fold(&t, true)), [None, Some(-1), Some(0), Some(1)]);
    }

    #[test]
    fn the_input_is_empty_right_after_the_prompt_only() {
        assert!(input_empty(&term(6, &prompt("$ "))));
        assert!(!input_empty(&term(6, &format!("{}ls", prompt("$ ")))));
        // The cursor moved back over what was typed.
        assert!(!input_empty(&term(6, &format!("{}ls\x1b[2D", prompt("$ ")))));
        // A plain prompt, without the hook's markers.
        assert!(!input_empty(&term(6, "$ ")));
    }
}
