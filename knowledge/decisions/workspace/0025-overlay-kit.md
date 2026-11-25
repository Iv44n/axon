---
type: decision
status: accepted
updated: 2026-11-25
related: [../interface/0017-settings-screen.md, ../interface/0021-ui-refactor-terminal-reference.md, 0023-windows-and-side-panel.md, ../core/0009-data-oriented-no-caches.md]
---
# 0025 · The overlay kit

**Decision:** the command palette, the overview, the dialogs, the context menus and the toasts of the `terminal (2).html`
reference are built from one small kit, so each is a composition and not a new screen system: `overlay.rs` (the capture
model, the modal frame, text, fade), `field.rs` (a line being edited), `list.rs` (filter + selection model and its
painting), `menu.rs`, `toast.rs`, `icons.rs` (the 16-unit outline icons), and `overlay_demo.rs` (a dialog, a palette-shaped
`Picker`, a menu and a toast to try each and as the example). The settings screen is untouched.

**Capture:** `App::overlays: Stack` (bottom to top) and a toast. With one open the top overlay gets every key (`Esc` closes it
unless it `handles_escape`), the pointer (a press off its `contains` is the scrim: it closes it and the release is
swallowed) and the wheel; the shell hears nothing, held keys stop (`overlay_reply` drops the repeat when one closes). The
window's own controls (buttons, edges, title bar where it moves the window) stay live, like under the settings screen; the
panel and the tabs are under the scrim. A toast takes no key and the pointer only over itself.

**Trait:** `Overlay { key, paste, pointer, button(Click), scroll, contains, cursor, step, draw, handles_escape }`, all but
`key`/`draw` defaulted. Answers are `Out { None, Redraw, Close, Run(id), Paste, Copy(text) }`; `Run` closes and tells the app
`Reply::ran = Ran { tag, id }` (the tag the app opened it under, `overlay::TOAST` for a toast button): the app matches on it
in `overlay_ran`. Coordinates are window pixels. The clipboard is the app's: a field says `Edit::Paste`, the stack reads
`Clipboard::get` only then and hands the text to `Overlay::paste`.

**Pieces:** `Field` (`key` → `Edit`; caret and selection in characters; Home/End/arrows/Ctrl+arrows, Backspace/Delete/
Ctrl+Backspace, Ctrl+A/C/X/V, longest length, placeholder; `press`/`drag` with click counts; solid caret; scroll is a function
of the caret; `Field::number()` for numeric boxes: what the settings screen's own box can move to, it was left alone)
painted in three `Look`s (`Box` .f-input, `Search` .ov-search, `Row` .pal-input-row). `List` (entries in a text
pool with spans; `filter` ranks as the reference: `100 - min(pos, 60)` for the query found whole in `label desc keywords`,
`20` for its letters in order, accents and case folded, 40 shown, 70 with no query; `hits` = positions for highlighting, which
the reference does not do; Up/Down wrap, Page/Home/End stop, Enter → `Nav::Chose(index)`, wheel, hover, thumb). `Menu`
(builder `item/dot/sep/head` with `DANGER|DISABLED|CHECKED`, placed at the pointer and clamped 8 px inside, Up/Down skip what
cannot be chosen). `Toast` (6.5 s; fade in 0.18, out 0.25; `deadline(now)` is the only timer, none without a toast).
`Modal` (scrim under the title bar, card r16 `#141416`, rises 6 px, 0.14 s; `Modal::at_top(60)` for the palette).

**Using it:**
```rust
self.open_overlay(TAG_MENU, Box::new(Menu::at(x, y).item("Rename", "F2", CMD_RENAME, 0).sep()
    .item("Close", "Alt+W", CMD_CLOSE, menu::DANGER)));            // in overlay_ran: match (ran.tag, ran.id)
impl Overlay for Palette { // modal: Modal::new(580, h).at_top(60), field: Field, list: List
    fn key(&mut self, k: &Key, cx: &Cx) -> Out {
        match self.list.key(k) { Nav::Chose(e) => return Out::Run(self.list.id(e)), Nav::Moved => return Out::Redraw, _ => {} }
        match self.field.key(k) { Edit::Changed => { self.list.filter(self.field.text()); Out::Redraw }
                                  Edit::Paste => Out::Paste, _ => Out::None } }
    fn draw(&self, p: &mut Paint, cx: &Cx) { let card = self.modal.paint(p, cx.g); /* field::paint, list::paint, footer */ } }
```
`Picker` in `overlay_demo.rs` is this, complete (Home/End go to the field, Ctrl+Home/End to the list). Strings: `tr(Str::..)`.

**Wiring (`wlapp.rs`):** `handle_key` first line, `on_pointer_move` / `PointerButton` / `on_scroll` via `overlay_owns` and
`overlay_reply`, `draw` (the grid is invalidated and the stack painted after the settings panel), the `poll` timeout takes
`overlays.deadline(now)`, `tick` (`Tick::Repaint` while a toast fades, `Gone` when it goes). `AXON_DEMO_OVERLAY=field|list|menu|toast|modal` opens one. Design board: group `overlays`
(`src/design/overlay_scenes.rs`, English and Spanish).

**Cost and rules:** text is the proportional UI font (`UiFont`), loaded when the first overlay opened and dropped when
the last closed (12 ms to load in release; RSS 15.7 to 18.9 MB while one was open; shared with the settings panel,
[0031](0031-review-fixes-paint-path.md)); since [0036](../interface/0036-the-reference-cleaner.md) the window holds it, narrowed, from
its first frame, so an overlay opens without loading anything. Else the grid's font. No cache: rows are summed from the metrics when asked,
positions come from widths. A frame with the palette open costs 3.6 ms over the grid (a menu 1.1 ms; the scrim and the card's
inside are plain fills, `overlay::fill`, because per-pixel coverage over them took 19 ms), repaints the grid and allocates
(glyphs; a toast alone repaints only its box since 0031); idle is 0 % CPU with a list open and with a toast shown and gone. Vertical clipping is a sub-slice of the
framebuffer (`list::clipped`).

**Deviations from the reference:** the scrim does not cover the title bar; a press outside a menu is swallowed, not passed on;
no 0.985 scale in the pop, no fade-out for dialogs; the label weight is regular (no medium); the toast's button lights on hover;
Ctrl+C/X work in fields. **Built on it since:** the palette (0027), the overview (0028), the group dialog, menus, renames and
toasts (0029). **Not yet:** a text field with several lines, IME, a scrollbar you can drag.
