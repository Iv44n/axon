---
type: decision
status: accepted
updated: 2026-11-26
related: [0023-windows-and-side-panel.md, 0034-a-group-is-the-split.md, 0038-a-scope-per-window.md, ../core/0010-own-implementations.md]
---
# 0042 · The windows are kept between runs: where each shell was, not what it ran

**Decision:** `[windows] restore = true` (the default) writes a small session file as things change and opens it again at
the next start: the windows in the order of the list, each in the folder its shell was in, with the name the user gave it and
its group, the groups (name, colour, folded, split layout) and the window that was on screen. What a shell was running
is not kept, only where it was. `restore.rs` is the file (pure, tested); `wlapp/keep.rs` is the app's side.

**Why:** closing axon and opening it again put the user back in `$HOME`, in one window, which loses what a day of work
arranged: the windows, the groups that are the split view (0034) and, above all, the folders.

**How:**
- `$XDG_STATE_HOME/axon/session` (`~/.local/state/axon/session`; `%LOCALAPPDATA%\axon\session`), own line format with a
  version line, tab-separated escaped fields, unknown lines ignored. No dependencies (0010).
- Written on a change (a window or group came, went or moved; a rename; the window on screen; a shell's folder, read at
  each prompt) after 1.5 s, the first change starting the wait and the rest joining it, and once more when the app ends.
  The write is a few hundred bytes to a temporary file renamed over the old one. The loop's timer exists only while a
  change waits, so idle stays at 0 % CPU. The shell's full folder is kept in the pane (`Pane::place`), read by the code that
  already reads it for the title: saving makes no syscall of its own.
- The last window closing (or its shell ending: `exit`) keeps the state it had, so reopening goes back to that folder.
- A folder that is gone opens in `$HOME`; at most 32 windows are restored; a bad file is ignored, a bad field put right.
- **One owner.** The file names the process that wrote it. A second axon started while the first runs starts with one
  window and leaves the session alone; it takes over only when the owner is gone (on Linux: no process of that id named like
  this one, so a reused id does not block it).
- Runs with a debugging aid (`AXON_WINDOWS`, `AXON_SELFTEST`, ...) or `AXON_SESSION=off` neither read nor write it;
  `scripts/bench.sh` sets it off. `AXON_SESSION=<file>` uses another file.

**Rejected:**
- Restoring the scrollback or the running programs: it needs the shell and the programs to cooperate (tmux's job), and a
  stale screen of output is worse than a fresh prompt in the right folder.
- Saving only on exit: a crash, a logout or a `SIGTERM` would lose everything.
- Every axon writing the file: two windows of the program would overwrite each other and a second one would open a copy of
  the first's windows.
- A row in the settings panel: it is a once-set preference, like `scope`; the file and `axon config` have it.

**Revisit if:** the own editor and blocks (task 6a) make the command history a thing axon owns: the session could keep it.
