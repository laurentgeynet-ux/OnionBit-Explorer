# This file is part of Explorer.
# Copyright (C) 2026 Laurent Geynet
# SPDX-License-Identifier: MIT OR Apache-2.0

# spawn_daemon.ps1 — lance explorer-daemon **hors du job de
# session** (WMI `Win32_Process.Create` : le process survit à
# un crash de Devin/de la session appelante) et **fenêtre
# cachée** (`SW_HIDE` : le cmd.exe watchdog n'apparaît pas
# sur le bureau — sa fermeture manuelle tuait tout l'arbre
# sans laisser de trace, mécanisme des morts silencieuses
# passées).
#
# Capture des trois signatures de mort :
#
#   - stdout → daemon-<port>.stdout.log
#   - stderr → daemon-<port>.stderr.log
#   - code de sortie → daemon-<port>.exit.log
#
# Le `cmd /c` parent est le chien de garde : quand le daemon
# se termine, `& echo EXIT:!ERRORLEVEL!` écrit le code
# (`/v:on` retarde l'expansion — `%ERRORLEVEL%` s'évaluerait
# au parse et rendrait toujours 0). 0 = sortie propre,
# -1073740791/0xC0000409 = fail-fast ou stack overflow,
# -1073741819/0xC0000005 = accès mémoire. Un exit.log ABSENT
# après disparition = kill externe (teardown de la machine,
# fermeture de session) — la signature est aussi dans
# l'absence.
#
# Usage :
#   pwsh -File scripts/spawn_daemon.ps1 [-Port 9377]
#     [-Profile <dir>] [-Exe <chemin>] [-LogDir <dossier>]
#   -Exe défaut : <target-dir>[/<triple>]\release\explorer-daemon.exe
#   (résolution cargo via `cargo metadata` — repli debug) ; -Profile omis =
#   EXPLORER_PROFILE de l'appelant (hérité via WMI).

param(
  [int]$Port = 9377,
  # --profile DIR du daemon. '' = non passé : EXPLORER_PROFILE
  # est propagé via les `set` cuits (voir plus bas — WMI
  # n'hérite PAS l'env de l'appelant) ; un -Profile explicite
  # l'emporte (même règle que le binaire : --profile > env).
  [string]$Profile = '',
  [string]$Exe = '',
  [string]$LogDir = ''
)

