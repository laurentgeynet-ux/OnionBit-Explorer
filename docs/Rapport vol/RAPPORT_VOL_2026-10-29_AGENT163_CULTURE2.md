<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 163 : CULTURE2 `[V10-main]`

- **Binaire** : `[V10-main]` — `C:/vol100-wt/target/release/explorer-daemon.exe` (HEAD `de24aba1`, JS-C204 inclus).
- **Mission** : CULTURE2 — premiere, jeuxactu, bdgest, manga-news.
- **Daemon** : PID **3924**, port 9696, watchdog armé (pic 1 017 Mo) ; arrêt propre, DELETE propres ×4.
- **Verdict** : manga-news **~85 %** (679 Ko — `jQuery`/`writeEmbed`/`getDefaultPage` legacy) ; bdgest ~75 % (`AfficheZone` + heap ×1 capé) ; premiere ~65 % (`install` inline) ; jeuxactu 503. §B-14 +5.

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| premiere.fr | **~65 %** — 47 Ko | `install` inline | §B-14 +1 |
| jeuxactu.com | **503** | `HTTP 503` | externe |
| bdgest.com | **~75 %** — 155 Ko | `AfficheZone`, heap ×1 | §B-14 +2 + §B-12 capé |
| manga-news.com | **~85 %** — **679 Ko** | `jQuery`/`writeEmbed`/`getDefaultPage` | §B-14 +3 |

## Arrêt
DELETE propres ×4 ; `Stop-Process -Id 3924` ; watchdog nominal.

## Re-vols reportés au vol 164
- §B-14 : `jQuery is not defined` legacy (manga-news — script dépendance ordre), `AfficheZone`, `install`, `getDefaultPage`.
- §B-62 ; §B-60 ×4 ; `Invalid URL` ×4 ; §B-59 ×3 ; §B-56/§B-57 résiduels ; §B-54/§B-23 ; §B-1-variante.
