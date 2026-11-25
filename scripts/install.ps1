# Installs axon on Windows from the folder this script sits in (a release bundle, scripts/package.ps1), or removes it:
#
#   .\install.ps1                      install under %LOCALAPPDATA%\Programs\axon, for you, no administrator needed
#   .\install.ps1 -Prefix D:\Tools\axon
#   .\install.ps1 -Uninstall           remove what an install put there (your settings in %APPDATA%\axon stay)
#
# An install copies axon.exe, the icon, config.example and this script into the prefix, puts the prefix on your PATH
# (so `axon` works in a new terminal), adds a Start menu shortcut and an entry in Settings > Apps.
# `scripts/get.ps1` is the web installer that downloads a release and runs this.
param(
    [string]$Prefix = $(if ($env:AXON_PREFIX) { $env:AXON_PREFIX } else { Join-Path $env:LOCALAPPDATA 'Programs\axon' }),
    [switch]$Uninstall
)
$ErrorActionPreference = 'Stop'

$UninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\axon'
$Shortcut = Join-Path ([Environment]::GetFolderPath('Programs')) 'axon.lnk'

function Say($msg) { Write-Host "axon: $msg" }

# The user's PATH entries (the registry value, not the merged one a session sees), without the prefix.
function Get-UserPath {
    $p = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (-not $p) { return @() }
    return @($p -split ';' | Where-Object { $_ })
}
function Set-UserPath($entries) {
    [Environment]::SetEnvironmentVariable('Path', ($entries -join ';'), 'User')
}
function Same-Dir($a, $b) { return $a.TrimEnd('\') -ieq $b.TrimEnd('\') }

if ($Uninstall) {
    if (-not (Test-Path (Join-Path $Prefix 'axon.exe'))) { Say "nothing to remove: there is no axon.exe in $Prefix" }
    Set-UserPath @(Get-UserPath | Where-Object { -not (Same-Dir $_ $Prefix) })
    Remove-Item -Force -ErrorAction SilentlyContinue $Shortcut
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $UninstallKey
    # This script may be the one running from the prefix: the files go once it is over.
    foreach ($f in 'axon.exe', 'axon.ico', 'config.example') { Remove-Item -Force -ErrorAction SilentlyContinue (Join-Path $Prefix $f) }
    if ($PSCommandPath -and (Split-Path $PSCommandPath) -ieq $Prefix.TrimEnd('\')) {
        Start-Process -WindowStyle Hidden -FilePath ([Environment]::GetEnvironmentVariable('ComSpec')) -ArgumentList '/c', "ping -n 3 127.0.0.1 >nul & del /q `"$PSCommandPath`" & rmdir `"$Prefix`""
    } else {
        Remove-Item -Force -ErrorAction SilentlyContinue (Join-Path $Prefix 'install.ps1')
        if ((Test-Path $Prefix) -and -not (Get-ChildItem -Force $Prefix)) { Remove-Item -Force $Prefix }
    }
    Say "removed from $Prefix (your settings in $env:APPDATA\axon were not touched)"
    return
}

$src = $PSScriptRoot
if (-not (Test-Path (Join-Path $src 'axon.exe'))) { throw "axon: there is no axon.exe next to install.ps1 ($src)" }
if (-not [IO.Path]::IsPathRooted($Prefix)) { throw "axon: -Prefix must be an absolute path" }

New-Item -ItemType Directory -Force $Prefix | Out-Null
# A running axon holds its exe: the new one goes beside it and takes its place when it can.
$exe = Join-Path $Prefix 'axon.exe'
try {
    Copy-Item -Force (Join-Path $src 'axon.exe') $exe
} catch {
    throw "axon: could not write $exe (is axon running? close its windows and run this again): $($_.Exception.Message)"
}
foreach ($f in 'axon.ico', 'config.example', 'install.ps1') {
    $from = Join-Path $src $f
    if ((Test-Path $from) -and -not (Same-Dir (Split-Path $from) $Prefix)) { Copy-Item -Force $from (Join-Path $Prefix $f) }
}

$path = Get-UserPath
if (-not ($path | Where-Object { Same-Dir $_ $Prefix })) { Set-UserPath (@($path) + $Prefix) }

$shell = New-Object -ComObject WScript.Shell
$lnk = $shell.CreateShortcut($Shortcut)
$lnk.TargetPath = $exe
$lnk.WorkingDirectory = $env:USERPROFILE
$lnk.IconLocation = (Join-Path $Prefix 'axon.ico')
$lnk.Description = 'axon, a minimalist terminal'
$lnk.Save()

# The line `Settings > Apps` shows, with its Uninstall button.
$version = ((& $exe --version | Out-String).Trim() -split '\s+' | Select-Object -Last 1)
New-Item -Force $UninstallKey | Out-Null
$entry = @{
    DisplayName     = 'axon'
    DisplayVersion  = $version
    Publisher       = 'axon'
    DisplayIcon     = (Join-Path $Prefix 'axon.ico')
    InstallLocation = $Prefix
    UninstallString = "powershell.exe -NoProfile -ExecutionPolicy Bypass -File `"$(Join-Path $Prefix 'install.ps1')`" -Prefix `"$Prefix`" -Uninstall"
}
foreach ($k in $entry.Keys) { Set-ItemProperty -Path $UninstallKey -Name $k -Value $entry[$k] }
Set-ItemProperty -Path $UninstallKey -Name NoModify -Value 1 -Type DWord
Set-ItemProperty -Path $UninstallKey -Name NoRepair -Value 1 -Type DWord

Say "installed $version in $Prefix"
Say "open it from the Start menu, or run ``axon`` in a new terminal"
