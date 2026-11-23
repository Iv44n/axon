---
type: risk
status: open
updated: 2026-11-22
related: [../plan/next-steps.md]
---
# Blinking cursor

**Risk:** the cursor does not blink today. If blinking is added with continuous redraw, it
breaks hard rule #3 in `CLAUDE.md` (0% idle CPU).

**Mitigation:** a `ControlFlow::WaitUntil` timer that wakes only on each cursor phase change
and redraws the minimum. Stop blinking after a few seconds of inactivity (like Alacritty's
`blink_timeout`) or when the window loses focus. Alternative: a steady cursor.

**Pending decision:** task 7 in [next steps](../plan/next-steps.md).
