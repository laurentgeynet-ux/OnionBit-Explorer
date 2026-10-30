# This file is part of Explorer.
# Copyright (C) 2026 Laurent Geynet
# SPDX-License-Identifier: MIT OR Apache-2.0

# build_dist.ps1 — Assemble `dist\Explorer\` : bundle portable.
#
# Usage :
#   pwsh -File scripts\build_dist.ps1
#   pwsh -File scripts\build_dist.ps1 -SkipCheck
#   pwsh -File scripts\build_dist.ps1 -Profile debug
#   pwsh -File scripts\build_dist.ps1 -ZipRelease
#                                                     (+ dist\Explorer-<ver>-windows-x64.zip)
#
# Layout (meme architecture de dossiers que le bundle portable
# OnionBit — les deux produits cohabitent sur un meme support
# amovible ou vivent independamment) :
#
#   dist\Explorer\Explorer.exe        lanceur GUI -> windows\explorer_app.exe
#   dist\Explorer\Explorer.portable   marqueur : state/ vit a la racine
#                                     du bundle
#   dist\Explorer\build-manifest.json version, commit, rustc, date UTC
#   dist\Explorer\windows\            payload : explorer_app.exe +
#                                     *.dll + data\ + explorer_ffi.dll
#   dist\Explorer\state\              cree au premier lancement (profil :
#                                     historique, cookies, cache,
#                                     downloads, logs) — JAMAIS efface
#                                     par ce script.
#
# Le marqueur est lu par le moteur lui-meme (explorer-core::paths::
# find_portable_root remonte depuis l'exe) — le lanceur ne fait que
# resoudre `windows\explorer_app.exe` relativement a sa position.

param(
    [switch]$SkipCheck,
    [ValidateSet("release", "debug")]
    [string]$Profile = "release",
    [switch]$ZipRelease
)

$ErrorActionPreference = "Stop"
$root       = Split-Path -Parent $PSScriptRoot
$app        = Join-Path $root "app"
$dist       = Join-Path $root "dist"
$bundleRoot = Join-Path $dist "Explorer"
$osDir      = Join-Path $bundleRoot "windows"
# CARGO_TARGET_DIR (worktrees paralleles) a priorite sur le
# target/ epingle par .cargo\config.toml.
$cargoTarget = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $root "target" }

