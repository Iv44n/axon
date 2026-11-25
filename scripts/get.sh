#!/bin/sh
# Installs a released axon (no Rust needed): the binary, a launcher entry and the icon, under ~/.local by default.
#
#   curl -fsSL https://iv44n.github.io/axon/install.sh | sh
#   curl -fsSL https://iv44n.github.io/axon/install.sh | sh -s -- --prefix /usr/local
#   curl -fsSL https://iv44n.github.io/axon/install.sh | sh -s -- --version v0.1.0
#   curl -fsSL https://iv44n.github.io/axon/install.sh | sh -s -- --uninstall
#
# It downloads axon-<arch>-linux.tar.gz from the GitHub release (latest unless --version), checks its SHA-256 and
# runs the install.sh inside it (scripts/install.sh). A prefix you cannot write to is installed with sudo.
# AXON_VERSION and AXON_PREFIX do what the options do; AXON_RELEASES replaces the releases URL (a mirror, file://).
# The website serves this file as install.sh (scripts/site.sh).
set -eu

REPO=https://github.com/Iv44n/axon

say() { printf 'axon: %s\n' "$*"; }
die() { printf 'axon: %s\n' "$*" >&2; exit 1; }

# Downloads $1 to $2 with curl or wget.
fetch() {
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL --retry 3 -o "$2" "$1"
  elif command -v wget >/dev/null 2>&1; then
    wget -q --tries=3 -O "$2" "$1"
  else
    die "needs curl or wget to download the release"
  fi
}

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
  elif command -v shasum >/dev/null 2>&1; then shasum -a 256 "$1" | cut -d' ' -f1
  else die "needs sha256sum or shasum to check the download"
  fi
}

# Runs a command as root when the prefix is not ours to write (its nearest existing folder decides).
writable() {
  d=$1
  while [ ! -d "$d" ]; do d=$(dirname "$d"); done
  [ -w "$d" ]
}

uninstall() {
  # The files scripts/install.sh writes; fonts and ~/.config/axon stay.
  as=""
  writable "$1/bin" || as=$(need_root "$1")
  $as rm -fv "$1/bin/axon" "$1/share/icons/hicolor/scalable/apps/axon.svg" \
    "$1/share/applications/axon.desktop" "$1/share/doc/axon/config.example"
  if command -v update-desktop-database >/dev/null 2>&1; then
    $as update-desktop-database -q "$1/share/applications" 2>/dev/null || true
  fi
  say "removed from $1 (your fonts and settings were not touched)"
}

need_root() {
  [ "$(id -u)" = 0 ] && return 0
  command -v sudo >/dev/null 2>&1 || die "cannot write to $1, and there is no sudo: run this as root or pick --prefix"
  say "$1 is not writable by you: the install step runs with sudo" >&2
  echo sudo
}

main() {
  version=${AXON_VERSION:-latest}
  prefix=${AXON_PREFIX:-$HOME/.local}
  remove=0
  while [ $# -gt 0 ]; do
    case "$1" in
      --prefix) [ $# -ge 2 ] || die "--prefix needs a directory"; prefix=$2; shift ;;
      --prefix=*) prefix=${1#--prefix=} ;;
      --version) [ $# -ge 2 ] || die "--version needs a release, such as v0.1.0"; version=$2; shift ;;
      --version=*) version=${1#--version=} ;;
      --uninstall) remove=1 ;;
      -h|--help)
        cat <<'HELP'
Installs a released axon: the binary, a launcher entry and the icon.

  curl -fsSL https://iv44n.github.io/axon/install.sh | sh [-s -- OPTIONS]

  --prefix DIR      where to install (default ~/.local; /usr/local installs for everyone, with sudo)
  --version vX.Y.Z  a given release (default: the latest)
  --uninstall       remove what an install put under the prefix
HELP
        exit 0 ;;
      *) die "unknown option '$1' (try --help)" ;;
    esac
    shift
  done
  case "$prefix" in /*) ;; *) die "--prefix must be an absolute path" ;; esac

  if [ "$remove" = 1 ]; then uninstall "$prefix"; return; fi

  [ "$(uname -s)" = Linux ] || die "axon runs on Linux only (this is $(uname -s))"
  case "$(uname -m)" in
    x86_64|amd64) arch=x86_64 ;;
    aarch64|arm64) arch=aarch64 ;;
    *) die "there is no released binary for $(uname -m); build it from source: $REPO#installation" ;;
  esac
  # The binaries are linked on Ubuntu 22.04: they need glibc 2.35 or newer (musl systems need a source build).
  if glibc=$(getconf GNU_LIBC_VERSION 2>/dev/null); then
    v=${glibc#glibc }; major=${v%%.*}; minor=${v#*.}; minor=${minor%%.*}
    if [ "$major" -lt 2 ] || { [ "$major" = 2 ] && [ "$minor" -lt 35 ]; }; then
      die "this system has glibc $v and the released binary needs 2.35; build it from source: $REPO#installation"
    fi
  else
    die "no glibc found (musl?): the released binary needs glibc 2.35; build it from source: $REPO#installation"
  fi

  case "$version" in latest|v*) ;; *) version="v$version" ;; esac
  base=${AXON_RELEASES:-$REPO/releases}
  name="axon-$arch-linux"
  if [ "$version" = latest ]; then url="$base/latest/download/$name.tar.gz"
  else url="$base/download/$version/$name.tar.gz"; fi

  tmp=$(mktemp -d "${TMPDIR:-/tmp}/axon-install.XXXXXX")
  trap 'rm -rf "$tmp"' EXIT
  trap 'exit 130' INT TERM

  say "downloading $url"
  fetch "$url" "$tmp/$name.tar.gz" || die "download failed: is there a release $version for $arch? ($REPO/releases)"
  fetch "$url.sha256" "$tmp/$name.tar.gz.sha256" || die "could not download the checksum ($url.sha256)"
  want=$(cut -d' ' -f1 < "$tmp/$name.tar.gz.sha256")
  got=$(sha256 "$tmp/$name.tar.gz")
  [ -n "$want" ] && [ "$want" = "$got" ] || die "checksum mismatch: expected $want, got $got; nothing was installed"
  say "checksum ok ($got)"

  tar -xzf "$tmp/$name.tar.gz" -C "$tmp"
  [ -x "$tmp/$name/install.sh" ] || die "the release has no install.sh in $name/"
  command -v bash >/dev/null 2>&1 || die "the bundled installer needs bash"

  as=""
  writable "$prefix" || as=$(need_root "$prefix")
  $as bash "$tmp/$name/install.sh" --prefix "$prefix"
  say "done: run \`axon\`, or open it from your launcher"
}

# Everything runs from here, so a download cut short never runs half a script.
main "$@"
