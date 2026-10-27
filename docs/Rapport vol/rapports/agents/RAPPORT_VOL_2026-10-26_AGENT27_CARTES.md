<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — AGENT 27 : cartographie / temps réel (canvas, tiles, workers)

- **Date** : 2026-10-26
- **Binaire** : `C:/obe-vol-integ/target/release/explorer-daemon.exe` (release,
  intégration V2)
- **Viewport** : `EXPLORER_VIEWPORT=1366x900`, daemon dédié `:9411`
- **Captures** : `docs/Rapport vol/captures/vol27_*.png`
- **Logs** : `docs/Rapport vol/logs/vol27_*_{dom,layout,console}.txt`

## Synthèse

| Cible | État | Carte visible ? | Erreur dominante |
|---|---|---|---|
| flightradar24.com | Shell mort | Non — page quasi blanche | Aucune erreur console (échec silencieux) |
| windy.com | Shell statique | Non — fond gris, `#leaflet-map` vide | Aucune erreur console (modules JS ?) |
| marinetraffic.com | Error boundary du site | Non — « Nothing to sea here » | `TypeError: cannot read 'get' of Undefined` (×N) |
| openstreetmap.org | **Leaflet monté** | Non — zone grise, mais UI complète | `TypeError: cannot read 'call' of Undefined` |
| accuweather.com | Rendu quasi complet | Radar (iframe) vide | `TypeError: 'map' is not a function (on WeakMap)` |

## 1. flightradar24.com — sévérité : bloquante

