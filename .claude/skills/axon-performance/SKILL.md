---
name: axon-performance
description: axon's performance budgets and rules (latency, idle CPU, throughput, memory) and how to measure them. Use it when touching the hot path (build/render/PTY), adding dependencies or optimizing.
---

# axon performance

## Budgets (measurable targets)
| Metric | Target |
|---|---|
| Idle CPU | 0% (no timers, no continuous redraw) |
| Startup to first frame | < 100 ms |
| Key-to-pixel latency | ≤ 1 vsync frame |
| Throughput, `cat` of 100 MB | bound by the VT parser, not by rendering |
| Idle RAM | < 50 MB (the metric is RSS, PSS as a second column; see `knowledge/decisions/0011`) |
| Release binary | < 10 MB |

## Hot-path rules
1. **On-demand rendering.** `ControlFlow::Wait`; only `request_redraw` in response to an event.
2. **Coalesce wakeups.** `Proxy` uses `AtomicBool pending`; do not send one `Ev::Wake` per PTY read.
3. **No per-frame allocations.** `Gpu.stage` is reused scratch (`clear`/`resize`); the GPU buffer only grows (powers of 2).
4. **Fixed stride, zero = empty.** One 12-byte `Cell` per grid position; a blank cell is all zeros and costs the
   vertex shader a degenerate triangle. No per-cell branching on the CPU for blanks beyond writing the cell.
4b. **Write only damaged rows.** `build()` reads `Term::damage()` every frame, adds the cursor and selection rows,
   and writes each run of damaged rows with one upload. No CPU-side copy of the buffer.
4c. **No font parsing on the UI thread at startup.** fontdue parses every glyph on load (CJK font: ~300 MB, ~600 ms); fallbacks load lazily and the CJK font only for CJK code points.
5. **Rasterize only on a miss, and never add caches.** The glyph table is a flat two-level table and is the only
   lazy structure of its kind (to be replaced by an eager bake, see `axon-data-oriented-design`).
6. **One draw call** for cells + one for the background. Do not add passes without measuring.
7. **Minimal `FairMutex` locking.** Only inside `build()`; no I/O or GPU work while holding the lock (`render()` already runs outside it).
8. **Vsync (`AutoVsync`)**: do not use `Immediate/Mailbox` by default; it burns GPU under heavy output.
9. Dependencies: every new crate is justified by its cost in size and compile time.

## How to measure
- Idle: `top -p $(pgrep axon)` with the terminal idle; must be 0.0%.
- Throughput: `time cat /tmp/big.txt` inside axon (`seq 1 12300000 > /tmp/big.txt`, ~100 MB), compare with alacritty/kitty.
- Frames: `RUST_LOG`/temporary counter in `redraw()` (do not leave it in the final code).
- Latency: record at 240 fps or use `typometer`.
- Memory: `ps -o rss= -p $(pgrep axon)`.
- Binary: `ls -l target/release/axon`.

## Optimization candidates (when data justifies them)
- Replace fontdue with an own font reader; the first-use stall of a fallback font disappears with it (`knowledge/plan/own-implementations.md`).
- Subpixel/LCD AA and an SDF map only if sharpness demands it.
- One `Session` per thread for multiple panes.
