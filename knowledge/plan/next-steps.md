---
type: task
status: in-progress
updated: 2026-11-22
related: [phases.md]
---
# Next steps

In priority order. Work on the first unchecked task; when done, tick it and add the date.
If a task needs design notes, create `plan/<task>.md` and link it from its line.

- [ ] 1. First full measurement against Alacritty, Ghostty and foot → [protocol](../performance/protocol.md); results in [measurements](../performance/measurements.md). axon alone measured 2026-11-22 (idle CPU, RAM, 100 MB `cat`); the other terminals are not installed here; startup and latency pending
- [x] 2. Per-line damage tracking with `Term::damage` and partial `write_buffer` → [0005](../decisions/0005-damage-tracking.md) (done 2026-11-22)
- [ ] 3. `config.rs` with an own minimal parser (no `serde`/`toml`, [0010](../decisions/0010-own-implementations.md)) to replace the loose `env::var` calls → `axon-architecture` skill, step 1
- [x] 4. `view.rs`: move `build()` out of `main.rs` (done 2026-11-22, shared by both renderers)
- [x] 5. `session.rs`: `Term` and the PTY thread behind `session::spawn` (done 2026-11-22; the `Proxy` stays per backend)
- [ ] 6. xterm 1000/1006 mouse reporting (closes phase 4) → `axon-extend` skill
- [ ] 7. Decide whether inline images and a blinking cursor are requirements → [risks](../risks/INDEX.md)
- [ ] 8. Tabs and panes: one `Session` per pane; the renderer does not change
- [x] 9. Idle RAM target: the user accepted [0011](../decisions/0011-software-render-wl-shm.md) (software rendering on `wl_shm`; the graphics stack alone maps ~94 MB, the spike reached 8.7 MB without it) (done 2026-11-22)
- [ ] 10. Remove the ~600 ms UI-thread stall on the first CJK glyph (and its ~300 MB) → [risk](../risks/fallback-fonts.md). Added after the first measurement; the own font reader (task 12) removes it
- [x] 11. Cell buffer redesign: fixed stride, flags in the tile bits, flat glyph table, no row cache → [0009](../decisions/0009-data-oriented-no-caches.md) (done 2026-11-22)
- [ ] 12. Own font reader and rasterizer (replaces `fontdue`); the `wl_shm` spike is done (2026-11-22) → [own-implementations](own-implementations.md) items 0 and 1
- [ ] 13. Own PTY and terminal core (VT parser, SoA grid), with `alacritty_terminal` as test oracle → [own-implementations](own-implementations.md) items 2 and 3
- [ ] 14. Swapchain format: `Rgba16Float` is chosen by accident (8 bytes per pixel); try `Bgra8Unorm` and check the look → [measurements](../performance/measurements.md). Moot: 0011 is accepted, the swapchain goes away with task 15
- [ ] 15. (stages A and B done 2026-11-22: `soft.rs`, `wl.rs`, `wlapp.rs`; next is C, input) Own Wayland client (registry, `xdg-shell`, `wl_shm`, input) and CPU cell renderer next to the wgpu one, pixel-compared on the fixed scenes; then delete `winit`/`wgpu` → [software-renderer](software-renderer.md), [0011](../decisions/0011-software-render-wl-shm.md)
