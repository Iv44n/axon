//! The application on the own Wayland client (`AXON_BACKEND=wl`): the terminal session, the CPU
//! renderer and `wl.rs`, in one blocking `poll` loop with no windowing or GPU library.
//! Pointer input drives the window chrome (buttons, move, resize); keyboard and clipboard arrive in
//! stage C of `knowledge/plan/software-renderer.md`, so today the window shows the shell's output.
use std::{
    collections::VecDeque,
    os::fd::{AsFd, OwnedFd},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use alacritty_terminal::event::{Event as TermEvent, EventListener, Notify, WindowSize};
use rustix::{
    event::{EventfdFlags, PollFd, PollFlags, eventfd, poll},
    io::{read, write},
};

use crate::{
    cell::pack,
    chrome::{self, EDGE_BOTTOM, EDGE_LEFT, EDGE_RIGHT, EDGE_TOP, Hit, Ui},
    palette, session,
    soft::{Soft, Target},
    view::View,
    wl::{Event, Wl, cursor},
};

/// `BTN_LEFT` from `linux/input-event-codes.h`.
const BTN_LEFT: u32 = 0x110;
/// Two presses on the title bar closer than this many milliseconds maximize or restore.
const DOUBLE_CLICK_MS: u32 = 400;

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

pub fn run() {
    if let Err(e) = run_inner() {
        eprintln!("axon: {e}");
        std::process::exit(1);
    }
}

fn run_inner() -> std::io::Result<()> {
    let base_px: f32 = std::env::var("AXON_FONT_SIZE").ok().and_then(|v| v.parse().ok()).unwrap_or(15.0);
    let opacity: f32 = std::env::var("AXON_OPACITY").ok().and_then(|v| v.parse().ok()).unwrap_or(0.80);
    let mut soft = Soft::new(base_px, opacity);
    soft.cursor_color = pack(palette::CURSOR.r, palette::CURSOR.g, palette::CURSOR.b, 255);
    soft.set_title("axon");
    let (cw, ch) = soft.cell();

    let mut wl = Wl::connect("axon", "axon")?;
    // The first configure decides the size; 0 x 0 means the compositor has no opinion.
    let (mut w, mut h) = (1000, 640);
    let mut ui = Ui::IDLE;
    let mut hover = Hit::Client;
    let mut last_click = 0u32;
    'first: loop {
        wl.wait(None)?;
        wl.read_events()?;
        while let Some(ev) = wl.next_event() {
            match ev {
                Event::Configure { w: nw, h: nh, maximized } => {
                    if nw > 0 && nh > 0 {
                        (w, h) = (nw as u32, nh as u32);
                    }
                    ui.maximized = maximized;
                    break 'first;
                }
                Event::Focus(f) => ui.focused = f,
                Event::Close => return Ok(()),
                _ => {}
            }
        }
    }

    let wake = Arc::new(eventfd(0, EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK)?);
    let proxy = Proxy { wake: wake.clone(), pending: Arc::new(AtomicBool::new(false)), queue: Arc::default() };
    let (mut cols, mut rows) = session::grid_size(w, h, cw, ch, soft.pad, soft.top);
    let (term, pty) = session::spawn(proxy.clone(), cols, rows, cw, ch, 0);
    let mut view = View::new();
    let mut dirty = true;

    loop {
        // Draw when something changed and the compositor has shown the previous frame.
        if dirty && !wl.frame_pending() {
            if let Some(fb) = wl.framebuffer(w as usize, h as usize) {
                proxy.pending.store(false, Ordering::Release);
                soft.set_ui(ui);
                let mut t = term.lock();
                view.build(&mut t, ui.focused, &mut Target { soft: &mut soft, fb, w: w as usize, h: h as usize });
                drop(t);
                wl.present();
                dirty = false;
            }
        }
        wl.flush()?;

        let wl_fd = wl.fd();
        let mut fds = [PollFd::new(&wl_fd, PollFlags::IN), PollFd::new(wake.as_ref(), PollFlags::IN)];
        match poll(&mut fds, None) {
            Ok(_) | Err(rustix::io::Errno::INTR) => {}
            Err(e) => return Err(e.into()),
        }
        let (wl_ready, wake_ready) = (fds[0].revents().contains(PollFlags::IN), fds[1].revents().contains(PollFlags::IN));
        if wake_ready {
            let _ = read(wake.as_fd(), &mut [0u8; 8]);
            dirty = true;
        }
        if wl_ready {
            wl.read_events()?;
        }

        // Pop in its own statement: the queue lock must not be held while an event is handled
        // (the PTY thread sends events while holding the terminal lock, and handling may lock it).
        loop {
            let Some(e) = proxy.queue.lock().unwrap().pop_front() else { break };
            match e {
                TermEvent::Title(t) => {
                    wl.set_title(&t);
                    soft.set_title(&t);
                    dirty = true;
                }
                TermEvent::ResetTitle => {
                    wl.set_title("axon");
                    soft.set_title("axon");
                    dirty = true;
                }
                TermEvent::PtyWrite(t) => pty.notify(t.into_bytes()),
                TermEvent::ColorRequest(i, f) => {
                    let rgb = term.lock().colors()[i].unwrap_or_else(|| palette::by_index(i));
                    pty.notify(f(rgb).into_bytes());
                }
                TermEvent::TextAreaSizeRequest(f) => {
                    let ws = WindowSize { num_lines: rows as u16, num_cols: cols as u16, cell_width: cw as u16, cell_height: ch as u16 };
                    pty.notify(f(ws).into_bytes());
                }
                TermEvent::Exit | TermEvent::ChildExit(_) => return Ok(()),
                _ => {}
            }
        }
        while let Some(ev) = wl.next_event() {
            match ev {
                Event::Configure { w: nw, h: nh, maximized } => {
                    ui.maximized = maximized;
                    if nw > 0 && nh > 0 && (nw as u32, nh as u32) != (w, h) {
                        (w, h) = (nw as u32, nh as u32);
                        let (c, r) = session::grid_size(w, h, cw, ch, soft.pad, soft.top);
                        if (c, r) != (cols, rows) {
                            (cols, rows) = (c, r);
                            session::resize(&term, &pty, cols, rows, cw, ch);
                        }
                    }
                    dirty = true;
                }
                Event::Focus(f) => {
                    ui.focused = f;
                    dirty = true;
                }
                Event::PointerMove { x, y } => {
                    let now = chrome::hit(x, y, w, h, ui.maximized);
                    if now != hover {
                        hover = now;
                        wl.set_cursor(cursor_for(now));
                        if now.button() != ui.hover {
                            ui.hover = now.button();
                            dirty = true;
                        }
                    }
                }
                Event::PointerLeave => {
                    hover = Hit::Client;
                    if ui.hover != 0 || ui.down {
                        (ui.hover, ui.down) = (0, false);
                        dirty = true;
                    }
                }
                Event::PointerButton { button: BTN_LEFT, pressed: true, time } => match hover {
                    Hit::Close | Hit::Minimize | Hit::Maximize => {
                        ui.down = true;
                        dirty = true;
                    }
                    Hit::Title if last_click != 0 && time.wrapping_sub(last_click) < DOUBLE_CLICK_MS => {
                        last_click = 0;
                        wl.toggle_maximized();
                    }
                    Hit::Title => {
                        last_click = time;
                        wl.begin_move();
                    }
                    Hit::Edge(edges) => wl.begin_resize(edges),
                    Hit::Client => {}
                },
                Event::PointerButton { button: BTN_LEFT, pressed: false, .. } => {
                    if ui.down {
                        ui.down = false;
                        dirty = true;
                        match hover {
                            Hit::Close => return Ok(()),
                            Hit::Minimize => wl.minimize(),
                            Hit::Maximize => wl.toggle_maximized(),
                            _ => {}
                        }
                    }
                }
                Event::Close => return Ok(()),
                Event::PointerButton { .. } | Event::FrameDone => {}
            }
        }
    }
}
