<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 133 : INTL `[V10-main]`

- **Binaire** : `[V10-main]` — `C:/vol100-wt/target/release/explorer-daemon.exe` à `f20477ba` (JS-C190 — `main` toujours cassé : `mse.rs` non commité ; JS-C191/192/194/195/197 en attente).
- **Mission** : INTL — guardian, reuters, elpais, lemoniteur.
- **Daemon** : PID **8860**, port 9666, watchdog armé (nominal ≤411 Mo) ; arrêt propre, DELETE propres ×4.
- **Verdict** : vol à 403 répétés (reuters 401, elpais 403, lemoniteur 403 — anti-bot/paywall) ; **guardian ~80 %** — doc_h 13 230, page riche, `not signed in` + fetch storm dédup (§B-60 famille). §B-14 inchangé.

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| theguardian.com | **~80 %** — doc_h 13 230 | `not signed in` + fetch storm | §B-60 |
| reuters.com | **401** | `HTTP 401` | paywall externe |
| elpais.com | **403** | `HTTP 403` | externe |
| lemoniteur.fr | **403** | `HTTP 403` | externe |

## Arrêt
DELETE propres ×4 ; `Stop-Process -Id 8860` ; watchdog nominal.

## Re-vols reportés au vol 134
- **Rebuilder si `mse.rs` commité** → JS-C191/192/194/195/197.
- §B-60 fetch storm (guardian + mastodon) ; §B-59 ×2 sites ; §B-54/§B-23.
