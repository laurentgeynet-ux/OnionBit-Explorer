# This file is part of Explorer.
# Copyright (C) 2026 Laurent Geynet
# SPDX-License-Identifier: MIT OR Apache-2.0

# package.ps1 — bundle release 0.1.0 (etape 40) :
#   dist/explorer-0.1.0/bin/explorer-daemon.exe, explorer-cli.exe
#   dist/explorer-0.1.0/app/        runner Flutter Windows (si flutter dispo)
#   dist/explorer-0.1.0.zip
# Usage : pwsh -File scripts/package.ps1 [-SkipApp] [-NoZip]

param([switch]$SkipApp, [switch]$NoZip)

$ErrorActionPreference = 'Stop'
Set-Location (Split-Path -Parent $PSScriptRoot)

$Version = '0.1.0'
$Dist    = "dist/explorer-$Version"

Write-Host "==> cargo build --release (daemon + cli)" -ForegroundColor Cyan
cargo build --release -p explorer-daemon -p explorer-cli
if ($LASTEXITCODE -ne 0) { throw "ECHEC : build release" }

New-Item -ItemType Directory -Force -Path "$Dist/bin" | Out-Null
Copy-Item "target/release/explorer-daemon.exe" "$Dist/bin/"
Copy-Item "target/release/explorer-cli.exe"    "$Dist/bin/"

if (-not $SkipApp -and (Test-Path 'app/pubspec.yaml')) {
    $flutter = Get-Command flutter -ErrorAction SilentlyContinue
    if ($flutter) {
        Write-Host "==> flutter build windows" -ForegroundColor Cyan
        Push-Location app
        try {
            flutter build windows --release
            if ($LASTEXITCODE -ne 0) { throw "ECHEC : flutter build" }
        } finally { Pop-Location }
        $runner = 'app/build/windows/x64/runner/Release'
        if (Test-Path $runner) {
            Copy-Item $runner "$Dist/app" -Recurse
        }
    } else {
        Write-Host "==> flutter absent — app non embarquee (bin/ seul)" -ForegroundColor Yellow
    }
}

Copy-Item README.md, CHANGELOG.md, ROADMAP.md "$Dist/"
Get-ChildItem LICENSE* -ErrorAction SilentlyContinue |
    Copy-Item -Destination $Dist

if (-not $NoZip) {
    Compress-Archive -Path $Dist -DestinationPath "dist/explorer-$Version.zip" -Force
    Write-Host "==> dist/explorer-$Version.zip" -ForegroundColor Green
}
Write-Host "==> bundle pret : $Dist" -ForegroundColor Green
