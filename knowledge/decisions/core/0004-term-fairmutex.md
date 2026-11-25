---
type: decision
status: accepted
updated: 2026-11-22
related: [0003-render-on-ui-thread.md]
---
# 0004 · Term shared via Arc<FairMutex>, coalesced wakeup

**Decision:** the PTY thread (`alacritty_terminal`'s EventLoop) writes into
`Arc<FairMutex<Term>>` and notifies the UI with a single `Event::Wakeup`, coalesced by an
`AtomicBool`. The UI locks `Term` only during `build()`, resize and selection; it never does
I/O or GPU work while holding the lock.

**Why:** this is `alacritty_terminal`'s model. Sending grid copies over channels would
allocate every frame (hard rule #1 in `CLAUDE.md`). The `AtomicBool` already batches updates
and avoids piling up stale frames.

**Rejected:** bounded channels carrying grid copies (original proposal).

**Revisit if:** lock contention is measured that delays the PTY thread or the frame.
