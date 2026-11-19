# This file is part of Explorer.
# Copyright (C) 2026 Laurent Geynet
# SPDX-License-Identifier: MIT OR Apache-2.0

<#
.SYNOPSIS
  Ménage des worktrees éphémères de vol — `*-vol<N>-wt` par
  vol et `*-wt-<équipe>` d'équipe (`vol_start -Agent <équipe>`).
.DESCRIPTION
  `vol_start.ps1` crée un worktree éphémère par vol
  (`<repo>-vol<N>-wt`, agent anonyme) ou d'équipe
  (`<repo>-wt-<équipe>`, `-Agent` déclaré, repris à chaque
  décollage) ; il survit au vol (suppression = décision de
  l'opérateur en fin de mission). Ce script liste
  les worktrees de vol et les classe d'après le corpus — le
  lien d'un wt d'équipe vit dans `worktree` du CLAIMED.json,
  pas dans son nom :

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
  `-Vol <N>` borne l'action à UN worktree : chaque équipe
  s'occupe de son propre worktree et de son propre daemon —
  le balayage global n'est que pour l'opérateur qui possède
  toutes les pistes.

  Les worktrees laissés vivants par crash de la station se
  reconnaissent aux noms `-vol<N>-wt` / `-wt-` — ce script est
  le ménage.
.EXAMPLE
  pwsh -File scripts/vol_clean.ps1 -Vol 916 -Apply   # son vol
  pwsh -File scripts/vol_clean.ps1                   # inventaire
  pwsh -File scripts/vol_clean.ps1 -Apply            # tous les clos
#>
param(
  # Repo dont on balaie les worktrees (défaut : repo du script).
  [string]$Repo = '',
  # Borner à un vol précis (chaque équipe nettoie le sien).
  [int]$Vol = 0,
  [switch]$Apply
)
$ErrorActionPreference = 'Stop'

$repo = if ($Repo -ne '') { (Resolve-Path $Repo).Path } else { Split-Path -Parent $PSScriptRoot }
# Corpus = repo du script (jonction → même corpus physique
# quel que soit le clone) ; -Repo ne choisit que les worktrees.
$vols = Join-Path (Split-Path -Parent $PSScriptRoot) 'Explorer-data\vols'

# Liens wt→vol via les claims : les worktrees d'équipe
# (`-wt-<token>`) n'ont pas de numéro dans le nom — leur
# identité vit dans `worktree` du CLAIMED.json. Un même wt
# peut servir plusieurs vols d'une équipe.
$claimWt = @{}
foreach ($d in Get-ChildItem $vols -Directory -Filter 'vol*' -ErrorAction SilentlyContinue) {
  $cj = Join-Path $d.FullName 'CLAIMED.json'
  if (-not (Test-Path $cj)) { continue }
  try { $j = Get-Content $cj -Raw | ConvertFrom-Json } catch { continue }
  if (-not $j.worktree) { continue }
  $wp = ([string]$j.worktree) -replace '/', '\'
  if (-not $claimWt.ContainsKey($wp)) { $claimWt[$wp] = @() }
  $claimWt[$wp] += [pscustomobject]@{
    vol  = [int]$j.vol
    done = (Test-Path (Join-Path $d.FullName 'DONE'))
  }
}

$rows = @()
foreach ($line in (git -C $repo worktree list --porcelain)) {
  if ($line -notmatch '^worktree (.+)$') { continue }
  $path = $Matches[1]
  $leaf = Split-Path -Leaf $path
  if ($leaf -match 'vol(\d+)-wt$') {
    $n = [int]$Matches[1]
    $vd = Join-Path $vols "vol$n"
    $state = if (Test-Path (Join-Path $vd 'DONE')) { 'fait' }
             elseif (Test-Path (Join-Path $vd 'CLAIMED.json')) { 'VIVANT' }
             else { 'orphelin' }
    $vs = @($n)
  } elseif ($leaf -match '-wt-') {
    $key = $path -replace '/', '\'
    if ($claimWt.ContainsKey($key)) {
      $uses = $claimWt[$key]
      $vs = @($uses.vol)
      $state = if ($uses | Where-Object { -not $_.done }) { 'VIVANT' } else { 'fait' }
    } else { $vs = @(); $state = 'orphelin' }
  } else { continue }
  $exists = Test-Path $path
  $rows += [pscustomobject]@{ worktree = $path; vol = $vs; etat = $state; present = $exists }
}
if ($Vol -gt 0) { $rows = $rows | Where-Object { $_.vol -contains $Vol } }

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
