# axon

Native terminal emulator: **simple, minimalist, high-performance and futuristic**.
Written in Rust, for Wayland and X11. GPU-rendered (wgpu) today; moving to a CPU renderer on `wl_shm` to cut idle memory (see `knowledge/decisions/0011-software-render-wl-shm.md`).

- **GPU rendering**: glyph atlas + one fixed cell per grid position, one draw call for the grid and another for the background.
- **Zero idle usage**: no timers or continuous redraw; it only paints when there are events.
- **Full VT emulation** thanks to [`alacritty_terminal`](https://crates.io/crates/alacritty_terminal).
- **Black-and-white glass look**: rounded translucent window, a title bar with round close/minimize/maximize buttons, white text on near-black glass, muted ANSI hues. The frame is computed per pixel (`src/chrome.rs`), no images. On the `wl` backend; the `winit` backend only has the black-and-white colours.
- **Small binary** (7.0 MiB, see [Benchmarks](#benchmarks)) with LTO, `panic=abort` and `strip`.

## Requirements

- Recent stable Rust (`cargo`, 2024 edition).
- Linux with Wayland or X11 and a Vulkan/GL driver supported by wgpu.
- A monospaced font. By default it looks for, in this order: `~/.local/share/fonts/CascadiaMonoNF.ttf`,
  `JetBrainsMono[wght].ttf`, `CascadiaMono.ttf` and DejaVu Sans Mono.

## Usage

```sh
cargo run --release      # run
cargo build --release    # binary at target/release/axon
```

Always use `--release`: debug mode is much slower.

### Shortcuts

| Shortcut | Action |
|---|---|
| `Ctrl+Shift+C` / `Ctrl+Shift+V` | Copy / paste |
| `Ctrl+Shift+Q` | Quit |
| `Ctrl` + `+` / `-` / `0` | Increase / decrease / reset font size |
| `Shift+PageUp/PageDown/Home/End` | Navigate the scrollback |
| Mouse wheel | Scroll (sends arrow keys on the alternate screen) |
| Click, double click, triple click | Selection: simple, word, line |
| Drag the title bar, double click it | Move the window, maximize / restore (`wl` backend) |
| Title bar buttons (left): close, minimize, maximize | Window controls (`wl` backend) |
| Drag from an edge or corner | Resize the window |
| `Alt` + drag | Move the window (`winit` backend, no decorations) |

### Configuration (environment variables)

| Variable | Default | Description |
|---|---|---|
| `AXON_FONT` | — | Path to a monospaced `.ttf`/`.otf` font |
| `AXON_FONT_SIZE` | `15` | Font size in logical pixels |
| `AXON_OPACITY` | `0.93` (`winit`), `0.80` (`wl`) | Opacity of the glass body (if the compositor supports transparency) |
| `AXON_DECORATIONS` | off | `1` to use the system title bar |
| `AXON_BACKEND` | `winit` | `wl`: own Wayland client + CPU renderer, ~12-54 MB idle instead of ~190 MB (experimental: pointer only so far, no keyboard) |

## Benchmarks

Measured with [`scripts/bench.sh`](scripts/bench.sh) (`--clean` adds a cold build). Release profile: fat LTO,
`codegen-units = 1`, `panic = abort`, `strip`. Window 1000x640, idle shell. **Updated on every change that
can move size or speed** (rule in `CLAUDE.md`); older numbers stay in the history table.

Machine: AMD Ryzen 5 5600H (12 threads) + RTX 3050 Mobile, Ubuntu, kernel 7.0.0-44, GNOME Shell 50.1 on Wayland.
Last measured: 2026-11-22, on top of `8db3cc9` plus the glass chrome change (the commit that adds `chrome.rs`).

**Build**

| Binary | Cold release build |
|---|---|
| 7.0 MiB (7,332,888 bytes) | 95 s |

**Idle memory and CPU** (`AXON_BACKEND` selects the backend; `wl` is the own Wayland client with the CPU renderer,
`winit` is the default `winit` + `wgpu` one)

| Backend | Font | RSS | PSS | Private (anon) | CPU |
|---|---|---|---|---|---|
| `wl` | DejaVu Sans Mono | **18 MB** | 14 MB | 11 MB | 0 ticks / 3 s |
| `wl` | Cascadia NF (default) | 55 MB | 51 MB | 47 MB | 0 ticks / 3 s |
| `winit` | DejaVu Sans Mono | 155 MB | 100 MB | 37 MB | 0 ticks / 3 s |
| `winit` | Cascadia NF (default) | 196 MB | 141 MB | 74 MB | 0 ticks / 3 s |

The idle target is under 50 MB of RSS. `wl` meets it with a small font; Cascadia NF costs ~45 MB inside `fontdue`,
which the own font reader will remove. The `winit` backend cannot meet it: the Vulkan loader and the drivers map ~94 MB.

**Throughput** (`cat` of 99.6 MB of text lines)

| Backend | Time | RSS afterwards |
|---|---|---|
| `wl` | 1.70 s | 80 MB |
| `winit` | 1.30 s | 217 MB |

Not measured yet: startup time, input latency, and a comparison with Alacritty, Ghostty and foot (not installed here).

**History** (binary size and idle RSS with the default font; newest first)

| Date | Change | Binary | Idle RSS `winit` | Idle RSS `wl` |
|---|---|---|---|---|
| 2026-11-22 | Glass chrome (`chrome.rs`), pointer input, black-and-white palette | 7,332,888 B | 196 MB | 55 MB |
| 2026-11-22 | `8db3cc9`: own Wayland client, `view.rs`, `session.rs` | 7,325,272 B | 191 MB | 54 MB |
| 2026-11-22 | `0b72197`: CPU renderer `soft.rs`, `cell.rs`, `font.rs` (not wired in) | 7,197,832 B | 191 MB | n/a |
| 2026-11-22 | `eae69b5`: initial commit (damage tracking, cell buffer, lazy fonts) | 7,197,400 B | 191 MB | n/a |
| 2026-11-22 | Before the lazy-font fix (never committed) | n/a | 946 MB | n/a |

## Architecture

```
src/
├─ main.rs      winit app, window/session state, grid → cells
├─ cell.rs      Cell format, flags, tile contract shared by the renderers
├─ font.rs      fonts, glyph table, special tiles (shared by the renderers)
├─ gpu.rs       wgpu, atlas, pipelines
├─ soft.rs      CPU renderer into an ARGB framebuffer
├─ chrome.rs    window frame: glass look, rounded corners, title bar, buttons, hit testing
├─ view.rs      grid → cells for any renderer (damage, cursor, selection)
├─ session.rs   Term + PTY thread
├─ wl.rs        own Wayland client: wire protocol, window, shm buffers
├─ wlapp.rs     the app on wl.rs + soft.rs (`AXON_BACKEND=wl`)
├─ input.rs     Keyboard → xterm sequences (pure)
├─ palette.rs   Black-and-white palette and ANSI color resolution
└─ shader.wgsl  Cell and background shaders
```

Flow: `winit → input → PTY → Term (alacritty) → coalesced Wakeup → build() → render()`.

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

- Mouse events are not yet sent to applications (vim/tmux with mouse); selection is local.
- Synthesized bold; no italics or ligatures; no color emoji.
- No config file: environment variables only (see the roadmap in `axon-architecture`).
- Tested only on Linux/Wayland.

## License

To be decided.
