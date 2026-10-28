# This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
# SPDX-License-Identifier: MIT OR Apache-2.0
<#
.SYNOPSIS
  Réduit une capture de vol PNG en vignette lisible par l'agent.

.DESCRIPTION
  Les frames du daemon font 2-6 Mo : injectées en contexte par l'outil
  read, elles peuvent dépasser la limite de requête du client et brique
  la session (crash non reprenable). Ce script produit une vignette
  `_view_<nom>.png` (largeur -Width, défaut 900 px) dans le même dossier.
  Le PNG d'origine reste la preuve archivée ; on ne `read` QUE la
  vignette.

.EXAMPLE
  pwsh -File scripts/shrink_capture.ps1 "docs/Rapport vol/captures/vol100_cryptpad.png"
#>
param(
  [Parameter(Mandatory=$true)][string[]]$Path,
  [int]$Width = 900
)

foreach ($p in $Path) {
  $src = Resolve-Path $p -ErrorAction Stop
  $dir = Split-Path $src -Parent
  $base = [IO.Path]::GetFileNameWithoutExtension($src)
  $dst = Join-Path $dir "_view_$base.png"
  & ffmpeg -y -loglevel error -i $src -vf "scale=${Width}:-1" -frames:v 1 $dst
  if ($LASTEXITCODE -ne 0) { Write-Error "ffmpeg a échoué sur $p"; exit 1 }
  $ko = [math]::Round((Get-Item $dst).Length / 1024)
  Write-Output "$dst  ($ko Ko)"
}
