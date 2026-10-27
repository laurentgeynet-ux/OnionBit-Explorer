<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 36 — Re-validation des anomalies §B/§E-bis

- **Date** : 2026-10-27
- **Binaire** : `C:/obe-vol-target/release/explorer-daemon.exe` — tag
  **`[V4-main]`** (main ~57 fixes mergés, build 08:35)
- **Viewport** : 1366×900 (`EXPLORER_VIEWPORT`) — port 9421 — daemon PID 14552
- **Protocole** : `docs/vol-agent.md` §8 — chaque constat porte `crate`
  + ID §B ; statuts canoniques §G.
- **Mission** : re-valider §B-2/N1 (cdiscount), §B-20/N6 (marmiton
  `data-src`), §B-7/J9 (geeksforgeeks `visibility:hidden`),
  §B-14/JS-C162 (openstreetmap `call of Undefined`).

## Verdicts en une table

| Cible | ID §B | Tâche | Verdict `[V4-main]` |
|---|---|---|---|
| cdiscount.com | §B-2 boucle navs auto | N1 | **RÉSOLU** — 0 nav auto en ~5 min |
| marmiton.org | §B-20 images `data-src` | N6 | **TOUJOURS PRÉSENT** |
| geeksforgeeks.org | §B-7 `visibility:hidden` | J9 | **TOUJOURS PRÉSENT** |
| openstreetmap.org | §B-14 `call` of Undefined | J5/JS-C162 | **DÉGRADÉ EN MIEUX** — erreur disparue, tuiles toujours absentes (silencieux) |

Constats secondaires : §B-5 `module non chargé` **toujours présent**
(marmiton) · §B-12 `heap memory limit exceeded` **non reproduit**
(marmiton, progrès) · §B-9/§B-11 `Invalid scheme` en
`unhandledrejection` (cdiscount) · §B-4 morts silencieuses : deux cas
frais (GFG scripts `defer`, OSM tuiles) · §B-18 artefact header
fixe au scroll (marmiton) · spam `sync_profile` continu multi-onglets
(note perf/observabilité).

---

## 1. Cdiscount — https://www.cdiscount.com — §B-2 **RÉSOLU**

- **Titre** : « Cdiscount : des prix bas qui ont de la voix ! »
- **doc_h** : 7 758,71 px (non clampé) — `can_back=false`,
  URL stable `https://www.cdiscount.com/` pendant toute l'observation
  (~5 min, au-delà de la timeboxe 3 min).
- **Navigation** : `POST navigate` retourné proprement en ~18 s.
  **Aucune navigation automatique** : 0 occurrence — la boucle
  « 3 812 navs / 20 001 streams / 16 min » du binaire pré-fix ne
  reprend pas. Le budget `max_auto_navs` (étape 144) n'a même pas eu
  à s'armer : aucune re-navigation n'a été initiée.
- **Disjoncteur wall-clock (J1/JS-C164) — validé en conditions
  réelles** : un script de **507 034 octets** (socle Akamai/anti-bot
  présumé) a dépassé la deadline :
  `eval fin tab=1 ms=10218 ok=false` → WARN `moteur figé —
  disjoncteur ouvert, appels JS court-circuités` à 07:37:03.
  L'API est restée pleinement responsive pendant et après
  (`GET /api/tabs` < 250 ms). C'est probablement *ce* mécanisme qui
  étrangle la boucle historique (le script tué portait la machine à
  navs).
- **Frame** (`captures/vol36_cdiscount.png`) : la **vraie page
  d'accueil** s'affiche — plus d'anti-bot « supported-browsers » :
  omnibox recherche, nav « Promos / Noël / Voyages / Forfait Mobile
  / Paiement 10x / Reconditionné / Programme de fidélité », bandeau
  promo « -10€ dès 50€ … code HELLO10 », heros **avec images
  peintes** (« L'appel des bons plans » iPhone, « Sélection gaming
  -50€ » laptop TUF, « Call of Duty Modern Warfare III »).
- **Console** (verbatim, 5 groupes dédupliqués) — `explorer-jscore`
  → §B-9/§B-11 (résolution d'URL résiduelle, famille JS-C149
  « partiel ») :

