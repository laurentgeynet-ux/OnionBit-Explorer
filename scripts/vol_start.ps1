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
    2. Arbre : le sha de référence est `git rev-parse main`
       (jamais HEAD — un worktree détaché le fige). Par défaut
       le build ne touche PAS l'arbre principal : un worktree
       éphémère `<repo>-vol<N>-wt` (voisin de la racine, nom
       reconnaissable → ménage par `vol_clean.ps1`) est créé
       au sha de main — le WIP des autres pistes ne peut pas
       contaminer le binaire. `-Repo <arbre>` = mode expert :
       build direct dans l'arbre donné (exigé propre, ou
       `-Force` assumé — `wip` noté dans CLAIMED.json). Le
       worktree n'est supprimé que sur décision de
       l'opérateur (fin de mission), pas à la clôture du vol.
    3. Build : `cargo build --release -p explorer-daemon` —
       target/ LOCAL de l'arbre de build (config cargo par
       défaut : un target par arbre, plus de `CARGO_TARGET_DIR`
       partagé ni de sous-dossier <triple> ; sccache amortit
       le premier build). Réutilisation : si le claim d'un
       vol précédent prouve (sha_main + exe + worktree) que
       le sha courant est déjà buildé, l'exe est repris tel
       quel via `spawn_daemon -Exe` — pas de rebuild en
       chaîne de campagne. `EXPLORER_DATA` est épinglé au
       corpus du repo du script — `data_root()` du daemon ne
       dépend plus du lieu du worktree.
    4. Spawn : `spawn_daemon.ps1` (watchdog cmd caché, canal
       `session`, exe.log + exit.log + pidfile). Port dérivé du
       numéro de vol (9385 + (N-1) mod 200 — arbitré par le même
       claim atomique) + vérification pid↔listener post-spawn :
       une défaite au bind relance sur le port suivant au lieu
       de laisser un vol zombie sur le daemon d'un voisin. Les envs
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
  # Build DIRECT dans cet arbre (mode expert) — défaut : un
  # worktree éphémère <repo>-vol<N>-wt est créé au sha de main.
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

$scriptRepo = Split-Path -Parent $PSScriptRoot
# Corpus = Explorer-data du repo du script (jonction : même
# corpus physique quel que soit le clone de travail — les
# chemins affichés peuvent résoudre vers la cible physique,
# ce n'est PAS un changement de projet).
$corpus = Join-Path $scriptRepo 'Explorer-data'
$vols = Join-Path $corpus 'vols'
New-Item -ItemType Directory -Force -Path $vols | Out-Null
$jtarget = (Get-Item $corpus -ErrorAction SilentlyContinue).Target
if ($jtarget) { Write-Output "corpus : $corpus → $jtarget (jonction, même corpus partagé)" }

# -Repo explicite = build direct dans cet arbre (expert) ;
# défaut = repo du script → le build vivra dans un worktree
# éphémère vol<N>-wt créé après le claim.
$explicitRepo = $Repo -ne ''
$refRepo = if ($explicitRepo) { (Resolve-Path $Repo).Path } else { $scriptRepo }

