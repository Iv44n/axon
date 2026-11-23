---
type: decision
status: superseded-by-0011
updated: 2026-11-22
related: [../performance/protocol.md]
---
# 0008 · Idle RAM < 50 MB

**Decision:** the idle RAM target drops from 80 MB to 50 MB. Decided by the user on
2026-11-22. The `axon-performance` skill already reflects the new value.

**Why:** reducing resource usage is the project's main motivation.

**How it is measured:** RSS and PSS, per the [protocol](../performance/protocol.md). With
wgpu on Vulkan, the driver's shared libraries weigh heavily in RSS; PSS splits them across
the processes that use them.

**Revisit if:** the first measurement shows the driver alone exceeds the target in RSS. Then
the target must be set explicitly on PSS, with a new ADR.
