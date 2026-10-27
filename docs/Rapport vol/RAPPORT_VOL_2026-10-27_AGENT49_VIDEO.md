<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — AGENT 49 — Vidéo / grands médias FR (2026-10-27)

- **Binaire** : `[V4-main]` — `C:/obe-vol-target/release/explorer-daemon.exe`
  (WIP `depth_limit` embarqué), `--port 9431`, `EXPLORER_VIEWPORT=1366x900`,
  journal `_vol49_daemon.log` (racine, 2,7 Mo), PID 4664
- **Cibles** : dailymotion.com · lefigaro.fr (re-vol §B-11 `new URL`) ·
  bfmtv.com — 3/3 prévues, vol écourté par crash de sécurité (§0)
- **Protocole** : `docs/vol-agent.md` — traçabilité §8, IDs §B de `SYNTHESE.md`

## ⚠ PRUDENCE déclenchée — arrêt d'urgence à 6,88 Go

Le seuil 2,5 Go a été **explosé** : RAM daemon 1,72 Go (fin figaro) →
**6 883 704 Ko (6,88 Go) en ~5 min** pendant le wedge de l'article bfmtv —
le pire dépassement de fuite mesuré à ce jour (précédent : caf §B-25,
2,93 Go non libérée). L'API est devenue **totalement muette** (toutes
sondes `curl -m 5` → timeout, y compris `GET /api/tabs` global et les
DELETE) : DELETE impossible, kill direct du PID 4664 par `taskkill //PID`
(jamais `/IM`). Signature : §B-25 (UnmappedKey → re-parse → apply figé)
qui **escalade en §B-1** (starvation post-apply + fuite massive).

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| dailymotion.com | **page blanche — exécution réussie sans effet** | `pas de journal` | **§B-4(c) recoupé** (19/19 évals `ok=true`, React jamais monté) + obs : `js_heap_bytes=0` |
| lefigaro.fr | **page servie puis wedge — DOM détaché terminal** | 2 erreurs appconsent (`delete`/`addEventListener`) | **§B-25 variant** (apply mort post-`layout`, **sans** UnmappedKey) + §B-14 ×2 ; **§B-11 reste sain** (zéro `new URL`) |
| bfmtv.com | **accueil servi ~85 % fidèle, article → wedge fatal** | `pas de journal` | **§B-25 reproduit** (UnmappedKey ×2) → **§B-1** (API morte, 6,88 Go) ; §B-18 chevauchement ; data-URI SVG peint en texte |

Fait marquant : **le wedge §B-25 n'est pas borné** — il fuit sans limite
(6,88 Go) et peut tuer l'API entière du daemon, pas seulement l'onglet.
Deuxième fait : le wedge frappe **aussi sans UnmappedKey** (figaro) — la
signature commune est « apply débuté, `layout` journalisé, jamais
`repaint`/délivré → DOM détaché permanent ».

---

## 1. dailymotion.com — SPA morte blanche → §B-4(c)

- `POST navigate` → titre réel « Dailymotion », `doc_h=900`
  (= viewport), URL `/`.
- `/console` verbatim : `pas de journal` — **0 erreur**.
- Journal daemon : **19/19 évals `ok=true`, 0 `ok=false`** — dont un
  bundle inline 291 684 o évalué en 137 ms et le tracking
  `__LOADABLE_LOADED_CHUNKS__` (XHR `app_enter`).
- `/mem` : `dom_nodes=159`, `layout_boxes=9`, `display_items=5`,
  `images_bytes=18 037 028` (les ressources images *sont* fetchées),
  **`js_heap_bytes=0`** (anomalie d'observabilité : 19 évals réussies
  mais heap nul — moteur détruit ? métrique morte ?),
  `journal_applies=1`, `journal_fallbacks=0`.
- `/dom` 63 Ko : **vrai HTML Dailymotion** — `__RUNTIME_CONFIG__`,
  `__GKS__`/`__FFS__`/`__EXS__` feature flags, bundle webpack complet,
  stub TCF, `__CLOUDFLARE_TURNSTILE_SITEKEY__`. `<body
  class="Player__body___z3Uc3">` quasi vide : seul enfant rendu =
  `<iframe src="…/pes.iframe.b9ac9c72.js">` (300×150 par défaut).
