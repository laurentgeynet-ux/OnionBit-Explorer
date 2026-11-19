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
  est la mesure. Sortie JSON sous `Explorer-data/vols/geo-smoke/`.

  Usage : pwsh -File scripts/geo_smoke.ps1 [sites.txt] [-Max N]
#>

param(
    [string]$ListFile = '',
    [int]$Max = 0,
    [ValidateSet('debug', 'release')][Alias('Profile')][string]$BuildProfile = 'release'
)

$ErrorActionPreference = 'Continue'
$root = Split-Path -Parent (Split-Path -Parent $PSCommandPath)
$outDir = Join-Path $root 'Explorer-data\vols\geo-smoke'
New-Item -ItemType Directory -Force -Path $outDir | Out-Null

if ($ListFile -eq '') {
    # Corpus court — géométrie mesurable en CI (temps borné).
    # Une famille de layout par site autant que possible —
    # filet de régression : la stabilité prime (pas de login,
    # pas de mur, pas de redirect géographique — craigslist
    # écarté pour geo.craigslist.org).
    $sites = @(
        'https://example.com/',          # bloc minimal — baseline
        'https://info.cern.ch/',         # pré-doctype / quirks
        'https://lobste.rs/',            # CSS réel + JS mutateur
        'https://news.ycombinator.com/', # tables imbriquées
        'https://text.npr.org/',         # contenu texte simple
        # Extensions étape 295 — familles absentes du filet
        # initial : flottants, flex/responsive, formulaires,
        # positionnement/design, contenu illustré standard.
        'https://slashdot.org/',         # flottants + contenu réel
        'https://www.w3.org/',           # flex/grid + media queries
        'https://validator.w3.org/',     # formulaires
        'https://www.csszengarden.com/', # positionnement/design
        'https://www.debian.org/'        # images + nav standard
    )
} else {
    $sites = Get-Content $ListFile |
        Where-Object { $_ -match '\S' -and $_ -notmatch '^\s*#' } |
        ForEach-Object { $_.Trim() }
}
if ($Max -gt 0) { $sites = $sites | Select-Object -First $Max }

# Release par défaut (convention site_harvest) : le debug
# est 10-20× plus lent, les shots mesurent les conditions
# réelles du moteur.
Write-Output "Construction d'explorer-cli ($BuildProfile)…"
$relArgs = @()
if ($BuildProfile -eq 'release') { $relArgs = @('--release') }
cargo build -p explorer-cli @relArgs 2>&1 | Out-Null
if ($LASTEXITCODE -ne 0) { Write-Error 'cargo build -p explorer-cli a échoué'; exit 1 }

$meta = cargo metadata --no-deps --format-version 1 | ConvertFrom-Json
$triple = ''
$v = & rustc -vV 2>$null | Select-String -Pattern '^host: (\S+)'
if ($v) { $triple = $v.Matches[0].Groups[1].Value }
$cli = @(
    $(if ($triple) { Join-Path $meta.target_directory "$triple\$BuildProfile\explorer-cli.exe" }),
    (Join-Path $meta.target_directory "$BuildProfile\explorer-cli.exe"),
    $(if ($triple) { Join-Path $meta.target_directory "$triple\debug\explorer-cli.exe" }),
    (Join-Path $meta.target_directory 'debug\explorer-cli.exe')
) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
if (-not $cli) { Write-Error "explorer-cli introuvable sous $($meta.target_directory)"; exit 1 }

$env:EXPLORER_VIEWPORT = '1366x900'
$refDir = Join-Path $outDir 'refs'
New-Item -ItemType Directory -Force -Path $refDir | Out-Null

$totalDiv = 0
$totalMatch = 0
$totalMoved = 0
$partiels = 0
$measured = 0
$horsMesure = 0
$echoues = 0
foreach ($site in $sites) {
    $name = (($site -replace '^https?://', '') -replace '/+$', '') `
        -replace '[^A-Za-z0-9_.-]', '_'
    $png = Join-Path $outDir "$name.png"
    $geom = Join-Path $outDir "$name.geom.json"
    $sty = Join-Path $outDir "$name.styles.json"
    $ref = Join-Path $refDir "$name.ref.json"
    Remove-Item $png,$geom,$sty,$ref -Force -ErrorAction SilentlyContinue

    # 1. Boîtes posées par le moteur + sidecar d'attribution
    #    (styles calculés — alimente `class`/`ancestors`).
    $null = & $cli shot $site $png --geometry $geom --styles $sty 2>&1
    if (-not (Test-Path $geom)) {
        Write-Output "  FAIL $site (geometry moteur)"
        $echoues++
        continue
    }

    # 2. Boîtes de référence Chrome (CDP natif, zéro npm).
    $null = & node (Join-Path $root 'scripts\ref_capture.mjs') $site --out $refDir --timeout 45000 2>&1
    if (-not (Test-Path $ref)) {
        Write-Output "  FAIL $site (référence Chrome)"
        $echoues++
        continue
    }

    # 3. Diff élément par élément — enrichi des styles quand
    #    le sidecar a été produit (attribution `class`/
    #    `suspect`/`ancestors` dans le geodiff).
    $diff = Join-Path $outDir "$name.geodiff.json"
    $diffArgs = @($geom, $ref, '--out', $diff, '--top', '5')
    if (Test-Path $sty) { $diffArgs += @('--styles', $sty) }
    $out = & node (Join-Path $root 'scripts\geo_diff.mjs') @diffArgs 2>&1
    $line = ($out | Select-String -Pattern 'divergents|matched|hors-mesure' | Select-Object -First 1)
    Write-Output ("  DIFF {0}  {1}" -f $site, $(if ($line) { $line.Line.Trim() } else { 'n/a' }))

    # geo_diff écrit --out même en hors-mesure (score_pct:null) —
    # un rapport absent = crash du diff, un verdict hors-mesure =
    # paire non mesurable : les deux ne doivent PAS entrer dans
    # l'agrégat (le % récapitulatif ne porterait plus sur des
    # mesures réelles).
    $j = $null
    if (Test-Path $diff) {
        try { $j = Get-Content $diff -Raw | ConvertFrom-Json } catch { }
    }
    if ($null -eq $j -or $null -eq $j.matched) {
        Write-Output "  FAIL $site (geo_diff sans rapport lisible)"
        $echoues++
        continue
    }
    if ($j.verdict -eq 'hors-mesure') {
        $horsMesure++
        continue
    }
    $totalDiv += [int]$j.divergent_n
    $totalMatch += [int]$j.matched
    $totalMoved += [int]$j.moved_n
    if ($j.score_partial) { $partiels++ }
    $measured++
}

Write-Output ''
$stats = ''
if ($totalMoved -gt 0) { $stats += " — $totalMoved déplacés" }
if ($horsMesure -gt 0) { $stats += " — $horsMesure hors-mesure" }
if ($partiels -gt 0) { $stats += " — $partiels score(s) partiel(s) (ref tronquée)" }
if ($echoues -gt 0) { $stats += " — $echoues échec(s)" }
if ($measured -gt 0) {
    $pct = [math]::Round(100.0 * $totalDiv / [math]::Max(1, $totalMatch), 1)
    Write-Output "=== geo_diff : $totalDiv divergents / $totalMatch appariés ($pct %) sur $measured sites$stats ==="
} else {
    Write-Output "=== geo_diff : aucune paire mesurable$stats ==="
}
