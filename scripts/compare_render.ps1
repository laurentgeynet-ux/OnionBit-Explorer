# This file is part of Explorer.
# Copyright (C) 2026 Laurent Geynet
# SPDX-License-Identifier: MIT OR Apache-2.0

# compare_render.ps1 — rendu moteur vs référence (Chrome via
# Playwright, ou tout PNG de même taille) : shot, puis diff
# pixel par région. Usage :
#   pwsh -File scripts/compare_render.ps1 https://example.com
#       [-Ref ref.png] [-Viewport 1366x900] [-OutDir out]
# Sans -Ref, produit seulement le shot moteur.

param(
    [Parameter(Mandatory = $true)][string]$Url,
    [string]$Ref = '',
    [string]$Viewport = '1366x900',
    [string]$OutDir = 'target\compare'
)

$ErrorActionPreference = 'Stop'
Set-Location (Split-Path -Parent $PSScriptRoot)

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$shot = Join-Path $OutDir 'engine.png'

Write-Host "==> shot moteur $Url ($Viewport)" -ForegroundColor Cyan
cargo run --release -q -p explorer-cli -- shot $Url $shot $Viewport
if ($LASTEXITCODE -ne 0) { throw 'shot moteur en échec' }

if (-not $Ref) {
    Write-Host "pas de référence — shot dans $shot" -ForegroundColor Yellow
    exit 0
}
if (-not (Test-Path $Ref)) { throw "référence absente : $Ref" }

Write-Host "==> diff vs $Ref" -ForegroundColor Cyan
cargo run --release -q -p explorer-cli -- diff $shot $Ref
# exit 2 = diffère (normal — c'est le constat), 0 = identique
if ($LASTEXITCODE -eq 1) { throw 'diff en échec' }
