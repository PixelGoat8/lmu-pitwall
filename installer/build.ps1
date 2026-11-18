# Builds the Windows installer: dashboard, bridge (release), then Inno Setup.
#
#   powershell -ExecutionPolicy Bypass -File installer\build.ps1
#
# Output: installer\Output\LMU-Pitwall-Setup-<version>.exe
# Needs: Node, Rust (MSVC), Inno Setup 6 (winget install JRSoftware.InnoSetup).

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot

# VERSION is the single source of truth (scripts/bump-version.py keeps the
# bridge, the dashboard and the installer script in step with it). Refuse to
# build if they disagree: the dashboard shows VERSION, the bridge reports
# Cargo.toml's, and a mismatch went out once.
$version = (Get-Content "$root\VERSION" -Raw).Trim()
$cargo = (Select-String -Path "$root\bridge\Cargo.toml" -Pattern '^version\s*=\s*"([^"]+)"' |
    Select-Object -First 1).Matches[0].Groups[1].Value
if ($cargo -ne $version) { throw "VERSION is $version but bridge\Cargo.toml says ${cargo}: run scripts/bump-version.py" }
Write-Host "LMU Pitwall $version"

$iscc = @(
    "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "$env:ProgramFiles\Inno Setup 6\ISCC.exe"
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) { throw 'Inno Setup 6 not found: winget install JRSoftware.InnoSetup' }

Write-Host '== dashboard'
Push-Location "$root\dashboard"
try {
    if (-not (Test-Path node_modules)) { npm ci; if ($LASTEXITCODE) { throw 'npm ci failed' } }
    npm run build; if ($LASTEXITCODE) { throw 'dashboard build failed' }
} finally { Pop-Location }

Write-Host '== bridge (embeds the dashboard)'
Push-Location "$root\bridge"
try {
    cargo build --release; if ($LASTEXITCODE) { throw 'bridge build failed' }
} finally { Pop-Location }

Write-Host '== installer'
& $iscc "/DMyAppVersion=$version" "$PSScriptRoot\lmu-pitwall-installer.iss"
if ($LASTEXITCODE) { throw 'Inno Setup failed' }

Get-Item "$PSScriptRoot\Output\LMU-Pitwall-Setup-$version.exe" | Select-Object FullName, Length
