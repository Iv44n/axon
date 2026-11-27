#!/bin/bash
# Builds the website (GitHub Pages, .github/workflows/pages.yml) into target/site:
#
#   scripts/site.sh            site/, the icon, scripts/get.sh served as install.sh and scripts/get.ps1 as install.ps1
#   scripts/site.sh --serve    the same, then serve it on http://localhost:8000 (python3)
#   scripts/site.sh --shots    refresh site/shots/ from design/frames (run scripts/design.sh first), then build
#
# The pictures are frames of the real renderer (src/design.rs), copied into site/shots/ so the site needs no build.
set -euo pipefail
cd "$(dirname "$0")/.."

SERVE=0
SHOTS=0
while [ $# -gt 0 ]; do
  case "$1" in
    --serve) SERVE=1 ;;
    --shots) SHOTS=1 ;;
    -h|--help) sed -n '2,8p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "site: unknown option '$1' (try --help)" >&2; exit 2 ;;
  esac
  shift
done

if [ "$SHOTS" = 1 ]; then
  # site/shots/<name>.png <- design/frames/<scene>.png
  for pair in split:panes-3 groups:group-dialog palette:palette-typed overview:overview chips:chips settings:prefs hosts:hosts; do
    frame="design/frames/${pair#*:}.png"
    [ -f "$frame" ] || { echo "site: $frame is missing (run scripts/design.sh)" >&2; exit 1; }
    cp "$frame" "site/shots/${pair%%:*}.png"
  done
  echo "site/shots/ refreshed from design/frames/"
fi

out=target/site
rm -rf "$out"
mkdir -p "$out"
cp -r site/. "$out/"
cp assets/axon.svg "$out/axon.svg"
cp scripts/get.sh "$out/install.sh"
cp scripts/get.ps1 "$out/install.ps1"
echo "built $out ($(du -sh "$out" | cut -f1))"

if [ "$SERVE" = 1 ]; then
  echo "serving on http://localhost:8000 (Ctrl+C stops)"
  exec python3 -m http.server 8000 --bind 127.0.0.1 --directory "$out"
fi
