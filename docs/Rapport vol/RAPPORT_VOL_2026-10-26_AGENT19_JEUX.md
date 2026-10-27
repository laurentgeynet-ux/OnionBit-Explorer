<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — AGENT 19 — sites jeux/tech

- **Date** : 2026-10-26
- **Binaire** : `C:/obe-vol-head/target/release/explorer-daemon.exe` (release, HEAD)
- **Daemon** : port 9403, `EXPLORER_VIEWPORT=1366x900`, PID 8541
- **Protocole** : `docs/vol-agent.md`

| Cible | URL finale | Verdict | Sévérité |
|---|---|---|---|
| Steam Store | `store.steampowered.com` | SSR partiel utile, app JS morte | Moyenne |
| IGN | redirigé `fr.ign.com` | Layout effondré en colonne ~380 px, contenu présent | Haute |
| GameFAQs | `gamefaqs.gamespot.com` | Rendu quasi complet | Basse |
| Speedtest | `speedtest.net` | Shell SSR correct, app JS morte | Moyenne |
| GOG (fallback) | `gog.com/fr/` | Shell non hydraté, `<#comment>` massifs, SVG géants | Critique |

---

## 1. Steam — `https://store.steampowered.com`

- `doc_h = 3042`, `dom_nodes = 9703`, `layout_boxes = 547`, `display_items = 310`,
  `img_bodies = 848 Ko`, `js_heap = 29,2 Mo`, `journal_fallbacks = 1`.
- Capture : `captures/vol19_steam.png` (1366×3042, doc entier) + `vol19_steam_app.png`.

### Erreur verbatim (1 seule, fatale pour l'app)

```
exception js : TypeError: cannot read `apply` of Undefined
    at <anonymous> (.../applications/store/main.js?v=gd7L5jBg_AN-&l=english&_cdn=fastly:14:285556)
    at s (.../main.js:14:237436)
    at <anonymous> (.../main.js:18:199791)
    ...
    at <anonymous> (.../main.js:147:62289) — module main.js
```

### Constats mesurés

- Le HTML SSR peint : nav (« STORE / COMMUNITY / SUPPORT » verticale
  au lieu d'horizontale), onglets « Popular New Releases / Top Sellers /
  Popular Upcoming / Specials / Trending Free » alignés, liste de 10 jeux
  avec titres, genres, dates, **prix barrés + remises** (« -15% 6,79€ »)
  correctement stylés.
- Carrousel héro **entièrement vide** (zone grise géante) : peuplé par JS → mort.
- « Loading more content… » figé en bas (lazy-loading JS mort).
- **`<#comment>` peint comme texte** en plusieurs endroits (nœuds commentaire
  rendus visibles — bug transversal, voir §6).
- Scroll ×2 OK (`scroll_y` 900 → 1800), clic sur un jeu → **navigation réelle**
  vers `/app/5316540/Tidy_Up_the_Halloween_Shop/` : page produit rendue
  remarquablement bien (image héros peinte, description, « Very Positive (211) »,
  bloc achat « -10% Add to Cart », tags Single-player/Achievements, tableau
  langues). Seules zones mortes : parties JS (wishlist boutons, « <#comment> »).

### Classification

`PARTIEL-UTILE` — navigation réelle possible, contenu SSR lisible ; tout le
dynamique (carrousel, infinite scroll, filtres) mort par l'exception `apply`.

---

## 2. IGN — `https://www.ign.com` → `https://fr.ign.com/`

- `doc_h = 16261` (!), `dom_nodes = 3874`, `layout_boxes = 1355`,
  `display_items = 2219`, `images_bytes = 73 Mo`, `js_heap = 36,1 Mo`.
- Frame initiale rendue en **viewport 1366×900** seulement (les autres cibles
  donnent le document entier — incohérence de comportement `/frame` à noter).
- Captures : `vol19_ign.png` (viewport), `vol19_ign_s900/1800/3300.png`.
- Note : `/frame` a rendu un fichier **vide (0 octet)** une fois après scroll
  rapide ×3 — retry nécessaire (dégradation transitoire, pas de crash).

### Erreur verbatim

```
exception js : TypeError: cannot read `indexOf` of Undefined
    at <anonymous> (inline:16:1:2)
```

(1 seule entrée console — la plupart des scripts tiers ont peut-être
échoué silencieusement ou pas été exécutés.)

### Constats mesurés

- **Tout le contenu tient dans une colonne d'environ 250-380 px** à gauche
  de l'écran ; le reste du viewport est vide. La grille homepage (hero +
  cards) ne se forme pas — le menu latéral « drawer » (IGN Store, GTA 6,
  Tests & Critiques, Discord, Ciné/Séries/Anime, Comics, Tech, PS5, Xbox,
  PC, Switch…) est **peint en permanence** au lieu d'être hors-canvas.
- Les images d'articles **se chargent et peignent** (vignettes Modern
  Warfare 4, AION 2, Castlevania…), les titres d'articles sont présents.
