# This file is part of Explorer.
# Copyright (C) 2026 Laurent Geynet
# SPDX-License-Identifier: MIT OR Apache-2.0

<#
.SYNOPSIS
  Récolte de corpus automatisée (étape 212 — sprint 2 du plan
  « recensement des features » de l'équipe PIPELINE).

.DESCRIPTION
  Inverse le flux du triage terrain : au lieu d'un vol humain
  par site, récolte en masse — pour chaque URL :

    moteur : PNG + report.json (census, quand `--report`
             existe) + timing + doc_h
    oracle : diff pixel vs référence Chrome (-Diff / -WithRef)

  Sortie : `target/harvest/<run>/` = `<nom>.png`, `<nom>.json`,
  `index.json` agrégé et `report.html` (dashboard statique :
  site × statut × timing × top manques — triable).

  Modes :
    -Mode cli    : `explorer-cli shot` par site (défaut —
                   pipeline Session complet, profil release
                   recommandé pour un corpus).
    -Mode daemon : spawn explorer-daemon sur port libre, puis
                   /navigate (doc_h réel) + /frame + /layout
                   + /mem par onglet — plus de signaux par site.

  Corpus : -ListFile (listes texte, une URL/ligne, '#' =
  commentaire — `sites-legacy.txt` au format natif), -TrancoCsv
  (CSV « rang,domaine » → https://domaine/, -TrancoTop borne),
  sinon la shortlist embarquée (10 sites du smoke).

  Usage : pwsh -File scripts/site_harvest.ps1
            [-ListFile sites-legacy.txt] [-TrancoCsv top.csv]
            [-TrancoTop 500] [-Mode daemon] [-WithRef] [-Diff]
            [-Max 20] [-OutDir target\harvest\run]
#>

param(
    [string[]]$ListFile = @(),
    [string]$TrancoCsv = '',
    [int]$TrancoTop = 500,
    [string]$OutDir = '',
    [string]$Viewport = '1366x900',
    [int]$Max = 0,
    [ValidateSet('cli', 'daemon')][string]$Mode = 'cli',
    [ValidateSet('debug', 'release')][string]$Profile = 'release',
    [switch]$SkipBuild,
    [switch]$WithRef,
    [switch]$Diff,
    [string]$RefDir = '',
    [int]$NavTimeoutSec = 240
)

$ErrorActionPreference = 'Continue'
$root = Split-Path -Parent (Split-Path -Parent $PSCommandPath)
$run = Get-Date -Format 'yyyyMMdd-HHmmss'
if ($OutDir -eq '') {
    $OutDir = Join-Path $root "target\harvest\$run"
}
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

# --- Corpus ---------------------------------------------------------------
function Get-SiteName([string]$site) {
    # Identique à ref_capture.mjs — le nom de fichier relie le
    # shot moteur, la référence et le rapport d'un même site.
    # Le `/` terminal est absorbé : `example.com/` et
    # `example.com` produisent le même artefact.
    return (($site -replace '^https?://', '') -replace '/+$', '') `
        -replace '[^A-Za-z0-9_.-]', '_'
}

$sites = @()
foreach ($f in $ListFile) {
    $path = if ([IO.Path]::IsPathRooted($f)) { $f } else {
        Join-Path $root $f }
    if (-not (Test-Path $path)) {
        Write-Error "liste introuvable : $path"
        exit 1
    }
    $sites += Get-Content $path |
        Where-Object { $_ -match '\S' -and $_ -notmatch '^\s*#' } |
        ForEach-Object { $_.Trim() }
}
if ($TrancoCsv -ne '') {
    $tpath = if ([IO.Path]::IsPathRooted($TrancoCsv)) { $TrancoCsv } else {
        Join-Path $root $TrancoCsv }
    if (-not (Test-Path $tpath)) {
        Write-Error "CSV Tranco introuvable : $tpath"
        exit 1
    }
    # Format Tranco : « rang,domaine » par ligne. On prend les
    # $TrancoTop premiers — le CSV est déjà trié par rang.
    $sites += Get-Content $tpath | Select-Object -First $TrancoTop |
        ForEach-Object {
            $parts = $_ -split ','
            if ($parts.Length -ge 2) { "https://$($parts[-1].Trim())/" }
        } | Where-Object { $_ }
}
if ($sites.Count -eq 0) {
    # Défaut : shortlist du smoke (sites stables, sans login).
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
}
# Dédup en conservant l'ordre — Tranco et les listes se
# recouvrent (wikipedia, github…).
$seen = @{}
$sites = $sites | Where-Object {
    if ($seen.ContainsKey($_)) { $false } else {
        $seen[$_] = $true; $true }
}
if ($Max -gt 0 -and $sites.Count -gt $Max) {
    $sites = $sites[0..($Max - 1)]
}
Write-Output "Corpus : $($sites.Count) sites -> $OutDir"

# --- Binaires -------------------------------------------------------------
if (-not $SkipBuild) {
    $relArgs = @()
    if ($Profile -eq 'release') { $relArgs = @('--release') }
    Write-Output "Construction d'explorer-cli ($Profile)…"
    & cargo build -p explorer-cli @relArgs 2>&1 | Out-Null
    if ($LASTEXITCODE -ne 0) {
        Write-Error 'cargo build -p explorer-cli a échoué'
        exit 1
    }
    if ($Mode -eq 'daemon') {
        Write-Output "Construction d'explorer-daemon ($Profile)…"
        & cargo build -p explorer-daemon @relArgs 2>&1 | Out-Null
        if ($LASTEXITCODE -ne 0) {
            Write-Error 'cargo build -p explorer-daemon a échoué'
            exit 1
        }
    }
}
$meta = cargo metadata --no-deps --format-version 1 | ConvertFrom-Json
$triple = ''
$v = & rustc -vV 2>$null | Select-String -Pattern '^host: (\S+)'
if ($v) { $triple = $v.Matches[0].Groups[1].Value }
function Find-Bin([string]$name) {
    @(
        $(if ($triple) {
            Join-Path $meta.target_directory `
                "$triple\$Profile\$name.exe" }),
        (Join-Path $meta.target_directory "$Profile\$name.exe"),
        $(if ($triple) {
            Join-Path $meta.target_directory `
                "$triple\debug\$name.exe" }),
        (Join-Path $meta.target_directory "debug\$name.exe")
    ) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
}
$cli = Find-Bin 'explorer-cli'
if (-not $cli) {
    Write-Error "explorer-cli introuvable sous $($meta.target_directory)"
    exit 1
}
Write-Output "cli : $cli"

# `--report` : contrat du sprint 1 PIPE — détecté sur le texte
# d'usage du binaire résolu (pas de probe réseau). Si le shot
# échoue avec le flag malgré la détection, on retombe sans.
$usage = & $cli 2>&1 | Out-String
$reportOk = $usage -match '--report'
if ($reportOk) {
    Write-Output 'census : --report détecté (sprint 1 PIPE)'
} else {
    Write-Output 'census : --report absent — PNG + timing seuls'
}

# Viewport radar — même convention que site_smoke (étape 129).
$env:EXPLORER_VIEWPORT = $Viewport

# --- Références Chrome (oracle pixel) -------------------------------------
$refOut = if ($RefDir -ne '') { $RefDir } else {
    Join-Path $OutDir 'refs' }
if ($WithRef) {
    $node = Get-Command node -ErrorAction SilentlyContinue
    if (-not $node) {
        Write-Output '  refs : node absent — capture ignorée'
    } else {
        $listTmp = Join-Path $OutDir '_corpus.txt'
        $sites | Set-Content -Path $listTmp -Encoding utf8
        Write-Output '  refs : capture Chrome (ref_capture.mjs)…'
        & node (Join-Path $root 'scripts\ref_capture.mjs') `
            --list $listTmp --out $refOut --viewport $Viewport
    }
}
$diffOk = $Diff -or $WithRef

# --- Mode daemon : spawn + client HTTP poolé ------------------------------
$daemonProc = $null
$FuncBase = ''
$script:http = $null
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
                    Bytes  = $res.Content.ReadAsByteArrayAsync().
                        GetAwaiter().GetResult()
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

if ($Mode -eq 'daemon') {
    $daemon = Find-Bin 'explorer-daemon'
    if (-not $daemon) {
        Write-Error 'explorer-daemon introuvable (build ou -SkipBuild ?)'
        exit 1
    }
    $FuncPort = 0
    foreach ($p in 9379..9399) {
        $l = [System.Net.Sockets.TcpListener]::new(
            [System.Net.IPAddress]::Loopback, $p)
        try { $l.Start(); $FuncPort = $p; break }
        catch { } finally { try { $l.Stop() } catch { } }
    }
    if ($FuncPort -eq 0) {
        Write-Error 'aucun port libre dans 9379..9399'
        exit 1
    }
    $FuncBase = "http://127.0.0.1:$FuncPort/api"
    $profDir = Join-Path $env:TEMP "explorer-harvest-$FuncPort"
    New-Item -ItemType Directory -Force -Path $profDir | Out-Null
    $daemonErr = Join-Path $OutDir '_daemon.log'
    $daemonProc = Start-Process -FilePath $daemon -PassThru `
        -WindowStyle Hidden `
        -ArgumentList '--port', "$FuncPort", '--profile', $profDir `
        -RedirectStandardError $daemonErr
    $script:http = [System.Net.Http.HttpClient]::new()
    $script:http.Timeout = [System.Threading.Timeout]::InfiniteTimeSpan
    $up = $false
    foreach ($i in 1..60) {
        if (Send-Api Get "$FuncBase/status" $null 2) { $up = $true; break }
        Start-Sleep -Milliseconds 200
    }
    if (-not $up) {
        Write-Error 'daemon indisponible au démarrage'
        if ($daemonProc -and -not $daemonProc.HasExited) {
            Stop-Process -Id $daemonProc.Id -Force }
        exit 1
    }
    Write-Output "daemon : $daemon (port $FuncPort)"
}

# --- Boucle de récolte ----------------------------------------------------
$records = [System.Collections.Generic.List[object]]::new()
$ok = 0; $fail = 0
foreach ($site in $sites) {
    $name = Get-SiteName $site
    $png = Join-Path $OutDir "$name.png"
    $jsonPath = Join-Path $OutDir "$name.json"
    $rec = [ordered]@{
        site = $site; name = $name; mode = $Mode
        ok = $false; png = "$name.png"; png_bytes = 0
        elapsed_ms = 0; doc_h = $null; doc_h_clamped = $null
        diff_pct = $null; report = $null; mem = $null; error = $null
    }
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    if ($Mode -eq 'cli') {
        $rep = Join-Path $OutDir "$name.census.json"
        $argList = @('shot', $site, $png, $Viewport)
        if ($reportOk) { $argList += @('--report', $rep) }
        # Timeout dur : un TLS suspendu ne doit pas figer la
        # récolte — kill propre, marqué `timeout`.
        $p = Start-Process -FilePath $cli -ArgumentList $argList `
            -NoNewWindow -PassThru `
            -RedirectStandardError "$OutDir\_last_stderr.txt"
        if (-not $p.WaitForExit($NavTimeoutSec * 1000)) {
            try { $p.Kill() } catch { }
            $rec.error = "timeout ${NavTimeoutSec}s"
        } elseif ($p.ExitCode -ne 0) {
            if ($reportOk) {
                # Le flag détecté à l'usage est peut-être d'une
                # autre forme — repli sans --report, une fois.
                $reportOk = $false
                $argList = @('shot', $site, $png, $Viewport)
                $p = Start-Process -FilePath $cli `
                    -ArgumentList $argList -NoNewWindow -PassThru `
                    -RedirectStandardError "$OutDir\_last_stderr.txt"
                $null = $p.WaitForExit($NavTimeoutSec * 1000)
            }
            if ($p.ExitCode -ne 0) {
                $rec.error = "exit $($p.ExitCode)"
            }
        }
        $rec.elapsed_ms = $sw.ElapsedMilliseconds
        if ((Test-Path $png) -and (Get-Item $png).Length -gt 0) {
            $rec.ok = $true
            $rec.png_bytes = (Get-Item $png).Length
        } elseif (-not $rec.error) {
            $rec.error = 'pas de frame'
        }
        if (Test-Path $rep) {
            try {
                $rec.report = Get-Content $rep -Raw | ConvertFrom-Json
                if ($rec.report.doc_h) { $rec.doc_h = $rec.report.doc_h }
            } catch { }
        }
    } else {
        # Mode daemon : onglet -> viewport -> navigate -> frame
        # -> layout -> mem -> close. doc_h réel exposé par
        # /navigate (étape 65).
        $tab = Send-Api Post "$FuncBase/tabs" $null 60
        if (-not $tab) {
            $rec.error = 'POST /tabs'
        } else {
            $id = $tab.id
            $vw = $Viewport -split 'x'
            $null = Send-Api Post "$FuncBase/tabs/$id/viewport" `
                @{ w = [double]$vw[0]; h = [double]$vw[1]; scale = 1 } 30
            $nav = Send-Api Post "$FuncBase/tabs/$id/navigate" `
                @{ url = $site } $NavTimeoutSec
            $rec.elapsed_ms = $sw.ElapsedMilliseconds
            if (-not $nav) {
                $rec.error = 'navigate'
            } else {
                $rec.doc_h = $nav.doc_h
                $rec.doc_h_clamped = [bool]$nav.doc_h_clamped
                $fr = Send-Api Get "$FuncBase/tabs/$id/frame" $null 60 -Raw
                if ($fr -and $fr.Bytes.Length -gt 0) {
                    [IO.File]::WriteAllBytes($png, [byte[]]$fr.Bytes)
                    $rec.ok = $true
                    $rec.png_bytes = $fr.Bytes.Length
                } else {
                    $rec.error = 'frame 204'
                }
                $lay = Send-Api Get "$FuncBase/tabs/$id/layout" $null 30 -Raw
                if ($lay -and $lay.Bytes.Length -gt 0) {
                    [IO.File]::WriteAllBytes(
                        (Join-Path $OutDir "$name.layout.txt"),
                        [byte[]]$lay.Bytes)
                }
                $mem = Send-Api Get "$FuncBase/tabs/$id/mem" $null 30
                if ($mem) {
                    $rec.mem = [ordered]@{
                        dom_nodes = $mem.dom_nodes
                        layout_boxes = $mem.layout_boxes
                        display_items = $mem.display_items
                        total_bytes = $mem.total_bytes
                        js_heap_bytes = $mem.js_heap_bytes
                    }
                }
            }
            $null = Send-Api Delete "$FuncBase/tabs/$id" $null 15
        }
    }
    $sw.Stop()

    # Oracle pixel : explorer-cli diff vs la référence Chrome.
    if ($diffOk -and $rec.ok) {
        $ref = Join-Path $refOut "$name.ref.png"
        if (Test-Path $ref) {
            $dout = & $cli diff $png $ref 2>&1 | Out-String
            if ($dout -match 'diff : ([\d.]+)%') {
                $rec.diff_pct = [double]$Matches[1]
            } elseif ($dout -match 'identiques') {
                $rec.diff_pct = 0.0
            }
        }
    }

    $records.Add([pscustomobject]$rec)
    $rec | ConvertTo-Json -Depth 8 -Compress |
        Set-Content -Path $jsonPath -Encoding utf8
    if ($rec.ok) { $ok++; $tag = 'OK  ' } else { $fail++; $tag = 'FAIL' }
    $extra = @(
        "$([math]::Round($rec.elapsed_ms/1000,1))s",
        $(if ($rec.doc_h) { "doc:$($rec.doc_h)" }),
        $(if ($null -ne $rec.diff_pct) { "diff:$($rec.diff_pct)%" })
    ) | Where-Object { $_ }
    Write-Output "  $tag $site  ->  $($extra -join ' ')"
}
if ($daemonProc -and -not $daemonProc.HasExited) {
    Stop-Process -Id $daemonProc.Id -Force
}
Remove-Item "$OutDir\_last_stderr.txt" -ErrorAction SilentlyContinue
Remove-Item "$OutDir\_corpus.txt" -ErrorAction SilentlyContinue

# --- index.json + report.html ---------------------------------------------
$index = [ordered]@{
    run = $run; generated = (Get-Date).ToString('o')
    viewport = $Viewport; mode = $Mode; profile = $Profile
    ok = $ok; fail = $fail
    sites = $records
}
$indexPath = Join-Path $OutDir 'index.json'
$index | ConvertTo-Json -Depth 10 | Set-Content $indexPath -Encoding utf8

# Dashboard statique autonome (file:// — aucune ressource
# externe). Table triable par clic d'en-tête + Pareto des
# compteurs census (agrégat générique des maps {clé: n} sous
# `report.census` — la forme exacte vient du sprint 1 PIPE).
$dataJson = $index | ConvertTo-Json -Depth 10 -Compress
$html = @'
<!doctype html><meta charset="utf-8"><title>harvest __RUN__</title>
<style>
body{font:13px/1.4 Consolas,monospace;margin:1.5em;background:#fafafa;color:#222}
h1{font-size:1.2em}table{border-collapse:collapse;width:100%;background:#fff}
th,td{border:1px solid #ddd;padding:.25em .5em;text-align:left;white-space:nowrap}
th{cursor:pointer;background:#eee;user-select:none}th:hover{background:#e2e2e2}
tr.fail td{background:#fde8e8}tr.ok td{background:#f3fbf3}
td.num{text-align:right;font-variant-numeric:tabular-nums}
.bar{height:.8em;background:#4a7;vertical-align:middle}
.small{color:#666;font-size:.9em}h2{font-size:1em;margin-top:2em}
</style>
<h1>harvest __RUN__ — __OK__/__N__ sites · __MODE__ · __VIEWPORT__</h1>
<p class="small">cliquer un en-tête trie la table · census = compteurs
<code>--report</code> du moteur (sprint 1 PIPE) · diff = % pixels vs référence
Chrome (oracle)</p>
<table id="t"><thead><tr>
<th data-k="site">site</th><th data-k="ok">statut</th>
<th data-k="elapsed_ms" class="num">temps</th>
<th data-k="png_bytes" class="num">png</th>
<th data-k="doc_h" class="num">doc_h</th>
<th data-k="diff_pct" class="num">diff %</th>
<th data-k="topmiss">top manques</th><th data-k="error">erreur</th>
</tr></thead><tbody></tbody></table>
<h2>Pareto des manques (agrégat census)</h2>
<table id="p"><thead><tr><th data-k="key">feature / compteur</th>
<th data-k="n" class="num">total</th>
<th data-k="sites" class="num">sites</th><th></th></tr></thead>
<tbody></tbody></table>
<script>
const D = __DATA__;
function flat(o,p,out){for(const k in o){const v=o[k];
 if(v&&typeof v==='object')flat(v,p?p+'.'+k:k,out);
 else if(typeof v==='number')out[p?p+'.'+k:k]=v}return out}
function miss(s){const r=s.report;if(!r)return'';
 const c=flat(r.census||r,'',{});const top=Object.entries(c)
 .sort((a,b)=>b[1]-a[1]).slice(0,3).map(e=>e[0]+'×'+e[1]);
 return top.join(' ')}
const agg={};
for(const s of D.sites){s.topmiss=miss(s);
 if(!s.report)continue;const c=flat(s.report.census||{},'',{});
 for(const k in c){const a=agg[k]||(agg[k]={key:k,n:0,sites:0});
 a.n+=c[k];a.sites++}}
const pareto=Object.values(agg).sort((a,b)=>b.n-a.n).slice(0,60);
const maxN=pareto.length?pareto[0].n:1;
function fmt(k,v){if(v==null)return'';
 if(k==='elapsed_ms')return(v/1000).toFixed(1)+'s';
 if(k==='png_bytes')return Math.round(v/1024)+'K';
 if(k==='diff_pct')return v+'%';return v}
function draw(tbl,rows,keys){const tb=tbl.tBodies[0];tb.innerHTML='';
 for(const r of rows){const tr=tb.insertRow();
 tr.className=r.ok===false?'fail':(r.ok===true?'ok':'');
 for(const k of keys){const td=tr.insertCell();
  if(/ms|bytes|pct|doc_h|^n$|sites/.test(k))td.className='num';
  td.textContent=fmt(k,r[k])}}}
let sk='site',sd=1;
function resort(){const rows=[...D.sites].sort((a,b)=>{
 let x=a[sk],y=b[sk];if(x==null)x='';if(y==null)y='';
 return(x<y?-1:x>y?1:0)*sd});
 draw(document.getElementById('t'),rows,
 ['site','ok','elapsed_ms','png_bytes','doc_h','diff_pct','topmiss','error']);
 const pb=document.getElementById('p');
 const pt=pb.tBodies[0];pt.innerHTML='';
 for(const r of pareto){const tr=pt.insertRow();
  for(const k of['key','n','sites']){const td=tr.insertCell();
   if(k!=='key')td.className='num';td.textContent=r[k]}
  const td=tr.insertCell();
  td.innerHTML='<div class="bar" style="width:'+
   Math.max(1,r.n/maxN*320)+'px"></div>'}}
document.querySelectorAll('th').forEach(th=>th.onclick=()=>{
 const k=th.dataset.k;if(sk===k)sd=-sd;else{sk=k;sd=1}resort()});
resort();
</script>
'@
# `.Replace` = substitution littérale (le JSON embarqué peut
# contenir `$` — `-replace` l'interpréterait).
$html = $html.Replace('__RUN__', $run).Replace('__OK__', "$ok").
    Replace('__N__', "$($sites.Count)").
    Replace('__MODE__', "$Mode/$Profile").
    Replace('__VIEWPORT__', $Viewport).Replace('__DATA__', $dataJson)
$htmlPath = Join-Path $OutDir 'report.html'
Set-Content -Path $htmlPath -Value $html -Encoding utf8

Write-Output ''
Write-Output "Récolte : $ok OK / $fail FAIL — $($sites.Count) sites"
Write-Output "index   : $indexPath"
Write-Output "rapport : $htmlPath"
if ($fail -gt 0) { exit 2 }
