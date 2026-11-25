---
type: decision
status: superseded-by-0010
updated: 2026-11-22
related: [../../risks/fallback-fonts.md, 0010-own-implementations.md]
---
# 0002 · Text with fontdue + own atlas, not glyphon

**Superseded by [0010](0010-own-implementations.md) on `fontdue` and the lazily baked atlas.** Still true: no `glyphon`/`cosmic-text`, and the renderer receives cells, not laid-out text.

**Decision:** keep `fontdue` for rasterization and the custom glyph atlas in `gpu.rs`.
The renderer receives cells `(col, row, tile, fg, bg)`, not laid-out text.

**Why:** a terminal is a monospaced grid; it does not need free-form text layout.
`glyphon` pulls in `cosmic-text`, `swash` and `fontdb` (which scans system fonts at startup)
and ties the `wgpu` version to its release schedule. Alacritty and WezTerm also use their own
atlases.

**Rejected:** `glyphon` + `cosmic-text` (original proposal).

**Handled separately:**
- Color emoji → a second RGBA atlas; `Inst` gains a flag.
- Ligatures → a shaper such as `rustybuzz`, to be evaluated as a dependency.
- Fallback fonts → [risks/fallback-fonts](../../risks/fallback-fonts.md).

**Revisit if:** ligatures are requested, or scripts that need complex shaping (Arabic, Indic).
