---
type: task
status: in-progress
updated: 2026-11-24
related: [phases.md]
---
# Next steps

In priority order. Work on the first unchecked task; when done, tick it and add the date.
If a task needs design notes, create `plan/<task>.md` and link it from its line.

- [ ] 1. First full measurement against Alacritty, Ghostty and foot → [protocol](../performance/protocol.md); results in [measurements](../performance/measurements.md). axon alone measured 2026-11-22 (idle CPU, RAM, 100 MB `cat`); the other terminals are not installed here; startup and latency pending
- [x] 2. Per-line damage tracking with `Term::damage` and partial `write_buffer` → [0005](../decisions/0005-damage-tracking.md) (done 2026-11-22)
- [x] 3a. Settings screen for the font settings: gear / `Ctrl+,`, live preview, Save keeps the file's comments ([0017](../decisions/0017-settings-screen.md), done 2026-11-23); since 2026-11-24 a docked, non-modal panel that saves as you change ([0026](../decisions/0026-docked-settings-panel.md))
- [~] 3. `config.rs` with an own minimal parser (no `serde`/`toml`, [0010](../decisions/0010-own-implementations.md)): done 2026-11-23 for fonts and opacity ([0016](../decisions/0016-config-file-font-settings.md)); colours, shortcuts and themes still to come → `axon-architecture` skill, step 1
- [x] 4. `view.rs`: move `build()` out of `main.rs` (done 2026-11-22, shared by both renderers)
- [x] 5. `session.rs`: `Term` and the PTY thread behind `session::spawn` (done 2026-11-22; the `Proxy` stays per backend)
- [x] 6. xterm 1000/1002/1003 mouse reporting with SGR, UTF-8 and classic encodings: `mouse.rs`, wired in `wlapp.rs` (done 2026-11-23; `winit` fallback has none) → `axon-extend` skill
- [x] 6b. Window blur setting (`[window] blur`), a request to the compositor, [0019](../decisions/0019-window-blur.md)
- [~] 6a. Input customization (Warp's *Input* settings): phase 1, the prompt chips and the Input tab, done 2026-11-24 ([0018](../decisions/0018-input-bar.md)); phase 2, an own line editor (*Input type* "Warp") with OSC 133 shell integration, only when asked; `[input] autosuggest` is already read, saved and shown in the panel ([0026](../decisions/0026-docked-settings-panel.md)), the editor will use it
- [ ] 7. Decide whether inline images and a blinking cursor are requirements → [risks](../risks/INDEX.md)
- [~] 8. Tabs and panes: windows and the side panel done 2026-11-24 ([0023](../decisions/0023-windows-and-side-panel.md)), split panes done 2026-11-24 ([0024](../decisions/0024-split-panes.md)), the overlay kit (fields, filtered lists, menus, toasts, the modal frame and the capture model) done 2026-11-24 ([0025](../decisions/0025-overlay-kit.md)), docked settings with a Windows page done 2026-11-24 ([0026](../decisions/0026-docked-settings-panel.md)), the command palette on `Ctrl+K` done 2026-11-24 ([0027](../decisions/0027-command-palette.md); its History heading waits for the own editor), the overview of windows (`Alt+O`) done 2026-11-24 ([0028](../decisions/0028-overview.md)), the group dialog, the context menus of windows, groups, panes and the list, inline rename and the toasts with *Undo* for a close done 2026-11-24 ([0029](../decisions/0029-group-dialog-menus-rename-toasts.md)), drag and drop of windows and groups, groups in the tabs and the animated status icons done 2026-11-24 ([0030](../decisions/0030-drag-groups-status.md)); to do after the `terminal (2).html` reference: pane resizing by dragging and a pane menu, then the own editor and blocks (task 6a phase 2)
- [x] 9. Idle RAM target: the user accepted [0011](../decisions/0011-software-render-wl-shm.md) (software rendering on `wl_shm`; the graphics stack alone maps ~94 MB, the spike reached 8.7 MB without it) (done 2026-11-22)
- [ ] 10. Remove the ~600 ms UI-thread stall on the first CJK glyph (and its ~300 MB) → [risk](../risks/fallback-fonts.md). Added after the first measurement; the own font reader (task 12) removes it
- [x] 11. Cell buffer redesign: fixed stride, flags in the tile bits, flat glyph table, no row cache → [0009](../decisions/0009-data-oriented-no-caches.md) (done 2026-11-22)
- [ ] 12. Own font reader and rasterizer (replaces `fontdue`); the `wl_shm` spike is done (2026-11-22) → [own-implementations](own-implementations.md) items 0 and 1
- [ ] 13. Own PTY and terminal core (VT parser, SoA grid), with `alacritty_terminal` as test oracle → [own-implementations](own-implementations.md) items 2 and 3
- [ ] 14. Swapchain format: `Rgba16Float` is chosen by accident (8 bytes per pixel); try `Bgra8Unorm` and check the look → [measurements](../performance/measurements.md). Moot: 0011 is accepted, the swapchain goes away with task 15
- [ ] 15. (stages A, B and C done: `soft.rs`, `wl.rs`, `wlapp.rs`, glass chrome [0012](../decisions/0012-glass-chrome-black-white.md), keyboard and clipboard, `wl` is the default [0013](../decisions/0013-wayland-default-xkb-dlopen.md), own shadow and the design board [0014](../decisions/0014-client-shadow-design-board.md); next is D, HiDPI, and E, deleting `winit`/`wgpu` once X11 has its own client) Own Wayland client (registry, `xdg-shell`, `wl_shm`, input) and CPU cell renderer next to the wgpu one, pixel-compared on the fixed scenes; then delete `winit`/`wgpu` → [software-renderer](software-renderer.md), [0011](../decisions/0011-software-render-wl-shm.md)
