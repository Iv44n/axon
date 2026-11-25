# Installs a released axon on Windows (no Rust needed): the exe, a Start menu shortcut and the icon, for you only.
#
#   irm https://iv44n.github.io/axon/install.ps1 | iex
#   & ([scriptblock]::Create((irm https://iv44n.github.io/axon/install.ps1))) -Version v0.1.0
#   & ([scriptblock]::Create((irm https://iv44n.github.io/axon/install.ps1))) -Uninstall
#
# It downloads axon-x86_64-windows.zip from the GitHub release (latest unless -Version), checks its SHA-256 and runs
# the install.ps1 inside it (scripts/install.ps1). $env:AXON_VERSION and $env:AXON_PREFIX do what the options do
# (the one-liner takes no options); $env:AXON_RELEASES replaces the releases URL (a mirror, file:///).
# The website serves this file as install.ps1 (scripts/site.sh), next to the Linux install.sh.
param(
    [string]$Version = $env:AXON_VERSION,
    [string]$Prefix = $env:AXON_PREFIX,
    [switch]$Uninstall
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'   # the progress bar slows Invoke-WebRequest to a crawl in Windows PowerShell

$Repo = 'https://github.com/Iv44n/axon'
function Say($msg) { Write-Host "axon: $msg" }
function Die($msg) { throw "axon: $msg" }

# Windows PowerShell 5.1 does not offer TLS 1.2 to GitHub by itself.
try { [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12 } catch { }

if ($env:OS -ne 'Windows_NT') { Die "this installer is for Windows (on Linux: curl -fsSL https://iv44n.github.io/axon/install.sh | sh)" }
if (-not $Prefix) { $Prefix = Join-Path $env:LOCALAPPDATA 'Programs\axon' }

# The installer is a script file, which a default Windows PowerShell refuses to run (execution policy `Restricted`);
# this one is run in a process of its own, with the policy off for it alone.
function Invoke-Installer($script) {
    $a = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $script, '-Prefix', $Prefix)
    if ($Uninstall) { $a += '-Uninstall' }
    & (Get-Process -Id $PID).Path @a
    if ($LASTEXITCODE -ne 0) { Die "the installer failed (exit code $LASTEXITCODE)" }
}

if ($Uninstall) {
    $installed = Join-Path $Prefix 'install.ps1'
    if (-not (Test-Path $installed)) { Die "there is no install in $Prefix (-Prefix names another place)" }
    Invoke-Installer $installed
    return
}

# Windows on ARM runs the x64 build through its emulation.
switch ($env:PROCESSOR_ARCHITECTURE) {
    { $_ -in 'AMD64', 'ARM64' } { $arch = 'x86_64' }
    default { Die "there is no released binary for $env:PROCESSOR_ARCHITECTURE; build it from source: $Repo#installation" }
}

if ($Version -and $Version -ne 'latest' -and -not $Version.StartsWith('v')) { $Version = "v$Version" }
$base = if ($env:AXON_RELEASES) { $env:AXON_RELEASES } else { "$Repo/releases" }
$name = "axon-$arch-windows"
$url = if (-not $Version -or $Version -eq 'latest') { "$base/latest/download/$name.zip" } else { "$base/download/$Version/$name.zip" }

# Downloads $1 to $2; a file:/// URL is copied.
function Get-File($from, $to) {
    if ($from.StartsWith('file:')) { Copy-Item -Force ([Uri]$from).LocalPath $to; return }
    Invoke-WebRequest -UseBasicParsing -Uri $from -OutFile $to
}

$tmp = Join-Path ([IO.Path]::GetTempPath()) ("axon-install-" + [Guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Force $tmp | Out-Null
try {
    Say "downloading $url"
    try { Get-File $url "$tmp\$name.zip" } catch { Die "download failed: is there a release $Version for Windows? ($Repo/releases) $($_.Exception.Message)" }
    try { Get-File "$url.sha256" "$tmp\$name.zip.sha256" } catch { Die "could not download the checksum ($url.sha256)" }
    $want = ((Get-Content -Raw "$tmp\$name.zip.sha256").Trim() -split '\s+')[0].ToLower()
    $got = (Get-FileHash -Algorithm SHA256 "$tmp\$name.zip").Hash.ToLower()
    if (-not $want -or $want -ne $got) { Die "checksum mismatch: expected $want, got $got; nothing was installed" }
    Say "checksum ok ($got)"

    Expand-Archive -Force "$tmp\$name.zip" $tmp
    $installer = Join-Path $tmp "$name\install.ps1"
    if (-not (Test-Path $installer)) { Die "the release has no install.ps1 in $name\" }
    Invoke-Installer $installer
} finally {
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $tmp
}
