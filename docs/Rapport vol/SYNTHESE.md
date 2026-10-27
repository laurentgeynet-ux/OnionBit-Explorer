<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Synthèse consolidée des vols — 2026-10-26/27

Synthèse de **49 documents** : 12 rapports de mission équipes
(vague 1, ~40 sites), 34 rapports de vol agents (vague 2, ~140 sites),
3 rapports de clôture FIX (vague correctifs, ~45 commits).
Corps total : ~180 cibles réelles, captures `captures/`,
télémétrie `logs/`.

> Chemins `captures/` et `logs/` cités dans les rapports : relatifs à
> la racine de ce dossier (`docs/Rapport vol/`).

---

## A. État des correctifs — ce que les vagues FIX ont résolu

Trois branches : `nav/vol-fix-2026-10-26` (jscore, JS-C134→162),
`nav/vol-fix2` (html/css/layout/render), `nav/vol-fix3`
(core/net/api/daemon).

### FIX-1 — `explorer-jscore` (~20 fix)

| Fix | Mur levé |
|---|---|
| JS-C134/139 regex : surrogate pairs, classes `\uXXXX`, `-` littéral, multi-items astraux | `invalid regex` gitlab/framagit/youtube/pinterest/bsky/guardian/ecma — **le tueur de bundles n°1** |
| JS-C135 `XMLHttpRequest.prototype` réel + méthodes | `'in' requires an object` (excalidraw), `setRequestHeader` (soundcloud), `fetch` (watch) |
| JS-C136 `XPathEvaluator.createExpression` | htmx.org, codeberg |
| JS-C137 `Intl.supportedValuesOf` | bandcamp |
| JS-C138 `Worker.addEventListener` + scope worker | openstreetmap (partiel : Leaflet démarre, tuiles non) |
| JS-C140 `Range.createContextualFragment` | cascade `sc_*` ×19 maps/support |
| JS-C141 interfaces IDB déclarées | `IDBIndex is not defined` rakuten |
| JS-C142 `window` hérite `EventTarget.prototype` | `__shady_native_addEventListener` (polyfill webcomponents YouTube) |
| JS-C143 commentaires exclus de `serialize_keyed` | **`<#comment>` peint en texte** — confirmé 0 occurrence au re-vol 22 |
| JS-C144 index `Insert` = enfants sérialisables | `<li>` sortis de leur `<ul>` (craigslist/lobste.rs), head→body (canada.ca) |
| JS-C145/146 `yield` dans `switch` + `in_gen` restauré après fn imbriquée | `yield outside a generator` github, `expected ';'/')'` spotify/netlify/GOG/coinmarketcap/cdiscount xcare |
| JS-C147 `this` sloppy boxé (ToObject) | `toLowerCase is not a function` jQuery (baidu, legifrance, soundcloud) |
| JS-C148 `head`/`body`/`html` auto-vivés | `appendChild` en boucle (jsfiddle ×12, cdiscount ×3811 probable) |
| JS-C149 `src`/`href` résolus contre `baseURI` | `Invalid URL`/`Invalid scheme` (partiel — cf. §B-9) |
| JS-C150 `super.m()` en ctor dérivé (`[[HomeObject]]`) | **`value is not a function` DSFR** — débloque legifrance/service-public/impots |
| JS-C151 `canvas.toDataURL`/`toBlob` | feature-detects anti-bot (aliexpress sufei-punish) |
| JS-C152 `atob` chaîne-octet Latin-1 | bsky bundle Sentry |
| JS-C153 `AbortSignal.throwIfAborted` + statique | youtube kevlar (restant vol 22 → désormais livré) |
| JS-C154 `unhandledrejection` → PipelineError | morts silencieuses — validé terrain (deezer/genius) |
| JS-C155 `URL.canParse`/`parse` | audit surface ES2023+ (70 entrées, tout le reste présent) |
| JS-C156 `JsError::Timeout` typé | signature « moteur figé » propre côté session |

### FIX-2 — pipeline (~15 fix)

`noscript` UA `!important` (mastodon) · échecs sous-ressources →
console (craigslist) · `pointer-events:none` hit-test · SVG
`viewBox` sans dims → `min(300px,100%)` (sr.ht, slashdot, w3.org,
kayak, indeed) · `grid-column` négatives `1/-1` (reddit 32 px) ·
`[popover]` fermé masqué (github, wikidata) · `opacity:0`
court-circuite (bookshop) · `inset` shorthand (ancres guardian) ·
flex column fit-content (quora) · **tables anonymes §17.2.1** +
`table-caption` (IGN, figures wikipedia) · hit-test ordre
d'empilement · `min()/max()/clamp()` (stripe min-content) ·
`text-overflow:ellipsis` + `nowrap` · `fixed` échappe au clip
`overflow` ancêtre (bandeau indeed) · bordures UA `input` · PUA
`.notdef` en slot muet (tofu icon-fonts) · `iframe` 300×150.

