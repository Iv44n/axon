---
name: axon-design-language
description: axon's visual language (futuristic, minimalist) - palette, background, borders, cursor, typography and what NOT to do. Use it when changing colors, effects, typography or any aesthetic detail.
---

# Design language

**Principles:** deep dark, one cool accent (cyan), one warm-cool secondary (violet), zero ornament
that is not information. The futuristic feel comes from light and contrast, not from decoration.

## Palette (source of truth: `src/palette.rs`)
- Background `#070a13` with a vertical night-blue → dark-violet gradient (`fs_bg`), opacity 0.93.
- Text `#cfd9f5`; cursor and accent `#19e3ff`; selection `#143a56`.
- Neon ANSI: red `#ff4d6d`, green `#38f2a0`, yellow `#ffd36b`, blue `#5b8cff`, magenta `#c77dff`, cyan `#19e3ff`.
- Border: 1 px line + short glow (≈7 px) with a cyan→violet gradient in `fs_bg`.

## Rules
1. A single dominant accent per screen (cyan). Violet only as the gradient's counterpoint.
2. No persistent title bars/tabs by default: undecorated window; move with `Alt+drag`, resize from the edges.
3. Inner padding ≈ 0.9 × font size. No thick separator lines.
4. Cursor: solid cyan block; hollow when unfocused; no blinking by default (idle cost).
5. Monospaced typography with Nerd/Powerline glyphs (Cascadia Mono NF by default); size 15 logical px.
6. Animations: only if they last < 200 ms and return to idle. Never infinite loops.
7. Minimum 4.5:1 contrast for the main text and for `ANSI[8]` (gray) on the background.

## What NOT to do
- Scanlines, CRT noise, permanently pulsing glows, heavy shadows, rounded borders with costly transparency.
- More than two new accent colors.
- Changing the palette without updating this document.
