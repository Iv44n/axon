---
type: decision
status: accepted
updated: 2026-11-26
related: [0004-term-fairmutex.md, 0011-software-render-wl-shm.md, 0005-damage-tracking.md, ../../performance/measurements.md]
---
# 0041 · The terminal's lock covers reading cells, not painting them

**Decision:** a frame locks a pane's `Term` only to read it: the damage, the cursor and selection, the composer's fold,
the prompts, and the cells of every row to repaint (`View::build_folded` into its `stage`; `bar::Chips::frame`). The lock is
released and then the rows are painted (`View::put`, `bar::Chips::paint`). The CPU renderer follows [0004](0004-term-fairmutex.md)
as the GPU one always did: what it does with the cells (pixels, an upload) happens outside the lock.

**Why:** [0004](0004-term-fairmutex.md)'s revisit condition was met. Measured on 2026-11-26 (`wl`, 1000x640, 100 MB `cat`,
a 120 Hz screen, so ~117 frames a second): the UI thread held the lock ~500 ms of a 1.87 s flood, 2.3 ms a frame, because
`soft.rs` painted every row (glass 1.0 ms, glyphs 1.2 ms) while holding it; reading the cells was 0.05 ms. The PTY thread cannot
parse while the lock is held. After: 16 ms held per flood (0.08 ms a frame). A/B of the flood, nine pairs in turn: 1.98 to
1.71 s on average, every pair faster; axon's CPU during it -20 %. Details in [measurements](../../performance/measurements.md) (2026-11-26).

**Rejected:** copying the whole grid out each frame (`stage` holds only the damaged rows, as before: the same peak, rows x cols
x 12 B on a full repaint); painting into a second buffer on another thread (a thread and a buffer more, for what moving the
painting out of the lock already gave); lowering the frame rate during floods (latency).

**Revisit if:** something drawn over the rows needs the terminal while it is painted (it must be read in `Chips::frame` and kept
for `paint`, as `used`); or the own terminal core (task 13) changes how the grid is shared.
