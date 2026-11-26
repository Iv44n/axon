---
type: decision
status: accepted
updated: 2026-11-26
related: [0044-an-agent-has-its-mark-not-a-spinner.md, 0030-drag-groups-status.md, ../interface/0036-the-reference-cleaner.md]
---
# 0046 · The agents' own marks, which say whether Claude waits for you; a title loses its program's status mark

**Decision:** the mark of a window that runs `claude`, `codex` or `agy` is the agent's own published mark, filled, in
its own colours: Claude's burst in its orange, Codex's cloud in its lilac-to-blue gradient with the prompt (`>_`) cut
out in white, Antigravity's arch in its field of Google's colours; 14 px in the list and the tabs, 16 in the overview
(were 12 and 14). A symbol plus a space at the start of a program's title is cut when the title arrives, and what
Claude Code's says is kept: `◐` `◑` (busy) make the mark faint (40 %), `✳` (its prompt, or a question or a permission:
the user's turn) gives it an amber dot at its lower right, in a gap cut out of the mark. A window's status puts an agent
waiting for the user above everything, a running command included: it is the one thing there that needs them.

**Why:** the user, with a screenshot: "mejora los iconos y no entiendo qué es lo que aparece al lado del icono, parece
un palillo", then "haz que la marca muestre si Claude espera". The 0044 marks were a few round strokes each and read as
other things at 12 px: Claude's as a five-pointed star, ChatGPT's knot as a hexagram, Antigravity's as the letter A. The
"palillo" was the UI font's missing-glyph box: Claude Code titles its terminal `✳ <task>`, and Inter (like Adwaita Sans,
Noto Sans, Geist) has no `✳`, `◐` or braille. 0044 said axon cannot tell an agent working from one waiting: its title
can (Claude Code 2.1.332 writes `◐`/`◑` in turn every 0.96 s while its status is `busy`, `✳` otherwise).

**How:** `icons.rs` holds each outline as a polygon in units of the mark's larger side (the published paths flattened
once, ~200 points in all) and fills it with `fill`: the non-zero rule, coverage exact across a scanline and 8 scanlines
a pixel, raised to the power 0.8 (as a font is emboldened when small: the rays are about a pixel wide), less a disc
for the waiting dot's gap. One row of coverage on the stack; nothing allocated, no image, no cache. The colours are in
`palette.rs` (`CLAUDE`, `CODEX`, `ANTIGRAVITY`: 5 x 5 colours read from the published mark's blurred patches, blended
bilinearly); the dot is the accent. `agent::State::of_title` reads the mark, `wlapp.rs` `status_mark` cuts it and the
pane keeps the state; `side::Status::Agent` carries it. A title that arrives the same, state included, repaints
nothing, so Claude Code turning its mark no longer repaints the panel (0-1 ticks in 10 s, 1-4 before). `Agent::ChatGpt`
and `Agent::Google` are now `Codex` and `Antigravity`, the products' names.

**The colours.** Design language rule 1 asks a reason the user gave for a colour outside the reference: the user asked
for the agents' marks (0044) and for them to look better. They are used for the marks and nothing else.

**Rejected:**
- Drawing the missing glyphs from a fallback font: the row would read "✳ ✳ task", the status twice, and every UI font
  would need a fallback chain the grid has and the UI does not.
- Cutting the title's mark only while an agent is known to run: an alias (`alias c=claude`) shows the ring, and its title
  had the same box.
- A mark that breathes or turns while the agent works: an animation that never ends and a timer (design rule 6, 0044).
- The cyan dot for "waiting": it says a hidden window printed; an amber dot on the mark is the agent's own.
- Simplified marks redrawn in strokes, as before: what the user did not recognise.

**Known gaps:** inside tmux or screen Claude Code writes `✳` always, so there it reads as waiting; Codex and
Antigravity set no such mark that axon knows, so their mark stays as it is; a folded group does not show a waiting agent
on its header.

**Revisit if:** an agent changes its mark or its title's marks, or another agent's title says what it does.
