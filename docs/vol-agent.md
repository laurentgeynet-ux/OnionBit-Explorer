<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Vol agent — conduire le navigateur par l'API

Procédure pour qu'une piste (PIPE, NAV, JS) joue un **vol humain
sans humain** : les yeux = la frame PNG du moteur, les mains =
l'API REST d'input, la télémétrie = `dom`/`layout`/`mem`/SSE.

> **Prérequis — binaire frais *et propre*** (règle §8 AGENTS,
> durcie au vol d'étalonnage 2026-10-24) : un verdict visuel
> n'est valable que sur un daemon buildé après le dernier
> commit touché **et sur un arbre sans WIP** — un binaire
> produit dans un worktree chargé du mi-chantier d'une autre
> piste produit des mesures mensongères (mesuré : télémétrie
> 404, `doc_h` tronqué, scroll bloqué — trois faux constats
> disparus sur HEAD propre). Si le worktree partagé porte du
> WIP, builder depuis un worktree éphémère à HEAD :
> `git worktree add <dir> HEAD`.

## 0. Checklist de lancement — séquence canonique

Chaque vol ouvre sur cette séquence, dans l'ordre, sans
exception. Les chemins corpus = `C:\OnionBit-vol\` (structure
canonique : `README.md` du corpus).

```powershell
# 0.1 arbre propre + HEAD connu
git rev-parse HEAD                    # le tag du vol
git status --short                    # WIP ? → worktree
                                    # éphémère à HEAD sinon

# 0.2 binaire frais (jamais un .exe qui traîne)
cargo build --release -p explorer-daemon   # attendre « Finished »

# 0.3 daemon dédié — port 9385+, profil déclaré
$env:EXPLORER_VIEWPORT='1366x900'
$env:EXPLORER_PROFILE='C:\OnionBit-vol\prof<N>'   # froid = défaut
.\target\release\explorer-daemon.exe --port 9385

# 0.4 watchdog OBLIGATOIRE — les deux lignes, dès le lancement
pwsh -File scripts/vol_joblimit.ps1 -TargetPid <pid> -LimitMB 5120
pwsh -File scripts/vol_memwatch.ps1 -DaemonPid <pid> -MaxMB 5120 `
  -Log "C:\OnionBit-vol\logs\_vol<N>_watch.log" &

# 0.5 artefacts au corpus, jamais dans le repo
#   logs    → C:\OnionBit-vol\logs\_vol<N>_*.log
#   PNG     → C:\OnionBit-vol\captures\vol<N>_<cible>.png
#   rapport → C:\OnionBit-vol\RAPPORT_VOL_<date>_AGENT<n>_<TAG>.md
```

**Coordination multi-équipes — `CLAIMS.md`** : plusieurs
équipes volent en parallèle. Avant le premier `navigate`,
ajouter sa ligne dans `C:\OnionBit-vol\CLAIMS.md` (vol N,
agent N, tag, cibles) — une cible sur une ligne récente
(< 2 h, sans rapport) est occupée. Le rapport déposé libère
les cibles ; un claim > 2 h sans rapport = vol mort, cibles
libres.

En clôture de vol : daemon tué par **PID** (jamais `taskkill
/IM` — les voisins appartiennent aux autres équipes), logs
archivés au corpus, `git add -A && git commit` dans
`C:\OnionBit-vol`, entrée journal + statuts §B synchronisés.

## 1. Lancer le daemon dédié

```powershell
cargo build -p explorer-daemon          # attendre « Finished »
.\target\debug\explorer-daemon.exe --port 9385
```

- **Port ≠ 9377** — celui-ci appartient au vol humain / à l'app.
  Choisir 9385+ pour les vols agents (pas de collision).
- `--release` recommandé pour les sites lourds (debug ≈ 10× plus
  lent — une nav github peut dépasser le timeout REST en debug).
- La variable `EXPLORER_VIEWPORT=1366x900` cale la fenêtre de vol
  (cohérent avec le radar nightly).

### Profil du daemon — froid, chaud, gabarit

`EXPLORER_PROFILE` choisit l'état persistant (cookies, HSTS,
DNS, IDB, cache) — trois modes, à **déclarer dans le rapport** :

