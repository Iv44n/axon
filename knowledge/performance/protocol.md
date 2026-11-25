---
type: reference
status: current
updated: 2026-11-22
related: [measurements.md, ../decisions/core/0008-idle-ram-50mb.md]
---
# Comparison protocol

Targets live in the `axon-performance` skill. This protocol adds what the skill does not
cover: how to compare against other terminals. axon's own numbers come from `scripts/bench.sh`
and are mirrored in the README `Benchmarks` section, which must be updated on every change
that can move them.

## Conditions
Same machine, compositor, font, font size and window size for axon, Alacritty, Ghostty and
foot. axon built with `--release`. Every measurement records machine, GPU, driver and
compositor.

## Metrics
| Metric | How |
|---|---|
| Idle CPU | `pidstat -p <pid> 1 60` with the terminal idle |
| Startup to first frame | Temporary timestamp: process start → first `present()` |
| Key-to-pixel latency | Typometer or 240 fps recording |
| Throughput | `time cat /tmp/big.txt` with `seq 1 12300000 > /tmp/big.txt` (99.6 MB); vtebench as a second reference |
| Idle RAM | RSS (`ps -o rss= -p <pid>`) and PSS (`smem -P <name>`); see [0008](../decisions/core/0008-idle-ram-50mb.md) |
| RAM per session | PSS delta when opening each session and filling the scrollback (once panes exist) |
| Render latency | Temporary counter `Event::Wakeup` → `present()` |
| GPU | `intel_gpu_top`, `nvtop` or `radeontop` during the throughput run |
| Binary | `ls -l target/release/axon` |

Temporary counters and timestamps do not stay in the final code.
