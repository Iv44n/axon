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

## B. Own Wayland client, one window
`$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY` unix socket, `SCM_RIGHTS` for fds, objects as indices into a flat
table (no `HashMap`), a hand-written dispatch for the few interfaces: registry, `wl_compositor`,
`wl_shm`, `xdg_wm_base`/`xdg_surface`/`xdg_toplevel`. Present the `Soft` framebuffer from a `memfd`.
Decide first: own wire code (fits [0010](../decisions/0010-own-implementations.md), ~500 lines) or
`wayland-client` as scaffolding for one stage (the spike used it). Idle must stay 0% CPU: block in
`poll`, redraw on `wl_surface.frame`.

## C. Input and clipboard
`wl_seat`, `wl_keyboard` (the keymap is xkb text: parse the subset we need or load `libxkbcommon`),
key repeat (client side), `wl_pointer` (selection, wheel, buttons), cursor shape, and
`wl_data_device_manager` for the clipboard (replaces `arboard`). `input.rs` takes keysyms instead of
`winit` events.

## D. Window behaviour
Resize and the buffer lifecycle (release-wait single buffer, second buffer only if a compositor needs
it), decorations (`xdg-decoration` or none), HiDPI (`wp_fractional_scale_v1` + `wp_viewporter`),
focus, close. Measure idle RSS at 1000x640 and 1080p against the spike (8.7 MB and 14 MB).

## E. Remove the scaffolding
Delete `winit`, `wgpu`, `pollster`, `bytemuck`, `arboard`, `gpu.rs`, `shader.wgsl`; update the
skills (`axon-renderer`, `axon-architecture`) and the README. X11 is a later task (MIT-SHM client).
