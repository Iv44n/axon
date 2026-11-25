---
type: decision
status: accepted
updated: 2026-11-24
related: [../core/0011-software-render-wl-shm.md, 0017-settings-screen.md, 0018-input-bar.md, ../core/0009-data-oriented-no-caches.md]
---
# 0020 · Sharp text: supersampled, vertically hinted, sub-pixel glyphs

**Problem:** text looked pixelated next to the rest of the desktop. There is no WebView, CSS or GPU in the `wl`
path, so the cause was in our own rasterizer. This machine (1920x1080 on 340x190 mm, ~144 DPI, scale 1) runs GNOME with
`font-antialiasing = rgba` and `font-hinting = slight`; every other application draws LCD sub-pixel text with the
horizontal stems on whole pixel rows. We drew grey, unhinted `fontdue` coverage plus a synthesized embolden smear: stems
and flat tops straddled two rows, which reads as soft and "pixelated" at 14-15 px.

**Decision (`font.rs`, `soft.rs`):**
- A glyph is rasterized once at `SS = 6` times the pixel size and area-averaged into the output. Horizontally the output
  has 1 sample per pixel (grey) or 3 (LCD sub-pixels), then the embolden step, then a 5-tap FIR over sub-pixels
  (`LCD_FIR`, FreeType's default filter shape) and a gamma `contrast()` of 0.70.
- Vertical hinting without a hinting engine: `Vmap` snaps five zones of the font (descender, baseline, x-height, cap
  height, ascender) to whole pixel rows and maps the outline piecewise-linearly between them, like FreeType's
  light auto-hinter (vertical only, so advances and shapes do not change). Zones are widened to the 1/`SS` grid so no
  sample row straddling an edge leaks a sliver of ink above a letter.
- Tiles hold 3 bytes per pixel (R, G, B coverage); grey and special tiles replicate the value. `text_px3` blends per
  channel. Fallback fonts (CJK, symbols) are rasterized grey and unhinted: they have no zones.
- `[font] smoothing = "rgb" | "bgr" | "gray"` (default `rgb`, matches the desktop) and `hinting = true`. Both are in the
  config file only; they follow the panel layout, not a taste.
- The settings screen and the prompt chips use `Soft::paint_text` / `UiFont`: a proportional UI font loaded only while the
  screen is open; the grid and the title bar stay in the monospaced font. (Since [0036](0036-the-reference-cleaner.md): Geist,
  Inter, Adwaita Sans, Ubuntu Sans, Noto Sans, Open Sans or DejaVu Sans, held by the window for the title, the panel and the tabs
  too; the chips stay in the grid's font.)

**Cost:** tiles are 3x larger (~0.5 MB at 15 px for the ASCII set); idle `wl` RSS is unchanged at 14 MB; flood 1.70 s
(was 1.74 s before; run-to-run noise). The settings screen open costs ~7 MB more (the proportional font's parsed outlines: Ubuntu
Sans 5.3 MB, Inter 9, DejaVu Sans 19), released on close. `fontdue` parsing everything is the reason; the own font reader
(`own-implementations.md`) removes it.

**Limits:** LCD coverage over translucent glass uses one alpha for the three channels, so colour fringes are slightly
inexact; at the default opacity (0.88+) it is invisible. Bold is synthesized (variable-font axes are ignored). Not
verified on the user's real screen from here, only on the design board frames (`text-plain`, `text-gray`,
`text-unhinted`, `settings-*`).

**Same change, interface:** settings controls fade (`Paint.fade`, `ease`): 0.16 s on open, 0.12 s on hover changes;
frames are requested only while a fade runs, idle stays at 0% CPU. Prompt chips are reordered by dragging in the
settings screen (or Alt+Left/Right on the focused chip): the order is the order of `[input] chips`, so it persists with
the file and is untouched by page changes or context updates.
