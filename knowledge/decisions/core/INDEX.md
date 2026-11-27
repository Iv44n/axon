# Decisions: core

The stack, the renderer, the data layout and the rule of owning what we ship. Never edited: superseded by a new ADR.

- [0001 · Native, no WebView](0001-native-no-webview.md) — winit + wgpu + alacritty_terminal; no Tauri, Electron or WebView.
- [0002 · Text with fontdue + own atlas](0002-text-fontdue-atlas.md) — superseded by 0010; still no glyphon/cosmic-text.
- [0003 · Render on the UI thread](0003-render-on-ui-thread.md) — no dedicated GPU thread.
- [0004 · Term shared via FairMutex](0004-term-fairmutex.md) — no channels carrying grid copies.
- [0005 · Damage tracking, not dirty rectangles](0005-damage-tracking.md) — the savings are on the CPU; implemented 2026-11-22.
- [0006 · Modules before crates](0006-modules-before-crates.md) — four-crate workspace only once there is a second consumer.
- [0007 · No SQLite, egui or Tokio](0007-no-sqlite-egui-tokio.md) — no use case pays for their cost.
- [0008 · Idle RAM < 50 MB](0008-idle-ram-50mb.md) — superseded by 0011; target lowered from 80 MB; measure RSS and PSS.
- [0009 · Data-oriented design; no caches](0009-data-oriented-no-caches.md) — flat arrays, indices, packed bits; redesign instead of caching.
- [0010 · Own implementations from scratch](0010-own-implementations.md) — dependencies are scaffolding; fonts, terminal core and PTY first.
- [0011 · Software rendering into wl_shm](0011-software-render-wl-shm.md) — own Wayland client and CPU renderer; 8.7 MB idle against 191 MB with wgpu.
- [0013 · Wayland default, xkb via dlopen](0013-wayland-default-xkb-dlopen.md) — `wl` is the default backend; `libxkbcommon` loaded at run time; `winit` fallback.
- [0039 · Windows port](0039-windows-port.md) — own Win32 layered window behind `plat.rs`, ConPTY shell, self-test on CI; no blur or HiDPI yet.
- [0041 · Paint outside the terminal's lock](0041-paint-outside-the-terminal-lock.md) — the lock covers reading cells; flood 13 % faster, lock held 30x less.
- [0049 · No `unsafe` code](0049-no-unsafe-code.md) — `unsafe_code = "deny"`; only `xkb.rs`, `win.rs` and the pixels' mapping allowed; own time zone reader.
