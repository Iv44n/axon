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
