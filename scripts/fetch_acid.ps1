# This file is part of Explorer.
# Copyright (C) 2026 Laurent Geynet
# SPDX-License-Identifier: MIT OR Apache-2.0

<#
.SYNOPSIS
  Extrait le jalon Acid2 (étape 48, ADR-0010).

.DESCRIPTION
  Télécharge la page de test Acid2 et son image de référence
  officielle depuis acid2.acidtests.org vers
  `crates/explorer-render/tests/data/` :

    acid2.html           la page de test (HTML + CSS inline)
    acid2-reference.png  rendu attendu du visage (168x168)

  Les fixtures extraites ne sont JAMAIS éditées à la main.
  Le harnais `tests/acid2.rs` les consomme : cliquet
  robustesse (pas de panic) + score d'alignement informatif.
#>

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSCommandPath)
$data = Join-Path $root 'crates\explorer-render\tests\data'

$page = Join-Path $data 'acid2.html'
if (!(Test-Path $page)) {
    Invoke-WebRequest -Uri 'http://acid2.acidtests.org/' `
        -OutFile $page -UseBasicParsing
    Write-Output '  + acid2.html'
}

$ref = Join-Path $data 'acid2-reference.png'
if (!(Test-Path $ref)) {
    Invoke-WebRequest -Uri 'http://acid2.acidtests.org/reference.png' `
        -OutFile $ref -UseBasicParsing
    Write-Output '  + acid2-reference.png'
}

Write-Output "Acid2 : fixtures présentes dans $data"
