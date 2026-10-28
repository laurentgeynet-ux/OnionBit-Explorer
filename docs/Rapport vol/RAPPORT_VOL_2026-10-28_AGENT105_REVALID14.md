<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 105 : REVALID14 `[V10-main]`

- **Binaire** : `[V10-main]` —
  `C:/vol100-wt/target/release/explorer-daemon.exe` (build
  `Finished`, HEAD `400eaf7d` — inclut **JS-C182 §B-56 bras 3
  octales Annex B** + É157/É154/JS-C181).
- **Mission** : REVALID14 — §B-56 bras 3 airfrance (JS-C182),
  suivi §B-1-variante vimeo, exploration presse FR :
  leparisien, 20minutes.
- **Daemon** : 2 process — PID 6208 port 9636 (wedged vimeo →
  kill PID), PID 4160 port 9637 (fin de vol, arrêt propre) ;
  journaux `_vol105_daemon*.log`.
- **Verdict global** : **§B-56 RÉSOLU (3/3 bras)** —
  airfrance : 0 `invalid regex` (bras octal `\141`/`\100`
  fermé par JS-C182 — l'anomalie complète §B-56 sort du
  catalogue). **§B-1-variante confirmée ×2** — vimeo :
  rafale d'évals ~5,5 s continue → navigate >110 s, API
  morte (signature stable : évals longues légitimes, budget
  global absent). 20minutes ~80 %, leparisien ~50 %.
  §B-14 +2 (`name of Undefined` leparisien, `textContent of
  Undefined` 20minutes).

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| airfrance.fr | **§B-56 RÉSOLU (3/3 bras)** — 0 rejet regex sur 7 entrées ; coquille i18n inchangée (`???label???`, défaut §B-14 restant) | 7 entrées nommées (dynatrace, `unregister token`, import 404…) | §B-56 sort du catalogue — reste la montée SPA (§B-4/§B-14) |
| vimeo.com | **§B-1-variante reproduite ×2** — évals ~5,2-5,8 s `ok=true` en boucle (scripts 100 o-105 Ko), navigate >110 s timeout, API 000 ×2 → kill PID | — | borne éval 65 s jamais atteinte par script ; la **série** d'évals longues affame le runtime — budget global §B-1/N3 reste le chantier |
| leparisien.fr | **~50 %** — structure peinte (menu, logo, offre, blocs), hero/images absents | `name of Undefined` (bundle obfusqué) | §B-14 ; doc_h=66 848 clamped |
| 20minutes.fr | **~80 %** — header, rubriques, direct, hero photo + titres | `textContent of Undefined` main-2.48.13 ; `addEventListener on Object` embed | §B-14 +2 ; doc_h=19 510 |

## Preuves marquantes

```
airfrance : 0 « invalid regex » / 7 entrées — bras octal
            JS-C182 fermé (rejet ×2 vols 98/101 → 0)
vimeo     : script: eval fin ms=5 710 ok=true ×N continu
            — navigate >110 s, probes 000 ×2, kill PID
            signature = §B-1 bras « rafale évals longues »
```

## Arrêt des daemons

PID 6208 kill (wedge vimeo) ; PID 4160 : DELETE propres ×2,
RAM 411 Mo `{"tabs":[]}`. Sessions voisines intactes.

## Re-vols reportés au vol 106

- flixbus → §B-54 vie-onglet (starvation partielle restante).
- vimeo → §B-1-variante (budget global à poser — preuve ×2).
- cryptpad/bookshop/caniuse → §B-23/§B-12/§B-57-chemin-3.
- uMap/mastodon/tumblr/wttj → §B-28/22/19/36.
- airfrance → montée SPA (§B-14 cascade à nommer).
