---
type: log
status: current
updated: 2026-11-22
related: [protocol.md, ../decisions/0008-idle-ram-50mb.md, ../decisions/0009-data-oriented-no-caches.md]
---
# Measurements

Rows are only added, newest on top; one row per terminal per measurement session. The
README `Benchmarks` section carries the current numbers (from `scripts/bench.sh`); this log keeps the detail.
If the table grows past ~40 rows, move the old ones to `measurements-YYYY.md` and link it here.

Reference machine: AMD Ryzen 5 5600H (Cezanne iGPU, Radeon Vega) + NVIDIA RTX 3050 Mobile,
Ubuntu, kernel 7.0.0-44, GNOME Shell 50.1 on Wayland, Mesa 26.0.8, NVIDIA driver 595.99.02.
adapter chosen with `PowerPreference::LowPower`. Window 1000x640, idle shell (`sleep`).

| Date | Terminal | Commit | Idle CPU | Startup | Latency | `cat` 100 MB | RSS | PSS | Notes |
|---|---|---|---|---|---|---|---|---|---|
| 2026-11-24 | axon `wl` (default), lost-frame watchdog, JetBrains Mono | 02c3490+ | 0.0% | - | - | 1.72 s | 15 MB | 10 MB | anon 6; binary 7,498,904 B (-0.7 KB); after the `cat`: 43 MB; `winit` 146 MB, flood 1.32 s; the watchdog only has a timeout while a redraw is blocked, so idle stays at 0 ticks |
| 2026-11-23 | axon `wl` (default), settings screen with typed numbers, JetBrains Mono | 02c3490+ | 0.0% | - | - | 1.75 s | 14 MB | 10 MB | anon 6; binary 7,499,608 B (+15.7 KB: the number box); after the `cat`: 42 MB; `winit` 149 MB, flood 1.50 s (noise: nothing in that path changed) |
| 2026-11-23 | axon `wl` (default), settings screen, JetBrains Mono | 02c3490+ | 0.0% | - | - | 1.74 s | 14 MB | 10 MB | anon 6; binary 7,483,864 B (+74 KB: panel, sfnt reader, config writer, gear); screen open (`AXON_SETTINGS=1`): RSS 18.8 MB, anon 9.3 (own 13 px font, dropped on close); first flood run 1.90 s, second 1.74 s (zed-editor at 14%); `winit` 150 MB |
| 2026-11-23 | axon `wl` (default), config file and font settings, JetBrains Mono | 02c3490+ | 0.0% | - | - | 1.77 s | 14 MB | 10 MB | anon 6; binary 7,409,792 B (+39.7 KB: parser, `axon config` with the embedded template, family search, weight and spacing); after the `cat`: 40 MB; `winit` 150 MB; settings act at tile-bake time, nothing per frame |
| 2026-11-23 | axon `wl` (default), mouse reporting, JetBrains Mono | 02c3490+ | 0.0% | - | - | 1.79 s | 14 MB | 9 MB | anon 6; binary 7,370,048 B (+3.5 KB); after the `cat`: 40 MB; `winit` 149 MB; run with zed-editor at 34% CPU, no tick in 3 s |
| 2026-11-23 | axon `wl` (default), continuous glass, JetBrains Mono | b30bc13+ | 0.0% | - | - | 1.75 s | 13 MB | 9 MB | anon 6; binary 7,366,560 B; after the `cat`: 39 MB; `winit` 149 MB; see window-frame.md |
| 2026-11-23 | axon `wl` (default), own shadow, opacity 0.88, JetBrains Mono | b30bc13+ | 0.0% | - | - | 1.8 s | 14 MB | 10 MB | anon 6; binary 7,366,064 B; after the `cat`: 40 MB; `winit` 149 MB; see window-frame.md |
| 2026-11-23 | axon `wl` (default), sharper text and pixel-aligned buttons, JetBrains Mono | b30bc13+ | 0.0% | - | - | 1.8 s | 14 MB | 10 MB | anon 6; binary 7,362,960 B; after the `cat`: 39 MB; `winit` 155 MB; see software-renderer.md |
| 2026-11-23 | axon `wl` (default) with keyboard and clipboard, JetBrains Mono | b30bc13+ | 0.0% | - | - | 1.8 s | 14 MB | 10 MB | anon 6; binary 7,363,488 B; after the `cat`: 39 MB; `winit` 150 MB; see software-renderer.md |
| 2026-11-23 | axon `AXON_BACKEND=wl`, JetBrains Mono (new default) | b30bc13+ | 0.0% | - | - | 1.8 s | 13 MB | 9 MB | anon 5; after the `cat`: 39 MB; `winit` 149 MB; see software-renderer.md |
| 2026-11-22 | axon `AXON_BACKEND=wl` with the glass chrome, Cascadia NF (then default) | 8db3cc9+ | 0.0% | - | - | 1.7 s | 55 MB | 51 MB | anon 47; binary 7,332,888 B; `winit` 196 MB; see software-renderer.md |
| 2026-11-22 | axon `AXON_BACKEND=wl` (own Wayland client, CPU renderer), JetBrains Mono | 0b72197+ | 0.0% | - | - | 1.7 s | 12 MB | 8 MB | anon 5, file 5, shmem 2; Cascadia NF default: 54 MB; see software-renderer.md |
| 2026-11-22 | spike: CPU into `wl_shm`, 1920x1048 window | not in repo | - | - | - | - | 14 MB | 8.2 MB | anon 3.7, file 1.9, shmem 8.4 (one buffer); see findings |
| 2026-11-22 | spike: CPU into `wl_shm`, 1000x640 window | not in repo | 0.0% | - | - | - | 8.7 MB | 5.6 MB | anon 3.7, file 1.9, shmem 3.1 (one buffer); no wgpu, no winit |
| 2026-11-22 | axon, damage tracking, lazy fonts, Cascadia NF (default) | uncommitted | 0.0% | - | - | 4.3 s | 191 MB | 170 MB | after the 100 MB `cat`: RSS 216 MB, anon 99 MB (scrollback) |
| 2026-11-22 | axon, lazy fonts, JetBrains Mono, radeon ICD only | uncommitted | 0.0% | - | - | - | 119 MB | 107 MB | anon 24 MB, file 94 MB (`VK_LOADER_DRIVERS_SELECT='*radeon*'`) |
| 2026-11-22 | axon, lazy fonts, JetBrains Mono | uncommitted | 0.0% | - | - | - | 148 MB | 127 MB | anon 31 MB, file 117 MB |
| 2026-11-22 | axon, lazy fonts, Cascadia NF (default) | uncommitted | 0.0% | - | - | - | 191 MB | 170 MB | anon 73 MB: the primary font alone is ~45 MB in fontdue |
| 2026-11-22 | axon, before the font fix | uncommitted | 0.0% | - | - | - | 946 MB | 883 MB | anon 827 MB, see findings |

