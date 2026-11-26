//! Moving the shell's cursor with a click on what is being typed.
//!
//! A terminal cannot move the shell's cursor: the shell owns the line. What it can do is press the arrow keys for the
//! user, as many as it takes to get from the cursor to the cell that was clicked (kitty, iTerm2 and others do the same
//! with the shell's cooperation). axon already has that cooperation: the hook puts a concealed space right before where
//! the user types (`view::prompt_end`), so it knows where the typed text starts, and a command that wrapped over several
//! rows is one line to the shell, which the arrows walk through, cell by cell, from one row's end to the next one's start.
//!
//! Pure: the terminal and the clicked cell in, the number of arrows out (`steps`) and the bytes they are (`keys`).
//! Whether to do it at all (the shell waits at its prompt, no program has the mouse, the button went down and up on the
//! same cell) is the app's to say (`wlapp.rs`).
use alacritty_terminal::{
    Term,
    event::EventListener,
    grid::{Dimensions, GridCell},
    index::{Column, Line, Point},
    term::{TermMode, cell::Flags},
    vte::ansi::{Color, NamedColor},
};

use crate::{input, view};

/// Whether `c` looks like a shell's suggestion for the rest of the command (zsh-autosuggestions, fish): dim, bright black
/// or a dark grey. It is drawn after the text but is not part of it, and a Right arrow at the end of the text would
/// accept it, so the click must not go there.
fn ghost(c: &alacritty_terminal::term::cell::Cell) -> bool {
    c.flags.contains(Flags::DIM)
        || match c.fg {
            Color::Named(NamedColor::BrightBlack) | Color::Indexed(8) => true,
            Color::Spec(rgb) => rgb.r == rgb.g && rgb.g == rgb.b && rgb.r < 0x90,
            _ => false,
        }
}

/// Whether the cell holds no character of its own: the second half of a wide character, or the gap a wide character
/// that did not fit left at the end of a row. The arrows count characters, not cells.
fn spacer(c: &alacritty_terminal::term::cell::Cell) -> bool {
    c.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
}

/// The characters from `a` up to (not including) `b`, `a` not after `b`, along rows that wrap into one another.
fn between<L: EventListener>(term: &Term<L>, a: Point, b: Point) -> i32 {
    let (grid, cols) = (term.grid(), term.columns());
    let mut n = 0;
    for line in a.line.0..=b.line.0 {
        let lo = if line == a.line.0 { a.column.0 } else { 0 };
        let hi = if line == b.line.0 { b.column.0 } else { cols };
        n += (lo..hi).filter(|&c| !spacer(&grid[Line(line)][Column(c)])).count() as i32;
    }
    n
}

/// Where what is typed starts: the cell after the prompt's end marker (`view::prompt_end`, on the next row when the marker
/// is in the last column) when the hook draws one; else the first cell of the first row of the line the cursor is on (the
/// shell's own prompt: where its text ends is unknown, and an arrow that goes past it is ignored by the shell). The line is
/// the cursor's row and the rows above it that wrap into it.
fn start_of<L: EventListener>(term: &Term<L>) -> Point {
    let (grid, cols) = (term.grid(), term.columns());
    if let Some(end) = view::prompt_end(term) {
        return if end.column.0 + 1 >= cols { Point::new(Line(end.line.0 + 1), Column(0)) } else { Point::new(end.line, Column(end.column.0 + 1)) };
    }
    let first = -(grid.history_size() as i32);
    let mut line = grid.cursor.point.line.0;
    while line > first && grid[Line(line - 1)][Column(cols - 1)].flags.contains(Flags::WRAPLINE) {
        line -= 1;
    }
    Point::new(Line(line), Column(0))
}

