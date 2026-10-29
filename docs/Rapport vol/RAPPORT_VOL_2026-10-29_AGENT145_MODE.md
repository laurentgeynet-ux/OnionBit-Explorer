<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 145 : MODE `[V10-main]`

- **Binaire** : `[V10-main]` — `C:/vol100-wt/target/release/explorer-daemon.exe` (HEAD `507b1b9a`, JS-C191..202 inclus).
- **Mission** : MODE — laredoute, spartoo, ikea (+ hermes reporté).
- **Daemon** : PID **4880**, port 9678, watchdog armé (nominal ≤1 060 Mo) ; arrêt propre, DELETE propres ×3.
- **Verdict** : spartoo ~80 % (347 Ko, `$`/`length`/`value` → §B-14) ; ikea ~70 % (`ikea.authentication` + `preferredStore` + `Unsupported layoutSize=0` → §B-14 nouvelle signature bento) ; laredoute 403.

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| laredoute.fr | **403** | `HTTP 403` | externe |
| spartoo.com | **~80 %** — doc_h 10 550 | `$`/`length`/`value` | §B-14 +3 |
| ikea.com | **~70 %** — doc_h 29 424 | `ikea.authentication`, `preferredStore`, **`Unsupported layoutSize=0`** (bento) | §B-14 +3 (signature layout nouvelle) |

## Arrêt
DELETE propres ×3 ; `Stop-Process -Id 4880` ; watchdog nominal.

## Re-vols reportés au vol 146
- JS-C202 `User Timing` — site à nommer ; JS-C193 MSE (autre cible que vimeo).
- §B-14 : `Unsupported layoutSize=0` (bento ikea — rail layout), `preferredStore`, `ikea.authentication`.
- §B-59 ×3 sites ; §B-60 ; §B-56/§B-57 résiduels ; §B-54/§B-23 ; §B-1-variante vimeo.
