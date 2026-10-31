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
foreach ($a in $artifacts) {
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
    Copy-Item (Join-Path $release 'explorer_ffi.dll') -Destination $appOut -Force
}

# onionmedia_ffi.dll — source officielle = le package
# C:\Onion-explorer-media (OnionMedia sur-mesure ; l'ancien
# repo D:\Projet\OnionMedia est obsolete pour Explorer).
# Re-deployee a chaque terrain : une DLL stale dans runner/
# apres un `flutter clean` resterait sinon en place, et le
# daemon de vol la resout adjacente a explorer-daemon.exe.
$mediaDll = 'C:\Onion-explorer-media\target\release\onionmedia_ffi.dll'
if (Test-Path $mediaDll) {
    Copy-Item $mediaDll -Destination $OutDir -Force
    if ($appOut -and (Test-Path $appOut)) {
        Copy-Item $mediaDll -Destination $appOut -Force
    }
} else {
    Write-Warning "onionmedia_ffi.dll package absente : $mediaDll (build le package avant terrain)"
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
