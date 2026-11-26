//! Own Win32 window (ADR 0039): the Windows counterpart of `wl.rs`. One top-level window without a frame whose pixels
//! are a CPU-drawn ARGB buffer (`UpdateLayeredWindow`, per-pixel alpha, so the glass, the rounded corners and the
//! shadow the renderer paints are the window), and the same few methods `wlapp.rs` calls on the Wayland client.
//!
//! The window is the whole buffer, shadow margin included, as on Wayland. Moving, resizing and maximizing are done
//! here, not by the system's frame (there is none): a drag is a few `SetWindowPos` calls driven by the mouse messages,
//! so the app is redrawn at every size on the way, in its own loop. Keys are translated here with the system's layout
//! (`ToUnicodeEx`: dead keys, AltGr) into the `Key` the app already understands; nothing is linked beyond
//! `windows-sys`' declarations.
//!
//! Messages are pumped by `read_events` and the window procedure turns them into `Event`s in a queue; no reference to
//! the state lives across a call into the system, because the window procedure runs inside many of them.
use std::{
    collections::VecDeque,
    ffi::c_void,
    io,
    mem::{size_of, zeroed},
    path::PathBuf,
    ptr::{null, null_mut},
    time::Duration,
};

use windows_sys::{
    Wdk::System::Threading::{NtQueryInformationProcess, ProcessBasicInformation},
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        Storage::FileSystem::{CreateFileW, FILE_SHARE_WRITE, OPEN_EXISTING},
        System::{
            Console::{ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle},
            DataExchange::{CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData},
            Diagnostics::Debug::ReadProcessMemory,
            LibraryLoader::GetModuleHandleW,
            Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock},
            ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
            Threading::{GetCurrentProcess, GetExitCodeProcess, GetProcessTimes, INFINITE, OpenProcess, PROCESS_BASIC_INFORMATION, PROCESS_QUERY_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ},
        },
        UI::{
            Input::KeyboardAndMouse::*,
            Shell::SetCurrentProcessExplicitAppUserModelID,
            WindowsAndMessaging::*,
        },
    },
};

use crate::{
    chrome,
    plat::{Event, Key, Mods, Wake, cursor},
};

/// Smallest window the title bar and a usable grid fit in (the same as the Wayland client's).
const MIN_SIZE: (i32, i32) = (320, 200);
/// What a first window measures, before the user changes it: the window itself, without its shadow.
const FIRST_SIZE: (i32, i32) = (1000, 640);
/// `CF_UNICODETEXT`.
const CF_UNICODETEXT: u32 = 13;
/// `WM_MOUSELEAVE` (declared with the common controls, which this window does not use).
const WM_MOUSELEAVE: u32 = 0x02A3;
/// The wheel's notch (`WHEEL_DELTA`).
const WHEEL_DELTA: i32 = 120;
/// Window-class name and the application identity the taskbar groups windows by.
const CLASS: &str = "axon_window";
const APP_ID: &str = "axon.terminal";
/// The window's icon: `assets/axon.ico`, PNG images from 16 to 256 px.
const ICON: &[u8] = include_bytes!("../assets/axon.ico");
/// `edges` bits (`chrome::EDGE_*`).
const EDGE_TOP: u32 = chrome::EDGE_TOP;
const EDGE_BOTTOM: u32 = chrome::EDGE_BOTTOM;
const EDGE_LEFT: u32 = chrome::EDGE_LEFT;
const EDGE_RIGHT: u32 = chrome::EDGE_RIGHT;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The PNG of the icon file's smallest image that is at least `px` wide.
fn ico_png(ico: &[u8], px: u32) -> Option<&[u8]> {
    let count = u16::from_le_bytes(ico.get(4..6)?.try_into().ok()?) as usize;
    let mut best: Option<(u32, &[u8])> = None;
    for i in 0..count {
        let e = ico.get(6 + 16 * i..22 + 16 * i)?;
        let w = if e[0] == 0 { 256 } else { e[0] as u32 };
        let len = u32::from_le_bytes(e[8..12].try_into().ok()?) as usize;
        let off = u32::from_le_bytes(e[12..16].try_into().ok()?) as usize;
        if w >= px && best.is_none_or(|(b, _)| w < b) {
            best = Some((w, ico.get(off..off + len)?));
        }
    }
    best.map(|(_, png)| png)
}

/// An icon `px` wide from the embedded file, or null (the window then has the system's default).
fn load_icon(px: i32) -> HICON {
    let Some(png) = ico_png(ICON, px as u32) else { return null_mut() };
    // SAFETY: `png` is a valid slice for the call; the icon lives as long as the process, which uses it for its window.
    unsafe { CreateIconFromResourceEx(png.as_ptr(), png.len() as u32, 1, 0x0003_0000, px, px, 0) }
}

fn loword(v: isize) -> i32 {
    (v & 0xffff) as u16 as i32
}

fn hiword(v: isize) -> i32 {
    ((v >> 16) & 0xffff) as u16 as i32
}

/// Signed x and y of a mouse message's `lParam`.
fn point_of(lp: isize) -> (i32, i32) {
    (loword(lp) as i16 as i32, hiword(lp) as i16 as i32)
}

/// A window or a resize in progress, driven by the mouse until its button goes up.
enum Drag {
    /// The cursor and the window's corner when it began.
    Move { cursor: POINT, origin: POINT },
    /// The edges (`EDGE_*`) being pulled, the cursor and the window's rectangle when it began.
    Resize { edges: u32, cursor: POINT, rect: RECT },
}