### FIX-3 — core/net/api (~15 fix)

Onglet parasite `.js` refermé (rakuten, soundcloud) · `/frame` 204
au lieu de 200 vide + **viewport par défaut** (`?full=1` document) ·
scroll différé rapporte projection (franceinfo) · images lazy
`data-src` collectées en repli (lemonde ~195, marmiton 216) · meta
refresh `<noscript>` honorée (google SERP, DDG) · **console vidée à
la navigation** + capacité 64→512 + dédup « ×N » · **disjoncteur
« moteur figé »** (cooldown 10 s, évals en vol sautées — kayak 90 s,
doctolib wedge) · `yield` inter-pompes multi-onglets · dyn import →
warn + PipelineError · budget daemon 20 M ops · keydown → `body`
(wordle) · hit-test conscient de `transform` · **AES-GCM côté
plateforme net** (ops jscore Encrypt/Decrypt restent à livrer) ·
**CSP `style-src https://*`** corrigé (dropbox, todoist) · repli
`rel=preload as=style` · scripts en échec tracés.

### Re-vols de confirmation

- **Agent 22 (post-FIX)** : toutes les erreurs verbatim d'avant
  disparues ; **Excalidraw montée** (100 % blanche → app rendue) ;
  0 `<#comment>` sur 5 cibles ; youtube scroll/doc_h sains.
- **OSM (vol 27)** : `Worker` fix confirmé — Leaflet s'instancie
  entièrement, reste `call` of Undefined avant les tuiles.
- **Vol 34 (integ V3)** : clics = vraies navigations (marmiton,
  750g) — progrès net vs vols où tout clic restait `ignored`.

---

## B. Anomalies restantes — par gravité

### 🔴 Critique — processus/daemon, pages jamais servies

| # | Anomalie | Crate | Preuve | Statut |
|---|---|---|---|---|
| 1 | **Starvation de l'exécuteur post-apply** : la page rend, puis une rafale JS (timers/microtasks, hors éval) sature un cœur → API muette >3-6 min, RAM 2,9 Go, zéro log après `apply: layout` | `explorer-core`/daemon | **allrecipes.com ×2** (vol 34, binaire V3 — survit aux fixes), jscore 32 k lignes | **OUVERT** — le disjoncteur couvre les évals, pas les tâches rafale ; watchdog/budget global absent |
| 2 | **Boucle infinie de navigations auto** (meta refresh / `location.*` cumulés) | `explorer-core` | cdiscount : 3 812 navs, 20 001 streams, 16 min, kill manuel (éq. 6) | **RÉSOLU `[V4-main]`** — vol 36 : 0 nav auto en 5 min, URL stable ; disjoncteur validé en conditions réelles (script 507 Ko tué à 10 s, API responsive) |
| 3 | **Stack overflow** `explorer-jscore` — récursion parser/AST non bornée | `explorer-jscore` | alpinejs.dev : thread mort (éq. 2) | **FIX LIVRÉ** — transporteurs mesurés bornés : C134 (drops AST + walk JSON), JS-C166 `deleg_hop` (chaînes proxy ×20k → `RangeError`, 10 méthodes internes + `bind`×N sur `instanceof`, sondes `depth_probe` 47/47) + `stack_guard` plancher pile native (callbacks `forEach`→callback hors `call_depth`) — **RE-VOL VOL 46 `[V4-main]` : TOUJOURS PRÉSENT — le process meurt** : `overflowed its stack` ~110 ms dans l'éval d'un script 55 935 o (14 évals ok avant), abort `STATUS_STACK_OVERFLOW` non rattrapable. L'overflow est à l'**exécution** (interprète récursif), pas au parse — C134/`deleg_hop`/`stack_guard` hors chemin ; suggéré : trace URL par éval + garde runtime |
| 4 | **Morts silencieuses résiduelles** : boot JS échoue sans aucune entrée console | `core`/`jscore` | pinterest (vol 22 post-fix, toujours muet), twitch, archive.org, radiogarden (33 scripts, 0 erreur), play2048, cartes.gouv (**sorti du silence `[V4-main]` vol 50** : `unhandledrejection: unsupported encoding ×4` tracé → §B-14), windy/flightradar (modules+importmap inertes) — **bras frais `[V4-main]` vol 36** : defer Next.js non exécutés sans erreur (GFG), tuiles Leaflet jamais demandées (OSM — `call of Undefined` mort, étape suivante muette) | **EN NETTE DÉCROISSANCE** — `unhandledrejection` + warns fetch stylesheet/script/iframe + dyn import livrés ; **3 bras précisés `[V4-main]` vols 38/43** :
(a) **drains modules/defer inertes** — windy : `type="module"`+importmap 0 trace, heap 8,7 Mio (drain top-level absent ?) ; pinterest ~70 scripts 0 éval ; GFG defer Next ;
(b) **exceptions post-instanciation** — archive.org **tracé** (`unhandledrejection: append/get of Undefined` ×2) ;
(c) **exécution réussie sans effet — NOUVEAU, le plus opaque** — twitch : 44/44 évals `ok=true` (bundles 355 Ko), 0 rejet, React jamais monté → instrument T2 `evals_sans_mutation` proposé comme détecteur |
| 5 | **`import()` : dep absente du MapLoader** — « module non chargé : {url absolue} » | `explorer-js`/`core` | mastodon, chess, letterboxd, marmiton | **FIX LIVRÉ** — racine mastodon fermée : `max_modules` 64→256 + warn `tronqué` (`2b61012`) ; re-vol post-fix : graphe `application-*.js` = 121 sources, axios (n°65+) chargé et instancié, erreur disparue. Restes §B : refus fetch / spec non extrait d'import map (J3) ; chess/letterboxd/marmiton à re-voler |
| 6 | **`import()` sur `data:` URL refusé** (résolution/CSP) | `explorer-core` | reddit ×2 ; confirmé `[V4-main]` vol 38 (refus identique **désormais tracé WARN** — fin du silence sur ce bras) | **OUVERT** — WIP équipe (J3 chantier `data:` modules) |
| 7 | **`visibility:hidden` jamais levé** — page blanche totale malgré DOM/layout complets | `jscore`/glue | geeksforgeeks (vols 30+36 `[V4-main]` : frame blanche byte-identique, wrapper `hidden` intact ~25 s, **defer Next non exécutés sans erreur** → recoupe §B-4/§B-19) | **OUVERT — confirmé V4** |
| 8 | **Frame blanche au scroll** — layout complet à y=5520, frame blanche à scroll_y=1600 | `explorer-render` (suspect `build_display_list_incremental`) | glassdoor (vol 26) | **OUVERT** — audit sans trou démontré, repro attendu |

