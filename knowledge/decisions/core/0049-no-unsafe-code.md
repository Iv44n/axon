---
type: decision
status: accepted
updated: 2026-11-27
related: [0010-own-implementations.md, 0011-software-render-wl-shm.md, 0013-wayland-default-xkb-dlopen.md, 0039-windows-port.md, ../workspace/0031-review-fixes-paint-path.md, ../../performance/measurements.md]
---
# 0049 · No `unsafe`: the compiler refuses it outside three places at the system's edge

**Decision:** `Cargo.toml` sets `unsafe_code = "deny"` for the whole crate. Only `xkb.rs` (the `libxkbcommon` calls),
`win.rs` (Win32) and the `shm` module of `wl.rs` (the mapping of the window's pixels) allow it, by name. A new `unsafe` anywhere
else does not build; one inside those places needs a case for why no safe form exists. The user asked for it on 2026-11-27.

**What went (93 lines with `unsafe` in 7 files, now 80 in 3; 42 in the Linux build, 59 before):**
- `restore.rs`: the test set `AXON_SESSION` with `env::set_var` (`unsafe` since edition 2024). `path_in`, `load_from`, `save_to`
  take the value and the file instead.
- `wl.rs`, the keymap: read with `pread` instead of mapped. The compositor's file may be shared with other clients; `pread` does
  not move its position (tested on a memfd). Mutter's keymap, 38,401 bytes, came through whole.
- `context.rs`, the clock chip: `localtime_r` declared by hand is replaced by `tz.rs`, an own reader of the zone database as the C
  library reads it (`TZ`, `:name`, a path, a POSIX rule, else `/etc/localtime`; TZif v1-v3 per RFC 8536, the footer's rule after
  the last transition). Checked against glibc's `date +%z`: 487 zones x 9 instants from 1906 to 2100, all equal, and every form of `TZ`.
- `plat.rs`, Windows' `Wake`: moved to `win.rs` and held in std's `OwnedHandle`, which closes it and is `Send + Sync`: no more
  `unsafe impl Send/Sync` or hand-written `Drop`. Windows' `GetLocalTime` moved there too.
- `font.rs`: `release_memory` (`malloc_trim`, ADR 0031) removed. Since the UI font is held by `Soft` (ADR 0036) and shared, closing
  an overlay or a panel frees little: A/B, three runs each, idle anon 8.56 MB with and without it; after a toast 8.34 against
  8.56 MB. It gave back 2.5 MB only with no UI font installed (the overlays then parse a monospaced one), which none of
  `UI_FAMILIES` missing (DejaVu Sans among them) makes nearly impossible on Linux; even then the pages are reused by the next overlay.

**What stays, and why:**
- `wl.rs` `shm::Pixels` (4 lines): `mmap`/`munmap` of the pool and the slice over it. The renderer draws straight into the buffer
  the compositor reads. The safe way, `pwrite` into the memfd, costs a copy of the whole frame every frame (3 MB at 1000x640,
  8 MB at 1080p, 33 MB at 4K, up to 120 times a second in a flood) and a second frame of memory: refused. The pool is now sealed
  against shrinking (`F_SEAL_SHRINK`), so the compositor cannot cut the file under the mapping. Copying the pixels to the second
  buffer is a safe `copy_from_slice`.
- `xkb.rs` (38): every call through `dlopen`/`dlsym` pointers. Removed by the own keymap parser of stage C
  (`plan/software-renderer.md`), not by a wrapper crate (that only moves the `unsafe` out of sight).
- `win.rs` (38): Win32 has no safe declarations; the window procedure's state pointer and the DIB section's bits are the system's
  memory. A thread-local state instead of the pointer is possible, but cannot be run here: a change for its own ADR.

**Why:** memory safety proved by the compiler, not by `SAFETY:` comments; the few places that cannot have it are named, small and
reviewed. The cost was 6.3 KB of binary (8,590,400 to 8,596,704 B, `tz.rs` most of it); the flood A/B against `7bd7092`, five
runs each in turn, 1.526 against 1.539 s (noise), idle as before, the design board's 163 frames byte-identical.

**Rejected:** `forbid` (would not let the three places allow it); a crate such as `memmap2` or `xkbcommon` (more dependencies, the
same `unsafe` inside them, ADR 0010); `pwrite` for the pixels (above); keeping `malloc_trim` for an almost impossible case.

**Revisit if:** the own keymap parser lands (`xkb.rs` loses its permission); a safe interface to the system's shared memory appears
in `std`; the own font reader (`plan/own-implementations.md`, item 1) wants to map font files: that is a new exception, argued here.
