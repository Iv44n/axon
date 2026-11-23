---
type: task
status: in-progress
updated: 2026-11-22
related: [next-steps.md, ../decisions/0009-data-oriented-no-caches.md, ../decisions/0010-own-implementations.md, ../performance/measurements.md]
---
# Own implementations: order and designs

Order = payoff per effort, with the data we have. Before writing each one, answer the questions
in the `axon-data-oriented-design` skill; measure before and after; log it in
[measurements](../performance/measurements.md). Nothing here is started except where marked.

## 0. Spike: what does the GPU stack cost? (decides item 4) - done 2026-11-22
The Vulkan loader plus Mesa/NVIDIA libraries map ~94 MB (RSS floor, measured). A throwaway window
drew the same kind of scene into a `wl_shm` buffer with the CPU: **8.7 MB idle RSS** at 1000x640,
14 MB at 1080p (axon today: 191 MB). The 50 MB target ([0008](../decisions/0008-idle-ram-50mb.md))
is reachable only without the GPU stack, so item 4 becomes an own Wayland client with a software
renderer: accepted in [0011](../decisions/0011-software-render-wl-shm.md); stages in [software-renderer](software-renderer.md).
Numbers and caveats: [measurements](../performance/measurements.md).

## 1. Font reader and rasterizer (replaces `fontdue`)
- **Payoff:** anon -45 MB (Cascadia NF) and -314 MB plus a ~600 ms stall (CJK), measured.
- **Layout:** memory-map the file (file-backed, shared, no anon). `cmap` formats 4 and 12 into a
  flat `u16` glyph-id array per 256-code-point page. `loca`/`glyf`/`hmtx` read in place.
  Quadratic outlines to 8-bit scanline coverage, written straight into the atlas.
- **Tile = glyph id** for the primary font, baked eagerly (13.5k glyphs x 171 B = 2.3 MB; measure
  the bake time). No lookup table, no miss path: not a cache.
- **Open question:** huge sparse fallback fonts (CJK: 65k glyphs). Eager bake is too big, lazy bake
  is a cache. Options: accept lazy baking for the long tail with its own ADR, or compute coverage
  from outlines on the GPU with no atlas.
- **Risk:** Noto CJK is CFF-flavoured (needs a Type 2 charstring interpreter); start with `glyf` fonts.

## 2. PTY (replaces `alacritty_terminal::tty` and its event loop)
`openpty`/`fork`/`exec` and an `epoll` read loop through `libc` or `rustix`, reading straight into
the parser's input buffer. Small; do it together with item 3.

## 3. Terminal core: VT parser and grid (replaces `alacritty_terminal`)
- **Parser:** table-driven state machine (`[u8; 256]` transition tables per state), no allocation.
- **Grid:** one ring of rows in flat arrays, scrollback in the same ring. Wide-character spacers and
  attributes as bits in the style word. Hyperlinks and zero-width extras in a side pool indexed by
  `u32`, not `Option<Arc>` per cell. Measured today: 10k lines of scrollback ~25 MB.
- **Open question (access pattern):** the parser writes characters and style, the renderer reads
  both with the colors. Decide between the 12-byte GPU `Cell` as the grid cell and separate arrays.
- **Damage:** one bit per row set by the writer; no diffing.
- **Risk:** conformance. Keep `alacritty_terminal` as the oracle: feed identical byte streams to
  both and compare the grids; run vttest and esctest.

## 4. Window and GPU stack (`winit`, `wgpu`)
Item 0 measured the outcomes: `wgpu` with one Vulkan driver is still 119 MB; a window with a CPU
renderer on `wl_shm` is 9-14 MB. Decided: an own Wayland client (xdg-shell, `wl_shm`) and a CPU cell
renderer, X11 later (MIT-SHM) ([0011](../decisions/0011-software-render-wl-shm.md)). Keep `winit` and
`wgpu` until the software path matches the pixels, then delete them.

## Not on this list
Config parser: a minimal own parser, not `serde` + `toml` (task 3 in next-steps). Clipboard
(`arboard`): replace with the Wayland data-control protocol when item 4 is decided.
