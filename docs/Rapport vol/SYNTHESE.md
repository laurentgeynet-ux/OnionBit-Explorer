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
| 4 | **Morts silencieuses résiduelles** : boot JS échoue sans aucune entrée console | `core`/`jscore` | pinterest (vol 22 post-fix, toujours muet), twitch, archive.org, radiogarden (33 scripts, 0 erreur), play2048, cartes.gouv (**sorti du silence `[V4-main]` vol 50** : `unhandledrejection: unsupported encoding ×4` tracé → §B-14), windy/flightradar (modules+importmap inertes) ; **RÉGRESSION `[V7-main]` vol 83 : data.gouv ex-« meilleur vol » → module Nuxt + importmap + ~34 preload : 0 fetch/0 éval/0 trace** — **bras frais `[V4-main]` vol 36** : defer Next.js non exécutés sans erreur (GFG), tuiles Leaflet jamais demandées (OSM — `call of Undefined` mort, étape suivante muette) | **EN NETTE DÉCROISSANCE** — `unhandledrejection` + warns fetch stylesheet/script/iframe + dyn import livrés ; **3 bras précisés `[V4-main]` vols 38/43** :
(a) **drains modules/defer inertes** — windy : `type="module"`+importmap 0 trace, heap 8,7 Mio (drain top-level absent ?) ; pinterest ~70 scripts 0 éval ; GFG defer Next ;
(b) **exceptions post-instanciation** — archive.org **tracé** (`unhandledrejection: append/get of Undefined` ×2) ;
(c) **exécution réussie sans effet — NOUVEAU, le plus opaque** — twitch : 44/44 évals `ok=true` (bundles 355 Ko), 0 rejet, React jamais monté → instrument T2 `evals_sans_mutation` proposé comme détecteur |
| 5 | **`import()` : dep absente du MapLoader** — « module non chargé : {url absolue} » | `explorer-js`/`core` | mastodon, chess, letterboxd, marmiton | **FIX LIVRÉ** — racine mastodon fermée : `max_modules` 64→256 + warn `tronqué` (`2b61012`) ; re-vol post-fix : graphe `application-*.js` = 121 sources, axios (n°65+) chargé et instancié, erreur disparue. Restes §B : refus fetch / spec non extrait d'import map (J3) ; chess/letterboxd/marmiton à re-voler |
| 6 | **`import()` sur `data:` URL refusé** (résolution/CSP) | `explorer-core` | reddit ×2 ; confirmé `[V4-main]` vol 38 (refus identique **désormais tracé WARN** — fin du silence sur ce bras) | **OUVERT** — WIP équipe (J3 chantier `data:` modules) |
| 7 | **`visibility:hidden` jamais levé** — page blanche totale malgré DOM/layout complets | `jscore`/glue | geeksforgeeks (vols 30+36 `[V4-main]` : frame blanche byte-identique, wrapper `hidden` intact ~25 s, **defer Next non exécutés sans erreur** → recoupe §B-4/§B-19) | **OUVERT — confirmé V4** |
| 8 | **Frame blanche au scroll** — layout complet à y=5520, frame blanche à scroll_y=1600 | `explorer-render` | glassdoor (vol 26) ; **variante partielle `[V7-main]` vol 82** : textes articles absents à scroll_y>0, repeints au retour (rtl/funradio) | **OUVERT** |

### 🟠 Majeur — sites dégradés, classes de bugs