Push-Location $root
try {
    # -- 1) Moteur FFI + lanceur ---------------------------------------
    if (-not $SkipCheck) {
        Write-Host "== cargo check pre-build ==" -ForegroundColor Cyan
        cargo check --workspace --all-targets
    }
    Write-Host "== cargo build --profile $Profile (ffi + launcher) ==" -ForegroundColor Cyan
    $cargoProfile = if ($Profile -eq "release") { "release" } else { "dev" }
    cargo build --profile $cargoProfile -p explorer-ffi -p explorer-launcher
    if ($LASTEXITCODE -ne 0) { throw "cargo build a echoue ($LASTEXITCODE)" }

    # -- 2) Application Flutter Windows --------------------------------
    Write-Host "== flutter build windows ($Profile) ==" -ForegroundColor Cyan
    Push-Location $app
    try {
        flutter pub get | Out-Null
        $flag = if ($Profile -eq "release") { "--release" } else { "--debug" }
        # Mode moteur fige : in-process seul — `auto` tomberait en
        # REST muet vers un daemon absent du bundle.
        flutter build windows $flag --dart-define=EXPLORER_ENGINE=ffi
        if ($LASTEXITCODE -ne 0) { throw "flutter build windows a echoue ($LASTEXITCODE)" }
    } finally {
        Pop-Location
    }

    # -- 3) Assemblage `dist\Explorer\` ---------------------------------
    $cargoOut   = Join-Path $cargoTarget $Profile
    $flutterOut = Join-Path $app "build\windows\x64\runner\$(@{$true='Release';$false='Debug'}[$Profile -eq 'release'])"
    if (-not (Test-Path "$cargoOut\explorer-launcher.exe")) {
        throw "explorer-launcher.exe introuvable dans $cargoOut"
    }
    if (-not (Test-Path "$cargoOut\explorer_ffi.dll")) {
        throw "explorer_ffi.dll introuvable dans $cargoOut"
    }
    if (-not (Test-Path "$flutterOut\explorer_app.exe")) {
        throw "explorer_app.exe introuvable dans $flutterOut"
    }

    New-Item -ItemType Directory -Force -Path $osDir | Out-Null

    # Lanceur -> Explorer.exe a la racine du bundle.
    Copy-Item (Join-Path $cargoOut "explorer-launcher.exe") `
        -Destination (Join-Path $bundleRoot "Explorer.exe") -Force

    # Marqueur portable : sa presence seule commande le layout
    # (find_portable_root remonte depuis l'exe jusqu'au marqueur).
    $marker = Join-Path $bundleRoot "Explorer.portable"
    "Bundle portable Explorer : state/ vit a cote de ce marqueur.`n" +
        "Ne pas supprimer — deplacable tel quel sur cle USB ou disque externe." |
        Set-Content $marker -Encoding ascii

    # Payload : exe Flutter + DLLs + data\ ; la DLL moteur compilee
    # ici est deposee a cote — jamais une dll perimee adjacente
    # (etape 119 : en mode auto elle gagnait la sonde tryLoad).
    Copy-Item (Join-Path $flutterOut "*") -Destination $osDir -Recurse -Force
    Copy-Item (Join-Path $cargoOut "explorer_ffi.dll") -Destination $osDir -Force

    # -- 4) Manifest ---------------------------------------------------
    $manifest = @{
        profile   = $Profile
        engine    = "ffi"
        ui        = "explorer_app (Flutter windows $Profile)"
        commit    = (git rev-parse --short HEAD 2>$null)
        rustc     = (rustc -V)
        built_utc = (Get-Date).ToUniversalTime().ToString("o")
        layout    = "portable (Explorer.portable + windows/)"
    }
    $manifest | ConvertTo-Json | Set-Content (Join-Path $bundleRoot "build-manifest.json")

    # -- 5) Zip de release (optionnel) ----------------------------------
    if ($ZipRelease) {
        Write-Host "== bundle release Explorer-<ver>-windows-x64 ==" -ForegroundColor Cyan
        $ver     = (cargo pkgid -p explorer-ffi).Split('#')[-1]
        $bundle  = Join-Path $dist "Explorer-$ver-windows-x64"
        $zipPath = "$bundle.zip"
        if (Test-Path $bundle)  { Remove-Item $bundle -Recurse -Force }
        if (Test-Path $zipPath) { Remove-Item $zipPath -Force }
        New-Item -ItemType Directory -Force -Path $bundle | Out-Null

        # Payload = contenu du bundle — jamais l'etat utilisateur ni
        # les artefacts de release eux-memes.
        Get-ChildItem $bundleRoot |
            Where-Object { $_.Name -ne 'state' } |
            Copy-Item -Destination $bundle -Recurse -Force

        Get-ChildItem (Join-Path $root 'LICENSE*') -File -ErrorAction SilentlyContinue |
            Copy-Item -Destination $bundle
        Copy-Item (Join-Path $bundleRoot 'build-manifest.json') -Destination $bundle
        # LISEZMOI : gabarit versionne (placeholder {{VERSION}}).
        (Get-Content (Join-Path $PSScriptRoot 'dist_lisezmoi.txt') -Raw -Encoding UTF8).
            Replace('{{VERSION}}', $ver) |
            Set-Content (Join-Path $bundle 'LISEZMOI.txt') -Encoding UTF8

        Compress-Archive -Path $bundle -DestinationPath $zipPath -CompressionLevel Optimal
        Write-Host "  Zip release : $zipPath" -ForegroundColor Green
        Write-Host "  SHA-256     : $((Get-FileHash $zipPath -Algorithm SHA256).Hash)"
    }

    Write-Host ""
    Write-Host "Build OK -> dist\Explorer\" -ForegroundColor Green
    Write-Host "  Lancement : dist\Explorer\Explorer.exe"
    Write-Host "  Etat      : dist\Explorer\state\ (cree au lancement, conserve entre builds)"
} finally {
    Pop-Location
}
