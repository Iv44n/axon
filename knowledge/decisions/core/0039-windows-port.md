---
type: decision
status: accepted
updated: 2026-11-25
related: [0010-own-implementations.md, 0011-software-render-wl-shm.md, 0013-wayland-default-xkb-dlopen.md, ../distribution/0040-windows-bundle-installer-site.md]
---
# 0039 · Windows runs on the same app through an own Win32 window layer

**Decision:** `wlapp.rs` (the app) runs unchanged on Windows over a small platform seam, `plat.rs`: the neutral `Event`, `Key`,
`Mods`, cursor shapes and `Wake`, and `Plat` = `wl::Wl` on Linux, `win::Win` on Windows. `win.rs` is the Win32 counterpart of
`wl.rs`: one frameless popup window (`WS_POPUP | WS_THICKFRAME`, `WS_EX_LAYERED`) whose pixels are the CPU renderer's premultiplied ARGB
buffer pushed with `UpdateLayeredWindow`, so the glass, the rounded corners and the shadow are the window, as on Wayland; moving,
resizing, maximizing and the grab bands are driven by the mouse messages (`SetWindowPos`), as `wl.rs` does with `xdg_toplevel`; keys
go through `ToUnicodeEx` (dead keys, AltGr) into the same `Key` xkb makes; the clipboard is `CF_UNICODETEXT`; the loop waits on the
message queue and the `Wake` event (`MsgWaitForMultipleObjectsEx`), so idle is 0 % CPU. The shell is `AXON_SHELL`, else `pwsh`,
`powershell`, `%COMSPEC%`, on `alacritty_terminal`'s ConPTY; a PowerShell hook (`context::POWERSHELL_HOOK`, run after the profile)
reports each command, its status and the directory through the title escapes the other shells use (`axon!hook`), plus a new `D`
for the directory, because PowerShell's `cd` does not move the process's. `winit` + `wgpu` (now `fallback.rs`) and the Linux
crates (`arboard`, `pollster`, `rustix`, `wgpu`, `winit`) are Linux-only dependencies. The only crate added is none: `windows-sys`
0.59, declarations only, is the version `alacritty_terminal` already builds for its ConPTY.

**Why:** the app's whole UI is in `wlapp.rs` and the renderer, and neither touches the platform except through `Wl`/`Xkb`, a
surface of a few dozen methods. Writing the other side of that surface is far less than porting the UI, and Windows then gets every
feature Linux has, not a subset. A layered window with per-pixel alpha is the one Win32 window that is a CPU buffer with
transparency and no GPU. It cannot be run here: `cargo check --target x86_64-pc-windows-msvc` proves it compiles, and
`.github/workflows/windows.yml` runs it for real on `windows-latest` with `AXON_SELFTEST=<dir>` (`wlapp/selftest.rs`): the app
types a command into a real PowerShell, reads its output, moves the shell's directory, round-trips the clipboard, maximizes and
restores, idles 3 s (it fails above 150 ms of CPU) and writes a report and a picture of its own window.

**Measured** (the runner, 1024x768, release build, 2026-11-25): the self-test passes; idle CPU 0 ms in 3 s; working set 30 MB.

**Not there on Windows:** background blur (`blur_supported()` is false); display scaling (the process is DPI-unaware, so the system
scales the window and it is blurry above 100 %: the same as Wayland today); prompt chips in PowerShell (the shell does not keep ConPTY's
concealed marks) and the `changes` chip; `[windows] scope` (systemd).

**Rejected:** `winit` + `wgpu` on Windows (the 190 MB stack 0011 left; two renderers to keep); `windows` or `winapi` crates (a
bigger dependency for the same declarations); a system frame with `DwmExtendFrameIntoClientArea` (the app draws its own chrome, and the
frame would be a second one); GDI `BitBlt` without alpha (no glass, no shadow); porting `wlapp.rs` itself to a second set of types.

**Revisit if:** HiDPI is wanted (`WM_DPICHANGED` and a scale in `soft.rs`, for both platforms); Windows 11's backdrop (`DwmSetWindowAttribute`)
is wanted for blur; ConPTY gets pass-through of the marks the chips need.
