//! The app's side of the overview of windows (`overview.rs`, [0028](../../knowledge/decisions/0028-overview.md)): open
//! it (`Alt+O`), bring the cards up to date from the windows every frame while it is open, run what it asks for
//! (`Cmd`), and give it the terminal's text and the grid's font at draw time (`AppGrid`, an `overlay::Grid`).
//! A child of `wlapp.rs` so that it sees `App`, `Win` and `Pane` without making them public.
use alacritty_terminal::{
    grid::Dimensions,
    index::{Column, Line},
    term::cell::Flags,
};

use super::groups_ui::Spawn;
use super::*;
use crate::overview::{self, Card, Cmd, Overview};

/// How many rows of a pane's text the filter looks in (the reference's `log.slice(-30)`).
const SEARCH_ROWS: usize = 30;

/// The last `rows` rows of text of `p`'s screen, ending at its last row with text (or the cursor's row), with the
/// trailing blanks of each row cut and `\n` between them, appended to `out`. Read under the terminal's lock and
/// copied at once: nothing of the terminal is kept. A terminal whose lock the PTY thread holds (it is parsing) is not
/// waited for, as the window on screen is: the frame must not block on a window out of sight. False then, and
/// nothing is appended.
pub(super) fn pane_tail(p: &Pane, rows: usize, out: &mut String) -> bool {
    let Some(t) = p.term.try_lock_unfair() else { return false };
    term_tail(&t, rows, out);
    true
}

/// `pane_tail` of a terminal.
fn term_tail<L: alacritty_terminal::event::EventListener>(t: &Term<L>, rows: usize, out: &mut String) {
    let g = t.grid();
    let (lines, cols) = (t.screen_lines() as i32, t.columns());
    if lines == 0 || cols == 0 || rows == 0 {
        return;
    }
    let ink = |l: i32, c: usize| {
        let ch = g[Line(l)][Column(c)].c;
        ch != ' ' && ch != '\0'
    };
    let cursor = g.cursor.point.line.0.clamp(0, lines - 1);
    let end = (0..lines).rev().find(|&l| (0..cols).any(|c| ink(l, c))).map_or(cursor, |l| l.max(cursor));
    let start = (end + 1 - rows as i32).max(g.topmost_line().0);
    for l in start..=end {
        if l > start {
            out.push('\n');
        }
        let used = (0..cols).rev().find(|&c| ink(l, c)).map_or(0, |c| c + 1);
        for c in 0..used {
            let cell = &g[Line(l)][Column(c)];
            if !cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                out.push(if cell.c == '\0' { ' ' } else { cell.c });
            }
        }
    }
}

/// The cards of `wins` for the overview, in their order (the panel's), reusing what `cards` already holds. `needle`
/// is the filter in lower case: a card whose terminal says it (in any of its panes) is marked, whatever its name.
fn fill_cards(wins: &[Win], groups: &[Group], cur: usize, needle: &str, cards: &mut Vec<Card>, out: &mut Vec<overview::Group>, scratch: &mut String) {
    cards.resize_with(wins.len(), Card::default);
    for (i, (c, w)) in cards.iter_mut().zip(wins).enumerate() {
        c.id = w.id as u32;
        c.group = w.group.map(|g| g as u8);
        c.title.clear();
        c.title.push_str(win_title(w));
        c.meta.clear();
        c.meta.push_str(win_place(w));
        let branch = w.pane().ctx.get(Chip::Branch);
        if !branch.is_empty() {
            c.meta.push_str(" \u{b7} ");
            c.meta.push_str(branch);
        }
        c.status = pane_status(w.pane());
        c.unread = w.unread;
        c.panes = w.panes.len() as u16;
        c.current = i == cur;
        if needle.is_empty() {
            c.text_hit = false;
        } else {
            // A terminal that is busy says nothing this frame: the card keeps the answer it had.
            let (mut hit, mut busy) = (false, false);
            for p in &w.panes {
                scratch.clear();
                if !pane_tail(p, SEARCH_ROWS, scratch) {
                    busy = true;
                } else if overview::contains_ci(scratch, needle) {
                    hit = true;
                    break;
                }
            }
            if hit || !busy {
                c.text_hit = hit;
            }
        }
    }
    out.resize_with(groups.len(), overview::Group::default);
    for (o, g) in out.iter_mut().zip(groups) {
        o.name.clear();
        o.name.push_str(&g.name);
        o.color = g.color;
    }
}

