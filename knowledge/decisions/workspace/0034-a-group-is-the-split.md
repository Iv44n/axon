---
type: decision
status: accepted
updated: 2026-11-25
related: [0024-split-panes.md, 0033-windows-feature-set-after-the-merge.md, 0023-windows-and-side-panel.md, 0029-group-dialog-menus-rename-toasts.md, ../core/0009-data-oriented-no-caches.md]
---
# 0034 · A group is the split: its windows are the panes

**Decision:** the windows of a group are shown side by side in the grid area, as the panes of a tab are in Warp. A window has
exactly one terminal and is never split inside itself. `Ctrl+Space`, `Alt+\`, `Alt+-` and the palette's split entries add one more
window to the group of the window on screen, right after it, starting in the same directory, and give it the focus; a window in
no group is put in a new group first (named after its folder, the next colour, as the dialog suggests them), so a split always
has a group to be. Supersedes 0024's "panes inside a window"; its layout arithmetic, renderer and input rules stay.

**What the user sees.** Going to any window of a group (a row, a tab, `Alt+1..9`, a click on a pane) shows the whole group; the
window on screen has the accent ring, the others are dimmer with a hollow cursor, and a click or `Alt+,` / `Alt+.` moves the focus
(the focus is the window on screen, so the title bar, the status bar and the keyboard follow it). The panel and the tabs list the
windows as before, one row each; the group's header counts them. A shell that exits, or the cross at a pane's corner or its row, closes
that window with the *Undo* toast, and the one before it in the group takes the focus (`groups::after_close_in`). Taking a window out of
the group (menu, `Alt+Shift+G`, a drag) shrinks the split; a group of one is one full window.

**Data (`wlapp.rs`).** Nothing is stored for the view: `Shown { first, grouped, kind, n, focus }` is computed from `wins`, which
`rebuild_side` keeps sorted so a group's windows are one run (`groups::span`). Pane `i` of the view is `locate(i)`: window `first + i`
when grouped, else pane `i` of the window. Every place that used "the panes of the window on screen" (draw, timers, hit test,
selection, wheel, mouse reports, the chips' refresh) goes through that one function. `Group::kind` remembers the last split
(`panes::after_split`); what it means for the windows there are now is `panes::fit_kind`, so a group made by the dialog or a drag
(kind never chosen) gets two columns or the grid. `rebuild_side` also resizes every terminal (`resize_panes`), because joining or
leaving a group changes the box a window's PTY is told. A window out of sight prints into `unread`; one in sight is painted, so
`term_events` tests membership of the run, not `cur`.

**Why.** The user's model, after Warp: the thing you split is what you group. Two ways to get the same picture (panes in a window,
windows in a group) meant a window in a group could be split again, and a group showed its windows one at a time. One concept is
less to learn and removes the pane rows from the panel.

**Rejected:** keeping panes inside windows beside groups (two concepts for one picture; the case "a window with three panes joins
a group" has no good answer); spreading a window's panes into windows when it joins a group (needed only because panes existed);
storing the run's first index and size (a cache that drifts: it is a scan of a short list).

**Left over, on purpose:** `Win` still holds `Vec<Pane>` and the panel still has pane rows (`Part::Pane`, `ClosePane`, the pane menu,
`Cmd::GoPane`). Nothing makes a window with two panes any more, so that code cannot run; removing it is its own change.
**Not done:** dragging to resize a split, a different layout per window of a group, a group view in the overview.

**Revisit if:** a window must be viewed in two places at once, or splits must survive a restart (then the group needs its layout in the saved state).