/// How many characters the shell's cursor has to move to be on the cell `to`: negative to the left, positive to the right,
/// 0 when it is there already. `None` when there is nothing to do: the click is not on the line being typed (the output
/// above it, the chips' row, below it) or the screen is a full-screen program's.
///
/// The line is what is typed after the prompt (`start_of`) up to the last character of the row the cursor is on, or of the
/// last row that wraps from it. A click before the start goes to the start, one past the end goes to the end.
pub fn steps<L: EventListener>(term: &Term<L>, to: Point) -> Option<i32> {
    if term.mode().contains(TermMode::ALT_SCREEN) {
        return None;
    }
    let (grid, cols) = (term.grid(), term.columns());
    let start = start_of(term);
    let cursor = grid.cursor.point;
    if cursor < start {
        return None;
    }
    // The last row of the line: as far down as the rows wrap into one another.
    let mut last = cursor.line.0;
    while last + 1 < term.screen_lines() as i32 && grid[Line(last)][Column(cols - 1)].flags.contains(Flags::WRAPLINE) {
        last += 1;
    }
    // Its text ends after the last cell that holds something that is not a suggestion; the cursor is never past it.
    let row = &grid[Line(last)];
    let mut stop = (0..cols).rev().find(|&c| !(row[Column(c)].is_empty() || ghost(&row[Column(c)]))).map_or(0, |c| c + 1);
    if last == cursor.line.0 {
        stop = stop.max(cursor.column.0);
    }
    let finish = Point::new(Line(last), Column(stop.max(if last == start.line.0 { start.column.0 } else { 0 })));
    if to.line.0 < start.line.0 || to.line.0 > last {
        return None;
    }
    // The second half of a wide character is the character.
    let mut to = to;
    if spacer(&grid[to.line][to.column]) && to.column.0 > 0 {
        to.column = Column(to.column.0 - 1);
    }
    let to = to.max(start).min(finish);
    Some(if to >= cursor { between(term, cursor, to) } else { -between(term, to, cursor) })
}

