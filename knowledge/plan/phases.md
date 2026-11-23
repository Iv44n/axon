---
type: reference
status: current
updated: 2026-11-22
related: [next-steps.md]
---
# Plan phases

Plan rule: first a core with measured performance; features come after.

| Phase | Status | Note |
|---|---|---|
| 1. Terminal engine | done | `alacritty_terminal` + PTY on its own thread |
| 2. Native app | done | winit, keyboard, Wayland/X11 |
| 3. GPU renderer | done | wgpu + fontdue + own atlas ([0002](../decisions/0002-text-fontdue-atlas.md)) |
| 4. Interaction | partial | Done: selection, copy/paste, scrollback, resize. Missing: xterm 1000/1006 mouse reporting |
| 5. Workspace | pending | Tabs, panes, multiple sessions, restore |
| 6. Distinct experience | pending | Command Palette, history, optional agents |
