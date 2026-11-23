//! Own Wayland client ([0011](../knowledge/decisions/0011-software-render-wl-shm.md)): the wire
//! protocol over the compositor's unix socket, and one window, an `xdg_toplevel` whose pixels are a
//! `wl_shm` buffer.
//!
//! Objects are indices into a flat table (`kinds`), not boxed trait objects or a hash map; an event
//! is a `match` on `(kind, opcode)`. Messages are built straight into one reused byte buffer.
//! Only the interfaces the window needs exist; the opcodes below come from the protocol XML.
//! Input is here too: pointer (buttons, motion, wheel), keyboard (raw keys, the keymap text goes to
//! `xkb.rs`) and the text clipboard through `wl_data_device`.
use std::{
    collections::VecDeque,
    io::{self, IoSlice, IoSliceMut},
    mem::MaybeUninit,
    os::{
        fd::{AsFd, BorrowedFd, OwnedFd},
        unix::net::UnixStream,
    },
    path::PathBuf,
};

use rustix::{
    event::{PollFd, PollFlags, poll},
    fs::{MemfdFlags, ftruncate, memfd_create},
    mm::{MapFlags, ProtFlags, mmap, munmap},
    net::{
        RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, SendAncillaryBuffer, SendAncillaryMessage, SendFlags,
        recvmsg, sendmsg,
    },
};

const DISPLAY: u32 = 1;
/// `wl_shm.format.argb8888`: premultiplied alpha, little-endian `a<<24 | r<<16 | g<<8 | b`.
const ARGB8888: u32 = 0;
/// `xdg_toplevel.state`: maximized, fullscreen, activated, and the tiled edges (`xdg_wm_base` v2).
const STATE_MAXIMIZED: u32 = 1;
const STATE_FULLSCREEN: u32 = 2;
const STATE_ACTIVATED: u32 = 4;
const STATE_TILED: [u32; 4] = [5, 6, 7, 8];
/// Smallest window the title bar and a usable grid fit in.
const MIN_SIZE: (i32, i32) = (320, 200);

/// `wp_cursor_shape_device_v1.shape` values the window uses.
pub mod cursor {
    pub const DEFAULT: u32 = 1;
    pub const POINTER: u32 = 4;
    pub const TEXT: u32 = 9;
    pub const E_RESIZE: u32 = 18;
    pub const N_RESIZE: u32 = 19;
    pub const NE_RESIZE: u32 = 20;
    pub const NW_RESIZE: u32 = 21;
    pub const S_RESIZE: u32 = 22;
    pub const SE_RESIZE: u32 = 23;
    pub const SW_RESIZE: u32 = 24;
    pub const W_RESIZE: u32 = 25;
}

/// What an object id means. Index `id` of `Wl::kinds`; `None` = free or unknown.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    None,
    Display,
    Registry,
    Sync,
    Frame,
    Compositor,
    Shm,
    Pool,
    Region,
    /// A `wl_buffer`; the payload is its slot.
    Buffer(u8),
    Surface,
    WmBase,
    XdgSurface,
    Toplevel,
    Seat,
    Pointer,
    Keyboard,
    CursorShapeManager,
    CursorShapeDevice,
    DataDeviceManager,
    DataDevice,
    DataSource,
}

/// Object ids the server allocates (the offers of `wl_data_device`) start here; they are not
/// indices into `kinds`.
const SERVER_ID_BASE: u32 = 0xff00_0000;

/// A clipboard offer from another client: which text formats it can give.
struct Offer {
    id: u32,
    /// `text/plain;charset=utf-8`, `text/plain` and `UTF8_STRING`.
    mimes: [bool; 3],
}

const TEXT_MIMES: [&str; 3] = ["text/plain;charset=utf-8", "UTF8_STRING", "text/plain"];

/// What the window layer tells the application.
#[derive(Clone, Debug)]
pub enum Event {
    /// The compositor proposes a size; 0 means "your choice". `square`: maximized, fullscreen or
    /// tiled, so no rounded corners, shadow or resize edges.
    Configure { w: i32, h: i32, square: bool },
    /// The window gained or lost the activated state (keyboard focus, to the user).
    Focus(bool),
    Close,
    /// The previous frame was shown: the next one may be drawn.
    FrameDone,
    /// The pointer entered or moved, in surface pixels.
    PointerMove { x: f32, y: f32 },
    PointerLeave,
    /// A button changed state; `time` is the compositor's millisecond clock.
    PointerButton { button: u32, pressed: bool, time: u32 },
    /// A wheel or touchpad scroll: `clicks` wheel notches (down is positive) or, for smooth
    /// scrolling, `pixels`.
    Scroll { pixels: f32, clicks: i32 },
    /// The keymap text of the layout in use (`xkb.rs` compiles it).
    Keymap(String),
    /// A key by its evdev code; layouts and modifiers are applied by `xkb.rs`.
    Key { key: u32, pressed: bool },
    Modifiers { depressed: u32, latched: u32, locked: u32, group: u32 },
    /// Client-side key repeat: keys per second (0 = off) and the delay before it starts, in ms.
    RepeatInfo { rate: i32, delay: i32 },
    /// The keyboard focus left the window: stop repeating.
    KeyboardLeave,
}

