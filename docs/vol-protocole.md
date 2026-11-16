# Vol — protocole opérationnel (distillé)

> Version légère de `docs/vol-agent.md` (913 lignes — la
> référence complète, ouverte en post-mortem). Ce fichier
> porte la séquence de vol et les règles qui préviennent
> les accidents ; le cas par cas renvoie « vol-agent §N ».
> Terrain lu au décollage : `Explorer-data/vols/FILE.md`.

## 0. Checklist de lancement

```powershell
# 0.1 arbre propre + HEAD connu
git status ; git rev-parse HEAD

# 0.2 binaire contractuel — worktree de vol PERSISTANT
# (jamais d'exe deviné dans un autre arbre — un daemon
# spawné depuis l'exe d'un autre worktree s'annonce dans
# SON registre ; le résolveur vit dans spawn_daemon.ps1,
# preuve = exe.log).
git worktree add --detach C:\OnionBit-Explorer-vol HEAD   # 1 fois
git -C C:\OnionBit-Explorer-vol checkout <sha-du-vol>     # par vol
Push-Location C:\OnionBit-Explorer-vol
cargo build --release -p explorer-daemon   # attendre « Finished »
Pop-Location

# 0.3 daemon dédié — port 9385+ (jamais 9377 = humain),
# profil déclaré, fenêtre cachée, canal `session` (défaut —
# meurt avec la session, signature lisible ; `-Via wmi` =
# détaché, parenté wmiprvse tuée par certaines politiques)
$env:EXPLORER_VIEWPORT='1366x900'
$env:EXPLORER_PROFILE='vol:vol<N>'
$env:EXPLORER_MEM_DIAG='1'
pwsh -File scripts/spawn_daemon.ps1 -Port 9385 `
  -Repo C:\OnionBit-Explorer-vol `
  -LogDir "Explorer-data\vols\vol<N>\logs"

# 0.4 joblimit obligatoire — <pid> = « daemon PID <n> »,
# JAMAIS le « cmd watchdog PID »
pwsh -File scripts/vol_joblimit.ps1 -TargetPid <pid> -LimitMB 5120

# 0.5 artefacts — rien hors Explorer-data\vols\vol<N>\
#   logs → vol<N>\logs\  (daemon + flight/death + watch)
#   PNG  → vol<N>\captures\<cible>.png
#   geom → vol<N>\captures\<cible>.geom.json (étape 288)
#   rapport → vols\rapports\RAPPORT_VOL_<date>_AGENT<n>_<TAG>.md
```

## 1. Séquence de vol

```powershell
# onglet VIDE puis nav explicite — {url} de POST /tabs n'est
# jamais navigué (vol 625)
curl -s -X POST http://127.0.0.1:9385/api/tabs        # → id
curl -s -X POST http://127.0.0.1:9385/api/tabs/1/navigate `
  -d '{"url":"https://site"}' -H "Content-Type: application/json"

# yeux : frame → vignette, JAMAIS read le PNG brut
# (2-6 Mo → crash session — vol 99 ; shrink_capture.ps1
#  → _view_*.png ~300 Ko, lire uniquement la vignette)
curl -s http://127.0.0.1:9385/api/tabs/1/frame -o f.png
pwsh -File scripts/shrink_capture.ps1 f.png

# premier réflexe bissection : /console AVANT le DOM
curl -s http://127.0.0.1:9385/api/tabs/1/console
# géométrie par cible : /api/tabs/1/geometry → .geom.json
```

## 2. Endpoints essentiels (carte complète : vol-agent §3)

| Endpoint | Rôle |
|---|---|
| `GET /tabs/{id}/frame` `?x&y&w&h` | yeux (région zoomée) |
| `POST /tabs/{id}/input` `click/scroll/key` | mains |
| `GET /tabs/{id}/console` | erreurs persistantes (ring 512) |
| `GET /tabs/{id}/dom` `/layout` `/mem` | télémétrie — **404 transitoire** = apply en cours, retenter |
| `GET /tabs/{id}/geometry` | oracle `geo_diff` (étape 288) |
| `GET /tabs/{id}/media` `…/segments?peek=` | média — `peek` lecture seule, JAMAIS `from` (vol 627) |
| `GET /api/events` | SSE |
| `POST /api/shutdown` | arrêt graceful (étape 296) |

## 3. Règles dures

- **Verdict = binaire frais** — `Finished` vu + `exe.log`
  cité ; jamais sur WIP ni exe qui traîne.
- **Arrêt = graceful d'abord** : `POST /api/shutdown` →
  `taskkill /PID /F` en dernier recours seulement — jamais
  `/IM` (daemons voisins d'autres sessions). PID depuis le
  pidfile `daemon-<port>.pid` ou l'annonce, pas à main levée.
- **Avant tout kill** : lire `Explorer-data\daemons\` —
  vivacité = pid + image + **port lié sous ce pid** (284) ;
  vérifier TOUS les registres (un par worktree : main, perf,
  chaque worktree de vol).
- **Job Object** : `vol_joblimit` dès le départ (il borne,
  ne tue pas — innocenté §B-135). `vol_memwatch` = chasses
  de fuite documentées seulement (sa signature = kill externe).
- **Profil froid** pour tout constat consentement
  (`consent_wall:soft|hard` — vol-agent §8).
- **Un constat = mesure** (`doc_h`, console, frame jointe) —
  jamais « ça a l'air ».

## 4. Signatures de mort (détail : vol-agent §7)

| Constat | Classe |
|---|---|
| sentinelle `panic:`/`rss_guard`/`console:*`/`shutdown:api` | nommée — lire la sentinelle |
| `alive` + beats sains + `exit.log` présent | **kill sélectif** externe (calibration `taskkill /F` → `EXIT:1`) |
| arbre entier mort (daemon+watchdog+frères) + `exit.log` ABSENT | **job-close** — session lanceuse fermée (canal `session` : contrat signé, pas mystère) |
| `alive` + beat figé sur balise | stack overflow/spin — `drain`/`host` nomment le site |
| `alive` + `commit` au plafond | OOM/alloc-abort |
| beats vivants, `drain` fixe, `calls` figé | wedge — `session.ping` EAP nomme le verbe tenu |

## 5. Clôture de vol

1. `git rev-parse main` relu (sha binaire vs commits post).
2. Géométrie : `ref_capture --list` + `geo_diff` par paire
   `*.geom.json` → ligne « x % divergents » au rapport
   (`hors-mesure` = non jugé, pas une casse).
3. Rapport gabarit `rapports\_TEMPLATE.md` — en-tête
   (binaire/PID/port/profil), constats avec ID §B ou
   proposition nouvelle entrée, statuts `OUVERT→FIX LIVRÉ
   →RÉSOLU` (fermé seulement par re-vol post-fix).
4. **SYNTHESE.md** : ligne §A journal + compteur §B
   (`vol N : site ×N`) ; **FILE.md** : ligne d'état mise à
   jour. CLAIMS libérée.
5. Daemon arrêté graceful (annonce retirée = preuve).

Post-mortem, cas par cas, signatures complètes, consentement,
MCP/EAP, gabarits : `docs/vol-agent.md`.
