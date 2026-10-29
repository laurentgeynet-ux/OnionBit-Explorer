<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 158 : ECO `[V10-main]`

- **Binaire** : `[V10-main]` — `C:/vol100-wt/target/release/explorer-daemon.exe` (HEAD `de24aba1`, JS-C204 inclus).
- **Mission** : ECO — challenges, usinenouvelle, lsa-conso, clubmed.
- **Daemon** : PID **5108**, port 9691, watchdog armé ; **daemon crashé en plein nav clubmed** (pas le watchdog : RAM tombée à 2 Mo puis PID disparu — crash pur, signature nouvelle à nommer).
- **Verdict** : challenges ~80 % (256 Ko, `get` nuxt) ; usinenouvelle + lsa-conso 403 ; clubmed = **crash daemon** pendant la nav (nav_ms 69 s → mort processus entier, frame/delete 000).

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| challenges.fr | **~80 %** — doc_h 13 401 | `get of Undefined` nuxt | §B-14 +1 |
| usinenouvelle.com | **403** | `HTTP 403` | externe |
| lsa-conso.fr | **403** | `HTTP 403` | externe |
| clubmed.fr | **CRASH DAEMON** — nav 69 s → processus mort | nav jamais retournée | crash inédit §B (non-mémoire) |

## Arrêt
PID disparu de lui-même ; watchdog a vu le crash et s'est arrêté proprement.

## Re-vols reportés au vol 159
- **Crash daemon clubmed** — première occurrence non-mémoire/non-watchdog ; à reproduire pour signature (log `_vol158_daemon.log`).
- §B-60 ×4 sites ; `Invalid URL` bras ×3 ; §B-59 ×3 sites ; §B-56/§B-57 résiduels ; §B-54/§B-23 ; §B-1-variante vimeo.