```
unhandledrejection: TypeError: Invalid scheme
    at <anonymous> (./getElementToHydrate-K9Znta0z.js:8:35887)
    … (×5 variantes via header-BpddzHPI.js, pa-D1Yimgle.js,
       footer-u9-Nf4ZB.js, global-AUbPqpZA.js — chemins relatifs
       « ./ » résolus en scheme invalide)
```

- **Classification** : anomalie §B-2 **RÉSOLU** ; reste un mur JS
  `Invalid scheme` (hydratation incomplète) → §B-9/§B-11.

## 2. Marmiton — https://www.marmiton.org — §B-20 **TOUJOURS PRÉSENT**

- **Titre** : « Marmiton : 78430 recettes de cuisine commentées… »
- **doc_h** : 11 192,96 px — 65 évals JS, dont 4 `ok=false`.
- **Constats mesurés** (`captures/vol36_marmiton.png`,
  `vol36_marmiton_2.png` à +10 s, `vol36_marmiton_scroll.png` à
  `scroll_y=900`) :
  - **Images `data-src` jamais peintes** — identique au vol 34 :
    hero = bloc gris pleine largeur, vignettes actus = blocs gris.
    DOM : **215 occurrences `data-src`** avec URL réelles
    (`assets.afcdn.com/recipe/…/166465_w46.webp`…) et `<img>` **sans
    attribut `src`** (4 `src=` seulement : placeholders/template
    `<%=…%>` + didomi). Le repli `data-src` livré (840a33c) ne
    produit aucune requête ni pixel sur cette cible — `explorer-core`
    → §B-20 **TOUJOURS PRÉSENT**.
  - **Artefact layout au scroll** (`vol36_marmiton_scroll.png`) :
    la barre fixe du haut apparaît **tronquée** — hamburger visible
    mais le texte de nav rogné en « Qu'… » à gauche et « …ette » à
    droite, comme si le fond sticky était peint hors de sa boîte
    clip — `explorer-render`/`layout` → §B-18 (famille « sticky
    repaint », cf. accuweather).
- **Console** (verbatim, 3 entrées — plus de flood) :

```
exception js : TypeError: cannot load module `app`: module non chargé : https://statics.marmiton.fr/assets/app-E0uZPb4.js — module «inline:18»
exception js : TypeError: cannot load module `js/front/user.js`: module non chargé : https://statics.marmiton.fr/assets/js/front/user-q3n638M.js — module «https://statics.marmiton.fr/assets/Organism/Header/header-user-FBkN7XD.js»
exception js : ReferenceError: GeolocationPosition is not defined — module «https://unpkg.com/mealz-components@0.7/dist/store-locator/store-locator.min.js»
```

- **Croisements catalogue** :
  - §B-5 (bras MapLoader `import()`/`module non chargé`, J3) —
    **TOUJOURS PRÉSENT** : marmiton cité dans la ligne §B-5 « restes
    à re-voler », les deux erreurs `module non chargé` persistent.
  - §B-14 (matrice APIs DOM) — `GeolocationPosition is not defined`
    = entrée nouvelle de la matrice (navigator.geolocation non
    peuplé).
  - §B-12 (`heap memory limit exceeded`) — **NON REPRODUIT** : les
    3 RangeError du vol 34 ont disparu de la console ; heap tenu
    sous le seuil sur cette cible. Recul favorable, à re-mesurer sur
    bookshop avant clôture.

## 3. GeeksforGeeks — https://www.geeksforgeeks.org — §B-7 **TOUJOURS PRÉSENT**

- **Titre** : « GeeksforGeeks | Your All-in-One Learning Portal »
- **doc_h** : 4 136,42 px — DOM 155 Ko, layout 46 Ko : arbre complet.
- **Frame** (`captures/vol36_geeksforgeeks.png` +
  `vol36_geeksforgeeks_2.png` à +10 s — byte-identiques) :
  **page blanche totale**, à l'identique du vol 30.
- **Preuve DOM** : le wrapper hydratation est intact ~25 s après nav —
  `<div id="__next">` → `<div
  style="visibility:hidden;background-color:#ffffff;min-height:100vh">`
  (dom.txt ligne 174). Le JS chargé de lever `visibility:hidden`
  n'a jamais abouti.
