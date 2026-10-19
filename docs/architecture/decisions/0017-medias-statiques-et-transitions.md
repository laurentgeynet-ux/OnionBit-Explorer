# ADR-0017 : SVG statique (`resvg`) et transitions CSS pilotées par le pull

- **Statut** : accepté
- **Date** : 2026-02
- **Contexte** : étape 45 — « Médias et graphiques ». Le
  périmètre est découpé en quatre sous-chantiers aux risques
  très différents : `<img src=*.svg>` / `<svg>` inline,
  `<canvas>` 2D (API JS + raster), transitions/animations CSS,
  `<video>`/`<audio>` (décodage lourd « à arbitrer »). Cette
  ADR fige le périmètre moteur livré et les déviations.

## Décision

1. **`<img src=*.svg>` via `resvg`/`usvg` 0.45 (pur Rust,
   apparié à notre `tiny-skia` 0.11).** La détection est un
   sniff octet (`<` + `<svg` dans le prolog) placé en tête de
   `decode_image` — `usvg` valide strictement ensuite, donc
   un document HTML portant un `<svg>` inline est refusé
   proprement. Le SVG est rasterisé à sa taille naturelle,
   **réduit proportionnellement au budget `max_px`** — un
   `viewBox` géant se comporte comme un bitmap hostile. Le
   pixmap tiny-skia (prémultiplié) est démultiplié vers le
   contrat `DecodedImage` (RGBA droit). SMIL, scripts et
   animations SVG sont hors périmètre (statique, roadmap).
2. **`<svg>` inline non livré — déviation.** Le contenu
   étranger SVG exige le mode « foreign content » du
   tokenizer HTML5 (la casse `viewBox`/`linearGradient` est
   perdue par le parsing HTML courant) ; re-sérialiser le DOM
   produirait un SVG cassé. C'est un chantier tokenizer à
   part entière, planifié avec la conformité HTML5 plutôt
   qu'ici.
3. **`<canvas>` 2D = couture `JsEngine` — côté équipe JS.**
   Le contexte 2D est une API de script ; le moteur expose
   déjà le blit bitmap (`DecodedImage`) nécessaire à
   l'affichage du backing store. Non livré ici, attribué au
   chantier JS dédié (hooks navigateurs, même modèle que
   `NavRequest`).
4. **`<video>`/`<audio>` : arbitrage repoussé** — le roadmap
   le note « décodage à arbitrer » : aucun décodeur A/V pur
   Rust n'est engagé (symphonia est audio-seul ; le
   multiplexer/demuxer vidéo resterait à écrire). Hors
   périmètre explicite.
5. **Transitions CSS : diff sur valeurs calculées, tick
   piloté par le pull de frames.** `explorer-css::transition`
   parse `transition`/longhands (`duration`, `delay`,
   `timing-function` — `linear`/`ease*`/`cubic-bezier`
   résolu par Newton+bissection) et interpole nombres,
   longueurs (même unité, ou absolues → px) et couleurs RGBA ;
   le reste commute discrètement à t=1 (spec).
   - **La clé de diff est l'ordinal d'élément** (ordre
     document), pas `NodeId` : la synchronisation DOM du
     miroir JS re-parse le document et régénère les ids.
     Déviation assumée : une insertion structurelle décale
     les ordinals et fait repartir les transitions des nœuds
     suivants.
   - **Pas de « before-change style » → pas de transition**
     : premier rendu et nœuds insérés n'animent rien (spec).
   - **L'animation est pilotée par le pull** : `relayout`
     échantillonne `Instant::now()` et applique les
     progressions en surcharge post-cascade (priorité maximale,
     spec « current transition ») ; `Session::tick_animations`
     est appelé par la route `GET /frame` de l'API — l'UI qui
     cadence ses pulls cadence l'animation. Le moteur n'a pas
     de boucle vsync propre ; `requestAnimationFrame` vit
     déjà côté `dillo-js` (= `setTimeout` 16 ms).
   - **Budgets** : `MAX_TRANSITIONS` = 256 par onglet ; les
     transitions meurent avec le document à la navigation.

## Conséquences

- `resvg` tire `fontdb`/`rustybuzz`/`roxmltree` — tous purs
  Rust, versions publiées depuis > 7 jours (règle 2).
- Le chemin `<img>` ne change pas : SVG passe par le même
  fetch borné, la même table d'images, le même blit.
- Aucune boucle d'animation interne : la latence perçue d'une
  transition dépend de la cadence de pull de l'UI (contrat
  inchangé pour Flutter — pull ou SSE, pas de push moteur).
- Les animations `@keyframes` restent à faire (même machine
  de surcharge — diff nécessaire, pas de keyframes parser
  encore) ; `MutationObserver`/workers restent à l'équipe JS.
