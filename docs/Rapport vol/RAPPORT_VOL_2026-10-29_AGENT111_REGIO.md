<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 111 : REGIO `[V10-main]`

- **Binaire** : `[V10-main]` — `C:/vol100-wt/target/release/explorer-daemon.exe` (HEAD `636aedc4`, aucun nouveau fix).
- **Mission** : REGIO — presse régionale FR sous-couverte : ladepeche, sudouest ; tech : 01net ; dev : developer.mozilla.org.
- **Daemon** : PID **2164**, port 9644, watchdog armé (RAM ≤434 Mo nominal) ; arrêt propre, DELETE propres ×4.
- **Verdict global** : ladepeche ~85 % rendu (console quasi propre), sudouest ~70 % (1 occurrence heap — cap tenu), 01net dégradé (moteur figé), MDN rendu (hydration mismatch nommé). 0 wedge, 0 fuite — vol calme sous harnais.

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| ladepeche.fr | **~85 %** — page riche peinte (doc_h=26 722) | 403 gsi/client + 404 météo (sous-ressources) | rien moteur — externe seul |
| sudouest.fr | **~70 %** — doc_h=21 329, page peinte | `require is not defined`, `define`/`push of ?`, JSONP timeout, **heap ×1** (gtm) | §B-12 cap favorable ; §B-14 (`require` CJS absent ?) |
| 01net.com | **dégradé** — doc_h=8 869, rendu partiel | `moteur figé — éval dépassée` + `inerte` | freeze éval |
| developer.mozilla.org | **rendu** — doc_h=2 538 | `Hydration value mismatch` (lit/ssr) | §B-14-adjacent lit hydrate |

## Arrêt
DELETE propres ×4 ; `Stop-Process -Id 2164` ; watchdog nominal (≤434 Mo).

## Re-vols reportés au vol 112
- flixbus §B-54 vie-onglet ; vimeo §B-1-variante ; github §B-59 (watchdog).
- cryptpad/bookshop → §B-23/§B-12 ; uMap/mastodon/tumblr/wttj → §B-28/22/19/36.
