<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 136 : EXPLOR16 `[V10-main]`

- **Binaire** : `[V10-main]` — `C:/vol100-wt/target/release/explorer-daemon.exe` à `f20477ba` (JS-C190 — `main` toujours cassé : `mse.rs` non commité ; JS-C191..199 en attente de build).
- **Mission** : EXPLOR16 — orpi, cuisineaz, yves-rocher, abritel.
- **Daemon** : PID **13616**, port 9669, watchdog armé (nominal ≤497 Mo) ; arrêt propre, DELETE propres ×4.
- **Verdict** : cuisineaz ~85 % (**doc_h 60 229 — record campagne**, `consentRejection` sourcepoint) ; orpi ~75 % (`replace`/`id`/`Translator` → §B-14) ; yves-rocher ~60 % (`deeply nested` résiduel + bazaarvoice `Array of Null`/`startTime` — §B-57 + §B-14) ; abritel 429 rate-limit.

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| orpi.com | **~75 %** — doc_h 9 365 | `replace`/`id of Undefined`, `Translator` (403 kleio) | §B-14 +3 |
| cuisineaz.com | **~85 %** — **doc_h 60 229** | `consentRejection` sourcepoint | §B-14 +1 |
| yves-rocher.fr | **~60 %** — doc_h 7 207, frame 000 | `document of Null`, `deeply nested` (p-DEDQ), `Array of Null`/`startTime` bazaarvoice | §B-57 résiduel + §B-14 +3 |
| abritel.fr | **429** | `HTTP 429` | rate-limit externe |

## Arrêt
DELETE propres ×4 ; `Stop-Process -Id 13616` ; watchdog nominal.

## Re-vols reportés au vol 137
- **Rebuilder si `mse.rs` commité** → JS-C191/192/194/195/197/198/199 (adoption sfr).
- §B-57 résiduel ×2 (SG, yves-rocher — chemins nouveaux).
- §B-59 ×2 sites ; §B-60 ; §B-56 lookbehind ; §B-54/§B-23.
