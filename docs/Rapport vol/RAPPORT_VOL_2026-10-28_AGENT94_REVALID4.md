<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 94 : REVALID4 `[V9-main]`

- **Binaire** : `[V9-main]` —
  `C:/OnionBit-Explorer/target/release/explorer-daemon.exe`
  (horodaté 28/10 19:40, HEAD `8260b6fa` — post-Étape 148,
  post R-127).
- **Mission** : REVALID4 — re-vol de validation de correctifs
  jamais confirmés terrain : §B-33 (francebleu nav_paint),
  §B-37 (caniuse panic dispatch), §B-40 (trainline hit-test
  scrollé), §B-34 (cjoint max-height:%), §B-23 (lemonde eval
  deadline), §B-21 (genius rafale ×512), §B-47 (santemagazine
  mojibake titre), §B-29 (linuxmint sodar).
- **Daemon** : PID **544**, port 9512,
  `EXPLORER_VIEWPORT=1366x900`, journal `_vol94_daemon.log`
  (~80 000 lignes) → `logs/daemons/`.
- **RAM** : base ~360 Mo → pics 1 573 (trainline) / 807
  (linuxmint+genius) Mo — purges complètes à chaque DELETE
  (579→552→~430 Mo constatés), aucune fuite §B-30.
- **Verdict global** : **6 correctifs confirmés RÉSOLUS
  `[V9-main]`** (§B-33, §B-40, §B-34, §B-23, §B-47, §B-29) —
  francebleu/ici navigue en ~3 min (vs wedge >25 min), clic
  trainline scrollé = vraie navigation, titre santemagazine
  UTF-8 propre, 0 iframe sodar linuxmint. **§B-37 toujours
  présent en signature intermittente** (panic `dom.rs:2752`
  ×1 rattrapé, nouveau site) et **§B-21 en décroissance**
  (×255 rejets nommés vs ×512 muets — source promise hors du
  cap intervalle). **2 nouveaux constats** : panic de la
  dépendance `resvg` (→ §B-55) et captures purgées du
  worktree (protocole — voir Hygiène).

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| francebleu.fr → ici.fr | **§B-33 RÉSOLU** — `POST navigate` retourné (~150 s) vs wedge >25 min ; page ici.fr rendue | `pas de journal` | **panic `resvg` composite.rs rattrapé** (assert width, tab 1) → proposition §B-55 ; `doc_h` clampé 128 127 |
| caniuse.com | **§B-37 TOUJOURS PRÉSENT (intermittent)** — panic `dom.rs:2752` `index out of bounds` ×1 rattrapé (disjoncteur), page néanmoins rendue complète ; non reproduit au re-vol | erreurs nommées | nouveau site de panic (2691→2752) — le fix `.get()` couvre un chemin, pas tous |
| thetrainline.com | **§B-40 RÉSOLU** — clic ancre footer à scroll_y=1 600 → `navigated` → `/train-times/london-to-edinburgh` | — | — |
| cjoint.com | **§B-34 RÉSOLU** — images bornées ~355 px dans leurs cartes (vs `<img>` 703×994 échappée) | `pas de journal` | chevauchement texte/titre carte §B-18 mineur |
| lemonde.fr | **§B-23 RÉSOLU** — éval 58 214 ms non tuée à 30 s ; boot aboutit, page montée+stylée | `TypeError`/`ReferenceError: lmd`/`RangeError script execution limit` **toutes nommées** | chevauchement hero/titre §B-18 ; `js_heap` 12,4 Mo |
| genius.com | **§B-21 PARTIEL — EN DÉCROISSANCE** — `r is not defined` ×255 en rejets **nommés** (vs ×512 muets) ; `withRetry went over retry limit of 6` ×1 | 255 `unhandledrejection` — chaque rejet tracé | source = promesses rejetées, **hors portée `TIMER_FAIL_CAP`** (cap = throws d'intervalle) |
| santemagazine.fr | **§B-47 RÉSOLU** — `title` API = « Santé Magazine… » UTF-8 propre (vs `idÃ©es`) | — | `doc_h` clampé 25 620 |
| linuxmint.com | **§B-29 RÉSOLU** — **0 iframe sodar** dans le DOM (vs 419, +90/min), `js_heap` 21 Mo (vs 160) | `pas de journal` | — |

Points marquants :

- **§B-33 fermé** : francebleu redirige vers ici.fr et
  `POST navigate` **retourne** — le wedge `nav_paint` perpétuel
  est mort ; la nav est longue (~150 s) mais bornée et la page
  se peint (logo ici, nav Télévision/Radio, hero article).
  Pendant cette même nav : **panic `resvg`** (`assertion
  failed: src1.width == src2.width`, filter composite) —
  dépendance, rattrapée, process vivant → §B-55.
- **§B-37 confirmé vivant** : sur caniuse (tab 2, 19:17:35),
  `index out of bounds: len 6, index 386` à
  `jscore/dom.rs:2752` — immédiatement suivi de
  `script sauté — disjoncteur moteur ouvert` (rattrap propre).
  Le fix `101a427` a borné `node_at_path` (2691) mais un
  **autre site d'indexation** (2752) indexe encore un NodeId
  obsolète post re-parse. Non reproduit au re-vol (signature
  intermittente, contexte Fingerprint).
- **§B-40 fermé** : clic (197,790) à `scroll_y=1 600` sur
  ancre footer réelle → `{"result":"navigated"}` → page
  `/train-times/london-to-edinburgh` complète (`can_back`).
- **§B-23 fermé** : l'éval de boot lemonde a tourné **58,2 s
  sans être tuée** (limite avant fix : 30 s) — le canal évals
  65 s protège la compile ; les erreurs restantes
  (`lmd is not defined`, `hosts/context of Undefined`,
  `RangeError: script execution limit` sur le challenge
  `_fs-ch-*`) sont toutes **nommées** à `/console`.
- **§B-21 en décroissance** : le raz-de-marée muet `r is not
  defined` ×512 est devenu 255 `unhandledrejection` tracées —
  visibles, nommées, avec stack `reactAds-*.js`. Le cap
  `TIMER_FAIL_CAP=8` borne les *intervalles* dont le callback
  jette ; ici chaque tick crée une **promesse qui rejette** —
  événement distinct, hors du cap.
- **§B-29 fermé** : 0 `<iframe>`/`sodar` dans le DOM linuxmint
  (cap `pending_dyn_frames` 64 — AdSense ne gonfle plus ni le
  DOM ni le heap : 21 Mo vs 160).

---

## 1. francebleu.fr → ici.fr — §B-33 fermé : navigate retourné

`POST /navigate` → **retourné ~150 s** (vs jamais retourné
>25 min vol 52), `doc_h=128 127` **clampé**, titre « ICI, le
média de la vie locale : actualités en direct ». Le correctif
`20f2336` (nav_paint borné 30 s + `CoreError::Timeout`)
transforme le wedge perpétuel en nav lente mais finie.

**Panic dépendance** : à 19:15:24 (tab 1), `panicked at
resvg-0.45.1/src/filter/composite.rs:25:5` — `assertion
failed: src1.width == src2.width && src1.width == dest.width`.
Un SVG de la page ici déclenche une assertion du rasteriseur
SVG tiers. **Process survivant** (panic rattrapé au niveau
thread, les apply continuent). → proposition §B-55
(dépendance, périmètre render — le moteur ne doit pas laisser
un asset hostile paniquer une dépendance).

**Rendu** (`captures/vol94_francebleu.png`) : header ici
(logo, « Mercredi 28 octobre », Télévision/Radio/« Choisir
votre ici »), placeholder pub, hero « En ce moment » +
article mobilisation du 31 octobre — page fonctionnelle.

**Télémétrie** (`logs/vol94_francebleu_mem2.json`) :
`dom_nodes=6 908`, `layout_boxes=5 389`, `js_heap=7,9 Mo`,
`images_bytes=154,9 Mo` — document très lourd (clamp
`doc_h` attendu).

## 2. caniuse.com — §B-37 : panic résiduel intermittent (dom.rs:2752)

`POST /navigate` → retour normal, `doc_h=2 836`, titre « Can
I use... Support tables for HTML5, CSS3, etc ». **Rendu
complet** (`vol94_caniuse.png`) : hero orange « Can I use ? »,
Index of features, Latest features (Lazy loading, CSS Grid
Lanes, CSS if(), View Transitions, WebAssembly), Most
searched, BrowserStack — la page fonctionne.

**Le panic survit en signature intermittente** : journal
19:17:35 tab=2 — `panicked at crates\explorer-jscore\src\
dom.rs:2752:33` : `index out of bounds: the len is 6 but the
index is 386`. Suivi immédiat de `WARN script sauté —
disjoncteur moteur ouvert` → rattrap typé (le moteur se
marque stalled, les scripts suivants sautés — comportement
disjoncteur voulu). Le fix `101a427` a borné le site 2691
(`node_at_path`) ; **2752 est un second site d'indexation
directe** — même cause (NodeId tué au re-parse ADR-0025,
contexte rejets `Fingerprint`), autre ligne. Un re-vol propre
ne l'a pas reproduit → **intermittent, pas fermé**.

## 3. thetrainline.com — §B-40 fermé : clic scrollé navigue

`POST /navigate` → `doc_h=3 896`, titre Trainline. Scroll ×2
→ `scroll_y=1 600`. Layout dump : ancres footer `inline<a>`
à doc_y 2 389-2 486 (liens routes populaires). **Clic
(197,790)** — viewport y=790 ↔ doc_y≈2 390 — →
`{"result":"navigated"}` →
`thetrainline.com/train-times/london-to-edinburgh`, page
article peinte (`vol94_trainline_click.png`), `can_back`.
Le défaut vol 60/80 (« ancre `ignored` à scroll_y>0 ») est
fermé — la position sticky est prise en compte au hit-test
scrollé (`efd662c`).

## 4. cjoint.com — §B-34 fermé : images bornées

`POST /navigate` → page produits Samsung (« Mode d'emploi et
notice »). Layout : cartes `block<img>` ~**355×355 / 355×502
px** bornées dans leurs conteneurs — vs `<img>` intrinsèque
703×994 échappée du `div.h-48` (vol 56). Le `cb_h`/`h_spec`
du conteneur atteint désormais les atomes (`39b4066`).
Résidu mineur : texte/titre superposé sur la carte SM-R322
(`vol94_cjoint.png`) — famille §B-18.

## 5. lemonde.fr — §B-23 fermé : compile 58 s non tuée

`POST /navigate` → `doc_h=12 598`, titre Le Monde. Journal :
`script: eval fin tab=5 ms=58214 ok=false` — l'éval de boot
a tourné **58,2 s** (le canal évals `recv_eval_long` = 65 s
a laissé la compile mener sa vie ; avant fix : kill à 30 s
→ boot mort). `ok=false` final = erreur métier du script
anti-bot `_fs-ch-*` (`RangeError: script execution limit`),
pas un kill prématuré.

**`/console`** : erreurs **toutes nommées** — `TypeError:
hosts/context of Undefined`, `ReferenceError: lmd is not
defined` (§B-10 famille — scripts définisseurs morts en
cascade §B-4), `TypeError: push is not a function`
(forecast sdk), `RangeError: script execution limit`
(init challenge).

**Rendu** (`vol94_lemonde.png`/`_scroll.png`) : header Le
Monde complet (logo, nav sections, tickers Live horodatés),
hero photo carrousel « 1/2 », articles réels stylés au scroll
(live Israël, midterms) — site utilisable. Anomalie :
chevauchement image hero / zone titre en haut du flux
(§B-18). `dom_nodes=13 197`, `js_heap=12,4 Mo`,
`journal_fallbacks=1` absorbé.

## 6. genius.com — §B-21 : tempête réduite mais source vivante

`POST /navigate` → `doc_h=2 654`, titre Genius.
`/console` = **268 entrées** dont **255×**
`unhandledrejection: ReferenceError: r is not defined`
(`reactAds-iceVJr5s.js:1:31073`, stacks complètes) + 1×
`withRetry went over the retry limit of 6` + `le is not
defined` + `RangeError: expression too deeply nested`.

Comparaison vol 33 : `r is not defined` **×512** en un seul
bloc muet → désormais **×255 en rejets nommés** — halved et
traçable. Mais le mécanisme échappe `TIMER_FAIL_CAP=8` : le
cap borne un intervalle dont le callback **jette** ; le code
reactAds crée à chaque tick une **promesse qui rejette** —
l'échec survit comme `unhandledrejection`, hors chemin du cap.
Verdict : **PARTIEL — la source promise reste à borner**
(dedup visuel ×N sur les rejets identiques ? cap sur les
rejections d'une même stack ?).

`js_heap=98 Mo` — JS lourd vivant. `journal_fallbacks=2`.

## 7. santemagazine.fr — §B-47 fermé : titre UTF-8 propre

`POST /navigate` → `doc_h=25 620` clampé. `GET /api/tabs` →
`title` = « Santé Magazine, le féminin qui fait du bien ! Le
magazine de la santé | Santé Magazine » — **zéro mojibake**
(vs `idÃ©es`/`actualitÃ©` vol 71). `extract_title(dom)` vivant
(`e268ec7`) + `charset=utf-8` (`8ddd26d`) tiennent.

## 8. linuxmint.com — §B-29 fermé : 0 iframe sodar

`POST /navigate` → `doc_h=9 138`. DOM dump : **0 `<iframe>`,
0 `sodar`** — vs 419 iframes télémétrie AdSense cumulées
(+~90/min, heap 53→160 Mo, vol 51). Le cap `pending_dyn_frames`
64 (`a1d0dbd`) empêche la boucle — `js_heap=21 Mo` stable,
page rendue (`vol94_linuxmint.png`), console vierge.

## Statuts proposés (§8)

| §B | Bras | Verdict ce vol |
|---|---|---|
| §B-33 | wedge `nav_paint` | **RÉSOLU `[V9-main]` vol 94** — francebleu→ici.fr navigate retourné ~150 s (vs >25 min), page peinte |
| §B-37 | panic `event.dispatch` NodeId | **TOUJOURS PRÉSENT — intermittent, nouveau site** : `dom.rs:2752` `len 6 / idx 386` ×1 rattrapé (disjoncteur), contexte re-parse Fingerprint ; non reproduit au re-vol |
| §B-40 | hit-test ancre scrollée | **RÉSOLU `[V9-main]` vol 94** — clic scroll_y=1 600 → `navigated` → article trainline |
| §B-34 | `max-height:%` remplacé | **RÉSOLU `[V9-main]` vol 94** — img cjoint ~355 px bornées (vs 703×994) |
| §B-23 | eval deadline boot | **RÉSOLU `[V9-main]` vol 94** — éval 58,2 s non tuée (canal 65 s), erreurs boot nommées |
| §B-21 | rafale `r is not defined` | **PARTIEL — EN DÉCROISSANCE** — ×512 muet → ×255 `unhandledrejection` nommés ; source = rejets de promesses hors `TIMER_FAIL_CAP` (cap = throws d'intervalle) |
| §B-47 | mojibake titre API | **RÉSOLU `[V9-main]` vol 94** — `title` santemagazine UTF-8 propre |
| §B-29 | iframes sodar | **RÉSOLU `[V9-main]` vol 94** — 0 iframe linuxmint (cap 64), heap 21 Mo vs 160 |
| §B-30 | fuite post-DELETE | **signal favorable ×8** — purges complètes après chaque DELETE (tabs→[], RAM 552-807 Mo) ; non mesurable sur cible dédiée manomano |
| §B-50 | re-parse ADR-0025 | **signal favorable** — `Fingerprint`/`UnmappedKey` absorbés (lemonde 1, genius 2, fallbacks comptés) |
| **§B-55 (nouveau)** | **panic `resvg` — dépendance** | **proposition d'entrée** — `resvg-0.45.1/filter/composite.rs:25` `assertion failed: src1.width == src2.width` sur SVG ici.fr (tab 1, 19:15) — rattrapé, process vivant ; périmètre `explorer-render` (policy : asset hostile ne doit pas paniquer une dep) |
| §B-18 | layout résiduel | **+2 instances** — overlap hero/titre lemonde, texte superposé carte cjoint SM-R322 |

## Hygiène de vol

- Onglets pilotés : 1 (francebleu), 2 (caniuse), 3 (trainline),
  4 (cjoint) par la session agent précédente — captures/log
  initiaux **perdus à la purge captures/ du 28/10** (7 Go,
  opérateur) — re-capturés ce vol (5=lemonde, 6=genius,
  7=francebleu, 8=caniuse, 9=trainline, 10=santemag,
  11=linuxmint) — tous `DELETE`, `{"tabs":[]}` final.
- **Protocole captures modifié** : le dossier `captures/` a
  été purgé par l'opérateur (1 200 PNG / 7 Go) — les fichiers
  restent en historique git ; les nouvelles captures vivent
  non commitées et `scripts/prune_captures.ps1` supprime
  toute PNG non citée par un rapport/synthèse.
- Daemon voisin PID 12052 (port 9510, autre worktree) jamais
  touché ; aucun `taskkill /IM`.
- Artefacts : `captures/vol94_{francebleu,francebleu_scroll,
  caniuse,trainline,trainline_click,cjoint,cjoint_click,
  cjoint_scroll,lemonde,lemonde_scroll,genius,santemag,
  linuxmint}.png` ; `logs/vol94_*_{console,mem,layout,dom}.
  json/txt` ; `logs/daemons/_vol94_daemon.log` (local,
  gitignoré).
