<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 146 : REVALID26 `[V10-main]`

- **Binaire** : `[V10-main]` — `C:/vol100-wt/target/release/explorer-daemon.exe` (HEAD `d5962969` — **inclut JS-C203** GC trace hosts DOM/IDB, build réparé `Finished` 3m42s).
- **Mission** : REVALID26 — JS-C203 sur son site trigger : assemblee (`createHTMLDocument` tué par GC), caf (records IDB + `javaEnabled` JS-C194 bis), mastodon (IDB + fetch storm), senat (`toLowerCase` zombie).
- **Daemon** : PID **8476**, port 9679, watchdog armé (nominal ≤567 Mo) ; arrêt propre, DELETE propres ×4.
- **Verdict** : **JS-C203 validé ×3 sites** — assemblee `createHTMLDocument (on ?)` disparu (nav 37 s vs 175 s) ; senat `toLowerCase of Undefined` disparu ; caf `javaEnabled` disparu (JS-C194 bis confirmé). Mastodon : 0 erreur IDB (records tracés), fetch storm persistant ×126. Reste : `ctor without new` ×2 + `childNodes` (assemblée), `tooltip`/`modal`/`secp256k1`/JSON (caf), `find of Null` (senat).

## Synthèse

| Cible | Fix | Verdict | Reste |
|---|---|---|---|
| assemblee-nationale.fr | JS-C203 (trigger) | **RÉSOLU** — `createHTMLDocument` disparu | `ctor without new`, `childNodes` |
| caf.fr | JS-C203 IDB + JS-C194 bis | **confirmé** — `javaEnabled` disparu | `tooltip`/`modal`/`secp256k1` |
| mastodon.social | JS-C203 IDB records | **tenu** — 0 erreur IDB | fetch storm ×126 §B-60 |
| senat.fr | JS-C203 zombies | **RÉSOLU** — `toLowerCase` disparu | `find of Null` |

## Arrêt
DELETE propres ×4 ; `Stop-Process -Id 8476` ; watchdog nominal.

## Re-vols reportés au vol 147
- §B-14 : `ctor without new` (assemblée ×2), `find of Null` (senat), `tooltip`/`modal` on ce (caf), `secp256k1` (caf + meteofrance adjacent), `value is not a function` (axa), `finally`/`on` (dailymotion).
- §B-59 ×3 sites ; §B-60 fetch storm ×2 sites (mastodon/guardian) ; §B-56/§B-57 résiduels ; §B-54/§B-23 ; §B-1-variante vimeo.
