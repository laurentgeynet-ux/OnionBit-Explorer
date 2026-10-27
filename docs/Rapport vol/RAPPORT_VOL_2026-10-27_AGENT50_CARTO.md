<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — AGENT 50 — Cartographie IGN / OSM (2026-10-27)

- **Binaire** : `[V4-main]` — `C:/obe-vol-target/release/explorer-daemon.exe`
  (WIP `depth_limit` embarqué), `--port 9432` (PID 4944),
  `EXPLORER_VIEWPORT=1366x900`, journal `_vol50_daemon.log` (racine)
- **Cibles** : cartes.gouv.fr · www.geoportail.gouv.fr ·
  umap.openstreetmap.fr — piste carto : mort silencieuse historique
  §B-4 (IGN) et tuiles Leaflet §B-14 (uMap)
- **Protocole** : `docs/vol-agent.md` — traçabilité §8, IDs §B de
  `SYNTHESE.md`

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| cartes.gouv.fr | **Page blanche, mais la mort n'est plus silencieuse** | `unhandledrejection: RangeError: unsupported encoding (×4)` | §B-4(b) tracé + nouvelle signature → §B-14 |
| www.geoportail.gouv.fr | **Redirection réelle suivie → même app cartes.gouv** | idem (×4) | repro cible 1 — frame byte-identique |
| umap.openstreetmap.fr | **Leaflet bout-en-bout : tuiles demandées *et* peintes** | 0 erreur accueil ; `cannot set 'hidden' of Undefined` éditeur | bras tuiles §B-14 levé · **§B-28 proposé** (chaîne `height:100%` effondrée) |

Faits marquants :

1. **Première tuile Leaflet de l'histoire du projet** — uMap accueil :
   `<img class="leaflet-tile" src="https://a.piano.tiles.quaidorsay.fr/en/6/31/20.png">`
   présent dans le DOM sérialisé et **peint** dans la frame (vol 27
   OSM : « Leaflet instancié, tuiles jamais demandées »). L'éditeur
   uMap en peint plusieurs (noms de villes lisibles : Bruxelles,
   Reims, Laon…, `leaflet-tile-loaded`).
2. **Modules ESM + importmap fonctionnels** sur uMap
   (`leaflet-src.esm.*.js` résolu via importmap, instancié) — recoupe
   §B-4(a) : les modules ne sont *pas* inertes partout.
3. **cartes.gouv n'est plus muet** : `RangeError: unsupported
   encoding` tracé ×4 — sort du périmètre §B-4 « 0 entrée console »,
   entre dans la matrice §B-14 (API manquante à nommer).
4. **Clic → vraie navigation** confirmé (« Create a map » →
   `navigated`) — §B-13 sain, une fois de plus.

RAM daemon : 607 → 1487 → 1669 Mo sur la session (sous le seuil
2,5 Go ; croissance ~1 Go attribuable au heap éditeur + bfcache
uMap 17,6 Mio js_heap — §T3 confirmé en conditions réelles).

---

## 1. cartes.gouv.fr — SPA Vue morte, erreur désormais tracée → §B-4(b) + §B-14

- `POST navigate https://cartes.gouv.fr` → **200, redirection
  applicative suivie** : URL finale
  `https://cartes.gouv.fr/explorer-les-cartes/`, titre réel
  « cartes.gouv.fr | Explorer les cartes », `doc_h=900` (=viewport —
  marqueur §T4 page morte).
- `/console` verbatim (grâce 12 s) :
  `{"console":["unhandledrejection: RangeError: unsupported encoding (×4)"]}`
- `/frame` → `captures/vol50_cartes.png` : **blanche totale**.
- `/dom` (`logs/vol50_cartes_dom.txt`) : **39 nœuds** — squelette
  Vite/Vue intact : `<div id="app">` vide, bundle
  `type="module"` `/explorer-les-cartes/assets/index-t8Cr_OTk.js`
  + 3 `modulepreload` (Load, StoreData, Patience).
