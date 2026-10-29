<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 168 : RADIO `[V10-main]`

- **Binaire** : `[V10-main]` — `C:/vol100-wt/target/release/explorer-daemon.exe` (HEAD `de24aba1`, JS-C204 inclus).
- **Mission** : RADIO — bfmtv, rmc, franceinter, fip.
- **Daemon** : PID **14920**, port 9701, watchdog armé — **tué par le watchdog** (2 445→5 771 Mo sur franceinter, **§B-59 ×5 sites**).
- **Verdict** : bfmtv ~80 % + rmc ~75 % **consoles vides** (283/204 Ko) ; **franceinter = §B-59 ×5** (groupe radio france — même famille que francemusique) ; fip non atteint.

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| bfmtv.com | **~80 %** — 283 Ko | 0 entrée | net positif |
| rmc.bfmtv.com | **~75 %** — 204 Ko | 0 entrée | net positif |
| franceinter.fr | **§B-59 ×5 (5ᵉ site)** — kill | nav 50 s | fuite groupe Radio France |
| fip.fr | non atteint | — | reporté |

## §B-59 — signature stable (×5 sites)

```
github       :   500→9 648 Mo (~15 s)
lindependant :   486→5 996 Mo (~45 s)
francemusique:   483→5 331 Mo (~46 s)
actugaming   : 1 648→7 806 Mo (~15 s)
franceinter  : 2 445→5 771 Mo (~50 s)
```

Groupe Radio France = 2/5 sites — la pompe de flux audio vit.

## Re-vols reportés au vol 169
- §B-59 caractérisé ; fip reporté ; §B-62 ; §B-60 ×5 ; `Invalid URL` ×5 ; §B-56/57 résiduels ; §B-54/§B-23 ; §B-1-variante.
