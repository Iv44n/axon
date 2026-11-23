//! The application on the own Wayland client: the terminal session, the CPU renderer and `wl.rs`,
//! in one blocking `poll` loop with no windowing or GPU library. It handles the keyboard (layouts
//! by `xkb.rs`, client-side repeat), the pointer (window chrome, selection, wheel) and the clipboard.
use std::{
    collections::VecDeque,
    os::fd::{AsFd, OwnedFd},
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
    cell::pack,
    chrome::{self, EDGE_BOTTOM, EDGE_LEFT, EDGE_RIGHT, EDGE_TOP, Hit, Ui},
    input, palette, session,
    soft::{Soft, Target},
    view::View,
    wl::{Event, Wl, cursor},
    xkb::{Key, Xkb},
};

/// `BTN_LEFT` from `linux/input-event-codes.h`.
const BTN_LEFT: u32 = 0x110;
/// Two presses on the title bar closer than this many milliseconds maximize or restore.
const DOUBLE_CLICK_MS: u32 = 400;
/// Clicks on the grid closer than this select a word, then a line.
const MULTI_CLICK_MS: u32 = 350;
/// Lines per wheel notch.
const WHEEL_LINES: f32 = 3.0;
const DEFAULT_PX: f32 = 15.0;

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
        Hit::Close | Hit::Minimize | Hit::Maximize => cursor::POINTER,
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
        Hit::Title => cursor::DEFAULT,
        Hit::Client => cursor::TEXT,
    }
}

/// How the PTY thread reaches the loop: events go in a queue and an eventfd wakes `poll`.
#[derive(Clone)]
struct Proxy {
    wake: Arc<OwnedFd>,
    pending: Arc<AtomicBool>,
    queue: Arc<Mutex<VecDeque<TermEvent>>>,
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
            e => {
                self.queue.lock().unwrap().push_back(e);
                self.wake();
            }
        }
    }
}

/// A key being held: sent again at `at`, then every `1 / rate` seconds.
struct Repeat {
    code: u32,
    key: Key,
    at: Instant,
}

struct App {
    wl: Wl,
    soft: Soft,
    view: View,
    term: Arc<FairMutex<Term<Proxy>>>,
    pty: Notifier,
    proxy: Proxy,
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
    base_px: f32,
    dirty: bool,
    quit: bool,
    mouse: (f32, f32),
    selecting: bool,
    scroll_acc: f32,
    /// Time of the last press on the title bar, and of the last press on the grid with its click count.
    title_click: u32,
    grid_click: (u32, u8),
    repeat: Option<Repeat>,
    /// Key repeat: keys per second (0 = off) and initial delay, as the compositor says.
    rate: i32,
    delay: i32,
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

impl App {
    fn new() -> std::io::Result<App> {
        let base_px: f32 = std::env::var("AXON_FONT_SIZE").ok().and_then(|v| v.parse().ok()).unwrap_or(DEFAULT_PX);
        let opacity: f32 = std::env::var("AXON_OPACITY").ok().and_then(|v| v.parse().ok()).unwrap_or(chrome::OPACITY);
        let mut soft = Soft::new(base_px, opacity);
        soft.cursor_color = pack(palette::CURSOR.r, palette::CURSOR.g, palette::CURSOR.b, 255);
        soft.set_title("axon");
        let (cw, ch) = soft.cell();

        let mut wl = Wl::connect("axon", "axon")?;
        let mut xkb = Some(Xkb::load().ok_or_else(|| std::io::Error::other("libxkbcommon.so.0 not found, the keyboard would not work"))?);
        // The first configure decides the size; 0 x 0 means the compositor has no opinion.
        let (mut w, mut h) = (1000, 640);
        let mut ui = Ui::IDLE;
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
        let proxy = Proxy { wake, pending: Arc::new(AtomicBool::new(false)), queue: Arc::default() };
        let (cols, rows) = session::grid_size(w, h, cw, ch, soft.pad, soft.top);
        let (term, pty) = session::spawn(proxy.clone(), cols, rows, cw, ch, 0);
        let mut app = App {
            wl,
            soft,
            view: View::new(),
            term,
            pty,
            proxy,
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
            dirty: true,
            quit: false,
            mouse: (0.0, 0.0),
            selecting: false,
            scroll_acc: 0.0,
            title_click: 0,
            grid_click: (0, 0),
            repeat: None,
            rate,
            delay,
        };
        app.set_geometry();
        Ok(app)
    }

