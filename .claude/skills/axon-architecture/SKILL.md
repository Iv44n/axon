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
- Overlays ([0025](../../../knowledge/decisions/workspace/0025-overlay-kit.md)): in `wlapp.rs` every key, pointer move, button and wheel event goes to `overlay::Stack` first; only
  what it does not take continues to the settings panel ([0026](../../../knowledge/decisions/workspace/0026-docked-settings-panel.md); not modal, so an overlay opens over it), `input.rs` and the selection. An overlay is a pure function of its state and the window size, painted over a complete grid.
- One direction: UI -> session (bytes, resize); session -> UI (`Ev` events). Never cross calls
  from the PTY thread into the UI except `Proxy::send_event`.

## Current module map (`src/`)

| Module | Responsibility | May depend on |
|---|---|---|
| `main.rs` | entry: `config` commands, then the platform's app: `wlapp::run` (Windows; Linux on Wayland) or `fallback::run` (Linux, X11 or no Wayland) | all |
| `fallback.rs` | the `winit` + `wgpu` app (`State`: window+session+mouse), Linux only, until X11 has its own client | all |
| `plat.rs` | the platform seam ([0039](../../../knowledge/decisions/core/0039-windows-port.md)): `Event`, `Key`, `Mods`, cursor shapes, `Wake`, and `Plat` = `wl::Wl` or `win::Win`; `wlapp.rs` sees only these | `xkb`/`win` (aliased) |
| `win.rs` | own Win32 window (Windows): layered, per-pixel alpha (`UpdateLayeredWindow`), own move/resize/maximize, `ToUnicodeEx` keys, clipboard, `MsgWaitForMultipleObjectsEx` loop; also `process_cwd`, `locale_name`, `attach_console` | `windows-sys`, `chrome`, `plat` |
| `cell.rs` | `Cell`, `F_*` flags, `T_*` special tiles, `pack`, the `Glyphs` trait | bytemuck |
| `font.rs` | `Fonts` (fallbacks, rasterizing), `GlyphTable`, special tile bitmaps; `UiFont::shared` hands the window (`Soft`, which holds it from its first frame), the settings panel and the overlays one parsed proportional font, read through `cmap::narrow` ([0036](../../../knowledge/decisions/interface/0036-the-reference-cleaner.md)) | fontdue, `cmap` |
| `cmap.rs` | a font file with its character map narrowed to what an interface draws (`narrow`, `ui_char`): own reader of `cmap` formats 4 and 12, writes one format 12 table, so `fontdue` parses ~900 outlines instead of ~2 900 (Inter) | - |
| `gpu.rs` | wgpu, atlas, pipelines, `render()` (to be deleted: [0011](../../../knowledge/decisions/core/0011-software-render-wl-shm.md)) | winit (`Window` only), `cell`, `font` |
| `soft.rs` | CPU renderer into an `Argb8888` framebuffer (grid, title, status bar when on, interface text in the UI font `ui_text`/`ui_width`/`ui_cut`, the `Dock`s beside the grid for the windows' panel and the settings panel); one `Slot` per pane (`set_layout`, `select`, border and ring on full frames, `set_dim` for panes without focus, the pane's cross, `restore` gives back the frame under a toast); the replacement for `gpu.rs` | `cell`, `chrome`, `font`, `palette`, `paint`, `panes` (`Rect`), `bar` (icons), `view` (trait only) |
| `chrome.rs` | window frame on the CPU: glass look, rounded corners, title bar and status bar hairlines, buttons, settings icon and shadow per pixel (buffer = window + `margins`), `hit` testing in window pixels, `Ui` state | — |
| `design.rs` | tests only: design board frames and tokens (`scripts/design.sh` → `design/board.html`), own PNG encoder, the incremental-equals-full repaint check | `soft`, `view`, `session`, `chrome`, `palette` |
| `settings.rs` | the settings panel (Font, Input and Windows pages), docked beside the grid and non-modal ([0026](../../../knowledge/decisions/workspace/0026-docked-settings-panel.md)): state, layout computed on demand, hit testing, keys, scrolling, painting into its cleared box; the desktop's proportional `UiFont` only while open | `chrome`, `config` (types), `paint`, `font`, `input::sym`, `xkb::Key` |
| `paint.rs` | antialiased shapes (`Paint`: put, shape, rrect, outline, disc, ring, chevron) composited into the framebuffer; shared by the overlays and panels | `chrome` |
| `i18n.rs` | every string the window draws in English and Spanish (`strings!` builds the key enum and both tables), `tr`, the `Setting`/`Lang` of `[ui] language`; no dependencies | — |
| `agent.rs` | The AI agents that run in a terminal (`Agent`: Claude, Codex, Antigravity) by the first word of the command (`Agent::of`, any case, no directory or `.exe`), and what it does by its title's mark (`State::of_title`: Claude Code's `◐`/`◑` working, `✳` waiting); a window that runs one shows its mark and does not spin (`history::History::agent` / `busy`, `side::Status::Agent`, `icons::agent`; [0044](../../../knowledge/decisions/workspace/0044-an-agent-has-its-mark-not-a-spinner.md), [0046](../../../knowledge/decisions/workspace/0046-the-agents-own-marks.md)); pure | - |
| `caret.rs` | A click on the line being typed: `steps` (how many characters the shell's cursor moves, from the prompt's end marker `view::prompt_end`, else the cursor's row and the rows that wrap into it; ignores a dim suggestion and a wide character's second half) and `keys` (the arrows, `input::arrow`); pure, tested on real terminals; `App::click_caret` decides whether to ([0043](../../../knowledge/decisions/interface/0043-a-click-moves-the-shells-cursor.md)) | `view`, `input` |
| `restore.rs` | The session kept between runs: the file (`Saved`, `encode`/`decode` with escaped tab-separated fields, `path`, `load`, `save` through a temporary file, `claims`: one axon owns it, a second leaves it alone; `AXON_SESSION`) ([0042](../../../knowledge/decisions/workspace/0042-the-session-is-kept-between-runs.md)); the app's side is `wlapp/keep.rs`; pure but for the file | `groups` (`COLORS`), `panes` (`Kind`) |
| `history.rs` | the prompt chips' memory: the hook's title escapes (`parse`), a ring of records (chips as they were, exit status, duration) found again by the prompt number in the reserved line; what runs now (its label, and where an ssh session goes: `remote`) | `context`, `ssh` (`Dest`) |
| `ssh.rs` | SSH servers by alias ([0048](../../../knowledge/decisions/workspace/0048-ssh-servers-by-alias.md)): `Host`, ssh's options (`args`, `session`, `pasted`), the proposed alias, the form's `check`, the search (`filter`, `by_recent`, `find`), axon's OpenSSH file (`write_config`, `read_config` with line numbers), the `Include` into `~/.ssh/config`, the uses; `Book` reads and writes them; pure but for `Book` | `config` (`path`), `context` (`home_dir`), `restore` (`path_named`) |
| `hosts.rs` | the panel of SSH servers (`Alt+H`), docked where the settings go: list (search and keys fixed, rows with status, buttons), edit (paste a command, fields, the command it amounts to, errors), import, export; layout on demand, hit testing, keys, painting into its cleared box; answers `Effect`s, never writes | `ssh`, `field`, `overlay` (`Text`, `kbd`), `icons`, `paint`, `palette`, `i18n` |
| `bar.rs` | the prompt chips: pure `layout` (which chips fit, cutting and dropping), icons, `paint` with the grid's own font via `Soft::paint_cells_at`, and `Chips::frame` (grid plus chips on the shell's reserved prompt line, repainted like a full frame) | `config` (`Chip`), `context`, `paint`, `soft`, `view`, `chrome` |
| `context.rs` | what the chips say (`gather` on a thread: `/proc`, `.git/HEAD`, kubeconfig, shell report file with the `git diff --numstat` totals; axon itself never runs git) plus the pure formatters, all tested | `config` (`Chip`) |
| `side.rs` | the list of windows, in the panel or as tabs: pure `Layout` (`Side`, `Top` with groups' containers and chips and a `first` entry the strip scrolls to; `each`, `hit`, `target`, `mark`, `scroll`, `reveal`), `entries` / `top_entries`, `Status`, the spinner's phase, painting over the box `Soft::side_rect` gives or the title bar (grid font, no UI font), the drag's ghost and marks; `Items`, the flat pool of text the app fills before each paint, so painting allocates nothing; one `Win` per window lives in `wlapp.rs` ([0023](../../../knowledge/decisions/workspace/0023-windows-and-side-panel.md), [0030](../../../knowledge/decisions/workspace/0030-drag-groups-status.md), [0032](../../../knowledge/decisions/workspace/0032-fidelity-second-reference.md)) | `dnd`, `paint`, `palette`, `soft` |
| `dnd.rs` | drag and drop in the list, pure: `Grab` (a window or a group), `Target` (near a window, into a group, beside a group, last), `arrange` (the new order of windows and groups, groups renumbered, empty ones dropped, ADR 0023's sort kept), the 5 px threshold; the pointer state is `Dnd` in `wlapp.rs` ([0030](../../../knowledge/decisions/workspace/0030-drag-groups-status.md)) | — |
| `panes.rs` | the split layouts, pure: `Kind` (single, columns, rows, grid), `rect(kind, n, i, area, tracks)`, `at`, `Tracks` (the sizes of a split's columns and rows, weights per track, even when unset) with `edge_at` / `drag` / `grow` (resizing, [0045](../../../knowledge/decisions/workspace/0045-a-split-can-be-resized.md)), `after_split` / `after_remove`, `fit_kind`, `cycle`, `fits` (the 20 x 5 minimum), `grid`; no state, no dependencies; the panes are a group's windows ([0034](../../../knowledge/decisions/workspace/0034-a-group-is-the-split.md), layout from [0024](../../../knowledge/decisions/workspace/0024-split-panes.md)) | — |
| `overlay.rs` | the overlay kit ([0025](../../../knowledge/decisions/workspace/0025-overlay-kit.md)): the `Overlay` trait, the `Stack` the app owns (top overlay gets keys, pointer and wheel; `Esc` and a press on the scrim close it; the terminal hears nothing; `Out::Act` runs a command and stays open; `Stack::with` gives the app the overlay by its type; `Grid` hands an overlay the terminal's text and the grid's font at draw time; `Stack::extent` says what a frame of overlays covers: the whole window, or only a toast's box), `Modal` (scrim, glass card, fade), `Text` (UI font loaded while open), `Ink`, key caps, buttons, `Fade` | `chrome`, `paint`, `font`, `palette`, `icons`, `toast`, `xkb::Key` |
| `field.rs` | a line of text being edited: pure state machine over `xkb::Key` (caret, selection, paste/copy through the app, longest length) and its three looks (`.f-input`, `.ov-search`, `.pal-input-row`) | `overlay`, `icons`, `input::sym` |
| `list.rs` | filtered, scrolling list model (the palette's: ranking as the reference, matched positions, Up/Down/Page/Home/End/Enter, wheel, hover) and its painting; entries in a text pool, no cache | `overlay`, `icons`, `side::group_color` |
| `menu.rs` | the context menu, an `Overlay`: items with shortcut, check, dot, danger, disabled, separators; placed at the pointer and clamped | `overlay`, `icons`, `side::group_color` |
| `toast.rs` | a toast: bottom-centre message with an action, a pure function of the moment (fades), `deadline` is the loop's only timer for it, `extent` its box (a lone toast repaints only that) | `overlay` |
| `icons.rs` | the outline icons of the overlays on the reference's 16-unit grid, drawn as per-pixel coverage; the agents' marks (`agent`: Claude's burst, Codex's cloud, Antigravity's arch, their published outlines as polygons filled by `fill`, scanline coverage on the stack, in `palette::CLAUDE` / `CODEX` / `ANTIGRAVITY`) | `agent`, `paint`, `chrome`, `palette` |
| `commands.rs` | what the command palette lists ([0027](../../../knowledge/decisions/workspace/0027-command-palette.md)): `Cmd` (id <-> command), `State` (the app's windows and groups at the moment it opens), `build` -> a filtered `List` (actions in the reference's order, windows, groups, servers); pure and tested | `config` (`TabsPos`), `i18n`, `icons`, `list`, `side` (`GroupItem`) |
| `picker.rs` | the palette's overlay (`Ctrl+K`): the input row, the list and the footer in a `Modal`; answers `Out::Run(id)`, `Ctrl+K` again closes; opened by `App::open_palette`, run by `App::run_cmd` | `overlay`, `field`, `list`, `icons`, `xkb::Key` |
| `overview.rs` | the overview of windows ([0028](../../../knowledge/decisions/workspace/0028-overview.md)), an `Overlay` over the workspace: pure layout (`columns`, `head`, `step`, `thumb`), the cards and groups as a snapshot the app refreshes every frame (`Stack::with`), filter, keyboard and pointer, painting; the preview's text comes at draw time through `overlay::Grid` | `overlay`, `field`, `list` (`clipped`), `icons`, `side` (`Status`, `group_color`), `chrome` |
| `groups.rs` | groups of windows, pure and tested: `Group`, `check` (the dialog's form: empty or taken name, nothing chosen), `suggest_name`, `next_color`, `prune` / `dissolve`, what a close takes out and puts back (`after_close`, `reinsert`, `resolve`), the templates (`detect`, `commands`, `template_wins`), `fill`; no dependencies ([0029](../../../knowledge/decisions/workspace/0029-group-dialog-menus-rename-toasts.md)) | — |
| `group_dialog.rs` | the group dialog, an `Overlay`: name, colours, windows, templates, error line, summary, edit mode; keyboard zones, body scroll, list shortening; leaves a valid form in a shared `Answer` | `overlay`, `field`, `groups`, `list::clipped`, `side::group_color` |
| `ctxmenu.rs` | the context menus of a window, a group, a pane (unreachable since 0034) and the empty list: items, shortcuts, danger, command ids (`Cmd`) | `menu`, `i18n`, `overlay` |
| `overlay_demo.rs` | the kit's debug screens (`AXON_DEMO_OVERLAY=field\|list\|menu\|toast\|modal`) and the usage example: `Dialog`, `picker()` (the palette over ten sample entries), `window_menu` | `overlay`, `field`, `list`, `menu`, `picker` |
| `config.rs` | settings file (`config.toml`), own parser, `FontCfg`, env overrides; read at startup and on reload | — |
| `mouse.rs` | mouse reports for programs (xterm 1000/1002/1003; SGR, UTF-8, classic): pure `encode`, own tests | `xkb` (`Mods`), alacritty (`TermMode`) |
| `view.rs` | grid -> cells: `View::build(term, focused, &mut impl Backend)`, damage, cursor and selection rows; `Fold` lays the lines on the rows (the composer docked at the bottom, ADR 0037; flat for the alternate screen and for a backend that does not dock) and maps rows to lines and back | `cell`, `palette`, alacritty |
| `session.rs` | `Term` + PTY thread (`spawn`, `resize`), `grid_size`; the shell in a systemd scope of its own (`scope_args`, `scope_probe` / `ScopeProbe::works`, ADR 0038: `systemd-run --user --scope` execs in place, so the pid is the shell's) | alacritty |
| `wl.rs` | own Wayland client: wire protocol, `xdg_toplevel`, `wl_shm` buffers, frame callbacks, `wl_seat` pointer + keyboard, cursor shape, `wl_data_device` clipboard | `rustix` |
| `wlapp.rs` | the app on `plat::Plat` (`wl.rs` on Linux, `win.rs` on Windows) + `soft.rs` + `session.rs`: windows (`Win`, one `Pane` = one shell each; a group's windows are shown side by side, `Shown`/`shown()`: ADR 0034), keyboard to the focused pane, pointer to the pane under it, selection, clipboard; one blocking `poll` loop; routes keys, pointer and wheel to `overlay::Stack` first (`overlay_owns`, `overlay_reply`). `wlapp/groups_ui.rs` (child module) is `impl App` for windows, groups, menus, rename and undo: `spawn_win`, `close_wins`, `open_group_dialog`, `menu_ran`, `start_rename` | all of the above |
| `wlapp/hosts_ui.rs` | The panel of servers, the app's side: open and close (it takes the settings' place), its effects (connect: the window in a session, the empty prompt, or a new window in the group *Servers*; save, delete with *Undo*, import, export, copy), the *More* menu, `ssh_session` (what the hook's `L` says of an ssh: label, cyan dot, *Save as* toast), the palette's servers, `AXON_HOSTS` | `wlapp`, `hosts`, `ssh`, `menu` |
| `wlapp/keep.rs` | The session between runs, the app's side: `plan` (restore or not: the setting, a debugging aid, who owns the file), `snapshot`, `note_session` / `tick_session` (written 1.5 s after the first change; the loop's only timer for it), `save_session` (also when the app ends), `restore_session` (the windows after the first, in their folders and groups) | `wlapp`, `restore` |
| `wlapp/selftest.rs` | `AXON_SELFTEST=<dir>`: the app tests itself (shell, directory, clipboard, maximize, idle CPU and memory) and writes a report and its frame; CI uses it | `wlapp` |
| `wlapp/windows_overview.rs` | the app's side of the overview, a child of `wlapp.rs` (sees `App`, `Win`, `Pane`): `Alt+O`, `sync_overview` (cards from the windows each frame), `overview_ran` (what a click asked), `AppGrid` (the `overlay::Grid` that reads a pane's last rows with `try_lock_unfair`, so a frame never waits for a terminal that is not on screen, and paints them in the grid's font) | `wlapp`, `overview`, `overlay` |
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
- Tabs/panes: done (0023, 0024, 0034): a `Pane` (session, view, chips) per shell, a split is a group's windows shown together (`Shown` in `wlapp.rs`), the layout in `panes.rs`, one `Slot` per pane in `soft.rs`.
- Mouse reporting (xterm 1000/1006): `mouse.rs` (encoding, done) + `wlapp.rs` (capture).
- Ligatures/color emoji: `axon-render` (second RGBA atlas); `Cell` gains a flag bit.
- Scrollback search, hyperlinks (OSC 8): `view` (highlighting) + `session` (queries to `Term`).
- Cross-platform: done for Windows ([0039](../../../knowledge/decisions/core/0039-windows-port.md)): the app is platform-neutral over `plat.rs`; a new platform is a new `Plat` (a window layer with the methods `wlapp.rs` calls: `wait_wake`, `read_events`, `framebuffer`, `present`, `set_clipboard`, ...), its `Wake`, and the shell in `session.rs`. Nothing but `win.rs`, `wl.rs`, `xkb.rs`, `fallback.rs`, `gpu.rs` and a few `#[cfg]` blocks in `session.rs`, `context.rs`, `font.rs`, `config.rs`, `i18n.rs` knows the system.

## When to update this skill
Every time a module is added or a dependency rule changes. Keep the table and the diagram up to date.
