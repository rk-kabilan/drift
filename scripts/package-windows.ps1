# Packages Drift for Windows. Everything lands in target\release\windows\:
#
#   Drift-<version>-windows-<arch>.zip          drift.exe on its own (portable)
#   Drift-<version>-setup-<arch>.exe            installer, when Inno Setup 6 is installed
#
#   powershell -ExecutionPolicy Bypass -File scripts\package-windows.ps1
#   powershell -ExecutionPolicy Bypass -File scripts\package-windows.ps1 -NoBuild
#
# Needs Rust with the MSVC build tools. Get Inno Setup from https://jrsoftware.org/isinfo.php
# (or: winget install JRSoftware.InnoSetup). Unsigned installers make SmartScreen
# warn on other PCs; sign drift.exe and the installer with signtool to avoid that.
param([switch]$NoBuild)
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.*)"' |
    Select-Object -First 1).Matches[0].Groups[1].Value
$arch = if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') { 'arm64' } else { 'x64' }
$exe = 'target\release\drift.exe'
$out = 'target\release\windows'

if (-not $NoBuild) {
    cargo build --release
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}
if (-not (Test-Path $exe)) { throw "$exe is missing; build it first." }

if (Test-Path $out) { Remove-Item $out -Recurse -Force }
New-Item -ItemType Directory -Path $out | Out-Null

# ---- portable zip ----
$zip = Join-Path $out "Drift-$version-windows-$arch.zip"
Compress-Archive -Path $exe -DestinationPath $zip
Write-Host "Built $zip"

# ---- installer ----
$iscc = @(
    (Get-Command iscc.exe -ErrorAction SilentlyContinue).Source,
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "$env:ProgramFiles\Inno Setup 6\ISCC.exe",
    "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe"
) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1

if ($iscc) {
    & $iscc /Q "/DAppVersion=$version" "/DArch=$arch" "/O$((Resolve-Path $out).Path)" 'scripts\windows\drift.iss'
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    Write-Host "Built $out\Drift-$version-setup-$arch.exe"
} else {
    Write-Host 'Skipped installer: Inno Setup 6 not found (winget install JRSoftware.InnoSetup).'
}
