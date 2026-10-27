<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — AGENT 52 — Presse française (2026-10-27)

- **Binaire** : `[V4-main]` — `C:/obe-vol-target/release/explorer-daemon.exe`
  (WIP `depth_limit` embarqué), `--port 9434` (PID 3444),
  `EXPLORER_VIEWPORT=1366x900`, journal `_vol52_daemon.log` (racine)
- **Cibles** : www.francebleu.fr · www.20minutes.fr ·
  www.liberation.fr — presse grand public, forte densité
  JS/iframe/pubs
- **Protocole** : `docs/vol-agent.md` — traçabilité §8, IDs §B de
  `SYNTHESE.md`

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| www.francebleu.fr → **ici.fr** | **Page jamais servie — wedge worker paint, navigate pendu >25 min** | `document of Null` (mPulse) + `no module loader` | **§B-29 proposé** · §B-14 · §B-5 |
| www.20minutes.fr | **Idem — wedge après `apply: layout ms=857`, jamais `repaint`** | `textContent of Undefined` (main-2.48.10) | **§B-29 proposé** (2ᵉ occurrence) · §B-14 |
| www.liberation.fr | **HTTP 403 — anti-bot, page d'erreur moteur peinte** | `HTTP 403` | Externe (UA/fingerprint) — dégradation correcte |

Faits marquants :

1. **2/3 cibles tombent dans le même wedge silencieux** — signature
   nouvelle et reproductible : `POST /navigate` **ne retourne
   jamais**, `dom_nodes=0`, `journal_applies=0`,
   `journal_fallbacks=0`, **aucun WARN** dans le journal daemon,
   `/dom`=404, `/frame`=204, `/layout`=404 en continu (>20 min),
   input `click` différé puis rejeté `« paint en vol »`. Le JS a
   tourné (heaps 7,7 et 24,3 Mio, 56 évals loguées) — c'est la
   **payload de peinture** qui est détenue à vie par un worker
   `spawn_blocking` wedged.
2. **francebleu.fr suit la redirection de marque réelle → ici.fr**
   (« ICI, le média de la vie locale ») — navigation + redirects
   sains en amont du wedge.
3. **liberation.fr dégrade proprement** : `navigate` retourne
   `{"error":{"handled":true,"message":"HTTP 403"}}`, page d'erreur
   « Page inaccessible » **peinte** par le moteur
   (`captures/vol52_liberation.png`, 1,8 Mo) — le chemin d'erreur
   est sain bout-en-bout.
4. **L'API reste responsive** malgré deux onglets wedged —
   `/tabs`, `/mem`, `/console`, `/input` servis en <1 s ; le verrou
   session n'est pas tenu (étape 53d tient ses promesses).

RAM daemon : 384 → 524 → 1 113 → **1 348 Mo** sur ~18 min avec deux
onglets wedged (croissance ~1 Mo/min — fuite modérée, même famille
que la fuite post-wedge §B-26 ; restée sous le seuil 2,5 Go). CPU
~1 370 s cumulés ≈ **1,5 cœur saturé** en continu — les workers
coincés bouclent.

---

## 1. www.francebleu.fr → ici.fr — wedge premier commit → §B-29

- `POST /api/tabs` → tab 1 ; `POST navigate` `https://www.francebleu.fr`
  → **redirection réelle suivie** vers `https://www.ici.fr/`, titre
  réel « ICI, le média de la vie locale : actualités en direct ».
- **La requête `navigate` n'a jamais retourné** (>25 min en vol à
  la clôture — pas de réponse HTTP du tout, curl toujours pendant).
- JS booté : 44 évals `js_glue` à 09:16:08 (1 `ok=false`), heap
  7,7 Mio. Puis **zéro ligne `apply:` pour cet onglet en 17 min** —
  le worker `nav_commit`/`relayout` n'a jamais atteint le log
  `apply: cascade` : le wedge est **en amont du relayout**, dans
  `insert_images_payload` (décodage images) ou `styles_payload`
  (`shared.rs:186-205`, `apply.rs:1453-1466`).
- `/console` verbatim (grâce 12 s, puis re-sondes) :
  ```
  exception js : TypeError: cannot read `document` of Null
      at t (1:453)
      at <anonymous> (1:295)
      at <anonymous> (1:2) — src «!function(e){var n="https://s.go-mpulse.net/boomerang/"…»
  unhandledrejection: TypeError: no module loader for `../chunks/DVd5xze_.js`
  ```
  (`document of Null` = signature §B-14 déjà cataloguée — dw ;
  `no module loader` spec relatif `../chunks/` = bras §B-5.)
