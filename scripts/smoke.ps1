# This file is part of Explorer.
# Copyright (C) 2026 Laurent Geynet
# SPDX-License-Identifier: MIT OR Apache-2.0

# smoke.ps1 — test de fumee bout en bout (etape 40), tout en
# loopback, aucune sortie reseau externe :
#   1. serveur HTTP local (TcpListener 127.0.0.1:9381) servant
#      une page fixture
#   2. daemon (release si present, sinon build debug)
#   3. GET /api/status -> POST /api/tabs -> POST navigate ->
#      GET frame -> signature PNG verifiee
# Code de sortie non nul sur echec.
# Usage : pwsh -File scripts/smoke.ps1

$ErrorActionPreference = 'Stop'
Set-Location (Split-Path -Parent $PSScriptRoot)

$DocPort  = 9381
$ApiPort  = 9378
$ApiBase  = "http://127.0.0.1:$ApiPort/api"
$DocUrl   = "http://127.0.0.1:$DocPort/"

# --- 1. serveur de document local -----------------------------------------
$page = '<!doctype html><html><head><title>smoke</title></head>' +
        '<body><h1>smoke ok</h1><p>pipeline bout en bout</p></body></html>'

$server = {
    param($Port, $Body)
    $listener = [System.Net.Sockets.TcpListener]::new(
        [System.Net.IPAddress]::Loopback, $Port)
    $listener.Start()
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($Body)
    try {
        # Jusqu'a 8 connexions (chaque reponse ferme — keep-alive off).
        for ($i = 0; $i -lt 8; $i++) {
            $client = $listener.AcceptTcpClient()
            $stream = $client.GetStream()
            $buf = New-Object byte[] 8192
            [void]$stream.Read($buf, 0, $buf.Length)
            $head = "HTTP/1.1 200 OK`r`nContent-Type: text/html; charset=utf-8`r`n" +
                    "Content-Length: $($bytes.Length)`r`nConnection: close`r`n`r`n"
            $hb = [System.Text.Encoding]::ASCII.GetBytes($head)
            $stream.Write($hb, 0, $hb.Length)
            $stream.Write($bytes, 0, $bytes.Length)
            $client.Close()
        }
    } finally { $listener.Stop() }
}
$job = Start-Job -ScriptBlock $server -ArgumentList $DocPort, $page

# --- 2. daemon -------------------------------------------------------------
$daemon = 'target/release/explorer-daemon.exe'
if (-not (Test-Path $daemon)) {
    Write-Host "==> cargo build -p explorer-daemon (release absent)" -ForegroundColor Cyan
    cargo build -p explorer-daemon
    if ($LASTEXITCODE -ne 0) { throw "ECHEC : build daemon" }
    $daemon = 'target/debug/explorer-daemon.exe'
}

$env:EXPLORER_PORT = "$ApiPort"
$proc = Start-Process -FilePath (Resolve-Path $daemon) -PassThru `
    -WindowStyle Hidden
Remove-Item env:EXPLORER_PORT

try {
    # Attente du bind API (≤ 10 s).
    $up = $false
    foreach ($i in 1..50) {
        try {
            Invoke-RestMethod -Uri "$ApiBase/status" -TimeoutSec 1 | Out-Null
            $up = $true; break
        } catch { Start-Sleep -Milliseconds 200 }
    }
    if (-not $up) { throw "daemon injoignable sur $ApiBase" }
    Write-Host "==> status OK" -ForegroundColor Green

    $tab = Invoke-RestMethod -Method Post -Uri "$ApiBase/tabs"
    $id = $tab.id
    Write-Host "==> onglet $id cree" -ForegroundColor Green

    Invoke-RestMethod -Method Post -Uri "$ApiBase/tabs/$id/navigate" `
        -ContentType 'application/json' -Body "{`"url`":`"$DocUrl`"}" | Out-Null
    Write-Host "==> navigation $DocUrl OK" -ForegroundColor Green

    $png = Invoke-WebRequest -Uri "$ApiBase/tabs/$id/frame"
    $sig = [byte[]]($png.Content[0..7])
    $want = [byte[]](0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A)
    if (($sig -join ',') -ne ($want -join ',')) {
        throw "frame non-PNG (signature $($sig -join ','))"
    }
    Write-Host "==> frame PNG valide ($($png.Content.Length) octets)" -ForegroundColor Green
    Write-Host "SMOKE OK — pipeline E2E fonctionnel" -ForegroundColor Green
} finally {
    if ($proc -and -not $proc.HasExited) { Stop-Process -Id $proc.Id -Force }
    Stop-Job $job -ErrorAction SilentlyContinue | Out-Null
    Remove-Job $job -Force -ErrorAction SilentlyContinue | Out-Null
}
