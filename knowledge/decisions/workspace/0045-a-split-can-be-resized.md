---
type: decision
status: accepted
updated: 2026-11-26
related: [0034-a-group-is-the-split.md, 0024-split-panes.md, 0042-the-session-is-kept-between-runs.md, ../core/0009-data-oriented-no-caches.md]
---
# 0045 · A split's columns and rows can be resized

**Decision:** the windows of a split (a group, ADR 0034) no longer share the grid area evenly for good. Dragging the gap between
two panes moves that boundary; `Alt+Shift+→` / `←` make the window on screen a step wider / narrower, `Alt+Shift+↑` / `↓` taller /
shorter; `Alt+=` (and a double click on a gap, for that direction) evens them again. The palette lists the same five actions in a
split ("agrandar", "achicar", "resize" find them). No pane goes under 20 columns x 5 rows (0024's minimum).

**Data (`panes::Tracks`, on `Group`).** Per axis, one `u16` weight per track (columns, then rows) and the count they were set for:
`n: [u8; 2]`, `w: [[u16; 16]; 2]`, 66 bytes per group. A track's share is its weight over the axis' sum; the weights are the
tracks' lengths in pixels when last set, so a split not resized since is exact to the pixel and a window resized afterwards keeps
the proportions. All zero is the even split, and `rect()` gives with it exactly the pixels of before (`k * avail / count`; a test
checks every layout up to 17 panes). Weights hold only for the count they were set for: a window joining or leaving the split (a
drag, a menu, a close, an *Undo*) evens that axis, instead of laying panes out with sizes meant for others. So no path that changes
a group's members has to know about sizes. The one exception is the split itself: a resized axis that keeps its layout gives
the new window half of the one it splits, the others keep their size. A grid's columns and rows are shared by the panes in
them; the last pane still spans the cells left empty, and a column boundary it covers is not draggable there.

**Pointer.** `panes::edge_at` finds the boundary whose gap (6 px) is under the pointer: not a pane, not the padding, not a gap the
last pane spans. Over it the pointer is `col-resize` / `row-resize` (`wp_cursor_shape_v1` 30 and 31; `IDC_SIZEWE` / `IDC_SIZENS`
on Windows) and a 2 px accent line lights the gap (`Soft::set_handle`, painted on a full frame: the gaps are only painted then,
and the line comes and goes with the pointer, so a repaint then costs nothing while idle); stronger while it drags. The drag keeps
the point of the gap that was grabbed under the pointer (`EdgeDrag::grip`) and holds the group by id.

**Shells hear the new size live.** Each move calls `resize_panes`, which tells a terminal and its PTY only when its box now holds
another number of cells: one `SIGWINCH` per cell crossed, not per pixel, the same path as a window resize. No timer, nothing on
the paint path, no allocation per move beyond what `Term::resize` does when the size really changes. Idle stays at 0 % CPU.

**Kept.** The session file (ADR 0042) writes the sizes after the group's layout, `cols=612,400 rows=...`, only for a resized axis;
an older axon reads the first three fields and ignores the rest, a file from before is even, a damaged list is even.

**Rejected:**
- Boundaries as fractions (one number per gap): a split and a close become shifting arithmetic; weights make "the new track takes
  the default" and "half of this one" one line each.
- Moving sizes along when a window leaves (giving its room to the neighbour): every path that changes members (close, undo, drag,
  menu, ungroup, a shell that ends) would need it; evening is predictable and needs none.
- Resizing the PTYs only when the button comes up: the text would be clipped or float in an empty box during the drag.
- A per-window size (a window that remembers its width): the size belongs to the split's track, which several windows share.

**Revisit if:** users want the room of a closed window to go to its neighbour, or splits nest (a tree instead of a grid).
