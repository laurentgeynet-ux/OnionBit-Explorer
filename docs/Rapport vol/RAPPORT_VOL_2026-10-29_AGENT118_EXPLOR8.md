<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 118 : EXPLOR8 `[V10-main]`

- **Binaire** : `[V10-main]` — `C:/vol100-wt/target/release/explorer-daemon.exe` (HEAD `7c1f43f7`, JS-C188 inclus).
- **Mission** : EXPLOR8 — exploration : allociné (cinéma), doctissimo (santé), pap.fr (immo).
- **Daemon** : PID **2020**, port 9651, watchdog armé (pic 1 446 Mo, nominal) ; arrêt propre, DELETE propres ×3.
- **Verdict** : **3/3 sites peints** — allociné ~70 % (doc_h 10,8k), doctissimo ~75 % (14,5k), pap.fr ~80 % (view 404 Ko). §B-14 +3 (`addEventListener` sur Array allociné, `toLowerCase of Undefined` doctissimo, `disableUaHints` id5-sync pap) ; heap cap tenu ×2 (1 occurrence par site).

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| allocine.fr | **~70 %** — doc_h=10 803 | `addEventListener on Array` ×2, heap ×1 (549,8 Mo) | §B-14 +1 ; §B-12 cap tenu |
| doctissimo.fr | **~75 %** — doc_h=14 516 | `toLowerCase of Undefined` ×2, heap ×1 (536,8 Mo) | §B-14 +1 ; cap tenu |
| pap.fr | **~80 %** — doc_h=5 476 | `disableUaHints of Undefined` (id5-sync) | §B-14 +1 |

## Arrêt
DELETE propres ×3 ; `Stop-Process -Id 2020` ; watchdog nominal.

## Re-vols reportés au vol 119
- §B-14 file : `IDBRequest` thenable, `FontFaceSet.addEventListener`, `postMessage` 2-args, `addEventListener`/`toLowerCase`/`disableUaHints`/`category`/`appendChild`/`St.tooltip`/`Promise.finally`/`get`/`delete`/`initContext`.
- flixbus §B-54 vie-onglet ; vimeo §B-1-variante ; github §B-59 (watchdog) ; cryptpad §B-23.
