# This file is part of Explorer.
# Copyright (C) 2026 Laurent Geynet
# SPDX-License-Identifier: MIT OR Apache-2.0

# smoke.ps1 — test de fumee bout en bout (etapes 40 puis 59),
# tout en loopback, aucune sortie reseau externe :
#   1. serveur HTTP local (TcpListener 127.0.0.1:9381) routant
#      des pages fixtures par chemin
#   2. daemon (release si present, sinon build debug)
#   3. status -> tab -> navigate -> frame PNG, puis les faits
#      0.2 : clic lien -> navigation, back, onclick= -> mutation
#      rendue, scroll -> scroll_y, <video> -> offre /media,
#      find -> compte des occurrences
# Code de sortie non nul sur echec.
# Usage : pwsh -File scripts/smoke.ps1

$ErrorActionPreference = 'Stop'
Set-Location (Split-Path -Parent $PSScriptRoot)

$DocPort  = 9381
$ApiPort  = 9378
$ApiBase  = "http://127.0.0.1:$ApiPort/api"
$DocBase  = "http://127.0.0.1:$DocPort"

# --- 1. serveur de documents local, routage par chemin ---------------------
# Chaque page est choisie pour isoler un fait teste. Elements
# cliquables EN TETE de body (marge 8 px par defaut) : le clic
# (20,16) tombe dans la premiere boite de ligne.
$server = {
    param($Port)
    # Map construite ICI : un hashtable passé en -ArgumentList
    # traverse la sérialisation CLIXML du job — source de
    # flakiness mesurée (reset 10054 aléatoire).
    $Pages = @{
        '/'       = '<!doctype html><html><head><title>smoke</title></head><body>' +
                    '<a href="/b">lien b</a><h1>smoke ok</h1>' +
                    '<p>pipeline bout en bout</p></body></html>'
        '/b'      = '<!doctype html><html><head><title>page b</title></head>' +
                    '<body><p>arrivee</p></body></html>'
        '/ev'     = '<!doctype html><html><body>' +
                    '<div id="d" onclick="this.textContent=''HIT''">avant</div>' +
                    '</body></html>'
        '/media'  = '<!doctype html><html><body>' +
                    '<video src="/v.mp4" controls width="200" height="100"></video>' +
                    '</body></html>'
        '/scroll' = '<!doctype html><html><body>' +
                    ('<div>ligne</div>' * 300) + '</body></html>'
    }
    $listener = [System.Net.Sockets.TcpListener]::new(
        [System.Net.IPAddress]::Loopback, $Port)
    $listener.Start()
    try {
        # Chaque reponse ferme (keep-alive off) — le compte
        # couvre navigations + retries de subresources.
        # try/catch PAR CONNEXION : un client qui aborte avant
        # la lecture (sonde TCP, keepalive) ne doit pas tuer
        # la boucle — sinon RST 10054 en cascade.
        for ($i = 0; $i -lt 64; $i++) {
            try {
                $client = $listener.AcceptTcpClient()
                $stream = $client.GetStream()
                $buf = New-Object byte[] 8192
                $n = $stream.Read($buf, 0, $buf.Length)
                $req = [System.Text.Encoding]::ASCII.GetString($buf, 0, $n)
                $path = '/'
                if ($req -match '^GET\s+(\S+)\s+HTTP') { $path = $Matches[1] }
                if ($Pages.ContainsKey($path)) {
                    $body = [System.Text.Encoding]::UTF8.GetBytes($Pages[$path])
                    $head = "HTTP/1.1 200 OK`r`nContent-Type: text/html; charset=utf-8`r`n" +
                            "Content-Length: $($body.Length)`r`nConnection: close`r`n`r`n"
                } else {
                    $body = [System.Text.Encoding]::UTF8.GetBytes('not found')
                    $head = "HTTP/1.1 404 Not Found`r`nContent-Type: text/plain`r`n" +
                            "Content-Length: $($body.Length)`r`nConnection: close`r`n`r`n"
                }
                $hb = [System.Text.Encoding]::ASCII.GetBytes($head)
                $stream.Write($hb, 0, $hb.Length)
                $stream.Write($body, 0, $body.Length)
                $client.Close()
            } catch { continue }
        }
    } finally { $listener.Stop() }
}
$job = Start-Job -ScriptBlock $server -ArgumentList $DocPort

