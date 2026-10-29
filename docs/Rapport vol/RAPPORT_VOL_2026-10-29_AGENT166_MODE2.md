<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 166 : MODE2 `[V10-main]`

- **Binaire** : `[V10-main]` — `C:/vol100-wt/target/release/explorer-daemon.exe` (HEAD `de24aba1`, JS-C204 inclus).
- **Mission** : MODE2 — vogue, elle, vanityfair, dezeen.
- **Daemon** : PID **15472**, port 9699, watchdog armé (pic 1 707 Mo) ; arrêt propre, DELETE propres ×4.
- **Verdict** : **moteur figé ×2** — vogue + vanityfair : évals dépassées puis `moteur js inerte` (60/64 s nav — bras §B-14 inerte récurrent) ; elle 403 ; dezeen ~60 % (`Invalid URL` — bras récurrent ×5). §B-14 +3.

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| vogue.fr | **coquille** — 17 Ko | `moteur figé`/`inerte` | bras §B-14 inerte (×2 avec numerama) |
| elle.fr | **403** | `HTTP 403` | externe |
| vanityfair.fr | **coquille** — 14 Ko | `moteur figé`/`inerte` | même bras |
| dezeen.com | **~60 %** — 34 Ko | `Invalid URL` inline | bras récurrent ×5 |

## Arrêt
DELETE propres ×4 ; `Stop-Process -Id 15472` ; watchdog nominal.

## Re-vols reportés au vol 167
- §B-14 : bras `moteur figé/inerte` (vogue+vf+doctolib+ numerama — évals longues), `Invalid URL` **×5 sites**.
- §B-60 ×5 ; §B-62 ; §B-59 ×4 ; §B-56/§B-57 résiduels ; §B-54/§B-23 ; §B-1-variante.
