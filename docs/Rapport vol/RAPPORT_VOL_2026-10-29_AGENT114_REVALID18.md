<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 114 : REVALID18 `[V10-main]`

- **Binaire** : `[V10-main]` — `C:/vol100-wt/target/release/explorer-daemon.exe` (build `Finished`, HEAD `d4fdf768` — inclut **JS-C186** `CSSStyleDeclaration` borné aux noms CSS).
- **Mission** : REVALID18 — jvc (bras `hasOwnProperty` §B-14 vol 103).
- **Daemon** : PID **14580**, port 9647, watchdog armé (nominal) ; arrêt propre, DELETE propre.
- **Verdict** : **JS-C186 validé terrain** — `hasOwnProperty is not a function (on CSSStyleDeclaration)` **disparu** sur jvc (vs vol 103) ; erreur suivante `insertAdjacentHTML: bad position` (hp.js) → §B-14 file ; doc_h=9 924, page ~peinte ; 0 wedge.

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| jeuxvideo.com | **§B-14 `hasOwnProperty` RÉSOLU** | `insertAdjacentHTML: bad position` + `fetch network error` | §B-14 suivante (insertAdjacentHTML) |

## Arrêt
DELETE propre ; `Stop-Process -Id 14580` ; watchdog nominal.

## Re-vols reportés au vol 115
- §B-14 file : `insertAdjacentHTML` (jvc), `IDBRequest.addEventListener` (mastodon), `appendChild`/`St.tooltip`/`Promise.finally`/`get`/`delete`/`initContext`.
- flixbus §B-54 vie-onglet ; vimeo §B-1-variante ; github §B-59 (watchdog).
- cryptpad → §B-23 ; uMap/tumblr/wttj → §B-28/19/36.
