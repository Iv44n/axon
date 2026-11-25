---
type: decision
status: accepted
updated: 2026-11-22
related: [0004-term-fairmutex.md]
---
# 0003 · Render on the UI thread

**Decision:** `build()` and `Gpu::render()` run on winit's UI thread. There is no dedicated
render thread.

**Why:** generating instances and sending them to the GPU is cheap. An extra thread forces
synchronization of the surface and state with no measurable gain. Alacritty also renders on
the main thread.

**Rejected:** a separate GPU thread (original proposal).

**Revisit if:** measurements show that `build()` + `render()` regularly exceed one vsync
frame.
