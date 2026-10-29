<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 159 : EMPLOI `[V10-main]`

- **Binaire** : `[V10-main]` — `C:/vol100-wt/target/release/explorer-daemon.exe` (HEAD `de24aba1`, JS-C204 inclus).
- **Mission** : EMPLOI — francetravail, cadremploi, monster, apec.
- **Daemon** : PID **5696** (extraction netstat ratée au démarrage — watchdog armé en 2ᵉ temps, leçon rappelée), port 9692 ; arrêt propre, DELETE propres ×4.
- **Verdict** : francetravail ~75 % (`value is not a function` ruxit — JS-C200 partiel confirmé ×3) ; cadremploi **coquille** (view 3 Ko — `addEventListener` on Object + fetch err, bras §B-14) ; monster + apec 403. §B-14 +2.

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| francetravail.fr | **~75 %** — doc_h 4 606 | `value is not a function` ruxit | JS-C200 bras suivant ×3 |
| cadremploi.fr | **coquille** — doc_h 8 088 | `addEventListener` on Object (fcms), fetch err | §B-14 +2 |
| monster.fr | **403** | `HTTP 403` | externe |
| apec.fr | **403** | `HTTP 403` | externe |

## Arrêt
DELETE propres ×4 ; `Stop-Process -Id 5696` ; watchdog armé tardivement mais nominal.

## Re-vols reportés au vol 160
- §B-14 : `value is not a function` ruxit (3 sites — axa/francetravail/mondialrelay-adjacent), `addEventListener` on Object (cadremploi fcms + lcl prismic), file connue.
- **§B-62 nouveau** : clubmed crash daemon `memory allocation of 34359738368 bytes failed` (32 Go d'un coup — allocation unique, non-§B-59 fuite progressive).
- §B-60 ×4 sites ; `Invalid URL` bras ×3 ; §B-59 ×3 sites ; §B-56/§B-57 résiduels ; §B-54/§B-23 ; §B-1-variante vimeo.