struct State {
    hwnd: HWND,
    events: VecDeque<Event>,
    /// The buffer the window shows: a DIB section in a memory DC, `bw` x `bh` pixels of premultiplied ARGB.
    dc: HDC,
    bmp: HBITMAP,
    bits: *mut u32,
    bw: usize,
    bh: usize,
    /// Maximized by us (`toggle_maximized`), and the rectangle to go back to.
    maximized: bool,
    normal: RECT,
    /// Where the window is inside the buffer and the band around it that takes the pointer (`set_geometry`).
    geo: (i32, i32, i32, i32),
    grab: Option<i32>,
    cursor: HCURSOR,
    /// `WM_MOUSELEAVE` is asked for, and the pointer is inside what takes it.
    tracking: bool,
    inside: bool,
    /// Buttons held: the mouse is captured while any is.
    held: u8,
    drag: Option<Drag>,
    /// What the wheel has turned that is not yet a notch.
    wheel: i32,
    /// The last client size told to the app.
    client: (i32, i32),
}

impl State {
    fn new() -> State {
        State {
            hwnd: null_mut(),
            events: VecDeque::new(),
            dc: null_mut(),
            bmp: null_mut(),
            bits: null_mut(),
            bw: 0,
            bh: 0,
            maximized: false,
            normal: RECT { left: 0, top: 0, right: 0, bottom: 0 },
            geo: (0, 0, 0, 0),
            grab: None,
            cursor: null_mut(),
            tracking: false,
            inside: true,
            held: 0,
            drag: None,
            wheel: 0,
            client: (0, 0),
        }
    }

    /// Whether `(x, y)` is where the pointer belongs to the window: its geometry and the grab band around it, the
    /// whole buffer when maximized. The rest of the shadow is not ours (the input region of the Wayland client).
    fn takes(&self, x: i32, y: i32) -> bool {
        let (gx, gy, gw, gh) = self.geo;
        let g = match self.grab {
            Some(g) if !self.maximized => g,
            _ => return true,
        };
        x >= gx - g && y >= gy - g && x < gx + gw + g && y < gy + gh + g
    }

    /// The size the app is told from a client size: what the window is without its shadow.
    fn configure(&mut self, cw: i32, ch: i32) {
        if (cw, ch) == self.client || cw <= 0 || ch <= 0 {
            return;
        }
        self.client = (cw, ch);
        let (ml, mt, mb) = chrome::margins(self.maximized);
        self.events.push_back(Event::Configure { w: cw - 2 * ml as i32, h: ch - (mt + mb) as i32, square: self.maximized });
    }

    /// A new buffer of `w` x `h` pixels (cleared: the app paints it all).
    fn realloc(&mut self, w: usize, h: usize) -> bool {
        // SAFETY: plain GDI calls on handles this state owns; the DIB section's memory lives as long as `bmp`.
        unsafe {
            if self.dc.is_null() {
                self.dc = CreateCompatibleDC(null_mut());
                if self.dc.is_null() {
                    return false;
                }
            }
            let mut bi: BITMAPINFO = zeroed();
            bi.bmiHeader = BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w as i32,
                biHeight: -(h as i32), // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            };
            let mut bits: *mut c_void = null_mut();
            let bmp = CreateDIBSection(self.dc, &bi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            if bmp.is_null() || bits.is_null() {
                return false;
            }
            SelectObject(self.dc, bmp);
            if !self.bmp.is_null() {
                DeleteObject(self.bmp);
            }
            (self.bmp, self.bits, self.bw, self.bh) = (bmp, bits as *mut u32, w, h);
        }
        true
    }

    fn push_pointer_leave(&mut self) {
        self.events.push_back(Event::PointerLeave);
    }
}

/// The window layer: one window, a queue of what happened to it.
pub struct Win {
    /// Owned, freed in `Drop`; the window procedure reaches it through the window's user data.
    st: *mut State,
}

// SAFETY: the window and its state belong to the thread that made them and `Win` is used from that thread only.
impl Win {
    /// The state. No reference to it may live across a call into the system (the window procedure mutates it).
    #[allow(clippy::mut_from_ref)]
    fn s(&self) -> &mut State {
        // SAFETY: `st` is valid from `connect` to `drop`; callers keep the reference to a statement.
        unsafe { &mut *self.st }
    }

