---
type: decision
status: accepted
updated: 2026-11-24
related: [0021-ui-refactor-terminal-reference.md, 0009-data-oriented-no-caches.md, 0018-input-bar.md]
---
# 0023 · Windows (tabs) and the side panel

**Decision:** the app holds several terminal sessions, shown one at a time, after the `terminal (1).html` reference (windows
and groups in a panel at the side, or tabs at the top). First slice: windows and the panel; groups, split panes, the
overview and the command palette follow (see [next steps](../plan/next-steps.md), task 8).

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
printed, or the pointer moved over another part). Rows are 28 px with the reference's look (selected 7.5 % white, hover
4.5 %, a cross on hover, a dot for unread output). The text is the grid's own font at 0.85: a proportional UI font would be
5 MB of resident outlines for a panel that is always on screen (idle RSS is the product).

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
testing; `Soft::title_painted` says when the bar was repainted so the tabs are drawn again over it.

**Not yet:** groups, split panes, the overview, the palette, window titles that can be renamed, a status that says a
command is running or failed.
