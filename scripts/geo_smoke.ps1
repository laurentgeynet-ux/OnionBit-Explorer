# This file is part of Explorer.
# Copyright (C) 2026 Laurent Geynet
# SPDX-License-Identifier: MIT OR Apache-2.0

<#
.SYNOPSIS
  Diff géométrique moteur vs Chrome de référence (étape 216).

.DESCRIPTION
  Pour chaque site de la liste :
    1. `explorer-cli shot --geometry` — boîtes posées moteur
    2. `ref_capture.mjs` — boîtes getBoundingClientRect Chrome
    3. `geo_diff.mjs` — divergents/appariés par élément

  Informatif uniquement (le web réel est hors contrôle) : un
  écart ne fait PAS échouer le script — le rapport console
  est la mesure. Sortie JSON sous `target/geo_smoke/`.

  Usage : pwsh -File scripts/geo_smoke.ps1 [sites.txt] [-Max N]
#>

param(
    [string]$ListFile = '',
    [int]$Max = 0
)

$ErrorActionPreference = 'Continue'
$root = Split-Path -Parent (Split-Path -Parent $PSCommandPath)
$outDir = Join-Path $root 'target\geo_smoke'
New-Item -ItemType Directory -Force -Path $outDir | Out-Null

if ($ListFile -eq '') {
    # Corpus court — géométrie mesurable en CI (temps borné).
    $sites = @(
        'https://example.com/',
        'https://info.cern.ch/',
        'https://lobste.rs/',
        'https://news.ycombinator.com/',
        'https://text.npr.org/'
    )
} else {
    $sites = Get-Content $ListFile |
        Where-Object { $_ -match '\S' -and $_ -notmatch '^\s*#' } |
        ForEach-Object { $_.Trim() }
}
if ($Max -gt 0) { $sites = $sites | Select-Object -First $Max }

Write-Output "Construction d'explorer-cli…"
cargo build -p explorer-cli 2>&1 | Out-Null
if ($LASTEXITCODE -ne 0) { Write-Error 'cargo build -p explorer-cli a échoué'; exit 1 }

$meta = cargo metadata --no-deps --format-version 1 | ConvertFrom-Json
$triple = ''
$v = & rustc -vV 2>$null | Select-String -Pattern '^host: (\S+)'
if ($v) { $triple = $v.Matches[0].Groups[1].Value }
$cli = @(
    $(if ($triple) { Join-Path $meta.target_directory "$triple\debug\explorer-cli.exe" }),
    (Join-Path $meta.target_directory 'debug\explorer-cli.exe')
) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
if (-not $cli) { Write-Error "explorer-cli introuvable sous $($meta.target_directory)"; exit 1 }

$env:EXPLORER_VIEWPORT = '1366x900'
$refDir = Join-Path $outDir 'refs'
New-Item -ItemType Directory -Force -Path $refDir | Out-Null

$totalDiv = 0
$totalMatch = 0
$measured = 0
foreach ($site in $sites) {
    $name = ($site -replace '^https?://', '') -replace '[^A-Za-z0-9_.-]', '_'
    $png = Join-Path $outDir "$name.png"
    $geom = Join-Path $outDir "$name.geom.json"
    $ref = Join-Path $refDir "$name.ref.json"
    Remove-Item $png,$geom,$ref -Force -ErrorAction SilentlyContinue

    # 1. Boîtes posées par le moteur.
    $null = & $cli shot $site $png --geometry $geom 2>&1
    if (-not (Test-Path $geom)) { Write-Output "  FAIL $site (geometry moteur)"; continue }

    # 2. Boîtes de référence Chrome (CDP natif, zéro npm).
    $null = & node (Join-Path $root 'scripts\ref_capture.mjs') $site --out $refDir --timeout 45000 2>&1
    if (-not (Test-Path $ref)) { Write-Output "  FAIL $site (référence Chrome)"; continue }

    # 3. Diff élément par élément.
    $diff = Join-Path $outDir "$name.geodiff.json"
    $out = & node (Join-Path $root 'scripts\geo_diff.mjs') $geom $ref --out $diff --top 5 2>&1
    $line = ($out | Select-String -Pattern 'divergents|matched' | Select-Object -First 1)
    Write-Output ("  DIFF {0}  {1}" -f $site, $(if ($line) { $line.Line.Trim() } else { 'n/a' }))

    if (Test-Path $diff) {
        $j = Get-Content $diff -Raw | ConvertFrom-Json
        if ($null -ne $j.matched) {
            $totalDiv += [int]$j.divergent_n
            $totalMatch += [int]$j.matched
            $measured++
        }
    }
}

Write-Output ''
if ($measured -gt 0) {
    $pct = [math]::Round(100.0 * $totalDiv / [math]::Max(1, $totalMatch), 1)
    Write-Output "=== geo_diff : $totalDiv divergents / $totalMatch appariés ($pct %) sur $measured sites ==="
} else {
    Write-Output '=== geo_diff : aucune paire mesurable ==='
}
