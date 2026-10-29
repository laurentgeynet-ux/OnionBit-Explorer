<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 147 : JEU `[V10-main]`

- **Binaire** : `[V10-main]` — `C:/vol100-wt/target/release/explorer-daemon.exe` (HEAD `d5962969`, JS-C203 inclus).
- **Mission** : JEU — winamax, betclic, fortuneo, bforbank.
- **Daemon** : PID **8244**, port 9680, watchdog armé (nominal ≤777 Mo) ; arrêt propre, DELETE propres ×4.
- **Verdict** : **4/4 cibles peintes, 2 consoles vides** — fortuneo ~85 % (view 726 Ko), bforbank ~80 % console **vide**, winamax ~80 % console **vide**, betclic ~75 % (`request`/`length` + fetch err → §B-14). Post-GC la campagne peint de plus en plus propre.

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| winamax.fr | **~80 %** — doc_h 4 122 | 0 entrée | net positif |
| betclic.fr | **~75 %** — doc_h 3 649 | `request`/`length of Undefined`, fetch err | §B-14 +2 |
| fortuneo.fr | **~85 %** — doc_h 18 416 | `define property apiClientVersion` | §B-14 +1 |
| bforbank.com | **~80 %** — doc_h 10 649, console **vide** | 0 entrée | net positif |

## Arrêt
DELETE propres ×4 ; `Stop-Process -Id 8244` ; watchdog nominal.

## Re-vols reportés au vol 148
- §B-14 : `request`/`length` (betclic), `define property apiClientVersion` (fortuneo), + file (`find of Null` senat, `ctor without new` assemblée, `tooltip`/`modal`/`secp256k1` caf).
- §B-59 ×3 sites ; §B-60 fetch storm ; §B-56/§B-57 résiduels ; §B-54/§B-23 ; §B-1-variante vimeo.
