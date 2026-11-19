# This file is part of Explorer.
# Copyright (C) 2026 Laurent Geynet
# SPDX-License-Identifier: MIT OR Apache-2.0

<#
.SYNOPSIS
  Ménage des worktrees éphémères de vol — `*-vol<N>-wt`.
.DESCRIPTION
  `vol_start.ps1` crée un worktree `<repo>-vol<N>-wt` par vol ;
  il survit au vol (suppression = décision de l'opérateur en
  fin de mission). Ce script liste les worktrees de vol et les
  classe d'après le corpus :

    fait    : `Explorer-data\vols\vol<N>\DONE` présent → le vol
              est clos, le worktree est retirable.
    orphelin: pas de `vol<N>` dans le corpus → jamais claimé
              (crash entre mkdir et CLAIMED.json) → retirable.
    VIVANT  : CLAIMED.json sans DONE → vol en cours, JAMAIS
              retiré ici.

  Sans `-Apply` c'est un inventaire en lecture seule ; avec
  `-Apply`, les `fait`/`orphelin` sont retirés
  (`git worktree remove --force` — target/ local part avec) et
  `git worktree prune` solde les entrées administratives.

  Les worktrees laissés vivants par crash de la station se
  reconnaissent au nom `-vol<N>-wt` — ce script est le ménage.
.EXAMPLE
  pwsh -File scripts/vol_clean.ps1            # inventaire
  pwsh -File scripts/vol_clean.ps1 -Apply     # retire les clos
#>
param(
  # Repo dont on balaie les worktrees (défaut : repo du script).
  [string]$Repo = '',
  [switch]$Apply
)
$ErrorActionPreference = 'Stop'

$repo = if ($Repo -ne '') { (Resolve-Path $Repo).Path } else { Split-Path -Parent $PSScriptRoot }
# Corpus = repo du script (jonction → même corpus physique
# quel que soit le clone) ; -Repo ne choisit que les worktrees.
$vols = Join-Path (Split-Path -Parent $PSScriptRoot) 'Explorer-data\vols'

$rows = @()
foreach ($line in (git -C $repo worktree list --porcelain)) {
  if ($line -notmatch '^worktree (.+)$') { continue }
  $path = $Matches[1]
  if ((Split-Path -Leaf $path) -notmatch 'vol(\d+)-wt$') { continue }
  $n = [int]$Matches[1]
  $vd = Join-Path $vols "vol$n"
  $state = if (Test-Path (Join-Path $vd 'DONE')) { 'fait' }
           elseif (Test-Path (Join-Path $vd 'CLAIMED.json')) { 'VIVANT' }
           else { 'orphelin' }
  $exists = Test-Path $path
  $rows += [pscustomobject]@{ worktree = $path; vol = $n; etat = $state; present = $exists }
}

if (-not $rows) { Write-Output 'aucun worktree de vol (*-vol<N>-wt)'; exit 0 }
$rows | Format-Table -AutoSize | Out-String | Write-Output

if (-not $Apply) {
  Write-Output 'lecture seule — -Apply retire les « fait » et « orphelin »'
  exit 0
}
foreach ($r in $rows) {
  if ($r.etat -eq 'VIVANT') { continue }
  git -C $repo worktree remove --force $r.worktree 2>&1 |
    ForEach-Object { Write-Output $_ }
  if ($LASTEXITCODE -eq 0) { Write-Output "retiré : $($r.worktree)" }
  else { Write-Warning "échec retrait $($r.worktree)" }
}
git -C $repo worktree prune
Write-Output 'prune fait'