- `/layout` : 3 boîtes, 1 display item. `/mem` : `js_heap=33,4 Mio`
  (le module s'est amorcé — heap non nul), `total=38,5 Mio`,
  `journal_applies=0`, `journal_fallbacks=0`.
- Input : scroll dy=800 → `scroll_y=0` (rien à scroller) ;
  clic (683,450) → `ignored`, `hit=true` (page vide, aucun contrôle
  sous le point — comportement correct).

**Attribution** : l'erreur `unhandledrejection` prouve que le bras
« mort silencieuse » historique de cette cible est **fermé** — le
rejet est intercepté et tracé (bras §B-4(b) « exceptions
post-instanciation »). Reste la racine : `RangeError: unsupported
encoding` — vraisemblablement `TextDecoder`/`TextEncoder` avec un
label non supporté ou un décodage de réponse — **à ranger en §B-14**
(matrice APIs) comme nouvelle signature, crate `explorer-jscore`.
Heap 33 Mio + rejet ×4 : le module principal a évalué puis a rejeté
en cascade — Vue jamais monté, `#app` vide.

## 2. www.geoportail.gouv.fr — redirection serveur réelle → repro cible 1

- `POST navigate https://www.geoportail.gouv.fr` → URL finale
  `https://cartes.gouv.fr/explorer-les-cartes/` : **la redirection
  Géoportail→cartes.gouv (fusion IGN réelle) est suivie par le
  moteur** — navigation réelle confirmée.
- Frame `captures/vol50_geoportail.png` **byte-identique** à celle
  de la cible 1 (md5 `dc4eacd70ae57c4ceff28b849486370c` les deux) —
  même app servie, même boot mort.
- Console identique : `RangeError: unsupported encoding (×4)`.
  DOM 39 nœuds identique. `js_heap=33,4 Mio`.
- Clic (683,450) → `ignored` (`hit=true`, page vide).

**Attribution** : aucune anomalie propre — la cible se réduit à la
cible 1. La chaîne de redirections multi-sauts fonctionne (§C
« redirections » confirmé).

## 3. umap.openstreetmap.fr — Leaflet bout-en-bout, premier succès carto

### 3a. Accueil `/` → `/en/` — SSR + Leaflet pleinement booté

- `POST navigate https://umap.openstreetmap.fr` → redirection `/en/`
  suivie, titre « uMap - Online map creator », **`doc_h=3202` réel**.
- `/console` : **`pas de journal` — 0 erreur JS** (grâce 15 s).
- `/frame` → `captures/vol50_umap.png` : page **rendue** — header
  (Log in / Sign in, About, Help, « Create a map »), zone carte avec
  **tuile raster réelle peinte**, champ de recherche + bouton Search,
  logo, argumentaire, cards « browse maps ».
- `/dom` (`logs/vol50_umap_dom.txt`, 262 Ko, 1082 nœuds) :
  - importmap `<script type="importmap">` résolue (`"leaflet":
    "/static/umap/vendors/leaflet/leaflet-src.esm.0e703c906b6c.js"`) ;
  - **structure Leaflet complète instanciée** : `leaflet-container`,
    `leaflet-map-pane`, `leaflet-tile-pane`, `leaflet-tile-container
    leaflet-zoom-animated`, 8+ panes custom `data-id` uMap ;
  - **tuile réelle** : `<img class="leaflet-tile"
    src="https://a.piano.tiles.quaidorsay.fr/en/6/31/20.png"
    style="width:256px;height:256px;left:-82px;top:-16px">` — le
    serveur de tuiles Quai d'Orsay/OSM-FR a été **contacté et servi** ;
  - `/mem` : `images=2,7 Mio`, `img_bodies=304 Kio`,
    `layout_boxes=422`, `display_items=214`, `js_heap=17,6 Mio`,
    `journal_applies=1`, `journal_fallbacks=0`.
- Input : clic carte (160,100) → `ignored`, `hit=true` — la zone
  carte n'est pas un lien (comportement correct, *pas* §B-13) ;
  scroll dy=700 → `scroll_y=700` : `vol50_umap_scroll.png` montre la
  suite SSR saine (« Create a map », « Get inspired, browse maps »,
  cards — **thumbnails grises vides** : placeholder non remplacé,
  cosmétique).

**Attribution** : le bras « tuiles jamais demandées » (§B-14, legs
vol 27 OSM) est **levé sur cette cible** — Leaflet va jusqu'au fetch
et à la peinture de tuiles. Les modules ESM tournent (contre-exemple
à §B-4(a)). À re-voler openstreetmap.org pour clore le bras OSM
proprement dit.

### 3b. Clic « Create a map » → éditeur — navigation réelle + §B-28 proposé

- Clic (1227,20) sur le lien « Create a map » → **`navigated`** —
  §B-13 confirmé sain (clic → vraie navigation).
- `/console` verbatim éditeur :
  `unhandledrejection: TypeError: cannot set 'hidden' of Undefined`
  `at <anonymous> (./ui/controls.d1d0f5d4cb36.js:181:1)` …
  `(../modules/app.395ccf268de3.js:33:1)` — un contrôle UI échoue
  (`§B-14`, API DOM manquante — probablement `HTMLElement.hidden`
  sur un custom element ou querySelector raté).
- `/frame` → `captures/vol50_umap_edit.png` : **tuiles OSM peintes**
  (`//c.tile.openstreetmap.fr/osmfr/6/32/21.png` —
  `leaflet-tile-loaded`, protocol-relative `//` résolu), contrôles
  zoom +/− rendus — **mais la carte ne fait qu'une tranche ~165 px**
  en haut à gauche.
- `/layout` (`logs/vol50_umap_edit_layout.txt`) : **toutes les
  hauteurs sont 0** — `block<html> [0 0 1366x0]`, `body [1366x0]`,
  conteneur carte `block<div> [0 0 256x0]` ; seule la tuile
  `block<img> [-91 -109 256x256]` déborde. L'éditeur uMap est une
  app plein-viewport (`html,body{height:100%}` + shell flex/absolute)
  : la chaîne de hauteurs en % **s'effondre à 0** → Leaflet lit
  `clientHeight≈0` → ne demande qu'**une tuile** (couvrant 256 px).
  Le moteur JS fait tout le travail ; c'est le layout qui le prive
  de hauteur.
- Boutons outils : `<i>` 36×36 peints « ? » — icônes/glyphes non
  résolus (cosmétique, famille tofu/icon-font).
- `/mem` : `total=33,3 Mio`, `js_heap=8,5 Mio`, `journal_applies=2`,
  `bfcache` retient l'accueil (1082 nœuds, js_heap 17,6 Mio —
  §T3 heap gelé confirmé).

