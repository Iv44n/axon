---
type: decision
status: superseded-by-0037
updated: 2026-11-25
related: [0037-the-prompt-docked-at-the-bottom.md, 0021-ui-refactor-terminal-reference.md, 0018-input-bar.md, 0020-text-quality.md, ../workspace/0023-windows-and-side-panel.md, ../workspace/0025-overlay-kit.md, ../workspace/0032-fidelity-second-reference.md]
---
# 0036 · The same as the reference, cleaner: one UI font around the grid, no status bar, quiet chips

> Superseded in part by [0037](0037-the-prompt-docked-at-the-bottom.md): the prompt being typed at is docked at the bottom of
> the pane below a hairline, with the shell's own editor (rejected below as needing an own one), and the grid keeps the space
> for its gaps. The UI font, the status bar, the chips, the margins and the scrim below still hold.

**Decision:** the user asked again for the window to be the same as `terminal.html` (byte for byte the `terminal (2).html` of
[0032](../workspace/0032-fidelity-second-reference.md)), "cleaner and more elegant". Frames of the real renderer were set beside
screenshots of the reference at the same size (900 x 620, Chrome); what still differed is now as the reference has it:

- **One UI font around the grid.** The title, the panel of windows, the tabs and the rename field are drawn in the proportional UI
  font (`Soft::ui_text`, `ui_width`, `ui_cut`) at the reference's sizes: 12.5 px titles, 12 px medium group names, 11.5 px header,
  11 px counts, 10.5 px pane counts; the title's name medium and 0.6 em before its place. They were the grid's monospace at 0.85
  ([0021](0021-ui-refactor-terminal-reference.md) left the title's font out for its memory, [0023](../workspace/0023-windows-and-side-panel.md)
  the panel's). The window holds the font from its first frame; the settings and the overlays share it (`UiFont::shared`). A group's
  chip at the top keeps its measured width (`GroupItem::label_w`, `side::measure`), so `Layout` stays pure.
- **Its cost, cut** (`cmap.rs`). `fontdue` parses up front every outline the character map reaches (2 937 for Inter). `cmap::narrow`
  appends a new `cmap` (one format 12 subtable: Latin, Latin-1, Extended-A, Greek, Cyrillic, punctuation, currency, letter-like,
  arrows, maths, technical, shapes, dingbats: 892 characters of Inter) and points the table directory at it; the outlines, ids and
  advances stay. Inter: 8.3 MB parsed in 13.5 ms becomes 2.2 MB in 4.9 ms. Other characters draw the missing-glyph box.
- **Families:** Geist (the reference's), Inter, Adwaita Sans (drawn from Inter), Ubuntu Sans, Noto Sans, Open Sans, DejaVu Sans. Not
  Cantarell: its file is CFF2, which `fontdue` parses and draws nothing from (a font that draws no `H` is now refused). Weights are
  synthesized from the regular file: 500 (`font::MEDIUM`) where the reference has 500 (the title's name, group names, palette
  entries, buttons, card and member titles, the settings' tabs and chips), `font::BOLD` for headings.
- **No status bar by default** (`[window] status_bar = false`): the reference has none. The setting keeps it.
- **Quiet chips, on the prompt being typed at only**, as the reference's composer (`.c-box .meta .pill`): no box, icon 1.05 em and
  text at 0.84 of the grid's font, 80 % opaque, 1.1 em apart. A prompt whose command ran keeps no chips: its reserved line is the gap
  between two commands, as between the reference's blocks. How a command ended moves to the right end of the command's own line
  (`.b-info`, 0.78 em) and shows only when it failed (not 130) or the pointer is over its block (`App::grid_hover`), where the
  command's text leaves room. The history ring still keeps each prompt's chips.
- **The reference's margins** (`soft::PAD_X`): 22 px beside the text, 16 above, 14 below (`.screen` plus `.block` paddings), whatever
  the font size; a split pane the same inside its border. A pane's cross now lies partly in that margin, which rows never repaint:
  `Soft::clear_pane_close` gives the glass back there before it is drawn and when it goes.
- **A deeper scrim** under dialogs, 78 % instead of 50 %: the reference's 50 % lies over a 3 px blur that smears the text behind
  into a haze; over the near-black glass the deeper fill takes that text down as far.
- `paint_text` honours its colour's alpha (an opacity, with the pane's dimming); the board's demo prints the `$` in bold amber as the
  hook does, ends its history at the live prompt, and has a `chips-hover` scene.

**Cost** (A/B against the build before, in turn, [measurements](../../performance/measurements.md)): idle RSS 14.9 to 18.7 MB:
2.7 MB of anonymous memory for the narrowed font, held for good (the heap 4.4 to 7.1 MB), and ~1.9 MB more of the binary's
code paged in (file-backed, shared). Binary +11.4 KB. Flood and idle CPU unchanged (0 ticks; 1.77 s against 1.80 s). Opening
the settings or an overlay no longer parses a font: 22.1 MB with the settings open (was 23.1), 18.5 with the overview (19.7).
The pointer over the grid asks for a frame only when it moves into another command's block, with the chips on.

**Rejected:**
- The chrome in the grid's font and the UI font only while a panel is open (the old way): the title and the panel read as code, and
  were what made the window look unlike the reference.
- Ubuntu Sans first (0.8 MB narrowed): the desktop's font here, but its humanist shapes are not Geist's; Inter is drawn after the
  same model.
- Subsetting the outlines (`glyf`, CFF): the map is all `fontdue` follows; rewriting outlines is a font compiler.
- A real 3 px blur behind a dialog: the whole window blurred at every frame a dialog is open (each key typed in its field).
- The reference's docked composer and blocks: they need the own line editor (task 6a, phase 2).

**Revisit if:** the own font reader (task 12) parses outlines on demand (narrowing is then moot), a user's window titles need a
script outside the kept blocks, or the composer is built.
