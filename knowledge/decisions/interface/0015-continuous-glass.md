---
type: decision
status: accepted
updated: 2026-11-23
related: [0012-glass-chrome-black-white.md, 0014-client-shadow-design-board.md, ../../../.claude/skills/axon-design-language/SKILL.md]
---
# 0015 · One continuous glass: integrated title bar, uniform edge

**Decision (requested by the user on 2026-11-23: "the lines in the corners should not appear, everything must
be continuous", pointing at Warp and similar terminals):** the window is one surface. The title bar is not drawn:
the buttons and the title sit on the same glass as the grid. The edge is the same all the way round.
Supersedes the look of [0012](0012-glass-chrome-black-white.md) (sheen, lighter title pane, hairline, rim
fading from 46% to 16%, edge light) and the contact line of [0014](0014-client-shadow-design-board.md).
What stands from 0012: black and white, chrome drawn per pixel on the CPU, three round buttons on the left.

**Why:** the rim was brightest at the top, so it gathered into bright arcs at the two top corners; the
hairline under the title bar met the rim at both sides; the lighter title pane and the sheen split the window
into regions. Terminals with a designed frame keep the top continuous: Ghostty's default macOS title bar is
transparent and takes the terminal background, WezTerm's `INTEGRATED_BUTTONS` puts the window buttons in the tab
bar with no title bar, and Warp's tab bar shares the terminal's surface.

**Now:** body `#090909` at `chrome::OPACITY` everywhere; a 1 px outline of 8% white, identical on every side and
anti-aliased round the corners, so the glass has an edge on a dark desktop; the soft shadow alone outlines it
on a light one. Cells a program paints in the terminal's own background colour are glass too (`view.rs`), so a
theme that sets it explicitly leaves no opaque block. Pinned by the chrome test
`the_glass_is_continuous_and_the_outline_uniform` and `design::background_coloured_cells_are_glass`.

**Rejected:**
- *No outline at all:* on a dark desktop a near-black window loses its edge entirely.
- *A darker or lighter title area:* exactly the split the user asked to remove.

**Revisit if:** tabs arrive (task 8): they go in the title bar area, on the same glass, without a separator.
