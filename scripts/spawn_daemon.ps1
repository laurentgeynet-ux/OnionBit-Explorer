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
#     [-Profile default] [-Exe <chemin>] [-LogDir <dossier>]

param(
  [int]$Port = 9377,
  [string]$Profile = 'default',
  [string]$Exe = 'D:\cargo-target\OnionBit-Explorer\debug\explorer-daemon.exe',
  [string]$LogDir = 'D:\cargo-target\explorer-nav\daemon-logs'
)

New-Item -ItemType Directory -Force -Path $LogDir | Out-Null
$stdout = Join-Path $LogDir "daemon-$Port.stdout.log"
$stderr = Join-Path $LogDir "daemon-$Port.stderr.log"
$exitlog = Join-Path $LogDir "daemon-$Port.exit.log"

# Une ligne de commande cmd unique : `&` enchaîne l'écriture
# du code de sortie APRES la fin du daemon (cmd /c attend
# l'enfant — sauf kill de tout l'arbre, d'où l'absence =
# signature). `!ERRORLEVEL!`/`!DATE!`/`!TIME!` sont retardés
# par /v:on pour être lus au moment de la mort.
$inner = "`"$Exe`" --port $Port --profile `"$Profile`" " +
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

# Vérification : le port doit répondre dans les ~5 s.
$deadline = (Get-Date).AddSeconds(5)
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
if ($ok) {
  Write-Output "daemon $Port en ligne"
} else {
  Write-Warning "daemon $Port muet — voir $stderr"
}