/// Interfaces the compositor offers, as `(registry name, version)`; 0 = absent.
#[derive(Default)]
struct Globals {
    compositor: (u32, u32),
    shm: (u32, u32),
    wm_base: (u32, u32),
    seat: (u32, u32),
    cursor_shape: (u32, u32),
    data_device_manager: (u32, u32),
}

/// A request argument. `Fd` travels in ancillary data, not in the byte stream.
enum Arg<'a> {
    U(u32),
    I(i32),
    S(&'a str),
    Fd(OwnedFd),
}

/// One shared-memory buffer and its mapping.
#[derive(Clone, Copy)]
struct Slot {
    id: u32,
    ptr: *mut u32,
    w: usize,
    h: usize,
    /// The compositor may still be reading it (between `attach` and `release`).
    busy: bool,
}

impl Slot {
    const EMPTY: Slot = Slot { id: 0, ptr: std::ptr::null_mut(), w: 0, h: 0, busy: false };

    fn allocated(&self) -> bool {
        self.id != 0
    }

    fn len(&self) -> usize {
        self.w * self.h
    }
}

/// Reader over the arguments of one event.
struct Args<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Args<'a> {
    fn u32(&mut self) -> u32 {
        let v = self.b.get(self.p..self.p + 4).map_or(0, |s| u32::from_ne_bytes(s.try_into().unwrap()));
        self.p += 4;
        v
    }

    fn i32(&mut self) -> i32 {
        self.u32() as i32
    }

    fn array(&mut self) -> &'a [u8] {
        let n = self.u32() as usize;
        let s = self.b.get(self.p..self.p + n).unwrap_or(&[]);
        self.p += n.next_multiple_of(4);
        s
    }

    /// A string argument (length includes the NUL terminator).
    fn str(&mut self) -> &'a str {
        let s = self.array();
        std::str::from_utf8(s.strip_suffix(&[0]).unwrap_or(s)).unwrap_or("")
    }
}

pub struct Wl {
    sock: OwnedFd,
    out: Vec<u8>,
    out_fds: Vec<OwnedFd>,
    inb: Vec<u8>,
    in_fds: Vec<OwnedFd>,
    kinds: Vec<Kind>,
    free: Vec<u32>,
    globals: Globals,
    registry: u32,
    compositor: u32,
    shm: u32,
    wm_base: u32,
    surface: u32,
    xdg_surface: u32,
    toplevel: u32,
    seat: u32,
    pointer: u32,
    keyboard: u32,
    cursor_manager: u32,
    cursor_device: u32,
    data_manager: u32,
    data_device: u32,
    /// Our `wl_data_source` (0 = none), the text it serves and whether we own the selection.
    source: u32,
    clip_out: String,
    /// The selection other clients offer (id, 0 = none) and the offers still alive.
    selection: u32,
    offers: Vec<Offer>,
    /// Scroll gathered until `wl_pointer.frame`: pixels and wheel notches.
    axis: (f32, i32),
    seat_version: u32,
    /// Serial of the last button press or pointer enter: what `move` and `resize` must quote.
    serial: u32,
    /// Serial of the last pointer enter, which `set_shape` must quote, and the shape asked for.
    enter_serial: u32,
    shape: u32,
    events: VecDeque<Event>,
    /// Size and states from the last `xdg_toplevel.configure`, applied on `xdg_surface.configure`:
    /// width, height, activated, maximized, square.
    pending: (i32, i32, bool, bool, bool),
    activated: bool,
    maximized: bool,
    slots: [Slot; 2],
    cur: usize,
    frame_pending: bool,
    sync_done: bool,
    error: Option<String>,
    debug: bool,
}

