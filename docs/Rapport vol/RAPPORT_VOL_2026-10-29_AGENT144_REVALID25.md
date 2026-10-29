<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 144 : REVALID25 `[V10-main]`

- **Binaire** : `[V10-main]` — `C:/vol100-wt/target/release/explorer-daemon.exe` (HEAD `507b1b9a`, JS-C191..202 inclus).
- **Mission** : REVALID25 — axa/mondialrelay (JS-C200 Worker.prototype ruxit), dailymotion (`Promise.finally`), vimeo (JS-C193 MSE + §B-1-variante sous watchdog).
- **Daemon** : PID **4852**, port 9677, watchdog armé ; **wedged sur vimeo** — kill PID.
- **Verdict** : axa `addEventListener`-ruxit disparu → `value is not a function` (maillon suivant — JS-C200 partiel validé) ; dailymotion `finally`/`on` toujours présents (bras §B-14 hors périmètre — promesse étrangère injectée, non résolu) ; **vimeo §B-1-variante PERSISTANT** : nav >120 s, API morte, RAM **stable 1 319 Mo** — wedge pur sans fuite (distinct §B-59) ; MSE JS-C193 non testable (le boot meurt avant).

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| axa.fr | **§B-14 ruxit progressé** — `addEventListener` disparu | `value is not a function: undefined` | maillon suivant |
| dailymotion.com | **§B-14 `finally`/`on` persistant** | `config`, `finally`/`on` non fonction, `object not iterable` | promesse étrangère injectée |
| vimeo.com | **§B-1-variante PERSISTANT** — nav >120 s, wedge API, RAM stable | — | wedge non-mémoire |

## Arrêt
`Stop-Process -Id 4852` ; watchdog nominal.

## Re-vols reportés au vol 145
- JS-C202 `User Timing` — site à nommer ; JS-C193 MSE réel (autre cible que vimeo ?).
- dailymotion `finally`/`on` — promesse étrangère (repro iframe requis).
- §B-59 ×3 sites ; §B-60 ; §B-56/§B-57 résiduels ; §B-54/§B-23 ; §B-1-variante vimeo.
