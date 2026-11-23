---
type: decision
status: accepted
updated: 2026-11-22
related: [0001-native-no-webview.md, 0002-text-fontdue-atlas.md, ../plan/own-implementations.md]
---
# 0010 · Own implementations from scratch

**Decision:** axon writes its own implementation of every layer it can, in order of RAM and CPU
payoff. Third-party crates are scaffolding with an exit plan, and no new one is added where a
few hundred lines of our own would do. First targets: font reading and rasterization (replaces
`fontdue`), the terminal core, VT parser and grid (replaces `alacritty_terminal`), and the PTY.
`winit` and `wgpu` stay until a measured spike shows what a lighter path saves. Order and
designs: [plan/own-implementations](../plan/own-implementations.md). Decided by the user on
2026-11-22: "the idea of this project is to reinvent the wheel".

**Why:** the goal is the least RAM and CPU, and the measured costs are in the dependencies:
`fontdue` 45 MB for one font and 314 MB for a CJK font; the Vulkan/Mesa/NVIDIA libraries
~94 MB of RSS; a converted copy of the terminal grid every frame.

**Rejected:** tuning the existing crates and keeping them as permanent architecture.

**Supersedes:** [0002](0002-text-fontdue-atlas.md) on `fontdue` and the lazily baked atlas.
Amends [0001](0001-native-no-webview.md): "native, no WebView" stands; `alacritty_terminal` is
scaffolding.

**Revisit if:** after a measured attempt, an own component cannot match the conformance or
performance of the crate it replaces. The terminal core is the main risk: keep
`alacritty_terminal` as a test oracle for it.