- Télémétrie figée : `/mem` `dom_nodes=0, layout_boxes=0,
  display_items=0, frame_bytes=0, js_heap=7 733 132,
  journal_applies=0, journal_fallbacks=0` (`logs/vol52_francebleu_mem.json`) ;
  `/dom` → 404 (6 sondes espacées), `/frame` → 204, `/layout` → 404.
- Input : scroll dy=800 → accepté `scroll_y=0` ; clic (400,300) →
  `ignored` + journal daemon `input différé — paint en vol, rejeté
  comme ignored tab=1` (09:28:02) — **`paint_detached` posé en
  permanence**, preuve qu'un worker détient la payload.
- La pompe JS continue de battre (`pump.commit_lock` ~16 Hz, 14 191
  cycles) : `take_js_engine`/`commit_js_pump` réussissent à chaque
  tour — le moteur JS n'est **pas** le détenteur coincé ; c'est un
  job apply/paint `spawn_blocking`.

## 2. www.20minutes.fr — même wedge, point de blocage nommé : après `apply: layout`

- `POST navigate` → titre réel « 20 Minutes - Toute l'actualité en
  direct… », 12 évals à 09:28:09-12 (bundle 1,36 Mo `ok=false`
  en 2 333 ms = l'exception ci-dessous). **Requête `navigate`
  jamais retournée** non plus.
- `/console` verbatim :
  ```
  exception js : TypeError: cannot read `textContent` of Undefined
      at <anonymous> (https://www.20minutes.fr/js/main-2.48.10.min.js:1:109235) … — §B-14
  ```
- **Trace directe du point de blocage** (journal daemon) :
  `apply: cascade ms=265` → `apply: boxtree ms=373` →
  **`apply: layout ms=857`** à 09:28:13 — puis **jamais
  `apply: repaint`**. Le worker s'est figé entre layout et repaint
  : `composite::composite_frames` (raster des sous-contextes
  `<iframe>` — page de presse chargée en iframes pubs) ou
  `repaint_payload` (`apply.rs:1504-1531`). Un apply postérieur
  trivial passe (09:30:52 : layout 2 ms, repaint 6 ms) → le pool
  `spawn_blocking` fonctionne, **un seul job est mortel**.
- `/mem` : `dom_nodes=0, js_heap=24 347 819, journal_applies=0,
  journal_fallbacks=0` (`logs/vol52_20minutes_mem.json`) ; `/dom`,
  `/frame`, `/layout` → 404/204/404 en continu.
- Input : scroll ×2 → **projection** `scroll_y=800` puis `1600`
  (scroll différé projeté, mécanisme FIX-3 vivant) ; clic →
  `ignored` + `input différé — paint en vol tab=2` (09:33:32).

### §B-29 proposé — Worker paint wedged détenant la payload : navigate pendu, DOM détaché à vie

- **Signature** : `POST /navigate` sans réponse (boucle
  `take_nav_paint → Busy → sleep(50 ms)` **sans timeout** dans
  `nav_commit_shared`, `shared.rs:157-166` — le contrat « le
  détenteur la restitue toujours » est **falsifié sur le
  terrain**) ; `dom_nodes=0` + `journal_applies=0` +
  `journal_fallbacks=0` permanents ; aucun WARN ; `/frame` 204,
  `/dom`/`/layout` 404 ; input différé « paint en vol ».
- **Mécanisme** : un job `spawn_blocking` (`insert_images_payload`,
  `styles_payload`, `composite_frames` ou `repaint_payload`) ne
  rend jamais — boucle infinie ou blocage — pendant qu'il détient
  la `PaintPayload`. Conséquence : l'onglet est muet pour tout ce
  qui exige le DOM, mais la pompe JS et l'API restent vivantes.
- **Dénominateur** : 2/3 cibles ce vol, les deux sites de presse
  (ici.fr : wedge avant `apply: cascade` ; 20minutes.fr : wedge
  après `apply: layout ms=857`) — deux points de blocage distincts
  ⇒ suspect transversal (décodage image ? composite iframe ?).
- **Différence §B-26** (caf.fr, vol 47) : même surface observable
  mais mécanisme amont distinct — caf avait `journal_fallbacks=5` +
  WARN `UnmappedKey` + navigate retourné ; ici le journal n'est
  jamais sollicité et navigate **reste en vol**. Variante ou
  entrée sœur — à arbitrer ; la fuite RAM modérée se retrouve
  dans les deux cas.
- **Crate** : `explorer-core` (`session/apply`, `session/shared`) ;
  racine possible dans `explorer-render` (composite/raster) ou
  `explorer-net` (décodage image).
- **Correctif structurel suggéré** : timeout/watchdog sur le poll
  `take_nav_paint` + abandon de l'epoch (la payload perdue est
  moins grave qu'un onglet zombie) — recoupe §D-1/N3 (watchdog
  post-apply) : **c'est le même trou contractuel**.