**§B-28 proposé — `height:100%` en chaîne effondrée à 0 sur app
plein-viewport** : `html`/`body`/`#map` tout à `1366x0` ; le bloc
englobant initial (viewport) n'alimente pas les pourcentages de
hauteur → les shells `height:100%`/`100dvh`/flex-column plein écran
(uMap éditeur ; famille commune avec §B-24 stretch vertical vinted)
s'effondrent. Crate : `explorer-layout`/`explorer-css`. Preuve :
`logs/vol50_umap_edit_layout.txt`, `captures/vol50_umap_edit.png`.
Effet secondaire mesuré : cascade fonctionnelle — Leaflet
sous-dimensionne ses tuiles (`clientHeight=0` → 1 tuile), donc
l'anomalie se *propage* du layout vers le JS.

---

## Journal de session

| Heure (UTC) | Action | Résultat |
|---|---|---|
| 09:06 | daemon :9432 (PID 4944) | prêt, viewport 1366×900 |
| 09:07 | tab1 cartes.gouv.fr | redirect `/explorer-les-cartes/`, blanche, `unsupported encoding ×4` |
| 09:07 | scroll+clic tab1 | `scroll_y=0`, `ignored` |
| 09:08 | tab2 geoportail.gouv.fr | redirect → cartes.gouv, frame identique |
| 09:09 | tab3 umap.openstreetmap.fr | `/en/`, doc_h 3202, 0 erreur, tuile peinte |
| 09:09 | clic carte + scroll tab3 | `ignored` (hit), `scroll_y=700` |
| 09:10 | clic « Create a map » | `navigated` → éditeur, tuiles OSM, hauteur 0 |
| 09:11 | télémétrie éditeur | §B-28 proposé ; RAM 1669 Mo |

## Clôture §B touchés

| ID | Mouvement proposé |
|---|---|
| §B-4(b) | cartes.gouv **sort du silence** — preuve `unhandledrejection` tracée ; reste la racine |
| §B-14 | +2 signatures : `RangeError: unsupported encoding` (cartes.gouv), `cannot set 'hidden' of Undefined` (uMap controls) ; **bras « tuiles Leaflet » levé sur uMap** |
| §B-13 | confirmé sain (clic → `navigated`) |
| §B-28 | **nouveau** — chaîne `height:100%` plein-viewport effondrée (uMap éditeur) ; cascade vers JS (Leaflet sous-dimensionné) |
| §T3 | heap JS gelé ~17,6 Mio/page en bfcache — confirmé terrain |
