<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 165 : MIX `[V10-main]`

- **Binaire** : `[V10-main]` — `C:/vol100-wt/target/release/explorer-daemon.exe` (HEAD `de24aba1`, JS-C204 inclus).
- **Mission** : MIX — houzz, wamiz, notretemps, mariages.
- **Daemon** : PID **3672**, port 9698, watchdog armé (nominal ≤951 Mo) ; arrêt propre, DELETE propres ×4.
- **Verdict** : houzz ~75 % (console vide) ; mariages ~75 % (**fetch storm ×164 — §B-60 ×5 sites**) ; wamiz ~55 % (`subtle.importKey: unknown algorithm` ×2 — crypto bras §B-14 nouveau) ; notretemps 403. §B-14 +2.

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| houzz.fr | **~75 %** — 68 Ko | 0 entrée | net positif |
| wamiz.com | **~55 %** — 17 Ko | `importKey: unknown algorithm` ×2 | §B-14 bras crypto nouveau |
| notretemps.com | **403** | `HTTP 403` | externe |
| mariages.net | **~75 %** — 131 Ko | fetch storm ×164 | **§B-60 ×5 sites** |

## Arrêt
DELETE propres ×4 ; `Stop-Process -Id 3672` ; watchdog nominal.

## Re-vols reportés au vol 166
- §B-14 : `subtle.importKey unknown algorithm` (wamiz — WebCrypto algo absent, même famille que `secp256k1` caf).
- §B-60 fetch storm **×5 sites** (mastodon/guardian/mycanal/gitlab/mariages) ; §B-62 ; §B-59 ×4 ; `Invalid URL` ×4 ; §B-56/§B-57 résiduels ; §B-54/§B-23 ; §B-1-variante.
