//! axon — minimalist GPU terminal.
mod cell;
mod font;
mod gpu;
mod input;
mod palette;
mod soft;

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use alacritty_terminal::{
    Term,
    event::{Event as TermEvent, EventListener, Notify, WindowSize},
    event_loop::{EventLoop as PtyLoop, Msg, Notifier},
    grid::{Dimensions, Scroll},
    index::{Column, Line, Point, Side},
    selection::{Selection, SelectionRange, SelectionType},
    sync::FairMutex,
    term::{Config as TermConfig, TermDamage, TermMode, cell::Flags, viewport_to_point},
    tty,
    vte::ansi::{Color, CursorShape, NamedColor},
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

use cell::{Cell, F_BAR, F_BEAM, F_BOX, F_STRIKE, F_UNDER, Glyphs, T_BLANK, pack};
use gpu::Gpu;

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

struct Size {
    cols: usize,
    rows: usize,
}
impl Dimensions for Size {
    fn total_lines(&self) -> usize { self.rows }
    fn screen_lines(&self) -> usize { self.rows }
    fn columns(&self) -> usize { self.cols }
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
    // Frame-to-frame state that `Term`'s damage does not cover.
    dirty: Vec<bool>,
    last_cursor: (Point, CursorShape, bool),
    last_sel: Option<SelectionRange>,
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

        let base_px: f32 = std::env::var("AXON_FONT_SIZE").ok().and_then(|v| v.parse().ok()).unwrap_or(15.0);
        let opacity: f32 = std::env::var("AXON_OPACITY").ok().and_then(|v| v.parse().ok()).unwrap_or(0.93);
        let scale = window.scale_factor() as f32;
        let mut gpu = Gpu::new(window.clone(), el.owned_display_handle(), base_px * scale, opacity);
        gpu.cursor_color = pack(palette::CURSOR.r, palette::CURSOR.g, palette::CURSOR.b, 255);

        let pending = Arc::new(AtomicBool::new(false));
        let listener = Proxy { p: proxy, pending: pending.clone() };

        let (w, h) = gpu.size();
        let (cw, ch) = gpu.cell();
        let pad = gpu.pad as u32;
        let cols = (w.saturating_sub(2 * pad) / cw).max(2) as usize;
        let rows = (h.saturating_sub(2 * pad) / ch).max(2) as usize;

        let term = Arc::new(FairMutex::new(Term::new(
            TermConfig::default(),
            &Size { cols, rows },
            listener.clone(),
        )));

        tty::setup_env();
        let opts = tty::Options { shell: None, working_directory: None, drain_on_exit: false, env: HashMap::new() };
        let ws = WindowSize { num_lines: rows as u16, num_cols: cols as u16, cell_width: cw as u16, cell_height: ch as u16 };
        let pty = tty::new(&opts, ws, u64::from(window.id())).expect("pty");
        let lp = PtyLoop::new(term.clone(), listener, pty, false, false).expect("pty event loop");
        let notifier = Notifier(lp.channel());
        lp.spawn();

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
            dirty: Vec::new(),
            last_cursor: (Point::default(), CursorShape::Block, true),
            last_sel: None,
        }
    }

    fn relayout(&mut self) {
        let (w, h) = self.gpu.size();
        let (cw, ch) = self.gpu.cell();
        let pad = self.gpu.pad as u32;
        let cols = (w.saturating_sub(2 * pad) / cw).max(2) as usize;
        let rows = (h.saturating_sub(2 * pad) / ch).max(2) as usize;
        if cols != self.cols || rows != self.rows {
            self.cols = cols;
            self.rows = rows;
            self.term.lock().resize(Size { cols, rows });
            let ws = WindowSize { num_lines: rows as u16, num_cols: cols as u16, cell_width: cw as u16, cell_height: ch as u16 };
            let _ = self.pty.0.send(Msg::Resize(ws));
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

    /// Brings the instances up to date: only rows that changed since the last frame are rebuilt.
    fn build(&mut self) {
        let (rows, cols) = (self.rows, self.cols);
        let mut term = self.term.lock();
        let mut full = self.gpu.begin_frame(rows, cols);
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

        // The cursor shape and the selection are not part of `Term`'s damage: compare them
        // with the previous frame and rebuild the rows they touch, old and new.
        let content = term.renderable_content();
        let offset = content.display_offset;
        let cur = content.cursor;
        let shown = content.mode.contains(TermMode::SHOW_CURSOR) && offset == 0;
        let shape = if !self.focused { CursorShape::HollowBlock } else { cur.shape };
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
            let focused = self.focused;
            self.gpu.write_rows(first, row - first, |g, cells| build_rows(g, &term, first, cells, focused));
        }
    }

    fn redraw(&mut self) {
        self.pending.store(false, Ordering::Release);
        self.build();
        self.window.pre_present_notify();
        self.gpu.render(false);
    }
}

/// Fills `cells` with the rows starting at `first`: row-major, `term.columns()` cells per row.
/// The slice arrives zeroed (blank), so only non-blank cells are written.
fn build_rows(g: &mut impl Glyphs, term: &Term<Proxy>, first: usize, cells: &mut [Cell], focused: bool) {
    let content = term.renderable_content();
    let colors = content.colors;
    let selection = content.selection;
    let offset = content.display_offset as i32;
    let cur = content.cursor;
    let show_cursor = content.mode.contains(TermMode::SHOW_CURSOR) && offset == 0;
    let shape = if !focused { CursorShape::HollowBlock } else { cur.shape };
    let cursor_color = palette::CURSOR;
    let dark = palette::BG;
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
            let mut bg_default = matches!(cell.bg, Color::Named(NamedColor::Background));
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
                TermEvent::Title(t) => s.window.set_title(&t),
                TermEvent::ResetTitle => s.window.set_title("axon"),
                TermEvent::PtyWrite(t) => s.write(t.into_bytes()),
                TermEvent::ClipboardStore(_, t) => {
                    if let Some(c) = s.clipboard.as_mut() {
                        let _ = c.set_text(t);
                    }
                }
                TermEvent::ClipboardLoad(_, f) => {
                    if let Some(t) = s.clipboard.as_mut().and_then(|c| c.get_text().ok()) {
                        s.write(f(&t).into_bytes());
                    }
                }
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
    let el = EventLoop::<Ev>::with_user_event().build().expect("event loop");
    let mut app = App { proxy: el.create_proxy(), st: None };
    el.run_app(&mut app).expect("run");
}
