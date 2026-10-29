<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 149 : AUTO `[V10-main]`

- **Binaire** : `[V10-main]` — `C:/vol100-wt/target/release/explorer-daemon.exe` (HEAD `d5962969`, JS-C203 inclus).
- **Mission** : AUTO — peugeot, renault, tesla, gog.
- **Daemon** : PID **15812**, port 9682, watchdog armé (pic 1 411 Mo) ; arrêt propre, DELETE propres ×4.
- **Verdict** : peugeot ~75 % + renault ~70 % **consoles vides** ; tesla ~65 % (`N`/`G@\n@;\x15` non fonction — **strings corrompues `Str(462321)`** — signature interne à nommer) ; gog ~60 % (`observe` on Object — 2ᵉ site après leclerc). §B-14 +4.

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| peugeot.fr | **~75 %** — doc_h 14 275 | 0 entrée | net positif |
| renault.fr | **~70 %** — doc_h 12 057 | 0 entrée | net positif |
| tesla.com | **~65 %** — doc_h 7 948 | `N`/`G@\n@;\x15` non fonction sur **`Str(462321)`**, `textContent of Null` | strings corrompues (§B-14 ou parser) |
| gog.com | **~60 %** — doc_h 9 387 | `observe` on Object | §B-14 +1 (2ᵉ site) |

## Arrêt
DELETE propres ×4 ; `Stop-Process -Id 15812` ; watchdog nominal.

## Re-vols reportés au vol 150
- §B-14 : `observe` on Object (gog + leclerc — bras récurrent), `Str(462321)` strings corrompues (tesla), `IDBRequest.get`, `appendChild` on HTMLDocument, `length` otBanner.
- §B-59 ×3 sites ; §B-60 ; §B-56/§B-57 résiduels ; §B-54/§B-23 ; §B-1-variante vimeo.