- **Chevauchement de texte** : « Test Castlevania : Belmont's Curse »
  chevauche « Politique de Cookies », le titre chevauche l'image au-dessus
  — positions absolues/recouvrements non résolus.
- `<#comment>` peints partout (même bug transversal).
- Scroll fonctionne (`scroll_y` jusqu'à 8200) ; le contenu défile mais
  reste dans la colonne étroite.

### Classification

`CASSÉ-LISIBLE` — le contenu existe et les images arrivent, mais le layout
de grille est effondré (probable trou CSS : grid/flex container ou media
query non appliquée) + menu off-canvas peint + chevauchements.

---

## 3. GameFAQs — `https://gamefaqs.gamespot.com`

- `doc_h = 3056`, `dom_nodes = 2404`, `layout_boxes = 1630`,
  `display_items = 1482`, `js_heap = 23 Mo`.
- Capture : `vol19_gamefaqs.png` (1366×3056, doc entier).

### Erreurs verbatim (2, non fatales)

```
exception js : TypeError: cannot read `appendChild` of Undefined
    at c (1:13)
```
```
exception js : TypeError: `set` is not a function (on t)
    at <anonymous> (https://script.wikia.nocookie.net/fandom-ae-assets/identity-service/v2.121.0/main.js:3:728)
    ... — service d'identité Fandom
```

### Constats mesurés

- **Meilleur rendu du vol** : header plateformes (PC PS4 PS5 Switch 2
  Xbox One Xbox Series), barre de recherche, nav Boards/News/Q&A/
  Community/Contribute/Games, toutes les sections peuplées : « Popular
  Board Topics » (10 sujets avec vignettes), « Today's Featured Top 10 »,
  sondage Pikmin interactif (radio buttons peints), « Gaming News »,
  « Popular Questions », « Popular Games » (tableau 1-10 avec codes
  plateformes), footer complet.
- Défauts mineurs : rail droit « Popular New Releases » — titres tronqués
  en colonne très étroite (« Dragon'sDragon's Dogma 2: Dark Arisen
  Dogma 2: » superposés), quelques ligatures de colonnes qui se touchent ;
  `<#comment>` peint en pied de page.
- Clic sur un sujet de forum → `{"result":"ignored"}` (pas de navigation
  — hit-test ou gestion de lien à creuser).

### Classification

`FONCTIONNEL` — navigation en lecture quasi complète ; trous JS
(identity Fandom, appendChild) sans impact visuel majeur ; clic ignoré
à instrumenter.

---

## 4. Speedtest — `https://www.speedtest.net`

- `doc_h = 1803`, `dom_nodes = 1127`, `layout_boxes = 618`,
  `display_items = 206`, `js_heap = 34,9 Mo`.
- Capture : `vol19_speedtest.png` (doc entier) + `vol19_speedtest_go.png`.

### Erreur verbatim (fatale pour l'app)

```
exception js : TypeError: `checkConfig` is not a function (on N)
    at e (https://r.cdnst.net/assets/index-DcaxsiQx.js:8:67626)
    at N (https://r.cdnst.net/assets/index-DcaxsiQx.js:8:70146) — module
```

### Constats mesurés

- Shell SSR **étonnamment complet** : header (logo SPEEDTEST, Menu,
  English, theme toggle, Download, Sign In), onglets Results/Settings,
  cartes produit avec icônes (Desktop Apps, Troubleshoot Your Wifi,
  Check for Outages, QR code), footer Ookla Brands complet.
- Bouton **GO rendu comme un carré** à dégradé teal au lieu du cercle
  emblématique — `border-radius: 50%` (ou clip) non appliqué.
- Zones dynamiques vides : « Finding provider », « Finding optimal
  server… », toggle « Multi / Single » — l'app React meurt au boot sur
  `checkConfig`, donc aucune valeur, aucun handler.
- **Clic sur GO → `{"result":"ignored"}`** ×2 : dégradation correcte
  (pas de crash, pas de fausse nav), le test est simplement impossible.
- `<#comment>` peints dans header et footer (« © 2006–<#comment>2026 »).

### Classification

`SHELL-MORT` — vitrine correcte, fonctionnalité nulle (attendu sans
WebGL/canvas app vivante) ; la dégradation est propre.

---

## 5. GOG (fallback) — `https://www.gog.com` → `https://www.gog.com/fr/`

- `doc_h = 14186`, `dom_nodes = 14916`, `layout_boxes = 9130`,
  `display_items = 7124`, `js_heap = 6,3 Mo` seulement (app jamais bootée).
- Captures : `vol19_gog.png` + tiers `vol19_gog_p0/1/2.png`.

### Erreurs verbatim (6 — première fatale en tête)

```
exception js : TypeError: `then` is not a function (on Object)
    at <anonymous> (https://www.gog.com/accessTokenClient.js:1:2329) ...
```
```
exception js : TypeError: `instanceof` requires an object
    at <anonymous> (.../fr/menu.js:1:8856) ... at Pt (.../menu.js:1:196946)
```
```
exception js : TypeError: `module` is not a function (on Object) — src «/* The host app
    requires gogMenuModule + gogFooterModule, which only the v2 bundle registers — s…»
```
```
exception js : TypeError: cannot read `__zone_symbol__addEventListener` of Undefined
    at L (.../fr/polyfills.s_t1792736500_a723f5dba65f5ae3.js:1:39348) ...
```
```
exception js : TypeError: `observe` is not a function (on Object)
    at <anonymous> (.../fr/scripts.s_t1792736500_a2e766d0cc21b656.js:1:190191) ...
```
```
exception js : SyntaxError: syntax error at 3:822174: expected `;` —
    «.../fr/main.s_t1792736500_34eafcaa205505f8.js» — module
```

### Constats mesurés

- Page Angular **jamais hydratée** : le DOM SSR (14 916 nœuds) contient
  le catalogue mais le rendu est un squelette gris : blocs
  placeholder gris, `<#comment>` **littéralement partout** (des centaines
  de nœuds commentaires Angular `<!-- -->` peints comme texte — le bug
  transversal atteint ici son paroxysme et rend la page illisible).
- Menu header partiellement lisible (JEUX / À PROPOS / COMMUNAUTÉ /
  BESOIN D'AIDE, recherche, panier) + sous-menus dépliés peints
  (Mode de couleur, COMMUNAUTÉ, BESOIN D'AIDE ?).
- **SVG non dimensionnés** : logo X (Twitter) et GOG peints en noir
  géant sur ~3000 px de haut — `viewBox`/width/height non honorés.
- Quelques îlots corrects : bouton « REJOIGNEZ GOG » violet, bannière
  dorée « Devenez GOG Patron aujourd'hui ».
- Erreurs JS révélatrices de trous moteur : `then` manquant (Promise
  thenable), `observe` (ResizeObserver/IntersectionObserver stubs),
  `__zone_symbol__addEventListener` (zone.js attend des props internes),
  SyntaxError dans le bundle principal (feature de syntaxe non supportée
  à 3:822174).

### Classification

`MORT` — vitrine illisible ; première erreur fatale = `then` is not a
function (promesse non thenable dans accessTokenClient) qui casse
l'amorçage, puis cascade.

---

## 6. Constats actionnables

1. **Nœuds commentaires peints comme texte `<#comment>`** — observé sur
   **les 5 cibles** (Steam, IGN, GameFAQs, Speedtest, GOG). Un nœud
   `Comment` du DOM arrive jusqu'à la display list comme texte. Sur GOG
   (Angular, commentaires structurels `ng-*`) ça détruit la page.
   → Vérifier que les nœuds `Comment` ne produisent jamais de run
   inline/boîte peinte (explorer-layout ou mapping DOM→layout).
   **Sévérité haute, transversal, probablement un fix unique.**

2. **Trou Promise/`then`** : `TypeError: 'then' is not a function (on
   Object)` (GOG accessTokenClient) — les objets thenable/Promise ne sont
   pas reconnus. Bloque l'amorçage de toute app à promesses.
   **Sévérité critique (JS).**

3. **`TypeError: cannot read 'apply' of Undefined`** (Steam main.js) —
   pattern `x.apply(...)` sur méthode absente, probablement un builtin
   manquant (Function.prototype, Reflect, ou polyfill). Tue le bootstrap
   React de Steam. **Sévérité haute (JS).**

4. **API d'observation manquantes** : ``observe` is not a function`
   (GOG scripts.js) — `IntersectionObserver`/`ResizeObserver`/
   `MutationObserver` absents ou stubs sans méthodes. Fréquent sur les
   SPA modernes. **Sévérité haute (DOM API).**

5. **`SyntaxError: expected ';'` en plein bundle** (GOG main.js
   3:822174) — feature de syntaxe ES non supportée par le parseur
   d'`explorer-jscore` (à extraire et bissecter sur le bundle sauvegardé).
   **Sévérité haute (JS parser).**

6. **Menu off-canvas peint** (IGN) : le drawer de navigation est visible
   en permanence — probablement `transform: translateX(-100%)` /
   `visibility`/`position: fixed` hors-écran non honoré. **Sévérité
   moyenne (CSS/layout).**

7. **Grille effondrée en colonne unique ~380 px** (IGN, 16 261 px de
   haut pour une homepage) : grid/flex container principal non appliqué.
   À corréler avec le support CSS Grid / media queries. **Sévérité
   haute (CSS layout).**

8. **Chevauchements de texte** (IGN : titres sur images/sur liens) —
   positionnement absolu/z-index ou hauteurs de ligne. **Sévérité
   moyenne.**

9. **`border-radius` non appliqué** (Speedtest : GO carré au lieu de
   cercle ; à confirmer : peut aussi être `clip-path` ou overflow).
   **Sévérité moyenne (CSS paint).**

10. **SVG sans dimensionnement** (GOG : icônes X/GOG géantes sur
    ~3000 px) — `viewBox` sans width/height explicite non mis à l'échelle.
    **Sévérité moyenne (render).**

11. **`click` → `{"result":"ignored"}`** sur liens valides (GameFAQs
    topics, Speedtest GO) alors que le clic Steam naviguait — hit-test
    ou dispatch d'événement inconsistent. **Sévérité moyenne (input).**

12. **`/frame` incohérent** : document entier sur Steam/GameFAQs/GOG,
    viewport seul sur IGN ; et une frame **vide 0 octet** après scroll
    rapide (retry OK). **Sévérité basse (API/outillage) — mais
    documenter pour les prochains vols.**

13. **`__zone_symbol__addEventListener` of Undefined** (GOG polyfills
    zone.js) — `addEventListener` non accessible là où zone.js le patch
    (probablement prototype/window). Bloque toute app Angular.
    **Sévérité haute (DOM API) — corollaire de 2/4.**

14. **Redirections géographiques suivies** : ign.com → fr.ign.com,
    gog.com → gog.com/fr/ — la stack net/redirection fonctionne.
    (Constat positif.)

## Synthèse

- **Ce qui marche** : SSR HTML+CSS de base, images, navigation par
  liens (Steam prouvé), scroll, redirections, dégradation sans crash
  (0 panic sur 5 sites hostiles).
- **Ce qui bloque** : un bug unique de nœuds commentaires peints
  (#1 — fix unique, impact 5/5 sites), puis les trous JS
  (`Promise/then`, `apply`, observers, zone.js, un SyntaxError
  parseur) qui tuent le bootstrap de toutes les SPA.
- **Hiérarchie de réparation suggérée** : #1 (commentaires) → #2
  (Promise) → #4+#13 (observers/addEventListener) → #3 (apply) →
  #5 (parser) → #6/#7 (layout off-canvas+grid).

## Artefacts

- Captures : `docs/Rapport vol/captures/vol19_{steam,steam_app,ign,
  ign_s900,ign_s1800,ign_s3300,gamefaqs,speedtest,speedtest_go,gog}.png`
- Logs : `docs/Rapport vol/logs/vol19_{steam,ign,gamefaqs,speedtest,
  gog}_{dom,layout}.txt`
- Daemon log : `_vol19_daemon.log`
