---
type: decision
status: accepted
updated: 2026-11-23
related: [0010-own-implementations.md, 0011-software-render-wl-shm.md, 0012-glass-chrome-black-white.md, ../plan/software-renderer.md]
---
# 0013 · The own Wayland client is the default; keyboard layouts through `libxkbcommon` loaded at run time

**Decision:** on a Wayland session axon starts on `wl.rs` + `soft.rs` (stage C of
[software-renderer](../plan/software-renderer.md) done), not on `winit` + `wgpu`. `winit` stays for X11 and as the
fallback; `AXON_BACKEND=winit` forces it. The backend falls back by itself, before showing anything, when
there is no Wayland socket or no `libxkbcommon`.

**Why now:** the window chrome of [0012](0012-glass-chrome-black-white.md) only exists on the CPU path, and a
window you cannot type in is not usable. Idle RSS is 14 MB against 150 MB ([measurements](../performance/measurements.md)).

**Keyboard layouts (`xkb.rs`):** the compositor sends the layout as xkb keymap text and raw evdev keys. Turning
those into keysyms and text (levels, AltGr, dead keys, compose sequences) is what `libxkbcommon` does, and a
Spanish or Latin American layout needs all of it (`@` is AltGr+Q, accents are dead keys). It is opened with
`dlopen` at start-up, not linked: no Cargo dependency, no build-time header, and a missing library means the
fallback, not a crash. Cost: +30 KB binary, +1 MB idle RSS (library and compose table).

**Rejected:**
- *Own keymap parser now:* the text is regular, but levels, virtual modifiers and compose tables are weeks of
  edge cases, and a wrong key is worse than a MB. It stays on the plan ([0010](0010-own-implementations.md)); `xkb.rs`
  is the only file that would change.
- *Link `libxkbcommon` normally:* needs the `-dev` package to build and cannot fail soft.
- *`xkbcommon` or `smithay-client-toolkit` crates:* a dependency tree for ~15 functions.

**Also decided with it:** client-side key repeat (Wayland sends no repeats; the loop blocks in `poll` with a
timeout only while a key is held, so idle stays 0%), the clipboard as `wl_data_device` text (our source is served
from a thread, a paste reads a pipe with a 1 s limit), and wheel scrolling from `wl_pointer.axis` +
`axis_discrete`.

**Not covered yet:** input methods (CJK composition needs `text-input-v3`), primary selection (middle click),
mouse reporting to applications (task 6), drag and drop.

**Revisit if:** a distribution ships without `libxkbcommon.so.0` (then write the own parser), or compose
sequences from the user's locale misbehave.
