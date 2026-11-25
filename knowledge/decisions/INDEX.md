# Decisions (ADR)

One leaf per decision. Never edited: superseded by a new ADR.

- [0001 · Native, no WebView](0001-native-no-webview.md) — winit + wgpu + alacritty_terminal; no Tauri, Electron or WebView.
- [0002 · Text with fontdue + own atlas](0002-text-fontdue-atlas.md) — superseded by 0010; still no glyphon/cosmic-text.
- [0003 · Render on the UI thread](0003-render-on-ui-thread.md) — no dedicated GPU thread.
- [0004 · Term shared via FairMutex](0004-term-fairmutex.md) — no channels carrying grid copies.
- [0005 · Damage tracking, not dirty rectangles](0005-damage-tracking.md) — the savings are on the CPU; implemented 2026-11-22.
- [0006 · Modules before crates](0006-modules-before-crates.md) — 4-crate workspace only once there is a second consumer.
- [0007 · No SQLite, egui or Tokio](0007-no-sqlite-egui-tokio.md) — no use case pays for their cost.
- [0008 · Idle RAM < 50 MB](0008-idle-ram-50mb.md) — superseded by 0011; target lowered from 80 MB; measure RSS and PSS.
- [0009 · Data-oriented design; no caches](0009-data-oriented-no-caches.md) — flat arrays, indices, packed bits; redesign instead of caching.
- [0010 · Own implementations from scratch](0010-own-implementations.md) — dependencies are scaffolding; fonts, terminal core and PTY first.
- [0011 · Software rendering into wl_shm](0011-software-render-wl-shm.md) — accepted 2026-11-22: own Wayland client + CPU renderer; 8.7 MB idle vs 191 MB with wgpu.
- [0012 · Black-and-white glass chrome](0012-glass-chrome-black-white.md) — superseded by 0015 (its look); CPU-drawn chrome and round buttons stand.
- [0013 · Wayland default, xkb via dlopen](0013-wayland-default-xkb-dlopen.md) — accepted 2026-11-23: `wl` is the default backend; `libxkbcommon` loaded at run time; fallback to `winit`.
- [0014 · Own shadow, design board](0014-client-shadow-design-board.md) — accepted 2026-11-23: shadow in a margin (`set_window_geometry`, input region); visual changes judged on real frames; opacity 0.88.
- [0029 · Group dialog, menus, rename, toasts](0029-group-dialog-menus-rename-toasts.md) — accepted 2026-11-24: `Alt+G` opens the reference's group dialog (templates open real windows with commands typed in); right-click menus on windows, groups, panes and the list; inline rename (double click, `F2` over a row); a close keeps the windows alive until its *Undo* toast is gone.
- [0026 · Docked settings panel](0026-docked-settings-panel.md) — accepted 2026-11-24: settings beside the terminal (non-modal, opposite the windows' panel), applied and saved as you change; supersedes the modal part of 0017.
- [0025 · The overlay kit](0025-overlay-kit.md) — accepted 2026-11-24: one `overlay::Stack` takes the keyboard and pointer; `Field`, `List`, `Menu`, `Toast` and the modal frame are what palette, overview, dialogs and menus are built from.
- [0024 · Split panes](0024-split-panes.md) — accepted 2026-11-24: panes per window (`panes.rs` layout, `Slot`s in `soft.rs`), focus ring, dimming, panel rows; a split below 20x5 cells is refused.
- [0023 · Windows and the side panel](0023-windows-and-side-panel.md) — accepted 2026-11-24: one `Win` per shell, `Alt+N/W/1-9/[/]/B`, a panel at the side (`side.rs`, `Soft::set_inset`); groups, panes, overview and palette still to come.
- [0022 · i18n](0022-i18n.md) — accepted 2026-11-24: English and Spanish through one `strings!` list in `i18n.rs`; `[ui] language = auto|en|es`; the window's text only.
- [0021 · UI after the terminal.html reference](0021-ui-refactor-terminal-reference.md) — accepted 2026-11-24: new palette with an amber accent, hairlines, coloured window buttons, status bar, outline chip icons, settings with live preview, sliding tabs, cursor shape.
- [0020 · Sharp text](0020-text-quality.md) — accepted 2026-11-24: glyphs supersampled 6x, vertical zone hinting and LCD sub-pixel coverage (`smoothing`, `hinting`); proportional settings font; fades; chips reordered by dragging.
- [0019 · Window blur](0019-window-blur.md) — accepted 2026-11-24: `[window] blur` asks the compositor (`ext-background-effect-v1`, KDE blur) to blur behind the glass; separate from opacity; GNOME offers none.
- [0018 · Prompt chips](0018-input-bar.md) — accepted 2026-11-24: opt-in row of context chips on the line above the prompt (time, user, dir, git, conda/venv, pyenv, kube); the shell marks its prompt with concealed spaces; grid's own font; read on a thread.
- [0017 · Settings screen](0017-settings-screen.md) — superseded in part by 0026 (the modal screen and Save/Cancel); its rules on state, text and the file's comments stand.
- [0016 · Config file, font settings](0016-config-file-font-settings.md) — accepted 2026-11-23: `config.toml` with an own parser; family, size, weight, spacing, line height, opacity; `Ctrl+Shift+R` reloads.
- [0015 · Continuous glass](0015-continuous-glass.md) — accepted 2026-11-23: integrated title bar, no separators, uniform 1 px outline; background-coloured cells are glass.
