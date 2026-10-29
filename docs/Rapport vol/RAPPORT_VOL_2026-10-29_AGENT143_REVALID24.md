<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 143 : REVALID24 `[V10-main]`

- **Binaire** : `[V10-main]` — `C:/vol100-wt/target/release/explorer-daemon.exe` (HEAD `507b1b9a`, JS-C191..202 inclus).
- **Mission** : REVALID24 — suite re-vols : maif (JS-C194), SG (JS-C198), lequipe (JS-C196), orpi (JS-C201).
- **Daemon** : PID **3872**, port 9676, watchdog armé (pic 1 168 Mo) ; arrêt propre, DELETE propres ×4.
- **Verdict** : **4/4 fixes validés** — maif console **vide** (`javaEnabled`/`AbortSignal.timeout` disparus, JS-C194) ; SG `hasAttribute` disparu (JS-C198, `deeply nested` résiduel) ; lequipe `postMessage clone host` disparu (JS-C196, reste `spécificateur irrésolvable` bras modules + `Body consumed`) ; orpi `replace`/`Translator` disparus (JS-C201, `WebAssembly unsupported` ×12 nouvelle signature). **Bilan vague JS-C191..202 : 9/9 bras validés en 2 vols.**

## Synthèse

| Cible | Fix | Verdict | Reste |
|---|---|---|---|
| maif.fr | JS-C194 | **RÉSOLU** — console **vide** | rien |
| societegenerale.fr | JS-C198 | **RÉSOLU** | `deeply nested` §B-57, `swmApi` |
| lequipe.fr | JS-C196 | **RÉSOLU** | `spécificateur irrésolvable` (bras modules), `Body consumed`, `name`/`getHash`/`getFeatureStoreValue` |
| orpi.com | JS-C201 | **RÉSOLU** | `WebAssembly unsupported` ×12 (kleio — signature nouvelle §B-14/rail wasm), `id` privacy-center |

## Arrêt
DELETE propres ×4 ; `Stop-Process -Id 3872` ; watchdog nominal.

## Re-vols reportés au vol 144
- JS-C200 ruxit (`Worker.prototype`), JS-C202 (`User Timing` — site à nommer), JS-C193 MSE (vimeo/dailymotion ?).
- §B-59 ×3 sites ; §B-60 fetch storm ; §B-56 résiduel lookbehind ; §B-57 résiduel ×3 ; §B-54/§B-23.
