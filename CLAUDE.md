# axon

Native terminal emulator: minimalist, Wayland/X11, Rust. On Wayland it runs on an own client and a CPU renderer into `wl_shm`
(`knowledge/decisions/0011`, `0013`; idle RSS 14 MB); the `winit` + `wgpu` path remains only for X11 and as fallback until stage E.
VT emulation: `alacritty_terminal`. Window: own `wl.rs` (`winit` fallback). Keyboard layouts: `libxkbcommon` via `dlopen` (`xkb.rs`). Glyphs: `fontdue` + tiles.
Direction: data-oriented, no caches, own implementations instead of dependencies (`knowledge/decisions/0009`, `0010`, `0011`);
`alacritty_terminal`, `fontdue`, `winit` and `wgpu` are scaffolding.

## Commands
- `cargo run --release` — run (debug is too slow for rendering).
- `cargo build --release` — fat LTO, `panic=abort`, strip.
- `scripts/design.sh [--baseline]` — renders the design board frames from the real renderer (`src/design.rs`) into
  `design/frames/`; open `design/board.html`. `--baseline` keeps the previous frames to compare. Use it for every visual change.
- `scripts/install.sh [--prefix DIR] [--no-build] [--uninstall]` — installs the binary, desktop entry and icon (`assets/`); `DESTDIR` is honoured. Test it with a throwaway `DESTDIR` or `HOME`, never the real one.
- Settings: `axon config` creates `~/.config/axon/config.toml` (`config edit`, `config show`, `config path`; own parser in `config.rs`; `assets/config.example` lists every key; `Ctrl+Shift+R` reloads in `wl`; the settings screen (gear in the title bar or `Ctrl+,`, `settings.rs`) edits and saves it; `AXON_SETTINGS=1` starts with it open, for screenshots and measurements). The window's text is in English or Spanish (`i18n.rs`, `[ui] language`, ADR 0022: a new string goes in `strings!` for both languages). The look follows the `terminal.html` reference (ADR 0021: amber accent, hairlines, status bar `[window] status_bar`, cursor shape `[input] cursor`). Text is supersampled, vertically hinted and LCD sub-pixel (`font.rs`, `soft.rs`, ADR 0020; `[font] smoothing`/`hinting`). Chips are reordered by dragging in the settings screen. `[window] blur = true` asks the compositor to blur behind the glass (a request, `wl.rs`, ADR 0019; GNOME offers none). `[input] mode = "chips"` draws context chips on the line above the prompt (`bar.rs`, `context.rs`, `view::prompt_rows`, `history.rs` for each command's status and time, ADR 0018/0021; bash gets its hook from axon, `axon shell-init bash|zsh|fish` prints it for an rc file). Env overrides it: `AXON_FONT`, `AXON_FONT_SIZE` (15), `AXON_OPACITY` (0.93 `winit`, 0.88 `wl`), `AXON_DECORATIONS=1`.
- `AXON_BACKEND`: `wl` (default when `WAYLAND_DISPLAY` is set) or `winit` (+ `wgpu`, the X11 fallback; `wl` falls back to it by itself if it cannot start). `AXON_WL_DEBUG=1` traces the protocol.

## Project skills (read them before touching the corresponding area)
- `axon-architecture` — layers, data flow, dependency rules, module roadmap.
- `axon-performance` — performance budgets and rules; how to measure.
- `axon-renderer` — atlas, cell buffer format, shader, wgpu 30 notes.
- `axon-extend` — recipes: shortcut, config, theme, escape sequence, new platform.
- `axon-design-language` — futuristic/minimalist visual identity.
- `axon-data-oriented-design` — design questions and rules for every data structure: flat arrays, indices, packed bits, SoA vs AoS, no caches.

## Knowledge tree
`knowledge/INDEX.md` — plan, decisions (ADRs), risks and measurements. Navigate through the
indexes and open only the leaf you need. To write in it: `knowledge/MAINTENANCE.md`.

## Hard rules
1. Nothing on the UI thread that blocks or allocates per frame on the hot path.
2. No new dependencies without justifying the cost (binary size, startup, compile time).
3. Idle = 0% CPU: render only on demand (`ControlFlow::Wait`).
4. Verify the real UI after a visual change (the design board, `scripts/design.sh`); `cargo build` does not prove it looks right. Renderer refactors must be pixel-exact (method in `axon-data-oriented-design`).
5. No caches: do not add one, redesign the data instead. Flat arrays over hash tables, indices over pointers.
6. Prefer our own implementation to a new dependency; existing dependencies are scaffolding with an exit plan (`knowledge/plan/own-implementations.md`).
7. Keep the numbers current: after any change that can move binary size, memory or speed (code, dependencies, build profile, fonts), run `scripts/bench.sh` and update the README `Benchmarks` section (tables, date, commit and a new history row) and `knowledge/performance/` in the same change. Never leave stale numbers.
