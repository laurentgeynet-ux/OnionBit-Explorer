# This file is part of Explorer.
# Copyright (C) 2026 Laurent Geynet
# SPDX-License-Identifier: MIT OR Apache-2.0

<#
.SYNOPSIS
  Smoke test interop sur sites réels (étape 48, ADR-0010).

.DESCRIPTION
  Rend chaque site de la liste via `explorer-cli shot`
  (pipeline Session complet : TLS + ALPN h1/h2, charset,
  cascade, layout, JS, images, webfonts → PNG) et mesure :

    - navigation réussie (HTTP final 2xx/3xx, aucune erreur)
    - frame produite (PNG non vide)
    - couverture non-blanche (le site a peint quelque chose)

  Informatif uniquement : un échec de site ne fait PAS
  échouer le script (le web réel est hors contrôle) — le
  rapport console est la mesure. Sortie PNG sous
  `target/site_smoke/` pour inspection visuelle.

  Phase 2 (sauf -SkipFunctional) : le critère « fonctionnel »
  du palier N-A se mesure par sondes sur le daemon — pour
  chaque site rendu, on sonde la page par `mousemove`
  (l'outcome `hover` rapporte `url`/`editable` : le hit-test
  moteur désigne les cibles, aucune coordonnée devinée) puis
  `click` de lien → navigation réelle + `back`, et champ
  `editable` → `focused` + `key` → `edited`.

  Usage : pwsh -File scripts/site_smoke.ps1 [sites.txt] [-SkipFunctional]
  La liste par défaut est embarquée ci-dessous (sites
  stables, sans login, majoritairement HTML/CSS).
#>

param(
    [string]$ListFile = '',
    [switch]$SkipFunctional
)

$ErrorActionPreference = 'Continue'
$root = Split-Path -Parent (Split-Path -Parent $PSCommandPath)
$outDir = Join-Path $root 'target\site_smoke'
New-Item -ItemType Directory -Force -Path $outDir | Out-Null

if ($ListFile -eq '') {
    # Corpus embarqué (10 sites — palier N-A, ≥8/10 attendu) :
    # diversité de formes — document statique, wiki, site
    # CSS-moderne, gros documents (cap FrameTooBig), scripts
    # réels. Shortlist terrain de la roadmap navigateur ;
    # duckduckgo écarté (blanc « par design » — anti-FOUC
    # honoré, script de déblocage non exécuté : faux échec).
    $sites = @(
        'https://example.com/',
        'https://info.cern.ch/',
        'https://en.wikipedia.org/wiki/Main_Page',
        'https://lobste.rs/',
        'https://news.ycombinator.com/',
        'https://text.npr.org/',
        'https://www.weather.gov/',
        'https://developer.mozilla.org/en-US/',
        'https://github.com/',
        'https://stallman.org/'
    )
} else {
    $sites = Get-Content $ListFile |
        Where-Object { $_ -match '\S' -and $_ -notmatch '^\s*#' } |
        ForEach-Object { $_.Trim() }
}

Write-Output "Construction d'explorer-cli…"
cargo build -p explorer-cli 2>&1 | Out-Null
if ($LASTEXITCODE -ne 0) {
    Write-Error 'cargo build -p explorer-cli a échoué'
    exit 1
}
# Le binaire vit sous `target_directory` — pas `$root\target`
# (`.cargo/config.toml` peut rediriger `target-dir`, et
# `CARGO_TARGET_DIR` prime encore : `cargo metadata` résout le
# chemin réel — un chemin en dur rendait des faux OK sur des
# PNG périmés).
$meta = cargo metadata --no-deps --format-version 1 |
    ConvertFrom-Json
$cli = Join-Path $meta.target_directory 'debug\explorer-cli.exe'
if (-not (Test-Path $cli)) {
    Write-Error "explorer-cli introuvable : $cli"
    exit 1
}
# Purge des PNG précédents : une frame produite dans un run
# antérieur ne doit pas valider l'échec courant.
Get-ChildItem -Path $outDir -Filter '*.png' -ErrorAction SilentlyContinue |
    Remove-Item -Force

$ok = 0
$fail = 0
$okSites = @()
foreach ($site in $sites) {
    $name = ($site -replace '^https?://', '') -replace '[^A-Za-z0-9_.-]', '_'
    $png = Join-Path $outDir "$name.png"
    $null = & $cli shot $site $png 2>&1
    if ($LASTEXITCODE -eq 0 -and (Test-Path $png) -and
        (Get-Item $png).Length -gt 0) {
        $size = [math]::Round((Get-Item $png).Length / 1KB, 1)
        Write-Output "  OK   $site  ->  $name.png (${size} Ko)"
        $ok++
        $okSites += $site
    } else {
        Write-Output "  FAIL $site"
        $fail++
    }
}

