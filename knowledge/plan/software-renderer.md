---
type: task
status: in-progress
updated: 2026-11-22
related: [next-steps.md, own-implementations.md, ../decisions/0011-software-render-wl-shm.md, ../performance/measurements.md]
---
# Software renderer and own Wayland client (task 15)

Stages, each one compiles, is verified and is logged before the next starts. The wgpu path stays
alive until stage E, so the app never stops working.

## A. CPU renderer on the cell grid - `src/soft.rs`
- **Done and verified 2026-11-22:** `soft.rs` paints the same `Cell` rows as `shader.wgsl` into a premultiplied `Argb8888`
  framebuffer. `Cell`, flags and the tile contract moved to `cell.rs`; fonts, the glyph table and the
  special tiles moved to `font.rs`; `Gpu` and `Soft` both implement `cell::Glyphs`, so `build_rows`
  is shared.
- **Design:** the framebuffer is the retained state, so there is no cell buffer: `write_rows` takes
  the damaged rows, fills a scratch `stage`, and paints those pixel rows (background, then cells).
  The background is computed, not stored: constant along a row away from the edges, the glow only
  within 48 px of an edge. Tiles are a flat `Vec<u8>` at `tile * cw * ch`: no atlas geometry.
- **Check (method in `axon-data-oriented-design`):** GPU readback against the CPU framebuffer on the 5
  fixed scenes (max difference 1/255), and every frame of two incremental runs against a full repaint
  (identical). Results in [measurements](../performance/measurements.md), [software-renderer](../performance/software-renderer.md).
- **Still a cache-like thing:** tiles are baked lazily like the GPU atlas; task 12 removes it.

## B. Own Wayland client, one window - `src/wl.rs`, `src/wlapp.rs` (done 2026-11-22)
- **Decided:** own wire code ([0010](../decisions/0010-own-implementations.md)), not `wayland-client`.
  `rustix` is the syscall layer (`sendmsg`/`recvmsg` with `SCM_RIGHTS`, `memfd`, `mmap`, `poll`,
  `eventfd`); it was already in the dependency tree.
- **Done:** `wl.rs` speaks the wire protocol over `$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY`: objects are
  indices into a flat table (`kinds[id]`), events are a `match` on `(kind, opcode)`, requests are built
  into one reused byte buffer. Interfaces: registry, `wl_compositor`, `wl_shm`, `xdg_wm_base`,
  `xdg_surface`, `xdg_toplevel`, `wl_surface` frame callbacks. The window shows the `Soft` framebuffer
  from a `memfd`; a second buffer only exists if the compositor keeps the first one until the next attach.
- **Loop:** `wlapp.rs` runs the session, `View`, `Soft` and `Wl` in one blocking `poll` on the socket
  and an `eventfd` the PTY thread signals; frames are drawn on the frame callback. Select it with
  `AXON_BACKEND=wl`; `winit` stays the default until stage E.
- **Also done:** `view.rs` (grid -> cells for any `Backend`) and `session.rs` (Term + PTY, grid size),
  tasks 4 and 5 in next-steps; the GPU output stayed byte-identical.
- **Missing on purpose:** input, clipboard and mouse (stage C), HiDPI, decorations (stage D).
- **Check:** protocol trace (`AXON_WL_DEBUG=1`) shows the full handshake, releases and frame callbacks
  with no error; resize by maximize/restore cycles; exit when the shell ends; idle 0% CPU. Numbers in
  [software-renderer](../performance/software-renderer.md).

## C. Input and clipboard (done 2026-11-23, [0013](../decisions/0013-wayland-default-xkb-dlopen.md))
- **Done:** `wl_keyboard` (keymap text to `xkb.rs`, raw keys, modifiers, repeat info), client-side key repeat in the
  `poll` loop, keysym encoder in `input.rs` (`encode_key`), shortcuts, wheel (`axis` + `axis_discrete`), selection
  (click, word, line), clipboard through `wl_data_device` (replaces `arboard` on this path), OSC 52 store only (reads are refused on purpose).
  `libxkbcommon` is loaded with `dlopen`; the backend is the default on Wayland and falls back to `winit`.
- **Left:** own keymap parser (then no `libxkbcommon`), `text-input-v3` for IMEs, primary selection, mouse reporting
  (task 6), drag and drop. The plan below is the original scope.
- **Pointer done 2026-11-22** (for the window chrome, [0012](../decisions/0012-glass-chrome-black-white.md)): `wl_seat`
  + `wl_pointer` (enter, leave, motion, button) and `wp_cursor_shape_v1` in `wl.rs`; `wlapp.rs` hit-tests with
  `chrome::hit` for the title-bar buttons, move, double-click maximize and resize edges. Still missing: wheel
  (axis), selection, mouse reporting, and everything below.
`wl_seat`, `wl_keyboard` (the keymap is xkb text: parse the subset we need or load `libxkbcommon`),
key repeat (client side), `wl_pointer` (selection, wheel, buttons), cursor shape, and
`wl_data_device_manager` for the clipboard (replaces `arboard`). `input.rs` takes keysyms instead of
`winit` events.

## D. Window behaviour
- **Chrome done 2026-11-22:** client-side title bar, buttons, rounded corners and glass look (`chrome.rs`), maximize
  and minimize through `xdg_toplevel`. **Shadow done 2026-11-23** ([0014](../decisions/0014-client-shadow-design-board.md)):
  margin outside the window geometry, input region, square and shadowless when maximized, fullscreen or tiled.
  Left: HiDPI (the margin scales with it), the second buffer on a holding compositor.
Resize and the buffer lifecycle (release-wait single buffer, second buffer only if a compositor needs
it), decorations (`xdg-decoration` or none), HiDPI (`wp_fractional_scale_v1` + `wp_viewporter`),
focus, close. Measure idle RSS at 1000x640 and 1080p against the spike (8.7 MB and 14 MB).

## E. Remove the scaffolding
Delete `winit`, `wgpu`, `pollster`, `bytemuck`, `arboard`, `gpu.rs`, `shader.wgsl`; update the
skills (`axon-renderer`, `axon-architecture`) and the README. X11 is a later task (MIT-SHM client).
