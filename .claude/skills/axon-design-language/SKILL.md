---
name: axon-design-language
description: axon's visual language (dark glass after the terminal.html reference: amber accent, hairlines, coloured window buttons, status bar, chips, settings) - palette, frame, cursor, typography and what NOT to do. Use it when changing colors, effects, typography or any aesthetic detail.
---

# Design language

**Principles:** dark translucent glass, rounded, one amber accent and six clear hues, hairlines instead of panes.
The look is the `terminal.html` reference the user supplied (ADR 0021, which supersedes the black-and-white
identity of 0012 and the no-separators rule of 0015): title bar and status bar share the glass of the grid, divided from it
by a 1 px hairline. Decision records: `knowledge/decisions/0021` (this look), `0020` (text), `0015`, `0014` (shadow, design
board). The frame lives in `src/chrome.rs` and `src/soft.rs`; the tokens in `src/palette.rs`. **Look before and after every
visual change** on the design board: `scripts/design.sh --baseline` (keeps the current frames), change the code,
`scripts/design.sh`, open `design/board.html` (backdrops, 1x/2x/4x pixel zoom, baseline/split compare). Its frames come
from the real renderer.

## Palette (source of truth: `src/palette.rs`)
- Glass body `#0b0b0c` at `AXON_OPACITY` (`chrome::OPACITY` = 0.88 on `wl`); the desktop shows through. No compositor blur
  on GNOME: below ~0.85 the text of a window behind is readable through ours.
- Text `#d9d9de`; secondary (`DIM`) `#74747d`; cursor `#f4f4f5`; selection `#4a3d27`; default background cells transparent.
- One accent, amber `#ffb454` (`ACCENT`): the prompt `$`, slider fill, switch on, the chosen radio, "Unsaved changes", focus.
- Hues: green `#3ddc84`, red `#ff5d5d`, magenta `#d86bdc`, yellow `#f6b73c`, blue `#6aa9ff`, cyan `#4fd6d0`: the ANSI 1-6 slots and the
  chip colours. ANSI grey (8) is `#7e7e87` (4.5:1 on the glass; `DIM` is 4.25:1, fine for chrome only).

## Frame (`chrome.rs`, all computed per pixel)
- Rounded rectangle, radius 14 px, anti-aliased; square when maximized, fullscreen or tiled.
- Shadow in a margin around the window (Mutter draws none for us): black, spread 26 px, cast 8 px down, 55% at most with a
  cubic falloff. The margin is outside the window geometry; only an 8 px band of it takes the pointer (resize). None when square.
- Glass: one tint (`GLASS`), outline of 7% white on the outermost pixel (none when square), a 1 px 5% light just inside the top edge.
- Title bar 42 px (`TITLE_H`): a 3.5% white light fading out downward, a 7% hairline on its last row. The title is centred in the grid
  font: `user@host:` in `DIM`, the rest in the text colour (100%, 55% unfocused). Buttons on the left, macOS order, 12 px discs 20 px apart,
  centred at y = 21: resting grey `#3a3a3e` (55% unfocused); the pointer over any of them colours all three (close `#ff5f57`, minimize
  `#febc2e`, maximize `#28c840`) and shows dark glyphs (x, -, a square); pressed is dimmer.
- Settings button: a 28 px square at the right end (centre 28 px from the edge) with a two-slider icon in `DIM`; hovered it turns to the text
  colour on a rounded 7% white highlight (12% pressed). Its own hover state (`Hit::Settings`, `Ui.hover == 4`).
- Status bar 28 px (`STATUS_H`, `Ui.status`, `[window] status_bar`): 2% white tint, a 7% hairline on its first row; a green dot with a faint
  halo and the shell, `UTF-8`, the branch (icon and name, with the chips on) at 82% of the font size in `DIM`; on the right the grid size and
  the hints (`↑↓ history`, `Tab complete`, `Ctrl L clear`, `Ctrl , settings`) as keycaps (4% fill, 7% outline, a thicker foot) followed by a
  label, dropped from the left when they do not fit. It takes grid rows (`Soft.bottom`).
