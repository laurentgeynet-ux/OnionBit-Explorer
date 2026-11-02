# This file is part of Explorer.
# Copyright (C) 2026 Laurent Geynet
# SPDX-License-Identifier: MIT OR Apache-2.0

# build_terrain_desktop.ps1 — livre une version « test terrain »
# complete pour le vol humain : moteur release (daemon + cli +
# DLL FFI in-process) + app Flutter release, copiees dans le
# dossier fixe du Bureau. Les rapports de terrain (captures,
# notes) se deposent dans <DestDir>\rapports\ — conserve
# d'une livraison a l'autre.
#
# Usage : pwsh -File scripts/build_terrain_desktop.ps1
#         [-DestDir 'C:\Users\Lou\Desktop\Onion explorer']
#         [-SkipApp] [-CargoTargetDir <dir>]

param(
    [string]$DestDir = "$env:USERPROFILE\Desktop\Onion explorer",
    [string]$CargoTargetDir = '',
    [switch]$SkipApp          # ne pas compiler l'app Flutter
)

$ErrorActionPreference = 'Stop'
Set-Location (Split-Path -Parent $PSScriptRoot)

$buildScript = Join-Path $PSScriptRoot 'build_terrain.ps1'
if (-not (Test-Path $buildScript)) {
    throw "introuvable : $buildScript"
}

# Menage cible : seul le dossier `app\` livre est supprime —
# une DLL de plugin perimee adjacente a l'exe gagnait
# silencieusement la sonde de chargement (leçon package.ps1,
# etape 119). `rapports\` et les autres fichiers de
# l'operateur ne sont jamais touches ; les binaires racine
# et MANIFEST.txt sont ecrases par le build.
$appDest = Join-Path $DestDir 'app'
if (Test-Path $appDest) {
    Remove-Item $appDest -Recurse -Force
}

# Restes d'une ancienne livraison a plat (runner Flutter a la
# racine, avant le rangement sous `app\`) : archives dans
# `_obsolete-runner\`, jamais supprimes — un exe perime a la
# racine se lance d'un double-clic et fausse le verdict.
$legacy = @(
    'explorer_app.exe', 'data', 'native_assets.json',
    'flutter_windows.dll', 'fullscreen_window_plugin.dll',
    'onionmedia_ffi.dll', 'webview_win_floating_plugin.dll',
    'WebView2Loader.dll'
)
$stale = $legacy | Where-Object { Test-Path (Join-Path $DestDir $_) }
if ($stale) {
    $obs = Join-Path $DestDir '_obsolete-runner'
    New-Item -ItemType Directory -Force -Path $obs | Out-Null
    foreach ($f in $stale) {
        Move-Item (Join-Path $DestDir $f) -Destination $obs -Force
    }
    Write-Host "==> restes racine archives vers _obsolete-runner\ : $($stale -join ', ')" -ForegroundColor Yellow
}

$buildArgs = @{ OutDir = $DestDir }
if ($CargoTargetDir) { $buildArgs['CargoTargetDir'] = $CargoTargetDir }
if ($SkipApp) { $buildArgs['SkipApp'] = $true }

Write-Host "==> livraison test terrain vers $DestDir" -ForegroundColor Cyan
& $buildScript @buildArgs

# Dossier rapports de terrain + aide-memoire de lancement.
$rapports = Join-Path $DestDir 'rapports'
New-Item -ItemType Directory -Force -Path $rapports | Out-Null

$lisez = @"
OnionBit Explorer — version test terrain
========================================

NAVIGUER (vol humain)
  app\explorer_app.exe      UI Flutter — moteur in-process :
                            explorer_ffi.dll est adjacente,
                            rien d'autre a lancer.

Mode REST alternatif (onglets partages avec l'API) :
  explorer-daemon.exe --port 9377
  puis regler le moteur REST dans les parametres de l'app.

RAPPORTS DE TERRAIN — deposer les preuves dans rapports\
  explorer-cli.exe shot <url> rapports\<site>.png 1366x900
                            capture PNG apres navigation reelle
  explorer-cli.exe dom <url> > rapports\<site>_dom.txt
  explorer-cli.exe mem <url> > rapports\<site>_mem.txt
  explorer-cli.exe jprobe <url>
                            sante JS de la page

MANIFEST.txt                commit + branche + etat de l'arbre
                            du build — a citer en tete de chaque
                            rapport (jamais de verdict sur un
                            binaire perime, leçon R-75).
"@
$lisez | Out-File -FilePath (Join-Path $DestDir 'LANCE-MOI.txt') -Encoding utf8

Write-Host ''
Write-Host "Test terrain livre : $DestDir" -ForegroundColor Green
Write-Host '  app\explorer_app.exe   naviguer (moteur in-process)' -ForegroundColor Green
Write-Host '  rapports\            deposer captures + notes' -ForegroundColor Green