impl Wl {
    /// Connects, binds the globals the window needs and creates the toplevel (not yet mapped: the
    /// first `Event::Configure` says when to draw).
    pub fn connect(title: &str, app_id: &str) -> io::Result<Wl> {
        let name = std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "wayland-0".into());
        let path = if name.starts_with('/') {
            PathBuf::from(name)
        } else {
            let dir = std::env::var("XDG_RUNTIME_DIR").map_err(|_| io::Error::other("XDG_RUNTIME_DIR is not set"))?;
            PathBuf::from(dir).join(name)
        };
        let mut wl = Wl {
            sock: UnixStream::connect(path)?.into(),
            out: Vec::with_capacity(4096),
            out_fds: Vec::new(),
            inb: Vec::with_capacity(8192),
            in_fds: Vec::new(),
            kinds: vec![Kind::None, Kind::Display],
            free: Vec::new(),
            globals: Globals::default(),
            registry: 0,
            compositor: 0,
            shm: 0,
            wm_base: 0,
            surface: 0,
            xdg_surface: 0,
            toplevel: 0,
            seat: 0,
            pointer: 0,
            keyboard: 0,
            cursor_manager: 0,
            cursor_device: 0,
            data_manager: 0,
            data_device: 0,
            source: 0,
            clip_out: String::new(),
            selection: 0,
            offers: Vec::new(),
            axis: (0.0, 0),
            seat_version: 0,
            serial: 0,
            enter_serial: 0,
            shape: cursor::DEFAULT,
            events: VecDeque::new(),
            pending: (0, 0, false, false, false),
            activated: false,
            maximized: false,
            slots: [Slot::EMPTY; 2],
            cur: 0,
            frame_pending: false,
            sync_done: false,
            error: None,
            debug: std::env::var_os("AXON_WL_DEBUG").is_some(),
        };

        wl.registry = wl.new_obj(Kind::Registry);
        wl.send(DISPLAY, 1, [Arg::U(wl.registry)]); // wl_display.get_registry
        wl.roundtrip()?;
        let g = &wl.globals;
        if g.compositor.0 == 0 || g.shm.0 == 0 || g.wm_base.0 == 0 {
            return Err(io::Error::other("the compositor lacks wl_compositor, wl_shm or xdg_wm_base"));
        }
        // wl_surface.damage_buffer needs wl_compositor version 4.
        let (c, s, w) = (g.compositor, g.shm, g.wm_base);
        wl.compositor = wl.bind(c.0, "wl_compositor", c.1.min(4), Kind::Compositor);
        wl.shm = wl.bind(s.0, "wl_shm", 1, Kind::Shm);
        wl.wm_base = wl.bind(w.0, "xdg_wm_base", w.1.min(2), Kind::WmBase); // 2: tiled states
        if wl.globals.seat.0 != 0 {
            let s = wl.globals.seat;
            wl.seat_version = s.1.min(5); // 5 adds wl_pointer.frame and axis_discrete
            wl.seat = wl.bind(s.0, "wl_seat", wl.seat_version, Kind::Seat);
            if wl.globals.data_device_manager.0 != 0 {
                let d = wl.globals.data_device_manager;
                wl.data_manager = wl.bind(d.0, "wl_data_device_manager", d.1.min(3), Kind::DataDeviceManager);
                wl.data_device = wl.new_obj(Kind::DataDevice);
                wl.send(wl.data_manager, 1, [Arg::U(wl.data_device), Arg::U(wl.seat)]); // get_data_device
            }
        }
        if wl.globals.cursor_shape.0 != 0 {
            let c = wl.globals.cursor_shape;
            wl.cursor_manager = wl.bind(c.0, "wp_cursor_shape_manager_v1", 1, Kind::CursorShapeManager);
        }

