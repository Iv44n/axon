---
type: decision
status: accepted
updated: 2026-11-25
related: [../core/0010-own-implementations.md, ../core/0007-no-sqlite-egui-tokio.md, ../interface/0021-ui-refactor-terminal-reference.md]
---
# 0035 · Release bundles on GitHub, a `curl | sh` installer and a static site on Pages

**Decision:** a tag `vX.Y.Z` equal to `Cargo.toml`'s version builds `axon-<arch>-linux.tar.gz` (x86_64, aarch64) on Ubuntu 22.04
and attaches it, with a `.sha256`, to a GitHub release (`.github/workflows/release.yml`, `scripts/package.sh`). A bundle holds the
binary, the icon, `config.example` and `scripts/install.sh`, which installs the files beside it when it is not in a checkout: one
installer for source and bundles. `scripts/get.sh` (POSIX sh, served as `https://iv44n.github.io/axon/install.sh`) picks the
architecture, downloads the bundle, checks the SHA-256 and runs that `install.sh`, with sudo only when the prefix is not writable.
The website is one hand-written page in `site/` (the app's tokens, no framework, no request to other hosts, frames of the real
renderer in `site/shots/`), built by `scripts/site.sh` and published by `.github/workflows/pages.yml`. The page is in English and
Spanish: English is the markup (what crawlers and no-JS visitors get), Spanish a dictionary in the page's own script, keyed by
`data-i18n*` attributes; `?lang=`, the visitor's last pick or the browser's languages choose, and an EN/ES switch sits in the header.

**Why:** installing meant a Rust toolchain and a ~100 s fat-LTO build. The binary links only libc, libm and libgcc_s (Wayland,
xkbcommon, X11 and Vulkan are `dlopen`ed), so one build per architecture runs on every glibc distro at least as new as the build
host: linked here (glibc 2.43) it would need 2.43; on 22.04 it needs 2.35 (Ubuntu 22.04, Debian 12, Fedora 36). Asset names
without the version make `releases/latest/download/<name>` a stable URL, so the installer needs no GitHub API and no `jq`. The
whole installer sits in `main()`, so a download cut short runs nothing. The site follows the binary's rules: no dependency, no
build step beyond copying, nothing loaded from elsewhere (fonts come from the system stack).

**Rejected:** a static musl binary (one file for every distro, but `dlopen` of the system's libwayland and libxkbcommon from a
static musl binary does not work); `cargo install` or crates.io (still needs Rust and the long build); distro packages, Flatpak or
AppImage (a packaging job each, later if asked); a static-site generator or a CSS framework for one page (a dependency and a build
step for nothing); the GitHub API for "latest" (rate limits and JSON parsing in sh); a separate installer that writes the desktop
entry itself (two copies of what `install.sh` already does); a second page `es/index.html` (a copy to keep in step, or a build step to make
it); an i18n library.

**Revisit if:** a user needs glibc older than 2.35 or musl (build on an older image, or ship a source-build fallback in `get.sh`);
X11 gets its own client and `winit`/`wgpu` go (the bundle shrinks, the rules stay); the site needs more than one page; a third language, or Spanish that must be found by search engines under its own URL (then
generate one page per language from the same keys).
