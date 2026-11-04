# This file is part of Explorer.
# Copyright (C) 2026 Laurent Geynet
# SPDX-License-Identifier: MIT OR Apache-2.0

# build_terrain.ps1 — compile en release les binaires du test terrain
# et les copie dans un dossier dedie avec un manifeste datable
# (commit, branche, etat de l'arbre). Le manifeste est la preuve
# de fraicheur du verdict — lecon R-75 : jamais de verdict sur
# un binaire perime.
#
# Usage : pwsh -File scripts/build_terrain.ps1 [-OutDir terrain-bin]
#         [-CargoTargetDir D:\explorer-g1\target]   # worktree isole

param(
    [string]$OutDir = 'terrain-bin',
    [string]$CargoTargetDir = '',
    [switch]$SkipApp          # ne pas compiler l'app Flutter
)

$ErrorActionPreference = 'Stop'
Set-Location (Split-Path -Parent $PSScriptRoot)

function Step([string]$Name, [scriptblock]$Cmd) {
    Write-Host "==> $Name" -ForegroundColor Cyan
    & $Cmd
    if ($LASTEXITCODE -ne 0) { throw "ECHEC : $Name" }
}

if ($CargoTargetDir) { $env:CARGO_TARGET_DIR = $CargoTargetDir }

# Provenance AVANT le build : le commit teste est celui-ci.
$commit = (git rev-parse HEAD).Trim()
$branch = (git branch --show-current).Trim()
if (-not $branch) { $branch = '(detache)' }
$dirty = [bool](git status --porcelain | Where-Object { $_ -notmatch '^\?\?' })

Step 'cargo build --release (daemon + cli + ffi)' {
    cargo build --release -p explorer-daemon -p explorer-cli -p explorer-ffi
}

# target dir effectif : parametre, sinon config cargo du workspace.
$target = if ($CargoTargetDir) {
    $CargoTargetDir
} else {
    $cfg = Join-Path (Get-Location) '.cargo\config.toml'
    if (Test-Path $cfg) {
        $m = Select-String -Path $cfg -Pattern 'target-dir\s*=\s*"([^"]+)"'
        if ($m) { $m.Matches[0].Groups[1].Value } else { 'target' }
    } else { 'target' }
}
$release = Join-Path $target 'release'

$artifacts = @('explorer-daemon.exe', 'explorer-cli.exe', 'explorer_ffi.dll')
foreach ($a in $artifacts) {
    if (-not (Test-Path (Join-Path $release $a))) {
        throw "artefact absent apres build : $release\$a"
    }
}

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
# explorer_ffi.dll : copie deleguee a deploy_engine.ps1 plus
# bas (manifest sidecar + audit re-hashe — jamais de copie
# ad-hoc, meme doctrine que deploy_media).
foreach ($a in $artifacts | Where-Object { $_ -ne 'explorer_ffi.dll' }) {
    Copy-Item (Join-Path $release $a) -Destination $OutDir -Force
}

# App Flutter release : tout le dossier runner est necessaire
# (exe + data/ + DLLs plugins). explorer_ffi.dll est deposee
# a cote de l'exe — le mode in-process est arme d'office.
if (-not $SkipApp -and (Test-Path 'app/pubspec.yaml')) {
    Push-Location app
    try {
        Step 'flutter build windows --release' { flutter build windows --release }
    } finally {
        Pop-Location
    }
    $runner = 'app\build\windows\x64\runner\Release'
    if (-not (Test-Path "$runner\explorer_app.exe")) {
        throw "app release absente apres build : $runner"
    }
    $appOut = Join-Path $OutDir 'app'
    New-Item -ItemType Directory -Force -Path $appOut | Out-Null
    Copy-Item "$runner\*" -Destination $appOut -Recurse -Force
}

# explorer_ffi.dll — distribution deleguee a
# deploy_engine.ps1 : build canonique -> copies -> manifest
# sidecar (sha256+abi+commit) -> audit re-hashe. Source de
# verite unique : plus de copie ad-hoc, plus de DLL stale
# dans runner/ apres `flutter clean`, et l'audit refuse un
# OutDir divergent de la canonique (etape 187).
$extra = @($OutDir)
if ($appOut -and (Test-Path $appOut)) { $extra += $appOut }
pwsh -NoProfile -File (Join-Path $PSScriptRoot 'deploy_engine.ps1') -NoBuild -Profile release -TargetDir $target -ExtraDestinations $extra
if ($LASTEXITCODE -ne 0) {
    throw "ECHEC : deploy_engine.ps1 (copies moteur divergentes de la canonique)"
}

# onionmedia_ffi.dll + rest_host_probe.exe — distribution
# deleguee au script canonique du workspace media
# (deploy_media.ps1 : build -> copie -> manifest sidecar ->
# audit re-hashe). Source de verite unique : plus de copie
# ad-hoc, plus de DLL stale dans runner/ apres `flutter clean`,
# et l'audit refuse un OutDir divergent de la canonique.
$deploy = 'C:\Onion-explorer-media\scripts\deploy_media.ps1'
if (Test-Path $deploy) {
    pwsh -NoProfile -File $deploy -NoBuild -ExtraDestinations $extra
    if ($LASTEXITCODE -ne 0) {
        throw "ECHEC : deploy_media.ps1 (copies media divergentes de la canonique)"
    }
} else {
    Write-Warning "deploy_media.ps1 absent : $deploy — DLL media non deployee"
}

$etat = if ($dirty) { 'MODIFIE — verdict terrain a risque (WIP)' } else { 'propre' }
$manifest = @"
OnionBit Explorer — binaires terrain
build    : $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')
commit   : $commit
branche  : $branch
arbre    : $etat
source   : $release
contenu  : explorer-daemon.exe  (REST+SSE, --port pour instance dediee)
           explorer-cli.exe     (shot / dom / layout / mem)
           explorer_ffi.dll     (moteur in-process)
           onionmedia_ffi.dll   (lecture media — adjacente
                                racine ET app\, source
                                C:\Onion-explorer-media release)
           rest_host_probe.exe  (hote media REST — dlopen
                                onionmedia_ffi.dll, cycle
                                claim->play->state->release)
           app\                 UI Flutter release — explorer_app.exe
                                avec explorer_ffi.dll adjacente
                                (moteur in-process arme)
"@
$manifest | Out-File -FilePath (Join-Path $OutDir 'MANIFEST.txt') -Encoding utf8

Write-Host ''
Write-Host "Binaires terrain dans $OutDir :" -ForegroundColor Green
Get-ChildItem $OutDir | Format-Table Name, Length, LastWriteTime -AutoSize
Write-Host "commit $commit ($branch) — arbre $etat" -ForegroundColor Green
if ($dirty) {
    Write-Host 'ATTENTION : arbre modifie — le binaire n''est pas' -ForegroundColor Yellow
    Write-Host 'le reflet d''un commit. Verdict terrain fragile.' -ForegroundColor Yellow
}