## 3. www.liberation.fr — 403 anti-bot, dégradation saine

- `POST navigate` → **retourné proprement** :
  `{"error":{"handled":true,"message":"HTTP 403"}}` ; titre onglet
  « Erreur de chargement ».
- `/console` : `HTTP 403`. `/frame` → **200, PNG 1 813 309 o** :
  page d'erreur moteur « Page inaccessible / La page
  https://www.liberation.fr/ n'a pas pu être chargée. Le serveur a
  répondu « HTTP 403 ». » — rendue proprement
  (`captures/vol52_liberation.png`, `layout_boxes=18`,
  `display_items=29`, `frame_bytes=4,9 Mo`).
- `/dom` → 404, `dom_nodes=0` : **normal sur le chemin d'erreur**
  (`paint_error_page` ne conserve pas de DOM) — à ne pas confondre
  avec le wedge §B-29.
- Input : scroll ×2 → `scroll_y=0` (page d'erreur non défilante) ;
  clic → `ignored` — comportement correct.
- **Attribution externe** : anti-bot 403 au premier hit (signature
  cumulée catalogue : `UA: Explorer/0.1`, fingerprint rustls) —
  liberation.fr rejoint la liste ebay/nytimes/medium… Hors moteur ;
  le chemin d'erreur est un acquis à conserver.

---

## Journal de session

| Heure (UTC) | Action | Résultat |
|---|---|---|
| 09:15:49 | daemon :9434 (PID 3444), viewport 1366×900 | prêt, RAM 384 Mo |
| 09:16:0x | tab1, navigate francebleu.fr | redirect → ici.fr ; 44 évals ; **réponse jamais retournée** |
| 09:16→33 | sondes tab1 | `dom_nodes=0` continu, frame 204, applies=0, fallbacks=0, **0 WARN** |
| 09:28:02 | clic tab1 | `input différé — paint en vol` → `ignored` |
| 09:28:0x | tab2, navigate 20minutes.fr | 12 évals ; apply figé post-`layout ms=857` ; **réponse jamais retournée** |
| 09:30:5x | tab3, navigate liberation.fr | retourné `HTTP 403`, page d'erreur peinte |
| 09:33:32 | scroll×2+clic tab2 | `scroll_y` 800→1600 (projection) ; `input différé — paint en vol` |
| 09:34 | clôture | RAM 1 348 Mo, CPU ~1 370 s (~1,5 cœur) |

## Clôture §B touchés

| ID | Mouvement proposé |
|---|---|
| **§B-29** | **NOUVEAU** — worker paint wedged détenant la payload : `POST /navigate` sans réponse (poll `take_nav_paint` sans timeout), DOM détaché à vie (`applies=0`, `fallbacks=0`, 0 WARN), « paint en vol » — 2/3 cibles `[V4-main]` (ici.fr, 20minutes.fr). Variante/mécanisme amont de §B-26 ; recoupe §D-1/N3 (watchdog post-apply = même trou contractuel) |
| §B-26 | recoupe — même surface observable, fuite RAM post-wedge confirmée (modérée ici : ~1 Mo/min vs 2,93 Go caf) |
| §B-5 | recoupe — `no module loader for '../chunks/DVd5xze_.js'` (spec relatif) sur ici.fr |
| §B-14 | +1 occurrence `document of Null` (mPulse/boomerang — même signature que dw) ; +1 `textContent of Undefined` (20minutes main-2.48.10.min.js) |
| §D-1 / N3 | le poll `take_nav_paint` sans timeout EST un cas concret du watchdog post-apply manquant — priorité confortée |
| Externe | liberation.fr → liste anti-bot 403 (dégradation saine : erreur typée + page peinte) |

## Artéfacts

- `captures/vol52_liberation.png` — page d'erreur « Page
  inaccessible » (1,8 Mo) ; `captures/vol52_francebleu.png` —
  fichier vide (preuve `/frame` 204).
- `logs/vol52_francebleu_console.json`, `vol52_francebleu_mem.json`,
  `vol52_francebleu_dom.txt` (404) · `logs/vol52_20minutes_console.json`,
  `vol52_20minutes_mem.json`, `vol52_20minutes_layout.txt` (404) ·
  `logs/vol52_liberation_console.json`, `vol52_liberation_dom.txt`,
  `vol52_liberation_layout.txt`.
- `_vol52_daemon.log` (racine) — lignes `apply:` à 09:28:07 /
  09:28:12-13 (layout 857, **sans repaint**) / 09:30:52 ;
  `input différé — paint en vol` tab=1 (09:28:02) et tab=2 (09:33:32).
