---
name: axon-architecture
description: axon's architecture (GPU terminal in Rust) - layers, data flow, dependency rules and modularization plan. Use it before adding modules, moving code or deciding where a new feature lives.
---

# axon architecture

## Data flow (one direction)

```
wl_keyboard/wl_pointer ─► wl.rs ─► xkb.rs ─► input.rs ─► bytes ─► PTY (alacritty thread)
                                                                     │ reads output
                                                     Term<Proxy> (FairMutex) ◄┘  parses VT, updates grid
                                                          │ Event::Wakeup (coalesced by AtomicBool, eventfd)
                                                          ▼
poll wakes wlapp ─► View::build ─► soft.rs rows ─► wl_shm buffer ─► wl.rs present (frame callback paces)
```
(The `winit` fallback: `winit` events ─► `input::encode` ─► PTY; `user_event` ─► `State::build` ─► `Gpu::render`.)

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
| `soft.rs` | CPU renderer into an `Argb8888` framebuffer (grid, title, status bar, the `Dock`s beside the grid for the windows' panel and the settings panel); the replacement for `gpu.rs` | `cell`, `chrome`, `font`, `palette`, `paint`, `bar` (icons), `view` (trait only) |
| `chrome.rs` | window frame on the CPU: glass look, rounded corners, title bar and status bar hairlines, buttons, settings icon and shadow per pixel (buffer = window + `margins`), `hit` testing in window pixels, `Ui` state | — |
| `design.rs` | tests only: design board frames and tokens (`scripts/design.sh` → `design/board.html`), own PNG encoder, the incremental-equals-full repaint check | `soft`, `view`, `session`, `chrome`, `palette` |
| `settings.rs` | the settings panel (Font, Input and Windows pages), docked beside the grid and non-modal ([0026](../../../knowledge/decisions/0026-docked-settings-panel.md)): state, layout computed on demand, hit testing, keys, scrolling, painting into its cleared box; the desktop's proportional `UiFont` only while open | `chrome`, `config` (types), `bar` (chip colours), `paint`, `font`, `input::sym`, `xkb::Key` |
| `paint.rs` | antialiased shapes (`Paint`: put, shape, rrect, outline, disc, ring, chevron) composited into the framebuffer; shared by the overlays and panels | `chrome` |
| `i18n.rs` | every string the window draws in English and Spanish (`strings!` builds the key enum and both tables), `tr`, the `Setting`/`Lang` of `[ui] language`; no dependencies | — |
| `history.rs` | the prompt chips' memory: the hook's title escapes (`parse`), a ring of records (chips as they were, exit status, duration) found again by the prompt number in the reserved line | `context` |
| `bar.rs` | the prompt chips: pure `layout` (which chips fit, cutting and dropping), icons, `paint` with the grid's own font via `Soft::paint_cells_at`, and `Chips::frame` (grid plus chips on the shell's reserved prompt line, repainted like a full frame) | `config` (`Chip`), `context`, `paint`, `soft`, `view`, `chrome` |
| `context.rs` | what the chips say (`gather` on a thread: `/proc`, `.git/HEAD`, kubeconfig, shell report file with the `git diff --numstat` totals; axon itself never runs git) plus the pure formatters, all tested | `config` (`Chip`) |
| `side.rs` | the panel of windows: pure layout and hit testing (`hit`, `fit`), painting over the box `Soft::side_rect` gives (grid font, no UI font); one `Win` per shell lives in `wlapp.rs` ([0023](../../../knowledge/decisions/0023-windows-and-side-panel.md)) | `paint`, `palette`, `soft` |
| `config.rs` | settings file (`config.toml`), own parser, `FontCfg`, env overrides; read at startup and on reload | — |
| `mouse.rs` | mouse reports for programs (xterm 1000/1002/1003; SGR, UTF-8, classic): pure `encode`, own tests | `xkb` (`Mods`), alacritty (`TermMode`) |
| `view.rs` | grid -> cells: `View::build(term, focused, &mut impl Backend)`, damage, cursor and selection rows | `cell`, `palette`, alacritty |
| `session.rs` | `Term` + PTY thread (`spawn`, `resize`), `grid_size` | alacritty |
| `wl.rs` | own Wayland client: wire protocol, `xdg_toplevel`, `wl_shm` buffers, frame callbacks, `wl_seat` pointer + keyboard, cursor shape, `wl_data_device` clipboard | `rustix` |
| `wlapp.rs` | the default app on `wl.rs` + `soft.rs` + `session.rs`: keyboard, repeat, pointer, selection, clipboard; one blocking `poll` loop | all of the above |
| `xkb.rs` | keyboard layouts: `libxkbcommon` through `dlopen`, keymap text → keysym + text, compose | libc only |
| `input.rs` | `encode_key(xkb key)` and the winit `encode` → xterm bytes | `xkb`, winit (until stage E), alacritty `TermMode` |
| `palette.rs` | The `terminal.html` tokens (text, `DIM`, amber `ACCENT`, six hues), the ANSI table, `Color`→`Rgb` resolution, `unit()` | alacritty `vte::ansi` |
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
- Mouse reporting (xterm 1000/1006): `mouse.rs` (encoding, done) + `wlapp.rs` (capture).
- Ligatures/color emoji: `axon-render` (second RGBA atlas); `Cell` gains a flag bit.
- Scrollback search, hyperlinks (OSC 8): `view` (highlighting) + `session` (queries to `Term`).
- Cross-platform: only `app` (winit) and `session` (`tty` already abstracts Windows/Unix). `wayland` in
  `main.rs` (`with_name`) must sit behind `#[cfg(target_os = "linux")]`.

## When to update this skill
Every time a module is added or a dependency rule changes. Keep the table and the diagram up to date.
