# Packs the Windows release bundle, the file a GitHub release carries (the counterpart of scripts/package.sh):
#
#   scripts\package.ps1              build, then pack target\dist\axon-x86_64-windows.zip and its .sha256
#   scripts\package.ps1 -NoBuild     pack the existing target\release\axon.exe
#
# The bundle holds axon.exe, the icon, config.example and install.ps1 (scripts/install.ps1, which installs from the
# folder it sits in). The name carries no version, so releases/latest/download/<name> is always the newest one.
param([switch]$NoBuild)
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot)

if (-not $NoBuild) {
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw 'package: cargo build failed' }
}
$exe = 'target\release\axon.exe'
if (-not (Test-Path $exe)) { throw "package: $exe is missing (run without -NoBuild)" }

$name = 'axon-x86_64-windows'
$dist = 'target\dist'
$stage = Join-Path $dist $name
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $stage, "$dist\$name.zip", "$dist\$name.zip.sha256"
New-Item -ItemType Directory -Force $stage | Out-Null
Copy-Item $exe, 'assets\axon.ico', 'assets\config.example', 'scripts\install.ps1' $stage

Compress-Archive -Path $stage -DestinationPath "$dist\$name.zip" -CompressionLevel Optimal
Remove-Item -Recurse -Force $stage

# `sha256sum` format (hash, two spaces, name), so the release job checks it like the Linux ones.
$hash = (Get-FileHash -Algorithm SHA256 "$dist\$name.zip").Hash.ToLower()
[IO.File]::WriteAllText((Join-Path (Resolve-Path $dist) "$name.zip.sha256"), "$hash  $name.zip`n")

$exeBytes = (Get-Item $exe).Length
$mb = [math]::Round((Get-Item "$dist\$name.zip").Length / 1MB, 2)
Write-Host "axon.exe: $exeBytes bytes ($([math]::Round($exeBytes / 1MB, 1)) MiB)"
Write-Host "$dist\$name.zip ($mb MB)"
Write-Host "$dist\$name.zip.sha256"
