//! The application on the own window layer: the terminal session, the CPU renderer and `plat.rs`'s `Plat` (the
//! own Wayland client `wl.rs` on Linux, the own Win32 window `win.rs` on Windows), in one blocking wait loop with no
//! windowing or GPU library. It handles the keyboard (layouts by `xkb.rs` and client-side repeat on Linux, the
//! system's on Windows), the pointer (window chrome, selection, wheel) and the clipboard.
use std::{
    collections::VecDeque,
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
    index::{Column, Line, Point, Side},
    selection::{Selection, SelectionType},
    sync::FairMutex,
    term::TermMode,
};
mod windows_overview;

use crate::{
    bar,
    side,
    history::{self, History, Hook},
    i18n::{self, Str, tr},
    cell::pack,
    chrome::{self, EDGE_BOTTOM, EDGE_LEFT, EDGE_RIGHT, EDGE_TOP, Hit, Ui},
    commands::{self, Cmd},
    config::{self, Chip, Config, FontCfg},
    context::{self, Context, Probe},
    ctxmenu, dnd, field, group_dialog,
    groups::{self, Group},
    input, mouse,
    overlay::{self, Click, Geo, Overlay, Ran, Reply, Stack},
    paint::Paint,
    overlay_demo, overview, palette, panes, picker::Picker, session,
    settings::{self, Effect, Settings},
    soft::{Dock, Slot, Soft, UNFOCUSED},
    view::View,
    plat::{Event, Key, Mods, Plat, Wake, cursor},
};
#[cfg(target_os = "linux")]
use crate::xkb::Xkb;
use windows_overview::AppGrid;

mod groups_ui;
mod selftest;

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
const KS_ESCAPE: u32 = 0xff1b;
const KS_PAGE_UP: u32 = 0xff55;
const KS_PAGE_DOWN: u32 = 0xff56;
const KS_HOME: u32 = 0xff50;
const KS_END: u32 = 0xff57;
const KS_KP_ADD: u32 = 0xffab;
const KS_KP_SUBTRACT: u32 = 0xffad;
const KS_KP_0: u32 = 0xffb0;

