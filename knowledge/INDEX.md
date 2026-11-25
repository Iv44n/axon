# axon · knowledge tree

What was decided, what is left, and what has been measured. The skills (`.claude/skills/axon-*`)
explain how things are done; this tree explains what and why, and links instead of copying.

**How to read it:** this index → a branch's `INDEX.md` → only the leaf that answers your
question. Each line says enough to decide whether to open it. Do not read the whole tree.

Summary: native terminal in Rust, no WebView: own Wayland client (Linux) or own Win32 window (Windows) + CPU renderer, `winit` + `wgpu` as the X11 fallback, `alacritty_terminal` for VT emulation.

## Branches
- [plan/](plan/INDEX.md) — phases, status and the next task. Start here if you are going to work.
- [decisions/](decisions/INDEX.md) — ADRs: what was chosen, what was rejected, when to revisit.
- [risks/](risks/INDEX.md) — known limits of the stack and their mitigation.
- [performance/](performance/INDEX.md) — comparison protocol and measurement log.
- [architecture/](architecture/INDEX.md) — final stack; module details live in the skill.

## Writing
Before creating, moving or changing nodes: [MAINTENANCE.md](MAINTENANCE.md).