- **Profil froid** (défaut de preuve) : dir dédié au vol
  (`EXPLORER_PROFILE=_vol<N>_profile`) — verdict sur premier
  contact réel. Exigé pour tout constat consentement/first-party.
- **Profil chaud partagé** : le profil utilisateur
  (`~/.config/explorer`) — historique cumulé : consentements
  déjà acceptés, cookies de session, jar rempli. Utile pour les
  bancs où l'état établi change la forme du challenge (SERP,
  re-vols §B-14 après premier passage). **Limites dures** :
  un seul daemon à la fois (`state/explorer.lock` fslock —
  jamais en flotte parallèle) ; et la chaleur peut **masquer**
  un mur (un consentement accepté au vol N ne se montre plus —
  le rapport écrit « profil chaud » en tête).
- **Profil gabarit** : copier un seed (`_profile_seed/` : fonts,
  hsts, dns pré-remplis) vers le dir du vol — warm-start
  reproductible **par agent**, sans collision ni contamination
  croisée (chaque vol garde son propre état à partir du même
  point de départ).

## 2. Séquence de vol

```powershell
$BASE = "http://127.0.0.1:9385"

# — mains : créer un onglet —
$tab = (Invoke-RestMethod "$BASE/api/tabs" -Method Post).id

# — mains : naviguer —
Invoke-RestMethod "$BASE/api/tabs/$tab/navigate" -Method Post `
  -ContentType 'application/json' -Body '{"url":"https://en.wikipedia.org"}'

# — yeux : la frame courante (PNG moteur) —
Invoke-WebRequest "$BASE/api/tabs/$tab/frame" -OutFile frame.png
# → lire frame.png : l'agent le voit (outil read sur image)

# — mains : scroller, cliquer, survoler —
Invoke-RestMethod "$BASE/api/tabs/$tab/input" -Method Post `
  -ContentType 'application/json' -Body '{"type":"scroll","dy":800}'
Invoke-RestMethod "$BASE/api/tabs/$tab/input" -Method Post `
  -ContentType 'application/json' -Body '{"type":"click","x":300,"y":200}'

