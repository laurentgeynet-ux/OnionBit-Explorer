<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 128 : BANQUE `[V10-main]`

- **Binaire** : `[V10-main]` — `C:/vol100-wt/target/release/explorer-daemon.exe` à `f20477ba` (JS-C190 — `main` toujours cassé : `mse.rs` non commité ; JS-C191/192/194 en attente de build).
- **Mission** : BANQUE — societegenerale, meteofrance, sarenza, creditmutuel.
- **Daemon** : PID **15000**, port 9661, watchdog armé (nominal ≤556 Mo) ; arrêt propre, DELETE propres ×4.
- **Verdict** : SG ~65 % (**`deeply nested` reproduit** sur public-dca.js — §B-57 résiduel nouveau chemin) ; meteofrance ~70 % ; sarenza ~55 % (`moteur figé` + cloudinary 401s) ; creditmutuel coquille (console vide mais non peint — suspect §B-4 silence). §B-14 +4 (`hasAttribute on HTMLDocument` SG, `invalid token`/`JSON` mf, `swmApi` sw).

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| societegenerale.fr | **~65 %** | `swmApi` ×3, **`deeply nested`** public-dca, `hasAttribute on HTMLDocument` | §B-57 résiduel + §B-14 +2 |
| meteofrance.com | **~70 %** | `writeEmbed`, `invalid token` ×N, JSON token | §B-14 +1 |
| sarenza.com | **~55 %** | `moteur figé` + cloudinary 401 ×2 | freeze |
| creditmutuel.fr | **coquille** — console vide | 0 entrée (`pas de journal`) | suspect §B-4 silence |

## Arrêt
DELETE propres ×4 ; `Stop-Process -Id 15000` ; watchdog nominal.

## Re-vols reportés au vol 129
- **Rebuilder si `mse.rs` commité** → allociné JS-C191, sephora/france.tv/arte JS-C192, maif/caf JS-C194.
- §B-14 : `hasAttribute on HTMLDocument`, `invalid token`, `MutationObserver: callback requis`, `postTask` ×2 sites, `currentScript` ×3, `secp256k1`, bras modules, + file.
- §B-59 ×2 sites ; §B-60 fetch storm ; §B-54/§B-23.
