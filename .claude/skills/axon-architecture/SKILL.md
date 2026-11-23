---
name: axon-architecture
description: axon's architecture (GPU terminal in Rust) - layers, data flow, dependency rules and modularization plan. Use it before adding modules, moving code or deciding where a new feature lives.
---

# axon architecture

## Data flow (one direction)

```
keyboard/mouse ─► winit ─► input.rs ─► bytes ─► PTY (alacritty thread)
                                                   │ reads output
                                   Term<Proxy> (FairMutex) ◄┘  parses VT, updates grid
                                        │ Event::Wakeup (coalesced by AtomicBool)
                                        ▼
winit user_event ─► request_redraw ─► State::build() ─► Gpu cell buffer ─► Gpu::render()
```

- The PTY thread belongs to `alacritty_terminal::EventLoop`; the UI thread only locks the `FairMutex`
  during `build()` (walking the visible cells) and `resize/selection`.
- One direction: UI -> session (bytes, resize); session -> UI (`Ev` events). Never cross calls
  from the PTY thread into the UI except `Proxy::send_event`.

## Current module map (`src/`)

| Module | Responsibility | May depend on |
|---|---|---|
| `main.rs` | winit app, `State` (window+session+mouse); picks the backend | all |
| `cell.rs` | `Cell`, `F_*` flags, `T_*` special tiles, `pack`, the `Glyphs` trait | bytemuck |
| `font.rs` | `Fonts` (fallbacks, rasterizing), `GlyphTable`, special tile bitmaps | fontdue |
| `gpu.rs` | wgpu, atlas, pipelines, `render()` (to be deleted: [0011](../../../knowledge/decisions/0011-software-render-wl-shm.md)) | winit (`Window` only), `cell`, `font` |
| `soft.rs` | CPU renderer into an `Argb8888` framebuffer; the replacement for `gpu.rs` | `cell`, `chrome`, `font`, `view` (trait only) |
| `chrome.rs` | window frame on the CPU: glass look, rounded corners, title bar and buttons per pixel, `hit` testing, `Ui` state | — |
| `view.rs` | grid -> cells: `View::build(term, focused, &mut impl Backend)`, damage, cursor and selection rows | `cell`, `palette`, alacritty |
| `session.rs` | `Term` + PTY thread (`spawn`, `resize`), `grid_size` | alacritty |
| `wl.rs` | own Wayland client: wire protocol, `xdg_toplevel`, `wl_shm` buffers, frame callbacks, `wl_seat` pointer, cursor shape | `rustix` |
| `wlapp.rs` | the app on `wl.rs` + `soft.rs` + `session.rs` (`AXON_BACKEND=wl`); one blocking `poll` loop | all of the above |
| `input.rs` | `KeyEvent` → xterm bytes | winit, alacritty `TermMode` |
| `palette.rs` | Black-and-white ANSI palette, `Color`→`Rgb` resolution | alacritty `vte::ansi` |
| `shader.wgsl` | Vertex/fragment for cells and background | — |

## Dependency rules (inviolable)
1. `gpu.rs`, `soft.rs` and `wl.rs` do **not** know about `alacritty_terminal` or `Term`. Renderers only receive rows of `Cell { tile|flags, fg, bg }` through `view::Backend`; the position is the index.
2. `input.rs` is pure: no state, no I/O, unit-testable.
3. `palette.rs` is data + pure functions. Future themes replace its constants, not its logic.
4. Nothing outside `main.rs`/`app` touches `winit::event_loop`.
5. All user state (font, opacity, theme, shortcuts) comes in through the config layer, not loose `env::var` calls.
6. Every data structure answers the questions in `axon-data-oriented-design`; no caches, no hash tables on hot paths.

## Modularization roadmap (do it in steps; each step compiles and is tested)
Goal: a Cargo workspace with small crates.

```
axon/                      (workspace)
├─ crates/axon-core/       Session: Term + PTY + events (no winit, no wgpu). Headless-testable.
├─ crates/axon-render/     Gpu, atlas, fonts, shader. API: begin/push/render. No alacritty.
├─ crates/axon-config/     Config (TOML), themes, keymap; hot-reload.
└─ crates/axon-app/        winit, input, mouse, selection; ties the other three together. `axon` binary.
```

Recommended extraction order:
1. `config.rs` (replaces the `env::var` calls; own minimal parser, no `serde`/`toml`).
2. `view.rs`: move `State::build()` (grid→cells) out of `main.rs`.
3. `session.rs`: `Term`+PTY+`Notifier`+`Proxy` in a `Session` type.
4. `app.rs` (winit handler), leaving `main.rs` minimal.
5. Convert to a workspace only when there is a second consumer (headless tests, another frontend).

Planned features and where they live:
- Tabs/panes: one `Session` per pane; `view` composes several into one grid; the renderer does not change.
- Mouse reporting (xterm 1000/1006): `input.rs` (encoding) + `app` (capture).
- Ligatures/color emoji: `axon-render` (second RGBA atlas); `Cell` gains a flag bit.
- Scrollback search, hyperlinks (OSC 8): `view` (highlighting) + `session` (queries to `Term`).
- Cross-platform: only `app` (winit) and `session` (`tty` already abstracts Windows/Unix). `wayland` in
  `main.rs` (`with_name`) must sit behind `#[cfg(target_os = "linux")]`.

## When to update this skill
Every time a module is added or a dependency rule changes. Keep the table and the diagram up to date.
