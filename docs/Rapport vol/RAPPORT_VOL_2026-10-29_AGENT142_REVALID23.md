<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 142 : REVALID23 `[V10-main]`

- **Binaire** : `[V10-main]` — `C:/vol100-wt/target/release/explorer-daemon.exe` (build `Finished`, HEAD `507b1b9a` — **inclut JS-C191..202 + JS-C193/196** : `mse.rs` désormais commité, build réparé).
- **Mission** : REVALID23 — vague de re-vols bloquée : allociné (JS-C191), arte (JS-C192), airbnb (JS-C195), insee (JS-C197), sfr (JS-C199).
- **Daemon** : PID **6880**, port 9675, watchdog armé (nominal ≤718 Mo) ; arrêt propre, DELETE propres ×5.
- **Verdict** : **5/5 fixes validés terrain** — allociné `addEventListener`-sur-Array disparu (JS-C191) ; arte `document.currentScript` invariant disparu (JS-C192) ; airbnb `postTask` disparu (JS-C195) ; insee `evaluate`/`toLowerCase` disparus (JS-C197) ; **sfr console entièrement vide** (`node is from a different document` disparu, JS-C199 adoption complète). La file §B-14 recule d'un cran à chaque site — les erreurs qui remontent sont les suivantes de la chaîne.

## Synthèse

| Cible | Fix testé | Verdict | Reste |
|---|---|---|---|
| allocine.fr | JS-C191 toStringTag | **RÉSOLU** | heap ×1, fetch err |
| arte.tv | JS-C192 currentScript | **RÉSOLU** | `Invalid URL` |
| airbnb.fr | JS-C195 postTask | **RÉSOLU** | `preloadRestQuery` ×6 |
| insee.fr | JS-C197 XPath/vendor | **RÉSOLU partiel** | `f` highcharts |
| sfr.fr | JS-C199 adoption | **RÉSOLU** — console **vide** | frame 204 transitoire |

## Arrêt
DELETE propres ×5 ; `Stop-Process -Id 6880` ; watchdog nominal.

## Re-vols reportés au vol 143
- JS-C194 maif/caf (`AbortSignal.timeout`/`javaEnabled`), JS-C198 SG (`localName`), JS-C196 lequipe (`postMessage clone host`), JS-C200 ruxit (`Worker.prototype`), JS-C201 orpi (`lang/dir`), JS-C202 (`User Timing`) — sites d'origine à re-voler.
- §B-59 ×3 sites ; §B-60 fetch storm ; §B-56 résiduel lookbehind ; §B-54/§B-23.
