---
type: decision
status: accepted
updated: 2026-11-25
related: [0023-windows-and-side-panel.md, ../interface/0021-ui-refactor-terminal-reference.md, ../core/0009-data-oriented-no-caches.md, ../interface/0018-input-bar.md]
---
# 0024 · Split panes

**Decision:** a window holds several terminal panes, after the `terminal (2).html` reference (`Tab.split`, `removePane`,
`.panes`, `.prow`). `Ctrl+Space` adds one (to the right first, then the automatic grid), `Alt+\` splits in columns, `Alt+-`
in rows, `Alt+,` / `Alt+.` move the focus, `Alt+Shift+W` closes the pane (`Alt+W` still closes the window). A window with
ONE pane is exactly what it was: no border, no padding, the same pixels (the 43 design frames and `incremental_repaint_*`
are unchanged).

**Data:** `Pane` (`wlapp.rs`) is what `Win` was: `Term` and PTY, `Proxy`, `View`, chips context, history, report and flag
files, refresh, title, and the size its PTY was told. `Win` is now `panes: Vec<Pane>`, `focus`, `kind: panes::Kind`,
`unread`, `id`, `group`. A shell that ends closes its pane, and the window only when it was the last. Report files keep
the `axon-<pid>-<n>` naming, one `n` per pane. A new pane starts in the focused pane's directory and goes right after it.

**Layout (`panes.rs`, pure):** `Kind` is `Single | Cols | Rows | Grid`. `rect(kind, n, i, area)` is arithmetic on the
index (nothing stored): 6 px gap and padding, `ceil(sqrt n)` columns for the grid, the last pane spanning the cells the
last row leaves empty, tracks sharing the remainder so they fill the space exactly. `after_split` / `after_remove` /
`cycle` / `focus_after_remove` copy the reference's rules (closing focuses the pane before the closed one). Tests cover
n = 1..12 for every layout, the spans, cycling and hit testing.

**Renderer (`soft.rs`):** `Slot { rect, focus, framed }` per pane; `set_layout` compares with what the framebuffer shows and
repaints the whole frame when it differs (a split, a close, a resize, a focus change), so no pane keeps stale pixels.
`select(i)` says which pane the next `begin_frame` / `paint_rows` / chips are about; pane 0 begins the frame (glass,
title, status, gaps), the others find it decided. Every pane has its own `View` damage state. The border (1 px line, or the
accent at 55 % plus a ring at 18 % just outside for the focused pane, radius 10) is painted on full frames only: rows
never touch it (their background stops 2 px inside). `set_dim(184)` multiplies the alpha of cells, text and chips of a
pane without focus (CSS `opacity: .72` on a layer) and the cursor there is hollow. The cross (20 px, top 7, right 9) of
the pane under the pointer is drawn over the grid rows it covers, which are repainted every frame, so it is never drawn
twice over the same pixels. Chips are laid out in the pane's own text box (`Soft::content_box`).

**Input:** keys go to the focused pane; a press gives a pane the focus (and goes on to the program or the selection);
selection, wheel and mouse reports go to the pane under the pointer, a drag stays in the pane it began in. Each PTY is
resized to its own cols x rows (`relayout`). The panel lists a window's panes (vertical panel only) under it: 26 px rows
`Pane n` (+ ` · title` a program set), status icon (prompt; ring for a running command; red dot for a failed one, from the
shell hook), cross on hover; a click focuses it. A group counts panes (`windowCount`); a tab at the top shows the number.

**Too small:** the reference lets the area scroll below 240 x 150 px. axon has no scrolling area: a split is **refused,
silently** when any pane would get fewer than 20 columns x 5 rows (`panes::MIN_COLS`, `MIN_ROWS`; `Soft::min_pane`). A window
resized below that keeps its panes at 2 x 2 cells at the least, as a lone pane always could.

**Differs from the reference:** no 1.2 % white tint inside a pane; a row says `Pane n` plus the program's title, not the job's
name; the title bar shows no count (a tab at the top does). Since this ADR: the running icon turns ([0030](0030-drag-groups-status.md))
and a pane's row has a menu, go to it and close it ([0029](0029-group-dialog-menus-rename-toasts.md)).

**Not yet:** dragging to resize or reorder panes, saving the layout between runs.
