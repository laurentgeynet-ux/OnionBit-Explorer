<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 139 : INSTIT `[V10-main]`

- **Binaire** : `[V10-main]` — `C:/vol100-wt/target/release/explorer-daemon.exe` à `f20477ba` (JS-C190 — `main` toujours cassé : `mse.rs` non commité ; JS-C191..201 en attente).
- **Mission** : INSTIT — senat, nicolas, but (+ philharmonie reportée).
- **Daemon** : PID **14500**, port 9672, watchdog armé (pic 1 124 Mo) ; arrêt propre, DELETE propres ×3.
- **Verdict** : senat ~80 % (`find`/`toLowerCase` → §B-14) ; nicolas ~60 % (`setFormValue` sur ElementInternals → §B-14 nouvelle signature, `toLowerCase` ×4) ; but 403.

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| senat.fr | **~80 %** — doc_h 3 719 | `find`/`toLowerCase` ×2 | §B-14 +2 |
| nicolas.com | **~60 %** — doc_h 3 551 | **`setFormValue` on ElementInternals** + `toLowerCase` ×4 | §B-14 +2 (ElementInternals signature nouvelle) |
| but.fr | **403** | `HTTP 403` | externe |

## Arrêt
DELETE propres ×3 ; `Stop-Process -Id 14500` ; watchdog nominal.

## Re-vols reportés au vol 140
- **Rebuilder si `mse.rs` commité** → JS-C191..201 (lang/dir, Worker WebIDL).
- §B-14 : `setFormValue` sur ElementInternals (custom elements form), `find`/`toLowerCase`, + file.
- §B-59 ×3 sites ; §B-60 ; §B-56 résiduel ; §B-54/§B-23.