# — télémétrie —
Invoke-RestMethod "$BASE/api/tabs/$tab/mem"      # empreinte par réservoir
Invoke-RestMethod "$BASE/api/tabs/$tab/dom"      # dump DOM (texte)
Invoke-RestMethod "$BASE/api/tabs/$tab/layout"   # arbre de boîtes
```

## 3. Carte des endpoints utiles

| Endpoint | Rôle « organe » |
|---|---|
| `POST /api/tabs` | nouvel onglet → `id` |
| `POST /api/tabs/{id}/navigate` `{url}` | naviguer |
| `GET  /api/tabs/{id}/frame` | **yeux** — PNG du framebuffer moteur |
| `GET  /api/tabs/{id}/frame?x=&y=&w=&h=` | yeux zoomés — région |
| `POST /api/tabs/{id}/input` | **mains** — `click`/`mousemove`/`scroll`/`key`/`keyup` |
| `POST /api/tabs/{id}/viewport` `{w,h}` | fenêtre de vol |
| `GET  /api/tabs/{id}/dom` `/layout` `/mem` | télémétrie — **404 transitoire possible sur page à JS continu** (DOM détaché pendant les apply jobs : alternance 200/404 mesurée sur github) — retenter, ne pas diagnostiquer un « DOM absent » |
| `POST /api/tabs/{id}/back` `/forward` `/reload` | historique |
| `GET  /api/tabs/{id}/media` | offres média vivantes (`<video>`/`<audio>`/MSE) |
| `GET  /api/tabs/{id}/media/{mid}` | fiche d'une surface (diagnostics, `410` id mort) |
| `POST …/media/{mid}/claim` `{codecs:[…]}` | premier arrivé gagne : `200` / `409` / `410` |
| `GET  …/media/{mid}/segments?buffer=&max=&from=` | pull MSE — `octet-stream` + `X-Mse-Cursor`/`Eos`/`Section`/`Pending` ; `from` hors fenêtre → `416` ; non-MSE → `{mse:false}` |
| `POST …/media/{mid}/release` | détachement hôte |
| `POST …/media/{mid}/state` | rapport hôte ≤ 4 Hz — `{position_ms,duration_ms,buffered_ms,state,error?}`, `state ∈ playing\|paused\|ended\|error` |
| `GET  /api/events` | SSE — `navigated`, `frame_invalidated`, `media_offered`/`moved`/`gone`/`command`… |

Format `input` : `{"type":"click","x":N,"y":N}` (px viewport),
`{"type":"scroll","dy":N}` (px CSS, + = vers le bas),
`{"type":"key","key":"Enter","mods":["ctrl"]}`.

## 3b. Média — vol e2e (offre/claim/segments/state, ADR-0029)

Trois faits d'abord — ils orientent tout le vol :

- **Le moteur ne décode pas.** Le daemon n'ouvre jamais
  `onionmedia_ffi.dll` : il *offre* les surfaces
  `<video>`/`<audio>`/MSE, l'hôte (app Flutter) *joue*.
  Un vol API pur éprouve le contrat — offre, claim,
  segments, state — jamais les pixels.
- **`EXPLORER_MEDIA_CODECS` au lancement du daemon** —
  obligatoire pour un vol média. Sans elle,
  `MediaSource.isTypeSupported` rend `""` pour **tout**
  type à froid (table `declared_codecs` vide) → tout
  player MSE réel baille au feature-detect
  (`Hls.isSupported()=false` vol 619 — §B-80 — puis
  `=true` vol 620 dès la variable posée) →
  aucune `MediaSource` → pas de surface à claimer.
  Poulet-et-œuf : le claim apprend les codecs mais le
  feature-detect précède tout claim. Liste `;`-séparée
  = `AppConfig.mediaCodecs` de l'app (préfixes nus ou
  MIME `codecs="a,b"` — le `;` interne est toléré) :

  ```powershell
  $env:EXPLORER_MEDIA_CODECS='avc1;hev1;hvc1;vp09;av01;vp08;mp4a;mp3;opus;vorbis;flac;ac3;ipcm'
  ```

  Avec elle : conteneur connu sans `codecs=` → `"maybe"`,
  `codecs=` dont tous les préfixes sont déclarés →
  `"maybe"`, sinon `""` — le player démarre.
- **Le vol vole en REST pur — jamais l'app.** Le
  décodage réel passe par `rest_host_probe` (hôte
  média headless, pont REST↔DLL — b) ci-dessous), qui
  `dlopen` le DLL **déployé** : `terrain-bin/
  onionmedia_ffi.dll` (adjacent à `rest_host_probe.exe`,
  `ONIONMEDIA_FFI_LIB` ou `--dll` sinon). Vérifier le
  md5 du manifest terrain (`Étape 181`, ABI 4) — un
  DLL périmé = constat invalide (règle binaire frais
  §0). L'app Flutter n'entre pas dans la boucle de vol.

### a) Sonde contractuelle — daemon seul, aucune DLL

```powershell
# sonde tout-en-un : nav + pompe + offres + claim d'essai
.\target\release\explorer-cli.exe jprobe <url-page-video>
# → "[media : N offres]" + "- #id kind WxH url (mime)"
#   + "claim #id → true" puis release
```

REST manuel équivalent :

```powershell
Invoke-RestMethod "$BASE/api/tabs/$tab/media"            # offres
Invoke-RestMethod "$BASE/api/tabs/$tab/media/$mid/claim" `
  -Method Post -ContentType 'application/json' `
  -Body '{"codecs":["video/mp4"]}'
Invoke-RestMethod "$BASE/api/tabs/$tab/media/$mid/state" `
  -Method Post -ContentType 'application/json' `
  -Body '{"state":"playing","position_ms":1000,"duration_ms":0,"buffered_ms":0}'
