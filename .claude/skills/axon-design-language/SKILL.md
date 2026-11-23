---
name: axon-design-language
description: axon's visual language (black-and-white glass, rounded window, title bar with round buttons) - palette, frame, cursor, typography and what NOT to do. Use it when changing colors, effects, typography or any aesthetic detail.
---

# Design language

**Principles:** black and white, translucent glass, rounded, **continuous**: the window is one surface, the
title bar is the same glass as the grid (integrated, like Ghostty, WezTerm and Warp), nothing separates regions
and the edge is the same all the way round. Colour only where a program asks for it.
Decision records: `knowledge/decisions/0015` (continuous glass), `0014` (shadow, design board), `0012` (superseded look). The frame lives in
`src/chrome.rs`; the palette in `src/palette.rs`. **Look before and after every visual change** on the design
board: `scripts/design.sh --baseline` (keeps the current frames), change the code, `scripts/design.sh`, open
`design/board.html` (backdrops, 1x/2x/4x pixel zoom, baseline/split compare). Its frames come from the real renderer.

## Palette (source of truth: `src/palette.rs`)
- Glass body: near-black `#090909` at `AXON_OPACITY` (`chrome::OPACITY` = 0.88 on `wl`), the desktop shows
  through. No compositor blur on GNOME: below ~0.85 the text of a window behind is readable through ours.
- Text `#f2f2f2`; cursor `#ffffff`; selection `#4a4a4a`; default background cells transparent.
- ANSI: black `#2c2c2e`, white `#d0d0d0`, bright black `#8e8e93`, bright white `#ffffff`; the six hues are
  muted (`red #e5736f`, `green #8fd19e`, `yellow #e3c98a`, `blue #7aa7e0`, `magenta #c4a0d8`, `cyan #86c9d0`).
  Making them gray is one array in `palette.rs`.

## Frame (`chrome.rs`, all computed per pixel)
- Rounded rectangle, radius 14 px, anti-aliased; square when maximized, fullscreen or tiled.
- Shadow in a margin around the window (Mutter draws none for us): black, spread 26 px, cast 8 px down,
  55% at most with a cubic falloff; it alone outlines the window on a light desktop. The margin is outside the
  window geometry (`set_window_geometry`); only an 8 px band of it takes the pointer (resize), the rest lets
  clicks through. None when square.
- Glass: one tint everywhere, no sheen, no gradient. Edge: a 1 px outline of 8% white, identical on all four
  sides and anti-aliased round the corners; none when square.
- Title bar 38 px: not drawn, only a drag area. The title centred in `#ebebeb` at 75% (40% when unfocused). Buttons on the left, macOS order: close, minimize, maximize; 13 px discs, 20 px apart.
  Resting light gray; the pointer over the group shows the glyphs (x, -, +) and turns the hovered disc near white;
  pressed is dimmer; an unfocused window dims them.
- Grid padding: 0.9 x font size on the sides and below, title bar + 0.45 x font size above.

## Rules
1. Neutral chrome. Any new colour needs a reason a program or the user gave; no accent colour.
2. Everything in the frame is a function of position and `Ui`: no textures, masks or caches (rule 5).
3. A frame change repaints only what it touches: title bar for hover/focus/title, whole frame for maximize.
4. Cursor: solid white block; hollow when unfocused; no blinking by default (idle cost).
5. Monospaced typography, JetBrains Mono by default (`~/.local/share/fonts/JetBrainsMono[wght].ttf`); size 15 logical px. It has no Nerd/Powerline icons: point `AXON_FONT` at a Nerd Font (e.g. `CascadiaMonoNF.ttf`, +45 MB of idle RSS in `fontdue`) if the prompt needs them.
6. Animations: only if they last < 200 ms and return to idle. Never infinite loops.
7. Minimum 4.5:1 contrast for the main text and for `ANSI[8]` (gray) on the glass body.
8. Sharpness: `chrome::pixel` samples at `x + 0.5`, so anything that should be crisp (button discs, 1 px strokes) is centred on `n + 0.5`, never on an integer. Text is plain source-over; glyph coverage is raised to 0.70 when a tile is baked (`font.rs`). Squaring coverage, or centring on a pixel border, is what made the first version look soft.

## What NOT to do
- Separators between regions (title bar hairline, lighter panes), anything on the edge that is brighter in one
  place than another (it gathers in the corners), opaque blocks where a program paints the background colour.
- Scanlines, CRT noise, permanently pulsing glows, hard or offset-only shadows, fake blur (there is no portable blur).
- Colour in the chrome, gradients that need a stored image, buttons that animate forever.
- Changing the palette or the frame without updating this document and `chrome.rs` tests.
