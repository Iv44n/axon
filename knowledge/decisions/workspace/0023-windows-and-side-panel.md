---
type: decision
status: accepted
updated: 2026-11-25
related: [../interface/0021-ui-refactor-terminal-reference.md, ../core/0009-data-oriented-no-caches.md, ../interface/0018-input-bar.md, 0033-windows-feature-set-after-the-merge.md]
---
# 0023 · Windows (tabs) and the side panel

**Decision:** the app holds several terminal sessions, shown one at a time, after the `terminal (1).html` reference (windows
and groups in a panel at the side, or tabs at the top). First slice: windows and the panel; split panes
([0024](0024-split-panes.md)), the palette ([0027](0027-command-palette.md)), the overview ([0028](0028-overview.md)), the group
dialog and menus ([0029](0029-group-dialog-menus-rename-toasts.md)) and drag and status ([0030](0030-drag-groups-status.md)) followed;
how they fit together, and the groups, are in [0033](0033-windows-feature-set-after-the-merge.md).

**Data:** `Win` (`wlapp.rs`) owns what belongs to one shell: the `Term` and its PTY, its `Proxy` (event queue, hook queue,
`pending` flag), the `View`, the chips' context and history, the report and flag files, the refresh. `App` has
`wins: Vec<Win>` and `cur: usize`; the grid size, the cell size, the pointer state and the settings are shared. All PTY
threads write to one eventfd (`App::wake`), so the loop still blocks in one `poll`. A window out of sight keeps running; its
output only sets `unread` (a cyan dot in the panel) and the grid is read again when it comes back (`Soft::invalidate`
paints the whole frame from the other `Term`). Closing a window drops its `Win` (the PTY is told to shut down, its files
go); the last one closing quits. A shell ends with two events (`ChildExit`, `Exit`): one close each.

**Layout:** `Soft::set_inset(left, right)` takes room beside the grid; rows, chips and the grid origin follow it, the title
and status bars stay full width. The panel (`side.rs`) is painted in that box, under the title bar and above the status
bar, when a frame is full or something in it changed (`App::side_dirty`: a window opened, closed, selected, retitled,
printed, or the pointer moved over another part). Rows are 30 px with the reference's look (selected 7.5 % white, hover
4.5 %, a cross on hover, a dot for unread output). The text was the grid's own font at 0.85 (a proportional UI font was 5 MB of
resident outlines for a panel that is always on screen); since [0036](../interface/0036-the-reference-cleaner.md) it is the UI font,
narrowed to ~2 MB, as the reference has it.

**Input:** `Alt+N` new, `Alt+W` close, `Alt+1…9` go to, `Alt+[` / `Alt+]` previous / next, `Alt+B` panel on / off; the panel
takes clicks (`Hit::Side`): a row goes to it, the cross closes, `+` opens one. `[windows] tabs = "left" | "right" | "top"`,
`side`, `side_width` (160 to 320). `AXON_WINDOWS=n` starts with `n` windows (measurements and screenshots).

**Files:** each window reports through `$XDG_RUNTIME_DIR/axon-<pid>-<n>.ctx` and `.chips` (the stale-file cleaner reads
the pid before the first `-`).

**Title bar (second slice):** after the reference's top bar, a 28 px button right of the window buttons shows and hides
the panel (`Hit::SideBtn`, `Ui::side`: 0 absent, 1 hidden, 2 shown, lit while the panel is on); the title is the
window's name in bold and its directory dim (`Soft::set_title(name, place)`; a program's own title wins, the shell's
default `user@host: path` does not). With `tabs = "top"` the windows are tabs in the title bar instead
(`side::Layout::Top`: 28 px tabs, 168 px wide down to 96, a `+` after the last), the panel button is gone, and the
title text is not drawn. `side::Layout` is the one place that knows where rows and tabs are, for painting and for hit
testing; `Soft::title_painted` says when the bar was repainted so the tabs are drawn again over it. Where the title, the strip
and the new-group button sit now (the strip scrolls, a button replaces the panel button at the top): [0032](0032-fidelity-second-reference.md).

**Groups (third slice):** `App::groups` and `Win::group`, a header per group in the panel, `Alt+G` and `Alt+Shift+G`: the
model as the code has it now is in [0033](0033-windows-feature-set-after-the-merge.md) (this slice's quick `Alt+G`, which named a
group `group N` at once, became the dialog of [0029](0029-group-dialog-menus-rename-toasts.md)). A new window starts in the
directory of the one on screen.

**Not in this slice, done since:** split panes (0024), the palette (0027), the overview (0028), renaming a window (0029), a
status that says a command is running or failed (0030). **Still not done:** keeping windows, groups or names between runs.
