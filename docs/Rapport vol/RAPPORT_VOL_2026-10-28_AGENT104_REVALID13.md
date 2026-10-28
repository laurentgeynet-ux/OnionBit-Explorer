<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 104 : REVALID13 `[V10-main]`

- **Binaire** : `[V10-main]` —
  `C:/vol100-wt/target/release/explorer-daemon.exe` (build
  `Finished`, HEAD `f79d9b79` — inclut **Étape 157 §B-54
  drain animationstart borné** + É154/JS-C181/R-131).
- **Commits post-binaire** : `32552a81` (JS-C182 §B-56 bras 3
  octales Annex B) + doc `b5d9659a` → priorité vol 105
  (airfrance).
- **Mission** : REVALID13 — §B-54/§B-30 flixbus (É154+157),
  §B-1 allrecipes (J1 jamais re-volé), exploration :
  fnac/boursorama/darty (e-commerce+banque FR), vimeo.
- **Daemon** : PID **11532**, port 9635 — vol jusqu'à vimeo
  (starvation → kill PID) ; journal `_vol104_daemon.log`.
- **Verdict global** : **§B-1 RÉSOLU** (allrecipes : API
  100 % responsive ~90 s, RAM stable 1,27 Go vs muette 3-6
  min/2,9 Go — J1 validé). **§B-54 bras post-DELETE
  RÉSOLU** (flixbus : DELETE 204 en 5,5 s, API pleine, purge
  390 Mo) ; **bras vie-onglet EN DÉCROISSANCE** (starvation
  partielle : probes 200 intermittentes vs API morte ×2
  vols ; `drain_anim_births` borné tiré terrain). **§B-12
  favorable** (allrecipes : 1 seule occurrence heap — cap
  par-moteur tient). **§B-14 +2** (FontFaceSet.add,
  getAttributeNS — boursorama). fnac+darty : 403 anti-bot
  (externe). **vimeo : §B-1-famille** (rafale d'évals ~5,5 s
  continue → navigate >240 s, API morte).

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| flixbus.fr | **§B-54 PARTIEL→décroissance** — navigate 63 s ; starvation partielle (probes 200/000 alternées, vs morte totale) ; **DELETE 204 5,5 s + API pleine + purge 390 Mo = bras post-DELETE RÉSOLU** | — | `drain_anim_births au-delà du budget` tiré ; `poll_net` 18,5 s / `dispatch` 5,2 s / `mirror_sync` 12 s résiduels → borne contenu des phases restante |
| allrecipes.com | **§B-1 RÉSOLU** — API responsive ×6 probes (~90 s), RAM plateau 1,26-1,27 Go ; page ~70 % peinte | `throttle is not a function` + **1 seule** occurrence heap (cap tenu) | §B-12 favorable ×1 ; `throttle` → §B-14 |
| fnac.com | **externe** — HTTP 403 | 403 | anti-bot (Datadome-famille) |
| boursorama.com | **~45 %** — header/nav + dropdown peints, contenu clairsemé | `FontFaceSet.add` ×2, `getAttributeNS` on HTMLElement, `location of Null` lazysizes, `DtkPlayer` iframe, `addEventListener of Undefined` | **§B-14 +2** ; doc_h=33 547 (clamped) |
| darty.com | **externe** — HTTP 403 | 403 | anti-bot e-commerce FR (×2 cibles ce vol) |
| vimeo.com | **wedge §B-1-famille** — rafale d'évals ~5,5 s en continu (881-105 Ko, ok=true), navigate >240 s jamais retourné, API morte ×3 probes, RAM 1,43 Go → kill PID | — | les évals 5,5 s passent sous la deadline 65 s mais la rafale affame le runtime — variante §B-1 survivante (budget global absent) |

## Preuves marquantes

### §B-54 — signature nouvelle : wedge → starvation

```
23:06:36 event.* au-delà du budget (B-54) elapsed=15 007
23:07:23 drain_anim_births au-delà du budget — reste différé
         (B-54) rest=23 elapsed=17 933   ← É157 tiré terrain
probes: 000/200/000/200/000 (API souffle mais ne meurt plus)
DELETE 1 → 204 en 5,57 s ; post-DELETE : 200 ×3 <2 ms,
RAM 1,31 Go → 390 Mo — pompe orpheline purgée (É154 validé)
```

### §B-1 — allrecipes libéré

```
nav_ms=23,8 ; probes ×6 : 200 (0,001-1,85 s) ; RAM plateau
1 261→1 268 Mo ; heap_dead ×1 (551 Mo, cap par-moteur tenu)
vs vol 34 : API muette >3-6 min, RAM 2,9 Go
```

### §B-1-variante — vimeo

```
script: eval fin ms=5 211-5 799 ok=true en boucle continue
(scripts 881 o-105 Ko) — navigate >240 s, API 000 ×3
→ évals longues légitimes en rafale : budget global absent
```

## Arrêt des daemons

PID 11532 wedged vimeo → `Stop-Process -Force`. DELETE
propres ×5 sur les cibles saines (flixbus inclus — fait
notable). Sessions voisines intactes.

## Re-vols reportés au vol 105

- **airfrance** → §B-56 bras 3 : CORRECTIF JS-C182 `32552a81`
  (octales Annex B) → **priorité 1**.
- **flixbus** → §B-54 bras vie-onglet : starvation partielle
  restante (borne contenu phases `event.*`).
- **vimeo** → §B-1-variante : rafale évals ~5,5 s —
  reproduire pour caractériser.
- cryptpad/bookshop/caniuse → §B-23/§B-12-cross-engine/
  §B-57-chemin-3 (sans correctif).
- uMap/mastodon/tumblr/wttj → §B-28/22/19/36 (ouverts).
