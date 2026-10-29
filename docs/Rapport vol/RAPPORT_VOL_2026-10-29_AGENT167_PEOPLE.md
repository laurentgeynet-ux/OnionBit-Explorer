<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 167 : PEOPLE `[V10-main]`

- **Binaire** : `[V10-main]` — `C:/vol100-wt/target/release/explorer-daemon.exe` (HEAD `de24aba1`, JS-C204 inclus).
- **Mission** : PEOPLE — telerama, gala, voici, public.
- **Daemon** : PID **12192**, port 9700, watchdog armé (pic 1 159 Mo) ; arrêt propre, DELETE propres ×4.
- **Verdict** : public ~80 % (504 Ko), telerama ~75 % (`children`/`forecastUrl`/`cannot convert`) ; gala + voici **coquilles sourcepoint** (`consentRejection`/`Fallback mandatory`/`forEach on Promise`/`FigAnalytics` — bras récurrent ×5 sites). §B-14 +6.

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| telerama.fr | **~75 %** — 281 Ko | `children`/`forecastUrl`/`cannot convert` | §B-14 +3 |
| gala.fr | **coquille** — 3 Ko | `FigAnalytics`/`forEach on Promise`/`destructure null` | §B-14 +3 |
| voici.fr | **coquille** — 3 Ko | sourcepoint `consentRejection`/`Fallback mandatory` | bras récurrent ×5 |
| public.fr | **~80 %** — 504 Ko | `id` privacy-center | §B-14 +1 |

## Arrêt
DELETE propres ×4 ; `Stop-Process -Id 12192` ; watchdog nominal.
