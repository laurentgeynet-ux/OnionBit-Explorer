# This file is part of Explorer. Copyright (C) 2026 Laurent Geynet
# SPDX-License-Identifier: MIT OR Apache-2.0
<#
.SYNOPSIS
  Shot de contrôle « vitrine » sur un rejeu DirTransport versionné.

.DESCRIPTION
  N4/N0.3 : rejoue un vol enregistré (défaut : la fixture
  `fuzz/replays/youtube-vol813` distillée du vol 813 — voir
  `fuzz/replays/README.md`), produit PNG + geom.json, puis
  diff géométrique `geo_diff.mjs` contre la référence Chrome
  archivée dans la fixture. **Aucun réseau, aucun Chrome** :
  l'écart mesuré est purement moteur.

  Sortie sous `Explorer-data/vols/vitrine/` :
    <site>.png          frame moteur (rejeu)
    <site>.geom.json    boîtes posées moteur
    <site>.geodiff.json diff vs référence Chrome
    <site>.ref.png      référence visuelle (copiée)

  Usage :
    pwsh -File scripts/vitrine_shot.ps1
    pwsh -File scripts/vitrine_shot.ps1 -Fixture fuzz\replays\youtube-vol813
#>

param(
    # Dossier fixture DirTransport (relatif à la racine du repo
    # ou absolu).
    [string]$Fixture = 'fuzz\replays\youtube-vol813',
    # URL rejouée — doit correspondre aux clés enregistrées.
    [string]$Site = 'https://www.youtube.com/',
    # Référence géométrique Chrome ; par défaut celle embarquée
    # dans la fixture (rejouage 100 % hors-ligne).
    [string]$Ref = '',
    [ValidateSet('debug', 'release')][string]$Profile = 'release'
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSCommandPath)
$fixtureDir = if ([IO.Path]::IsPathRooted($Fixture)) { $Fixture } else { Join-Path $root $Fixture }
if (-not (Test-Path $fixtureDir)) {
    Write-Error "fixture absente : $fixtureDir (cf. fuzz/replays/README.md)"
}
if ($Ref -eq '') { $Ref = Join-Path $fixtureDir 'www.youtube.com.ref.json' }
if (-not (Test-Path $Ref)) { Write-Error "référence Chrome absente : $Ref" }

$outDir = Join-Path $root 'Explorer-data\vols\vitrine'
New-Item -ItemType Directory -Force -Path $outDir | Out-Null

# Binaire cli — même recherche que geo_smoke.ps1.
$meta = cargo metadata --no-deps --format-version 1 | ConvertFrom-Json
$triple = ''
$v = & rustc -vV 2>$null | Select-String -Pattern '^host: (\S+)'
if ($v) { $triple = $v.Matches[0].Groups[1].Value }
$cli = @(
    $(if ($triple) { Join-Path $meta.target_directory "$triple\$Profile\explorer-cli.exe" }),
    (Join-Path $meta.target_directory "$Profile\explorer-cli.exe"),
    $(if ($triple) { Join-Path $meta.target_directory "$triple\debug\explorer-cli.exe" }),
    (Join-Path $meta.target_directory 'debug\explorer-cli.exe')
) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
if (-not $cli) { Write-Error "explorer-cli introuvable — cargo build -p explorer-cli$(if ($Profile -eq 'release') { ' --release' })" }

$name = (($Site -replace '^https?://', '') -replace '/+$', '') `
    -replace '[^A-Za-z0-9_.-]', '_'
$png = Join-Path $outDir "$name.png"
$geom = Join-Path $outDir "$name.geom.json"
Remove-Item $png, $geom -Force -ErrorAction SilentlyContinue

# 1. Shot moteur sur le rejeu — aucun socket ouvert.
$env:EXPLORER_NET_REPLAY = $fixtureDir
try {
    $null = & $cli shot $Site $png --geometry $geom 2>&1
} finally {
    Remove-Item Env:EXPLORER_NET_REPLAY -ErrorAction SilentlyContinue
}
if (-not (Test-Path $geom)) { Write-Error "geometry moteur non produite (shot $Site)" }
$boxes = (Get-Content $geom -Raw | ConvertFrom-Json).boxes.Count
Write-Output "SHOT $Site  $boxes boîtes (rejeu $Fixture)"

# 2. Diff vs référence Chrome archivée.
$diff = Join-Path $outDir "$name.geodiff.json"
$out = & node (Join-Path $root 'scripts\geo_diff.mjs') $geom $Ref --out $diff --top 5 2>&1
$line = ($out | Select-String -Pattern 'divergents|matched|hors-mesure' | Select-Object -First 1)
Write-Output ("DIFF {0}" -f $(if ($line) { $line.Line.Trim() } else { 'n/a' }))

# Preuve visuelle à côté du verdict.
$refPng = [IO.Path]::ChangeExtension($Ref, '.png')
if (Test-Path $refPng) {
    Copy-Item $refPng (Join-Path $outDir "$name.ref.png") -Force
}
Write-Output "artefacts → $outDir"
