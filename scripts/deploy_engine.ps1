# This file is part of Explorer.
# Copyright (C) 2026 Laurent Geynet
# SPDX-License-Identifier: MIT OR Apache-2.0

# deploy_engine.ps1 — source de verite du runtime moteur.
#
# Canonique : <target-dir>[/<triple>]\<Profile>\explorer_ffi.dll
# (cdylib du crate explorer-ffi — `cargo build --release -p
# explorer-ffi` SEUL la produit). Le target-dir se resout comme
# cargo : -TargetDir > `cargo metadata` (CARGO_TARGET_DIR >
# configs projet et globale > .\target) ; le sous-dossier
# <triple> couvre un `build.target` explicite (config globale).
#
# Les copies sont INEVITABLES : LoadLibrary resout la DLL
# adjacente a l'exe consommateur (explorer_app, daemon,
# probes). Ce script est le SEUL chemin de distribution :
# build -> copie -> manifest sidecar (sha256+abi+commit) ->
# audit re-hashe de chaque destination. Plus de copie a la
# main, plus de DLL qui se balade sans provenance (lecon
# etape 119 : une dll perimee adjacente gagnait la sonde
# tryLoad ; etape 187 : la dll s'auto-identifie via
# explorer_abi_version, le sidecar la date au depot).
#
# Usage :
#   pwsh scripts\deploy_engine.ps1              # build + deploie + audit
#   pwsh scripts\deploy_engine.ps1 -VerifyOnly  # audit seul (vols, CI)
#   pwsh scripts\deploy_engine.ps1 -NoBuild     # deploie l'existant + audit
#   pwsh scripts\deploy_engine.ps1 -ExtraDestinations <d1,d2,...>

