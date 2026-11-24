#!/bin/bash
# Installs axon: the binary, a desktop entry (so it shows up in the launcher) and an icon.
#
#   scripts/install.sh                       build, then install under ~/.local
#   scripts/install.sh --prefix /usr/local   system-wide (needs write access; see below)
#   scripts/install.sh --no-build            install the existing target/release/axon
#   scripts/install.sh --uninstall           remove what was installed under the prefix
#
# System-wide as root: build as your user first, then `sudo scripts/install.sh --no-build --prefix /usr/local`
# (root usually has no `cargo` in its PATH). Packagers can set DESTDIR.
set -euo pipefail
cd "$(dirname "$0")/.."

PREFIX="$HOME/.local"
BUILD=1
UNINSTALL=0
while [ $# -gt 0 ]; do
  case "$1" in
    --prefix) [ $# -ge 2 ] || { echo "install: --prefix needs a directory" >&2; exit 2; }; PREFIX=$2; shift ;;
    --prefix=*) PREFIX=${1#--prefix=} ;;
    --no-build) BUILD=0 ;;
    --uninstall) UNINSTALL=1 ;;
    -h|--help) sed -n '2,10p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "install: unknown option '$1' (try --help)" >&2; exit 2 ;;
  esac
  shift
done
case "$PREFIX" in /*) ;; *) echo "install: --prefix must be an absolute path" >&2; exit 2 ;; esac

DEST="${DESTDIR:-}$PREFIX"
BIN="$DEST/bin/axon"
ICON="$DEST/share/icons/hicolor/scalable/apps/axon.svg"
DESKTOP="$DEST/share/applications/axon.desktop"
CONF="$DEST/share/doc/axon/config.example"

refresh_caches() {
  # Best effort: the launcher finds the entry without these, only a little later.
  [ -z "${DESTDIR:-}" ] || return 0
  command -v update-desktop-database >/dev/null && update-desktop-database -q "$PREFIX/share/applications" 2>/dev/null || true
  command -v gtk-update-icon-cache >/dev/null && [ -f "$PREFIX/share/icons/hicolor/index.theme" ] \
    && gtk-update-icon-cache -q -t "$PREFIX/share/icons/hicolor" 2>/dev/null || true
}

if [ "$UNINSTALL" = 1 ]; then
  rm -fv "$BIN" "$ICON" "$DESKTOP" "$CONF"
  refresh_caches
  echo "axon removed from $PREFIX (your fonts and settings were not touched)."
  exit 0
fi

if [ "$BUILD" = 1 ]; then
  command -v cargo >/dev/null || {
    echo "install: cargo not found. Install Rust (https://rustup.rs), or build elsewhere and use --no-build." >&2
    exit 1
  }
  cargo build --release
fi
[ -x target/release/axon ] || { echo "install: target/release/axon is missing (run without --no-build)." >&2; exit 1; }

install -Dm755 target/release/axon "$BIN"
install -Dm644 assets/axon.svg "$ICON"
install -Dm644 assets/config.example "$CONF"

# The Exec line must be an absolute path (the launcher has no PATH of ours); quote it if it has spaces.
exec_path="$PREFIX/bin/axon"
case "$exec_path" in *[[:space:]]*) exec_path="\"$exec_path\"" ;; esac
mkdir -p "$(dirname "$DESKTOP")"
cat > "$DESKTOP" <<DESK
[Desktop Entry]
Type=Application
Name=axon
GenericName=Terminal
Comment=Minimal native terminal emulator
Exec=$exec_path
Icon=axon
Terminal=false
Categories=System;TerminalEmulator;
Keywords=terminal;shell;console;command;
StartupWMClass=axon
DESK
chmod 644 "$DESKTOP"
refresh_caches

echo "Installed axon $(grep -m1 '^version' Cargo.toml | cut -d'"' -f2) under $PREFIX:"
echo "  $BIN"
echo "  $DESKTOP"
echo "  $ICON"
echo "  $CONF   (copy it to ~/.config/axon/config.toml to change fonts and opacity)"

case ":$PATH:" in
  *":$PREFIX/bin:"*) ;;
  *) echo "Note: $PREFIX/bin is not in your PATH; add it to run \`axon\` from a shell (the launcher entry works anyway)." ;;
esac

# Keyboard layouts come from libxkbcommon, loaded at run time; without it axon falls back to winit + wgpu.
if command -v ldconfig >/dev/null && ! ldconfig -p 2>/dev/null | grep -q 'libxkbcommon\.so\.0'; then
  echo "Note: libxkbcommon.so.0 not found (package libxkbcommon0): axon will start its heavier winit fallback."
fi

# axon exits at startup without a monospaced font; mirror the lookup in src/font.rs.
if [ -z "${AXON_FONT:-}" ]; then
  found=0
  for f in "$HOME/.local/share/fonts/JetBrainsMono[wght].ttf" "$HOME/.local/share/fonts/CascadiaMonoNF.ttf" \
           "$HOME/.local/share/fonts/CascadiaMono.ttf" /usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf \
           /usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf /usr/share/fonts/truetype/ubuntu/UbuntuMono-R.ttf; do
    [ -f "$f" ] && { found=1; break; }
  done
  [ "$found" = 1 ] || echo "Warning: no monospaced font found. Install one (e.g. fonts-dejavu-core) or set AXON_FONT=/path/to/font.ttf."
fi