Alacritty, Ghostty and foot are not installed on the reference machine, so no comparison row
exists yet (task 1 in [next-steps](../plan/next-steps.md)).

## Findings

- **CPU renderer and the `wl_shm` spike** (experiment 0, stages A and B of task 15): idle RSS 8.7 MB in the
  spike and 12-54 MB in the real app on the own Wayland client (font dependent), against 191 MB on
  `wgpu`; CPU output within 1/255 of the GPU. Details in [software-renderer](software-renderer.md).


- **Cell buffer redesign** ([0009](../decisions/0009-data-oriented-no-caches.md)): the renderer output is
  byte-identical to the previous one on 5 fixed scenes (block, beam and underline cursors, hollow cursor,
  hidden cursor; every SGR attribute, wide characters, box drawing), and a long incremental run matches a full
  rebuild every frame, both at 0 differing bytes of 2.56 MB. Idle RSS and the 100 MB `cat` time are unchanged
  (191 MB, 4.3 s); the saving is structural (no row vectors, no mirror, 12 B per cell, no `rc`).
- **Swapchain format is `Rgba16Float`** (8 bytes per pixel, 5 MB per image at 1000x640): `Gpu::new` takes the
  first non-sRGB format the surface offers. `Bgra8Unorm` would halve it; task 14 in
  [next-steps](../plan/next-steps.md).

- **Damage tracking** (1000x640 window, 34 rows, workload script, 40 frames): `build()` median
  79 us for a full rebuild, 12 us when 2 rows are rebuilt. Absolute savings are small at this
  size and grow with the window. Incremental output matched a full rebuild byte for byte in all
  40 frames (temporary check, removed). Output floods gain nothing: scrolling is Full damage.

- **fontdue parses every glyph when a font is loaded.** Standalone: DejaVu Sans Mono 9 MB,
  Cascadia Mono NF (13.5k glyphs) 45 MB and 76 ms, Noto Sans CJK (65k glyphs, 19 MB file)
  314 MB and 624 ms. The old code loaded the CJK font at startup and again on
  `ScaleFactorChanged`, which is why idle RSS was ~900 MB.
- **Fix applied:** fallback fonts are loaded on first use, the CJK font is only consulted for
  CJK code points, and a size change recomputes metrics without re-reading files.
- **Floor from the graphics stack:** ~94 MB of file-backed pages (libLLVM from lavapipe, NVIDIA
  and Mesa libraries mapped by the Vulkan loader), even with only the Radeon ICD enabled. This
  is why 50 MB of RSS is not reachable here; the revisit condition of
  [0008](../decisions/0008-idle-ram-50mb.md) is met. The private heap (`anon`) is 24-31 MB
  with a small font.