### 🟠 Majeur — sites dégradés, classes de bugs

| # | Anomalie | Crate | Preuve | Statut |
|---|---|---|---|---|
| 9 | **Parser résiduel** : `` `...` `` « outside a call or array literal » encore vu **post-fix** ; `Cannot call a class as a function` (tiktok — cascade honnête) ; `constructor is not a function: function(){[code]}` (gitlab rspack — diagnostic posé) ; `JSON.parse unexpected token` (gitlab GraphQL — troncature ?) | `explorer-jscore` | bbcgoodfood `_app` (vol 34 V3), ikea sentry | **OUVERT** (rest/spread en position non couverte par JS-C146) |
| 10 | **Scope global inter-scripts** : `var`/assignations top-level non propagées à `window` partagé | `explorer-jscore` | lemonde `lmd` ×10, ouest-france `firstparty.push` ×3 — **bras RÉSOLU `[V4-main]` vol 48** (23/23 évals ok, `window.firstparty` posé, console vide), OSM `OSM.i18n` | **VÉRIFIÉ SAIN** — sonde session `global_var_shared_between_script_els` : `var`/`window.=`/undeclared traversent les `<script>` bout à bout, script mort à mi-éval laisse ses liaisons hissées (`dead=7`, `later` bound) — les `lmd`/`didomi`/`OSM` absents = scripts définisseurs jamais exécutés (**cascade §B-4**, pas un bug de scope) |
| 11 | **`new URL(".", location)`** — base `Location` non stringifiée ; `Invalid scheme` sur chemins `./` non résolus | `explorer-jscore` | fastapi, svelte, lemonde, figaro ; cdiscount `unhandledrejection: Invalid scheme` ×5 `[V4-main]` vol 36 | **VÉRIFIÉ SAIN** — `url_ctor_with_location_base`/`new_url_against_location_resolves` : `new URL('.', location)` → base correcte, résolution `./`/`x` OK ; les `Invalid scheme` restants = autre forme d'entrée à nommer au prochain vol (attribution tracée) |
| 12 | **`heap memory limit exceeded` en rafale** | `explorer-jscore` (budget ou fuite) | marmiton ×3 au boot (vol 34 — **non reproduit `[V4-main]` vol 36**), bookshop ×14 beacon.min.js | **OUVERT** — capper à 1 occurrence + mesurer le seuil |
| 13 | **Hit-test `ignored` en série** — **confirmé sain `[V4-main]` vol 37** (figma clic→nav réelle) — liens/inputs réels non trouvés | `explorer-layout`/`core` | vols 26/29/30 (V2 integ) : 4/4 clics refusés | **PROBABLEMENT RÉSOLU** — cause identifiée : wrappers `visibility:hidden` en attente d'hydratation (GFG), pas un bug hit-test ; **clics = vraies navigations V3/V4** (vols 34/37/44/45) ; **formulaire bout-en-bout validé** (ddg lite : focus→saisie→Enter→SERP→article, vol 44) | **RÉSOLU confirmé `[V4-main]`** — attention variante : élément sous overlay reste `ignored` (§B-18 qwant, §B-17 ratp) |
| 14 | **APIs DOM restantes** : `document.write` ordonnancé post-commit (tumblr), `instanceof Element` trop strict (impots, ameli), `document` of Null (dw), `Illegal component name ""`/`Function.name` (dw), `debugger` token tue un module (w3schools), `get`/`bind` of Undefined (marinetraffic, tiktok — cascades à nommer), `GeolocationPosition is not defined` (marmiton `[V4-main]`), `document.contains` absent (ratp `[V4-main]` vol 45), `Node.replaceChild: not a node` (dsfr-1.14 service-public `[V4-main]` vol 47), `customElements.define` reçoit `undefined` (reddit vol 38), `Response.json` absent (regex101 `'json' is not a function` `[V4-main]` vol 55) | `explorer-jscore` | vols 7/8/16/18/23/24/25/27-30/36 | **PARTIEL** — `call of Undefined` Bootstrap **RÉSOLU confirmé `[V4-main]`** (JS-C162 — console OSM vide, Leaflet instancié ; reste tuiles absentes → §B-4) ; `attachShadow` sorti du stub minimal ; reste la matrice ci-contre |
| 15 | **Shadow DOM / custom elements** : `attachShadow` stub, `faceplate-*`/`shreddit-*` 0×0, slotted content absent | `explorer-jscore` | reddit login (28 partials vides) ; **`[V4-main]` vol 38 : DÉGRADÉ EN MIEUX** — challenge JS franchi, vraie page r/rust peinte (13 shreddit-post avec contenu), upgrade custom elements tenté (`customElements.define` reçoit `undefined` ×3 → §B-14) | **STRUCTUREL, amorce livrée** — surface JS posée (attachShadow→fragment, getElementById, instanceof, proto chaîné WebSocket/BC/EventSource lisibles) ; slotted/custom-elements upgrade reste le chantier (issue #25) |
| 16 | **WebGL/canvas lourd** | `jscore`+`render` | threejs, ciechanowski, pixi, radio.garden, flightradar | Hors scope court terme (ADR stubs) — stub Ctx2d/toDataURL livrés |
| 17 | **Menus/overlays résiduels** — **reproduit `[V4-main]` figma** (méga-menus peints ouverts landing+interne) : ceux fermés **par JS** restent ouverts : ceux fermés **par JS** restent ouverts (le CSS est corrigé : `hidden`, `[popover]`, `opacity`, `inset`, `details`) ; off-canvas GOG/IGN, backdrop francetravail, megamenus 750g/doctolib | `jscore` (JS mort) + résiduels css | nombreux vols | **EN DÉCROISSANCE** — reste dû aux boots morts, pas au pipeline ; **sous-cas confirmé `[V4-main]` vol 45** : nav off-canvas ratp fermée mais enfants déployés/peints/interceptant hit-test (clic burger `ignored`) |
| 18 | **Layout résiduel** : chevauchements de runs inline (excalidraw/gitlab/DW/deezer — non reproduit), `overflow-x` clip (guardian), marquee dupliqué (vercel), sticky repaint (accuweather), dark theme classe JS (vercel), double émission suspectée synchro DOM (franceinfo/gitlab header), bandeau noir 250px CNN, runs éphéméride wikipedia `[V4-main]` vol 56, **runs inline superposés confirmés `[V4-main]`** (lemonde « À Marseille… » + en-tête ouest-france vol 48), **hero img occluant l'input recherche** (qwant `[V4-main]` vol 44 : input layouté mais 884×772 img par-dessus → clic `hit=true` sur l'image, saisie impossible) | `explorer-layout`/`render` | vols 11/13/20/22/31/32/33/44 | **PARTIEL** — **hero colonne RÉSOLU** (calc `min/max/clamp` factor + plancher min-content du shrink flex, repro notion verbatim vert) ; restes à re-voler |
| 19 | **`document.write`/`async` ordonnancement** — scripts dyn drainés post-commit, pas en ordre de parse | `explorer-core` | tumblr main-*.js jamais exécuté | **OUVERT** — chantier profond documenté |
| 20 | **Images `data-src`** : repli livré (840a33c) mais marmiton encore gris | `explorer-core` | vols 34+36 `[V4-main]` : 215 `data-src`, `<img>` sans `src`, gris à +30 s et scroll | **TOUJOURS PRÉSENT `[V4-main]`** — le repli ne produit rien sur cette cible |
| 21 | **Console `×N` ne borne pas la source** : `r is not defined` ×512 (genius reactAds) = retry/intervalle non borné | `jscore` budget | vol 33 | dédup visuel OK ; boucle applicative reste |
| 22 | **Redux `dispatch` : action non-plain** — `Minified Redux error #7` (« Actions must be plain objects ») : une action non-plain traverse la chaîne middleware/thunk jusqu'au dispatch de base | `explorer-jscore` | mastodon `/about` (vol opérateur, binaire post-C165) — l'app exécute 121 modules jusqu'à `createStore` | **OUVERT** — suspect : composition `applyMiddleware`/`typeof` action thunk non interceptée ; immer résolu (J10) |

