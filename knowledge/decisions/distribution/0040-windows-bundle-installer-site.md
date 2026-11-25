---
type: decision
status: accepted
updated: 2026-11-25
related: [0035-release-bundles-web-installer-site.md, ../core/0039-windows-port.md]
---
# 0040 · The Windows release is a zip with a PowerShell installer, behind its own self-test

**Decision:** a tag also builds `axon-x86_64-windows.zip` (+ `.sha256`, `sha256sum` format) on `windows-latest`
(`.github/workflows/release.yml`, `scripts/package.ps1`), and publishes it with the Linux bundles only after the exe passed its own
self-test (`scripts/selftest.ps1`, [0039](../core/0039-windows-port.md)). The zip holds `axon.exe`, `axon.ico`, `config.example`
and `scripts/install.ps1`, which installs from the folder it is in or from a checkout (`target\release`): files under
`%LOCALAPPDATA%\Programs\axon` for the current user (no administrator), that folder on the user's `PATH`, a Start menu shortcut,
and an entry under *Settings > Apps* whose Uninstall runs `install.ps1 -Uninstall`. `scripts/get.ps1`, served by Pages as
`https://iv44n.github.io/axon/install.ps1`, is `irm | iex`: it downloads the zip, checks the SHA-256 and runs `install.ps1` in a
process of its own with `-ExecutionPolicy Bypass` (a default Windows PowerShell refuses script files). The site has a Linux/Windows
picker (the visitor's last pick, else the system they are on) in the hero and the install section; what is for the other system is hidden.
`.github/workflows/windows.yml` runs the whole chain (package, install from a file:// "release", uninstall) on every push to `main`.

**Why:** the same shape as 0035 (asset names without the version, no API, the checksum), in the tools Windows has: PowerShell and
zip need nothing installed. A per-user install needs no elevation and cannot break the system. The `.ico` is embedded in the exe for
the window and used by the shortcut and the Apps entry.

**Rejected:** an MSI or MSIX (a packaging toolchain, signing, for a single exe); winget or Scoop manifests (later, when asked: they
point at this zip); installing for all users (needs elevation); a code-signing certificate (not available: Windows SmartScreen may warn
on a downloaded exe; `irm | iex` and the installer are not blocked by it).

**Revisit if:** SmartScreen warnings become a support burden (sign the exe); an ARM64 build is wanted (a second runner and zip);
a winget manifest is wanted.
