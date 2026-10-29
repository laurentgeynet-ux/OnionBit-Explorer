<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 129 : EXPLOR13 `[V10-main]`

- **Binaire** : `[V10-main]` — `C:/vol100-wt/target/release/explorer-daemon.exe` à `f20477ba` (JS-C190 — `main` toujours cassé : `mse.rs` non commité ; JS-C191/192/194 en attente de build).
- **Mission** : EXPLOR13 — lesechos, liberation, lavoixdunord, epicgames.
- **Daemon** : PID **6256**, port 9662, watchdog armé (nominal ≤930 Mo) ; arrêt propre, DELETE propres ×4.
- **Verdict** : epic ~85 % (643 Ko — **`invalid regex` reproduit** : `(?<=-RC\.\d+)\.g[0-9a-f]+` lookbehind conditionnel → résiduel §B-56 nouveau motif) ; lavoixdunord ~75 % (`push of Undefined` ×5) ; lesechos boot mort (doc_h=8, `name of Undefined` Akamai) ; liberation 403. §B-14 +4.

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| lesechos.fr | **boot mort** — doc_h=8 | `name of Undefined` (akamai) | anti-bot JS |
| liberation.fr | **403** | `HTTP 403` | externe |
| lavoixdunord.fr | **~75 %** — doc_h 15 680 | `push`/`0 of Undefined` ×5 | §B-14 +2 |
| store.epicgames.com | **~85 %** — doc_h 6 248 | `pipeTo source stalled`, **`invalid regex` lookbehind conditionnel**, `unsafeSVG child bindings`, `replace` otBanner | **§B-56 résiduel** + §B-14 +3 |

## Arrêt
DELETE propres ×4 ; `Stop-Process -Id 6256` ; watchdog nominal.

## Re-vols reportés au vol 130
- **Rebuilder si `mse.rs` commité** → allociné JS-C191, ×3 JS-C192, maif/caf JS-C194.
- §B-56 résiduel : lookbehind conditionnel `(?<=...)` imbriqué (epic).
- §B-14 : `pipeTo source stalled`, `unsafeSVG`, `push`/`0`/`name of Undefined`, + file.
- §B-59 ×2 sites ; §B-60 fetch storm ; §B-54/§B-23.
