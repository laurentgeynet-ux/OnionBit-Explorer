<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 153 : OCCASION `[V10-main]`

- **Binaire** : `[V10-main]` — `C:/vol100-wt/target/release/explorer-daemon.exe` (HEAD `de24aba1`, JS-C204 inclus).
- **Mission** : OCCASION — autoscout24, lacentrale, interencheres, ecole-directe.
- **Daemon** : PID **4820**, port 9686, watchdog armé (pic 1 009 Mo) ; arrêt propre, DELETE propres ×4.
- **Verdict** : autoscout24 ~75 % + interencheres ~70 % **consoles vides** ; lacentrale 403 ; ecole-directe **boot mort** (doc_h=0 — `export outside a module` sur bundle `scripts-NZZ6WKOO.js` → bras modules §B-14 + `getElementsByTagNameNS` on HTMLElement absent).

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| autoscout24.fr | **~75 %** — doc_h 7 084 | 0 entrée | net positif |
| lacentrale.fr | **403** | `HTTP 403` | externe |
| interencheres.com | **~70 %** — doc_h 5 266 | 0 entrée | net positif |
| ecoledirecte.com | **boot mort** — doc_h=0 | `export outside a module`, `getComponent`, `getElementsByTagNameNS` | §B-14 bras modules + NS |

## Arrêt
DELETE propres ×4 ; `Stop-Process -Id 4820` ; watchdog nominal.

## Re-vols reportés au vol 154
- §B-14 : `export outside a module` (ecole-directe bundle détecté non-module → bras modules §B-14 ×2 avec `spécificateur irrésolvable`), `getElementsByTagNameNS` absent.
- §B-60 ×4 sites ; `Invalid URL` bras ×3 ; §B-59 ×3 sites ; §B-56/§B-57 résiduels ; §B-54/§B-23 ; §B-1-variante vimeo.
