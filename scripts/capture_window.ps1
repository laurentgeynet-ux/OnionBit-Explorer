# This file is part of Explorer.
# Copyright (C) 2026 Laurent Geynet
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# capture_window.ps1 — capture Win32 de la fenêtre d'un processus
# en PNG (les vrais pixels affichés, chrome Flutter compris —
# complément « yeux niveau app » du vol agent, docs/vol-agent.md).
#
# Usage :
#   pwsh -File scripts/capture_window.ps1 [-ProcessName explorer_app]
#       [-Out capture.png] [-ClientOnly]
#
# -ProcessName : nom du processus sans .exe (défaut explorer_app)
# -ClientOnly  : ne capture que la zone cliente (sans barre de titre)
# Sortie : chemin du PNG écrit sur stdout (dernière ligne).

param(
    [string]$ProcessName = 'explorer_app',
    [string]$Out = "capture_$(Get-Date -Format 'yyyyMMdd_HHmmss').png",
    [switch]$ClientOnly
)

Add-Type -AssemblyName System.Drawing

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class Win32Cap {
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
    public struct RECT { public int Left, Top, Right, Bottom; }
    public struct POINT { public int X, Y; }
}
'@

$proc = Get-Process -Name $ProcessName -ErrorAction SilentlyContinue |
    Where-Object { $_.MainWindowHandle -ne [IntPtr]::Zero } |
    Select-Object -First 1
if (-not $proc) {
    Write-Error "fenêtre introuvable : processus '$ProcessName' sans MainWindowHandle (app lancée ?)"
    exit 1
}
$hwnd = $proc.MainWindowHandle

if ($ClientOnly) {
    $cr = New-Object Win32Cap+RECT
    [void][Win32Cap]::GetClientRect($hwnd, [ref]$cr)
    $pt = New-Object Win32Cap+POINT
    [void][Win32Cap]::ClientToScreen($hwnd, [ref]$pt)
    $x = $pt.X; $y = $pt.Y
    $w = $cr.Right - $cr.Left; $h = $cr.Bottom - $cr.Top
} else {
    $wr = New-Object Win32Cap+RECT
    [void][Win32Cap]::GetWindowRect($hwnd, [ref]$wr)
    $x = $wr.Left; $y = $wr.Top
    $w = $wr.Right - $wr.Left; $h = $wr.Bottom - $wr.Top
}
if ($w -le 0 -or $h -le 0) {
    Write-Error "fenêtre '$ProcessName' de taille nulle (minimisée ?)"
    exit 1
}

# PrintWindow (PW_RENDERFULLCONTENT=2) capture même une fenêtre
# masquée — repli sur copie écran si elle échoue (fond noir).
$bmp = New-Object System.Drawing.Bitmap($w, $h)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$hdc = $g.GetHdc()
$ok = [Win32Cap]::PrintWindow($hwnd, $hdc, 2)
$g.ReleaseHdc($hdc)
if (-not $ok) {
    # Copie écran classique — exige la fenêtre visible/au premier plan.
    $g.CopyFromScreen($x, $y, 0, 0, (New-Object System.Drawing.Size($w, $h)))
}
$g.Dispose()

$full = Join-Path (Get-Location) $Out
$bmp.Save($full, [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()

Write-Output ("capturé : {0} ({1}x{2} px{3}) — PID {4}" -f `
    $full, $w, $h, $(if ($ClientOnly) { ', zone cliente' } else { '' }), $proc.Id)
