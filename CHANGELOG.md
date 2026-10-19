# Changelog

Toutes les modifications notables de ce projet sont consignées ici.

Le format suit [Keep a Changelog](https://keepachangelog.com/fr/1.1.0/)
et le projet adhère au [Semantic Versioning](https://semver.org/lang/fr/).

## [Unreleased]

## [0.1.0] - 2026-10-19

### Added

- **Étape 40 — release 0.1** : version workspace
  `0.0.0` → `0.1.0` (`version.workspace = true` dans les
  14 crates) ; `scripts/package.ps1` (bundle
  `dist/explorer-0.1.0` : daemon + cli release, runner
  Flutter Windows si disponible, zip) ; `scripts/smoke.ps1`
  — test de fumée E2E intégralement loopback (serveur
  `TcpListener` local + daemon + API `/status`, `/tabs`,
  `navigate`, `/frame` → signature PNG vérifiée) ;
  `README` complété (build & vérification).

- **Étape 39 — perf + fuzz** (`explorer-html`,
  `explorer-css`, `explorer-core`, `fuzz/`) : bornes de
  profondeur contre le contenu hostile — pile d'éléments
  ouverts du tree-builder plafonnée à `MAX_DEPTH = 512`
  (les tags excédentaires sont ignorés, contenu inséré dans
  le parent courant : sans elle, `<div>`×10⁶ dépassait la
  pile d'appels des traversées récursives aval boxtree/
  display list) ; imbrication `@media`/`@supports` plafonnée
  à `MAX_NESTING = 32` (`nested_rules` est récursif — les
  blocs excédentaires sont avalés comme des at-rules
  inconnus). Corpus fuzz étendu : cibles `css` (parse +
  sélecteurs + cascade) et `url` (`parse`/`resolve`) en
  plus de `tokenizer` ; `fuzz/Cargo.toml` déclare son
  propre workspace. Benchmarks criterion :
  `crates/explorer-core/benches/pipeline.rs` — baseline
  page représentative (~80 éléments, 800×600) :
  `pipeline_navigate` ≈ 63 ms (fetch→PNG complet),
  `pipeline_scroll_repaint` ≈ 19 ms. 2 tests de bornes.

- **Étape 38 — formulaires** (`explorer-core`,
  `explorer-layout`, `explorer-render`, `explorer-css`) :
  champs natifs rendus par le moteur (ADR-0014) —
  `LayoutBox.control_checked` + enfant `Text` synthétique
  pour `value`/`placeholder`/libellé de bouton/option
  `selected`, marque pleine centrée pour `checkbox`/`radio`,
  feuille UA complétée (`select`, `button`, types `input`,
  `input[type=hidden]`). Soumission : clic sur
  `<button>`/`<input type=submit|image>` ou `Enter` dans un
  `<input>` → événement `submit` dispatché au miroir JS
  (`preventDefault` annule) → contrôles *successful*
  (nommés, non `disabled`, cases cochées, submitter seul) →
  `GET` query ou `POST` urlencodé via `HttpTransport` →
  pipeline de navigation complet + historique.
  `checkbox`/`radio` se basculent au clic (exclusivité
  `radio` par `name`), `<select>` cycle, `label[for]`
  délègue. Déviations documentées : popup `<select>` natif
  différé, `reload` d'un POST refait `GET`, pas de
  `multipart`, `password` non masqué. 6 tests session +
  `Enter` implicite.

- **Étape 37 — images `<img>`** (`explorer-layout`,
  `explorer-render`, `explorer-core`) : `LayoutBox.intrinsic`
  + `build_box_tree_sized`/`build_display_list_imaged` (les
  wrappers historiques sans table restent compatibles) ;
  `ImageTable` (pixels RGBA `Arc` par `NodeId`) et
  `DisplayItem::Image` — blit nearest-neighbor borné,
  alpha prémultiplié et clip respectés. `Session` fetch
  chaque `<img src>` via le `HttpTransport` injecté
  (résolution `Url::resolve`, statut 2xx requis), décode
  via le crate pur Rust `image` (png/jpeg/gif/webp), borne
  `SessionConfig::max_images`/`image_max_px` ; tout échec
  (réseau, format, borne) laisse le nœud hors table → le
  boxtree peint le texte `alt` (itemisé malgré
  `display:inline-block` — fix `collect_child`). Attributs
  `width`/`height` HTML honnêtement résolus axe par axe.
  Tests : unitaires render (décodage, blit, clip, garbage),
  boxtree (alt↔intrinsèque), session (PNG rouge ≡ bloc,
  alt cassé, borne 32 requêtes).

- **Étape 36 — boucle d'événements** (`explorer-js`,
  `explorer-core`) : `JsEngine::dispatch_event(ty, path)`
  — chemin `(tag, index same-tag)` depuis la racine,
  rejoué sur le miroir avec saut des wrappers implicites ;
  `Engine::dom_event_on` donne la propagation
  capture→cible→bulle complète (`addEventListener` +
  `onclick=`), microtasks drainées, `defaultPrevented`
  remonté → un clic `preventDefault` sur `<a>` supprime
  la navigation (événement avant action par défaut).
  `run_pending` post-dispatch draine les timers dus
  (`setTimeout(0)` au chargement rendu) ; mutations de
  listener → `sync_dom` → re-parse/relayout/
  `frame_invalidated` via `apply_dom_sync` factorisé ;
  erreur de listener → `pipeline_error` sans blocage.
  Non dispatchés à cette étape : `mouseover`, `keydown`,
  `focus`/`blur` ; tick de timers différés différé au
  prochain point de synchro (input/navigation).
  5 tests adaptateur + 3 tests session.

- **Étape 35 — bindings DOM par miroir synchronisé**
  (`explorer-js`, `explorer-core`) : `JsEngine` gagne
  `install_dom(html)` + `sync_dom()` (no-op par défaut).
  `DilloEngine` : le worker installe la surface DOM
  complète de `dillo-js` (`document`, `navigator`,
  `window`, `getElementById`, `querySelector`,
  `createElement`/`appendChild`/`textContent`/`innerHTML`,
  attributs…) sur un `dillo_js::Dom` parsé depuis le HTML
  source — **aucune ligne de binding réécrite**, la
  vérité script vit dans le miroir `Rc`. `sync_dom()`
  sonde le dirty bit ; si levé, le core re-parse
  `serialize_document()` dans `explorer-dom` → relayout
  → `frame_invalidated` (les `NodeId` générationnels ne
  traversent jamais la frontière — la génération reste
  interne à chaque arène). Le titre est relu au passage
  (`TitleChanged` si mutation indirecte de `<title>`).
  Limites documentées : miroir re-parsé (divergence
  possible vs corrections HTML5), `document.title=` non
  setter en amont, synchro en fin de navigation uniquement
  (boucle d'événements = étape 36). 4 tests adaptateur +
  2 tests session bout en bout (PNG muté ≡ PNG statique).

- **Étape 34 — intégration `dillo-js`** (`explorer-js`,
  `explorer-core`, `explorer-daemon`) : trait `JsEngine`
  (`Send`) — `eval` → complétion décrite, `run_pending`,
  `set_budget` ; `JsError` typé (`Syntax`/`Uncaught`/`Dead`).
  `DilloEngine` = **proxy canal → thread dédié par
  contexte** : le tas `Rc` de `dillo_js::Engine` ne quitte
  pas son thread, la `Session` reste `Send` derrière
  l'API, un script ne peut pas suspendre le pipeline ;
  `Shutdown` + `join` au `Drop`. `NullEngine` par défaut,
  `Session::set_js_factory` (fabrique injectée),
  `SessionConfig::js_ops` (budget 5M ops), `Tab.js` :
  contexte neuf par document — `<script>` inline
  (`type` classique, sans `src`) évalués dans l'ordre
  après parse ; un échec émet `pipeline_error` sans
  bloquer la page. Daemon branche `DilloEngine::factory()`.
  8 tests adaptateur + 2 tests session ; workspace
  (dillo-js inclus) clippy-propre.

### Fixed

- `dillo-js/regexp.rs` : 4 lints `unnested_or_patterns` +
  `single_match` (correction mécanique débloquant le
  clippy workspace — fichier WIP de l'équipe JS).

- **Étape 33 — pages d'erreur** (`app/`) : `pageErrors` par
  onglet alimenté par `pipeline_error` (SSE) et les échecs
  `navigate` — la zone frame affiche une page d'erreur
  stylée (icône danger, URL, message moteur, bouton
  réessayer) au lieu d'une frame vide ; effacée au prochain
  `navigated` de l'onglet.

- **Étape 32 — design system** (`app/`) : section `metrics`
  dans `tokens.json` (+ repli embarqué) : `tabHeight`,
  `tabMaxWidth`, `tabIndicatorWidth`, `statusBarHeight`,
  `iconSm/Md/Xl`, `progressSize/Stroke`. `buildTheme` couvre
  désormais icônes, tooltips, scrollbar, appBar, snackBar,
  boutons, sélection de texte, cartes, progress indicators —
  tout dérivé des tokens (couleurs, espacements, rayons,
  typographie avec graisse, motion, élévation). Barre de
  statut en bas de page : `href` survolé ou indicateur de
  champ actif. Plus aucune constante visuelle en dur dans
  `browser_page.dart`.

- **Étape 31 — interactions** (`explorer-core`, `explorer-api`,
  `explorer-layout`, `explorer-css`, `app/`) : champs de
  formulaire focalisables — clic sur `input`/`textarea` →
  `Focused`, `input key` édite `value` (caractère, Backspace,
  Enter/Escape sort du champ) puis restyle + repaint +
  `frame_invalidated` ; clic hors champ → blur ; navigation
  invalide le focus. `InputOutcome::Hover` expose `editable`
  (curseur texte) en plus de `url`. Feuille UA : `input`,
  `textarea`, `select`, `button` en `inline-block` avec
  bordure/taille. `<br>` devient une boîte texte `\n` →
  saut de ligne forcé réel. Flutter : curseur dynamique
  (lien/texte/flèche), `KeyboardListener` qui relaie les
  frappes quand `fieldFocused`, `sendInput` retourne le
  résultat JSON.

- **Étape 30 — viewport moteur** (`app/`) : `BrowserState.frames`
  (PNG par onglet) alimenté en pull sur `frame_invalidated` et
  après navigation ; `Image.memory(gaplessPlayback)` sans flash.
  Molette → `input scroll`, survol → `mousemove`, clic →
  `input click` (le moteur hit-teste et navigue via SSE).
  `LayoutBuilder` + debounce 150 ms → `POST …/viewport
  {w,h,scale}` avec déduplication de taille. `fetchFrame`
  tolère 204/vide (`null`). Le transport texture externe
  (ADR-0004) reste à l'étape 47.

- **Étape 29 — chrome navigateur Flutter** (`app/`) :
  `BrowserController` (`NotifierProvider` Riverpod 3) pilote
  le daemon via `BrowserRepository` (CRUD onglets, navigate,
  reload, back, forward — `BrowserTab.fromJson` mappe
  `can_back`/`can_forward`) et applique les événements SSE
  (`navigated`, `title_changed`, `frame_invalidated` →
  `frameTick`, `pipeline_error` → snackbar, `tab_closed`) ;
  `loading` optimiste à la soumission, `daemonOnline` prouvé
  par le bootstrap + bandeau « injoignable » + retry.
  `BrowserPage` : barre d'onglets (titre/spinner/fermeture +
  « + »), boutons nav conditionnés par `canBack`/`canForward`,
  omnibox synchronisée et complétion de schéma. Port daemon
  aligné sur 9377, `EventsClient` rend son délai de
  reconnexion annulable (timer purgé au `dispose`), repo
  enrichi (`listTabs`, actions, `DELETE` avec corps).
  6 tests : contrôleur (fake repo + SSE injectés) et widget
  fumée.

### Fixed

- `BrowserRepository.createTab` acceptait seulement un `id`
  texte — l'API renvoie un numérique (`'${id}'`).

- **Étape 28 — profil persistant (historique + signets)** :
  port `ProfileStore` injecté dans `Session` (`record_visit`,
  `load_bookmarks`, `save_bookmarks`, `history` — le core
  reste sans I/O) ; `Bookmark`/`Visit` en types publics ;
  signets en mémoire dédupliqués par URL persistés à chaque
  mutation, chaque navigation aboutie journalisée.
  `JsonFileStore` côté daemon : `history.json` (journal
  borné à 5000 entrées) + `bookmarks.json` dans le répertoire
  de profil (`EXPLORER_PROFILE`, sinon `%APPDATA%`/`XDG`/macOS
  par défaut), écriture atomique tmp+rename, tolérant aux
  fichiers corrompus. API : `GET/POST/DELETE /api/bookmarks`
  (409 dédup, 404 absent), `GET /api/history`. 2 tests
  (`MemStore` in-process, routes).
- **Étape 27 — daemon durci** : clé d'API optionnelle
  (`EXPLORER_API_KEY` → middleware `auth` sur `router_with_key` —
  `x-api-key` prioritaire, `?key=` en repli pour `EventSource`,
  401 au format du contrat sinon), arrêt propre
  `with_graceful_shutdown` (SIGINT, + SIGTERM unix) sans couper
  les requêtes en vol, logs `tracing` (démarrage, fontes,
  bind, clé). Feature tokio `signal` ajoutée.
- **Étape 26 — entrées et scroll** : `InputEvent`
  (`Click`, `MouseMove`, `Scroll`, `Key` — ignorée au MVP
  sans champs focalisables) et `InputOutcome` (`Ignored`,
  `Scrolled`, `Navigated`, `Hover`) ; `handle_input` fait le
  hit-test profond sur l'arbre posé (`hit_test` renvoie le
  `NodeId` de la boîte la plus imbriquée, `ancestor_href`
  remonte au `<a>` le plus proche, `Url::resolve` résout
  l'`href` relatif) — un clic sur un lien navigue, le survol
  rapporte l'URL pour le curseur. Scroll : `scroll_y` borné
  `[0, doc_h - viewport_h]` par onglet, `repaint` réémet la
  display list sans re-layout (`build_display_list` prend
  `scroll_y` ; les sous-arbres `position:fixed` peignent avec
  un offset nul — ancrés viewport) puis `frame_invalidated`.
  `POST /api/tabs/{id}/input` route les quatre types
  (`mousemove` coalescé côté client, ADR-0003). 6 tests
  (clic lien/hors-lien, hover, bornes de scroll, clavier,
  route API).
- **Étape 25 — contrat REST + historique de session** :
  `explorer-core` : `Tab` porte `title`, `hist`/`hist_idx`
  (historique local navigateur — `navigate` tronque le futur,
  `reload` re-fetch sans toucher l'historique, `go_back`/
  `go_forward` déplacent l'index) ; `TabInfo`/`tab_infos`
  (`url`, `title`, `can_back`, `can_forward`) exposés à l'API ;
  `extract_title` lit le premier `<title>` non vide ;
  événements `TabClosed` et `TitleChanged` ajoutés au bus
  (`Navigated` porte désormais le titre). `explorer-api` :
  `GET /api/status`, `GET /api/tabs`, `DELETE /api/tabs/{id}`,
  `POST …/reload|back|forward` — corps d'erreur du contrat
  `{"error":{"handled":true,"message":"…"}}` (ADR-0003),
  `UnknownTab` → 404, erreurs de pipeline → 502 ; SSE émet
  `tab_closed` et `title_changed`. 7 tests de contrat
  (`oneshot` : status, CRUD, back/forward/reload, format
  d'erreur) + 3 tests session (historique, titre, close).
- **Étape 24 — `explorer-core`/`explorer-api`/`explorer-daemon` :
  session et frame PNG servie** : `Session` (`session.rs`)
  orchestre le pipeline complet `fetch → décodage → parse →
  cascade → boîtes → layout → display list → raster → PNG`
  par onglet, derrière les ports injectés `HttpTransport` et
  `FontSource` (`Send + Sync` — session partagée derrière
  l'API). Onglets `TabId` monotones, viewport + DPI
  (`SessionConfig` — seuils en config, règle 13), re-render
  depuis l'état conservé sans re-fetch. Bus `broadcast`
  `CoreEvent` (`TabOpened`, `Navigated`, `FrameInvalidated`,
  `PipelineError`) — le pull PNG corrige les événements sauté
  par un abonné lent. `explorer-api` : routeur axum
  `POST /api/tabs`, `POST /api/tabs/{id}/navigate`,
  `POST /api/tabs/{id}/viewport`, `GET /api/tabs/{id}/frame`
  (`image/png`), `GET /api/events` SSE (JSON par événement,
  keep-alive 15 s) ; session derrière `tokio::Mutex`
  (single-writer à travers les `.await`). `explorer-render` :
  `DirFontSource` — scanne un répertoire `.ttf`/`.otf`,
  familles lues en table `name`, génériques CSS mappés sur
  les familles plateforme (`sans-serif` → Segoe UI/Arial…).
  `explorer-daemon` : assemble transport `TLS → H1 → décodage
  → redirections` + fontes système + session + routeur, bind
  **loopback strict** (ADR-0003), config par env
  (`EXPLORER_PORT`, `EXPLORER_FONT_DIR`, `EXPLORER_MAX_BODY`).
  12 tests : pipeline bout-en-bout via `FakeTransport`
  in-process (PNG signé, événements ordonnés, re-render sans
  re-fetch) + routeur `oneshot` (PNG `200`, `204` avant
  navigation, SSE `text/event-stream`, `400`).
- **Étape 23 — `explorer-render` : rasterisation tiny-skia**
  (`raster.rs`, dep `tiny-skia` 0.11 + `ttf-parser`) :
  `rasterize` peint la display list dans un framebuffer
  RGBA prémultiplié — `FillRect`/`Border` en rectangles
  pleins, `Text` via `FontSource` + `Face::outline_glyph`
  → path tiny-skia rempli à la baseline (adaptateur
  `OutlineBuilder`→`PathBuilder`, avances/offsets
  multipliés par le facteur DPI), `Placeholder` en cadre
  gris. `PushClip`/`PopClip` → `Mask` empilées (clipping
  réel du `overflow`). `scale` HiDPI : géométrie × scale,
  framebuffer en pixels physiques, borne 16384 px (page
  hostile). `Frame::encode_png` prêt pour l'API (étape 24).
  Fixture TTF enrichie : glyphes 1-255 = rectangle 4 points
  (coords TrueType = flux x/y séparés) — la première fonte
  réellement dessinable. 6 tests (pixels sondés, clip, DPI,
  signature PNG).
- **Étape 22 — `explorer-render` : display list**
  (`display.rs`) : `build_display_list` aplatit l'arbre de
  layout en `Vec<DisplayItem>` ordonnés — `FillRect` (fond
  `background-color` non transparente), `Border` (4 côtés,
  épaisseurs/couleurs/styles par côté), `Text` (run shapé +
  origine baseline + `color` hérité), `PushClip`/`PopClip`
  (padding box, `overflow` ≠ `visible`), `Placeholder`.
  Ordre de peinture borné (CSS 2.1 App. E simplifié) : self
  → enfants non positionnés dans l'ordre → positionnés
  triés stablement par `z-index`. `DisplayList::dump` texte
  pour golden headless. Fixture TTF déplacée dans
  `explorer-test-support::font_fixture` (source unique pour
  layout/render, sans dépendances internes). 6 tests.
- **Étape 21 — `explorer-layout` : tables, flottants,
  positionnement** (`tables.rs`, `block.rs`, `boxtree.rs`) :
  `LayoutBox` porte les métadonnées usées `position`/`float`/
  `overflow`/`z_index`. Tables — auto layout CSS 2.1 borné :
  grille `table-row`/`table-cell` collectée en traversant les
  groupes, largeurs de colonnes min (plus long mot mesuré) et
  pref (contenu mesuré) via la fonte injectée, `width` de
  cellule en plancher, distribution proportionnelle des
  surplus, hauteurs de rangée = max des cellules uniformisées
  en seconde passe ; `display` de la feuille UA corrigés
  (`table`, `table-row`, `table-row-group`). Flottants —
  `float: left|right` posés au bord de la bande courante
  sans avancer le curseur, bandes réservées rétrécissant les
  frères ; `clear: left|right|both` descend sous leurs bas.
  Positionnement — `relative` décale post-flux sans sortir du
  flux ; `absolute`/`fixed` sortis du flux (rect statique
  placeholder) et résolus par `resolve_absolutes` en second
  passage contre le padding box du plus proche ancêtre
  positionné, viewport pour `fixed` ; `top/right/bottom/left`
  en base du bloc conteneur, ancrage opposé sans offset
  primaire. `z-index` parsé (i32), `overflow` lu des axes
  `overflow-x`/`-y` (shorthand étape 16) avec la valeur la
  plus restrictive. Limites documentées : bandes prises au
  bord haut de l'enfant (pas ligne par ligne), flottants
  bornés au contexte du bloc parent, pas de
  `border-collapse`/`rowspan`/`colspan`. 11 tests
  (tables, floats/clear, positions, métadonnées).
- **Étape 20 — `explorer-layout` : inline layout, line
  boxes** (`inline.rs`) : `LineBox`/`Fragment` produits des
  items en-ligne — texte découpé sur les opportunités UAX #14
  de l'itemisation (pièces), mesuré et shapé par la fonte
  injectée ; wrapping greedy, blancs de tête de ligne
  supprimés ; la fin de texte (UAX #14 mandatory) distingue
  les vraies coupures dures (`\n`, NEL, LS/PS). `text-align`
  left/right/center/justify (étirement inter-fragments).
  Fragments posés sur la baseline commune de la ligne ;
  `display: inline-block` coulé comme atome incassable
  (dimensionné par `layout_block`). Rects des enfants =
  union de leurs fragments ; clusters octets conservés pour
  le hit-test. Runs RTL en ordre logique (réordonnancement
  visuel reporté à la display list — documenté). 6 tests.

- **Étape 19 — `explorer-layout` : block layout, flux
  normal** (`block.rs`) : résolution horizontale §10.3.3 —
  `width` spécifiée/auto, slack aux marges `auto` (centrage),
  `margin-right` reprend la surcontrainte LTR ; `%` et
  `em`/`rem`/`ex`/`ch` résolus (base font-size chaînée par
  parent). Empilement vertical avec **marges collapsées**
  §8.3.1 (positives→max, négatives→min, mixte→somme ;
  premier/dernier enfant adjoining traversent le parent
  sans padding/bordure). `height` auto = somme du flux,
  `min|max-*` bornent. Blocs anonymes : lignes estimées par
  `break_lines` (hauteurs seules — la pose inline arrive à
  l'étape 20), mesure via `FontSource` injectée ou repli
  métrique `0.55em` documenté (headless uniquement). 9 tests.

- **Étape 18 — `explorer-layout` : stack texte et fontes
  (ADR-0009)** (`text.rs` + `font.rs`) : port `FontSource`
  (données de fontes — `fontdb` reste en infra, injecté par
  `explorer-core`), `FontRequest`/`FontMetrics`/`Glyph`/
  `ShapedRun`. Métriques réelles via `ttf-parser`, shaping
  via `rustybuzz` (`shape`/`measure`, direction RTL).
  `itemize` : segmentation bidi UAX #9 par niveau
  (`unicode-bidi`) + opportunités de coupure UAX #14
  (`unicode-linebreak`) recalées au run. `break_lines` :
  greedy avec blancs de fin hors mesure, débordement de mot
  géant sans coupure forcée (`overflow-wrap: normal`),
  progression stricte garantie. **Fonte fixture embarquée**
  (`tests/common`) : TTF minimal construit en code
  (`unitsPerEm` 1000, ascender 800/descender -200, cmap
  format 4) — métriques réelles en headless, aucune mesure
  fixe approximative. 11 tests.

- **Étape 17 — `explorer-layout` : arbre de boîtes**
  (`boxtree.rs`) : `LayoutBox`/`BoxKind`
  (block/inline/list-item/texte/anonyme) + lien
  `node: Option<NodeId>` ↔ boîte, `Rect` (géométrie remplie
  par les étapes 19+). `build_box_tree` : `display:none`
  saute le sous-arbre, texte porte le style du parent,
  enveloppes anonymes CSS 2.1 §9.2.1.1 avec troncature des
  blancs purs en bord de run (l'espace entre inlines
  survit). `dump` wireframe + commande `explorer-cli layout`
  (pipeline fetch → parse → style → boîtes, `<style>`
  embarqués collectés — feuilles liées hors périmètre).
  8 tests.

- **JS-C1 — lexique/parser conforme test262** (`dillo-js`) :
  **836/859 (97 %), 23 échecs résiduels** — progression
  531 → 836 sur le sous-ensemble JS-B.
  - identifiants Unicode complets : bascule sur `unicode-ident`
    (tables UAX #31 compilées) + `Other_ID_Start`/`_Continue`
    + `$`/`_`/ZWNJ/ZWJ — corrige `char::is_alphabetic` trop
    permissif (U+2E2F `Lm`∩`Pattern_Syntax`) et trop pauvre ;
    échappes `\uXXXX`/`\u{…}` en identifiant, mots réservés
    échappés rejetés ;
  - trivia : U+FEFF whitespace, LS/PS/CR terminateurs
    (ASI + comptage de lignes), commentaires HTML Annex B
    (`<!--` partout, `-->` en début de ligne), commentaire
    `/*` non terminé = erreur ;
  - littéraux : `\u{…}` + paires de surrogates `\uD8xx\uDCxx`
    fusionnées dans les chaînes/templates ; octales legacy
    `\1`–`\377` sloppy ; terminateur brut dans regex = erreur ;
    `3in`/exposants tronqués rejetés ; regex validée **au
    parse** (SyntaxError eager, spec `negative.phase: parse`) ;
  - traduction `regexp` : `\u{…}`→`\x{…}`, paires de
    surrogates fusionnées en mode `u`, `\0`→`\x{0}`,
    `\cX`→contrôle, identity escapes Annex B sloppy
    (`\y`→'y' — strict en `u`), `\k<nom>`→`(?P=nom)` ;
  - mode strict : directive prologue `"use strict"` scannée au
    niveau script et corps de fonction (héritage vers
    l'intérieur), `Token::legacy_octal` flaggé par le lexer →
    early errors (octales numériques/`08`/`09`, échappes
    `\8`/`\9`/octales), `delete` d'identifiant nu, liaisons
    `eval`/`arguments`/mots réservés strict — le harnais
    test262 place désormais la directive en tête de script
    (runner officiel idem) ;
  - résidu documenté (hors scope C1) : attributs de
    propriétés (writable/enumerable/configurable),
    `ReferenceError` strict au runtime, backrefs/lookahead
    regex, modificateurs ES2025, `eval` héritant le strict.
  15 tests de régression `tests/lang.rs`.
- **JS-C2 — attributs de propriétés + strict runtime**
  (`dillo-js`) : **851/859 (99 %), 8 échecs résiduels** —
  progression 836 → 851.
  - modèle : `PropAttrs` `{writable, enumerable, configurable}`
    sur `Prop::Data`/`Prop::Acc` + `[[Extensible]]` (remplace
    `enumerable` seul + `frozen`) — `[[DefineOwnProperty]]`
    simplifié (fusion des champs absents, `writable` ne peut
    que rétrécir sur prop verrouillée), `[[Set]]`/`[[Delete]]`
    ordinaires respectant flags/extensibilité/accès proto ;
  - API : `Object.defineProperty` descripteur complet
    (`value`/`writable`/`enumerable`/`configurable`/`get`/`set`,
    data-vs-accessor exclusif), `defineProperties`,
    `getOwnPropertyDescriptor(s)`, `create(proto, map)`,
    `setPrototypeOf` (+ rejet de cycle et non-extensible),
    `freeze`/`seal`/`preventExtensions` + `isFrozen`/`isSealed`/
    `isExtensible`, `isPrototypeOf` — constantes verrouillées
    `{w,e,c} = false` (`undefined`, `NaN`, `Infinity`, `Math.*`,
    `Number.*`), `regexp.lastIndex` `{w:true,e,c}` non
    configurable, `var` global non-configurable ;
  - strict runtime : `FnLit::strict`/`Program::strict` propagés
    au moteur (corps de classe stricts par spec), écriture sur
    identifiant non déclaré → `ReferenceError`, écriture refusée
    (non-writable own/héritée, accessor sans setter, primitive,
    objet gelé) → `TypeError` strict vs silence sloppy, `delete`
    non-configurable → `TypeError`, assignation `const` →
    `TypeError` dans les deux modes, `this` coercé vers le
    global en sloppy (`undefined` préservé en strict), `eval`
    direct hérite du strict appelant (`parse_in` — early errors
    comprises) ;
  - fix : `TextEncoder`/`TextDecoder`/`Symbol` installaient
    `constructor` sur `Object.prototype` partagé —
    `objInstance.constructor` retournait le mauvais ctor ;
  - `delete ident` : liaison d'env record → `false`, sinon
    `[[Delete]]` sur l'objet global (global implicite
    configurable → `true`) ;
  - résidu (8 tests) : backrefs/forward-refs regexp (`\1`,
    `\k<a>` avant `(?<a>…)`), lookahead, modificateurs ES2025,
    case-mapping non-`u`, validations `u` profondes — limites
    du crate `regex` documentées.
  3 tests de régression `tests/lang.rs` (descripteurs,
  freeze/seal, strict runtime).
- **JS-C3 — RegExp complètes : `fancy-regex` + validateur
  ECMA-262** (`dillo-js`) : **859/859 (100 %)** — progression
  851 → 859.
  - backend `regex` → `fancy-regex` (backtracker hybride pur
    Rust, `BACKTRACK_LIMIT = 100_000` contre les patterns
    hostiles) — lookahead/lookbehind `(?= (?! (?<= (?<!` et
    backrefs `\1`–`\9`/`\k<nom>` compilés nativement ;
  - `translate_pattern` réécrit en validateur ECMA-262 (le
    backend est plus permissif que la spec) : noms de groupes
    validés par les tables UAX #31 du lexer (wrappers
    `is_ident_*_pub`), doublons rejetés, `\k<nom>` référençant
    un groupe déclaré plus loin → `(?:)` (match vide, spec
    21.2.2.9), `\k` malformé/nu → `SyntaxError` dès qu'un
    groupe nommé existe ou en `u` (identity Annex B sinon),
    `\N` hors bornes → préfixe valide/octale legacy/identité
    `\8`/`\9`, `\0`+chiffre → octale `\07` (erreur en `u`) ;
  - syntaxe : modificateurs inline `(?ims:`/`(?ims-ims:` ES2025
    validés (flags `ims`, disjoints, `:` obligatoire — `(?u:`,
    `(?P`, `(?#` rejetés), quantificateurs `{n,m}` sans atome
    ou `n>m` rejetés, `]`/`{`/`}` orphelins erreurs en `u`,
    ranges de classes avec borne class-escape (`[--\d]`)
    erreur en `u` et `-` littéral sloppy, ranges vides
    `[z-a]` toujours rejetées ;
  - échappements : `\xNN`/`\uHHHH` consommés entièrement,
    `\u{…}` → `\x{…}` normalisé (≤ U+10FFFF), paires de
    surrogates fusionnées en `u`, `\cX` contrôle, `\c` seul →
    `\\c` sloppy, identités Annex B sloppy vs strict `u` ;
  - fold `i` non-`u` manuel : `Canonicalize` = `ToUppercase`
    simple — `[cCU]` explicites au lieu du folding Unicode du
    backend, donc `/\u{212a}/i` ne matche ni 'k' ni 'K'
    (KELVIN) ; ranges foldées par paires de bornes gardées
    (`upper(v) == upper(borne)`) ; backrefs wrappées `(?i:…)`.
  16 assertions de régression `tests/lang.rs`
  (`regexp_js_c3_semantics`).
- **JS-C10 — Symboles bien connus, `with`, `Reflect` complet**
  (`dillo-js`) :
  - `Symbol.toPrimitive` honoré dans `ToPrimitive` avec hint
    spec à trois états (`"default"` pour `+`/`==`, `"number"`
    pour l'arithmétique, `"string"` pour `ToString`) — retour
    non-primitif → `TypeError` ;
  - `Symbol.hasInstance` lu **avant** le garde callable dans
    `instanceof` (spec `InstanceofOperator`) ;
  - `Symbol.toStringTag` dans `Object.prototype.toString`
    (propre ou hérité) ;
  - `Symbol.unscopables` installé sur `Array.prototype` (17
    noms spec) et honoré par `with` ;
  - `with (o)` : `Stmt::With` + `EnvKind::With(ObjId)` —
    résolution des noms via les props de `o` (proto incluse,
    filtre `@@unscopables`), écriture sur prop existante sinon
    scope parent, `var` hoiste au scope fonction, primitives
    enrobées dans un wrapper jetable, `null`/`undefined` →
    `TypeError`, `SyntaxError` en strict ;
  - `Object.getOwnPropertySymbols` + exclusion des clés-
    sentinelles de `getOwnPropertyNames` ;
  - `Reflect` complété à 13 méthodes : `defineProperty`,
    `deleteProperty`, `getOwnPropertyDescriptor`,
    `getPrototypeOf`, `setPrototypeOf`, `isExtensible`,
    `preventExtensions` — `false` sur refus plutôt que throw
    (déviation : une trap lancée est aussi rendue `false`) ;
  - test262 : 859/859 conservé ; 7 tests de régression.
- **JS-C9 — Générateurs eager** (`dillo-js`) : `function*` /
  `yield` / `yield*`, méthodes génératrices `*m(){}` (objet et
  classe), `async function*`. Le corps tourne à l'appel en
  empilant un collecteur `Engine::gen_collect` — pas de
  suspension (déviation documentée cohérente avec `await`
  settled-only : `next(v)` ignore `v`, l'expression `yield`
  vaut `undefined`, générateur infini → budget d'opérations).
  `ObjKind::Iter` étendu d'un champ `ret` (valeur terminale
  `{value, done:true}` du `return`), `gen_iter_obj` pour les
  appels ; proto itérateur enrichi : `return(v)` (clôture),
  `throw(e)` (relève). Parser : flag `in_gen` save/restore
  (flèches et get/set réinitialisés), `yield` redevient
  contextuel (hors table de mots-clés — identifiant sloppy,
  `SyntaxError` hors générateur), pas de saut de ligne avant
  l'opérande. `for await` sur générateur = fallback sync avec
  valeurs awaitées (spec `AsyncFromSyncIterator`). test262
  reste 859/859 ; 1 test de régression `lang.rs`.
- **JS-C8 — APIs modernes** (`dillo-js`) :
  - `AggregateError` (prototype dédié, `errors` own prop) +
    `Error(m, {cause})` sur toute la famille Error ;
    `Promise.withResolvers()`/`Promise.try()`/`Promise.any`
    collectant les raisons de rejet dans `errors` ;
  - `structuredClone` — copie profonde spec
    `StructuredSerialize` : cycles et aliasing préservés
    (table id→clone), objets ordinaires (props énumérables
    propres), tableaux, `Map`/`Set`, `ArrayBuffer`/`DataView`/
    vues typées (buffers dupliqués), `RegExp` (source+flags) ;
    `Fun`/`Symbol`/`Promise`/`Proxy`/`Iter`/host/`FetchResponse`
    → `TypeError` (classe `DataCloneError`) ;
  - `Object.groupBy` (objet à prototype nul, clés `ToString`)
    et `Map.groupBy` (clés `SameValueZero` — pas de `ToString`),
    callbacks `(item, index)` ;
  - `Array.fromAsync` — `await` par élément puis par résultat
    de mapfn ; jamais de throw synchrone (erreurs → promesse
    rejetée, spec async) ;
  - RegExp : flag `d` → `hasIndices` + `match.indices`/
    `indices.groups` en unités char (groupes non-participants
    → `undefined`), ordre canonique `dgimsuy` ; lookbehind
    `(?<=)`/`(?<!)` natif fancy-regex ;
  - `Symbol.asyncIterator` + `for await` — `next()` awaité
    par étape, fallback itérable sync avec valeur awaitée
    (`CreateAsyncFromSyncIterator`), `for await … in` →
    `SyntaxError` ; `await_value` extrait d'`Expr::Await`
    (partagé par `fromAsync`/`for await`) ;
  - Iterator helpers ES2025 (`map`/`filter`/`take`/`drop`/
    `flatMap`/`toArray`/`forEach`/`reduce`/`some`/`every`/
    `find`) — drain eager en snapshot `Iter` partagé
    (paresse spec → déviation documentée de timing) ;
  test262 reste 859/859 ; 7 tests de régression `lang.rs`.
- **JS-C7 — `BigInt` + `BigInt64Array`/`BigUint64Array`**
  (`dillo-js`) : primitive `Value::Big(ObjId)` →
  `ObjKind::BigInt` (`num-bigint`, tracée GC). Littéraux
  `123n` et radix (`0xFn`/`0o17n`/`0b101n`) ; formes
  malformées rejetées (`08n`, `123n4`, `1.5n`, `1e3n`).
  Sémantique : `typeof` → `"bigint"`, arithmétique exacte
  (`/` tronquée, `%`, `**`, `-`/`~`/`++`/`--`), bitwise avec
  `>>>` interdit, `TypeError` sur mixte bigint/number,
  comparaisons relationnelles mixtes, `==`/`===` spec
  (`1n == 1` vrai, `1n === 1` faux), concat string,
  `to_number`/`to_js_string`/`prop_key`/`describe` intégrés,
  `0n` falsy. `ToBigInt` : chaîne non-parsable →
  `SyntaxError` (`0x`/`0o`/`0b` reconnus), number non-entier
  → `RangeError`. Builtins : `BigInt()` (`new` → `TypeError`),
  `asIntN`/`asUintN` (mod 2^bits), proto `toString(radix)`/
  `valueOf`, `Number(1n)` (perte documentée > 2^53),
  `JSON.stringify` → `TypeError`, `[object BigInt]`.
  `TaKind::I64`/`U64` — éléments `BigInt` (mod 2^64,
  complément à deux) : refactor `ta_nums`/`src_nums`/
  `ta_from_nums` → `ta_vals`/`src_vals`/`ta_from_vals`
  (`Vec<Value>`, `Big` préservés, `conv_vals`/`write_conv`
  convertissent avant le `&mut` du buffer), `reverse`/
  `copyWithin` au niveau octet, `sort` comparateur big-aware,
  `indexOf`/`includes` en `StrictEqual`/`SameValueZero` sans
  conversion. `DataView` `get`/`setBigInt64`/`BigUint64`
  endianness paramétrable. `find_own`/`find_prop`/`get_prop`/
  `env_get`/`ta_elements` passés `&mut` (matérialisation des
  `Value::Big`), `set_prop` → `JsResult` pour propager les
  `TypeError` de conversion (`u8[0]=1n`, `b64[0]=1` lèvent,
  spec `SetValueInBuffer`). test262 reste 859/859 (sous-
  ensemble extrait) ; 7 tests de régression `lang.rs`.
- **JS-C6 — `ArrayBuffer`/`DataView`/vues typées** (`dillo-js`) :
  `ObjKind::ByteBuf` (buffer d'octets fixe, proto
  `ArrayBuffer.prototype`), `ObjKind::DataView {buffer, offset,
  len}`, `ObjKind::TypedArray {ta, buffer, offset, len}` —
  les vues référencent leur buffer (tracé GC, la vue retient
  la mémoire). `TaKind` : 9 kinds (`Int8`/`Uint8`/`Uint8Clamped`/
  `Int16`/`Uint16`/`Int32`/`Uint32`/`Float32`/`Float64`) avec
  `size`/`name`/`get`/`set` — little-endian vues, wrap modulo
  entiers (`to_int32`/`to_uint32`), saturation arrondie
  `Uint8Clamped`, `NaN`→0. Indices canoniques et props
  `length`/`byteLength`/`byteOffset`/`buffer`/`BYTES_PER_ELEMENT`
  synthétisés (`find_own`/`set_prop`/`delete`/`own_keys`/
  `iter_values`), région `byteOffset` honorée dans toutes les
  lectures/écritures. Ctors : `new TA(len)`, `new TA(buffer,
  byteOffset?, length?)` (alignement/`RangeError` spec),
  `new TA(iterable|array-like|view)`, `TA.from(src, mapfn?)`,
  `TA.of(...)` ; `ArrayBuffer(len)`/`slice`/`isView` ;
  `DataView(buffer, off?, len?)` + `get/set{Int,Uint,Float}8/16/32/64`
  endianness paramétrable (BE défaut). Méthodes vues : `set`,
  `subarray` (partage le buffer), `fill`, `slice`, `map`, `filter`,
  `forEach`, `join`/`toString`, `indexOf`/`lastIndexOf`/`includes`
  (SameValueZero), `every`/`some`/`find`/`findIndex`, `at`,
  `reduce`/`reduceRight`, `reverse`, `sort` (numérique par défaut,
  comparateur JS optionnel), `copyWithin`, `keys`/`values`/`entries`/
  `[Symbol.iterator]`. `TextEncoder.encode` → vrai `Uint8Array`,
  `TextDecoder.decode` lit `Uint8Array`/`DataView`/`ArrayBuffer`.
  Déviations : props = data props (spec : accesseurs proto),
  pas de `BigInt64Array`/`BigUint64Array`/`Atomics`/resizable/
  `SharedArrayBuffer`/`transfer`. 10 tests `tests/lang.rs`
  (construction, conversions, partage buffer, offsets, endianness,
  itération, GC, TextEncoder/Decoder). test262 toujours 859/859.
- **JS-C5 — Modules ES** (`dillo-js`) : `parse_module` (but
  module : `import`/`export` top-level, toujours strict) ;
  **live bindings** via `EnvCell::Alias(EnvRef, name)` —
  l'importeur lit l'env de l'exporteur à chaque accès
  (mutations visibles), écriture sur import → `TypeError`,
  chaînes d'alias bornées par visited-set (ré-exports
  cycliques). Instanciation DFS : module enregistré avant
  récursion (cycles re-résolvent au même index), exports
  *locaux* statiques d'abord, `pending` (`export {…} from`,
  `export *`) résolus une fois les deps instanciées ;
  `ResolveExport` → `SyntaxError` précoce sur nom manquant.
  Évaluation post-order avec `Evaluating`/`Evaluated` pour
  les cycles. Namespace exotique : proto nul, getters bornés
  sur `(env_index, binding)` résolvant live. `import(spec)`
  → promesse du namespace, `import.meta.url`. Chargeur hôte
  `ModuleLoader` injecté (`Engine::set_module_loader`,
  `Engine::eval_module`). GC : envs de modules tracés,
  cellules `Alias` remontent l'env cible. 15 tests
  `tests/lang.rs` (imports nommés/default/namespace, live
  binding, ré-exports, cycle, immuabilité, strict, `import()`,
  `import.meta`). test262 toujours 859/859.
- **JS-C4 — `Proxy` complet** (`dillo-js`) : `ObjKind::Proxy
  {target, handler}` (tracé GC, `handler: None` = révoqué) — les
  13 méthodes internes routées : `get`/`set`/`has`/`deleteProperty`
  (chemins `read_prop`/`write_prop`/`has_prop`/`delete_result`),
  `ownKeys` + `getOwnPropertyDescriptor` (énumérations
  `Object.keys`/`getOwnPropertyNames`/`Reflect.ownKeys`/`for-in`/
  spreads/rest — versions `*_trapped`), `defineProperty`
  (`Object.defineProperty`/`defineProperties`/`create`),
  `get/setPrototypeOf`, `isExtensible`/`preventExtensions`
  (`seal`/`freeze` délégués), `apply`/`construct` (`call_value`/
  `construct` — `typeof`/`is_callable`/`instanceof` suivent la
  cible). `Proxy.revocable` + `revoke` (`BoundNative` lié au proxy
  → handler nul → `TypeError` sur toute opération), `Proxy()`
  sans `new` → `TypeError`, traps non-callables → `TypeError`,
  `ownKeys` non-tableau/doublon → `TypeError`. test262 toujours
  859/859. 15 assertions `tests/lang.rs` (`proxy_traps`).
- **JS-B — harness test262 (ADR-0010)** : extrait versionné de
  la suite ECMAScript officielle @ `d53ebc8` (1077 fichiers,
  sous-ensembles `test/language/{types,literals,comments,
  white-space,line-terminators,identifiers,hashbang}` +
  `harness/`), frontmatter `flags`/`features`/`includes`/
  `negative` exécuté, skips whitelistés comptés, cliquet
  `BASELINE_PASSED = 531` — **score initial 531/859 (62 %)**,
  218 skips (BigInt, champs privés, séparateurs numériques,
  hashbang, modules…), 328 échecs documentés.
- **fix(dillo-js)** : wrappers `new String/Number/Boolean` sans
  `[[PrimitiveValue]]` — `ToPrimitive`→`toString`→`ToString`
  bouclait à l'infini (trouvé par test262 `S8.7_A4`). Ajout de
  la prop cachée `__prim`, `valueOf`/`toString` stricts
  (`TypeError` sur receiver incompatible, spec) et `__prim` sur
  les prototypes eux-mêmes.
- **JS-A — GC mark-sweep `dillo-js` (ADR-0008)** : l'arène
  `ObjId` ne « meurt plus avec la page » —
  - `Heap` : tombstones + freelist (réutilisation de slots),
    rooting explicite ref-counté (`add_root`/`remove_root`),
    `is_alive`/`is_rooted`/`live_count` ;
  - `Engine::collect` : mark itératif depuis toutes les racines
    (global, prototypes, env global, `Symbol.for`, timers,
    microtasks, XHR/fetch en vol, cache DOM↔JS + listeners,
    hooks, complétion) puis sweep + `Host::finalize` ;
  - trace : proto, props data/accesseurs, éléments de tableau,
    `Map`/`Set`/itérateurs, promesses + réactions + jobs,
    envs capturés par `Fun::Script` (visited-set anti-cycle),
    hooks `Host::trace`/`finalize` pour les objets hôtes ;
  - contrat : `collect` entre deux jobs — la pile d'appel JS
    n'est pas une racine ; les handles hôtes stables passent
    par les racines explicites, un `ObjId` obsolète résout
    `None` (jamais d'accès dangling) ;
  - 13 tests `tests/gc.rs` : cycles, fermetures, timers,
    promesses, cache DOM, racines explicites, idempotence.

- **Étape 16 — `explorer-css` : propriétés typées +
  shorthands + pseudo-classes dynamiques** (`value.rs`,
  ~560 lignes) : `Length`/`Unit` (px + absolus avec facteur,
  relatives `em`/`rem`/`ex`/`ch` + viewport `vw`/`vh`/`vmin`/
  `vmax` différées à la used value), `Color` RGBA (`#rgb`,
  `#rrggbb[a/a]`, `rgb()/rgba()`, ~50 noms + `transparent`),
  exposants `2e3` gérés à la découpe nombre/unité. Expansion
  des shorthands à la cascade : `margin`/`padding` (1-4
  valeurs), `border`/`border-<side>` (ordre libre
  width/style/color), `border-width|style|color`, `overflow`,
  `background` (couleur + image), `font` (style/variant/
  weight/size[/line-height]/family), `list-style`. Accesseurs
  typés sur `ComputedStyle` : `length`/`color`/`number`
  (`currentcolor` résolu via `color`). Pseudo-classes
  dynamiques (`:hover`, `:focus*`, `:checked`, `:disabled`,
  `:target`…) : `matches_with_state`/`compute_with_state`/
  `compute_tree_with_state` pilotées par une fonction d'état
  externe — couture restyle ADR-0012 ; `:link` structurel sur
  `a[href]`/`area[href]`, `:visited` toujours faux (pas de
  tracking, conformité vie privée). 10 tests + corrections
  d'attentes (`:hover` désormais valide, alpha rgba arrondi).
  *Mesure WPT selectors/cascade : reste à brancher dans le
  chantier conformance (ADR-0010).*

- **Étape 15 — `explorer-css` : cascade + héritage +
  computed style** (`cascade.rs`, ~500 lignes) : ordre de la
  cascade **UA normal < auteur normal < auteur `!important` <
  UA `!important`**, à rang égal spécificité puis ordre source ;
  `StyleEngine` accumule les feuilles (`add_css`/`add_stylesheet`,
  origine typée) avec sélecteurs pré-parsés. Mots-clés CSS-wide
  `inherit`/`initial`/`unset`/`revert` ; table des propriétés
  héritées (custom `--*` incluses) + table `initial_value`
  couvrant les propriétés de l'étape 16. Media queries
  évaluées sur `StyleEnv` (type, `min/max-width`, `-height`,
  `orientation`, négation, imbrication AND) — une règle sous
  `@media` non vérifié est sautée. **Feuille UA intégrée**
  (`UA_CSS`, défauts HTML Rendering) écrite en CSS et parsée
  par notre propre pipeline (dogfooding) ; `compute` (nœud
  seul) et `compute_tree` (mémoïsation parent→enfant, O(n)).
  16 tests : origines, `!important`, héritage explicite,
  `unset`, media queries, handles obsolètes.

- **Étape 14 — `explorer-css` : sélecteurs + matching DOM**
  (`selector.rs`, ~700 lignes) : parsing depuis le flux de
  tokens — type, `*`, `.classe`, `#id`, `[a]`/`[a=v]`/`~=`/
  `|=`/`^=`/`$=`/`*=` + drapeau `i`, combinateurs descendant,
  `>`, `+`, `~`, listes `,` ; pseudo-classes structurelles
  `:root`, `:empty`, `:first/last/only-child`, `:nth-child()`,
  `:nth-last-child()`, les quatre `*-of-type`, `:not()`, `:is()`,
  `:where()` — dynamiques (`:hover`…) et pseudo-éléments
  réservés à l'étape 16 (présence → sélecteur invalide, règle
  ignorée). `an+b` complet (`odd`/`even`/`-n+3`). Matching
  droite→gauche sur `explorer-dom` via ses accesseurs
  `Result` (handle obsolète → non-match, jamais de panic) ;
  combinateurs frères ignorant les nœuds texte. Spécificité
  `(id, classe, type)` exposée pour la cascade. 17 tests :
  parsing, ops attributs, nth, combinateurs, matching réel,
  handles obsolètes.

- **Étape 13 — `explorer-css` : tokenizer + parser**
  (`tokenizer.rs`/`parser.rs`, ~1 600 lignes) : tokens CSS
  Syntax (ident, function, nombre/dimension/pourcentage,
  chaîne, url, hash, CDO/CDC, delim) avec échappements résolus
  et `BadString`/`BadUrl` en dégradé ; buffer d'avance
  `VecDeque` (look-ahead 2-3 chars) ; borne `MAX_TOKENS`
  anti-contenu-hostile. Parser tolérant : feuille de style →
  règles qualifiées + déclarations ordonnées, `!important`
  détecté/retiré, récupération spec (saut à `;`/`}` avec
  profondeur, blocs non fermés à EOF avalés, garde
  anti-progression) ; at-rules `@charset`, `@import`,
  `@font-face`, `@keyframes` (règles imbriquées), at-rules
  inconnues avalées sainement ; `@media` minimal : type,
  conditions `(min-width: 600px)`, négation `not`, listes `,`.
  32 tests (14 tokenizer + 18 parser) couvrant nombres
  signés/exposants, `url()` brut vs `url("…")`, `calc(`,
  déclarations invalides sautées sans perte des suivantes.

- **Étape 12 — golden tests + premier artefact** :
  - Corpus **html5lib-tests tree-construction** (57 fichiers
    `.dat`, ~1 600 cas) extrait @ `9329e64694` vers
    `tests/data/tree-construction/` — harness
    `tests/tree_construction.rs` avec cliquet
    `BASELINE_PASSED = 953` (**59 %**, 200 cas fragment/
    script-on ignorés) ; échecs concentrés sur l'adoption
    agency complète, conformément aux écarts documentés.
  - `explorer-cli` : `dump <fichier|url>` (dump DOM format
    html5lib, sniffing d'encodage, pile `explorer-net`
    TLS+redirections+décodage) et `fetch <url>` — jalon J2.
  - `explorer-html::debug::dump` : sérialiseur partagé.
  - Spike **ADR-0008** (`explorer-dom/tests/rooting_spike.rs`) :
    faux heap JS validant rooting explicite, réclamation
    `collect`, `StaleHandle` sur slot réalloué, détachement
    sans destruction.
  - `fuzz/` : cible cargo-fuzz du tokenizer + tree-builder
    (chunks de 7 octets, hors workspace).

- **Étape 11 — `explorer-html` : tree-builder HTML5**
  (`tree.rs`, ~2 100 lignes) : pipeline token → `explorer-dom::Dom`
  avec les modes d'insertion de la spec — Initial, BeforeHtml,
  BeforeHead, InHead, AfterHead, InBody, Text, InTable,
  InCaption, InColumnGroup, InTableBody, InRow, InCell, InSelect,
  AfterBody, AfterAfterBody. `html`/`head`/`body` synthétisés,
  attributs `<html>`/`<body>` fusionnés aux nœuds existants.
  Implicites : `<p>` fermé par les blocs (`P_CLOSERS` — `table`,
  `form`, `hr`, `pre` traités par leurs règles dédiées), `li`,
  `dt`/`dd`, `h1`–`h6`, `option`/`optgroup`, `rt`/`rp` (scope
  `ruby`). Scopes : button, list-item, table, formatting (marqueurs
  `<applet>`/`marquee`/`object`/`template`). **AFE** : règle de Noé
  (3 entrées identiques max), reconstruction après fermeture de
  bloc, adoption agency basique (`<b><i>x</b>y</i>` → `y` ré-enrobé
  dans un `<i>` frère). Tables : `<tbody>`/`<tr>` synthétisés,
  cellules `td`/`th` fermées en chaîne, **foster parenting** du
  texte et des éléments mal placés (insérés avant `<table>` dans
  son parent, repli sous-table). Contenu spécial : `script`/
  `style`/`noframes`/`xmp`/`iframe` (rawtext/ScriptData),
  `title`/`textarea` (RCDATA), `plaintext`, `\n` initial sauté
  après `pre`/`textarea`/`listing`, CDATA autorisé en `<svg>`/
  `<math>` via `set_cdata_allowed`. DOCTYPE → `quirks`/
  `limited-quirks` : liste de préfixes WHATWG complète, cas
  « missing **or empty** » vs « neither missing nor empty »
  distingués, DTD IBM XHTML reconnu, `strict 4.01` sans system
  reste `NoQuirks`. Tolérance : contenu post-`</body>`/`</html>`
  retraité en InBody, `</p>` sans `<p>` → `<p>` vide, `</br>` →
  `<br>`. **API** : `TreeBuilder::process` token par token,
  `drive` intercalant `next_token`/`process` (les états de contenu
  sont réglés *avant* la lecture de la suite — `Tokenizer::run`
  ne doit pas précéder `process`), `parse` raccourci, accès
  `dom`/`into_dom`/`quirks_mode`. 26 tests : synthèse, fermetures
  implicites, foster parenting, rawtext/RCDATA/script, quirks,
  tag-soup sans panic, découpage incrémental équivalent au
  one-shot.
- **Étape 10 — `explorer-html` : encodage, entités et tokenizer
  incrémental** (`encoding.rs`, `entities.rs`, `token.rs`,
  `tokenizer.rs`) : sniffing WHATWG — BOM UTF-8/UTF-16LE/BE,
  `<meta charset>` prescanné dans les 1024 premiers octets (case
  et quoting tolérants, hors-balise rejeté), défaut
  `windows-1252` — puis transcodage tolérant (`U+FFFD`, BOM
  consommé). Tokenizer machine-à-états incrémental : chunks
  bornés via `feed`/`next_token`, reprise de coupure au milieu
  d'une balise, d'une entité ou d'un mot-clé (`doctype`,
  `public`, `system`, `[CDATA[` — matching incrémental), texte
  `Character` coalescé avec ordre du flux préservé (token retenu
  pendant la purge), états rawtext/RCDATA/script/PLAINTEXT
  pilotés par le tree-builder (`ContentKind`, « appropriate end
  tag » via `last_start`), CDATA en contenu étranger, `Eof` après
  résolution des constructions suspendues (orphelins `<`/`</`/
  `<!`/`]]` réinjectés en texte). Entités : table nommée
  longest-prefix (`;` d'abord, legacy sans `;` — règle de
  l'esperluette ambiguë en attribut), numériques décimales/hexa
  avec remap `0x80..0x9F` windows-1252 et `U+FFFD` sur
  surrogates/dépassements. Bornes contenu hostile : noms
  1 Kio, valeurs/commentaires 64 Kio, purge texte 16 Kio
  (ADR-0011). 34 tests dont coupures de chunk exhaustives.
- **Étape 8 — `explorer-net` : redirections & politique**
  (`policy.rs`, `proxy.rs`) : `RedirectTransport` — suivi 3xx borné
  (`TooManyRedirects`), `Location` résolu contre la base, 303 → GET
  (sauf HEAD), 301/302 POST → GET, 307/308 préservés intégralement.
  `HstsStore` mémoire borné — `max-age`/`includeSubDomains`,
  `max-age=0` révoque, upgrade `http→https` appliqué par
  `HstsTransport` sur le fil, header capturé sur réponses https.
  `ProxyConfig` env (`http_proxy`/`https_proxy`/`no_proxy`,
  suffixes `.dom`/`*.dom`) + `ProxyConnector` — absolute-form pour
  HTTP, tunnel `CONNECT` pour HTTPS (TLS posé sur le tunnel via le
  `inner` injectable de `TlsConnector`). `RefererTransport` —
  URL complète same-origin, origine seule cross-origin, droppé en
  `https→http` ; `Origin` posé sur les requêtes mutantes
  cross-origin. `PolicyTransport` — mixed-content `http` sous page
  `https` bloqué (`BlockedMixedContent`), localhost/`data:`/`about:`
  exempts. `Connector` gagne `absolute_form()`. 18 tests.
- **Étape 7 — `explorer-net` : cookies + cache** (`cookies.rs`,
  `cache.rs`) : `CookieJar` RFC 6265 durci — `Domain=` validé par
  **Public Suffix List** (`psl`, supercookies `.co.uk` rejetés),
  `Secure` https-only (localhost toléré), `HttpOnly` filtré de la
  vue `document.cookie`, `SameSite`, préfixes `__Host-`/`__Secure-`
  requis, `Max-Age=0` = suppression, chemins RFC 6265 §5.1.4,
  budgets `CookieConfig` (total/par-domaine, éviction). `MemoryCache`
  — GET only, `no-store`/`Vary: *` exclus, statuts cachables RFC
  9111, fraîcheur `max-age`/`Expires`, revalidation `If-None-Match`/
  `If-Modified-Since` sur 304 (corps caché servi, en-têtes
  fusionnés), éviction FIFO bornée. Décorateurs `CookieTransport`
  (Cookie: injecté + Set-Cookie capturé) et `CachingTransport`
  composables. 17 tests dont wire end-to-end.
- **Étape 6 — `explorer-net` : décodage des corps** (`decode.rs`)
  : `DecodingTransport` décorateur — `Accept-Encoding: gzip,
  deflate, br` injecté, `Content-Encoding` multi-codages décodés
  en ordre inverse, zlib **et** raw deflate (le web réel mélange),
  `br` via `brotli`, budget `max_decoded_bytes` anti-bombe zip →
  `NetError::DecodedTooLarge`, flux corrompus/codages inconnus →
  `NetError::ContentDecode`, `Content-Length` ré-écrit à la taille
  décodée. `Response` gagne `content_type()`/`charset()`/
  `nosniff()` (`X-Content-Type-Options` exposé) et `text()` —
  transcodage WHATWG par `encoding_rs` (défaut `windows-1252`).
  Deps : `flate2`/`brotli`/`encoding_rs` (pur Rust). 13 tests.
- **Étape 5 — `explorer-net` : TLS** (`tls.rs`) : `TlsConnector`
  implémente la couture `Connector` — TCP clair pour `http:`,
  handshake `rustls` pour `https:` (SNI/ALPN par `tokio-rustls`),
  racines Mozilla `webpki-roots` ou store injecté
  (`with_roots` — tests/enterprise), erreurs certificat/SNI/
  handshake en `NetError::Tls` typées. Provider crypto **`ring`** :
  exception `*-sys*` consignée ADR-0002 (pas de pile TLS viable
  100 % Rust ; `ring` build Windows sans cmake, `aws-lc-rs` exclu).
  3 tests loopback avec cert `rcgen` : handshake OK, cert non
  approuvé → `Tls`, mauvais hostname → `Tls`.
- **Étape 4 — `explorer-net` : client HTTP/1.1** (`http.rs`,
  `h1.rs`, `error.rs`) : `Request`/`Response`/`Method`, trait
  `HttpTransport` injectable (ADR-0002), `H1Client` sur `Connector`
  pluggable (`TcpConnector` clair, TLS à l'étape 5 par la même
  couture `IoStream`), en-têtes via `httparse`, corps
  `Content-Length`/`chunked` (trailers)/EOF HTTP/1.0, pool
  keep-alive par origine borné + retry unique sur méthode
  idempotente, budgets `HttpConfig` (timeouts connect/io, tailles
  tête/corps, `User-Agent`, cap pool), erreurs `NetError` typées
  (DNS/connect/timeout/InvalidResponse/TruncatedBody/InvalidChunked/
  BodyTooLarge/HeadTooLarge/UnsupportedScheme). 14 tests
  d'intégration sur `TcpListener` loopback (aucun hôte externe).
  Dépendances consignées ADR-0002 : `tokio`, `httparse`,
  `async-trait`.
- **Étape 9 — `explorer-dom` : arène DOM** : `NodeId`
  générationnels (index+génération, handles périmés →
  `DomError::StaleHandle`), nœuds `Document`/`Doctype`/`Element`/
  `Text`/`Comment`, attributs ordonnés, navigation complète,
  `append_child`/`insert_before`/`remove` avec gardes de
  hiérarchie (cycles, parent sans capacité), `remove` détache sans
  détruire, table de **rooting** explicite (`add_root`/`remove_root`,
  cycles DOM↔JS par table — ADR-0008), `collect` mark-sweep réclamant
  les sous-arbres non rootés, `normalize()` fusionnant les `Text`
  adjacents (les nœuds rootés bloquent la fusion), newlines
  `\r\n`→`\n`, `text_content`, budget `max_nodes` borné (ADR-0011).
  13 tests verts.
- **Étape 3 — `explorer-net` : URL WHATWG** (`url.rs`) : parseur
  hiérarchique (`http`/`https`/`file`) + opaque (`data:`/`about:`),
  hôtes IDNA UTS-46 via la crate `idna` (arbitrage consigné
  ADR-0002), IPv4 aux formes WHATWG (octal/hex/réduites `127.1`),
  IPv6 `[…]`, userinfo, ports par défaut élidés, segments `.`/`..`
  normalisés, percent-encoding des jeux path/query/fragment/
  userinfo, résolution relative RFC 3986 §5.2, `Origin` tuple/
  opaque pour la politique same-origin, `path_query` (origin-form).
  27 tests de conformité sous-ensemble WHATWG.
- **Étape 2 — squelette Flutter `app/`** : `flutter create` Windows,
  clean architecture `core/` (`api/` client REST + SSE `dart:io`
  zéro-dépendance avec reconnexion, `design/` tokens + thème dérivé,
  `config/` `AppConfig`, `di/` providers Riverpod) + `features/
  browser/` (entité `BrowserTab`, `BrowserRepository` sur le contrat
  REST ADR-0003, `BrowserPage` coquille omnibox). `assets/design/
  tokens.json` chargé au démarrage avec repli embarqué (ADR-0005) ;
  `ExplorerApp` attend les tokens puis dérive `ThemeData`.
- **Étape 1 — workspace cargo** : 13 crates `explorer-*` squelettes
  (lib/binaire, `#![forbid(unsafe_code)]`, lint clippy pedantic,
  rustdoc), `[workspace]` Cargo.toml, `scripts/verify_all.ps1`
  (fmt/clippy/test/doc + flutter conditionnel).
- **`dillo-js` dans le workspace** : le moteur JavaScript est devenu
  `crates/dillo-js` — **version canonique et unique**, licence
  `MIT OR Apache-2.0`, chantier de l'équipe JS dédiée (ADR-0007
  révisé). Le crate a été **retiré du workspace du port voisin** —
  son `dillo-browser` consomme désormais le crate d'ici via la dep
  `path` `../OnionBit-Explorer/crates/dillo-js` (jonction locale
  `D:\Projet\OnionBit-Explorer` → `C:\OnionBit-Explorer`), en
  attendant la bascule `git`+`rev` à la publication d'Explorer.
- **ADR-0012** — pipeline incrémental : scheduler de
  sous-ressources dans `core`, entrée tokenizer par chunks, modèle
  d'invalidation style/layout/paint en trois niveaux.
- **Jalons J0–J5** dans `ROADMAP.md` : repères de motivation, dont
  J2 (`explorer-cli dump` — premier artefact visible dès la fin de
  phase 2, avant tout pixel).
- **Chantier `dillo-js`** (piste parallèle équipe JS, ROADMAP
  phase 8) : JS-A alignement heap → ADR-0008 + GC mark-sweep
  (prérequis de l'étape 34), JS-B harness test262, JS-C couverture
  (modules ES, `Proxy`, `Intl`), JS-D médias.

### Changed

- **Rebranding** : le projet s'appelle désormais « Explorer » (nom
  de travail — le répertoire physique sera renommé à la fermeture
  de l'IDE). Tous les documents ont été débarrassés des références
  à l'ancien écosystème ; les coutures (proxy, profil déplaçable,
  transport injectable) restent ouvertes sous justification
  propre.
- **Roadmap durcie** : politique cookies complète (`SameSite`,
  `__Host-`, Public Suffix List) en étape 7 ; `Referer`/`Origin`/
  mixed-content en étape 8 ; entrée tokenizer incrémentale (étape
  10) ; `position`/`z-index`/`overflow`/stacking/`list-item`/
  `white-space` + pseudo-classes dynamiques (étapes 16/21) ;
  `FontProvider` comme port injecté (étape 18, ADR-0009) ; spike
  ADR-0008 à faux handles + alignement heap `dillo-js` comme
  prérequis de l'étape 34 ; `<svg>` statique en étape 45 ;
  dégradation flexbox du palier 1 assumée et mesurée ; exclusions :
  SVG animé (SMIL) ajouté.
- **ADR corrigés** : référence `tiny-skia` (ADR-0001 : 0005 →
  présente décision) ; ADR-0003 réécrite (justifications propres,
  compromis `?key=` documenté) ; ADR-0004 (glue FFI côté Flutter
  pour la mémoire partagée) ; ADR-0006 (justification licence sans
  lien amont) ; ADR-0007 (moteur en workspace, échappatoire
  `boa_engine` nommée) ; ADR-0008 (contrat imposé à `dillo-js`) ;
  ADR-0009 (frontière I/O `fontdb`/`FontProvider`) ; ADR-0011
  (profils cloisonnés génériques).
- **Règle §4.3 assouplie** : `debug_assert!` permis sur invariants
  internes purs (jamais sur contenu externe).
- `.gitignore` : `app/pubspec.lock` n'est plus ignoré — une
  application Dart commit son lockfile (builds reproductibles).

### Added (fondations, étape 0)

- Fondations documentaires du dépôt (étape 0) : `AGENTS.md`
  (contrat de travail), `ROADMAP.md` (11 phases, étapes 0–48),
  `CHANGELOG.md`, `docs/architecture.md`.
- ADR initiaux dans `docs/architecture/decisions/` :
  - ADR-0001 — périmètre : moteur from scratch, dépendances
    d'infrastructure pur Rust autorisées, moteurs existants exclus ;
  - ADR-0002 — pile réseau : HTTP/1.1 maison sur tokio + rustls,
    `httparse` pour les en-têtes ;
  - ADR-0003 — couture UI : API de contrôle REST + SSE sur loopback,
    l'app Flutter ne touche jamais le core ;
  - ADR-0004 — transport des frames : PNG tiré sur invalidation SSE
    (MVP), WebSocket/texture FFI en voies d'upgrade ;
  - ADR-0005 — design tokens : `app/assets/design/tokens.json`
    source de vérité unique du design system ;
  - ADR-0006 — double licence MIT OR Apache-2.0 ;
  - ADR-0007 — couture JavaScript : trait `JsEngine`, moteur
    `dillo-js` intégré en phase 8 ;
  - ADR-0008 — modèle mémoire DOM ↔ GC figé avant la phase 2 :
    `NodeId` générationnels, rooting explicite, cycles par table de
    callbacks, invalidation de génération documentée ;
  - ADR-0009 — stack texte/fontes : primitives pur Rust (`fontdb`,
    `ttf-parser`, `rustybuzz`, `unicode-bidi`, `unicode-linebreak`),
    runs itemisés, fonte fixture embarquée ;
  - ADR-0010 — corpus de conformité publics (html5lib-tests, WPT,
    test262, Acid2/Acid3), % tracké par phase ;
  - ADR-0011 — modèle de menace : budgets + cloisonnement de session
    actifs, isolation processus différée, réévaluation obligatoire
    en phase 8.

### Changed

- Cap du projet rétabli : **compatibilité avec le web moderne** —
  le but d'un navigateur est d'accéder à tous les sites. Une version
  initiale des documents plafonnait par erreur le périmètre à un
  « navigateur documentaire » (ES3, pas de flexbox/grid) ; la roadmap
  porte désormais une phase 10 « Compatibilité web moderne »
  (HTTP/2+, flexbox/grid, ES6+, Web APIs, médias, webfonts) et les
  ADR 0001/0004/0007 ont été corrigés en conséquence.
- Optimisations d'architecture et de roadmap (ADR-0004, 0006, 0007,
  `ROADMAP.md`, `architecture.md`) :
  - **Licence `dillo-js` tranchée** : double licence `MIT OR Apache-2.0`
    formalisée par son auteur unique (Laurent Geynet), éliminant tout
    risque de contamination GPL-3.0 dans `explorer-js`.
  - **Gestion mémoire DOM ↔ GC** : `NodeId` générationnels (`slotmap`)
    et système de handles sécurisés pour prévenir les *dangling IDs*
    lors du détachement de nœuds par du script.
  - **Transport de frames accéléré** : intégration dès la phase 6/7
    d'un buffer brut de pixels décompressés en mémoire partagée /
    named pipe local, cantonnant le pull PNG au MVP strict.
  - **Outillage visuel layout précoce** : export et dump wireframe
    dans `explorer-cli` dès la phase 4 pour calibrer le layout sans
    attendre le pipeline de rendu complet.
  - **Paliers de compatibilité web** : découpage formalisé en 3
    paliers structurants (Documentaire → Web 2.0/SSR → Applications riches SPA).
- Optimisation documentaire suite à revue critique (ADR-0001,
  0002, 0003, 0004, 0007, `ROADMAP.md`, `architecture.md`,
  `AGENTS.md`, `README.md`) :
  - **Cap reformulé** : « tous les sites » devient une direction
    asymptotique pilotée par paliers mesurés sur corpus (ADR-0001,
    ADR-0010).
  - **Réseau** : IDNA/punycode, schémas `data:`/`about:`, HSTS,
    `X-Content-Type-Options`, transcodage charset + sniffing WHATWG
    — `windows-1252` avancé aux phases 1–2 (défaut réel de la spec
    HTML), plus phase 10 (ADR-0002).
  - **Entrées** : coalescing des événements mouvement documenté
    (ADR-0003) ; tension texture GPU ↔ isolation de processus
    explicitée pour l'étape 47 (ADR-0004).
  - **JS** : modèle mémoire DOM ↔ GC délégué à ADR-0008, mesure
    test262, échappatoire ES6+ formalisée (ADR-0007).
  - **Roadmap** : étapes 3, 6, 8–10, 12, 16, 18, 20, 23, 26, 34,
    48 enrichies ; référence tiny-skia corrigée (ADR-0001, pas
    ADR-0005) ; section « Hors périmètre assumé » ajoutée.

