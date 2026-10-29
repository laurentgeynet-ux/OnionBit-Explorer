<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 108 : REVALID15 `[V10-main]`

- **Binaire** : `[V10-main]` —
  `C:/vol100-wt/target/release/explorer-daemon.exe` (build
  `Finished`, HEAD `636aedc4` — inclut **JS-C183 §B-57 bras
  3** `ownKeys` Symbol).
- **Mission** : REVALID15 — §B-57 bras 3 caniuse (JS-C183),
  suivi §B-23 lemonde, §B-59 github sous watchdog.
- **Daemon** : PID **14148**, port 9641, watchdog armé —
  **tué par le watchdog** (9 648 Mo sur github) ; journal
  `_vol108_daemon.log` + `_vol108_watch.log`.
- **Verdict global** : **§B-57 RÉSOLU 3/3 bras** — caniuse :
  0 `deeply nested` (seul `osdlfm` iframe restant), page
  SSR ~95 %. **§B-59 reproduit ×2** — github : 500→9 648 Mo
  en ~15 s pendant la nav, watchdog killé — signature stable
  (~600 Mo/s), racine à nommer. lemonde sain (doc_h 12 924,
  erreurs toutes nommées — `lmd` cascade §B-14).

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| caniuse.com | **§B-57 RÉSOLU (3/3 bras)** — 0 `deeply nested` ; page ~95 % | `osdlfm` iframe seul | sort du catalogue — reste iframe dynamique |
| lemonde.fr | **favorable** — doc_h=12 924, page peinte ; erreurs nommées | `lmd is not defined`, `context`/`hosts of Undefined` inline, `script execution limit exceeded` RangeError, `push on ?` forecast | §B-14 cascade + signature `script execution limit` (budget ops ?) |
| github.com | **§B-59 ×2** — explosion 500→**9 648 Mo en ~15 s** pendant nav (nav_ms=25,1 n'a jamais retourné côté client), watchdog kill | — | fuite ~600 Mo/s pendant pompe nav — racine à nommer (preuve `_vol108_watch.log`) |

## Preuve marquante — §B-59 ×2

```
vol 106 : 356→430→8 079 Mo en ~30 s  → kill 5 120 Mo
vol 108 : 405→500→9 648 Mo en ~15 s  → kill 5 120 Mo
```

Reproduction stable, agressive (~250-600 Mo/s) : la fuite
vit pendant la phase nav/github (pompe, avant apply). Le
watchdog rend le re-vol possible sans risque machine.

## Arrêt

Watchdog a tué le daemon (PID 14148). DELETE propres
caniuse+lemonde avant le wedge. Sessions voisines intactes.

## Re-vols reportés au vol 109

- **github** → §B-59 : racine à nommer (instrumenter `/mem`
  par phase si un fix atterrit — sinon éviter, signature ×2
  suffit).
- flixbus → §B-54 vie-onglet ; vimeo → §B-1-variante.
- cryptpad/bookshop/caniuse-chemin → §B-23/§B-12/§B-57
  (fermé) ; uMap/mastodon/tumblr/wttj → §B-28/22/19/36.
