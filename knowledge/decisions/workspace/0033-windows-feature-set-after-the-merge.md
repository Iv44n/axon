---
type: decision
status: accepted
updated: 2026-11-25
related: [0023-windows-and-side-panel.md, 0024-split-panes.md, 0027-command-palette.md, 0028-overview.md, 0029-group-dialog-menus-rename-toasts.md, 0030-drag-groups-status.md, 0032-fidelity-second-reference.md]
---
# 0033 · Windows, groups, palette, overview and drag are one feature set

**Decision:** the palette (0027), the overview (0028), the group dialog and menus (0029) and the drag and status icons (0030)
were built on separate branches, and each ADR describes the state of its own branch. From the merge on they are one set: where
one of them says "not yet" about another, this ADR and the code are right, and the model of groups (which 0023 carried) lives here.

**What the merge connected** (each is one call, so the ways in cannot differ):
- *Overview* is listed by the palette (`State::overview` is `true` in `App::open_palette`) and by the empty-list menu
  (`strip_menu(at, true)`); both run `toggle_overview`, as `Alt+O` does.
- *New group* (the palette, `Alt+G`, the empty-list menu, the panel's and the title bar's buttons) opens the dialog
  (`group_dialog_here`). In the overview the button and the tile open it empty and a card dropped on the tile opens it with that
  window ticked (`Cmd::NewGroup(Option<id>)` into `open_group_dialog`); a group's chip opens the edit dialog.
- A card's cross in the overview closes through `close_wins`, so it has the *Undo* toast, and the overview stays open.
- A pane has a menu (`ctxmenu::pane_menu`: go to it, close it) on a right click in its row.
- Dragging a window to another group says so with the menu's sentences (`Str::ToastMoved`, `ToastUngrouped`: the quote marks
  belong to the language), the same `move_to_group` toast whoever moved it.
- The palette reaches windows and groups by id and closing the last pane or the window from it is undoable (0031).

**Groups (the model, from 0023 and as the code has it).** `App::groups: Vec<Group>` (`id`, name, one of eight colours, folded)
and `Win::group`, an index into it; the `id` is what a menu or the dialog holds while indices shift. The windows are kept sorted:
a group's windows together, groups in order, the windows in none last (`rebuild_side`, and `dnd::arrange` after a drop), so
`Alt+1…9` and `Alt+[` / `Alt+]` follow the list. `side::entries` is what the panel shows: a header per group (26 px) with its
windows (30 px rows, 1 px gap, indented 8 under a 2 px bar in the colour) unless it is folded, which keeps only the window on
screen; 10 px above a group, none above a loose window (0032). A click on a header folds it. `Alt+G` opens the dialog,
`Alt+Shift+G` takes the window on screen out of its group; a group with no windows is dropped. **Per run: neither windows,
groups nor names are saved.**

**Not done, deliberately (so nobody looks for it):** right-click menus and `Delete` in the overview; dragging to resize or reorder
panes; a scrollbar to drag in the overview, the palette and the tabs; groups and names between runs; the palette's *History*
heading (0027); group tabs narrower than loose ones (0032); a ghost under the card dragged in the overview (0032).

**Why one more ADR:** MAINTENANCE rule 3 says a decision is replaced by a new one, and rule 7 that the code wins over a leaf. The
statements of 0023 to 0030 that described a state the merge changed were corrected where they stood (they said "no overview
item yet", "the app passes `false`", "closing shows no Undo toast", "no pane menu"); what crosses several features is written once, here.

**Rejected:** repeating the integration in each ADR as a note (it drifts again); an ADR per merged branch with only the
difference (the facts belong to the feature's own ADR).

**Revisit if:** groups or names are to survive a restart (then a state file, and a rule for windows that cannot be restored), or
the overview gets context menus (they reuse `ctxmenu`).
