# This file is part of Explorer.
# Copyright (C) 2026 Laurent Geynet
# SPDX-License-Identifier: MIT OR Apache-2.0

<#
.SYNOPSIS
  Lance une sonde (shot, jprobe…) sous surveillance anti
  « mort silencieuse ».

.DESCRIPTION
  La faille découverte au vol élargi : `explorer-cli` peut
  mourir sans aucune trace (exit 1, stderr vide) — signature
  fail-fast / stack overflow. Ce wrapper garantit qu'une
  mort laisse TOUJOURS quelque chose :

    - stdout/stderr redirigés vers target/probe_watch/*.log
    - budget temps global (-TimeoutSec) : dépassement = kill
      contrôlé et rapporté, pas un processus qui brûle 900 s
    - à la mort ou au timeout : code de sortie, fin de stderr,
      état CPU/mémoire — et un minidump si `procdump` ou
      `createdump` est disponible sur le PATH
    - RUST_LOG est posé à `info` par défaut : si le binaire
      a un subscriber tracing, la dernière ligne du log =
      l'étape fatale

  Usage :
    pwsh -File scripts/probe_watch.ps1 -Exe target\debug\explorer-cli.exe `
        -ExeArgs jprobe,https://example.com -Tag jprobe-ex -TimeoutSec 600

  Code de sortie : celui de l'enfant, ou 124 sur timeout.
#>

param(
    [Parameter(Mandatory)][string]$Exe,
    [string[]]$ExeArgs = @(),
    [int]$TimeoutSec = 900,
    [string]$Tag = 'probe',
    [string]$OutDir = ''
)

$ErrorActionPreference = 'Continue'
$root = Split-Path -Parent (Split-Path -Parent $PSCommandPath)
if ($OutDir -eq '') { $OutDir = Join-Path $root 'target\probe_watch' }
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$base = Join-Path $OutDir "$Tag-$stamp"
$outLog = "$base.out.log"
$errLog = "$base.err.log"

if (-not $env:RUST_LOG) { $env:RUST_LOG = 'info' }

Write-Output "watch: $Exe $($ExeArgs -join ' ')"
Write-Output "watch: logs $base.{out,err}.log — budget ${TimeoutSec}s"

$proc = Start-Process -FilePath $Exe -ArgumentList $ExeArgs `
    -PassThru -WindowStyle Hidden `
    -RedirectStandardOutput $outLog -RedirectStandardError $errLog

$deadline = (Get-Date).AddSeconds($TimeoutSec)
$timedOut = $false
while (-not $proc.HasExited) {
    if ((Get-Date) -ge $deadline) {
        $timedOut = $true
        break
    }
    Start-Sleep -Seconds 2
    $proc.Refresh()
}

if ($timedOut) {
    Write-Output "watch: TIMEOUT ${TimeoutSec}s — arrêt contrôlé (pid $($proc.Id))"
    # Dernière trace possible avant le kill : dump si l'outil
    # existe (Sysinternals procdump, ou createdump du SDK .NET).
    $dumper = Get-Command procdump, procdump64, createdump `
        -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($dumper) {
        $dump = "$base.dmp"
        Write-Output "watch: dump via $($dumper.Name) -> $dump"
        & $dumper.Source -accepteula -ma $proc.Id $dump 2>&1 |
            Out-Null
    } else {
        Write-Output 'watch: pas de dumper sur le PATH ' +
            '(procdump/createdump) — pas de capture mémoire'
    }
    Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
    $code = 124
} else {
    $proc.Refresh()
    $code = $proc.ExitCode
    $cpu = [math]::Round($proc.TotalProcessorTime.TotalSeconds, 1)
    $mem = [math]::Round($proc.PeakWorkingSet64 / 1MB)
    Write-Output "watch: exit $code après ${cpu}s CPU, ${mem} Mo pic"
    if ($code -ne 0) {
        Write-Output "watch: sortie anormale — fin de stderr :"
        if (Test-Path $errLog) {
            Get-Content $errLog -Tail 20
        }
    }
}

exit $code