        wl.surface = wl.new_obj(Kind::Surface);
        wl.send(wl.compositor, 0, [Arg::U(wl.surface)]); // wl_compositor.create_surface
        wl.xdg_surface = wl.new_obj(Kind::XdgSurface);
        wl.send(wl.wm_base, 2, [Arg::U(wl.xdg_surface), Arg::U(wl.surface)]); // get_xdg_surface
        wl.toplevel = wl.new_obj(Kind::Toplevel);
        wl.send(wl.xdg_surface, 1, [Arg::U(wl.toplevel)]); // get_toplevel
        wl.send(wl.toplevel, 2, [Arg::S(title)]); // set_title
        wl.send(wl.toplevel, 3, [Arg::S(app_id)]); // set_app_id
        wl.send(wl.toplevel, 8, [Arg::I(MIN_SIZE.0), Arg::I(MIN_SIZE.1)]); // set_min_size
        wl.send(wl.surface, 6, []); // commit: asks for the first configure
        wl.flush()?;
        Ok(wl)
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.sock.as_fd()
    }

    pub fn set_title(&mut self, title: &str) {
        self.send(self.toplevel, 2, [Arg::S(title)]);
    }

    /// Starts an interactive window move, quoting the press that began it.
    pub fn begin_move(&mut self) {
        if self.seat != 0 {
            self.send(self.toplevel, 5, [Arg::U(self.seat), Arg::U(self.serial)]);
        }
    }

    /// Starts an interactive resize from `edges` (`xdg_toplevel.resize_edge` bits).
    pub fn begin_resize(&mut self, edges: u32) {
        if self.seat != 0 {
            self.send(self.toplevel, 6, [Arg::U(self.seat), Arg::U(self.serial), Arg::U(edges)]);
        }
    }

    /// Where the window is inside the surface (the rest is its shadow) and, with `grab`, the band
    /// around it that still takes the pointer (for resizing); without it the whole surface does.
    /// Applies with the next commit.
    pub fn set_geometry(&mut self, x: i32, y: i32, w: i32, h: i32, grab: Option<i32>) {
        self.send(self.xdg_surface, 3, [Arg::I(x), Arg::I(y), Arg::I(w), Arg::I(h)]); // set_window_geometry
        match grab {
            Some(g) => {
                let region = self.new_obj(Kind::Region);
                self.send(self.compositor, 1, [Arg::U(region)]); // create_region
                self.send(region, 1, [Arg::I(x - g), Arg::I(y - g), Arg::I(w + 2 * g), Arg::I(h + 2 * g)]); // add
                self.send(self.surface, 5, [Arg::U(region)]); // set_input_region (copies the region)
                self.send(region, 0, []); // destroy; the id comes back with delete_id
            }
            None => self.send(self.surface, 5, [Arg::U(0)]), // set_input_region(null): everything
        }
    }

    pub fn minimize(&mut self) {
        self.send(self.toplevel, 13, []);
    }

    pub fn toggle_maximized(&mut self) {
        self.send(self.toplevel, if self.maximized { 10 } else { 9 }, []);
    }

    /// Asks for a pointer image by `cursor::*` shape; a no-op without `wp_cursor_shape_v1`.
    pub fn set_cursor(&mut self, shape: u32) {
        if shape == self.shape {
            return;
        }
        self.shape = shape;
        if self.cursor_device != 0 {
            self.send(self.cursor_device, 1, [Arg::U(self.enter_serial), Arg::U(shape)]);
        }
    }

    /// Puts `text` on the clipboard. Needs a recent key or button press (its serial).
    pub fn set_clipboard(&mut self, text: &str) {
        if self.data_manager == 0 || self.data_device == 0 {
            return;
        }
        let src = self.new_obj(Kind::DataSource);
        self.send(self.data_manager, 0, [Arg::U(src)]); // create_data_source
        for m in TEXT_MIMES {
            self.send(src, 0, [Arg::S(m)]); // offer
        }
        self.send(self.data_device, 1, [Arg::U(src), Arg::U(self.serial)]); // set_selection; the old source gets `cancelled`
        self.source = src;
        self.clip_out = text.to_owned();
    }

    /// The text on the clipboard, if any. Blocks up to a second on a slow or broken owner.
    pub fn clipboard_text(&mut self) -> Option<String> {
        if self.source != 0 {
            return Some(self.clip_out.clone()); // ours: asking ourselves over the wire would deadlock
        }
        let offer = self.offers.iter().find(|o| o.id == self.selection)?;
        let mime = TEXT_MIMES[offer.mimes.iter().position(|&m| m)?];
        let (read_end, write_end) = rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC).ok()?;
        let id = offer.id;
        self.send(id, 1, [Arg::S(mime), Arg::Fd(write_end)]); // receive
        self.flush().ok()?; // the write end is closed here: EOF arrives when the owner is done
        let mut out = Vec::new();
        let mut chunk = [0u8; 8192];
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        while out.len() < (8 << 20) {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                break;
            }
            let ts = rustix::event::Timespec { tv_sec: 0, tv_nsec: left.subsec_nanos() as i64 + left.as_secs() as i64 * 1_000_000_000 };
            let mut fds = [PollFd::new(&read_end, PollFlags::IN)];
            match poll(&mut fds, Some(&ts)) {
                Ok(0) => break,
                Ok(_) => match rustix::io::read(&read_end, &mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => out.extend_from_slice(&chunk[..n]),
                },
                Err(rustix::io::Errno::INTR) => {}
                Err(_) => break,
            }
        }
        String::from_utf8(out).ok()
    }

    fn flush_scroll(&mut self) {
        let (pixels, clicks) = std::mem::take(&mut self.axis);
        if pixels != 0.0 || clicks != 0 {
            self.events.push_back(Event::Scroll { pixels, clicks });
        }
    }

    pub fn next_event(&mut self) -> Option<Event> {
        self.events.pop_front()
    }

    /// True between `present` and the compositor showing that frame.
    pub fn frame_pending(&self) -> bool {
        self.frame_pending
    }

    // -------------------------------------------------------------- wire ---

    fn new_obj(&mut self, kind: Kind) -> u32 {
        let id = self.free.pop().unwrap_or(self.kinds.len() as u32);
        if id as usize == self.kinds.len() {
            self.kinds.push(kind);
        } else {
            self.kinds[id as usize] = kind;
        }
        id
    }

    fn bind(&mut self, name: u32, interface: &str, version: u32, kind: Kind) -> u32 {
        let id = self.new_obj(kind);
        self.send(self.registry, 0, [Arg::U(name), Arg::S(interface), Arg::U(version), Arg::U(id)]);
        id
    }

    /// Appends one request to the output buffer; `flush` sends it.
    fn send<const N: usize>(&mut self, obj: u32, op: u16, args: [Arg<'_>; N]) {
        let start = self.out.len();
        self.out.extend_from_slice(&obj.to_ne_bytes());
        self.out.extend_from_slice(&[0; 4]);
        for a in args {
            match a {
                Arg::U(v) => self.out.extend_from_slice(&v.to_ne_bytes()),
                Arg::I(v) => self.out.extend_from_slice(&v.to_ne_bytes()),
                Arg::S(s) => {
                    self.out.extend_from_slice(&(s.len() as u32 + 1).to_ne_bytes());
                    self.out.extend_from_slice(s.as_bytes());
                    self.out.push(0);
                    self.out.resize(self.out.len().next_multiple_of(4), 0);
                }
                Arg::Fd(fd) => self.out_fds.push(fd),
            }
        }
        let size = (self.out.len() - start) as u32;
        self.out[start + 4..start + 8].copy_from_slice(&(size << 16 | op as u32).to_ne_bytes());
        if self.debug {
            eprintln!("wl -> {:?}#{obj}.{op} ({size} bytes)", self.kinds.get(obj as usize));
        }
    }

    pub fn flush(&mut self) -> io::Result<()> {
        let mut sent = 0;
        while sent < self.out.len() {
            let iov = [IoSlice::new(&self.out[sent..])];
            let n = if self.out_fds.is_empty() {
                sendmsg(&self.sock, &iov, &mut SendAncillaryBuffer::default(), SendFlags::NOSIGNAL)?
            } else {
                let fds: Vec<BorrowedFd<'_>> = self.out_fds.iter().map(|f| f.as_fd()).collect();
                let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(4))];
                let mut ctl = SendAncillaryBuffer::new(&mut space);
                if !ctl.push(SendAncillaryMessage::ScmRights(&fds)) {
                    return Err(io::Error::other("too many file descriptors in one flush"));
                }
                sendmsg(&self.sock, &iov, &mut ctl, SendFlags::NOSIGNAL)?
            };
            self.out_fds.clear(); // the kernel has duplicated them
            sent += n;
        }
        self.out.clear();
        Ok(())
    }

    /// Blocks until the socket is readable or `timeout_ms` passes (`None` = forever).
    pub fn wait(&self, timeout_ms: Option<i64>) -> io::Result<()> {
        let mut fds = [PollFd::new(&self.sock, PollFlags::IN)];
        let ts = timeout_ms.map(|ms| rustix::event::Timespec { tv_sec: ms / 1000, tv_nsec: (ms % 1000) * 1_000_000 });
        loop {
            match poll(&mut fds, ts.as_ref()) {
                Err(rustix::io::Errno::INTR) => continue,
                r => return r.map(|_| ()).map_err(io::Error::from),
            }
        }
    }

    /// Reads what the compositor has sent (without blocking) and handles every complete event.
    pub fn read_events(&mut self) -> io::Result<()> {
        loop {
            let at = self.inb.len();
            self.inb.resize(at + 4096, 0);
            let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(4))];
            let mut ctl = RecvAncillaryBuffer::new(&mut space);
            let r = recvmsg(
                &self.sock,
                &mut [IoSliceMut::new(&mut self.inb[at..])],
                &mut ctl,
                RecvFlags::DONTWAIT | RecvFlags::CMSG_CLOEXEC,
            );
            match r {
                Ok(m) => {
                    self.inb.truncate(at + m.bytes);
                    for msg in ctl.drain() {
                        if let RecvAncillaryMessage::ScmRights(fds) = msg {
                            self.in_fds.extend(fds);
                        }
                    }
                    if m.bytes == 0 {
                        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "the compositor closed the connection"));
                    }
                }
                Err(rustix::io::Errno::AGAIN) => {
                    self.inb.truncate(at);
                    break;
                }
                Err(rustix::io::Errno::INTR) => self.inb.truncate(at),
                Err(e) => return Err(e.into()),
            }
        }
        self.dispatch();
        match self.error.take() {
            Some(e) => Err(io::Error::other(e)),
            None => Ok(()),
        }
    }

    pub fn roundtrip(&mut self) -> io::Result<()> {
        self.sync_done = false;
        let cb = self.new_obj(Kind::Sync);
        self.send(DISPLAY, 0, [Arg::U(cb)]); // wl_display.sync
        self.flush()?;
        while !self.sync_done {
            self.wait(None)?;
            self.read_events()?;
        }
        Ok(())
    }

    fn dispatch(&mut self) {
        let mut buf = std::mem::take(&mut self.inb);
        let mut pos = 0;
        while buf.len() - pos >= 8 {
            let id = u32::from_ne_bytes(buf[pos..pos + 4].try_into().unwrap());
            let w = u32::from_ne_bytes(buf[pos + 4..pos + 8].try_into().unwrap());
            let (size, op) = ((w >> 16) as usize, (w & 0xffff) as u16);
            if size < 8 || buf.len() - pos < size {
                break;
            }
            let mut args = Args { b: &buf[pos + 8..pos + size], p: 0 };
            self.event(id, op, &mut args);
            pos += size;
        }
        buf.drain(..pos);
        self.inb = buf;
    }

    fn event(&mut self, id: u32, op: u16, a: &mut Args<'_>) {
        if id >= SERVER_ID_BASE {
            // wl_data_offer.offer: one more format the offer can give.
            if op == 0 {
                let mime = a.str();
                if let (Some(o), Some(i)) = (self.offers.iter_mut().find(|o| o.id == id), TEXT_MIMES.iter().position(|m| *m == mime)) {
                    o.mimes[i] = true;
                }
            }
            return;
        }
        let kind = self.kinds.get(id as usize).copied().unwrap_or(Kind::None);
        if self.debug {
            eprintln!("wl <- {kind:?}#{id}.{op}");
        }
        match (kind, op) {
            (Kind::Display, 0) => {
                let (obj, code, msg) = (a.u32(), a.u32(), a.str());
                self.error = Some(format!("wayland error on object {obj}, code {code}: {msg}"));
            }
            (Kind::Display, 1) => {
                // delete_id: the server is done with the id, it may be reused.
                let gone = a.u32() as usize;
                if gone < self.kinds.len() {
                    self.kinds[gone] = Kind::None;
                    self.free.push(gone as u32);
                }
            }
            (Kind::Registry, 0) => {
                let (name, interface, version) = (a.u32(), a.str(), a.u32());
                match interface {
                    "wl_compositor" => self.globals.compositor = (name, version),
                    "wl_shm" => self.globals.shm = (name, version),
                    "xdg_wm_base" => self.globals.wm_base = (name, version),
                    "wl_seat" => self.globals.seat = (name, version),
                    "wp_cursor_shape_manager_v1" => self.globals.cursor_shape = (name, version),
                    "wl_data_device_manager" => self.globals.data_device_manager = (name, version),
                    _ => {}
                }
            }
            (Kind::Sync, 0) => self.sync_done = true,
            (Kind::Frame, 0) => {
                self.frame_pending = false;
                self.events.push_back(Event::FrameDone);
            }
            (Kind::Buffer(slot), 0) => self.slots[slot as usize].busy = false, // wl_buffer.release
            (Kind::WmBase, 0) => {
                let serial = a.u32();
                self.send(self.wm_base, 3, [Arg::U(serial)]); // pong
            }
            (Kind::Toplevel, 0) => {
                let (w, h) = (a.i32(), a.i32());
                let states = a.array();
                let has = |state| states.chunks_exact(4).any(|s| u32::from_ne_bytes(s.try_into().unwrap()) == state);
                let maximized = has(STATE_MAXIMIZED);
                let square = maximized || has(STATE_FULLSCREEN) || STATE_TILED.iter().any(|&t| has(t));
                self.pending = (w, h, has(STATE_ACTIVATED), maximized, square);
            }
            (Kind::Toplevel, 1) => self.events.push_back(Event::Close),
            (Kind::XdgSurface, 0) => {
                let serial = a.u32();
                self.send(self.xdg_surface, 4, [Arg::U(serial)]); // ack_configure
                let (w, h, activated, maximized, square) = self.pending;
                self.maximized = maximized;
                self.events.push_back(Event::Configure { w, h, square });
                if activated != self.activated {
                    self.activated = activated;
                    self.events.push_back(Event::Focus(activated));
                }
            }
            (Kind::Seat, 0) => {
                // wl_seat.capabilities: bit 0 is the pointer, bit 1 the keyboard.
                let caps = a.u32();
                if caps & 2 != 0 && self.keyboard == 0 {
                    self.keyboard = self.new_obj(Kind::Keyboard);
                    self.send(self.seat, 1, [Arg::U(self.keyboard)]); // get_keyboard
                }
                if caps & 1 != 0 && self.pointer == 0 {
                    self.pointer = self.new_obj(Kind::Pointer);
                    self.send(self.seat, 0, [Arg::U(self.pointer)]); // get_pointer
                    if self.cursor_manager != 0 {
                        self.cursor_device = self.new_obj(Kind::CursorShapeDevice);
                        self.send(self.cursor_manager, 1, [Arg::U(self.cursor_device), Arg::U(self.pointer)]);
                    }
                }
            }
            (Kind::Pointer, 0) => {
                let (serial, _surface, x, y) = (a.u32(), a.u32(), a.i32(), a.i32());
                self.serial = serial;
                self.enter_serial = serial;
                if self.cursor_device != 0 {
                    self.send(self.cursor_device, 1, [Arg::U(serial), Arg::U(self.shape)]);
                }
                self.events.push_back(Event::PointerMove { x: x as f32 / 256.0, y: y as f32 / 256.0 });
            }
            (Kind::Pointer, 1) => self.events.push_back(Event::PointerLeave),
            (Kind::Pointer, 2) => {
                let (_time, x, y) = (a.u32(), a.i32(), a.i32());
                self.events.push_back(Event::PointerMove { x: x as f32 / 256.0, y: y as f32 / 256.0 });
            }
            (Kind::Pointer, 4) => {
                // axis: vertical scroll in surface pixels; horizontal is not used by a terminal.
                let (_time, axis, value) = (a.u32(), a.u32(), a.i32());
                if axis == 0 {
                    self.axis.0 += value as f32 / 256.0;
                    if self.seat_version < 5 {
                        self.flush_scroll();
                    }
                }
            }
            (Kind::Pointer, 5) => self.flush_scroll(),
            (Kind::Pointer, 8) => {
                let (axis, discrete) = (a.u32(), a.i32());
                if axis == 0 {
                    self.axis.1 += discrete;
                }
            }
            (Kind::Keyboard, 0) => {
                let (_format, size) = (a.u32(), a.u32());
                if !self.in_fds.is_empty() {
                    let fd = self.in_fds.remove(0);
                    if let Some(text) = read_keymap(&fd, size as usize) {
                        self.events.push_back(Event::Keymap(text));
                    }
                }
            }
            (Kind::Keyboard, 1) => self.serial = a.u32(), // enter: its serial is valid for set_selection
            (Kind::Keyboard, 2) => self.events.push_back(Event::KeyboardLeave),
            (Kind::Keyboard, 3) => {
                let (serial, _time, key, state) = (a.u32(), a.u32(), a.u32(), a.u32());
                self.serial = serial;
                self.events.push_back(Event::Key { key, pressed: state == 1 });
            }
            (Kind::Keyboard, 4) => {
                let (_serial, depressed, latched, locked, group) = (a.u32(), a.u32(), a.u32(), a.u32(), a.u32());
                self.events.push_back(Event::Modifiers { depressed, latched, locked, group });
            }
            (Kind::Keyboard, 5) => {
                let (rate, delay) = (a.i32(), a.i32());
                self.events.push_back(Event::RepeatInfo { rate, delay });
            }
            (Kind::DataDevice, 0) => {
                // data_offer: a new offer, whose formats follow as events on it.
                let id = a.u32();
                self.offers.push(Offer { id, mimes: [false; 3] });
            }
            (Kind::DataDevice, 5) => {
                // selection: the clipboard owner changed; offers that are not it are dead weight.
                self.selection = a.u32();
                let keep = self.selection;
                let gone: Vec<u32> = self.offers.iter().map(|o| o.id).filter(|&i| i != keep).collect();
                for id in gone {
                    self.send(id, 2, []); // wl_data_offer.destroy
                }
                self.offers.retain(|o| o.id == keep);
            }
            (Kind::DataSource, 1) => {
                // send: another client wants our text, written to the fd it gave us. A thread, so a
                // slow reader cannot stall the window.
                let _mime = a.str();
                if !self.in_fds.is_empty() {
                    let fd = self.in_fds.remove(0);
                    let text = self.clip_out.clone();
                    std::thread::spawn(move || {
                        use std::io::Write;
                        let _ = std::fs::File::from(fd).write_all(text.as_bytes());
                    });
                }
            }
            (Kind::DataSource, 2) => {
                // cancelled: someone else owns the clipboard now.
                if id == self.source {
                    self.source = 0;
                    self.clip_out = String::new();
                }
                self.kinds[id as usize] = Kind::None;
                self.send(id, 1, []); // wl_data_source.destroy
            }
            (Kind::Pointer, 3) => {
                let (serial, time, button, state) = (a.u32(), a.u32(), a.u32(), a.u32());
                self.serial = serial;
                self.events.push_back(Event::PointerButton { button, pressed: state == 1, time });
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------ pixels ---

    fn alloc_slot(&mut self, i: usize, w: usize, h: usize) -> io::Result<()> {
        let bytes = w * h * 4;
        let fd = memfd_create("axon-shm", MemfdFlags::CLOEXEC)?;
        ftruncate(&fd, bytes as u64)?;
        // SAFETY: a fresh shared mapping of a file we just sized; unmapped in `free_slot`.
        let ptr = unsafe { mmap(std::ptr::null_mut(), bytes, ProtFlags::READ | ProtFlags::WRITE, MapFlags::SHARED, &fd, 0)? };
        let pool = self.new_obj(Kind::Pool);
        self.send(self.shm, 0, [Arg::U(pool), Arg::Fd(fd), Arg::I(bytes as i32)]); // create_pool
        let buf = self.new_obj(Kind::Buffer(i as u8));
        let (wi, hi) = (w as i32, h as i32);
        self.send(pool, 0, [Arg::U(buf), Arg::I(0), Arg::I(wi), Arg::I(hi), Arg::I(wi * 4), Arg::U(ARGB8888)]); // create_buffer
        self.send(pool, 1, []); // destroy: the buffer keeps the memory alive
        self.slots[i] = Slot { id: buf, ptr: ptr as *mut u32, w, h, busy: false };
        self.flush()
    }

    fn free_slot(&mut self, i: usize) {
        let s = std::mem::replace(&mut self.slots[i], Slot::EMPTY);
        if s.allocated() {
            self.kinds[s.id as usize] = Kind::None; // a late `release` must not touch the next buffer in this slot
            self.send(s.id, 0, []); // wl_buffer.destroy
            // SAFETY: the mapping created in `alloc_slot` with this length.
            unsafe { munmap(s.ptr as *mut _, s.len() * 4).ok() };
        }
    }

    /// A `w` x `h` framebuffer to draw into, holding what was last presented, or `None` while the
    /// compositor still holds every buffer (wait for its `release` and ask again).
    ///
    /// One buffer is enough when the compositor releases it right after copying (Mutter does). One
    /// that keeps it until the next attach gets a second buffer, filled with the current pixels, so
    /// the renderer can go on repainting only what changed.
    pub fn framebuffer(&mut self, w: usize, h: usize) -> Option<&mut [u32]> {
        let cur = self.cur;
        if self.slots[cur].allocated() && (self.slots[cur].w, self.slots[cur].h) != (w, h) {
            for i in 0..2 {
                self.free_slot(i); // resized: both buffers are the wrong size
            }
        }
        if !self.slots[cur].allocated() {
            self.alloc_slot(cur, w, h).ok()?;
        } else if self.slots[cur].busy {
            let o = 1 - cur;
            if self.slots[o].busy {
                return None;
            }
            if !self.slots[o].allocated() {
                self.alloc_slot(o, w, h).ok()?;
            }
            // SAFETY: two distinct mappings of `len` pixels each.
            unsafe { std::ptr::copy_nonoverlapping(self.slots[cur].ptr, self.slots[o].ptr, self.slots[o].len()) };
            self.cur = o;
        }
        let s = self.slots[self.cur];
        // SAFETY: the mapping holds `w * h` pixels and nothing else touches a non-busy buffer.
        Some(unsafe { std::slice::from_raw_parts_mut(s.ptr, s.len()) })
    }

    /// Shows what was drawn into the slice `framebuffer` returned, and asks for a frame callback.
    pub fn present(&mut self) {
        let s = &mut self.slots[self.cur];
        s.busy = true;
        let (id, w, h) = (s.id, s.w as i32, s.h as i32);
        self.send(self.surface, 1, [Arg::U(id), Arg::I(0), Arg::I(0)]); // attach
        self.send(self.surface, 9, [Arg::I(0), Arg::I(0), Arg::I(w), Arg::I(h)]); // damage_buffer
        let cb = self.new_obj(Kind::Frame);
        self.send(self.surface, 3, [Arg::U(cb)]); // frame
        self.send(self.surface, 6, []); // commit
        self.frame_pending = true;
    }
}

/// The keymap the compositor shares: `size` bytes of text in `fd` (mapped private, as the protocol asks).
fn read_keymap(fd: &OwnedFd, size: usize) -> Option<String> {
    // SAFETY: a read-only private mapping of `size` bytes, unmapped before returning.
    let ptr = unsafe { mmap(std::ptr::null_mut(), size, ProtFlags::READ, MapFlags::PRIVATE, fd, 0).ok()? };
    // SAFETY: the mapping is `size` bytes long and nothing else writes to a private mapping of ours.
    let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, size) };
    let text = String::from_utf8_lossy(bytes.split(|&b| b == 0).next().unwrap_or(&[])).into_owned();
    unsafe { munmap(ptr, size).ok() };
    Some(text)
}

impl Drop for Wl {
    fn drop(&mut self) {
        for i in 0..2 {
            self.free_slot(i);
        }
        let _ = self.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_read_numbers_strings_and_arrays() {
        let mut b = Vec::new();
        b.extend_from_slice(&7u32.to_ne_bytes());
        b.extend_from_slice(&(-3i32).to_ne_bytes());
        b.extend_from_slice(&6u32.to_ne_bytes()); // "hello" + NUL
        b.extend_from_slice(b"hello\0\0\0");
        b.extend_from_slice(&4u32.to_ne_bytes());
        b.extend_from_slice(&[1, 2, 3, 4]);
        let mut a = Args { b: &b, p: 0 };
        assert_eq!(a.u32(), 7);
        assert_eq!(a.i32(), -3);
        assert_eq!(a.str(), "hello");
        assert_eq!(a.array(), &[1, 2, 3, 4]);
    }

    #[test]
    fn short_input_reads_zeros_instead_of_panicking() {
        let mut a = Args { b: &[1, 0], p: 0 };
        assert_eq!(a.u32(), 0);
        assert_eq!(a.str(), "");
    }
}
