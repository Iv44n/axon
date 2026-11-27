//! axon — minimalist terminal.
// A Windows build is a window program: no console flashes up when it is started from a shortcut. The commands that print
// (`axon --version`, `axon config show`) attach to the console they were typed in (`win::attach_console`).
#![cfg_attr(windows, windows_subsystem = "windows")]
mod agent;
mod bar;
mod caret;
mod cell;
mod chrome;
mod cmap;
mod commands;
mod config;
mod context;
mod ctxmenu;
mod design;
mod dnd;
#[cfg(target_os = "linux")]
mod fallback;
mod field;
mod font;
#[cfg(target_os = "linux")]
mod gpu;
mod group_dialog;
mod groups;
mod history;
mod hosts;
mod i18n;
mod icons;
mod input;
mod list;
mod menu;
mod mouse;
mod overlay;
mod overlay_demo;
mod overview;
mod paint;
mod panes;
mod palette;
mod picker;
mod plat;
mod restore;
mod session;
mod settings;
mod side;
mod soft;
mod ssh;
mod toast;
mod view;
#[cfg(windows)]
mod win;
#[cfg(target_os = "linux")]
mod wl;
mod wlapp;
#[cfg(target_os = "linux")]
mod xkb;

fn main() {
    #[cfg(windows)]
    win::attach_console();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = config::command(&args) {
        std::process::exit(code);
    }
    // The own window layer: the Wayland client on Linux, the Win32 window on Windows (one `wlapp`).
    #[cfg(windows)]
    if !wlapp::run() {
        std::process::exit(1);
    }
    // On Linux, `winit` serves X11 until that has an own client, and `AXON_BACKEND=winit` forces it.
    #[cfg(target_os = "linux")]
    {
        let backend = std::env::var("AXON_BACKEND")
            .unwrap_or_else(|_| if std::env::var_os("WAYLAND_DISPLAY").is_some() { "wl".into() } else { "winit".into() });
        if backend == "wl" && wlapp::run() {
            return;
        }
        fallback::run();
    }
}
