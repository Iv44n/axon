#!/bin/bash
# Packs a release bundle for this machine's architecture, the file a GitHub release carries:
#
#   scripts/package.sh              build, then pack target/dist/axon-<arch>-linux.tar.gz and its .sha256
#   scripts/package.sh --no-build   pack the existing target/release/axon
#
# The bundle holds the binary, the icon, config.example and install.sh (scripts/install.sh, which installs from
# the folder it sits in), so `tar xzf axon-x86_64-linux.tar.gz && axon-x86_64-linux/install.sh` works without the
# web installer. Asset names carry no version, so releases/latest/download/<name> always points at the newest one.
# Build where glibc is old (the release workflow uses Ubuntu 22.04): the binary needs the glibc it was linked against.
set -euo pipefail
cd "$(dirname "$0")/.."

BUILD=1
while [ $# -gt 0 ]; do
  case "$1" in
    --no-build) BUILD=0 ;;
    -h|--help) sed -n '2,10p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "package: unknown option '$1' (try --help)" >&2; exit 2 ;;
  esac
  shift
done

case "$(uname -m)" in
  x86_64|amd64) arch=x86_64 ;;
  aarch64|arm64) arch=aarch64 ;;
  *) echo "package: no release name for architecture $(uname -m)" >&2; exit 1 ;;
esac

[ "$BUILD" = 0 ] || cargo build --release
[ -x target/release/axon ] || { echo "package: target/release/axon is missing (run without --no-build)." >&2; exit 1; }

name="axon-$arch-linux"
dist=target/dist
rm -rf "${dist:?}/$name" "$dist/$name.tar.gz" "$dist/$name.tar.gz.sha256"
mkdir -p "$dist/$name"
install -m755 target/release/axon "$dist/$name/axon"
install -m755 scripts/install.sh "$dist/$name/install.sh"
install -m644 assets/axon.svg assets/config.example "$dist/$name/"

# Same bytes for the same inputs: fixed order, owner and time (the last commit's, or SOURCE_DATE_EPOCH).
epoch=${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct 2>/dev/null || echo 0)}
tar -C "$dist" --sort=name --owner=0 --group=0 --numeric-owner --mtime="@$epoch" -cf - "$name" \
  | gzip -9n > "$dist/$name.tar.gz"
(cd "$dist" && sha256sum "$name.tar.gz" > "$name.tar.gz.sha256")
rm -rf "${dist:?}/$name"

echo "$dist/$name.tar.gz ($(du -h "$dist/$name.tar.gz" | cut -f1)), $("target/release/axon" --version)"
echo "$dist/$name.tar.gz.sha256"