| 23 | **`eval deadline` trop agressif au boot** — 2ᵉ cas terrain : lemonde `[V4-main]` vol 48 (script 254 Ko tué à 30 s → disjoncteur, heap 0) — compile légitime tuée | `explorer-jscore`/`js` | cryptpad `[V4-main]` vol 37 : `RangeError: eval deadline exceeded` sur less.min.js → boot RequireJS mort (J1 livré mais seuil bas — à étalonner/exempter phase boot, lien J1/N3) | **OUVERT — terrain V4** |
| 24 | **Stretched-link incliquable** : `<a>` `position:absolute`+`inset:0` → boîte 235×0 px, hit-test `ignored` sur toutes les cards | `explorer-layout`/`css` | vinted `[V4-main]` vol 39 : rendu ~95 % fidèle mais entièrement incliquable ; `logs/vol39_vinted_layout.txt` 15 ancres ×0 hauteur — le fix `inset` ne couvre pas le stretch vertical (hauteur parent auto ?) | **OUVERT — terrain V4** |

| 25 | **Navigation `location` spontanée non sollicitée** — `location.href` posé sans input (détection env ?) | `explorer-jscore`/UA | qwant `[V4-main]` vol 44 : redirigé vers `/mobile-apps?utm_campaign=w2m` — nav unique (§B-2 tient) mais suspect `navigator.*`/UA feature-detect | **OUVERT — terrain V4** |

