# This file is part of Explorer.
# Copyright (C) 2026 Laurent Geynet
# SPDX-License-Identifier: MIT OR Apache-2.0

<#
.SYNOPSIS
  Extrait le sous-ensemble WPT ciblé (étape 48, ADR-0010).

.DESCRIPTION
  Lit `crates/explorer-render/tests/data/wpt/manifest.txt`,
  télécharge chaque test depuis web-platform-tests/wpt
  (@ master) ainsi que ses références `rel="match|mismatch"`
  — résolution récursive, un seul niveau suffit en pratique
  (les refs ne référencent jamais d'autres refs).
  Télécharge aussi la fonte de test Ahem.

  Les fixtures extraites ne sont JAMAIS éditées à la main :
  un cas divergent entre dans `wpt_skip.txt` (versionné,
  raisonné), pas dans une correction locale.
#>

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSCommandPath)
$data = Join-Path $root 'crates\explorer-render\tests\data\wpt'
$manifest = Join-Path $data 'manifest.txt'
$base = 'https://raw.githubusercontent.com/web-platform-tests/wpt/master'

function Get-Rel([string]$html) {
    if ($html -match 'rel\s*=\s*["'']?\s*match\s*["'']?[^>]*href\s*=\s*["'']([^"'']+)') {
        return @{ kind = 'match'; href = $Matches[1] }
    }
    if ($html -match 'href\s*=\s*["'']([^"'']+)["''][^>]*rel\s*=\s*["'']?\s*match\s*["'']?') {
        return @{ kind = 'match'; href = $Matches[1] }
    }
    if ($html -match 'rel\s*=\s*["'']?\s*mismatch\s*["'']?[^>]*href\s*=\s*["'']([^"'']+)') {
        return @{ kind = 'mismatch'; href = $Matches[1] }
    }
    if ($html -match 'href\s*=\s*["'']([^"'']+)["''][^>]*rel\s*=\s*["'']?\s*mismatch\s*["'']?') {
        return @{ kind = 'mismatch'; href = $Matches[1] }
    }
    return $null
}

function Save-Rel([string]$relUrl, [string]$relLocal) {
    if (Test-Path $relLocal) { return }
    $dir = Split-Path -Parent $relLocal
    if (!(Test-Path $dir)) { New-Item -ItemType Directory -Force -Path $dir | Out-Null }
    Invoke-WebRequest -Uri $relUrl -OutFile $relLocal -UseBasicParsing
    Write-Output "  + $relUrl"
}

$tests = Get-Content $manifest |
    Where-Object { $_ -match '\S' -and $_ -notmatch '^\s*#' }

$count = 0
foreach ($t in $tests) {
    $t = $t.Trim()
    $local = Join-Path $data ($t -replace '/', '\')
    if (!(Test-Path $local)) {
        $dir = Split-Path -Parent $local
        if (!(Test-Path $dir)) { New-Item -ItemType Directory -Force -Path $dir | Out-Null }
        Invoke-WebRequest -Uri "$base/$t" -OutFile $local -UseBasicParsing
        Write-Output "  $t"
    }
    $count++
    $html = Get-Content -Raw $local
    $rel = Get-Rel $html
    if ($null -ne $rel) {
        $href = $rel.href -replace '^\./', ''
        # `/css/...` = chemin racine-dépôt (R-32 :
        # margin-collapse-clear-001 référence floats-clear).
        $baseDir = Split-Path -Parent $t
        if ($href.StartsWith('/')) {
            $baseDir = ''
            $href = $href.TrimStart('/')
        }
        $relPath = ("$baseDir/$href" -replace '/', '\').TrimStart('\')
        # Normalise les « ../ » simples.
        while ($relPath -match '[^\\]+\\\.\.\\') {
            $relPath = $relPath -replace '[^\\]+\\\.\.\\', ''
        }
        $relUrlPath = $relPath -replace '\\', '/'
        Save-Rel "$base/$relUrlPath" (Join-Path $data $relPath)
    }
}

# Fonte de test Ahem — métriques 1em×1em, référence reftest.
$ahem = Join-Path $data 'ahem.ttf'
if (!(Test-Path $ahem)) {
    Invoke-WebRequest -Uri "$base/fonts/Ahem.ttf" -OutFile $ahem -UseBasicParsing
    Write-Output "  + ahem.ttf"
}

Write-Output "WPT : $count tests présents dans $data"
