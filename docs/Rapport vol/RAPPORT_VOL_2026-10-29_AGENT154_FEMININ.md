<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 154 : FEMININ `[V10-main]`

- **Binaire** : `[V10-main]` — `C:/vol100-wt/target/release/explorer-daemon.exe` (HEAD `de24aba1`, JS-C204 inclus).
- **Mission** : FEMININ — journaldesfemmes, aufeminin, ter.sncf, parents.
- **Daemon** : PID **10384**, port 9687, watchdog armé (pic 1 395 Mo) ; arrêt propre, DELETE propres ×4.
- **Verdict** : journaldesfemmes ~75 % + parents ~70 % **consoles vides** (doc_h 34,3k) ; aufeminin ~75 % (marfeel `Invalid scheme`/`hasWebVitals`/`stableUserId` → §B-14 récurrent) ; ter.sncf 403. §B-14 +3.

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| journaldesfemmes.fr | **~75 %** — doc_h 9 043 | 0 entrée | net positif |
| aufeminin.com | **~75 %** — doc_h 12 272 | `Invalid scheme`/`hasWebVitals`/`stableUserId` marfeel | §B-14 +3 (signature marfeel récurrente) |
| ter.sncf.com | **403** | `HTTP 403` | externe |
| parents.fr | **~70 %** — doc_h 34 317 | 0 entrée | net positif |

## Arrêt
DELETE propres ×4 ; `Stop-Process -Id 10384` ; watchdog nominal.

## Re-vols reportés au vol 155
- §B-14 : `hasWebVitals`/`stableUserId`/`Invalid scheme` marfeel (aufeminin+corriere/yves-rocher — bras SDK récurrent).
- §B-60 ×4 sites ; `Invalid URL` bras ×3 ; §B-59 ×3 sites ; §B-56/§B-57 résiduels ; §B-54/§B-23 ; §B-1-variante vimeo.
