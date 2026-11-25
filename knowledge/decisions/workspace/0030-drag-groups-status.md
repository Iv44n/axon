---
type: decision
status: accepted
updated: 2026-11-25
related: [0033-windows-feature-set-after-the-merge.md, 0023-windows-and-side-panel.md, 0024-split-panes.md, 0025-overlay-kit.md, ../core/0009-data-oriented-no-caches.md]
---
# 0030 · Drag and drop in the list of windows, groups in the tabs, status icons

**Decision:** the list of windows (the panel, or the tabs at the top) takes drags to reorder windows and groups, the
tabs at the top show groups as the reference does, and every row says what its shell is doing, with a spinner that turns
only while a command runs. After `terminal (2).html` ("Barra de pestañas", `.tgroup`, `.gchip`, `.drop-before`,
`.drop-into`, `.spin`, `.dot-err`, `.t-panes`).

**Drag, pointer driven.** A press on a window's row or tab, or on a group's header or chip, does nothing yet
(`App::dnd`, `Dnd`); when the button comes up without the pointer having moved `dnd::THRESHOLD` (5 px) it is the click
it always was (go to the window, fold the group, now on release as in the reference). Past the threshold it is a drag:
the grabbed row stays where it is at 40 % opacity (`.dragging`), a ghost of it (the row at 40 % on a little glass)
follows the pointer inside the list (clamped to the panel, or to the strip at the tab's own height), and a mark says
where a drop would go. Esc, an overlay opening, or windows or groups coming or going cancel it. We did not use the
compositor's drag-and-drop (`wl_data_device`): it needs more protocol, a drag icon surface and a second event path for
something that never leaves the window.

**Targets** (`Layout::target`, pure, tested) follow the reference's `dragover`: a window over a window goes before or
after it by which half the pointer is in (a window with panes is one block: its row and its pane rows), and takes its
group; over a group's header or chip it goes *into* the group, first (a 2 px ring in the group's colour, `.drop-into`);
over the empty part of the list, or the `+`, it leaves its group and goes last. A group over another group, or over a
window of one, goes before or after that group's whole block. Over itself, or off the list: no target, nothing moves on
release. The marks (`Layout::mark`) are the reference's: a 2 px accent line along the near or far edge (`.drop-before`,
`.drop-after`: horizontal in the panel, vertical in the tabs), the ring, and one we added, a line at the end of the list
for "last".

**Result** (`dnd::arrange`, pure, tested): the windows' and groups' permutation, with the groups renumbered, a group left
with no window dropped (`pruneGroups`) and the group a window went into unfolded. ADR 0023's invariant holds after every
drop: windows sorted (groups in order, each group's windows together, the windows in none last). That is where we part from
the reference, which lets a loose tab sit between two groups: a group dropped near a loose window goes after the last group.
`App::drop_on` applies the permutation to `wins` and `groups`, keeps the window on screen on screen, and says where a
window went in a toast (`“~” moved to “arcton”`, `“~” out of the group`: the menu's sentences, `Str::ToastMoved` and
`ToastUngrouped`) when its group changed.

**Groups in the tabs** (`Layout::Top { entries, groups, cw }`, `top_entries`): a group is a container (2 px padding, 2 px
gaps, radius 11, its colour at 6 %) with a chip (24 px: dot, name in the heavier weight, count; at most 180 px) and its
tabs; the tab on screen is the group's colour at 16 %. Tabs are as wide as the bar allows after the chips, the containers
and the gaps take theirs (168 px down to 96, as before); what does not fit is left out, with the `+` pinned to the end of
the bar. A folded group keeps only the window on screen if it is one of its own, in the panel too (`entries(…, active)`,
the reference's `renderTabs`); going to a window of a folded group unfolds it (`activate`). The tabs' entries depend on
the window on screen, so `relist` runs when it changes and a group is folded.

**Status.** A window's icon is the busiest of its panes (`Status::of`: running, then failed, then idle), a pane's row has
its own; both come from `History::running` / `failed` (the shell's hook: bash gets it from axon, zsh and fish through
`axon shell-init`; Ctrl+C, status 130, is not a failure). The icons are the prompt (idle), a 11 px ring with a bright quarter that turns once in 800 ms (running)
and a 7 px red dot (failed); a folded group's dot turns while a window in it runs (`GroupItem::busy`). The spinner is the
only animation the list has: `App::spin_at` is set when the list is painted with something running in view and
`side::SPIN_EVERY_MS` (40 ms, 25 frames a second) ahead, cleared when nothing runs, and is one more deadline of the loop's
`poll`; nothing is scheduled otherwise, so idle stays at 0 % CPU. A step does not paint the list again: painting its text
is most of the cost (no glyph cache, rule 5) and was 5 % of a core at 25 frames a second. `side::paint_spin` wipes each ring
back to the glass, lays down what is under it (the group's container tint, the row's lit background: flat at that spot)
and draws the ring at the new phase; `design.rs` checks that this is pixel-for-pixel what painting everything again gives.
Measured with three windows running, the panel costs about 0.8 % of a core, the tabs about 1.6 % (the title bar's glass is
costlier to restore); idle with nothing running is 0 ticks in 8 s. A window out of sight keeps the compositor's frame
callbacks away and the loop then repaints at the stall watchdog's pace. The unread dot and the pane-count badge (tabs only)
were already there; `AXON_RUNNING=1` makes every pane count as running (a shell with the hook says it is at a prompt a
moment after it starts, so a real command is awkward to hold for a screenshot or a measurement).

**Data.** No cache: `entries` (what the list shows) and `gitems` (the groups' names, counts and busy flags) are rebuilt by
`relist` when windows, groups, folds, panes or the tabs' position change, and hit testing walks them on demand
(`Layout::each`, no stored rectangles). A press and the drag's position are one small struct; the ghost's place is a
pure function of the pointer (`ghost_at`).

**Not done:** (the strip scrolls a unit at a time since 0032) group tabs are as
wide as loose ones (the reference's are narrower, by content); no keyboard way to move a window or a group; no animation
for the drop (the reference has none either); panes cannot be dragged (the reference's pane rows are not draggable
either); the drag started in the overview is the card's, not this one (0032).
