---
type: reference
status: current
updated: 2026-11-22
related: [../decisions/core/0001-native-no-webview.md]
---
# Stack

Versions per `Cargo.toml`; if they differ, `Cargo.toml` wins.

| Component | Technology |
|---|---|
| Language | Rust, 2024 edition |
| VT emulation and PTY | `alacritty_terminal` 0.26; to be replaced ([0010](../decisions/core/0010-own-implementations.md)) |
| Windows and input | Own Wayland client (`wl.rs`) and, on Windows, an own Win32 window (`win.rs`, `windows-sys` 0.59, [0039](../decisions/core/0039-windows-port.md)); `winit` 0.30 only as the Linux X11 fallback |
| Rendering | `wgpu` 30 (Vulkan) today; `src/soft.rs` is the CPU renderer that replaces it ([0011](../decisions/core/0011-software-render-wl-shm.md), [plan](../plan/software-renderer.md)) |
| Text | `fontdue` 0.9 + own atlas; to be replaced ([0010](../decisions/core/0010-own-implementations.md)) |
| System calls | `rustix` (sockets with fd passing, `memfd`, `mmap`, `poll`, `eventfd`) for the own Wayland client; `windows-sys` declarations on Windows |
| Clipboard | `arboard` 3.7, `wayland-data-control` only |
| UI | Custom, drawn with the cell pipeline |
| Configuration | Own minimal parser (pending; no `serde`/`toml`) |
| Concurrency | `std` threads ([0007](../decisions/core/0007-no-sqlite-egui-tokio.md)) |
| Persistence | None |
| Release build | fat LTO, `codegen-units = 1`, `panic = abort`, `strip` |