Invoke-RestMethod "$BASE/api/tabs/$tab/media/$mid/release" -Method Post
```

Surface MSE (`"mse":true`, `mseBuffers`) — pull de segments :

```powershell
Invoke-WebRequest "$BASE/api/tabs/$tab/media/$mid/segments?buffer=0&max=1048576" `
  -OutFile seg.bin   # octet-stream ; en-têtes X-Mse-Cursor
                     # (position absolue), X-Mse-Eos:1 (fin),
                     # X-Mse-Pending:1 (re-sonder — pas un bug)
# `?from=N` = seek à la position absolue N ; hors fenêtre → 416
```

Commands JS → hôte : `media_command` en SSE (`play`/`pause`/
`seek`/`volume`/`muted`/`rate`) — tiré quand la page appelle
`.play()` etc. Rapport hôte → `POST …/state` borné ≤ 4 Hz.

**États du contrat mesurés en vol (618/619)** — attendus,
pas des bugs :

- `<video>` **sans `src`** n'est jamais offert — rien à
  claimer (players masquant l'élément au feature-detect
  → `media_surfaces=0`, contractuellement juste).
- Réassigner `src` **tue la surface** : `410` sur l'ancien
  `mid`, nouvelle offre sous un autre id.
- `GET …/segments` sur surface progressive → `{mse:false}`
  (réponse honnête, pas d'invention de segments).
- 2ᵉ `claim` → `409` (first-wins) ; les codecs du claim
  sont persistés dans l'offre **et** alimentent la table
  `isTypeSupported` du document (un claim local débloque
  `"maybe"` au niveau conteneur pour tout le doc).
- Claim immédiat post-offre : le §B-79 « surfaces mortes
  au claim » était un produit de la destruction moteur
  (§B-1), pas un défaut de la couche media — mesuré vol 618.

### b) Lecture réelle — `rest_host_probe` (REST pur, jamais l'app)

`rest_host_probe` = l'hôte média headless : il boucle le
**même** cycle REST (offer→claim→state→release) mais les
rapports d'état sortent du vrai player — c'est la DLL
déployée qui décode, mesurée par trames présentées et
octets PCM (audio en mode pull, aucune carte son).

```powershell
# binaire : terrain-bin\rest_host_probe.exe (adjacent à
#   onionmedia_ffi.dll — dlopen résout tout seul), ou
#   cargo build -r -p onionmedia-ffi --example rest_host_probe
#   (C:\Onion-explorer-media, sortie target\release\examples\)
terrain-bin\rest_host_probe.exe <base> <tab> <mid> [secondes] `
  [--dll <path>] [--codecs "a;b;c"] [--hw] [--seek-ms N]
```

Ce qu'il fait, à chaque étape vérifiable par `GET …/media` :

1. `GET …/media/{mid}` → lit `url`/`mime`/`mse`/`mseBuffers`/
   `mseOffsets` ;
2. `POST …/claim {codecs:[mime de l'offre]}` (`--codecs`
   pour autre chose) ;
3. `dlopen` du DLL → `open_url` (http/https), `open_path`
   (`file:`) ou `open_mse`/`open_mse_multi` (surface
   `mse:true` — `MseSource` pull `GET …/segments` sur le
   **même daemon**, `pts_offset_secs` par rail) ;
4. `play` puis, à ~4 Hz : `POST …/state` avec
   `position_ms`/`duration_ms`/`buffered_ms`/`state`
   **réels** (`Loading`→`playing`, `Stopped`→`ended` ou
   `error` si `player_error`) ;
5. `free` + `POST …/release`. Exit 0 = trames ou PCM
   décodés, 1 sinon.

Sortie à citer au rapport (§B) — exemple mesuré
(mp4 h264 progressif, première exécution du pont) :

```
[offer] #1 mse=false mime="video/mp4" url=…mp4
[claim] → 200 ; [dll] ABI 4
[state] playing pos=0ms
[verdict] frames=60 640x360 pcm=0o pos=1966ms underrun=0
          streams=1 kind=1 codec=H264
[release] → 200
```

→ `GET …/media` montre ensuite `state:{playing,pos:1966}`
**persisté par le moteur** — la preuve que le rapport vient
du decode, pas d'une simulation.

Actifs de test : `C:\Onion-explorer-media\target\bench-media\`
(`onion_bench_*.mp4`/`*.webm` + vecteurs `ac3-`/`wav-`/
`av1-vectors/`) — régénérables via `codec_bench_cli` ;
page minimale `<video src controls>` servie par
`python -m http.server 8011`, ou URL `file:///` directe.

### Verdicts à rapporter (§B)

`GET /api/tabs/1/media` : `claimed:true`,
`state:"playing"`, `position_ms` qui avance entre deux
lectures, `buffered_ms`. SSE `media_offered` porte
`mime`, `mse`, `mseBuffers`.

Décodage (niveau b) : la ligne `[verdict]` de
`rest_host_probe` — `frames` (trames présentées),
`WxH`, `pcm` octets, `pos`, `underrun`, `streams`+codecs.
La distinction avec le niveau a) est entière : a) = rapports
**simulés** (POST state à la main), b) = rapports **mesurés**
sur ce que la DLL a réellement joué.

