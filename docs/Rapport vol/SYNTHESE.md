<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Synthèse consolidée des vols — 2026-10-26/27

Synthèse de **52 documents** : 12 rapports de mission équipes
(vague 1, ~40 sites), 37 rapports de vol agents (vague 2, ~159 sites),
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
- **Agent 93 (`[V9-main]`, post-É148)** : §B-3 RÉSOLU (alpinejs
  entièrement fonctionnel, 0 overflow), §B-4(d) RÉSOLU
  (data.gouv montée, modulepreload fetché+évalué), §B-26
  RÉSOLU toutes variantes (bfmtv RAM plate, disjoncteur §B-41
  tiré terrain) ; **wedge migré → §B-54** (flixbus : phases
  `event.*` non bornées → API morte) ; §B-33/§B-37 non
  re-testés (daemon livelocké).
- **Agent 94 (`[V9-main]`)** : §B-33 RÉSOLU (francebleu→ici
  navigate ~150 s vs wedge), §B-40 RÉSOLU (trainline clic
  scrollé → nav), §B-34 RÉSOLU (cjoint img bornée), §B-23
  RÉSOLU (lemonde éval 58 s non tuée), §B-47 RÉSOLU
  (santemag titre propre), §B-29 RÉSOLU (linuxmint 0 sodar) ;
  §B-37 toujours présent intermittent (panic `dom.rs:2752`),
  §B-21 en décroissance (×255 nommés) ; nouveau §B-55 (panic
  dep `resvg`).
- **Agent 95 (`[V9-main]`, sans sous-agent)** : **§B-54
  reproduit sur un 2ᵉ site** — credit-agricole :
  `event.poll_net` 110,8 s, navigate jamais retourné, API
  morte, arrêt forcé PID (Étape 151 à re-voler vol 96) ;
  **§B-23 reproduit cryptpad** (site d'origine, deadline au
  boot, disjoncteur §B-41 armé ×1) ; **§B-12 persistant** —
  bookshop ×9 heap identiques (dedup `83916a6` inopérante
  terrain) ; §B-51 non reproduit ×2 (échecs nommés) ; §B-20
  favorable (marmiton peint au scroll, É149 hors binaire) ;
  §B-6 non reproduit (challenge reddit passé) ; §B-30 :
  7 DELETE propres, RAM 483 Mo à 0 onglet ; anomalie rendu
  letterboxd (texte peint miroir) ; manomano/frandroid non
  volés (daemon wedged) → vol 96.
- **Agent 96 (`[V10-main]`, campagne skill `/vol`)** : **§B-54
  RÉSOLU credit-agricole** (navigate 67,9 s vs jamais retourné,
  `poll_net` max 3,49 s vs 110,8 s, `deferred=59` ×13, DELETE
  0,49 s) ; **§B-54 PARTIEL flixbus** (nav retourné 95 s mais
  wedge post-DELETE, pompe orpheline → §B-30) ; **§B-12 étendu
  3ᵉ site** (frandroid ×6+, dedup toujours inopérante) ; §B-46
  non reproduit ; §B-20 favorable confirmé ; **§B-56/§B-57
  nouveaux** (regex valides rejetées ; `deeply nested` sur
  polyfill cdnjs réel → mur incompatible manomano).
- **Agent 97 (`[V10-main]`)** : **4 RÉSOLU** — §B-27
  (ouest-france : mosaïque disparue), §B-55 (ici.fr : panic
  resvg confiné, pompe reprend), §B-37 (caniuse : 0 panic
  dom), §B-46 confirmé ×2 ; **§B-12 EN DÉCROISSANCE**
  (frandroid ×0 — É152 efficace ; bookshop ×3, cap non
  atteint) ; **§B-23 PERSISTANT** (cryptpad less.min.js
  malgré JS-C176) ; §B-32 PARTIEL (regex101 logo flou) ;
  §B-57 confirmé 2ᵉ site (caniuse bundle).
- **Agent 98 (`[V10-main]`)** : **§B-56 PARTIEL** — bras
  `{n,m}` RÉSOLU (frandroid ×0) ; bras `\p{}` nié + octal
  PERSISTANT (airfrance) ; §B-12 cap confirmé ×2 vols ;
  **§B-58 nouveau** (letterboxd texte miroir ×2 vols) ;
  js_heap marmiton revenu à 49 Mo (hausse vol 96
  transitoire) ; leboncoin mur Datadome 403 ; twitch
  coquille blanche à console vide (variante §B-51).
- **Agent 100 (`[V10-main]` post-É154, campagne `/vol`)** :
  **§B-45 RÉSOLU** (clubic : doc_h 11 940, scroll effectif) ;
  **§B-12 PERSISTANT** (bookshop ×5 occurrences heap
  identiques — cap par-moteur insuffisant → **CORRECTIF
  Étape 156** : flag partagé vague détachée + livraisons
  routées) ; **§B-23
  PERSISTANT** (cryptpad : variante sans deadline — chaîne
  dyn requirejs/less jamais évaluée, disjoncteur ×6) ;
  **§B-57 PERSISTANT ×2** (manomano `ToObject`, caniuse —
  trampoline JS-C178 hors chemin) ; **§B-54 REPRODUIT**
  (flixbus : wedge pendant vie onglet, API morte — É154 non
  mesurable → **CORRECTIF Étape 157** : `drain_anim_births` borné au budget `event.*`) ; **§B-7 réattribué** (GFG : hydrate
  `getInitialProps` mort → §B-4, pas cssText) ; §B-46 non
  reproduit ×4 (frandroid) ; 6 DELETE propres.
- **Agent 101 (`[V10-main]` post-JS-C180/É155)** : **2
  RÉSOLU** — §B-38 (chrome.dev : 0 boîte expulsée) et §B-49
  (paris.fr peinte) ; **§B-51 bras silence RÉSOLU ×3**
  (sosh/edf nommés, twitch détecté `evals_sans_mutation` —
  WARN non routée au ring, trou résiduel) ; **§B-56 PARTIEL**
  (bras `\p` nié fermé ; octal → **CORRECTIF JS-C182** : octale Annex B en classe, nombre entier hors classe) ;
  §B-32 persistant (regex101 + `Response.json` → §B-14) ;
  §B-23 stable ×2 ; 7 DELETE propres.
- **Agent 102 (`[V10-main]` post-R-131)** : **2 RÉSOLU** —
  §B-58 (letterboxd miroir disparu) et §B-8 (glassdoor
  scroll peint) ; **§B-54 REPRODUIT ×2** (flixbus wedge
  vie-onglet, É154 non mesurable) ; **§B-28/§B-19/§B-36
  PERSISTANTS** ; §B-22 non atteint (mastodon : fetch ×127
  + `IDBRequest.addEventListener` → §B-14) ; 2 daemons
  (1 wedged kill PID), 5 DELETE propres.
- **Agent 104 (`[V10-main]` post-É157)** : **§B-1 RÉSOLU**
  (allrecipes API vivante, RAM plateau) ; **§B-54 bras
  post-DELETE RÉSOLU** (DELETE 5,5 s + purge — É154+157) ;
  vie-onglet en décroissance ; §B-14 +3 ; fnac/darty 403
  externe ; **vimeo : §B-1-variante** (évals 5,5 s rafale,
  kill PID) ; §B-12 cap favorable ×1.
- **Agent 105 (`[V10-main]` post-JS-C182)** : **§B-56
  RÉSOLU 3/3 bras** (airfrance 0 rejet regex) ;
  §B-1-variante vimeo confirmée ×2 (rafale évals longues,
  kill PID) ; 20minutes ~80 %, leparisien ~50 % ; §B-14 +2 ;
  2 daemons (1 wedged).
- **Agent 106 (`[V10-main]`, MEMWATCH)** : **harnais
  `vol_memwatch.ps1` validé terrain** — github : explosion
  430→8 079 Mo en ~30 s à la nav → kill auto au seuil 5 Go,
  station sauvée ; **§B-59 nouveau** (fuite mémoire nav) ;
  pinterest §B-4(a) reproduit.
- **Agent 108 (`[V10-main]` post-JS-C183)** : **§B-57
  RÉSOLU 3/3 bras** (caniuse 0 rejet) ; **§B-59 ×2**
  (github 9,6 Go/15 s, watchdog killé) ; lemonde sain.
- **Agent 109 (`[V10-main]`, FRMASSA)** : orange coquille
  (`HTMLDocument` §B-14), carrefour ~50 %, doctolib boot
  figé (wedge atténué — API vivante) ; 0 wedge/fuite,
  watchdog nominal.
- **Agent 110 (`[V10-main]`, EXPLOR6)** : OSM tuiles
  absentes (`St.tooltip` §B-14), lachainemeteo blanche
  (appconsent §B-14) ; 0 wedge, watchdog nominal.
- **Agent 111 (`[V10-main]`, REGIO)** : ladepeche ~85 %,
  sudouest ~70 % (heap cap ×1), 01net figé, MDN rendu ;
  0 wedge, watchdog nominal.
- **Agent 112 (`[V10-main]` post-JS-C184)** : **§B-14
  partiel RÉSOLU** — FontFaceSet.add/getAttributeNS
  disparus sur boursorama ; erreurs restantes nommées.
- **Agent 113 (`[V10-main]` post-JS-C185/É156)** : **3/3
  fixes validés** — §B-12 cap tenu (1 occurrence),
  `HTMLDocument` + `insertBefore` disparus ; §B-14
  cascades suivantes nommées.
- **Agent 114 (`[V10-main]` post-JS-C186)** : jvc
  `hasOwnProperty` disparu ; suivant `insertAdjacentHTML`.
- **Agent 115 (`[V10-main]`, EXPLOR7)** : impots ~80 %
  (DSFR), steam dégradé ; §B-14 +2 ; watchdog nominal.
- **Agent 116 (`[V10-main]` post-JS-C187)** : mastodon
  `IDBRequest.addEventListener` résolu → `then` thenable
  requis ; 125× fetch storm (§B-22-adjacent).
- **Agent 117 (`[V10-main]` post-JS-C188)** : jvc
  `insertAdjacentHTML` résolu → `category` getjan.
- **Agent 118 (`[V10-main]`, EXPLOR8)** : allociné/doctissimo/pap peints (~70-80 %) ; §B-14 +3 ; cap heap tenu.
- **Agent 119 (`[V10-main]` post-JS-C189)** : `postMessage`
  + `FontFaceSet.aEL` résolus ×2 (steam/impots) ; impots
  console vide.
- **Agent 120 (`[V10-main]` post-JS-C190)** : mastodon
  `IDBRequest` entièrement résolu ; reste fetch storm
  ×126 (proposition §B-60).
- **Agent 121 (`[V10-main]`, EXPLOR9)** : 4/4 peints
  (midilibre ~90 %, sephora ~80 %) ; §B-14 +3 ; cap tenu.
- **Agent 122 (`[V10-main]`, TVFR)** : france.tv/arte/maif/
  insee peints ~75-80 % ; `currentScript` invariant ×3
  sites (bras §B-14 prioritaire) ; XPath + signatures.
- **Agent 123 (`[V10-main]` @f20477ba — JS-C191 build
  cassé : mse.rs non commité)** : asos ~80 %, x/cdiscount
  morts ; §B-14 +4.
- **Agent 124 (`[V10-main]`, SPORTSRADIO)** : lequipe/rtl/
  europe1/nicematin peints ~70-80 % ; §B-14 +7 dont
  signatures nouvelles (postMessage clone, Body consumed).
- **Agent 125 (`[V10-main]`, MASSA2)** : leboncoin 403,
  caf ~50 %, franceculture ~70 % ; **§B-59 2ᵉ site**
  (lindependant 6 Go, fuite généralisée) ; §B-14 +5.
- **Agent 126 (`[V10-main]`, EXPLOR11)** : ouest-france ~80 %,
  action quasi parfait (console vide), dna ~65 % (`postTask`
  Scheduler API), leroymerlin 403 ; §B-14 +2.
- **Agent 127 (`[V10-main]`, EXPLOR12)** : 2×403, ryanair
  ~50 % (`MutationObserver: callback requis`), airbnb
  coquille ; `postTask` 2ᵉ site ; §B-14 +4.
- **Agent 128 (`[V10-main]`, BANQUE)** : SG ~65 %
  (`deeply nested` résiduel §B-57), meteofrance ~70 %,
  sarenza ~55 %, creditmutuel coquille ; §B-14 +4.
- **Agent 129 (`[V10-main]`, EXPLOR13)** : epic ~85 %
  (**invalid regex lookbehind** §B-56 résiduel), vdn ~75 %,
  lesechos mort, liberation 403 ; §B-14 +4.
- **Agent 130 (`[V10-main]`, EXPLOR14)** : argus ~80 %
  (console vide), sante ~75 %, DLP ~65 % (pompes chargées),
  MDM 403 ; §B-14 +7.
- **Agent 131 (`[V10-main]`, POSTE)** : lyon ~85 % (console
  vide), axa ~80 %, laposte ~70 % (`fetch: url required`),
  sfr coquille (adoption cross-doc) ; §B-14 +4.
