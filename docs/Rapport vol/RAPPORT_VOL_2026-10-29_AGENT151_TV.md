<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 151 : TV `[V10-main]`

- **Binaire** : `[V10-main]` — `C:/vol100-wt/target/release/explorer-daemon.exe` (HEAD `de24aba1`, JS-C204 inclus).
- **Mission** : TV — mycanal, lcl, intermarche, ubisoft.
- **Daemon** : PID **12716**, port 9684, watchdog armé (nominal ≤685 Mo) ; arrêt propre, DELETE propres ×4.
- **Verdict** : mycanal ~75 % (**fetch storm ×53 — §B-60 3ᵉ site** après mastodon/guardian) ; ubisoft ~75 % + doctipharma ~85 % — **`Invalid URL` inline ×3 sites** (arte, doctipharma, ubisoft — signature inline identique `l@1:5xxx` → bras récurrent §B-11 ou §B-14 à nommer) ; lcl ~70 % (`addEventListener` on Object prismic) ; intermarche 403.

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| mycanal.fr → canalplus.com | **~75 %** — doc_h 12 708 | fetch storm ×53 | **§B-60 ×3 sites** |
| lcl.fr | **~70 %** — doc_h 7 274 | `addEventListener` on Object (prismic) | §B-14 +1 |
| intermarche.com | **403** | `HTTP 403` | externe |
| ubisoft.com | **~75 %** — doc_h 3 350 | `Invalid URL` inline (signature arte/doctipharma) | **bras récurrent ×3 sites** |

## Arrêt
DELETE propres ×4 ; `Stop-Process -Id 12716` ; watchdog nominal.

## Re-vols reportés au vol 152
- **Bras `Invalid URL` inline ×3 sites** (arte/doctipharma/ubisoft — même empreinte `l@1:5xxx, a@1:3xx, W@4:7xxx`) : priorité — 1 fix touche 3+ sites.
- §B-60 fetch storm ×3 sites (mastodon/guardian/mycanal) ; §B-59 ×3 sites ; §B-56/§B-57 résiduels ; §B-54/§B-23 ; §B-1-variante vimeo.
