# Decisions: workspace

Windows, groups, panes, the overlays built on one kit, and the screens of the second reference. Never edited: superseded by a new ADR.

- [0023 · Windows and the side panel](0023-windows-and-side-panel.md) — one `Win` per shell, `Alt+N/W/1-9/[/]/B`, a panel or tabs at the top.
- [0024 · Split panes](0024-split-panes.md) — superseded by 0034 for panes inside a window; the layout, focus ring, dimming and 20x5 minimum stay.
- [0025 · The overlay kit](0025-overlay-kit.md) — one `overlay::Stack` takes keyboard and pointer; field, list, menu, toast, modal frame.
- [0026 · Docked settings panel](0026-docked-settings-panel.md) — settings beside the terminal, non-modal, saved as you change; supersedes 0017's modal part.
- [0027 · The command palette](0027-command-palette.md) — `Ctrl+K` on the overlay kit: actions, windows, groups; no History heading.
- [0028 · The overview of windows](0028-overview.md) — `Alt+O`: a card per window with its last rows, sections per group, filter.
- [0029 · Group dialog, menus, rename, toasts](0029-group-dialog-menus-rename-toasts.md) — `Alt+G` dialog with templates, right-click menus, inline rename, Undo toast.
- [0030 · Drag and drop, groups in the tabs, status icons](0030-drag-groups-status.md) — pointer-driven drag of windows and groups; a spinner that turns only while a command runs.
- [0031 · The paint path after the review](0031-review-fixes-paint-path.md) — no syscalls, locks or copies while painting; a toast repaints its box; one UI font.
- [0032 · Fidelity to the second reference](0032-fidelity-second-reference.md) — 40 px title bar, panel measures, scrolling tabs, overview drag, window label from the command.
- [0033 · Windows, groups, palette, overview and drag as one set](0033-windows-feature-set-after-the-merge.md) — what the merge connected; the groups model; what is left out on purpose.
- [0034 · A group is the split](0034-a-group-is-the-split.md) — a group's windows are shown side by side as panes; `Ctrl+Space` makes the group.
- [0038 · A scope per window](0038-a-scope-per-window.md) — each shell in its own systemd scope, probed first; memory and oomd kills are per window.
- [0042 · The session is kept between runs](0042-the-session-is-kept-between-runs.md) — windows, folders, names and groups reopen where they were; one owner; `[windows] restore`.
- [0044 · An agent has its mark, not a spinner](0044-an-agent-has-its-mark-not-a-spinner.md) — superseded in part by 0046; the still mark instead of a ring, no timer, stand.
- [0046 · The agents' own marks](0046-the-agents-own-marks.md) — published marks in their colours; faint while Claude works, a dot while it waits; title glyph cut.