- **Agent 132 (`[V10-main]`, EXPLOR15)** : estrepublicain/
  capital/ouigo/pappers peints ; `postTask` 3ᵉ site ;
  `Invalid scheme` §B-11 résiduel ; §B-14 +4.
- **Agent 133 (`[V10-main]`, INTL)** : guardian ~80 %
  (fetch storm §B-60) ; reuters 401, elpais/lemoniteur 403.
- **Agent 134 (`[V10-main]`, LIGHT)** : HN/dev.to/larousse
  consoles **vides** (~85-95 %) ; corriere doc_h 48k +
  `allocation exceeds` ×N.
- **Agent 135 (`[V10-main]`, AIR)** : transavia ~85 %
  (`[object Object]` sérialisation trou) ; eurostar
  challenge ; easyjet/kingjouet 403.
- **Agent 136 (`[V10-main]`, EXPLOR16)** : cuisineaz ~85 %
  (doc_h 60k record), orpi ~75 %, yves-rocher ~60 %
  (`deeply nested` résiduel) ; abritel 429 ; §B-14 +4.
- **Agent 107 (`[V10-main]`, PRESSE2)** : franceinfo ~70 %
  (panneau compte → §B-17), deezer ~60 % ; §B-14 +3 ;
  watchdog nominal (pic 1,98 Go, seuil non approché).
- **Agent 103 (`[V10-main]` post-JS-C181)** : **§B-57 bras 2
  PARTIEL** — manomano `deeply nested` disparu (mur migré
  didomi §B-14), caniuse restant ; **§B-5 favorable**
  (marmiton) ; **§B-14 +5** (Promise.finally,
  hasOwnProperty/CSSStyleDeclaration, osano, didomi) ;
  jvc mégamenu → §B-17 ; 0 wedge, 6 DELETE propres.

---

## B. Anomalies restantes — par gravité

### 🔴 Critique — processus/daemon, pages jamais servies

| # | Anomalie | Crate | Preuve | Statut |
|---|---|---|---|---|
| 1 | **Starvation de l'exécuteur post-apply** : la page rend, puis une rafale JS (timers/microtasks, hors éval) sature un cœur → API muette >3-6 min, RAM 2,9 Go, zéro log après `apply: layout` | `explorer-core`/daemon | **allrecipes.com ×2** (vol 34, binaire V3 — survit aux fixes), jscore 32 k lignes | **OUVERT** — le disjoncteur couvre les évals, pas les tâches rafale ; watchdog/budget global absent ; **RÉSOLU `[V10-main]` vol 104** — allrecipes : API responsive ~90 s (6/6 probes 200), RAM plateau 1,27 Go vs muette 3-6 min/2,9 Go ; rafale timers désamorcée par le watchdog éval ; reste variante vimeo : évals ~5,5 s en rafale continue (navigate >240 s, API morte) — budget global toujours absent ; **variante vimeo ×2 `[V10-main]` vols 104/105** — évals ~5,5 s `ok=true` en rafale continue, navigate >110-240 s, API morte — la série d'évals longues échappe à la deadline par-script : budget global N3 reste le chantier |
| 2 | **Boucle infinie de navigations auto** (meta refresh / `location.*` cumulés) | `explorer-core` | cdiscount : 3 812 navs, 20 001 streams, 16 min, kill manuel (éq. 6) | **RÉSOLU `[V4-main]`** — vol 36 : 0 nav auto en 5 min, URL stable ; disjoncteur validé en conditions réelles (script 507 Ko tué à 10 s, API responsive) |
| 3 | **Stack overflow** `explorer-jscore` — récursion parser/AST non bornée | `explorer-jscore` | alpinejs.dev : thread mort (éq. 2) | **CORRECTIF `adf9273` (JS-C167) à re-voler** — tueur réel nommé : `loose_eq` (Str,Obj) × `Symbol` → `to_primitive` rend le symbole → reboucle infini en natif pur, hors toute borne (`with(mergeProxies)` + trap `has`/`Symbol.unscopables`, délégation `[[Call]]` unifiée, `super()` natif). Historique : C134 (drops AST + walk JSON), JS-C166 `deleg_hop` (chaînes proxy ×20k → `RangeError`) + `stack_guard` pile native livrés mais **hors chemin** — re-vol vol 46 `[V4-main]` : `overflowed its stack` ~110 ms dans l'éval d'un script 55 935 o (14 évals ok avant), `STATUS_STACK_OVERFLOW` non rattrapable — l'overflow est à l'**exécution** (interprète récursif), pas au parse ; à re-voler alpinejs — **RÉSOLU `[V9-main]` vol 93** : process survit à alpinejs **entièrement fonctionnel** (36/36 évals `ok=true`, hydratation complète, clic → nav réelle vers la doc, 0 `overflow`/`panic`/`STATUS_` au log) ; seules `ReferenceError` nommées à `/console` |
| 4 | **Morts silencieuses résiduelles** : boot JS échoue sans aucune entrée console | `core`/`jscore` | pinterest (vol 22 post-fix, toujours muet), twitch, archive.org, radiogarden (33 scripts, 0 erreur), play2048, cartes.gouv (**sorti du silence `[V4-main]` vol 50** : `unhandledrejection: unsupported encoding ×4` tracé → §B-14), windy/flightradar (modules+importmap inertes) ; **RÉGRESSION `[V7-main]` vol 83 : data.gouv ex-« meilleur vol » → module Nuxt + importmap + ~34 preload : 0 fetch/0 éval/0 trace** — **bras frais `[V4-main]` vol 36** : defer Next.js non exécutés sans erreur (GFG), tuiles Leaflet jamais demandées (OSM — `call of Undefined` mort, étape suivante muette) | **EN NETTE DÉCROISSANCE** — `unhandledrejection` + warns fetch stylesheet/script/iframe + dyn import livrés ; **3 bras précisés `[V4-main]` vols 38/43** ; **vol 106 : bras (a) reproduit** — pinterest : 10 évals, `pas de journal`, coquille doc_h=900 ; **vol 110 : tuiles OSM toujours absentes** — bras = `St.tooltip` jscore (§B-14) |
(a) **drains modules/defer inertes** — windy : `type="module"`+importmap 0 trace, heap 8,7 Mio (drain top-level absent ?) ; pinterest ~70 scripts 0 éval ; GFG defer Next ;
(b) **exceptions post-instanciation** — archive.org **tracé** (`unhandledrejection: append/get of Undefined` ×2) ;
(c) **exécution réussie sans effet — NOUVEAU, le plus opaque** — twitch : 44/44 évals `ok=true` (bundles 355 Ko), 0 rejet, React jamais monté → instrument T2 `evals_sans_mutation` proposé comme détecteur |
(d) **data.gouv `[V7]` vol 83** — bras modulepreload livré (`étape 148`) : `rel=modulepreload`/`preload as=script|module` collectés en plan (CSP `script-src`/mixed-content filtrés, bornés `max_modules`) et semés dans la frontier initiale de `fetch_module_sources` — les chunks importmap Nuxt hors `import` statique sont désormais préfetchés ; **RÉSOLU `[V9-main]` vol 93** — homepage Nuxt entièrement montée+stylée (vs 0 fetch/0 éval du vol 83), chunk `DL4muOy-.js` fetché+évalué, `TypeError` nommée à `/console` ; reste : page interne `/datasets` naviguée mais non stylée (CSS route interne absente — anomalie rapportée) |
| 5 | **`import()` : dep absente du MapLoader** — « module non chargé : {url absolue} » | `explorer-js`/`core` | mastodon, chess, letterboxd, marmiton | **FIX LIVRÉ** — racine mastodon fermée : `max_modules` 64→256 + warn `tronqué` (`2b61012`) ; re-vol post-fix : graphe `application-*.js` = 121 sources, axios (n°65+) chargé et instancié, erreur disparue. Restes §B : refus fetch / spec non extrait d'import map (J3) ; chess/letterboxd/marmiton à re-voler ; **vol 95 `[V9-main]`** : letterboxd favorable (modules `homepage-*.js` évalués, erreurs nommées), marmiton `module non chargé` ×2, **bookshop `import() → …/null` (404)** = spécificateur résolu « null » — recoupe le bras « spec non extrait » J3 ; **vol 103 : favorable** — marmiton `module non chargé` non reproduit (heap 54 Mo sain, page ~90 %) |
| 6 | **`import()` sur `data:` URL refusé** (résolution/CSP) | `explorer-core` | reddit ×2 ; confirmé `[V4-main]` vol 38 (refus identique **désormais tracé WARN** — fin du silence sur ce bras) | **LIVRÉ — à re-voler** : `import()` dynamique décode `data:` inline (`data_url_bytes` → `deliver_dyn_import_req`, test `import_data_url_livre` JS-C131) **et** `data:` dans le graphe statique via `queue_module_dep` (test J3 `data:` en dep de module) — CSP `script-src`/`data:` scheme-source honorée ; **vol 95 `[V9-main]` : non reproduit** — reddit challenge `js_challenge=1` exécuté, homepage peinte, aucune trace `import()` sur `data:` (2 rejets `SML` nommés) |
| 7 | **`visibility:hidden` jamais levé** — page blanche totale malgré DOM/layout complets | `jscore`/glue | geeksforgeeks (vols 30+36 `[V4-main]` : frame blanche byte-identique, wrapper `hidden` intact ~25 s, **defer Next non exécutés sans erreur** → recoupe §B-4/§B-19) | **CORRECTIF JS-C174 à re-voler** — `style.cssText` implémenté (getter sérialise, setter re-parse `parse_decls`, vide → attr retiré) : l'anti-FOUC réel `visibility:hidden` se lève par `cssText`, pas par `style.visibility` — la mutation `camel_to_kebab` produisait une pseudo-propriété `css-text` ignorée ; **vol 100 `[V10-main]` : PERSISTANT, réattribution** — GFG frame blanche, `visibility:hidden` ×1 survivant, mais hydrate Next mort en amont (`unhandledrejection: getInitialProps of Undefined` tracé) → bras §B-4, cssText non testable sans boot vivant |
| 8 | **Frame blanche au scroll** — layout complet à y=5520, frame blanche à scroll_y=1600 | `explorer-render` | glassdoor (vol 26) ; **variante partielle `[V7-main]` vol 82** : textes articles absents à scroll_y>0, repeints au retour (rtl/funradio) | **RÉSOLU `[V10-main]` vol 102** — glassdoor : scroll_y=1600 contenu texte peint (vs frame blanche) |

### 🟠 Majeur — sites dégradés, classes de bugs

