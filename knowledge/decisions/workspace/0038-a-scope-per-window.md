---
type: decision
status: accepted
updated: 2026-11-25
related: [0023-windows-and-side-panel.md, 0034-a-group-is-the-split.md, ../core/0010-own-implementations.md, ../../performance/measurements.md]
---
# 0038 · Every window's shell starts in a systemd scope of its own

**Decision:** `[windows] scope = true` (the default) starts each window's shell as `systemd-run --user --scope` does, so it
lives in `app.slice/app-axon-<axon's pid>w<window>.scope` (described "axon window N"), apart from axon's other windows and from
axon itself. Where systemd's user manager does not answer, or the setting is off, the shell starts as it always did.

**The problem it answers (measured 2026-11-25).** A system monitor (GNOME Resources, `systemd-cgtop`) showed "axon" at ~4 GB
while the `axon` process held 61 MB. GNOME makes one scope per launched app (`app-gnome-axon-<pid>.scope`) and everything started
from a terminal inherits it: that scope held Zed, its language servers, `claude-acp` and an MCP server (4.9 GB, 2.8 GB of it file
cache), and it stayed "running" after the `axon` process in it was closed. `systemd-oomd` kills by cgroup: it killed such a scope
(12.7 GB peak) with every window in it. With one scope per window a runaway program takes its own window, and the monitor
says which window holds the memory.

**What the others do.** VTE (GNOME Terminal) gives each spawn a `vte-spawn-<uuid>.scope` through `StartTransientUnit` (found in the
installed `libvte`). Warp does not (its binary has no such call): its `terminal-server` and every shell sat in
`org.gnome.Shell@ubuntu.service`, the cgroup of GNOME Shell itself, probably because the server was forked before GNOME moved the
app into its scope (not verified). Ghostty and Konsole have per-surface cgroups too (sources: mirrors and a merge request).

**How (`session.rs`).** `tty::Options.shell` is `systemd-run --user --scope --collect --quiet --slice=app.slice --unit=… --
$SHELL`. Measured on this machine: `systemd-run --scope` registers its own pid in the scope and `exec`s the command in that
process, so the pid `pty.child()` reports is the shell's and `context.rs` reads `/proc/<pid>` as before; it adds ~1-8 ms to
a window's start (timed, in turn, on and off: 57-59 ms to the first shell off, 60-65 on). `--quiet` keeps its notice off the
terminal. It turns `$$` into `$` in arguments even for a scope, so a `$` in `$SHELL` is doubled. No `$SHELL`: no scope.

**It must not leave a window without a shell** (the last window closing quits axon). `scope_possible` checks the cheap facts (systemd
booted, the user manager's socket or the user bus, `systemd-run` on the `PATH`); then `scope_probe` runs `systemd-run … -- true`
while the window is being made and `works()` awaits it, at most 2 s, before the first shell. A failure, a hang or a missing piece
means no scope for the session. Tried: no `systemd-run` on the `PATH`, one that exits 1, one that hangs (2.16 s to the shells, then
the old behaviour): axon starts every time. `scope = true` set at a reload (it was off at the start) probes then, once.

**Rejected:**
- Calling `StartTransientUnit` over D-Bus as VTE does: axon has no D-Bus client, and an own one (auth, marshalling) is a lot for
  one call (0010 prefers our own code to a dependency, not to a new subsystem).
- A `sh -c 'systemd-run … || exec $SHELL'` wrapper: `sh` would stay the PTY's child, so `/proc/<pid>` would be `sh`'s, and a
  shell that exits non-zero could not be told from a `systemd-run` that failed.
- Off by default: the user's problem is the default behaviour; the probe makes on safe.
- A row in the settings panel: it is an escape hatch, not something to flip; the file key and `config show` carry it.

**Left as it was:** a program that outlives its shell (`nohup zed &`) keeps its window's scope alive until it ends, now named for
the window. The scope of a window that ends is collected (`--collect`); none was left after closing an instance (checked).
Whether GNOME Resources folds the per-window scopes under one "axon" entry or lists them apart is not checked; the unit names follow
the freedesktop `app-<id>-<random>.scope` convention so it can do either.

**Revisit if:** a second platform lands (macOS and Windows have no scopes: the code is Linux/systemd only, behind the probe); a D-Bus
client is written for another reason (then `StartTransientUnit` replaces the `exec`); a user manager that is slow but alive makes the
2 s probe cut it off.