- `/layout` 9 boîtes : `body` 1366×150, divs enfants toutes à hauteur 0.
- Frame (`captures/vol49_dailymotion.png`) : **blanche byte-identique**
  avant et après scroll (1 844 526 o les deux). `scroll` ×2 → `scroll_y=0`
  (rien à scroller), clic (683,450) → `ignored`.

→ **§B-4(c) « exécution réussie sans effet » recoupé `[V4-main]`** —
même signature que twitch (vol 38) : tout évalue ok, zéro rejet, React
jamais monté, squelette servi mais app jamais hydratée. La cible appuie
le détecteur T2 `evals_sans_mutation`. Note obs : `js_heap_bytes=0`
malgré évals réussies = métrique `/mem` peu fiable en l'état.

## 2. lefigaro.fr — servi puis wedge sans UnmappedKey → §B-25 variant ; §B-11 sain

- `POST navigate` → titre réel, `doc_h=23 684` (clampé — vraie page
  servie), 16 évals `ok=true` + rafale de chunks appconsent (75 Ko, 372 Ko…).
- `/console` verbatim (intégral) :

```
unhandledrejection: TypeError: `delete` is not a function (on Array)
    at <anonymous> (…/cdn.appconsent.io/tcf2-clear/current/core.bundle.js:1:218400)
    … ×8 frames, dernier frame dans initial-vendors.218f34229fccc283.js
uncaught: TypeError: `addEventListener` is not a function (on Object)
    at <anonymous> (…/cdn.appconsent.io/tcf2-clear/33.3.0/5572.bundle.js:1:112366)
    at E (…/1284.bundle.js:1:21728) …
```

→ **§B-11 re-vol : TOUJOURS SAIN `[V4-main]`** — aucune erreur
`new URL`/`Invalid scheme` ; le mur figaro historique n'est plus que le
consent-manager : `` `delete` `` appelé en méthode sur un `Array`
(probablement un `Map`/`Set` retourné en `Array` par une API manquante)
et `addEventListener` sur un objet non-EventTarget → **2 nouvelles
entrées matrice §B-14** (`explorer-jscore`).

### Wedge — DOM détaché permanent, *sans* rejet journal

- Dès la première sonde `/mem` (+10 s) : `dom_nodes=0`, `layout_boxes=0`,
  `display_items=0`, `js_heap=60,3 Mio`, `journal_applies=1`,
  **`journal_fallbacks=0`**.
- `/dom` → 404 `pas de DOM` et `/frame` → **204** en continu pendant
  **>3 min** (~15 sondes espacées) — *pas* l'alternance 200/404
  documentée : détachement terminal.
- Journal daemon : apply #1 complet (`cascade→boxtree→layout→repaint`
  09:07:44,793 ms) puis apply #2 `cascade→boxtree→layout` 09:07:48
  **et plus jamais `repaint` ni `fin`** — l'apply meurt entre layout et
  délivrance, **sans aucun WARN UnmappedKey** (contrairement à §B-25/caf
  et à bfmtv §3 : `journal_fallbacks` reste 0).
- API **restée réponsive** : `scroll` ×2 → `scroll_y=800→1600` projeté
  (doc de 23 684 px), clic → `ignored` + log `input différé — paint en
  vol, rejeté comme ignored` — paint figé en vol, même constat que caf.
- Aucune frame récupérable : `vol49_lefigaro.png` = 0 o (204).

→ **§B-25 recoupé avec variante** — `explorer-core` : le wedge
« DOM détaché permanent + paint en vol figé » se produit **aussi sans**
`journal DOM rejeté/UnmappedKey` : un apply qui meurt silencieusement
entre `layout` et `repaint` laisse le même état terminal. Élargir §B-25 :
« détachement DOM terminal post-apply non délivré » (déclencheurs :
rejet journal *ou* apply figé).

## 3. bfmtv.com — accueil fidèle, article → §B-25 → §B-1 fatal

### Accueil — servi et scrollable (~85 % fidèle)

- `POST navigate` → titre réel, `doc_h=17 291` (clampé). 83 évals dont
  **2 `ok=false`** (09:11:15, 502 ms et 71 ms — postérieures à ma sonde
  `/console` qui disait `pas de journal` ; non revérifié, daemon mort).
- `/mem` : 4 526 nœuds DOM, 4 323 boîtes, 8 930 display items,
  `js_heap=39,8 Mio`, `journal_fallbacks=0`.
