# This file is part of Explorer.
# Copyright (C) 2026 Laurent Geynet
# SPDX-License-Identifier: MIT OR Apache-2.0

<#
.SYNOPSIS
  Prospecte le MANIFEST.json officiel du WPT (ADR-0010).

.DESCRIPTION
  Le `MANIFEST.json` des web-platform-tests est l'index canonique
  de la suite : chaque test y est listé avec son type
  (`reftest`, `testharness`, `crashtest`…), ses références et
  ses flags — l'équivalent du frontmatter test262, en table
  unique. Il n'est pas versionné dans le dépôt WPT : les
  releases `merge_pr_*` publient l'artefact pré-généré
  (`MANIFEST-<sha>.json.gz`), téléchargé et mis en cache sous
  `Explorer-data/wpt/` (gitignoré, réduit via `EXPLORER_DATA`).

  Le filtrage tourne dans `explorer-cli wpt-scan` (le JSON
  décompressé fait ~80 Mo — hors de portée raisonnable de
  ConvertFrom-Json). La sortie est comparée au manifeste curé
  `crates/explorer-render/tests/data/wpt/manifest.txt` : seuls
  les candidats nouveaux sont listés ; `-Add` les ajoute à la
  fin du manifeste sous un commentaire de salve.

.PARAMETER Prefix
  Préfixes de dossiers WPT à prospecter (obligatoire), ex.
  `css/css-grid`, `css/CSS2/floats`. Répétable :
  `-Prefix a, b` ou plusieurs fois.

.PARAMETER Type
  Type d'entrées du manifest (défaut `reftest` — seul type
  exécutable par le harnais statique aujourd'hui).

.PARAMETER Update
  Retélécharge le dernier MANIFEST.json publié même si un
  cache existe.

.PARAMETER Add
  Ajoute les candidats nouveaux à manifest.txt sous un
  commentaire de salve daté — la curation reste manuelle,
  l'entrée est à relire avant commit.

.EXAMPLE
  pwsh -File scripts/scan_wpt.ps1 -Update -Prefix css/css-grid
  pwsh -File scripts/scan_wpt.ps1 -Prefix css/css-flexbox, css/css-grid -Add
#>

[CmdletBinding()]
param(
    [Parameter(Mandatory)][string[]]$Prefix,
    [string]$Type = 'reftest',
    [switch]$Update,
    [switch]$Add
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSCommandPath)
$dataRoot = if ($env:EXPLORER_DATA) { $env:EXPLORER_DATA } else { Join-Path $root 'Explorer-data' }
$data = Join-Path $dataRoot 'wpt'
$manifest = Join-Path $data 'MANIFEST.json'
$manifestTxt = Join-Path $root 'crates\explorer-render\tests\data\wpt\manifest.txt'

# --- 1. MANIFEST.json : cache ou téléchargement ---------------------------

if (!(Test-Path $manifest) -or $Update) {
    New-Item -ItemType Directory -Force -Path $data | Out-Null
    Write-Output 'MANIFEST.json : recherche de la dernière release WPT…'
    $rel = Invoke-RestMethod -Uri 'https://api.github.com/repos/web-platform-tests/wpt/releases/latest'
    $asset = $rel.assets | Where-Object { $_.name -match '^MANIFEST-.*\.json\.gz$' } | Select-Object -First 1
    if ($null -eq $asset) { throw "Aucun asset MANIFEST-*.json.gz dans la release $($rel.tag_name)" }
    $gz = "$manifest.gz"
    Write-Output "  $($rel.tag_name) : $($asset.name) ($([math]::Round($asset.size/1MB,1)) Mo)…"
    Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $gz -UseBasicParsing
    $inp = [System.IO.File]::OpenRead($gz)
    try {
        $gun = [System.IO.Compression.GzipStream]::new($inp, [System.IO.Compression.CompressionMode]::Decompress)
        try {
            $outp = [System.IO.File]::Create($manifest)
            try { $gun.CopyTo($outp) } finally { $outp.Close() }
        } finally { $gun.Close() }
    } finally { $inp.Close() }
    Remove-Item $gz
    Set-Content -Path "$manifest.source.txt" -Value "$($rel.tag_name)`n$($asset.browser_download_url)"
    Write-Output "  → $manifest ($([math]::Round((Get-Item $manifest).Length/1MB)) Mo)"
} else {
    Write-Output "MANIFEST.json : cache $manifest"
}

