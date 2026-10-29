<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 127 : EXPLOR12 `[V10-main]`

- **Binaire** : `[V10-main]` — `C:/vol100-wt/target/release/explorer-daemon.exe` à `f20477ba` (JS-C190 — `main` toujours cassé : `mse.rs` non commité ; JS-C191+JS-C192 en attente de build).
- **Mission** : EXPLOR12 — engie, ryanair, airbnb, kiabi (secteurs énergie/voyage/locatif/mode).
- **Daemon** : PID **10360**, port 9660, watchdog armé (nominal ≤413 Mo) ; arrêt propre, DELETE propres ×4.
- **Verdict** : 2×403 (engie, kiabi) ; ryanair ~50 % (`MutationObserver: callback requis` — signature nouvelle + `toString` vendor + assets `0 Unknown Error`) ; airbnb coquille (`postTask` + `preloadRestQuery` ×6). **`postTask` confirmé 2ᵉ site** (dna + airbnb) → bras §B-14 récurrent à prioriser (Scheduler API).

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| particulier.engie.fr | **403** | `HTTP 403` | externe |
| ryanair.com | **~50 %** — doc_h=900 | `MutationObserver: callback requis`, `toString` vendor, assets `0 Unknown Error` | §B-14 +2 |
| airbnb.fr | **coquille** — doc_h=900 | `postTask` ×2 + `preloadRestQuery` ×6 | §B-14 +2 (`postTask` 2ᵉ site) |
| kiabi.com | **403** | `HTTP 403` | externe |

## Arrêt
DELETE propres ×4 ; `Stop-Process -Id 10360` ; watchdog nominal.

## Re-vols reportés au vol 128
- **Rebuilder si `mse.rs` commité** → allociné JS-C191 + sephora/france.tv/arte JS-C192.
- §B-14 priorités récurrents : **`postTask` ×2 sites** (dna, airbnb), `currentScript` ×3, `MutationObserver: callback requis` (ryanair), `preloadRestQuery`, + file.
- §B-59 ×3 sites (github, lindependant) ; §B-60 fetch storm ; §B-54/§B-23.