| 26 | **Wedge DOM détaché terminal post-apply** — `journal_fallbacks` non livré OU apply mort entre `layout` et `repaint` → `/dom` 404, `/frame` 204, **fuite mémoire sans borne** (caf 2,93 Go vol 47 ; **bfmtv 6,88 Go en 5 min vol 49 — escalade §B-26→§B-1, API entière morte** ; figaro variante sans UnmappedKey) | `explorer-core` | caf/figaro/bfmtv/radiofrance `[V4-main]` vols 47/49/54 (radiofrance : apply en escalade 0,4→9,2→>49,5 s, +90 Mo/s, API vivante) — signature commune : apply journalisé jusqu'à `layout`, jamais `repaint`/`fin` | **CRITIQUE — suspect n°1 du crash poste ; métriques proposées : `apply_started_without_fin`, `paint_in_flight_age_ms`** |

| 27 | **Image en mosaïque géante plein viewport au scroll** | `explorer-render`/`layout` | ouest-france `[V4-main]` vol 48 scroll_y=1800 — tuile répétée couvrant tout l'écran (background-repeat ? border-image ?) | **OUVERT — terrain V4** |

| 28 | **`height:100%` plein-viewport effondrée à 0** — chaîne `html`/`body`/`#map` = `1366x0` → Leaflet lit `clientHeight≈0`, ne demande qu'1 tuile | `explorer-layout`/`css` | uMap éditeur `[V4-main]` vol 50 — tuiles OSM peintes mais page à hauteur 0 (paradoxe : tuiles visibles) | **OUVERT — terrain V4** |

| 29 | **Boucle iframes `sodar` non bornée** — AdSense crée des iframes télémétrie en continu : 419 dans le DOM (+~90/min), `js_heap` 53→160 Mo (~40 Mo/min), cycles apply ~1,3 s | `explorer-core`/budget | linuxmint `[V4-main]` vol 51 — ad-tech fonctionnel (slot peint) mais DOM+heap gonflent ; API responsive, RAM libérée au DELETE | **OUVERT — famille §B-1/§B-21, signature DOM+heap** |

| 30 | **Fuite mémoire post-DELETE sans borne** — croissance +~28 Mo/s après fermeture de tous les onglets (`{"tabs":[]}`), API responsive, 0 log | `explorer-core` | manomano `[V4-main]` vol 53 : 1,7→2,98→4,27 Go avec zéro onglet — suspect : tâche `POST navigate` orpheline (jamais retournée) allouant après destruction | **CRITIQUE — repro : navigate + DELETE pendant in-flight ; distinct de §B-26** |

| 31 | **Clic `<button onclick>` → `ignored`** — boîte bien layoutée (1311×59), clic centré, refusé ; bloque les apps à bouton primaire (éditeur photopea injoignable) — `<a target=_blank>` fonctionne | `explorer-core`/hit-test | photopea `[V4-main]` vol 55 — suspect : handler `onclick` attribut non branché côté hit-test | **OUVERT — terrain V4** |
| 32 | **Rasters pixelisés basse résolution** — icônes/logo/éditeur en blocs (downscale ou upscale brut ?) | `explorer-render` | regex101 `[V4-main]` vol 55 (certitude moyenne — verifier si srcset/lowres) | **OUVERT — terrain V4** |

