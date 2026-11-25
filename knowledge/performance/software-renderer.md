---
type: log
status: current
updated: 2026-11-23
related: [measurements.md, ../decisions/core/0011-software-render-wl-shm.md, ../plan/software-renderer.md]
---
# Software renderer measurements

Findings for [0011](../decisions/core/0011-software-render-wl-shm.md); the idle RSS rows are in
[measurements](measurements.md). Newest first.

- **Sharper text and chrome** (2026-11-23, user: "a bit pixelated"; the screen is at scale 1.0, so not compositor
  scaling). Found by zooming headless frames 3-6x: (1) a cell with no background (alpha 0, the normal case) got
  `fg * c^2` at alpha `c`, half the brightness on glyph edges; now plain source-over in `text_px`, `overlay` and
  `shader.wgsl`; (2) coverage raised to 0.70 when a tile is baked (`font.rs`), the sRGB-blend fix for light text;
  (3) button centres moved from pixel borders to pixel centres. Binary 7,362,960 B, idle 14 MB, 0 ticks, flood
  1.81 s, 25 tests pass. The shader got the same formula but was not compared byte for byte with `soft.rs`.

- **Stage C: keyboard, clipboard, wheel, selection** ([0013](../decisions/core/0013-wayland-default-xkb-dlopen.md)): binary
  +30 KB (7,332,888 -> 7,363,488 B), idle RSS 14 MB with JetBrains Mono (was 13: `libxkbcommon` and the compose
  table), 0 ticks idle (a held key makes `poll` wake at the repeat rate, nothing else does), flood 1.78 s and 39 MB.
  Checked: 22 unit tests, among them AltGr+Q = `@` and dead acute + `a` = `a` with an acute on a latam layout through the
  real library, and the xterm encoding of keys. The protocol trace against Mutter shows keymap, enter, modifiers,
  repeat info and the data device selection arriving with no error. Not testable here: real key presses, copy and
  paste (no input injection into GNOME), so those were reviewed, not run.

- **Default font is JetBrains Mono** (2026-11-23, `font.rs` candidate order): `wl` idles at **13 MB** RSS (PSS 9,
  anon 5) against 55 MB with Cascadia Mono NF, and `winit` at 149 MB against 192 MB; 0 ticks idle. After the
  100 MB flood `wl` holds 39 MB (1.80 s) and `winit` 175 MB (1.31 s): 10k lines of scrollback add ~26 MB
  (10k x 108 columns x 24 B per `alacritty_terminal` cell). JetBrains Mono has no Nerd/Powerline icons; a
  Nerd Font stays one `AXON_FONT` away at the price measured above.

- **Glass chrome** ([0012](../decisions/interface/0012-glass-chrome-black-white.md), `src/chrome.rs`): binary +7.6 KB
  (7,325,272 -> 7,332,888 B), idle RSS 55 MB on `wl` with Cascadia (was 54), 0 ticks idle, flood 1.70 s and
  80 MB. The frame is computed per pixel, nothing stored; rows away from the edges are one fill. Checked
  headless in a temp copy: 5 scenes (rest, hover, pressed, unfocused, maximized) composited over light and
  dark backdrops and read by eye, and every chrome change (hover, press, title, focus, maximize) painted
  incrementally equals a full repaint (0 differing pixels of 468k, 6 steps). Pointer input and the window
  requests were traced against Mutter (seat, pointer, cursor shape, enter) but not clicked by a test.

- **Stage B: the app on the own Wayland client** (`AXON_BACKEND=wl`, no `winit`, no `wgpu`, so no Vulkan
  library is ever loaded), idle shell, 1000x640, default build: RSS **54 MB** with Cascadia NF (anon 47:
  `fontdue` keeps ~45 MB of it), **12 MB** with JetBrains Mono, 18 MB with DejaVu Sans Mono; PSS 50, 8 and
  14 MB. The same binary on `winit` + `wgpu`: 191 MB with Cascadia. Idle CPU 0 ticks in 5 s. The protocol
  trace shows the compositor (Mutter) releasing the single shm buffer right after each commit, so no second
  buffer was allocated.
- **100 MB flood** (`cat` of 99.6 MB of 90-column lines): 1.73 s on `wl`, 1.37 s on `winit`; RSS afterwards
  80 MB against 216 MB (10k lines of scrollback are ~26 MB on both: the 72 MB first written here was wrong). The CPU path is 26% slower on a
  flood because every frame is a full repaint (scrolling is Full damage); not a target yet.
- **Resize:** maximize and restore four times (1000x640 <-> 1920x1048, grid 108x34 <-> 210x56): each
  size change reallocates the buffers and repaints, no protocol error. Not covered: other compositors
  (the second-buffer path is written but only exercised by reading), fractional scale, X11.

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

- **Experiment 0: the GPU stack is the whole problem** ([0011](../decisions/core/0011-software-render-wl-shm.md)).
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
