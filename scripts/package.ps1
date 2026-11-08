# This file is part of Explorer.
# Copyright (C) 2026 Laurent Geynet
# SPDX-License-Identifier: MIT OR Apache-2.0

# package.ps1 — bundle release 0.2.0 (etape 59) :
#   dist/explorer-0.2.0/bin/explorer-daemon.exe, explorer-cli.exe
#   dist/explorer-0.2.0/app/        runner Flutter Windows (si flutter dispo)
#   dist/explorer-0.2.0.zip
# Usage : pwsh -File scripts/package.ps1 [-SkipApp] [-NoZip]

param([switch]$SkipApp, [switch]$NoZip)

$ErrorActionPreference = 'Stop'
Set-Location (Split-Path -Parent $PSScriptRoot)

$Version = '0.2.0'
$Dist    = "dist/explorer-$Version"

Write-Host "==> cargo build --release (daemon + cli + ffi)" -ForegroundColor Cyan
cargo build --release -p explorer-daemon -p explorer-cli -p explorer-ffi
if ($LASTEXITCODE -ne 0) { throw "ECHEC : build release" }

New-Item -ItemType Directory -Force -Path "$Dist/bin" | Out-Null
# `target_directory` reel via `cargo metadata` (CARGO_TARGET_DIR /
# configs le deportent — jamais `target/release` en dur) + repli
# <target>/<triple>/release quand `build.target` est explicite
# (config globale) — meme regle que spawn_daemon.ps1.
$meta = cargo metadata --no-deps --format-version 1 | ConvertFrom-Json
$triple = ''
$v = & rustc -vV 2>$null | Select-String -Pattern '^host: (\S+)'
if ($v) { $triple = $v.Matches[0].Groups[1].Value }
$relTarget = @(
    $(if ($triple) { Join-Path $meta.target_directory "$triple\release" }),
    (Join-Path $meta.target_directory 'release')
) | Where-Object { $_ -and (Test-Path (Join-Path $_ 'explorer-daemon.exe')) } |
    Select-Object -First 1
if (-not $relTarget) {
    throw "explorer-daemon.exe introuvable sous $($meta.target_directory)"
}
Copy-Item (Join-Path $relTarget 'explorer-daemon.exe') "$Dist/bin/"
Copy-Item (Join-Path $relTarget 'explorer-cli.exe')    "$Dist/bin/"

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
            # Etape 119 — DLL fraîche À CÔTÉ de l'exe : en mode
            # `auto` une `explorer_ffi.dll` périmée adjacente
            # gagnait silencieusement la sonde `tryLoad`
            # (terrain : latence 20 s + gel). Le binaire
            # compilé ici est déposé dans le bundle.
            $ffiDll = Join-Path $relTarget 'explorer_ffi.dll'
            if (Test-Path $ffiDll) {
                Copy-Item $ffiDll "$Dist/app/"
            }
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