- URL finale `https://www.flightradar24.com/`, titre correct,
  `doc_h=900` (= viewport : la page n'a pas de contenu scrollable).
- Frame : **page blanche** avec uniquement « 24 » (bout du logo) et
  l'icône hamburger. `vol27_flightradar.png`.
- Layout : sidebar partiellement construite (header 265×52, input,
  boutons svg) mais conteneur carte à `block<div> [1366 0 0x900]` —
  **hors viewport, largeur 0** ; `block<main> [1366 0 0x0]`.
  Tout le corps de l'app est poussé à x=1366 / hauteur 0.
- DOM 242 Ko : le SSR/markup est là, **aucun `<canvas>`**.
- Console : `pas de journal` — **zéro erreur JS rapportée** alors que
  l'app ne s'hydrate manifestement pas. C'est le pire cas : échec
  silencieux, rien à bissecter côté console.
- Interactions (scroll 800, clic 230,25) : aucun changement de frame
  ni de console. `vol27_flightradar_after.png` identique.

## 2. windy.com — sévérité : bloquante

- URL finale `https://www.windy.com/`, `doc_h=0` (body vide de
  hauteur — contenu absolute-positionné).
- Frame : **fond gris** plein écran avec motif de grille (background
  CSS du conteneur carte), logo « Windy.com » centré en haut,
  attribution « © OpenStreetMap contributors » en bas à droite.
  `vol27_windy.png`, inchangé après 10 s (`vol27_windy_2.png`).
- DOM 10 Ko : shell statique uniquement. `#leaflet-map` **vide** —
  aucun pane, aucune tuile, aucun canvas injecté. 0 occurrence de
  `canvas`, 0 `leaflet-tile`.
- Les scripts d'amorçage sont `type="module"` + `importmap`
  (`index.js`, `leaflet-gl.js`). Le script inline
  `window.W={detectedDevice:...}` a tourné (`id="device-desktop"`
  posé sur `<html>`), donc le JS classique fonctionne — mais **les
  modules ES semblent ne jamais s'exécuter** (hypothèse : `type=
  "module"` / importmap non supportés par explorer-jscore).
- Console : `pas de journal` — encore un échec silencieux.

## 3. marinetraffic.com — sévérité : bloquante (mais JS actif)

- `doc_h=8` (la page s'est réduite à l'error boundary).
- Frame : erreur applicative rendue par le site lui-même —
  « **Nothing to sea here / Oops... Something went wrong. Please
  refresh the page, or contact us if the error persists.** »
  `vol27_marinetraffic.png`.
- Console verbatim (répété, ring plein) :

  ```
  unhandledrejection: TypeError: cannot read `get` of Undefined
      at <anonymous> (/js/8df3055b44d8f38cca86.mtapp.v1642-40324908690.js:1:28384)
      at <anonymous> (/js/8df3055b44d8f38cca86.mtapp.v1642-40324908690.js:1:26494)
      at <anonymous> (/js/8df3055b44d8f38cca86.mtapp.v1642-40324908690.js:1:26185)
      at <anonymous> (/js/8df3055b44d8f38cca86.mtapp.v1642-40324908690.js:1:200850)
      at <anonymous> (/js/8df3055b44d8f38cca86.mtapp.v1642-40324908690.js:1:185978)
      at <anonymous> (/js/8df3055b44d8f38cca86.mtapp.v1642-40324908690.js:1:210642)
      at <anonymous> (/js/8df3055b44d8f38cca86.mtapp.v1642-40324908690.js:1:2318)
      at <anonymous> (/js/8df3055b44d8f38cca86.mtapp.v1642-40324908690.js:1:2299)
      at a (/js/8df3055b44d8f38cca86.mtapp.v1642-40324908690.js:1:1754)
  ```

- DOM 1,2 Mo (le markup est énorme — l'app React a construit puis
  basculé en erreur). Le `.get` d'`Undefined` évoque un
  `Map.get`/`localStorage.get`/`headers.get` sur un objet non
  initialisé ou une API absente du runtime.

## 4. openstreetmap.org — RE-VOL JS-C138 : fix **partiellement confirmé**

- **Avant** (constat antérieur) : Leaflet mourait sur
  `Worker.addEventListener` → zone grise morte.
- **Maintenant** : l'exception `Worker.addEventListener` a
  **disparu** de la console — fix JS-C138 confirmé sur ce point.
  La page a progressé nettement plus loin :
  - DOM 79 Ko, **Leaflet complètement instancié** : `leaflet-container`,
    `leaflet-map-pane`, `leaflet-tile-pane`, `leaflet-marker-pane`,
    `leaflet-overlay-pane`, `leaflet-popup-pane`, `leaflet-control-
    attribution`, `leaflet-control-scale`, `leaflet-touch`,
    `leaflet-grab`, `leaflet-touch-zoom` — toute la machinerie est là.
  - Frame : header complet (Edit, History, Export…, Log In / Sign Up),
    sidebar avec bannière « 31 DAY OSM CHALLENGE », **contrôles carte
    Leaflet rendus** (zoom +/−, couches, partage, ?, localisation).
    `vol27_osm.png` puis `vol27_osm_2.png`.
- **Mais** : `leaflet-tile-pane` est **vide** — aucun `<img
  class="leaflet-tile">` créé, carte grise (fond `#ddd` du container).
  La couche de tuiles n'est jamais peuplée.
- Erreur restante, verbatim (×2 après clic+scroll) :

  ```
  exception js : TypeError: cannot read `call` of Undefined
      at <anonymous> (https://www.openstreetmap.org/assets/application-3901b15b6f7d3d1a2230f955618d0bf378642e9ba2d6e87a67ac7c26875231cf.js:62:606)
      at <anonymous> (https://www.openstreetmap.org/assets/application-3901b15b6f7d3d1a2230f955618d0bf378642e9ba2d6e87a67ac7c26875231cf.js:122:4217)
      at i (https://www.openstreetmap.org/assets/application-3901b15b6f7d3d1a2230f955618d0bf378642e9ba2d6e87a67ac7c26875231cf.js:32:436)
  ```

  `…call` d'`Undefined` ressemble à `Function.prototype.call` /
  `$.fn.call` sur un callback absent — probablement ce qui empêche
  la couche de tuiles (`L.tileLayer(...)`) de démarrer ou de se
  peupler.
- Interactions : clic carte (700,300) et scroll −240 (zoom out)
  → aucun changement, carte toujours grise, même erreur rejetée.
- **Verdict** : « fix confirmé » pour Worker.addEventListener
  (l'erreur a disparu, Leaflet s'instancie entièrement) — mais la
  mesure « la carte apparaît » n'est **pas atteinte** : il manque
  l'étape tuiles, bloquée par une erreur JS distincte.

## 5. accuweather.com (fallback) — sévérité : mineure

- `doc_h=5862`, page **quasi complète** : header, recherche, carte
  « France 13°C RealFeel® 12° », tableau « France Weather
  Conditions » (14 villes avec icônes météo rendues), colonne Top
  Stories complète. `vol27_accuweather.png`,
  `vol27_accuweather_scroll.png`.
- Trou : la zone « France Weather Radar » = rectangle blanc vide —
  radar servi en iframe/emeb JS non rempli.
- Console verbatim :

  ```
  unhandledrejection: TypeError: `map` is not a function (on WeakMap)
      at <anonymous> (https://cdn.p-n.io/pushly-sdk.min.js?domain_key=…:1:1075)
      … (×3, script tiers pushly-sdk)
  ```

  Script tiers push-notifications ; le site principal fonctionne
  malgré l'erreur.
- Artefact visuel secondaire : après scroll, le header sticky
  flotte en surimpression au tiers supérieur de la frame
  (`vol27_accuweather_scroll.png`) — repaint/clip du fixed header
  à vérifier, sévérité faible.

## Constats actionnables

1. **[Bloquante, JS] `type="module"` + `importmap` non exécutés** —
   windy.com : shell statique OK, `#leaflet-map` jamais peuplé,
   zéro erreur console. Vérifier le support des modules ES et des
   importmaps dans explorer-jscore ; à minima logger « module
   script non supporté » dans `/console` au lieu de l'échec
   silencieux. Probable cause aussi de l'échec flightradar24.
2. **[Bloquante, JS] `TypeError: cannot read 'call' of Undefined`
   sur openstreetmap.org** (application.js:62:606) — Leaflet
   s'instancie mais la couche de tuiles ne démarre jamais
   (`leaflet-tile-pane` vide). Bissecter quel `Undefined.call` est
   invoqué (callback jQuery/Leaflet ?). C'est le prochain verrou
   après JS-C138.
3. **[Bloquante, JS] `TypeError: cannot read 'get' of Undefined`
   sur marinetraffic.com** (mtapp.v1642.js:1:28384, en boucle) —
   l'app construit 1,2 Mo de DOM puis tombe dans son error
   boundary. Identifier l'API dont le `.get` est appelé
   (`Map`/`URLSearchParams`/`localStorage`/`Headers` ?).
4. **[Majeure, console] Échecs silencieux sans trace** —
   flightradar24 (app jamais hydratée, conteneur carte 0×900 à
   x=1366) et windy : **zéro entrée console**. Un navigateur qui
   ne rapporte pas ses échecs JS empêche toute bissection terrain —
   priorité instrumentale : toute exception/rejet non géré doit
   apparaître dans `/console`.
5. **[Mineure, JS] `WeakMap.map` inexistant** — pushly-sdk sur
   accuweather : `map` is not a function (on WeakMap). Vérifier le
   surface API exposée sur les built-ins (erreur message suggère
   un lookup de méthode sur le prototype WeakMap).
6. **[Mineure, rendu] Header sticky AccuWeather mal repeint** —
   après scroll 800 px, barre fixe en surimpression à ~y=200 px au
   lieu de coller au top (logo coupé). Repaint des positionnés
   fixed après scroll à vérifier.

## Fix confirmé

- **JS-C138 (`Worker.addEventListener`)** : erreur disparue sur
  openstreetmap.org ; Leaflet s'instancie en entier (panes,
  contrôles, attribution, grab/zoom handlers). Verrou suivant =
  constat #2.

## Arrêt

Onglets 1–5 supprimés via `DELETE /api/tabs/{id}` ; daemon tué par
son PID Windows (14704) — `taskkill /IM` non utilisé.
