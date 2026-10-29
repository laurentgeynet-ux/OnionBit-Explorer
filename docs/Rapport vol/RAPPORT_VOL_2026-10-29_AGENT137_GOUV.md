<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 137 : GOUV `[V10-main]`

- **Binaire** : `[V10-main]` — `C:/vol100-wt/target/release/explorer-daemon.exe` à `f20477ba` (JS-C190 — `main` toujours cassé : `mse.rs` non commité ; JS-C191..199 en attente).
- **Mission** : GOUV — assemblee-nationale, economie.gouv, truffaut, zoomalia.
- **Daemon** : PID **14056**, port 9670, watchdog armé (pic 1 198 Mo) ; arrêt propre, DELETE propres ×4.
- **Verdict** : truffaut ~70 %, assemblee ~70 % (`createHTMLDocument` + ctor-sans-new + `childNodes` → §B-14), economie ~65 % (`setAttribute`), zoomalia ~55 % (404 images cdn-cgi). §B-14 +5.

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| assemblee-nationale.fr | **~70 %** — nav 175 s | `class ctor without new`, `createHTMLDocument on ?`, `childNodes` | §B-14 +3 (dont `createHTMLDocument` JS-C185 adjacent) |
| economie.gouv.fr | **~65 %** — doc_h 8 499 | `setAttribute of Undefined` | §B-14 +1 |
| truffaut.com | **~70 %** — nav 149 s | `id of Undefined` privacy-center | §B-14 +1 |
| zoomalia.com | **~55 %** — doc_h 5 428 | 404 images cdn-cgi ×10 | externe |

## Arrêt
DELETE propres ×4 ; `Stop-Process -Id 14056` ; watchdog nominal.

## Re-vols reportés au vol 138
- **Rebuilder si `mse.rs` commité** → JS-C191..199.
- §B-14 : `createHTMLDocument`, `class ctor without new`, `childNodes`, `setAttribute`, `id` privacy-center, + file.
- §B-59 ×2 sites ; §B-60 ; §B-56 résiduel ; §B-54/§B-23.
