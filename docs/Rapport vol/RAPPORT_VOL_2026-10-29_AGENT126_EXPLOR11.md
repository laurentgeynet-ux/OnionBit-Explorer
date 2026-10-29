<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 126 : EXPLOR11 `[V10-main]`

- **Binaire** : `[V10-main]` — `C:/vol100-wt/target/release/explorer-daemon.exe` à `f20477ba` (JS-C190 — `main` toujours cassé : `mse.rs` non commité ; **JS-C192 livré** `c4cdf710` — invariant `currentScript` ×3 sites, à re-voler dès build réparé).
- **Mission** : EXPLOR11 — ouest-france, dna, action, leroymerlin.
- **Daemon** : PID **6064**, port 9659, watchdog armé (pic 1 873 Mo, nominal) ; arrêt propre, DELETE propres ×4.
- **Verdict** : ouest-france ~80 % (doc_h 39k, `push of Undefined` ×3) ; dna ~65 % (`postTask` absent — Scheduler API §B-14 nouvelle signature) ; **action quasi parfait** (console VIDE, view 611 Ko) ; leroymerlin 403.

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| ouest-france.fr | **~80 %** — doc_h 39 132 | `push of Undefined` ×3 + `undefined not a function` | §B-14 +2 |
| dna.fr | **~65 %** — doc_h 21 513 | `ORDONNANCEUR_NO_CONFIG` + `postTask` (Scheduler API) | §B-14 +2 (signature nouvelle) |
| action.com | **~90 %** — console **vide**, view 611 Ko | 0 entrée | net positif |
| leroymerlin.fr | **403** | `HTTP 403` | externe |

## Arrêt
DELETE propres ×4 ; `Stop-Process -Id 6064` ; watchdog nominal.

## Re-vols reportés au vol 127
- **Rebuilder si `mse.rs` commité** → allociné JS-C191 + **sephora/france.tv/arte JS-C192** (currentScript ×3).
- §B-14 : `postTask` (Scheduler API — dna), `push of Undefined`, `secp256k1`, `no module loader`/`spécificateur irrésolvable` (bras modules), + file.
- §B-59 §B-60 §B-54 §B-23 connus.