- Frame (`captures/vol49_bfmtv.png`) : header BFM + nav rubriques,
  colonne fil d'actu avec horodatages, images réelles (photo boucherie),
  « placeholder video » + « BFM Direct ».
- Scroll ×2 → `scroll_y=1600`, frame scroll
  (`captures/vol49_bfmtv_scroll.png`) : cards articles avec **vraies
  photos** (BFMTV, station-essence, Ballon d'or) — rendu correct.
- **Clic (190,250) sur card « Attentat à Trappes » → `navigated`** :
  vraie navigation déclenchée — **§B-13 continue de tenir `[V4-main]`**.

### Anomalies visuelles observées (accueil)

- **`data:image/svg+xml;charset=utf-8,%3Csvg…` peint en texte brut**
  dans le bandeau header (à droite du logo BFM, à côté de « Replay ») —
  URI data SVG non décodée, rendue comme chaîne littérale. Absente du
  dump `/dom` (probablement `background-image`/`<use>`/attribut) →
  cosmétique render, recoupe la famille « logo SVG aplati » (§B-cosm.)
  mais en variante **texte brut** : candidat entrée §B nouvelle
  (`explorer-render`/`html`).
- **Chevauchement de runs inline** colonne gauche (badge « Live en
  cours » superposé au titre, paragraphe à glyphes doublés
  « Des…s/de syndicaux ens…seignants… ») → **§B-18** (`explorer-layout`).
- Glyphes orphelins `u` devant chaque horodatage et `s` encadré sur les
  vignettes vidéo — icon-fonts retombés en texte (§B-cosm., famille
  tofu/shortcode).

### Article — wedge §B-25 exact, puis starvation §B-1

Chronologie journal daemon (tab=3) :

```
09:11:44  apply: début tab=3 journal=false → apply: fin ms=1267
09:12:01  WARN journal DOM rejeté → repli re-parse (ADR-0025) tab=3 why=UnmappedKey
09:12:02  WARN journal DOM rejeté → repli re-parse (ADR-0025) tab=3 why=UnmappedKey
09:12:20  script: eval fin tab=3 ms=10 ok=true
09:12:21  apply: début tab=3 journal=false
          apply: cascade ms=161 → boxtree ms=173 → layout ms=232
          <<< plus RIEN — ni repaint, ni fin, ni aucune ligne daemon >>>
```

- L'apply de l'article, lancé après les deux re-parse `UnmappedKey`,
  meurt après `layout` (même point de mort que figaro). DOM détaché,
  page jamais servie.
- **Escalade inédite vs caf** : entre ~09:12:30 et 09:17, l'API entière
  se fige — `/api/tabs`, `/mem`, DELETE onglets → timeouts `curl -m 5`.
  RAM 1,72 → **6,88 Go** en ~5 min, montée continue : le re-parse
  fallback et/ou les arbres orphelins fuient sans borne pendant que
  l'exécuteur s'étouffe.
- Arrêt d'urgence : DELETE impossibles (API morte) → `taskkill //PID
  4664 //F`, sonde post-kill morte. Aucun autre daemon touché.

→ **§B-25 reproduit `[V4-main]` à l'identique** (UnmappedKey ×2 →
re-parse → apply figé → DOM détaché) **avec aggravation documentée** :
le wedge **escalade en §B-1** (starvation totale de l'exécuteur) et la
fuite n'est **pas bornée** (6,88 Go > 2,93 Go caf). Le point commun
figaro+bfmtv+caf : **mort d'apply entre `layout` et `repaint`** —
un seul verrou à investiguer dans `explorer-core::session::apply`.

## 4. Télémétrie / observabilité

- RAM daemon PID 4664 : 364 Mo départ → 608 Mo (dailymotion) →
  1,50 Go (figaro wedged) → 1,72 Go (pré-bfmtv) → **6,88 Go** (wedge
  bfmtv, API morte) → kill.
- `/mem` figaro en état wedged : `journal_applies=1`,
  `journal_fallbacks=0` — le compteur de fallbacks **ne bouge pas** pour
  la variante sans UnmappedKey ; seuls `dom_nodes=0` persistant + `/frame`
  204 + input « paint en vol » trahissent l'état → signal T2/métrique
  à proposer : `apply_started_without_fin` ou `paint_in_flight_age_ms`.
- `js_heap_bytes=0` sur dailymotion malgré 19 évals ok — métrique
  suspecte (heap relevé après teardown ? ou jamais alimenté pour ce
  type de page).
- `/console` retourne parfois **404** au lieu de `pas de journal`
  (observé ×1 sur tab=1 entre deux sondes 200) — comportement d'état
  transitoire non documenté.
- Dédup `×N` non sollicité (consoles courtes) ; `unhandledrejection`
  tracé proprement sur figaro.

## 5. Actions proposées

| # | Proposition | Cible |
|---|---|---|
| §B-25 | **Reproduit `[V4-main]` ×2** — bfmtv : signature exacte caf (UnmappedKey ×2 → re-parse → apply figé → DOM détaché) ; **élargir la définition** : le même état terminal survient sans rejet journal (figaro : apply mort entre `layout` et `repaint`, `journal_fallbacks=0`). Point commun unique : apply non délivré post-`layout` | `explorer-core` (`session::apply`, ADR-0025) |
| §B-1 | **Aggravation mesurée** : le wedge §B-25 escalade en starvation totale — API muette (DELETE impossibles), RAM 6,88 Go en ~5 min (vs 2,93 Go caf). La fuite post-wedge n'est **pas bornée** — watchdog N3 encore plus urgent | `explorer-core`/daemon |
| §B-4(c) | Recoupe : dailymotion 19/19 évals `ok=true`, 0 rejet, React jamais monté, frame blanche byte-identique — identique twitch | `explorer-jscore`/`core` — détecteur T2 |
| §B-11 | **Re-vol figaro : reste VÉRIFIÉ SAIN `[V4-main]`** — aucune erreur `new URL`/`Invalid scheme` | `explorer-jscore` — statut confirmé |
| §B-14 | +2 entrées matrice : `` `delete` is not a function (on Array) `` (Map/Set→Array ?) et `` `addEventListener` is not a function (on Object) `` — appconsent.io tcf2 (figaro) | `explorer-jscore` — J5 |
| §B-18 | +1 recoupe : chevauchement runs inline colonne actu bfmtv (badge « Live en cours » + paragraphe doublé) | `explorer-layout` |
| §B-nouveau (cosm.) | **`data:image/svg+xml` peint en texte brut** dans le header (bfmtv) — URI data non décodée rendue littérale ; absent du `/dom` → render/css | `explorer-render`/`css` |
| §B-13 | Tient `[V4-main]` : clic card bfmtv → `navigated` réel | `explorer-layout`/`core` |
| obs | `js_heap_bytes=0` malgré évals ok (dailymotion) · `/console` 404 transitoire · métrique `apply_started_without_fin`/`paint_in_flight_age_ms` proposée (détecter le wedge sans UnmappedKey) | `explorer-core` — télémétrie |

**Statuts proposés** : §B-25 `OUVERT — reproduit [V4-main] ×2,
variante sans UnmappedKey, escalade §B-1 non bornée (6,88 Go)` ·
§B-4(c) enrichi (dailymotion) · §B-11 `VÉRIFIÉ SAIN [V4-main]` ·
§B-14 `PARTIEL — +2 entrées` · §B-18 `PARTIEL — +1 recoupe` ·
§B-13 `RÉSOLU confirmé [V4-main]` · nouvelle entrée cosmétique
`data:image/svg+xml` peint en texte à créer au prochain numéro.

### Fichiers produits

- `captures/vol49_dailymotion.png` · `vol49_dailymotion_scroll.png`
  (blanches byte-identiques) · `vol49_lefigaro.png` (0 o — `/frame` 204,
  wedge) · `vol49_bfmtv.png` · `vol49_bfmtv_scroll.png`
- `logs/vol49_dailymotion_{dom,layout,mem.json,console}.txt` ·
  `vol49_lefigaro_{console,dom,layout,mem.json}.txt`
  (`dom`/`layout` = « pas de DOM »/« pas de layout » — preuve wedge) ·
  `vol49_bfmtv_{console,dom,layout,mem.json}.txt`
- `_vol49_daemon.log` (racine, 2,7 Mo) — WARN UnmappedKey ×2 tab=3
  (09:12:01-02), dernier apply figé 09:12:21 post-`layout`