# --- 2. Scan via explorer-cli ---------------------------------------------

# Le binaire peut vivre sous <target>/<triple>/debug (env
# CARGO_BUILD_TARGET) — chemins candidats explicites, pas de
# scan récursif (target\ fait des millions de fichiers).
function Find-Cli {
    $cands = @()
    if ($env:CARGO_TARGET_DIR) {
        $cands += Join-Path $env:CARGO_TARGET_DIR 'debug\explorer-cli.exe'
        if ($env:CARGO_BUILD_TARGET) {
            $cands += Join-Path $env:CARGO_TARGET_DIR "$env:CARGO_BUILD_TARGET\debug\explorer-cli.exe"
        }
    }
    if ($env:CARGO_BUILD_TARGET) {
        $cands += Join-Path $root "target\$env:CARGO_BUILD_TARGET\debug\explorer-cli.exe"
    }
    # Triple hôte : certains builds passent --target
    # explicitement → <target>/<triple>/debug.
    $hostTriple = (rustc -vV | Select-String 'host: (\S+)').Matches.Groups[1].Value
    if ($hostTriple) {
        $cands += Join-Path $root "target\$hostTriple\debug\explorer-cli.exe"
    }
    $cands += Join-Path $root 'target\debug\explorer-cli.exe'
    $cands | Where-Object { Test-Path $_ } | Select-Object -First 1
}
$cli = Find-Cli
if (!$cli) {
    Write-Output 'explorer-cli absent — build debug…'
    cargo build -q -p explorer-cli --manifest-path (Join-Path $root 'Cargo.toml')
    if ($LASTEXITCODE -ne 0) { throw 'cargo build -p explorer-cli a échoué' }
    $cli = Find-Cli
    if (!$cli) { throw 'explorer-cli.exe introuvable après build' }
}

$scanArgs = @('wpt-scan', $manifest, '--type', $Type) + ($Prefix | ForEach-Object { $_.TrimEnd('/') })
$lines = & $cli @scanArgs
if ($LASTEXITCODE -ne 0) { throw "explorer-cli wpt-scan a échoué ($LASTEXITCODE)" }

# --- 3. Diff contre le manifeste curé -------------------------------------

$listed = @{}
Get-Content $manifestTxt | ForEach-Object {
    $l = $_.Trim()
    if ($l -and !$l.StartsWith('#')) { $listed[$l] = $true }
}

$new = @(); $known = 0
foreach ($line in $lines) {
    $path = ($line -split "`t")[0]
    if ($listed.ContainsKey($path)) { $known++ } else { $new += $line }
}

Write-Output ''
Write-Output "=== wpt-scan : $($lines.Count) $Type sous « $($Prefix -join ', ') » ==="
Write-Output "déjà dans manifest.txt : $known — candidats nouveaux : $($new.Count)"
foreach ($line in $new) {
    $f = $line -split "`t"
    $refs = if ($f[1]) { "  $($f[1])" } else { '' }
    $extras = if ($f[2]) { "  [$($f[2])]" } else { '' }
    Write-Output "  + $($f[0])$refs$extras"
}

if ($Add -and $new.Count -gt 0) {
    $stamp = Get-Date -Format 'yyyy-MM-dd'
    $block = @("# Salve scan_wpt $stamp — $($Prefix -join ', ') (prospection MANIFEST.json — à relire)")
    $block += $new | ForEach-Object { ($_ -split "`t")[0] }
    Add-Content -Path $manifestTxt -Value $block
    Write-Output "$($new.Count) entrées ajoutées à manifest.txt — lancer fetch_wpt.ps1 pour extraire."
}