    /// Makes the window, hidden until the first frame is presented, and queues its first size.
    pub fn connect(title: &str, _app_id: &str) -> io::Result<Win> {
        // SAFETY: Win32 calls with valid arguments; the state pointer outlives the window (freed in `Drop` after it).
        unsafe {
            // Windows of this process group in the taskbar under one identity, not under the executable's.
            SetCurrentProcessExplicitAppUserModelID(wide(APP_ID).as_ptr());
            let inst = GetModuleHandleW(null());
            let class = wide(CLASS);
            let wc = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                style: 0,
                lpfnWndProc: Some(wndproc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: inst,
                hIcon: load_icon(GetSystemMetrics(SM_CXICON)),
                hCursor: LoadCursorW(null_mut(), IDC_ARROW),
                hbrBackground: null_mut(),
                lpszMenuName: null(),
                lpszClassName: class.as_ptr(),
                hIconSm: load_icon(GetSystemMetrics(SM_CXSMICON)),
            };
            if RegisterClassExW(&wc) == 0 {
                return Err(io::Error::last_os_error());
            }
            let st = Box::into_raw(Box::new(State::new()));
            (*st).cursor = LoadCursorW(null_mut(), IDC_ARROW);

            let (ml, mt, mb) = chrome::margins(false);
            let (w, h) = (FIRST_SIZE.0 + 2 * ml as i32, FIRST_SIZE.1 + (mt + mb) as i32);
            let mut work: RECT = zeroed();
            SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut work as *mut RECT as *mut c_void, 0);
            let (w, h) = (w.min(work.right - work.left), h.min(work.bottom - work.top));
            let (x, y) = (work.left + (work.right - work.left - w) / 2, work.top + (work.bottom - work.top - h) / 2);

            let hwnd = CreateWindowExW(
                WS_EX_APPWINDOW | WS_EX_LAYERED,
                class.as_ptr(),
                wide(title).as_ptr(),
                WS_POPUP | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX,
                x,
                y,
                w,
                h,
                null_mut(),
                null_mut(),
                inst,
                st as *const c_void,
            );
            if hwnd.is_null() {
                let e = io::Error::last_os_error();
                drop(Box::from_raw(st));
                return Err(e);
            }
            (*st).hwnd = hwnd;
            (*st).client = (w, h);
            (*st).events.push_back(Event::Configure { w: w - 2 * ml as i32, h: h - (mt + mb) as i32, square: false });
            ShowWindow(hwnd, SW_SHOW);
            Ok(Win { st })
        }
    }

    // --------------------------------------------------------- the loop --

    /// Blocks until a message arrives or `timeout_ms` passes (`None` = forever).
    pub fn wait(&self, timeout_ms: Option<i64>) -> io::Result<()> {
        wait_messages(None, timeout_ms.map(|ms| Duration::from_millis(ms.max(0) as u64))).map(|_| ())
    }

    /// Blocks until a message arrives, `wake` is signalled or `timeout` passes (`None` = forever): `(messages may be
    /// waiting, wake was signalled)`. Asking for the messages is cheap, so the first is always true.
    pub fn wait_wake(&self, wake: &Wake, timeout: Option<Duration>) -> io::Result<(bool, bool)> {
        let woken = wait_messages(Some(wake.0), timeout)?;
        Ok((true, woken))
    }