| 33 | **Wedge `nav_paint` sans timeout** — `take_nav_paint → Busy → sleep(50ms)` en boucle à vie (`shared.rs:157-166`) : `POST navigate` jamais retourné (>25 min), DOM 404/frame 204 permanent, 0 WARN, `journal_fallbacks=0`, ~1,5 cœur saturé | `explorer-core` | francebleu→ici.fr + 20minutes `[V4-main]` vol 52 — wedge en amont (`insert_images`/`styles_payload`) ou `composite_frames` ; distinct de §B-26 (fallbacks=0, navigate en vol) | **CRITIQUE — racine localisée : ajouter timeout + abandon payload** |

| 34 | **`max-height` (%) ignorée sur élément remplacé** — `<img>` taille intrinsèque 703×994 dans `div.h-48` au lieu de ≤192 px, clip `overflow` | `explorer-css`/`layout` | cjoint `[V4-main]` vol 56 (`img [-159 -222 703x994]` dans div 366×192) — bras opposé de §B-28 | **OUVERT — terrain V4** |

### 🟡 Moyen — perf, métriques, observabilité résiduelle

- **Framebuffer pleine page** : PNG 48-54 Mo (guardian, youtube) —
  `/frame` sert désormais le viewport ; le **raster plein document
  reste** (repaint 16 s guardian) → tiling/plafond à planifier.
- **Mémoire** : bfcache ne retient plus rasters/PNG ; **heap JS gelé
  ~50 Mio/page domine** (note FIX-1 : politique de trim à décider).
- **`doc_h` trompeur** : =viewport sur pages mortes ; clamp 16 384/scale
  documenté ; `doc_h_clamped` informatif.
- **`/console` >10 s** : non reproduit — suspect runtime affamé,
  ring lui-même bref.
- **Timeouts « jprobe » 25 s non qualifiés** : curl.se,
  developer.apple.com, phoronix, markojs, pixijs, konvajs — cause
  inconnue (fetch ? script ? eval ?).
- **`<noscript>` fallback DDG `/html`** : couvert par le repli
  head+fallback pur.
- **Anomalies cosmétiques** : avatars discourse absents (DOM sans
  `<img>`), `<img>` cassée sans picto 16×16, skip-links peints,
  `text-overflow` livré, emoji shortcode `:waving_hand:` littéral
  (fidèle source), logo SVG aplati (reddit — `<use>` cross-îlots
  non résolu, **ouvert** render).

### ⚪ Externe — hors moteur (catalogue à jour)

Anti-bot : 403 sec (ebay, etsy, stackoverflow, tripadvisor, sncf-connect, affinity.serif, hardware.fr, liberation.fr,
seloger, medium, khanacademy, apec, nytimes) · Datadome (leboncoin) ·
Akamai Bot Manager (cdiscount, zalando→`supported-browsers`) ·
x5sec Alibaba (aliexpress→punish) · **AWS WAF insolvable** (imdb,
booking — challenge obfusqué) · challenge JS reddit **résolu côté
serveur** · login-walls (old.reddit, quora partiel) · Cloudflare
urssaf.fr RST 10054 reproduit curl schannel `[V4-main]` vol 45 · Turnstile (quora). Signature cumulée : `UA: Explorer/0.1`, pas de
`sec-ch-ua`/`sec-fetch-*`, fingerprint rustls, H2 par défaut —
politique déléguée à l'app hôte (AGENTS §9).

---

## C. Acquis mesurés

- **Tolérance** : 0 panic Rust livré hors le stack overflow jscore
  (1 cas). Erreurs typées partout.
- **SSR-first passe** : bbc (64 scripts ok), data.gouv (34 modules
  Nuxt), hacker news (référence : clic→commentaires), ikea ~90 %,
  discourse crawler, yandex, 750g.
- **Navigation réelle** : clic→nav, back/forward bfcache, consentement
  Google franchissable, ancres `#`, redirections locales/instit.
- **Réseau** : jamais en cause hors timeouts non qualifiés — H2
  multiplexé ~200 streams (amazon), GOAWAY absorbé, redirects UTF-8.
- **Observabilité** : ring console per-tab, reset à nav, PipelineError
  sous-ressources+scripts+dyn import, `unhandledrejection`, dédup ×N,
  iframe XFO warn, défaut `info` daemon.

## D. Priorités résiduelles (ordre suggéré)

1. **Watchdog/budget JS global post-apply** (allrecipes — seul
   blocage dur restant en V3) + `depth_limit` jscore (stack overflow).
2. **Re-vol boucle nav** (cdiscount) pour valider `max_auto_navs`.
3. **Bras MapLoader `import()`** + `data:` modules — dernier verrou
   des SPA code-splittées.
4. **Traçage « script non exécuté / rejet non tracé »** — éliminer
   les morts silencieuses résiduelles.
5. **Scope global `var`→`window`** + `new URL` 2-args + spread `...`
   résiduel — trois fixes courts à fort dénominateur.
