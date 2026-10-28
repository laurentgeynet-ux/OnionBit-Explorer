# This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
# SPDX-License-Identifier: MIT OR Apache-2.0
<#
.SYNOPSIS
  Supprime les PNG orphelines de docs/Rapport vol/captures/.

.DESCRIPTION
  Une capture est « liée » si son nom de fichier est cité dans un
  document Markdown du dossier Rapport vol (SYNTHESE.md, rapports
  de vol, rapports de mission, journal). Toute *.png non citée est
  supprimée — le dossier ne doit pas regonfler (purge 7 Go du
  2026-10-28). À lancer après chaque vol, ou périodiquement.

.PARAMETER DryRun
  Affiche ce qui serait supprimé sans toucher au disque.

.EXAMPLE
  pwsh -File scripts/prune_captures.ps1
  pwsh -File scripts/prune_captures.ps1 -DryRun
#>
[CmdletBinding()]
param(
  [switch]$DryRun
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$volDir = Join-Path $root 'docs/Rapport vol'
$captures = Join-Path $volDir 'captures'

if (-not (Test-Path $captures)) {
  Write-Output "captures/ absent — rien à faire."
  exit 0
}

# Références citées : noms de fichiers PNG présents dans les .md
$refs = [System.Collections.Generic.HashSet[string]]::new(
  [System.StringComparer]::OrdinalIgnoreCase)
Get-ChildItem -Path $volDir -Filter '*.md' -Recurse -File |
  ForEach-Object {
    $text = Get-Content $_.FullName -Raw -Encoding UTF8
    foreach ($m in [regex]::Matches($text, '[\w.\-]+\.png')) {
      [void]$refs.Add($m.Value)
    }
  }

$kept = 0
$deleted = 0
$freed = 0L
Get-ChildItem -Path $captures -Filter '*.png' -File |
  ForEach-Object {
    if ($refs.Contains($_.Name)) {
      $kept++
    } else {
      $freed += $_.Length
      if ($DryRun) {
        Write-Output ("orpheline : {0} ({1:N1} Ko)" -f $_.Name, ($_.Length / 1KB))
      } else {
        Remove-Item $_.FullName -Force
      }
      $deleted++
    }
  }

$verb = if ($DryRun) { 'seraient supprimées' } else { 'supprimées' }
Write-Output ("captures/ : {0} liées conservées, {1} {2} ({3:N1} Mo)" -f
  $kept, $deleted, $verb, ($freed / 1MB))