    /// Handles every message waiting (the window procedure queues the events they mean).
    pub fn read_events(&mut self) -> io::Result<()> {
        // SAFETY: the standard message pump; `msg` is a valid MSG for each call. No `TranslateMessage`: keys are
        // translated by hand (`translate`), the system's second translation would type everything twice.
        unsafe {
            let mut msg: MSG = zeroed();
            while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
                if msg.message == WM_QUIT {
                    self.s().events.push_back(Event::Close);
                    continue;
                }
                DispatchMessageW(&msg);
            }
        }
        Ok(())
    }

    pub fn next_event(&mut self) -> Option<Event> {
        self.s().events.pop_front()
    }

    /// Nothing is buffered between the app and the window: there is nothing to send.
    pub fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }

    /// There is no frame callback: a frame can always be drawn.
    pub fn frame_pending(&self) -> bool {
        false
    }

    pub fn give_up_waiting(&mut self) {}

    // ------------------------------------------------------------ window --

    pub fn set_title(&mut self, title: &str) {
        // SAFETY: a valid window and a NUL-terminated UTF-16 string.
        unsafe { SetWindowTextW(self.s().hwnd, wide(title).as_ptr()) };
    }

    /// Where the window is inside the buffer and the band around it that takes the pointer.
    pub fn set_geometry(&mut self, x: i32, y: i32, w: i32, h: i32, grab: Option<i32>) {
        let s = self.s();
        s.geo = (x, y, w, h);
        s.grab = grab;
    }

    /// The compositor blurs behind a Wayland window; Windows has no call that does so for a window like this one.
    pub fn blur_supported(&self) -> bool {
        false
    }

    pub fn set_blur(&mut self, _rects: &[[i32; 4]]) {}

    pub fn minimize(&mut self) {
        // SAFETY: a valid window.
        unsafe { ShowWindow(self.s().hwnd, SW_MINIMIZE) };
    }

    /// Fills the work area (the screen without the taskbar) or goes back to where the window was.
    pub fn toggle_maximized(&mut self) {
        toggle_maximized(self.st);
    }

    /// Starts moving the window with the pointer, which holds the button that was pressed.
    pub fn begin_move(&mut self) {
        let st = self.st;
        // SAFETY: valid window; the drag is state of ours, ended by the button going up or the capture being lost.
        unsafe {
            if GetKeyState(VK_LBUTTON as i32) >= 0 {
                return; // the button is already up: nothing to drag
            }
            if (*st).maximized {
                // Pulled from the top of a maximized window: it comes back to its size under the pointer.
                let mut p: POINT = zeroed();
                GetCursorPos(&mut p);
                let work = work_area((*st).hwnd);
                let n = (*st).normal;
                let (nw, nh) = (n.right - n.left, n.bottom - n.top);
                let frac = (p.x - work.left) as f32 / (work.right - work.left).max(1) as f32;
                let left = p.x - (nw as f32 * frac) as i32;
                (*st).normal = RECT { left, top: p.y - 20, right: left + nw, bottom: p.y - 20 + nh };
                toggle_maximized(st);
            }
            let mut p: POINT = zeroed();
            GetCursorPos(&mut p);
            let mut r: RECT = zeroed();
            GetWindowRect((*st).hwnd, &mut r);
            (*st).drag = Some(Drag::Move { cursor: p, origin: POINT { x: r.left, y: r.top } });
            SetCapture((*st).hwnd);
            (*st).push_pointer_leave();
        }
    }

    /// Starts pulling the window by `edges` (`chrome::EDGE_*` bits).
    pub fn begin_resize(&mut self, edges: u32) {
        let st = self.st;
        // SAFETY: as in `begin_move`.
        unsafe {
            if GetKeyState(VK_LBUTTON as i32) >= 0 || (*st).maximized {
                return;
            }
            let mut p: POINT = zeroed();
            GetCursorPos(&mut p);
            let mut r: RECT = zeroed();
            GetWindowRect((*st).hwnd, &mut r);
            (*st).drag = Some(Drag::Resize { edges, cursor: p, rect: r });
            SetCapture((*st).hwnd);
            (*st).push_pointer_leave();
        }
    }

    /// The pointer's shape (`plat::cursor`) as Windows draws it.
    pub fn set_cursor(&mut self, shape: u32) {
        let id = match shape {
            cursor::POINTER => IDC_HAND,
            cursor::TEXT => IDC_IBEAM,
            cursor::GRABBING => IDC_SIZEALL,
            cursor::E_RESIZE | cursor::W_RESIZE | cursor::COL_RESIZE => IDC_SIZEWE,
            cursor::N_RESIZE | cursor::S_RESIZE | cursor::ROW_RESIZE => IDC_SIZENS,
            cursor::NE_RESIZE | cursor::SW_RESIZE => IDC_SIZENESW,
            cursor::NW_RESIZE | cursor::SE_RESIZE => IDC_SIZENWSE,
            _ => IDC_ARROW,
        };
        // SAFETY: stock cursors are shared and never freed.
        unsafe {
            let c = LoadCursorW(null_mut(), id);
            self.s().cursor = c;
            SetCursor(c);
        }
    }

    // --------------------------------------------------------- clipboard --

    /// Puts `text` on the clipboard (lines end in CR LF there).
    pub fn set_clipboard(&mut self, text: &str) {
        let mut data: Vec<u16> = Vec::with_capacity(text.len() + 1);
        let mut prev = 0u16;
        for u in text.encode_utf16() {
            if u == b'\n' as u16 && prev != b'\r' as u16 {
                data.push(b'\r' as u16);
            }
            data.push(u);
            prev = u;
        }
        data.push(0);
        // SAFETY: the clipboard is opened and closed here; the global memory is moved into it only when it was filled.
        unsafe {
            if !open_clipboard(self.s().hwnd) {
                return;
            }
            EmptyClipboard();
            let h = GlobalAlloc(GMEM_MOVEABLE, data.len() * 2);
            if !h.is_null() {
                let p = GlobalLock(h) as *mut u16;
                if p.is_null() {
                    GlobalFree(h);
                } else {
                    std::ptr::copy_nonoverlapping(data.as_ptr(), p, data.len());
                    GlobalUnlock(h);
                    if SetClipboardData(CF_UNICODETEXT, h as HANDLE).is_null() {
                        GlobalFree(h); // not taken: still ours
                    }
                }
            }
            CloseClipboard();
        }
    }

    /// The text on the clipboard (lines end in LF here), if it holds text.
    pub fn clipboard_text(&mut self) -> Option<String> {
        // SAFETY: as in `set_clipboard`; the memory is read, as UTF-16 up to its NUL, only while locked.
        unsafe {
            if !open_clipboard(self.s().hwnd) {
                return None;
            }
            let mut out = None;
            let h = GetClipboardData(CF_UNICODETEXT);
            if !h.is_null() {
                let p = GlobalLock(h as _) as *const u16;
                if !p.is_null() {
                    let mut n = 0;
                    while *p.add(n) != 0 {
                        n += 1;
                    }
                    out = Some(String::from_utf16_lossy(std::slice::from_raw_parts(p, n)).replace("\r\n", "\n"));
                    GlobalUnlock(h as _);
                }
            }
            CloseClipboard();
            out
        }
    }

    // ------------------------------------------------------------ pixels --

    /// The pixels of the window, `w` x `h`, premultiplied ARGB, as they were left by the last frame (the renderer
    /// repaints only what changed). A new size gives a cleared buffer.
    pub fn framebuffer(&mut self, w: usize, h: usize) -> Option<&mut [u32]> {
        let s = self.s();
        if (s.bw, s.bh) != (w, h) || s.bits.is_null() {
            if w == 0 || h == 0 || !s.realloc(w, h) {
                return None;
            }
        }
        // SAFETY: the DIB section holds `bw * bh` pixels and nothing else touches them while the app draws.
        Some(unsafe { std::slice::from_raw_parts_mut(self.s().bits, w * h) })
    }

    /// Shows what was drawn: the window becomes the buffer (its size too) with its own alpha.
    pub fn present(&mut self) {
        let st = self.st;
        // SAFETY: valid window and a memory DC holding the DIB section; the call may resize the window, which runs the
        // window procedure, so only copies of the fields are taken before it.
        unsafe {
            let (hwnd, dc, w, h) = ((*st).hwnd, (*st).dc, (*st).bw as i32, (*st).bh as i32);
            if dc.is_null() || IsIconic(hwnd) != 0 {
                return;
            }
            let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
            let (size, src) = (SIZE { cx: w, cy: h }, POINT { x: 0, y: 0 });
            UpdateLayeredWindow(hwnd, null_mut(), null(), &size, dc, &src, 0, &blend, ULW_ALPHA);
        }
    }
}

