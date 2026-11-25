---
type: decision
status: accepted
updated: 2026-11-25
related: [0025-overlay-kit.md, 0028-overview.md, 0023-windows-and-side-panel.md, ../core/0009-data-oriented-no-caches.md, ../core/0005-damage-tracking.md]
---
# 0031 · The paint path after the review: no syscalls, no locks, no copies, damage for toasts

**Decision:** what a frame reads is data the app already holds, filled when something happens, never asked of the
system while painting; a frame never waits for a terminal that is not on screen; a toast is a damage rectangle, not a
whole frame. Found by a review of the overlay, overview, drag and list work (0025 to 0030) against rules 1 and 5.

**Directories.** `Pane::dir` is the shell's directory in short form (`~/lab/axon`), read from `/proc/<pid>/cwd`
(`Pane::read_dir`) when the shell may have moved: at spawn, at each hook prompt, at a new title, when the context probe
reports a move, when a hidden window prints for the first time (`unread`) and when a window comes on screen. The panel,
the tabs, the title bar, the overview and the palette read it (`win_title`, `win_place` return `&str`); before, each
repaint of the list did a `readlink` per window (27 a frame in the overview with nine windows; now 37 in nine seconds
in the same scene, with the toast, the overview and every frame of them). A shell with neither the hook nor a title that moves in a window out of sight shows its old directory until it
prints for the first time, or the window is shown: the limit of this design.

**The list is flat data.** `side::Items` is one pool of text, the windows and the panes under them in the order of the
list; the app keeps one and fills it before each paint (`fill_items`), so painting the panel, the tabs or a spinner step
allocates nothing once it has grown (before: a `String` per window, a `Vec` per multi-pane window, every 40 ms while a
command runs). `side::fit` and the counts are iterators of characters, not strings. The design board is byte-identical
(141 scenes). Same for the palette (`list::highlighted` cuts slices of the label), `overlay::fit` (halving the cut, not
measuring the prefix once per character) and the overview's numbers (`overlay::dec`). Left as they were: the palette's
"nothing matches" line (`format!`, only when the list is empty) and `fontdue`'s bitmap per glyph (0010 removes it).

**Terminals out of sight are not waited for.** The overview reads each card's last rows (and the filter's 30 rows) with
`try_lock_unfair`: a pane the PTY thread is parsing gives nothing this frame (the preview has no text; the filter keeps its
last answer for that card) and the next frame tries again. Measured before: 46 futex waits and 99 ms blocked in a second
with a flood in another window. Under a flood that never lets go the preview stays empty: accepted, nobody reads it. The
filter still scans each frame while one is typed (cheap, no lock); scanning only on change needs a per-pane stamp.

**A toast repaints its box.** `Stack::extent` says what the overlays paint over a frame: the whole window while one is
open (it dims it), the toast's box with its shadow otherwise. The frame under it is given back (`Soft::restore`: the glass
over the box, the status bar again, the rows it touches rewritten with their chips, `rows_under`) instead of repainting
everything; a toast that reaches the title bar, a panel or a pane's frame, or any open overlay, still repaints the window
(`give_back_to_overlays`). Test: a toast fades in, stays and out over a printing terminal, byte-identical to a full frame
each time (`design::a_toast_fading_over_the_window...`), and nothing it paints is outside `Toast::extent`. Measured: the fade
out costs 9 ticks of CPU (90 ms) instead of 20, the toast's own painting (shadow by coverage, text rasterized per frame)
is most of the rest. A window with several panes (framed) still takes the whole frame under a toast.

**The UI font.** `UiFont::shared` hands the settings panel and the overlays one parsed font (a `Weak` finds the live one:
no copy kept, nothing to invalidate) and walks the font directories once for all candidates (`find_any`; before up to six
walks, 1,875 `statx` for one toast); `release_memory` (`malloc_trim`) after the last holder goes gives the pages back: a
window with three shells measured 8.2 MB of anonymous memory, 11.8 after a toast, now 9.3 (a toast and the settings
panel together hold one 3.6 MB font, not two). The font is parsed again at each open from empty (12 ms): the alternative,
keeping it, is a permanent 3.6 MB. `find_family("Ubuntu Sans")` no longer picks `UbuntuSansMono` (same length, directory
order): a name that does not say "mono" ranks the files that continue with it below.

**Fixed in the same pass (correctness).** With the settings docked the grid must keep `wlapp::MIN_GRID` (120 px): the
windows' panel gives way (`fit_side`) in a narrow window, where the workspace used to go negative and `paint_bg` aborted
(the half of a 1024 px screen; also guarded in `Soft::paint_bg`). The palette names windows and groups by id (`Cmd::Window(id)`),
not by place. Closing the last pane or the window from the palette, a cross or `Alt+Shift+W` goes through the undo toast
(`close_pane_asked`); a shell that ends in a pane without the focus leaves the focus alone; a window closing out of sight
does not reset the gestures of the one on screen; `set_tabs` relists; a drag the overlays took the pointer from is
cancelled; the wheel over the docked settings is the overlay's when one is open; keys that act once (Esc closing the
settings, `Ctrl+,`, `Alt+N`, `Alt+B`, `Alt+O`, `Ctrl+Shift+R`) do not repeat while held.

**Revisit if:** the overview needs readable previews of a window that floods (a copy the PTY thread publishes), a
shell without the hook must show its directory live in the background (poll `/proc` on a slow timer), or a toast in a
split window is common (restore round the frames).