Pixels : aucun overlay hôte en REST — la vidéo décodée
reste dans le player headless (`frame_pull`/`frame_acquire`
disponibles au besoin). La frame moteur `GET …/frame`
montre la surface `<video>` côté document seulement.

Références terrain : vol 618 (commons.wikimedia.org) —
cycle progressif complet sur site réel (2 surfaces,
claim/409/`{mse:false}`/state/release) ; vol 619
(mdn.github.io) — cycle MSE complet avec octets réels
(`appendBuffer` 64 o → `GET segments` rend les 64 o
exacts, cursors/`?from=`/416/`X-Mse-Eos` justes) ;
vol 620 — §B-80 levée par la variable ci-dessus
(`Hls.isSupported()=true`, manifeste hls.js 5 niveaux
fetché/parsé — attache mort sur gap DOM démo `hls-demo.js`,
pas hls.js core) ; vol 621 — **deux cycles complets sur
sites vidéo réels** (media.ccc.de, coub ×7 surfaces) : la
formule gagnante = `<video>` en markup **SSR** (claim à
coup sûr, aucune race JS). Niveau b) validé en local
(`rest_host_probe` + mp4 h264 : 60/60 trames, état réel
persisté). Reste à mesurer : b) sur site réel (mp4
progressif ccc/coub/commons), `mse:true` décodé (player
sans armure UI — shaka/videojs-http-streaming), PCM
audio `--pull`, `--seek-ms` sur surface vivante.

Claims codecs : l'oracle = `onionmedia_can_play_type`
(`""`/`maybe`/`probable`), table déclarée alignée
Étape 181. Rejets propres attendus — pas des bugs :
h264-high10 / av1-10b (décodeurs 8-bit), eac3 `bsid>10`,
mp2 (Layer II), `video/x-msvideo` (pas de démuxeur AVI),
`video/ogg` (ogv = theora indécodable).

## 4. Ce que l'agent voit — et ne voit pas

**Visible** : le rendu moteur pur — layout, texte, images, couleurs,
peinture incrémentale, frames des sous-contextes iframe.

