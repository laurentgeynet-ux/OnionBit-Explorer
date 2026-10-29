<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 150 : REVALID27 `[V10-main]`

- **Binaire** : `[V10-main]` — `C:/vol100-wt/target/release/explorer-daemon.exe` (HEAD `d5962969` + **JS-C204 copié** protos EventSource/MessagePort/SharedWorker/MutationObserver, `Finished` 4m32s).
- **Mission** : REVALID27 — mondialrelay + laposte (triggers ruxit `addEventListener`), hermes, doctipharma.
- **Daemon** : PID **2524**, port 9683, watchdog armé (nominal ≤800 Mo) ; arrêt propre, DELETE propres ×4.
- **Verdict** : **JS-C204 validé ×2 sites** — mondialrelay `addEventListener` ruxit disparu (reste `in requires an object` hs-analytics + `[object Event]` sérialisation) ; laposte **50 → 3 entrées** (ruxit disparu, reste `fetch: url required` + `messages` + FetchError). doctipharma ~85 % (`Invalid URL` — même signature qu'arte, bras §B-11/§B-14 adjacent). hermes 403.

## Synthèse

| Cible | Fix | Verdict | Reste |
|---|---|---|---|
| mondialrelay.fr | JS-C204 | **RÉSOLU** — `addEventListener` ruxit disparu | `in requires an object`, `[object Event]` trou |
| laposte.fr | JS-C204 | **RÉSOLU** — 50→3 entrées | `fetch: url required`, `messages`, FetchError |
| hermes.com | — | 403 | externe |
| doctipharma.fr | — | **~85 %** | `Invalid URL` (signature arte — bras à nommer) |

## Arrêt
DELETE propres ×4 ; `Stop-Process -Id 2524` ; watchdog nominal.

## Re-vols reportés au vol 151
- §B-14 : `in requires an object` (hs-analytics mondialrelay+groupama), `[object Event]`/`[object Object]` trou sérialisation console, `fetch: url required` (laposte), `IDBRequest.get` (coinbase), `appendChild` on HTMLDocument (degiro).
- §B-59 ×3 sites ; §B-60 fetch storm ; §B-56/§B-57 résiduels ; §B-54/§B-23 ; §B-1-variante vimeo.
