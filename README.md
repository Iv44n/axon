# axon

Native terminal emulator: **simple, minimalist, high-performance and futuristic**.
Written in Rust, for Wayland (own client, CPU-rendered into `wl_shm`, no GPU stack) with a `winit` + `wgpu` fallback for X11 until it gets its own client (see `knowledge/decisions/0011-software-render-wl-shm.md`).

![axon: black-and-white glass window with its shadow](assets/screenshot.png)

<sub>A frame from the real renderer (`scripts/design.sh`), with alpha: what you see behind it is this page.</sub>

- **Own Wayland client and CPU renderer**: the grid is painted straight into a shared-memory buffer, only the damaged rows. ~14 MB idle, no Vulkan or GL loaded.
- **Zero idle usage**: no timers or continuous redraw; it only paints when there are events.
- **Full VT emulation** thanks to [`alacritty_terminal`](https://crates.io/crates/alacritty_terminal).
- **Black-and-white glass look**: one continuous rounded translucent pane with a soft shadow; the title bar is part of the same glass (no separators), with round close/minimize/maximize buttons, white text on near-black glass, muted ANSI hues. The frame is computed per pixel (`src/chrome.rs`), no images. On the default backend; the `winit` fallback only has the black-and-white colours.
- **Design board**: `design/board.html` shows frames from the real renderer over different desktops, zoomed to the pixel and compared with a baseline (see [Design](#design)).
- **Small binary** (7.2 MiB, see [Benchmarks](#benchmarks)) with LTO, `panic=abort` and `strip`.

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
| `Ctrl+,` | Open or close the settings screen (also the gear in the title bar) |
| `Ctrl+Shift+C` / `Ctrl+Shift+V` | Copy / paste |
| `Ctrl+Shift+Q` | Quit |
| `Ctrl` + `+` / `-` / `0` | Increase / decrease / reset font size |
| `Shift+PageUp/PageDown/Home/End` | Navigate the scrollback |
| `Ctrl+Shift+R` | Reload `config.toml` (fonts, spacing, opacity) |
| Mouse wheel | Scroll (sends arrow keys on the alternate screen; wheel reports when the program asks for the mouse) |
| Mouse in vim, tmux, htop, less | Clicks, drags, motion and wheel go to the program; hold `Shift` to select text anyway (`wl` backend) |
| Click, double click, triple click | Selection: simple, word, line |
| Drag the title bar, double click it | Move the window, maximize / restore |
| Title bar buttons (left): close, minimize, maximize | Window controls |
| Drag from an edge or corner, or just outside it | Resize the window |
| `Alt` + drag | Move the window (`winit` fallback only, it has no title bar) |

### Settings screen

Click the **gear** at the right end of the title bar, or press **`Ctrl+,`**. A glass panel opens over the grid with the
font (a picker over the installed monospaced fonts), size, weight, bold text, letter spacing, line height and window
opacity. Everything changes the real window as you drag, so you judge the text by eye. **Save** writes
`~/.config/axon/config.toml` (your comments stay, only what you changed is written); **Cancel**, `Esc`, the gear or
`Ctrl+,` again put everything back. Keys: arrows and `Tab` move between rows, left and right change a value (`Shift`:
five steps), `Space` flips the switch, `Enter` saves.

**Typing a number:** click the number at the right of a slider (or select its row and just start typing). It becomes a
box: type digits, a `.` or `,` for decimals and a leading `-`; `Enter`, `Tab`, or a click elsewhere keeps it, `Esc`
drops it. A typed number is not limited to the slider's span, only to the file's limits (size 6 to 96, weight 100
to 900, spacing -8 to 32, line height 0.8 to 3, opacity 0.3 to 1); weights snap to tens.

![axon settings screen](assets/settings.png)

### Configuration file

The settings screen writes the file for you; it can also be edited by hand. axon reads `~/.config/axon/config.toml`
(or `$XDG_CONFIG_HOME/axon/config.toml`) if it exists; every setting is optional. **Create it from the terminal with `axon config`**: it writes the file with every setting, commented, and never
overwrites an existing one.

| Command | Does |
|---|---|
| `axon config` | Creates the file if it is missing and prints where it is |
| `axon config edit` | The same, then opens it in `$VISUAL` or `$EDITOR` |
| `axon config show` | Prints the settings in effect (file plus environment) and the mistakes in the file |
| `axon config path` | Prints the path |
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

[window]
opacity = 0.88              # 0.3 to 1
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
| `AXON_BACKEND` | `wl` on Wayland, else `winit` | `wl`: own Wayland client + CPU renderer (~14 MB idle). `winit`: `winit` + `wgpu` (~150 MB idle), also the automatic fallback if `wl` cannot start |

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
Last measured: 2026-11-24, on top of `02c3490` plus mouse reporting, the config file with the `axon config` command, the settings screen and the lost-frame watchdog (the cold build time is from `02c3490`).

**Build**

| Binary | Cold release build |
|---|---|
| 7.2 MiB (7,498,904 bytes) | 96 s |

**Idle memory and CPU** (`AXON_BACKEND` selects the backend; `wl` is the own Wayland client with the CPU renderer and the
default on Wayland, `winit` is the `winit` + `wgpu` fallback)

| Backend | Font | RSS | PSS | Private (anon) | CPU |
|---|---|---|---|---|---|
| `wl` | JetBrains Mono (default) | **15 MB** | 10 MB | 6 MB | 0 ticks / 3 s |
| `wl` | DejaVu Sans Mono | 20 MB | 15 MB | 11 MB | 0 ticks / 3 s |
| `wl` | Cascadia Mono NF | 58 MB | 53 MB | 49 MB | 0 ticks / 3 s |
| `winit` | JetBrains Mono (default) | 149 MB | 56 MB | 32 MB | 0 ticks / 3 s |
| `winit` | DejaVu Sans Mono | 155 MB | 60 MB | 37 MB | 0 ticks / 3 s |
| `winit` | Cascadia Mono NF | 192 MB | 96 MB | 74 MB | 0 ticks / 3 s |

The idle target is under 50 MB of RSS. `wl` meets it with the default font; a Nerd Font such as Cascadia Mono NF costs
~45 MB inside `fontdue` (it parses every glyph), which the own font reader will remove. The `winit` backend cannot meet
the target: the Vulkan loader and the drivers map ~94 MB.

**Throughput** (`cat` of 99.6 MB of text lines)

| Backend | Time | RSS afterwards |
|---|---|---|
| `wl` | 1.72 s | 43 MB |
| `winit` | 1.32 s | 173 MB |

Not measured yet: startup time, input latency, and a comparison with Alacritty, Ghostty and foot (not installed here).

**History** (binary size and idle RSS with the default font; newest first)

| Date | Change | Binary | Idle RSS `winit` | Idle RSS `wl` |
|---|---|---|---|---|
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
├─ view.rs      grid → cells for any renderer (damage, cursor, selection)
├─ session.rs   Term + PTY thread
├─ wl.rs        own Wayland client: wire protocol, window, shm buffers
├─ wlapp.rs     the default app on wl.rs + soft.rs: keyboard, pointer, clipboard, one poll loop
├─ xkb.rs       keyboard layouts through libxkbcommon (dlopen)
├─ input.rs     Keys → xterm sequences (pure)
├─ palette.rs   Black-and-white palette and ANSI color resolution
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
- Synthesized bold; no italics or ligatures; no color emoji.
- Settings: fonts and opacity only (settings screen or `config.toml`); colours, shortcuts and themes are not configurable yet. The weight is synthesized.
- Tested only on Linux/Wayland.

## License

To be decided.