- Prompt chips (`bar.rs`, ADR 0018): on the reserved line above the prompt being typed at, each an 8 px-radius pill 90% of the row high,
  10% fill of its hue, 58% outline, an outline icon (1.5-unit strokes on the reference's 16-unit grid, never filled shapes), regular text in
  the grid font in the hue. Hues: time red, user green, env cyan, pyenv amber, host grey `#a1a1aa`, dir magenta, branch yellow, changes
  `#ff8a4c`, kube blue. The chips are 90% of the grid font size and 8 px apart (at 14 px); cut with `…` then dropped from the end. The shell's
  own `$` is bold amber (hook). Chips stay on every prompt (history ring) and, once its command ran, the right end of the line shows a green check or a red cross with the exit status when not 0, then the duration in `FAINT` (`12ms`, `1.4s`, `2m 5s`), at 80% of the chip font. Judge it on the `chips*` scenes.
- Settings panel (`settings.rs`, ADR 0026 after the reference's `.prefs`): not modal, docked beside the grid, 372 px (62% of a narrow window at
  most), no background of its own (the glass shows), a 7% hairline on the grid's side; at the edge opposite the windows' panel. Header: the title
  (15 px, bold) and a 28 px close cross; tabs in a rounded 5% container with a 10% marker that slides (0.22 s); a body of 40 px rows (label
  112 px in `DIM`, the text colour when hovered or selected; hover 4% white; the control column is 213 px) that scrolls under a thin
  `#2a2a2e` bar; sliders with a 4 px `#2c2c31` track, amber fill, a 14 px `#f4f4f5` knob and a 58 px number box; amber pill switches with their
  "on"/"off" word; radios (`#5a5a62` ring, amber with a dot when chosen) that wrap onto a second line; the font picker as two 30 px buttons around
  a name box; the chip grid in two columns (on = hue at 12% with a 45% outline and a dot; off = grey, hollow dot); a quiet 3% note with
  keycaps and code; a footer under a hairline with one 7% button, *Reset to defaults*. The clock and chips rows are 38% and inert with the shell's
  own prompt; the windows' panel rows are inert with the tabs at the top. While it is open the title bar's sliders icon stays lit.
  Controls fade (`Paint.fade`, 0.12-0.16 s). Judge it on the `prefs*` scenes.
- Overlays (`overlay.rs` and the kit, ADR 0025; judge them on the `overlays` group of the board): a scrim `rgba(6,6,7,.5)` under the title bar (the title bar stays live); a card `#141416` radius 16, 10% white outline, the settings' soft shadow, rising 6 px while it fades in over 0.14 s; a menu or toast `#17171a` (radius 12 / 11, tighter shadow). Text in the desktop's proportional font at the reference's sizes (13 base, 12.5 menu and toast, 11.5 small, 11 headings, 10.5 key caps, 14.5 the palette's input, 15 a dialog's title). Fields: 36 px box (black 35%, 7% outline; focused: amber 60% outline and a 3 px amber 16% glow), 30 px search box, the palette's bare row with a hairline; a solid 1 px caret, amber 30% selection, placeholder `FAINT`. Lists: rows 40 px (51 with a description), headings 11 px `FAINT`, 24 px icon box at 5%, selected row 9% white, matched characters amber, shortcuts as one key cap per key, a thin scrollbar thumb. Menus: 29 px rows (a heading 23, a separator 11), a 14 px column for the amber check, danger `#ff8d8d`, disabled at 40%. Toast: 37 px high (39 with an action, whose button has 6 px of padding), 6.5 s, 40 px above the window's bottom edge. Footer of the palette 35 px. Never a blinking caret.
- Overview of windows (`overview.rs`, ADR 0028; judge it on the `overview*` scenes): over the workspace only, not the panel or the title bar; `--bg` at 98.5 % (the reference's 94 % over a blur, which we do not have). Header 57 px (14 above, a 30 px search box, 12 below, a 7 % hairline): the title (15 px, bold), `6 windows · 2 groups` (12 px `DIM`), the filter (220 px, focused: amber outline), two ghost buttons (28 px, 7 % outline, 14 px icon) and a 28 px cross; narrow, it drops the count, the words, the buttons. Body padding 16 / 20 / 24: a section per group (radius 14, the group's colour at 5 %, padding 10, a 24 px head with the dot, name, count and a *+ Window* ghost button), then the windows in none; cards in `auto-fill` columns of at least 210 px, 12 px apart, 178 px high: `#101012`, radius 12, a 7 % outline (18 % and lifted 1 px under the pointer, amber 70 % plus a 25 % ring for the window on screen), a 118 px preview (the grid's font at 0.75, `#b4b4bb`, the last line in the text colour or amber when a command runs, fading out over the top 38 %), a foot under a hairline (status icon, name, unread dot, pane count, key cap, cross) and the directory in 11.5 px `DIM`. The dashed tile at the end (88 px, 14 radius) turns amber under the pointer.
- Grid padding: 0.9 x font size on the sides, title bar + 0.45 x font size above, status bar + 0.45 x font size below.

## Rules
1. One accent (amber) and six hues, all from `palette.rs`; a new colour needs a reason the user gave. Chrome stays dark and quiet.
2. Everything in the frame is a function of position and `Ui`: no textures, masks or caches (rule 5).
3. A frame change repaints only what it touches: title bar for hover/focus/title, whole frame for maximize.
4. Cursor: solid white block; hollow when unfocused; no blinking by default (idle cost).
5. Monospaced typography, JetBrains Mono by default (`~/.local/share/fonts/JetBrainsMono[wght].ttf`); size 15 logical px. It has no Nerd/Powerline icons: point `AXON_FONT` at a Nerd Font (e.g. `CascadiaMonoNF.ttf`, +45 MB of idle RSS in `fontdue`) if the prompt needs them.
6. Animations: only if they last < 200 ms and return to idle. Never infinite loops.
7. Minimum 4.5:1 contrast for the main text and for `ANSI[8]` (gray) on the glass body.
8. Sharpness: `chrome::pixel` samples at `x + 0.5`, so anything that should be crisp (button discs, 1 px strokes) is centred on `n + 0.5`, never on an integer. Text is plain source-over; glyph coverage is raised to 0.70 when a tile is baked (`font.rs`). Squaring coverage, or centring on a pixel border, is what made the first version look soft.
9. Text sharpness (ADR 0020): glyphs are supersampled 6x, vertically hinted (`Vmap`: baseline, x-height, cap height snapped to whole rows) and drawn with LCD sub-pixel coverage (`[font] smoothing`, `hinting`). Never go back to grey, unhinted `fontdue` coverage for the grid; check `text-plain`, `text-gray` and `text-unhinted` in the design board after touching `font.rs`.
10. The settings panel uses one proportional UI font (`UiFont`) at 13 px (12 in the notes); the grid and the title bar stay monospaced. Controls fade (`Paint.fade`, 0.12-0.16 s) and frames are requested only while a fade runs.

## What NOT to do
- Lighter panes or opaque blocks (hairlines and faint tints are the only dividers), anything on the edge that is brighter in one
  place than another (it gathers in the corners), opaque blocks where a program paints the background colour.
- Scanlines, CRT noise, permanently pulsing glows, hard or offset-only shadows, fake blur (there is no portable blur).
- Gradients that need a stored image, buttons or a cursor that animate forever (a blinking cursor wakes the process: idle = 0% CPU).
- Changing the palette or the frame without updating this document and `chrome.rs` tests.
