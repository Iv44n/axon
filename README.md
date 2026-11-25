# axon

Native terminal emulator: **simple, minimalist, high-performance and futuristic**.
Written in Rust, for Wayland (own client, CPU-rendered into `wl_shm`, no GPU stack) with a `winit` + `wgpu` fallback for X11 until it gets its own client (see `knowledge/decisions/core/0011-software-render-wl-shm.md`).

![axon: a dark glass window with its shadow](assets/screenshot.png)

<sub>A frame from the real renderer (`scripts/design.sh`), with alpha: what you see behind it is this page.</sub>

- **Own Wayland client and CPU renderer**: the grid is painted straight into a shared-memory buffer, only the damaged rows. ~15 MB idle, no Vulkan or GL loaded.
- **Zero idle usage**: no continuous redraw; it only paints when there are events (the one timer is the time chip's clock, once a minute, and only while that chip is shown).
- **Full VT emulation** thanks to [`alacritty_terminal`](https://crates.io/crates/alacritty_terminal).
- **Dark glass look** (after the `terminal.html` reference, ADR 0021): one rounded translucent pane with a soft shadow; a title bar and a status bar divided from the grid by hairlines, grey window buttons that turn red/yellow/green on hover, one amber accent and six clear hues (also the ANSI colours), outline chip icons and a settings panel docked beside the terminal. The frame is computed per pixel (`src/chrome.rs`), no images. On the default backend; the `winit` fallback only has the black-and-white colours.
- **Windows, groups and panes**: one shell per window, listed in a panel at the left or right or as tabs in the title bar; windows can be put in coloured groups (a dialog with templates that open real windows with their commands typed in), renamed in place, reordered and regrouped by dragging, and closed with an *Undo*; each row says what its shell is doing (the prompt, a turning ring while a command runs, a red dot when the last one failed). **A group is a split view**, as a tab with split panes is in Warp: the windows of the group you are in are shown side by side (columns, rows or a grid), and `Ctrl+Space` on a window in no group makes the group. Right click opens the menu of whatever is under the pointer. See [Shortcuts](#shortcuts).
- **Command palette and overview**: `Ctrl+K` lists actions, windows and groups (ranked as you type, each with its shortcut); `Alt+O` shows every window as a card with the last rows of its terminal, in a section per group, with a filter and drag and drop between groups.
- **Prompt chips** (opt-in): a row of context chips on the line above your prompt (time, user, directory, git branch and changes, conda/venv, pyenv, Kubernetes), chosen and ordered by you. See [Prompt chips](#prompt-chips).
- **Design board**: `design/board.html` shows frames of every screen (the window, the settings panel, the panes, the palette, the overview, the group dialog and menus, in English and Spanish) from the real renderer over different desktops, zoomed to the pixel and compared with a baseline (see [Design](#design)).
- **Small binary** (7.9 MiB, see [Benchmarks](#benchmarks)) with LTO, `panic=abort` and `strip`.

## Requirements

- Recent stable Rust (`cargo`, 2024 edition).
- Linux. On Wayland: `libxkbcommon.so.0` (loaded at run time for keyboard layouts; every desktop with Wayland has it). On X11, or without that library, axon falls back to `winit` + `wgpu`, which needs a Vulkan/GL driver.
- A monospaced font. By default it looks for, in this order: `~/.local/share/fonts/JetBrainsMono[wght].ttf`,
  `CascadiaMonoNF.ttf`, `CascadiaMono.ttf` and DejaVu Sans Mono. JetBrains Mono has no Nerd/Powerline icons: set
  `AXON_FONT` to a Nerd Font if your prompt uses them.

## Installation

```sh
scripts/install.sh                       # build and install for your user under ~/.local
scripts/install.sh --prefix /usr/local   # system-wide (run as root with --no-build after building as yourself)
scripts/install.sh --uninstall           # remove it again
```

It installs the binary (`bin/axon`), a desktop entry (so axon appears in your launcher) and the icon
(`assets/axon.svg`) under the prefix, and warns if the prefix is not in your `PATH` or no monospaced font is found.
`--no-build` reuses `target/release/axon`; `DESTDIR` is honoured for packaging.

## Usage

```sh
cargo run --release      # run
cargo build --release    # binary at target/release/axon
```

Always use `--release`: debug mode is much slower.

### Shortcuts

| Shortcut | Action |
|---|---|
| `Ctrl+K` | The command palette: type to filter windows, groups and actions (each with its shortcut), `Up` / `Down` move, `Enter` runs, `Esc` or `Ctrl+K` closes; it changes the windows' panel position too (`[windows] tabs`) |
| `Ctrl+,` | Open or close the settings panel docked beside the terminal (also the sliders icon in the title bar); `Esc` closes it when it has the keyboard |
| `Alt+N` / `Alt+W` | New window (a shell of its own) / close the window: the toast says so with an *Undo* button, and the shell stays alive until it is gone |
| `Alt+1…9`, `Alt+[` / `Alt+]` | Go to window *n* / the previous / the next one |
| `Alt+G` / `Alt+Shift+G` | Open the new-group dialog (name, colour, the windows to group, templates that open new windows; the window on screen is ticked when it is in no group; the panel's second header button, or the new-group button left of the gear when the windows are tabs, does the same) / take the window on screen out of its group |
| Right click on the panel or tabs | Menu of what is under the pointer: a window (rename, duplicate, split, groups, close, close the others), a group's header (edit, fold, new window, dissolve, close), or the empty part (new window, new group) |
| Double click on a window's row or tab / `F2` over it | Rename it in place: `Enter` keeps the name, `Esc` cancels, an empty name goes back to the automatic one. A double click on a group's header edits the group |
| `Ctrl+Space` | Split: one more terminal in the group of the window on screen, shown beside it (to the right first, then an automatic grid). A window in no group is put in a new group first. A split that would leave a pane under 20 columns x 5 rows is refused, and no group is made |
| `Alt+\` / `Alt+-` | The same split, with the group's windows in columns / in rows (mixing the two gives the grid) |
| `Alt+,` / `Alt+.` | Focus the previous / the next window of the group (a click on a pane focuses it too) |
| `Alt+Shift+W` | Close the focused pane, which is a window of the group (with the *Undo* toast; a shell that exits closes its window); the cross at a pane's corner and the cross of its row in the panel do the same |
| `Alt+O` | The overview of windows: every window as a card (the last rows of its terminal, status, name, directory) in a section per group; type to filter (name, directory, group or terminal text), arrows or `Tab` to move, `Enter` or a click to go to a window, `Alt+N` for a new one, `Esc` clears the filter and then closes it |
| Drag a card in the overview | Drop it on a group's section to put the window into the group (on the *No group* one to take it out), or on the dashed tile to make a group with it; `Esc` puts it back |
| Drag a row or a tab | Reorder windows: drop it before or after another (it takes that window's group), on a group's header or chip to put it into the group, on the empty part of the list to take it out of its group. Drag a group's header or chip to move the group. `Esc` cancels. A status icon says what the shell is doing: the prompt, a turning ring while a command runs, a red dot when the last one failed (both need the shell's hook: `axon shell-init`; bash gets it from axon) |
| `Alt+B` | Show or hide the panel of windows (`[windows] tabs = "left" \| "right" \| "top"`, `side`, `side_width`); the button right of the window buttons does the same |
| `Ctrl+Shift+C` / `Ctrl+Shift+V` | Copy / paste |
| `Ctrl+Shift+Q` | Quit |
| `Ctrl` + `+` / `-` / `0` | Increase / decrease / reset font size |
| `Shift+PageUp/PageDown/Home/End` | Navigate the scrollback |
| `Ctrl+Shift+R` | Reload `config.toml` (fonts, spacing, opacity, blur, status bar, language, cursor, prompt chips, the windows' panel) |
| Mouse wheel | Scroll (sends arrow keys on the alternate screen; wheel reports when the program asks for the mouse); over the tabs at the top it scrolls them, a group or a window at a time |
| Middle click on a window's row or tab | Close the window (with the *Undo* toast) |
| Mouse in vim, tmux, htop, less | Clicks, drags, motion and wheel go to the program; hold `Shift` to select text anyway (`wl` backend) |
| Click, double click, triple click | Selection: simple, word, line |
| Drag the title bar, double click it | Move the window, maximize / restore |
| Title bar buttons (left): close, minimize, maximize | Window controls (grey; they turn red, yellow and green under the pointer) |
| Drag from an edge or corner, or just outside it | Resize the window |
| `Alt` + drag | Move the window (`winit` fallback only, it has no title bar) |

### Settings panel

Click the **sliders icon** at the right end of the title bar, or press **`Ctrl+,`**. A panel opens at the edge of the window
and the terminal gets narrower to make room (every window is resized); nothing is modal, so the terminal and the title bar keep
working beside it. With the windows' panel at the left the settings are at the right; **with it at the right they go to the left**.
It has three tabs. **Font**: the font (a picker over the installed monospaced fonts), size, weight, bold text, letter spacing, line
height, window opacity and compositor blur. **Input**: the input type (shell prompt or prompt chips), the clock format, the cursor
shape (block, bar or underline), a switch per chip (see [Prompt chips](#prompt-chips)) and **Autosuggestions** (`[input] autosuggest`: read
and saved already, the editor that draws the suggestion comes later). **Windows**: where the list of windows lives (left, right or top),
whether its panel is visible, its width, and the interface **language** (auto, English or Español). Every change shows on the real
terminal beside the panel, so you judge the text by eye, and **is saved at once** to `~/.config/axon/config.toml` (your comments stay,
only what you changed is written; a slider writes when you let go, a key held down writes once). There is no Save and no Cancel:
**Reset to defaults** at the bottom puts every setting back. A failed write is written next to that button.

Keys (when the panel has the keyboard: it does when it opens or is clicked, and the terminal has it after you click there): arrows and
`Tab` move between rows (the ones that do nothing now are skipped), left and right change a value (`Shift`: five steps; on the chip
grid they move the cursor; radios wrap), `Space` or `Enter` flip the switch, pick the next radio or switch the chip, `Alt+Left` /
`Alt+Right` move a chip, `PageUp` / `PageDown` or `Ctrl+Tab` change tab, the wheel scrolls the page, `Esc` closes the panel. The
app's own shortcuts (`Alt+...`) keep working.

**Typing a number:** click the number at the right of a slider (or select its row and just start typing). It becomes a
box: type digits, a `.` or `,` for decimals and a leading `-`; `Enter`, `Tab`, or a click elsewhere keeps it, `Esc`
drops it. A typed number is not limited to the slider's span, only to the file's limits (size 6 to 96, weight 100
to 900, spacing -8 to 32, line height 0.8 to 3, opacity 0.3 to 1); weights snap to tens.

![axon settings panel](assets/settings.png)

### Prompt chips

`[input] mode = "chips"` (or **Input type → Prompt chips** in the settings panel) draws a row of rounded chips on the
line right above the prompt you are typing at, in the style of Warp's prompt chips. They sit exactly where you type, move
with the prompt, and disappear while a program runs. The shell still edits the line: axon does not replace your prompt,
it adds context above it. **The chips replace your prompt's own text** (`user@host:~/dir`) with a bare `$`, since they say it already; your `PS1` is kept and comes back when you switch to *Shell prompt*, and the window title is still set. Chips are drawn at **90 % of the font size** set in the settings (a 14 px font gives 12.6 px chips: text, icon, padding and height all follow), with outline icons, from the same font file, rasterized on the fly through one reused buffer, so they keep no tiles or caches.

![axon prompt chips](assets/input-bar.png)

| Chip | Shows | Comes from |
|---|---|---|
| `time` | `03:48 pm` (or `15:48`, `clock = "24h"`) | the clock; the only wake-up while idle, once a minute |
| `user`, `host` | `alice`, `alice@machine` | `$USER`, `/proc/sys/kernel/hostname` |
| `dir` | `~/Desktop` (the last three parts if longer) | `/proc/<shell>/cwd`: right with no help from the shell |
| `branch` | `main`, or a short hash when detached | `.git/HEAD`, worktrees and submodules included |
| `changes` | `3 • +10 -2`: files, lines added, lines removed | `git diff HEAD --numstat`, run by the shell's hook at each prompt (with the repository's own hooks and filters off, under `timeout 0.5`), so it needs the hook (`axon shell-init`; bash gets it from axon) |
| `pyenv` | `3.12.1` | `$PYENV_VERSION`, else the nearest `.python-version`, else `~/.pyenv/version` |
| `kube` | `prod-eu` | `current-context` of `$KUBECONFIG` or `~/.kube/config` |
| `env` | the conda environment or virtualenv | the shell's own variables, through the hook below |

A chip with nothing to say is skipped. **Order:** in the settings panel (Input tab) drag a chip to a new slot to reorder it, drag it into the greyed area to switch it off (and back to switch it on), or focus it and press `Alt+Left` / `Alt+Right`; a click still toggles it. The order is the `chips = [...]` list, so it is saved with the file and survives page changes and context updates. The list decides which appear and in what order; chips that
do not fit the window width are shortened with `…` and then dropped from the end. They are re-read when the terminal
prints (at most twice a second), never on the UI thread.

**How axon finds your prompt.** axon cannot see where the shell draws its prompt, so the shell marks it: while the chips
are on, the prompt starts with a line of its own holding one concealed space (whose colour is the prompt's number) and
ends with another where you type. axon finds those marks, so the chips stay on the line of every prompt, in the
scrollback too, with what they said when you typed there. The marks are invisible, count as one column and copy as
spaces. **axon starts bash with the
hook already in `PROMPT_COMMAND`**, so it works with nothing to configure and switches on and off at the next prompt
without a restart. For zsh, fish, or a bash whose rc file replaces `PROMPT_COMMAND`, add one line to the rc file:

```sh
eval "$(axon shell-init bash)"          # ~/.bashrc;  zsh: ~/.zshrc;  fish: axon shell-init fish | source
```

**How long each command took.** At the right end of a prompt's chips, once its command has finished, axon shows a green check
or a red cross with the exit status (when it is not 0) and the time: `12ms`, `1.4s`, `2m 5s`. The hook tells axon when a
command starts and when the next prompt appears, through window-title escapes that axon swallows (they never reach your
title bar), and axon times them as it reads them. The last 256 prompts are remembered; older rows keep an empty line.
The hook also tells axon your conda/venv, `PYENV_VERSION` and `KUBECONFIG` (only the shell knows them). A prompt
framework that rewrites `PS1` on every prompt (starship, oh-my-posh) may need the hook after it. The chips are on the `wl`
backend only. The terminal beside the settings panel is the preview.

### Language

The window's text (settings panel, status bar hints) is available in **English and Spanish**. `[ui] language`
is `auto` (the default: Spanish when `LC_ALL`, `LC_MESSAGES` or `LANG` starts with `es`, English otherwise), `en` or `es`;
the Language row of the settings panel's Windows page changes it live. The config file, the command line and the shell hooks stay
in English. Strings live in `src/i18n.rs` (ADR 0022).

### Configuration file

The settings panel writes the file for you as you change things; it can also be edited by hand. axon reads `~/.config/axon/config.toml`
(or `$XDG_CONFIG_HOME/axon/config.toml`) if it exists; every setting is optional. **Create it from the terminal with `axon config`**: it writes the file with every setting, commented, and never
overwrites an existing one.

| Command | Does |
|---|---|
| `axon config` | Creates the file if it is missing and prints where it is |
| `axon config edit` | The same, then opens it in `$VISUAL` or `$EDITOR` |
| `axon config show` | Prints the settings in effect (file plus environment) and the mistakes in the file |
| `axon config path` | Prints the path |
| `axon shell-init bash\|zsh\|fish` | Prints what to add to the shell's rc file so the prompt chips work in it |
| `axon --help`, `axon --version` | Usage, version |
 [`assets/config.example`](assets/config.example) lists them all, commented, and `scripts/install.sh` puts a
copy in `share/doc/axon/`. **`Ctrl+Shift+R` applies the file to the running window.** A mistake is printed to stderr
with its line number and skipped; a number out of range is clamped.

```toml
[font]
family = "JetBrains Mono"   # a family name, or the path of a .ttf / .otf
size = 15                   # pixels, 6 to 96
weight = 400                # 100 thin ... 900 black (synthesized for now)
bold = true                 # bold text at weight + 300; false draws it regular
letter_spacing = 0          # extra pixels between letters, -8 to 32
line_height = 1.0           # multiple of the font's own height, 0.8 to 3
smoothing = "rgb"           # "rgb" or "bgr" (LCD sub-pixels, match your panel) or "gray"
hinting = true              # snap flat tops, x-height and baseline to whole pixel rows

[window]
opacity = 0.88              # 0.3 to 1
blur = false                # ask the compositor to blur behind the glass (KDE, ext-background-effect; not GNOME)
status_bar = true           # the 28 px bar at the bottom: shell, encoding, branch, key hints, grid size

[ui]
language = "auto"           # "auto" (your locale), "en" or "es": the language of the window's text

[input]
mode = "shell"              # "shell" leaves the prompt as it is; "chips" puts the chips above it
clock = "12h"               # "12h" (03:48 pm) or "24h"
cursor = "block"            # "block", "bar" or "underline" until a program asks for another (it does not blink)
autosuggest = true          # suggest the rest of a command while typing (read and saved; the editor that shows it comes later)
chips = ["time", "user", "env", "pyenv", "host", "dir", "branch", "changes", "kube"]   # shown left to right

[windows]
tabs = "left"               # where the list of windows lives: "left" or "right" (a panel), or "top" (tabs in the title bar)
side = true                 # whether the panel is shown (Alt+B toggles it)
side_width = 220            # the panel's width in pixels, 160 to 320
```

The file is a small subset of TOML read by our own parser (`src/config.rs`): sections, `key = value`, `#` comments,
strings, numbers and booleans. The Weight is synthesized because the font reader (`fontdue`) cannot read the variable
axis of a font like `JetBrainsMono[wght].ttf`; the own font reader (task 12) will use the real axis.

### Environment variables (win over the file)

| Variable | Default | Description |
|---|---|---|
| `AXON_FONT` | — | Path to a monospaced `.ttf`/`.otf` font |
| `AXON_FONT_SIZE` | `15` | Font size in logical pixels |
| `AXON_OPACITY` | `0.93` (`winit`), `0.88` (`wl`) | Opacity of the glass body (if the compositor supports transparency) |
| `AXON_DECORATIONS` | off | `1` to use the system title bar (`winit` fallback only) |
| `AXON_BACKEND` | `wl` on Wayland, else `winit` | `wl`: own Wayland client + CPU renderer (~15 MB idle). `winit`: `winit` + `wgpu` (~150 MB idle), also the automatic fallback if `wl` cannot start |
| `AXON_SETTINGS` | — | A debugging aid: start with the settings panel open (screenshots, measurements) |
| `AXON_WINDOWS` | `1` | A debugging aid: start with *n* windows (up to 9) |
| `AXON_PANES` | `1` | A debugging aid: start with *n* terminals split in a group around the first window (up to 12) |
| `AXON_GROUPS` | — | A debugging aid: set (any value) with `AXON_WINDOWS` of 2 or more, the first two windows start in one group |
| `AXON_RUNNING` | — | A debugging aid: every window counts as running a command (the turning ring, the folded groups' dots) |
| `AXON_OVERVIEW` | — | A debugging aid: start with the overview of windows open |
| `AXON_LIST_UI` | — | A debugging aid: start with one of the window list's screens open: `dialog`, `edit`, `menu`, `group-menu`, `strip-menu`, `rename`, `close` or `toast` (`edit` and `group-menu` need `AXON_GROUPS`) |
| `AXON_DEMO_OVERLAY` | — | A debugging aid: start with one of the overlay kit's screens open (`field`, `list`, `menu`, `toast`, `modal`), or `palette` for the real command palette |
| `AXON_WL_DEBUG` | — | A debugging aid: trace the Wayland protocol on stderr |

## Design

The look is designed on real frames, not mockups. `src/design.rs` renders the window through the actual
renderer in a fixed set of scenes (states, palette, opacity variants) and writes PNGs with alpha plus the design
tokens; [`design/board.html`](design/board.html) puts them over several desktops (light, dark, a page of text,
stripes, checker), zooms to device pixels (1×, 2×, 4×) and compares against a baseline, side by side or split.

```sh
scripts/design.sh --baseline   # keep the current frames as the baseline
# ... change chrome.rs, palette.rs, soft.rs ...
scripts/design.sh              # render the new frames, then open design/board.html
```

The frames are build output (`design/frames/`, `design/baseline/`, git-ignored). `assets/screenshot.png` is a copy of
`design/frames/rest.png`.

## Benchmarks

Measured with [`scripts/bench.sh`](scripts/bench.sh) (`--clean` adds a cold build). Release profile: fat LTO,
`codegen-units = 1`, `panic = abort`, `strip`. Window 1000x640, idle shell. **Updated on every change that
can move size or speed** (rule in `CLAUDE.md`); older numbers stay in the history table.

Machine: AMD Ryzen 5 5600H (12 threads) + RTX 3050 Mobile, Ubuntu, kernel 7.0.0-44, GNOME Shell 50.1 on Wayland.
Last measured: 2026-11-25, at `169bda9` plus the change that makes a group the split (ADR 0034): everything since `02c3490` (mouse reporting, the config file, the prompt chips, the blur request, the sub-pixel text rasterizer, the UI after `terminal.html`) plus the second reference: windows and groups (side panel or top tabs), split panes, the overlay kit, the docked settings panel, the command palette, the overview of windows, the group dialog, context menus, inline rename, toasts with undo, drag and drop, status icons, and the fixes after the review. The machine was idle (`vmstat`: 96-99 % idle; a browser open).

**Build**

| Binary | Cold release build |
|---|---|
| 7.9 MiB (8,302,768 bytes) | 101 s |

**Idle memory and CPU** (`AXON_BACKEND` selects the backend; `wl` is the own Wayland client with the CPU renderer and the
default on Wayland, `winit` is the `winit` + `wgpu` fallback)

| Backend | Font | RSS | PSS | Private (anon) | CPU |
|---|---|---|---|---|---|
| `wl` | JetBrains Mono (default) | **15 MB** | 11 MB | 6 MB | 0 ticks / 3 s |
| `wl` | DejaVu Sans Mono | 21 MB | 17 MB | 11 MB | 0 ticks / 3 s |
| `wl` | Cascadia Mono NF | 59 MB | 55 MB | 49 MB | 0 ticks / 3 s |
| `winit` | JetBrains Mono (default) | 148 MB | 139 MB | 31 MB | 0 ticks / 3 s |
| `winit` | DejaVu Sans Mono | 154 MB | 145 MB | 37 MB | 0 ticks / 3 s |
| `winit` | Cascadia Mono NF | 189 MB | 180 MB | 74 MB | 0 ticks / 3 s |

The idle target is under 50 MB of RSS. `wl` meets it with the default font; a Nerd Font such as Cascadia Mono NF costs
~45 MB inside `fontdue` (it parses every glyph), which the own font reader will remove. The `winit` backend cannot meet
the target: the Vulkan loader and the drivers map ~94 MB. (`winit`'s PSS went from 56 to 96 and now 139 MB with the RSS
unchanged: PSS divides shared pages among every process that maps them, so it follows what else runs on the desktop.)

**Windows, groups and screens** (`wl`, default font, idle shell in every window; the debugging knobs `AXON_WINDOWS`,
`AXON_PANES`, `AXON_SETTINGS`, `AXON_OVERVIEW`)

| State | RSS | Private (anon) | CPU |
|---|---|---|---|
| One window | 14.8 MB | 5.7 MB | 0 ticks / 5 s |
| Three windows (grouped or not) | 17.0 MB | 8.0 MB | 0 ticks / 5 s |
| A group of four windows shown split (`AXON_PANES=4`) | 18.0 MB | 9.1 MB | 0 ticks / 5 s |
| Settings panel open | 23.1 MB | 14.1 MB | 0 ticks / 5 s |
| Overview of windows open (one window) | 19.8 MB | 10.7 MB | 0 ticks / 5 s |
| Three windows, each marked as running (the status spinners turn) | 17.0 MB | 8.0 MB | 3 ticks / 5 s |

A shell killed with `SIGKILL` closes only its own window; when the last one goes, axon quits and removes its report files.

**Throughput** (`cat` of 99.6 MB of text lines)

| Backend | Time | RSS afterwards |
|---|---|---|
| `wl` | 1.65 s | 37 MB |
| `winit` | 1.35 s | 177 MB |

Idle machine, four runs of `scripts/bench.sh` in this stretch (`wl` 1.58, 1.57, 1.63 and 1.65 s, `winit` 1.33, 1.30, 1.38 and 1.35 s):
run-to-run noise is about 0.1 s, and the last two are within it of the first two. The 1.95-2.08 s of the title bar build was the machine (a browser held the load average at 3-4), not the code.

Not measured yet: startup time, input latency, and a comparison with Alacritty, Ghostty and foot (not installed here).

**History** (binary size and idle RSS with the default font; newest first)

| Date | Change | Binary | Idle RSS `winit` | Idle RSS `wl` |
|---|---|---|---|---|
| 2026-11-25 | A group is the split (ADR 0034): a group's windows are shown side by side, `Ctrl+Space` makes the group; no new dependency, +8.2 KB over the row below; a group of four windows 18 MB | 8,302,768 B | 148 MB | 15 MB |
| 2026-11-25 | The second reference (`a44df93`): groups, split panes, overlay kit, docked settings, command palette, overview, group dialog, menus, rename, toasts, drag and drop, status icons, and the review fixes; +561 KB over the title bar row; three windows 17 MB, settings open 23 MB, overview open 20 MB | 8,294,576 B | 147 MB | 15 MB |
| 2026-11-24 | Title bar after the reference: panel button, name and place as the title, windows as tabs at the top (`tabs = "top"`) | 7,733,936 B | 149 MB | 15 MB |
| 2026-11-24 | Windows and the side panel (`Win` per shell, `side.rs`, `[windows]`, `Alt+N/W/1-9/[/]/B`, ADR 0023); one window idle 15 MB, three windows ~18 MB | 7,730,960 B | 150 MB | 15 MB |
| 2026-11-24 | The changes chip comes from the shell's hook (`git diff` with the repository's hooks and filters off, `timeout 0.5`), axon no longer runs git: `safe.directory` works, the 3 s timer and the git thread are gone | 7,694,984 B | 150 MB | 15 MB |
| 2026-11-24 | Interface in English and Spanish (`i18n.rs`, `[ui] language`, a Language row in the settings) | 7,694,728 B | 150 MB | 15 MB |
| 2026-11-24 | Per-command status (`history.rs`): the hook's title escapes timed on the PTY thread, a ring of 256 prompt records, chips kept on every prompt with a check or cross and the duration | 7,694,728 B | 148 MB | 15 MB |
| 2026-11-24 | UI after `terminal.html` (ADR 0021): new palette, hairlines, coloured window buttons, status bar, outline chip icons from the reference, settings with live preview, sliding tabs and cursor shape | 7,694,728 B | 145 MB | 14 MB |
| 2026-11-24 | Text quality: 6x supersampled glyphs with vertical hinting and LCD sub-pixel coverage (`font.rs`, `soft.rs`), settings in a proportional font with fades, draggable chip order; the settings screen open costs ~22 MB RSS | 7,617,368 B | 148 MB | 14 MB |
| 2026-11-24 | Prompt chips at 80 % of the font size (later 90 %, outline icons) (`Fonts::small`, `Soft::paint_text`): text rasterized on the fly, icons scaled | 7,585,040 B | 149 MB | 14 MB |
| 2026-11-24 | Window blur (`[window] blur`, settings switch): `ext-background-effect-v1` and KDE blur protocols in `wl.rs`; GNOME 50 offers neither | 7,586,856 B | 149 MB | 14 MB |
| 2026-11-24 | Prompt chips (`bar.rs`, `context.rs`, `paint.rs`): context chips on the line above the prompt, Input tab in the settings screen, `axon shell-init`; idle with the chips on 14.9 MB, flood 1.74 s with them | 7,583,032 B | 149 MB | 15 MB |
| 2026-11-24 | Watchdog: a redraw stuck on a lost frame callback or buffer release goes on after 250 ms (suspend and resume froze the window) | 7,498,904 B | 146 MB | 15 MB |
| 2026-11-23 | Settings screen: numbers can be typed into boxes | 7,499,608 B | 149 MB | 14 MB |
| 2026-11-23 | Settings screen (`settings.rs`, gear, sfnt reader, `config::save`); +5 MB RSS while it is open | 7,483,864 B | 150 MB | 14 MB |
| 2026-11-23 | Config file (`config.rs`): font family, size, weight, letter spacing, line height, opacity; `axon config` creates it, `Ctrl+Shift+R` reloads | 7,409,792 B | 150 MB | 14 MB |
| 2026-11-23 | Mouse reporting (`mouse.rs`: modes 1000/1002/1003, SGR, UTF-8, classic) | 7,370,048 B | 149 MB | 14 MB |
| 2026-11-23 | Continuous glass (no title pane, sheen or rim gradient), background-coloured cells are glass | 7,366,560 B | 149 MB | 13 MB |
| 2026-11-23 | Own window shadow (margin, window geometry, input region), opacity 0.88, design board | 7,366,064 B | 149 MB | 14 MB |
| 2026-11-23 | Sharper text (plain source-over, coverage contrast) and pixel-aligned buttons | 7,362,960 B | 155 MB | 14 MB |
| 2026-11-23 | Stage C: keyboard (`xkb.rs`), clipboard, wheel, selection; `wl` is the default backend | 7,363,488 B | 150 MB | 14 MB |
| 2026-11-23 | JetBrains Mono becomes the default font | 7,332,888 B | 149 MB | 13 MB |
| 2026-11-22 | `b30bc13`: glass chrome (`chrome.rs`), pointer input, black-and-white palette (Cascadia NF default) | 7,332,888 B | 196 MB | 55 MB |
| 2026-11-22 | `8db3cc9`: own Wayland client, `view.rs`, `session.rs` | 7,325,272 B | 191 MB | 54 MB |
| 2026-11-22 | `0b72197`: CPU renderer `soft.rs`, `cell.rs`, `font.rs` (not wired in) | 7,197,832 B | 191 MB | n/a |
| 2026-11-22 | `eae69b5`: initial commit (damage tracking, cell buffer, lazy fonts) | 7,197,400 B | 191 MB | n/a |
| 2026-11-22 | Before the lazy-font fix (never committed) | n/a | 946 MB | n/a |

## Architecture

```
src/
├─ main.rs      picks the backend; the `winit` + `wgpu` fallback app
├─ cell.rs      Cell format, flags, tile contract shared by the renderers
├─ font.rs      fonts, glyph table, special tiles (shared by the renderers)
├─ gpu.rs       wgpu, atlas, pipelines
├─ soft.rs      CPU renderer into an ARGB framebuffer
├─ chrome.rs    window frame: glass look, rounded corners, title bar, buttons, shadow, hit testing
├─ palette.rs   the colour tokens (amber accent, six hues) and ANSI resolution
├─ side.rs      the list of windows (panel or tabs): layout, groups, status icons, drag marks, painting
├─ dnd.rs       drag and drop in that list: what a drop means and the new order (pure)
├─ panes.rs     the split layouts (columns, rows, grid), focus order, the 20 x 5 minimum (pure); a group's windows are the panes
├─ paint.rs     antialiased shapes composited into the framebuffer (settings panel, prompt chips)
├─ bar.rs       the prompt chips: layout, icons, painting over the reserved prompt line (grid's own font)
├─ i18n.rs      the interface's strings in English and Spanish (`tr`), the language setting
├─ history.rs   each prompt's chips as they were, and how long its command ran (the hook's title escapes)
├─ context.rs   what the chips say: directory, git, Python, Kubernetes, time (read on a thread)
├─ settings.rs  the settings panel docked beside the grid (Font, Input and Windows pages)
├─ overlay.rs   the overlay kit: the `Overlay` trait, the stack that takes keys and pointer, the modal frame, text
├─ field.rs     a line of text being edited (caret, selection, paste) and its painting
├─ list.rs      filtered, scrolling list model and its painting (the palette's)
├─ commands.rs  what the command palette lists (actions, windows, groups) and what each entry means; pure
├─ picker.rs    the command palette's overlay: input row, list, footer (`Ctrl+K`)
├─ groups.rs    groups of windows: the dialog's form check, names, colours, what a close puts back, templates (pure)
├─ group_dialog.rs  the group dialog (`Alt+G`), an overlay
├─ ctxmenu.rs   the context menus of a window, a group and the empty list
├─ menu.rs      the context menu
├─ toast.rs     the toast (with an *Undo* button)
├─ icons.rs     the overlays' outline icons
├─ overlay_demo.rs  the kit's debug screens (`AXON_DEMO_OVERLAY`)
├─ overview.rs  the overview of windows (`Alt+O`): cards, sections, filter, keyboard and pointer, an overlay
├─ config.rs    the settings file: parser, writer, `axon config`, `axon shell-init`
├─ view.rs      grid → cells for any renderer (damage, cursor, selection)
├─ session.rs   Term + PTY thread
├─ wl.rs        own Wayland client: wire protocol, window, shm buffers
├─ wlapp.rs     the default app on wl.rs + soft.rs: windows (a group's are shown split), keyboard, pointer, clipboard, one poll loop
│  ├─ groups_ui.rs         windows, groups, menus, rename and undo: what owns the shells
│  └─ windows_overview.rs  the app's side of the overview: cards from the windows, the terminals' text for the previews
├─ xkb.rs       keyboard layouts through libxkbcommon (dlopen)
├─ input.rs     Keys → xterm sequences (pure)
├─ mouse.rs     mouse reports for programs (xterm 1000/1002/1003, SGR, UTF-8, classic; pure)
├─ design.rs    (tests) design board frames, own PNG encoder, incremental-repaint check
└─ shader.wgsl  Cell and background shaders
design/board.html   the design board (frames from scripts/design.sh)
```

Flow: `wl_keyboard → xkb → input → PTY → Term (alacritty) → coalesced Wakeup → View::build → soft → wl_shm buffer`.

The architecture, dependency rules, performance budgets and modularization roadmap are documented
as Claude Code skills in [`.claude/skills/`](.claude/skills):

| Skill | Contents |
|---|---|
| `axon-architecture` | Layers, data flow, dependency rules, crate plan |
| `axon-performance` | Budgets (CPU, latency, RAM) and how to measure them |
| `axon-renderer` | Atlas, cell buffer format, shader, wgpu 30 notes |
| `axon-extend` | Recipes: shortcuts, config, themes, mouse, new platforms |
| `axon-design-language` | Visual identity and what to avoid |

Plan, decisions (ADRs), risks and measurements live in the knowledge tree at
[`knowledge/`](knowledge/INDEX.md). Summary in [`CLAUDE.md`](CLAUDE.md).

## Status and known limitations

- Mouse reports reach applications on the `wl` backend only; the `winit` fallback keeps local selection.
- Text is drawn with sub-pixel (LCD) coverage and vertical hinting by our own rasterizer (ADR 0020), tuned for an RGB panel; set `smoothing = "gray"` for a rotated or OLED-triangle screen. Fallback fonts (CJK, symbols) are grey and unhinted. The settings panel uses a proportional UI font (+7 MB RSS while open).
- Synthesized bold; no italics or ligatures; no color emoji.
- Blur: a request to the compositor, not our own effect; GNOME (Mutter) offers none, so there it does nothing. Settings: fonts, opacity, blur, the prompt chips and the windows' panel (settings panel or `config.toml`); colours, shortcuts and themes are not configurable yet. The weight is synthesized.
- Input: the chips sit above the prompt, but the line is still edited by the shell. An own input editor (Warp's "Input type") is the next step, not built yet. The chip colours are fixed; the chips are `wl` only, and zsh and fish need the `axon shell-init` line (and keep their own prompt text).
- Windows, groups and their names are not saved between runs; a split's panes cannot be resized by dragging yet; the overview has no context menus (ADR 0033 lists what is left out on purpose).
- Tested only on Linux/Wayland.

## License

To be decided.
