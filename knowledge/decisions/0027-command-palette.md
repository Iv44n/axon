---
type: decision
status: accepted
updated: 2026-11-24
related: [0025-overlay-kit.md, 0023-windows-and-side-panel.md, 0024-split-panes.md, 0026-docked-settings-panel.md, 0022-i18n.md, 0009-data-oriented-no-caches.md]
---
# 0027 · The command palette

**Decision:** `Ctrl+K` opens the reference's command palette (`terminal (2).html`, `.palette`, `paletteAll()`,
`renderPalette()`) as a composition of the overlay kit (0025), with the reference's three headings and its keyboard. Two
modules: `commands.rs` decides which entries exist (pure: the app describes its windows and groups in a `State`, `build`
returns a filtered `List`, `Cmd::id` / `Cmd::from_id` turn an entry into a number and back) and `picker.rs` is the overlay
(`Picker`: the input row, the list, the footer; it answers `Out::Run(id)`). The app opens it (`open_palette`) and runs what
comes back (`overlay_ran` -> `run_cmd`) with the same calls the shortcuts make, so the two cannot differ.

**Entries** (heading in brackets; the first fourteen rows, with no query, are the reference's, in its order):
- *Actions*: new window `Alt+N`, new group `Alt+G`, new pane `Ctrl+Space`, split in columns `Alt+\`, in rows `Alt+-`,
  overview `Alt+O`, show or hide the panel `Alt+B`, windows and groups on the left / right / top (no shortcut: it sets
  `[windows] tabs`, shows the panel when it is a side, writes the file, toasts, and the settings panel follows), search the
  history `Ctrl+R`, clear the screen `Ctrl+L`, settings `Ctrl+,`, close the window `Alt+W`. After them, only where they apply
  (ours, not the reference's): close the pane `Alt+Shift+W` (the window has several), take the window out of its group
  `Alt+Shift+G` (it is in one).
- *Windows*: one per window (its place and group as the description, `Alt+1..9`), going to it.
- *Groups*: the group (its colour dot, "3 windows"), going to its first window; and "Collapse group x" / "Expand group x",
  which folds it in the panel (asked for as "fold / go to"; the reference has fold only in the group's context menu).

**Ranking, highlighting, keyboard:** the list's (0025): the reference's score (`100 - min(pos, 60)` for the query whole in
`label desc keywords`, `20` for its letters in order, accents and case folded, 40 shown, 70 with no query), the group name
at the right of each row while a query is typed, Up/Down wrap, Page/Home/End, Enter runs, click runs on release, hover selects,
the wheel scrolls, Esc or `Ctrl+K` again or a press on the scrim closes. Matched characters are lit in amber (ours; the
reference does not). Hidden keywords are in both languages (the reference's Spanish plus English), so a Spanish word finds a
command in an English window.

**What maps to what in axon, and what does not:**
- *Search the history* and *Clear the screen* type `Ctrl+R` / `Ctrl+L` into the focused pane's shell: the shell owns the line.
- The reference's *History* heading (its last eight commands, which it inserts into its own editor) is **left out**: axon never
  sees the commands (phase 2 of task 6a, an own line editor, would). The search box says "windows, groups, actions" and not
  "workflows": the reference's placeholder says it, but its JS defines no workflows.
- *Overview* is listed only when `State::overview` is true (the app passes `false`): the overview screen is another
  feature. Whoever builds it sets that flag in `App::open_palette` and fills the `Cmd::Overview` arm of `run_cmd`.
- *New group* does what `Alt+G` does today (a group named for the next in line); when the group dialog replaces it, both change
  together because they call the same method.
- `Alt+B` with the tabs at the top toasts "With the tabs at the top there is no side panel", as the reference does (shortcut and
  palette alike).

**Cost and rules:** the entries are built when the palette opens (a few dozen strings copied into the list's pool, one
`/proc/<pid>/cwd` read per window for its place), never per frame; the palette holds no cache and no per-frame allocation
beyond the kit's (3.6 ms a frame with it open, measured in 0025). Idle stays 0 % CPU. `Ctrl+K` does not repeat while held and
is taken from the shell (`kill-line` in readline), as in the reference. Strings: 18 new ones in `i18n.rs` (both languages); the
design board has `palette-*` scenes (empty, selection moved, scrolled to the end, typed, a window's name typed, no match) and the
Spanish ones (`palette-*-es`).

**Rejected:** a palette inside `overlay_demo.rs` (the demo's `Picker` moved to `picker.rs`, the demo keeps `overlay_demo::picker()`
with its ten sample entries); a table of shortcuts shared by `handle_key` and the palette (the keys are literals in
`handle_key`; `commands::tests::the_actions_show_the_shortcuts_that_run_them` pins the hints, and a shortcut that changes must
change both); fuzzy scoring beyond the reference's.

**Revisit if:** axon gets its own line editor (then a History heading with real commands), workflows exist, or the palette needs
entries that take an argument (then a second step in the same overlay, not a new screen).
