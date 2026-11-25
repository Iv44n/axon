---
type: decision
status: accepted
updated: 2026-11-24
related: [0025-overlay-kit.md, 0023-windows-and-side-panel.md, 0024-split-panes.md, 0021-ui-refactor-terminal-reference.md, 0009-data-oriented-no-caches.md, 0022-i18n.md]
---
# 0028 · The overview of windows

**Decision:** `Alt+O` opens the reference's "Vista general de ventanas" (`.overview`, `renderOverview`, `cardEl`,
`sectionEl` in `terminal (2).html`) as an overlay of the kit (0025): over the workspace only (the panel of windows, the
tabs and the title bar beside it stay live), a header (title, `6 windows · 2 groups`, a 220 px filter, *New group*, *New
window*, a cross), a body that scrolls with a section per group (tinted box, dot, name, count, *+ Window*) and one for the
windows in none, each window a 210 px-minimum card (`auto-fill`, 12 px gap, 178 px high), and at the end the dashed
*New group* tile. A card is the last rows of the window's focused pane in the grid's font at 0.75 of its size fading out
towards the top, a foot (status icon, name, unread dot, pane count, `Alt+n` key cap, cross on hover) and the directory
with the branch. The window on screen has the amber border. `overview.rs` is the overlay; `wlapp/windows_overview.rs` is
the app's side.

**Behaviour:** typing filters (name, directory and branch, group name, or the text of any pane's last 30 rows; case
ignored; the section counts stay unfiltered, as in the reference); `Up`/`Down` move by rows through the sections,
`Left`/`Right` by cards while the filter is empty (else they are its caret's), `Tab`/`Shift+Tab` walk and wrap,
`Ctrl+Home/End` first and last, `PageUp/Down` scroll, `Enter` or a click goes to the window and closes the overview,
`Esc` clears the filter and then closes it, `Alt+O` closes, `Alt+N` makes a window (and stays), `Alt+1..9` go to one.
Hover lifts a card 1 px and shows its cross (not on the only window); the cross closes the window and the overview goes
on; *New window* and a group's *+ Window* open a window that does not take the screen (toast, as in the reference); *New
group* and the tile make a group around the selected card. Clicking acts on release over the same target.

**Data (no cache):** the overlay holds a snapshot, `cards: Vec<Card>` and `groups`, which the app rewrites from `wins` and
`groups` before every frame while it is open (`sync_overview`; the strings reuse their capacity), together with the
workspace rectangle, so a window closed by its shell, a resize, the settings docking or a hook's status are in the next
frame. Commands name a window by its number (`Win::id`), never an index (`Cmd`, 8 bits of kind and 24 of number in
`Ran::id`), so a stale click finds nothing. Layout is computed on demand from the width and the counts (`layout` walks
sections and cards and hands each rectangle to a closure; `columns`, `column_width`, `head`, `step`, `thumb` are pure and
tested; nothing is stored). The only per-window text read from the terminal is `term_tail`: under the `FairMutex`, the
last N rows ending at the last row with text or the cursor, copied and dropped.

**Kit changes (all small, none breaks an overlay):** `Out::Act(id)` runs a command and keeps the overlay open (`Run`
closes it); `Overlay::as_any_mut` and `Stack::with::<T>(tag)` give the app the overlay by its type to refresh it;
`Stack::top()`; `overlay::Grid` (`tail`, `text`, `metrics`) with `Overlay::draw_grid` and `Stack::draw_with`, how an
overlay gets the terminal's text and the grid's font at draw time without holding either (`draw` is `draw_with` and
`NoGrid`, so every existing caller is unchanged). The app's `Grid` is `AppGrid` (`Soft::paint_text`). In `wlapp.rs`:
`overlay_owns` gives the overview only the workspace (a press on the panel is the panel's, and a click on a window there
closes the overview: `go_to`), `new_win` is `open_win` plus the switch, `new_group` is `new_group_for(i)`.

**Cost:** a frame with the overview open repaints the grid and paints the overview over it (the palette's rule); the
straight parts of sections and cards are plain fills and only corners go through coverage; cards out of the viewport are
skipped; the previews rasterize glyphs per call (no cache), a screenful is ~4 cards x 6 lines. No timer while it is open
(the fade lasts 0.14 s); a shell that prints wakes the loop and repaints it, so previews are live.

**Deviations from the reference:** no backdrop blur (the backdrop is `--bg` at 98.5 % instead of 94 % + blur so the grid
under it does not read through); weight 500 labels are regular; the running status is a still ring; a card dragged
into a group or onto the tile (0032) has no ghost under the pointer, and *New group* and the tile open the dialog empty, as the
reference's `openGroupDialog({})`, and the chip opens the edit dialog; the reference re-renders every 300 ms, here a
frame is drawn when anything wakes the loop; the preview shows the pane's screen rows, not a log of commands (so a
running job has no `▸ job…` line: the last row is amber instead); a narrow workspace drops the count, then the buttons'
words, then the buttons, where the reference wraps and clips its header; closing a window shows no *Undo* toast.

**Not done:** (drag and drop between groups and the group dialog from the overview came with 0029 and 0032) the context menus from the overview, an
*Undo* for a closed window, keyboard `Delete` to close the selected card, a draggable scrollbar.

**Revisit if:** a blur exists (`ext-background-effect` on GNOME or an own box blur) and the backdrop can go back to 94 %;
the group dialog lands (`Cmd::NewGroup` and the tile open it); windows can be dragged (the tile and sections become
drop targets).