- **Console : « pas de journal » — zéro erreur** — recoupement
  §B-4 : 23 `<script>` dans le DOM (14 `defer`, chunks Next.js
  `webpack`/`framework`/`main`/`_app`/`index`), mais seulement
  **16 évals** loguées, toutes `ok=true` — les scripts `defer` du
  boot Next semblent drainés trop tard ou pas du tout, **sans
  aucune erreur tracée** (famille §B-19 ordonnancement post-commit +
  §B-4 mort silencieux). `explorer-jscore`/glue → §B-7
  **TOUJOURS PRÉSENT**.

## 4. OpenStreetMap — https://www.openstreetmap.org — §B-14 **DÉGRADÉ EN MIEUX**

- **Titre** : « OpenStreetMap » — doc_h 477 px.
- **Console : « pas de journal »** — **`call` of Undefined
  disparu** : JS-C162 (`Element.prototype.querySelector*`/
  `getElementsBy*`, cause racine Bootstrap) confirmé au re-vol.
  3 évals seulement (bundle 1 529 756 octets exécuté `ok=true`
  en 848 ms + 2 petits scripts), 0 erreur.
- **Frame** (`captures/vol36_osm.png`, `vol36_osm_2.png` à +20 s) :
  chrome OSM complet — header, recherche, contrôles Leaflet peints
  (zoom +/−, layers, attribution, échelle), bannière « State of the
  Map LATAM 2026 » image incluse ; **zone carte = gris vide**.
- **Preuve DOM** : les panes Leaflet existent
  (`leaflet-pane leaflet-tile-pane`, `leaflet-map-pane`…) mais
  **0 élément `leaflet-tile`** et **0 URL `tile.openstreetmap.org`**
  dans le DOM — Leaflet s'instancie entièrement, puis meurt
  **silencieusement** avant la création des tuiles (aucune erreur
  console → §B-4 résiduel ; hypothèses inchangées : Worker de tuiles
  ou cascade post-init non tracée).
- **Note télémétrie** : WARN `journal DOM rejeté → repli re-parse
  (ADR-0025) why=UnmappedKey` (tab=4, 07:42:43) — repli gracieux
  fonctionnel, à garder sous observation.
- **Classification** : §B-14 (entrée « `call` of Undefined
  Bootstrap ») **DÉGRADÉ EN MIEUX** — la cause racine JS-C162 est
  éteinte ; le site reste non fonctionnel (tuiles) par un bras
  silencieux distinct.

## 5. Observation transverse — pompe `sync_profile` permanente

- Le journal daemon émet **en continu** des cycles
  `sync_profile: phase sync … phase="commit.*"/"pump.*"` à ~4 Hz
  **par onglet** et ce pour les 4 onglets simultanément — le log
  atteint **186 000 lignes en ~9 min** (~160 lignes/s agrégé),
  y compris sur tab=1 alors que son JS est court-circuité par le
  disjoncteur.
- L'API reste responsive (sondes < 250 ms) — ce n'est **pas** le
  blocage dur §B-1 (allrecipes) ; mais une pompe qui spinne à vide
  sur des onglets inertes est un gaspillage mesuré et un bruit
  d'observabilité — `explorer-core`/daemon → parenté §B-1
  (famille « tâches post-apply non bornées »), formulation :
  **pompe de synchronisation jamais au repos**.

---

## Synthèse §8

- **`[V4-main]`** : 1 anomalie critique **RÉSOLU** (§B-2 cdiscount,
  avec validation terrain du disjoncteur J1/JS-C164) ; 1 **DÉGRADÉ
  EN MIEUX** (§B-14 OSM — JS-C162 confirmé) ; 2 **TOUJOURS
  PRÉSENT** (§B-7 GFG, §B-20 marmiton).
- Progrès annexes mesurés : §B-12 non reproduit sur marmiton ;
  page cdiscount réelle servie (anti-bot absorbé par le kill
  d'éval) ; hit-test non testé ici mais navigations propres.
- Points d'attention pour SYNTHESE : §B-5 confirmer « toujours
  présent » (marmiton) ; §B-14 ajouter `GeolocationPosition` à la
  matrice ; §B-4 deux bras silencieux frais (GFG `defer` Next.js,
  OSM tuiles Leaflet) ; §B-18 ajouter l'artefact header marmiton ;
  §B-1 note « pompe sync_profile permanente ».
