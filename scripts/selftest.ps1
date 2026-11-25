# Runs axon's own self-test on Windows (AXON_SELFTEST, src/wlapp/selftest.rs): the real window and a real shell, a
# command typed and its output read back, a directory change, the clipboard, maximize and restore. Prints the report,
# leaves selftest.txt, frame.argb and frame.png (a picture of the window) in -Out, and exits 1 if it did not pass.
#
#   scripts\selftest.ps1 [-Exe target\release\axon.exe] [-Out target\selftest]
param(
    [string]$Exe = 'target\release\axon.exe',
    [string]$Out = 'target\selftest'
)
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot)

Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $Out
New-Item -ItemType Directory -Force $Out | Out-Null
$Out = (Resolve-Path $Out).Path
$report = Join-Path $Out 'selftest.txt'

$env:AXON_SELFTEST = $Out
$p = Start-Process -FilePath $Exe -PassThru
if (-not $p.WaitForExit(120000)) {
    $p.Kill()
    Add-Content $report 'axon did not exit in 120 s'
    Write-Host 'axon did not exit in 120 s'
}
$code = if ($p.HasExited) { $p.ExitCode } else { 1 }
Write-Host "exit code: $code"
if (Test-Path $report) { Get-Content $report } else { Write-Host 'no report was written' }
$frame = Join-Path $Out 'frame.argb'
if (Test-Path $frame) { python (Join-Path $PSScriptRoot 'argb2png.py') $frame (Join-Path $Out 'frame.png') }
if ($code -ne 0) { exit 1 }