fn cursor_for(hit: Hit) -> u32 {
    match hit {
        Hit::Close | Hit::Minimize | Hit::Maximize | Hit::Settings | Hit::SideBtn | Hit::NewGroup => cursor::POINTER,
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

/// How the PTY thread reaches the loop: events go in a queue and `Wake` wakes the loop's wait.
#[derive(Clone)]
struct Proxy {
    wake: Arc<Wake>,
    pending: Arc<AtomicBool>,
    queue: Arc<Mutex<VecDeque<TermEvent>>>,
    /// What the shell's hook said, with the moment the PTY thread read it: commands are timed there.
    hooks: Arc<Mutex<VecDeque<(Instant, Hook)>>>,
}

impl Proxy {
    fn wake(&self) {
        self.wake.signal();
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

/// The app's clipboard as the overlays see it (read only when one asks to paste).
struct PlatClip<'a>(&'a mut Plat);

impl overlay::Clipboard for PlatClip<'_> {
    fn get(&mut self) -> Option<String> {
        self.0.clipboard_text()
    }

    fn set(&mut self, text: &str) {
        self.0.set_clipboard(text);
    }
}

/// A key being held: sent again at `at`, then every `1 / rate` seconds.
#[cfg_attr(windows, allow(dead_code))]
struct Repeat {
    code: u32,
    key: Key,
    at: Instant,
}

/// One pane: a terminal session and everything that belongs to that shell. A window (`Win`) holds one or
/// several, laid out side by side (`panes.rs`); keys go to the one with the focus.
struct Pane {
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
    /// Where the shell's own hook said it is, for the shells whose process does not move with them (`Hook::Dir`).
    cwd: Option<PathBuf>,
    report: Option<PathBuf>,
    flag: Option<PathBuf>,
    refresh: Refresh,
    /// The title the program set (empty: the directory's name is used).
    title: String,
    /// The directory of its shell in short form (`~/lab/axon`), read again when the shell may have moved
    /// (`read_dir`): the list, the tabs and the title bar show it without asking the system each time.
    dir: String,
    /// The size, in columns and rows, its terminal and PTY were told.
    size: (usize, usize),
}

/// One window (a tab of the panel): its panes, which one has the focus and how they are arranged. The app draws
/// and types into one window at a time (`App::cur`); the others keep running and are listed in the side panel.
struct Win {
    panes: Vec<Pane>,
    focus: usize,
    kind: panes::Kind,
    /// Whether a pane printed while the window was out of sight.
    unread: bool,
    /// The number the window was given when it opened, and the group it is in (an index into `App::groups`).
    id: u64,
    group: Option<usize>,
    /// The name the user gave it (empty: the automatic one, `auto_title`).
    name: String,
}

impl Win {
    /// The pane with the focus.
    fn pane(&self) -> &Pane {
        &self.panes[self.focus]
    }
}

/// What the grid area holds: the window on screen with its panes, or, when that window is in a group, the group's windows
/// side by side, one pane each (ADR 0034). Either is a run of `wins`, and the panes in that run, in order, are numbered
/// the same everywhere: the layout, the hit test, the focus.
#[derive(Clone, Copy)]
struct Shown {
    /// The first window of the run.
    first: usize,
    /// Whether the run is a group's: its panes are its windows, which have one pane each.
    grouped: bool,
    /// How the panes are arranged, how many there are, and the one with the focus.
    kind: panes::Kind,
    n: usize,
    focus: usize,
}

impl Shown {
    /// The windows the run covers.
    fn run(self) -> std::ops::Range<usize> {
        self.first..self.first + if self.grouped { self.n.max(1) } else { 1 }
    }

    /// The window, and the pane in it, of pane `i` of the run.
    fn locate(self, i: usize) -> (usize, usize) {
        if self.grouped { (self.first + i, 0) } else { (self.first, i) }
    }
}

/// A press on a window's row or tab, or on a group's header or chip, which becomes a drag when the pointer moves
/// far enough before the button comes up (`dnd.rs`), and a click when it does not.
struct Dnd {
    grab: dnd::Grab,
    /// The moment of the press (the compositor's, in ms): a second click soon after the first is a double click.
    time: u32,
    /// Where the button went down, how far inside the row's box, and the size of that box.
    press: (f32, f32),
    off: (f32, f32),
    size: (i32, i32),
    /// Whether it is a drag now, where a drop would put it, and where the pointer is.
    live: bool,
    target: Option<dnd::Target>,
    at: (f32, f32),
}

impl Drop for Pane {
    /// The files the shell and this pane talk through belong to this pane; the shell is told to end.
    fn drop(&mut self) {
        for f in [&self.report, &self.flag].into_iter().flatten() {
            let _ = std::fs::remove_file(f);
        }
        let _ = self.pty.0.send(alacritty_terminal::event_loop::Msg::Shutdown);
    }
}

struct App {
    plat: Plat,
    soft: Soft,
    /// The windows, in the order of the side panel, and the one on screen.
    wins: Vec<Win>,
    cur: usize,
    /// What wakes the wait: the PTY threads signal it.
    wake: Arc<Wake>,
    /// The number the next window gets (for its report file and its PTY).
    next_win: u64,
    /// Whether a shell can be given a scope of its own on this machine (`session::scope_probe`); None while the
    /// file had `[windows] scope = false` and nothing was tried. Read through `scope_for_new`.
    scope_ok: Option<bool>,
    /// What the pointer is over in the panel of windows, and whether the panel has to be painted again.
    side_hover: Option<side::Part>,
    side_dirty: bool,
    side_scratch: Vec<u8>,
    /// What the list shows of the windows, filled again for each paint (`fill_items`).
    items: side::Items,
    /// The groups, in the order the panel shows them, how many were ever made (for their names), and what
    /// the panel lists now (rebuilt whenever the windows or the groups change).
    groups: Vec<Group>,
    groups_made: usize,
    entries: Vec<side::Entry>,
    /// A window's name being edited in its row, what a close took out (kept until its toast is gone), what the open
    /// context menu is about, the last press on a row and on a group's header (for double clicks), and where the group
    /// dialog leaves its answer (`wlapp/groups_ui.rs`).
    rename: Option<groups_ui::Rename>,
    undo: Option<groups_ui::Undo>,
    menu_for: Option<groups_ui::Target>,
    row_click: Option<(u64, u32)>,
    head_click: Option<(u32, u32)>,
    answer: group_dialog::Answer,
    /// The groups as the list shows them (names, counts, which are folded and busy), rebuilt with `entries`.
    gitems: Vec<side::GroupItem>,
    /// A press on the list that may be a drag, when the spinners are painted next (none: no command runs, so
    /// nothing is scheduled and idle stays at 0 % CPU), and the moment they count their turns from.
    dnd: Option<Dnd>,
    spin_at: Option<Instant>,
    /// The tabs at the top scroll a group or a window at a time: the entry they start at (`side::Layout::scroll`), and the
    /// pixels of a touchpad's swipe not yet worth a tab.
    strip_first: usize,
    strip_acc: f32,
    spin_due: bool,
    spin_t0: Instant,
    #[cfg(target_os = "linux")]
    xkb: Option<Xkb>,
    /// The last key pressed (`Event::Press`) may repeat while it is held.
    press_repeats: bool,
    /// `AXON_SELFTEST`: the app testing itself for CI (`selftest.rs`).
    selftest: Option<selftest::Run>,
    ui: Ui,
    hover: Hit,
    /// Window size in pixels, the grid of a window with one pane, cell size.
    w: u32,
    h: u32,
    cols: usize,
    rows: usize,
    cw: u32,
    ch: u32,
    /// The panes of the window on screen as they are drawn (scratch, filled every frame).
    slots: Vec<Slot>,
    /// The pane the pointer is over (when there are several), whether it is over that pane's cross, the pane
    /// whose cross is on screen, and the pane whose cross the button went down on.
    pane_hover: Option<usize>,
    close_hot: bool,
    close_shown: Option<usize>,
    cross_down: Option<usize>,
    /// The pane a selection or a mouse report in progress belongs to.
    drag: usize,
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
    /// The overlays (palette, dialogs, menus) and the toast: the first to get the keyboard and the pointer
    /// (`overlay.rs`), and the pointer image they asked for last.
    overlays: Stack,
    overlay_cursor: Option<overlay::Cursor>,
    prefs_dirty: bool,
    /// What the overlays painted over the last frame (`give_back_to_overlays`).
    overlay_extent: overlay::Extent,
    save_at: Option<Instant>,
    dirty: bool,
    quit: bool,
    mouse: (f32, f32),
    /// The pane and viewport row the pointer is over in the grid, when it is: the status of the command whose block
    /// that is shows at the end of its line (`bar::Chips::frame`).
    grid_hover: Option<(usize, usize)>,
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

/// Whether a program set a title of its own: the one a shell sets by default says `user@host: path` (bash) or is the
/// shell's own path (a Windows console, `C:\...\pwsh.exe`, with `Administrator: ` before it when elevated), which the
/// window's directory says better.
fn own_title(t: &str) -> bool {
    if t.is_empty() || t.split_once(':').is_some_and(|(host, _)| host.contains('@') && !host.contains(' ')) {
        return false;
    }
    let t = t.strip_prefix("Administrator: ").unwrap_or(t).to_ascii_lowercase();
    !(t.ends_with(".exe") || t == "windows powershell")
}

/// The last part of a directory in short form (`~/lab/axon`, `~\code\axon`, `…/a/b/c`, `/`, `C:\`): `axon`, `c`, the
/// root itself.
fn dir_name(dir: &str) -> &str {
    match dir.rsplit(['/', '\\']).next() {
        Some("") | None => if dir.is_empty() || dir.starts_with('/') { "/" } else { dir },
        Some(name) => name,
    }
}

impl Pane {
    /// The directory the pane's shell is in: what its hook said, else what its process says.
    fn cwd_now(&self) -> Option<PathBuf> {
        self.cwd.clone().or_else(|| context::cwd_of(self.shell))
    }

    /// Reads the directory of the pane's shell again. True when it is another one. Asked when the shell may have
    /// moved (a prompt, a new title, the window coming on screen), not when the list is painted: the painters
    /// only read `dir`.
    fn read_dir(&mut self) -> bool {
        let cwd = self.cwd_now().unwrap_or_default();
        let home = context::home_dir();
        let dir = context::tidy_dir(&cwd.to_string_lossy(), home.as_deref().and_then(std::path::Path::to_str));
        let moved = dir != self.dir;
        self.dir = dir;
        moved
    }
}

/// The name of a pane: what its program set, else the command it runs (the shell's hook says it), else the name of its directory.
fn pane_name(p: &Pane) -> &str {
    if own_title(&p.title) { &p.title } else { p.history.command().unwrap_or_else(|| dir_name(&p.dir)) }
}

/// The directory of a window's shell, in short form: the one in the pane with the focus.
fn win_place(w: &Win) -> &str {
    &w.pane().dir
}

/// The name a window has without one of its own (`autoLabel`): that of the pane with the focus, or, when that one has
/// nothing to say but another pane of the window runs a command, the command's.
fn auto_title(w: &Win) -> &str {
    let p = w.pane();
    if own_title(&p.title) || p.history.command().is_some() {
        return pane_name(p);
    }
    w.panes.iter().find_map(|q| q.history.command()).unwrap_or_else(|| pane_name(p))
}

/// The name of a window: the one the user gave it, else the automatic one.
fn win_title(w: &Win) -> &str {
    if w.name.is_empty() { auto_title(w) } else { &w.name }
}

/// A debugging aid (`AXON_RUNNING`): every pane counts as running a command, whatever its shell says (a shell with
/// the hook says it is at a prompt a moment after it starts).
static ALL_RUNNING: AtomicBool = AtomicBool::new(false);

/// Whether a command runs in pane `p`, from its shell's hook.
fn running(p: &Pane) -> bool {
    ALL_RUNNING.load(Ordering::Relaxed) || p.history.running()
}

/// What a pane's shell is doing, from its hook.
fn pane_status(p: &Pane) -> side::Status {
    if running(p) {
        side::Status::Running
    } else if p.history.failed() {
        side::Status::Failed
    } else {
        side::Status::Idle
    }
}

/// Writes the text of pane `i`'s row in the panel: `Pane 2`, and what a program set as its title after it.
fn pane_label(out: &mut String, i: usize, p: &Pane) {
    use std::fmt::Write;
    let _ = write!(out, "{} {}", tr(Str::Pane), i + 1);
    if own_title(&p.title) {
        let _ = write!(out, " \u{b7} {}", p.title);
    }
}

/// Tells a program that asked for focus reports (mode 1004) that its pane got or lost the focus.
fn report_focus(p: &Pane, on: bool) {
    if p.term.lock().mode().contains(TermMode::FOCUS_IN_OUT) {
        p.pty.notify(if on { &b"\x1b[I"[..] } else { &b"\x1b[O"[..] });
    }
}

/// The title bar between the window buttons and the new-group button, where the tabs go (window pixels): `.top-slot`, 10 px
/// from each.
fn tabs_span(w: u32) -> (i32, i32) {
    (chrome::TABS_X0 as i32, w as i32 - chrome::TABS_FROM_RIGHT as i32)
}

/// The two panels beside the grid: the windows' (as wide as the settings say, none when it is hidden or the windows
/// are tabs at the top) and the settings' (`prefs_w` wide, 0 when closed). The reference puts the settings on the
/// edge opposite the windows' panel: `.main` is a row of windows, workspace, settings, and with the windows at the
/// right the settings go to the left. They never share an edge, so nothing has to say how they would stack.
/// In a window `ww` wide the grid keeps `MIN_GRID` pixels at least: the windows' panel gives way (`fit_side`).
fn docks(w: &config::WindowsCfg, prefs_w: u32, ww: u32) -> (Dock, Dock) {
    let right = w.tabs == config::TabsPos::Right;
    let side = if w.side && w.tabs != config::TabsPos::Top { fit_side(ww, w.side_w, prefs_w) } else { 0 };
    (Dock { w: side, left: !right }, Dock { w: prefs_w, left: right })
}

/// How many pixels of a touchpad's swipe scroll the tabs at the top by one.
const TAB_SCROLL_PX: f32 = 40.0;

/// The least width the grid keeps beside the panels, in pixels (the settings panel takes at most 62 % of the
/// window, so it alone never leaves less in a window of the least size).
const MIN_GRID: u32 = 120;

/// The room of the windows' panel, `side` wide, in a window `ww` wide with the settings `prefs_w` wide: its own
/// width, or none when that would leave the grid less than `MIN_GRID` (a narrow window, or a half-screen tile, with
/// both panels open). A negative workspace is what `Soft::paint_rows` could not paint.
fn fit_side(ww: u32, side: u32, prefs_w: u32) -> u32 {
    if side + prefs_w + MIN_GRID <= ww { side } else { 0 }
}

/// The list of windows as the panel and the tabs show it, written into `items` (kept between frames: nothing is
/// allocated once it has grown). A tab at the top says how many panes its window has (`top`); the panel lists them
/// under the window instead. A window being renamed (`edit`) has no title under its field.
fn fill_items(items: &mut side::Items, wins: &[Win], cur: usize, top: bool, edit: Editing) {
    items.clear();
    for (i, w) in wins.iter().enumerate() {
        let title = if edit.is_some_and(|(wi, _)| wi == i) { "" } else { win_title(w) };
        let badge = if top && w.panes.len() > 1 { w.panes.len() } else { 0 };
        items.push_win(title, i == cur, w.unread, badge, side::Status::of(w.panes.iter().map(pane_status)));
        if w.panes.len() > 1 {
            for (j, p) in w.panes.iter().enumerate() {
                items.push_pane(i == cur && j == w.focus, pane_status(p), |t| pane_label(t, j, p));
            }
        }
    }
}

/// The groups as the panel shows them: each pane counts as a window of its group (the reference's `windowCount`).
fn group_items(groups: &[Group], wins: &[Win]) -> Vec<side::GroupItem> {
    groups
        .iter()
        .enumerate()
        .map(|(g, x)| {
            let members = || wins.iter().filter(move |w| w.group == Some(g));
            side::GroupItem {
                id: x.id,
                name: x.name.clone(),
                color: x.color,
                collapsed: x.collapsed,
                count: members().map(|w| w.panes.len()).sum(),
                // A folded group says that a window in it runs a command, which its own rows no longer show.
                busy: x.collapsed && members().any(|w| w.panes.iter().any(running)),
                label_w: 0,
            }
        })
        .collect()
}

/// A window's name being edited: its index and the field, painted over its row (with no title under it).
type Editing<'a> = Option<(usize, &'a field::Field)>;

/// The edit view of a field for the panel's painter.
fn edit_of(f: &field::Field) -> side::Edit<'_> {
    side::Edit { text: f.text(), caret: f.caret(), sel: f.selection() }
}

/// Paints the panel of windows over its cleared box.
#[allow(clippy::too_many_arguments)]
fn paint_side(soft: &Soft, fb: &mut [u32], items: &side::Items, groups: &[side::GroupItem], entries: &[side::Entry], right: bool, look: side::Look, edit: Editing, scratch: &mut Vec<u8>) {
    let (Some(rect), Some(b)) = (soft.side_rect(), side_box(soft)) else { return };
    soft.clear_rect(fb, rect);
    side::paint(soft, fb, b, !right, tr(Str::Windows), entries, groups, items, look, scratch);
    if let Some((wi, f)) = edit {
        side::paint_rename(soft, fb, side::Layout::Side { x0: b.0, y0: b.1, w: b.2, entries }, wi, &edit_of(f), look.hover, scratch);
    }
}

/// Paints the tabs in the title bar, over the bar cleared between the buttons.
#[allow(clippy::too_many_arguments)]
fn paint_tabs(soft: &Soft, fb: &mut [u32], items: &side::Items, groups: &[side::GroupItem], entries: &[side::Entry], (w, first): (u32, usize), look: side::Look, edit: Editing, scratch: &mut Vec<u8>) {
    let (ox, oy) = soft.origin();
    let (x0, x1) = tabs_span(w);
    soft.clear_rect(fb, (ox + x0 as usize, oy, ox + x1 as usize, oy + chrome::TITLE_H));
    side::paint_top(soft, fb, (x0, x1), first, entries, groups, items, look, scratch);
    if let Some((wi, f)) = edit {
        side::paint_rename(soft, fb, side::Layout::Top { x0, x1, entries, groups, first }, wi, &edit_of(f), look.hover, scratch);
    }
}

/// Turns the spinners of the panel a step (`side::paint_spin`): only the rings are painted again.
fn spin_side(soft: &Soft, fb: &mut [u32], items: &side::Items, groups: &[side::GroupItem], entries: &[side::Entry], look: side::Look) {
    let Some(b) = side_box(soft) else { return };
    side::paint_spin(soft, fb, side::Layout::Side { x0: b.0, y0: b.1, w: b.2, entries }, b.1 + b.3, entries, groups, items, look);
}

/// The same for the tabs in the title bar.
fn spin_tabs(soft: &Soft, fb: &mut [u32], items: &side::Items, groups: &[side::GroupItem], entries: &[side::Entry], (w, first): (u32, usize), look: side::Look) {
    let (x0, x1) = tabs_span(w);
    let lay = side::Layout::Top { x0, x1, entries, groups, first };
    side::paint_spin(soft, fb, lay, 0, entries, groups, items, look);
}

/// The overlays are painted over a frame that has to be clean under them (their translucent pixels would pile up on
/// what the last frame left): all of the window while one is open, and under a toast only the toast's box, so the
/// fade of a toast costs what a toast covers and not a whole window. `extent` is what they painted over the last
/// frame; `win` is the window on screen.
fn give_back_to_overlays<'a>(overlays: &Stack, extent: &mut overlay::Extent, soft: &mut Soft, panes: impl Iterator<Item = &'a mut Pane>, geo: Geo) {
    let now = overlays.extent(geo);
    let was = std::mem::replace(extent, now);
    let part = match was.to_restore(now) {
        None => return,
        Some(overlay::Extent::Part(b)) => b,
        Some(_) => return soft.invalidate(),
    };
    let r = panes::Rect::new(part.0, part.1, part.2, part.3);
    if !soft.restore(r) {
        return soft.invalidate();
    }
    // The rows under it are written again (with the chips over them).
    for (pi, pane) in panes.enumerate() {
        for row in soft.rows_under(pi, r.y, r.y2()) {
            pane.view.invalidate_row(row);
        }
    }
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

impl Pane {
    /// A new pane with its own shell. `n` numbers it for the files it reports through.
    /// `scope` starts its shell in a scope of its own (`App::scope_for_new`).
    fn spawn(wake: &Arc<Wake>, n: u64, (cols, rows, cw, ch): (usize, usize, u32, u32), cursor: config::CursorKind, chips: bool, dir: Option<PathBuf>, scope: bool) -> Pane {
        let proxy = Proxy { wake: wake.clone(), pending: Arc::new(AtomicBool::new(false)), queue: Arc::default(), hooks: Arc::default() };
        // The files the shell and axon talk through (`context.rs`): what the shell reports, and the flag
        // that says the prompt chips are on.
        let (report, flag) = (context::report_path(std::process::id(), n), context::flag_path(std::process::id(), n));
        let shell_path = session::shell_path();
        let inherited = std::env::var("PROMPT_COMMAND").ok();
        let env = context::shell_env(&shell_path, report.as_deref(), flag.as_deref(), inherited.as_deref()).into_iter().collect();
        let (term, pty, shell) = session::spawn(proxy.clone(), cols, rows, cw, ch, n, env, cursor, dir, scope);
        let mut pane = Pane {
            term,
            pty,
            proxy,
            view: View::new(),
            ctx: Context::default(),
            history: History::default(),
            ctx_dirty: true,
            chips: bar::Chips::default(),
            shell,
            cwd: None,
            report,
            flag,
            refresh: Refresh { due: chips.then(Instant::now), last: Instant::now(), running: Arc::new(AtomicBool::new(false)), done: Arc::default(), clock_at: None },
            title: String::new(),
            dir: String::new(),
            size: (cols, rows),
        };
        pane.read_dir();
        pane
    }
}

impl Win {
    /// A window with one pane, numbered `id`.
    fn new(pane: Pane, id: u64) -> Win {
        Win { panes: vec![pane], focus: 0, kind: panes::Kind::Single, unread: false, id, group: None, name: String::new() }
    }
}

impl App {
    fn new() -> std::io::Result<App> {
        let cfg = config::load();
        i18n::set(cfg.lang.resolve_here());
        // The scope's try runs while the window is made (ADR 0038); it is awaited before the first shell.
        let scope_probe = session::scope_probe(cfg.windows.scope);
        let base_px = cfg.font.size;
        let shown_font = cfg.font.clone();
        let opacity = cfg.opacity.unwrap_or(chrome::OPACITY);
        let mut soft = Soft::new(&cfg.font, opacity);
        soft.cursor_color = pack(palette::CURSOR.r, palette::CURSOR.g, palette::CURSOR.b, 255);
        soft.set_title("axon", "");
        let (cw, ch) = soft.cell();

        let mut plat = Plat::connect("axon", "axon")?;
        #[cfg(target_os = "linux")]
        let mut xkb = Some(Xkb::load().ok_or_else(|| std::io::Error::other("libxkbcommon.so.0 not found, the keyboard would not work"))?);
        // The first configure decides the size; 0 x 0 means the compositor has no opinion.
        let (mut w, mut h) = (1000, 640);
        let mut ui = Ui { status: cfg.status_bar, ..Ui::IDLE };
        let (mut rate, mut delay) = (25, 600);
        'first: loop {
            plat.wait(None)?;
            plat.read_events()?;
            while let Some(ev) = plat.next_event() {
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
                    #[cfg(target_os = "linux")]
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

        let wake = Arc::new(Wake::new()?);
        let (side_dock, prefs_dock) = docks(&cfg.windows, 0, w);
        soft.set_docks(side_dock, prefs_dock);
        ui.side = match (cfg.windows.tabs, cfg.windows.side) {
            (config::TabsPos::Top, _) => 0,
            (_, true) => 2,
            (_, false) => 1,
        };
        soft.set_ui(ui);
        // The grid of a window with one pane: the whole workspace.
        let lone = soft.grid(soft.workspace(w, h), false);
        let (cols, rows) = (lone.cols, lone.rows);
        if let Some(dir) = context::report_path(std::process::id(), 0).as_deref().and_then(std::path::Path::parent) {
            context::remove_stale_reports(dir);
        }
        let cfg_chips = cfg.input.inline && !cfg.input.chips.is_empty();
        let scope_ok = cfg.windows.scope.then(|| scope_probe.works());
        let first = Win::new(Pane::spawn(&wake, 0, (cols, rows, cw, ch), cfg.input.cursor, cfg_chips, None, scope_ok == Some(true)), 0);
        let mut app = App {
            plat,
            soft,
            wins: vec![first],
            cur: 0,
            wake,
            next_win: 1,
            scope_ok,
            side_hover: None,
            side_dirty: true,
            side_scratch: Vec::new(),
            items: side::Items::default(),
            groups: Vec::new(),
            groups_made: 0,
            entries: Vec::new(),
            rename: None,
            undo: None,
            menu_for: None,
            row_click: None,
            head_click: None,
            answer: group_dialog::Answer::default(),
            gitems: Vec::new(),
            dnd: None,
            spin_at: None,
            strip_first: 0,
            strip_acc: 0.0,
            spin_due: false,
            spin_t0: Instant::now(),
            #[cfg(target_os = "linux")]
            xkb,
            press_repeats: false,
            selftest: selftest::Run::from_env(),
            ui,
            hover: Hit::Client,
            w,
            h,
            cols,
            rows,
            cw,
            ch,
            slots: Vec::new(),
            pane_hover: None,
            close_hot: false,
            close_shown: None,
            cross_down: None,
            drag: 0,
            base_px,
            cfg_px: base_px,
            shown_font,
            cfg,
            opacity,
            panel: None,
            overlays: Stack::default(),
            overlay_cursor: None,
            prefs_dirty: false,
            overlay_extent: overlay::Extent::Nothing,
            save_at: None,
            dirty: true,
            waiting: None,
            quit: false,
            mouse: (0.0, 0.0),
            grid_hover: None,
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
        // Another: the first window starts with this many panes (screenshots, smoke tests of the layouts).
        for _ in 1..std::env::var("AXON_PANES").ok().and_then(|n| n.parse::<usize>().ok()).unwrap_or(1).min(12) {
            app.split(panes::Dir::Auto);
        }
        // A debugging aid: every pane runs a command, as far as the panel and the tabs can tell (the spinners, and what
        // they cost while they turn).
        if std::env::var_os("AXON_RUNNING").is_some() {
            ALL_RUNNING.store(true, Ordering::Relaxed);
            app.refresh_groups();
        }
        // A debugging aid: start with the settings panel open (screenshots, memory measurements).
        if std::env::var_os("AXON_SETTINGS").is_some() {
            app.toggle_settings();
        }
        // Another: start with the overview of windows open (screenshots, memory measurements).
        if std::env::var_os("AXON_OVERVIEW").is_some() {
            app.toggle_overview();
        }
        // Another: start with one of the window list's screens open (the group dialog, a menu, a rename, a close with its toast).
        if let Ok(kind) = std::env::var("AXON_LIST_UI")
            && !app.list_ui(&kind)
        {
            eprintln!("axon: AXON_LIST_UI is one of dialog, edit, menu, group-menu, pane-menu, strip-menu, rename, close, toast (edit and group-menu need AXON_GROUPS=1)");
        }
        // Another: start with one of the overlay kit's screens open (`field`, `list`, `menu`, `toast`, `modal`) or the palette.
        if let Ok(kind) = std::env::var("AXON_DEMO_OVERLAY")
            && !app.demo_overlay(&kind)
        {
            eprintln!("axon: AXON_DEMO_OVERLAY is one of {}, palette", overlay_demo::KINDS.join(", "));
        }
        Ok(app)
    }

    fn run(mut self) -> std::io::Result<()> {
        while !self.quit {
            self.draw();
            self.plat.flush()?;

            // Block until the compositor, the PTY thread or a held key needs us; and, when a redraw is
            // stuck waiting for the compositor, only until it is clear that it will not answer.
            let now = Instant::now();
            let timeout = [self.repeat.as_ref().map(|r| r.at), self.waiting.map(|t| t + STALL), self.save_at, self.overlays.deadline(now), self.undo.as_ref().map(|u| u.until), self.spin_at, self.selftest.as_ref().map(|t| t.next)]
                .into_iter()
                .chain(self.shown_panes().flat_map(|p| [p.refresh.due, p.refresh.clock_at]))
                .flatten()
                .min()
                .map(|at| at.saturating_duration_since(now));
            let wake = self.wake.clone();
            let (plat_ready, wake_ready) = self.plat.wait_wake(&wake, timeout)?;
            if wake_ready {
                wake.clear();
                self.dirty = true;
                // A terminal printed something: the directory or the branch may have changed.
                let now = Instant::now();
                for pi in 0..self.shown().n {
                    if self.pane(pi).proxy.pending.load(Ordering::Acquire) {
                        self.context_due(pi, now);
                    }
                }
            }
            if plat_ready {
                self.plat.read_events()?;
            }
            self.term_events();
            while let Some(ev) = self.plat.next_event() {
                self.wl_event(ev);
            }
            self.tick_repeat();
            self.tick_context();
            // A toast fades out and goes by itself; the grid under it is painted again.
            match self.overlays.tick(Instant::now()) {
                overlay::Tick::Gone => {
                    self.soft.invalidate();
                    self.dirty = true;
                }
                overlay::Tick::Repaint => self.dirty = true,
                overlay::Tick::Rest => {}
            }
            self.tick_save();
            self.tick_undo(Instant::now());
            self.tick_spin();
            self.tick_selftest();
            if self.waiting.is_some_and(|t| t.elapsed() >= STALL) {
                self.plat.give_up_waiting();
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
        if self.plat.frame_pending() {
            self.waiting.get_or_insert_with(Instant::now);
            return;
        }
        // The buffer is the window plus the margin its shadow is drawn in.
        let (ml, mt, mb) = chrome::margins(self.ui.maximized);
        let (w, h) = (self.w as usize + 2 * ml, self.h as usize + mt + mb);
        let geo = self.geo();
        let chips_on = self.chips_on();
        // What the list of windows needs from the app, before the framebuffer is borrowed.
        let (drag, spinning) = (self.dnd_view(), self.spinning());
        // Where the panes of the window on screen are: when that is not what the framebuffer shows, the frame
        // is repainted whole. The cross at a pane's corner is drawn over rows that are repainted every frame.
        let sh = self.shown();
        let (n, kind, focus) = (sh.n, sh.kind, sh.focus);
        let ws = self.soft.workspace(self.w, self.h);
        self.slots.clear();
        self.slots.extend((0..n).map(|i| Slot { rect: panes::rect(kind, n, i, ws), focus: i == focus, framed: n > 1 }));
        self.soft.set_layout(&self.slots);
        let cross = if n > 1 { self.pane_hover.filter(|&i| i < n) } else { None };
        for i in [self.close_shown, cross].into_iter().flatten().filter(|&i| i < n) {
            for r in self.soft.close_rows(i) {
                let (wi, pj) = sh.locate(i);
                self.wins[wi].panes[pj].view.invalidate_row(r);
            }
        }
        // The overview lists the windows as they are now.
        if self.overlays.has(overview::TAG) {
            self.sync_overview();
        }
        let Some(fb) = self.plat.framebuffer(w, h) else {
            self.waiting.get_or_insert_with(Instant::now);
            return;
        };
        self.waiting = None;
        let win = &self.wins[self.cur];
        let branch = if chips_on { win.pane().ctx.get(Chip::Branch) } else { "" };
        let shell = session::shell_path();
        let shell = shell.rsplit(['/', '\\']).next().filter(|s| !s.is_empty()).unwrap_or("shell");
        let shell = shell.strip_suffix(".exe").unwrap_or(shell);
        let (cols, rows) = win.pane().size;
        self.soft.set_status(shell, branch, cols, rows);
        self.soft.set_ui(self.ui);
        let run = sh.run();
        give_back_to_overlays(&self.overlays, &mut self.overlay_extent, &mut self.soft, self.wins[run.clone()].iter_mut().flat_map(|w| w.panes.iter_mut()), geo);
        // Every pane in turn, pane 0 first (it begins the frame); one without the focus is drawn dimmer and
        // with a hollow cursor. A group's panes are its windows.
        let chips = &self.cfg.input.chips;
        for (i, pane) in self.wins[run].iter_mut().flat_map(|w| w.panes.iter_mut()).enumerate() {
            pane.proxy.pending.store(false, Ordering::Release);
            self.soft.select(i);
            self.soft.set_dim(if i == focus { 255 } else { UNFOCUSED });
            let mut t = pane.term.lock();
            let hover = self.grid_hover.filter(|&(p, _)| p == i).map(|(_, r)| r);
            pane.chips.frame(chips_on, pane.ctx_dirty, &pane.history, chips, &mut t, &mut pane.view, &mut self.soft, fb, (w, h), self.ui.focused && i == focus, hover);
            drop(t);
            pane.ctx_dirty = false;
        }
        self.soft.set_dim(255);
        self.soft.select(0);
        if let Some(i) = self.close_shown.filter(|&i| i < n && Some(i) != cross) {
            self.soft.clear_pane_close(fb, i);
        }
        if let Some(i) = cross {
            self.soft.paint_pane_close(fb, i, self.close_hot);
        }
        self.close_shown = cross;
        let top = self.cfg.windows.tabs == config::TabsPos::Top;
        let full = self.soft.last_full || self.side_dirty || (self.soft.title_painted && top) || drag.is_some();
        if full || self.spin_due {
            let look = side::Look { hover: self.side_hover, drag, phase: side::spin_phase(self.spin_t0.elapsed().as_millis()) };
            let edit: Editing = self.rename.as_ref().and_then(|r| self.wins.iter().position(|w| w.id == r.win).map(|wi| (wi, &r.field)));
            fill_items(&mut self.items, &self.wins, self.cur, top, edit);
            if !full {
                // Only the spinners are due: the rings are painted again, not the list.
                if top {
                    spin_tabs(&self.soft, fb, &self.items, &self.gitems, &self.entries, (self.w, self.strip_first), look);
                } else {
                    spin_side(&self.soft, fb, &self.items, &self.gitems, &self.entries, look);
                }
            } else if top {
                paint_tabs(&self.soft, fb, &self.items, &self.gitems, &self.entries, (self.w, self.strip_first), look, edit, &mut self.side_scratch);
            } else {
                let right = self.cfg.windows.tabs == config::TabsPos::Right;
                paint_side(&self.soft, fb, &self.items, &self.gitems, &self.entries, right, look, edit, &mut self.side_scratch);
            }
            self.side_dirty = false;
            self.spin_due = false;
            // The spinners are painted again in a moment, for as long as a command runs where they can be seen.
            self.spin_at = spinning.then(|| Instant::now() + Duration::from_millis(side::SPIN_EVERY_MS));
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
        let mut grid = AppGrid { soft: &self.soft, wins: &self.wins, scratch: &mut self.side_scratch };
        animating |= self.overlays.draw_with(fb, (w, h), (ml, mt), geo, Instant::now(), &mut grid);
        self.plat.present();
        // A fade in progress asks for the next frame itself; at rest nothing does (idle is 0 % CPU).
        self.dirty = animating;
    }

    /// Events from the terminals' threads, window by window. The queue lock is not held while one is
    /// handled: the PTY thread sends events while holding the terminal lock, and handling may take it.
    fn term_events(&mut self) {
        let mut gone = Vec::new();
        // The windows in sight (the one on screen, or its whole group) are painted as they print; the others say so in the panel.
        let seen = self.shown().run();
        for i in 0..self.wins.len() {
            for j in 0..self.wins[i].panes.len() {
                if !seen.contains(&i) && self.wins[i].panes[j].proxy.pending.swap(false, Ordering::AcqRel) {
                    // A window out of sight printed: say so in the panel; the grid is read when it comes back. It may
                    // have moved: the panel says where it is.
                    if !self.wins[i].unread {
                        self.wins[i].panes[j].read_dir();
                        self.wins[i].unread = true;
                        self.side_dirty = true;
                        self.dirty = true;
                    }
                }
                loop {
                    let Some((at, hook)) = self.wins[i].panes[j].proxy.hooks.lock().unwrap().pop_front() else { break };
                    let p = &mut self.wins[i].panes[j];
                    let named = p.history.command().is_some();
                    let said = matches!(hook, Hook::Label(_));
                    let moved = match hook {
                        Hook::Start => {
                            p.history.start(at);
                            false
                        }
                        // What runs: the window's label while it does.
                        Hook::Label(word) => {
                            p.history.set_label(&word);
                            false
                        }
                        // A prompt is where the shell has moved to, if it did.
                        Hook::Prompt { n, status } => {
                            p.history.prompt(n, status, at, &p.ctx);
                            p.read_dir()
                        }
                        Hook::Dir(dir) => {
                            p.cwd = Some(PathBuf::from(dir));
                            p.read_dir()
                        }
                    };
                    // The panel and the tabs say whether a command runs or failed (a folded group, that one does).
                    self.refresh_groups();
                    if moved || said || named != self.wins[i].panes[j].history.command().is_some() {
                        self.titled(i);
                    }
                    self.side_dirty = true;
                    self.dirty = true;
                }
                loop {
                    let Some(e) = self.wins[i].panes[j].proxy.queue.lock().unwrap().pop_front() else { break };
                    match e {
                        // Shells set the title at their prompt, with the directory in it: a moment to look where they are.
                        TermEvent::Title(t) => {
                            self.wins[i].panes[j].title = t;
                            self.wins[i].panes[j].read_dir();
                            self.titled(i);
                        }
                        TermEvent::ResetTitle => {
                            self.wins[i].panes[j].title.clear();
                            self.wins[i].panes[j].read_dir();
                            self.titled(i);
                        }
                        TermEvent::PtyWrite(t) => self.wins[i].panes[j].pty.notify(t.into_bytes()),
                        TermEvent::ClipboardStore(_, t) => self.plat.set_clipboard(&t),
                        // OSC 52 reads are refused (`session::term_config`): output must not read the clipboard.
                        TermEvent::ClipboardLoad(..) => {}
                        TermEvent::ColorRequest(c, f) => {
                            let p = &self.wins[i].panes[j];
                            let rgb = p.term.lock().colors()[c].unwrap_or_else(|| palette::by_index(c));
                            p.pty.notify(f(rgb).into_bytes());
                        }
                        TermEvent::TextAreaSizeRequest(f) => {
                            let p = &self.wins[i].panes[j];
                            let ws = WindowSize { num_lines: p.size.1 as u16, num_cols: p.size.0 as u16, cell_width: self.cw as u16, cell_height: self.ch as u16 };
                            p.pty.notify(f(ws).into_bytes());
                        }
                        TermEvent::Exit | TermEvent::ChildExit(_) => gone.push((i, j)),
                        _ => {}
                    }
                }
            }
        }
        // A shell that ends says so twice (`ChildExit` and `Exit`): one close each. A pane that ends closes
        // itself, and its window when it was the last; from the end, so the indices of the rest stay right.
        gone.sort_unstable();
        gone.dedup();
        for (i, j) in gone.into_iter().rev() {
            self.close_pane(i, j);
        }
    }

    /// The window `i` got a new title or moved: the title bar shows the current one.
    fn titled(&mut self, i: usize) {
        if i == self.cur {
            let (name, place) = (win_title(&self.wins[i]), win_place(&self.wins[i]));
            self.plat.set_title(&format!("{name} {place}"));
            self.soft.set_title(name, place);
        }
        self.side_dirty = true;
        self.dirty = true;
    }

    // --------------------------------------------------------- side panel --

    /// Leaves the panels' room beside the grid, at the sides the settings say, and tells the settings panel where
    /// its box is.
    fn apply_inset(&mut self) {
        let prefs_w = if self.panel.is_some() { settings::panel_width(self.w) } else { 0 };
        let (side, prefs) = docks(&self.cfg.windows, prefs_w, self.w);
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

    /// Where the list of windows is, as its layout and the box it fills (window pixels): the panel, or the strip
    /// of the title bar between the window buttons and the settings button. `None` when the panel is hidden.
    fn list_layout(&self) -> Option<(side::Layout<'_>, (i32, i32, i32, i32))> {
        if self.tabs_on_top() {
            let (x0, x1) = tabs_span(self.w);
            let lay = side::Layout::Top { x0, x1, entries: &self.entries, groups: &self.gitems, first: self.strip_first };
            return Some((lay, (x0, 0, x1 - x0, chrome::TITLE_H as i32 - 1)));
        }
        let (bx, by, bw, bh) = self.side_box()?;
        Some((side::Layout::Side { x0: bx, y0: by, w: bw, entries: &self.entries }, (bx, by, bw, bh)))
    }

    fn side_part_at(&self, x: f32, y: f32) -> Option<side::Part> {
        let (lay, (bx, by, bw, bh)) = self.list_layout()?;
        if x < bx as f32 || x >= (bx + bw) as f32 || y < by as f32 || y >= (by + bh) as f32 {
            return None;
        }
        lay.hit(x, y)
    }

    /// Puts the windows and groups at `pos` (the palette's three entries): a side shows the panel there, as picking
    /// it in the settings does; it is written to the file, and the panel of settings, if open, follows.
    fn set_tabs(&mut self, pos: config::TabsPos) {
        let mut windows = self.cfg.windows.clone();
        windows.tabs = pos;
        if pos != config::TabsPos::Top {
            windows.side = true;
        }
        let msg = match pos {
            config::TabsPos::Left => Str::ToastPanelLeft,
            config::TabsPos::Right => Str::ToastPanelRight,
            config::TabsPos::Top => Str::ToastTabsTop,
        };
        let saved = config::save(&[("windows.tabs", Some(format!("{:?}", pos.name()))), ("windows.side", Some(windows.side.to_string()))]);
        self.cfg.windows = windows;
        // The list differs between the tabs at the top (no pane rows) and the panel at a side.
        self.relist();
        if let Some(p) = self.panel.as_mut() {
            p.follow_tabs(&self.cfg.windows);
            self.prefs_dirty = true;
        }
        self.relayout();
        match saved {
            Ok(_) => self.overlays.toast(tr(msg), None, 0, Instant::now()),
            Err(e) => self.overlays.toast(&format!("{}{e}", tr(Str::NotSaved)), None, 0, Instant::now()),
        }
    }

    fn toggle_side(&mut self) {
        // The tabs at the top have no panel to show or hide: the reference says so.
        if self.tabs_on_top() {
            self.toast(tr(Str::ToastNoPanel));
            return;
        }
        self.cfg.windows.side = !self.cfg.windows.side;
        self.side_hover = None;
        if let Some(p) = self.panel.as_mut() {
            p.follow_windows(&self.cfg.windows);
            self.prefs_dirty = true;
        }
        self.relayout();
    }

    // ------------------------------------------------------------ windows --

    /// Opens a window with a shell of its own, where the one on screen is, and goes to it.
    fn new_win(&mut self) {
        self.close_overview();
        self.spawn_win(groups_ui::Spawn { activate: true, ..groups_ui::Spawn::default() });
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
        let was = self.wins[self.cur].id;
        let of: Vec<Option<usize>> = self.wins.iter().map(|w| w.group).collect();
        let cur = groups::after_close_in(&of, self.cur, &[i]);
        self.wins.remove(i);
        self.cur = cur;
        self.prune_groups();
        self.rebuild_side();
        self.show_after_close(was);
    }

    /// Windows left the list, and `was` (a window's id) was on screen: another window is shown only when that one went.
    /// A shell that ends out of sight must not stop the key repeat, the selection or the mouse drag of the window
    /// that stays (`switch_to` drops them); the list was painted again by `rebuild_side`.
    fn show_after_close(&mut self, was: u64) {
        if self.wins[self.cur].id != was {
            self.switch_to(self.cur);
        }
    }

    // -------------------------------------------------------------- panes --

    /// Splits what is on screen in direction `dir` (`Ctrl+Space`, `Alt+\\`, `Alt+-`, the palette): another terminal joins the
    /// group of the window on screen, right after it, starting in the same directory, and gets the focus (ADR 0034: the
    /// panes of a split are the windows of a group). A window in no group is put in a new one first, so the split
    /// always has a group to be. Refused (nothing happens, no group made) when a pane would be left with fewer than
    /// `panes::MIN_COLS` x `panes::MIN_ROWS` cells.
    fn split(&mut self, dir: panes::Dir) {
        let (n, kind) = match self.wins[self.cur].group {
            Some(_) => {
                let sh = self.shown();
                (sh.n, sh.kind)
            }
            None => (1, panes::Kind::Single),
        };
        let kind = panes::after_split(kind, n, dir);
        let ws = self.soft.workspace(self.w, self.h);
        let (min_w, min_h) = self.soft.min_pane(panes::MIN_COLS, panes::MIN_ROWS);
        if !panes::fits(kind, n + 1, ws, min_w, min_h) {
            self.toast(tr(Str::ToastNoRoom));
            return;
        }
        let g = self.wins[self.cur].group.unwrap_or_else(|| self.group_around(self.cur));
        self.groups[g].kind = kind;
        self.groups[g].collapsed = false;
        let o = groups_ui::Spawn { group: Some(g), after: Some(self.wins[self.cur].id), activate: true, ..groups_ui::Spawn::default() };
        self.spawn_win(o);
    }

    /// Gives the focus to pane `pi` of what is on screen (a click on it, `Alt+,` and `Alt+.`, its row in the panel). In
    /// a group the panes are windows, so the focus is the window on screen: it changes, and the group stays in view.
    fn set_focus(&mut self, pi: usize) {
        let sh = self.shown();
        if sh.grouped {
            if pi < sh.n && pi != sh.focus {
                self.switch_to(sh.first + pi);
            }
            return;
        }
        let (cur, reporting) = (self.cur, self.ui.focused);
        let w = &mut self.wins[cur];
        if pi >= w.panes.len() || pi == w.focus {
            return;
        }
        if reporting {
            report_focus(&w.panes[w.focus], false);
            report_focus(&w.panes[pi], true);
        }
        w.focus = pi;
        self.titled(cur);
    }

    /// Moves the focus `d` panes along (wrapping round): `Alt+,` back, `Alt+.` forward.
    fn cycle_pane(&mut self, d: i32) {
        let sh = self.shown();
        if sh.n > 1 {
            self.set_focus(panes::cycle(sh.focus, sh.n, d));
        }
    }

    /// The user closes pane `pi` of window `wi` (a shortcut, a cross, a menu or the palette): a pane that is the
    /// last of its window closes the window as `Alt+W` does, with the toast that undoes it.
    fn close_pane_asked(&mut self, wi: usize, pi: usize) {
        match self.wins.get(wi) {
            Some(w) if w.panes.len() == 1 && pi == 0 => self.close_wins(&[w.id], None),
            _ => self.close_pane(wi, pi),
        }
    }

    /// Closes pane `pi` of window `wi` (its shell is told to end; also what a shell that ended does). The last pane
    /// of a window closes the window. The focus stays where it was, or, when this pane had it, goes to the pane
    /// before it, as in the reference.
    fn close_pane(&mut self, wi: usize, pi: usize) {
        let Some(w) = self.wins.get_mut(wi) else { return };
        if pi >= w.panes.len() {
            return;
        }
        if w.panes.len() == 1 {
            return self.close_win(wi);
        }
        // Where the pane that had the focus is once this one is gone (None: it is this one).
        let kept = (w.focus != pi).then(|| if w.focus > pi { w.focus - 1 } else { w.focus });
        w.panes.remove(pi);
        w.kind = panes::after_remove(w.kind, w.panes.len());
        // A shell that ended in a pane without the focus leaves the focus where it was.
        w.focus = kept.unwrap_or_else(|| panes::focus_after_remove(pi));
        // The pane that had the focus went: the one that has it now is told.
        if self.ui.focused && wi == self.cur && kept.is_none() {
            report_focus(&w.panes[w.focus], true);
        }
        self.pane_hover = None;
        if wi == self.cur {
            (self.selecting, self.held, self.cross_down) = (false, None, None);
        }
        self.relayout();
        self.rebuild_side();
        self.titled(wi);
    }

    /// What window `wi` is shown with: itself and its panes, or, in a group, the group's windows.
    fn shown_for(&self, wi: usize) -> Shown {
        let w = &self.wins[wi];
        let Some(g) = w.group else { return Shown { first: wi, grouped: false, kind: w.kind, n: w.panes.len(), focus: w.focus } };
        let (first, n) = groups::span(self.wins.iter().map(|w| w.group), g).unwrap_or((wi, 1));
        let kind = self.groups.get(g).map_or(panes::Kind::Single, |g| g.kind);
        Shown { first, grouped: true, kind: panes::fit_kind(kind, n), n, focus: wi.saturating_sub(first).min(n - 1) }
    }

    /// What the grid area shows now.
    fn shown(&self) -> Shown {
        self.shown_for(self.cur)
    }

    /// Pane `i` of what is on screen.
    fn pane(&self, i: usize) -> &Pane {
        let (wi, pj) = self.shown().locate(i);
        &self.wins[wi].panes[pj]
    }

    fn pane_mut(&mut self, i: usize) -> &mut Pane {
        let (wi, pj) = self.shown().locate(i);
        &mut self.wins[wi].panes[pj]
    }

    /// The panes of what is on screen, in order.
    fn shown_panes(&self) -> impl Iterator<Item = &Pane> {
        self.wins[self.shown().run()].iter().flat_map(|w| w.panes.iter())
    }

    fn shown_panes_mut(&mut self) -> impl Iterator<Item = &mut Pane> {
        let run = self.shown().run();
        self.wins[run].iter_mut().flat_map(|w| w.panes.iter_mut())
    }

    /// The pane of the window on screen under window pixel `(x, y)`: with one pane, that is all of the
    /// workspace; with several, the gaps and the padding belong to none.
    fn pane_under(&self, x: f32, y: f32) -> Option<usize> {
        let sh = self.shown();
        if sh.n == 1 {
            return Some(0);
        }
        panes::at(sh.kind, sh.n, self.soft.workspace(self.w, self.h), x, y)
    }

    /// The box of pane `pi` of the window on screen, and whether it has a frame.
    fn pane_box(&self, pi: usize) -> (panes::Rect, bool) {
        let sh = self.shown();
        (panes::rect(sh.kind, sh.n, pi, self.soft.workspace(self.w, self.h)), sh.n > 1)
    }

    /// Puts the windows in the order the panel shows them (a group's windows together, groups in order, the
    /// windows in none last: so `Alt+1..9` and `Alt+[` follow what is on screen) and lists them.
    fn rebuild_side(&mut self) {
        let id = self.wins[self.cur].id;
        self.wins.sort_by_key(|w| w.group.unwrap_or(usize::MAX));
        self.cur = self.wins.iter().position(|w| w.id == id).unwrap_or(0);
        // A group's windows share the grid area, so joining or leaving one changes the size of their terminals.
        self.resize_panes(false);
        // Whatever is in sight now is not "printed while out of sight" (a window dragged into the group on screen).
        let seen = self.shown().run();
        for w in &mut self.wins[seen] {
            w.unread = false;
        }
        // Windows or groups came or went, so what a drag in progress points at is not what it was.
        self.dnd_cancel();
        self.relist();
    }

    /// Lists what the panel or the tabs show for the windows and groups as they are: a folded group still shows
    /// the window on screen if it is one of its own, the tabs list no panes.
    fn relist(&mut self) {
        let of: Vec<Option<usize>> = self.wins.iter().map(|w| w.group).collect();
        let folded: Vec<bool> = self.groups.iter().map(|g| g.collapsed).collect();
        let panes: Vec<usize> = self.wins.iter().map(|w| w.panes.len()).collect();
        self.entries = if self.tabs_on_top() { side::top_entries(&of, &folded, self.cur) } else { side::entries(&of, &folded, &panes, Some(self.cur)) };
        self.refresh_groups();
        self.reveal_active();
        self.side_hover = None;
        self.side_dirty = true;
        self.dirty = true;
    }

    /// Scrolls the tabs at the top, if they need it, so that the window on screen is in view (the reference's
    /// `scrollIntoView` of the active tab); the panel does not scroll.
    fn reveal_active(&mut self) {
        let cur = self.cur;
        let Some(k) = self.entries.iter().position(|e| matches!(e, side::Entry::Row { win, .. } if *win == cur)) else { return };
        let first = match self.list_layout() {
            Some((lay @ side::Layout::Top { .. }, _)) => lay.reveal(k),
            _ => return,
        };
        if first != self.strip_first {
            self.strip_first = first;
            self.side_dirty = true;
            self.dirty = true;
        }
    }

    /// The wheel over the tabs at the top scrolls them, a group or a window a notch (a touchpad's pixels add up to one).
    /// Returns whether the pointer was over them.
    fn scroll_tabs(&mut self, pixels: f32, clicks: i32) -> bool {
        let (x, y) = self.mouse;
        let over = match self.list_layout() {
            Some((side::Layout::Top { .. }, (bx, by, bw, bh))) => x >= bx as f32 && x < (bx + bw) as f32 && y >= by as f32 && y < (by + bh) as f32,
            _ => false,
        };
        if !over {
            return false;
        }
        self.strip_acc += pixels;
        let by = if clicks != 0 { clicks } else { (self.strip_acc / TAB_SCROLL_PX).trunc() as i32 };
        self.strip_acc -= by as f32 * TAB_SCROLL_PX;
        let first = self.list_layout().map_or(self.strip_first, |(lay, _)| lay.scroll(by));
        if first != self.strip_first {
            self.strip_first = first;
            self.side_hover = None;
            self.side_dirty = true;
            self.dirty = true;
        }
        true
    }

    /// The groups as the list shows them, from the windows and groups as they are now.
    fn refresh_groups(&mut self) {
        self.gitems = group_items(&self.groups, &self.wins);
        side::measure(&mut self.gitems, &self.soft);
    }

    /// Drops the groups that no window is in, and renumbers the others in the windows.
    fn prune_groups(&mut self) {
        let mut of: Vec<Option<usize>> = self.wins.iter().map(|w| w.group).collect();
        groups::prune(&mut self.groups, &mut of);
        for (w, g) in self.wins.iter_mut().zip(of) {
            w.group = g;
        }
    }

    /// A new group, named and coloured for the next in line, around the window on screen (a debugging aid:
    /// `AXON_GROUPS`; the dialog, `Alt+G`, is how a group is made).
    fn new_group(&mut self) {
        self.groups_made += 1;
        let color = groups::next_color(&self.groups.iter().map(|g| g.color).collect::<Vec<_>>());
        self.groups.push(Group { id: self.groups_made as u32, name: format!("{} {}", tr(Str::GroupName), self.groups_made), color, collapsed: false, kind: panes::Kind::Single });
        self.wins[self.cur].group = Some(self.groups.len() - 1);
        self.prune_groups();
        self.rebuild_side();
    }

    /// Takes the window on screen out of its group.
    fn ungroup(&mut self) {
        self.move_to_group(self.cur, None);
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
        (self.selecting, self.held, self.repeat, self.cross_down) = (false, None, None, None);
        // Going to a window of a folded group opens the group (the reference's `activate`); the folded groups
        // that remain show the window on screen, which changes.
        let was = self.cur;
        // Going from one window of a group to another keeps the group in view: the program in the pane that loses the
        // focus is told so, as with panes, and the one that gets it too.
        if self.ui.focused && i != was && self.wins[i].group.is_some() && self.wins[i].group == self.wins[was].group {
            report_focus(self.wins[was].pane(), false);
            report_focus(self.wins[i].pane(), true);
        }
        self.cur = i;
        if i != was {
            let unfolded = self.wins[i].group.is_some_and(|g| std::mem::take(&mut self.groups[g].collapsed));
            if unfolded || self.groups.iter().any(|g| g.collapsed) {
                self.relist();
            }
        }
        self.pane_hover = None;
        self.reveal_active();
        // Everything in sight is read again: the window, or all of its group.
        let run = self.shown().run();
        for w in &mut self.wins[run] {
            w.unread = false;
            for p in &mut w.panes {
                p.ctx_dirty = true;
                p.refresh.due = Some(Instant::now());
                p.read_dir();
            }
        }
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
                report_focus(self.wins[self.cur].pane(), f);
                self.dirty = true;
            }
            Event::Close => self.quit = true,
            Event::FrameDone => {}
            #[cfg(target_os = "linux")]
            Event::Keymap(text) => {
                if let Some(x) = self.xkb.as_mut() {
                    x.set_keymap(&text);
                }
            }
            #[cfg(target_os = "linux")]
            Event::Modifiers { depressed, latched, locked, group } => {
                if let Some(x) = self.xkb.as_mut() {
                    x.update_mods(depressed, latched, locked, group);
                }
            }
            Event::RepeatInfo { rate, delay } => (self.rate, self.delay) = (rate, delay),
            Event::KeyboardLeave => self.repeat = None,
            #[cfg(target_os = "linux")]
            Event::Key { key, pressed } => self.on_key(key, pressed),
            // A key the platform already turned into what it means: it repeats by itself, so nothing is held here.
            Event::Press { key, repeat } => {
                // Held, a key goes on only if its first press may repeat (the shortcuts that act once per press do not).
                if !repeat || self.press_repeats {
                    self.press_repeats = self.handle_key(&key);
                }
            }
            #[cfg(not(target_os = "linux"))]
            Event::Keymap(_) | Event::Modifiers { .. } | Event::Key { .. } => {}
            Event::Scroll { pixels, clicks } => self.on_scroll(pixels, clicks),
            Event::PointerMove { x, y } => self.on_pointer_move(x, y),
            Event::PointerLeave => {
                if !self.overlays.is_empty() {
                    // Nothing is under the pointer any more: a lit menu item or toast button goes dark.
                    let g = self.geo();
                    let r = self.overlays.pointer(-1e4, -1e4, g);
                    self.overlay_reply(Reply { cursor: None, ..r });
                }
                self.hover = Hit::Client;
                if let Some(p) = self.panel.as_mut() {
                    let e = p.pointer(-1.0, -1.0);
                    self.panel_effect(e);
                }
                if self.side_hover.take().is_some() {
                    self.side_dirty = true;
                    self.dirty = true;
                }
                if self.pane_hover.take().is_some() || self.close_hot {
                    self.close_hot = false;
                    self.dirty = true;
                }
                if let Some((pi, r)) = self.grid_hover.take()
                    && self.chips_on()
                    && self.pane(pi).chips.block_of(r).is_some()
                {
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
                if self.overlay_button(b, pressed, time) {
                    return;
                }
                // A window's name being edited: a press on its field is the field's, a press elsewhere ends the edit
                // and goes on; a right click on the list of windows opens the menu of what is under it.
                if b == 0 && !pressed {
                    self.rename_release();
                }
                if b == 0 && pressed && self.rename.is_some() && self.rename_press(self.mouse.0, self.mouse.1, time) {
                    return;
                }
                if b == 2 && pressed && self.open_context_menu(self.mouse.0, self.mouse.1) {
                    return;
                }
                if b == 1 && pressed && self.middle_click_close() {
                    return;
                }
                if self.panel.is_some() && b == 0 && self.panel_button(pressed) {
                    return;
                }
                // A press on a pane gives it the focus, whatever the program does with the click.
                if pressed
                    && self.hover == Hit::Client
                    && let Some(pi) = self.pane_under(self.mouse.0, self.mouse.1)
                {
                    self.set_focus(pi);
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
        self.plat.set_geometry(ml as i32, mt as i32, self.w as i32, self.h as i32, grab);
        self.set_blur();
    }

    /// Asks the compositor to blur behind the window when the settings say so (its rounded outline).
    fn set_blur(&mut self) {
        if !self.cfg.blur {
            return self.plat.set_blur(&[]);
        }
        let (ml, mt, _) = chrome::margins(self.ui.maximized);
        let radius = if self.ui.maximized { 0 } else { chrome::RADIUS as i32 };
        let (rects, n) = chrome::blur_strips(ml as i32, mt as i32, self.w as i32, self.h as i32, radius);
        self.plat.set_blur(&rects[..n]);
    }

    /// Grid size of every pane of every window, after a resize, a new font size, a split or a closed pane: each
    /// terminal and its PTY are told the size of its own box. A window with one pane has the whole workspace.
    fn relayout(&mut self) {
        self.apply_inset();
        let ws = self.soft.workspace(self.w, self.h);
        let lone = self.soft.grid(ws, false);
        (self.cols, self.rows) = (lone.cols, lone.rows);
        self.resize_panes(true);
        self.reveal_active();
        self.side_dirty = true;
        self.dirty = true;
    }

    /// Tells every terminal and PTY the size of its own box: a pane of a window has its share of the workspace, and so
    /// has a window of a group (ADR 0034). `all` repaints the chips of the ones that did not change too (a new font).
    fn resize_panes(&mut self, all: bool) {
        let ws = self.soft.workspace(self.w, self.h);
        for wi in 0..self.wins.len() {
            let sh = self.shown_for(wi);
            for pj in 0..self.wins[wi].panes.len() {
                let i = if sh.grouped { wi - sh.first } else { pj };
                let g = self.soft.grid(panes::rect(sh.kind, sh.n, i, ws), sh.n > 1);
                let p = &mut self.wins[wi].panes[pj];
                let moved = p.size != (g.cols, g.rows);
                if moved {
                    p.size = (g.cols, g.rows);
                    session::resize(&p.term, &p.pty, g.cols, g.rows, self.cw, self.ch);
                }
                p.ctx_dirty |= all || moved;
            }
        }
    }

    // ------------------------------------------------------- prompt chips --

    /// Whether the next shell starts in a scope of its own: the setting says so (it follows a reload) and the machine
    /// can (tried once, here, when the file did not want it at the start and wants it now).
    fn scope_for_new(&mut self) -> bool {
        self.cfg.windows.scope && *self.scope_ok.get_or_insert_with(|| session::scope_probe(true).works())
    }

    /// Whether the chips are shown: asked for and with at least one chip to show.
    fn chips_on(&self) -> bool {
        self.cfg.input.inline && !self.cfg.input.chips.is_empty()
    }

    /// Puts the flag file where the shell's hook looks: it exists while the chips are on.
    fn sync_flag(&self) {
        let on = self.chips_on();
        for f in self.wins.iter().flat_map(|w| &w.panes).filter_map(|p| p.flag.as_ref()) {
            if on {
                let _ = std::fs::write(f, b"");
            } else {
                let _ = std::fs::remove_file(f);
            }
        }
    }

    /// Asks for the chips to be read again: at once, or at most `CONTEXT_EVERY` after the last time.
    fn context_due(&mut self, pi: usize, now: Instant) {
        if !self.chips_on() {
            return;
        }
        let r = &mut self.pane_mut(pi).refresh;
        let at = now.max(r.last + CONTEXT_EVERY);
        r.due = Some(r.due.map_or(at, |d| d.min(at)));
    }

    /// Takes in the finished refreshes of the panes on screen, starts the ones that are due, and keeps the minute timers.
    fn tick_context(&mut self) {
        for pi in 0..self.shown().n {
            self.tick_pane_context(pi);
        }
    }

    fn tick_pane_context(&mut self, pi: usize) {
        let (cur, pj) = self.shown().locate(pi);
        let pane = &mut self.wins[cur].panes[pj];
        let done = pane.refresh.done.lock().unwrap().take();
        if let Some(c) = done
            && c != pane.ctx
        {
            let moved = c.cwd != pane.ctx.cwd;
            pane.ctx = c;
            pane.history.set_ctx(&pane.ctx);
            pane.ctx_dirty = true;
            self.dirty = true;
            if moved {
                pane.read_dir();
                self.titled(cur);
            }
        }
        if !self.chips_on() {
            let r = &mut self.wins[cur].panes[pj].refresh;
            (r.due, r.clock_at) = (None, None);
            return;
        }
        let now = Instant::now();
        let r = &self.wins[cur].panes[pj].refresh;
        if r.clock_at.is_none_or(|t| now >= t) && self.cfg.input.chips.contains(&Chip::Time) {
            if r.clock_at.is_some() {
                self.context_due(pi, now);
            }
            self.wins[cur].panes[pj].refresh.clock_at = Some(now + context::until_next_minute());
        }
        let pane = &mut self.wins[cur].panes[pj];
        if pane.refresh.due.is_some_and(|d| now >= d) {
            if pane.refresh.running.swap(true, Ordering::AcqRel) {
                pane.refresh.due = Some(now + Duration::from_millis(200)); // one at a time
                return;
            }
            pane.refresh.due = None;
            pane.refresh.last = now;
            let probe = Probe { pid: pane.shell, cwd: pane.cwd.clone(), report: pane.report.clone(), clock24: self.cfg.input.clock24 };
            let (done, running, proxy) = (pane.refresh.done.clone(), pane.refresh.running.clone(), pane.proxy.clone());
            let spawned = std::thread::Builder::new().name("context".into()).spawn(move || {
                let c = context::gather(&probe);
                *done.lock().unwrap() = Some(c);
                running.store(false, Ordering::Release);
                proxy.wake();
            });
            if spawned.is_err() {
                pane.refresh.running.store(false, Ordering::Release);
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
            .with_blur(self.cfg.blur, self.plat.blur_supported())
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

    /// Opens the panel, or, when it is open, gives it the keyboard (the palette's "Settings").
    fn open_settings(&mut self) {
        match self.panel.as_mut() {
            None => self.toggle_settings(),
            Some(p) => {
                p.focus = true;
                self.prefs_dirty = true;
                self.dirty = true;
            }
        }
    }

    /// Closes the panel: what it changed last is written first, and the grid takes its room back.
    fn close_settings(&mut self) {
        self.flush_save();
        if self.panel.take().is_none() {
            return;
        }
        crate::font::release_memory();
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
            self.relist();
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
            for p in self.wins.iter().flat_map(|w| &w.panes) {
                session::set_cursor(&p.term, input.cursor);
            }
        }
        self.cfg.input = input.clone();
        let on = self.chips_on();
        for p in self.shown_panes_mut() {
            p.refresh.clock_at = None;
            if on {
                p.refresh.due = Some(Instant::now());
                if clock_changed {
                    p.ctx = Context::default(); // the clock text is rebuilt by the refresh
                    p.history.set_ctx(&p.ctx);
                }
            }
            p.ctx_dirty = true;
        }
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
        for p in self.wins.iter().flat_map(|w| &w.panes) {
            session::set_cursor(&p.term, cfg.input.cursor);
        }
        self.cfg = cfg.clone();
        self.relist();
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
        for p in self.wins.iter_mut().flat_map(|w| &mut w.panes) {
            p.refresh.clock_at = None;
        }
        if self.chips_on() {
            for p in self.shown_panes_mut() {
                p.refresh.due = Some(Instant::now());
            }
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

    /// The modifiers held now.
    fn mods(&self) -> Mods {
        #[cfg(target_os = "linux")]
        return self.xkb.as_ref().map(|x| x.mods()).unwrap_or_default();
        #[cfg(windows)]
        return crate::win::mods();
    }

    #[cfg(target_os = "linux")]
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

    /// Runs a shortcut or sends the key to the focused pane's shell. Returns whether anything was done and the
    /// key may repeat while held: splitting and closing a pane do once per press.
    fn handle_key(&mut self, k: &Key) -> bool {
        // A window or group being dragged: Esc drops the drag, nothing moves.
        if k.sym == KS_ESCAPE && self.dnd.as_ref().is_some_and(|d| d.live) {
            self.dnd_cancel();
            return true;
        }
        // An overlay open has the keyboard before everything: the shell hears nothing, Esc closes the top one.
        if self.overlays.is_open() {
            let g = self.geo();
            let r = self.overlays.key(k, g, &mut PlatClip(&mut self.plat));
            let closed = r.closed;
            self.overlay_reply(r);
            return !closed; // the key that closed it must not repeat into the shell
        }
        // A window's name being edited has the keyboard: Enter commits, Esc cancels, the shell hears nothing.
        if let Some(keep) = self.rename_key(k) {
            return keep;
        }
        if self.f2_rename(k) {
            return false;
        }
        let m = k.mods;
        if m.ctrl && !m.alt && !m.shift && k.sym == 0x2c {
            self.toggle_settings();
            return false; // once per press: held, it would open and close the panel
        }
        // Ctrl+K: the command palette (the shell does not get it, as in the reference). Not a key that repeats.
        if m.ctrl && !m.alt && !m.shift && matches!(k.sym, 0x6b | 0x4b) {
            self.open_palette();
            return false;
        }
        // The settings panel has the keyboard (it opened, or was clicked, and the terminal was not clicked since):
        // the app's own shortcuts still work, and everything else is for the panel. Escape closes it.
        let docked = self.panel.as_ref().is_some_and(|p| p.focus);
        // Ctrl+Space: one more pane in this window (the reference takes it from the shell too).
        if m.ctrl && !m.alt && !m.shift && k.sym == 0x20 {
            self.split(panes::Dir::Auto);
            return false;
        }
        let letter = char::from_u32(k.sym).map(|c| c.to_ascii_lowercase());
        if m.alt && !m.ctrl {
            let last = self.wins.len() - 1;
            match (letter, k.sym) {
                (Some('n'), _) => {
                    self.new_win();
                    return false;
                }
                (Some('w'), _) if m.shift => {
                    self.close_pane_asked(self.cur, self.wins[self.cur].focus);
                    return false;
                }
                (Some('w'), _) => {
                    self.close_wins(&[self.wins[self.cur].id], None);
                    return false;
                }
                (Some('b'), _) => {
                    self.toggle_side();
                    return false;
                }
                (Some('o'), _) => {
                    self.toggle_overview();
                    return false;
                }
                (Some('g'), _) if m.shift => self.ungroup(),
                (Some('g'), _) => {
                    self.group_dialog_here();
                    return false;
                }
                (_, 0x5c) => {
                    self.split(panes::Dir::Cols);
                    return false;
                }
                (_, 0x2d) => {
                    self.split(panes::Dir::Rows);
                    return false;
                }
                (_, 0x2c) => self.cycle_pane(-1),
                (_, 0x2e) => self.cycle_pane(1),
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
                Some('r') => {
                    self.reload();
                    return false;
                }
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
                self.wins[self.cur].pane().term.lock().scroll_display(s);
                self.dirty = true;
                return true;
            }
        }
        let pane = self.wins[self.cur].pane();
        let mode = *pane.term.lock().mode();
        let Some(bytes) = input::encode_key(k, mode) else { return false };
        let mut t = pane.term.lock();
        if t.grid().display_offset() > 0 {
            t.scroll_display(Scroll::Bottom);
        }
        t.selection = None;
        drop(t);
        pane.pty.notify(bytes);
        self.dirty = true;
        true
    }

    // ------------------------------------------------------------ overlays --

    /// The window as the overlays see it.
    fn geo(&self) -> Geo {
        Geo { w: self.w as i32, h: self.h as i32, square: self.ui.maximized }
    }

    /// Whatever the pointer or a held key was doing stops when an overlay takes the window.
    fn grab_input(&mut self) {
        (self.selecting, self.held, self.repeat) = (false, None, None);
        self.dnd_cancel();
        self.wins[self.cur].pane().term.lock().selection = None;
        // A slider being dragged in the settings panel lets go: the overlay has the pointer now.
        if let Some(p) = self.panel.as_mut() {
            let e = p.release();
            self.panel_effect(e);
        }
    }

    /// Opens an overlay over the window (the palette, the overview, a dialog, a menu) under the app's `tag` for
    /// it; the settings panel is docked beside the grid and stays open under it. Its command comes back
    /// through `overlay_ran`.
    fn open_overlay(&mut self, tag: u32, o: Box<dyn Overlay>) {
        self.grab_input();
        self.overlays.open(tag, o);
        self.dirty = true;
    }

    /// Opens one of the kit's debug screens by name (`AXON_DEMO_OVERLAY`), or the real palette (`palette`). False if
    /// there is no such one.
    fn demo_overlay(&mut self, kind: &str) -> bool {
        if kind == "palette" {
            self.open_palette();
            return true;
        }
        self.grab_input();
        let g = self.geo();
        let ok = overlay_demo::open(kind, &mut self.overlays, g, Instant::now());
        self.dirty |= ok;
        ok
    }

    /// What the overlays did with an input: repaint, paint the grid again where one went away, the pointer
    /// image they want, and the command they ran.
    fn overlay_reply(&mut self, r: Reply) {
        if r.redraw || r.closed {
            self.dirty = true;
        }
        if r.closed {
            self.soft.invalidate();
            self.repeat = None;
            if self.overlays.is_empty() {
                self.overlay_cursor = None;
                self.plat.set_cursor(cursor_for(self.hover));
            }
        }
        if let Some(c) = r.cursor.filter(|&c| Some(c) != self.overlay_cursor) {
            self.overlay_cursor = Some(c);
            self.plat.set_cursor(match c {
                overlay::Cursor::Default => cursor::DEFAULT,
                overlay::Cursor::Pointer => cursor::POINTER,
                overlay::Cursor::Text => cursor::TEXT,
            });
        }
        if let Some(ran) = r.ran {
            self.overlay_ran(ran);
        }
    }

    /// A command an overlay chose. The kit's own screens only say so; the real ones (palette, menus,
    /// dialogs) match on their tag here.
    fn overlay_ran(&mut self, ran: Ran) {
        match ran.tag {
            commands::TAG => {
                if let Some(cmd) = Cmd::from_id(ran.id) {
                    self.run_cmd(cmd);
                }
            }
            overlay_demo::TAG => {
                eprintln!("axon: demo overlay ran command {}", ran.id);
                self.overlays.toast(tr(Str::OvToastClosed), Some(tr(Str::OvUndo)), overlay_demo::CMD_UNDO, Instant::now());
            }
            overview::TAG => self.overview_ran(ran.id),
            overlay::TOAST => self.toast_ran(ran.id),
            ctxmenu::TAG => self.menu_ran(ran.id),
            group_dialog::TAG => self.dialog_ran(ran.id),
            _ => {}
        }
    }

    /// Opens the command palette (`Ctrl+K`) over the windows and groups there are now.
    fn open_palette(&mut self) {
        let wins: Vec<commands::WinInfo> = self.wins.iter().map(|w| commands::WinInfo { id: w.id, title: win_title(w).to_string(), place: win_place(w).to_string(), group: w.group }).collect();
        let groups = group_items(&self.groups, &self.wins);
        let state = commands::State { wins: &wins, groups: &groups, cur: self.cur, panes: self.shown().n, overview: true };
        let list = commands::build(&state);
        self.open_overlay(commands::TAG, Box::new(Picker::new(list)));
    }

    /// What a palette entry does: the same call the entry's shortcut makes, so the two cannot differ.
    fn run_cmd(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::NewWindow => self.new_win(),
            Cmd::NewGroup => self.group_dialog_here(),
            Cmd::NewPane => self.split(panes::Dir::Auto),
            Cmd::SplitCols => self.split(panes::Dir::Cols),
            Cmd::SplitRows => self.split(panes::Dir::Rows),
            Cmd::Overview => self.toggle_overview(),
            Cmd::TogglePanel => self.toggle_side(),
            Cmd::Tabs(pos) => self.set_tabs(pos),
            Cmd::HistorySearch => self.type_into_shell(b"\x12"),
            Cmd::ClearScreen => self.type_into_shell(b"\x0c"),
            Cmd::Settings => self.open_settings(),
            Cmd::CloseWindow => self.close_wins(&[self.wins[self.cur].id], None),
            Cmd::ClosePane => self.close_pane_asked(self.cur, self.wins[self.cur].focus),
            Cmd::Ungroup => self.ungroup(),
            // The palette names windows and groups by id: where they are now is looked up, as `menu_ran` does.
            Cmd::Window(id) => {
                if let Some(i) = self.wins.iter().position(|w| commands::short(w.id) == id) {
                    self.switch_to(i);
                }
            }
            Cmd::Group(id) => {
                if let Some(g) = groups::index_of(&self.groups, id)
                    && let Some(i) = self.wins.iter().position(|w| w.group == Some(g))
                {
                    self.switch_to(i);
                }
            }
            Cmd::Fold(id) => {
                if let Some(g) = groups::index_of(&self.groups, id) {
                    self.fold(g);
                }
            }
        }
    }

    /// Types `bytes` into the shell of the pane with the focus, as the key would (`Ctrl+R`, `Ctrl+L`): the view
    /// goes back to the bottom of the scrollback first.
    fn type_into_shell(&mut self, bytes: &'static [u8]) {
        let pane = self.wins[self.cur].pane();
        let mut t = pane.term.lock();
        if t.grid().display_offset() > 0 {
            t.scroll_display(Scroll::Bottom);
        }
        t.selection = None;
        drop(t);
        pane.pty.notify(bytes);
        self.dirty = true;
    }

    /// A button press or release while overlays are shown. Returns whether they took it: with one open every
    /// press in the window's client area is theirs (on the scrim it closes the top one), and over a toast the
    /// toast's.
    fn overlay_button(&mut self, button: u8, pressed: bool, time: u32) -> bool {
        let (x, y) = self.mouse;
        if !self.overlay_owns(x, y) {
            return false;
        }
        if button == 0 {
            // A drag from the list that the overlays took the pointer from (the overview's area, a toast) ends here: its
            // release is theirs, so it would never reach `dnd_release`, and a drop would come later at a stale target.
            if !pressed {
                self.dnd_cancel();
            }
            let g = self.geo();
            let r = self.overlays.button(Click { x, y, pressed, time }, g);
            self.overlay_reply(r);
        }
        true
    }

    /// Whether the overlays own the pointer at window pixel `(x, y)`: over a toast, or, with one open, anywhere
    /// but the window's own controls (the buttons, the edges, the title bar where it moves the window).
    fn overlay_owns(&self, x: f32, y: f32) -> bool {
        if self.overlays.is_empty() {
            return false;
        }
        if self.overlays.on_toast(x, y, self.geo()) {
            return true;
        }
        if !self.overlays.is_open() {
            return false;
        }
        // The overview is the workspace's: the panel of windows, the tabs and the title bar beside it stay live.
        if self.overlays.top() == Some(overview::TAG) {
            return overlay::inside(self.overview_area(), x, y);
        }
        match chrome::hit(x, y, self.w, self.h, self.ui.maximized, self.ui.side != 0) {
            Hit::Client | Hit::Side | Hit::Prefs | Hit::Settings | Hit::SideBtn | Hit::NewGroup => true,
            // The tabs in the title bar are part of what is dimmed; the bar itself still moves the window.
            Hit::Title => self.side_part_at(x, y).is_some(),
            Hit::Close | Hit::Minimize | Hit::Maximize | Hit::Edge(_) => false,
        }
    }

    /// A key for the settings panel; it never reaches the shell.
    fn panel_key(&mut self, k: &Key) -> bool {
        let Some(p) = self.panel.as_mut() else { return false };
        let e = p.key(k);
        self.panel_effect(e);
        // The key that closed the panel (Escape) must not repeat: held on, it would go into the shell.
        self.panel.is_some()
    }

    // ---------------------------------------------------------- clipboard --

    fn paste(&mut self) -> bool {
        let Some(text) = self.plat.clipboard_text() else { return true };
        let text = text.replace('\x1b', "");
        let pane = self.wins[self.cur].pane();
        let bracketed = pane.term.lock().mode().contains(TermMode::BRACKETED_PASTE);
        if bracketed {
            pane.pty.notify(format!("\x1b[200~{text}\x1b[201~").into_bytes());
        } else {
            pane.pty.notify(text.replace("\r\n", "\r").replace('\n', "\r").into_bytes());
        }
        true
    }

    fn copy(&mut self) -> bool {
        let text = self.wins[self.cur].pane().term.lock().selection_to_string();
        if let Some(t) = text {
            self.plat.set_clipboard(&t);
        }
        true
    }

    // ------------------------------------------------------------ pointer --

    /// The pane a gesture in progress (a selection, a held button) belongs to: the one it began on.
    fn drag_pane(&self) -> usize {
        self.drag.min(self.shown().n - 1)
    }

    /// The cell of pane `pi` under the pointer `(col, row)`, and which half of it: the pointer outside the pane's
    /// grid counts as the nearest cell (a drag that left the pane goes on selecting in it). The row is one of the
    /// pane's rows as its last frame laid the lines on them (`view::Fold`): the composer's are below its gap.
    fn viewport_cell(&self, pi: usize) -> (usize, usize, Side) {
        let cw = self.cw as f32;
        let (rect, framed) = self.pane_box(pi);
        let g = self.soft.grid(rect, framed);
        let (cols, rows) = self.pane(pi).size;
        let fx = ((self.mouse.0 - g.x as f32) / cw).max(0.0);
        let col = (fx as usize).min(cols - 1);
        let row = self.soft.row_at(pi, self.mouse.1).min(rows - 1);
        (col, row, if fx.fract() < 0.5 { Side::Left } else { Side::Right })
    }

    /// The line of pane `pi`'s terminal its row `row` shows (the history's are negative): a blank row above the
    /// first line in view stands for that line.
    fn row_line(&self, pi: usize, row: usize) -> i32 {
        let fold = self.pane(pi).view.fold();
        if fold.rows == 0 {
            let off = self.pane(pi).term.lock().grid().display_offset();
            return row as i32 - off as i32;
        }
        fold.line(row).unwrap_or(fold.floor)
    }

    /// The terminal point of pane `pi` under the pointer, and which half of its cell.
    fn cell_at(&self, pi: usize) -> (Point, Side) {
        let (col, row, side) = self.viewport_cell(pi);
        (Point::new(Line(self.row_line(pi, row)), Column(col)), side)
    }

    /// Whether the pointer feeds the program in pane `pi` instead of selecting: it asked for reports, and Shift
    /// (the usual way out, to select text anyway) is not held.
    fn reporting(&self, pi: usize) -> bool {
        let shift = self.mods().shift;
        !shift && mouse::reporting(*self.pane(pi).term.lock().mode())
    }

    /// Sends `ev` to pane `pi` at the cell under the pointer. Returns whether a report went out.
    fn report(&mut self, pi: usize, ev: mouse::Event) -> bool {
        let (col, row, _) = self.viewport_cell(pi);
        // The program counts the rows of its screen: the line the row shows.
        let screen = self.row_line(pi, row).clamp(0, self.pane(pi).size.1 as i32 - 1) as usize;
        let mods = self.mods();
        let (wi, pj) = self.shown().locate(pi);
        let pane = &self.wins[wi].panes[pj];
        let mode = *pane.term.lock().mode();
        let Some(r) = mouse::encode(mode, ev, mods, col, screen) else { return false };
        self.reported = (col, row);
        pane.pty.notify(r.bytes().to_vec());
        true
    }

    /// A button press or release in mouse-reporting mode. Returns whether the program took it.
    fn report_button(&mut self, b: u8, pressed: bool) -> bool {
        if pressed {
            if self.hover != Hit::Client || self.held.is_some() {
                return false;
            }
            let Some(pi) = self.pane_under(self.mouse.0, self.mouse.1).filter(|&pi| self.reporting(pi)) else { return false };
            (self.held, self.drag) = (Some(b), pi);
            self.pane(pi).term.lock().selection = None;
            self.dirty = true;
            self.report(pi, mouse::Event::Press(b));
            true
        } else if self.held == Some(b) {
            self.held = None;
            self.report(self.drag_pane(), mouse::Event::Release(b));
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
        if self.rename_drag(x) {
            return;
        }
        if !self.overlays.is_empty() {
            let g = self.geo();
            let r = self.overlays.pointer(x, y, g);
            if self.overlay_owns(x, y) {
                // The pointer went from a drag in the list onto the overlay: the drag is over, no ghost is left behind.
                self.dnd_cancel();
                self.overlay_reply(r);
                // The window's own hover lights (title buttons, the panels' rows) go dark under an overlay.
                if self.hover != Hit::Client || self.ui.hover != 0 || self.side_hover.is_some() {
                    (self.hover, self.ui.hover, self.side_hover) = (Hit::Client, 0, None);
                    self.side_dirty = true;
                    self.dirty = true;
                }
                if let Some(p) = self.panel.as_mut() {
                    let e = p.pointer(-1.0, -1.0);
                    self.panel_effect(e);
                }
                return;
            }
            // Off the overlays, the window's own pointer image is back: the next reply sets theirs again.
            self.overlay_cursor = None;
            self.overlay_reply(Reply { cursor: None, ..r });
        }
        if self.dnd_move(x, y) {
            return;
        }
        if let Some(p) = self.panel.as_mut() {
            let e = p.pointer(x, y);
            self.panel_effect(e);
        }
        // A gesture goes on in the pane it began in; a plain move is the business of the pane under the pointer.
        let target = if self.held.is_some() || self.selecting {
            Some(self.drag_pane())
        } else if self.hover == Hit::Client {
            self.pane_under(x, y)
        } else {
            None
        };
        if let Some(pi) = target.filter(|&pi| self.held.is_some() || self.reporting(pi)) {
            // Programs hear about a move only when it crosses into another cell.
            let (col, row, _) = self.viewport_cell(pi);
            if (col, row) != self.reported {
                self.report(pi, mouse::Event::Move(self.held));
            }
            if self.held.is_some() {
                return;
            }
        }
        if self.selecting {
            let pi = self.drag_pane();
            let (p, side) = self.cell_at(pi);
            if let Some(sel) = self.pane(pi).term.lock().selection.as_mut() {
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
        // The row of the grid under the pointer: a command's status shows while the pointer is over its block.
        // Only a move into another command's block is drawn (with the chips on): a move within one changes nothing.
        let row = if now == Hit::Client { self.pane_under(x, y).map(|pi| (pi, self.viewport_cell(pi).1)) } else { None };
        if row != self.grid_hover {
            let block = |h: Option<(usize, usize)>| h.and_then(|(pi, r)| self.pane(pi).chips.block_of(r).map(|b| (pi, b)));
            if self.chips_on() && block(row) != block(self.grid_hover) {
                self.dirty = true;
            }
            self.grid_hover = row;
        }
        // Over a pane of several: its cross shows at the corner, and the pointer is a hand on it.
        let hp = if now == Hit::Client && self.shown().n > 1 { self.pane_under(x, y) } else { None };
        let hot = hp.is_some_and(|pi| panes::close_box(self.pane_box(pi).0).contains(x, y));
        if (hp, hot) != (self.pane_hover, self.close_hot) {
            (self.pane_hover, self.close_hot) = (hp, hot);
            self.dirty = true;
        }
        // One shape for what is under the pointer (the call is a no-op when it is the one already set).
        let shape = if hot {
            cursor::POINTER
        } else if now == Hit::Prefs {
            self.prefs_cursor()
        } else if part.is_some() {
            cursor::POINTER
        } else {
            cursor_for(now)
        };
        self.plat.set_cursor(shape);
    }

    fn on_button(&mut self, pressed: bool, time: u32) {
        if !pressed {
            // The button came up on a row of the list or on a drag: a click, or a drop.
            if self.dnd_release() {
                return;
            }
            self.selecting = false;
            // The cross of a pane closes it when the button comes up over it too.
            if let Some(pi) = self.cross_down.take()
                && self.close_hot && self.pane_hover == Some(pi)
            {
                let (wi, pj) = self.shown().locate(pi);
                self.close_pane_asked(wi, pj);
            }
            if self.ui.down {
                self.ui.down = false;
                self.dirty = true;
                match self.hover {
                    Hit::Close => self.quit = true,
                    Hit::Minimize => self.plat.minimize(),
                    Hit::Maximize => self.plat.toggle_maximized(),
                    Hit::Settings => self.toggle_settings(),
                    Hit::SideBtn => self.toggle_side(),
                    Hit::NewGroup => self.group_dialog_here(),
                    _ => {}
                }
            }
            return;
        }
        match self.hover {
            Hit::Close | Hit::Minimize | Hit::Maximize | Hit::Settings | Hit::SideBtn | Hit::NewGroup => {
                self.ui.down = true;
                self.dirty = true;
            }
            Hit::Title if self.title_click != 0 && time.wrapping_sub(self.title_click) < DOUBLE_CLICK_MS => {
                self.title_click = 0;
                self.plat.toggle_maximized();
            }
            Hit::Title => {
                self.title_click = time;
                self.plat.begin_move();
            }
            Hit::Edge(edges) => self.plat.begin_resize(edges),
            Hit::Prefs => {}
            Hit::Side => match self.side_hover {
                // A row or a header is a click when the button comes up where it went down, a drag when it moves. A second
                // click on a row soon after the first renames its window, on a header edits the group (`dnd_release`).
                Some(part @ (side::Part::Row(_) | side::Part::Head(_))) => self.dnd_press(part, time),
                Some(side::Part::Close(i)) => self.close_wins(&[self.wins[i].id], None),
                Some(side::Part::New) => self.new_win(),
                Some(side::Part::NewGroup) => self.group_dialog_here(),
                Some(side::Part::Pane(wi, pi)) => {
                    self.go_to(wi);
                    self.set_focus(pi);
                }
                Some(side::Part::ClosePane(wi, pi)) => self.close_pane_asked(wi, pi),
                None => {}
            },
            Hit::Client => {
                // The gaps between panes belong to none; a press on a pane's cross waits for the button to come up.
                let Some(pi) = self.pane_under(self.mouse.0, self.mouse.1) else { return };
                if self.close_hot && self.pane_hover == Some(pi) {
                    self.cross_down = Some(pi);
                    return;
                }
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
                let (p, side) = self.cell_at(pi);
                self.pane(pi).term.lock().selection = Some(Selection::new(ty, p, side));
                (self.selecting, self.drag) = (true, pi);
                self.dirty = true;
            }
        }
    }

    fn on_scroll(&mut self, pixels: f32, clicks: i32) {
        // Over the settings panel the wheel scrolls its body (a notch is `WHEEL_PX`); elsewhere the terminal.
        // (An overlay that owns the pointer here has the wheel too, as it has the presses: nothing scrolls behind its scrim.)
        if self.in_prefs(self.mouse.0, self.mouse.1) && !self.overlay_owns(self.mouse.0, self.mouse.1) {
            let px = if clicks != 0 { clicks * settings::WHEEL_PX } else { pixels.round() as i32 };
            let e = self.panel.as_mut().map_or(Effect::None, |p| p.scroll_by(px));
            self.panel_effect(e);
            return;
        }
        if self.overlays.is_open() {
            // The wheel is the top overlay's, a notch as many pixels as the terminal's three lines.
            let dy = if clicks != 0 { clicks as f32 * WHEEL_LINES * self.ch as f32 } else { pixels };
            let (g, (x, y)) = (self.geo(), self.mouse);
            let r = self.overlays.scroll(dy, x, y, g);
            self.overlay_reply(r);
            return;
        }
        if self.scroll_tabs(pixels, clicks) {
            return;
        }
        // Wayland counts down as positive; the terminal's `Scroll::Delta` counts up into history.
        self.scroll_acc -= if clicks != 0 { clicks as f32 * WHEEL_LINES } else { pixels / self.ch as f32 };
        let lines = self.scroll_acc.trunc() as i32;
        if lines == 0 {
            return;
        }
        self.scroll_acc -= lines as f32;
        // The wheel is for the pane under the pointer, focused or not.
        let pi = if self.hover == Hit::Client { self.pane_under(self.mouse.0, self.mouse.1) } else { None }.unwrap_or(self.shown().focus);
        if self.hover == Hit::Client && self.reporting(pi) {
            for _ in 0..lines.unsigned_abs() {
                self.report(pi, mouse::Event::Wheel(lines > 0));
            }
            return;
        }
        let pane = self.pane(pi);
        let mode = *pane.term.lock().mode();
        if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) {
            let k = match (lines > 0, mode.contains(TermMode::APP_CURSOR)) {
                (true, true) => &b"\x1bOA"[..],
                (true, false) => &b"\x1b[A"[..],
                (false, true) => &b"\x1bOB"[..],
                (false, false) => &b"\x1b[B"[..],
            };
            for _ in 0..lines.unsigned_abs() {
                pane.pty.notify(k);
            }
        } else {
            pane.term.lock().scroll_display(Scroll::Delta(lines));
            self.dirty = true;
        }
    }
}

// ---------------------------------------------------------- list: drag and drop, spinners --

impl App {
    /// The button went down on a window's row or tab, or a group's header or chip (`part`): it is a click when it
    /// comes up without the pointer having moved far, and a drag when it has (`dnd.rs`).
    fn dnd_press(&mut self, part: side::Part, time: u32) {
        let grab = match part {
            side::Part::Row(w) => dnd::Grab::Win(w),
            side::Part::Head(g) => dnd::Grab::Group(g),
            _ => return,
        };
        let Some(r) = self.list_layout().and_then(|(lay, _)| lay.grab_box(grab)) else { return };
        let (x, y) = self.mouse;
        self.dnd = Some(Dnd { grab, time, press: (x, y), off: (x - r.0 as f32, y - r.1 as f32), size: (r.2, r.3), live: false, target: None, at: (x, y) });
    }

    /// The pointer moved to window pixel `(x, y)` with a press on the list. True when that made it a drag (or it
    /// already was): the move is the drag's, nothing else hears of it. The mark follows what is under the pointer,
    /// and the panel or the tabs are painted again with the ghost where the pointer is.
    fn dnd_move(&mut self, x: f32, y: f32) -> bool {
        let Some(d) = self.dnd.as_ref() else { return false };
        if !d.live && !dnd::started(d.press, (x, y)) {
            return false;
        }
        let grab = d.grab;
        let target = self.list_layout().filter(|(_, (bx, by, bw, bh))| x >= *bx as f32 && x < (bx + bw) as f32 && y >= *by as f32 && y < (by + bh) as f32).and_then(|(lay, _)| lay.target(grab, x, y));
        let Some(d) = self.dnd.as_mut() else { return false };
        let changed = !d.live || d.target != target || d.at != (x, y);
        (d.live, d.at, d.target) = (true, (x, y), target);
        self.plat.set_cursor(cursor::GRABBING);
        if changed {
            self.side_hover = None;
            self.side_dirty = true;
            self.dirty = true;
        }
        true
    }

    /// The button came up. True when the press was on the list: a click that goes to the window or folds the
    /// group, or a drop that moves what was dragged to where its mark was (nowhere, if it was off the list).
    fn dnd_release(&mut self) -> bool {
        let Some(d) = self.dnd.take() else { return false };
        self.side_dirty = true;
        self.dirty = true;
        if !d.live {
            match d.grab {
                dnd::Grab::Win(w) => {
                    // A second click on a row soon after the first renames its window.
                    let id = self.wins[w].id;
                    let double = self.row_click.is_some_and(|(r, t)| r == id && d.time.wrapping_sub(t) < MULTI_CLICK_MS);
                    self.row_click = if double { None } else { Some((id, d.time)) };
                    self.go_to(w);
                    if double {
                        self.start_rename(w);
                    }
                }
                dnd::Grab::Group(g) => {
                    // On a group's header it edits the group.
                    let id = self.groups[g].id;
                    let double = self.head_click.is_some_and(|(h, t)| h == id && d.time.wrapping_sub(t) < MULTI_CLICK_MS);
                    self.head_click = if double { None } else { Some((id, d.time)) };
                    self.fold(g);
                    if double {
                        self.open_group_dialog(Some(id), Vec::new());
                    }
                }
            }
            return true;
        }
        self.plat.set_cursor(cursor::POINTER);
        if let Some(to) = d.target {
            self.drop_on(d.grab, to);
        }
        true
    }

    /// Drops what is being dragged (Esc, an overlay opening, windows or groups coming or going): nothing moves.
    fn dnd_cancel(&mut self) {
        if let Some(d) = self.dnd.take()
            && d.live
        {
            self.plat.set_cursor(cursor::DEFAULT);
            self.side_dirty = true;
            self.dirty = true;
        }
    }

    /// The drag as the painter needs it: what is dragged, where a drop would put it, and where the ghost is. The ghost
    /// stays in the panel, or in the strip of tabs at the height of the tab it was grabbed by.
    fn dnd_view(&self) -> Option<side::Dragging> {
        let d = self.dnd.as_ref().filter(|d| d.live)?;
        let (lay, region) = self.list_layout()?;
        let region = if self.tabs_on_top() {
            let r = lay.grab_box(d.grab)?;
            (region.0, r.1, region.2, r.3)
        } else {
            region
        };
        Some(side::Dragging { grab: d.grab, target: d.target, at: side::ghost_at(region, d.size, d.at, d.off) })
    }

    /// Moves what was dragged where it was dropped: the windows and groups are put in their new order (`dnd::arrange`),
    /// a group a window went into is unfolded, and a toast says where a window went when its group changed.
    fn drop_on(&mut self, grab: dnd::Grab, to: dnd::Target) {
        let before: Vec<Option<usize>> = self.wins.iter().map(|w| w.group).collect();
        let Some(a) = dnd::arrange(&before, self.groups.len(), grab, to) else { return };
        let id = self.wins[self.cur].id;
        // What the toast says, from the window as it was: its name, and the group it was in.
        let moved = if let dnd::Grab::Win(w) = grab { Some((win_title(&self.wins[w]).to_string(), before[w].map(|g| self.groups[g].name.clone()))) } else { None };
        let mut wins: Vec<Option<Win>> = self.wins.drain(..).map(Some).collect();
        self.wins = a.order.iter().filter_map(|&i| wins.get_mut(i)?.take()).collect();
        let mut groups: Vec<Option<Group>> = self.groups.drain(..).map(Some).collect();
        self.groups = a.groups.iter().filter_map(|&g| groups.get_mut(g)?.take()).collect();
        for (w, g) in self.wins.iter_mut().zip(&a.group) {
            w.group = *g;
        }
        if let Some(g) = a.unfold {
            self.groups[g].collapsed = false;
        }
        self.cur = self.wins.iter().position(|w| w.id == id).unwrap_or(0);
        self.rebuild_side();
        if let (Some((title, was)), dnd::Grab::Win(w)) = (moved, grab) {
            let now = a.order.iter().position(|&i| i == w).and_then(|at| a.group[at]).map(|g| self.groups[g].name.clone());
            if was != now {
                // The same sentences as the menu's move, so both ways of moving a window say it alike.
                let msg = match now {
                    Some(g) => groups::fill(tr(Str::ToastMoved), &[&title, &g]),
                    None => groups::fill(tr(Str::ToastUngrouped), &[&title]),
                };
                self.overlays.toast(&msg, None, 0, Instant::now());
            }
        }
    }

    /// Whether a spinner is on screen: a command runs in some window and the list that shows it (the panel, or the
    /// tabs) is visible. While it is, the list is painted again every `side::SPIN_EVERY_MS`; otherwise nothing is
    /// scheduled.
    fn spinning(&self) -> bool {
        (self.tabs_on_top() || self.soft.side_rect().is_some()) && self.wins.iter().any(|w| w.panes.iter().any(running))
    }

    /// Time to turn the spinners a little: the rings are painted again, not the list (and the paint schedules the
    /// next turn itself).
    fn tick_spin(&mut self) {
        if self.spin_at.is_some_and(|t| Instant::now() >= t) {
            self.spin_at = None;
            self.spin_due = true;
            self.dirty = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use config::{TabsPos, WindowsCfg};

    #[test]
    fn a_shells_default_title_is_not_a_title_of_its_own() {
        for default in ["", "ana@pc: ~/code", "ana@pc:~", "C:\\Program Files\\PowerShell\\7\\pwsh.exe", "Administrator: C:\\Windows\\System32\\cmd.exe", "Windows PowerShell"] {
            assert!(!own_title(default), "{default:?}");
        }
        for own in ["vim main.rs", "htop", "Deploy: step 2", "build.exe is running"] {
            assert!(own_title(own), "{own:?}");
        }
    }

    #[test]
    fn a_directorys_name_is_its_last_part_on_either_system() {
        for (dir, name) in [("~/lab/axon", "axon"), ("…/a/b/c", "c"), ("/", "/"), ("", "/"), ("~", "~"), ("~\\code\\axon", "axon"), ("C:\\Users\\Ana", "Ana"), ("C:\\", "C:\\")] {
            assert_eq!(dir_name(dir), name, "{dir:?}");
        }
    }

    #[test]
    fn the_settings_are_at_the_edge_opposite_the_windows_panel() {
        let cfg = |tabs, side| WindowsCfg { tabs, side, side_w: 240, ..WindowsCfg::default() };
        // Windows at the left: settings at the right.
        assert_eq!(docks(&cfg(TabsPos::Left, true), 372, 1000), (Dock { w: 240, left: true }, Dock { w: 372, left: false }));
        // Windows at the right: settings at the left, as the reference orders them (`.prefs` is `order: -1`).
        assert_eq!(docks(&cfg(TabsPos::Right, true), 372, 1000), (Dock { w: 240, left: false }, Dock { w: 372, left: true }));
        // The panel hidden, or the windows as tabs at the top: only the settings take room, at the right.
        assert_eq!(docks(&cfg(TabsPos::Left, false), 372, 1000), (Dock { w: 0, left: true }, Dock { w: 372, left: false }));
        assert_eq!(docks(&cfg(TabsPos::Top, true), 372, 1000), (Dock { w: 0, left: true }, Dock { w: 372, left: false }));
        // Closed: no room; they never share an edge, so the grid is the window minus both.
        let (a, b) = docks(&cfg(TabsPos::Right, true), 0, 1000);
        assert_eq!((a.w, b.w), (240, 0));
        for tabs in TabsPos::ALL {
            let (a, b) = docks(&cfg(tabs, true), 372, 1000);
            assert!(a.w == 0 || a.left != b.left, "{tabs:?}");
        }
    }

    #[test]
    fn a_narrow_window_keeps_room_for_the_grid_by_hiding_the_windows_panel() {
        let cfg = WindowsCfg { tabs: TabsPos::Left, side: true, side_w: 220, ..WindowsCfg::default() };
        // Roomy: both panels. At the least size with the settings open (`panel_width(320)` = 198): the grid would be
        // 320 - 220 - 198 < 0, so the windows' panel goes.
        assert_eq!(docks(&cfg, 198, 1000).0.w, 220);
        let (side, prefs) = docks(&cfg, 198, 320);
        assert_eq!((side.w, prefs.w), (0, 198));
        // At every width a window can have, with the settings open or not, the grid keeps its room.
        for ww in 320..1200 {
            for prefs_w in [0, settings::panel_width(ww)] {
                let (a, b) = docks(&cfg, prefs_w, ww);
                assert!(ww >= a.w + b.w + MIN_GRID, "{ww} {prefs_w}");
            }
        }
    }
}
