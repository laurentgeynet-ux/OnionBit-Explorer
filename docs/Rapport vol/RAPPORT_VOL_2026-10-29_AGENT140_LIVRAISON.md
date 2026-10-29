<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 140 : LIVRAISON `[V10-main]`

- **Binaire** : `[V10-main]` — `C:/vol100-wt/target/release/explorer-daemon.exe` à `f20477ba` (JS-C190 — `main` toujours cassé : `mse.rs` non commité ; JS-C191..201 en attente).
- **Mission** : LIVRAISON — macif, groupama, chronopost, mondialrelay.
- **Daemon** : PID **15856**, port 9673, watchdog armé (pic 1 335 Mo) ; arrêt propre, DELETE propres ×4.
- **Verdict** : chronopost ~80 % console **vide** ; groupama ~75 % (`dt`/`replace` otBanner) ; mondialrelay ~70 % (`addEventListener` ruxit + `indexOf` hs-analytics + `[object Event]` — sérialisation trou ×2) ; macif 403. §B-14 +3.

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| macif.fr | **403** | `HTTP 403` | externe |
| groupama.fr | **~75 %** — doc_h 5 398 | `dt is not defined`, `replace` otBanner | §B-14 +2 |
| chronopost.fr | **~80 %** — console **vide** | 0 entrée | net positif |
| mondialrelay.fr | **~70 %** — doc_h 14 666 | `addEventListener` ruxit, `indexOf` hs-analytics, `[object Event]`, `id` privacy-center | §B-14 +4 (incl. `[object Event]` trou sérialisation) |

## Arrêt
DELETE propres ×4 ; `Stop-Process -Id 15856` ; watchdog nominal.

## Re-vols reportés au vol 141
- **Rebuilder si `mse.rs` commité** → JS-C191..201.
- §B-14 : `[object Event]`/`[object Object]` sérialisation (mondialrelay/transavia — objet rejeté non dumpé), + file.
- §B-59 ×3 sites ; §B-60 ; §B-56 résiduel ; §B-54/§B-23.
