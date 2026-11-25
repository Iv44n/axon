//! The application on the own Wayland client: the terminal session, the CPU renderer and `wl.rs`,
//! in one blocking `poll` loop with no windowing or GPU library. It handles the keyboard (layouts
//! by `xkb.rs`, client-side repeat), the pointer (window chrome, selection, wheel) and the clipboard.
use std::{
    collections::VecDeque,
    os::fd::{AsFd, OwnedFd},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use alacritty_terminal::{
    Term,
    event::{Event as TermEvent, EventListener, Notify, WindowSize},
    event_loop::Notifier,
    grid::Scroll,
    index::{Column, Point, Side},
    selection::{Selection, SelectionType},
    sync::FairMutex,
    term::{TermMode, viewport_to_point},
};
use rustix::{
    event::{EventfdFlags, PollFd, PollFlags, Timespec, eventfd, poll},
    io::{read, write},
};

use crate::{
    bar,
    side,
    history::{self, History, Hook},
    i18n::{self, Str, tr},
    cell::pack,
    chrome::{self, EDGE_BOTTOM, EDGE_LEFT, EDGE_RIGHT, EDGE_TOP, Hit, Ui},
    config::{self, Chip, Config, FontCfg},
    context::{self, Context, Probe},
    input, mouse, palette, session,
    settings::{self, Effect, Settings},
    soft::{Dock, Soft},
    view::View,
    wl::{Event, Wl, cursor},
    xkb::{Key, Xkb},
};

/// How long a redraw may wait for the compositor's frame callback or buffer release before axon
/// assumes it was lost (suspend and resume, a hidden window) and draws anyway.
const STALL: Duration = Duration::from_millis(250);
/// The prompt chips are re-read at most this often while the terminal prints.
const CONTEXT_EVERY: Duration = Duration::from_millis(500);
/// `BTN_LEFT` from `linux/input-event-codes.h`.
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;
/// Two presses on the title bar closer than this many milliseconds maximize or restore.
const DOUBLE_CLICK_MS: u32 = 400;
/// Clicks on the grid closer than this select a word, then a line.
const MULTI_CLICK_MS: u32 = 350;
/// Lines per wheel notch.
const WHEEL_LINES: f32 = 3.0;

/// Keysyms the app's own shortcuts look at.
const KS_PAGE_UP: u32 = 0xff55;
const KS_PAGE_DOWN: u32 = 0xff56;
const KS_HOME: u32 = 0xff50;
const KS_END: u32 = 0xff57;
const KS_KP_ADD: u32 = 0xffab;
const KS_KP_SUBTRACT: u32 = 0xffad;
const KS_KP_0: u32 = 0xffb0;

fn cursor_for(hit: Hit) -> u32 {
    match hit {
        Hit::Close | Hit::Minimize | Hit::Maximize | Hit::Settings | Hit::SideBtn => cursor::POINTER,
        Hit::Edge(e) => match e {
            EDGE_TOP => cursor::N_RESIZE,
            EDGE_BOTTOM => cursor::S_RESIZE,
            EDGE_LEFT => cursor::W_RESIZE,
            EDGE_RIGHT => cursor::E_RESIZE,
            e if e == EDGE_TOP | EDGE_LEFT => cursor::NW_RESIZE,
            e if e == EDGE_TOP | EDGE_RIGHT => cursor::NE_RESIZE,
            e if e == EDGE_BOTTOM | EDGE_LEFT => cursor::SW_RESIZE,
            _ => cursor::SE_RESIZE,
        },
        Hit::Title | Hit::Side | Hit::Prefs => cursor::DEFAULT,
        Hit::Client => cursor::TEXT,
    }
}

/// How the PTY thread reaches the loop: events go in a queue and an eventfd wakes `poll`.
#[derive(Clone)]
struct Proxy {
    wake: Arc<OwnedFd>,
    pending: Arc<AtomicBool>,
    queue: Arc<Mutex<VecDeque<TermEvent>>>,
    /// What the shell's hook said, with the moment the PTY thread read it: commands are timed there.
    hooks: Arc<Mutex<VecDeque<(Instant, Hook)>>>,
}

impl Proxy {
    fn wake(&self) {
        let _ = write(self.wake.as_ref(), &1u64.to_ne_bytes());
    }
}

impl EventListener for Proxy {
    fn send_event(&self, e: TermEvent) {
        match e {
            // Coalesce: at most one wake in flight, even if the PTY spews megabytes.
            TermEvent::Wakeup => {
                if !self.pending.swap(true, Ordering::AcqRel) {
                    self.wake();
                }
            }
            // The shell's hook talks through window titles: they are timed and kept out of the title.
            TermEvent::Title(t) if history::parse(&t).is_some() => {
                if let Some(h) = history::parse(&t) {
                    self.hooks.lock().unwrap().push_back((Instant::now(), h));
                    self.wake();
                }
            }
            e => {
                self.queue.lock().unwrap().push_back(e);
                self.wake();
            }
        }
    }
}

/// When the input bar's chips are re-read, and the thread that does it.
struct Refresh {
    /// A refresh is wanted from this moment on.
    due: Option<Instant>,
    last: Instant,
    running: Arc<AtomicBool>,
    /// Where the thread leaves its result.
    done: Arc<Mutex<Option<Context>>>,
    /// The time chip needs a repaint at this moment (the minute changes).
    clock_at: Option<Instant>,
}

/// A key being held: sent again at `at`, then every `1 / rate` seconds.
struct Repeat {
    code: u32,
    key: Key,
    at: Instant,
}

/// One window (a tab): its terminal session and everything that belongs to that shell. The app draws
/// and types into one at a time (`App::cur`); the others keep running and are listed in the side panel.
struct Win {
    term: Arc<FairMutex<Term<Proxy>>>,
    pty: Notifier,
    proxy: Proxy,
    view: View,
    /// What the prompt chips say now, whether that changed since the last frame, and what each prompt said.
    ctx: Context,
    history: History,
    ctx_dirty: bool,
    chips: bar::Chips,
    /// The shell's process id, the file it reports to, the flag that turns the chips on, the refresh in progress.
    shell: u32,
    report: Option<PathBuf>,
    flag: Option<PathBuf>,
    refresh: Refresh,
    /// The title the program set (empty: the directory's name is used) and whether it printed while hidden.
    title: String,
    unread: bool,
    /// The number the window was given when it opened, and the group it is in (an index into `App::groups`).
    id: u64,
    group: Option<usize>,
}

/// A group of windows: a name, one of the eight colours, and whether the panel shows its windows.
struct Group {
    name: String,
    color: u8,
    collapsed: bool,
}

impl Drop for Win {
    /// The files the shell and this window talk through belong to this window; the shell is told to end.
    fn drop(&mut self) {
        for f in [&self.report, &self.flag].into_iter().flatten() {
            let _ = std::fs::remove_file(f);
        }
        let _ = self.pty.0.send(alacritty_terminal::event_loop::Msg::Shutdown);
    }
}

struct App {
    wl: Wl,
    soft: Soft,
    /// The windows, in the order of the side panel, and the one on screen.
    wins: Vec<Win>,
    cur: usize,
    /// What wakes `poll`: the PTY threads write to it.
    wake: Arc<OwnedFd>,
    /// The number the next window gets (for its report file and its PTY).
    next_win: u64,
    /// What the pointer is over in the panel of windows, and whether the panel has to be painted again.
    side_hover: Option<side::Part>,
    side_dirty: bool,
    side_scratch: Vec<u8>,
    /// The groups, in the order the panel shows them, how many were ever made (for their names), and what
    /// the panel lists now (rebuilt whenever the windows or the groups change).
    groups: Vec<Group>,
    groups_made: usize,
    entries: Vec<side::Entry>,
    xkb: Option<Xkb>,
    ui: Ui,
    hover: Hit,
    /// Window size in pixels, grid size, cell size.
    w: u32,
    h: u32,
    cols: usize,
    rows: usize,
    cw: u32,
    ch: u32,
    /// Current font size (zoomed) and the configured one, which Ctrl+0 returns to.
    base_px: f32,
    cfg_px: f32,
    /// The font settings the renderer has now (the file's, the settings panel's draft or the zoom): rebuilding the
    /// glyphs only when they differ keeps a click on a switch from dropping every tile.
    shown_font: FontCfg,
    /// The settings as loaded (file and environment), and the glass opacity on screen.
    cfg: Config,
    opacity: f32,
    /// Since when a redraw has been waiting for the compositor (a frame callback or a free buffer).
    waiting: Option<Instant>,
    /// The settings panel, while it is open (docked beside the grid), whether it has to be painted again, and
    /// when what it changed is written to the file (a moment after the last change, or at once when a gesture ends).
    panel: Option<Box<Settings>>,
    prefs_dirty: bool,
    save_at: Option<Instant>,
    dirty: bool,
    quit: bool,
    mouse: (f32, f32),
    selecting: bool,
    /// The button a program is being told about (mouse reporting), and the cell last reported.
    held: Option<u8>,
    reported: (usize, usize),
    scroll_acc: f32,
    /// Time of the last press on the title bar, and of the last press on the grid with its click count.
    title_click: u32,
    grid_click: (u32, u8),
    repeat: Option<Repeat>,
    /// Key repeat: keys per second (0 = off) and initial delay, as the compositor says.
    rate: i32,
    delay: i32,
}

/// The panel's box `(x, y, w, h)` in window pixels, when there is one.
fn side_box(soft: &Soft) -> Option<(i32, i32, i32, i32)> {
    let (x0, y0, x1, y1) = soft.side_rect()?;
    let (ox, oy) = soft.origin();
    Some((x0 as i32 - ox as i32, y0 as i32 - oy as i32, (x1 - x0) as i32, (y1 - y0) as i32))
}

/// Whether a program set a title of its own: the one a shell sets by default says `user@host: path`, which
/// the window's directory says better.
fn own_title(t: &str) -> bool {
    !t.is_empty() && !t.split_once(':').is_some_and(|(host, _)| host.contains('@') && !host.contains(' '))
}

/// The directory of a window's shell, in short form (`~/lab/axon`).
fn win_place(w: &Win) -> String {
    let cwd = std::fs::read_link(format!("/proc/{}/cwd", w.shell)).unwrap_or_default();
    let home = std::env::var("HOME").ok();
    context::tidy_dir(&cwd.to_string_lossy(), home.as_deref())
}

/// The name of a window: what its program set, else the name of its directory.
fn win_title(w: &Win) -> String {
    if own_title(&w.title) {
        return w.title.clone();
    }
    let cwd = std::fs::read_link(format!("/proc/{}/cwd", w.shell)).unwrap_or_default();
    if std::env::var_os("HOME").map(PathBuf::from).as_deref() == Some(cwd.as_path()) {
        return "~".into();
    }
    cwd.file_name().map_or_else(|| "/".into(), |n| n.to_string_lossy().into_owned())
}

/// The title bar between the window buttons and the settings button, where the tabs go (window pixels).
fn tabs_span(w: u32) -> (i32, i32) {
    (chrome::TITLE_CLEAR as i32 - 8, w as i32 - 50)
}

/// The two panels beside the grid: the windows' (as wide as the settings say, none when it is hidden or the windows
/// are tabs at the top) and the settings' (`prefs_w` wide, 0 when closed). The reference puts the settings on the
/// edge opposite the windows' panel: `.main` is a row of windows, workspace, settings, and with the windows at the
/// right the settings go to the left. They never share an edge, so nothing has to say how they would stack.
fn docks(w: &config::WindowsCfg, prefs_w: u32) -> (Dock, Dock) {
    let right = w.tabs == config::TabsPos::Right;
    let side = if w.side && w.tabs != config::TabsPos::Top { w.side_w } else { 0 };
    (Dock { w: side, left: !right }, Dock { w: prefs_w, left: right })
}

/// The list of windows as the panel and the tabs show it.
fn items(wins: &[Win], cur: usize) -> Vec<side::Item> {
    wins.iter().enumerate().map(|(i, w)| side::Item { title: win_title(w), selected: i == cur, unread: w.unread }).collect()
}

/// The groups as the panel shows them.
fn group_items(groups: &[Group], wins: &[Win]) -> Vec<side::GroupItem> {
    groups.iter().enumerate().map(|(g, x)| side::GroupItem { name: x.name.clone(), color: x.color, collapsed: x.collapsed, count: wins.iter().filter(|w| w.group == Some(g)).count() }).collect()
}

/// Paints the panel of windows over its cleared box.
#[allow(clippy::too_many_arguments)]
fn paint_side(soft: &Soft, fb: &mut [u32], wins: &[Win], groups: &[Group], entries: &[side::Entry], cur: usize, right: bool, hover: Option<side::Part>, scratch: &mut Vec<u8>) {
    let (Some(rect), Some(b)) = (soft.side_rect(), side_box(soft)) else { return };
    soft.clear_rect(fb, rect);
    side::paint(soft, fb, b, !right, &tr(Str::Windows), entries, &group_items(groups, wins), &items(wins, cur), hover, scratch);
}

/// Paints the tabs in the title bar, over the bar cleared between the buttons.
fn paint_tabs(soft: &Soft, fb: &mut [u32], wins: &[Win], cur: usize, w: u32, hover: Option<side::Part>, scratch: &mut Vec<u8>) {
    let (ox, oy) = soft.origin();
    let (x0, x1) = tabs_span(w);
    soft.clear_rect(fb, (ox + x0 as usize, oy, ox + x1 as usize, oy + chrome::TITLE_H));
    side::paint_top(soft, fb, (x0, x1), &items(wins, cur), hover, scratch);
}

/// Runs the app. Returns false, with nothing shown, if this backend cannot start here (no Wayland
/// socket, no `libxkbcommon`): the caller falls back to `winit`.
pub fn run() -> bool {
    let app = match App::new() {
        Ok(app) => app,
        Err(e) => {
            eprintln!("axon: {e}; falling back to winit");
            return false;
        }
    };
    if let Err(e) = app.run() {
        eprintln!("axon: {e}");
        std::process::exit(1);
    }
    true
}

impl Win {
    /// A new window with its own shell. `n` numbers it for the files it reports through.
    fn spawn(wake: &Arc<OwnedFd>, n: u64, (cols, rows, cw, ch): (usize, usize, u32, u32), cursor: config::CursorKind, chips: bool, dir: Option<PathBuf>) -> Win {
        let proxy = Proxy { wake: wake.clone(), pending: Arc::new(AtomicBool::new(false)), queue: Arc::default(), hooks: Arc::default() };
        // The files the shell and axon talk through (`context.rs`): what the shell reports, and the flag
        // that says the prompt chips are on.
        let (report, flag) = (context::report_path(std::process::id(), n), context::flag_path(std::process::id(), n));
        let shell_path = std::env::var("SHELL").unwrap_or_default();
        let inherited = std::env::var("PROMPT_COMMAND").ok();
        let env = context::shell_env(&shell_path, report.as_deref(), flag.as_deref(), inherited.as_deref()).into_iter().collect();
        let (term, pty, shell) = session::spawn(proxy.clone(), cols, rows, cw, ch, n, env, cursor, dir);
        Win {
            term,
            pty,
            proxy,
            view: View::new(),
            ctx: Context::default(),
            history: History::default(),
            ctx_dirty: true,
            chips: bar::Chips::default(),
            shell,
            report,
            flag,
            refresh: Refresh { due: chips.then(Instant::now), last: Instant::now(), running: Arc::new(AtomicBool::new(false)), done: Arc::default(), clock_at: None },
            title: String::new(),
            unread: false,
            id: n,
            group: None,
        }
    }
}

impl App {
    fn new() -> std::io::Result<App> {
        let cfg = config::load();
        i18n::set(cfg.lang.resolve_here());
        let base_px = cfg.font.size;
        let shown_font = cfg.font.clone();
        let opacity = cfg.opacity.unwrap_or(chrome::OPACITY);
        let mut soft = Soft::new(&cfg.font, opacity);
        soft.cursor_color = pack(palette::CURSOR.r, palette::CURSOR.g, palette::CURSOR.b, 255);
        soft.set_title("axon", "");
        let (cw, ch) = soft.cell();

        let mut wl = Wl::connect("axon", "axon")?;
        let mut xkb = Some(Xkb::load().ok_or_else(|| std::io::Error::other("libxkbcommon.so.0 not found, the keyboard would not work"))?);
        // The first configure decides the size; 0 x 0 means the compositor has no opinion.
        let (mut w, mut h) = (1000, 640);
        let mut ui = Ui { status: cfg.status_bar, ..Ui::IDLE };
        let (mut rate, mut delay) = (25, 600);
        'first: loop {
            wl.wait(None)?;
            wl.read_events()?;
            while let Some(ev) = wl.next_event() {
                match ev {
                    Event::Configure { w: nw, h: nh, square } => {
                        if nw > 0 && nh > 0 {
                            (w, h) = (nw as u32, nh as u32);
                        }
                        ui.maximized = square;
                        break 'first;
                    }
                    Event::Focus(f) => ui.focused = f,
                    Event::Close => std::process::exit(0),
                    Event::Keymap(text) => {
                        if let Some(x) = xkb.as_mut() {
                            x.set_keymap(&text);
                        }
                    }
                    Event::RepeatInfo { rate: r, delay: d } => (rate, delay) = (r, d),
                    _ => {}
                }
            }
        }

        let wake = Arc::new(eventfd(0, EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK)?);
        let (side_dock, prefs_dock) = docks(&cfg.windows, 0);
        let side = side_dock.w;
        soft.set_docks(side_dock, prefs_dock);
        ui.side = match (cfg.windows.tabs, cfg.windows.side) {
            (config::TabsPos::Top, _) => 0,
            (_, true) => 2,
            (_, false) => 1,
        };
        let (cols, rows) = session::grid_size(w.saturating_sub(side), h, cw, ch, soft.pad, soft.top, soft.bottom);
        if let Some(dir) = context::report_path(std::process::id(), 0).as_deref().and_then(std::path::Path::parent) {
            context::remove_stale_reports(dir);
        }
        let cfg_chips = cfg.input.inline && !cfg.input.chips.is_empty();
        let first = Win::spawn(&wake, 0, (cols, rows, cw, ch), cfg.input.cursor, cfg_chips, None);
        let mut app = App {
            wl,
            soft,
            wins: vec![first],
            cur: 0,
            wake,
            next_win: 1,
            side_hover: None,
            side_dirty: true,
            side_scratch: Vec::new(),
            groups: Vec::new(),
            groups_made: 0,
            entries: Vec::new(),
            xkb,
            ui,
            hover: Hit::Client,
            w,
            h,
            cols,
            rows,
            cw,
            ch,
            base_px,
            cfg_px: base_px,
            shown_font,
            cfg,
            opacity,
            panel: None,
            prefs_dirty: false,
            save_at: None,
            dirty: true,
            waiting: None,
            quit: false,
            mouse: (0.0, 0.0),
            selecting: false,
            held: None,
            reported: (usize::MAX, 0),
            scroll_acc: 0.0,
            title_click: 0,
            grid_click: (0, 0),
            repeat: None,
            rate,
            delay,
        };
        app.set_geometry();
        app.rebuild_side();
        app.sync_flag();
        // A debugging aid: start with this many windows (screenshots, memory measurements).
        for _ in 1..std::env::var("AXON_WINDOWS").ok().and_then(|n| n.parse::<usize>().ok()).unwrap_or(1).min(9) {
            app.new_win();
        }
        // Another aid: the first two windows start in one group.
        if std::env::var_os("AXON_GROUPS").is_some() && app.wins.len() > 1 {
            app.new_group();
            app.wins[1].group = Some(0);
            app.rebuild_side();
        }
        app.switch_to(0);
        // A debugging aid: start with the settings panel open (screenshots, memory measurements).
        if std::env::var_os("AXON_SETTINGS").is_some() {
            app.toggle_settings();
        }
        Ok(app)
    }

    fn run(mut self) -> std::io::Result<()> {
        while !self.quit {
            self.draw();
            self.wl.flush()?;

            // Block until the compositor, the PTY thread or a held key needs us; and, when a redraw is
            // stuck waiting for the compositor, only until it is clear that it will not answer.
            let now = Instant::now();
            let timeout = [self.repeat.as_ref().map(|r| r.at), self.waiting.map(|t| t + STALL), self.wins[self.cur].refresh.due, self.wins[self.cur].refresh.clock_at, self.save_at]
                .into_iter()
                .flatten()
                .min()
                .map(|at| {
                    let left = at.saturating_duration_since(now);
                    Timespec { tv_sec: left.as_secs() as i64, tv_nsec: left.subsec_nanos() as i64 }
                });
            let wake = self.wake.clone();
            let wl_fd = self.wl.fd();
            let mut fds = [PollFd::new(&wl_fd, PollFlags::IN), PollFd::new(wake.as_ref(), PollFlags::IN)];
            match poll(&mut fds, timeout.as_ref()) {
                Ok(_) | Err(rustix::io::Errno::INTR) => {}
                Err(e) => return Err(e.into()),
            }
            let (wl_ready, wake_ready) = (fds[0].revents().contains(PollFlags::IN), fds[1].revents().contains(PollFlags::IN));
            if wake_ready {
                let _ = read(wake.as_fd(), &mut [0u8; 8]);
                self.dirty = true;
                // The terminal printed something: the directory or the branch may have changed.
                if self.wins[self.cur].proxy.pending.load(Ordering::Acquire) {
                    self.context_due(Instant::now());
                }
            }
            if wl_ready {
                self.wl.read_events()?;
            }
            self.term_events();
            while let Some(ev) = self.wl.next_event() {
                self.wl_event(ev);
            }
            self.tick_repeat();
            self.tick_context();
            self.tick_save();
            if self.waiting.is_some_and(|t| t.elapsed() >= STALL) {
                self.wl.give_up_waiting();
                self.waiting = None;
            }
        }
        // What the panel changed last is written even if the window closes a moment later.
        self.flush_save();
        Ok(())
    }

    /// Draws when something changed and the compositor has shown the previous frame.
    fn draw(&mut self) {
        if !self.dirty {
            self.waiting = None;
            return;
        }
        if self.wl.frame_pending() {
            self.waiting.get_or_insert_with(Instant::now);
            return;
        }
        // The buffer is the window plus the margin its shadow is drawn in.
        let (ml, mt, mb) = chrome::margins(self.ui.maximized);
        let (w, h) = (self.w as usize + 2 * ml, self.h as usize + mt + mb);
        let chips_on = self.chips_on();
        let Some(fb) = self.wl.framebuffer(w, h) else {
            self.waiting.get_or_insert_with(Instant::now);
            return;
        };
        self.waiting = None;
        self.wins[self.cur].proxy.pending.store(false, Ordering::Release);
        let branch = if chips_on { self.wins[self.cur].ctx.get(Chip::Branch) } else { "" };
        let shell = std::env::var("SHELL").ok();
        let shell = shell.as_deref().and_then(|s| s.rsplit('/').next()).unwrap_or("shell");
        self.soft.set_status(shell, branch, self.cols, self.rows);
        self.soft.set_ui(self.ui);
        let win = &mut self.wins[self.cur];
        let mut t = win.term.lock();
        let chips = &self.cfg.input.chips;
        win.chips.frame(chips_on, win.ctx_dirty, &win.history, chips, &mut t, &mut win.view, &mut self.soft, fb, (w, h), self.ui.focused);
        drop(t);
        win.ctx_dirty = false;
        let top = self.cfg.windows.tabs == config::TabsPos::Top;
        if self.soft.last_full || self.side_dirty || (self.soft.title_painted && top) {
            if top {
                paint_tabs(&self.soft, fb, &self.wins, self.cur, self.w, self.side_hover, &mut self.side_scratch);
            } else {
                let right = self.cfg.windows.tabs == config::TabsPos::Right;
                paint_side(&self.soft, fb, &self.wins, &self.groups, &self.entries, self.cur, right, self.side_hover, &mut self.side_scratch);
            }
            self.side_dirty = false;
        }
        // The settings panel is painted in its own box (cleared first) when the frame is whole again or something
        // in it changed, a fade included: the grid beside it is repainted row by row as usual.
        let mut animating = false;
        if let (Some(p), Some(rect)) = (self.panel.as_mut(), self.soft.prefs_rect()) {
            let was = p.animating();
            animating = p.step(Instant::now());
            if self.soft.last_full || self.prefs_dirty || was || animating {
                self.soft.clear_rect(fb, rect);
                p.draw(fb, w, h, (ml, mt));
            }
        }
        self.prefs_dirty = false;
        self.wl.present();
        // A fade in progress asks for the next frame itself; at rest nothing does (idle is 0 % CPU).
        self.dirty = animating;
    }

    /// Events from the terminals' threads, window by window. The queue lock is not held while one is
    /// handled: the PTY thread sends events while holding the terminal lock, and handling may take it.
    fn term_events(&mut self) {
        let mut gone = Vec::new();
        for i in 0..self.wins.len() {
            if i != self.cur && self.wins[i].proxy.pending.swap(false, Ordering::AcqRel) {
                // A window out of sight printed: say so in the panel; the grid is read when it comes back.
                if !self.wins[i].unread {
                    self.wins[i].unread = true;
                    self.side_dirty = true;
                    self.dirty = true;
                }
            }
            loop {
                let Some((at, hook)) = self.wins[i].proxy.hooks.lock().unwrap().pop_front() else { break };
                let w = &mut self.wins[i];
                match hook {
                    Hook::Start => w.history.start(at),
                    Hook::Prompt { n, status } => {
                        w.history.prompt(n, status, at, &w.ctx);
                        self.dirty = true;
                    }
                }
            }
            loop {
                let Some(e) = self.wins[i].proxy.queue.lock().unwrap().pop_front() else { break };
                match e {
                    TermEvent::Title(t) => {
                        self.wins[i].title = t;
                        self.titled(i);
                    }
                    TermEvent::ResetTitle => {
                        self.wins[i].title.clear();
                        self.titled(i);
                    }
                    TermEvent::PtyWrite(t) => self.wins[i].pty.notify(t.into_bytes()),
                    TermEvent::ClipboardStore(_, t) => self.wl.set_clipboard(&t),
                    // OSC 52 reads are refused (`session::term_config`): output must not read the clipboard.
                    TermEvent::ClipboardLoad(..) => {}
                    TermEvent::ColorRequest(c, f) => {
                        let rgb = self.wins[i].term.lock().colors()[c].unwrap_or_else(|| palette::by_index(c));
                        self.wins[i].pty.notify(f(rgb).into_bytes());
                    }
                    TermEvent::TextAreaSizeRequest(f) => {
                        let ws = WindowSize {
                            num_lines: self.rows as u16,
                            num_cols: self.cols as u16,
                            cell_width: self.cw as u16,
                            cell_height: self.ch as u16,
                        };
                        self.wins[i].pty.notify(f(ws).into_bytes());
                    }
                    TermEvent::Exit | TermEvent::ChildExit(_) => gone.push(i),
                    _ => {}
                }
            }
        }
        // A shell that ends says so twice (`ChildExit` and `Exit`): one close each.
        gone.sort_unstable();
        gone.dedup();
        for i in gone.into_iter().rev() {
            self.close_win(i);
        }
    }

    /// The window `i` got a new title or moved: the title bar shows the current one.
    fn titled(&mut self, i: usize) {
        if i == self.cur {
            let (name, place) = (win_title(&self.wins[i]), win_place(&self.wins[i]));
            self.wl.set_title(&format!("{name} {place}"));
            self.soft.set_title(&name, &place);
        }
        self.side_dirty = true;
        self.dirty = true;
    }

    // --------------------------------------------------------- side panel --

    /// Leaves the panels' room beside the grid, at the sides the settings say, and tells the settings panel where
    /// its box is.
    fn apply_inset(&mut self) {
        let prefs_w = if self.panel.is_some() { settings::panel_width(self.w) } else { 0 };
        let (side, prefs) = docks(&self.cfg.windows, prefs_w);
        self.soft.set_ui(self.ui);
        self.soft.set_docks(side, prefs);
        self.soft.set_tabs_top(self.cfg.windows.tabs == config::TabsPos::Top);
        if let (Some(p), Some(b)) = (self.panel.as_mut(), self.soft.prefs_box((self.w, self.h))) {
            p.place(b, prefs.left);
            self.prefs_dirty = true;
        }
        self.ui.side = match (self.cfg.windows.tabs, self.cfg.windows.side) {
            (config::TabsPos::Top, _) => 0,
            (_, true) => 2,
            (_, false) => 1,
        };
    }

    fn side_box(&self) -> Option<(i32, i32, i32, i32)> {
        side_box(&self.soft)
    }

    fn tabs_on_top(&self) -> bool {
        self.cfg.windows.tabs == config::TabsPos::Top
    }

    fn side_part_at(&self, x: f32, y: f32) -> Option<side::Part> {
        if self.tabs_on_top() {
            let (x0, x1) = tabs_span(self.w);
            if y < 0.0 || y >= chrome::TITLE_H as f32 - 1.0 {
                return None;
            }
            return side::Layout::Top { x0, x1, n: self.wins.len() }.hit(x, y);
        }
        let (bx, by, bw, bh) = self.side_box()?;
        if x < bx as f32 || x >= (bx + bw) as f32 || y < by as f32 || y >= (by + bh) as f32 {
            return None;
        }
        side::Layout::Side { x0: bx, y0: by, w: bw, entries: &self.entries }.hit(x, y)
    }

    fn toggle_side(&mut self) {
        self.cfg.windows.side = !self.cfg.windows.side;
        self.side_hover = None;
        if let Some(p) = self.panel.as_mut() {
            p.follow_windows(&self.cfg.windows);
            self.prefs_dirty = true;
        }
        self.relayout();
    }

    // ------------------------------------------------------------ windows --

    /// Opens a window with a shell of its own and goes to it.
    fn new_win(&mut self) {
        let n = self.next_win;
        self.next_win += 1;
        let cfg_chips = self.chips_on();
        // The new shell starts where the one you were in is.
        let dir = std::fs::read_link(format!("/proc/{}/cwd", self.wins[self.cur].shell)).ok();
        let win = Win::spawn(&self.wake, n, (self.cols, self.rows, self.cw, self.ch), self.cfg.input.cursor, cfg_chips, dir);
        self.wins.push(win);
        self.rebuild_side();
        self.switch_to(self.wins.len() - 1);
        self.sync_flag();
    }

    /// Closes window `i` (its shell is told to end); the last one closing quits.
    fn close_win(&mut self, i: usize) {
        if i >= self.wins.len() {
            return;
        }
        if self.wins.len() == 1 {
            // The last one: the app ends (its files and shell are cleaned up when it drops).
            self.quit = true;
            return;
        }
        self.wins.remove(i);
        if i < self.cur || self.cur >= self.wins.len() {
            self.cur = self.cur.saturating_sub(1).min(self.wins.len() - 1);
        }
        self.prune_groups();
        self.rebuild_side();
        self.switch_to(self.cur);
    }

    // ------------------------------------------------------------- groups --

    /// Puts the windows in the order the panel shows them (a group's windows together, groups in order, the
    /// windows in none last: so `Alt+1..9` and `Alt+[` follow what is on screen) and lists them.
    fn rebuild_side(&mut self) {
        let id = self.wins[self.cur].id;
        self.wins.sort_by_key(|w| w.group.unwrap_or(usize::MAX));
        self.cur = self.wins.iter().position(|w| w.id == id).unwrap_or(0);
        let of: Vec<Option<usize>> = self.wins.iter().map(|w| w.group).collect();
        let folded: Vec<bool> = self.groups.iter().map(|g| g.collapsed).collect();
        self.entries = side::entries(&of, &folded);
        self.side_hover = None;
        self.side_dirty = true;
        self.dirty = true;
    }

    /// Drops the groups that no window is in, and renumbers the others in the windows.
    fn prune_groups(&mut self) {
        let mut g = 0;
        while g < self.groups.len() {
            if self.wins.iter().any(|w| w.group == Some(g)) {
                g += 1;
                continue;
            }
            self.groups.remove(g);
            for w in &mut self.wins {
                w.group = w.group.map(|x| if x > g { x - 1 } else { x });
            }
        }
    }

    /// A new group, named and coloured for the next in line, around the window on screen.
    fn new_group(&mut self) {
        self.groups_made += 1;
        let color = (self.groups.len() % 8) as u8;
        self.groups.push(Group { name: format!("{} {}", tr(Str::GroupName), self.groups_made), color, collapsed: false });
        self.wins[self.cur].group = Some(self.groups.len() - 1);
        self.prune_groups();
        self.rebuild_side();
    }

    /// Takes the window on screen out of its group.
    fn ungroup(&mut self) {
        if self.wins[self.cur].group.take().is_some() {
            self.prune_groups();
            self.rebuild_side();
        }
    }

    /// Folds or unfolds group `g` in the panel.
    fn fold(&mut self, g: usize) {
        if let Some(group) = self.groups.get_mut(g) {
            group.collapsed = !group.collapsed;
            self.rebuild_side();
        }
    }

    /// Shows window `i`: what was in the middle of a pointer gesture or a key repeat is dropped, and the
    /// whole frame is painted again from the other terminal.
    fn switch_to(&mut self, i: usize) {
        let i = i.min(self.wins.len() - 1);
        (self.selecting, self.held, self.repeat) = (false, None, None);
        self.cur = i;
        let w = &mut self.wins[i];
        w.unread = false;
        w.ctx_dirty = true;
        w.refresh.due = Some(Instant::now());
        self.titled(i);
        self.soft.invalidate();
        self.side_dirty = true;
        self.dirty = true;
    }

    fn wl_event(&mut self, ev: Event) {
        match ev {
            Event::Configure { w, h, square } => {
                let was = (self.w, self.h, self.ui.maximized);
                self.ui.maximized = square;
                if w > 0 && h > 0 && (w as u32, h as u32) != (self.w, self.h) {
                    (self.w, self.h) = (w as u32, h as u32);
                    self.relayout();
                }
                if (self.w, self.h, self.ui.maximized) != was {
                    self.set_geometry();
                }
                self.dirty = true;
            }
            Event::Focus(f) => {
                self.ui.focused = f;
                if !f {
                    self.repeat = None;
                }
                if self.wins[self.cur].term.lock().mode().contains(TermMode::FOCUS_IN_OUT) {
                    self.wins[self.cur].pty.notify(if f { &b"\x1b[I"[..] } else { &b"\x1b[O"[..] });
                }
                self.dirty = true;
            }
            Event::Close => self.quit = true,
            Event::FrameDone => {}
            Event::Keymap(text) => {
                if let Some(x) = self.xkb.as_mut() {
                    x.set_keymap(&text);
                }
            }
            Event::Modifiers { depressed, latched, locked, group } => {
                if let Some(x) = self.xkb.as_mut() {
                    x.update_mods(depressed, latched, locked, group);
                }
            }
            Event::RepeatInfo { rate, delay } => (self.rate, self.delay) = (rate, delay),
            Event::KeyboardLeave => self.repeat = None,
            Event::Key { key, pressed } => self.on_key(key, pressed),
            Event::Scroll { pixels, clicks } => self.on_scroll(pixels, clicks),
            Event::PointerMove { x, y } => self.on_pointer_move(x, y),
            Event::PointerLeave => {
                self.hover = Hit::Client;
                if let Some(p) = self.panel.as_mut() {
                    let e = p.pointer(-1.0, -1.0);
                    self.panel_effect(e);
                }
                if self.side_hover.take().is_some() {
                    self.side_dirty = true;
                    self.dirty = true;
                }
                if self.ui.hover != 0 || self.ui.down {
                    (self.ui.hover, self.ui.down) = (0, false);
                    self.dirty = true;
                }
            }
            Event::PointerButton { button, pressed, time } => {
                let b = match button {
                    BTN_LEFT => 0,
                    BTN_MIDDLE => 1,
                    BTN_RIGHT => 2,
                    _ => return,
                };
                if self.panel.is_some() && b == 0 && self.panel_button(pressed) {
                    return;
                }
                if !self.report_button(b, pressed) && b == 0 {
                    self.on_button(pressed, time);
                }
            }
        }
    }

    // ------------------------------------------------------------- layout --

    /// Tells the compositor where the window is inside the buffer: the shadow around it is not
    /// part of it (placement, tiling, maximizing) and only a thin band of it takes the pointer.
    fn set_geometry(&mut self) {
        let (ml, mt, _) = chrome::margins(self.ui.maximized);
        let grab = if self.ui.maximized { None } else { Some(chrome::EDGE_OUT as i32) };
        self.wl.set_geometry(ml as i32, mt as i32, self.w as i32, self.h as i32, grab);
        self.set_blur();
    }

    /// Asks the compositor to blur behind the window when the settings say so (its rounded outline).
    fn set_blur(&mut self) {
        if !self.cfg.blur {
            return self.wl.set_blur(&[]);
        }
        let (ml, mt, _) = chrome::margins(self.ui.maximized);
        let radius = if self.ui.maximized { 0 } else { chrome::RADIUS as i32 };
        let (rects, n) = chrome::blur_strips(ml as i32, mt as i32, self.w as i32, self.h as i32, radius);
        self.wl.set_blur(&rects[..n]);
    }

    /// Grid size for the window size, after a resize or a new font size.
    fn relayout(&mut self) {
        self.apply_inset();
        let (il, ir) = self.soft.inset();
        let (c, r) = session::grid_size(self.w.saturating_sub(il + ir), self.h, self.cw, self.ch, self.soft.pad, self.soft.top, self.soft.bottom);
        if (c, r) != (self.cols, self.rows) {
            (self.cols, self.rows) = (c, r);
            for w in &self.wins {
                session::resize(&w.term, &w.pty, c, r, self.cw, self.ch);
            }
        }
        for w in &mut self.wins {
            w.ctx_dirty = true;
        }
        self.side_dirty = true;
        self.dirty = true;
    }

    // ------------------------------------------------------- prompt chips --

    /// Whether the chips are shown: asked for and with at least one chip to show.
    fn chips_on(&self) -> bool {
        self.cfg.input.inline && !self.cfg.input.chips.is_empty()
    }

    /// Puts the flag file where the shell's hook looks: it exists while the chips are on.
    fn sync_flag(&self) {
        let on = self.chips_on();
        for f in self.wins.iter().filter_map(|w| w.flag.as_ref()) {
            if on {
                let _ = std::fs::write(f, b"");
            } else {
                let _ = std::fs::remove_file(f);
            }
        }
    }

    /// Asks for the chips to be read again: at once, or at most `CONTEXT_EVERY` after the last time.
    fn context_due(&mut self, now: Instant) {
        if !self.chips_on() {
            return;
        }
        let at = now.max(self.wins[self.cur].refresh.last + CONTEXT_EVERY);
        self.wins[self.cur].refresh.due = Some(self.wins[self.cur].refresh.due.map_or(at, |d| d.min(at)));
    }

    /// Takes in a finished refresh, starts one that is due, and keeps the minute timer.
    fn tick_context(&mut self) {
        let win = &mut self.wins[self.cur];
        let done = win.refresh.done.lock().unwrap().take();
        if let Some(c) = done {
            if c != win.ctx {
                let moved = c.cwd != win.ctx.cwd;
                win.ctx = c;
                win.history.set_ctx(&win.ctx);
                win.ctx_dirty = true;
                self.dirty = true;
                if moved {
                    self.titled(self.cur);
                }
            }
        }
        if !self.chips_on() {
            (self.wins[self.cur].refresh.due, self.wins[self.cur].refresh.clock_at) = (None, None);
            return;
        }
        let now = Instant::now();
        if self.wins[self.cur].refresh.clock_at.is_none_or(|t| now >= t) && self.cfg.input.chips.contains(&Chip::Time) {
            if self.wins[self.cur].refresh.clock_at.is_some() {
                self.context_due(now);
            }
            self.wins[self.cur].refresh.clock_at = Some(now + context::until_next_minute());
        }
        if self.wins[self.cur].refresh.due.is_some_and(|d| now >= d) {
            if self.wins[self.cur].refresh.running.swap(true, Ordering::AcqRel) {
                self.wins[self.cur].refresh.due = Some(now + Duration::from_millis(200)); // one at a time
                return;
            }
            self.wins[self.cur].refresh.due = None;
            self.wins[self.cur].refresh.last = now;
            let probe = Probe {
                pid: self.wins[self.cur].shell,
                report: self.wins[self.cur].report.clone(),
                clock24: self.cfg.input.clock24,
            };
            let (done, running, proxy) = (self.wins[self.cur].refresh.done.clone(), self.wins[self.cur].refresh.running.clone(), self.wins[self.cur].proxy.clone());
            let spawned = std::thread::Builder::new().name("context".into()).spawn(move || {
                let c = context::gather(&probe);
                *done.lock().unwrap() = Some(c);
                running.store(false, Ordering::Release);
                proxy.wake();
            });
            if spawned.is_err() {
                self.wins[self.cur].refresh.running.store(false, Ordering::Release);
            }
        }
    }

    // ----------------------------------------------------------- settings --

    /// How long after the last change (a key held down on a slider, a click) the settings are written. A
    /// gesture that ends (a slider let go, a number typed) writes at once.
    const SAVE_DELAY: Duration = Duration::from_millis(300);

    /// A settings panel on what is in effect now.
    fn make_settings(&self) -> Settings {
        Settings::open(&self.shown_font, self.opacity)
            .with_input(&self.cfg.input)
            .with_windows(&self.cfg.windows)
            .with_blur(self.cfg.blur, self.wl.blur_supported())
            .with_lang(self.cfg.lang)
    }

    /// Opens the panel beside the grid (the grid gets narrower) or closes it. It starts with the keyboard.
    fn toggle_settings(&mut self) {
        if self.panel.is_some() {
            self.close_settings();
            return;
        }
        let mut s = self.make_settings();
        s.start_faded();
        self.panel = Some(Box::new(s));
        self.ui.prefs = true;
        self.repeat = None;
        self.relayout();
    }

    /// Closes the panel: what it changed last is written first, and the grid takes its room back.
    fn close_settings(&mut self) {
        self.flush_save();
        if self.panel.take().is_none() {
            return;
        }
        self.ui.prefs = false;
        if self.hover == Hit::Prefs {
            self.hover = Hit::Client; // the grid is under the pointer now
        }
        self.relayout();
    }

    /// Shows a draft on the window: fonts, spacing, opacity, the windows' panel and the chips change in place.
    fn apply(&mut self, d: &settings::Draft) {
        self.opacity = d.opacity;
        if self.cfg.lang != d.lang {
            self.cfg.lang = d.lang;
            i18n::set(d.lang.resolve_here());
        }
        if self.cfg.blur != d.blur {
            self.cfg.blur = d.blur;
            self.set_blur();
        }
        if self.cfg.windows != d.windows {
            self.cfg.windows = d.windows.clone();
            self.side_hover = None;
        }
        if d.font != self.shown_font {
            self.base_px = d.font.size;
            self.soft.update_fonts(&d.font);
            self.shown_font = d.font.clone();
            (self.cw, self.ch) = self.soft.cell();
        }
        self.soft.set_opacity(d.opacity);
        self.set_input(&d.input);
        self.relayout();
    }

    /// Switches the prompt chips to `input`: the flag the shell's hook looks at is put right and the
    /// chips are read now, not after the next output.
    fn set_input(&mut self, input: &config::InputCfg) {
        if *input == self.cfg.input {
            return;
        }
        let clock_changed = input.clock24 != self.cfg.input.clock24;
        if input.cursor != self.cfg.input.cursor {
            for w in &self.wins {
                session::set_cursor(&w.term, input.cursor);
            }
        }
        self.cfg.input = input.clone();
        self.wins[self.cur].refresh.clock_at = None;
        if self.chips_on() {
            self.wins[self.cur].refresh.due = Some(Instant::now());
            if clock_changed {
                let win = &mut self.wins[self.cur];
                win.ctx = Context::default(); // the clock text is rebuilt by the refresh
                win.history.set_ctx(&win.ctx);
            }
        }
        self.wins[self.cur].ctx_dirty = true;
        self.sync_flag();
    }

    /// What an input to the panel asks of the app. Changes apply at once and are saved when they settle.
    fn panel_effect(&mut self, e: Effect) {
        if e == Effect::None {
            return;
        }
        self.prefs_dirty = true;
        self.dirty = true;
        match e {
            Effect::None | Effect::Redraw => {}
            Effect::Apply => {
                let Some(d) = self.panel.as_ref().map(|p| p.draft.clone()) else { return };
                self.apply(&d);
                // Not while a slider is being dragged or a number typed: the write waits for the gesture's end.
                let settled = self.panel.as_ref().is_some_and(|p| p.settled());
                self.save_at = settled.then(|| Instant::now() + Self::SAVE_DELAY);
            }
            Effect::Settled => self.save_at = Some(Instant::now()),
            Effect::Close => {
                // The key that did this may still be held: it must not go on to the shell.
                self.repeat = None;
                self.close_settings();
            }
        }
    }

    /// Writes what the panel changed when its moment has come.
    fn tick_save(&mut self) {
        if self.save_at.is_none_or(|t| Instant::now() < t) {
            return;
        }
        // A slider held or a number being typed: the end of the gesture writes, not this timer.
        if self.panel.as_ref().is_some_and(|p| !p.settled()) {
            self.save_at = None;
            return;
        }
        self.flush_save();
    }

    /// Writes what the panel changed now, if anything (and a number still being typed counts).
    fn flush_save(&mut self) {
        self.save_at = None;
        let Some(p) = self.panel.as_mut() else { return };
        let d = p.commit().clone();
        let changes = p.changes();
        if changes.is_empty() {
            return;
        }
        // What was just typed is on the window too (a number committed by closing the panel).
        self.apply(&d);
        let Some(p) = self.panel.as_mut() else { return };
        match config::save(&changes) {
            Ok(_) => {
                p.saved();
                self.cfg.font = d.font.clone();
                self.cfg.opacity = Some(d.opacity);
                self.cfg_px = d.font.size;
            }
            Err(e) => {
                p.notice = Some(format!("{}{e}", tr(Str::NotSaved)));
                self.prefs_dirty = true;
                self.dirty = true;
            }
        }
    }

    /// The settings panel's box in window pixels, when it is open.
    fn prefs_box(&self) -> Option<(i32, i32, i32, i32)> {
        self.panel.as_ref()?;
        self.soft.prefs_box((self.w, self.h))
    }

    fn in_prefs(&self, x: f32, y: f32) -> bool {
        self.prefs_box().is_some_and(|(bx, by, bw, bh)| x >= bx as f32 && x < (bx + bw) as f32 && y >= by as f32 && y < (by + bh) as f32)
    }

    /// The pointer's shape over the panel: the I-beam on a number's box, a hand on what takes a click.
    fn prefs_cursor(&self) -> u32 {
        match self.panel.as_ref().map(|p| p.hover) {
            Some(settings::Hit::Value(_)) => cursor::TEXT,
            Some(settings::Hit::Panel | settings::Hit::Outside | settings::Hit::Row(_)) | None => cursor::DEFAULT,
            Some(_) => cursor::POINTER,
        }
    }

    /// A primary-button press or release while the panel is open. Returns whether the panel took it: a press on
    /// the panel is its own (and gives it the keyboard); a press anywhere else takes the keyboard away and goes
    /// on to the terminal or the window, which stay usable beside the panel.
    fn panel_button(&mut self, pressed: bool) -> bool {
        let over = self.in_prefs(self.mouse.0, self.mouse.1);
        let Some(p) = self.panel.as_mut() else { return false };
        if !pressed {
            let e = p.release();
            self.panel_effect(e);
            return false;
        }
        if !over {
            if p.focus {
                p.focus = false;
                self.prefs_dirty = true;
                self.dirty = true;
            }
            return false;
        }
        let e = p.press(self.mouse.0, self.mouse.1);
        self.prefs_dirty = true;
        self.panel_effect(e);
        true
    }

    /// Reads the configuration again (Ctrl+Shift+R): fonts, spacing and opacity change in place.
    fn reload(&mut self) -> bool {
        let cfg = config::load();
        i18n::set(cfg.lang.resolve_here());
        for w in &self.wins {
            session::set_cursor(&w.term, cfg.input.cursor);
        }
        self.cfg = cfg.clone();
        self.ui.status = cfg.status_bar;
        self.soft.set_ui(self.ui);
        self.set_blur();
        self.opacity = cfg.opacity.unwrap_or(chrome::OPACITY);
        self.base_px = cfg.font.size;
        self.cfg_px = cfg.font.size;
        self.soft.set_fonts(&cfg.font);
        self.shown_font = cfg.font.clone();
        self.soft.set_opacity(cfg.opacity.unwrap_or(chrome::OPACITY));
        (self.cw, self.ch) = self.soft.cell();
        for w in &mut self.wins {
            w.refresh.clock_at = None;
        }
        if self.chips_on() {
            self.wins[self.cur].refresh.due = Some(Instant::now());
        }
        self.sync_flag();
        if self.panel.is_some() {
            let mut s = self.make_settings();
            if let Some(old) = self.panel.as_ref() {
                (s.page, s.focus) = (old.page, old.focus);
            }
            self.panel = Some(Box::new(s));
            self.save_at = None;
        }
        self.relayout();
        true
    }

    fn zoom(&mut self, px: f32) -> bool {
        self.base_px = px.clamp(8.0, 48.0);
        self.shown_font.size = self.base_px;
        self.soft.set_font_px(self.base_px);
        (self.cw, self.ch) = self.soft.cell();
        self.relayout();
        true
    }

    // ----------------------------------------------------------- keyboard --

    fn on_key(&mut self, code: u32, pressed: bool) {
        if !pressed {
            if self.repeat.as_ref().is_some_and(|r| r.code == code) {
                self.repeat = None;
            }
            return;
        }
        let Some(xkb) = self.xkb.as_mut() else { return };
        let repeats = xkb.repeats(code);
        let Some(key) = xkb.press(code) else { return }; // consumed by a dead key or compose
        let done = self.handle_key(&key);
        self.repeat = (done && repeats && self.rate > 0)
            .then(|| Repeat { code, key, at: Instant::now() + Duration::from_millis(self.delay.max(0) as u64) });
    }

    fn tick_repeat(&mut self) {
        let now = Instant::now();
        let Some(r) = self.repeat.as_mut().filter(|r| now >= r.at) else { return };
        r.at = now + Duration::from_secs_f64(1.0 / self.rate.max(1) as f64);
        let key = r.key.clone();
        self.handle_key(&key);
    }

    /// Runs a shortcut or sends the key to the shell. Returns whether anything was done.
    fn handle_key(&mut self, k: &Key) -> bool {
        let m = k.mods;
        if m.ctrl && !m.alt && !m.shift && k.sym == 0x2c {
            self.toggle_settings();
            return true;
        }
        // The settings panel has the keyboard (it opened, or was clicked, and the terminal was not clicked since):
        // the app's own shortcuts still work, and everything else is for the panel. Escape closes it.
        let docked = self.panel.as_ref().is_some_and(|p| p.focus);
        let letter = char::from_u32(k.sym).map(|c| c.to_ascii_lowercase());
        if m.alt && !m.ctrl {
            let last = self.wins.len() - 1;
            match (letter, k.sym) {
                (Some('n'), _) => self.new_win(),
                (Some('w'), _) => self.close_win(self.cur),
                (Some('b'), _) => self.toggle_side(),
                (Some('g'), _) if m.shift => self.ungroup(),
                (Some('g'), _) => self.new_group(),
                (_, 0x5b) => self.switch_to(if self.cur == 0 { last } else { self.cur - 1 }),
                (_, 0x5d) => self.switch_to(if self.cur == last { 0 } else { self.cur + 1 }),
                (_, d @ 0x31..=0x39) if ((d - 0x31) as usize) < self.wins.len() => self.switch_to((d - 0x31) as usize),
                // Alt with the arrows moves a chip in the panel.
                _ if docked => return self.panel_key(k),
                _ => return false,
            }
            return true;
        }
        if m.ctrl && m.shift {
            match letter {
                Some('c') => return self.copy(),
                Some('v') => return self.paste(),
                Some('q') => {
                    self.quit = true;
                    return true;
                }
                Some('r') => return self.reload(),
                _ => {}
            }
        }
        if docked {
            return self.panel_key(k);
        }
        if m.ctrl && !m.alt {
            match k.sym {
                0x3d | 0x2b | KS_KP_ADD => return self.zoom(self.base_px + 1.0),
                0x2d | KS_KP_SUBTRACT => return self.zoom(self.base_px - 1.0),
                0x30 | KS_KP_0 => return self.zoom(self.cfg_px),
                _ => {}
            }
        }
        if m.shift && !m.ctrl && !m.alt {
            let scroll = match k.sym {
                KS_PAGE_UP => Some(Scroll::PageUp),
                KS_PAGE_DOWN => Some(Scroll::PageDown),
                KS_HOME => Some(Scroll::Top),
                KS_END => Some(Scroll::Bottom),
                _ => None,
            };
            if let Some(s) = scroll {
                self.wins[self.cur].term.lock().scroll_display(s);
                self.dirty = true;
                return true;
            }
        }
        let mode = *self.wins[self.cur].term.lock().mode();
        let Some(bytes) = input::encode_key(k, mode) else { return false };
        let mut t = self.wins[self.cur].term.lock();
        if t.grid().display_offset() > 0 {
            t.scroll_display(Scroll::Bottom);
        }
        t.selection = None;
        drop(t);
        self.wins[self.cur].pty.notify(bytes);
        self.dirty = true;
        true
    }

    /// A key for the settings panel; it never reaches the shell.
    fn panel_key(&mut self, k: &Key) -> bool {
        let Some(p) = self.panel.as_mut() else { return false };
        let e = p.key(k);
        self.panel_effect(e);
        true
    }

    // ---------------------------------------------------------- clipboard --

    fn paste(&mut self) -> bool {
        let Some(text) = self.wl.clipboard_text() else { return true };
        let text = text.replace('\x1b', "");
        let bracketed = self.wins[self.cur].term.lock().mode().contains(TermMode::BRACKETED_PASTE);
        if bracketed {
            self.wins[self.cur].pty.notify(format!("\x1b[200~{text}\x1b[201~").into_bytes());
        } else {
            self.wins[self.cur].pty.notify(text.replace("\r\n", "\r").replace('\n', "\r").into_bytes());
        }
        true
    }

    fn copy(&mut self) -> bool {
        let text = self.wins[self.cur].term.lock().selection_to_string();
        if let Some(t) = text {
            self.wl.set_clipboard(&t);
        }
        true
    }

    // ------------------------------------------------------------ pointer --

    /// The viewport cell under the pointer `(col, row)`, and which half of it.
    fn viewport_cell(&self) -> (usize, usize, Side) {
        let (cw, ch) = (self.cw as f32, self.ch as f32);
        let fx = ((self.mouse.0 - (self.soft.pad + self.soft.inset().0) as f32) / cw).max(0.0);
        let fy = ((self.mouse.1 - self.soft.top as f32) / ch).max(0.0);
        let col = (fx as usize).min(self.cols - 1);
        let row = (fy as usize).min(self.rows - 1);
        (col, row, if fx.fract() < 0.5 { Side::Left } else { Side::Right })
    }

    /// The terminal point under the pointer, and which half of its cell.
    fn cell_at(&self) -> (Point, Side) {
        let (col, row, side) = self.viewport_cell();
        let off = self.wins[self.cur].term.lock().grid().display_offset();
        (viewport_to_point(off, Point::new(row, Column(col))), side)
    }

    /// Whether the pointer feeds the program instead of selecting: it asked for reports, and Shift
    /// (the usual way out, to select text anyway) is not held.
    fn reporting(&mut self) -> bool {
        let shift = self.xkb.as_ref().is_some_and(|x| x.mods().shift);
        !shift && mouse::reporting(*self.wins[self.cur].term.lock().mode())
    }

    /// Sends `ev` at the cell under the pointer. Returns whether a report went out.
    fn report(&mut self, ev: mouse::Event) -> bool {
        let (col, row, _) = self.viewport_cell();
        let mods = self.xkb.as_ref().map(|x| x.mods()).unwrap_or_default();
        let mode = *self.wins[self.cur].term.lock().mode();
        let Some(r) = mouse::encode(mode, ev, mods, col, row) else { return false };
        self.reported = (col, row);
        self.wins[self.cur].pty.notify(r.bytes().to_vec());
        true
    }

    /// A button press or release in mouse-reporting mode. Returns whether the program took it.
    fn report_button(&mut self, b: u8, pressed: bool) -> bool {
        if pressed {
            if self.hover != Hit::Client || self.held.is_some() || !self.reporting() {
                return false;
            }
            self.held = Some(b);
            self.wins[self.cur].term.lock().selection = None;
            self.dirty = true;
            self.report(mouse::Event::Press(b));
            true
        } else if self.held == Some(b) {
            self.held = None;
            self.report(mouse::Event::Release(b));
            true
        } else {
            false
        }
    }

    /// `(x, y)` is in surface pixels; everything else here is in window pixels.
    fn on_pointer_move(&mut self, x: f32, y: f32) {
        let (ml, mt, _) = chrome::margins(self.ui.maximized);
        let (x, y) = (x - ml as f32, y - mt as f32);
        self.mouse = (x, y);
        if let Some(p) = self.panel.as_mut() {
            let e = p.pointer(x, y);
            self.panel_effect(e);
        }
        if self.held.is_some() || (self.hover == Hit::Client && self.reporting()) {
            // Programs hear about a move only when it crosses into another cell.
            let (col, row, _) = self.viewport_cell();
            if (col, row) != self.reported {
                self.report(mouse::Event::Move(self.held));
            }
            if self.held.is_some() {
                return;
            }
        }
        if self.selecting {
            let (p, side) = self.cell_at();
            if let Some(sel) = self.wins[self.cur].term.lock().selection.as_mut() {
                sel.update(p, side);
            }
            self.dirty = true;
            return;
        }
        let mut now = chrome::hit(x, y, self.w, self.h, self.ui.maximized, self.ui.side != 0);
        let part = self.side_part_at(x, y);
        let in_panel = self.side_box().is_some_and(|(bx, by, bw, bh)| x >= bx as f32 && x < (bx + bw) as f32 && y >= by as f32 && y < (by + bh) as f32);
        if (now == Hit::Client && in_panel) || (now == Hit::Title && part.is_some()) {
            now = Hit::Side;
        }
        // Over the settings panel the grid is not under the pointer: no selection, no reports to a program.
        if now == Hit::Client && self.in_prefs(x, y) {
            now = Hit::Prefs;
        }
        if part != self.side_hover {
            self.side_hover = part;
            self.side_dirty = true;
            self.dirty = true;
        }
        if now != self.hover {
            self.hover = now;
            if now.button() != self.ui.hover {
                self.ui.hover = now.button();
                self.dirty = true;
            }
        }
        // One shape for what is under the pointer (the call is a no-op when it is the one already set).
        let shape = if now == Hit::Prefs {
            self.prefs_cursor()
        } else if part.is_some() {
            cursor::POINTER
        } else {
            cursor_for(now)
        };
        self.wl.set_cursor(shape);
    }

    fn on_button(&mut self, pressed: bool, time: u32) {
        if !pressed {
            self.selecting = false;
            if self.ui.down {
                self.ui.down = false;
                self.dirty = true;
                match self.hover {
                    Hit::Close => self.quit = true,
                    Hit::Minimize => self.wl.minimize(),
                    Hit::Maximize => self.wl.toggle_maximized(),
                    Hit::Settings => self.toggle_settings(),
                    Hit::SideBtn => self.toggle_side(),
                    _ => {}
                }
            }
            return;
        }
        match self.hover {
            Hit::Close | Hit::Minimize | Hit::Maximize | Hit::Settings | Hit::SideBtn => {
                self.ui.down = true;
                self.dirty = true;
            }
            Hit::Title if self.title_click != 0 && time.wrapping_sub(self.title_click) < DOUBLE_CLICK_MS => {
                self.title_click = 0;
                self.wl.toggle_maximized();
            }
            Hit::Title => {
                self.title_click = time;
                self.wl.begin_move();
            }
            Hit::Edge(edges) => self.wl.begin_resize(edges),
            Hit::Prefs => {}
            Hit::Side => match self.side_hover {
                Some(side::Part::Row(i)) => self.switch_to(i),
                Some(side::Part::Close(i)) => self.close_win(i),
                Some(side::Part::New) => self.new_win(),
                Some(side::Part::Head(g)) => self.fold(g),
                None => {}
            },
            Hit::Client => {
                let n = if self.grid_click.0 != 0 && time.wrapping_sub(self.grid_click.0) < MULTI_CLICK_MS {
                    self.grid_click.1 % 3 + 1
                } else {
                    0
                };
                self.grid_click = (time, n);
                let ty = match n {
                    1 => SelectionType::Semantic,
                    2 => SelectionType::Lines,
                    _ => SelectionType::Simple,
                };
                let (p, side) = self.cell_at();
                self.wins[self.cur].term.lock().selection = Some(Selection::new(ty, p, side));
                self.selecting = true;
                self.dirty = true;
            }
        }
    }

    fn on_scroll(&mut self, pixels: f32, clicks: i32) {
        // Over the settings panel the wheel scrolls its body (a notch is `WHEEL_PX`); elsewhere the terminal.
        if self.in_prefs(self.mouse.0, self.mouse.1) {
            let px = if clicks != 0 { clicks * settings::WHEEL_PX } else { pixels.round() as i32 };
            let e = self.panel.as_mut().map_or(Effect::None, |p| p.scroll_by(px));
            self.panel_effect(e);
            return;
        }
        // Wayland counts down as positive; the terminal's `Scroll::Delta` counts up into history.
        self.scroll_acc -= if clicks != 0 { clicks as f32 * WHEEL_LINES } else { pixels / self.ch as f32 };
        let lines = self.scroll_acc.trunc() as i32;
        if lines == 0 {
            return;
        }
        self.scroll_acc -= lines as f32;
        if self.hover == Hit::Client && self.reporting() {
            for _ in 0..lines.unsigned_abs() {
                self.report(mouse::Event::Wheel(lines > 0));
            }
            return;
        }
        let mode = *self.wins[self.cur].term.lock().mode();
        if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) {
            let k = match (lines > 0, mode.contains(TermMode::APP_CURSOR)) {
                (true, true) => &b"\x1bOA"[..],
                (true, false) => &b"\x1b[A"[..],
                (false, true) => &b"\x1bOB"[..],
                (false, false) => &b"\x1b[B"[..],
            };
            for _ in 0..lines.unsigned_abs() {
                self.wins[self.cur].pty.notify(k);
            }
        } else {
            self.wins[self.cur].term.lock().scroll_display(Scroll::Delta(lines));
            self.dirty = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use config::{TabsPos, WindowsCfg};

    #[test]
    fn the_settings_are_at_the_edge_opposite_the_windows_panel() {
        let cfg = |tabs, side| WindowsCfg { tabs, side, side_w: 240 };
        // Windows at the left: settings at the right.
        assert_eq!(docks(&cfg(TabsPos::Left, true), 372), (Dock { w: 240, left: true }, Dock { w: 372, left: false }));
        // Windows at the right: settings at the left, as the reference orders them (`.prefs` is `order: -1`).
        assert_eq!(docks(&cfg(TabsPos::Right, true), 372), (Dock { w: 240, left: false }, Dock { w: 372, left: true }));
        // The panel hidden, or the windows as tabs at the top: only the settings take room, at the right.
        assert_eq!(docks(&cfg(TabsPos::Left, false), 372), (Dock { w: 0, left: true }, Dock { w: 372, left: false }));
        assert_eq!(docks(&cfg(TabsPos::Top, true), 372), (Dock { w: 0, left: true }, Dock { w: 372, left: false }));
        // Closed: no room; they never share an edge, so the grid is the window minus both.
        let (a, b) = docks(&cfg(TabsPos::Right, true), 0);
        assert_eq!((a.w, b.w), (240, 0));
        for tabs in TabsPos::ALL {
            let (a, b) = docks(&cfg(tabs, true), 372);
            assert!(a.w == 0 || a.left != b.left, "{tabs:?}");
        }
    }
}
