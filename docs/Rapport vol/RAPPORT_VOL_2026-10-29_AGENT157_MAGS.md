<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 157 : MAGS `[V10-main]`

- **Binaire** : `[V10-main]` — `C:/vol100-wt/target/release/explorer-daemon.exe` (HEAD `de24aba1`, JS-C204 inclus).
- **Mission** : MAGS — caminteresse, nationalgeographic, korben, konbini.
- **Daemon** : PID **8332**, port 9690, watchdog armé (nominal ≤725 Mo) ; arrêt propre, DELETE propres ×4.
- **Verdict** : korben ~85 % + konbini ~80 % **consoles vides** (325/293 Ko) ; natgeo ~65 % (`isDev` inline) ; caminteresse **coquille** (sourcepoint `consentRejection`/`Fallback mandatory` — 3ᵉ occurrence du bras). §B-14 +3.

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| caminteresse.fr | **coquille** — doc_h 17 705 | `consentRejection`/`Fallback mandatory` sourcepoint | §B-14 bras récurrent ×3 (geo/capital/cuisineaz) |
| nationalgeographic.fr | **~65 %** — doc_h 10 129 | `isDev` inline | §B-14 +1 |
| korben.info | **~85 %** — doc_h 14 446 | 0 entrée | net positif |
| konbini.com | **~80 %** — doc_h 4 625 | 0 entrée | net positif |

## Arrêt
DELETE propres ×4 ; `Stop-Process -Id 8332` ; watchdog nominal.

## Re-vols reportés au vol 158
- §B-14 : bras sourcepoint `consentRejection`/`Fallback mandatory` (3 sites — priorité, 1 fix touche tout le CMP), `isDev`.
- §B-60 ×4 sites ; `Invalid URL` bras ×3 ; §B-59 ×3 sites ; §B-56/§B-57 résiduels ; §B-54/§B-23 ; §B-1-variante vimeo.
