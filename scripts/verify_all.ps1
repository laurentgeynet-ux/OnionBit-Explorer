# This file is part of Explorer.
# Copyright (C) 2026 Laurent Geynet
# SPDX-License-Identifier: MIT OR Apache-2.0

# verify_all.ps1 — verification locale complete avant commit
# (AGENTS.md §3). Necessite pwsh 7+. Usage : pwsh -File scripts/verify_all.ps1

$ErrorActionPreference = 'Stop'
Set-Location (Split-Path -Parent $PSScriptRoot)

function Step([string]$Name, [scriptblock]$Cmd) {
    Write-Host "==> $Name" -ForegroundColor Cyan
    & $Cmd
    if ($LASTEXITCODE -ne 0) { throw "ECHEC : $Name" }
}

Step 'cargo fmt --all -- --check'            { cargo fmt --all -- --check }
Step 'cargo clippy --workspace --all-targets' { cargo clippy --workspace --all-targets -- -D warnings }
Step 'cargo test --workspace'                { cargo test --workspace }
Step 'cargo doc --workspace --no-deps'       { cargo doc --workspace --no-deps }

# Cliquet BCD (ADR-0010) — les scores ne doivent jamais
# descendre sous scripts/bcd_baseline.json. Sauté si la
# dépendance @mdn/browser-compat-data n'est pas installée
# (`npm i` dans scripts/).
if (Test-Path 'scripts/node_modules/@mdn/browser-compat-data/package.json') {
    Step 'cargo build explorer-cli' { cargo build -p explorer-cli }
    Step 'cliquet BCD (--gate)' {
        $cli = Get-ChildItem "$env:CARGO_TARGET_DIR\explorer-cli.exe" -Recurse -ErrorAction SilentlyContinue |
            Select-Object -First 1 -ExpandProperty FullName
        if (-not $cli) { $cli = "target\debug\explorer-cli.exe" }
        node scripts/bcd_coverage.mjs --cli $cli --gate scripts/bcd_baseline.json
    }
}

if (Test-Path 'app/pubspec.yaml') {
    Push-Location app
    try {
        Step 'flutter analyze' { flutter analyze }
        Step 'flutter test'    { flutter test }
    } finally {
        Pop-Location
    }
}

Write-Host 'OK — verification complete.' -ForegroundColor Green
