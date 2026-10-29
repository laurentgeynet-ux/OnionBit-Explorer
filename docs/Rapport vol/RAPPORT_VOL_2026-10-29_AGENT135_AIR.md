<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 135 : AIR `[V10-main]`

- **Binaire** : `[V10-main]` — `C:/vol100-wt/target/release/explorer-daemon.exe` à `f20477ba` (JS-C190 — `main` toujours cassé : `mse.rs` non commité ; JS-C191/192/194/195/197/198 en attente).
- **Mission** : AIR — eurostar, transavia, easyjet, kingjouet.
- **Daemon** : PID **5900**, port 9668, watchdog armé (nominal ~400 Mo) ; arrêt propre, DELETE propres ×4.
- **Verdict** : **transavia ~85 %** (view 622 Ko, unhandledrejection `[object Object]` — trou de sérialisation console à nommer) ; eurostar « Client Challenge » anti-bot ; easyjet + kingjouet 403. Vol calme, rien moteur nouveau hors sérialisation.

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| eurostar.com | **challenge** — doc_h=16 | « Client Challenge » | anti-bot externe |
| transavia.com | **~85 %** — doc_h 953 | `unhandledrejection [object Object]` | trou sérialisation (objet non dumpé) — §B-14 adjacent |
| easyjet.com | **403** | `HTTP 403` | externe |
| kingjouet.com | **403** | `HTTP 403` | externe |

## Arrêt
DELETE propres ×4 ; `Stop-Process -Id 5900` ; watchdog nominal.

## Re-vols reportés au vol 136
- **Rebuilder si `mse.rs` commité** → JS-C191/192/194/195/197/198.
- `unhandledrejection [object Object]` — sérialiser l'objet rejeté (transavia).
- §B-59 ×2 sites ; §B-60 ; §B-56 résiduel ; §B-54/§B-23.
