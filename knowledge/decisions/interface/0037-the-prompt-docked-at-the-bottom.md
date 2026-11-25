---
type: decision
status: accepted
updated: 2026-11-25
related: [0036-the-reference-cleaner.md, 0018-input-bar.md, 0021-ui-refactor-terminal-reference.md, ../workspace/0024-split-panes.md]
---
# 0037 · The prompt is docked at the bottom, below a hairline

**Decision:** the line being typed sits at the bottom of each pane, under a hairline, as the reference's composer
(`.composer`: docked under `.screen`, `border-top: 1px solid rgba(255,255,255,.05)`, padding 11/22/14 px). It is still the
shell's own prompt and line editor: what moves is where its lines are drawn. `view::Fold` lays the terminal's lines on a
pane's rows each frame:

- **The composer** is the prompt being typed at, from its reserved line (the chips' row, found by the hook's marks:
  `view::live_prompt`), or, without the marks or while a command runs, the line the cursor is on with the lines it wraps
  from; and whatever is below either (completions listed under the prompt). While a command runs it is the line the
  program writes and reads at: a progress line, a password or a REPL's prompt is typed into there.
- **The rows above** end right over it, so the first prompt is at the bottom and output rises above it. A fresh or
  cleared screen shows nothing above the prompt (the cleared lines are history, shown when scrolled to); scrolled back,
  the rows above scroll and the composer stays, with its cursor.
- **The alternate screen** (a full-screen program), and the `winit` path (`Backend::docks`), get the rows as they are.
- **Geometry** (`soft.rs`): 22 px between the rows above and the composer (the screen's 10 px, the hairline, the
  composer's 11), 0.45 em between the chips and the prompt, 14 px under it. That space is always kept in the grid's bottom
  margin (`Soft::bottom`), so the terminal's rows never change when the composer comes or goes; a full-screen program
  gets it as a margin. Each row owns a band of pixels (`Soft::band`: a gap belongs to the row below it, the space left
  to the last row), so a frame that lays the lines out anew repaints every row of the pane and leaves nothing of the old
  layout: the incremental repaint stays pixel-exact (`design.rs` tests).
- **The placeholder** "Type a command" (`.ph`, faint) follows the cursor while nothing is typed: the cursor right after
  the prompt's end mark and the rest of its line empty (`view::input_empty`). It needs the marks (the chips on).
- **The pointer** maps through the layout of the last frame (`Soft::row_at`, `View::fold`); a program that asked for
  mouse reports is told the line, as before. A command's status (0036) is drawn on its line wherever that is.

**Why:** the user, with a screenshot of the reference: "el input del texto es abajo, además hay unas líneas sutiles que
separan el resto de la ventana". The prompt sat wherever the shell left it, mid-screen on a fresh window, and nothing set
the input apart from the output.

**Cost** ([measurements](../../performance/measurements.md)): per frame and pane, a scan of the lines below the cursor
(none once the screen is full) and of up to five lines for the marks; no allocation. Binary +0.9 KB; against the same tree
drawing the rows flat, in turn: the 100 MB flood 1.87 s both, the spinners of three running windows 11 ticks / 5 s both. A new layout repaints the pane's
rows: once per prompt, and per line while output has not yet reached the bottom. The kept space (29 px at a 15 px
font) is one or two rows fewer for every program: the board's window went from 95 x 26 to 95 x 24.

**Rejected:**
- The reference's own composer, an editor of axon's (task 6a phase 2): completion, history search, vi mode and the
  shell's plugins would have to be rebuilt; the layout gives the same place and look with the shell's own.
- Printing blank lines at start and after `clear` so the shell's prompt starts at the bottom: it writes into the
  output, every `clear` undoes it, and the hairline still needs the layout.
- Resizing the terminal as the composer comes and goes: a reflow and a `SIGWINCH` at every prompt.
- The reference's job line while a command runs (spinner, label, stop button): it would hide the line the program
  reads from, where the user types.

**Revisit if:** the own editor is built, or a full-screen program that stays in the primary screen moves the hairline
with its cursor often enough to matter.