impl Drop for Win {
    fn drop(&mut self) {
        // SAFETY: the window, then the buffer behind it, then the state the window procedure used.
        unsafe {
            let s = Box::from_raw(self.st);
            if !s.hwnd.is_null() {
                SetWindowLongPtrW(s.hwnd, GWLP_USERDATA, 0);
                DestroyWindow(s.hwnd);
            }
            if !s.bmp.is_null() {
                DeleteObject(s.bmp);
            }
            if !s.dc.is_null() {
                DeleteDC(s.dc);
            }
        }
    }
}

/// Opens the clipboard, trying a few times: another program may hold it for a moment.
unsafe fn open_clipboard(hwnd: HWND) -> bool {
    for _ in 0..10 {
        // SAFETY: documented to be callable with any window handle or null.
        if unsafe { OpenClipboard(hwnd) } != 0 {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}

/// Waits for a message in this thread's queue or for `handle`, up to `timeout`. True when it was `handle`.
fn wait_messages(handle: Option<HANDLE>, timeout: Option<Duration>) -> io::Result<bool> {
    // Whole milliseconds, rounded up: a wait that ends a hair early would only go round again.
    let ms = timeout.map_or(INFINITE, |d| (d.as_nanos().div_ceil(1_000_000)).min(u128::from(u32::MAX - 1)) as u32);
    let handles = handle.map_or([null_mut(); 1], |h| [h]);
    let count = handle.is_some() as u32;
    // SAFETY: `handles` holds `count` live handles; MWMO_INPUTAVAILABLE returns for input that is already queued.
    let r = unsafe { MsgWaitForMultipleObjectsEx(count, handles.as_ptr(), ms, QS_ALLINPUT, MWMO_INPUTAVAILABLE) };
    match r {
        WAIT_FAILED => Err(io::Error::last_os_error()),
        r => Ok(count == 1 && r == WAIT_OBJECT_0),
    }
}

/// The work area (the monitor without the taskbar) of the monitor `hwnd` is on.
unsafe fn work_area(hwnd: HWND) -> RECT {
    // SAFETY: `mi` is a valid MONITORINFO with its size set.
    unsafe {
        let mut mi: MONITORINFO = zeroed();
        mi.cbSize = size_of::<MONITORINFO>() as u32;
        let mon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        if GetMonitorInfoW(mon, &mut mi) == 0 {
            let mut r: RECT = zeroed();
            SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut r as *mut RECT as *mut c_void, 0);
            return r;
        }
        mi.rcWork
    }
}

fn toggle_maximized(st: *mut State) {
    // SAFETY: `st` is the live state of a live window; `SetWindowPos` runs the window procedure (a `WM_SIZE`), so the
    // fields are written before it and no reference is kept.
    unsafe {
        let hwnd = (*st).hwnd;
        if (*st).maximized {
            (*st).maximized = false;
            let r = (*st).normal;
            SetWindowPos(hwnd, null_mut(), r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
        } else {
            let mut r: RECT = zeroed();
            GetWindowRect(hwnd, &mut r);
            (*st).normal = r;
            (*st).maximized = true;
            let w = work_area(hwnd);
            SetWindowPos(hwnd, null_mut(), w.left, w.top, w.right - w.left, w.bottom - w.top, SWP_NOZORDER | SWP_NOACTIVATE);
        }
    }
}

// ------------------------------------------------------------- messages --

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    // SAFETY: the state pointer is the one `connect` made, set at creation and cleared before the window is destroyed.
    unsafe {
        if msg == WM_NCCREATE {
            let cs = lp as *const CREATESTRUCTW;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, (*cs).lpCreateParams as isize);
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        let st = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
        if st.is_null() {
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        match handle(&mut *st, hwnd, msg, wp, lp) {
            Some(r) => r,
            None => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

/// What axon does with a message; `None` leaves it to the system.
unsafe fn handle(s: &mut State, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
    // SAFETY: calls into the system with the window's own handles and the message's own arguments.
    unsafe {
        match msg {
            // No frame: the client area is the whole window.
            WM_NCCALCSIZE if wp != 0 => Some(0),
            // The default would paint an inactive frame (there is none) and flash.
            WM_NCACTIVATE => Some(DefWindowProcW(hwnd, msg, wp, -1)),
            WM_ERASEBKGND => Some(1),
            WM_PAINT => {
                ValidateRect(hwnd, null());
                Some(0)
            }
            WM_GETMINMAXINFO => {
                let (ml, mt, mb) = chrome::margins(false);
                let mi = lp as *mut MINMAXINFO;
                (*mi).ptMinTrackSize = POINT { x: MIN_SIZE.0 + 2 * ml as i32, y: MIN_SIZE.1 + (mt + mb) as i32 };
                Some(0)
            }
            WM_SIZE => {
                if wp as u32 != SIZE_MINIMIZED {
                    s.configure(loword(lp), hiword(lp));
                }
                Some(0)
            }
            WM_ACTIVATE => {
                s.events.push_back(Event::Focus(loword(wp as isize) != WA_INACTIVE as i32));
                Some(0)
            }
            WM_KILLFOCUS => {
                s.events.push_back(Event::KeyboardLeave);
                Some(0)
            }
            WM_CLOSE => {
                s.events.push_back(Event::Close);
                Some(0)
            }
            // The system menu (Alt+Space) and the menu bar (a bare Alt, F10) are not ours; maximize and restore are.
            WM_SYSCOMMAND => match (wp & 0xfff0) as u32 {
                SC_KEYMENU => Some(0),
                SC_MAXIMIZE if !s.maximized => {
                    toggle_maximized(s);
                    Some(0)
                }
                SC_RESTORE if s.maximized && IsIconic(hwnd) == 0 => {
                    toggle_maximized(s);
                    Some(0)
                }
                _ => None,
            },
            WM_SETCURSOR if loword(lp) == HTCLIENT as i32 => {
                SetCursor(s.cursor);
                Some(1)
            }
            WM_CAPTURECHANGED => {
                s.drag = None;
                s.held = 0;
                None
            }
            WM_MOUSEMOVE => {
                let (x, y) = point_of(lp);
                if s.drag.is_some() {
                    drag_to(s);
                    return Some(0);
                }
                if !s.tracking {
                    let mut t = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                    TrackMouseEvent(&mut t);
                    s.tracking = true;
                }
                if s.takes(x, y) {
                    s.inside = true;
                    s.events.push_back(Event::PointerMove { x: x as f32, y: y as f32 });
                } else if s.inside {
                    s.inside = false;
                    s.push_pointer_leave();
                }
                Some(0)
            }
            WM_MOUSELEAVE => {
                s.tracking = false;
                if s.held == 0 && s.drag.is_none() {
                    s.inside = true;
                    s.push_pointer_leave();
                }
                Some(0)
            }
            WM_LBUTTONDOWN | WM_LBUTTONUP | WM_RBUTTONDOWN | WM_RBUTTONUP | WM_MBUTTONDOWN | WM_MBUTTONUP => {
                let (bit, button) = match msg {
                    WM_LBUTTONDOWN | WM_LBUTTONUP => (1u8, 0x110u32),
                    WM_RBUTTONDOWN | WM_RBUTTONUP => (2, 0x111),
                    _ => (4, 0x112),
                };
                let pressed = matches!(msg, WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN);
                let (x, y) = point_of(lp);
                if s.drag.is_some() {
                    // The button that moved the window goes up: the drag ends and the app never hears of the press.
                    if !pressed && bit == 1 {
                        s.drag = None;
                        s.held = 0;
                        ReleaseCapture();
                    }
                    return Some(0);
                }
                if pressed {
                    if !s.takes(x, y) {
                        return Some(0); // the shadow beyond the grab band: not ours
                    }
                    if s.held == 0 {
                        SetCapture(hwnd);
                    }
                    s.held |= bit;
                    // Moves are not sent while no button is down and the pointer is over nothing of ours: say where it is.
                    s.events.push_back(Event::PointerMove { x: x as f32, y: y as f32 });
                } else {
                    if s.held & bit == 0 {
                        return Some(0);
                    }
                    s.held &= !bit;
                    if s.held == 0 {
                        ReleaseCapture();
                    }
                }
                s.events.push_back(Event::PointerButton { button, pressed, time: GetMessageTime() as u32 });
                Some(0)
            }
            WM_MOUSEWHEEL => {
                s.wheel += ((wp >> 16) & 0xffff) as u16 as i16 as i32;
                let notches = s.wheel / WHEEL_DELTA;
                if notches != 0 {
                    s.wheel -= notches * WHEEL_DELTA;
                    // Windows counts a wheel turned away from the user as positive, the app counts down as positive.
                    s.events.push_back(Event::Scroll { pixels: 0.0, clicks: -notches });
                }
                Some(0)
            }
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                let vk = wp as u32;
                // Alt+F4 closes the window: the system's own.
                if msg == WM_SYSKEYDOWN && vk == VK_F4 as u32 {
                    return None;
                }
                if let Some(key) = translate(vk, lp) {
                    s.events.push_back(Event::Press { key, repeat: (lp >> 30) & 1 == 1 });
                }
                Some(0)
            }
            WM_KEYUP | WM_SYSKEYUP => Some(0),
            _ => None,
        }
    }
}

/// Moves or resizes the window to where the pointer has taken it.
fn drag_to(s: &mut State) {
    // SAFETY: valid window, and `p` is a valid POINT. `SetWindowPos` runs the window procedure (a `WM_SIZE` that
    // queues the new size), so nothing of `s` is borrowed across it.
    unsafe {
        let mut p: POINT = zeroed();
        GetCursorPos(&mut p);
        let hwnd = s.hwnd;
        match &s.drag {
            Some(Drag::Move { cursor, origin }) => {
                let (x, y) = (origin.x + p.x - cursor.x, origin.y + p.y - cursor.y);
                SetWindowPos(hwnd, null_mut(), x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
            }
            Some(Drag::Resize { edges, cursor, rect }) => {
                let (ml, mt, mb) = chrome::margins(false);
                let (min_w, min_h) = (MIN_SIZE.0 + 2 * ml as i32, MIN_SIZE.1 + (mt + mb) as i32);
                let (dx, dy) = (p.x - cursor.x, p.y - cursor.y);
                let (mut l, mut t, mut r, mut b) = (rect.left, rect.top, rect.right, rect.bottom);
                if edges & EDGE_LEFT != 0 {
                    l = (rect.left + dx).min(rect.right - min_w);
                }
                if edges & EDGE_RIGHT != 0 {
                    r = (rect.right + dx).max(rect.left + min_w);
                }
                if edges & EDGE_TOP != 0 {
                    t = (rect.top + dy).min(rect.bottom - min_h);
                }
                if edges & EDGE_BOTTOM != 0 {
                    b = (rect.bottom + dy).max(rect.top + min_h);
                }
                SetWindowPos(hwnd, null_mut(), l, t, r - l, b - t, SWP_NOZORDER | SWP_NOACTIVATE);
            }
            None => {}
        }
    }
}

// ------------------------------------------------------------------ keys --

/// X11 keysyms of the keys that are not characters (what `input.rs` and the shortcuts speak, as `xkb.rs` gives them).
fn special_sym(vk: u16, extended: bool) -> Option<u32> {
    Some(match vk {
        VK_BACK => 0xff08,
        VK_TAB => 0xff09,
        VK_RETURN if extended => 0xff8d, // the keypad's
        VK_RETURN => 0xff0d,
        VK_ESCAPE => 0xff1b,
        VK_HOME => 0xff50,
        VK_LEFT => 0xff51,
        VK_UP => 0xff52,
        VK_RIGHT => 0xff53,
        VK_DOWN => 0xff54,
        VK_PRIOR => 0xff55,
        VK_NEXT => 0xff56,
        VK_END => 0xff57,
        VK_INSERT => 0xff63,
        VK_DELETE => 0xffff,
        VK_F1..=VK_F12 => 0xffbe + (vk - VK_F1) as u32,
        _ => return None,
    })
}

/// Keys that change a state and type nothing by themselves.
fn is_modifier(vk: u16) -> bool {
    matches!(
        vk,
        VK_SHIFT | VK_CONTROL | VK_MENU | VK_LSHIFT | VK_RSHIFT | VK_LCONTROL | VK_RCONTROL | VK_LMENU | VK_RMENU | VK_LWIN | VK_RWIN | VK_CAPITAL | VK_NUMLOCK | VK_SCROLL | VK_APPS
    )
}

fn down(vk: u16) -> bool {
    // SAFETY: reads the thread's keyboard state.
    unsafe { GetKeyState(vk as i32) < 0 }
}

/// The modifiers held now. AltGr is how Windows spells "the third level of this layout": it reports Ctrl and Alt both,
/// and is neither for what a program is sent.
pub fn mods() -> Mods {
    let altgr = down(VK_RMENU) && down(VK_LCONTROL);
    Mods { shift: down(VK_SHIFT), ctrl: down(VK_CONTROL) && !altgr, alt: down(VK_MENU) && !altgr }
}

/// What the key pressed by a `WM_KEYDOWN` means, with the layout in use. `None` for a key that is only a modifier,
/// for a dead key (it is kept by the system and joins the next one) and for one the layout gives nothing for.
fn translate(vk: u32, lp: LPARAM) -> Option<Key> {
    let vk = vk as u16;
    if is_modifier(vk) {
        return None;
    }
    let (scan, extended) = (((lp >> 16) & 0xff) as u32, (lp >> 24) & 1 == 1);
    let m = mods();
    if let Some(sym) = special_sym(vk, extended) {
        return Some(Key { sym, text: String::new(), mods: m });
    }
    // The keypad: its own keysyms, so the shortcuts that name them (Ctrl and + - 0 for the size) work from it.
    let keypad = match vk {
        VK_NUMPAD0..=VK_NUMPAD9 => Some((0xffb0 + (vk - VK_NUMPAD0) as u32, (b'0' + (vk - VK_NUMPAD0) as u8) as char)),
        VK_MULTIPLY => Some((0xffaa, '*')),
        VK_ADD => Some((0xffab, '+')),
        VK_SUBTRACT => Some((0xffad, '-')),
        VK_DECIMAL => Some((0xffae, '.')),
        VK_DIVIDE => Some((0xffaf, '/')),
        _ => None,
    };
    if let Some((sym, c)) = keypad {
        return Some(Key { sym, text: c.to_string(), mods: m });
    }
    // Ask the layout for the character, without Control and Alt (a program is told those separately, and with Control
    // held the system answers with a C0 code): except AltGr, which is the layout's own third level.
    let mut state = [0u8; 256];
    // SAFETY: `state` is the 256 bytes the call fills; `buf` holds `buf.len()` UTF-16 units; the layout is the thread's.
    let n = unsafe {
        GetKeyboardState(state.as_mut_ptr());
        let altgr = down(VK_RMENU) && down(VK_LCONTROL);
        if !altgr {
            for v in [VK_CONTROL, VK_LCONTROL, VK_RCONTROL, VK_MENU, VK_LMENU, VK_RMENU] {
                state[v as usize] = 0;
            }
        }
        let mut buf = [0u16; 8];
        let layout = GetKeyboardLayout(0);
        let n = ToUnicodeEx(vk as u32, scan, state.as_ptr(), buf.as_mut_ptr(), buf.len() as i32, 0, layout);
        if n <= 0 {
            return None; // a dead key (n < 0: the system holds it for the next key) or nothing typed
        }
        String::from_utf16_lossy(&buf[..n as usize])
    };
    let sym = n.chars().next().map_or(0, |c| c as u32);
    Some(Key { sym, text: n, mods: m })
}

// ------------------------------------------------------------- processes --

/// The directory process `pid` is in, read from its own block: the pid's PEB holds a pointer to its parameters, which
/// hold the current directory. What a program that does not change it (PowerShell's `cd` does not) never moves.
pub fn process_cwd(pid: u32) -> Option<PathBuf> {
    // SAFETY: reads another process's memory at the addresses its PEB gives, into buffers of the sizes asked for; every
    // read is checked. Offsets are those of a 64-bit process (`PEB.ProcessParameters` at 0x20,
    // `RTL_USER_PROCESS_PARAMETERS.CurrentDirectory.DosPath` at 0x38: Length, MaximumLength, Buffer at +8).
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid);
        if h.is_null() {
            return None;
        }
        let r = (|| {
            let mut pbi: PROCESS_BASIC_INFORMATION = zeroed();
            let mut ret = 0u32;
            let status = NtQueryInformationProcess(h, ProcessBasicInformation, &mut pbi as *mut _ as *mut c_void, size_of::<PROCESS_BASIC_INFORMATION>() as u32, &mut ret);
            if status != 0 || pbi.PebBaseAddress.is_null() {
                return None;
            }
            let read = |addr: usize, buf: &mut [u8]| ReadProcessMemory(h, addr as *const c_void, buf.as_mut_ptr() as *mut c_void, buf.len(), null_mut()) != 0;
            let mut ptr = [0u8; 8];
            if !read(pbi.PebBaseAddress as usize + 0x20, &mut ptr) {
                return None;
            }
            let params = usize::from_ne_bytes(ptr);
            let mut us = [0u8; 16];
            if params == 0 || !read(params + 0x38, &mut us) {
                return None;
            }
            let len = u16::from_ne_bytes([us[0], us[1]]) as usize;
            let buf = usize::from_ne_bytes(us[8..16].try_into().ok()?);
            if buf == 0 || len == 0 || len > 32768 {
                return None;
            }
            let mut raw = vec![0u8; len];
            if !read(buf, &mut raw) {
                return None;
            }
            let units: Vec<u16> = raw.chunks_exact(2).map(|c| u16::from_ne_bytes([c[0], c[1]])).collect();
            let mut s = String::from_utf16_lossy(&units);
            // "C:\Users\me\" -> "C:\Users\me", but "C:\" stays.
            if s.len() > 3 && s.ends_with('\\') {
                s.pop();
            }
            Some(PathBuf::from(s))
        })();
        CloseHandle(h);
        r
    }
}

/// Whether process `pid` still runs.
pub fn process_alive(pid: u32) -> bool {
    // SAFETY: opens a handle with the least access that can ask, and closes it.
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return false;
        }
        let mut code = 0u32;
        let ok = GetExitCodeProcess(h, &mut code) != 0 && code == STILL_ACTIVE as u32;
        CloseHandle(h);
        ok
    }
}

/// The CPU time this process has used so far (kernel and user), in milliseconds: the self-test's idle check.
pub fn cpu_ms() -> u64 {
    // SAFETY: the pseudo-handle of this process and four FILETIMEs to fill.
    unsafe {
        let (mut created, mut exited, mut kernel, mut user): (FILETIME, FILETIME, FILETIME, FILETIME) = zeroed();
        if GetProcessTimes(GetCurrentProcess(), &mut created, &mut exited, &mut kernel, &mut user) == 0 {
            return 0;
        }
        let t = |f: FILETIME| ((f.dwHighDateTime as u64) << 32 | f.dwLowDateTime as u64) / 10_000; // 100 ns units
        t(kernel) + t(user)
    }
}

/// The memory this process holds in RAM (its working set), in kilobytes: the Windows counterpart of the resident set.
pub fn resident_kb() -> u64 {
    // SAFETY: a counters struct of the size it says, filled for this process.
    unsafe {
        let mut c: PROCESS_MEMORY_COUNTERS = zeroed();
        c.cb = size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        if K32GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) == 0 {
            return 0;
        }
        c.WorkingSetSize as u64 / 1024
    }
}

