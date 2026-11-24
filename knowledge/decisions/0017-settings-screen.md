---
type: decision
status: accepted
updated: 2026-11-23
related: [0016-config-file-font-settings.md, 0015-continuous-glass.md, 0014-client-shadow-design-board.md, 0009-data-oriented-no-caches.md]
---
# 0017 · A settings screen inside the window, drawn by the CPU renderer

**Decision:** a gear at the right end of the title bar (or `Ctrl+,`) opens a modal glass panel over the dimmed grid
(`src/settings.rs`). It edits the [0016](0016-config-file-font-settings.md) settings: font family (a picker over the
installed monospaced fonts), size, weight, bold, letter spacing, line height, opacity. Changes show on the window
as they are made (dragging a slider is live); **Save** writes `config.toml`, **Cancel**, `Esc`, the gear or `Ctrl+,`
put back what the screen opened with.

**Why in the window, not a command:** the user asked for a screen (2026-11-23, after `axon config` was built as a
command). Fonts are judged by eye, so the controls must act on the real text while they move.

**How it stays within the rules:**
- *Nothing is stored.* The panel is a pure function of the `Settings` state and the window size, painted over a
  complete grid on every redraw while it is open (`Soft::invalidate`, so the grid below is never partial). Closed,
  it costs nothing: no state, no redraw, no memory.
- *Own text rasterizer, only while open.* Panel text uses a `Fonts` at 13 px created on opening and dropped on
  closing, so the panel does not move while the size slider does, and the font being edited never changes it.
  +5 MB RSS while open (18.8 MB), none after.
- *No new dependency, no toolkit.* Shapes are the signed-distance coverage the chrome already uses
  (`chrome::rounded_rect`); the gear is a function too (`chrome::gear`, ring with eight teeth).
- *The file keeps the user's words.* `config::set_key` rewrites one line at a time (or turns the template's commented
  line into the setting) and leaves every other line and comment alone; only settings that changed are written, via a
  temporary file and a rename.
- *The font list is read, not guessed.* `font.rs` reads two sfnt tables with seeks (`post.isFixedPitch`, the `name`
  table's family) so only monospaced fonts are offered, under their real names, one file per family, without loading
  any of them. It runs when the screen opens, not at startup. A first piece of the own font reader (task 12).

**Typing numbers (2026-11-23):** a click on the number at a slider's right, or a digit, `.`, `,` or `-` typed on a
selected slider row, opens a box (`Edit`: the text and whether it is still selected). The first key replaces a
selected number. `Enter`, `Tab`, `Up`/`Down` and any click keep it, `Esc` drops it (and only it: with no box open it
cancels the screen). Typed values are clamped to the file's limits (`config::SIZE`...), not the slider's span, and
the knob pins at the track's end when the number lies beyond it; an unreadable box (`-`, empty) changes nothing.
Save takes in a box still open.

**Behaviour:** the title bar and the window edges keep working while it is open; presses over the grid belong to the
panel (no selection, no mouse reports), keys never reach the shell, the wheel is ignored. A key held while it closes
does not repeat into the shell. Arrows and `Tab` move between rows, left and right change a value (`Shift` = five
steps), `Space` flips the switch, `Enter` saves. A failed save is written in the panel's header and the screen
stays open.

**Rejected:**
- *A separate settings window:* a second surface, a second buffer (~3 MB) and a second event path for a few seconds
  of use; it also could not show the change on the real terminal.
- *Sliders only* (first version): the user asked for exact values, so each slider's number is also a box (below).
  *A general text-field widget:* only numbers are typed here, so the box is a short string with six rules
  (digits, one `.`, a leading `-`, seven characters, Backspace, no caret movement).
- *Caching the panel's pixels or glyphs:* [0009](0009-data-oriented-no-caches.md); a repaint is a few milliseconds
  and happens only on input.

**Limits:** weight is still synthesized ([0016](0016-config-file-font-settings.md)); the panel has fixed pixel sizes
(HiDPI, stage D, will scale them); colours, shortcuts and themes are not editable yet.

**Revisit if:** more settings arrive than fit one panel (then sections or scrolling), or tabs and panes
([task 8](../plan/next-steps.md)) need a different layer for overlays.
