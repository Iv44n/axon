# axon

Native terminal emulator: **simple, minimalist, high-performance and futuristic**.
Written in Rust, GPU-rendered (wgpu), for Wayland and X11.

- **GPU rendering**: glyph atlas + one fixed cell per grid position, one draw call for the grid and another for the background.
- **Zero idle usage**: no timers or continuous redraw; it only paints when there are events.
- **Full VT emulation** thanks to [`alacritty_terminal`](https://crates.io/crates/alacritty_terminal).
- **Futuristic look**: translucent gradient background, cyan→violet glowing border, neon ANSI palette, no title bar.
- **Small binary** (~7 MB) with LTO, `panic=abort` and `strip`.

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
| `Alt` + drag | Move the window (no decorations) |
| Drag from an edge | Resize the window |

### Configuration (environment variables)

| Variable | Default | Description |
|---|---|---|
| `AXON_FONT` | — | Path to a monospaced `.ttf`/`.otf` font |
| `AXON_FONT_SIZE` | `15` | Font size in logical pixels |
| `AXON_OPACITY` | `0.93` | Background opacity (if the compositor supports transparency) |
| `AXON_DECORATIONS` | off | `1` to use the system title bar |

## Architecture

```
src/
├─ main.rs      winit app, window/session state, grid → cells
├─ gpu.rs       wgpu, fonts, atlas, pipelines
├─ input.rs     Keyboard → xterm sequences (pure)
├─ palette.rs   Neon palette and ANSI color resolution
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
