<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 122 : TVFR `[V10-main]`

- **Binaire** : `[V10-main]` — `C:/vol100-wt/target/release/explorer-daemon.exe` (HEAD `f20477ba`, JS-C190 inclus — aucun nouveau fix).
- **Mission** : TVFR — france.tv, arte.tv (TV publique jamais volées), maif (assurance), insee (stats public).
- **Daemon** : PID **4168**, port 9655, watchdog armé (pic 1 122 Mo, nominal) ; arrêt propre, DELETE propres ×4.
- **Verdict** : **4/4 sites peints** (france.tv ~80 %, arte ~80 %, maif ~75 %, insee ~75 %). Signal fort : l'invariant **`document.currentScript`** (`Expected <script>, received [object HTMLScriptElement]`) reproduit sur **3 sites** (sephora v121, france.tv, arte — vraisemblablement Next.js/shared runtime) → bras §B-14 à prioriser (un correctif touche 3+ sites). §B-14 +4 (`javaEnabled`, `AbortSignal.timeout`, `evaluate: unsupported XPath` insee — signature XPath nouvelle, `f`/`toLowerCase`).

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| france.tv | **~80 %** — doc_h 13 656 | `document.currentScript` invariant | §B-14 bras récurrent |
| arte.tv | **~80 %** — doc_h 3 575 | `document.currentScript` invariant | idem |
| maif.fr | **~75 %** — doc_h 15 698 | `javaEnabled`, `AbortSignal.timeout` | §B-14 +2 |
| insee.fr | **~75 %** — doc_h 5 828 | `f is not a function`, `toLowerCase`, **`evaluate: unsupported XPath`** | §B-14 +2 + signature XPath nouvelle |

## Arrêt
DELETE propres ×4 ; `Stop-Process -Id 4168` ; watchdog nominal.

## Re-vols reportés au vol 123
- §B-14 priorités : **`document.currentScript` invariant ×3 sites**, `AbortSignal.timeout`, `evaluate` XPath, `javaEnabled`.
- §B-60 fetch storm (mastodon ×126) ; §B-54 vie-onglet flixbus ; §B-1-variante vimeo ; §B-59 github (watchdog) ; §B-23 cryptpad.
