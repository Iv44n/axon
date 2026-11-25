---
name: axon-extend
description: Step-by-step recipes for extending axon - new keyboard shortcut, config option, color theme, escape sequence/mouse support, new platform. Use it when the user asks for a new feature.
---

# Extension recipes

Before any change: read `axon-architecture` (dependency rules) and, if it touches rendering, `axon-renderer`.

## New app keyboard shortcut
1. App shortcuts (not sent to the shell) are intercepted in `App::handle_key` in `wlapp.rs` (keysyms from `xkb.rs`), **before** `input::encode_key`; the `winit` fallback has its own copy in `window_event → KeyboardInput` in `main.rs` until stage E. Use `Ctrl+Shift+…` to avoid clashing with terminal apps.
2. Implement the action as an `App` method (`copy`, `paste`, `zoom`, …), not inline.
3. If a redraw is needed: `self.dirty = true` (the loop draws on the next frame callback).
4. Document the shortcut in `CLAUDE.md` or the README.
5. A shortcut that does something the command palette could list also gets an entry in `commands.rs` (`build`, with the shortcut text, a `Cmd` variant and its arm in `App::run_cmd` that calls the same method as step 2; [0027](../../../knowledge/decisions/0027-command-palette.md)).

## New key/sequence sent to the PTY
Add it in `input.rs` (`encode_key`, a pure function of a resolved xkb key). Cover modifiers with the parameter `mods = 1+shift+2·alt+4·ctrl`. Add a unit test with the expected sequence.

## Config option
1. `config.rs` is the file (`~/.config/axon/config.toml`, own parser, [0016](../../../knowledge/decisions/0016-config-file-font-settings.md)). Add the field to `Config` / `FontCfg`, its range constant, a line in `apply`, a test in `config.rs`, and a commented line in `assets/config.example` and the README.
2. Never read config on the hot path: it loads at startup and on `Ctrl+Shift+R` (`App::reload` in `wlapp.rs`); a setting that changes the glyph bitmaps goes through `Fonts` (`set_px` / `raster`), so tiles are rebuilt and the renderers stay unchanged.
3. The environment variables win over the file (`env_overrides`).
4. A setting the user should change also gets a row in the settings panel (`settings.rs`, [0026](../../../knowledge/decisions/0026-docked-settings-panel.md)): a `Row` variant in `FONT_ROWS`, `INPUT_ROWS` or `WINDOWS_ROWS`, its `kind()` (picker, slider, switch, radios, chips), `label`, and for a slider `range`, `limits`, `Draft::get` / `store` / `text`; radios also `options`, `choose`, `chosen`; switches `flip` and their arm in `draw_body`; and a line in `changes()` with its file key. The panel applies each change at once and the app writes it (`wlapp.rs`: `apply`, `flush_save`), so the setting needs its effect in `App::apply`. Its tests need the new row.
5. `axon config` writes `assets/config.example` as is (a test parses it with every line uncommented): keep it in step, and add the key to `describe` (`config show`).

## New chip for the prompt chips
1. `config::Chip`: add the variant to the enum, `COUNT`, `ALL` and the `name` list (the file's word for it). `Chip::ALL` order is the default order and the settings panel's grid order (2 per row).
2. `context.rs`: fill `Context.text[Chip::X as usize]` in `gather` from a pure function (tested) that reads a file or `/proc`; empty means "nothing to show". Anything slow (a process, a network) stays on the thread `gather` already runs on; what only the shell knows goes through `axon shell-init` (`SHELL_*` in `config.rs`) and `parse_report`.
3. `bar.rs`: a colour in `COLORS` and an icon arm in `icon` (14 px, outline only, built from `stroke`, `box_outline`, `Paint::shape`; its test (`every_icon_draws_something...`) covers it.
4. `assets/config.example`, the README chip table, and a scene in `src/design.rs` (`sample_context`).

## Color theme
Edit only `palette.rs` (`FG`, `BG`, `DIM`, `ACCENT`, the hues, `CURSOR`, `SELECTION_BG`, the `ANSI` table; the chip colours in `bar::colors` read them) and the gradient/border in `fs_bg`. Follow `axon-design-language`. Check the contrast of `ANSI[0]`/`ANSI[8]` on `BG`.

## Mouse support for applications (vim, tmux)
Done for the `wl` backend: `mouse.rs` is the pure encoder (`encode(mode, Event, Mods, col, row)`: modes 1000/1002/1003, SGR 1006, UTF-8 1005, classic), tested without a window. `wlapp.rs` calls it from `report_button`, `on_pointer_move` (only when the cell changes) and `on_scroll`, and starts no selection while a program has the mouse (Shift gives it back). To add an event, extend `mouse::Event` and its test.

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

## A new text in the interface
Add one line to `strings!` in `src/i18n.rs` with both languages and draw it with `tr(Str::Name)`; never a literal in `settings.rs`, `soft.rs` or `bar.rs`. Check the Spanish length against its box on the design board (`*-es` scenes). Config keys, CLI output and hooks are not translated (ADR 0022).
