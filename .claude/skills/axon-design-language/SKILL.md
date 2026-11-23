---
name: axon-design-language
description: axon's visual language (black-and-white glass, rounded window, title bar with round buttons) - palette, frame, cursor, typography and what NOT to do. Use it when changing colors, effects, typography or any aesthetic detail.
---

# Design language

**Principles:** black and white, translucent glass, rounded. The look comes from light on a dark pane
(sheen, a fading rim, a lighter title bar), not from decoration. Colour only where a program asks for it.
Decision record: `knowledge/decisions/0012`. The frame lives in `src/chrome.rs`; the palette in `src/palette.rs`.

## Palette (source of truth: `src/palette.rs`)
- Glass body: near-black `#090909` at `AXON_OPACITY` (0.80 on the `wl` backend), the desktop shows through.
- Text `#f2f2f2`; cursor `#ffffff`; selection `#4a4a4a`; default background cells transparent.
- ANSI: black `#2c2c2e`, white `#d0d0d0`, bright black `#8e8e93`, bright white `#ffffff`; the six hues are
  muted (`red #e5736f`, `green #8fd19e`, `yellow #e3c98a`, `blue #7aa7e0`, `magenta #c4a0d8`, `cyan #86c9d0`).
  Making them gray is one array in `palette.rs`.

## Frame (`chrome.rs`, all computed per pixel)
- Rounded rectangle, radius 14 px, anti-aliased; square when maximized. Alpha 0 outside.
- Sheen: white, 10% at the top fading out by ~200 px. Rim: 1 px white, 46% at the top to 16% at the bottom.
  Edge light: white 7.5%, ~5 px reach, stronger on top.
- Title bar 38 px: lighter pane (+4.5% white), hairline underneath, the title centred in `#ebebeb` at
  75% (40% when unfocused). Buttons on the left, macOS order: close, minimize, maximize; 13 px discs, 20 px apart.
  Resting light gray; the pointer over the group shows the glyphs (x, -, +) and turns the hovered disc near white;
  pressed is dimmer; an unfocused window dims them.
- Grid padding: 0.9 x font size on the sides and below, title bar + 0.45 x font size above.

## Rules
1. Neutral chrome. Any new colour needs a reason a program or the user gave; no accent colour.
2. Everything in the frame is a function of position and `Ui`: no textures, masks or caches (rule 5).
3. A frame change repaints only what it touches: title bar for hover/focus/title, whole frame for maximize.
4. Cursor: solid white block; hollow when unfocused; no blinking by default (idle cost).
5. Monospaced typography with Nerd/Powerline glyphs (Cascadia Mono NF by default); size 15 logical px.
6. Animations: only if they last < 200 ms and return to idle. Never infinite loops.
7. Minimum 4.5:1 contrast for the main text and for `ANSI[8]` (gray) on the glass body.

## What NOT to do
- Scanlines, CRT noise, permanently pulsing glows, heavy shadows, fake blur (there is no portable blur).
- Colour in the chrome, gradients that need a stored image, buttons that animate forever.
- Changing the palette or the frame without updating this document and `chrome.rs` tests.