    fn run(mut self) -> std::io::Result<()> {
        while !self.quit {
            self.draw();
            self.wl.flush()?;

            // Block until the compositor, the PTY thread or a held key needs us.
            let timeout = self.repeat.as_ref().map(|r| {
                let left = r.at.saturating_duration_since(Instant::now());
                Timespec { tv_sec: left.as_secs() as i64, tv_nsec: left.subsec_nanos() as i64 }
            });
            let wake = self.proxy.wake.clone();
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
            }
            if wl_ready {
                self.wl.read_events()?;
            }
            self.term_events();
            while let Some(ev) = self.wl.next_event() {
                self.wl_event(ev);
            }
            self.tick_repeat();
        }
        Ok(())
    }

    /// Draws when something changed and the compositor has shown the previous frame.
    fn draw(&mut self) {
        if !self.dirty || self.wl.frame_pending() {
            return;
        }
        // The buffer is the window plus the margin its shadow is drawn in.
        let (ml, mt, mb) = chrome::margins(self.ui.maximized);
        let (w, h) = (self.w as usize + 2 * ml, self.h as usize + mt + mb);
        let Some(fb) = self.wl.framebuffer(w, h) else { return };
        self.proxy.pending.store(false, Ordering::Release);
        self.soft.set_ui(self.ui);
        let mut t = self.term.lock();
        self.view.build(&mut t, self.ui.focused, &mut Target { soft: &mut self.soft, fb, w, h });
        drop(t);
        self.wl.present();
        self.dirty = false;
    }

    /// Events from the terminal thread. The queue lock is not held while one is handled: the PTY
    /// thread sends events while holding the terminal lock, and handling may take it.
    fn term_events(&mut self) {
        loop {
            let Some(e) = self.proxy.queue.lock().unwrap().pop_front() else { break };
            match e {
                TermEvent::Title(t) => self.set_title(&t),
                TermEvent::ResetTitle => self.set_title("axon"),
                TermEvent::PtyWrite(t) => self.pty.notify(t.into_bytes()),
                TermEvent::ClipboardStore(_, t) => self.wl.set_clipboard(&t),
                // OSC 52 reads are refused (`session::term_config`): output must not read the clipboard.
                TermEvent::ClipboardLoad(..) => {}
                TermEvent::ColorRequest(i, f) => {
                    let rgb = self.term.lock().colors()[i].unwrap_or_else(|| palette::by_index(i));
                    self.pty.notify(f(rgb).into_bytes());
                }
                TermEvent::TextAreaSizeRequest(f) => {
                    let ws = WindowSize {
                        num_lines: self.rows as u16,
                        num_cols: self.cols as u16,
                        cell_width: self.cw as u16,
                        cell_height: self.ch as u16,
                    };
                    self.pty.notify(f(ws).into_bytes());
                }
                TermEvent::Exit | TermEvent::ChildExit(_) => self.quit = true,
                _ => {}
            }
        }
    }

