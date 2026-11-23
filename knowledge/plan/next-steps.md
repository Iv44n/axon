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
- [ ] 4. `view.rs`: move `build()` out of `main.rs`
- [ ] 5. `session.rs`: `Term`, PTY, `Notifier` and `Proxy` in one `Session` type
- [ ] 6. xterm 1000/1006 mouse reporting (closes phase 4) → `axon-extend` skill
- [ ] 7. Decide whether inline images and a blinking cursor are requirements → [risks](../risks/INDEX.md)
- [ ] 8. Tabs and panes: one `Session` per pane; the renderer does not change
- [ ] 9. Decide the idle RAM target: the graphics stack alone maps ~94 MB (RSS) on the reference machine, so 50 MB of RSS is unreachable → [measurements](../performance/measurements.md), [0008](../decisions/0008-idle-ram-50mb.md); needs a new ADR. Added after the first measurement; do it before 3
- [ ] 10. Remove the ~600 ms UI-thread stall on the first CJK glyph (and its ~300 MB) → [risk](../risks/fallback-fonts.md). Added after the first measurement; the own font reader (task 12) removes it
- [x] 11. Cell buffer redesign: fixed stride, flags in the tile bits, flat glyph table, no row cache → [0009](../decisions/0009-data-oriented-no-caches.md) (done 2026-11-22)
- [ ] 12. Spike, then own font reader and rasterizer (replaces `fontdue`) → [own-implementations](own-implementations.md) items 0 and 1
- [ ] 13. Own PTY and terminal core (VT parser, SoA grid), with `alacritty_terminal` as test oracle → [own-implementations](own-implementations.md) items 2 and 3
- [ ] 14. Swapchain format: `Rgba16Float` is chosen by accident (8 bytes per pixel); try `Bgra8Unorm` and check the look → [measurements](../performance/measurements.md)
