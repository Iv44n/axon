---
type: log
status: current
updated: 2026-11-22
related: [measurements.md, ../decisions/0011-software-render-wl-shm.md, ../plan/software-renderer.md]
---
# Software renderer measurements

Findings for [0011](../decisions/0011-software-render-wl-shm.md); the idle RSS rows are in
[measurements](measurements.md). Newest first.

- **Stage A of the software renderer** ([plan](../plan/software-renderer.md), `src/soft.rs`), 1000x640, 32 rows:
  CPU framebuffer against the GPU swapchain readback on the 5 fixed scenes (block, beam, underline,
  hidden and hollow cursors; every SGR attribute, wide characters, box drawing): 99.0% of bytes are
  identical, the rest differ by exactly 1 (f16 and float against integer rounding), max 1, 0 pixels
  off by more than 2. The refactor that moved `Cell`, fonts and the glyph table out of `gpu.rs` left
  the GPU output byte-identical (0 of 2.56 MB differ, 5 scenes). Incremental repaint equals a full
  repaint from the same `Term` on every frame of two workloads (32 and 11 frames, 0 differing pixels).
  CPU time of `write_rows` including `build_rows`: 30-300 us for a few rows, 2.8-3.9 ms for a full
  repaint (flood or scroll frames), naive integer code, no SIMD. Temporary harness under `/tmp`, not in
  the repo; the tests in `soft.rs` cover the pixel maths only.

- **Experiment 0: the GPU stack is the whole problem** ([0011](../decisions/0011-software-render-wl-shm.md)).
  Throwaway program in `/tmp/axon-spike` (not in the repo): `wayland-client` 0.31 with its pure-Rust
  backend, `xdg_toplevel`, one `memfd` + `wl_shm` buffer (`Argb8888`, premultiplied, opacity 0.93),
  ASCII atlas of JetBrains Mono 15 px baked once with `fontdue` and the parser dropped, same kind of
  grid (12 B per cell), CPU rendering. Idle RSS **8.7 MB** at 1000x640 (axon today: 191 MB), 14 MB at
  1920x1048; idle CPU 0 ticks in 5 s; binary 655 KB, links only libc and libgcc. The window needs
  anon 3.7 MB (allocator leftovers from the bake, the atlas itself is 17 KB), file 1.9 MB, and the
  shm buffer at 4 bytes per pixel: 3 MB at 1000x640, 8 MB at 1080p, 33 MB at 4K.
- **CPU frame cost** (naive: per-pixel integer maths, no SIMD, no damage, full redraw): median 1.9 ms
  at 1000x640, 3.9 ms at 1920x1080, 10.7 ms at 3840x2160. Damage tracking already exists, so a typical
  frame redraws a few rows.
- **Not covered by the spike:** PTY, VT parser, scrollback, input, HiDPI scale, decorations, double
  buffering (GNOME's Mutter releases shm buffers right after copying them; other compositors may not),
  underline/strike/cursor flags, wide characters. Pixels were checked by dumping the buffer to a PNG,
  not by reading the compositor output (GNOME denies screenshots and window introspection).