# Le job met ~1-2 s a binder (nouveau process pwsh) — naviguer
# avant le listen donne un RST (mesure : 10054 flaky).
foreach ($i in 1..50) {
    $c = New-Object System.Net.Sockets.TcpClient
    try {
        $c.Connect('127.0.0.1', $DocPort); $c.Close(); break
    } catch { Start-Sleep -Milliseconds 200 }
    finally { $c.Dispose() }
}

# --- 2. daemon -------------------------------------------------------------
# `target_directory` reel via `cargo metadata` : `.cargo/
# config.toml` (et `CARGO_TARGET_DIR`) deportent `target/` —
# un chemin en dur `target/release` raterait le binaire.
$meta = cargo metadata --no-deps --format-version 1 | ConvertFrom-Json
$daemon = Join-Path $meta.target_directory 'release\explorer-daemon.exe'
if (-not (Test-Path $daemon)) {
    Write-Host "==> cargo build -p explorer-daemon (release absent)" -ForegroundColor Cyan
    cargo build -p explorer-daemon
    if ($LASTEXITCODE -ne 0) { throw "ECHEC : build daemon" }
    $daemon = Join-Path $meta.target_directory 'debug\explorer-daemon.exe'
}

$env:EXPLORER_PORT = "$ApiPort"
$proc = Start-Process -FilePath (Resolve-Path $daemon) -PassThru `
    -WindowStyle Hidden
Remove-Item env:EXPLORER_PORT

# --- helpers ----------------------------------------------------------------
function Wait-For([scriptblock]$Probe, [string]$What, [int]$Tries = 50) {
    foreach ($i in 1..$Tries) {
        $r = & $Probe
        if ($null -ne $r -and $r -ne $false) { return $r }
        Start-Sleep -Milliseconds 200
    }
    throw "timeout : $What"
}

function Get-Frame($Id) {
    # 204 No Content tant que la navigation n'a pas paint
    # (la nav est posee, la pipeline tourne en async) → $null.
    try { (Invoke-WebRequest -Uri "$ApiBase/tabs/$Id/frame" -TimeoutSec 5).Content }
    catch { $null }
}

function Assert-Png([byte[]]$Bytes) {
    $want = [byte[]](0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A)
    if (($Bytes[0..7] -join ',') -ne ($want -join ',')) {
        throw "frame non-PNG"
    }
}

function Send-Input($Id, [hashtable]$Ev) {
    Invoke-RestMethod -Method Post -Uri "$ApiBase/tabs/$Id/input" `
        -ContentType 'application/json' -Body ($Ev | ConvertTo-Json)
}

function Send-Navigate($Id, [string]$Path) {
    Invoke-RestMethod -Method Post -Uri "$ApiBase/tabs/$Id/navigate" `
        -ContentType 'application/json' `
        -Body "{`"url`":`"$DocBase$Path`"}" | Out-Null
}

function Get-TabInfo($Id) {
    (Invoke-RestMethod -Uri "$ApiBase/tabs" -TimeoutSec 5).tabs |
        Where-Object { $_.id -eq $Id }
}

