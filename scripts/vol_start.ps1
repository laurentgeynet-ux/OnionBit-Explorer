# This file is part of Explorer.
# Copyright (C) 2026 Laurent Geynet
# SPDX-License-Identifier: MIT OR Apache-2.0

<#
.SYNOPSIS
  Démarre un vol agent — UNE commande (étape 317) : allocation
  atomique vol<N>, build release, spawn daemon + Job Object.
.DESCRIPTION
  Remplace la checklist de lancement du skill `vol` — les
  conventions deviennent des mécanismes :

    1. CLAIM ATOMIQUE : `vol<N>` est créé puis `CLAIMED.json`
       ouvert en `CreateNew` (O_EXCL) — le premier écrivain
       possède le numéro ; plus de course `ls → max+1 → claim`
       entre pistes parallèles sur corpus partagé (jonction).
       Un claim vivant = CLAIMED.json sans `DONE` (`vol_status`
       est imprimé en tête de sortie — pas de fichier à tenir).
    2. Arbre : `git status --porcelain` doit être vide sur le
       repo de build (`-Force` pour un WIP assumé — noté dans
       CLAIMED.json) ; `git rev-parse main` est relevé (jamais
       HEAD — un worktree détaché le fige).
    3. Build : `cargo build --release -p explorer-daemon` —
       `CARGO_TARGET_DIR` honoré tel quel, défaut
       `C:\cargo-target\<leaf-repo>` (incrémental stable, la
       convention « un target par agent » est laissée à
       l'appelant qui pose sa variable).
    4. Spawn : `spawn_daemon.ps1` (watchdog cmd caché, canal
       `session`, exe.log + exit.log + pidfile). Les envs
       `EXPLORER_DEATH_LOG`/`EXPLORER_FLIGHT_LOG` pointent sur
       `vol<N>\logs\` — l'instrumentation atterrit dans le vol,
       plus d'archivage manuel des journaux.
    5. Job Object : `vol_joblimit.ps1` détaché (-LimitMB) — le
       handle du job vit tant que le pwsh gardien vit (enfant
       de la session : mort-en-groupe lisible, pas de kill par
       PID au seuil).

  En cas d'échec à une étape : CLAIMED.json passe à
  `state=aborted` (archéologie honnête, pas de trou silencieux).
.EXAMPLE
  pwsh -File scripts/vol_start.ps1 -Mission REVALID-C628 `
    -Cibles nypost,shopify,notion
#>
param(
  # Code mission majuscules (REVALID<n>, MONDE-INEDIT<n>…).
  [string]$Mission = 'VOL',
  # Nom déclaré — défaut "agent<N>".
  [string]$Agent = '',
  # Cibles déclarées (coordination — lisible par les autres pistes).
  [string[]]$Cibles = @(),
  # Repo de build — défaut : dépôt du script (passer le worktree
  # de vol si le binaire ne sort pas du dépôt principal).
  [string]$Repo = '',
  # Port forcé ; 0 = premier libre ≥ 9385 (jamais 9377 = humain).
  [int]$Port = 0,
  [long]$LimitMB = 5120,
  # Sauter le build (binaire déjà frais, vérifié autrement).
  [switch]$NoBuild,
  # Autoriser un arbre sale (noté `wip` dans CLAIMED.json).
  [switch]$Force
)
$ErrorActionPreference = 'Stop'

$repo = if ($Repo -ne '') { $Repo } else { Split-Path -Parent $PSScriptRoot }
$repo = (Resolve-Path $repo).Path
$vols = Join-Path $repo 'Explorer-data\vols'
New-Item -ItemType Directory -Force -Path $vols | Out-Null

# --- claims vivants (CLAIMED.json sans DONE) : coordination ---
$usedPorts = @{}
foreach ($d in Get-ChildItem $vols -Directory -ErrorAction SilentlyContinue) {
  $cj = Join-Path $d.FullName 'CLAIMED.json'
  if ((Test-Path $cj) -and -not (Test-Path (Join-Path $d.FullName 'DONE'))) {
    try {
      $j = Get-Content $cj -Raw | ConvertFrom-Json
      if ($j.port) { $usedPorts[[int]$j.port] = $true }
      Write-Output ("actif : {0} agent={1} port={2} mission={3}" -f `
        $d.Name, $j.agent, $j.port, $j.mission)
    } catch { }
  }
}

# --- arbre propre avant tout build (verdict = binaire frais) ---
$dirty = git -C $repo status --porcelain
$sha = (git -C $repo rev-parse main).Trim()
$head = (git -C $repo rev-parse HEAD).Trim()
if ($dirty -and -not $Force) {
  Write-Error "arbre de build sale — commiter/stasher, voler un worktree, ou -Force assumé"
  exit 1
}
if ($head -ne $sha) {
  Write-Output "note : HEAD $head ≠ main $sha (worktree détaché — le sha de référence est main)"
}

# --- 1. allocation atomique : max(vol<N>) + mkdir + CreateNew ---
# (seuls `vol<N>` comptent — `field<N>`/`evaldump<N>` sont des
# séquences à part ; les suffixes de timestamp type
# `harvest-*-075503` doivent rester hors du max)
$max = 0
foreach ($d in Get-ChildItem $vols -Directory -Filter 'vol*' `
    -ErrorAction SilentlyContinue) {
  if ($d.Name -match '^vol(\d+)$') { $max = [Math]::Max($max, [int]$Matches[1]) }
}
$N = 0; $voldir = ''; $claimStream = $null
for ($cand = $max + 1; $cand -lt $max + 500 -and -not $claimStream; $cand++) {
  $dir = Join-Path $vols "vol$cand"
  New-Item -ItemType Directory -Force -Path $dir | Out-Null
  try {
    # CreateNew = O_CREAT|O_EXCL : le premier écrivain possède N.
    $claimStream = [System.IO.File]::Open(
      (Join-Path $dir 'CLAIMED.json'),
      [System.IO.FileMode]::CreateNew,
      [System.IO.FileAccess]::Write,
      [System.IO.FileShare]::None)
    $N = $cand; $voldir = $dir
  } catch [System.IO.IOException] { } # déjà claimé — numéro suivant
}
if (-not $claimStream) { Write-Error 'aucun numéro de vol libre'; exit 1 }

$agent = if ($Agent -ne '') { $Agent } else { "agent$N" }
$logdir = Join-Path $voldir 'logs'
New-Item -ItemType Directory -Force -Path $logdir | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $voldir 'captures') | Out-Null

function Write-Claim([hashtable]$Extra) {
  $j = [ordered]@{
    vol        = $N
    agent      = $agent
    mission    = $Mission
    repo       = $repo
    sha_main   = $sha
    head       = $head
    wip        = [bool]$dirty
    cibles     = $Cibles
    claimed_at = [DateTimeOffset]::Now.ToUnixTimeSeconds()
  }
  foreach ($k in $Extra.Keys) { $j[$k] = $Extra[$k] }
  $text = ($j | ConvertTo-Json)
  $claimStream.Position = 0
  $claimStream.SetLength(0)
  $b = [Text.Encoding]::UTF8.GetBytes($text)
  $claimStream.Write($b, 0, $b.Length)
  $claimStream.Flush()
}
function Abort-Vol([string]$Why) {
  Write-Claim @{ state = 'aborted'; error = $Why }
  $claimStream.Close()
  Write-Error $Why; exit 1
}
Write-Claim @{ state = 'claimed' }

# --- 2/3. build release (Finished attendu par cargo) ---
if (-not $NoBuild) {
  if (-not $env:CARGO_TARGET_DIR) {
    $env:CARGO_TARGET_DIR = "C:\cargo-target\$(Split-Path -Leaf $repo)"
  }
  Write-Output "build : cargo build --release -p explorer-daemon (CARGO_TARGET_DIR=$env:CARGO_TARGET_DIR)"
  Push-Location $repo
  try { cargo build --release -p explorer-daemon } finally { Pop-Location }
  if ($LASTEXITCODE -ne 0) { Abort-Vol 'build échoué' }
}

# --- 4. port libre ≥ 9385 (hors claims vivants + listeners) ---
if ($Port -eq 0) {
  for ($p = 9385; $p -lt 9600 -and $Port -eq 0; $p++) {
    if ($usedPorts[$p]) { continue }
    try {
      $l = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Parse('127.0.0.1'), $p)
      $l.Start(); $l.Stop(); $Port = $p
    } catch { }
  }
}
if ($Port -eq 0) { Abort-Vol 'aucun port libre 9385-9599' }

# --- 5. spawn daemon (envs cuits dans la ligne cmd par le script) ---
$env:EXPLORER_VIEWPORT   = '1366x900'
$env:EXPLORER_MEM_DIAG   = '1'
$env:EXPLORER_PROFILE    = "vol:vol$N"
$env:EXPLORER_DEATH_LOG  = Join-Path $logdir 'explorer_daemon_death.log'
$env:EXPLORER_FLIGHT_LOG = Join-Path $logdir 'explorer_daemon_flight.log'
Write-Claim @{ state = 'spawning'; port = $Port }

$out = & pwsh -NoProfile -File (Join-Path $PSScriptRoot 'spawn_daemon.ps1') `
  -Port $Port -Repo $repo -LogDir $logdir 2>&1
$out | ForEach-Object { Write-Output $_ }
if (($out | Select-String -Pattern 'en ligne').Count -eq 0) {
  Abort-Vol "daemon $Port muet — voir $logdir"
}
$dpid = 0
$m = $out | Select-String -Pattern 'daemon PID (\d+)' | Select-Object -First 1
if ($m) { $dpid = [int]$m.Matches[0].Groups[1].Value }
if ($dpid -eq 0) { Abort-Vol 'daemon en ligne mais PID introuvable' }
$exe = (($out | Select-String -Pattern '^exe=(\S+)') | Select-Object -First 1)
$exePath = if ($exe) { $exe.Matches[0].Groups[1].Value } else { '' }

# --- 6. Job Object mémoire (gardien détaché, meurt avec la session) ---
$null = Start-Process pwsh -WindowStyle Hidden -PassThru -ArgumentList @(
  '-NoProfile', '-File', (Join-Path $PSScriptRoot 'vol_joblimit.ps1'),
  '-TargetPid', "$dpid", '-LimitMB', "$LimitMB")

Write-Claim @{ state = 'flying'; port = $Port; daemon_pid = $dpid; exe = $exePath; limit_mb = $LimitMB }
$claimStream.Close()

Write-Output ''
Write-Output "=== vol $N prêt ==="
Write-Output "agent=$agent mission=$Mission port=$Port daemon_pid=$dpid"
Write-Output "binaire=$exePath sha_main=$sha"
Write-Output "profil=vol:vol$N logs=$logdir"
Write-Output "clôture : pwsh -File scripts/vol_close.ps1 -Vol $N"