param(
    [switch]$NoBuild,
    [switch]$VerifyOnly,
    [ValidateSet('debug', 'release')]
    [string]$Profile = 'release',
    [string]$TargetDir = '',
    [string[]]$ExtraDestinations = @()
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot

# target dir effectif : parametre, sinon resolution canonique
# cargo via `cargo metadata` (CARGO_TARGET_DIR > configs projet
# et globale > <workspace>/target — la chaine entiere, y compris
# la config utilisateur hors depot, qu'une regex de config.toml
# ne voit pas).
$target = if ($TargetDir) {
    $TargetDir
} else {
    Push-Location $repo
    try {
        (cargo metadata --no-deps --format-version 1 |
            ConvertFrom-Json).target_directory
    } finally { Pop-Location }
}
if (-not [System.IO.Path]::IsPathRooted($target)) {
    $target = Join-Path $repo $target
}
# Un `build.target` explicite (config globale) range les
# artefacts finaux sous <target>/<triple>/<profil> — repli
# <target>/<profil> (meme regle que spawn_daemon.ps1).
# Resolu par Get-CanonDll : le binaire peut n'apparaitre
# qu'APRES le `cargo build` ci-dessous.
$triple = ''
$v = & rustc -vV 2>$null | Select-String -Pattern '^host: (\S+)'
if ($v) { $triple = $v.Matches[0].Groups[1].Value }
$script:dllCands = @(
    $(if ($triple) { Join-Path $target "$triple\$Profile\explorer_ffi.dll" }),
    (Join-Path $target "$Profile\explorer_ffi.dll")
) | Where-Object { $_ }
function Get-CanonDll {
    $hit = $script:dllCands | Where-Object { Test-Path $_ } |
        Select-Object -First 1
    if ($hit) { $hit } else { $script:dllCands[0] }
}
$dll = Get-CanonDll

# Registre des consommateurs — chaque entree = un dossier qui
# doit contenir explorer_ffi.dll ADJACENTE a l'exe qu'il
# heberge. Ajouter ici toute nouvelle cible (jamais de copie
# ad-hoc).
$destinations = @(
    'C:\OnionBit-Explorer\app\build\windows\x64\runner\Debug',
    'C:\OnionBit-Explorer\app\build\windows\x64\runner\Release',
    'C:\OnionBit-Explorer\dist\Explorer\windows',
    'C:\OnionBit-Explorer\dist\Explorer-0.2.0-windows-x64\windows',
    'C:\OnionBit-Explorer\terrain-bin\app',
    'C:\OnionBit-Explorer\terrain-bin',
    'C:\Users\Lou\Desktop\Onion explorer\windows'
) + $ExtraDestinations | Select-Object -Unique

# Sites audites mais JAMAIS ecrits — reliques de l'ancienne
# config target-dir epinglee (C:\cargo-target) : signalees
# DIVERGENT a chaque run tant qu'une copie orpheline y
# traine ; suppression manuelle a la charge du projet.
$auditOnly = @(
    'C:\cargo-target\OnionBit-Explorer\debug',
    'C:\cargo-target\OnionBit-Explorer\release'
)

function Get-AbiVersion {
    # ABI lue dans la source — verite code, pas devinee du
    # binaire : premiere ligne du corps `fn explorer_abi_version`
    # reduite a un litteral entier (`    1`).
    $src = Join-Path $repo 'crates\explorer-ffi\src\lib.rs'
    $m = Select-String -Path $src -Pattern 'fn explorer_abi_version' -Context 0,8 |
        Select-Object -First 1
    if ($m) {
        foreach ($line in $m.Context.PostContext) {
            if ($line -match '^\s*(\d+)\s*$') { return [int]$Matches[1] }
            if ($line -match '\}') { break }
        }
    }
    return -1
}

function Get-RepoState {
    $commit = (git -C $repo rev-parse --short HEAD).Trim()
    $dirty = (git -C $repo status --porcelain).Length -gt 0
    return @{ commit = $commit; dirty = $dirty }
}

if (-not $VerifyOnly) {
    if (-not $NoBuild) {
        Write-Host ">> build canonique : cargo build --$Profile -p explorer-ffi"
        cargo build --$Profile -p explorer-ffi
        if ($LASTEXITCODE -ne 0) { throw "cargo build : $LASTEXITCODE" }
    }
    $dll = Get-CanonDll
    if (-not (Test-Path $dll)) { throw "DLL canonique absente : $dll" }

    $canon = (Get-FileHash $dll -Algorithm SHA256).Hash.ToLower()
    $state = Get-RepoState
    $abi = Get-AbiVersion
    $manifest = @{
        file    = 'explorer_ffi.dll'
        sha256  = $canon
        abi     = $abi
        commit  = $state.commit
        dirty   = $state.dirty
        utc     = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
        source  = $target
        profile = $Profile
    } | ConvertTo-Json -Compress

    foreach ($d in $destinations) {
        if (-not (Test-Path $d)) {
            Write-Warning "destination absente (ignoree) : $d"
            continue
        }
        Copy-Item $dll -Destination $d -Force
        $manifest | Out-File (Join-Path $d 'explorer_ffi.manifest.json') -Encoding utf8 -NoNewline
        Write-Host "  -> $d"
    }
}

# -- audit : chaque copie doit egaler la canonique, et son
# sidecar declarer le meme hash ---------------------------------

$dll = Get-CanonDll
if (-not (Test-Path $dll)) { throw "DLL canonique absente : $dll" }
$canon = (Get-FileHash $dll -Algorithm SHA256).Hash.ToLower()
$bad = 0
Write-Host ''
Write-Host ('canonique {0} ({1})' -f $canon.Substring(0, 12), $Profile)
foreach ($d in $destinations) {
    $copy = Join-Path $d 'explorer_ffi.dll'
    if (-not (Test-Path $copy)) { Write-Host "  ABSENT   $d" ; continue }
    $h = (Get-FileHash $copy -Algorithm SHA256).Hash.ToLower()
    $side = Join-Path $d 'explorer_ffi.manifest.json'
    $decl = ''
    if (Test-Path $side) {
        $decl = (Get-Content $side -Raw | ConvertFrom-Json).sha256
    }
    $ok = ($h -eq $canon) -and ($decl -eq '' -or $decl -eq $canon)
    if (-not $ok) { $bad++ }
    $tag = if ($ok) { 'OK' } else { 'DIVERGENT' }
    Write-Host ('  {0,-9} {1}  {2}' -f $tag, $h.Substring(0, 12), $d)
}
foreach ($d in $auditOnly) {
    $copy = Join-Path $d 'explorer_ffi.dll'
    if (-not (Test-Path $copy)) { continue }
    $h = (Get-FileHash $copy -Algorithm SHA256).Hash.ToLower()
    $tag = if ($h -eq $canon) { 'EXTERNE=' } else { 'EXTERNE=' + [char]0x2260 }
    Write-Host ('  {0,-9} {1}  {2} (audit-only — relique, a purger manuellement)' -f $tag, $h.Substring(0, 12), $d)
}
if ($bad -gt 0) {
    Write-Error "$bad copie(s) divergente(s) de la canonique"
    exit 1
}
Write-Host 'audit : toutes les copies == canonique' -ForegroundColor Green