try {
    Wait-For { try { Invoke-RestMethod -Uri "$ApiBase/status" -TimeoutSec 1 | Out-Null; $true } catch { $false } } 'daemon up'
    Write-Host "==> status OK" -ForegroundColor Green

    $tab = Invoke-RestMethod -Method Post -Uri "$ApiBase/tabs"
    $id = $tab.id
    Write-Host "==> onglet $id cree" -ForegroundColor Green

    # -- JS-C45 : handler inline onclick= compile et rendu -------------
    Send-Navigate $id '/ev'
    $f1 = Wait-For { $f = Get-Frame $id; if ($f -and $f.Length -gt 0) { $f } else { $false } } 'frame /ev'
    Assert-Png $f1
    $sha = [System.Security.Cryptography.SHA256]::Create()
    $h1 = [Convert]::ToBase64String($sha.ComputeHash([byte[]]$f1))
    $out = Send-Input $id @{ type = 'click'; x = 20; y = 16 }
    $changed = Wait-For {
        $f2 = Get-Frame $id
        if ($f2 -and $f2.Length -gt 0 -and
            [Convert]::ToBase64String($sha.ComputeHash([byte[]]$f2)) -ne $h1) { $f2 } else { $false }
    } 'frame mute apres onclick'
    Write-Host "==> onclick= compile : mutation rendue ($($f1.Length) -> $($changed.Length) octets, outcome=$($out.result))" -ForegroundColor Green

    # -- lien : clic -> navigation, back retourne -----------------------
    Send-Navigate $id '/'
    $null = Wait-For { $f = Get-Frame $id; if ($f -and $f.Length -gt 0) { $true } else { $false } } 'frame /'
    $out = Send-Input $id @{ type = 'click'; x = 20; y = 16 }
    $info = Wait-For {
        $t = Get-TabInfo $id
        if ($t.url -eq "$DocBase/b") { $t } else { $false }
    } 'navigation par clic de lien'
    Write-Host "==> clic lien -> $($info.url) (outcome=$($out.result))" -ForegroundColor Green

    Invoke-RestMethod -Method Post -Uri "$ApiBase/tabs/$id/back" | Out-Null
    $null = Wait-For {
        $t = Get-TabInfo $id
        if ($t.url -eq "$DocBase/") { $t } else { $false }
    } 'back'
    Write-Host "==> back -> /" -ForegroundColor Green

    # -- find : occurrences comptees -----------------------------------
    $found = Invoke-RestMethod -Method Post -Uri "$ApiBase/tabs/$id/find" `
        -ContentType 'application/json' -Body '{"query":"pipeline"}'
    if ($found.count -lt 1) { throw "find : 0 occurrence de 'pipeline'" }
    Write-Host "==> find 'pipeline' : $($found.count) occurrence(s)" -ForegroundColor Green

    # -- scroll : outcome scrolled + scroll_y du contrat 71d ------------
    Send-Navigate $id '/scroll'
    $null = Wait-For { $f = Get-Frame $id; if ($f -and $f.Length -gt 0) { $true } else { $false } } 'frame /scroll'
    $out = Send-Input $id @{ type = 'scroll'; dy = 300 }
    if ($out.result -ne 'scrolled' -or $out.scroll_y -le 0) {
        throw "scroll : outcome inattendu $($out | ConvertTo-Json -Compress)"
    }
    Write-Host "==> scroll : result=$($out.result) scroll_y=$($out.scroll_y)" -ForegroundColor Green

    # -- media : <video> offerte via GET /media (seam 71b) --------------
    Send-Navigate $id '/media'
    $media = Wait-For {
        $m = (Invoke-RestMethod -Uri "$ApiBase/tabs/$id/media" -TimeoutSec 5).media
        if ($m.Count -ge 1) { $m } else { $false }
    } 'offre media'
    if ($media[0].kind -ne 'video') { throw "media kind inattendu : $($media[0].kind)" }
    Write-Host "==> media offer : kind=$($media[0].kind) rect=$($media[0].rect.w)x$($media[0].rect.h)" -ForegroundColor Green

    Write-Host "SMOKE OK — nav, lien, back, onclick=, scroll_y, media, find" -ForegroundColor Green
} finally {
    if ($proc -and -not $proc.HasExited) { Stop-Process -Id $proc.Id -Force }
    Stop-Job $job -ErrorAction SilentlyContinue | Out-Null
    Remove-Job $job -Force -ErrorAction SilentlyContinue | Out-Null
}
