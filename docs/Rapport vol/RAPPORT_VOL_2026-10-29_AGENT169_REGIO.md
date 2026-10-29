<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 169 : REGIO `[V10-main]`

- **Binaire** : `[V10-main]` — `C:/vol100-wt/target/release/explorer-daemon.exe` (HEAD `de24aba1`, JS-C204 inclus).
- **Mission** : REGIO — letelegramme, larepubliquedespyrenees, sudradio, radioclassique.
- **Daemon** : PID **12692**, port 9702, watchdog armé (pic 1 531 Mo) ; arrêt propre, DELETE propres ×4.
- **Verdict** : lrp ~80 % (264 Ko — `push`/`require`/`define` legacy) ; radioclassique ~75 % (85 Ko, console vide) ; letelegramme ~60 % (`moteur inerte` bras récurrent) ; sudradio ~55 % (`instance`/`missing input`/`object is not a function` axeptio). §B-14 +7.

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| letelegramme.fr | **~60 %** — 41 Ko | `moteur inerte` (bras récurrent ×3) | §B-14 |
| larepubliquedespyrenees.fr | **~80 %** — 264 Ko | `push`, `require`, `define` | §B-14 +3 |
| sudradio.fr | **~55 %** | `instance`, `missing input`, `object is not a function` axeptio | §B-14 +3 |
| radioclassique.fr | **~75 %** — 85 Ko | 0 entrée | net positif |

## Arrêt
DELETE propres ×4 ; `Stop-Process -Id 12692` ; watchdog nominal.
