---
type: risk
status: open
updated: 2026-11-22
related: [../plan/next-steps.md]
---
# Inline images

**Risk:** `alacritty_terminal` 0.26 implements neither sixel nor the kitty graphics protocol
(checked in its source code on 2026-11-22). The original plan's "inline images" phase does
not come for free from the engine.

**Impact:** offering inline images requires work outside the engine and an extra texture
pipeline in the renderer.

**Mitigation:** first decide whether it is a real requirement. If it is, intercept those
sequences before the parser, which likely requires a custom PTY read loop.

**Pending decision:** task 7 in [next steps](../plan/next-steps.md).
