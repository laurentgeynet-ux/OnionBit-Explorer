<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 124 : SPORTSRADIO `[V10-main]`

- **Binaire** : `[V10-main]` — `C:/vol100-wt/target/release/explorer-daemon.exe` à `f20477ba` (JS-C190 — `main` toujours cassé : `mse.rs` non commité).
- **Mission** : SPORTSRADIO — lequipe (sport), rtl/europe1 (radio FR — secteur vierge), nicematin (régio).
- **Daemon** : PID **15472**, port 9657, watchdog armé (pic 1 550 Mo, nominal) ; arrêt propre, DELETE propres ×4.
- **Verdict** : **4/4 sites peints** (rtl ~80 %, lequipe ~70 %, europe1 ~75 %, nicematin ~75 %). §B-14 +7 dont signatures nouvelles : `postMessage: cannot clone host object` (lequipe permutive), `Body has already been consumed` (lequipe), `spécificateur irrésolvable` module relatif (lequipe), `hydrateWhenVisible` (lequipe — astro/islands ?).

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| lequipe.fr | **~70 %** — doc_h 21 976 | `hydrateWhenVisible`, `postMessage clone host`, `Body consumed`, `spécificateur irrésolvable`, `getFeatureStoreValue` | §B-14 +5 (signatures riches) |
| rtl.fr | **~80 %** — doc_h 18 685 | `getTech on Object` | §B-14 +1 |
| europe1.fr | **~75 %** — doc_h 12 174 | `Invalid scheme`/`post`/`type` (marfeel-sdk) + `undefined` ×3 | §B-14 +3 |
| nicematin.com | **~75 %** — doc_h 4 123 | `displayedMegaMenu`/`brand`/`footer` | §B-14 +3 |

## Arrêt
DELETE propres ×4 ; `Stop-Process -Id 15472` ; watchdog nominal.

## Re-vols reportés au vol 125
- **Rebuilder si `mse.rs` commité** → allociné JS-C191.
- §B-14 : `postMessage clone host object`, `Body has already been consumed`, `spécificateur irrésolvable`, `hydrateWhenVisible`, `getTech`, `Invalid scheme`/`post`/`type` marfeel, `displayedMegaMenu`/`brand`/`footer`, `defineProperty` non-object, `document of Null`, + file existante.
- §B-60 fetch storm ; §B-54/§B-1-variante ; §B-59 github ; §B-23 cryptpad.
