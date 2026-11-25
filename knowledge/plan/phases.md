---
type: reference
status: current
updated: 2026-11-25
related: [next-steps.md]
---
# Plan phases

Plan rule: first a core with measured performance; features come after.

| Phase | Status | Note |
|---|---|---|
| 1. Terminal engine | done | `alacritty_terminal` + PTY on its own thread |
| 2. Native app | done | winit, keyboard, Wayland/X11 |
| 3. GPU renderer | done | wgpu + fontdue + own atlas ([0002](../decisions/core/0002-text-fontdue-atlas.md)) |
| 4. Interaction | done | Selection, copy/paste, scrollback, resize, xterm mouse reporting (`wl` backend) |
| 5. Workspace | in progress | Done: windows, groups, split panes, overview, drag and drop ([0023](../decisions/workspace/0023-windows-and-side-panel.md) to [0033](../decisions/workspace/0033-windows-feature-set-after-the-merge.md)). Left: resizing panes by dragging, restoring windows between runs |
| 6. Distinct experience | in progress | Done: command palette ([0027](../decisions/workspace/0027-command-palette.md)), settings panel, prompt chips. Left: history (needs the own editor, task 6a phase 2), optional agents |
