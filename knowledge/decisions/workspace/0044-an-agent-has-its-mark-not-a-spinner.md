---
type: decision
status: superseded-by-0046
updated: 2026-11-26
related: [0046-the-agents-own-marks.md, 0030-drag-groups-status.md, 0032-fidelity-second-reference.md, ../interface/0021-ui-refactor-terminal-reference.md]
---
# 0044 · A window that runs an AI agent shows the agent's mark, not a spinner

> Superseded in part by [0046](0046-the-agents-own-marks.md): the marks are the agents' own published ones, filled in
> their own colours (Codex's cloud, not ChatGPT's knot), 14 px in the list and 16 in the overview; a program's status
> mark is cut from its title, and Claude Code's says whether it works (the mark faint) or waits for the user (an amber
> dot): axon can tell them apart. The mark instead of a ring, still and without a timer, stands.

**Decision:** when the command that runs in a window is `claude`, `codex` (or `chatgpt`), or `agy` (or `antigravity`), its row,
tab and overview card show that agent's mark (Claude's burst of rays in its orange, ChatGPT's knot in the text colour,
Antigravity's arch in blue) instead of the ring that turns. The mark is still. `agent.rs` maps the command to the agent;
`icons::agent` draws it; `side::Status::Agent` carries it.

**Why:** the shell's hook says a command runs from its start to the next prompt, and `claude` ends when the user quits it:
for hours the list said "busy" with a ring that never stopped, which is not true (it waits for the user) and cost a repaint 25 times a
second. Now the ring means a command that will end, and the mark says what is there.

**How:** the hook already reports the command's first word (`history::History::command`, the window's label); `agent()`
and `busy()` read it. `wlapp.rs` `running` (the ring, the folded group's busy dot, the timer `spinning`) counts only `busy()`,
so **a window with an agent running costs no timer**. A window's status is the busiest of its panes: a ring, else an agent,
else a failure. The name matches in any case, without a directory or `.exe`, `.cmd`, `.bat`, `.ps1`.

**The colour.** `palette::CLAUDE` (`#d97757`) is the one colour that is not from the reference (design language, rule 1): the
user asked for the agents' own marks. ChatGPT and Antigravity use the text colour and the blue.

**Rejected:**
- Detecting by the program's title (Claude Code sets one): the hook's word is there for every shell, a title is not.
- Looking at the foreground process: a syscall per check and a platform apart; the typed word covers `claude` as typed.
- A second animation (a breathing mark while the agent works): axon cannot tell working from waiting, and an animation that
  never ends is what this removes (design language, rule 6).

**Known gaps:** an alias (`alias c=claude`), a wrapper (`npx`, `sudo claude`) or a script have another first word and show the
ring as before. Adding a name is one line in `Agent::of`; Gemini CLI is not mapped (it is not Antigravity).

**Revisit if:** the own editor knows the command line whole, or a shell reports the foreground process.
