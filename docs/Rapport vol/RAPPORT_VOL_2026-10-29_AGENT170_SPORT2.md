<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 170 : SPORT2 `[V10-main]`

- **Binaire** : `[V10-main]` — `C:/vol100-wt/target/release/explorer-daemon.exe` (HEAD `de24aba1`, JS-C204 inclus).
- **Mission** : SPORT2 — science-et-vie, francefootball, rugbyrama, velo101.
- **Daemon** : PID **16024**, port 9703, watchdog armé (pic 789 Mo) ; arrêt propre, DELETE propres ×4.
- **Verdict** : rugbyrama ~80 % + science-et-vie ~80 % + francefootball ~75 % + velo101 ~70 % — **4/4 peintes**, 2 consoles vides. §B-14 +4.

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| science-et-vie.com | **~80 %** — 306 Ko | `id` privacy-center + `Invalid scheme` marfeel | bras récurrents |
| francefootball.fr | **~75 %** — 222 Ko | 0 entrée | net positif |
| rugbyrama.fr | **~80 %** — 310 Ko | gsi 403 | quasi net |
| velo101.com | **~70 %** — 173 Ko | `javascript is not defined`, `moteur inerte` | §B-14 +2 |

## Arrêt
DELETE propres ×4 ; `Stop-Process -Id 16024` ; watchdog nominal.
