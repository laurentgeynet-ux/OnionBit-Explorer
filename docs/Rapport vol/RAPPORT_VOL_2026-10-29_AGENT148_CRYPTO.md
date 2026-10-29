<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 148 : CRYPTO `[V10-main]`

- **Binaire** : `[V10-main]` — `C:/vol100-wt/target/release/explorer-daemon.exe` (HEAD `d5962969`, JS-C203 inclus).
- **Mission** : CRYPTO — kraken, coinbase, binance, degiro.
- **Daemon** : PID **7952**, port 9681, watchdog armé (pic 1 736 Mo) ; arrêt propre, DELETE propres ×4.
- **Verdict** : degiro ~65 % (`appendChild`/`createGsapMarqueeElement` sur `HTMLDocument` — bras §B-14 méthode-document) ; kraken ~70 % (otBanner `length`) ; coinbase ~50 % (**`get` absent sur `IDBRequest`** ×2 — nouveau bras IDB + heap ×1) ; binance challenge AWS WAF (doc_h=8).

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| kraken.com | **~70 %** — doc_h 7 774 | `length of Null` otBanner | §B-14 +1 |
| coinbase.com | **~50 %** — doc_h 19 847 | **`get` on IDBRequest ×2**, heap ×1 | §B-14 bras IDB nouveau |
| binance.com | **WAF challenge** — doc_h=8 | `encrypt of Undefined`, `AwsWafIntegration` | anti-bot externe |
| degiro.fr | **~65 %** — doc_h 13 127 | `appendChild`/`createGsapMarqueeElement` on HTMLDocument | §B-14 +2 (méthode document) |

## Arrêt
DELETE propres ×4 ; `Stop-Process -Id 7952` ; watchdog nominal.

## Re-vols reportés au vol 149
- §B-14 : `IDBRequest.get` (coinbase — bras IDB suivant), `appendChild`/`createGsapMarqueeElement` on HTMLDocument (degiro), `length` otBanner (kraken/groupama).
- §B-59 ×3 sites ; §B-60 ; §B-56/§B-57 résiduels ; §B-54/§B-23 ; §B-1-variante vimeo.
