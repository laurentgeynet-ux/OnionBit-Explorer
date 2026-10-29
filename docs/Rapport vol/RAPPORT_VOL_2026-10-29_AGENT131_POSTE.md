<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 131 : POSTE `[V10-main]`

- **Binaire** : `[V10-main]` — `C:/vol100-wt/target/release/explorer-daemon.exe` à `f20477ba` (JS-C190 — `main` toujours cassé : `mse.rs` non commité ; JS-C191/192/194/195 en attente de build).
- **Mission** : POSTE — laposte, sfr, axa, lyon.fr (mairies).
- **Daemon** : PID **4344** (extraction initiale erronée — grep `9664` a matché port `49664`/lsass ; watchdog a surveillé le mauvais PID — **leçon : matcher `127.0.0.1:PORT` complet**), port 9664 ; arrêt propre, DELETE propres ×4.
- **Verdict** : axa ~80 %, lyon ~85 % (console **vide**), laposte ~70 % (`fetch: url required` — signature §B-60 nouvelle + storm), sfr coquille (`node is from a different document` — adoption cross-doc ratée, bras JS-C185 adjacent).

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| laposte.fr | **~70 %** — 50 entrées | `fetch: url required`, `messages of Undefined`, fetch storm, `addEventListener` ruxit | §B-14 +3 + §B-60 signature |
| sfr.fr | **coquille** — frame 204 | `node is from a different document` | §B-14 adoption résiduelle |
| axa.fr | **~80 %** | `addEventListener of Undefined` ruxit | §B-14 +1 |
| lyon.fr | **~85 %** — console **vide** | 0 entrée | net positif |

## Arrêt
DELETE propres ×4 ; `Stop-Process -Id 4344` (PID réel retrouvé via `netstat 127.0.0.1:9664`).

## Re-vols reportés au vol 132
- **Rebuilder si `mse.rs` commité** → JS-C191/192/194/195.
- §B-14 : `node is from a different document` (adoption cross-doc résiduelle — sfr), `fetch: url required` (laposte), + file.
- §B-59 ×2 sites ; §B-60 fetch storm + `url required` ; §B-56 résiduel lookbehind ; §B-54/§B-23.
