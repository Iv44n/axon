---
type: decision
status: superseded-by-0015
updated: 2026-11-23
related: [0009-data-oriented-no-caches.md, 0011-software-render-wl-shm.md, ../../.claude/skills/axon-design-language/SKILL.md]
---
# 0012 · Black-and-white glass look and client-drawn window chrome

**Decision (requested by the user on 2026-11-22):** the window is a rounded, translucent black "glass"
pane with a title bar and three round buttons (close, minimize, maximize), drawn by axon itself on
the CPU renderer ([0011](0011-software-render-wl-shm.md)). Black and white: the chrome and the default
text, cursor and selection are neutral; the eight ANSI hues stay, muted, because programs ask for them.
It replaces the neon look (cyan/violet accent, no title bar, square window) of the first design language.

**How (all in `chrome.rs`, nothing stored):**
- Every background pixel is a pure function of position, window size and `Ui` (hover, pressed, focused,
  maximized): rounded rectangle by signed distance (anti-aliased, alpha 0 outside), body `opacity` alpha,
  white sheen from the top, a 1 px rim that fades toward the bottom, a lighter title bar with a hairline,
  buttons as discs with glyphs that appear when the pointer is over the group. No texture, mask or cache.
- Away from the edges the frame only varies with `y`, so rows are filled with one value; the band near an
  edge and the left of the title bar are computed per pixel.
- The title bar repaints alone when `Ui` or the title changes (`Soft::set_ui`, `set_title`); a change of
  `maximized` (square corners) repaints the frame. Hit testing (`chrome::hit`) serves the buttons, moving
  by the title bar, double click to maximize, and resize edges, through `xdg_toplevel` requests.

**Why client-side:** the compositor cannot round or style a window it does not own, and server-side
decorations would add a protocol and a look we do not control. Cost: the grid starts 45 px lower and the
title text needs glyph tiles (already baked for the grid).

**Not possible without a protocol:** real blur behind the glass. Wayland has no portable blur; the glass is
a translucent fill plus light, so it reads best over a calm background. `ext-background-effect-v1` is the
future route where a compositor offers it (GNOME does not today).

**Limits today:** the `wgpu` path (frozen until stage E) keeps square corners and no title bar; only its
palette and background colours are black and white. Window shadows are the compositor's.

**Revisit if:** a compositor shows a rectangular shadow through the transparent corners (then draw our own
shadow margin), or users want the colours fully grayscale (one array in `palette.rs`).
