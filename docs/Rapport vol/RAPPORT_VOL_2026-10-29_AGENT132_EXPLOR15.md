<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 132 : EXPLOR15 `[V10-main]`

- **Binaire** : `[V10-main]` — `C:/vol100-wt/target/release/explorer-daemon.exe` à `f20477ba` (JS-C190 — `main` toujours cassé : `mse.rs` non commité ; JS-C191/192/194/195 en attente).
- **Mission** : EXPLOR15 — pappers, estrepublicain, capital, ouigo.
- **Daemon** : PID **6540** (extraction `127.0.0.1:PORT` corrigée), port 9665, watchdog armé (pic 1 291 Mo) ; arrêt propre, DELETE propres ×4.
- **Verdict** : estrepublicain ~75 % (**`postTask` 3ᵉ site** — SDK prsmedia partagé avec dna), capital ~70 % (sourcepoint consentRejection), ouigo ~65 % (Turnstile warning + 404 image), pappers ~75 % (`Invalid scheme` — §B-11 résiduel). §B-14 +4.

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| pappers.fr | **~75 %** | `Invalid scheme` | §B-11 résiduel |
| estrepublicain.fr | **~75 %** — doc_h 24 276 | `ORDONNANCEUR_NO_CONFIG` + `postTask` (prsmedia — **3ᵉ site**) | §B-14 bras récurrent |
| capital.fr | **~70 %** — doc_h 20 026 | `value undefined` + `consentRejection` sourcepoint | §B-14 +2 |
| ouigo.com | **~65 %** — doc_h 5 682 | Turnstile tag + 404 image | externe |

## Arrêt
DELETE propres ×4 ; `Stop-Process -Id 6540` ; watchdog nominal.

## Re-vols reportés au vol 133
- **Rebuilder si `mse.rs` commité** → JS-C191/192/194/195 (dont `postTask` ×3 sites).
- §B-11 résiduel `Invalid scheme` (pappers — bras entrées à nommer).
- §B-14 : `consentRejection`, `value is not a function: undefined`, + file.
- §B-59 ×2 sites ; §B-60 ; §B-54/§B-23.
