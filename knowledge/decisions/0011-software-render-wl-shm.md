---
type: decision
status: accepted
updated: 2026-11-22
related: [0008-idle-ram-50mb.md, 0009-data-oriented-no-caches.md, 0010-own-implementations.md, ../plan/own-implementations.md, ../performance/measurements.md]
---
# 0011 · Software rendering into `wl_shm`; no GPU stack

**Decision (accepted by the user on 2026-11-22):** replace `winit` + `wgpu` with an own
Wayland client (`xdg-shell`, `wl_shm`, `memfd`) and a CPU renderer that writes the cell grid straight
into the shared buffer. The 50 MB idle RSS target of [0008](0008-idle-ram-50mb.md) stays and becomes
reachable. 0008 is `superseded-by-0011`: the metric stays RSS. Work plan: [software-renderer](../plan/software-renderer.md).

**Why:** experiment 0 ([measurements](../performance/measurements.md)).
- A window with a CPU renderer on `wl_shm` idles at **8.7 MB RSS** (1000x640) and 14 MB (1080p), with
  0% CPU. The same scene through `wgpu` is 119-191 MB.
- The floor of the GPU path is the Vulkan loader and the driver libraries (~94 MB of file-backed pages,
  still there with one ICD enabled). No amount of data-oriented work in our code can remove it.
- A terminal draws a grid of small bitmaps: no shaders, 3D or large textures are needed. The CPU cost
  is small: a naive full-frame redraw is 1.9 ms at 1000x640 and 3.9 ms at 1080p, and damage tracking
  ([0005](0005-damage-tracking.md)) means most frames touch a few rows.
- It fits [0010](0010-own-implementations.md): the protocol subset is small (registry, compositor,
  shm, xdg-shell, later seat, data device, fractional scale) and the wire format is simple.

**Budget it implies (idle, 1080p window):** ~14 MB window and buffer, + the atlas (~2.3 MB for a
13.5k-glyph font), + scrollback at 12 B per cell (5k lines x 120 columns = 7 MB). The buffer costs
4 B per pixel per buffer: 8 MB at 1080p, 33 MB at 4K, so 4K needs one buffer and care.

**Rejected:**
- *Keep `wgpu`, measure PSS instead of RSS:* PSS was 107-170 MB, still 2-3x the target, and it hides a
  cost the user pays.
- *`wgpu` with one Vulkan driver:* measured 119 MB.
- *Raw Vulkan or OpenGL/EGL:* not measured, but the loader and the driver are the floor, not `wgpu`.

**Open questions (answer before or during the work):**
- Double buffering: Mutter releases shm buffers right after copying, so one buffer is enough there
  (measured in stage B); other compositors need a second buffer (+8 MB at 1080p), which `wl.rs` allocates
  only when the first is still busy. Untested on a compositor that holds buffers.
- HiDPI and fractional scale (`wp_fractional_scale_v1` + `wp_viewporter`) enlarge the buffer.
- X11 (MIT-SHM or `XPutImage`) and the clipboard (`arboard`) need their own clients later.
- Text quality: reproduce the squared-coverage blend of the shader on the CPU; compare pixels.

**Revisit if:** a realistic flood (`cat` of 100 MB) at 4K cannot hold 60 fps on the CPU after row
damage and SIMD, or the user prefers a GPU path with a higher memory target.