6. **Heap budget** : seuil + 1 occurrence max du RangeError.
7. **Matrice APIs DOM** (write ordering, `instanceof Element`,
   Function.name, document Null) — chaque entrée débloque un site ;
   **JS-C163** (deadline wall-clock éval, spec prête §reprise 16
   FIX-1) débloque le point 1.
8. **Re-vols ciblés** : hit-test V2→V3, `data-src` marmiton,
   franceinfo/vercel résiduels layout, glassdoor scroll.

## E-bis. File de tâches par piste

Découpage de §B/§D par propriétaire (AGENTS §10). `NAV-JS` =
piste spécialiste JS (jscore/js) ; `NAV` = session/core/net/
daemon/app ; `PIPE` = html/css/layout/render ; `Terrain` =
vols/observabilité (transverse).

### NAV-JS — `explorer-jscore` / `explorer-js`

| # | Tâche | Source | État |
|---|---|---|---|
| J1 | **Deadline wall-clock éval** (`EXPLORER_JS_EVAL_TIMEOUT_MS`) — rafale timers post-apply saturant un cœur (allrecipes : API muette 3-6 min, RAM 2,9 Go) → `JsError::Timeout` → disjoncteur existant | §B-1, §D-1 | **livré** (JS-C164, `1ec32d5` — par batch, disjoncteur armé) — à re-voler allrecipes |
| J2 | `depth_limit` parser/AST — le seul crash process (alpinejs) ; C134 a borné drop AST + walk JSON, pas la descente syntaxique | §B-3 | **fix livré** — JS-C166 `deleg_hop` (boucles bornées : 10 délégations proxy + `bind`×N `instanceof`, `e044f69`) + `stack_guard` pile native ; sondes `depth_probe` ×20k vertes — à re-voler alpine |
| J3 | Bras MapLoader `import()` : `fetch_module_sources` ne collecte pas tout (`max_modules`, refus, spec non extrait) + `data:` refusé en module | §B-5/6 | partiel — `data:` livré (JS-C131), cap 64→256 + warn `tronqué` (`2b61012`) ; reste refus/spec non extrait |
| J4 | Heap budget : capper `heap memory limit exceeded` à 1 occurrence + mesurer le seuil réel (marmiton, bookshop beacon) | §B-12 | ouvert |
| J5 | Matrice APIs résiduelles : `instanceof Element` trop strict, `Function.name` classes minifiées, `document` of Null (dw), token `debugger` tuant un module, `Illegal component name ""` | §B-14 | partiel (C162 Bootstrap, attachShadow sorti du stub) |
| J6 | Spread/rest `...` « outside a call or array literal » — position non couverte par C146 | §B-9 | ouvert |
| J7 | `document.write`/async : ordonnancement post-commit vs ordre de parse (tumblr) | §B-19 | chantier profond documenté |
| J8 | Boucle applicative non bornée (`r is not defined` ×512 — intervalle retry en rafale) | §B-21 | dédup visuel OK ; borne runtime à poser |
| J9 | `visibility:hidden` jamais levé (geeksforgeeks — hydrate JS) | §B-7 | ouvert |
| J10 | Proxy : traps reçoivent `PropertyKey` symbol (immer/mastodon `isDraft`) | §B-9/19 | **RÉSOLU** — JS-C165 : `prop_trap_key` démange `sym_index`, re-vol mastodon : `ownKeys` TypeError disparu, app atteint `createStore` |

### NAV — `explorer-core` / `net` / `daemon` / `api`

| # | Tâche | Source | État |
|---|---|---|---|
| N1 | **Re-vol cdiscount** — valider `max_auto_navs` (É144) + sentinel death daemon | §B-2 | armé, attend le vol |
| N2 | Timeouts « jprobe » 25 s non qualifiés (curl.se, apple.dev, phoronix, markojs, pixijs, konvajs) — qualifier fetch/script/eval | §D-4 | ouvert |
| N3 | Watchdog/budget **post-apply** transversal (RAM 2,9 Go + saturation cœur) — couvre ce que le disjoncteur évals ne voit pas | §B-1 | dépend de J1 |
| N4 | Morts silencieuses résiduelles : MapLoader bras + drains modules (pinterest, twitch, windy/flightradar importmap) | §B-4 | en décroissance — tracé à compléter |
| N5 | `document.write` ordonnancé post-commit (avec J7) | §B-19 | chantier profond |
| N6 | `data-src` marmiton gris en V3 — re-valider sur binaire post-fix | §B-20 | à re-voler |

### PIPE — `html` / `css` / `layout` / `render`

| # | Tâche | Source | État |
|---|---|---|---|
| P1 | Frame blanche au scroll (glassdoor, `build_display_list_incremental` suspect) | §B-8 | ouvert — repro attendu |
| P2 | Résiduels layout : chevauchements runs inline (excalidraw/gitlab/DW), `overflow-x` clip guardian, marquee vercel, sticky accuweather, double émission synchro franceinfo/gitlab, bandeau CNN 250 px | §B-18 | partiel (hero colonne + min-content livrés) |
| P3 | SVG `<use>` cross-îlots non résolu (reddit logo aplati) | §B-cosm. | ouvert |
| P4 | Framebuffer tuilé + plafond hauteur (PNG 48-54 Mo guardian/youtube, repaint 16 s) | §B-perf | à planifier |
| P5 | Off-canvas/backdrop JS-closables restants une fois boots réparés — revérifier après J5 | §B-17 | en décroissance |

