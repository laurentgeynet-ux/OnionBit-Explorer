<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 130 : EXPLOR14 `[V10-main]`

- **Binaire** : `[V10-main]` — `C:/vol100-wt/target/release/explorer-daemon.exe` à `f20477ba` (JS-C190 — `main` toujours cassé : `mse.rs` non commité ; JS-C191/192/194/195 en attente de build).
- **Mission** : EXPLOR14 — sante.fr, largus, disneylandparis, maisonsdumonde.
- **Daemon** : PID **4064**, port 9663, watchdog armé (pic 1 625 Mo, nominal) ; arrêt propre.
- **Verdict** : argus ~80 % console **vide** ; sante.fr ~75 % (`push`/`get` → §B-14) ; DLP ~65 % (nav 131 s, frame en timeout — pompes occupées, `crisis`/`browserupgrade`/`map`/`oneId`/`a function is required` → §B-14) ; MDM 403.

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| sante.fr | **~75 %** — doc_h 7 323 | `push on ?` + `get of Undefined` (nuxt) | §B-14 +2 |
| largus.fr | **~80 %** — console **vide** | 0 entrée | net positif |
| disneylandparis.com | **~65 %** — nav 131 s, frame timeout | `crisis`/`browserupgrade`/`map`/`oneId`/`a function is required` | §B-14 +5 + pompes chargées |
| maisonsdumonde.com | **403** | `HTTP 403` | externe |

## Arrêt
DELETE propres ; `Stop-Process -Id 4064` ; watchdog nominal.

## Re-vols reportés au vol 131
- **Rebuilder si `mse.rs` commité** → JS-C191 allociné, JS-C192 ×3 sites, JS-C194 maif/caf, JS-C195 dna/airbnb.
- §B-14 : `crisis`/`browserupgrade`/`map`/`oneId`/`a function is required` (DLP), `push`/`get` (sante nuxt), + file.
- §B-59 ×2 sites ; §B-60 fetch storm ; §B-56 résiduel lookbehind (epic) ; §B-54/§B-23.