/// The terminal's text and the grid's font, for the overlays that preview a window.
pub(super) struct AppGrid<'a> {
    pub soft: &'a Soft,
    pub wins: &'a [Win],
    pub scratch: &'a mut Vec<u8>,
}

impl overlay::Grid for AppGrid<'_> {
    fn tail(&mut self, id: u32, rows: usize, out: &mut String) -> bool {
        let Some(w) = self.wins.iter().find(|w| w.id as u32 == id) else { return false };
        // A busy terminal leaves the card's text out for this frame; the next one tries again.
        pane_tail(w.pane(), rows, out);
        w.pane().history.running()
    }

    fn text(&mut self, p: &mut Paint, x: i32, cy: i32, s: &str, scale: f32, c: [f32; 3]) {
        let (fx, fy) = (x + p.ox, cy + p.oy);
        if fx < 0 || fy < 0 {
            return;
        }
        let ch = |v: f32| (v * 255.0 + 0.5) as u8;
        let m = self.soft.small(scale);
        self.soft.paint_text(p.fb, fx as usize, fy as usize, s.chars(), pack(ch(c[0]), ch(c[1]), ch(c[2]), 255), false, &m, self.scratch);
    }

    fn metrics(&self, scale: f32) -> (i32, i32) {
        let m = self.soft.small(scale);
        (m.cell_w as i32, m.cell_h as i32)
    }
}

impl App {
    /// The workspace in window pixels: where the overview is, with the panels of windows and settings beside it.
    pub(super) fn overview_area(&self) -> overlay::Rect {
        let r = self.soft.workspace(self.w, self.h);
        (r.x, r.y, r.w, r.h)
    }

    /// `Alt+O`: opens the overview of windows, or closes it.
    pub(super) fn toggle_overview(&mut self) {
        if self.overlays.has(overview::TAG) {
            return self.close_overview();
        }
        let mut ov = Overview::new(Vec::new(), Vec::new(), self.overview_area());
        fill_cards(&self.wins, &self.groups, self.cur, "", &mut ov.cards, &mut ov.groups, &mut ov.text);
        ov.settle();
        self.open_overlay(overview::TAG, Box::new(ov));
    }

    /// Takes the overview away (a window was chosen, or one was picked in the panel).
    pub(super) fn close_overview(&mut self) {
        if self.overlays.close(overview::TAG) {
            self.overlay_reply(Reply { closed: true, ..Reply::default() });
        }
    }

    /// Goes to window `i` the way the panel does: whatever is open over the workspace gives way.
    pub(super) fn go_to(&mut self, i: usize) {
        self.close_overview();
        self.switch_to(i);
    }

    /// Brings the open overview up to date: where the workspace is, and what each window is (title, directory,
    /// status, panes, unread), read from the windows themselves. Once per frame while it is open.
    pub(super) fn sync_overview(&mut self) {
        let area = self.overview_area();
        let Some(ov) = self.overlays.with::<Overview>(overview::TAG) else { return };
        ov.area = area;
        // The filter is borrowed out of the overlay while its cards are rewritten.
        let needle = std::mem::take(&mut ov.lower);
        fill_cards(&self.wins, &self.groups, self.cur, &needle, &mut ov.cards, &mut ov.groups, &mut ov.text);
        ov.lower = needle;
        ov.settle();
    }

