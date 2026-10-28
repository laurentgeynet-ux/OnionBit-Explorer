<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 102 : REVALID11 `[V10-main]`

- **Binaire** : `[V10-main]` —
  `C:/vol100-wt/target/release/explorer-daemon.exe` (build
  `Finished`, HEAD `a1764264` — inclut **R-131 §B-58**,
  JS-C180, É155, É154/153).
- **Commits post-binaire** : `0a71a528` (JS-C181 §B-57 bras 2
  `fn.length` natives) + doc `2f3c048b` — arrivés en fin de
  vol → priorité 1 vol 103 (manomano/caniuse).
- **Mission** : REVALID11 — re-vols : §B-58 letterboxd
  (R-131), §B-54/§B-30 flixbus (É154), §B-28 uMap
  (`816f7c9`), §B-22 mastodon, §B-8 glassdoor, §B-19 tumblr,
  §B-36 wttj.
- **Daemon** : 2 process — PID 22432 port 9632 (wedged
  flixbus → kill PID), PID 21440 port 9633 (fin de vol,
  arrêt propre) ; journaux `_vol102_daemon_wedged.log` +
  `_vol102_daemon2.log`. RAM 410 Mo à `{"tabs":[]}`.
- **Verdict global** : **2 RÉSOLU** — §B-58 (letterboxd :
  « Wildwood » vertical propre, miroir disparu) et §B-8
  (glassdoor : contenu peint à scroll_y=1600 vs frame
  blanche). **§B-54 REPRODUIT ×2** — flixbus wedge « pendant
  vie » à l'identique, É154 toujours non mesurable.
  **§B-28/§B-19/§B-36 PERSISTANTS** (uMap chaîne `#map`
  1366×0 ; tumblr `main-*.js` jamais évalués ; wttj bloc
  géant). §B-22 non atteint (mastodon mort en amont :
  127× `fetch: network error` + `addEventListener` sur
  `IDBRequest` → §B-14).

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| letterboxd.com | **§B-58 RÉSOLU** — « Wildwood (2026) » peint vertical net (vs miroir ×2 vols) — R-131 validé terrain | — | — |
| flixbus.fr | **§B-54 REPRODUIT ×2** — navigate 52,4 s puis wedge vie-onglet : `event.*` budget tiré (elapsed 16,8 s) mais `dispatch` 6 s continue, API morte (tabs ×4 + DELETE timeout), kill PID | — | É154 non mesurable (jamais post-DELETE) — borne queue insuffisante, le *contenu* des phases reste non borné |
| umap.openstreetmap.fr | **§B-28 PERSISTANT** — chaîne `html/body/#map` = 1366×0 (×6 boîtes), tuiles OSM ~100 px coin haut-gauche | — | projection `dvh` insuffisante — motif à isoler (`min-height`/`flex` wrapper ?) |
| mastodon.social/about | **§B-22 non atteint** — page blanche doc_h=0 ; app morte avant Redux | 127× `fetch: network error` (dédup ×N par occurrence) + `addEventListener is not a function` **on IDBRequest** | → §B-14 nouvelle entrée (`IDBRequest.addEventListener` absent) ; fetch tous en échec (bras à nommer — API stream/request ?) |
| glassdoor.fr | **§B-8 RÉSOLU** — scroll_y=1600 : contenu texte peint (vs frame blanche vol 26) | — | rendu peu stylé (CSS partielle — autre sujet) |
| tumblr.com | **§B-19 PERSISTANT** — 4 évals inline courts, `main-*.js` jamais évalués ; carte blanche, app non montée | `appendChild: not a node` (cmp-bundle) | drain de scripts dynamiques toujours absent + §B-14 `appendChild` |
| welcometothejungle.com | **§B-36 PERSISTANT** — bloc jaune géant ~plein viewport (« tech. » seul visible) | — | élément peint à échelle massive — SVG/hero hors boîte confirmé ×2 vols |

## Preuves marquantes

### §B-54 — signature stable, borne insuffisante

```
22:45:41 event.* au-delà du budget — queue différée (B-54) elapsed_ms=16786
22:45:47 event.dispatch us=6 069 248   ← reprend après defer
GET /api/tabs → 000 ×4, DELETE → 000, kill PID 22432
```

Même profil qu'au vol 100 : la queue est différée mais chaque
phase longue continue — wedge « pendant vie ». À borner le
contenu des phases (pas seulement leur cadence).

### §B-22 → dépendances amont (mastodon)

```
127× unhandledrejection: fetch: network error   (dédup ×N)
unhandledrejection: TypeError: addEventListener is not a
    function (on IDBRequest)  at render-Bl7ln8HX.js
doc_h=0, frame blanche — createStore jamais atteint
```

## Arrêt des daemons

- PID 22432 (9632) : wedge flixbus → kill forcé (PID ciblé).
- PID 21440 (9633) : DELETE propres ×5, `Stop-Process`, RAM
  410 Mo `{"tabs":[]}`.

## Re-vols reportés au vol 103

- **manomano / caniuse** → §B-57 bras 2 : CORRECTIF JS-C181
  `0a71a528` (`fn.length` absent des natives) — post-binaire
  → **priorité 1**.
- **flixbus** → §B-54 : wedge vie-onglet — borne du contenu
  des phases `event.*` (à remonter en file NAV).
- **uMap** → §B-28 : chaîne height à isoler (dump layout
  dispo : `logs/vol102_umap_layout.txt`).
- **mastodon** → §B-22 dépend : bras `fetch network error` +
  `IDBRequest.addEventListener` (§B-14).
- tumblr → §B-19 ; wttj → §B-36 (ouverts, preuves fraîches).
- cryptpad/bookshop/airfrance → §B-23/§B-12/§B-56-octal
  (reportés, sans correctif).
