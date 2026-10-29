<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 155 : SPORT `[V10-main]`

- **Binaire** : `[V10-main]` — `C:/vol100-wt/target/release/explorer-daemon.exe` (HEAD `de24aba1`, JS-C204 inclus).
- **Mission** : SPORT — maaf, allianz, eurosport, qobuz.
- **Daemon** : PID **1848**, port 9688, watchdog armé (nominal ≤807 Mo) ; arrêt propre, DELETE propres ×4.
- **Verdict** : eurosport ~70 % (`join` inline) ; qobuz ~70 % (`document of Null` — signature récurrente famille §B-14) ; maaf + allianz 403 (assurances anti-bot ×2).

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| maaf.fr | **403** | `HTTP 403` | externe |
| allianz.fr | **403** | `HTTP 403` | externe |
| eurosport.fr | **~70 %** — doc_h 15 421 | `join` inline | §B-14 +1 |
| qobuz.com | **~70 %** — doc_h 5 013 | `document of Null` + 403 gsi | §B-14 +1 |

## Arrêt
DELETE propres ×4 ; `Stop-Process -Id 1848` ; watchdog nominal.

## Re-vols reportés au vol 156
- §B-14 : `join` (eurosport), `document of Null` (qobuz — récurrent), file connue.
- §B-60 ×4 sites ; `Invalid URL` bras ×3 ; §B-59 ×3 sites ; §B-56/§B-57 résiduels ; §B-54/§B-23 ; §B-1-variante vimeo.
