---
type: decision
status: accepted
updated: 2026-11-24
related: [0017-settings-screen.md, 0016-config-file-font-settings.md, 0009-data-oriented-no-caches.md, 0010-own-implementations.md]
---
# 0018 · Context chips on the line above the prompt

**Decision (phase 1 of the "input" customization the user asked for, 2026-11-24, from Warp's *Input* settings):**
`[input] mode = "chips"` draws a row of rounded chips on the line right above the prompt being typed at: time, user,
conda or venv environment, pyenv, `user@host`, directory, git branch, git changes (`3 • +10 -2`) and the Kubernetes
context. Which chips, in what order and the clock format are settings (`[input] chips`, `clock`), edited from the
settings screen's **Input** tab ([0017](0017-settings-screen.md)) or the file. The default is `mode = "shell"`: nothing
changes until it is asked for. The shell still edits the line; axon adds context above it. Phase 2, when asked, is
Warp's other *Input type*: an own line editor that sends the line to the shell on Enter (needs OSC 133 integration and
a real text-input layer).

**History of this decision:** the first version drew the chips in a bar under the grid. The user (2026-11-24, seeing it
on the real window) said they must be "right where I type, the input I am going to write in, not decorations": a bar
at the window's bottom is far from the prompt, which sits wherever the output ended. So the chips now follow the prompt.
The bar, its reserved rows and its hit-test exclusions were removed.

**How axon finds the prompt.** axon cannot see where the shell draws it, so the shell marks it. While the chips are on,
the prompt starts with a line of its own holding one concealed space (SGR 8) in its first column and ends with another
concealed space right where the user types. `view::prompt_row` looks for the end mark on the cursor's row (or the first
row of a wrapped command), then for the start mark up to four rows above, and the chips go on that reserved line. A
program's output leaves no mark at the cursor, so chips follow the *active* prompt only and vanish while a command runs;
old prompts keep a blank line. Marks are invisible, one column each, copied as spaces.
- *The chips replace the prompt's text* (user report, 2026-11-24: the host kept showing in `user@host:~$` after the host chip
  was switched off). While the flag is on the hook saves `PS1` and sets the reserved line, the window title
  (`\u@\h: \w`, as Debian's prompt does) and a bare `\$`; when the flag goes it puts the saved `PS1` back. What the
  prompt shows is then exactly the chips the user picked.
- *axon starts bash with the hook in `PROMPT_COMMAND`* (`context::shell_env`), so it works with no configuration;
  `axon shell-init bash|zsh|fish` prints the same hook for an rc file (zsh and fish, a bash whose rc replaces
  `PROMPT_COMMAND`). The hook keeps `$?`.
- *A flag file toggles it live.* axon creates `$XDG_RUNTIME_DIR/axon-<pid>.chips` while the chips are on and removes it
  when off; at every prompt the hook adds or strips its marks to match, so the settings screen takes effect at the next
  prompt, with no restart. Stale files of killed windows are removed at the next start.
- *The shell reports what only it knows* (conda, venv, `PYENV_VERSION`, `KUBECONFIG`) to `$AXON_CONTEXT`
  (`axon-<pid>.ctx`) at each prompt.
- Verified in a real bash on a pty: with the flag the prompt is the reserved line, the title and `$`; with it gone the
  shell's own prompt is back; `$?` kept; on, off and on again in one session. zsh and fish hooks are written but not run (not installed here).

**How it stays within the rules:**
- *No memory of its own.* The chips are drawn with the grid's `Fonts` and glyph tiles (`Soft::paint_cells_at`, a cell
  row at any pixel position). An own text rasterizer would have cost ~5 MB for good. Idle RSS with chips on: 14.9 MB.
- *Painted over rows, kept equal to a full repaint.* The framebuffer is the retained state, repainted by damaged rows, so
  an overlay on a row the terminal did not change would leave stale pixels or blend twice. `bar::Chips::frame` repaints
  the row the chips arrive on, leave and change on (`View::invalidate_row`) and draws them again only on rows that were
  repainted (`Soft::row_repainted`). `chips_repaint_incrementally_like_a_full_frame` checks it pixel for pixel over a moving
  prompt, typing, scrolling, a chip change, off and on; two deliberate mutations of the invalidation make it fail.
