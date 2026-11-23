---
type: decision
status: accepted
updated: 2026-11-22
related: [../plan/next-steps.md]
---
# 0006 · Modules before crates

**Decision:** stay a single crate and extract modules (`config`, `view`, `session`, `app`).
Move to a 4-crate workspace (`axon-core`, `axon-render`, `axon-config`, `axon-app`) only
once there is a second consumer, such as headless tests.

**Why:** more crates lengthen compile times with no benefit while there is a single binary.
winit already abstracts what the proposal put in `platform/` and `keyboard/`.

**Rejected:** the original proposal's 8-crate layout.

**Details:** extraction order and the contents of each crate are in the `axon-architecture`
skill.

**Revisit if:** a second consumer of the core appears.
