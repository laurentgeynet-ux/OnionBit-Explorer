# This file is part of Explorer.
# Copyright (C) 2026 Laurent Geynet
# SPDX-License-Identifier: MIT OR Apache-2.0

# daemon_watch.ps1 — veille mémoire (roadmap NAV, item « veille »).
#
# Protocole : sur un daemon DÉDIÉ (pas celui du test humain), répéter
# le cycle qui accompagnait la disparition du 22/10 —
#   onglet → navigate wikipedia → tempête de mousemove → back →
#   DELETE → RSS — et poller la mémoire à chaque cycle.
# Critère : mémoire engagée croissante sans borne = repro fuite ;
# process mort = repro mort ; plat sur fenêtre longue =
# exonération progressive. La colonne log `ws/priv` (Mio) :
# WorkingSet64 est rogné par Windows sous pression (creux =
# trims OS), PrivateMemorySize64 est la métrique fuite.
#
# Usage : pwsh -File scripts/daemon_watch.ps1 [-Port 9382] [-Cycles 20]
# Log CSV : <LogDir>\watch-<port>.csv — colonnes
#   heure, cycle, rss_mio, etat

param(
    [int]$Port = 9382,
    [int]$Cycles = 20,
    [int]$ProbesPerCycle = 40,
    [string]$Site = 'https://en.wikipedia.org/',
    # 2e document par cycle : une entrée d'historique réelle
    # pour que `back` restaure (sinon 502 EmptyDocument —
    # attendu, pas un bug).
    [string]$BackSite = 'https://example.com/',
    # PID de l'app Flutter à mesurer en parallèle (M-H1 #5 :
    # explorer_app 3,7→4,5 Go/10 min — la veille couvre les
    # DEUX processus). Vide = daemon seul.
    [int]$AppPid = 0,
    [string]$LogDir = "$PSScriptRoot\..\target\watch"
)

$ErrorActionPreference = 'Continue'
$base = "http://127.0.0.1:$Port/api"
New-Item -ItemType Directory -Force $LogDir | Out-Null
$csv = Join-Path $LogDir "watch-$Port.csv"
if (-not (Test-Path $csv)) {
    'heure;cycle;ws_priv_mio;app_ws_priv_mio;etat' |
        Out-File -Encoding utf8 $csv
}

function Get-DaemonPid {
    try {
        (Get-NetTCPConnection -LocalPort $Port -State Listen `
            -ErrorAction Stop | Select-Object -First 1).OwningProcess
    } catch { $null }
}

function Get-MemMio([int]$ProcId) {
    # WorkingSet64 (RSS) EST rogné par Windows sous pression
    # mémoire — les creux du CSV sont des trims OS, pas des
    # libérations. PrivateMemorySize64 (octets engagés) est la
    # métrique fuite : elle ne retombe qu'à une vraie
    # libération.
    try {
        $p = Get-Process -Id $ProcId -ErrorAction Stop
        '{0}/{1}' -f [math]::Round($p.WorkingSet64 / 1MB, 1),
            [math]::Round($p.PrivateMemorySize64 / 1MB, 1)
    } catch { $null }
}

function Write-Row([int]$Cycle, $Mem, [string]$Etat) {
    $app = if ($AppPid) { Get-MemMio $AppPid } else { '' }
    '{0};{1};{2};{3};{4}' -f (Get-Date -Format 'HH:mm:ss'), $Cycle,
        $Mem, $app, $Etat | Out-File -Append -Encoding utf8 $csv
}

Write-Host "watch : port $Port, $Cycles cycles x $ProbesPerCycle sondes -> $csv"

for ($cycle = 1; $cycle -le $Cycles; $cycle++) {
    $pid0 = Get-DaemonPid
    if (-not $pid0) {
        Write-Row $cycle '' 'MORT-avant-cycle'
        Write-Host "cycle $cycle : daemon MORT (aucun listener $Port)"
        break
    }

    $id = $null
    try {
        $tab = Invoke-RestMethod -Method Post -Uri "$base/tabs" -TimeoutSec 30
        $id = $tab.id
        Invoke-RestMethod -Method Post -Uri "$base/tabs/$id/navigate" `
            -ContentType 'application/json' `
            -Body (@{ url = $Site } | ConvertTo-Json) `
            -TimeoutSec 240 | Out-Null
        # Deuxième document : le `back` qui suit a une vraie
        # cible d'historique (le cycle nav→nav→back→DELETE est
        # le motif de la disparition du 22/10).
        Invoke-RestMethod -Method Post -Uri "$base/tabs/$id/navigate" `
            -ContentType 'application/json' `
            -Body (@{ url = $BackSite } | ConvertTo-Json) `
            -TimeoutSec 240 | Out-Null

        # Tempête de sondes : le motif observé lors de la disparition.
        for ($i = 0; $i -lt $ProbesPerCycle; $i++) {
            $x = 8 + ($i % 10) * 120
            $y = 8 + [math]::Floor($i / 10) * 150
            Invoke-RestMethod -Method Post -Uri "$base/tabs/$id/input" `
                -ContentType 'application/json' `
                -Body (@{ type = 'mousemove'; x = $x; y = $y } |
                    ConvertTo-Json) -TimeoutSec 30 | Out-Null
        }

        Invoke-RestMethod -Method Post -Uri "$base/tabs/$id/back" `
            -TimeoutSec 60 | Out-Null
    } catch {
        # Un daemon mort rend ses appels en échec — le distinguer
        # d'une simple erreur applicative via le listener.
        if (-not (Get-DaemonPid)) {
            Write-Row $cycle '' 'MORT-pendant-cycle'
            Write-Host "cycle $cycle : daemon MORT pendant le cycle"
            break
        }
        Write-Row $cycle (Get-MemMio $pid0) "erreur-app:$($_.Exception.Message)"
    } finally {
        if ($id) {
            try {
                Invoke-RestMethod -Method Delete `
                    -Uri "$base/tabs/$id" -TimeoutSec 30 | Out-Null
            } catch { }
        }
    }

    $mem = Get-MemMio $pid0
    Write-Row $cycle $mem 'ok'
    Write-Host ("cycle {0}/{1} : memo {2} Mio" -f $cycle, $Cycles, $mem)
}

Write-Host 'watch : fin — voir CSV pour la tendance.'
