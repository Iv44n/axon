---
type: decision
status: accepted
updated: 2026-11-24
related: [0012-glass-chrome-black-white.md, 0015-continuous-glass.md, 0017-settings-screen.md, 0018-input-bar.md, 0020-text-quality.md]
---
# 0021 · The interface follows the `terminal.html` reference

**Decision:** the window chrome, the status line, the prompt chips and the settings screen are drawn after the HTML
mock-up the user supplied (`terminal.html`, "arcton"). It replaces the strictly black-and-white identity of
[0012](0012-glass-chrome-black-white.md) and the "no separators" rule of [0015](0015-continuous-glass.md): the user asked
for it, and the mock-up is the new source of truth for the look. Everything is still computed per pixel by our own
renderer (no WebView, no CSS engine, no new dependency); the mock-up is a design spec, not code we run.

**Tokens (`palette.rs`):** ground `#0b0b0c`, text `#d9d9de`, secondary `#74747d`, cursor `#f4f4f5`, one amber accent
`#ffb454` (the prompt `$`, sliders, switches, the chosen radio, focus) and six hues: green `#3ddc84`, red
`#ff5d5d`, magenta `#d86bdc`, yellow `#f6b73c`, blue `#6aa9ff`, cyan `#4fd6d0`. They are also the ANSI 1-6 slots, so
programs' colours match the chrome. ANSI 8 (grey) is one step lighter than the secondary text to keep the 4.5:1 rule.

**Window (`chrome.rs`):** title bar 42 px (40 since 0031) with a top light that fades out and a hairline at its foot; the buttons are
12 px grey discs that turn red, yellow and green (with dark glyphs) when the pointer is over the group; the settings
button is a sliders icon in a 28 px square that highlights on hover; the title is `user@host:` in the secondary colour and
the rest in the text colour; a 1 px light inside the top edge. A 28 px **status bar** at the bottom (hairline, faint
tint): a green dot and the shell, `UTF-8`, the git branch (with the chips on), the key hints in keycaps (dropped from the
left when they do not fit) and the grid size. It takes rows from the grid (`session::grid_size` gained `bottom`) and can
be switched off with `[window] status_bar = false`.

**Chips (`bar.rs`):** outline icons redrawn from the mock-up's 16-unit SVG paths (user, clock, folder, branch, hexagon
for the environment, `<>` for Python, pencil for changes, wheel for Kubernetes, monitor for the host), 8 px radius, 10%
fill, 58% border, regular weight text, at 90% of the font size. The shell hook now prints the `$` in bold amber.

**Settings (`settings.rs`):** 500 px panel, 16 px radius; a live **preview** of the chips and a prompt line in the font,
weight and spacing being edited (`bar::paint_preview`, scaled down to fit); tabs in a rounded container with a marker that
slides (0.22 s); amber sliders, switches and radios; a **Cursor** row (block, bar, underline: `[input] cursor`, applied
through alacritty's `default_cursor_style`, programs may still change it with DECSCUSR); chips in their own colours with the
drag-to-reorder of 0020; a quiet note box; a footer with a hairline and **Reset** (the amber "Unsaved changes", **Cancel** and
**Save** of this first version went with the modal screen: [0026](../workspace/0026-docked-settings-panel.md) saves as you change). The clock and
chips rows are faded and inert while the input type is the shell's own prompt.

**What was left out, and why:**
- The blinking cursor: it would wake the process twice a second forever (hard rule 3, idle = 0% CPU). The cursor is steady.
- The amber glow around the window, the 3 px blur behind the settings overlay and the proportional font in the title bar:
  they need a bigger shadow margin, a full-screen blur and 5-9 MB more resident memory respectively.
- The mock-up's sample text (the message of the day, the demo commands) is not a feature of a terminal.

**Per-command status (added later the same day, `history.rs`):** each prompt's chips keep their row and gain, once the
command has run, a check or a cross with the exit status and the duration at the right end, as in the mock-up. The
channel is the one thing a terminal emulator gets from a shell for free: window titles. The hook prints
`ESC ] 2 ; axon!hook S BEL` when a command starts (bash `PS0`, zsh `preexec`, fish `fish_preexec`) and
`axon!hook E <n> <status>` at each prompt (and `axon!hook L <command line>` after the start, 0031: the window's label); the listener on the PTY thread swallows them and stamps `Instant::now()`, so
the time is taken where the bytes are parsed, not when the UI wakes. The prompt number `n` rides on the reserved line in
the *colour* of its concealed cell (`ESC[8;38;2;R;G;Bm`, 24 bits): invisible, no extra characters to copy, and it
scrolls with the row, so any row on screen or in the scrollback finds its `Record` again. `History` is a ring of 256
records (the chips as they were when the next prompt came, the result, a revision); the newest record follows the live
chips until then. A shell whose hook predates this (an old `axon shell-init` in an rc file that was not re-evaluated)
still gets the live chips on the active prompt, through `view::prompt_row`.

**Application icon (`assets/axon.svg`):** redrawn in the same language: dark glass window with a hairline under the title bar,
the three buttons in their traffic-light colours, two outline chips (green, magenta) on the line above the prompt, a bold amber
chevron and a white cursor. No gradients beyond the body's and rim's; it still reads at 48 px (the chips become two coloured bars).
