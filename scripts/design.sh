#!/bin/bash
# Renders the design board frames from the real renderer (src/design.rs) into design/frames/.
#
#   scripts/design.sh              regenerate design/frames
#   scripts/design.sh --baseline   first move the current frames to design/baseline, to compare
#                                  a change against them (the board's "Baseline" and "Split" modes)
#
# Then open design/board.html in a browser. Both directories are build output (git-ignored).
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ "${1:-}" == "--baseline" ]]; then
  if [[ -d design/frames ]]; then
    rm -rf design/baseline
    mv design/frames design/baseline
    echo "baseline: design/baseline ($(sed -n 's/.*stamp: "\(.*\)",/\1/p' design/baseline/frames.js))"
  else
    echo "no design/frames to keep as baseline; run once without --baseline first" >&2
    exit 1
  fi
fi

stamp="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
git diff --quiet 2>/dev/null || stamp+="+dirty"
export AXON_DESIGN_STAMP="$stamp $(date -I)"
cargo test --release -q --bin axon design::frames -- --ignored --exact >/dev/null

# The PNG encoder is our own: when Python is around, have a real decoder read every frame back.
if command -v python3 >/dev/null; then
  python3 -I - design/frames/*.png <<'EOF'
import struct, sys, zlib
for path in sys.argv[1:]:
    data = open(path, 'rb').read()
    w, h = struct.unpack('>II', data[16:24])
    idat, i = b'', 8
    while i < len(data):
        n, = struct.unpack('>I', data[i:i + 4])
        if data[i + 4:i + 8] == b'IDAT':
            idat += data[i + 8:i + 8 + n]
        i += 12 + n
    assert len(zlib.decompress(idat)) == h * (w * 4 + 1), path
EOF
fi
echo "frames: design/frames ($(ls design/frames/*.png | wc -l) scenes, $(du -sh design/frames | cut -f1), $AXON_DESIGN_STAMP)"
echo "board:  file://$PWD/design/board.html"
