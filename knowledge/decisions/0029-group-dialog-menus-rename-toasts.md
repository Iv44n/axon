---
type: decision
status: accepted
updated: 2026-11-24
related: [0023-windows-and-side-panel.md, 0024-split-panes.md, 0025-overlay-kit.md, 0009-data-oriented-no-caches.md]
---
# 0029 · The group dialog, context menus, inline rename and toasts

**Decision:** the window list's interactions of the `terminal (2).html` reference (`openGroupDialog`, `tabMenu`, `groupMenu`,
`paneMenu`, `stripMenu`, `startRename`, `closeTabs` + `undoClose`, `toast`) are built on the overlay kit
([0025](0025-overlay-kit.md)) and the pane model ([0024](0024-split-panes.md)), each doing the real thing to windows, groups
and shells. `Alt+G` opens the dialog and replaces the quick `Alt+G` of [0023](0023-windows-and-side-panel.md); `Alt+Shift+G` stays.

**Where:** `groups.rs` (pure and tested: `Group`, the form's validation, names, colours, what a close takes out and puts back,
the templates), `group_dialog.rs` (an `Overlay`), `ctxmenu.rs` (the menus' items and command ids), `side.rs` (the rename field,
the header's new-group button), `wlapp/groups_ui.rs` (`impl App`, a child module of `wlapp.rs`: the part that owns the shells),
`design/group_scenes.rs` (board group `groups`). A group is `Group { id, name, color, collapsed }`; a window's group is still an
index (`Win::group`) and `id` is what a menu or the dialog holds while indices shift. `Win::name` is the user's name for it
(empty: the automatic `auto_title`; `win_title` is what the panel, title bar and dialog show). Geometry is computed when asked.

**Dialog** (the reference's measures): name (24), eight colours, the windows (the one on screen ticked when in no group),
templates, summary, Cancel / Create; edit mode (`Edit group`, from the menu or a double click on the header) has only name and
colour. It checks its own form (`groups::check`) and shows the reason in `.f-err`: an empty name, **a name another group has**
(an addition; case and edge spaces ignored), nothing chosen. A valid form goes to the app in a shared cell (`Submit`) before
`Out::Run`. Tab / Shift+Tab: name, colours, windows, templates, Cancel, Create; arrows pick or move; Space ticks; Enter submits.
A window too short for the card shortens the windows' list first, then scrolls the body (the kit keeps the title bar live, so
the card has 34 px less than the reference's).

**Templates, for real:** *None*; *One window*; *Development*: three windows in the folder of the one on screen, named `server`,
`tests`, `git`, each with a command **typed into its shell** from the folder's files (`groups::commands`: `Cargo.toml` `cargo
run` / `cargo test`; `package.json` `npm run dev` / `npm test`; `go.mod`; Python `pytest`; `git status` inside a repository).
Nothing known leaves the window named and empty: the reference's `dev` and `test` are its demo shell's commands.

**Menus** (right click; the first item lit, as the reference's `first.focus()`): a window (rename `F2`, duplicate, split `Alt+\` /
`Alt+-`, the groups with a check, new group `Alt+G`, remove from group, close `Alt+W`, close the others; the last two red), a
group's header (edit, collapse / expand, new window in it, dissolve, close), a pane's row (go to it, close it), the empty list or
the `+` (new window, new group). The target is kept by id while the menu is open. The reference's *Overview* item is built
(`strip_menu(at, overview)`) and asked for **without**: whoever merges the overview passes `true` and handles `Cmd::Overview`.
`F2` renames the window **under the pointer** (the reference: the tab with the keyboard on it); elsewhere F2 is the shell's.

**Rename:** a second press on a row within 350 ms, `F2` or the menu. A field (`side::paint_rename`: 22 px, hairline 25 %, black
45 %, radius 6, the grid's font, solid caret, selection; the cross stays) holds the name, all selected: a `field::Field` (max
28) scrolled by the caret. Enter commits (`rename_result`: empty or the automatic name goes back to automatic), Esc cancels, a
press outside commits and goes on (the reference's blur), a press inside places the caret (double click a word, three all), a
drag selects, paste / copy / cut use the clipboard. While it is open every key is the field's.

**Close with undo:** `Alt+W`, a row's cross and the menus' close items call `close_wins`: the windows leave the list and **stay
alive, hidden**, in `App::undo` until their toast is gone (`toast::TTL` and a moment); *Undo* puts them back where they were
(`groups::reinsert`) with their groups (`resolve` remakes one the close dropped); otherwise the stash is dropped and `Pane::drop`
ends the shells. `poll` wakes once for that. A new close replaces the stash. Closing every window (a group that holds them
all; `Alt+W` on the last still quits) opens a fresh one first, the reference's placeholder, which *Undo* removes unless
something else was opened beside it. The last pane's cross and a shell that exits are not undoable.

**Toasts** (both languages, `groups::fill` puts names in): closed (with *Undo*), *n windows closed*, group closed (*Undo*), moved to
a group, out of its group, dissolved, made (`· n windows`), updated, *With the tabs on top there is no side panel* (`Alt+B`
there) and, not in the reference, *No room for another pane* when a split is refused (it refused silently).

**Header:** a second button, new group (`Part::NewGroup`), right of a `+` that moved 26 px left; the tab strip has none (the
reference puts one in the title bar: it has had one since 0032). **Aid:** `AXON_LIST_UI=dialog|edit|menu|group-menu|pane-menu|strip-menu|rename|close|toast`
starts with one open (`edit` and `group-menu` need `AXON_GROUPS=1`).

**Rejected:** a dialog that hands the app a half-checked form (it would check twice); closing for good and "undoing" by respawning
(the shell and its history would be gone); a global `F2` (it is the shell's: mc, htop).
**Differs from the reference:** no *Overview* item yet; labels regular weight; scrim under the title bar and a press outside a
menu swallowed (0025); the rename uses the grid's font (0023); names and groups are per run; no dragging of rows, groups or
panes; the undo placeholder is not checked for being empty.
**Revisit if:** the overview or dragging lands (they reuse `close_wins`, `open_group_dialog`, `move_to_group`, `spawn_win`), or
groups and names are to be saved between runs.
