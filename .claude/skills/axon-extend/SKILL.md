---
name: axon-extend
description: Step-by-step recipes for extending axon - new keyboard shortcut, config option, color theme, escape sequence/mouse support, new platform. Use it when the user asks for a new feature.
---

# Extension recipes

Before any change: read `axon-architecture` (dependency rules) and, if it touches rendering, `axon-renderer`.

## New app keyboard shortcut
1. App shortcuts (not sent to the shell) are intercepted in `window_event → KeyboardInput` in `main.rs`, **before** `input::encode`. Use `Ctrl+Shift+…` to avoid clashing with terminal apps.
2. Implement the action as a `State` method (`copy`, `paste`, …), not inline.
3. If a redraw is needed: `window.request_redraw()`.
4. Document the shortcut in `CLAUDE.md` or the README.

## New key/sequence sent to the PTY
Add it in `input.rs` (pure function). Cover modifiers with the parameter `mods = 1+shift+2·alt+4·ctrl`. Add a unit test with the expected sequence.

## Config option
1. Today: `std::env::var` in `State::new` (`AXON_*`). As it grows, create `config.rs` (see the roadmap) with `struct Config` + `Default`, loaded from `~/.config/axon/config.toml` by an own minimal parser (no `serde`/`toml`, see `knowledge/decisions/0010`).
2. Never read config on the hot path; load it at startup and propagate it to `Gpu`/`State`.

## Color theme
Edit only `palette.rs` (`FG`, `BG`, `CURSOR`, `SELECTION_BG`, the `ANSI` table) and the gradient/border in `fs_bg`. Follow `axon-design-language`. Check the contrast of `ANSI[0]`/`ANSI[8]` on `BG`.

## Mouse support for applications (vim, tmux)
Check `TermMode::MOUSE_MODE` / `SGR_MOUSE`; if active, encode events in `input.rs` (`CSI < b ; x ; y M/m`) and **do not** start a selection (unless Shift is held).

## New event from the terminal (bell, notification, etc.)
`TermEvent` arrives through `Ev::Term` in `user_event`. Add an arm there; keep the action off the PTY thread.

## New platform
- Windows/macOS: `alacritty_terminal::tty` already abstracts the PTY; put `platform::wayland` in `main.rs` behind `#[cfg]`, and adjust the font list in `Fonts::new`.
- Change `PresentMode`/alpha only with evidence.

## Definition of done for any change
- [ ] `cargo build --release` with no new warnings.
- [ ] Tested in the real window (keyboard, resize, HiDPI scaling if applicable).
- [ ] Idle still at 0% CPU.
- [ ] Skills/`CLAUDE.md` updated if a rule or module changed; `knowledge/` updated if a task, decision or measurement changed.
