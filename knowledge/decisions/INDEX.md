# Decisions (ADR)

One leaf per decision. Never edited: superseded by a new ADR.

- [0001 · Native, no WebView](0001-native-no-webview.md) — winit + wgpu + alacritty_terminal; no Tauri, Electron or WebView.
- [0002 · Text with fontdue + own atlas](0002-text-fontdue-atlas.md) — superseded by 0010; still no glyphon/cosmic-text.
- [0003 · Render on the UI thread](0003-render-on-ui-thread.md) — no dedicated GPU thread.
- [0004 · Term shared via FairMutex](0004-term-fairmutex.md) — no channels carrying grid copies.
- [0005 · Damage tracking, not dirty rectangles](0005-damage-tracking.md) — the savings are on the CPU; implemented 2026-11-22.
- [0006 · Modules before crates](0006-modules-before-crates.md) — 4-crate workspace only once there is a second consumer.
- [0007 · No SQLite, egui or Tokio](0007-no-sqlite-egui-tokio.md) — no use case pays for their cost.
- [0008 · Idle RAM < 50 MB](0008-idle-ram-50mb.md) — superseded by 0011; target lowered from 80 MB; measure RSS and PSS.
- [0009 · Data-oriented design; no caches](0009-data-oriented-no-caches.md) — flat arrays, indices, packed bits; redesign instead of caching.
- [0010 · Own implementations from scratch](0010-own-implementations.md) — dependencies are scaffolding; fonts, terminal core and PTY first.
- [0011 · Software rendering into wl_shm](0011-software-render-wl-shm.md) — accepted 2026-11-22: own Wayland client + CPU renderer; 8.7 MB idle vs 191 MB with wgpu.
- [0012 · Black-and-white glass chrome](0012-glass-chrome-black-white.md) — superseded by 0015 (its look); CPU-drawn chrome and round buttons stand.
- [0013 · Wayland default, xkb via dlopen](0013-wayland-default-xkb-dlopen.md) — accepted 2026-11-23: `wl` is the default backend; `libxkbcommon` loaded at run time; fallback to `winit`.
- [0014 · Own shadow, design board](0014-client-shadow-design-board.md) — accepted 2026-11-23: shadow in a margin (`set_window_geometry`, input region); visual changes judged on real frames; opacity 0.88.
- [0017 · Settings screen](0017-settings-screen.md) — accepted 2026-11-23: gear / `Ctrl+,` opens a live panel over the grid; Save writes the file keeping comments; no state when closed.
- [0016 · Config file, font settings](0016-config-file-font-settings.md) — accepted 2026-11-23: `config.toml` with an own parser; family, size, weight, spacing, line height, opacity; `Ctrl+Shift+R` reloads.
- [0015 · Continuous glass](0015-continuous-glass.md) — accepted 2026-11-23: integrated title bar, no separators, uniform 1 px outline; background-coloured cells are glass.
