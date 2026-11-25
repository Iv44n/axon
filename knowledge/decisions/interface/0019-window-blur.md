---
type: decision
status: accepted
updated: 2026-11-24
related: [0017-settings-screen.md, 0012-glass-chrome-black-white.md, ../core/0011-software-render-wl-shm.md]
---
# 0019 · Window blur is a request to the compositor

**Decision:** `[window] blur = true` (default false; a switch under Opacity in the settings screen) asks the
compositor to blur what is behind the window. It is a different setting from `opacity`: opacity is how much of the
background shows through the glass, blur is how sharp that background is.

**Why a request and not an own effect:** a Wayland client cannot read what is behind its window, so it cannot blur it
itself. Only the compositor can. Two protocols exist and `wl.rs` speaks both, preferring the first:
- `ext_background_effect_manager_v1` (`ext-background-effect-v1`): `get_background_effect(surface)`, then
  `set_blur_region(region)`. The `capabilities` event says whether blur is available; the region is not set before it
  arrives (that is a protocol error).
- `org_kde_kwin_blur_manager` (KDE Plasma): `create(surface)`, `set_region`, `commit`; `unset` to remove.
Neither takes a radius: the compositor chooses the strength, so the setting is on/off, not a number.

**The region** is the window's rounded outline (`chrome::blur_strips`: one 1 px strip per row of each corner, one for
the middle), not the whole buffer, so the shadow margin is not blurred. Maximized it is the plain rectangle. It is
resent on every geometry change. No memory is added: a few rectangles and one object per protocol.

**Limit that matters:** GNOME 50 (Mutter), the desktop this was developed on, advertises neither protocol (checked
2026-11-24 against its registry). There the switch saves and does nothing; the settings screen says "not offered here"
next to it instead of pretending. Rejected: drawing a blurred copy of the wallpaper behind the glass. It would look
like blur until any other window sits behind axon, and it needs a full-screen bitmap in memory (rule 5 and the
50 MB goal both argue against it). Revisit if Mutter adds `ext-background-effect-v1`: nothing else needs to change.

**Not verified here:** no compositor with blur is installed, so the blur request itself was only checked by tests
(`blur_strips`, the settings switch, config round trip) and by the app running without protocol errors on GNOME.