| # | Anomalie | Crate | Preuve | Statut |
|---|---|---|---|---|
| 9 | **Parser résiduel** : `` `...` `` « outside a call or array literal » encore vu **post-fix** ; `Cannot call a class as a function` (tiktok — cascade honnête) ; `constructor is not a function: function(){[code]}` (gitlab rspack — diagnostic posé) ; `JSON.parse unexpected token` (gitlab GraphQL — troncature ?) | `explorer-jscore` | bbcgoodfood `_app` (vol 34 V3), ikea sentry | **OUVERT** (rest/spread en position non couverte par JS-C146) |
| 10 | **Scope global inter-scripts** : `var`/assignations top-level non propagées à `window` partagé | `explorer-jscore` | lemonde `lmd` ×10, ouest-france `firstparty.push` ×3 — **bras RÉSOLU `[V4-main]` vol 48** (23/23 évals ok, `window.firstparty` posé, console vide), OSM `OSM.i18n` | **VÉRIFIÉ SAIN** — sonde session `global_var_shared_between_script_els` : `var`/`window.=`/undeclared traversent les `<script>` bout à bout, script mort à mi-éval laisse ses liaisons hissées (`dead=7`, `later` bound) — les `lmd`/`didomi`/`OSM` absents = scripts définisseurs jamais exécutés (**cascade §B-4**, pas un bug de scope) |
| 11 | **`new URL(".", location)`** — base `Location` non stringifiée ; `Invalid scheme` sur chemins `./` non résolus | `explorer-jscore` | fastapi, svelte, lemonde, figaro ; cdiscount `unhandledrejection: Invalid scheme` ×5 `[V4-main]` vol 36 | **VÉRIFIÉ SAIN** — `url_ctor_with_location_base`/`new_url_against_location_resolves` : `new URL('.', location)` → base correcte, résolution `./`/`x` OK ; les `Invalid scheme` restants = autre forme d'entrée à nommer au prochain vol (attribution tracée) |
| 12 | **`heap memory limit exceeded` en rafale** | `explorer-jscore` (budget ou fuite) | marmiton ×3 au boot (vol 34 — **non reproduit `[V4-main]` vol 36**), bookshop ×14 beacon.min.js | **OUVERT** — capper à 1 occurrence + mesurer le seuil |
| 13 | **Hit-test `ignored` en série** — **confirmé sain `[V4-main]` vol 37** (figma clic→nav réelle) — liens/inputs réels non trouvés | `explorer-layout`/`core` | vols 26/29/30 (V2 integ) : 4/4 clics refusés | **PROBABLEMENT RÉSOLU** — cause identifiée : wrappers `visibility:hidden` en attente d'hydratation (GFG), pas un bug hit-test ; **clics = vraies navigations V3/V4** (vols 34/37/44/45) ; **formulaire bout-en-bout validé** (ddg lite : focus→saisie→Enter→SERP→article, vol 44) | **RÉSOLU confirmé `[V4-main]`** — attention variante : élément sous overlay reste `ignored` (§B-18 qwant, §B-17 ratp) |
| 14 | **APIs DOM restantes** : `document.write` ordonnancé post-commit (tumblr), `instanceof Element` trop strict (impots, ameli), `document` of Null (dw), `Illegal component name ""`/`Function.name` (dw), `debugger` token tue un module (w3schools), `get`/`bind` of Undefined (marinetraffic, tiktok — cascades à nommer), `GeolocationPosition is not defined` (marmiton `[V4-main]`), `document.contains` absent (ratp vol 45), `GeolocationPosition`/`navigator.geolocation` absent (marmiton vol 36 — « le navigateur n'a pas de GPS », feature-detect géoloc des sites), `Node.replaceChild: not a node` (dsfr-1.14 service-public vol 47), **ameli `call of Undefined` Bootstrap RÉSOLU confirmé `[V4-main]` vol 59** (console vide, icônes peintes vs carrés vides vol 29), `customElements.define` reçoit `undefined` (reddit vol 38), `navigator.userAgentData`/`getHighEntropyValues` absent (speedtest vol 78), `Response.json` absent (regex101 `'json' is not a function` `[V4-main]` vol 55) | `explorer-jscore` | vols 7/8/16/18/23/24/25/27-30/36 | **PARTIEL** — `call of Undefined` Bootstrap **RÉSOLU confirmé `[V4-main]`** (JS-C162 — console OSM vide, Leaflet instancié ; reste tuiles absentes → §B-4) ; `attachShadow` sorti du stub minimal ; reste la matrice ci-contre |
| 15 | **Shadow DOM / custom elements** : `attachShadow` stub, `faceplate-*`/`shreddit-*` 0×0, slotted content absent | `explorer-jscore` | reddit login (28 partials vides) ; **`[V4-main]` vol 38 : DÉGRADÉ EN MIEUX** — challenge JS franchi, vraie page r/rust peinte (13 shreddit-post avec contenu), upgrade custom elements tenté (`customElements.define` reçoit `undefined` ×3 → §B-14) | **STRUCTUREL, amorce livrée** — surface JS posée (attachShadow→fragment, getElementById, instanceof, proto chaîné WebSocket/BC/EventSource lisibles) ; slotted/custom-elements upgrade reste le chantier (issue #25) ; **+1 bras `[V4-main]` vol 57** : DSD `shadowrootmode=open` (pe-cookies francetravail) aplatie → `position:fixed` 1366×900 en backdrop plein écran |
| 16 | **WebGL/canvas lourd** | `jscore`+`render` | threejs, ciechanowski, pixi, radio.garden, flightradar | Hors scope court terme (ADR stubs) — stub Ctx2d/toDataURL livrés |
| 17 | **Menus/overlays résiduels** — **reproduit `[V4-main]` figma** (méga-menus peints ouverts landing+interne) : ceux fermés **par JS** restent ouverts (drawer `inert`+`aria-hidden` peint ouvert santemagazine `[V6-main]` vol 71) : ceux fermés **par JS** restent ouverts (drawer `inert`+`aria-hidden` peint ouvert santemagazine `[V6-main]` vol 71) (le CSS est corrigé : `hidden`, `[popover]`, `opacity`, `inset`, `details`) ; off-canvas GOG/IGN, backdrop francetravail, megamenus 750g/doctolib | `jscore` (JS mort) + résiduels css | nombreux vols | **EN DÉCROISSANCE** — reste dû aux boots morts, pas au pipeline ; **sous-cas confirmé `[V4-main]` vol 45** : nav off-canvas ratp fermée mais enfants déployés/peints/interceptant hit-test (clic burger `ignored`) |
| 18 | **Layout résiduel** : chevauchements de runs inline (excalidraw/gitlab/DW/deezer — non reproduit), `overflow-x` clip (guardian), marquee dupliqué (vercel), sticky repaint (accuweather), dark theme classe JS (vercel), double émission suspectée synchro DOM (franceinfo/gitlab header), bandeau noir 250px CNN, runs éphéméride wikipedia `[V4-main]` vol 56, **runs inline superposés confirmés `[V4-main]`** (lemonde « À Marseille… » + en-tête ouest-france vol 48), **hero img occluant l'input recherche** (qwant `[V4-main]` vol 44 : input layouté mais 884×772 img par-dessus → clic `hit=true` sur l'image, saisie impossible) | `explorer-layout`/`render` | vols 11/13/20/22/31/32/33/44 | **PARTIEL** — **hero colonne RÉSOLU** (calc `min/max/clamp` factor + plancher min-content du shrink flex, repro notion verbatim vert) ; restes à re-voler |
| 19 | **`document.write`/`async` ordonnancement** — scripts dyn drainés post-commit, pas en ordre de parse | `explorer-core` | tumblr main-*.js jamais exécuté | **OUVERT** — chantier profond documenté |
| 20 | **Images `data-src`** : repli livré (840a33c) mais marmiton encore gris | `explorer-core` | vols 34+36 `[V4-main]` : 215 `data-src`, `<img>` sans `src`, gris à +30 s et scroll | **TOUJOURS PRÉSENT `[V4-main]`** — le repli ne produit rien sur cette cible |
| 21 | **Console `×N` ne borne pas la source** : `r is not defined` ×512 (genius reactAds) = retry/intervalle non borné | `jscore` budget | vol 33 | dédup visuel OK ; boucle applicative reste |
| 22 | **Redux `dispatch` : action non-plain** — `Minified Redux error #7` (« Actions must be plain objects ») : une action non-plain traverse la chaîne middleware/thunk jusqu'au dispatch de base | `explorer-jscore` | mastodon `/about` (vol opérateur, binaire post-C165) — l'app exécute 121 modules jusqu'à `createStore` | **OUVERT** — suspect : composition `applyMiddleware`/`typeof` action thunk non interceptée ; immer résolu (J10) |

| 23 | **`eval deadline` trop agressif au boot** — 2ᵉ cas terrain : lemonde `[V4-main]` vol 48 (script 254 Ko tué à 30 s → disjoncteur, heap 0) — compile légitime tuée | `explorer-jscore`/`js` | cryptpad `[V4-main]` vol 37 : `RangeError: eval deadline exceeded` sur less.min.js → boot RequireJS mort (J1 livré mais seuil bas — à étalonner/exempter phase boot, lien J1/N3) | **OUVERT — terrain V4** |
| 24 | **Stretched-link incliquable** : `<a>` `position:absolute`+`inset:0` → boîte 235×0 px, hit-test `ignored` sur toutes les cards | `explorer-layout`/`css` | vinted `[V4-main]` vol 39 : rendu ~95 % fidèle mais entièrement incliquable ; `logs/vol39_vinted_layout.txt` 15 ancres ×0 hauteur — le fix `inset` ne couvre pas le stretch vertical (hauteur parent auto ?) | **OUVERT — terrain V4** |

| 25 | **Navigation `location` spontanée non sollicitée** — `location.href` posé sans input (détection env ?) | `explorer-jscore`/UA | qwant `[V4-main]` vol 44 : redirigé vers `/mobile-apps?utm_campaign=w2m` — nav unique (§B-2 tient) mais suspect `navigator.*`/UA feature-detect | **OUVERT — terrain V4** |

| 26 | **Wedge DOM détaché terminal post-apply** — `journal_fallbacks` non livré OU apply mort entre `layout` et `repaint` → `/dom` 404, `/frame` 204, **fuite mémoire sans borne** (caf 2,93 Go vol 47 ; **bfmtv 6,88 Go en 5 min vol 49 — escalade §B-26→§B-1, API entière morte** ; figaro variante sans UnmappedKey) | `explorer-render` (fix) | caf/figaro/bfmtv/radiofrance/meteofrance `[V4-main]` vols 47/49/54/59 (meteofrance : apply arrêté à `layout ms=253`, dent de scie) ; **musee-orsay vol 62** : apply stoppé post-`layout ms=968`, `pump.js_run_sync` ~8 ms/5 s persistant, fuite 976→2,35 Go **poursuivie post-DELETE** (recoupe §B-30) (radiofrance : apply en escalade 0,4→9,2→>49,5 s, +90 Mo/s, API vivante) — signature commune : apply journalisé jusqu'à `layout`, jamais `repaint`/`fin` | **CORRECTIF `[V5-main]`** : localisé dans `build_display_list` — une file de porteurs reportés se re-remplissait de la même cible (cycle de report, chemin `[0,0,0,0]` mesuré) → borne `MAX_EMIT_PER_PATH=4` + plafond 100 k, `dropped_emits`/`first_cycle_path` remontés. meteofrance re-vol : `dropped=1`, `apply: fin ms=2308`, process vivant. Variantes à re-valider : figaro (libérée au DELETE), musee-orsay (post-DELETE = §B-30), bfmtv (dent de scie) **+`[V6-main]` vol 67 : RÉSOLU ×3** — borne `emit_scoped_at` (`1e24942`) : meteofrance (`cycle cassé dropped=1 path=[0,0,0,0]` ×994, `apply: fin`, RAM stable 746 Mo), orsay (`[0,6,0]`, purge post-DELETE → §B-30 favorable), figaro (`[0,1]`, ~0,1 Mo/s vs 29-38 ; reste frame blanche JS → §B-4) ; bfmtv dent-de-scie non re-volée | **RÉSOLU `[V6-main]` vol 67 — 3 variantes (meteofrance/orsay/figaro) ; bfmtv à re-voler** |

| 27 | **Image en mosaïque géante plein viewport au scroll** | `explorer-render`/`layout` | ouest-france `[V4-main]` vol 48 scroll_y=1800 — tuile répétée couvrant tout l'écran (background-repeat ? border-image ?) | **OUVERT — terrain V4** |

| 28 | **`height:100%` plein-viewport effondrée à 0** — chaîne `html`/`body`/`#map` = `1366x0` → Leaflet lit `clientHeight≈0`, ne demande qu'1 tuile | `explorer-layout`/`css` | uMap éditeur `[V4-main]` vol 50 — tuiles OSM peintes mais page à hauteur 0 (paradoxe : tuiles visibles) | **OUVERT — terrain V4** |

| 29 | **Boucle iframes `sodar` non bornée** — AdSense crée des iframes télémétrie en continu : 419 dans le DOM (+~90/min), `js_heap` 53→160 Mo (~40 Mo/min), cycles apply ~1,3 s | `explorer-core`/budget | linuxmint `[V4-main]` vol 51 — ad-tech fonctionnel (slot peint) mais DOM+heap gonflent ; API responsive, RAM libérée au DELETE | **OUVERT — famille §B-1/§B-21, signature DOM+heap** |

| 30 | **Fuite mémoire post-DELETE sans borne** — croissance +~28 Mo/s après fermeture de tous les onglets (`{"tabs":[]}`), API responsive, 0 log | `explorer-core` | manomano `[V4-main]` vol 53 : 1,7→2,98→4,27 Go avec zéro onglet — suspect : tâche navigate/pompe orpheline — **preuve directe vol 59** : `pump.commit_lock us=89 918 900` (~90 s) pour tab=3 **12 min après DELETE**, RAM 5,10 Go avec `{"tabs":[]}` ; **signal contraire `[V6-main]` vol 67** : orsay DELETE → purge 1 015→780 Mo stable 65 s (variante in-flight purgée par le fix §B-26 ?) | **CRITIQUE — repro : navigate + DELETE pendant in-flight ; distinct de §B-26 — vol 67 : purge orsay saine, reste manomano/flixbus à re-voler** |

| 31 | **Clic `<button onclick>` → `ignored`** — **RÉSOLU `[V7-main]` vol 80** : clic → `edited`, éditeur Photopea monte entièrement — boîte bien layoutée (1311×59), clic centré, refusé ; bloque les apps à bouton primaire (éditeur photopea injoignable) — `<a target=_blank>` fonctionne | `explorer-core`/hit-test | photopea `[V4-main]` vol 55 — suspect : handler `onclick` attribut non branché côté hit-test | **OUVERT — terrain V4** |
| 32 | **Rasters pixelisés basse résolution** — icônes/logo/éditeur en blocs (downscale ou upscale brut ?) | `explorer-render` | regex101 `[V4-main]` vol 55 (certitude moyenne — verifier si srcset/lowres) | **OUVERT — terrain V4** |

| 33 | **Wedge `nav_paint` sans timeout** — `take_nav_paint → Busy → sleep(50ms)` en boucle à vie (`shared.rs:157-166`) : `POST navigate` jamais retourné (>25 min), DOM 404/frame 204 permanent, 0 WARN, `journal_fallbacks=0`, ~1,5 cœur saturé | `explorer-core` | francebleu→ici.fr + 20minutes `[V4-main]` vol 52 — wedge en amont (`insert_images`/`styles_payload`) ou `composite_frames` ; distinct de §B-26 (fallbacks=0, navigate en vol) | **CRITIQUE — racine localisée : ajouter timeout + abandon payload** |

| 34 | **`max-height` (%) ignorée sur élément remplacé** — `<img>` taille intrinsèque 703×994 dans `div.h-48` au lieu de ≤192 px, clip `overflow` | `explorer-css`/`layout` | cjoint `[V4-main]` vol 56 (`img [-159 -222 703x994]` dans div 366×192) — bras opposé de §B-28 | **OUVERT — terrain V4** |

| 35 | **Colonne `block` effondrée à largeur 0** — texte mot-par-ligne | `explorer-layout`/`css` | wttj `[V4-main]` vol 57 | **RÉSOLU `[V5-main]` vol 64** — h2 1261 px, p 252-1286 px (subgrid col-span-N) |
| 36 | **SVG peint/placé hors de sa boîte** — `block<svg>` positionné hors du bouton parent (x décalé) ou à échelle géante (wordmark 700 px dans boîte 300×81) | `explorer-layout`/`render` | wttj smear + indeed wordmark `[V4-main]` vol 57 — recoupe §B-32 pixelisé | **OUVERT — terrain V4** |

| 37 | **Panic `index out of bounds` `dom.rs:2691` en `event.dispatch`** — NodeId obsolète post re-parse ADR-0025 (24 rejets `Fingerprint` en rafale) → garde-fou a rattrapé (disjoncteur, onglet JS mort) — **violation ADR-0008**, 2ᵉ panic jscore, 1ᵉ rattrapé | `explorer-jscore` | caniuse `[V4-main]` vol 58 | **CRITIQUE — handle JS invalide après re-parse** |
| 38 | **Item flex layouté à 5 839 px** — nav expulsée hors viewport (x=5 879), logo en ellipse aplatie | `explorer-layout`/`css` | developer.chrome.com vols 58+80 (`[V7-main]` verbatim : `[24 6 5839×36]`, nav expulsée) — le fix §B-49 ne couvre pas ce motif | **OUVERT — confirmé V7** |
| 39 | **`DELETE /api/tabs` → deadlock** — mémoire libérée, CPU nul, port LISTENING, API muette >20 s | `explorer-core`/`api` | vol 58 tab 1 — distinct de §B-33 (pas de spin) et §B-30 (pas de fuite) | **CRITIQUE — nouvelle classe d'arrêt** |

| 40 | **Hit-test `ignored` sur ancre réelle à scroll_y>0** — `<a href>` layoutée, clic refusé en position scrollée alors que clic en haut navigue | `explorer-core`/hit-test | trainline vol 60 + **chrome.dev vol 80 `[V7-main]`** (ancre `ignored` à scroll_y=800) ; « Railcards » → `navigated` à 0 — décalage scroll suspect | **OUVERT — terrain V4** |
| 41 | **Wedge starvation phases `event.*` sans fin** — boucle `poll_net` 5-31 s / `mirror_sync` 10-13 s / `dispatch` ~5 s en continu, navigate jamais retourné, API affamée, RAM stable | `explorer-core` | flixbus `[V4-main]` vol 60 **2/2** + **credit-agricole vol 63** (`commit.drains` 60→185 s, `poll_net` 218 s, wedge scopé onglet, RAM libérée au DELETE — sans fuite), corrélé à `UnmappedKey` re-parse ; distinct de §B-26 (pas de dernier `apply: layout`) et §B-33 (journal non muet, fallback>0) | **CRITIQUE — déclencheur fiable ; prudence vol : flixbus wedge garanti** |

| 42 | **Hit-test traverse un overlay `position:fixed`** — clic sur bouton consentement résolu en coords document sur l'ancre pleine-largeur SOUS l'overlay → `navigated` vers la mauvaise cible | `explorer-core`/hit-test | culture.gouv `[V4-main]` vol 62 : clic « Tout accepter » (804,809) → nav `/thematiques/audiovisuel` (ancre 1366×48 à doc-y 2409) — preuve géométrique complète | **CRITIQUE — variante §B-13/§B-17 ; z-order hit-test vs fixed** |

| 43 | **Tiroir off-canvas invisible avale tous les clics** — `<mini-cart class="layer…">` fermé (non peint) mais boîte `[0 0 1366x900]` prioritaire au hit-test : 2/2 clics → `edited` (« livré au DOM — aucune action par défaut »), aucune ancre atteignable | `explorer-core`/hit-test | auchan `[V6-main]` vol 69 : clics « Promos » (295,40) + « Politique de confidentialité » (760,712) interceptés ; transform/état fermé ignoré par le hit-test — variante §B-42 croisée §B-17 | **NON REPRODUIT — diagnostic vol 69 infirmé** : re-vol instrumenté (`hit_path` au debug) même layout (`doc_h` 2261) — le sous-arbre `mini-cart` est correctement exclu (`visibility:hidden` hérité → `unhittable`) : clic (760,712) **traverse** le tiroir → `navigated` sur le `<a>` dessous ; clic (295,40) → `edited` sur la vraie barre haute `html>body>div[2]>header>div` (12 px au-dessus du lien « Promos » posé à y=52). Le vol 69 a inféré l'interception du dump layout seul ; à ré-ouvrir si un porteur frais reproduit avec `hit_path` dans le sous-arbre du layer |

| 44 | **Escalade quadratique du coût par apply** — `repaint ms=363→9 558→199 721`, cascade 46 s/boxtree 55 s/layout 72 s sur la même page ; borne §B-26 non déclenchée, mécanisme distinct (invalidation qui grandit) | `explorer-core`/`layout` | radiofrance `[V6-main]` vol 68 + **`[V7-main]` vol 81 verbatim** (franceculture→radiofrance : cascade 35 s/boxtree 39 s/layout 47 s/repaint 63 s, RAM 4,79 Go +~90 Mo/s, purge complète post-DELETE) ; stabilité OK, perf seule | **OUVERT — perf, à profiler** ; +variante vol 83 : mutations DOM continues SANS `UnmappedKey` (conseil-etat, cascade→14,6 s, §B-41 non-terminal : phases aboutissent, scroll honoré en 57 s) |

| 45 | **`doc_h` verrouillé au viewport** — `html,body{height:100%}` + `overflow-x:hidden` → `doc_h`=900, scroll bloqué à 0 alors que le contenu déborde (~13 450 px) | `explorer-layout`/`core` | clubic `[V6-main]` vol 70 — main 11 750 px layouté, frame identique après scroll | **CORRECTIF `34a019b` à re-voler** — `scrollable_overflow_bottom` clippait sous `body{overflow-x:hidden}` (y→auto, R-120) ; l'overflow `html`/`body` se propage désormais au viewport (css-overflow-3 §3.3) : `auto`/`scroll` ne bornent plus l'étendue, `hidden` clippe ; scroll conteneurs internes inchangés — tests repro verts |
| 46 | **Wedge post-apply silencieux CPU 200%** — apply `fin` journalisé (borne §B-26 tirée) puis silence total, navigate jamais retourné, API morte, RAM 2,7 Go | `explorer-core` | frandroid `[V6-main]` vol 70 — distinct §B-33 (fin journalisée) et §B-41 (pas de phases) | **CRITIQUE — repro 1/1 ; prudence vol : frandroid, conseil-etat** |

| 47 | **Mojibake UTF-8→Latin-1 des champs JSON API** — `title` de `/api/tabs` garbled (`idÃ©es`, `actualitÃ©`), suffixe dédup `/console` `Ã—3` — DOM/rendu corrects, titre clapote correct↔mojibake selon cycle re-parse | `explorer-core`/`api` | santemagazine/doctissimo/femmeactuelle `[V6-main]` vol 71 | **CORRECTIF ×2 à re-voler** — (a) `e268ec7` : `/api/tabs` relit `extract_title(dom)` vivant — le `t.title` stocké peut dévier du `<title>` commis au cycle re-parse/miroir ; (b) `8ddd26d` : `charset=utf-8` annoncé sur les réponses JSON (`application/json` nu laissait un client replier Latin-1 — `×`→`Ã—` ne pouvait être corrompu côté serveur) |

| 48 | **Micro-script en boucle ≥60 s** — script 285 o bouclant jusqu'au kill disjoncteur (`eval ms=60002 ok=false`), scripts suivants sautés ; kill absent de `/console` (« pas de journal » mensonger) | `explorer-jscore` | kayak `[V6-main]` vol 72 — porteur historique wedge vol 24 : nav retourné 65 s, modale erreur propre | **OUVERT — contenu par disjoncteur ; J1 validé terrain** |

| 49 | **Wrapper racine expulsé hors viewport** — `.paris-main-wrapper` layouté `[1366 0 0×9927]` : largeur 0 au bord droit, tout descendant x≥1366 → frame blanche malgré DOM/layout/images complets (doc_h=9 927) | `explorer-layout`/`css` | paris.fr `[V6-main]` vol 73 — motif suspect : item flex row compressé à 0 ; famille §B-35, symptôme §B-7 | **CORRECTIF `741f810` à re-voler** — cause nommée : `.skip-links{position:absolute;width:100%}` était compté comme item flex (Flexbox §4.1 : hors flux ≠ item) → base 100% avalait tout l'espace libre → `flex:1` du wrapper retombait à 0 → contenu à x=1366. Fix : enfants `absolute`/`fixed` exclus des items flex ET grid + contribution intrinsèque filtrée ; placeholder statique pour `resolve_absolutes`. Repro `flex_enfant_absolu_ne_consomme_pas_lespace` verrouillée |
| 50 | **Re-parse post-apply : DOM jamais réattaché** — apply complet (`repaint` journalisé, borne §B-26 tirée) puis rejet `UnmappedKey` → re-parse → `dom` 404 / `frame` 204 / `dom_nodes=0` >7 min, flag « paint en vol » bloqué | `explorer-css` (parser) | bordeaux.fr `[V6-main]` vol 73 ; lumiplan vol 76 (oscillant) | **FIX LIVRÉ** — cause racine nommée : un `style="transform: translateX(0px;…"` muté par JS laissait une `(` jamais refermée ; la boucle de valeur de `declaration()` attendait la `)` à EOF sans consommer → boucle infinie 1 cœur dans la cascade du re-parse (bissection jalons `apply:` : `imgs` passé, `cascade` jamais). Fix : EOF clos la déclaration même parens ouvertes (spec) + test `parenthese_non_fermee_eof_termine`. **Re-vol bordeaux validé** : rejet `UnmappedKey` → re-parse → `cascade ms=2324` → `layout` → commit, `/dom` 200 `/frame` 200 |

| 51 | **Évals `ok=false ms=0` silencieuses** — même script ré-évalué après chaque re-parse `UnmappedKey`, zéro trace `/console` | `explorer-jscore`/glue | messenger vol 78, **airfrance vol 87 (4 bundles 467-614 Ko silencieux/nav → coquille vide, meilleur vol SPA Angular sinon ~80 %)**, sosh (863 Ko→coquille vide) + edf (605 Ko) vol 85 `[V7]`** + **élargi aux évals initiales `ms>0`** (challenge Incapsula micromania : 103 Ko `ok=false ms=703` sans trace, vol 81) — trou d'observabilité §B-4-adjacent | **OUVERT — plateforme RTL (rtl+funradio vol 82 : bundle 4,1 Mo `ok=false` silencieux ×2, js_heap=0) ; mécanisme nommé vol 79 : erreurs éval de boot émises *avant* `Navigated` → vidées au commit (api/lib.rs) — purge uniquement sur Navigated cross-doc suggérée** |

| 52 | **Rafale `UnmappedKey` continue** — mutations DOM périodiques → re-parse ~1×/5 s (56×/2,5 min) sans wedge | `explorer-core` | bouygues-immobilier `[V7-main]` vol 84 : contenu, API vivante — famille §B-41 atténuée | **OUVERT — V7** |

| 53 | **`translateX(-50%)` popin déplacée +320 px** — `#popin_tc_privacy` edf `left:683px` layoutée x=1046 (≈50%+viewport) → boutons consentement hors écran, page verrouillée (`body{position:fixed}`) | `explorer-layout`/`css` | edf `[V7-main]` vol 85 — suspect translate appliqué en sens inverse | **OUVERT — V7** |

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

Anti-bot : 403 sec (ebay, etsy, stackoverflow, tripadvisor, sncf-connect, affinity.serif, hardware.fr, liberation.fr, logic-immo, lacentrale, intersport, cultura, nexity, · vhost TLS mort www.figaro.fr (UnrecognisedName, curl idem — externe) ·
seloger, medium, khanacademy, apec, nytimes) · Datadome (leboncoin) ·
Akamai Bot Manager (cdiscount, zalando→`supported-browsers`) ·
x5sec Alibaba (aliexpress→punish) · **AWS WAF insolvable** (imdb,
booking — challenge obfusqué) · challenge JS reddit **résolu côté
serveur** · login-walls (old.reddit, quora partiel) · Cloudflare
urssaf.fr RST 10054 reproduit curl schannel `[V4-main]` vol 45 · Incapsula/Imperva challenge (micromania, coquille noindex, `[V7-main]` vol 81) · calameo (connexion fermée avant fin d'en-têtes, curl H1.1 OK — fingerprint TLS/H2, `[V6-main]` vol 75) · Turnstile (quora). Signature cumulée : `UA: Explorer/0.1`, pas de
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
