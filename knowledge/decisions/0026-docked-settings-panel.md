---
type: decision
status: accepted
updated: 2026-11-24
related: [0017-settings-screen.md, 0023-windows-and-side-panel.md, 0022-i18n.md, 0009-data-oriented-no-caches.md, 0018-input-bar.md]
---
# 0026 · The settings are a panel docked beside the terminal, not a modal screen

**Decision:** `Ctrl+,` and the gear open a panel at the edge of the window (`settings.rs`, after the reference's
`.prefs`, "Ajustes: panel acoplado (no modal)"). The grid shrinks to make room (every window is resized), the terminal
and the title bar stay usable, changes apply at once and are saved by the app: there is no Save and no Cancel.
Supersedes the modal part of [0017](0017-settings-screen.md) (its rules about state, the own text rasterizer and the file
that keeps the user's words stand).

**Layout:** a header (title, a close cross), three tabs with a marker that slides (Font, Input, Windows), a body of rows
(label 112 px at the left, control 213 px at the right, 40 px high; the reference's measures, read from its computed
styles), a note under the rows of two pages, and a footer with *Reset to defaults*. Controls: the font picker
(`<` name `>`), sliders with a number box (click it or just type), switches, radio groups that wrap onto more lines when
their labels do not fit (so a row's height depends on the text: the layout is computed when asked, never stored),
the chips (two columns; click toggles, drag reorders, as in [0018](0018-input-bar.md)). The clock and chips rows are
inert (38 %) with the shell's own prompt, the panel's rows when the windows are tabs at the top. The body scrolls
(wheel, or the keyboard bringing the selected row into view): the Input page does not fit a 640 px window, as in the
reference. The painter is clipped by lending it only the framebuffer rows of the body, so what is scrolled out of view
cannot draw over the header or the footer, with no clip field in `Paint`.
Pages: *Font* (font, size, weight, bold, letter spacing, line height, opacity, blur), *Input* (type, clock, cursor, chips,
autosuggestions), *Windows* (tabs position, panel visible, panel width, and the **Language** row, which the reference
does not have; it moved here from the Font page). The live preview box is gone: the terminal beside the panel is the
preview.

**Stacking:** `.main` is a row of windows' panel, workspace, settings. With the windows at the left the settings are at the
right; **with the windows at the right the settings go to the left** (`.prefs { order: -1 }`), as in the reference, so the
two never share an edge. `Soft::set_docks(side, prefs)` takes a `Dock { w, left }` for each; `inset()` is their sum per
edge, `side_box` / `prefs_box` give the boxes for a window size (the app knows the new size before the next frame tells
the renderer). Should two ever share an edge the windows' panel is outermost. The panel is painted into its own cleared
box (`Soft::clear_rect`) when the frame is whole or something in it changed (`App::prefs_dirty`, or an animation in
progress); the grid beside it repaints row by row as before (`design.rs` checks that equals a full repaint). Width: 372 px,
62 % of a narrow window at most (`settings::panel_width`).

**Saving:** every change goes to the window at once (`Effect::Apply`); it is written when it settles. A slider being dragged
or a number being typed is not settled (`Settings::settled`): the write waits for the release (`Effect::Settled`) or the
commit. Discrete changes (a switch, a radio, a chip, a key) are written 300 ms after the last one (`App::save_at`, one timer
in the `poll` list, none when idle), so a key held on a slider writes once. Closing and quitting flush it. Only what
differs from the file is written, through `config::save` (a temporary file and a rename, comments kept); a failure is
written in the footer and the panel stays. *Reset to defaults* also saves. `Alt+B` moves the panel's switch without being
something to save (it is per run, [0023](0023-windows-and-side-panel.md)).

**Focus:** the panel has the keyboard when it opens (on the active tab, which gets a ring, as the reference focuses it) and
when it is clicked; clicking the terminal or the windows' panel gives it back to the shell (the selected row stops being lit).
The app's shortcuts (`Alt+...`, `Ctrl+Shift+...`) work in both cases; `Esc` closes the panel when it has the keyboard (and first
cancels a number being typed). Keys: the tab bar is one stop of the cycle with the rows (`Tab`, `Shift+Tab`, up and down;
the inert rows are skipped) and there left and right change page; on a row left and right change a value (radios wrap),
`Space` or `Enter` flip a switch or pick the next radio, `PageUp`/`PageDown` or `Ctrl+Tab` change page, `Alt+Left/Right`
move a chip. A label longer than its 112 px column wraps and the row grows, as in the reference ("Espaciado de letras").

**New key:** `[input] autosuggest = true | false` (default true), parsed, saved, shown as a switch; nothing reads it yet (the
line editor of task 6a phase 2 will).

**Rejected:**
- *Keeping Save and Cancel:* the reference applies and saves at once, and a docked panel next to the terminal has no
  moment to confirm in.
- *A clip rectangle in `Paint`:* every painter builds one by hand; lending the body's rows as a sub-slice does it for the
  one caller that scrolls.
- *Storing the rows' geometry:* it follows the labels' widths, which follow the language; recomputing it costs a few
  microseconds per event and cannot go stale ([0009](0009-data-oriented-no-caches.md)).
- *Writing on every pointer move of a drag:* a file rewrite per pixel; the release writes once.

**Not done:** the reference fades and slides the panel in (opacity and `translateX(10px)`): only the fade is done. The
reference's chips all come out green (a CSS accident: `.chip { --c: green }` is declared after the per-chip colours);
axon first kept each chip's own hue, and follows the reference since 0031 (the prompt keeps the hues). No toast when the values are reset. The switch knob does not animate.

**Revisit if:** a setting needs a text field (a general one, not the number box), or a fourth page makes the tab bar too
wide for the narrowest panel.