    /// What the overview asked for (`overview::Cmd`). `Go` came with the overview closed; the others left it open.
    pub(super) fn overview_ran(&mut self, id: u32) {
        let Some(cmd) = Cmd::decode(id) else { return };
        match cmd {
            Cmd::Go(id) => {
                if let Some(i) = self.win_index(id as u64) {
                    self.switch_to(i);
                }
            }
            Cmd::Close(id) => self.close_wins(&[id as u64], None),
            Cmd::NewWindow => {
                self.spawn_win(Spawn::default());
                self.overlays.toast(tr(Str::OvToastNewWindow), None, 0, Instant::now());
            }
            // The group dialog, over the overview: empty for the button and the tile, with a card's window ticked for a drop on the tile.
            Cmd::NewGroup(id) => self.open_group_dialog(None, id.into_iter().map(u64::from).collect()),
            // A card dropped on a section: its window goes into the group (or out of every group), with the toast.
            Cmd::Move => {
                let dropped = self.overlays.with::<Overview>(overview::TAG).and_then(|ov| ov.dropped.take());
                if let Some((id, g)) = dropped
                    && let Some(wi) = self.win_index(id as u64)
                {
                    let gid = g.and_then(|g| self.groups.get(g)).map(|g| g.id);
                    if g.is_none() || gid.is_some() {
                        self.move_to_group(wi, gid);
                    }
                }
            }
            // The chip of a group's section: the dialog in edit mode (the overview's groups are the app's, in order).
            Cmd::EditGroup(g) => {
                if let Some(id) = self.groups.get(g as usize).map(|g| g.id) {
                    self.open_group_dialog(Some(id), Vec::new());
                }
            }
            Cmd::AddTo(g) => {
                if (g as usize) < self.groups.len() {
                    self.spawn_win(Spawn { group: Some(g as usize), ..Spawn::default() });
                    self.overlays.toast(tr(Str::OvToastAdded), None, 0, Instant::now());
                }
            }
        }
        self.dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use alacritty_terminal::{event::VoidListener, term::Config, vte::ansi::Processor};

    use super::*;

    /// A terminal of `cols` x `rows` that has been sent `output`.
    fn term(cols: usize, rows: usize, output: &str) -> Term<VoidListener> {
        let mut t = Term::new(Config::default(), &session::Size { cols, rows }, VoidListener);
        Processor::<alacritty_terminal::vte::ansi::StdSyncHandler>::new().advance(&mut t, output.as_bytes());
        t
    }

    fn tail(t: &Term<VoidListener>, rows: usize) -> String {
        let mut s = String::new();
        term_tail(t, rows, &mut s);
        s
    }

    #[test]
    fn the_tail_is_the_last_rows_of_text_with_the_blanks_cut() {
        let t = term(40, 10, "one\r\ntwo   \r\n\r\nfour\r\n$ ");
        assert_eq!(tail(&t, 3), "\nfour\n$");
        assert_eq!(tail(&t, 99), "one\ntwo\n\nfour\n$", "more than there is: everything, from the first row");
        assert_eq!(tail(&t, 1), "$");
        assert_eq!(tail(&t, 0), "");
    }

    #[test]
    fn rows_below_the_text_are_left_out_and_a_fresh_terminal_says_nothing() {
        let t = term(20, 24, "a\r\nb");
        assert_eq!(tail(&t, 3), "a\nb", "the 22 blank rows under the text are not part of it");
        assert_eq!(tail(&term(20, 24, ""), 5), "", "one empty row at the cursor");
        // The cursor below the last text keeps its row: a prompt line with nothing typed yet.
        assert_eq!(tail(&term(20, 24, "a\r\n\r\n"), 3), "a\n\n");
    }

    #[test]
    fn the_history_above_the_screen_is_part_of_the_tail_and_wide_characters_are_whole() {
        let mut out = String::new();
        for i in 0..8 {
            out += &format!("line {i}\r\n");
        }
        let t = term(30, 4, &(out + "last"));
        assert_eq!(tail(&t, 3), "line 6\nline 7\nlast", "scrolled off the screen, still in the scrollback");
        let w = term(10, 3, "a\u{4e2d}b");
        assert_eq!(tail(&w, 1), "a\u{4e2d}b", "the spacer cell after a wide character is not a space");
    }
}
