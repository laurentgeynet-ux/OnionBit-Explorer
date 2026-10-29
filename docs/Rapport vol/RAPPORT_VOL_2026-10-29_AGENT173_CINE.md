<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 173 : CINE `[V10-main]` + WIP ctor_fresh

- **Binaire** : `C:/vol100-wt/target/release/explorer-daemon.exe` — HEAD `de24aba1` + WIP `Reflect.construct`.
- **Mission** : CINE — ecranlarge, topsante, autoplus, gentside.
- **Daemon** : PID **4324**, port 9706, watchdog armé (pic 1 328 Mo) ; arrêt propre, DELETE propres ×4.
- **Verdict** : topsante ~80 % (362 Ko — `is_desktop`/`$`/`is_mobile` legacy jQuery) ; autoplus ~70 % (`id` privacy-center) ; ecranlarge ~60 % (`hasWebVitals` marfeel — bras récurrent) ; gentside frame 204 (nav 84 s, moteur inerte). §B-14 +6.

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| ecranlarge.com | **~60 %** — 29 Ko | `hasWebVitals` marfeel | bras récurrent |
| topsante.com | **~80 %** — 362 Ko | `is_desktop`/`$`/`is_mobile` | §B-14 +3 |
| autoplus.fr | **~70 %** — 74 Ko | `id` privacy-center | bras récurrent |
| gentside.com | **frame 204** — nav 84 s | console vide | moteur inerte |

## Arrêt
DELETE propres ×4 ; `Stop-Process -Id 4324` ; watchdog nominal.
