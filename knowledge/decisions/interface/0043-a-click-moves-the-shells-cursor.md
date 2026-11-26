---
type: decision
status: accepted
updated: 2026-11-26
related: [0018-input-bar.md, 0037-the-prompt-docked-at-the-bottom.md, ../core/0009-data-oriented-no-caches.md]
---
# 0043 · A click on the line being typed moves the shell's cursor there

**Decision:** a plain click (down and up on the same cell) on the command being typed at a prompt presses as many Left or
Right arrows as it takes to put the shell's cursor on that cell. `caret.rs` computes them (pure: the terminal and the
cell in, the number of characters out); `App::click_caret` decides whether to and sends them.

**Why:** a long command with a mistake in its second row could only be fixed by holding the left arrow. A terminal cannot move
the shell's cursor, but it can press the keys, which is what kitty and iTerm2 do with shell integration.

**How:**
- The line is what is typed after the prompt's end marker (`view::prompt_end`, the concealed space the hook draws, ADR 0018)
  up to the last character of the cursor's row and of the rows that wrap from it: to the shell a wrapped command is one line,
  and an arrow crosses the wrap. Without the marker (`mode = "shell"`, PowerShell on ConPTY) the line is the cursor's row
  and the rows above it that wrap into it, from column 0: an arrow past the start of the editor's text is ignored.
- A click before the start goes to the start, one past the end to the end. Characters are counted, not cells (a wide character
  is one arrow, its second half is it). The arrows follow the cursor-keys mode (`CSI D` or `SS3 D`).
- Not done when: a command runs or the shell's hook has said nothing yet (the arrows would go to a program), a program has the
  mouse (without Shift), the screen is a full-screen program's, the button moved to another cell (a selection), it is not the
  first click of a multi-click, or the cell is not on that line (the output, the chips' row, the gap above the composer).
- **A suggestion is not text.** zsh-autosuggestions and fish draw the rest of a command dim (bright black, a dark grey, `SGR 2`)
  after the cursor; Right at the end of the text would accept it. Trailing cells like that are not part of the line, so a click
  on them goes to the end of the real text.

**Rejected:**
- Home, End or Ctrl+A to reach the ends: shells bind them differently; arrows are the one thing every line editor has.
- An own line editor (task 6a phase 2): the right place for exact editing, but a click that works today in the shell's own
  editor does not wait for it.
- A modifier (Ctrl or Alt) with the click: the plain click is what was asked, and a click on a prompt that selects nothing
  costs nothing.

**Revisit if:** the own editor lands (it can place the caret itself), or a shell where the arrows do not move by character.
