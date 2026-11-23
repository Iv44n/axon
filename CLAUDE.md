# axon

Native terminal emulator: minimalist, Wayland/X11, Rust. Today it renders on the GPU (wgpu); the decided direction is a CPU
renderer into `wl_shm` with an own Wayland client (`knowledge/decisions/0011`, idle RSS 8.7 MB in the spike vs 191 MB).
VT emulation: `alacritty_terminal`. Window: `winit`. Glyphs: `fontdue` + tiles.
Direction: data-oriented, no caches, own implementations instead of dependencies (`knowledge/decisions/0009`, `0010`, `0011`);
`alacritty_terminal`, `fontdue`, `winit` and `wgpu` are scaffolding.

## Commands
- `cargo run --release` — run (debug is too slow for rendering).
- `cargo build --release` — fat LTO, `panic=abort`, strip.
- Env: `AXON_FONT`, `AXON_FONT_SIZE` (15), `AXON_OPACITY` (0.93), `AXON_DECORATIONS=1`.

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
4. Verify the real UI after a visual change; `cargo build` does not prove it looks right. Renderer refactors must be pixel-exact (method in `axon-data-oriented-design`).
5. No caches: do not add one, redesign the data instead. Flat arrays over hash tables, indices over pointers.
6. Prefer our own implementation to a new dependency; existing dependencies are scaffolding with an exit plan (`knowledge/plan/own-implementations.md`).