**Invisible** : le chrome Flutter (omnibox, scrollbar, bannière
d'erreur « Impossible de charger »), le curseur, la mise à l'échelle
DPI. Un bug **de l'app** (position du thumb, clip Dart) est invisible
ici ; un bug **moteur** est toujours visible.

Pour les vrais pixels de la fenêtre (chrome compris) :
`scripts/capture_window.ps1` — capture Win32 de la fenêtre
`explorer_app` → PNG lisible. Combiner les deux : un écart
frame-moteur vs fenêtre-app est un constat en soi (bug UI).

## 5. Règles de vol

- **Verdict = binaire frais** (§8) — rebuildé après le commit visé.
- **Un constat = une issue GitHub** — la procédure « vol humain »
  s'applique : mesurer (`doc_h`/`scroll_y`/pixel/frame PNG jointe),
  jamais « ça a l'air ».
- **Daemon propre à l'agent** — ne jamais attacher un vol agent au
  daemon humain :9377 (les onglets se partageraient).
- **Arrêt** : fermer les onglets (`DELETE /api/tabs/{id}`) puis
  tuer le process par **son PID** — jamais `taskkill /IM` (risque
  de toucher le daemon de l'utilisateur).
- **Watchdog mémoire obligatoire** : un daemon wedged a monté à
  ~7 Go + swap et crashé la station (vol 106). Lancer en tâche de
  fond dès le démarrage du daemon :
  `pwsh -File scripts/vol_memwatch.ps1 -DaemonPid <PID>
  [-MaxMB 5120] [-IntervalSec 15] [-Log <file>]` — kill **par PID**
  au-delà du seuil, journal dans `_vol*_watch.log`. Le wedge
  reste un constat §B à rapporter ; le watchdog protège la
  machine, pas le diagnostic.

## 6. Différences avec le vol humain

| | Humain (app) | Agent (API) |
|---|---|---|
| rendu moteur | ✓ | ✓ identique |
| chrome Flutter | ✓ | via `capture_window.ps1` |
| vrais clics souris | ✓ | `POST input` (équivalent moteur) |
| drag scrollbar | ✓ | — (geste UI, testé en `flutter test`) |
| latence ressentie | ✓ | mesurable via SSE timings |

## 7. Outillage de vol (depuis 2026-10-24)

- **`GET /api/tabs/{id}/console`** — erreurs pipeline par onglet,
  **persistantes** (ring 512) : les erreurs JS de boot restent
  consultables après coup — fini l'écoute SSE en direct pendant
  la nav. Premier réflexe de bissection : `/console` avant de
  lire le DOM.
- **`explorer-cli diff a.png b.png`** — diff pixel (%, boîte
  englobante, grille 4×4) — exit 0 identique / 2 diffère.
- **`explorer-cli save <url> <dir>`** — fixture terrain :
  `frame.png` + `dom.txt` + `layout.txt` + `manifest.txt`
  (url, url finale, version) pour repro offline et bissection.
- **`explorer-cli mcp [port]`** — serveur MCP stdio qui wrappe
  cette API : un agent vole en natif (tabs, viewport, navigate,
  frame, dom, layout, mem, console, input) sans pwsh maison.
- **`scripts/build_terrain.ps1`** — build release des trois
  artefacts + copie `terrain-bin/` + `MANIFEST.txt` (commit,
  date, état arbre) : la preuve de fraîcheur du binaire testé
  (leçon R-75 — jamais de verdict sur un binaire périmé).
- **`scripts/compare_render.ps1 <url> [-Ref ref.png]`** — shot
  moteur + `diff` contre une référence (Chrome/Playwright).
- **En file** : sonde `/net` (verdicts sous-ressources
  CSP/SRI/CORS par URL) — entaille session, file NAV.

### Garde anti-crash — ne jamais `read` un PNG brut

Les frames font 2-6 Mo (1366×900 RGBA). Injectées entières dans le
contexte par l'outil `read`, elles peuvent dépasser la limite de
requête du client → **crash de session non reprenable** (l'image
fautive rejoue à chaque reprise). Observé terrain vol 99.

- Télécharger la capture normalement (`/frame` → PNG preuve).
- Produire la vignette : `pwsh -File scripts/shrink_capture.ps1
  <capture.png>` → `_view_<nom>.png` (~300 Ko, largeur 900 px).
- **`read` uniquement la vignette `_view_*`** — jamais le PNG
  d'origine. Le brut reste la preuve archivée dans `captures/`.

## 8. Traçabilité — synchro avec le corpus de vol

Protocole complet : `C:\OnionBit-vol\SYNTHESE.md` §G.

**Le corpus de vol vit hors du repo** : `C:\OnionBit-vol\`
(dossier dédié, `git init` local SANS remote — versionné
localement, jamais poussé vers GitHub). Tous les vols lisent et
écrivent là : `SYNTHESE.md`, `SYNTHESE_JOURNAL.md`,
`RAPPORT_VOL_*.md`, `logs/`, `captures/` — **peu importe le
worktree qui fait tourner le daemon**. Un worktree n'a jamais
de copie locale du corpus : il accède au canon par chemin
absolu — une copie se diverge, le canon non. L'ancien
`docs/Rapport vol/` du repo est supprimé (gitignoré depuis
`df8888eb` ; la migration préserve l'historique dans le git
local du corpus).

**Règle artefacts — le repo reste propre** : aucun artefact de
vol n'atterrit dans l'arbre du projet — dumps `_nav_*.json`,
scripts `_probe*.py`, logs `vol*-watch.log`/`watch-*.log`,
fixtures téléchargées (`*.js` de site réel), profils froids,
captures : tout part dans `C:\OnionBit-vol\` (ou `%TEMP%` pour
le jetable). `git status` du repo ne doit montrer que du WIP
code — un `??` non-code = artefact mal garé. Dans le corpus,
les caches de profils froids (`prof*/cache/`) sont exclus du
versionning (régénérables, pas de la preuve) — le reste se
commit régulièrement pour garder l'historique.

- Chaque constat du rapport porte `crate` + l'**ID §B** s'il
  correspond à une anomalie catalogue (`→ §B-5`) ; un trou
  nouveau = proposition d'entrée §B au numéro suivant.
- Le rapport indique le **tag de binaire** (`[V1]`/`[V2]`/`[V3]`)
  — un constat sur binaire périmé n'est pas une régression.
- **Occurrence d'une anomalie §B existante → la ligne §B** :
  incrémenter le statut de la ligne au format `vol N : site ×N`
  dans le même passage (le journal §A reste la preuve datée,
  la ligne §B est le compteur). Exception : **§B-14** (matrice
  d'APIs géante) — journal seulement, l'intégrateur fusionne.
- Statuts canoniques : `OUVERT` → `FIX LIVRÉ` → `RÉSOLU`
  (fermé uniquement par re-vol sur binaire post-fix).
- L'agent FIX référence les IDs tâche §E-bis (`J`/`N`/`P`/`T`)
  dans ses commits et met à jour ses lignes §E-bis + statuts §B
  à la clôture.

### Verdicts mur de consentement (Étape 171, ADR-0031)

Depuis `consent_policy` (`EXPLORER_CONSENT_POLICY`,
défaut `reject`), un mur de consentement se lit sous **deux
verdicts distincts** — à déclarer dans le rapport avec la
politique du vol :

- **`consent_wall:soft`** — la page reste un shell inerte
  (`evals_sans_mutation`, doc_h tronqué) alors que le
  consentement est résolu : gate `__tcfapi`/vendor jamais
  servie, balises `type="text/plain"` marquées CMP
  (`optanon-category`, `data-cookieconsent`, `data-purpose`)
  non réarmées, SDK CMP crashé (§B-14) ou figé (§B-48).
  Régression moteur — un site doit booter sur sa partie
  fonctionnelle après « Tout refuser ».
- **`consent_wall:hard`** — le site refuse tout contenu sans
  consentement **d'acceptation** (« consent or pay »,
  hard-gate serveur) : sous `reject` la page bloque
  volontairement. **Pas une régression** — un vrai « Tout
  refuser » produit le même résultat dans un navigateur
  réel ; le constat documente le modèle du site, pas un
  défaut. À comparer sous `accept` en diagnostic : si le
  site s'ouvre, le verdict `hard` tient ; s'il reste inerte
  sous `accept` aussi, reclasser en `soft`/§B-4.

Tout constat consentement exige un **profil froid** (§1) —
un consentement persisté dans un profil chaud masque le mur.
