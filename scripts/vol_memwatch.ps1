# This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
# SPDX-License-Identifier: MIT OR Apache-2.0
<#
.SYNOPSIS
  Harnais mémoire pour daemon de vol — taskkill au-delà du seuil.

.DESCRIPTION
  Un daemon wedged a déjà monté à 7 Go et déstabilisé la station
  (overflow RAM + swap). Ce script surveille un PID de daemon :
  toutes les -IntervalSec secondes, si WorkingSet64 > -MaxMB,
  il tue le process PAR SON PID (jamais taskkill /IM — les daemons
  voisins appartiennent à d'autres sessions) et écrit une ligne
  dans le journal. S'arrête quand le process disparaît.

.EXAMPLE
  pwsh -File scripts/vol_memwatch.ps1 -DaemonPid 3572 -MaxMB 5120
  # en tâche de fond depuis un shell bash de vol :
  #   pwsh -File scripts/vol_memwatch.ps1 -DaemonPid $PID -Log _volN_watch.log &
#>
param(
  [Parameter(Mandatory=$true)][int]$DaemonPid,
  [int]$MaxMB = 5120,
  [int]$IntervalSec = 15,
  [string]$Log = "_vol_memwatch.log"
)

"watch start pid=$DaemonPid max=$MaxMB Mo interval=${IntervalSec}s" |
  Out-File -Append $Log
while ($true) {
  $p = Get-Process -Id $DaemonPid -ErrorAction SilentlyContinue
  if (-not $p) {
    "watch end: pid $DaemonPid disparu" | Out-File -Append $Log
    break
  }
  $mb = [math]::Round($p.WorkingSet64 / 1MB)
  if ($mb -gt $MaxMB) {
    "$(Get-Date -Format o) WATCHDOG: pid $DaemonPid ${mb} Mo > $MaxMB — kill" |
      Out-File -Append $Log
    Stop-Process -Id $DaemonPid -Force
    break
  }
  "$(Get-Date -Format o) pid=$DaemonPid ${mb} Mo" | Out-File -Append $Log
  Start-Sleep -Seconds $IntervalSec
}
