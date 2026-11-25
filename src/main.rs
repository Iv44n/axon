//! axon — minimalist GPU terminal.
mod bar;
mod cell;
mod chrome;
mod config;
mod context;
mod design;
mod font;
mod gpu;
mod history;
mod i18n;
mod input;
mod mouse;
mod paint;
mod panes;
mod palette;
mod session;
mod settings;
mod side;
mod soft;
mod view;
mod wl;
mod wlapp;
mod xkb;

use std::{
    sync::{
        Arc,
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
use winit::{
    application::ApplicationHandler,
    dpi::{LogicalSize, PhysicalPosition},
    event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy},
    keyboard::{Key, ModifiersState, NamedKey},
    platform::wayland::WindowAttributesExtWayland,
    window::{CursorIcon, ResizeDirection, Window, WindowId},
};

use cell::pack;
use config::FontCfg;
use gpu::Gpu;
use view::View;

// -------------------------------------------------------------- events ----

enum Ev {
    Wake,
    Term(TermEvent),
}

#[derive(Clone)]
struct Proxy {
    p: EventLoopProxy<Ev>,
    pending: Arc<AtomicBool>,
}

impl EventListener for Proxy {
    fn send_event(&self, e: TermEvent) {
        match e {
            // Coalesce: at most one Wake in flight, even if the PTY spews megabytes.
            TermEvent::Wakeup => {
                if !self.pending.swap(true, Ordering::AcqRel) {
                    let _ = self.p.send_event(Ev::Wake);
                }
            }
            e => {
                let _ = self.p.send_event(Ev::Term(e));
            }
        }
    }
}

// ----------------------------------------------------------------- app ----

struct State {
    window: Arc<Window>,
    gpu: Gpu,
    term: Arc<FairMutex<Term<Proxy>>>,
    pty: Notifier,
    cols: usize,
    rows: usize,
    base_px: f32,
    mods: ModifiersState,
    mouse: PhysicalPosition<f64>,
    selecting: bool,
    focused: bool,
    scroll_acc: f32,
    last_click: (Instant, u8),
    clipboard: Option<arboard::Clipboard>,
    pending: Arc<AtomicBool>,
    view: View,
}

struct App {
    proxy: EventLoopProxy<Ev>,
    st: Option<State>,
}

const EDGE: f64 = 6.0;

impl State {
    fn new(el: &ActiveEventLoop, proxy: EventLoopProxy<Ev>) -> State {
        let decorations = std::env::var("AXON_DECORATIONS").is_ok_and(|v| v != "0");
        let attrs = Window::default_attributes()
            .with_title("axon")
            .with_name("axon", "axon")
            .with_inner_size(LogicalSize::new(1000.0, 640.0))
            .with_transparent(true)
            .with_decorations(decorations);
        let window = Arc::new(el.create_window(attrs).expect("window"));

        let cfg = config::load();
        let base_px = cfg.font.size;
        let opacity = cfg.opacity.unwrap_or(0.93);
        let scale = window.scale_factor() as f32;
        let font = FontCfg { size: base_px * scale, letter_spacing: cfg.font.letter_spacing * scale, ..cfg.font };
        let mut gpu = Gpu::new(window.clone(), el.owned_display_handle(), &font, opacity);
        gpu.cursor_color = pack(palette::CURSOR.r, palette::CURSOR.g, palette::CURSOR.b, 255);

        let pending = Arc::new(AtomicBool::new(false));
        let listener = Proxy { p: proxy, pending: pending.clone() };

        let (w, h) = gpu.size();
        let (cw, ch) = gpu.cell();
        let pad = gpu.pad as u32;
        let (cols, rows) = session::grid_size(w, h, cw, ch, pad, pad, pad);
        let (term, notifier, _) = session::spawn(listener, cols, rows, cw, ch, u64::from(window.id()), Default::default(), Default::default(), None);

        window.request_redraw();
        State {
            window,
            gpu,
            term,
            pty: notifier,
            cols,
            rows,
            base_px,
            mods: ModifiersState::empty(),
            mouse: PhysicalPosition::new(0.0, 0.0),
            selecting: false,
            focused: true,
            scroll_acc: 0.0,
            last_click: (Instant::now() - Duration::from_secs(10), 0),
            clipboard: arboard::Clipboard::new().ok(),
            pending,
            view: View::new(),
        }
    }

    fn relayout(&mut self) {
        let (w, h) = self.gpu.size();
        let (cw, ch) = self.gpu.cell();
        let pad = self.gpu.pad as u32;
        let (cols, rows) = session::grid_size(w, h, cw, ch, pad, pad, pad);
        if cols != self.cols || rows != self.rows {
            self.cols = cols;
            self.rows = rows;
            session::resize(&self.term, &self.pty, cols, rows, cw, ch);
        }
        self.window.request_redraw();
    }

    fn write(&self, bytes: impl Into<std::borrow::Cow<'static, [u8]>>) {
        self.pty.notify(bytes);
    }

    fn paste(&mut self) {
        let Some(text) = self.clipboard.as_mut().and_then(|c| c.get_text().ok()) else { return };
        let text = text.replace('\x1b', "");
        let bracketed = self.term.lock().mode().contains(TermMode::BRACKETED_PASTE);
        if bracketed {
            self.write(format!("\x1b[200~{text}\x1b[201~").into_bytes());
        } else {
            self.write(text.replace("\r\n", "\r").replace('\n', "\r").into_bytes());
        }
    }

    fn copy(&mut self) {
        let text = self.term.lock().selection_to_string();
        if let (Some(t), Some(c)) = (text, self.clipboard.as_mut()) {
            let _ = c.set_text(t);
        }
    }

    fn cell_at(&self) -> (Point, Side) {
        let (cw, ch) = self.gpu.cell();
        let (cw, ch) = (cw as f64, ch as f64);
        let pad = self.gpu.pad as f64;
        let fx = ((self.mouse.x - pad) / cw).max(0.0);
        let fy = ((self.mouse.y - pad) / ch).max(0.0);
        let col = (fx as usize).min(self.cols - 1);
        let row = (fy as usize).min(self.rows - 1);
        let side = if fx.fract() < 0.5 { Side::Left } else { Side::Right };
        let off = self.term.lock().grid().display_offset();
        (viewport_to_point(off, Point::new(row, Column(col))), side)
    }

    fn edge_dir(&self) -> Option<ResizeDirection> {
        let s = self.window.inner_size();
        let (x, y, w, h) = (self.mouse.x, self.mouse.y, s.width as f64, s.height as f64);
        let (l, r, t, b) = (x < EDGE, x > w - EDGE, y < EDGE, y > h - EDGE);
        Some(match (l, r, t, b) {
            (true, _, true, _) => ResizeDirection::NorthWest,
            (_, true, true, _) => ResizeDirection::NorthEast,
            (true, _, _, true) => ResizeDirection::SouthWest,
            (_, true, _, true) => ResizeDirection::SouthEast,
            (true, ..) => ResizeDirection::West,
            (_, true, ..) => ResizeDirection::East,
            (_, _, true, _) => ResizeDirection::North,
            (_, _, _, true) => ResizeDirection::South,
            _ => return None,
        })
    }

    /// Brings the renderer up to date with the terminal.
    fn build(&mut self) {
        let mut term = self.term.lock();
        self.view.build(&mut term, self.focused, &mut self.gpu);
    }

    fn redraw(&mut self) {
        self.pending.store(false, Ordering::Release);
        self.build();
        self.window.pre_present_notify();
        self.gpu.render(false);
    }
}

impl ApplicationHandler<Ev> for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.st.is_none() {
            el.set_control_flow(ControlFlow::Wait);
            self.st = Some(State::new(el, self.proxy.clone()));
        }
    }

    fn user_event(&mut self, el: &ActiveEventLoop, ev: Ev) {
        let Some(s) = self.st.as_mut() else { return };
        match ev {
            Ev::Wake => s.window.request_redraw(),
            Ev::Term(e) => match e {
                // The prompt chips' hook (`history.rs`) talks through titles; this backend has no chips.
                TermEvent::Title(t) if history::parse(&t).is_some() => {}
                TermEvent::Title(t) => s.window.set_title(&t),
                TermEvent::ResetTitle => s.window.set_title("axon"),
                TermEvent::PtyWrite(t) => s.write(t.into_bytes()),
                TermEvent::ClipboardStore(_, t) => {
                    if let Some(c) = s.clipboard.as_mut() {
                        let _ = c.set_text(t);
                    }
                }
                // OSC 52 reads are refused (`session::term_config`): output must not read the clipboard.
                TermEvent::ClipboardLoad(..) => {}
                TermEvent::ColorRequest(i, f) => {
                    let rgb = s.term.lock().colors()[i].unwrap_or_else(|| palette::by_index(i));
                    s.write(f(rgb).into_bytes());
                }
                TermEvent::TextAreaSizeRequest(f) => {
                    let (cw, ch) = s.gpu.cell();
                    let ws = WindowSize { num_lines: s.rows as u16, num_cols: s.cols as u16, cell_width: cw as u16, cell_height: ch as u16 };
                    s.write(f(ws).into_bytes());
                }
                TermEvent::Exit | TermEvent::ChildExit(_) => el.exit(),
                _ => {}
            },
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, ev: WindowEvent) {
        let Some(s) = self.st.as_mut() else { return };
        match ev {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::RedrawRequested => s.redraw(),
            WindowEvent::Resized(size) => {
                s.gpu.resize(size.width, size.height);
                s.relayout();
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                s.gpu.set_font_px(s.base_px * scale_factor as f32);
                s.relayout();
            }
            WindowEvent::Focused(f) => {
                s.focused = f;
                if s.term.lock().mode().contains(TermMode::FOCUS_IN_OUT) {
                    s.write(if f { &b"\x1b[I"[..] } else { &b"\x1b[O"[..] });
                }
                s.window.request_redraw();
            }
            WindowEvent::ModifiersChanged(m) => s.mods = m.state(),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                let m = s.mods;
                let ctrl_shift = m.control_key() && m.shift_key();
                if let Key::Character(c) = &event.logical_key {
                    if ctrl_shift && c.eq_ignore_ascii_case("c") { return s.copy(); }
                    if ctrl_shift && c.eq_ignore_ascii_case("v") { return s.paste(); }
                    if ctrl_shift && c.eq_ignore_ascii_case("q") { return el.exit(); }
                    if m.control_key() && (c == "=" || c == "+" || c == "-" || c == "0") {
                        s.base_px = match c.as_str() {
                            "-" => (s.base_px - 1.0).max(8.0),
                            "0" => 15.0,
                            _ => (s.base_px + 1.0).min(48.0),
                        };
                        s.gpu.set_font_px(s.base_px * s.window.scale_factor() as f32);
                        return s.relayout();
                    }
                }
                if m.shift_key() {
                    let sc = match event.logical_key {
                        Key::Named(NamedKey::PageUp) => Some(Scroll::PageUp),
                        Key::Named(NamedKey::PageDown) => Some(Scroll::PageDown),
                        Key::Named(NamedKey::Home) => Some(Scroll::Top),
                        Key::Named(NamedKey::End) => Some(Scroll::Bottom),
                        _ => None,
                    };
                    if let Some(sc) = sc {
                        s.term.lock().scroll_display(sc);
                        return s.window.request_redraw();
                    }
                }
                let mode = *s.term.lock().mode();
                if let Some(bytes) = input::encode(&event, m, mode) {
                    let mut t = s.term.lock();
                    if t.grid().display_offset() > 0 {
                        t.scroll_display(Scroll::Bottom);
                    }
                    t.selection = None;
                    drop(t);
                    s.write(bytes);
                    s.window.request_redraw();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                s.mouse = position;
                if s.selecting {
                    let (p, side) = s.cell_at();
                    if let Some(sel) = s.term.lock().selection.as_mut() {
                        sel.update(p, side);
                    }
                    s.window.request_redraw();
                } else {
                    let icon = match s.edge_dir() {
                        Some(ResizeDirection::North | ResizeDirection::South) => CursorIcon::NsResize,
                        Some(ResizeDirection::East | ResizeDirection::West) => CursorIcon::EwResize,
                        Some(ResizeDirection::NorthWest | ResizeDirection::SouthEast) => CursorIcon::NwseResize,
                        Some(_) => CursorIcon::NeswResize,
                        None => CursorIcon::Text,
                    };
                    s.window.set_cursor(icon);
                }
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => match state {
                ElementState::Pressed => {
                    if s.mods.alt_key() {
                        let _ = s.window.drag_window();
                    } else if let Some(d) = s.edge_dir() {
                        let _ = s.window.drag_resize_window(d);
                    } else {
                        let now = Instant::now();
                        let n = if now.duration_since(s.last_click.0) < Duration::from_millis(350) {
                            s.last_click.1 % 3 + 1
                        } else {
                            0
                        };
                        s.last_click = (now, n);
                        let ty = match n {
                            1 => SelectionType::Semantic,
                            2 => SelectionType::Lines,
                            _ => SelectionType::Simple,
                        };
                        let (p, side) = s.cell_at();
                        s.term.lock().selection = Some(Selection::new(ty, p, side));
                        s.selecting = true;
                        s.window.request_redraw();
                    }
                }
                ElementState::Released => s.selecting = false,
            },
            WindowEvent::MouseWheel { delta, .. } => {
                let (_, ch) = s.gpu.cell();
                s.scroll_acc += match delta {
                    MouseScrollDelta::LineDelta(_, y) => y * 3.0,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / ch as f32,
                };
                let lines = s.scroll_acc.trunc() as i32;
                if lines == 0 {
                    return;
                }
                s.scroll_acc -= lines as f32;
                let mode = *s.term.lock().mode();
                if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL)
                    || (mode.contains(TermMode::ALT_SCREEN) && mode.contains(TermMode::ALTERNATE_SCROLL))
                {
                    let k = match (lines > 0, mode.contains(TermMode::APP_CURSOR)) {
                        (true, true) => &b"\x1bOA"[..],
                        (true, false) => &b"\x1b[A"[..],
                        (false, true) => &b"\x1bOB"[..],
                        (false, false) => &b"\x1b[B"[..],
                    };
                    for _ in 0..lines.unsigned_abs() {
                        s.write(k);
                    }
                } else {
                    s.term.lock().scroll_display(Scroll::Delta(lines));
                    s.window.request_redraw();
                }
            }
            _ => {}
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = config::command(&args) {
        std::process::exit(code);
    }
    // The own Wayland client by default; `winit` serves X11 until that has an own client, and
    // `AXON_BACKEND=winit` forces it.
    let backend = std::env::var("AXON_BACKEND")
        .unwrap_or_else(|_| if std::env::var_os("WAYLAND_DISPLAY").is_some() { "wl".into() } else { "winit".into() });
    if backend == "wl" && wlapp::run() {
        return;
    }
    let el = EventLoop::<Ev>::with_user_event().build().expect("event loop");
    let mut app = App { proxy: el.create_proxy(), st: None };
    el.run_app(&mut app).expect("run");
}
