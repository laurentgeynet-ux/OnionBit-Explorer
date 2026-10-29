<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 161 : VOYAGE `[V10-main]`

- **Binaire** : `[V10-main]` — `C:/vol100-wt/target/release/explorer-daemon.exe` (HEAD `de24aba1`, JS-C204 inclus).
- **Mission** : VOYAGE — latribune, volotea, brittany-ferries, accor.
- **Daemon** : PID **13268**, port 9694, watchdog armé (nominal ≤741 Mo) ; arrêt propre, DELETE propres ×4.
- **Verdict** : latribune ~80 % (`Invalid URL` — bras récurrent ×4 sites) ; accor ~70 % (`tryParBridge` ×2) ; volotea **coquille incapsula** (`push on ?` — challenge JS incapsula, même famille que eurostar) ; brittany-ferries coquille (view 7 Ko, console vide — SSR probable). §B-14 +2.

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| latribune.fr | **~80 %** — doc_h 13 495 | `Invalid URL` inline (4ᵉ site — bras récurrent) | §B-14 +1 |
| volotea.com | **incapsula** — doc_h=900 | `push on ?` | challenge anti-bot |
| brittany-ferries.fr | **coquille** — view 7 Ko | console vide | SSR probable |
| all.accor.com | **~70 %** | `tryParBridge` ×2 | §B-14 +1 |

## Arrêt
DELETE propres ×4 ; `Stop-Process -Id 13268` ; watchdog nominal.

## Re-vols reportés au vol 162
- §B-14 : `Invalid URL` inline **×4 sites** (arte/doctipharma/ubisoft/latribune — bras prioritaire), `tryParBridge` (accor), `push on ?` incapsula (volotea — famille challenge).
- §B-62 alloc 32 Go ; §B-60 ×4 sites ; §B-59 ×3 sites ; §B-56/§B-57 résiduels ; §B-54/§B-23 ; §B-1-variante vimeo.
