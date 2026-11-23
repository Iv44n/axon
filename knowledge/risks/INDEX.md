# Risks

- [Inline images](inline-images.md) — `alacritty_terminal` supports neither sixel nor kitty graphics.
- [Blinking cursor](blinking-cursor.md) — blinking with continuous redraw breaks 0% idle CPU.
- [Fallback fonts](fallback-fonts.md) — hardcoded paths, no color emoji, ~600 ms stall on first CJK glyph.