# Résolution du binaire : -Exe explicite, sinon la canonique
# cargo via `cargo metadata` (CARGO_TARGET_DIR > configs projet
# et globale > .\target — la chaîne entière, y compris la
# config utilisateur hors dépôt). release préféré à debug.
# Sous un `cargo build --target <triple>` ou un `build.target`
# explicite (config globale), le binaire vit dans
# `<target>\<triple>\<prof>\` — la racine `<target>\<prof>\`
# est alors vide et l'ancien script échouait « introuvable ».
if ($Exe -eq '') {
  $repo = Split-Path -Parent $PSScriptRoot
  Push-Location $repo
  try {
    $target = (cargo metadata --no-deps --format-version 1 |
      ConvertFrom-Json).target_directory
  } finally { Pop-Location }
  # Triple hôte (`x86_64-pc-windows-msvc`…) — rustc absent =
  # chaîne vide, le repli glob plus bas couvre alors.
  $triple = ''
  try {
    $v = & rustc -vV 2>$null | Select-String -Pattern '^host: (\S+)'
    if ($v) { $triple = $v.Matches[0].Groups[1].Value }
  } catch { }
  $cands = @(
    (Join-Path $target 'release\explorer-daemon.exe')
    $(if ($triple) { Join-Path $target "$triple\release\explorer-daemon.exe" })
    (Join-Path $target 'debug\explorer-daemon.exe')
    $(if ($triple) { Join-Path $target "$triple\debug\explorer-daemon.exe" })
  ) | Where-Object { $_ }
  foreach ($cand in $cands) {
    if (Test-Path $cand) { $Exe = $cand; break }
  }
  # Repli borné : tout `<target>\<triple>\<prof>\explorer-
  # daemon.exe` — couvre un triple différent de `rustc -vV`
  # (cross-build) ou un rustc absent du PATH.
  if ($Exe -eq '') {
    $hit = Get-ChildItem -Path $target -Recurse -Depth 2 `
      -Filter 'explorer-daemon.exe' -ErrorAction SilentlyContinue |
      Sort-Object `
        { if ($_.FullName -match '\\release\\') { 0 } else { 1 } }, `
        { -$_.LastWriteTime.Ticks } |
      Select-Object -First 1
    if ($hit) { $Exe = $hit.FullName }
  }
}
if (-not (Test-Path $Exe)) {
  Write-Error "explorer-daemon.exe introuvable (cargo build ou -Exe)"
  exit 1
}
if ($LogDir -eq '') {
  $LogDir = Join-Path (Split-Path -Parent $PSScriptRoot) 'target\daemon-logs'
}

New-Item -ItemType Directory -Force -Path $LogDir | Out-Null
$stdout = Join-Path $LogDir "daemon-$Port.stdout.log"
$stderr = Join-Path $LogDir "daemon-$Port.stderr.log"
$exitlog = Join-Path $LogDir "daemon-$Port.exit.log"

# Une ligne de commande cmd unique : `&` enchaîne l'écriture
# du code de sortie APRES la fin du daemon (cmd /c attend
# l'enfant — sauf kill de tout l'arbre, d'où l'absence =
# signature). `!ERRORLEVEL!`/`!DATE!`/`!TIME!` sont retardés
# par /v:on pour être lus au moment de la mort.
$profArg = if ($Profile -ne '') { " --profile `"$Profile`"" } else { '' }

# Win32_Process.Create tourne sous l'environnement du
# provider WMI — l'env de l'appelant ne traverse PAS
# (mesuré au vol 636 : EXPLORER_PROFILE/EXPLORER_MEDIA_CODECS
# ignorés → profil `default` chargé). On cuit donc les
# EXPLORER_* de l'appelant dans la ligne cmd (`set "K=V" &`
# séquentiels) — propagation garantie quel que soit
# l'appelant. `"` est évacué des valeurs (un guillemet
# casserait la ligne ; les valeurs Explorer n'en portent pas).
$sets = ((Get-ChildItem env: |
  Where-Object { $_.Name -like 'EXPLORER_*' } |
  ForEach-Object { 'set "{0}={1}" & ' -f $_.Name, ($_.Value -replace '"', '') }) -join '')
$inner = $sets + "`"$Exe`" --port $Port$profArg " +
  "> `"$stdout`" 2> `"$stderr`" " +
  "& <nul set /p _=EXIT:!ERRORLEVEL! > `"$exitlog`" " +
  "& echo !DATE! !TIME! >> `"$exitlog`""

# `ShowWindow = SW_HIDE` : la fenêtre console du cmd
# watchdog existe mais est invisible — rien à fermer sur le
# bureau (les morts silencieuses passées venaient de sa
# fermeture manuelle). `CREATE_NO_WINDOW` est refusé par
# WMI (`ReturnValue 21` — il alloue toujours une console) :
# SW_HIDE est le retrait maximum accessible ici.
$startup = New-CimInstance -ClassName Win32_ProcessStartup `
  -ClientOnly `
  -Property @{ ShowWindow = [uint16]0 }

$proc = Invoke-CimMethod -ClassName Win32_Process `
  -MethodName Create `
  -Arguments @{
    CommandLine = "cmd.exe /v:on /c `"$inner`""
    ProcessStartupInformation = $startup
  }
if ($proc.ReturnValue -ne 0) {
  Write-Error "Win32_Process.Create a échoué : $($proc.ReturnValue)"
  exit 1
}
Write-Output "cmd watchdog PID $($proc.ProcessId) (daemon enfant)"

# Vérification : le port doit répondre dans les ~20 s — le
# premier boot scanne les fontes système (~7 s mesurés) et
# charge le profil ; 5 s coupaient en « muet » un daemon sain.
$deadline = (Get-Date).AddSeconds(20)
$ok = $false
while ((Get-Date) -lt $deadline -and -not $ok) {
  Start-Sleep -Milliseconds 400
  try {
    $null = Invoke-WebRequest `
      -Uri "http://127.0.0.1:$Port/api/status" `
      -UseBasicParsing -TimeoutSec 2
    $ok = $true
  } catch {
    $ok = $false
  }
}

# Les chiens de garde du vol (vol_joblimit -TargetPid,
# vol_memwatch -DaemonPid) veulent le PID du daemon, pas
# celui du cmd watchdog — on résout l'enfant explorer-daemon.
$dpid = (Get-CimInstance Win32_Process `
  -Filter "ParentProcessId = $($proc.ProcessId)" `
  | Where-Object { $_.Name -eq 'explorer-daemon.exe' } `
  | Select-Object -First 1 -ExpandProperty ProcessId)
if ($dpid) { Write-Output "daemon PID $dpid" }

if ($ok) {
  Write-Output "daemon $Port en ligne"
} else {
  Write-Warning "daemon $Port muet — voir $stderr"
}
