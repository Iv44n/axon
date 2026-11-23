---
type: decision
status: accepted
updated: 2026-11-22
related: [../plan/next-steps.md, ../performance/measurements.md]
---
# 0005 · Per-line damage tracking, not dirty rectangles

**Decision:** use `Term`'s damage (`Term::damage`) to rebuild only the lines that change and
upload them with a partial `write_buffer`.

**Why:** wgpu cannot present only a damaged region of the surface, so dirty rectangles save
nothing on the GPU. The real savings are on the CPU, when generating instances.

**Rejected:** dirty rectangles at the presentation level (original proposal).

**Implemented 2026-11-22** (task 2 in [plan/next-steps](../plan/next-steps.md)): `Term::damage()`
plus cursor and selection diffs in `main.rs` choose the rows; `gpu.rs` writes each run of damaged
rows straight into the fixed-stride cell buffer ([0009](0009-data-oriented-no-caches.md)); no
CPU-side copy. Scrolling is Full damage in `alacritty_terminal`, so output floods gain nothing;
typing at a prompt rewrites 1-2 rows ([measurements](../performance/measurements.md)).

**Revisit if:** wgpu exposes partial-damage presentation.
