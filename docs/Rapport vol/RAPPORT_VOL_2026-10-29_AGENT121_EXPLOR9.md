<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 121 : EXPLOR9 `[V10-main]`

- **Binaire** : `[V10-main]` — `C:/vol100-wt/target/release/explorer-daemon.exe` (HEAD `de325019`, JS-C190 inclus).
- **Mission** : EXPLOR9 — spotify (musique), blablacar (mobilité), midilibre (régio), sephora (beauté).
- **Daemon** : PID **14392**, port 9654, watchdog armé (pic 1 160 Mo, nominal) ; arrêt propre, DELETE propres ×4.
- **Verdict** : **4/4 sites peints — vol le plus favorable de la campagne** : midilibre ~90 % (doc_h 23,7k, seules 403 externes), sephora ~80 %, blablacar ~75 %, spotify ~70 % (web player coquille peinte). §B-14 +3 (`fromRGB` spotify, `x`/`addEventListener`/`name` sephora, `webview-communication` blablacar — erreurs nommées, rien de silencieux). Le passage des fixes JS-C181..190 montre : chaque bras §B-14 fermé déverrouille un palier de sites.

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| open.spotify.com | **~70 %** — coquille web player peinte (doc_h 2 180) | `fromRGB on function` | §B-14 +1 |
| blablacar.fr | **~75 %** — doc_h 13 283 | `webview-communication` state manquant | externe/module loader |
| midilibre.fr | **~90 %** — doc_h 23 716 | 403 gsi/client seuls | externe |
| sephora.fr | **~80 %** — doc_h 6 146 | `document.currentScript` invariant, `x`/`addEventListener`/`name of Undefined` | §B-14 +2 |

## Arrêt
DELETE propres ×4 ; `Stop-Process -Id 14392` ; watchdog nominal.

## Re-vols reportés au vol 122
- §B-60 à confirmer : fetch storm mastodon ×126.
- §B-14 file : `apply of Undefined`/`no supported media`/`fromRGB`/`x`/`addEventListener`/`name`/`toLowerCase`/`disableUaHints`/`category`/`appendChild`/`St.tooltip`/`Promise.finally`/`get`/`delete`/`initContext`.
- flixbus §B-54 vie-onglet ; vimeo §B-1-variante ; github §B-59 (watchdog) ; cryptpad §B-23.
