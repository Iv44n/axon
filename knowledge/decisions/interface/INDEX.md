# Decisions: interface

How the window looks and what the user can configure: glass, the settings file, chips, text, language. Never edited: superseded by a new ADR.

- [0012 · Black-and-white glass chrome](0012-glass-chrome-black-white.md) — superseded by 0015 (its look); CPU-drawn chrome and round buttons stand.
- [0014 · Own shadow, design board](0014-client-shadow-design-board.md) — shadow in a margin; visual changes judged on real frames; opacity 0.88.
- [0015 · Continuous glass](0015-continuous-glass.md) — integrated title bar, uniform 1 px outline; background-coloured cells are glass.
- [0016 · Config file, font settings](0016-config-file-font-settings.md) — `config.toml` with an own parser; font settings; `Ctrl+Shift+R` reloads.
- [0017 · Settings screen](0017-settings-screen.md) — superseded in part by 0026; its rules on state, text and comments stand.
- [0018 · Prompt chips](0018-input-bar.md) — opt-in context chips above the prompt; the shell marks its prompt; grid's own font.
- [0019 · Window blur](0019-window-blur.md) — `[window] blur` asks the compositor to blur behind the glass; GNOME offers none.
- [0020 · Sharp text](0020-text-quality.md) — glyphs supersampled 6x, vertical hinting, LCD sub-pixel coverage; proportional settings font.
- [0021 · UI after the terminal.html reference](0021-ui-refactor-terminal-reference.md) — superseded in part by 0036; amber accent, hairlines, coloured buttons, outline chip icons stand.
- [0022 · i18n](0022-i18n.md) — English and Spanish through one `strings!` list; `[ui] language`.
- [0036 · The reference, cleaner](0036-the-reference-cleaner.md) — superseded in part by 0037; UI font around the grid (narrowed `cmap`), no status bar, quiet chips, 22 px margins stand.
- [0037 · The prompt docked at the bottom](0037-the-prompt-docked-at-the-bottom.md) — the shell's prompt below a hairline at the bottom, output rising above; `view::Fold`.
- [0043 · A click moves the shell's cursor](0043-a-click-moves-the-shells-cursor.md) — a click on the typed line presses the arrows to that cell; not on suggestions or programs.
