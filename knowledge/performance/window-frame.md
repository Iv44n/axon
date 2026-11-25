---
type: log
status: current
updated: 2026-11-23
related: [measurements.md, software-renderer.md, ../decisions/interface/0014-client-shadow-design-board.md]
---
# Window frame measurements

The frame around the grid (`chrome.rs`): what it costs and how its pictures are checked. Renderer
measurements are in [software-renderer](software-renderer.md). Newest first.

- **Continuous glass** (2026-11-23, [0015](../decisions/interface/0015-continuous-glass.md)): binary 7,366,064 -> 7,366,560 B,
  idle RSS 13 MB, 0 ticks, flood 1.75 s and 39 MB. Less work per pixel (no sheen, rim gradient or edge light), so
  nothing slower. A first run on a loaded machine gave 3.36 s and 1 tick: discarded, rerun after a reboot with
  nothing else running. 28 tests pass; new ones pin the uniform edge, the title bar being the same glass as the
  grid, and background-coloured cells being glass.
- **Own shadow and opacity 0.88** (2026-11-23, [0014](../decisions/interface/0014-client-shadow-design-board.md)): binary
  7,362,960 -> 7,366,064 B; idle RSS 14 MB, 0 ticks; flood 1.78 s and 40 MB afterwards (39 before: the buffer
  is larger by the margin, 1000 x 640 -> 1052 x 692 pixels, +0.35 MB). The margin is painted on full repaints
  only; row repaints write the window's columns. Checked: `incremental_repaint_matches_a_full_one` (7 steps:
  hover, press, title, focus, maximize with a new buffer full of garbage, clear, colour; 0 differing pixels),
  chrome tests for the shadow (black, cast downward, zero at the buffer edge, rows constant between the side
  bands), and a 4 s run against Mutter with the protocol trace: `set_window_geometry`, the input region and
  `xdg_wm_base` v2 raise no error. Not seen on the real desktop by the agent (no screen capture on GNOME
  without a portal prompt): the user checks placement, tiling and resizing from the shadow band.
- **Design board frames** (`scripts/design.sh`, ~10 s): 9 scenes at 900 x 520 plus margin, 1.6 MB of PNG
  with the own encoder (per-row filter, fixed-Huffman deflate with runs); Python's `zlib` inflates every
  frame back to `h * (4w + 1)` bytes on each run where Python exists; PIL decoded all of them once (2026-11-23).