# --- claims vivants (CLAIMED.json sans DONE) : coordination ---
$usedPorts = @{}
foreach ($d in Get-ChildItem $vols -Directory -ErrorAction SilentlyContinue) {
  $cj = Join-Path $d.FullName 'CLAIMED.json'
  if ((Test-Path $cj) -and -not (Test-Path (Join-Path $d.FullName 'DONE'))) {
    try {
      $j = Get-Content $cj -Raw | ConvertFrom-Json
      if ($j.port) { $usedPorts[[int]$j.port] = $true }
      # claim > 2 h sans DONE = probable zombie (session morte) —
      # la règle « >2 h » est mécanisée ici, pas mémorisée.
      $age = [DateTimeOffset]::Now.ToUnixTimeSeconds() - [long]$j.claimed_at
      $z = if ($age -gt 7200) { ' (zombie? >2h)' } else { '' }
      Write-Output ("actif : {0} agent={1} port={2} mission={3}{4}" -f `
        $d.Name, $j.agent, $j.port, $j.mission, $z)
    } catch { }
  }
}

# --- sha de référence + arbre (mode expert seul : propre) ---
$dirty = git -C $refRepo status --porcelain
$sha = (git -C $refRepo rev-parse main).Trim()
$head = (git -C $refRepo rev-parse HEAD).Trim()
if ($explicitRepo -and $dirty -and -not $Force) {
  Write-Error "arbre de build sale — commiter/stasher ou -Force assumé"
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
    # FileShare.Read : les équipes concurrentes (Read-UsedPorts,
    # « actif », l'opérateur) lisent le claim PENDANT le vol —
    # avec FileShare.None le handle tenu ~build-long rendait le
    # claim invisible aux pairs (course vols 908/909 nourrie).
    $claimStream = [System.IO.File]::Open(
      (Join-Path $dir 'CLAIMED.json'),
      [System.IO.FileMode]::CreateNew,
      [System.IO.FileAccess]::Write,
      [System.IO.FileShare]::Read)
    $N = $cand; $voldir = $dir
  } catch [System.IO.IOException] { } # déjà claimé — numéro suivant
}
if (-not $claimStream) { Write-Error 'aucun numéro de vol libre'; exit 1 }

$agent = if ($Agent -ne '') { $Agent } else { "agent$N" }
$logdir = Join-Path $voldir 'logs'
New-Item -ItemType Directory -Force -Path $logdir | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $voldir 'captures') | Out-Null

$claimData = [ordered]@{
  vol        = $N
  agent      = $agent
  mission    = $Mission
  repo       = $refRepo
  sha_main   = $sha
  head       = $head
  wip        = [bool]$dirty
  cibles     = $Cibles
  claimed_at = [DateTimeOffset]::Now.ToUnixTimeSeconds()
}
function Write-Claim([hashtable]$Extra) {
  # Merge cumulatif : chaque Write-Claim ajoute des champs à
  # l'état courant (worktree posé à 'building' survit à
  # 'flying' — une réécriture complète l'aurait effacé).
  foreach ($k in $Extra.Keys) { $claimData[$k] = $Extra[$k] }
  $text = ($claimData | ConvertTo-Json)
  $claimStream.Position = 0
  $claimStream.SetLength(0)
  $b = [Text.Encoding]::UTF8.GetBytes($text)
  $claimStream.Write($b, 0, $b.Length)
  $claimStream.Flush()
}
function Stop-VolClaim([string]$Why) {
  Write-Claim @{ state = 'aborted'; error = $Why }
  $claimStream.Close()
  Write-Error $Why; exit 1
}
Write-Claim @{ state = 'claimed' }

# --- 2/3. worktree éphémère + build release ---
# Le binaire ne sort JAMAIS de l'arbre principal : worktree
# `vol<N>-wt` voisin de la racine, épinglé au sha de main —
# un arbre principal sale ne contamine plus le vol, et le nom
# reconnaissable rend le ménage trivial (vol_clean.ps1).
# target/ est local au worktree : provenance = chemin.
$buildRepo = $refRepo; $wt = ''; $exeOverride = ''
if (-not $explicitRepo) {
  if ($NoBuild) {
    Stop-VolClaim '-NoBuild sans -Repo : le worktree frais n''a pas de binaire — passer -Repo <arbre buildé>'
  }
  # Réutilisation : le claim d'un vol précédent prouve
  # (sha_main + exe + worktree) que CE sha est déjà buildé —
  # ~9 min de rebuild épargnées par vol enchaîné à sha
  # constant. L'exe reste dans l'ancien worktree tant que le
  # daemon tourne (vol_clean dessus échouerait proprement —
  # fichier verrouillé).
  foreach ($d in Get-ChildItem $vols -Directory -Filter 'vol*' | Sort-Object Name -Descending) {
    $cj = Join-Path $d.FullName 'CLAIMED.json'
    if (-not (Test-Path $cj)) { continue }
    try { $j = Get-Content $cj -Raw | ConvertFrom-Json } catch { continue }
    if (-not ($j.exe -and $j.sha_main -and (Test-Path $j.exe))) { continue }
    # Binaire-équivalent ≠ commit-identique : un commit
    # scripts/docs change `rev-parse main` sans toucher le
    # moteur. On compare les CHEMINES qui produisent l'exe.
    git -C $refRepo diff --quiet $j.sha_main $sha -- crates/ Cargo.toml Cargo.lock
    if ($LASTEXITCODE -eq 0) { $exeOverride = $j.exe; break }
  }
  if ($exeOverride) {
    Write-Output "binaire réutilisé : $exeOverride (sha $sha déjà buildé — pas de rebuild)"
  } else {
    $leaf = Split-Path -Leaf $refRepo
    $wt = Join-Path (Split-Path -Parent $refRepo) "$leaf-vol$N-wt"
    git -C $refRepo worktree add --detach $wt $sha 2>&1 | ForEach-Object { Write-Output $_ }
    if ($LASTEXITCODE -ne 0) { Stop-VolClaim "worktree éphémère $wt impossible" }
    $buildRepo = $wt
    Write-Output "worktree éphémère : $wt @ $sha"
  }
}
Write-Claim @{ state = 'building'; worktree = $wt }
if (-not $NoBuild -and -not $exeOverride) {
  Write-Output "build : cargo build --release -p explorer-daemon ($buildRepo)"
  Push-Location $buildRepo
  try { cargo build --release -p explorer-daemon } finally { Pop-Location }
  if ($LASTEXITCODE -ne 0) { Stop-VolClaim 'build échoué' }
}

# --- 4/5. port + spawn ---
# Le numéro de vol est DÉJÀ arbitré atomiquement → le port par
# défaut en dérive (9385 + (N-1) mod 200) : pas de coordination
# partagée à respecter. Le sondage « port libre » reste un
# check-then-act face au bind du daemon (~1 s de fenêtre — course
# constatée : deux « en ligne » sur un même port, le perdant mort
# os error 10048 pendant que /api/status était répondu par le
# daemon VAINQUEUR). Après chaque spawn on vérifie donc que le pid
# du pidfile possède réellement le listener — défaite au bind =
# port suivant, jamais un vol zombie pointant un daemon voisin.
function Read-UsedPorts {
  $h = @{}
  foreach ($d in Get-ChildItem $vols -Directory -ErrorAction SilentlyContinue) {
    $cj = Join-Path $d.FullName 'CLAIMED.json'
    if ((Test-Path $cj) -and -not (Test-Path (Join-Path $d.FullName 'DONE'))) {
      try {
        $j = Get-Content $cj -Raw | ConvertFrom-Json
        if ($j.port) { $h[[int]$j.port] = $true }
      } catch { }
    }
  }
  return $h
}
function Test-PortFree([int]$p, [hashtable]$used) {
  if ($used[$p]) { return $false }
  try {
    $l = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Parse('127.0.0.1'), $p)
    $l.Start(); $l.Stop(); return $true
  } catch { return $false }
}
function Get-PortOwner([int]$p) {
  foreach ($line in (netstat -ano)) {
    if ($line -notmatch 'LISTENING') { continue }
    $t = $line -split '\s+' | Where-Object { $_ }
    if ($t.Count -ge 5 -and $t[1] -match ":(\d+)$" -and [int]$Matches[1] -eq $p) {
      return [int]$t[-1]
    }
  }
  return 0
}

$env:EXPLORER_VIEWPORT   = '1366x900'
$env:EXPLORER_MEM_DIAG   = '1'
$env:EXPLORER_PROFILE    = "vol:vol$N"
$env:EXPLORER_DATA       = $corpus
$env:EXPLORER_DEATH_LOG  = Join-Path $logdir 'explorer_daemon_death.log'
$env:EXPLORER_FLIGHT_LOG = Join-Path $logdir 'explorer_daemon_flight.log'

$forcedPort = $Port -gt 0
$dpid = 0; $exePath = ''; $spawned = $false
for ($try = 1; $try -le 4 -and -not $spawned; $try++) {
  if ($Port -eq 0) {
    $used = Read-UsedPorts
    $cand = 9385 + (($N - 1) % 200)
    if (-not (Test-PortFree $cand $used)) {
      $cand = 0
      for ($p = 9385; $p -lt 9600 -and $cand -eq 0; $p++) {
        if (Test-PortFree $p $used) { $cand = $p }
      }
    }
    if ($cand -eq 0) { Stop-VolClaim 'aucun port libre 9385-9599' }
    $Port = $cand
  }
  Write-Claim @{ state = 'spawning'; port = $Port }

  $exeArgs = if ($exeOverride) { @('-Exe', $exeOverride) } else { @() }
  $out = & pwsh -NoProfile -File (Join-Path $PSScriptRoot 'spawn_daemon.ps1') `
    -Port $Port -Repo $buildRepo -LogDir $logdir @exeArgs 2>&1
  $out | ForEach-Object { Write-Output $_ }
  $dpid = 0
  $m = $out | Select-String -Pattern 'daemon PID (\d+)' | Select-Object -First 1
  if ($m) { $dpid = [int]$m.Matches[0].Groups[1].Value }
  if (-not $exePath) {
    $exe = (($out | Select-String -Pattern '^exe=(\S+)') | Select-Object -First 1)
    $exePath = if ($exe) { $exe.Matches[0].Groups[1].Value } else { '' }
  }
  # Appartenance pid↔port — le seul arbitre qui compte (« en ligne »
  # peut être répondu par un daemon voisin ayant gagné le bind).
  $alive = $dpid -gt 0 -and `
    [bool](Get-Process -Id $dpid -ErrorAction SilentlyContinue)
  if ($alive -and (Get-PortOwner $Port) -eq $dpid) { $spawned = $true; break }
  Write-Output "port $Port : pid $dpid ne possède pas le listener — essai suivant"
  if ($alive) {
    $proc = Get-Process -Id $dpid -ErrorAction SilentlyContinue
    if ($proc.ProcessName -eq 'explorer-daemon') { taskkill /PID $dpid /F | Out-Null }
  }
  if ($forcedPort) { Stop-VolClaim "port forcé $Port perdu au bind" }
  $Port = 0
}
if (-not $spawned) { Stop-VolClaim "aucun bind gagné après les essais — voir $logdir" }

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
if ($wt) { Write-Output "worktree=$wt (fin de mission : scripts/vol_clean.ps1 -Vol $N -Apply)" }
Write-Output "profil=vol:vol$N logs=$logdir"
Write-Output "clôture : pwsh -File scripts/vol_close.ps1 -Vol $N"
