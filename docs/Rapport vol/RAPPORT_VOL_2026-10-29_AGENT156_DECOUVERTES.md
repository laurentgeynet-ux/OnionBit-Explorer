<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 156 : DECOUVERTES `[V10-main]`

- **Binaire** : `[V10-main]` — `C:/vol100-wt/target/release/explorer-daemon.exe` (HEAD `de24aba1`, JS-C204 inclus).
- **Mission** : DECOUVERTES — futura-sciences, geo.fr, numerama, sciencesetavenir.
- **Daemon** : PID **17224**, port 9689, watchdog armé (pic 1 491 Mo) ; arrêt propre, DELETE propres ×4.
- **Verdict** : futura ~85 % (466 Ko, heap ×1 capé) ; numerama ~75 % (`moteur js inerte` ×16 opti-digital — signature nouvelle) ; sciencesetavenir ~70 % (`instance` script.min) ; geo.fr **coquille** (sourcepoint `consentRejection`/`Fallback mandatory` — bras §B-14 récurrent). §B-14 +4.

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| futura-sciences.com | **~85 %** — doc_h 6 504 | heap ×1 sparteo | §B-12 capé |
| geo.fr | **coquille** — view 3 Ko | `consentRejection`/`Fallback mandatory` sourcepoint | §B-14 +2 (bras sourcepoint récurrent) |
| numerama.com | **~75 %** — doc_h 11 979 | `moteur js inerte` ×16 opti-digital | signature nouvelle (inerte ≠ erreur nommée) |
| sciencesetavenir.fr | **~70 %** — doc_h 12 064 | `instance` script.min | §B-14 +1 |

## Arrêt
DELETE propres ×4 ; `Stop-Process -Id 17224` ; watchdog nominal.

## Re-vols reportés au vol 157
- §B-14 : `moteur js inerte` opti-digital (numerama — à nommer), `consentRejection`/`Fallback mandatory` sourcepoint (geo/capital/cuisineaz — bras récurrent), `instance`.
- §B-60 ×4 sites ; `Invalid URL` bras ×3 ; §B-59 ×3 sites ; §B-56/§B-57 résiduels ; §B-54/§B-23 ; §B-1-variante vimeo.