    fn set_title(&mut self, title: &str) {
        self.wl.set_title(title);
        self.soft.set_title(title);
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
                if self.term.lock().mode().contains(TermMode::FOCUS_IN_OUT) {
                    self.pty.notify(if f { &b"\x1b[I"[..] } else { &b"\x1b[O"[..] });
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
                if self.ui.hover != 0 || self.ui.down {
                    (self.ui.hover, self.ui.down) = (0, false);
                    self.dirty = true;
                }
            }
            Event::PointerButton { button: BTN_LEFT, pressed, time } => self.on_button(pressed, time),
            Event::PointerButton { .. } => {}
        }
    }

    // ------------------------------------------------------------- layout --

    /// Tells the compositor where the window is inside the buffer: the shadow around it is not
    /// part of it (placement, tiling, maximizing) and only a thin band of it takes the pointer.
    fn set_geometry(&mut self) {
        let (ml, mt, _) = chrome::margins(self.ui.maximized);
        let grab = if self.ui.maximized { None } else { Some(chrome::EDGE_OUT as i32) };
        self.wl.set_geometry(ml as i32, mt as i32, self.w as i32, self.h as i32, grab);
    }

    /// Grid size for the window size, after a resize or a new font size.
    fn relayout(&mut self) {
        let (c, r) = session::grid_size(self.w, self.h, self.cw, self.ch, self.soft.pad, self.soft.top);
        if (c, r) != (self.cols, self.rows) {
            (self.cols, self.rows) = (c, r);
            session::resize(&self.term, &self.pty, c, r, self.cw, self.ch);
        }
        self.dirty = true;
    }

    fn zoom(&mut self, px: f32) -> bool {
        self.base_px = px.clamp(8.0, 48.0);
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
        let letter = char::from_u32(k.sym).map(|c| c.to_ascii_lowercase());
        if m.ctrl && m.shift {
            match letter {
                Some('c') => return self.copy(),
                Some('v') => return self.paste(),
                Some('q') => {
                    self.quit = true;
                    return true;
                }
                _ => {}
            }
        }
        if m.ctrl && !m.alt {
            match k.sym {
                0x3d | 0x2b | KS_KP_ADD => return self.zoom(self.base_px + 1.0),
                0x2d | KS_KP_SUBTRACT => return self.zoom(self.base_px - 1.0),
                0x30 | KS_KP_0 => return self.zoom(DEFAULT_PX),
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
                self.term.lock().scroll_display(s);
                self.dirty = true;
                return true;
            }
        }
        let mode = *self.term.lock().mode();
        let Some(bytes) = input::encode_key(k, mode) else { return false };
        let mut t = self.term.lock();
        if t.grid().display_offset() > 0 {
            t.scroll_display(Scroll::Bottom);
        }
        t.selection = None;
        drop(t);
        self.pty.notify(bytes);
        self.dirty = true;
        true
    }

    // ---------------------------------------------------------- clipboard --

    fn paste(&mut self) -> bool {
        let Some(text) = self.wl.clipboard_text() else { return true };
        let text = text.replace('\x1b', "");
        let bracketed = self.term.lock().mode().contains(TermMode::BRACKETED_PASTE);
        if bracketed {
            self.pty.notify(format!("\x1b[200~{text}\x1b[201~").into_bytes());
        } else {
            self.pty.notify(text.replace("\r\n", "\r").replace('\n', "\r").into_bytes());
        }
        true
    }

    fn copy(&mut self) -> bool {
        let text = self.term.lock().selection_to_string();
        if let Some(t) = text {
            self.wl.set_clipboard(&t);
        }
        true
    }

    // ------------------------------------------------------------ pointer --

    /// The grid cell under the pointer, and which half of it.
    fn cell_at(&self) -> (Point, Side) {
        let (cw, ch) = (self.cw as f32, self.ch as f32);
        let fx = ((self.mouse.0 - self.soft.pad as f32) / cw).max(0.0);
        let fy = ((self.mouse.1 - self.soft.top as f32) / ch).max(0.0);
        let col = (fx as usize).min(self.cols - 1);
        let row = (fy as usize).min(self.rows - 1);
        let side = if fx.fract() < 0.5 { Side::Left } else { Side::Right };
        let off = self.term.lock().grid().display_offset();
        (viewport_to_point(off, Point::new(row, Column(col))), side)
    }

    /// `(x, y)` is in surface pixels; everything else here is in window pixels.
    fn on_pointer_move(&mut self, x: f32, y: f32) {
        let (ml, mt, _) = chrome::margins(self.ui.maximized);
        let (x, y) = (x - ml as f32, y - mt as f32);
        self.mouse = (x, y);
        if self.selecting {
            let (p, side) = self.cell_at();
            if let Some(sel) = self.term.lock().selection.as_mut() {
                sel.update(p, side);
            }
            self.dirty = true;
            return;
        }
        let now = chrome::hit(x, y, self.w, self.h, self.ui.maximized);
        if now != self.hover {
            self.hover = now;
            self.wl.set_cursor(cursor_for(now));
            if now.button() != self.ui.hover {
                self.ui.hover = now.button();
                self.dirty = true;
            }
        }
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
                    _ => {}
                }
            }
            return;
        }
        match self.hover {
            Hit::Close | Hit::Minimize | Hit::Maximize => {
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
                self.term.lock().selection = Some(Selection::new(ty, p, side));
                self.selecting = true;
                self.dirty = true;
            }
        }
    }

    fn on_scroll(&mut self, pixels: f32, clicks: i32) {
        // Wayland counts down as positive; the terminal's `Scroll::Delta` counts up into history.
        self.scroll_acc -= if clicks != 0 { clicks as f32 * WHEEL_LINES } else { pixels / self.ch as f32 };
        let lines = self.scroll_acc.trunc() as i32;
        if lines == 0 {
            return;
        }
        self.scroll_acc -= lines as f32;
        let mode = *self.term.lock().mode();
        if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) {
            let k = match (lines > 0, mode.contains(TermMode::APP_CURSOR)) {
                (true, true) => &b"\x1bOA"[..],
                (true, false) => &b"\x1b[A"[..],
                (false, true) => &b"\x1bOB"[..],
                (false, false) => &b"\x1b[B"[..],
            };
            for _ in 0..lines.unsigned_abs() {
                self.pty.notify(k);
            }
        } else {
            self.term.lock().scroll_display(Scroll::Delta(lines));
            self.dirty = true;
        }
    }
}