### Terrain / observabilité — transverse

| # | Tâche | Source | État |
|---|---|---|---|
| T1 | Re-vols ciblés : cdiscount (N1), hit-test V2→V3, data-src marmiton, franceinfo/vercel, glassdoor | §D-8 | attend binaire |
| T2 | `evals_sans_mutation` compteur + promotion WARN — morts silencieuses auto-détectées | §B-4/obs | instrument proposé |
| T3 | Heap JS gelé ~50 Mio/page — politique de trim (bfcache) | §B-perf | décision à prendre |
| T4 | `doc_h` trompeur / clamp documenté — metrique `doc_h_clamped` | §B-perf | documenté |

## E. Organisation du dossier

```
Rapport vol/
  SYNTHESE.md                  ← ce document
  rapports/
    equipes/                   vague 1 — RAPPORT_MISSION_EQUIPE_1..8 (+VOL2)
    agents/                    vague 2 — RAPPORT_VOL_*_AGENT1..34
    fix/                       clôtures FIX-1/2/3 (commits livrés)
  archives/                    zip mission équipe 4 (instantané)
  captures/                    757 PNG (volN_*, 2026-10-26_*Equipe_*)
  logs/                        516 fichiers (volN_*, eqN_*, *_console)
    daemons/                   journaux daemon _volN_* remontés de la racine
    scratch/                   fixtures verbatim téléchargées par les agents FIX
```

---

## F. Addendum — fixes mergés après rédaction (main @ e4c4c6b+)

- **JS-C154→C162 complétés** : `unhandledrejection`→console (validé
  terrain marinetraffic/deezer), URL.canParse/parse, navigator props,
  TextEncoderStream/DecoderStream, surface ShadowRoot/`instanceof` +
  WebSocket/BroadcastChannel/EventSource lisibles, **AES-GCM
  bout-en-bout** (jscore→net `ring::aead`, NIST vectors),
  dédoublonnage `unhandledrejection` (flood genius 512→1),
  **`Element.prototype.querySelector*`/`getElementsBy*`** — cause
  racine `call of Undefined` Bootstrap (OSM, ameli).
- **FIX-2 tardifs** : `min/max/clamp` dans `calc()` (notion grid
  `var(--inset)`), plancher min-content du shrink flex (todoist),
  iframe 300×150, tofu PUA `.notdef` muet, `fixed` hors clip
  overflow ancêtre, UA `input` border.
- **FIX-3 tardifs** : **CSP `host-source "*"`** — le vrai bug
  dropbox (`style-src https://*` refusait toute feuille) ;
  `rel="preload" as="style"` collecté ; warns fetch
  stylesheet/script/iframe → ring ; disjoncteur avant chaque éval
  (wedge doctolib réduit à 1×30 s) ; `JsError::Timeout` branché ;
  `import()` : warn `fetch échoué` (bras MapLoader = note FIX-1).
- **Non appliqué** : JS-C163 (deadline wall-clock éval +
  `EXPLORER_JS_EVAL_TIMEOUT_MS`) — spec complète §reprise 16 du
  rapport FIX-1, chantier suivant.

---

## G. Protocole de synchro (rapports ↔ ce fichier)

- **`SYNTHESE.md` = source de vérité des statuts** ; les rapports
  de vol sont des preuves **immuables** (instantané d'un binaire).
- **Rapport de vol** : chaque constat porte `crate` + l'ID §B s'il
  correspond (`→ §B-5`) ; un trou nouveau = nouvelle entrée §B au
  numéro suivant, pas de texte libre.
- **Commit FIX** : référence l'ID tâche
  (`fix(jscore): J6 — …`) → l'état §E-bis se met à jour au merge.
- **Clôture FIX** : l'agent met à jour la colonne État de **ses
  lignes** §E-bis + le statut §B — dans le même commit.
- **Statuts canoniques** : `OUVERT` → `FIX LIVRÉ` → `RÉSOLU`
  (fermé **uniquement par un re-vol** T1 sur binaire post-fix) ;
  `PARTIEL`, `EN DÉCROISSANCE`, `STRUCTUREL`, `INTERMITTENT` pour
  les cas intermédiaires.
- **Tag de binaire** : toute anomalie porte sa version
  (`[V1]`/`[V2]`/`[V3]`) — un constat pré-V3 n'est jamais présumé
  régression.
- **Cohérence** : §D = priorités (ordre), §E-bis = propriétaires
  (pistes NAV-JS / NAV / PIPE / Terrain), §B = catalogue — les
  IDs §B font le lien, pas de duplication.
