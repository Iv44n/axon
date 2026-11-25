//! What the window layer under the app says and is told, the same on every platform. On Linux that layer is the own
//! Wayland client (`wl.rs`, keys by `xkb.rs`); on Windows the own Win32 window (`win.rs`, ADR 0039). `wlapp.rs` is
//! written against these types and the few methods both layers have in common.
//!
//! The shapes of the pointer are `wp_cursor_shape_device_v1`'s numbers: Wayland takes them as they are, Windows maps
//! them to its own cursors.

/// `wp_cursor_shape_device_v1.shape` values the window uses.
pub mod cursor {
    pub const DEFAULT: u32 = 1;
    pub const POINTER: u32 = 4;
    pub const TEXT: u32 = 9;
    pub const GRABBING: u32 = 17;
    pub const E_RESIZE: u32 = 18;
    pub const N_RESIZE: u32 = 19;
    pub const NE_RESIZE: u32 = 20;
    pub const NW_RESIZE: u32 = 21;
    pub const S_RESIZE: u32 = 22;
    pub const SE_RESIZE: u32 = 23;
    pub const SW_RESIZE: u32 = 24;
    pub const W_RESIZE: u32 = 25;
}

/// What the window layer tells the application. Each layer says only some of these (Wayland has keymaps and a frame
/// callback, Windows has translated key presses).
#[derive(Clone, Debug)]
#[cfg_attr(windows, allow(dead_code))]
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
    /// A key press already turned into what it means (Windows: the layout, dead keys and modifiers are the system's, and
    /// it repeats a held key itself, as presses with `repeat` set: there is no client-side repeat and no release to wait for).
    #[cfg_attr(target_os = "linux", allow(dead_code))]
    Press { key: Key, repeat: bool },
}

/// Modifiers that change what a key sends to a terminal.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Mods {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

/// What a key press means.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Key {
    pub sym: u32,
    /// The text the key types, after dead keys and compose; empty for keys that type nothing.
    pub text: String,
    pub mods: Mods,
}

/// The window layer the app runs on.
#[cfg(target_os = "linux")]
pub use crate::wl::Wl as Plat;
#[cfg(windows)]
pub use crate::win::Win as Plat;

/// What the PTY threads signal to wake the app's loop out of its wait: an `eventfd` on Linux (polled together with the
/// compositor's socket), an auto-reset event on Windows (waited on together with the message queue).
#[cfg(target_os = "linux")]
pub struct Wake(std::os::fd::OwnedFd);

#[cfg(target_os = "linux")]
impl Wake {
    pub fn new() -> std::io::Result<Wake> {
        use rustix::event::{EventfdFlags, eventfd};
        Ok(Wake(eventfd(0, EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK)?))
    }

    /// Makes the wait return; any number of signals before it count as one.
    pub fn signal(&self) {
        let _ = rustix::io::write(&self.0, &1u64.to_ne_bytes());
    }

    /// Takes the signal back, so the next wait blocks again.
    pub fn clear(&self) {
        let _ = rustix::io::read(&self.0, &mut [0u8; 8]);
    }
}

#[cfg(target_os = "linux")]
impl std::os::fd::AsFd for Wake {
    fn as_fd(&self) -> std::os::fd::BorrowedFd<'_> {
        self.0.as_fd()
    }
}

#[cfg(windows)]
pub struct Wake(pub(crate) windows_sys::Win32::Foundation::HANDLE);

// SAFETY: an event handle is a kernel object that any thread may signal and wait on.
#[cfg(windows)]
unsafe impl Send for Wake {}
#[cfg(windows)]
unsafe impl Sync for Wake {}

#[cfg(windows)]
impl Wake {
    pub fn new() -> std::io::Result<Wake> {
        use windows_sys::Win32::System::Threading::CreateEventW;
        // SAFETY: no security attributes and no name; the handle is ours and closed in `Drop`.
        let h = unsafe { CreateEventW(std::ptr::null(), 0, 0, std::ptr::null()) };
        if h.is_null() { Err(std::io::Error::last_os_error()) } else { Ok(Wake(h)) }
    }

    pub fn signal(&self) {
        // SAFETY: `self.0` is a live event handle.
        unsafe { windows_sys::Win32::System::Threading::SetEvent(self.0) };
    }

    pub fn clear(&self) {
        // SAFETY: `self.0` is a live event handle.
        unsafe { windows_sys::Win32::System::Threading::ResetEvent(self.0) };
    }
}

#[cfg(windows)]
impl Drop for Wake {
    fn drop(&mut self) {
        // SAFETY: the handle is ours and nothing waits on it any more.
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0) };
    }
}