- *Nothing blocking on the UI thread.* `context::gather` reads `/proc`, `.git/HEAD` and small files and may run `git`,
  so it runs on a short-lived thread (`Refresh` in `wlapp.rs`), one at a time, and hands the result back through a slot
  and the eventfd. It starts when the terminal printed something (at most every 500 ms; the trailing one is scheduled,
  so the last prompt is always read) and when the minute changes. `git diff HEAD --numstat` runs when the directory
  changes and at most every 3 s. A 100 MB flood in a repository: 1.74 s with chips (1.72 s without), one `git` call.
- *Idle is still zero.* The only timer is the minute boundary, and only while the `time` chip is shown.
- *No dependency.* Time comes from `localtime_r` (declared by hand; `std` has no time zone), the rest from files.

**Where each chip's data comes from:** directory `/proc/<shell>/cwd`; branch `.git/HEAD` (worktrees and submodules);
changes `git diff HEAD --numstat`; pyenv `$PYENV_VERSION`, else the nearest `.python-version`, else
`~/.pyenv/version`; kube `current-context` of `$KUBECONFIG` or `~/.kube/config`; conda and virtualenv from the hook.

**Untrusted repositories (security review, 2026-11-24):** git runs what a repository's own config names: `core.fsmonitor`
is a command that `git diff` executes, and `diff.external` or a textconv filter run programs on the files. Entering a
freshly cloned repository must not run anything, so the changes chip calls git with `-c core.fsmonitor=false`,
`-c core.hooksPath=/dev/null`, `--no-ext-diff` and `--no-textconv`. A test makes a repository whose config points both
at a script and checks that the script does not run (plain `git diff` does run it).

**Fitting:** the pill is two pixels shorter than the row, so it never touches the lines above and below. Chips that do not
fit the width are shortened with `…` (longest first, never below 8 characters) and then dropped from the end
(`bar::layout`, pure and tested). Text from the system (a directory named with escape characters, a long branch) is
cleaned of control characters and capped at 40 characters.

**Colour:** the chips are the one splash of colour in the chrome, each with its own hue (as in the reference), opt-in
and fixed for now. The chrome itself stays black and white ([0015](0015-continuous-glass.md)).

**Rejected:**
- *A bar at the window's bottom* (the first version): far from where the user types.
- *An own line editor now:* it is the real Warp design but needs OSC 133 integration, history, completion and a text
  layer; the marks give "chips where I type" at a fraction of that. Kept as phase 2.
- *Chips overlaid on the previous output line, with no reserved line:* it would hide text.
- *Reading the shell's environment from `/proc/<pid>/environ`:* it never changes after exec, so `conda activate` would
  never show. *Parsing the shell's prompt output:* fragile, and alacritty gives no hook for unknown OSC sequences (the
  concealed-space marks are ordinary cells, so `alacritty_terminal` already keeps them).
- *An own `Fonts` for the chips (a smaller size):* +5 MB resident for a row of text.
- *`git2` or an own diff for the changes chip:* a dependency, or a diff engine, for a number that `git` already
  computes; one process every few seconds on a thread is cheaper than either.
- *Refreshing on every frame or on a fixed timer:* the first allocates and reads files on the hot path, the second wakes
  an idle terminal.

**Size (2026-11-24, asked by the user):** chips are 90 % of the font size set in the settings (first 80 %, then 90 % on the user's request): text, icon, padding
and pill height (`bar::SCALE`, `Dims`). The grid's tiles are at the full size, so the chip text is rasterized
through `Fonts::small`/`raster_into` into one reused buffer, only when a chip row is repainted; nothing is kept
(rule 5). Icons are outlines (1.2-1.3 px strokes at the 14 px design), never filled shapes: person, folder, fork and page are rings and boxes drawn with `stroke`/`box_outline`. It follows the zoom (`Ctrl +/-`) as the grid does.

**Limits:** `wl` backend only (the `winit` fallback has no chips, panel or mouse reports); chip colours are not
configurable; a wide character counts as one cell (CJK directory names overlap their neighbours); zsh and fish hooks are
untested; a prompt that is not `PS1` (fish's `fish_prompt`) is replaced by a bare `$`/`#` without the title; a prompt framework that rewrites `PS1` on every
prompt (starship, oh-my-posh) may need the hook after it; a prompt taller than four lines loses its chips; the old
prompts leave a blank line in the history.

**Revisit if:** the own input editor (phase 2) arrives, since it would own the prompt line and the chips would sit above
it without marks; or tabs and panes ([task 8](../plan/next-steps.md)) need chips per pane.
