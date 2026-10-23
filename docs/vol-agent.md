<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Vol agent — conduire le navigateur par l'API

Procédure pour qu'une piste (PIPE, NAV, JS) joue un **vol humain
sans humain** : les yeux = la frame PNG du moteur, les mains =
l'API REST d'input, la télémétrie = `dom`/`layout`/`mem`/SSE.

> **Prérequis — binaire frais** (règle §8 AGENTS) : un verdict
> visuel n'est valable que sur un daemon buildé **après** le dernier
> commit touché. Attendre `Finished` — jamais de screenshot sur un
> `.exe` périmé.

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
| `GET  /api/tabs/{id}/dom` `/layout` `/mem` | télémétrie |
| `POST /api/tabs/{id}/back` `/forward` `/reload` | historique |
| `GET  /api/events` | SSE — `navigated`, `frame_invalidated`… |

Format `input` : `{"type":"click","x":N,"y":N}` (px viewport),
`{"type":"scroll","dy":N}` (px CSS, + = vers le bas),
`{"type":"key","key":"Enter","mods":["ctrl"]}`.

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

## 6. Différences avec le vol humain

| | Humain (app) | Agent (API) |
|---|---|---|
| rendu moteur | ✓ | ✓ identique |
| chrome Flutter | ✓ | via `capture_window.ps1` |
| vrais clics souris | ✓ | `POST input` (équivalent moteur) |
| drag scrollbar | ✓ | — (geste UI, testé en `flutter test`) |
| latence ressentie | ✓ | mesurable via SSE timings |
