---
type: decision
status: accepted
updated: 2026-11-22
related: [../architecture/stack.md]
---
# 0007 · No SQLite, egui or Tokio

**Decision:** do not include these dependencies.

**Why:** each adds binary size and compile time without a concrete use case (hard rule #2).
The shell already keeps history. The UI is custom and egui does not fit. `std` threads are
enough for current concurrency.

**Rejected:** optional SQLite, egui as the UI, selective Tokio (original proposal).

**Revisit if:**
- SQLite → searchable cross-session history or other real persistence is needed.
- Tokio → network integrations appear, for example with agents.
- egui → never for the main UI.
