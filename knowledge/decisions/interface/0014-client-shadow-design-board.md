---
type: decision
status: accepted
updated: 2026-11-23
related: [0012-glass-chrome-black-white.md, ../core/0011-software-render-wl-shm.md, ../../performance/software-renderer.md]
---
# 0014 · axon draws its own window shadow; the look is designed on a board of real frames

**Decision:** the CPU renderer draws a soft black shadow in a margin around the window, and every visual change
is looked at on `design/board.html`, whose pictures are frames from the real renderer (`src/design.rs`,
`scripts/design.sh`), not mockups.

**Why the shadow:** Mutter draws no shadow for client-decorated windows and offers no blur
(`ext-background-effect` is not advertised, checked 2026-11-23), so the glass window sat flat on the desktop.
The shadow is a pure function of position like the rest of the frame ([0012](0012-glass-chrome-black-white.md)):
spread 26 px, cast 8 px down, cubic falloff to zero inside the margin, plus a 2.5 px contact line that
outlines the window on light desktops.

**How it fits the protocol:** the buffer is the window plus the margin (26 px left and right, 18 above, 34
below); `xdg_surface.set_window_geometry` tells the compositor which part is the window, so placement,
maximizing and tiling ignore the shadow; a `wl_region` input region keeps an 8 px band of it for resizing and
lets every other click through to what is below. Maximized, fullscreen and tiled windows (`xdg_wm_base` v2
states) have no margin. Row repaints leave the margin alone: it changes only on a full repaint.

**Cost:** +3 KB binary; the buffer grows by the margin (+0.35 MB for a 1000 x 640 window, +1 MB RSS after the
100 MB flood); idle RSS unchanged at 14 MB, 0% idle CPU.

**Why the board:** a terminal's look cannot be judged from code or from one screenshot: it depends on the
desktop behind the translucent glass, on focus and hover states and on pixel alignment. The board puts each
frame over several backdrops, zooms to device pixels and compares against a baseline (side by side or split).
Tokens (sizes, palette) are written by the same test, so the board cannot drift from the code. The frames are
build output (git-ignored); `scripts/design.sh` makes them in about 10 s.

**Also decided with it:** default opacity 0.88 instead of 0.80 (`chrome::OPACITY`): without blur, text of a
window behind was readable through ours at 0.80.

**Rejected:**
- *Ask the compositor for a shadow:* GNOME has no server-side decorations for Wayland clients
  (`xdg-decoration` is not advertised).
- *A shadow image or nine-patch:* a stored texture, against [0009](../core/0009-data-oriented-no-caches.md); the
  function costs a few `hypot` per pixel in the margin, on full repaints only.
- *HTML or Figma mockups:* they drift from what `chrome.rs` draws; the board shows the real pixels.

**Revisit if:** the compositor offers blur or server-side shadows, or HiDPI (stage D) changes the margin.
