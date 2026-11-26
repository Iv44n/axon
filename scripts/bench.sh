#!/bin/bash
# Measures the release build and prints the numbers the README "Benchmarks" section needs.
#
#   scripts/bench.sh            binary size, idle and flood numbers for both backends
#   scripts/bench.sh --clean    also times a cold build in a throwaway target directory
#
# Windows open and close on screen for a few seconds each. Run it on an idle machine, with
# nothing else heavy running, and paste the output into README.md and knowledge/performance/.
set -euo pipefail
export LC_ALL=C   # decimal points, whatever the user locale
cd "$(dirname "$0")/.."
# The runs below are not the user's session: they must neither restore it nor overwrite it (src/restore.rs).
export AXON_SESSION=off

cargo build --release -q
BIN=target/release/axon
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

printf '#!/bin/bash\nexec sleep 600\n' > "$TMP/idle.sh"
printf '#!/bin/bash\ns=$(date +%%s.%%N); cat %s/big.txt; e=$(date +%%s.%%N)\necho "$e - $s" | bc > %s/flood.result\nexec sleep 600\n' "$TMP" "$TMP" > "$TMP/flood.sh"
chmod +x "$TMP/idle.sh" "$TMP/flood.sh"
# `yes` dies of SIGPIPE when `head` is done: that is not a failure.
( set +o pipefail; yes "abcdefghij klmnopqrst 0123456789 the quick brown fox jumps over the lazy dog 0123456789 ABCDEFGHIJ" | head -c 99600000 > "$TMP/big.txt" )

kb() { awk -v f="$1" '$1==f {print $2}' "$2"; }
mb() { awk -v v="$1" 'BEGIN {printf "%.0f", v / 1024}'; }

# idle <backend> [extra env...]: prints "RSS PSS anon cpu_ticks_in_3s"
idle() {
  local backend=$1; shift
  env AXON_BACKEND="$backend" SHELL="$TMP/idle.sh" "$@" "$BIN" >/dev/null 2>&1 &
  local pid=$!
  sleep 4
  local t0 t1
  t0=$(awk '{print $14+$15}' /proc/$pid/stat)
  sleep 3
  t1=$(awk '{print $14+$15}' /proc/$pid/stat)
  echo "$(mb "$(kb VmRSS: /proc/$pid/status)") $(mb "$(kb Pss: /proc/$pid/smaps_rollup)") $(mb "$(kb RssAnon: /proc/$pid/status)") $((t1 - t0))"
  kill $pid 2>/dev/null; wait $pid 2>/dev/null || true
}

# flood <backend>: prints "seconds RSS_MB_after"
flood() {
  rm -f "$TMP/flood.result"
  env AXON_BACKEND="$1" SHELL="$TMP/flood.sh" "$BIN" >/dev/null 2>&1 &
  local pid=$!
  for _ in $(seq 1 120); do sleep 1; [ -f "$TMP/flood.result" ] && break; done
  sleep 2
  echo "$(printf '%.2f' "$(cat "$TMP/flood.result")") $(mb "$(kb VmRSS: /proc/$pid/status)")"
  kill $pid 2>/dev/null; wait $pid 2>/dev/null || true
}

DEJAVU=/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf
CASCADIA=$HOME/.local/share/fonts/CascadiaMonoNF.ttf   # the heavy one: 13.5k glyphs
size=$(stat -c %s "$BIN")
echo "commit:  $(git rev-parse --short HEAD)$(git diff --quiet HEAD -- src Cargo.toml Cargo.lock || echo '+dirty')"
echo "date:    $(date +%F)"
echo "machine: $(grep -m1 'model name' /proc/cpuinfo | cut -d: -f2 | xargs), kernel $(uname -r), ${XDG_CURRENT_DESKTOP:-?} on ${XDG_SESSION_TYPE:-?}"
echo "binary:  $size bytes ($(awk -v s="$size" 'BEGIN {printf "%.1f", s / 1048576}') MiB)"
if [ "${1:-}" = "--clean" ]; then
  start=$(date +%s)
  CARGO_TARGET_DIR="$TMP/target" cargo build --release -q
  echo "cold build: $(( $(date +%s) - start )) s"
fi
echo
echo "| Backend | Font | Idle RSS | Idle PSS | Idle anon | Idle CPU (ticks/3 s) |"
echo "|---|---|---|---|---|---|"
for backend in wl winit; do
  for font in default cascadia dejavu; do
    case "$font" in
      default) r=$(idle "$backend") ;;
      cascadia) [ -f "$CASCADIA" ] || continue; r=$(idle "$backend" AXON_FONT="$CASCADIA") ;;
      dejavu) [ -f "$DEJAVU" ] || continue; r=$(idle "$backend" AXON_FONT="$DEJAVU") ;;
    esac
    set -- $r
    echo "| $backend | $font | $1 MB | $2 MB | $3 MB | $4 |"
  done
done
echo
echo "| Backend | 100 MB flood | RSS after |"
echo "|---|---|---|"
for backend in wl winit; do
  set -- $(flood "$backend")
  echo "| $backend | $1 s | $2 MB |"
done