| # | Anomalie | Crate | Preuve | Statut |
|---|---|---|---|---|
| 9 | **Parser résiduel** : `` `...` `` « outside a call or array literal » encore vu **post-fix** ; `Cannot call a class as a function` (tiktok — cascade honnête) ; `constructor is not a function: function(){[code]}` (gitlab rspack — diagnostic posé) ; `JSON.parse unexpected token` (gitlab GraphQL — troncature ?) | `explorer-jscore` | bbcgoodfood `_app` (vol 34 V3), ikea sentry | **PARTIEL** — bras spread BBC non reproduit : JS-C173 sonde le `_app` verbatim vol 34 (eval+pompes+rejets propres) → le reste vue terrain était un rejet de promesse tardif ; restes ouverts : tiktok `call a class`, gitlab `constructor`/`JSON.parse` |
| 10 | **Scope global inter-scripts** : `var`/assignations top-level non propagées à `window` partagé | `explorer-jscore` | lemonde `lmd` ×10, ouest-france `firstparty.push` ×3 — **bras RÉSOLU `[V4-main]` vol 48** (23/23 évals ok, `window.firstparty` posé, console vide), OSM `OSM.i18n` | **VÉRIFIÉ SAIN** — sonde session `global_var_shared_between_script_els` : `var`/`window.=`/undeclared traversent les `<script>` bout à bout, script mort à mi-éval laisse ses liaisons hissées (`dead=7`, `later` bound) — les `lmd`/`didomi`/`OSM` absents = scripts définisseurs jamais exécutés (**cascade §B-4**, pas un bug de scope) |
| 11 | **`new URL(".", location)`** — base `Location` non stringifiée ; `Invalid scheme` sur chemins `./` non résolus | `explorer-jscore` | fastapi, svelte, lemonde, figaro ; cdiscount `unhandledrejection: Invalid scheme` ×5 `[V4-main]` vol 36 | **VÉRIFIÉ SAIN** — `url_ctor_with_location_base`/`new_url_against_location_resolves` : `new URL('.', location)` → base correcte, résolution `./`/`x` OK ; les `Invalid scheme` restants = autre forme d'entrée à nommer au prochain vol (attribution tracée) ; **vol 132 : résiduel** — `Invalid scheme` encore sur pappers app.js (entrée à nommer) |
| 12 | **`heap memory limit exceeded` en rafale** | `explorer-jscore` (budget ou fuite) | marmiton ×3 au boot (vol 34 — **non reproduit `[V4-main]` vol 36**), bookshop ×14 beacon.min.js | **LIVRÉ — à re-voler** : `heap_dead` latch JS-C170 — le premier dépassement post-GC verrouille une signature d'erreur stable (répétitions tues, `set_mem_limit` réarme) + dedup erreurs pipeline `emit_pipeline_error` (`83916a6` — répétitions consécutives identiques non émises, compteur tracé /16) ; **vol 95 `[V9-main]` : PERSISTANT** — bookshop ×9 occurrences strictement identiques `(1087980 cells, 540750652 B > 536870912 B)` — dedup/latch inopérants terrain ; js_heap 542 Mo résiduel, page néanmoins peinte ; marmiton heap sain (51,4 Mo) ; **vol 96 `[V10-main]` : PERSISTANT ×2 sites** — bookshop ×7 identiques (540,9 Mo, `import()→/null` 404 ×2 reproduit J3) + **frandroid ×6+** (538 Mo, 3ᵉ site après lemonde/bookshop) — dedup `83916a6` toujours inopérante ; js_heap marmiton 343 Mo (hausse vs 51,4 Mo — à suivre) ; **vol 97 post-É152 : EN DÉCROISSANCE** — frandroid ×0 (capé), bookshop ×3 (cap 1/moteur non atteint — variantes de piles passent encore) — racine nommée : le latch tenait la signature stable mais chaque site d'appel produisait une pile JS différente → messages exacts distincts → le dédup sur égalité ne les regroupait pas ; `heap_dead_reported` plafonne désormais à 1 occurrence heap **par moteur** (ré-armé à la nav, oublié à `close_tab`) — verrou `heap_dead_errors_reported_once_per_document` ; **CORRECTIF Étape 153 à re-voler** — vol 97 bookshop ×3 : le drain `unhandledrejection` émettait `PipelineError` en envoi **direct**, contournant dédup + cap (les erreurs heap qui rejettent des promesses) ; routé par `emit_pipeline_error` ; **vol 100 `[V10-main]` post-É153 : PERSISTANT** — bookshop ×5 entrées heap signature identique (1 078 884 cells, 540,9 Mo) dont ×11 et ×3 dédup — cap 1 **par moteur** insuffisant : rafale cross-engine / drain post-mort non plafonné ; **CORRECTIF Étape 156 à re-voler** — racine nommée : les scripts `async` évalués par la vague détachée (`eval_async_arrival`, moteur hors verrou) émettaient `PipelineError` **en direct**, hors portée du verrou — idem livraisons réseau/crypto/WS/SSE et évals iframe dynamique ; le verrou devient `Arc<AtomicBool>` partagé (`SubJob` le porte, la porte l'arme par `swap`), les 5 sites live routés par `emit_pipeline_error` — verrou `heap_dead_capped_across_detached_wave` | ; **vol 104 : favorable** — allrecipes 1 seule occurrence heap (cap par-moteur tenu sur ce site) ; **vol 111 : favorable ×2** — sudouest gtm ×1 seule occurrence ; **vol 113 `[V10-main]` post-É156 : RÉSOLU** — bookshop 1 seule entrée heap (550 Mo, cap cross-wave tenu vs ×5) ; **vol 118 : favorable ×2** — allociné/doctissimo ×1 occurrence chacun ; **vol 134 : variante** — `allocation exceeds engine limits` (corriere, sans `heap memory limit exceeded` — à rattacher ou distinguer) |
| 13 | **Hit-test `ignored` en série** — **confirmé sain `[V4-main]` vol 37** (figma clic→nav réelle) — liens/inputs réels non trouvés | `explorer-layout`/`core` | vols 26/29/30 (V2 integ) : 4/4 clics refusés | **PROBABLEMENT RÉSOLU** — cause identifiée : wrappers `visibility:hidden` en attente d'hydratation (GFG), pas un bug hit-test ; **clics = vraies navigations V3/V4** (vols 34/37/44/45) ; **formulaire bout-en-bout validé** (ddg lite : focus→saisie→Enter→SERP→article, vol 44) | **RÉSOLU confirmé `[V4-main]`** — attention variante : élément sous overlay reste `ignored` (§B-18 qwant, §B-17 ratp) |
| 14 | **APIs DOM restantes** : `document.write` ordonnancé post-commit (tumblr), `instanceof Element` trop strict (impots, ameli), `document` of Null (dw), `Illegal component name ""`/`Function.name` (dw), `debugger` token tue un module (w3schools), `get`/`bind` of Undefined (marinetraffic, tiktok — cascades à nommer), `GeolocationPosition is not defined` (marmiton `[V4-main]`), `document.contains` absent (ratp vol 45), `GeolocationPosition`/`navigator.geolocation` absent (marmiton vol 36 — « le navigateur n'a pas de GPS », feature-detect géoloc des sites), `Node.replaceChild: not a node` (dsfr-1.14 service-public vol 47), **ameli `call of Undefined` Bootstrap RÉSOLU confirmé `[V4-main]` vol 59** (console vide, icônes peintes vs carrés vides vol 29), `customElements.define` reçoit `undefined` (reddit vol 38), `navigator.userAgentData`/`getHighEntropyValues` absent (speedtest vol 78), `Response.json` absent (regex101 `'json' is not a function` `[V4-main]` vol 55) | `explorer-jscore` | vols 7/8/16/18/23/24/25/27-30/36 | **PARTIEL** — `call of Undefined` Bootstrap **RÉSOLU confirmé `[V4-main]`** (JS-C162 — console OSM vide, Leaflet instancié ; reste tuiles absentes → §B-4) ; `attachShadow` sorti du stub minimal ; reste la matrice ci-contre ; **vol 101 : +2** — `Response.json` absent (regex101 `json is not a function`, SPA morte), `addEventListener is not a function` sur Object (edf clientlib/dynatrace) ; **vol 102 : +2** — `addEventListener` sur `IDBRequest` (mastodon), `appendChild: not a node` (tumblr cmp-bundle) ; **vol 103 : +5** — `hasOwnProperty` sur `CSSStyleDeclaration` (jvc), `Promise.finally`/`.on` absents (dailymotion), `configurable of Undefined` (osano flickr), `defineProperty QuickEffectIds` (flickr), `id of Undefined` didomi legacy (manomano) ; **vol 104 : +2** — `FontFaceSet.add` ×2, `getAttributeNS` sur HTMLElement (boursorama) ; +`throttle` (allrecipes) ; **vol 105 : +2** — `name of Undefined` (leparisien), `textContent of Undefined` (20minutes) ; **vol 107 : +3** — `Promise.onReadyStateChange`/`onPrimary` (franceinfo), `filter` sur function + `settings` survicate (deezer) ; **vol 109 : +3** — `HTMLDocument is not defined` (orange cookie-monster), `initContext` sur Object (carrefour), cascade `moteur figé/éval sautée` doctolib rspack ; **vol 110 : +3** — `tooltip` sur `St` (OSM tuiles toujours absentes → recoupe §B-4), `delete` sur Array + `insertBefore: not a node` appconsent (lachainemeteo blanche) ; **CORRECTIF JS-C184 à re-voler** — `*AttributeNS` ×4 (get/has/remove/set, match partie locale `p:l`, `ns` nul/`""` = non préfixé — boursorama vol 104) + `FontFaceSet.add` retournant le set + `has_method` (`typeof fonts.add` détectable) — verrous `b14_probe.rs` ; bras `Promise.finally`/`.on` (dailymotion vol 103) **analysé, non reproduit** — receveur Promise réel (`this.player=await getPlayer()`), aplatissement promesses vérifié sain (7 cas) ; suspect promesse étrangère injectée amont — repro env iframe requis ; **vol 111 : +2** — `require` CJS absent (sudouest), hydration lit/ssr mismatch (MDN) ; **vol 112 `[V10-main]` post-JS-C184 : bras `FontFaceSet.add`/`getAttributeNS` RÉSOLU** (boursorama 0 occurrence vs ×3 vol 104) ; restants `update`/`translate`/`addEventListener of Undefined` ; **CORRECTIF JS-C185 à re-voler** — deux bras nommés : (a) `HTMLDocument is not defined` (orange vol 109) → constructeur global ajouté, `HTMLDocument.prototype` hérite `Document.prototype`, `document`/`createHTMLDocument` portent le bon proto, back-props `constructor` posées ; (b) `insertBefore: not a node` (appconsent vol 110) → adoption spec `[[Insert]]` : `Dom::adopt_from` copie itérative cross-arène câblée sur `appendChild`/`insertBefore`/`replaceChild` + rebinding de l'objet JS (move, identité préservée) — verrous `b14_probe.rs` + `dom.rs` ; le rejet borné reste sur `removeChild`/`insertAdjacentElement`/`prepend` et observateurs ; **CORRECTIF JS-C186 à re-voler** — bras `hasOwnProperty` sur `CSSStyleDeclaration` (jvc vol 103) : `StyleHost::get` servait `""` pour tout nom → `Object.prototype` avalé, méthodes host `typeof`='string', symboles avalés ; borné aux noms CSS + `has_method` ajouté ; **vol 113 post-JS-C185 : ×2 RÉSOLU** — `HTMLDocument` (orange) + `insertBefore` adoption (lachainemeteo) disparus ; restants `get`/`delete`/`styled-components` ; **vol 114 post-JS-C186 : `hasOwnProperty`/`CSSStyleDeclaration` RÉSOLU** (jvc) ; suivant `insertAdjacentHTML: bad position` ; **CORRECTIF JS-C187 à re-voler** — bras `addEventListener` sur `IDBRequest` (mastodon vol 102) : `IDBRequest` devient `EventTarget` via rail `__et` partagé (map paresseuse — couvre aussi les greffes `addEventListener.call(o)` des polyfills), `fire_idb` tire les listeners après les `on*` — dexie/idb-keyval couverts ; **vol 115 : +2** — `addEventListener` sur FontFaceSet (impots DSFR), `postMessage` 2-args + `apply of Undefined` (steam) ; **CORRECTIF JS-C188 à re-voler** — bras `insertAdjacentHTML: bad position` (jvc vol 114) : jQuery écrit `beforeEnd` camelCase, spec = positions ASCII case-insensitive ; `pos` normalisé lowercase sur les deux rails (HTML + Text/Element) ; **vol 116 post-JS-C187 : `IDBRequest.addEventListener` RÉSOLU** (mastodon) — mur reculé : `then is not a function (on IDBRequest)` (await direct → thenable requis) ; **vol 117 post-JS-C188 : `insertAdjacentHTML` RÉSOLU** (jvc) ; suivant `category of Undefined` getjan ; **CORRECTIF JS-C189 à re-voler** — deux bras : (a) `postMessage` 2-args (steam vol 115) → forme WHATWG moderne : `targetOrigin` optionnel défaut `"/"` (same-origin), objet `options` accepté, `'*'`/`''` inchangés ; (b) `addEventListener` sur `FontFaceSet` (impots DSFR) → spec `EventTarget` : rail `__et` partagé délégué en invoke ; **vol 118 : +3** — `addEventListener` sur Array (allociné), `toLowerCase of Undefined` (doctissimo), `disableUaHints` id5-sync (pap) ; **CORRECTIF JS-C190 à re-voler** — bras `then` sur `IDBRequest` (mastodon vol 116) : proto exposé thenable — promesse interne résolue `result`/`error` via `__et` + vrai `Promise.prototype.then` chaîné (déviation spec assumée : `await req` rend le résultat, pas la request) ; **vol 119 post-JS-C189 : `postMessage` 1-arg + `FontFaceSet` EventTarget RÉSOLUS ×2** (steam console sans postMessage, impots vide) ; **vol 120 post-JS-C190 : bras `IDBRequest` ENTIÈREMENT RÉSOLU** (mastodon : thenable 0 occurrence — IDB traversé, reste fetch storm) ; **vol 121 : +3** — `fromRGB` (spotify), `x`/`addEventListener`/`name` sephora + invariant `document.currentScript` ; **CORRECTIF JS-C191 à re-voler** — bras `addEventListener` sur Array (allociné vol 118) : collections DOM sans `Symbol.toStringTag` → détecteurs `^\[object (HTMLCollection|NodeList|Object)\]$` rejetaient ; tags spec posés (NodeList/HTMLCollection/FormControls/Options/RadioNodeList) ; **vol 122 : +4** — `javaEnabled`/`AbortSignal.timeout` (maif), `evaluate: unsupported XPath` + `f`/`toLowerCase` (insee) ; **invariant `document.currentScript` reproduit ×3 sites** (sephora, france.tv, arte — priorité : un fix touche 3+ sites) ; **vol 123 : +4** — `defineProperty` non-object (x.com), `document of Null`/`name`/`AK`/`replace` (asos) ; ⚠ JS-C191 non testé : build cassé (`mse.rs`+`mod mse` non commités) ; **vol 124 : +7** — `postMessage: cannot clone host object` + `Body has already been consumed` + `spécificateur irrésolvable` + `hydrateWhenVisible` (lequipe), `getTech` (rtl), marfeel `Invalid scheme`/`post`/`type` (europe1), `displayedMegaMenu`/`brand`/`footer` (nicematin) ; **CORRECTIF JS-C192 à re-voler** — invariant `document.currentScript` ×3 sites (sephora/france.tv/arte) : interfaces DOM tag-spécifiques — `node_value` pose le proto `HTML*Element` par tag (~70 entrées, fallback `HTMLElement`), `HTML*Element.prototype` hérite `HTMLElement.prototype` (plus `Element` direct), `el.constructor` rend le ctor global tag-spécifique avec accesseurs IDL portés sur les protos globaux (`inputValueTracking` préservé) ; bras vol 121 `fromRGB`/`x`/`addEventListener`/`name` restent ; **vol 125 : +5** — `secp256k1` unknown curve (caf stormcaster), `tooltip`/`modal` on ce, `no module loader` (franceculture — bras modules partagé lequipe), `javaEnabled`, `document of Null` ; **vol 126 : +2** — `postTask` Scheduler API (dna), `push of Undefined` (ouest-france) ; **CORRECTIF JS-C194 à re-voler** — `AbortSignal.timeout` + `navigator.javaEnabled` (maif vol 122, `javaEnabled` re-frappé vol 125) : `timeout(ms)` ajouté au ctor xhr (signal host aborté par timer, raison `TimeoutError` — monde consommé par `fetch`), `AbortSignal.prototype` chaîné sur les signaux (`instanceof` réparé), `javaEnabled()` → `false` spec ; **vol 127 : +4** — `MutationObserver: callback requis` (ryanair), `postTask` **2ᵉ site** (airbnb après dna — Scheduler API récurrent), `preloadRestQuery`, assets `0 Unknown Error` ; **vol 128 : +4** — `hasAttribute on HTMLDocument` (SG), `invalid token`/JSON (meteofrance), `swmApi` (SG) ; **CORRECTIF JS-C195 à re-voler** — `scheduler.postTask` ×2 sites (dna vol 126, airbnb vol 127) : promesse résolue au retour de `cb` schedulée via timers (`delay`), rejet `AbortError` si `signal` aborté (re-lu au tir), rejet si `cb` jette ; `priority` accepté non ordonnancé (FIFO — déviation bornée) ; **vol 129 : +4** — `pipeTo source stalled`/`unsafeSVG` (epic), `push`/`0`/`name of Undefined` (vdn/lesechos) ; **vol 130 : +7** — `crisis`/`browserupgrade`/`map`/`oneId`/`a function is required` (DLP), `push`/`get` (sante) ; **CORRECTIF JS-C197 à re-voler** — `evaluate: unsupported XPath` (insee vol 122) : grammaire élargie (axes `ancestor::`/`self::`/`following(-sibling)::`/`preceding(-sibling)::`/`parent::`/`descendant*`/`attribute::`, prédicats `contains`/`starts-with`/`ends-with`/`text()`/`last()`/`position()`/`not()`, union `|`) + dégradé résultat-vide/`console.warn` hors sous-ensemble (plus de `TypeError` fatal) ; racine ryanair `MutationObserver: callback requis` (vol 127) : `navigator.vendor` absent → `vendor.toString()` tuait le boot → props spec posées (`vendor`/`appCodeName`/`product`/`productSub`/`vendorSub`) ; **vol 131 : +4** — `node is from a different document` (sfr — adoption résiduelle), `fetch: url required` + `messages` (laposte), `addEventListener` ruxit (axa) ; **vol 132 : +4** — `postTask` **3ᵉ site** (estrepublicain — SDK prsmedia), `consentRejection` sourcepoint, `value undefined`, `Invalid scheme` (pappers → recoupe §B-11) ; **CORRECTIF JS-C198 à re-voler** — racine nommée : `hasAttribute is not a function (on HTMLDocument)` (SG vol 128) — marcheur d'ancêtres `parentNode`×n gardé par `"body"===r.localName` ; `localName` absent → remontée jusqu'au document → TypeError ; spec posé : minuscule sur éléments, `null` nœuds non-élément ; **CORRECTIF JS-C199 à re-voler** — racine nommée : `node is from a different document` (sfr vol 131) — résidu JS-C185 : `norm_children` (`append`/`prepend`/`before`/`after`/`replaceWith`) et `insertAdjacentElement` rejetaient les nœuds étrangers ; adoption câblée (move + rebind), seul `removeChild` reste borné ; **vol 134 : +3** — `allocation exceeds engine limits` ×N (corriere nielsen — allocation brute vs heap), `useCanonical`/`hasWebVitals` marfeel ; **vol 136 : +4** — `replace`/`id`/`Translator` (orpi), `Array of Null`/`startTime` bazaarvoice (yves-rocher) |
| 15 | **Shadow DOM / custom elements** : `attachShadow` stub, `faceplate-*`/`shreddit-*` 0×0, slotted content absent | `explorer-jscore` | reddit login (28 partials vides) ; **`[V4-main]` vol 38 : DÉGRADÉ EN MIEUX** — challenge JS franchi, vraie page r/rust peinte (13 shreddit-post avec contenu), upgrade custom elements tenté (`customElements.define` reçoit `undefined` ×3 → §B-14) | **STRUCTUREL, amorce livrée** — surface JS posée (attachShadow→fragment, getElementById, instanceof, proto chaîné WebSocket/BC/EventSource lisibles) ; slotted/custom-elements upgrade reste le chantier (issue #25) ; **+1 bras `[V4-main]` vol 57** : DSD `shadowrootmode=open` (pe-cookies francetravail) aplatie → `position:fixed` 1366×900 en backdrop plein écran |
| 16 | **WebGL/canvas lourd** | `jscore`+`render` | threejs, ciechanowski, pixi, radio.garden, flightradar | Hors scope court terme (ADR stubs) — stub Ctx2d/toDataURL livrés |
| 17 | **Menus/overlays résiduels** — **reproduit `[V4-main]` figma** (méga-menus peints ouverts landing+interne) : ceux fermés **par JS** restent ouverts (drawer `inert`+`aria-hidden` peint ouvert santemagazine `[V6-main]` vol 71) : ceux fermés **par JS** restent ouverts (drawer `inert`+`aria-hidden` peint ouvert santemagazine `[V6-main]` vol 71) (le CSS est corrigé : `hidden`, `[popover]`, `opacity`, `inset`, `details`) ; off-canvas GOG/IGN, backdrop francetravail, megamenus 750g/doctolib | `jscore` (JS mort) + résiduels css | nombreux vols | **EN DÉCROISSANCE** — reste dû aux boots morts, pas au pipeline ; **sous-cas confirmé `[V4-main]` vol 45** : nav off-canvas ratp fermée mais enfants déployés/peints/interceptant hit-test (clic burger `ignored`) ; **volet interaction R-127** : `[inert]` porte désormais `pointer-events:none` en UA (HTML §6.4.2 — le sous-arbre inert n'avale plus les clics même peint) + `pointer-events` rejoint `INHERITED_PROPS` (héritage css-ui-4 manquant — un `none` de conteneur n'atteignait pas les enfants) ; `aria-hidden` inchangé (accessibilité seule, pas d'effet paint/hit) ; le « peint ouvert » lui-même reste du ressort des boots JS ; **vol 103 : +1 jvc** — mégamenu peint ouvert non stylé (boot mort `hasOwnProperty` → §B-14) ; **vol 107 : +1 franceinfo** — panneau compte peint ouvert |
| 18 | **Layout résiduel** : chevauchements de runs inline (excalidraw/gitlab/DW/deezer — non reproduit), `overflow-x` clip (guardian), marquee dupliqué (vercel), sticky repaint (accuweather), dark theme classe JS (vercel), double émission suspectée synchro DOM (franceinfo/gitlab header), bandeau noir 250px CNN, runs éphéméride wikipedia `[V4-main]` vol 56, **runs inline superposés confirmés `[V4-main]`** (lemonde « À Marseille… » + en-tête ouest-france vol 48), **hero img occluant l'input recherche** (qwant `[V4-main]` vol 44 : input layouté mais 884×772 img par-dessus → clic `hit=true` sur l'image, saisie impossible), **+2 `[V9-main]` vol 94** (lemonde hero/titre superposés, cjoint texte sur titre carte SM-R322), **+1 vol 95** (letterboxd : texte d'affiche peint **miroir/retourné** — transform raster non composé ?) | `explorer-layout`/`render` | vols 11/13/20/22/31/32/33/44 | **PARTIEL** — **hero colonne RÉSOLU** (calc `min/max/clamp` factor + plancher min-content du shrink flex, repro notion verbatim vert) ; restes à re-voler |
| 19 | **`document.write`/`async` ordonnancement** — scripts dyn drainés post-commit, pas en ordre de parse | `explorer-core` | tumblr main-*.js jamais exécuté | **OUVERT — vol 102 `[V10-main]` reproduit** : 4 évals inline courts, `main-*.js` jamais évalués, app non montée |
| 20 | **Images `data-src`** : repli livré (840a33c) mais marmiton encore gris | `explorer-core` | vols 34+36 `[V4-main]` : 215 `data-src`, `<img>` sans `src`, gris à +30 s et scroll | **CORRECTIF Étape 149 — à re-voler** : double racine posée — (a) `data-srcset` collecté comme URL brute (`url 1x, url2 2x` → `Url::resolve` en échec, jamais fetché) → passe désormais par `pick_srcset` comme `srcset` ; (b) `max_images` 32→256 : la première vague (icônes menus cachés) épuisait le budget avant les images éditoriales (marmiton ~216). Verrou `collect_images_parses_lazy_data_srcset` ; **vol 95 `[V9-main]` (binaire pré-É149) : favorable** — cartes recettes + bannière peintes au scroll (repli `data-src` `840a33c` opérant seul, images_bytes 29,9 Mo) ; **vol 96 `[V10-main]` post-É149 : favorable confirmé** — photos peintes au scroll (bannière, packshots, cookies), doc_h=11 098, DELETE propre ; js_heap 343 Mo en hausse (vs 51,4 Mo — régression évals É148 ? à suivre) |
| 21 | **Console `×N` ne borne pas la source** : `r is not defined` ×512 (genius reactAds) = retry/intervalle non borné | `jscore` budget | vol 33 | **LIVRÉ — à re-voler** : `TIMER_FAIL_CAP=8` dans `timers.rs` — un intervalle dont le callback jette 8 fois consécutives n'est **pas réarmé** (warn tracé une fois, `fail_streak` remise au succès) ; exceptions isolées par callback (les autres timers gardent leur tour) + dédup visuel `console_ring_dedup_consecutif` ; **PARTIEL — EN DÉCROISSANCE `[V9-main]` vol 94** — genius : ×512 muet → **×255 `unhandledrejection` nommés** (`reactAds`, stacks complètes) — la source échappe au cap : chaque tick crée une **promesse qui rejette** (hors chemin throws d'intervalle) — reste à borner les rejets de promesses en rafale |
| 22 | **Redux `dispatch` : action non-plain** — `Minified Redux error #7` (« Actions must be plain objects ») : une action non-plain traverse la chaîne middleware/thunk jusqu'au dispatch de base | `explorer-jscore` | mastodon `/about` (vol opérateur, binaire post-C165) — l'app exécute 121 modules jusqu'à `createStore` | **OUVERT** — suspect : composition `applyMiddleware`/`typeof` action thunk non interceptée ; immer résolu (J10) ; **vol 102 : non atteint** — mastodon `/about` mort en amont (127× `fetch: network error`, `addEventListener` sur `IDBRequest` → §B-14), doc_h=0 ; **vol 120 : reste `fetch: network error` ×126 dédup sur mastodon — IDB levé, rafale réseau dernier blocant |

| 23 | **`eval deadline` trop agressif au boot** — 2ᵉ cas terrain : lemonde `[V4-main]` vol 48 (script 254 Ko tué à 30 s → disjoncteur, heap 0) — compile légitime tuée | `explorer-jscore`/`js` | cryptpad `[V4-main]` vol 37 : `RangeError: eval deadline exceeded` sur less.min.js → boot RequireJS mort (J1 livré mais seuil bas — à étalonner/exempter phase boot, lien J1/N3) | **CORRECTIF `c5d6dde` à re-voler** — split canal adaptateur : évals (`eval_cmd`/`eval_ctx`/`eval_script_el`/`eval_module`) passées à 65 s, pompes/drains conservés à 30 s (lemonde boot 254 Ko tué au `recv_timeout` 30 s alors que le budget boot est 60 s) ; **RÉSOLU `[V9-main]` vol 94** — lemonde : éval boot 58,2 s **non tuée** (vs kill à 30 s), boot aboutit, erreurs restantes toutes nommées à `/console` ; **vol 95 : REPRODUIT sur cryptpad (site d'origine)** — `eval deadline exceeded` fige le boot requirejs/less au splash, doc_h=8 — le canal 65 s ne couvre pas les boots multi-chaînes → **RÉOUVERT partiel** (variante cryptpad) ; disjoncteur §B-41 armé en bonus (streak=64) ; **CORRECTIF JS-C176 à re-voler** — racine nommée : `run_microtasks` ré-armait la borne **standard** 10 s à l'entrée du drain, écrasant la deadline boot 60 s → la chaîne promise produite par le corps de boot mourait à 10 s ; `run_microtasks_in_boot` partage désormais l'armement boot pour `run_program`/`eval_module` (pompes/événements inchangés à la borne standard) — verrou `boot_microtask_drain_shares_boot_budget` ; **vol 97 `[V10-main]` : PERSISTANT** — cryptpad : `eval deadline exceeded` sur `less.min.js` (l'éval elle-même, hors chemin microtask drain), boot figé doc_h=8, disjoncteur §B-41 ×1 ; **CORRECTIF `JS-C179` à re-voler** — racine nommée : la factory d'init (requirejs/less compile) court dans un callback `setTimeout`/promise de **pompe**, pas dans l'éval du script — le batch armait la borne standard alors que le document bootait ; `CtxMeta.boot_phase` ouvre une fenêtre boot par contexte (pompes + drains partagent le budget boot), refermée définitivement à `readyState "complete"` — verrou `boot_window_extends_to_timer_pump` ; **vol 100 `[V10-main]` : PERSISTANT, variante sans deadline** — cryptpad : 1 seul éval (2 217 o) sur 55 `<script>`, chaîne dyn requirejs/less **jamais évaluée**, splash figé doc_h=8, disjoncteur §B-41 ×6 — la fenêtre boot couvre les pompes pendant l'éval, ici l'éval du script de boot n'a jamais lieu (injection dynamique hors canal — recoupe §B-4/§B-19) ; **vol 101 : reproduit à l'identique ×2** (signature stable) |
| 24 | **Stretched-link incliquable** : `<a>` `position:absolute`+`inset:0` → boîte 235×0 px, hit-test `ignored` sur toutes les cards | `explorer-layout`/`css` | vinted `[V4-main]` vol 39 : rendu ~95 % fidèle mais entièrement incliquable ; `logs/vol39_vinted_layout.txt` 15 ancres ×0 hauteur — le fix `inset` ne couvre pas le stretch vertical (hauteur parent auto ?) | **RÉSOLU `[V4-main]`** — `9b0bf5c` R-121 : ancre `top+bottom`+`height:auto` s étire (§10.6.4) ; validé terrain vinted `lien:OK+back` |

| 25 | **Navigation `location` spontanée non sollicitée** — `location.href` posé sans input (détection env ?) | `explorer-jscore`/UA | qwant `[V4-main]` vol 44 : redirigé vers `/mobile-apps?utm_campaign=w2m` — nav unique (§B-2 tient) mais suspect `navigator.*`/UA feature-detect | **OUVERT — terrain V4** |

| 26 | **Wedge DOM détaché terminal post-apply** — `journal_fallbacks` non livré OU apply mort entre `layout` et `repaint` → `/dom` 404, `/frame` 204, **fuite mémoire sans borne** (caf 2,93 Go vol 47 ; **bfmtv 6,88 Go en 5 min vol 49 — escalade §B-26→§B-1, API entière morte** ; figaro variante sans UnmappedKey) | `explorer-render` (fix) | caf/figaro/bfmtv/radiofrance/meteofrance `[V4-main]` vols 47/49/54/59 (meteofrance : apply arrêté à `layout ms=253`, dent de scie) ; **musee-orsay vol 62** : apply stoppé post-`layout ms=968`, `pump.js_run_sync` ~8 ms/5 s persistant, fuite 976→2,35 Go **poursuivie post-DELETE** (recoupe §B-30) (radiofrance : apply en escalade 0,4→9,2→>49,5 s, +90 Mo/s, API vivante) — signature commune : apply journalisé jusqu'à `layout`, jamais `repaint`/`fin` | **CORRECTIF `[V5-main]`** : localisé dans `build_display_list` — une file de porteurs reportés se re-remplissait de la même cible (cycle de report, chemin `[0,0,0,0]` mesuré) → borne `MAX_EMIT_PER_PATH=4` + plafond 100 k, `dropped_emits`/`first_cycle_path` remontés. meteofrance re-vol : `dropped=1`, `apply: fin ms=2308`, process vivant. Variantes à re-valider : figaro (libérée au DELETE), musee-orsay (post-DELETE = §B-30), bfmtv (dent de scie) **+`[V6-main]` vol 67 : RÉSOLU ×3** — borne `emit_scoped_at` (`1e24942`) : meteofrance (`cycle cassé dropped=1 path=[0,0,0,0]` ×994, `apply: fin`, RAM stable 746 Mo), orsay (`[0,6,0]`, purge post-DELETE → §B-30 favorable), figaro (`[0,1]`, ~0,1 Mo/s vs 29-38 ; reste frame blanche JS → §B-4) ; bfmtv dent-de-scie non re-volée | **RÉSOLU `[V6-main]` vol 67 — 3 variantes (meteofrance/orsay/figaro) ; bfmtv à re-voler. +R-98 cause racine** : `push_z` triait sur le préfixe déjà drainé (entrée fusionnée sous le curseur perdue + entrée émise ressortie → cycle reproduit) **et** `emit_scoped`/`emit_scoped_at` oubliaient `escaped` (descendants différés perdus à la copie) — les deux corrigés, verrou `incr_fixed_dans_contexte_empilement`, garde conservée en dernier recours ; **confirmé `[V8-main]` vol 88** : meteofrance apply complet en boucle (layout ~200 ms → repaint ~250 ms), **0 `cycle de report`**, RAM bornée ~1 Go/3 onglets ; **bfmtv RÉSOLU `[V9-main]` vol 93 — toutes variantes fermées** : `apply: fin ms=1440`, repaints ~700 ms, RAM plate ~864 Mo sur 60 s de pompe JS (vs 6,88 Go/5 min), API vivante, `commit.drains` 126-339 µs, disjoncteur §B-41 armé (`tempête de pompes` streak=64) — anomalies résiduelles rapportées : mégamenu peint alors que boîte à x=-344 (écart raster/layout), `data:image/svg+xml` brut peint en texte |

| 27 | **Image en mosaïque géante plein viewport au scroll** | `explorer-render`/`layout` | ouest-france `[V4-main]` vol 48 scroll_y=1800 — tuile répétée couvrant tout l'écran (background-repeat ? border-image ?) | **CORRECTIF R-129 à re-voler** — root cause : `expand_flex` éclatait `flex:0 0 calc(75% - 5px)` sur l'espace interne du `calc` → basis refusé → repli `max_content` 13387.949px figé par `flex-shrink:0` ; + `layout_item` résolvait les `%`/`max-width` de l'item contre sa propre taille (940→700) — cb item = boîte conteneur. Verrous `b27_probe_*` sur markup+CSS réels ; **RÉSOLU `[V10-main]` vol 97** — ouest-france : plus de fils 13 387 px, cartes en colonnes normales (`captures/vol97_ouestfrance_scroll.png`) |

| 28 | **`height:100%` plein-viewport effondrée à 0** — chaîne `html`/`body`/`#map` = `1366x0` → Leaflet lit `clientHeight≈0`, ne demande qu'1 tuile | `explorer-layout`/`css` | uMap éditeur `[V4-main]` vol 50 — tuiles OSM peintes mais page à hauteur 0 (paradoxe : tuiles visibles) | **CORRECTIF `816f7c9` a re-voler** — motif plausible : unites viewport dynamiques `100dvh`/`svh`/`lvh` non parsees -> chaine `height` ignoree -> html/body/#map a 0 (dump : 1366x0). Fix : projection d*/s*/l* sur vh/vw (sans chrome mobile, §6.1.3). Si le re-vol persiste : motif `min-height`/`flex` wrapper a isoler ; **vol 102 `[V10-main]` : PERSISTANT** — uMap : chaîne `#map` toujours 1366×0 (×6 boîtes), tuiles ~100 px — motif wrapper reste à isoler (`logs/vol102_umap_layout.txt`) |

| 29 | **Boucle iframes `sodar` non bornée** — AdSense crée des iframes télémétrie en continu : 419 dans le DOM (+~90/min), `js_heap` 53→160 Mo (~40 Mo/min), cycles apply ~1,3 s | `explorer-core`/budget | linuxmint `[V4-main]` vol 51 — ad-tech fonctionnel (slot peint) mais DOM+heap gonflent ; API responsive, RAM libérée au DELETE | **CORRECTIF `a1d0dbd` à re-voler** — iframes dynamiques bornés (`pending_dyn_frames` cap 64 — sodar AdSense) ; **RÉSOLU `[V9-main]` vol 94** — linuxmint : **0 iframe/0 sodar** dans le DOM (vs 419, +90/min), `js_heap` 21 Mo vs 160 | |

| 30 | **Fuite mémoire post-DELETE sans borne** — croissance +~28 Mo/s après fermeture de tous les onglets (`{"tabs":[]}`), API responsive, 0 log | `explorer-core` | manomano `[V4-main]` vol 53 : 1,7→2,98→4,27 Go avec zéro onglet — suspect : tâche navigate/pompe orpheline — **preuve directe vol 59** : `pump.commit_lock us=89 918 900` (~90 s) pour tab=3 **12 min après DELETE**, RAM 5,10 Go avec `{"tabs":[]}` ; **signal contraire `[V6-main]` vol 67** : orsay DELETE → purge 1 015→780 Mo stable 65 s (variante in-flight purgée par le fix §B-26 ?) | **CORRECTIFS à re-voler** — phases nav orphelines coupées par `nav_alive` (`0ffca68`) + purge payloads/jobs au close (`b408570`) ; vol 67 : purge orsay saine ; **vol 93 : signaux favorables ×3** (purges complètes 721→402 / 610→427 / 1 206→462 Mo, `{"tabs":[]}`), flixbus non mesurable (DELETE impossible sous wedge §B-54) ; **vol 94 : +8 purges complètes** (tous DELETE propres, RAM 552-807 Mo) — reste manomano/flixbus à re-voler ; **vol 95 : +7 purges propres** (dont cryptpad post-disjoncteur, RAM 483 Mo à `{"tabs":[]}` — credit-agricole non mesurable, DELETE impossible sous §B-54) ; **vol 96 `[V10-main]` : +4 purges propres** (marmiton, bookshop, manomano, credit-agricole) **mais signal adverse flixbus** — DELETE en timeout puis **pompe orpheline : API morte >30 s, livelock, RAM 1,6 Go** (variante post-DELETE du §B-54, arrêt forcé PID) ; **CORRECTIF Étape 154 `ee6008fa` à re-voler** — moteurs orphelins détruits sur fil dédié hors verrou (`destroy_engine_offlock` : `commit_js_pump` onglet mort/époque périmée, `close_tab`, commit nav, re-take dyn-module) + purge `dyn_tasks`/streaks au DELETE ; **vol 100 : É154 non mesurable** — wedge flixbus pré-DELETE (§B-54 vie onglet), DELETE propres ×6 ailleurs (RAM ~400 Mo) ; reste à re-voler : flixbus DELETE post-wedge, manomano | |

| 31 | **Clic `<button onclick>` → `ignored`** — **RÉSOLU `[V7-main]` vol 80** : clic → `edited`, éditeur Photopea monte entièrement — boîte bien layoutée (1311×59), clic centré, refusé ; bloque les apps à bouton primaire (éditeur photopea injoignable) — `<a target=_blank>` fonctionne | `explorer-core`/hit-test | photopea `[V4-main]` vol 55 — suspect : handler `onclick` attribut non branché côté hit-test | **RÉSOLU `[V7-main]` vol 80** — clic → `edited`, éditeur Photopea monté intégralement |
| 32 | **Rasters pixelisés basse résolution** — icônes/logo/éditeur en blocs (downscale ou upscale brut ?) | `explorer-render` | regex101 `[V4-main]` vol 55 (certitude moyenne — verifier si srcset/lowres) | **CORRECTIF R-128 — à re-voler** : `paint_image_axis` (chemin commun axis-aligned) échantillonnait en nearest-neighbor → bilinéaire 2×2 en prémultiplié (pas de halo alpha), bord clampé, arrondi au pixel (la troncature 254.9→254 cassait 2 reftests WPT, corrigée, 115/151 conservés) ; `paint_tiled` inchangé (tuiles 1:1) ; **vol 97 `[V10-main]` : PARTIEL** — regex101 : logo encore flou/bloc (source basse rés ? à trancher vs réf) ; SPA non montée, menu vertical empilé (anomalie layout adjacente) ; **vol 101 : PERSISTANT ×2** — logo toujours bloc, SPA morte `Response.json` absent (→ §B-14) |

| 33 | **Wedge `nav_paint` sans timeout** — `take_nav_paint → Busy → sleep(50ms)` en boucle à vie (`shared.rs:157-166`) : `POST navigate` jamais retourné (>25 min), DOM 404/frame 204 permanent, 0 WARN, `journal_fallbacks=0`, ~1,5 cœur saturé | `explorer-core` | francebleu→ici.fr + 20minutes `[V4-main]` vol 52 — wedge en amont (`insert_images`/`styles_payload`) ou `composite_frames` ; distinct de §B-26 (fallbacks=0, navigate en vol) | **CORRECTIF `20f2336` à re-voler** — `nav_paint` borné 30 s + `CoreError::Timeout` — le `Busy` perpétuel ne peut plus retenir `POST navigate` ; **RÉSOLU `[V9-main]` vol 94** — francebleu→ici.fr navigate retourné ~150 s (vs >25 min), page peinte ; même nav : panic dépendance `resvg` rattrapé → §B-55 | |

| 34 | **`max-height` (%) ignorée sur élément remplacé** — `<img>` taille intrinsèque 703×994 dans `div.h-48` au lieu de ≤192 px, clip `overflow` | `explorer-css`/`layout` | cjoint `[V4-main]` vol 56 (`img [-159 -222 703x994]` dans div 366×192) — bras opposé de §B-28 | **CORRECTIF `39b4066` a re-voler** — `layout_inline` recevait `cb_h` du grand-parent ; les atomes recoivent `h_spec` du conteneur — img 703x994 clampee a 192, verrou vert ; **RÉSOLU `[V9-main]` vol 94** — cjoint : images ~355 px bornées dans leurs cartes |

| 35 | **Colonne `block` effondrée à largeur 0** — texte mot-par-ligne | `explorer-layout`/`css` | wttj `[V4-main]` vol 57 | **RÉSOLU `[V5-main]` vol 64** — h2 1261 px, p 252-1286 px (subgrid col-span-N) |
| 36 | **SVG peint/placé hors de sa boîte** — `block<svg>` positionné hors du bouton parent (x décalé) ou à échelle géante (wordmark 700 px dans boîte 300×81) | `explorer-layout`/`render` | wttj smear + indeed wordmark `[V4-main]` vol 57 — recoupe §B-32 pixelisé | **OUVERT — `[V10-main]` vol 102 confirmé ×2** : wttj bloc jaune géant plein viewport (« tech. » seul) |

| 37 | **Panic `index out of bounds` `dom.rs:2691` en `event.dispatch`** — NodeId obsolète post re-parse ADR-0025 (24 rejets `Fingerprint` en rafale) → garde-fou a rattrapé (disjoncteur, onglet JS mort) — **violation ADR-0008**, 2ᵉ panic jscore, 1ᵉ rattrapé | `explorer-jscore` | caniuse `[V4-main]` vol 58 | **CORRECTIF `101a427` à re-voler** — `node_at_path` borné : `.get()` → `None` typé au lieu de l'indexation directe sur un `NodeId` tué au re-parse (ADR-0008) ; **TOUJOURS PRÉSENT intermittent `[V9-main]` vol 94** — caniuse : panic `dom.rs:2752` `len 6 / idx 386` ×1 rattrapé (disjoncteur, scripts sautés) — **second site d'indexation** hors du fix (2691), contexte re-parse `Fingerprint`, non reproduit au re-vol ; **CORRECTIF JS-C175 à re-voler** — `ancestors`/`element_children`/`slots_in`/`slot_assigned`/`assigned_slot` passés en `.get()` + `propagate_event` déclare `node_valid(target)` en entrée — id mort = no-op, jamais panic ; **RÉSOLU `[V10-main]` vol 97** — caniuse : 0 `index out of bounds`/`dom.rs` au log | |
| 38 | **Item flex layouté à 5 839 px** — nav expulsée hors viewport (x=5 879), logo en ellipse aplatie | `explorer-layout`/`css` | developer.chrome.com vols 58+80 (`[V7-main]` verbatim : `[24 6 5839×36]`, nav expulsée) — le fix §B-49 ne couvre pas ce motif | **CORRECTIF `1c9102e` a re-voler sur binaire frais** — verrou local : 0 element expulse x>=800 (430 boites) ; vol89 montre encore 5839 car binaire anterieur au fix — `%` de `min|max-width` des descendants resolus contre la taille main du conteneur (§9.9.3) + `height` resolue derive la contribution par ratio (§10.6.2) ; chemin atome : le remplace recoit sa taille utilisee, pas la ligne. Repro verbatim : img{max-width:100%;height:28px} intrinseque 5839x745 dans <a{display:flex}> -> item ~220 px ; **RÉSOLU `[V10-main]` vol 101** — developer.chrome.com : 0 boîte >1366 px au dump layout, nav/logo/hero ~98 % fidèle |
| 39 | **`DELETE /api/tabs` → deadlock** — mémoire libérée, CPU nul, port LISTENING, API muette >20 s | `explorer-core`/`api` | vol 58 tab 1 — distinct de §B-33 (pas de spin) et §B-30 (pas de fuite) | **CORRECTIF `b9189d5` à re-voler** — livraisons `commit_js_pump` plafonnées à 45 s sous le verrou session (le 90 s `commit_lock` du vol 59 ne peut plus bloquer `DELETE`/API) + stall disjoncteur armé à l'expiration | |

| 40 | **Hit-test `ignored` sur ancre réelle à scroll_y>0** — `<a href>` layoutée, clic refusé en position scrollée alors que clic en haut navigue | `explorer-core`/hit-test | trainline vol 60 + **chrome.dev vol 80 `[V7-main]`** (ancre `ignored` à scroll_y=800) ; « Railcards » → `navigated` à 0 — décalage scroll suspect | **CORRECTIF `efd662c` à re-voler** — position visuelle sticky prise en compte au hit-test scrollé (PIPE) ; **RÉSOLU `[V9-main]` vol 94** — trainline : clic ancre footer à scroll_y=1 600 → `navigated` → article complet |
| 41 | **Wedge starvation phases `event.*` sans fin** — boucle `poll_net` 5-31 s / `mirror_sync` 10-13 s / `dispatch` ~5 s en continu, navigate jamais retourné, API affamée, RAM stable | `explorer-core` | flixbus `[V4-main]` vol 60 **2/2** + **credit-agricole vol 63** (`commit.drains` 60→185 s, `poll_net` 218 s, wedge scopé onglet, RAM libérée au DELETE — sans fuite), corrélé à `UnmappedKey` re-parse ; distinct de §B-26 (pas de dernier `apply: layout`) et §B-33 (journal non muet, fallback>0) | **CORRECTIFS à re-voler** — série de pompes productives bornée à 64 → stall nommé (`17f5cfb`) + drains finaux plafonnés (`b93107c`, credit-agricole `commit.drains` 185 s) + série `UnmappedKey` bornée à 8 → voie journal sautée (`911dce2`, RÉSOLU `[V8]` vol 88 sur bouygues) ; **vol 93** : disjoncteur tiré terrain sur bfmtv (`tempête de pompes — disjoncteur armé` streak=64), `commit.drains` 126-339 µs ; flixbus : **1×53 s** résiduel + relais par phases `event.*` non bornées → §B-54 ; **vol 95** : disjoncteur tiré ×2ᵉ fois (cryptpad streak=64, API restée vivante, DELETE propre) — credit-agricole : pas de rafale, wedge direct §B-54 | |

| 42 | **Hit-test traverse un overlay `position:fixed`** — clic sur bouton consentement résolu en coords document sur l'ancre pleine-largeur SOUS l'overlay → `navigated` vers la mauvaise cible | `explorer-core`/hit-test | culture.gouv `[V4-main]` vol 62 : clic « Tout accepter » (804,809) → nav `/thematiques/audiovisuel` (ancre 1366×48 à doc-y 2409) — preuve géométrique complète | **CRITIQUE — variante §B-13/§B-17 ; z-order hit-test vs fixed** |

| 43 | **Tiroir off-canvas invisible avale tous les clics** — `<mini-cart class="layer…">` fermé (non peint) mais boîte `[0 0 1366x900]` prioritaire au hit-test : 2/2 clics → `edited` (« livré au DOM — aucune action par défaut »), aucune ancre atteignable | `explorer-core`/hit-test | auchan `[V6-main]` vol 69 : clics « Promos » (295,40) + « Politique de confidentialité » (760,712) interceptés ; transform/état fermé ignoré par le hit-test — variante §B-42 croisée §B-17 | **NON REPRODUIT — diagnostic vol 69 infirmé** : re-vol instrumenté (`hit_path` au debug) même layout (`doc_h` 2261) — le sous-arbre `mini-cart` est correctement exclu (`visibility:hidden` hérité → `unhittable`) : clic (760,712) **traverse** le tiroir → `navigated` sur le `<a>` dessous ; clic (295,40) → `edited` sur la vraie barre haute `html>body>div[2]>header>div` (12 px au-dessus du lien « Promos » posé à y=52). Le vol 69 a inféré l'interception du dump layout seul ; à ré-ouvrir si un porteur frais reproduit avec `hit_path` dans le sous-arbre du layer |

| 44 | **Escalade quadratique du coût par apply** — `repaint ms=363→9 558→199 721`, cascade 46 s/boxtree 55 s/layout 72 s sur la même page ; borne §B-26 non déclenchée, mécanisme distinct (invalidation qui grandit) | `explorer-core`/`layout` | radiofrance `[V6-main]` vol 68 + **`[V7-main]` vol 81 verbatim** (franceculture→radiofrance : cascade 35 s/boxtree 39 s/layout 47 s/repaint 63 s, RAM 4,79 Go +~90 Mo/s, purge complète post-DELETE) ; stabilité OK, perf seule | **OUVERT — perf** ; plafond livré `e34208d` (DOM miroir : insertions refusées >100 k nœuds) — escalade sous le plafond toujours présente (vol 81 verbatim, vol 83 conseil-etat sans `UnmappedKey`, cascade→14,6 s, phases aboutissent) ; reste à profiler |

| 45 | **`doc_h` verrouillé au viewport** — `html,body{height:100%}` + `overflow-x:hidden` → `doc_h`=900, scroll bloqué à 0 alors que le contenu déborde (~13 450 px) | `explorer-layout`/`core` | clubic `[V6-main]` vol 70 — main 11 750 px layouté, frame identique après scroll | **CORRECTIF `34a019b` à re-voler** — `scrollable_overflow_bottom` clippait sous `body{overflow-x:hidden}` (y→auto, R-120) ; l'overflow `html`/`body` se propage désormais au viewport (css-overflow-3 §3.3) : `auto`/`scroll` ne bornent plus l'étendue, `hidden` clippe ; scroll conteneurs internes inchangés — tests repro verts ; **RÉSOLU `[V10-main]` vol 100** — clubic : doc_h=11 940 (vs 900 verrouillé), `scroll_y=1600` effectif, contenu scrollé peint |
| 46 | **Wedge post-apply silencieux CPU 200%** — apply `fin` journalisé (borne §B-26 tirée) puis silence total, navigate jamais retourné, API morte, RAM 2,7 Go | `explorer-core` | frandroid `[V6-main]` vol 70 — distinct §B-33 (fin journalisée) et §B-41 (pas de phases) | **CORRECTIF `2c8cb0b` à re-voler** — `spawn_blocking` post-apply borné 90 s : timeout → rearm + page d'erreur nommée ; repro 1/1 (frandroid) à re-tester ; prudence vol : frandroid, conseil-etat ; **vol 96 `[V10-main]` : NON REPRODUIT** — frandroid navigate 32,3 s, doc_h=17 348, DELETE effectif, API vivante ; **vol 97 : NON REPRODUIT ×2** (navigate 58,6 s, DELETE 0,33 s) — favorable |

| 47 | **Mojibake UTF-8→Latin-1 des champs JSON API** — `title` de `/api/tabs` garbled (`idÃ©es`, `actualitÃ©`), suffixe dédup `/console` `Ã—3` — DOM/rendu corrects, titre clapote correct↔mojibake selon cycle re-parse | `explorer-core`/`api` | santemagazine/doctissimo/femmeactuelle `[V6-main]` vol 71 | **CORRECTIF ×2 à re-voler** — (a) `e268ec7` : `/api/tabs` relit `extract_title(dom)` vivant — le `t.title` stocké peut dévier du `<title>` commis au cycle re-parse/miroir ; (b) `8ddd26d` : `charset=utf-8` annoncé sur les réponses JSON (`application/json` nu laissait un client replier Latin-1 — `×`→`Ã—` ne pouvait être corrompu côté serveur) ; **RÉSOLU `[V9-main]` vol 94** — santemagazine : `title` API UTF-8 propre (« Santé Magazine… », zéro mojibake) |

| 48 | **Micro-script en boucle ≥60 s** — script 285 o bouclant jusqu'au kill disjoncteur (`eval ms=60002 ok=false`), scripts suivants sautés ; kill absent de `/console` (« pas de journal » mensonger) | `explorer-jscore` | kayak `[V6-main]` vol 72 — porteur historique wedge vol 24 : nav retourné 65 s, modale erreur propre | **FIX LIVRÉ (Étape 146)** — le « pas de journal » mensonger : le recorder `/console` essayait `try_lock` **une fois** au démarrage → échec silencieux sous contention → aucun recorder pour toute la session ; désormais `lock().await` dans la tâche + `Lagged` warné + verrou `recorder_vit_meme_sous_verrou`. Les sauts de scripts post-disjoncteur sont le comportement voulu (cooldown 10 s) et étaient déjà rapportés en `PipelineError` — désormais effectivement visibles. Re-vol kayak à confirmer |

| 49 | **Wrapper racine expulsé hors viewport** — `.paris-main-wrapper` layouté `[1366 0 0×9927]` : largeur 0 au bord droit, tout descendant x≥1366 → frame blanche malgré DOM/layout/images complets (doc_h=9 927) | `explorer-layout`/`css` | paris.fr `[V6-main]` vol 73 — motif suspect : item flex row compressé à 0 ; famille §B-35, symptôme §B-7 | **CORRECTIF `741f810` à re-voler** — mesure locale positive (367 boites, 0 expulse, doc 800x13037) ; verdict visuel 1366 par equipe de vol requis ; **RÉSOLU `[V10-main]` vol 101** — paris.fr : wrapper dans viewport, page peinte (vs frame blanche vol 73) ; résidu overlay menu pixelisé peint ouvert → §B-17 |
| 50 | **Re-parse post-apply : DOM jamais réattaché** — apply complet (`repaint` journalisé, borne §B-26 tirée) puis rejet `UnmappedKey` → re-parse → `dom` 404 / `frame` 204 / `dom_nodes=0` >7 min, flag « paint en vol » bloqué | `explorer-css` (parser) | bordeaux.fr `[V6-main]` vol 73 ; lumiplan vol 76 (oscillant) | **RÉSOLU `[V7-main]` vol 79 (×2 : bordeaux + lumiplan)** — cause racine nommée : un `style="transform: translateX(0px;…"` muté par JS laissait une `(` jamais refermée ; la boucle de valeur de `declaration()` attendait la `)` à EOF sans consommer → boucle infinie 1 cœur dans la cascade du re-parse (bissection jalons `apply:` : `imgs` passé, `cascade` jamais). Fix : EOF clos la déclaration même parens ouvertes (spec) + test `parenthese_non_fermee_eof_termine`. **Re-vol bordeaux validé** : rejet `UnmappedKey` → re-parse → `cascade ms=2324` → `layout` → commit, `/dom` 200 `/frame` 200 |

| 51 | **Évals `ok=false ms=0` silencieuses** — même script ré-évalué après chaque re-parse `UnmappedKey`, zéro trace `/console` | `explorer-jscore`/glue | messenger vol 78, **airfrance vol 87 (4 bundles 467-614 Ko silencieux/nav → coquille vide, meilleur vol SPA Angular sinon ~80 %)**, sosh (863 Ko→coquille vide) + edf (605 Ko) vol 85 `[V7]`** + **élargi aux évals initiales `ms>0`** (challenge Incapsula micromania : 103 Ko `ok=false ms=703` sans trace, vol 81) — trou d'observabilité §B-4-adjacent | **OUVERT — toujours présent V7** (airfrance vol 87 ×4, sosh/edf vol 85, plateforme RTL vol 82 : bundle 4,1 Mo `ok=false` silencieux ×2, js_heap=0) ; mécanisme nommé vol 79 : erreurs éval de boot émises *avant* `Navigated` → vidées au commit (api/lib.rs) ; correctifs livrés : purge déplacée sur `Navigating` émis au départ (`1e5662f`) + `emit_pipeline_error` ré-émis en direct (`3ff1da6` — le wrapper dedup s'appelait lui-même) ; **vol 95 `[V9-main]` : non reproduit ×2** — airfrance : 4 évals ok=false ms=104-670 **nommés** à `/console` (vs ×4 silencieux vol 87 — le bras « purge avant Navigated » semble fermé) ; micromania : éval Incapsula 103 Ko ok=false ms=666 **tracé** (`TypeError: push is not a function` — vs silence vol 81) — **en décroissance** ; **vol 98 `[V10-main]` : variante « silence total »** — twitch : coquille blanche, 246 nœuds, js_heap 7,3 Mo, **0 entrée console** (bundles jamais évalués ou échec invisible — à instrumenter : compter évals reçus vs scripts du DOM) ; **vol 101 `[V10-main]` post-É155 : bras « silence » RÉSOLU ×3** — sosh (863 Ko `ok=false` nommée + src verbatim vs silence vol 85), edf (4× échecs nommés, page ~85 %), twitch (WARN `evals_sans_mutation evals=44` — §B-4(c) détecté ; WARN non routée au ring `/console` — trou résiduel) ; défaut fonctionnel restant = §B-4/§B-14, pas §B-51 |

| 52 | **Rafale `UnmappedKey` continue** — mutations DOM périodiques → re-parse ~1×/5 s (56×/2,5 min) sans wedge | `explorer-core` | bouygues-immobilier `[V7-main]` vol 84 : contenu, API vivante — famille §B-41 atténuée | **RÉSOLU `[V8-main]` vol 88** — WARN « série UnmappedKey — voie journal sautée » tirée sur bouygues (bornée à 8), page rendue (`vol88_bouygues.png`) |

| 53 | **Popin consentement déplacée** — `#popin_tc_privacy` edf `left:683px` layoutée x=1046 → boutons consentement hors écran, page verrouillée (`body{position:fixed}`) | `explorer-layout` | edf `[V7-main]` vol 85 — **deux bugs empilés** : (a) `margin:auto` de la boîte `fixed` re-centrée par le §10.3.3 du flux **après** ancrage `left` → `683+(1366-640)/2=1046` exact (le « translate inversé » aurait donné 1003 sur 640 px — écarté) ; (b) `translate3d` non reconnu → `transform` entier ignoré | **RÉSOLU `[V8-main]` vol 88** — popin `[683 450 640x765]` entièrement dans le viewport, boutons « Accepter »/« Continuer sans accepter » peints et atteignables (`captures/vol88_edf.png`) |

| 54 | **Wedge navigate : phases `event.*` non bornées → verrou session global** — boucle d'événements dont chaque cycle = `poll_net` 15-62 s + `dispatch` ~5,5 s + `mirror_sync` ~10-13 s (~30-80 s/cycle) → `navigate` jamais retourné, **API entière morte** (tous endpoints timeout, DELETE impossible), livelock pas deadlock (log actif), RAM dérive lente ~0,6 Mo/s ; le disjoncteur §B-41 est inopérant (phases *longues*, pas fréquence de pompes) | `explorer-core` session/pump | flixbus `[V9-main]` vol 93 : `poll_net` max 61,8 s / `dispatch` 4,7-6,4 s / `mirror_sync` ≤13,3 s, ~18 cycles en 13 min, API morte >6 min — preuve `logs/daemons/_vol93_daemon.log` 75 800+ | **OUVERT — terrain V9** (proposée §B-55 au rapport, numérotée 54) ; bras adjacent §B-41 : `commit.drains` réduit à 1×53 s mais le wedge migre vers `event.*` ; **vol 95 : REPRODUIT 2ᵉ site — credit-agricole** : `event.poll_net us=110 783 422` (110,8 s), navigate jamais retourné >150 s, `/api/tabs`+`DELETE` sans réponse, livelock (log actif), arrêt forcé PID — **tout site à flux net continu est exposé** ; **vol 96 `[V10-main]` post-É151 : RÉSOLU credit-agricole** — navigate retourné 67,9 s, `poll_net` max 3,49 s, `livraisons différées (B-54) deferred=59` ×13 (borne tirée terrain), DELETE 0,49 s, API vivante ; **PARTIEL flixbus** — navigate retourné 95 s (`poll_net` max 17,8 s) mais **wedge post-DELETE : pompe orpheline, API morte >30 s, livelock** (recoupe §B-30) — le bras « navigate » est borné ; **bras « pompe post-DELETE » : CORRECTIF Étape 154 `ee6008fa` à re-voler** (moteur orphelin détruit hors verrou, `dyn_tasks` purgés au close) ; **vol 100 `[V10-main]` post-É154 : REPRODUIT** — flixbus : navigate retourné 24,6 s puis **wedge pendant vie de l'onglet** (`event.poll_net` ×51, `mirror_sync` 9,9 s, `dispatch` 5 s, defer B-54 tiré mais phases longues reprennent), API morte >60 s incl. DELETE, livelock → arrêt forcé PID ; bras É154 **non mesurable** (wedge pré-DELETE) — reste à borner le *contenu* des phases `event.*`, pas seulement la queue ; **CORRECTIF Étape 157 à re-voler** — premier générateur de boucle non bornée nommé : `drain_anim_births` dispatchait chaque naissance `animationstart` en série (~15 s dispatch+miroir chacun ×N sous verrou) ; le drain partage désormais `event_tail_budget` — ≥1 naissance/passe (progression), reste différé dans la deque — verrou `anim_births_drain_bounded_by_event_budget` ; si le wedge persiste au re-vol, sonder les autres boucles de dispatch (deferred_inputs, lifecycle replays) ; **vol 104 post-É157 : bras post-DELETE RÉSOLU** — DELETE 204 en 5,5 s, API pleine ×3, purge 1,31→390 Mo (É154+157 validés) ; **bras vie-onglet EN DÉCROISSANCE** — starvation partielle (probes alternées 200/000 vs morte totale ×2 vols), `drain_anim_births` borné tiré terrain ; reste `poll_net` 18,5 s/`dispatch` 5,2 s/`mirror_sync` 12 s |

| 55 | **Panic `resvg` (dépendance) sur SVG hostile** — `resvg-0.45.1/filter/composite.rs:25` `assertion failed: src1.width == src2.width` pendant un apply : asset SVG externe panique la dep de raster — rattrapé au niveau thread, process vivant, apply continue | `explorer-render` (dep resvg) | ici.fr `[V9-main]` vol 94 (tab 1, 19:15:24 — nav francebleu) — preuve `logs/daemons/_vol94_daemon.log` | **CORRECTIF R-130 à re-voler** — `catch_unwind` autour du parse `usvg` et du `render` resvg dans `decode_svg_raster` (couvre `<img src=*.svg>` et `<svg>` inline) — panic de la dep confiné, dégradation `None` = repli alt ; verrou `svg_filtres_regions_divergentes_ne_panique_pas` reproduit l'assert exact (`filter=` à deux régions divergentes, `feComposite arithmetic`) ; **RÉSOLU `[V10-main]` vol 97** — ici.fr : panic `composite.rs:25` imprimé puis pompe reprend us=149, API vivante, DELETE propre |
| 56 | **Moteur regex : patterns valides du web réel rejetés** — `SyntaxError: invalid regex` sur des expressions standards → bundles en échec | `explorer-jscore` (regex) | frandroid `[V10-main]` vol 96 : `/^[\w:-]{32,512}$/` rejetée (`Error compiling regex` — quantificateur `{n,m}` sur classe ?) ; airfrance `[V9-main]` vol 95 : `\p{Pat_Syn}`/`\p{Pat_WS}` (classes Unicode) + notation octale `\141\164` — SyntaxError nommées, dégradation propre | **CORRECTIF `JS-C177` à re-voler** — racine nommée : `{n,m}` sur grosse classe Unicode déroule ~`m` copies d'états NFA, `delegate_size_limit` 10 Mio défaut dépassé (~30 Mio mesurés) → borne 64 Mio (`{…,≤~1024}` sur `\w`) ; `\p{Pat_Syn}`/`\p{Pat_WS}` déjà fonctionnels (backend les connaît), `\141` octal conforme Annex B — verrous `b56_probe.rs` ; **vol 98 `[V10-main]` : PARTIEL** — bras `{n,m}` RÉSOLU terrain (frandroid 0 rejet) ; bras **PERSISTANT** : classe **niée** `[^\p{Pat_Syn}\p{Pat_WS}]` + octal en contexte rejetés (airfrance ×2) ; **CORRECTIF JS-C180 `897f523e` à re-voler** — racine nommée : hors `u`, `escape` rendait `\p` nu et `{Nom}` devenait des littéraux échappés → backend lisait `\p\x{7b}…` → rejet ; Annex B : `\p` sloppy en classe = identité littérale `p` (Chrome : set négatif `{p,{,P,a,t,…}}`), range `[\p-z]`=`p`-`z`, `u` inchangé ; **vol 101 `[V10-main]` : PARTIEL** — bras `[^\p{Pat_Syn}\p{Pat_WS}]` **fermé** (airfrance : classe niée acceptée) ; bras **octal `\141`/`\100` CORRECTIF `JS-C182` à re-voler** — racines nommées ×2 : (a) `\N` chiffre en classe passait par `escape`/`decimal_escape` → backref `\1(?:00)` dans la classe → « Invalid character class » ; intercept `class_octal` (jamais backref en classe — Annex B) ; (b) hors classe, le « plus long préfixe » coupait `\141` en `\1`+`41` (matchait « 41 ») — Annex B lit le nombre **entier** → 141 > groupes → octale `\x{61}` ; verrous `b56c_probe.rs` (motif jscrambler verbatim compilé + sémantique octale/backref/forward) ; **vol 105 `[V10-main]` : RÉSOLU (3/3 bras)** — airfrance 0 `invalid regex` (octales Annex B JS-C182 validé) ; reste montée SPA (§B-4/§B-14) ; **vol 129 : RÉSIDUEL nouveau motif** — lookbehind conditionnel `(?<=-RC\.\d+)` (epic) |
| 57 | **`expression too deeply nested` sur polyfill cdnjs réel** — `polyfill.min.js?version=4.8.0&flags=gated&features=…` (multi-features) dépasse la borne de profondeur → la détection de compat meurt → mur « navigateur incompatible » | `explorer-jscore` (parser) | manomano `[V10-main]` vol 96 : redirect `/navigateur-incompatible`, doc_h=0 — famille JS-C106 mais bundle polyfill.io réel | **CORRECTIF `JS-C178` à re-voler** — racine nommée : le parseur produit `a+b+…`, `a.b.…`, `a[k]…`, `f()()…` en boucle itérative (borne `nest` ~1) mais l'AST est profonde à gauche — `eval_expr` comptait chaque maillon dans `expr_depth` (4096) ; trampoline `eval_spine` dépile l'épine sur pile explicite (court-circuits/ordre/`this` préservé, right-deep toujours rejeté) — verrous `b57_probe.rs` ; **vol 97 : 2ᵉ site** — caniuse `bundle.js` ×N (constat pré-JS-C178) ; **vol 100 `[V10-main]` post-JS-C178 : PERSISTANT ×2** — manomano `deeply nested` **at ToObject** (récursion autre chemin que l'épine `eval_expr`), caniuse `bundle.js` (page néanmoins rendue SSR) ; bras §B-58 confirmé par sonde session voisine ; **CORRECTIF `JS-C181` à re-voler** — racine nommée (bundle cdnjs 1,26 Mo évalué in-process) : `Fun::Native` sans `length` → gate polyfill `0===self.Symbol.length` échouait → faux Symbol installé → `Object.prototype.toString` polyfillé lisait le faux symbole-objet en clé → `toString` → récursion **infinie** `e↔Get` ; `native_fn` pose `length:0` own `{w:false,e:false,c:true}` → bundle complet `OK` ; **vol 103 `[V10-main]` : PARTIEL** — manomano `deeply nested` **disparu** (mur migré §B-14 didomi) ; caniuse `bundle.js` toujours rejeté (chemin hors `fn.length` — dernier bras) ; **CORRECTIF `JS-C183` à re-voler** — racine nommée (bundle caniuse 331 Ko évalué in-process, getter MobX `qr` à 2:51736 tracé ×256) : `Reflect.ownKeys` rendait les clés-symboles comme **strings sentinelles** `\u{f130}sym{N}` au lieu des `Symbol` → `Set(ownKeys).delete(z)` manquait la clé → `make_("sym{N}")` posait un accesseur `qr` **sur le slot du symbole `z`** → `this[z]` → getter auto-réentrant → ∞ ; `ownKeys` rend désormais les `Symbol` (`sym_index` + live check), `Object.keys`/`entries`/`values`/`for-in` excluent les sentinelles énumérables — bundle complet : `RangeError` disparu ; **vol 108 `[V10-main]` post-JS-C183 : RÉSOLU (3/3 bras)** — caniuse 0 `deeply nested`, page ~95 % ; anomalie fermée (bras : épine JS-C178, fn.length JS-C181, ownKeys JS-C183) ; **vol 128 : RÉSIDUEL** — `deeply nested` sur SG public-dca.js (chemin nouveau post-JS-C178/181) ; **vol 136 : RÉSIDUEL ×3** — `deeply nested` yves-rocher p-DEDQ |
| 58 | **Texte peint en miroir / retourné 180°** — run de texte de la hero inversé (backdrop `transform` non composé ?) | `explorer-layout` | letterboxd `[V9-main]` vol 95 + **`[V10-main]` vol 98** : « Wildwood (2026) » peint retourné, signature stable ×2 binaires (`captures/vol98_letterboxd.png`) ; recoupe l'écart raster/layout vol 93 (mégamenu x=-344) | **CORRECTIF `R-131` à re-voler** — racine nommée (repro in-process) : `wrap_anonymous`/feuilles `Text` clonent le style parent **entier** → `box_transform` repoussait la matrice à chaque niveau d'enveloppe, texte peint sous M²/M³ (`rotate(45)`→90°, `rotate(90)`→180°, `rotate(180)`/`scaleX(-1)`→identité — mesuré) ; letterboxd `rotate(-90)` lisait `-180` = miroir. CSS Transforms : seule une boîte d'élément est transformable — `box_transform` ignore `AnonymousBlock`/`Text` (peinture + hit-test, source unique) ; verrous `b58_probe.rs` (profondeur PushTransform = 1 : flux, absolu, motif `<p transform><a>`) ; **RÉSOLU `[V10-main]` vol 102** — letterboxd : « Wildwood » vertical net, miroir disparu |

| 59 | **Fuite mémoire massive à la nav github** — RAM 430→8 079 Mo en ~30 s pendant `navigate` (avant tout apply) ; croissance ~250 Mo/s non bornée → crash station (vol 106 : daemon tué par watchdog `vol_memwatch.ps1` au seuil 5 Go, signature §B-26-like en amont) | `explorer-core`/pompe | github.com `[V10-main]` vol 106 — preuve `logs/daemons/_vol106_watch.log` | **OUVERT** — watchdog posé, racine à nommer (fuite pendant nav — pompe/miroir ?) ; **vol 108 : reproduit ×2** — 500→9 648 Mo en ~15 s (~600 Mo/s), signature stable, watchdog killé ; **vol 125 : 2ᵉ site — GÉNÉRALISÉ** — lindependant 486→5 996 Mo en ~45 s ; fuite = churn pompe post-nav, pas la cible |

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
| J2 | `depth_limit` parser/AST — le seul crash process (alpinejs) ; C134 a borné drop AST + walk JSON, pas la descente syntaxique | §B-3 | **correctif livré ×2** — JS-C166 `deleg_hop` + `stack_guard` hors chemin (vol 46 : overflow à l'**exécution**) ; JS-C167 `adf9273` tueur réel : `loose_eq` × Symbol dans `with(proxy)`, `[[Call]]` unifié, `super()` natif — **RÉSOLU `[V9-main]` vol 93** (alpinejs entièrement fonctionnel) |
| J3 | Bras MapLoader `import()` : `fetch_module_sources` ne collecte pas tout (`max_modules`, refus, spec non extrait) + `data:` refusé en module | §B-5/6 | partiel — `data:` livré (JS-C131), cap 64→256 + warn `tronqué` (`2b61012`) ; reste refus/spec non extrait |
| J4 | Heap budget : capper `heap memory limit exceeded` à 1 occurrence + mesurer le seuil réel (marmiton, bookshop beacon) | §B-12 | **livré** — JS-C170 `heap_dead` latch : une signature stable, répétitions tues, `set_mem_limit` réarme + dedup `emit_pipeline_error` (`83916a6`) ; **vol 95 : dedup inopérante terrain** — ×9 identiques sur bookshop ; **vol 96 `[V10-main]` : confirmé ×2 sites** (bookshop ×7, frandroid ×6+ — 3ᵉ site) ; **Étape 152** : racine trouvée — piles JS distinctes par site d'appel cassaient l'égalité du dédup → `heap_dead_reported` borne à 1 occurrence/moteur, à re-voler ; **vol 100 : ×5 résiduel** — chemins hors porte (vague détachée `async`, livraisons réseau/WS/SSE, iframe dyn) → **Étape 156** : flag `Arc<AtomicBool>` partagé `SubJob`↔session, à re-voler |
| J5 | Matrice APIs résiduelles : `instanceof Element` trop strict, `Function.name` classes minifiées, `document` of Null (dw), token `debugger` tuant un module, `Illegal component name ""` | §B-14 | partiel→élargi : audité — `debugger` no-op (parser 622), `document.contains` (ratp), `instanceof Element` chaîne proto (C159), `Function.name`+`bound`, `Response.json` proto+statique, `replaceChild`, `customElements.define` (C68), `navigator.geolocation`, `getHighEntropyValues` — restent les cascades `document` of Null/`Illegal component name` à nommer au vol |
| J6 | Spread/rest `...` « outside a call or array literal » — position non couverte par C146 | §B-9 | **non reproduit en-process** — JS-C173 : sonde sur le bundle réel BBC `_app` vol 34 (fixture verbatim) : eval top-level + 8 rounds microtasks/timers + drain rejets = propre ; les positions `...` hors call/array qui échouent encore (`yield ...x`, `throw ...x`) sont spec-correctes ; l'erreur terrain venait d'un rejet de promesse tardif — à confirmer au re-vol de la cible |
| J7 | `document.write`/async : ordonnancement post-commit vs ordre de parse (tumblr) | §B-19 | chantier profond documenté |
| J8 | Boucle applicative non bornée (`r is not defined` ×512 — intervalle retry en rafale) | §B-21 | **livré — partiel terrain vol 94** — `TIMER_FAIL_CAP=8` : intervalle mort-né désarmé + isolation par callback + dédup visuel ; **la source genius échappe** : rejets de promesses (×255 nommés), hors chemin throws d'intervalle — bras complémentaire à borner |
| J9 | `visibility:hidden` jamais levé (geeksforgeeks — hydrate JS) | §B-7 | **CORRECTIF JS-C174 — à re-voler** : `style.cssText` posé (get+set, remplace toute la liste inline — avant : pseudo-propriété `css-text`, `visibility` survivait) ; si le re-vol persiste : hydrate mort en amont (console vol 36 vide — à nommer) |
| J10 | Proxy : traps reçoivent `PropertyKey` symbol (immer/mastodon `isDraft`) | §B-9/19 | **RÉSOLU** — JS-C165 : `prop_trap_key` démange `sym_index`, re-vol mastodon : `ownKeys` TypeError disparu, app atteint `createStore` |

### NAV — `explorer-core` / `net` / `daemon` / `api`

| # | Tâche | Source | État |
|---|---|---|---|
| N1 | **Re-vol cdiscount** — valider `max_auto_navs` (É144) + sentinel death daemon | §B-2 | **rempli vol 36** — `max_auto_navs` validé terrain (0 nav auto, disjoncteur OK, B-2 RÉSOLU) ; sentinelle arrêt propre présente (daemon `main.rs`) — si « sentinel death » visait une détection externe de mort du daemon, reste à préciser |
| N2 | Timeouts « jprobe » 25 s non qualifiés (curl.se, apple.dev, phoronix, markojs, pixijs, konvajs) — qualifier fetch/script/eval | §D-4 | **instrumenté (Étape 150)** — `NavJob::run` trace `en-têtes→corps` / `corps→arbre` / `arbre→métadonnées` (ms+octets+statut) et `SubJob::run` horodate `vague reçue`/`@import`/`fonds`/`fontes`/`modules` ; le dernier marqueur avant le timeout nomme la phase — verdict au prochain jprobe |
| N3 | Watchdog/budget **post-apply** transversal (RAM 2,9 Go + saturation cœur) — couvre ce que le disjoncteur évals ne voit pas | §B-1 | dépend de J1 |
| N4 | Morts silencieuses résiduelles : MapLoader bras + drains modules (pinterest, twitch, windy/flightradar importmap) | §B-4 | en décroissance — bras (d) data.gouv **RÉSOLU `[V9-main]` vol 93** (modulepreload É148 validé) ; tracé à compléter |
| N5 | `document.write` ordonnancé post-commit (avec J7) | §B-19 | chantier profond |
| N6 | `data-src` marmiton gris en V3 — re-valider sur binaire post-fix | §B-20 | vol 95 : repli `840a33c` seul suffit à peindre (images au scroll) ; verdict É149 (`data-srcset`/`max_images 256`) → vol 96 binaire frais |
| N7 | **Bornes phases `event.*`** (poll_net/dispatch/mirror_sync multi-secondes) — wedge `navigate` → verrou session global → API morte ; disjoncteur §B-41 inopérant (phases longues, pas fréquence) | §B-54 | **VALIDÉ vol 96 `[V10-main]` — bras navigate RÉSOLU** (credit-agricole : `poll_net` max 3,49 s vs 110,8 s, `deferred=59` ×13, DELETE propre) ; **bras post-DELETE : CORRECTIF Étape 154 `ee6008fa` à re-voler** — pompe orpheline post-DELETE : moteur détaché détruit sur fil dédié hors verrou (GC multi-Go ne tient plus le verrou session) + `dyn_tasks`/streaks purgés à `close_tab` ; **vol 100 : non mesurable** — wedge flixbus pré-DELETE (vie onglet, §B-54), DELETE propres ×6 hors wedge ; à re-tester sur flixbus DELETE post-wedge, manomano |

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
| T2 | `evals_sans_mutation` compteur + promotion WARN — morts silencieuses auto-détectées | §B-4/obs | **livré** — compteur + WARN ≥8 évals sans mutation DOM (net.rs ~236) |
| T3 | Heap JS gelé ~50 Mio/page — politique de trim (bfcache) | §B-perf | **livré** (`e0985cb`) — `JsEngine::gc` invoqué au gel bfcache + caches bitmap/display-list purgés |
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
  les cas intermédiaires ; `NON REPRODUIT` pour un diagnostic
  infirmé par re-vol instrumenté ; `CORRECTIF <sha> à re-voler`
  pour un fix livré en attente de terrain.
- **Tag de binaire** : toute anomalie porte sa version
  (`[V4-main]`/`[V5-main]`/`[V6-main]`/`[V7-main]`) — un constat
  pré-V3 n'est jamais présumé régression.
- **Cohérence** : §D = priorités (ordre), §E-bis = propriétaires
  (pistes NAV-JS / NAV / PIPE / Terrain), §B = catalogue — les
  IDs §B font le lien, pas de duplication.
