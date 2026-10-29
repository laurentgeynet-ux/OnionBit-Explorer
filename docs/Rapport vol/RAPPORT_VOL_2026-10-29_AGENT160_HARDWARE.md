<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 160 : HARDWARE `[V10-main]`

- **Binaire** : `[V10-main]` — `C:/vol100-wt/target/release/explorer-daemon.exe` (HEAD `de24aba1`, JS-C204 inclus).
- **Mission** : HARDWARE — caradisiac, ldlc, materiel.net, rueducommerce.
- **Daemon** : PID **17100**, port 9693, watchdog armé (nominal ≤557 Mo) ; arrêt propre, DELETE propres ×4.
- **Verdict** : caradisiac ~80 % (230 Ko, `pubConteneurDesktop`/`target` consent-raiser + `id` privacy-center) ; ldlc ~75 % (`Invalid scheme`/`isConnected`/`tooltip` on S) ; materiel.net ~70 % (`Cannot call a class as a function` — bras §B-14 ctor-sans-new familier) ; rueducommerce ~60 % (`ctor without new` + `firstChild` + `Invalid scheme`). §B-14 +7.

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| caradisiac.com | **~80 %** — doc_h 7 237 | `pubConteneurDesktop`, `target` consent-raiser, `id` privacy-center | §B-14 +3 |
| ldlc.com | **~75 %** — doc_h ~9k | `Invalid scheme`, `isConnected`, `tooltip` on S | §B-14 +3 |
| materiel.net | **~70 %** | `Cannot call a class as a function` ×2 | §B-14 ctor-sans-new (bras assemblée) |
| rueducommerce.fr | **~60 %** | `ctor without new`, `firstChild of Null`, `Invalid scheme` | §B-14 +3 |

## Arrêt
DELETE propres ×4 ; `Stop-Process -Id 17100` ; watchdog nominal.

## Re-vols reportés au vol 161
- §B-14 : `ctor without new`/`Cannot call a class as a function` (bras récurrent — assemblée/materiel/rdc ×3 sites), `Invalid scheme` (recoupe §B-11 ×N), `tooltip` on S (ldlc — bras custom-elements adjacents).
- §B-62 alloc 32 Go clubmed ; §B-60 ×4 sites ; `Invalid URL` bras ×3 ; §B-59 ×3 sites ; §B-56/§B-57 résiduels ; §B-54/§B-23 ; §B-1-variante vimeo.
