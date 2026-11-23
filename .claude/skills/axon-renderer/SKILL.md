---
name: axon-renderer
description: Internals of axon's renderer (wgpu 30) - glyph atlas, cell buffer format, uniforms, WGSL shader, special tiles and API gotchas. Use it when modifying gpu.rs or shader.wgsl, or adding visual effects.
---

# Renderer (`src/gpu.rs`, `src/shader.wgsl`; CPU twin in `src/soft.rs`)

**Direction ([0011](../../../knowledge/decisions/0011-software-render-wl-shm.md)):** the GPU path is going away. `soft.rs` already paints the
same `Cell` rows on the CPU into an `Argb8888` framebuffer (premultiplied) and matches `shader.wgsl` to within 1/255 on the fixed scenes.
`AXON_BACKEND=wl` runs it on the own Wayland client (`wl.rs`). Until `gpu.rs` is deleted, any look change (colors, overlays, background, blending) must be made in **both** `shader.wgsl` and `soft.rs`,
and checked with the pixel diff in `axon-data-oriented-design`. The framebuffer is the retained state: `Soft::write_rows` repaints only
the damaged pixel rows, so there is no cell buffer.

## Model
- Font → `fontdue` rasterizes into bitmaps **exactly one cell in size** (`cell_w × cell_h`).
- `R8Unorm` 2048×2048 atlas; glyphs are cell-sized *tiles*: `tile → (tile % tiles_per_row, tile / tiles_per_row)`.
  The tile index has 16 bits (`MAX_TILES`). `GlyphTable` maps (code point, style) to its tile: a two-level flat
  table, no hashing; 0 = not baked.
- **One fixed cell per grid position**: `Cell { tf: u32, fg: u32, bg: u32 }`, 12 bytes. The cell buffer is a flat
  array indexed by `row * cols + col` (the shader gets the position from the instance index, nothing stores it).
  `tf`: tile in bits 0-15; flags above: `F_UNDER` (16), `F_STRIKE` (17), cursor shape at bits 18-19
  (`F_BEAM`, `F_BAR`, `F_BOX`). Colors are RGBA8 packed with `pack`. Constants are mirrored in `shader.wgsl`.
- A cell with `tf == 0` and `bg.a == 0` is empty: the vertex shader emits a degenerate triangle.
- 6-vertex quad generated in the shader (`QUAD[vi]`), `draw(0..6, 0..rows*cols)`.
- Fragment: `textureLoad` (no sampler, pixel-exact) → `mix(bg, fg, coverage)`; **premultiplied** output,
  blend `One / OneMinusSrcAlpha`. Underline, strikeout and cursor are `overlay()` calls over that result, in that order.
  The cursor color is the `cursor` uniform (`Gpu.cursor_color`).
- Background: full-screen triangle (`vs_bg`/`fs_bg`) with gradient, neon border and `opacity`.
  Cells with the default background emit `bg.a = 0` so this background shows through.

## Writing cells (damage tracking)
`begin_frame(rows, cols) -> bool` (true = write every row: first frame, grid size changed, or glyph tiles
reassigned). `write_rows(first, count, |gpu, cells| ..)` hands the closure `count * cols` zeroed cells, which
it fills; they are uploaded at `first * cols * 12` bytes. `Gpu.stage` is scratch that is dead after each call;
there is no CPU-side copy of the buffer. `build()` in `main.rs` picks the rows: `Term::damage()` plus the
previous and current cursor rows and selection rows (not part of `Term`'s damage), then writes each run of
consecutive damaged rows. Scrolling is Full damage in `alacritty_terminal`, so output floods write every row.

## Reserved tiles
`0 BLANK, 1 BEAM, 2 BAR (underline cursor), 3 BOX (hollow cursor), 4 UNDER, 5 STRIKE`; glyphs start at 6.
These are only sampled by the shader for the `F_*` flags; they never appear as a cell's tile.

## Wide characters
`glyph(c, bold, wide=true)` reserves 2 consecutive tiles (left/right half). `build_rows()` stores `right_half` and uses it
in the following `WIDE_CHAR_SPACER` cell.

## Bold / fallback
Synthesized bold (1 px smear). Fallback fonts are the `FALLBACKS` table: loaded on first use, each with optional code point `ranges` (the CJK font is only consulted for CJK); glyphs are centered horizontally when they come from a fallback.
With variable fonts, `fontdue` uses the default instance (Regular).

## Scale / size change
`set_font_px` recomputes the font metrics (no file is re-read), clears the glyph table and the atlas, re-bakes the special tiles and marks the cell buffer stale (`begin_frame` returns true). Then `relayout()` in `main.rs`.

## wgpu 30 gotchas (different from older versions)
- `Instance::new(InstanceDescriptor::new_with_display_handle(Box::new(event_loop.owned_display_handle())))`.
- `RequestAdapterOptions.apply_limit_buckets` and `SurfaceConfiguration.color_space` are required fields.
- `get_current_texture()` returns the `CurrentSurfaceTexture` enum; present with `queue.present(frame)`.
- `VertexState.buffers: &[Option<VertexBufferLayout>]`; pipelines use `multiview_mask`; layouts use `Option<&BindGroupLayout>` + `immediate_size`.
- A **non-sRGB** format is chosen so blending happens in the palette's color space as-is; if that changes, convert in the shader.
- `CompositeAlphaMode::PreMultiplied` if available; otherwise `opacity` is forced to 1.0.

## Adding an effect
1. Can it be done in `fs_bg` (per-pixel cost, one pass)? Do it there.
2. Does it need per-cell state? Use a spare flag bit of `Cell.tf` (bits 20-31 are free) before adding a field; a new field changes the stride and `vertex_attr_array!`.
3. Does it need animation? Mind the idle budget: redraw only while the animation lasts, then go back to `Wait`.
4. Measure with the `axon-performance` skill.

## Known quirk (kept on purpose)
`fs_cell` multiplies the color by the alpha it already derived from coverage, and `overlay()` squares the
coverage, so antialiased edges are darker and text looks thinner than a plain premultiplied blend. The
refactor to the cell buffer preserved it byte for byte. Changing it is a visual decision (see
`axon-design-language`), not a cleanup.
