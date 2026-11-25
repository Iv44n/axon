---
type: decision
status: accepted
updated: 2026-11-22
related: [../../architecture/stack.md]
---
# 0001 · Native terminal, no WebView

**Decision:** a native Rust application: `winit` for windows and input, `wgpu` for the GPU and
`alacritty_terminal` for VT emulation and the PTY. Custom UI, drawn with the same renderer.

**Why:** the main goal is minimal resource usage. With no Rust↔JavaScript layer, the UI and
the engine share memory in a single process, and the cost of Chromium/WebView and its
JavaScript runtime disappears.

**Rejected:** Tauri, Electron and any WebView; React or any other web UI.

**Revisit if:** the project's goal changes. No technical trigger is foreseen.
