<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 162 : MAIRIES `[V10-main]`

- **Binaire** : `[V10-main]` — `C:/vol100-wt/target/release/explorer-daemon.exe` (HEAD `de24aba1`, JS-C204 inclus).
- **Mission** : MAIRIES — nice, marseille, bordeaux, generali.
- **Daemon** : PID **11912**, port 9695, watchdog armé (pic 978 Mo) ; arrêt propre, DELETE propres ×4.
- **Verdict** : **3/3 mairies consoles vides** — nice ~80 %, marseille ~80 %, bordeaux ~85 % (209 Ko) ; generali coquille (`push on ?` incapsula-family + `supports` addtoany + `instance` lazysizes). §B-14 +3.

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| nice.fr | **~80 %** — doc_h 14 556 | 0 entrée | net positif |
| marseille.fr | **~80 %** — 166 Ko | 0 entrée | net positif |
| bordeaux.fr | **~85 %** — 209 Ko | 0 entrée | net positif |
| generali.fr | **coquille** — 5 Ko | `push on ?`, `supports` addtoany, `instance` lazysizes | §B-14 +3 |

## Arrêt
DELETE propres ×4 ; `Stop-Process -Id 11912` ; watchdog nominal.

## Re-vols reportés au vol 163
- §B-14 : `push on ?` (volotea/generali — incapsula family récurrent), `supports` addtoany (philharmonie/generali), `instance` lazysizes.
- §B-62 ; §B-60 ×4 ; `Invalid URL` ×4 ; §B-59 ×3 ; §B-56/§B-57 résiduels ; §B-54/§B-23 ; §B-1-variante.
