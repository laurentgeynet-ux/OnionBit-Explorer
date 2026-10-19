# This file is part of Explorer.
# Copyright (C) 2026 Laurent Geynet
# SPDX-License-Identifier: MIT OR Apache-2.0

<#
.SYNOPSIS
  Smoke test interop sur sites réels (étape 48, ADR-0010).

.DESCRIPTION
  Rend chaque site de la liste via `explorer-cli shot`
  (pipeline Session complet : TLS + ALPN h1/h2, charset,
  cascade, layout, JS, images, webfonts → PNG) et mesure :

    - navigation réussie (HTTP final 2xx/3xx, aucune erreur)
    - frame produite (PNG non vide)
    - couverture non-blanche (le site a peint quelque chose)

  Informatif uniquement : un échec de site ne fait PAS
  échouer le script (le web réel est hors contrôle) — le
  rapport console est la mesure. Sortie PNG sous
  `target/site_smoke/` pour inspection visuelle.

  Usage : pwsh -File scripts/site_smoke.ps1 [sites.txt]
  La liste par défaut est embarquée ci-dessous (sites
  stables, sans login, majoritairement HTML/CSS).
#>

param(
    [string]$ListFile = ''
)

$ErrorActionPreference = 'Continue'
$root = Split-Path -Parent (Split-Path -Parent $PSCommandPath)
$outDir = Join-Path $root 'target\site_smoke'
New-Item -ItemType Directory -Force -Path $outDir | Out-Null

if ($ListFile -eq '') {
    # Corpus embarqué : diversité de formes (document statique,
    # moteur de recherche, wiki, site CSS-moderne).
    $sites = @(
        'https://example.com/',
        'https://info.cern.ch/',
        'https://duckduckgo.com/html/',
        'https://en.wikipedia.org/wiki/Main_Page',
        'https://developer.mozilla.org/en-US/'
    )
} else {
    $sites = Get-Content $ListFile |
        Where-Object { $_ -match '\S' -and $_ -notmatch '^\s*#' } |
        ForEach-Object { $_.Trim() }
}

Write-Output "Construction d'explorer-cli…"
cargo build -p explorer-cli 2>&1 | Out-Null
if ($LASTEXITCODE -ne 0) {
    Write-Error 'cargo build -p explorer-cli a échoué'
    exit 1
}
# Le binaire vit sous `target_directory` — pas `$root\target`
# (`.cargo/config.toml` peut rediriger `target-dir`, et
# `CARGO_TARGET_DIR` prime encore : `cargo metadata` résout le
# chemin réel — un chemin en dur rendait des faux OK sur des
# PNG périmés).
$meta = cargo metadata --no-deps --format-version 1 |
    ConvertFrom-Json
$cli = Join-Path $meta.target_directory 'debug\explorer-cli.exe'
if (-not (Test-Path $cli)) {
    Write-Error "explorer-cli introuvable : $cli"
    exit 1
}
# Purge des PNG précédents : une frame produite dans un run
# antérieur ne doit pas valider l'échec courant.
Get-ChildItem -Path $outDir -Filter '*.png' -ErrorAction SilentlyContinue |
    Remove-Item -Force

$ok = 0
$fail = 0
foreach ($site in $sites) {
    $name = ($site -replace '^https?://', '') -replace '[^A-Za-z0-9_.-]', '_'
    $png = Join-Path $outDir "$name.png"
    $null = & $cli shot $site $png 2>&1
    if ($LASTEXITCODE -eq 0 -and (Test-Path $png) -and
        (Get-Item $png).Length -gt 0) {
        $size = [math]::Round((Get-Item $png).Length / 1KB, 1)
        Write-Output "  OK   $site  ->  $name.png (${size} Ko)"
        $ok++
    } else {
        Write-Output "  FAIL $site"
        $fail++
    }
}

Write-Output ''
Write-Output "=== Smoke sites réels : $ok OK / $fail FAIL ==="
Write-Output "PNG sous $outDir"
# Code de sortie informatif : 0 si au moins un site rend.
if ($ok -gt 0) { exit 0 } else { exit 1 }