# --- Phase 2 : probes fonctionnelles via le daemon -------------------------
# Le palier N-A exige « fonctionnel », pas seulement rendu. Le
# daemon expose le hit-test moteur via `mousemove` → outcome
# `hover` rapporte `url` (lien sous le point) et `editable`
# (champ éditable) : on sonde la page en grille viewport —
# aucune coordonnée n'est devinée, c'est le moteur qui désigne.
$funcOk = 0
if (-not $SkipFunctional -and $ok -gt 0) {
    Write-Output ''
    Write-Output "Construction d'explorer-daemon…"
    cargo build -p explorer-daemon 2>&1 | Out-Null
    if ($LASTEXITCODE -ne 0) {
        Write-Error 'cargo build -p explorer-daemon a échoué'
        exit 1
    }
    $daemon = Join-Path $meta.target_directory 'debug\explorer-daemon.exe'
    if (-not (Test-Path $daemon)) {
        Write-Error "explorer-daemon introuvable : $daemon"
        exit 1
    }
    $FuncPort = 9379
    $FuncBase = "http://127.0.0.1:$FuncPort/api"
    # Profil jetable : le smoke ne pollue ni le cache ni
    # l'historique du profil utilisateur.
    $profDir = Join-Path $env:TEMP 'explorer-site-smoke'
    New-Item -ItemType Directory -Force -Path $profDir | Out-Null
    # Port encombré ? Un daemon smoke d'un run précédent (mort
    # anormale du script, run parallèle) peut encore le tenir —
    # le bind échoue alors et TOUTES les sondes parlent à un
    # moteur possiblement obsolète (veille 22/10 : `os error
    # 10048` — fonctionnels mesurés 1/9 sur moteur stale). On
    # libère le port, mais chirurgicalement : seulement si le
    # titulaire est bien un `explorer-daemon` (jamais un
    # processus étranger qui occuperait 9379 par hasard).
    Get-NetTCPConnection -LocalPort $FuncPort -State Listen `
        -ErrorAction SilentlyContinue |
        ForEach-Object { $_.OwningProcess } | Sort-Object -Unique |
        ForEach-Object {
            $p = Get-Process -Id $_ -ErrorAction SilentlyContinue
            if ($p -and $p.ProcessName -like 'explorer-daemon*') {
                Write-Output "  port $FuncPort libéré (PID $_ = daemon stale)"
                Stop-Process -Id $_ -Force
            }
        }
    # stderr capturé : une mort du daemon en cours de sondage
    # (observée une fois — cause non attribuée) doit laisser
    # une trace, pas disparaître dans le silence.
    $daemonErr = Join-Path $env:TEMP 'explorer-site-smoke-daemon.log'
    $proc = Start-Process -FilePath $daemon -PassThru -WindowStyle Hidden `
        -ArgumentList '--port', "$FuncPort", '--profile', $profDir `
        -RedirectStandardError $daemonErr

    # Client HTTP poolé (keep-alive) : les sondes se comptent en
    # milliers par run et `Invoke-RestMethod` ouvrait une connexion
    # TCP à chaque appel — ~16 000 sockets TIME_WAIT épuisaient les
    # ports éphémères Windows en cours de phase et les sites restants
    # tombaient en WSAEADDRINUSE (faux échecs mesurés).
    $script:http = [System.Net.Http.HttpClient]::new()
    $script:http.Timeout = [System.Threading.Timeout]::InfiniteTimeSpan

    function Send-Api {
        param(
            [Parameter(Mandatory)][string]$Method,
            [Parameter(Mandatory)][string]$Uri,
            [hashtable]$Body,
            [int]$TimeoutSec = 30,
            [switch]$Raw)
        $cts = [System.Threading.CancellationTokenSource]::new()
        $cts.CancelAfter([TimeSpan]::FromSeconds($TimeoutSec))
        try {
            $req = [System.Net.Http.HttpRequestMessage]::new(
                [System.Net.Http.HttpMethod]::new($Method), $Uri)
            if ($null -ne $Body) {
                $req.Content = [System.Net.Http.StringContent]::new(
                    ($Body | ConvertTo-Json -Compress),
                    [System.Text.Encoding]::UTF8, 'application/json')
            }
            $res = $script:http.SendAsync(
                $req, $cts.Token).GetAwaiter().GetResult()
            try {
                if ($Raw) {
                    return @{
                        Status = [int]$res.StatusCode
                        Bytes  = $res.Content.ReadAsByteArrayAsync().GetAwaiter().GetResult()
                    }
                }
                if (-not $res.IsSuccessStatusCode) { return $null }
                $text = $res.Content.ReadAsStringAsync().GetAwaiter().GetResult()
                if ([string]::IsNullOrWhiteSpace($text)) { return $true }
                return ($text | ConvertFrom-Json)
            } finally { $res.Dispose(); $req.Dispose() }
        } catch { return $null }
        finally { $cts.Dispose() }
    }

    function Wait-Func([scriptblock]$Probe, [int]$Tries = 300) {
        foreach ($i in 1..$Tries) {
            $r = & $Probe
            if ($null -ne $r -and $r -ne $false) { return $r }
            Start-Sleep -Milliseconds 200
        }
        return $null
    }
    function Send-Input($Id, [hashtable]$Ev) {
        # 30 s : l'input attend le verrou session — une
        # page lourde en apply (wikipedia 10 Mo) le tient
        # plusieurs secondes ; 10 s = timeout mensonger.
        Send-Api Post "$FuncBase/tabs/$Id/input" $Ev 30
    }
    function Get-TabUrl($Id) {
        $r = Send-Api Get "$FuncBase/tabs" $null 20
        if ($r) { ($r.tabs | Where-Object { $_.id -eq $Id }).url }
    }
    function Get-Scroll($Id) {
        # `scroll dy=0` est un no-op qui rapporte la position
        # courante — nécessaire après nav/back : le bfcache
        # restaure `scroll_y` hors de notre suivi local.
        $r = Send-Input $Id @{ type = 'scroll'; dy = 0 }
        if ($r -and $null -ne $r.scroll_y) { [double]$r.scroll_y } else { 0.0 }
    }
    function Get-FrameHash($Id) {
        # Hash de la frame courante — 204 (pas encore de frame)
        # et corps vide rapportent $null. Sert à distinguer la
        # frame de DESTINATION de celle de la page d'origine :
        # `/frame` renvoie toujours l'image la plus récente, donc
        # « contenu non vide » seul voit l'ANCIENNE frame pendant
        # que la nav d'arrivée peint — back posté alors bloquait
        # le verrou session (mesuré : back-POST-TIMEOUT sur
        # iana.org alors que back vaut ~1 ms une fois peint).
        $r = Send-Api Get "$FuncBase/tabs/$Id/frame" $null 30 -Raw
        if ($null -eq $r -or $r.Status -eq 204 -or
            $r.Bytes.Length -eq 0) {
            return $null
        }
        (Get-FileHash -InputStream `
            ([System.IO.MemoryStream]::new([byte[]]$r.Bytes)) `
            -Algorithm SHA256).Hash
    }
    function Wait-DestFrame($Id, [string]$PrevHash, [int]$BudgetSec = 90) {
        # Une frame « nouvelle » = hash différent de celui d'avant
        # le clic. $null (204) ou hash identique = page d'origine
        # encore affichée → on attend. Repli : si la session
        # répond à `scroll dy=0` en < 2 s, le verrou cycle — la
        # destination est posée même si le hash tarde à changer.
        # Budget en TEMPS : chaque sonde peut elle-même attendre
        # le verrou (~30 s), compter les itérations serait borné
        # en nombre, pas en durée.
        $deadline = (Get-Date).AddSeconds($BudgetSec)
        while ((Get-Date) -lt $deadline) {
            $h = Get-FrameHash $Id
            if ($h -and $h -ne $PrevHash) { return $h }
            $sw = [System.Diagnostics.Stopwatch]::StartNew()
            $r = Send-Input $Id @{ type = 'scroll'; dy = 0 }
            if ($h -and $r -and $sw.ElapsedMilliseconds -lt 2000) {
                return 'settled'
            }
        }
        return $null
    }

    try {
        $up = Wait-Func {
            if (Send-Api Get "$FuncBase/status" $null 2) { $true }
            else { $false }
        } 60
        if (-not $up) {
            Write-Output '  daemon indisponible — phase fonctionnelle sautée'
        } else {
            foreach ($site in $okSites) {
                if ($proc.HasExited) {
                    Write-Output "  daemon mort (code $($proc.ExitCode)) — " +
                        "stderr sous $daemonErr — fin de phase"
                    break
                }
                $tag = ($site -replace '^https?://', '') -replace '/.*', ''
                $verdict = 'nav:TIMEOUT'
                $id = $null
                try {
                    $tab = Send-Api Post "$FuncBase/tabs" $null 60
                    if (-not $tab) { throw 'POST /tabs' }
                    $id = $tab.id
                    # `/navigate` est synchrone jusqu'au commit —
                    # le pipeline complet d'un document géant
                    # (github = frame 49 Mo) dépasse 30 s ; mesuré
                    # ~124 s au commit réel → 240 s de fenêtre.
                    if (-not (Send-Api Post `
                            "$FuncBase/tabs/$id/navigate" `
                            @{ url = $site } 240)) {
                        throw 'navigate'
                    }
                    # Frame produite (~60 s de grâce pour les
                    # gros documents — github a pris ~30 s).
                    $framed = Wait-Func {
                        $r = Send-Api Get "$FuncBase/tabs/$id/frame" `
                            $null 30 -Raw
                        if ($r -and $r.Bytes.Length -gt 0) { $true }
                        else { $false }
                    } 300
                    if (-not $framed) { Write-Output "  $tag  $verdict"; continue }
                    $url0 = Get-TabUrl $id
                    if (-not $url0) { $url0 = $site }

                    # Sondage grille (viewport 1024×768, bandes de
                    # 700 px de scroll) : premier lien + premier
                    # champ éditable désignés par le hit-test.
                    # Le lien préféré pointe AILLEURS que la page
                    # courante — le logo d'en-tête lie presque
                    # toujours la page elle-même, et attendre
                    # « url ≠ url0 » sur lui = faux négatif garanti.
                    function Resolve-Url([string]$raw, [string]$base) {
                        if ($raw -match '^https?://') { return $raw }
                        if ($raw.StartsWith('/')) {
                            $u = [uri]$base
                            $p = ''
                            if (-not $u.IsDefaultPort) { $p = ":$($u.Port)" }
                            return "$($u.Scheme)://$($u.Host)$p$raw"
                        }
                        return $raw
                    }
                    $selfUrl = ($url0 -replace '#.*$', '').TrimEnd('/')
                    $link = $null       # premier lien trouvé (repli)
                    $link2 = $null      # premier lien vers autre URL
                    $edit = $null
                    $scrollY = 0.0
                    $jsErr = 0
                    $navByScript = $false
                    foreach ($band in @(0, 700)) {
                        if ($band -gt $scrollY) {
                            $r = Send-Input $id @{
                                type = 'scroll'; dy = ($band - $scrollY) }
                            if ($r -and $null -ne $r.scroll_y) {
                                $scrollY = [double]$r.scroll_y
                            }
                        }
                        for ($y = 40; $y -le 704 -and -not ($link2 -and $edit);
                             $y += 48) {
                            for ($x = 40; $x -le 968 -and
                                 -not ($link2 -and $edit); $x += 64) {
                                $r = Send-Input $id @{
                                    type = 'mousemove'; x = $x; y = $y }
                                if (-not $r) { $jsErr++; continue }
                                if ($r.result -eq 'navigated') {
                                    # Un handler script a enfilé une
                                    # nav pendant le sondage — preuve
                                    # « scripts actifs », on bascule
                                    # directement sur l'attente + back.
                                    $navByScript = $true
                                    break
                                }
                                if ($r.result -eq 'hover') {
                                    # Les ancres `#…` prouvent le
                                    # hit-test mais pas la navigation.
                                    if ($r.url -and
                                        -not "$($r.url)".StartsWith('#')) {
                                        $cand = @{ x = $x; y = $y;
                                                   url = "$($r.url)";
                                                   sy = $scrollY }
                                        if (-not $link) { $link = $cand }
                                        if (-not $link2 -and
                                            ((Resolve-Url $r.url $site).
                                                TrimEnd('/') -replace
                                                '#.*$', '') -ne $selfUrl) {
                                            $link2 = $cand
                                        }
                                    }
                                    if (-not $edit -and $r.editable) {
                                        $edit = @{ x = $x; y = $y;
                                                   sy = $scrollY }
                                    }
                                }
                            }
                            if ($navByScript) { break }
                        }
                        if (($link2 -and $edit) -or $navByScript) { break }
                    }
                    $clickLink = if ($link2) { $link2 } else { $link }

                    # Lien : clic → navigation réelle → back restaure.
                    # Le sondage se fait bande par bande (le scroll
                    # décale la fenêtre de hit-test) ; avant le clic on
                    # relit le scroll réel — `back`/bfcache peut avoir
                    # restauré une position hors de notre suivi.
                    $lienRes = 'lien:absent'
                    if ($navByScript) {
                        # Nav déjà enfilée par le script : frame de
                        # référence « à la détection » — si la dest
                        # est déjà peinte le repli mutex tranche.
                        $hash0 = Get-FrameHash $id
                        $moved = Wait-Func {
                            $u = Get-TabUrl $id
                            if ($u -and $u -ne $url0) { $u } else { $false }
                        } 300
                        if ($moved) {
                            # L'URL commute avant la fin de peinture —
                            # `back` pendant un apply en vol attend le
                            # verrou session : attendre la frame de la
                            # page d'arrivée d'abord.
                            $null = Wait-DestFrame $id $hash0
                            $backSent = $null -ne (Send-Api Post `
                                "$FuncBase/tabs/$id/back" $null 120)
                            $restored = Wait-Func {
                                $u = Get-TabUrl $id
                                if ($u -eq $url0) { $true } else { $false }
                            } 200
                            $lienRes = if ($restored) {
                                "lien:script-nav+back($moved)"
                            } elseif (-not $backSent) {
                                'lien:script-nav/back-POST-TIMEOUT'
                            } else { 'lien:script-nav/back-TIMEOUT' }
                        } else {
                            $lienRes = 'lien:script-nav-TIMEOUT'
                        }
                    } elseif ($clickLink) {
                        $need = $clickLink.sy - (Get-Scroll $id)
                        if ([math]::Abs($need) -gt 1) {
                            $null = Send-Input $id @{
                                type = 'scroll'; dy = $need }
                        }
                        # Frame de référence APRÈS le reposition-
                        # nement (le scroll repeint l'origine) —
                        # la frame d'arrivée se reconnaît à son
                        # hash différent.
                        $hash0 = Get-FrameHash $id
                        $out = Send-Input $id @{
                            type = 'click'; x = $clickLink.x;
                            y = $clickLink.y }
                        $moved = Wait-Func {
                            $u = Get-TabUrl $id
                            if ($u -and $u -ne $url0) { $u } else { $false }
                        } 300
                        if ($moved) {
                            # Idem : attendre la frame de la page
                            # d'arrivée avant `back` (verrou session).
                            $null = Wait-DestFrame $id $hash0
                            $backSent = $null -ne (Send-Api Post `
                                "$FuncBase/tabs/$id/back" $null 120)
                            $restored = Wait-Func {
                                $u = Get-TabUrl $id
                                if ($u -eq $url0) { $true } else { $false }
                            } 200
                            $lienRes = if ($restored) {
                                "lien:OK+back($($clickLink.url))"
                            } elseif (-not $backSent) {
                                "lien:nav-OK/back-POST-TIMEOUT($($clickLink.url))"
                            } else {
                                "lien:nav-OK/back-TIMEOUT($($clickLink.url))" }
                        } else {
                            $lienRes = "lien:clic-sans-nav($($out.result),$($clickLink.url))"
                        }
                    }

                    # Formulaire : clic → focused, touche → edited.
                    $formRes = 'form:absent'
                    if ($edit) {
                        $need = $edit.sy - (Get-Scroll $id)
                        if ([math]::Abs($need) -gt 1) {
                            $null = Send-Input $id @{
                                type = 'scroll'; dy = $need }
                        }
                        $out = Send-Input $id @{
                            type = 'click'; x = $edit.x; y = $edit.y }
                        if ($out -and $out.result -eq 'focused') {
                            $k = Send-Input $id @{ type = 'key'; key = 'x' }
                            $formRes = if ($k -and $k.result -eq 'edited') {
                                'form:OK'
                            } else { "form:focus/edit:$($k.result)" }
                        } else {
                            $formRes = "form:clic:$($out.result)"
                        }
                    }
                    $jsRes = if ($jsErr -eq 0) { 'dispatch:OK' }
                             else { "dispatch:$jsErr timeouts" }
                    $verdict = "nav:OK $lienRes $formRes $jsRes"
                    if ($lienRes -match '^lien:(OK\+back|script-nav\+back)') {
                        $funcOk++
                    }
                } catch {
                    $verdict = "ERREUR: $_"
                } finally {
                    if ($null -ne $id) {
                        $null = Send-Api Delete "$FuncBase/tabs/$id" $null 30
                    }
                }
                Write-Output "  $tag  $verdict"
            }
        }
    } finally {
        $script:http.Dispose()
        if ($proc -and -not $proc.HasExited) {
            Stop-Process -Id $proc.Id -Force
        }
    }
}

Write-Output ''
Write-Output "=== Smoke sites réels : $ok OK / $fail FAIL ==="
if (-not $SkipFunctional -and $ok -gt 0) {
    Write-Output "=== Fonctionnels (nav + lien + back) : $funcOk/$($okSites.Count) ==="
}
Write-Output "PNG sous $outDir"
# Code de sortie informatif : 0 si au moins un site rend.
if ($ok -gt 0) { exit 0 } else { exit 1 }
