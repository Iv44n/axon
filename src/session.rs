//! The terminal session: `Term` plus its PTY thread, and the grid geometry for a window size.
//! Independent of the windowing backend; the listener is how the PTY thread reaches the UI.
use std::{collections::HashMap, sync::Arc};

use alacritty_terminal::{
    Term,
    event::{EventListener, WindowSize},
    event_loop::{EventLoop as PtyLoop, Msg, Notifier},
    grid::Dimensions,
    sync::FairMutex,
    term::{Config as TermConfig, Osc52},
    tty,
};

pub struct Size {
    pub cols: usize,
    pub rows: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize { self.rows }
    fn screen_lines(&self) -> usize { self.rows }
    fn columns(&self) -> usize { self.cols }
}

/// Terminal policy. OSC 52 may only write the clipboard: letting program output read it would hand
/// the clipboard to anything that can print to the terminal (a remote host, a `cat`ed file). Set
/// explicitly so it does not hang on a library default.
fn term_config() -> TermConfig {
    TermConfig { osc52: Osc52::OnlyCopy, ..TermConfig::default() }
}

/// Columns and rows that fit in a `w` x `h` pixel window with `pad` pixels of margin left, right and
/// below, and `top` pixels above.
pub fn grid_size(w: u32, h: u32, cw: u32, ch: u32, pad: u32, top: u32) -> (usize, usize) {
    let cols = (w.saturating_sub(2 * pad) / cw).max(2) as usize;
    let rows = (h.saturating_sub(top + pad) / ch).max(2) as usize;
    (cols, rows)
}

fn window_size(cols: usize, rows: usize, cw: u32, ch: u32) -> WindowSize {
    WindowSize { num_lines: rows as u16, num_cols: cols as u16, cell_width: cw as u16, cell_height: ch as u16 }
}

/// Starts the shell on a new PTY and its reader thread. `id` identifies the window to the PTY layer.
pub fn spawn<L: EventListener + Clone + Send + 'static>(
    listener: L,
    cols: usize,
    rows: usize,
    cw: u32,
    ch: u32,
    id: u64,
) -> (Arc<FairMutex<Term<L>>>, Notifier) {
    let term = Arc::new(FairMutex::new(Term::new(term_config(), &Size { cols, rows }, listener.clone())));
    tty::setup_env();
    let opts = tty::Options { shell: None, working_directory: None, drain_on_exit: false, env: HashMap::new() };
    let pty = tty::new(&opts, window_size(cols, rows, cw, ch), id).expect("pty");
    let lp = PtyLoop::new(term.clone(), listener, pty, false, false).expect("pty event loop");
    let notifier = Notifier(lp.channel());
    lp.spawn();
    (term, notifier)
}

/// Tells the terminal and the PTY about a new grid size.
pub fn resize<L: EventListener>(term: &FairMutex<Term<L>>, pty: &Notifier, cols: usize, rows: usize, cw: u32, ch: u32) {
    term.lock().resize(Size { cols, rows });
    let _ = pty.0.send(Msg::Resize(window_size(cols, rows, cw, ch)));
}
