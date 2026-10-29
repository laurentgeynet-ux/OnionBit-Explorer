<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 109 : FRMASSA `[V10-main]`

- **Binaire** : `[V10-main]` —
  `C:/vol100-wt/target/release/explorer-daemon.exe` (HEAD
  `636aedc4`, JS-C183 inclus — aucun nouveau fix).
- **Mission** : FRMASSA — couverture grand public FR :
  orange, carrefour, doctolib (re-vol ancien wedge), +
  lachainemeteo/openstreetmap reportés.
- **Daemon** : PID **2292**, port 9642, watchdog armé (RAM
  max ~1 Go — seuil jamais approché) ; arrêt propre,
  DELETE propres ×3.
- **Verdict global** : 3/3 cibles dégradées mais **aucun
  wedge ni fuite** — orange coquille (`HTMLDocument` absent
  → §B-14), carrefour ~50 % (`initContext` + moteur figé),
  doctolib boot figé (`moteur figé` cascade, doc_h=0 —
  disjoncteur tient : API vivante, DELETE propre — vs wedge
  historique). §B-14 +3 (`HTMLDocument`, `initContext`,
  screeb/doctolib cascade).

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| orange.fr | **coquille** — doc_h=151, frame 204 | `HTMLDocument is not defined` (cookie-monster woopic) | **§B-14 +1** (`HTMLDocument` interface absente) |
| carrefour.fr | **~50 %** — nav 71,8 s, doc_h=11 014, page partiellement peinte | `initContext is not a function (on Object)` main-XuKZDRhE + `moteur figé` + `inerte` | §B-14 + freeze éval |
| doctolib.fr | **boot figé** — doc_h=0, frame blanche | `moteur figé — éval dépassée` (country_config inline) puis `éval sautée` ×N (rspack chunks) | wedge historique atténué en freeze scopé : API vivante, DELETE propre — signature §B-48-like |

## Arrêt

DELETE propres ×3 ; `Stop-Process -Id 2292` ; watchdog
nominal (~0,4-1 Go). Sessions voisines intactes.

## Re-vols reportés au vol 110

- lachainemeteo, openstreetmap (tuiles §B-4) — non volés.
- flixbus §B-54 vie-onglet ; vimeo §B-1-variante ; github
  §B-59 (watchdog requis).
- cryptpad/bookshop → §B-23/§B-12 ; uMap/mastodon/tumblr/
  wttj → §B-28/22/19/36.