/// The arrow presses that do `steps`: `steps` of them, left or right as it says, as the program in `term` takes them.
pub fn keys<L: EventListener>(term: &Term<L>, to: Point) -> Option<Vec<u8>> {
    let n = steps(term, to).filter(|&n| n != 0)?;
    let arrow = input::arrow(n > 0, *term.mode());
    Some(arrow.repeat(n.unsigned_abs() as usize))
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

    /// A 20-column terminal after `text`.
    fn term(text: &str) -> Term<VoidListener> {
        let mut t = Term::new(Config::default(), &Size { cols: 20, rows: 8 }, VoidListener);
        Processor::<StdSyncHandler>::new().advance(&mut t, text.as_bytes());
        t
    }

    /// The hook's prompt, then what was typed: the reserved line, `$ `, the end marker (the typed text starts at column 3).
    fn typed(text: &str) -> Term<VoidListener> {
        term(&format!("out\r\n{MARK}\r\n$ {MARK}{text}"))
    }

    fn at(line: i32, col: usize) -> Point {
        Point::new(Line(line), Column(col))
    }

    #[test]
    fn a_click_inside_the_text_moves_the_cursor_there_one_arrow_per_character() {
        let t = typed("echo hello");
        // Line 2 is the prompt's: `$ ` and the marker take columns 0 to 2, the text is in 3..13 with the cursor at 13.
        assert_eq!(t.grid().cursor.point, at(2, 13));
        assert_eq!(steps(&t, at(2, 5)), Some(-8), "column 5 is the third character, the `h`");
        assert_eq!(steps(&t, at(2, 3)), Some(-10), "the first character");
        assert_eq!(steps(&t, at(2, 12)), Some(-1));
    }

    #[test]
    fn a_click_past_the_end_stays_and_one_before_the_start_goes_to_the_start() {
        let t = typed("echo hello");
        assert_eq!(steps(&t, at(2, 19)), Some(0), "to the right of the text: it is at its end already");
        assert_eq!(steps(&t, at(2, 0)), Some(-10), "on the prompt: to where the text starts");
        assert_eq!(steps(&t, at(2, 2)), Some(-10), "on the marker");
    }

    #[test]
    fn what_is_not_the_line_being_typed_is_left_alone() {
        let t = typed("echo hello");
        assert_eq!(steps(&t, at(0, 1)), None, "output above");
        assert_eq!(steps(&t, at(1, 0)), None, "the chips' row");
        assert_eq!(steps(&t, at(3, 0)), None, "below the line");
        // Another row of the screen than the line the cursor is on, with no marker either.
        assert_eq!(steps(&term("out\r\n$ echo hello"), at(0, 1)), None);
        // A full-screen program.
        assert_eq!(steps(&term(&format!("\x1b[?1049h{MARK}\r\n$ {MARK}ls")), at(1, 4)), None);
        // Output printed after the prompt leaves no marker at the cursor.
        assert_eq!(steps(&term(&format!("{MARK}\r\n$ {MARK}ls\r\nresult")), at(1, 4)), None);
    }

    #[test]
    fn without_the_hooks_marker_the_line_is_the_cursors_row_and_the_rows_that_wrap_into_it() {
        // The shell's own prompt: `$ ` is part of the line as far as axon can tell, the text is in 2..12, the cursor at 12.
        let t = term("out\r\n$ echo hello");
        assert_eq!(t.grid().cursor.point, at(1, 12));
        assert_eq!(steps(&t, at(1, 4)), Some(-8));
        assert_eq!(steps(&t, at(1, 19)), Some(0));
        assert_eq!(steps(&t, at(0, 1)), None, "the output above");
        assert_eq!(steps(&t, at(2, 0)), None, "below");
        // A wrapped command: the line starts at the first of its rows.
        let t = term(&format!("out\r\n$ {}", "x".repeat(30)));
        assert_eq!(t.grid().cursor.point, at(2, 12));
        assert_eq!(steps(&t, at(1, 2)), Some(-30));
        assert_eq!(steps(&t, at(0, 1)), None);
    }

    #[test]
    fn a_cursor_in_the_middle_moves_either_way() {
        // `echo hello` with the cursor moved back four characters: column 9, on the `h`'s right neighbour, the `e`.
        let t = typed("echo hello\x1b[4D");
        assert_eq!(t.grid().cursor.point, at(2, 9));
        assert_eq!(steps(&t, at(2, 4)), Some(-5));
        assert_eq!(steps(&t, at(2, 12)), Some(3));
        assert_eq!(steps(&t, at(2, 19)), Some(4), "past the end: to the end");
        assert_eq!(steps(&t, at(2, 9)), Some(0));
        assert_eq!(keys(&t, at(2, 9)), None, "nothing to press");
    }

    #[test]
    fn a_command_that_wrapped_is_one_line() {
        // 20 columns: the text starts at column 3, so 17 characters fit on the first row; 30 characters wrap onto the next.
        let text: String = ('a'..='z').chain('A'..='D').collect();
        let t = typed(&text);
        assert_eq!(t.grid().cursor.point, at(3, 13));
        assert_eq!(steps(&t, at(2, 3)), Some(-30), "the first character, a row up");
        assert_eq!(steps(&t, at(2, 10)), Some(-23));
        assert_eq!(steps(&t, at(3, 2)), Some(-11), "on the second row");
        assert_eq!(steps(&t, at(3, 19)), Some(0), "past the end of the last row");
        assert_eq!(steps(&t, at(4, 0)), None, "below the last row");
    }

    #[test]
    fn a_wide_character_counts_once_and_its_second_half_is_it() {
        let t = typed("日本語");
        assert_eq!(t.grid().cursor.point, at(2, 9));
        assert_eq!(steps(&t, at(2, 3)), Some(-3));
        assert_eq!(steps(&t, at(2, 4)), Some(-3), "the second half of the first character");
        assert_eq!(steps(&t, at(2, 5)), Some(-2));
        assert_eq!(steps(&t, at(2, 8)), Some(-1));
    }

    #[test]
    fn a_suggestion_after_the_cursor_is_not_part_of_the_text() {
        // zsh-autosuggestions: `ls` typed, `-la` drawn in bright black after it, the cursor back on the text's end.
        let t = typed("ls\x1b[90m -la\x1b[0m\x1b[4D");
        assert_eq!(t.grid().cursor.point, at(2, 5));
        assert_eq!(steps(&t, at(2, 8)), Some(0), "clicking on the suggestion must not press Right (it would accept it)");
        assert_eq!(steps(&t, at(2, 3)), Some(-2));
        // The same suggestion in a dark grey and in `dim`.
        for style in ["\x1b[38;2;85;85;85m", "\x1b[2m"] {
            let t = typed(&format!("ls{style} -la\x1b[0m\x1b[4D"));
            assert_eq!(steps(&t, at(2, 8)), Some(0), "{style:?}");
        }
        // Text that ends in a space the user typed: the cursor is at its end, so the end is there.
        assert_eq!(steps(&typed("ls "), at(2, 12)), Some(0));
    }

    #[test]
    fn the_keys_are_the_arrows_the_program_asked_for() {
        let t = typed("echo hello");
        assert_eq!(keys(&t, at(2, 11)), Some(b"\x1b[D\x1b[D".to_vec()));
        let t = typed("echo hello\x1b[4D");
        assert_eq!(keys(&t, at(2, 11)), Some(b"\x1b[C\x1b[C".to_vec()));
        // Applications that set the cursor keys mode (readline's editing mode sets it, some prompts too).
        let t = typed("echo hello\x1b[?1h");
        assert_eq!(keys(&t, at(2, 12)), Some(b"\x1bOD".to_vec()));
    }
}
