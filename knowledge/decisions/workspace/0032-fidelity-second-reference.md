---
type: decision
status: accepted
updated: 2026-11-25
related: [../interface/0021-ui-refactor-terminal-reference.md, 0028-overview.md, 0030-drag-groups-status.md, 0026-docked-settings-panel.md, 0031-review-fixes-paint-path.md]
---
# 0032 · Fidelity to the second reference: title bar, panel, tabs, chips, labels

**Decision:** the measures of `terminal (2).html` (computed styles and screenshots of each state) win over what 0021 to 0030
copied from the first one, wherever they differ and the grid's font does not get in the way. Found by a visual review.

**Window.** The title bar is 40 px (`TITLE_H`: the reference's 39 plus its hairline); its buttons are 28 px boxes at y = 5..33
(the reference centres them at 19.5; 19 keeps the discs and strokes on whole pixels and lines them up with the tabs, `BTN_CY`);
the gear is centred 22 px from the edge. The title is centred in the part of the bar between the panel button and the gear
(`TITLE_X0` 114, `TITLE_FROM_RIGHT` 46: its middle is 34 px right of the window's). With the windows at the top the strip of tabs
runs from x = 76 to 76 px short of the edge (`TABS_X0`, `TABS_FROM_RIGHT`), and the new-group button the reference has there
(`Hit::NewGroup`, the `Alt+G` dialog) takes the panel button's place; it is a per-pixel icon like the gear's.

**Panel.** The header is 38 px with its 24 px buttons 10 px under the top and the list starts 4 px below (the first group's
header is at +52, as measured); only a group has a margin above it, a window in none follows the last group with the 1 px gap
(it had 10); a group's name is dim, bright under the pointer; the unread dot is a flex item left of the pane count or the cross
(it used to jump on hover and sat on top of the cross). A middle click on a row or a tab closes it (`auxclick`).

**Tabs scroll** (`Layout::Top::first`): by a group or a loose window at a time, so nothing is clipped half way (`Paint` has no
clip rectangle, rule 5 and the overlay's cost). The wheel over the strip moves it (a touchpad's pixels add up, 40 per tab);
`reveal_active` brings the window on screen into view after any change of the list or of the width. The widths follow all the
tabs, so they do not change while it scrolls; what is before `first` has no box (`rect` says so, `shown` is false, `hit` finds
nothing); `scroll` stops with the last tab in view.

**Settings.** Every enabled chip is green (`.chip { --c: var(--green) }` is declared after the per-chip hues, so it wins in the
reference); the prompt keeps its own hues. 0026 had kept them.

**Overview.** *New group* and the tile open the dialog empty (`openGroupDialog({})`: the template "one window" is ticked), the
group's chip opens the edit dialog (`Cmd::EditGroup`, lit with the group's colour under the pointer), and while nothing is filtered
"No group" stays when groups exist and no window is in none, with an empty dashed box ("Drag a window here."). A card can be
carried (`Overview::drag`, 5 px make it a drag; the card goes to 40 %): over a section it takes the group's colour at 10 % and a
dashed border at 70 % (the windows in none, the accent), over the tile it lights amber; dropped there the app moves the window
(`Cmd::Move`, the pair is in `Overview::dropped`, the same `move_to_group` and toast as the panel's drag) or opens the group
dialog with that window ticked (`Cmd::NewGroup(Some(id))`); `Esc` puts it back, and no key is heard while it is carried. There is
no ghost under the pointer (the reference's is the browser's drag image).

**A window's label while a command runs** is the command's first word (`autoLabel`), through one more hook escape,
`axon!hook L <command line>`: bash's `PS0` arms a `DEBUG` trap (`${__axon_arm:=1}` sets the flag in the shell and goes, harmlessly,
in the title; `[[ ]]` tests it so `$_` is not touched; the function gives `$_` back); zsh and fish say it from `preexec`. A trap
of the user's own is left alone (then there is no label). `history::command_word` keeps one word, without directory or
`VAR=` prefixes and never the arguments: the title is readable by other programs. Own OSC titles still win; of the panes the
focused one's name comes first, else the first that runs.

**Known limits (deliberate).** The overview's dragged card has no ghost. The tabs are all one width (the reference shrinks group tabs
first), a drag does not scroll the strip at its edges, no horizontal wheel or scrollbar. A panel row keeps 5 px of right padding
and a 9 px icon offset (the reference's panel rows have 6 and 10; its tabs 5 and 9). The label is the first command of the line
(`cd x && make` says `cd`), and a bash with `HISTCONTROL`-style tricks is not involved (the trap reads `$BASH_COMMAND`).

**Revisit if:** the strip needs a scrollbar, a dragged card needs a ghost, or a shell hands over the whole command line safely.
