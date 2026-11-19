# This file is part of Explorer.
# Copyright (C) 2026 Laurent Geynet
# SPDX-License-Identifier: MIT OR Apache-2.0

<#
.SYNOPSIS
  Clôt un vol agent — UNE commande (étape 317) : arrêt daemon,
  GC du registre, marqueur DONE, commit du corpus.
.DESCRIPTION
  Remplace la checklist de clôture du skill `vol` :

    1. Lit `vol<N>\CLAIMED.json` (port, daemon_pid — ou pidfile
       `logs\daemon-<port>.pid`) ; avertit si le rapport
       `rapports\RAPPORT_VOL_*_AGENT<N>_*.md` est absent.
    2. Arrêt : `POST /api/shutdown` (sentinelle `shutdown:api`,
       annonce retirée par sa garde) puis attente ; si le pid
       vit encore → `taskkill /PID /F` — **uniquement** le pid
       du claim/pidfile, jamais `/IM` (daemons voisins d'autres
       pistes). `-KeepDaemon` : daemon sain réutilisé au vol
       suivant — saut de l'arrêt.
    3. GC du registre `Explorer-data\daemons\*.json` : une
       annonce est périmée si pid mort, recyclé hors
       explorer-daemon, ou (pid, port) absent des listeners —
       même arbitre que `announce_stale` (étape 284). Les
       fantômes sont supprimés, pas devinés.
    4. `vol<N>\DONE` : libère le claim (un claim vivant =
       CLAIMED.json sans DONE — lisible par `vol_start`).
    5. Commit ciblé dans le git local du corpus
       (`Explorer-data\vols\.git`) : `vol<N>/` + rapports de
       l'agent N + SYNTHESE.md/FILE.md/CLAIMS.md/README.md —
       jamais `-A` (d'autres pistes écrivent en parallèle).
.EXAMPLE
  pwsh -File scripts/vol_close.ps1 -Vol 904
#>
param(
  # Numéro de vol — "904" ou "vol904".
  [Parameter(Mandatory = $true)]
  [string]$Vol,
  # Garder le daemon vivant (sain, réutilisé au vol suivant).
  [switch]$KeepDaemon,
  [switch]$NoCommit
)
$ErrorActionPreference = 'Stop'

if ($Vol -notmatch '\d+') { Write-Error "vol illisible : '$Vol'"; exit 1 }
$N = [int]$Matches[0]
$repo = Split-Path -Parent $PSScriptRoot
$vols = Join-Path $repo 'Explorer-data\vols'
$voldir = Join-Path $vols "vol$N"
$claimFile = Join-Path $voldir 'CLAIMED.json'
if (-not (Test-Path $claimFile)) { Write-Error "pas de claim : $claimFile"; exit 1 }
$claim = Get-Content $claimFile -Raw | ConvertFrom-Json
$port = if ($claim.port) { [int]$claim.port } else { 0 }
$dpid = if ($claim.daemon_pid) { [int]$claim.daemon_pid } else { 0 }
if ($dpid -eq 0 -and $port -gt 0) {
  $pf = Join-Path $voldir "logs\daemon-$port.pid"
  if (Test-Path $pf) { $dpid = [int](Get-Content $pf -Raw).Trim() }
}

# Rapport attendu — avertissement, pas un blocage (le rapport
# est la preuve immuable, mais la clôture ne le juge pas).
$rep = Get-ChildItem (Join-Path $vols 'rapports') `
  -Filter "RAPPORT_VOL_*_AGENT${N}_*.md" -ErrorAction SilentlyContinue
if (-not $rep) {
  Write-Warning "aucun RAPPORT_VOL_*_AGENT${N}_*.md déposé — synchro §G en dette ?"
}

# --- 1. arrêt daemon : graceful d'abord, pidfile seul ensuite ---
$outcome = 'absent'
if (-not $KeepDaemon -and $port -gt 0) {
  # Le POST /api/shutdown vise le PORT : on n'envoie que si le
  # listener appartient au pid du claim. Une course au bind (vols
  # 908/909) peut laisser le daemon d'une AUTRE piste sur ce port —
  # sans cette preuve le POST le tuerait en plein vol.
  $owner = 0
  foreach ($line in (netstat -ano)) {
    if ($line -notmatch 'LISTENING') { continue }
    $t = $line -split '\s+' | Where-Object { $_ }
    if ($t.Count -ge 5 -and $t[1] -match ":(\d+)$" -and `
        [int]$Matches[1] -eq $port) { $owner = [int]$t[-1]; break }
  }
  if ($owner -gt 0 -and $owner -ne $dpid) {
    Write-Warning "port $port possédé par pid $owner ≠ claim $dpid — daemon voisin épargné"
    $outcome = "foreign-owner:$owner"
  } else {
    try {
      $null = Invoke-RestMethod "http://127.0.0.1:$port/api/shutdown" `
        -Method Post -TimeoutSec 3
    } catch { }
  }
  if ($dpid -gt 0) {
    $deadline = (Get-Date).AddSeconds(5)
    while ((Get-Date) -lt $deadline -and
           (Get-Process -Id $dpid -ErrorAction SilentlyContinue)) {
      Start-Sleep -Milliseconds 400
    }
    $proc = Get-Process -Id $dpid -ErrorAction SilentlyContinue
    if ($proc -and $proc.ProcessName -eq 'explorer-daemon') {
      taskkill /PID $dpid /F | Out-Null
      $outcome = "$outcome+killed" -replace '^absent\+', ''
    } elseif ($proc) {
      # pid recyclé sur un processus tiers — jamais de kill
      # à main levée (vol 698 : PID 7392 = autre daemon tué).
      Write-Warning "pid $dpid recyclé ($($proc.ProcessName)) — taskkill sauté"
      $outcome = "$outcome+pid-recycled" -replace '^absent\+', ''
    } elseif ($outcome -eq 'absent') {
      $outcome = 'graceful'
    }
  } elseif ($outcome -eq 'absent') {
    $outcome = 'shutdown-posted'
  }
} elseif ($KeepDaemon) {
  $alive = [bool](Get-Process -Id $dpid -ErrorAction SilentlyContinue)
  $outcome = if ($alive) { 'kept-alive' } else { 'kept-but-dead' }
}

# --- 2. GC du registre : périmé = pid mort / image autre /
#        port non lié par ce pid ---
$dreg = Join-Path $repo 'Explorer-data\daemons'
$removed = 0
if (Test-Path $dreg) {
  $listening = @{}
  foreach ($line in (netstat -ano)) {
    if ($line -match 'LISTENING' -and $line -match 'TCP') {
      $t = $line -split '\s+' | Where-Object { $_ }
      if ($t.Count -ge 5 -and $t[1] -match ':(\d+)$') {
        $listening["$($t[4]):$($Matches[1])"] = $true
      }
    }
  }
  foreach ($f in Get-ChildItem $dreg -Filter '*.json' -ErrorAction SilentlyContinue) {
    try { $a = Get-Content $f.FullName -Raw | ConvertFrom-Json } catch { continue }
    $proc = Get-Process -Id $a.pid -ErrorAction SilentlyContinue
    $stale = (-not $proc) -or ($proc.ProcessName -ne 'explorer-daemon') -or
             (-not $listening["$($a.pid):$($a.port)"])
    if ($stale) { Remove-Item $f.FullName -Force; $removed++ }
  }
}

# --- 3. DONE : libère le claim ---
$DONE = [ordered]@{
  vol          = $N
  closed_at    = [DateTimeOffset]::Now.ToUnixTimeSeconds()
  daemon       = $outcome
  registry_gc  = $removed
}
$DONE | ConvertTo-Json | Out-File (Join-Path $voldir 'DONE') -Encoding utf8

# --- 4. commit ciblé du corpus (jamais -A : pistes parallèles) ---
if (-not $NoCommit -and (Test-Path (Join-Path $vols '.git'))) {
  # Expansion en PS — un pathspec glob sans correspondance fait
  # échouer TOUT le `git add`.
  $paths = @("vol$N")
  $paths += Get-ChildItem (Join-Path $vols 'rapports') `
    -Filter "RAPPORT_VOL_*_AGENT${N}_*.md" -ErrorAction SilentlyContinue |
    ForEach-Object { "rapports/$($_.Name)" }
  foreach ($f in 'SYNTHESE.md', 'FILE.md', 'CLAIMS.md', 'README.md') {
    if (Test-Path (Join-Path $vols $f)) { $paths += $f }
  }
  git -C $vols add -- $paths
  # -A ferait entrer les WIP des autres pistes ; la liste est
  # bornée au vol N + fichiers d'état partagés écrits par le
  # closer (SYNTHESE/FILE — la synchro §G est leur porte).
  git -C $vols commit -q -m "vol $N — $($claim.mission)"
  if ($LASTEXITCODE -ne 0) { Write-Output 'corpus : rien à committer' }
}

Write-Output ''
Write-Output "=== vol $N clos ==="
Write-Output "daemon=$outcome registre_gc=$removed claim=libéré (DONE)"
if ($claim.worktree) {
  Write-Output "worktree conservé : $($claim.worktree) — ménage fin de mission : scripts/vol_clean.ps1 -Apply"
}
Write-Output "vol suivant : pwsh -File scripts/vol_start.ps1 -Mission <TAG> -Cibles <liste>"