/// The user's locale as `es-ES`: what the language of the interface follows (no `LANG` on Windows).
pub fn locale_name() -> Option<String> {
    use windows_sys::Win32::Globalization::GetUserDefaultLocaleName;
    let mut buf = [0u16; 85];
    // SAFETY: `buf` is the 85 units (LOCALE_NAME_MAX_LENGTH) the call may write, NUL included.
    let n = unsafe { GetUserDefaultLocaleName(buf.as_mut_ptr(), buf.len() as i32) };
    (n > 1).then(|| String::from_utf16_lossy(&buf[..n as usize - 1]))
}

/// A window program has no console of its own: `axon --version` and `axon config show`, typed in one, print to the
/// console that started them (where the standard handles are not set yet).
pub fn attach_console() {
    if std::env::args_os().nth(1).is_none() {
        return; // no command: the window opens, there is nothing to print
    }
    // SAFETY: attaches to the parent's console and points the standard handles that are not set at it.
    unsafe {
        if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
            return;
        }
        for which in [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            let h = GetStdHandle(which);
            if h.is_null() || h == INVALID_HANDLE_VALUE {
                let con = CreateFileW(wide("CONOUT$").as_ptr(), GENERIC_READ | GENERIC_WRITE, FILE_SHARE_WRITE, null(), OPEN_EXISTING, 0, null_mut());
                if con != INVALID_HANDLE_VALUE {
                    SetStdHandle(which, con);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_icon_file_has_a_png_for_every_size_the_system_asks() {
        for px in [16, 24, 32, 48, 64, 128, 256] {
            let png = ico_png(ICON, px).unwrap_or_else(|| panic!("no image for {px} px"));
            assert!(png.starts_with(b"\x89PNG"), "{px} px is not a PNG");
        }
        // An odd size takes the next bigger image; bigger than all has none.
        assert_eq!(ico_png(ICON, 20).map(<[u8]>::len), ico_png(ICON, 24).map(<[u8]>::len));
        assert!(ico_png(ICON, 257).is_none());
    }

    #[test]
    fn the_system_makes_icons_of_them() {
        for px in [16, 32] {
            assert!(!load_icon(px).is_null(), "CreateIconFromResourceEx refused the {px} px PNG");
        }
    }
}
