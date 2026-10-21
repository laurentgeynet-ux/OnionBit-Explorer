# Changelog

Toutes les modifications notables de ce projet sont consignées ici.

Le format suit [Keep a Changelog](https://keepachangelog.com/fr/1.1.0/)
et le projet adhère au [Semantic Versioning](https://semver.org/lang/fr/).

## [Unreleased]

### Added

- **R-58 — `clip-path: inset()` minimal**
  (`explorer-render`) : la propriété est reconnue par le
  parser/cascade et `paint_box` enveloppe `paint_self` +
  sous-arbre d'un `PushClip`/`PopClip` quand la valeur est
  `inset(<length-percentage>{1,4})` (syntaxe margin 1→4
  côtés, `%` sur la border box, transform appliquée comme
  l'overflow). Motif `.visuallyHidden` (`clip-path:inset(50%)`
  = clip 0×0) couvert pour les variantes sans 1×1+overflow.
  Hors scope : `circle()`/`polygon()`/`path()`/`url()` —
  ignorés tolérants. Tests display-list + raster pixel.
- **JS-C54 — sélecteurs d'attribut au mini-DOM** : `querySelector(All)`/`matches`/`closest` acceptent `[attr]`, `[attr=v]`, `~=`/`|=`/`^=`/`$=`/`*=` (valeurs nues ou quotées, flag `i`/`s`, filtres et classes répétables). Déclencheur terrain : le module lobste.rs itère `time[data-at-unix]` au `DOMContentLoaded` — la liste vide masquait le listener (les TypeError ×4 historiques ne se reproduisent plus : `dataset`/`innerText`/`addEventListener`/NodeList itérable fonctionnent, timestamps désormais réécrits en texte relatif). Bonus outillage : `explorer-cli` initialise `tracing-subscriber` (`RUST_LOG` rend visibles les `debug!` session — scripts en échec, etc.).
- **JS-C53 — test262 re-baseliné : 1046/1046 exécutés,
  31 skips, 0 échec** (sous-ensemble extrait d53ebc8, 1077
  fichiers) : la référence 1007/1046 datait d'avant la vague
  regexp — re-mesurée identique, puis les 39 échecs soldés.
  `Lexer::number` rejette `_` dans les formes à zéro en tête
  (ES2021 : séparateur interdit dans
  `LegacyOctalLike`/`NonOctalDecimal` — 8 négatifs) ;
  `set_prop_value` route le `[[Set]]` des bases primitives
  vers les intercepteurs de la chaîne proto (`prim_set_intercept`)
  et `write_prop` consulte désormais la trap `set` des proxies
  **à chaque maillon** de la chaîne, pas seulement en tête
  (`proxy_set_step` factorisé — cas `Symbol().x = v`, 1 test) ;
  `Lexer::private_name` accepte `\uXXXX`/`\u{…}` — les 30 cas
  « Unicode identifiers » du corpus étaient tous des noms
  privés échappés. Cliquet `BASELINE_PASSED` remonté à 1046.
- **R-32 — corpus WPT élargi : margin-collapse** : tranche
  `margin-padding-clear` + `floats-clear` + `normal-flow`
  extraite par `fetch_wpt.ps1` — 154 fichiers (36 sans
  référence exclus du score). Baseline honnête mesurée
  **115/152 exécutés, 6 skips** ; cliquet `WPT_BASELINE`
  reposé de 57 à 115. Corrections moteur au fil du triage :
  collapse-through des chaînes de marges hautes évalué
  pré-layout (`chain_transparent`/`candidate_chain`,
  purement stylistiques), marges de l'élément racine jamais
  collapsées, position statique des boîtes absolues ancrée
  au bas réel du frère en flux précédent (§10.6.4), marges
  des flottants jamais collapsées, `table-layout:fixed` +
  règles UA `col`/`colgroup` (table `width:auto` = somme
  des colonnes spécifiées), `border-width` mots-clés dans
  la mesure intrinsèque des cellules, propagation du fond
  de `body` au canvas (§14.2), `background-position`
  résolu en `em`/`rem`, reset `line-height` et `font-*`
  du shorthand `font` (§3.6), ordre de pile Appendix E
  (portée inline fermée par tout conteneur bloc ;
  positionnés `z≥0` différés au contexte de pilement
  englobant), translation du sous-arbre complet pour
  `position:relative`, marges horizontales des éléments
  inline non-remplacés. Feuille UA `body { margin: 8px }`
  ajoutée (compat réelle — les références partagées WPT la
  supposent) ; les fixtures des tests downstream écrites
  pour une marge nulle la neutralisent (`margin:0`).
  Skip documenté :
  `margin-collapse-through-percentage-padding.html` exige
  mutation JS + relayout — piste NAVIGATEUR.
- **R-34 — médias vrais remplacés boxtree** : `<video>` et
  `<audio>` sont des feuilles remplacées **natives**
  (`build_box_tree_sized`) — taille intrinsèque de
  l'affiche décodée, gabarit standard sinon (vidéo
  300×150, audio 300×54) ; le contenu de repli n'est
  jamais boxifié. `object-fit` minimal à la peinture :
  `fill` (défaut, comportement inchangé), `contain`,
  `scale-down`, `cover`, `none` + `object-position`
  (mots-clés, `%`, `px`) — les modes débordants sont
  clipés à la content box. `collect_data_images` couvre
  désormais `<video poster="data:…">` (le complément NAV
  `poster_data_images` est retiré) ; les hints
  présentationnels `width`/`height` sur `video`
  existaient déjà (axe par axe). Feuille UA : `video`
  repasse en élément en-ligne remplacé (sorti de la
  liste `display:block`), `audio:not([controls])`
  devient `display:none` côté domaine — le complément
  UA session ne porte plus que le « look produit »
  (fond noir vidéo, chrome barre audio). Tests :
  feuilles natives (`boxtree`), rect d'`object-fit` +
  clip (`display`), collecte poster (`dataurl`),
  équivalence intrinsèque `<video poster>` ≡ `<img>`
  bout en bout (session).
- **R-35 — métriques de runs pour find** : le surlignage
  find passe de la boîte `Text` entière au **glyphe près**.
  API additive `explorer_layout::fragment_sub_rect` :
  sous-rect d'une tranche d'octets d'un fragment posé —
  union des cellules des glyphes dont le `cluster` tombe
  dans la tranche (ordre visuel → exact LTR/RTL, ligatures
  couvertes entières), bande verticale = hauteur de ligne
  du fragment ; repli proportionnel aux octets sans run
  shapé, `None` sur tranche vide ou atome. `find.rs`
  recherche sur le texte tel qu'**affiché** (fragments
  concaténés par boîte — un match enjambe fragments,
  éléments inline internes et retours à la ligne) et
  groupe les sous-rects par occurrence : le compte et la
  navigation sont à l'**occurrence** (deux « lorem » dans
  un même paragraphe = 2, comme Chrome — carve-out du
  test unitaire session). Repli boîte-entière conservé
  pour les `Text` rendus hors lignes (valeurs de
  contrôle), dédupliqué par collecte globale des nœuds
  couverts (le texte niché `<b>` vit dans les lignes de
  l'ancêtre). Tests : sous-rect exact au glyphe sur
  fixture 0.5em, replis proportionnel/invalide, bande
  peinte 3 glyphes sur 10 vs boîte entière, dédup match
  niché + groupement occurrence transversale.
- **R-36 — cache glyphes cross-passe du raster** :
  `explorer_render::GlyphCache` (module `glyph_cache`)
  persiste entre repaints — faces `ttf_parser` possédées
  (`owned_ttf_parser::OwnedFace`, nouvelle dep appariée
  0.25, pur Rust) et contours de glyphes en unités fonte
  (`Path::transform` = même `map_point` par point que
  l'ancien sink transformé, pixels identiques). APIs
  additives `rasterize_cached` / `rasterize_dirty_cached`
  — `rasterize`/`rasterize_dirty` inchangées (délèguent
  à cache jetable). Invalidation : `FontSource::revision`
  (trait, méthode additive `0` par défaut) bumpée par
  `WebFontSource::register`/`clear` — une face `@font-face`
  arrivée entre deux repaints vide le cache au sync
  suivant. `PaintPayload.font_cache` câblé (`Send` —
  `Arc`, jamais `Rc`). **Mesure `sync_profile`** : ~0 %
  de gain (165.5 ms/passe neuf = chaud sur page dense en
  texte) — `outline_glyph`/`Face::parse` ≪ 1 % du raster ;
  le poste dominant est le fill scanline tiny-skia → la
  suite perf texte = atlas bitmap de glyphes (travail
  séparé consigné en roadmap). Tests : équivalence pixels
  passe cachée/neuve (plein + dirty), réutilisation face
  `Arc`, vidage à révision changée.
- **R-37 (résidu) — encodage PNG streamé par bandes** :
  `Frame::encode_png` réécrit sur `png::StreamWriter`
  (nouvelle dep directe `png` 0.18 — déjà tirée par
  `image`, backend fdeflate identique à
  `CompressionType::Fast`). La démultiplication
  prémultipliée se fait par bandes de 256 lignes dans un
  scratch borné (~`width*4*256` octets) — fini le clone
  RGBA monolithique (~50 Mio sur une page de 16 384 px) ;
  les `IDAT` partent par chunks de ~256 Kio. API additive
  `Frame::encode_png_to(W: Write)` : écriture directe vers
  une réponse HTTP/SSE sans matérialiser le `Vec`.
  Tests : équivalence bytes `encode_png`/`encode_png_to`,
  décodage pixel-exact en travers de la frontière de
  bande, démultiplication (alpha 0 → noir, 128/128 → 255).
- **R-37 (résidu) — `DAMAGE_INFLATE_CSS` validée, bounds
  d'encre réelles** : `ShapedRun.ink` (champ additif) =
  union des `ttf_parser::glyph_bounding_box` décalées par
  position de plume et offsets, calculée au shaping et
  `Arc`-partagée — `item_bounds` des items `Text` passe
  de la boîte de métriques à l'encre réelle : l'overhang
  glyphe (italique, accents, fontes plein-em type Ahem)
  est couvert **sans inflation**. `DAMAGE_INFLATE_CSS = 2`
  ne couvre plus que l'AA sous-pixel (les items non-texte
  peignent tous dans leurs bounds). Fixture additive
  `explorer_test_support::fixture_ttf_overhang` : encre
  mesurée débordant de 0.10/0.45/0.25em (9 px droite,
  5 px haut à 20 px > 2 px d'inflation) — le test
  mutationnel texte↔vide de `dirty_raster` verrouille
  l'absence de reliques aux bords de fenêtre ; unit-test
  des débords mesurés sur `run.ink`.
- **Étape 85 — gestes chrome pour le test humain** (`app/`) :
  les raccourcis navigateur attendus au premier usage, liés au
  niveau **page** (`CallbackShortcuts`) — auparavant le zoom et
  la recherche n'existaient que sous focus de la frame. Clavier
  : `Ctrl+L` (focus + sélection omnibox), `Ctrl+T`/`Ctrl+W`
  (onglet neuf/fermé), `Ctrl+R`/`F5` (recharger), `Alt+←`/`→` +
  touches dédiées `browserBack`/`Forward` (précédent/suivant),
  `Ctrl+Tab`/`Ctrl+Shift+Tab` (cyclage), `Ctrl+1..8`/`Ctrl+9`
  (index/dernier), zoom `Ctrl`+`+`/`−`/`0`, `Ctrl+F`, `Échap`.
  Souris : boutons `X1`/`X2` = précédent/suivant, clic-molette
  sur un lien = **nouvel onglet en arrière-plan**
  (`openInNewTab` — l'onglet courant garde le focus pendant le
  chargement), clic-molette sur l'onglet = fermer. Le relais
  clavier moteur n'envoie désormais que des frappes **nues** —
  un combo modifié n'est plus tapé dans le champ (`Ctrl+V`
  envoyait `v`). Tests : `cycleTab`/`selectTabAt`/
  `openInNewTab` (arrière-plan + échec de création).
  `analyze` + `test` verts (28/28).

- **Veille daemon — `scripts/daemon_watch.ps1`** :
  instrument du protocole RSS (item « veille » roadmap
  NAV) — sur un daemon dédié, répète le cycle de la
  disparition du 22/10 (onglet → navigate → tempête de
  `mousemove` → back → DELETE) et échantillonne le
  WorkingSet64 à chaque cycle en CSV. Distinction
  « process mort » vs « erreur applicative » par le
  listener du port. Critère : RSS croissant sans borne
  ou mort = repro ; RSS plat sur fenêtre longue =
  exonération progressive.
- **JS-C48 — rapatriement des avancées Dillo `dillo-js`**
  (`explorer-jscore`) : la copie vendored avait dérivé
  (~1 200 lignes de delta bidirectionnel sur 6 fichiers) —
  port-back complet dans le canonique avant suppression de
  la copie. Surface `window` : `add`/`removeEventListener`,
  `dispatchEvent` (sommet de propagation), `getComputedStyle`
  (inline), `matchMedia`, `CSS.supports`/`escape`,
  constructeur `Node` + constantes, `top`/`parent`/`frames`,
  `scrollX/Y`/`pageX/YOffset`. `document` : `fonts`
  (`FontFaceSet`), `defaultView`, `visibilityState`/`hidden`,
  `nodeType`/`nodeName`, `hasFocus`. Éléments :
  `parentElement` ≠ `parentNode` (spec), dimensions
  client/scroll/offset honnêtes (racine ≙ viewport).
  `history.pushState`/`replaceState`, `navigator.sendBeacon`,
  handler `on*` retournant `false` → `preventDefault` (spec),
  `AbortController`/`AbortSignal` (rejet `fetch` sur signal
  déjà aborted), diagnostic TypeError nommant méthode +
  receveur. Regexp : quantificateurs lazy `*?`/`+?`/`??`/
  `{n,m}?`, bornes de classe échappées décodées (`\xHH`,
  `\uHHHH`, `\u{…}`, `\0`, `\cX`, octales Annex B), surrogates
  omis/clampés (classe vidée → `(?!)`/`[\s\S]`).
  `performance.mark`/`measure`/`timeOrigin`/`timing`/
  `navigation` (stubs honnêtes). Re-baseline test262
  **1007/1046 exécutés (96 %)** — whitelist `UNSUPPORTED`
  périmée re-vérifiée (31→4 entrées). Outil `examples/
  census.rs` porté (compte-rendu d'erreurs de pages réelles).
  Sens inverse vérifié : JS-C46, JS-C45 et la vague JS-C26+
  étaient déjà Explorer-only — `src/dom.rs` reste canonique.
- **JS-C35 — `document.write` complet + insertion dynamique**
  (`explorer-jscore`, miroir) : `write`/`writeln` deviennent
  variadiques (concaténation de tous les arguments, `\n`
  terminal pour `writeln` — spec) ; `document.open()`/
  `close()` existent et pilotent le flag `Dom.doc_open`
  — ouvert pendant le chargement, fermé par `close()` ou
  la distribution de `DOMContentLoaded`/`load` ; un `write`
  sur document fermé fait l'`open()` implicite : le contenu
  est effacé puis reconstruit (spec `document open steps` —
  le `document.write` post-`load` qui remplace la page).
  Correctif découvert en route : `Dom::by_id` exclut les
  nœuds détachés — un élément `remove()`d ou effacé par
  `open()` ne ressort plus par `getElementById`. Écart
  documenté : les `<script>` écrits restent inertes dans le
  miroir — l'ordonnancement revient au scheduler amont
  (étape 50), qui les découvre à la re-sérialisation.
- **JS-C44 — import maps minimales** (`explorer-jscore` +
  `explorer-js`) : les bare specifiers (`import 'react'`)
  sont réécrits avant l'appel au `ModuleLoader` —
  correspondance exacte puis préfixe le plus long en `/`
  (le reste du spécificateur est conservé, spec « resolve a
  module specifier »). `Engine::set_import_map(json)` lit la
  clé `imports` d'un `<script type="importmap">` (JSON parsé
  par le moteur ; map invalide ignorée, valeurs non-string
  sautées, `scopes` non lu — minimal documenté) ; fusion
  additive, première clé gagnante en conflit. Exposé côté
  couture par `JsEngine::set_import_map` (défaut noop —
  contrat additif, la session pousse le JSON avant son
  premier `eval_module`). `census` enregistre les
  `type="importmap"` dans l'ordre du document. Un bare hors
  map garde l'erreur `cannot load module` typée.

- **JS-C29 — `Worker` bornés** (`explorer-jscore`) : un
  `Engine` par thread (patron `JscoreEngine` — le moteur ne
  quitte jamais son thread), `postMessage`/`onmessage`/
  `onerror` des deux côtés, valeurs clonées en forme fil
  `WireValue` (aucun handle de heap ne traverse ; cycles
  préservés par `Ref`, fonctions/Promise/Symbol/Proxy/host
  rejetées en `TypeError` DataCloneError). `importScripts`
  synchrone via `WorkerLoader`/`WorkerFactory` (résolution
  et fetch à l'embeddeur, jamais de socket ici),
  `terminate()` côté parent et `close()` côté worker,
  files `mpsc` bornées (`WorkerConfig.queue_len` =
  back-pressure), pool borné (`max_workers`, défaut 8),
  budget ops du parent propagé à chaque worker. Le wrapper
  GC'd termine son worker via `Host::finalize` ; les
  messages sont livrés à la pompe (`run_microtasks` →
  `pump_workers`). Sans fabrique, `new Worker` jette un
  `TypeError` honnête (`typeof Worker` reste `"function"`).
  Déviation documentée : pas de `SharedArrayBuffer`
  inter-thread — un buffer posté est **copié**.

- **JS-C42 — `Drop` itératif de l'AST** (`explorer-jscore`,
  livré par `5f19372` en résidu JS-C41, case cochée à
  retardement) : les chaînes d'opérandes `1+1+…`/`a&&a&&…`
  (~15 000 maillons, parsées itérativement) se détruisent
  par file explicite — `Expr::take_children` déplace les
  enfants `Box`/`Vec` hors de la récursion native de
  `Drop`. Ni `eval` (borne `expr_depth`, erreur typée) ni
  `module_specifiers` (parse + drop immédiat) ne débordent
  la pile. Test : `c41_deep_operand_chain_drops_iteratively`.
- **JS-C37 — borne mémoire heap + compteurs**
  (`explorer-jscore`, exigence ADR-0019) : `Heap` suit les
  octets estimés des cellules vivantes (coût à l'alloc,
  recalcul exact au sweep — la croissance des props entre
  dans le compte) ; `Engine::set_mem_limit` borne le heap
  par contexte avec kill doux — `RangeError` « heap memory
  limit exceeded » au `tick` suivant, le `maybe_collect`
  de frontière de jobs offre un sweep avant de re-tuer ;
  `Engine::set_gc_threshold` (défaut 131 072 cellules)
  déclenche une passe mark-sweep automatique aux
  frontières de jobs ; `heap_stats()` (vivantes, octets,
  pics, passes) et `ops_used()` exposent les compteurs ;
  `Dom::serialize_stats` instrumente le coût de
  resynchronisation `serialize_document` (appels/octets).
  Tests : `c37_*`, `serialize_stats_counts_calls_and_bytes`.
- **JS-C38 — cible cargo-fuzz du moteur JS** (`fuzz/`) :
  `fuzz/fuzz_targets/js.rs` alimente `Engine::eval`,
  `eval_module` et `module_specifiers` en octets arbitraires
  avec budgets serrés (50 K ops, 32 Mio heap) — le contrat
  « dégrader, jamais paniquer » est fuzzé sur lexer, parser
  et évaluateur (ADR-0019). Smoke déterministe sans nightly :
  `c38_fuzz_smoke_no_panic` (128 sources hostiles par LCG).
- **JS-C39 — synchro DOM instrumentée + test croisé miroir**
  (`explorer-js`, `explorer-jscore`) :
  `Dom::serialize_stats` rapporte appels/octets/micros cumulés
  de `serialize_document` ; `JsEngine::dom_sync_stats`
  (additif, défaut `None`) expose `DomSyncStats` à la session
  — la mesure par synchro exigée par le point de décision.
  Test croisé `dom_sync.rs` : le HTML du miroir re-parsé par
  `explorer_html::TreeBuilder` porte fidèlement les mutations
  JS dans `explorer-dom`. Décision : statu quo
  `serialize_document`, escalade `drain_dom_mutations`
  conditionnée au dépassement mesuré (~10 % frame budget ou
  churn DoS démontré).
- **JS-C40 — découpage interne `explorer-jscore`** : `builtins.rs`
  (7 528 lignes) éclaté en `builtins/` 20 modules par domaine
  (`mod.rs` conserve l'installation des natives et les helpers
  communs) ; `interp.rs` (5 718 lignes) éclaté en `interp/`
  (`mod.rs` moteur/envs/props/coercions, `exec.rs` statements,
  `eval.rs` expressions, `ops.rs` fonctions libres). Refactor
  purement mécanique : zéro changement sémantique, méthodes
  d'`impl Engine` réparties promues `pub(crate)` — surface
  publique intacte, suite complète inchangée au vert.
- **JS-C34 — déclencheur rendu observable** (`explorer-jscore`,
  livrable partiel) : fixtures `#[ignore]` encodant le
  comportement spec — `c34_generator_lazy_spec_fixture`
  (corps de générateur inerte jusqu'au 1er `next()`) et
  `c34_await_pending_spec_fixture` (`await` sur promesse
  pending suspend au lieu de jeter). Échec volontaire
  documenté : la refacto générateurs lazy / continuations
  d'`await` reste conditionnée à un site réel bloquant — une
  panne silencieuse devient attribuable à cet écart.
- **Étape 94 — media queries évaluées sur le viewport réel**
  (`explorer-core`, `explorer-cli`) : la cascade de session
  (`styles_payload`) et la résolution `@font-face`/`media=`
  (`SubJob::fetch_fonts`) lisaient `StyleEnv::default()` =
  1024×768 fixe — `max-width`/`min-width` étaient faux dès
  que le viewport différait (constat PIPE : BBC `span 8`
  perdu à 800px). Le `StyleEnv` est désormais construit
  depuis `ctx.viewport`/`SubJob.viewport` ; `cli layout`
  aligné sur sa largeur de pose (800px).
- **Étape 95 — attribution du pic mémoire « Wikipedia Rust »**
  (`explorer-js`, `explorer-core`) : le RSS ~1,9 Go reproduit en
  vol piloté n'est pas un tampon réseau mais ~1,1 Go de
  **rétention commise** — la cascade copie les custom
  properties `--*` héritées dans chaque `ComputedStyle`
  (~200 vars × ~19k nœuds : 3,26M entrées dans le boxtree,
  1,89M dans `prev_styles`). Instrumentation livrée :
  `JsEngine::heap_stats` (trait additif, défaut `None`),
  `pub use explorer_jscore::HeapStats`, sonde in-crate
  `tab_mem_inventory` (`#[ignore]`, inventaire par champ +
  bissection RSS). Le correctif (partage `Rc` des vars
  héritées) relève de `explorer-css` — demande journal.
- **Étape 96 — `<link media>` filtré + `@import` expansé +
  `max_sheets` revu** (`explorer-core`) : `plan_subresources`
  évalue l'attribut `media` contre `viewport_env(viewport)`
  **avant** la borne — les variantes `print`/thème hors
  viewport ne sont ni fetchées ni comptées ; `max_sheets`
  passe à 32 (corpus site_smoke : GitHub ~22 feuilles
  effectives, MDN ~20). Nouveau `SubJob::fetch_imports` :
  chaîne `@import` fetchée récursivement, contenu préfixé au
  slot (ordre de cascade), bornes `max_imports` (16) +
  profondeur 8 + dédup par URL — media d'import respectée,
  `@font-face` des feuilles importées éligible (expansion
  avant `fetch_fonts`). Évaluation via `media_attr_matches`
  (shim `@media` synthétique sur `parse_stylesheet` —
  aucune API `explorer-css` nouvelle). Soldé la demande
  `site_smoke` et la demande R-22 (`var(--color-*)`
  lobste.rs venant d'un `@import` non résolu). Complément
  terrain : dédup des `<link>` par URL résolue (GitHub
  répète ~7 hrefs — un doublon brûlait quota + fetch).
- **Étape 98 — `background-image` HTTP fetchés**
  (`explorer-core`) : les `url(http…)` des déclarations
  `background`/`background-image` de feuilles liées —
  `@import` expansées comprises — et du `<style>` inline
  étaient collectés par `collect_data_images` seulement en
  `data:` ; les HTTP n'étaient jamais fetchés (logo
  lobste.rs, icônes CSS du corpus). `SubJob::fetch_backgrounds`
  re-parcourt les règles (avec `@media` imbriqués évalués
  contre l'env viewport), résout chaque `url()` contre la
  base de **sa** feuille, déduplique par URL résolue et
  fetches dans le budget `max_images` partagé avec `<img>`
  — borné, jamais de panic sur contenu hostile. Lien
  texte-brut → URL résolue conservé dans `bg_aliases`
  (la feuille d'origine ne survit pas à la cascade) ; les
  corps transitent `SubCommitTail` → tampon `Tab`
  (`bg_bodies`) → payload → `insert_fetched_backgrounds`
  les insère dans `images.backgrounds` à la cascade, quand
  le `NodeId` propriétaire est connu — décodage dédupliqué
  par URL, `Arc` partagé entre boîtes. Deux chemins
  couverts : `sub_commit` synchrone (FFI) et rendu détaché
  `nav_commit_shared` (daemon). Tests : fetch + résolution
  feuille-relative + peinture prouvée par pixels, 404
  tolérée, borne `max_images` partagée respectée.
- **R-52 — `mask-image` borne le fond** (`explorer-render`) :
  le motif icône universel `mask-image:url(svg)` +
  `background-color` (Codex/Primer/Material) peignait un
  carré plein — la loupe des recherches Wikipedia, l'avatar
  avant « Donate ». Chaîne additive : `PaintImages.masks`
  (déjà couverte par `box_sig` via le style haché),
  `collect_data_images` lit `mask-image`/`mask` en
  `url(data:)`, nouveau `DisplayItem::MaskedFill` émis par
  `paint_self` quand un mask décodé existe ; raster = mask
  d'alpha tiny-skia échantillonné nearest dans la zone
  device, composé `min` avec le clip `overflow` actif.
  Repli tolérant : mask absent/indécodable → `FillRect`
  plein conservé. `url_arg` décode les échappements CSS
  (`\"` des data-URL `utf8` sérialisées) — répare aussi les
  `background-image` échappées. Hors portée :
  `mask-size`/`mask-position`, multi-couches,
  `mask-mode:luminance`. Tests `raster.rs` : alpha bornant
  le fond (dedans/dehors), repli sur SVG indécodable.

### Fixed

- **R-44 — `width` explicite d'un remplacé dans la mesure
  intrinsèque** (`explorer-layout`) : bug site-réel au triage
  `site_smoke` — les 4 icônes sociales de rust-lang.org
  s'empilaient une par ligne. `footer img{width:40px}` :
  `content_px` retournait l'intrinsèque (512) avant la taille
  spécifiée → base flex des `<a>` > conteneur → `flex-wrap`
  posait une ligne par item (§9.5, avant le shrink). Livré :
  `width`/`min|max-width` non-% font foi avant l'intrinsèque
  (§5.1) — généralise aux bases flex, pistes grid, flottants,
  cellules. `shot` : icônes en ligne horizontale.
- **R-45 — `float:right` translate le sous-arbre entier**
  (`explorer-layout`) : bug site-réel — les vignettes
  ITN/OTD de en.wikipedia.org se peintaient au bord gauche
  de la colonne, texte par-dessus. `place_float` corrigeait
  `child.rect.x` après le layout interne sans translater les
  descendants : boîte à droite, contenu resté à gauche.
  Livré : `translate_subtree(child, delta, 0)` — même motif
  que l'ancrage `right`/`bottom` des absolus. `shot` :
  vignettes à droite, légendes en place, texte enroulé.
- **R-46 — item flex/grid = contexte de formatage
  indépendant** (`explorer-layout`) : les marges des
  descendants d'un item s'évaporaient (traversaient la boîte
  item sans être posées ni comptées) — un `<p>` en tête
  d'item perdait sa marge et l'item se mesurait trop petit.
  Flexbox §4.1 : l'item établit un FC indépendant. Livré :
  `LayoutBox.fc_root` posé par `layout_item` (flex + grid),
  `is_fc_root` aux gardes §8.3.1 + englobage des flottants
  §9.4.1 ; le champ survit au relayout incrémental.
- **R-47 — `colspan`/`rowspan` : grille de table à
  occupation** (`explorer-css` + `explorer-layout`) : bug
  site-réel — sur Hacker News, le rang `subtext`
  (`<td colspan=2></td>` vide puis la cellule utile) posait
  la méta sous la colonne rang au lieu du titre ; les
  cellules étaient comptées 1×1. Livré : hints
  `-x-colspan`/`-x-rowspan` livrés par la cascade (`td`/`th`
  — attributs du modèle de grille, pas des propriétés CSS),
  `place_cells` assigne les colonnes en sautant celles
  retenues par les `rowspan` antérieurs, la mesure d'une
  cellule à span se répartit en déficit égal sur les
  colonnes couvertes (§17.5.2.2), la boîte peinte s'étend
  sur les rangs/colonnes traversés. Correctif connexe : le
  retrait du résidu de distribution ne descend plus sous
  `min` — un span massif (`colspan=999999999`) ne peut plus
  annuler une colonne voisine. Limites : hauteur d'un
  `rowspan` non distribuée entre rangs, `colspan=0` = 1.
- **R-48 — piste `auto` grid : limite par piste, pas par
  item** (`explorer-layout`) : bug site-réel — les cartes
  BBC (`display:grid` sans template → colonne `auto`
  implicite, 3 enfants en `auto-flow: row`) rendaient leur
  texte en rivières de 60px (= largeur conteneur / nombre
  d'enfants). `col_widths` collectait les bases par **item**
  : le libre était divisé par le nombre d'enfants et
  `w[col]` écrasé à chaque tour (dernier gagnant). Livré :
  regroupement par colonne — limite de piste = max des
  bases de ses items (§12.6 Maximize Tracks), distribution
  du libre entre pistes ; les deux branches préservent le
  plancher `lo` d'un `minmax(lo, auto)`. Terrain : `h2`
  60×308 → 180×132, descriptions et métadonnées BBC en
  ligne normale. Test : `grid_colonne_auto_multi_items`.
- **R-49 — `visibility:hidden`/`collapse` honoré à
  l'émission** (`explorer-render`) : la propriété était en
  cascade (héritée, init `visible`) mais jamais consommée —
  le motif « visually hidden » (accordéon `LanguageDropdown`
  BBC, contenus sr-only) se peignait en rivières de 1px.
  `paint_box` saute `paint_self`/`paint_replaced` des boîtes
  `hidden`/`collapse` sans court-circuiter transform/clip/
  descente — §11.2, un descendant `visibility:visible`
  explicite se repeint. Incrémental couvert (`visibility`
  dans `box_sig`). Terrain : le dropdown BBC n'émet plus
  d'item. Limite : le hit-test ne filtre pas `visibility` ;
  `collapse` sur ligne de table non retirée.
- **R-50 — blockification des items flex/grid** (§9.7 /
  CSS Display §2.7) (`explorer-layout`) : bug site-réel —
  `styled-components` émet `display:initial` (→ `inline`)
  sur ses `GridItem` ; `wrap_children` hoisait les blocs
  **avant** de savoir que le parent est flex/grid, donc
  l'item inline était scindé et son contenu hissé comme
  item auto-placé à 1 piste (`LondonMedia` BBC = 15.5px,
  12 cartes manchester à 49px). Livré : `blockify_item` —
  kind `Inline`→`Block`, contenu repassé par le chemin
  normal au lieu de hoister. Terrain : 48 → 5 boîtes
  étroites sur bbc.com (les 5 restantes = le dropdown
  caché à 1px, correct), LondonMedia 15.5→488px, héros
  3 colonnes et sections 4 colonnes conformes. Tests
  `modern.rs` : item `display:initial` en grid (cellule
  et contenu préservés) et en flex.
- **R-51 — remplacé `width:%` hors mesure intrinsèque**
  (§4.1/§5.1) (`explorer-layout`) : `content_px` retombait
  sur `bx.intrinsic` quand `width` était un pourcentage —
  en mesure min/max-content le bloc contenant est indéfini,
  donc un `img{width:100%}` imposait son bitmap entier aux
  ancêtres shrink-to-fit. Latent jusqu'à l'arrivée du
  bitmap : carte BBC `min-width:fit-content` gonflée à
  480px (intrinsèque du `src 480w`), débordant sa piste de
  grille 236px — titre LIVE déployé sur ~430px chevauchant
  le héros. Livré : un remplacé à `width:%` contribue 0 à
  la mesure intrinsèque ; `min|max-width` non-% bornent
  toujours ; non-remplacé inchangé (retombe sur le contenu).
  Terrain : titre LIVE + description wrappés dans la piste,
  héros propre. Tests `modern.rs` : `img{width:100%}` ne
  gonfle plus `min-width:fit-content` (contrôle
  `width:auto` : l'intrinsèque mesure toujours).
- **`scripts/spawn_daemon.ps1` — spawn WMI durci** :
  les morts silencieuses du daemon (3 occurrences
  non reproductibles, zéro stderr, zéro event) sont
  attribuées à la **fenêtre console visible** du
  `cmd.exe` parent — sa fermeture manuelle tuait tout
  l'arbre. Le script lance via
  `Win32_ProcessStartup{ShowWindow=SW_HIDE}` (le
  `CREATE_NO_WINDOW` est refusé par WMI,
  `ReturnValue 21`) un `cmd /v:on` chien de garde :
  stdout+stderr redirigés, et `& echo
  EXIT:!ERRORLEVEL!` écrit le code de sortie dans
  `daemon-<port>.exit.log` — `0` propre, `-1` kill
  externe (`Stop-Process`), `0xC0000409` fail-fast/
  stack overflow ; **absence** du fichier = kill de
  l'arbre entier (teardown machine), signature en
  soi. Validé en direct : kill → `EXIT:-1` horodaté.
- **Étape 89 — flux de frames app borné** (`app`) :
  constat M-H1 #5 — `explorer_app` gonflait ~80 Mio/min
  en usage réel (3,7→4,5 Go/10 min). Deux fuites
  composées : 1. `_pullFrame` tiré à **chaque**
  `frame_invalidated` sans coalescence — les rafales
  (hover, scroll, caret) empilaient fetch+decode PNG
  sans borne, onglets d'arrière-plan inclus ;
  2. `Image.memory(Uint8List.fromList(bytes))` créait
  une entrée `imageCache` par frame (clé = identité
  des bytes) + une copie de mémoire externe par
  rebuild + `gaplessPlayback` retenait l'ancienne.
  Livré : coalescence « dernière gagne » (au plus
  1 pull en vol + 1 en attente par onglet) et le
  chemin REST décode **une fois** en `ui.Image`
  (`instantiateImageCodec`) posée dans `rawFrames` —
  même discipline que le canal FFI : l'ancienne image
  est `dispose()`d explicitement, `RawImage` la peint.
  `frames` (bytes) reste écrit — marqueur de présence
  de `selectTab`. Test : rafale de 3 invalidations
  pendant un pull bloqué → exactement 2 fetches.
- **JS-C46 — entités du miroir JS décodées**
  (`explorer-jscore` + `explorer-core`) : verdict
  PIPE→NAV au triage M-H1 — `parse_fragment_into`
  n'appelait `decode_entities` que sur les valeurs
  d'attribut et le RCDATA (`title`/`textarea`) ; le texte
  courant passait brut → `escape_text` ré-échappait `&` →
  `&amp;nbsp;` affiché en clair après re-parse (toute page
  scriptée corrompait ses entités — google.fr montrait
  `&nbsp;`/`&copy;` littéraux), et `textContent` lisait le
  littéral. Livré : décodage au `flush_text!` + mini-table
  élargie au jeu HTML 4.01 (~200 noms), `&#x…;` hex, remap
  windows-1252 des `0x80..=0x9F` et `U+FFFD` spec.
  jscore reste autonome (pas de dep `explorer-html`) —
  écart connu : entités legacy sans `;` non décodées ;
  couture pour la table WHATWG complète consignée dans
  `docs/interface-equipes.md`. Bonus du même commit :
  `PipelineError` porte l'en-tête de la source/module
  fautif — `Error: b` localise enfin son script.
- **JS-C49 — hit-test de clic logué + validation terrain
  consent** (`explorer-core`) : `node_at` émet un
  `tracing::debug!` du tag frappé par le clic — premier
  réflexe de triage des « clics ignorés » sur pages réelles
  (M-H1 : la carte du consent google est un patchwork
  `input`/`form`/`div`/∅ — comprendre où le point atterrit
  explique chaque `Ignored`). Validation terrain consent
  (avec les quantifieurs lazy de JS-C48) : la page rend ses
  scripts sains, `location` redirect JS effectif →
  google.fr — la chaîne consent→accueil google marche de
  bout en bout, preuve live de M-H1 #7. Constat restant
  noté : `keydown` google jongle le focus entre les events
  (un `key` sur deux rapporte `unfocused` — la machinerie
  suggest re-focus ; à trier côté miroir).
- **Étape 90 — rendu de commit nav détaché**
  (`explorer-core`) : constat M-H1 #6 — `sub_commit`
  tenait `sess.lock()` pendant toute la cascade + layout
  + raster du document final : ~85 s de famine sur
  terrain (`/status`, `/frame`, input et autres onglets
  gelés — le « daemon injoignable sous charge » du vol).
  Livré : le commit partagé se scinde en tête sous verrou
  court (`sub_commit_head` : epoch, CSS liées, fontes,
  évaluations JS, drains post-script), rendu hors verrou
  en `spawn_blocking` sur la `PaintPayload` détachée
  (`insert_images_payload` + `relayout_dom_payload` —
  même mécanique que l'apply 55h : `take_nav_paint`,
  `NavPaint::Busy` pollé, `commit_nav_paint` gardé par
  epoch, `rearm_nav_paint` au panic), puis queue sous
  verrou (`sub_commit_tail` : historique, `Navigated`,
  cookies, fragment, `mark_hist_ok`). Le chemin
  `&mut self` (CLI/FFI/tests) conserve le commit inline.
  Réserve honnête : la durée de la cascade est inchangée
  — c'est le chantier perf PIPE ; l'API reste serviable
  pendant le rendu, c'est la famine qui disparaît.
- **JS-C47 — `error.stack` = pile d'appels réelle**
  (`explorer-jscore`) : le stub `Name: message` est remplacé
  — `Engine.call_stack` empile le `Rc<FnLit>` à chaque appel
  script (natives exclues), `FnLit.pos` = `(ligne, colonne)`
  du jeton ouvrant le littéral, capturée aux 10 sites du
  parseur. `stack` se capture à la construction (spec) au
  format V8 `at f (l:c)` borné à 10 frames ;
  `Error.captureStackTrace` rend les mêmes frames ;
  `describe` préfère la prop `stack` → `JsError::Uncaught`
  cite désormais la fonction fautive et son site — `Error: b`
  est localisable *dans* le script (le src vient de JS-C46).
  Écart : position du site d'appel (l'AST n'a pas de span
  par statement — la définition y supplée).
- **JS-C51 — `navigator.languages`** (`explorer-jscore`) :
  demande PIPE→NAV au journal — sur example.com,
  `navigator.languages.some(...)` jetait `TypeError` et
  tuait `/s.js` entre ses deux `insertAdjacentHTML` : le
  `<svg>` logo (style inclus, `"afterbegin"`) n'entrait
  jamais dans le miroir alors que la chaîne PIPE
  (sérialisation → re-parse → `collect_svg`) était saine.
  Livré : tableau **gelé** `["en-US","en"]` dérivé de
  `language`, même instance à chaque lecture (spec),
  low-fingerprint — la seconde injection s'exécute et le
  svg atteint le DOM.
- **JS-C43 — parseur regex, motif `[` littéral en classe**
  (`explorer-jscore`) : dernier rejet du quadruplet mesuré
  sur github.com/githubstatus.com — `[.*+?^${}()|[\]\\]`
  échouait « Invalid character class » car les littéraux de
  classe étaient émis bruts alors que `[`/`&`/`~` ont une
  syntaxe de classe côté backend (set/intersection/diff).
  Livré : émission via `class_char` (`\x{…}`), y compris la
  borne haute brute d'une plage — les trois autres motifs
  (lazy `*?`, `[\0-\x1f]`, `\uD800`) étaient déjà colmatés
  par le rapatriement JS-C48. Test :
  `regexp_class_syntax_chars_escaped`.
- **JS-C36 — harnais Acid3 + 3 fixes mesurés**
  (`explorer-jscore`) : `tests/acid3.rs` exécute les 16
  sous-tests ECMAScript purs d'Acid3 (81-96, extraits
  verbatim de `wpt/acid/acid3/test.html` vers
  `tests/conformance/acid3-js-subset.js`) — **16/16, 0 skip**
  sur ce sous-ensemble, cliquet anti-régression. Écarts
  colmatés : `-0` rendu sans signe par `toFixed`/
  `toExponential`/`toPrecision` ; `\N` backref vers un
  groupe pas encore fermé → match vide (spec 21.2.2.9,
  suivi `open_stack`/`closed` dans le traducteur) ; le nom
  d'une `FunctionExpression` est désormais ReadOnly dans son
  propre scope (`Binding.sloppy_silent` — silencieux en
  sloppy, `TypeError` en strict).
- **R-53 — `width:%` d'un flottant résolu sur le bloc
  conteneur** (`explorer-layout`) : `place_float` passait la
  bande résiduelle comme base `cb_w` — un `float; width:100%`
  résolvait son % sur l'espace laissé par les flottants
  voisins au lieu du bloc contenant (§10.3). Bug site-réel :
  le squelette `float:left; width:100%` de weather.gov
  (`.div-full`, `.full-width-*`) s'effondrait en colonne de
  ~1 caractère/ligne. Livré : `layout_block_in_band` — la
  bande ne borne plus que le shrink-to-fit `width:auto`
  (§10.3.5) ; marges pré-résolues et `min|max-width` alignés
  sur la même base. Terrain : carte d'alertes US pleine
  largeur, légende 4 colonnes lisible. Tests `block.rs` :
  `width:100%` = 400 du conteneur + descente sous le voisin,
  `width:50%` = 200 en ligne.
- **R-54 — `grid-column:<ident>` résout `-start`/`-end`
  implicites** (`explorer-layout`) : CSS Grid §8.5 — le match
  des noms de lignes était exact ; un ident nu comme
  `grid-column: content` avec `[content-start]`/`[content-end]`
  définis retombait en auto-placement (squelette MDN
  `var(--layout-*)` + `.homepage > *{grid-column:content}` →
  cartes écrasées à ~115px). Livré : repli `<ident>-start`
  côté début / `<ident>-end` côté fin ; ident seul → span
  bornée par `<ident>-end` quand elle existe. Terrain :
  cartes featured/latest-news/contributions en pleine
  colonne `content`. Tests `modern.rs` : ident nu sur piste
  minmax, forme `a / b` sur lignes implicites.
- **R-55 — `overflow` clippe les lignes propres de la boîte**
  (`explorer-render`) : `paint_self` émettait les fragments
  `bx.lines` avant le `PushClip` — une boîte racine de
  contexte inline (élément `display:inline` positionné
  `absolute`/`fixed` ou flottant) à `overflow:hidden`
  laissait son propre texte fuir le clip. Bug site-réel :
  le `.sr-only` du wordmark lobste.rs (`position:absolute;
  width/height:1px;overflow:hidden`) peignait « Lobsters »
  par-dessus la nav. Livré : émission extraite en
  `paint_lines`, appelée dans la zone clippée ; l'ouverture
  du clip compte aussi `bx.lines`. Terrain : wordmark
  masqué, seul le carré logo 1rem reste. Tests
  `display.rs` : texte d'inline absolu entre `PushClip` 1px
  et `PopClip` ; `inline-block` déjà juste, verrouillé.
- **R-56 — media queries range MQ4 `(width <= N)`**
  (`explorer-css`) : les conditions range sans `:` —
  `(width <= 1044px)`, `(width > N)`, `(v1 <= width <= v2)`
  — devenaient un nom de condition inconnu → query fausse.
  Bug site-réel : tout l'habillage conditionnel de MDN est
  en syntaxe range — le `display:none` du popup nav sauté
  (menu brut peint), styles mobile/desktop absents. Livré :
  `parse_range_condition` normalise en `min-`/`max-`/`gt-`/
  `lt-` (`>=`≡min, `<=`≡max, stricts exclusifs) — formes
  `(name op val)`, `(val op name)`, double borne, ops
  espacés ou collés ; `condition_matches` gagne les bornes
  strictes `gt-|lt-width|height`. Terrain : shot MDN —
  popup masqué, cartes grille 2 colonnes, footer 4
  colonnes. Tests `cascade.rs` : `<=`/`>`/borne exacte,
  collage, double borne, `and`.
- **R-57 — custom properties `--*` en table `Arc`
  partagée** (`explorer-css`) : `ComputedStyle` copiait
  chaque `--*` héritée dans chaque nœud — mesuré
  3,26 M d'entrées dans le boxtree + 1,89 M dans
  `prev_styles` sur Wikipedia « Rust » (~1,1 Go de
  rétention commise). `vars: Arc<HashMap>` : un nœud
  sans `--*` propre partage la table du parent
  (héritage O(1)) ; déclaration = clone-on-write
  `Arc::make_mut` unique. `Arc` requis par `Send`
  (`spawn_blocking` du payload). `var()` résout
  propres-puis-héritées, `set("--x")` en CoW isolé ;
  `get`/`iter`/`PartialEq` inchangés. Sonde
  `tab_mem_inventory` : **RSS 1249 → 205 Mio**,
  drop tree 850→58 Mio, prev_styles 244→21 Mio.
  Tests : sémantiques externes + `ptr_eq` internes.

### Added

- **R-43 — `box-sizing` (css-sizing-3 §4)** (`explorer-layout`) :
  trou site-réel au triage M-H1 — le bouton Connexion de
  google.fr rendait ~60 px au lieu de 40 (`min-height:40px`
  + `padding:10px` sous `*{box-sizing:border-box}`, toutes
  les tailles spécifiées étaient lues en content-box).
  `box_size_to_content` convertit `width`/`height`/`min-*`/
  `max-*` border-box → contenu (plancher 0) dans
  `layout_block`, `abs_geo`, les bases flex (`item_flex`),
  la mesure de piste grid `auto` et `content_px` ; les
  write-backs flex écrivent la valeur border-box
  équivalente pour que `layout_block` retombe sur la cible.
  `shot` google.fr : bouton à ~40 px.
- **R-42 — unités viewport `vw`/`vh`/`vmin`/`vmax`**
  (`explorer-layout`/`explorer-css`) : parsées mais jamais
  résolues (`px_factor` → `None`) — `min-height:100vh`,
  `max-width:90vw`, héros `100vh` s'évaporaient.
  `BlockCtx.viewport_w` (additif, défaut 800), résolution
  dans `raw_length_px` + `eval_calc` (`calc(100vh-60px)`
  inclus), media queries (MQ4 §6.4) et `font-size:Nvw`
  contre `env`/`ctx`. Carve-out : `explorer-core` câble
  `ctx.viewport` dans `BlockCtx` (`viewport_h` était au
  défaut 600 — `position:fixed` aussi plus juste).

### Fixed

- **R-41 — flex colonne : base `%` et `grow` en hauteur
  indéfinie** (`explorer-layout`) : bug site-réel au
  triage `site_smoke` — le pied de rust-lang.org se
  peignait sur le héros. `body{flex column}` +
  `main{flex:1}` : `flex-basis:0%` contre taille main
  indéfinie résolvait `0` au lieu de `content` (§9.2) et
  l'espace libre indéfini ne distribuait rien (§9.9.2 :
  `grow` → max-content) — `main` contribuait 0 → footer
  posé à l'origine de main. Livré : `%` `flex-basis`/
  `height` contre indéfini → `content` ; `grow` en
  indéfini → hauteur mesurée. `shot` après fix : page
  complète enchaînée, footer en bas.
- **R-40 — ratio intrinsèque des remplacés (§10.6.2)**
  (`explorer-layout`) : bug site-réel au triage
  `site_smoke` — la bande blanche verticale de
  rust-lang.org était ses 4 icônes sociales SVG
  empilées : `img{max-width:100%}` (Normalize/Tachyons)
  bridait la largeur (~70 px) mais la hauteur restait
  à l'intrinsèque (511 px) → blobs étirés ~70×500.
  Spec : un axe `auto` d'un remplacé dérive de l'axe
  utilisé via le ratio. `width:auto`+`height` →
  `w = h × iw/ih` ; `height:auto` → `h = w × ih/iw`
  après bridage `min|max-width`. `shot` rust-lang.org
  après fix : icônes carrées proportionnées, bande
  disparue.
- **R-39 — `em`/`rem` dans les media queries**
  (`explorer-css`) : bug site-réel au triage `site_smoke`
  — rust-lang.org se rendait comme une page mobile
  empilée à 1024 px : Tachyons met tout son CSS desktop
  derrière `(min-width: 30em)`/`60em`/`64rem`, mais
  `condition_matches` ne résolvait que les `px` nus —
  toute feature non-px évaluée fausse, colonnes/flex
  desktop morts. Spec MQ4 : `em`/`rem` en media query =
  font-size **initiale** (`env.font_size_base`, jamais
  le `:root` calculé). Résolution `px`/`em`/`rem` sur
  les six conditions `*-width`/`*-height` ; `shot`
  rust-lang.org après fix : footer 3 colonnes, flex
  restaurés.
- **R-38 — mode quirks propagé à la cascade** : bug
  site-réel au triage `site_smoke` — les titres de
  news.ycombinator.com rendaient **centrés** dans leur
  cellule au lieu d'alignés à gauche après le rang.
  Page sans DOCTYPE (quirks) enveloppée de `<center>` :
  son `text-align: -webkit-center` UA traversait
  l'héritage jusqu'aux `td`. La spec (HTML Standard §
  rendering, « In quirks mode ») exige que `table`
  réinitialise les propriétés texte héritées —
  `font-weight`, `font-style`, `font-variant`,
  `font-size`, `line-height`, `white-space`,
  `text-align` à `initial`. Le `QuirksMode` du
  tree-builder n'atteignait pas la cascade ; il voyage
  désormais par le `Dom` (`DocumentMode` + accesseurs,
  synchronisé dès la décision du DOCTYPE — valable sur
  arbre partiel, peinture progressive) et une feuille
  UA conditionnelle `QUIRKS_UA_CSS` ne candidatant
  qu'en quirks. Zéro changement de signature.
- **Étape 84 — `back` pendant une navigation en vol**
  (`explorer-core`) : bug site-réel mesuré par `site_smoke`
  — « précédent » pendant la vague de sous-ressources d'une
  page répondait **502 `EmptyDocument`** (repro clic→iana.org,
  fenêtre ~10 s ; sur github ~100 s). Racine : `nav_apply`
  commet l'URL mais l'entrée d'historique n'est poussée qu'en
  `sub_commit` après `sub.run()` — pendant le vol,
  `hist_target(-1)` partait de `hist_idx` (page quittée) au
  lieu de la position logique `hist_idx + 1`. Nouveau marqueur
  `Tab::hist_pending` posé à `nav_apply`, résolu à
  `sub_commit`/`paint_error_page`/restauration ;
  `hist_target`, `history_move`, `can_back`, `can_forward`
  (le futur est mort — tronqué au commit) et
  `try_forward_nav` comptent la position logique. La page en
  vol n'est **pas gelée** (pas d'entrée = pas de slot — la
  geler sous `hist_idx` empoisonnait le snapshot de la page
  quittée) et `restore` bump `nav_epoch` : la nav en vol est
  supplantée, son `sub_commit` tardif jeté par la garde
  epoch. Comme Chrome : back annule le chargement et restaure
  l'entrée précédente (bfcache instantané, sinon rejeu).
  Régression : `back_during_inflight_nav_restores_previous` +
  `hist_target_during_inflight_nav`. Terrain : clic→iana,
  `back` en vol = 3 ms, `example.com` restauré.

- **Étape 83 — revalidation cache : 304 gzip vide +
  partiels tronqués** (`explorer-net`) : bug site-réel
  mesuré par `site_smoke` — wikipedia échouait de façon
  **déterministe** via daemon (5/5) alors que le cli
  passait toujours. Frontière : le cache disque du
  daemon (absent du cli). Deux défauts. (1) Une entrée
  périmée déclenche une revalidation conditionnelle ;
  le 304 porte `Content-Encoding: gzip` (en-têtes de la
  ressource) avec un corps **vide** — `DecodingTransport`
  (couche interne, pile `Cache → Decoding → h1`)
  l'échouait en `unexpected end of file` avant que le
  cache ne serve l'entrée. `decode_response` saute
  désormais les statuts sans corps (1xx, 204, 304 —
  RFC 9110 §15), et la fusion 304 ne propage plus
  `content-encoding`/`content-length`/`transfer-encoding`
  vers l'entrée (corps stocké décodé). (2) Un corps
  compressé **tronqué** (h2 qui coupe tôt) produisait
  une erreur sèche : on conserve maintenant le partiel
  décodé — le parseur tag-soup amont est fait pour ça
  et Chrome/Firefox rendent pareil — mais marqué
  `Cache-Control: no-store` pour que le cache ne stocke
  jamais un incomplet comme sain. Repro verrouillé :
  `stale_304_through_decoding_layer` sur la chaîne
  réelle `Caching → Decoding`, plus 4 tests décodeur
  (304/204/1xx, partiel→no-store, intact→cacheable,
  header-corrompu→erreur honnête).

- **Étape 85 (suite) — AltGr et caractères composés
  atteignent les champs** (`app/`) : deux trous du relais
  clavier repérés avant le test humain. (1) Le garde
  « frappes nues » bloquait Ctrl **et** Alt — or Windows
  synthétise `AltGr` en `Ctrl+Alt` : `@ € { } [ ] ~`
  n'atteignaient plus le champ. `altRight` est exempté —
  corde de texte, pas geste chrome. (2) Le relais envoyait
  `keyLabel` (la touche nue) : `AltGr+à` tapait `à` au
  lieu de `@`, `maj+lettre` tapait la minuscule, et les
  accents par touches mortes (`ê`, `ë`) sortaient sans
  accent. On relaie désormais `e.character` — le
  caractère **produit** par la plateforme — avec repli
  `keyLabel` pour les touches nommées (`Enter`,
  `Backspace`) qui n'ont pas de caractère.

- **Étape 86 — resynchronisation `fieldFocused` /
  outcome `unfocused`** (`explorer-core` + `api` + `ffi`
  + `app/`) : désync mesurée en relisant la chaîne de
  saisie avant le test humain — l'app n'apprenait
  `fieldFocused` qu'au clic et jamais ne le lâchait ;
  après `Enter`/`Escape` (focus effacé côté moteur) ou
  la suppression du nœud par un handler, chaque frappe
  suivante partait dans le vide en `ignored` muet, le
  champ semblant focalisé. Nouveau `InputOutcome::
  Unfocused` — « aucun champ focalisé à l'arrivée ou à
  l'issue » — distinct d'`Ignored` (un `keydown`
  consommé par `preventDefault` laisse le champ
  focalisé). Chemins : frappe sans champ résolu,
  `Enter`/`Escape`, soumission `Enter` bloquée par la
  validation. API → `"unfocused"`, FFI → code `6`.
  Côté app : `key()` lit enfin son résultat (`unfocused`
  /`navigated` → flag à zéro), l'événement `navigated`
  de l'onglet actif et tout `selectTab` le
  réinitialisent. Tests : `enter_escape_unfocus_reports_
  unfocused`, 3 assertions existantes rebasées
  (`Ignored`→`Unfocused`), contrôleur `unfocused`→flag
  et `selectTab`→reset.
- **Étape 87 — caret du champ focalisé** (`explorer-core`) :
  constat M-H1 #1 du vol humain — un `input`/`textarea`
  focalisé n'affichait **rien** avant la première frappe
  (le PNG ne portait aucune barre d'insertion). Le caret
  est un overlay de la display list (même mécanisme que
  find/popup `<select>`), en coords **frame** (`scroll_y`
  retiré) : barre 1 px couleur du texte du champ, à la fin
  du dernier fragment posé (`rect.x + width + trail_w`),
  ou à l'origine du contenu quand la valeur est vide — un
  `placeholder` affiché n'avance pas le point d'insertion.
  `PaintPayload` transporte un snapshot `focused`
  (copié à la sortie, jamais ré-écrit — le focus reste
  autoritaire sur `Tab`) ; `repaint_payload` reçoit un
  `dom_hint` pour résoudre le chemin quand `p.dom` est
  sorti le temps du relayout. Effets de bord corrigés au
  passage : le clic-focus et la perte de focus
  (`Escape`/clic hors champ) repeignent désormais +
  `emit_invalidated` — avant, le caret n'aurait pu
  apparaître qu'à la frappe suivante. MVP sans
  clignotement ni point d'insertion interne (même champ
  que `edit_field` : insertion en fin seule). Test :
  `caret_paints_moves_and_clears_with_focus` (barre
  ≥16 px contigus — discriminant fût-de-glyphe) +
  `key_and_focus_events_dispatch_in_order` rebasé
  (défocalisé par `Escape` avant la comparaison PNG).
  Divergence connue documentée dans le module : les
  overlays find/popup restent en coords **document**
  (décalage sous scroll pré-existant).
- **Étape 88 — quick wins M-H1 (#2 #3 #4)** (`app`,
  `explorer-core`) : trois frictions du vol humain.
  1. **Snackbar `ApiException` persistant** : `lastError`
  n'était jamais purgé — l'erreur « daemon injoignable »
  survivait à la reconnexion auto (et une récidive à
  l'identique ne re-signalait rien, le `select` ne
  refirant pas sur valeur égale). Le bootstrap réussi
  pose `lastError: null` ; la page masque le snackbar
  encore visible quand l'état s'efface.
  2. **`Ctrl`+molette** : `onPointerSignal` ne lisait que
  `scrollDelta.dy` → scroll pur. `isControlPressed` aiguille
  vers `zoomProvider` (molette haut = zoom avant,
  convention navigateur) ; sans modificateur, scroll
  moteur inchangé.
  3. **Bouton Home + `about:blank`** : icône `⌂` après
  « Recharger » (barre large) et entrée « Accueil » du
  menu ⋮ (compact) → `navigate('about:blank')`. Le
  moteur intercepte `about:`(`blank`|vide) dans
  `NavJob::run` **avant le fetch** : le squelette
  html/head/body du tree-builder (Eof) est commis tel
  quel — `navigate`, `reload`, `back`/`forward`
  partagent le même chemin. Tout autre `about:*` tombe
  sur le transport → page d'erreur. L'omnibox laisse
  passer `about:` sans préfixer `http://`. Tests :
  `about_blank_navigates_sans_fetch` (compteur transport
  inchangé = preuve du non-fetch, frame PNG, back) ;
  contrôleur : purge `lastError` à la reconnexion,
  `about:blank` non préfixé.
- **Étape 91 — entrées différées pendant la peinture
  détachée** (`explorer-core`) : triage jonglerie focus
  google (M-H1) — la fenêtre `paint_detached` d'Étape 90
  laisse l'onglet sans `dom`/`tree` (chez la tâche
  d'apply) : chaque entrée y tombait `unfocused`
  mensonger ou tuait le chemin de focus. Livré :
  `deferred_inputs` bornée (32) sur `Tab`, coalescence
  `Scroll`/`MouseMove` comme le client (ADR-0003) ;
  `handle_input_now` diffère quand la fenêtre est
  ouverte **ou la file non vide** — FIFO strict, sinon
  une frappe directe dépasse le replay (« explorer »
  rendait « erx ») ; drain à la réinsertion de payload
  (apply synchrone/détachée, commit nav), purge à la
  nav/erreur/rearm. Terrain : les touches différées
  d'une frappe google rejouent à la livraison. Test :
  `deferred_inputs_fifo_and_coalesce`.
- **Étape 92 — focus ré-ancré par identité quand le
  chemin meurt** (`explorer-core`) : suite du triage
  jonglerie google — le listener `input` (suggest) mute
  le DOM, l'apply re-parse et le chemin `(tag, index)`
  du champ meurt ou résout sur un voisin. Livré :
  `focused_key = (id|name, value)` capturée à
  `set_focus` ; `focused_node` vérifie l'ancre sur le
  nœud résolu et, mort ou usurpé, retrouve l'élément
  par `find_by_attr` puis ré-écrit le chemin — le
  focus survit au déplacement ; l'ancre voyage dans le
  `PageSnapshot` (bfcache). Terrain : « explorer »
  tapé sur google rend « explorer| » exact, Enter
  soumet. Test : `focus_reattaches_by_identity`.
- **JS-C50 — `eval_hook`, observabilité des sources
  évaluées dynamiquement** (`explorer-jscore`) : le code
  obfusqué des pages réelles (bootstrap Google) vit dans
  `eval`/`Function`/timer-string — les frames JS-C47
  pointaient des positions dans des sources invisibles.
  Livré : `Engine::set_eval_hook` additif, invoqué avant
  parse sur `eval_source`, `eval_source_in` et
  `Function(...)` ; `census --dump-scripts` écrit
  `eval_<n>.js` — le bundle Closure complet de Google
  (`eval_0.js`) est devenu rejouable en isolation,
  permettant la bissection du bug corrigé en JS-C52.
- **JS-C52 — ordre d'évaluation des affectations :
  LHS avant RHS** (`explorer-jscore`) : le bootstrap
  Google mourait `TypeError: vG is not a function` —
  `Expr::Assign` évaluait le RHS avant la référence
  LHS, alors que le code Closure installe l'accesseur
  dans le LHS : `(..., SL(kW,…), K).Wn = kW.vG()`.
  Livré : `LValueRef` réifiée — `resolve_lvalue`
  évalue `base`/`key` avant le RHS,
  `read_lvalue_ref`/`store_lvalue` la consomment ;
  bonus spec : `a[i()] op= x` n'évalue plus `i()`
  deux fois. Bonus diagnostic : `recv_desc` affine —
  callable → `function <anon>@l:c` ou `.name`, objet
  → `constructor.name` (« `x` is not a function (on
  `D7`) » en minifié). Terrain : `eval_0.js` rejoue
  `ok` ; la page résultats google émet `[navigate]
  https://www.google.com/?sei=…` — le redirect
  `location.replace` tire. Tests :
  `assign_lhs_evalue_avant_rhs`,
  `assign_index_key_evalue_avant_rhs`,
  `assign_compound_key_evaluee_une_fois`.
- **Étape 93 — reload POST : verrou `resubmit_confirm`**
  (`explorer-core`, `explorer-api`, `app/`) : le endpoint
  REST `…/reload` court-circuitait `Session::reload` —
  il reconstruisait un `GET` nu sur l'URL, si bien que
  **tout** reload applicatif était un GET vide, même sur
  une page servie par POST avec `last_req` présent
  (constat M-H1). Livré : `Session::reload_request`
  expose la requête de rejeu — `last_req` (méthode +
  corps, étape 56), la requête `failed` de l'entrée
  courante (page d'erreur : retenter la requête tombée
  au lieu du `GET` vide), sinon `GET` top-level ; le
  endpoint rejoue cette requête, mais une méthode non
  sûre (≠ `GET`/`HEAD`) exige le corps `{"confirm":true}`
  — sinon statut typé `resubmit_confirm`, aucune
  resoumission silencieuse (effet de bord : commande,
  paiement). L'app affiche le dialogue navigateur
  « Renvoyer le formulaire ? » : « Renvoyer » rappelle
  confirmé, « Annuler » n'émet rien ; le chemin FFI
  in-process rejoue directement (le verrou vit dans
  REST). Tests : `reload_request_on_failed_post_replays_request`,
  `post_reload_requires_confirm`, contrôleur
  `resubmit_confirm`.
- **`scripts/site_smoke.ps1` — sondes sur client HTTP
  poolé** : chaque sonde `Invoke-RestMethod`
  ouvrait une connexion TCP neuve ; ~16 000 sockets
  TIME_WAIT épuisaient les ports éphémères Windows en cours
  de phase 2 → les 4 derniers sites tombaient en
  `WSAEADDRINUSE`, faux échecs. `Send-Api` mutualise un
  `HttpClient` keep-alive (timeouts par
  `CancellationTokenSource`, mode `-Raw` pour `/frame`).
  Re-mesure complète après le lot JS-C36→C52 / R-43→R-53 /
  étapes 91-94 : **10/10 rendus, 10/10 fonctionnels**
  (nav + lien + back + dispatch), 3 TIME_WAIT résiduels.

## [0.2.0] - 2026-10-20

### Added

- **Étape 59 — release 0.2** : version workspace
  `0.1.0` → `0.2.0` ; `scripts/package.ps1` mis à jour
  (bundle `dist/explorer-0.2.0`) ; `scripts/smoke.ps1`
  **étendu** — le serveur loopback route désormais par
  chemin et le test couvre les faits de la release en bout
  en bout : `onclick=` compilé (mutation rendue — PNG
  différent après clic), clic de lien → navigation puis
  `back`, `scroll` → outcome `scrolled` + `scroll_y`
  (contrat 71d), `<video>` → offre `GET /media` (seam
  71b), `find` → occurrences comptées. Faits marquants du
  cycle : `MediaSurface` complet moteur→hôte (71a-d) avec
  lecteur natif `video_player`+`fvp`, handlers inline
  `on*` (JS-C45), récupération automatique `daemonOnline`
  (81), navbar compacte (80), frame affichée à `dpr`
  exact (82), renommage `dillo-js` → `explorer-jscore`
  (ADR-0020).

- **Étape 71d — composite Flutter `MediaSurface`** (`app/`) :
  le navigateur joue de la vidéo/audio natifs. `scroll_y`
  transporté par `frame_invalidated` (champ additif du
  contrat — l'overlay se repositionne **dans le même event**
  que le repaint : scroll moteur, ancre `#`, `scrollTo` JS
  ne dérivent plus). `media_offered` → `POST …/claim`
  (`AppConfig.mediaCodecs`) → lecteur `video_player`+`fvp`
  (MDK/FFmpeg, linking dynamique — licence vérifiée) ;
  `media_command` routé vers le lecteur, `POST …/state`
  borné à ≤ 4 Hz sauf changement d'état ; `media_moved`/
  `gone`/`tab_closed`/`navigated` purgent et disposent.
  Overlay `Positioned` à `rect − scroll_y` × zoom sur la
  frame étape 82 ; audio 0×0 joue sans overlay ;
  `controls` → tap = play/pause (v1). FFI : stubs
  `UnsupportedError` (variante in-process au premier hôte
  FFI — journal d'interface).
- **R-37 — Display-list incrémentale** (`explorer-render`,
  `explorer-layout`, `explorer-core`) :
  `build_display_list_incremental(prev, tree_epoch)`
  copie les plages d'items des sous-arbres prouvés
  équivalents au lieu de les ré-émettre —
  `DisplayList.spans` indexe chaque boîte (chemin
  d'enfants) vers sa plage `[start,end)` avec rect,
  offset effectif, signature de contenu (`box_sig`) et
  identité mémoire. Une plage ne se copie que si
  signature + taille identiques **et** translation
  uniforme récursive — un descendant `fixed`, sale ou
  restructuré fait redescendre le parent (ré-émission
  locale, jamais de copie fautive). Porteurs inline
  reportés : `F_DEFERS` + chemins `escaped` re-poussés
  au drain englobant (le premier essai faisait
  disparaître le texte — attrapé par l'équivalence).
  **Fast-path `tree_epoch`** : le relayout incrémental
  session mute `p.tree` en place → epoch inchangée ⇒
  identité par pointeurs `{bx_ptr, kids_ptr, lines_ptr,
  kids_len}` sans signature ; `*bx = fresh` alloue avant
  de libérer → adresses disjointes garanties, feuilles
  (ptrs dangling) exclues du fast-path. Contrat
  auto-testé : en debug la signature reste vérifiée
  quand le fast-path tire — il a attrapé `translate_y`
  (géométrie de fragments mutée in-place → retirée de
  `box_sig`, dérivée et couverte par rect+Δ).
  `FontCache::shape` retourne `Arc<ShapedRun>` (runs
  partagés, mémoire dédupliquée) ; `box_sig` hache le
  **contenu** des glyphes (une fonte chargée entre
  builds est détectée). Session : `p.list` stockée non
  cullée (le cull purgeait les spans à chaque frame) —
  cullée = copie jetable du plein cadre, diff calculé
  non cullé (plus précis). **Équivalence d'abord** :
  15 tests `incremental_list` assertent dump
  item-par-item + frame pixel-à-pixel sur identique,
  texte, restyle, translations rigide/non-rigide,
  scroll, fixed/sticky, clip, floats/positionnés,
  inline drains, image, insertion de frère, frame
  session. Mesure release (doc 84 000 items / 60 002
  spans) : scroll **74 vs 197 ms (×2,7)**, restyle
  142 vs 196 (×1,4), shift 161 vs 206 (×1,3).
- **R-37 — Raster borné par dirty-rects** (`explorer-render`,
  `explorer-core`) : `rasterize_dirty` repeint seulement
  les fenêtres de `DisplayList::changed_rects` par-dessus
  la frame précédente — calque blanc par fenêtre (fond UA),
  inflation 2 px CSS (AA + overhang glyphe), fusion des
  recouvrements, filtre d'items par `resolved_bounds`
  (clip/transforms résolus), repli plein cadre sur cible
  différente ou borne dépassée. `repaint_payload` consomme
  les régions — avant, le diff n'alimentait que les events
  `frame_invalidated`. Équivalence **pixel-à-pixel**
  « régional == plein cadre » assertée par 9 tests
  permanents avant le gain — la méthode a attrapé au
  premier atterrissage un bug de fusion de fenêtres
  (disjointes en y avalées → session
  `checkbox_toggles_then_submits` rouge ; régression
  verrouillée). Mesure : doc 18 001 items, mutation
  localisée — 2,10 s → **8,2 ms** (×256, pixels
  identiques). Seam `MediaSurface` intacte : rects média
  toujours relus du boxtree post-relayout — le dirty ne
  borne que le raster.
- **R-33b — Perf : `FontCache` + encodage PNG rapide**
  (`explorer-layout`, `explorer-render`) : part pipeline
  du « mur frère » (nav `heavy.html` ~3 000 nœuds gelant
  l'API sous verrou). Attribution sur
  `profile_resync_phases` (~9 000 nœuds, debug) :
  `layout_tree` = **27,7 s**, quasi-totalité fonte —
  chaque run re-facturait `font_data` + `Face::parse` ×2 +
  shaping `rustybuzz` (~10 appels/item, aucun cache).
  **`FontCache`** borné à une passe : octets par
  `FontRequest` (miss figé), métriques par (fonte, taille),
  shaping par (fonte, taille, texte, dir) borné à 8 192
  entrées ; `BlockCtx::font_cache` en layout, cache propre
  en `rasterize`. **`Frame::encode_png`** :
  `image::PngEncoder` `Compression::Fast`/`NoFilter`
  (fdeflate) — démultiplication prémultiplié → alpha
  conservée à l'identique. Résultats : layout 27,7 →
  **1,09 s** debug (×25), `relayout` 24,8 → 3,78 s debug /
  **289 ms** release, `encode_png` 4,3 s → **125 ms**
  release. Reste chez NAV : répétition des `relayout_dom`
  progressifs sous verrou et miroir DOM→JS de
  `spawn_engine` (réception mission 21-F).
- **Étape 71b — Seam `MediaSurface` : session + REST**
  (`explorer-core`, `explorer-api`, ADR-0024) : les
  éléments `<video>`/`<audio>` lisibles sont détectés au
  commit et offerts à l'hôte. `CoreEvent` gagne
  `MediaOffered`/`MediaMoved`/`MediaGone`/`MediaCommand`
  et les DTO figés `MediaKind`/`MediaRect`/`MediaCmd`/
  `MediaHostState`/`MediaReport`/`MediaOfferInfo`/
  `MediaError` (contrat `docs/interface-equipes.md`).
  Scan **DOM** + carte `NodeId→rect` du boxtree : un
  `audio` sans `controls` (`display:none` UA) est offert
  à `rect` 0×0 — pipeline seul, pas d'overlay. Identité
  `media: u64` par instance, retenue positionnelle
  (`kind`+`url`) sur apply JS, `MediaGone` à la mort de
  page (bfcache : ré-offre = ids neufs) et à la fermeture
  d'onglet ; `MediaMoved` uniquement au refresh
  post-repaint (borne « 1/frame » par construction).
  REST : `GET /api/tabs/{id}/media[/{mid}]`,
  `POST …/claim` (premier arrivé gagne → `409`, id mort
  → `410`), `…/release`, `…/state` (`MediaReport`
  stocké, exige un claim). Sans claim : stubs ADR-0023
  inchangés ; la délégation JS (`MediaCommand`, props)
  arrive à 71c.

- **Étape 71c — Seam `MediaSurface` : délégation JS**
  (`explorer-jscore`, `explorer-js`, `explorer-core`,
  `explorer-api`) : la surface **claimée** délègue la
  lecture à l'hôte — `claim`/`release`/`state` poussés au
  miroir via `JsEngine::set_media_claim`/
  `deliver_media_state` (résolution `path` → nœud miroir),
  les ordres page (`play`/`pause`/`seek`/`volume`/`muted`/
  `rate`) drainés par la session partent en
  `CoreEvent::MediaCommand` SSE avec l'id de surface
  rappelé. `play()` rend une promesse tenue au premier
  rapport `playing` (rejetée `error`/`release` —
  `NotSupportedError`), `canPlayType` reflète les `codecs`
  du claim (`"probably"`/`"maybe"`/`""`), les props
  `HTMLMediaElement` reflètent le `MediaReport`
  (`paused`/`ended`/`seeking`/`currentTime`/`duration`/
  `buffered`/`played`/`seekable` en `TimeRanges` réels,
  `readyState`/`networkState`/`volume`/`playbackRate`/
  `error`) et les events DOM non-bubblants tirent sur les
  transitions (`loadedmetadata`, `durationchange`,
  `play`+`playing`, `pause`, `seeked`, `timeupdate`,
  `ended`, `error`, `volumechange`, `ratechange`,
  `seeking`). Identité : le `media: u64` session est
  transmis au claim et rappelé dans les commandes —
  robuste aux re-parse (les chemins d'élément peuvent
  bouger). **Clause honnêteté conservée** : non claimée,
  release ou hôte muet = stubs ADR-0023 inchangés, jamais
  de faux succès. Ajustement contractuel v1
  (pré-consommateur) : `MediaCmd::Muted{on}` ajouté —
  distinct de `volume=0` (le dé-mute restaure le volume) —
  et `Volume`/`Rate` alignés `f64`. Tests :
  `media_js_commands_only_when_claimed`,
  `media_js_report_resolves_play_and_reflects_props`.

- **R-33 — Perf : propagation par translation dans le
  relayout incrémental** (`explorer-layout`) : attribution
  du ×10 `incr.remap_relayout` (150,3 s terrain / ~14 s
  profil sur `storm.html`) — `step` rejouait `layout_block`
  sur **chaque ancêtre** dont la hauteur bougeait, soit
  ~2× le layout complet par mutation sur un document plat
  (`#cont` + ~600 `<p>` frères ; mesure : relayout 154 ms
  vs layout 74 ms). Le relayout propage désormais la hausse
  Δ par **translation** des frères suivants (§8.3.1 : sans
  flottant — garanti par `has_float` — leurs layouts sont
  invariants, marges inchangées), avec `resolve_absolutes`
  rejoué pour les positionnés. Gardes conservatives →
  `relay` classique : fils hors-flux, transition vide ↔
  non vide (casse la chaîne de margins collapsing),
  conteneur flex/grid/table/`min|max-height`. **Résultat :
  relayout 154 → 9,7 ms (×16)**, arbre strictement
  identique au reflow complet (2 tests permanents dans
  `relayout.rs`, équivalence positions à 0,01 près, dont
  le repli vide → non vide). Le poste suivant borné :
  cascade globale `styles_payload` (~69 ms, O(n) — non
  anormale).

- **R-31 — Acid2, première passe de causes** (`explorer-css`,
  `explorer-layout`, `explorer-render`) : 90,1 % → **96,5 %**
  (1282/1329 pixels de structure) — quatre causes racines.
  **Strut de ligne** (`inline.rs`, §10.8.1) : chaque line box
  démarre d'une boîte invisible de la `line-height` du bloc —
  baseline = demi-leading + ascent — `.chin` passe 4 px →
  12 px. **`;` orphelin en prélude** (`parser.rs`, CSS Syntax
  §5.4.4) : il entre dans la règle qualifiée au lieu de la
  terminer — `; .parser{height:3em}` devient un sélecteur
  invalide et la règle tombe (`.parser` = 1em, comme la
  référence 2005). **Marge basse échappée** (`block.rs`,
  §8.3.1) : le `margin-bottom` du dernier fils in-flow fuit
  sous un parent à `height:auto` sans bordure/padding — le
  `<ul>` à marge négative ne recouvre plus `.parser`.
  **`min|max-height|width` dans l'ordre spec** (§10.4/10.7) :
  max appliqué avant min (le min gagne les conflits) — la
  barre fixe `.picture p` fait bien 12 px (8 → 2mm → 1em).
  **CB des inlines `position:relative`** (`block.rs`, §10.1) :
  `resolve_absolutes` lit le style pour les boîtes inline
  (jamais de métadonnées posées), en excluant les boîtes
  anonymes/texte qui clonent le style parent. **Harnais** :
  émulation du scroll de capture (la référence a été shootée
  document défilé — `build_display_list_full(scroll)` en deux
  passes ; les boîtes/fonds `fixed` restent ancrés au
  viewport). **Seconde passe → 99,5 %** (1323/1329) : phase
  de tuile `background-attachment:fixed` ancrée au viewport
  (§14.2.1 — bug dormant : tuile déphasée de `scroll mod
  taille` dès qu'on scroll) ; `line-height: normal` aux
  **métriques réelles `hhea`** (§10.8.1 — le 1.2em codé en
  dur décalait tout le visage de 0.2 px, la vraie cause des
  « AA » yeux/nez ; Ahem sert 1em pile). **Plancher AA
  refermé** : 6 échantillons restants = mélanges sub-pixel
  sur les mitres 45° du losange (jamais une couleur franche)
  — borne connue, pas de la conformité.

- **ADR-0024 — lecture média : responsabilité d'hôte**
  (proposée) : seam `MediaSurface` (`MediaOffered` SSE +
  REST claim/play/pause/seek/volume) — le moteur garde
  stubs + poster (ADR-0023 intact sans hôte), l'app
  composite une surface vidéo au-dessus du rect déclaré ;
  surface JS devient réelle par délégation quand un hôte
  claim (jamais de faux succès). Zéro codec dans le
  workspace ; DRM (Widevine/FairPlay) et MSE documentés
  hors atteinte v1. Roadmap : étapes 71 (seam) et 72
  (résolveur DoH optionnel, prolonge ADR-0022) ouvertes
  côté NAVIGATEUR — aucun code modifié.

### Changed

- **Étape 73 — `/input` rend l'outcome à l'enfilement**
  (`explorer-api`) : les drains apply (spawn_blocking,
  >30 s sur heavy.html) et navs tournent en tâche
  détachée — la réponse ne retient plus le clic toute la
  durée du pipeline ; `result` = intention, le commit
  effectif arrive par SSE (`navigated`/`pipeline_error`/
  `frame_invalidated`). `handle_input` `&mut` (FFI/CLI)
  inchangé : outcome de commit réel.

### Changed

- **Étape 76 — nav échouée : entrée d'historique réelle**
  (`explorer-core`) : `Tab.hist` passe à
  `Vec<HistEntry { req, failed }>` — une navigation en
  échec pousse son entrée marquée (method+body, replay
  POST couvert) comme Chrome/Firefox. `back` depuis la
  page d'erreur revient à la page précédente — la cible
  n'est plus sautée (repro A→B→refus : back rendait A).
  La traversée d'une entrée `failed` **retente** la
  requête d'origine au lieu de restaurer la page d'erreur
  — bfcache ignore les entrées échouées et ne snapshotte
  pas la page d'erreur courante. Chemins `&mut` et
  partagé alignés.

### Fixed

- **JS-C45 — handlers inline `on*` compilés et dispatchés**
  (`explorer-jscore`, `explorer-core`) : premier bug
  site-réel mesuré (mission 10/24 — `jsmut.html`,
  `onclick=` muet). Deux trous : `fire_listeners` ne
  lisait que `listeners` + la prop scriptée `el.onclick` —
  l'attribut est maintenant compilé **paresseusement** au
  dispatch (cache `(attr, src, fn)` sur le nœud miroir,
  invalidé par la source : `setAttribute` recompile) avec
  le wrapper spec `with(document){with(this.form||{})
  {with(this){…}}}` — `env_get`/`env_set` des scopes
  `with` rendus host-aware (props réfléchies `id`,
  `class`… résolues en accès nu). **Et** la session ne
  spawnait le moteur que si des `<script>` étaient
  collectés : `has_inline_handlers` donne aussi un
  moteur aux pages à `on*` seuls — sans quoi le clic
  restait muet même compilé. Régression verrouillée :
  `onclick=` → mutation → PNG identique à la page
  statique cible (test session).

- **Étape 80 — navbar : trailing replié en menu
  « ⋮ » sous fenêtre étroite** (`app/`) : sous
  ~560 px, les 9 `IconButton` à largeur fixe
  (48 dp minimum M3) poussaient étoile,
  historique, signets, find, zoom et réglages
  **hors de la frame** — silencieux en release,
  « icônes absentes » en capture M22 alors que
  visibles en session large. `LayoutBuilder` +
  seuil `metrics.navCompact` : mode compact avec
  les actions dans un menu de débordement, à la
  manière de Chrome. Régression verrouillée par
  un widget test à 380 px.

- **Étape 81 — `daemonOnline` récupère automatiquement**
  (`app/`) : un bootstrap échoué figeait `daemonOnline:
  false` jusqu'au bouton « Réessayer » — alors que le
  canal SSE reconnectait en silence (`EventsClient` a sa
  propre boucle). La bannière « daemon injoignable »
  pouvait donc affirmer le contraire de la réalité.
  Re-bootstrap borné à `AppConfig.daemonRetryDelay`
  (3 s) : armé sur échec bootstrap **et** sur tout
  `ApiException` sans `statusCode` (« injoignable »,
  distinct d'une erreur HTTP qui prouve le daemon
  vivant), annulé au premier `listTabs` réussi.
  Récupération mid-session couverte : un daemon qui
  meurt puis revient remet l'UI en ligne sans geste.

- **Étape 82 — FrameView : frame clippée de `1−1/dpr`**
  (`app/`) : le raster moteur fait `viewport_css ×
  zoom·dpr` px physiques mais `Image.memory`/`RawImage`
  étaient rendus à `scale: 1.0` (défaut) → affichés en
  px logiques = px physiques, `dpr×` trop grands dans
  un `Clip.hardEdge` — ~33 % tronqués à dpr 1,5, ~43 %
  à 1,75, invisible à dpr 1,0. `scale: dpr` pose 1 px
  image = 1 px physique (rendu net conservé) ; le
  diviseur d'entrée passe de `zoom·dpr` à `zoom` — il
  n'était cohérent qu'avec l'affichage surdimensionné.
  Prérequis posé pour la validation de l'overlay média
  71d (positionner sur une frame tronquée ne mesurait
  rien).

- **Étape 78 — `DoH` : noms d'usage spécial sur le
  système** (`explorer-net`) : `localhost`, `.localhost`,
  `.local` (mDNS), `.test`, `.invalid`, `.example` et les
  noms de documentation RFC 6761 §6.5 ne partent plus en
  requête `DoH` — délégation `SystemResolver` avant cache
  (fixtures locales vivantes en mode `--doh`, comme
  Chrome/Firefox). Les littéraux IP court-circuitaient
  déjà côté `ResolvingConnector`.

- **Étape 75 — page d'erreur sans reliquats `find`**
  (`explorer-core`) : `paint_error_page` purge l'état
  interactif (find, popup, focus, hover) **avant**
  `relayout_dom` — les `FillRect` de surlignage de la
  page précédente ne cuisent plus dans la frame d'erreur
  (bande orange du 404 ; régression : 624 px cuits → 0).
  Les erreurs post-fetch de `nav_apply`/`sub_commit`
  peignent désormais la page d'erreur sur les deux
  chemins (trou de couverture de l'étape 69).

### Changed

- **Renommage `DilloEngine` → `JscoreEngine`**
  (`explorer-js`, ADR-0020) : le proxy-canal vers
  `explorer_jscore::Engine` portait encore son nom
  d'antan — le dernier résidu `Dillo*` du code est purgé
  tant que l'API est interne au workspace (cli, daemon,
  ffi, tests core mis à jour).

- **Étape 77 — sortie fatale du daemon visible**
  (`explorer-daemon`) : `bind` en échec et `axum::serve`
  terminé en `Err` écrivent la raison sur stderr direct
  (`eprintln!`) en plus du `tracing::error!` — un filtre
  `RUST_LOG` ciblé n'avale plus la cause de mort.

### Added

- **Étape 72 — résolveur DoH optionnel**
  (`explorer-net` + `explorer-api` + `explorer-daemon`) :
  trait `resolver::Resolver` injecté dans
  `h1::ResolvingConnector` (sous `TlsConnector`) — `system`
  (défaut, `lookup_host`) ou `doh` (`POST
  application/dns-message`, RFC 8484, format filaire
  embarqué sans dépendance : `A`/`AAAA`, pointeurs de
  compression) ; cache par hôte borné par TTL RR
  (10–300 s), IP littérales court-circuitées, endpoint
  résolu par le système (pas de récursion). Daemon
  `--doh`/`EXPLORER_DOH` + `EXPLORER_DOH_TIMEOUT_MS` ;
  `GET /api/egress` expose `{resolver}` — jamais de
  basculement silencieux.

- **Étape 74 — `constructor.name` des éléments**
  (`dillo-js`) : `v.constructor.name` vaut
  `HTMLVideoElement`/`HTMLAudioElement`/`HTMLCanvasElement`
  (`HTMLElement` ailleurs) au lieu de `Object` — objet
  allégé `{name}` non constructible (déviation v1).

- **Étape 70 — téléchargements** (`explorer-core` +
  `explorer-api` + `explorer-daemon` + app) : une réponse
  top-level non affichable (`attachment`,
  `application/octet-stream`, type inconnu) est bufferisée
  (borne 64 Mio) et proposée via SSE
  `download_offered`/`download_progress`/`download_done`
  au lieu d'être peinte en mojibake — la page courante
  reste. REST `GET /api/tabs/{id}/downloads`,
  `POST …/accept` (chemin ou répertoire du profil, nom
  sanitisé), `POST …/decline` (rejeu « dump texte »).
  Daemon `--download-dir`, dialogue Flutter
  « Enregistrer / Ouvrir comme texte ».

- **Étape 69 — pages d'erreur navigateur**
  (`explorer-core`) : une nav en échec (HTTP 404/5xx,
  connexion refusée/DNS, document vide) commet un document
  d'erreur généré par le moteur — statut, URL tentée, cause
  lisible — au lieu de laisser la page précédente ou un
  vide. `can_back` préservé (historique intact), reload
  retente la cible, `PipelineError` SSE inchangé.

- **Étape 53e — charset transport honoré pour le
  document** (`explorer-core`) : `StreamDecoder` prend le
  `charset=` du `Content-Type` en amorce — ordre WHATWG
  BOM > transport > prescan `<meta>` > 1252. Les pages
  déclarant UTF-8 en en-tête seul (HN, APIs) ne sont plus
  relues en Latin-1 (`â€“` → `–`).

- **Étape 65 — `doc_h` mesurée vs plafond raster**
  (`explorer-core` + `explorer-api`) : `TabInfo` expose
  `doc_h` (hauteur réelle du document posé) et
  `doc_h_clamped` (`> MAX_FRAME_DIM / scale`) — les
  documents plus hauts que 16 384 px ont enfin un canal
  d'observation ; `GET /tabs/{id}` les sérialise.

- **Étape 64 — `navigated` rapporté au commit effectif**
  (`explorer-core` + `explorer-api`) : les drains de nav
  (`drain_script_nav`, `drain_navs_shared`) comptent les
  navigations commises (`is_ok` — bfcache et fragments
  inclus) ; `handle_input` et `POST /api/tabs/{id}/input`
  dégradent `Navigated` en `Edited` quand la nav enfilée
  échoue — plus de « navigated » vers une page jamais
  atteinte (fetch mort → `edited`, URL inchangée).

- **Étape 63 — popup `<select>` : étiquettes en vrai
  texte** (`explorer-core`) : les options figées sont
  shapées via `explorer_layout::font::shape` + les fontes
  de la session (`sans-serif`, ~¾ de ligne, tronquées
  `...` à la largeur) et peintes en `DisplayItem::Text` —
  avant : `Placeholder` gris sans glyphe, le surlignage
  clavier restait invisible. Baseline centrée, encre
  `#202020`, blanc sur ligne sélectionnée.

- **Étape 66 — `SameSite` appliqué à l'envoi**
  (`explorer-net` + `explorer-core` + `explorer-api`) :
  le flag était stocké mais `CookieTransport` renvoyait
  tout cookie de domaine sans contexte — un
  `SameSite=Strict` partait cross-site. `Request` gagne
  `initiator: Option<Initiator>` (`TopLevelNav`,
  `NavFrom(page)`, `Subresource(page)`) ; `CookieJar::
  header_for_ctx` filtre selon le « site pour cookies »
  `(scheme, registrable)` PSL : `Strict` same-site seul,
  `Lax` + navigation top-level sûre (`GET`/`HEAD`),
  `None` libre (`Secure` exigé à la pose, repli `Lax`
  sinon). `nav_begin` pose le défaut `NavFrom(page
  courante)` ; navigations navigateur (API REST,
  `navigate`, reload, back/forward) = `TopLevelNav` ;
  sous-ressources, modules, polices, `fetch`/`XHR` =
  `Subresource` ; formulaires (GET/POST) = `NavFrom`.
  Sans initiateur = permissif (compat fakes/CLI).

- **Étape 58 — `<video>`/`<audio>` : stubs honnêtes +
  affiche (ADR-0023, aucun décodeur)** : `video[poster]`
  suit le canal image existant (fetch borné `max_images`,
  décodage partagé, bitmap remplacé peint dans la
  content-box) ; affiches `data:` via `poster_data_images`.
  Feuille UA complémentaire `MEDIA_UA_CSS` en
  `Origin::UserAgent` depuis `explorer-core` : `video`
  inline-block 300×150 fond noir, `audio` masqué sans
  `controls`, `audio[controls]` barre 300×54 — l'auteur
  l'emporte toujours. Stubs `HTMLMediaElement` dans
  `dillo-js` : `play()`/`setSinkId`/`setMediaKeys`
  rejettent `NotSupportedError` (chemin `.catch()` des
  sites), `canPlayType` répond `""`, props honnêtes
  (`paused`, `duration` NaN, `readyState` 0,
  `networkState` NO_SOURCE si `src`), `TimeRanges` vides,
  écritures IDL acceptées-ignorées, `class_name` par tag —
  aucun faux succès. Les héros vidéo des sites réels ne
  cassent plus leurs scripts ; les fallbacks des pages
  s'activent proprement.

- **R-30 — `fr` vertical grid + résiduels `display:grid`**
  (`explorer-layout` + `explorer-render`) : pistes `fr`
  verticales distribuées contre la hauteur définie du
  conteneur (§12.7) ; `height:%` des items grid résolu
  contre l'aire de cellule — `cb_h` propagé à
  `layout_item` (§6.5) ; position statique des absolus
  inclut la marge collapsée en attente du frère précédent
  (§10.6.4) ; `width` de cellule de table cloue sa
  colonne, reste aux colonnes auto (§17.5.2.2) ; `height`
  définie de table distribuée aux rangs auto (§17.5.3) ;
  peinture des positionnés `z-index` négatif avant les
  blocs en flux (§E.2). Corpus WPT **57/57** (100 %,
  3 ignorés documentés).

- **R-29 — wrap flex horizontal/colonne + `column/row-gap`**
  (`explorer-layout`) : flottant `width:auto` shrink-to-fit
  avec `min|max-width` appliqué à la boîte de contenu avant
  la pose des enfants (§10.3.5/§10.4) ; enfants flottants
  comptés en somme dans le max-content d'un parent
  shrink-to-fit ; `column wrap` complet (découpe en
  colonnes sur hauteur définie, grow/shrink sur `height`,
  marges `auto` et `justify-content` par colonne,
  `align-content` entre colonnes, gaps sur les bons axes,
  `wrap-reverse` miroir cross). Corpus WPT **55/57**
  (96 %, 3 ignorés documentés).

- **ADR-0022 — politique d'égress unique et détection de
  fuites (préparation Tor)** : `EgressConnector` par
  capacité dans `explorer-net` (en mode Tor, aucun dialer
  direct n'existe dans la variante), DNS délégué au nœud
  de sortie, UDP absent plutôt que routé, `no_proxy`
  ignoré en mode Tor, détection à trois niveaux
  (lint/`cargo-deny`, compteur de flux, canari e2e dans
  `explorer-test-support`) — politique figée,
  implémentation différée par déclencheurs ; aucun code
  modifié.

- **Étape 61 — FFI : profil + état d'onglet**
  (`explorer-ffi` + `explorer-core` + `app`) :
  `JsonFileStore` migre de `explorer-daemon` vers
  `explorer_core::profile_store` (partagé par les deux
  hôtes ; les formes filaires `Raw*` restent privées — les
  DTO du cœur gardent zéro derive serde) ;
  `explorer_create` honore enfin `EXPLORER_PROFILE` —
  le mode in-process persistait dans le vide — et
  `explorer_create_with_profile(dir, len)` (ABI additive)
  accepte un chemin explicite (invalide = `NULL` +
  `last_error`, jamais de repli silencieux). Nouveaux
  exports `explorer_tab_title` et `explorer_tab_scroll`
  (`-1` non ambigu) ; côté app les lookups sont gardés par
  `providesSymbol` (dll antérieure = repli URL) et
  `sendInput` `scrolled` rapporte `scroll_y` à parité REST.

- **Étape 62 — CLI du daemon** (`explorer-daemon`) :
  `--port N` / `--profile DIR` (formes `--x v` et
  `--x=v`) honorent désormais leurs surcharges sur
  `EXPLORER_PORT`/`EXPLORER_PROFILE` ; `--help` affiche
  l'usage ; tout argument inconnu ou invalide est refusé
  (exit 2) — fin de l'ignorance silencieuse qui faisait
  écrire les missions terrain dans le profil par défaut
  sans le savoir.

### Fixed

- **Étape 67 — URL post-redirection jamais commise**
  (`explorer-net` + `explorer-core`) : `tab.url` gardait
  l'URL de requête après un 3xx — barre d'adresse fausse,
  fragment du `Location` perdu (`scroll_y: 0`), entrée
  d'historique cléée sur l'URL pré-redirect, sous-
  ressources relatives résolues sur la mauvaise base.
  `resolved_url: Option<Url>` (additif) sur `Response` et
  `StreamedResponse` ; `RedirectTransport` pose la cible
  finale (fragment inclus) ; la navigation commet l'URL
  résolue — la requête d'origine reste celle stockée
  pour le replay historique (POST).

- **Étape 68 — find : surlignage invisible sans
  déplacement** (`explorer-core`) : occurrence déjà dans
  le viewport → `scroll_to_y` court-circuitait repaint +
  `FrameInvalidated` → les `FillRect` de surlignage
  restaient calculés mais jamais rasterisés (0 px sur
  page courte). `scroll_to_y` rapporte désormais le
  déplacement ; `find_scroll` repeint quand la position
  n'a pas bougé — la surbrillance est un changement de
  frame indépendant du scroll.

- **Étape 55e (rouvert) — animations figées sous pompe
  partagée** (`explorer-core`) : `pump_timers_shared`
  rendait `0` dès l'absence de moteur JS détachable —
  une page `@keyframes` purement CSS (`js: None`) ne
  battait jamais côté daemon, et l'UI ne tirant
  `/frame` que sur `FrameInvalidated` (jamais émis),
  `anim.html` restait figé (6 pulls identiques). La
  branche « pas de moteur » appelle désormais
  `advance_animations` sous verrou court — early-out
  ~gratuit quand rien n'est en lecture
  (`shared_pump_advances_scriptless_animations`).

### Changed

- **Étape 55h — apply de resynchro borné hors verrou**
  (`explorer-core`, `explorer-api`, `explorer-daemon`) :
  les synchros DOM rapportées par le miroir JS ne sont plus
  appliquées atomiquement sous `&mut Session` (jusqu'à
  **157 s** de verrou mesuré sur `storm.html`). Nouveau
  `session/apply.rs` : file `pending_applies` coalescée par
  onglet + `PaintPayload` détachable (`take_paint`/
  `put_paint` — l'onglet dégrade naturellement pendant la
  fenêtre : `/frame` 204 lazy-heal, hit-test mort, tick
  d'animation inerte, scroll accepté et repeint au commit).
  Tous les points de synchro (dispatch événement, pompe
  JS, livraisons XHR) enfilent ; l'apply tourne en
  `spawn_blocking` via `drain_applies_shared` (pompe
  partagée, handler `/input`, battement réseau du daemon —
  le verrou n'est tenu qu'au take/commit). Hôtes `&mut`
  (FFI, CLI, tests) : `drain_applies` synchrone aux mêmes
  frontières — sémantique inchangée. Fidélité préservée :
  `freshen_apply` re-sérialise au drain si le miroir a
  remuté depuis l'enfilement (ex. `preventDefault` d'un
  clic checkbox : bascule → synchro enfilée → restauration),
  epoch-guards anti-supplantation, op-lists `<canvas>`
  capturées à l'enfilement et re-résolues sur le DOM
  commis, `sync_floor` 55g armé sur le coût réel de
  l'apply.

- **Étape 60 — éclatement de `session.rs`** (`explorer-core`) :
  le god module (7 307 lignes) devient `session/` — 20
  sous-fichiers par responsabilité (`navigate`, `net`,
  `js_glue`, `input`, `paint`, `frame`, `store`, `anim`,
  `popup`, `forms`, `scripts`, `decode`, `dom_helpers`,
  `paint_helpers`, `types`, `tab`, `nav_types`, `net_items`,
  `anim_items`, `shared`, `tests`). `Session`/`Tab` gardent
  état et champs privés dans `mod.rs` — les sous-modules
  restent descendants (visibilité inchangée) ; items
  internes levés en `pub(crate)`. API publique et
  ré-exports `lib.rs` identiques ; dénombrement des
  items inchangé (237 fonctions, 49 types). Critère
  d'isolation rempli : `session::decode` embarque son
  propre `mod tests` (5 tests — BOM, labels charset,
  morceaux UTF-8/UTF-16 coupés, ordre WHATWG des feuilles)
  sans session ni réseau.

### Added

- **Étape 57bis — find-in-page** (`explorer-core` + `app`,
  ADR-0021) : recherche textuelle livrée sur la surface
  existante — `session/find.rs` parcourt les boîtes `Text`
  du boxtree (insensible à la casse, ordre document) →
  `FindState` sur `Tab`/`PageSnapshot` (survit au bfcache,
  effacé à la navigation), surlignage `FillRect`
  translucide injecté dans la display list (jaune atténué,
  orange pour l'occurrence courante), `find_step` boucle
  ±1, scroll-vers-occurrence au tiers supérieur.
  `POST /api/tabs/{id}/find` `{query, dir}` →
  `{count, index, query}` ; FFI dégradé `FindResult.empty`.
  UI : barre `_FindBar` (Ctrl+F, Entrée = suivante, Échap
  ferme, compteur `i+1/n`, boutons ↑↓, entrée navbar).
  Granularité v1 = boîte de texte entière — le surlignage
  caractère-près reste une demande PIPE réduite
  (métriques de runs, `interface-equipes.md`).

- **R-27 — direction d'axe flex `row-reverse`/`column`**
  (`explorer-layout`) : premier lot des 17 échecs WPT
  résiduels — les 3 `flex-direction-*` passent, cliquet
  `WPT_BASELINE` 41 → **44/58**, zéro régression.
  - `row-reverse` pose les items dans l'ordre logique
    inversé (avant : ordre DOM décalé) ; `lead` miroir
    incluant l'espacement `between` (§8.1).
  - `text-align` mesure la ligne sur les **margin boxes**
    (`Fragment::marg_l/marg_r` — §16.2 : les marges des
    atomes comptent dans la largeur de ligne ; une ligne
    `text-align:right` d'inline-blocks à marges ne déborde
    plus du conteneur).
  - La chaîne de marges basse du dernier fils en flux reste
    dans la hauteur de contenu quand un séparateur existe
    — bordure/padding bas, BFC, hauteur fixe (§8.3.1 ; un
    conteneur `border:1px` englobe le `margin-bottom` de
    son dernier fils au lieu de le laisser s'échapper).
  Régressions : `flex_row_reverse_ordre_et_ancrage`,
  `text_align_mesure_les_margin_boxes`,
  `marge_basse_dernier_fils_reste_dans_conteneur_borde`,
  `marge_basse_dernier_fils_sechappe_sans_separateur`.

- **R-28 — distribution d'espace flex justify/align**
  (`explorer-layout` + `explorer-render`) : les 7 échecs
  WPT `justify-content`-horiz/`align-items` résolus —
  cliquet `WPT_BASELINE` 44 → **53/57** (3 ignorés), zéro
  régression suite.
  - Alias `left`/`right`/`end` du `justify-content`
    (css-align-3 §4.1 — MVP horizontal LTR) et lead
    **négatif** conservé en débordement pour
    `flex-end`/`center`/`right`/`end` (§9.5 ; les `space-*`
    replient en flex-start).
  - Ligne mono-ligne : cross = taille cross interne du
    conteneur à hauteur définie (§9.4) ; `align-items`
    center/flex-end/stretch conformes.
  - `flex-basis`/`width` = content-box : padding et
    bordures comptés dans la taille externe ; la cible
    shrink/grow écrase le `width` déclaré à la pose (§9.7).
  - `vertical-align: top/bottom/middle` des atomes
    inline-block — margin box ancrée aux bords de ligne
    (§10.8) au lieu de l'alignement baseline unique.
  - Marge haute du premier fils en flux ne traverse plus
    un BFC (`adjoining_top`, §8.3.1) ; marges échappées des
    descendants agrégées dans la chaîne du bloc.
  - Positionné `bottom`/`right` : le décalage translate
    tout le sous-arbre (`translate_subtree`) — les enfants
    suivaient la position statique.
  - Flottants CSS 2.1 : règle n°3 (descendre sous le
    flottant adverse si la bande est insuffisante) et
    n°7 (descendre sous le flottant même-côté si le bord du
    bloc conteneur est dépassé) appliquées dans
    `place_float`.
  - `wpt_skip.txt` : `flexbox-justify-content-horiz-004`
    exclu — fixture défectueuse (bug Mozilla 1954193 :
    échec dans tous les navigateurs, rangées `left`/`right`
    encodant une sémantique inexistante, diff normalement
    sous le viewport).
  Régressions : `flex_justify_right_end_alias`,
  `flex_justify_overflow_lead_negatif`,
  `flex_align_hauteur_conteneur`, `flex_basis_content_box`,
  `flex_shrink_cible_appliquee`,
  `inline_block_vertical_align_top`,
  `marge_haute_premier_fils_reste_dans_bfc`.

- **Étape 57 — UI Flutter v2** (`app/`) : thème clair via
  variante `tokens.light.json` (`meta.scheme` pilote
  `ColorScheme`/`ThemeMode` — clair, sombre ou système) ;
  réglages persistés (`AppSettings` → `ui_settings.json`,
  zéro dépendance) : thème, moteur auto/REST/FFI (le dépôt
  se recrée via provider), répertoire de profil exporté vers
  `EXPLORER_PROFILE` avant chargement FFI (`setenv` process,
  effet au prochain démarrage du moteur) ; bibliothèque —
  signets + historique du store moteur consommés
  (`/api/bookmarks`, `/api/history` : étoile de la barre,
  panneaux historique/signets — mode FFI dégradé, l'ABI
  n'exporte pas le store) ; zoom de page **côté UI**
  (ADR-0021 : `setViewport(w/z, h/z, z·dpr)` — reflow réel,
  raster net, entrées ÷`z·dpr`, Ctrl±/0 + menu, borné
  [0,25 ; 4]) ; find-in-page reclassé (étape 57bis — surface
  moteur requise).

- **Étape 55g-suite — resyncs pompe coalescés + cible
  `sync_profile`** (`explorer-core`, terrain mission 13) :
  le gel ~7 min terrain s'explique par des apply **en
  série sous verrou** — une page qui mute en continu
  (`setInterval`/`setTimeout` réarmé, motif courant réel)
  rend le miroir sale à chaque battement et chaque pompe
  payait re-parse + relayout + repaint sous le mutex
  session. Deux livrables NAV :
  (1) **instrumentation de phase** — `tracing::info!` sur
  la cible `sync_profile` à chaque frontière (dispatch,
  run_pending, sérialisation miroir, re-parse, incrémental
  vs reflow global, repaint, commit pompe vs off-lock) :
  `RUST_LOG=sync_profile=info` attribue le prochain gel à
  ses phases exactes ;
  (2) **coalescence `sync_floor`** — après chaque
  `apply_dom_sync` piloté par la pompe, la prochaine
  synchro timer attend `coût_apply.clamp(paint_interval,
  resync_gap_max_ms)` (config, défaut 4 s) : le miroir
  reste sale, le premier apply autorisé absorbe toutes
  les mutations accumulées en une passe — le verrou
  retrouve des fenêtres proportionnelles au coût réel et
  l'API (`/status`, `/frame`, `/input`) redevient
  joignable entre deux peintures. Les synchros
  d'événements restent immédiates (clic = retour visuel
  attendu) ; les pushes d'attributs session→miroir sont
  toujours rejoués (saisie préservée). Régression :
  `pump_resync_coalesced_under_floor`.

- **JS-C28 — surface `<canvas>` 2D** (`dillo-js`,
  `explorer-js`, `explorer-core` — adoption de la couture
  **R-11** côté NAV) : `canvas.getContext('2d')` rend un
  `CanvasRenderingContext2D` qui **enregistre** ses appels —
  `CanvasOpRecord`, miroir sérialisable de `CanvasOp` —
  dans une side-table du DOM miroir (bitmap cumulative,
  spec). Couverture MVP : `fillRect`/`strokeRect`/`clearRect`,
  chemins (`beginPath`/`moveTo`/`lineTo`/`quadraticCurveTo`/
  `bezierCurveTo`/`arc`/`rect`/`closePath`), `fill`/`stroke`/
  `clip`, transforms complètes, `fillStyle`/`strokeStyle`
  (couleurs `#rgb[a]`/`#rrggbb[aa]`/`rgb[a]`/noms),
  `globalAlpha`, `lineWidth`, `save`/`restore` ; `canvas.
  width`/`height` numériques, leur écriture efface le bitmap
  (spec). APIs hors MVP absorbées sans `TypeError`
  (`drawImage`, `fillText`, gradients opaques, `measureText`,
  `getImageData` minimal, `isPointIn*`) ; mode ≠ `2d` →
  `null`. Chaque op marque le `<canvas>` sale → synchro à
  chemin ciblé (`relayout_subtree`), bornée
  `canvas_op_limit` ; contextes semés en racines GC. La
  session traduit `CanvasOpRecord → CanvasOp` et résout les
  chemins miroir en `NodeId` du DOM commis
  (`PaintImages.canvases`, y compris bfcache). Régressions :
  `canvas_context_records_ops`, `canvas_getctx_kinds_and_
  identity`, `canvas_op_marks_dirty_path`,
  `canvas_width_reset_clears_ops` (dillo-js) ;
  `canvas_2d_script_paints_bitmap` (session, pixels).

- **Étape 56b — auto-réparation du `/frame` 204 persistant**
  (`explorer-core`, rouvert terrain mission 13) : un DOM
  commis sans produit peint — fenêtre `nav_apply` →
  `sub_commit`, `unspec` sur relayout impossible, tout
  chemin jetant la frame après commit — produisait un
  `/frame` 204 à vie, hit-test mort, scroll figé.
  `ensure_painted` re-peint paresseusement depuis le DOM
  commis à la première demande (`frame_png`, dispatch
  d'input) — l'état empoisonné devient transitoire ; un
  DOM réellement non-layable sert un 204 honnête.
  Régression : `committed_dom_repaints_on_frame_pull`.

- **Étape 55g-suite — frontière du clic-mutation
  instrumentée** (`explorer-core`, terrain mission 13) :
  `dispatch_path_event` ~18 s debug sous verrou sur ~3 000
  nœuds, quasi entièrement dans `try_sync_incremental` —
  `relayout_subtree` ~14 s (append en tête de flux =
  sous-arbre ≈ document), cascade ~0,3 s, repaint ~3,7 s.
  Le remap des `NodeId` conservés est mémoïsé par parent
  (~3,4 s sortis, 21,7 → 18 s). Le reste est consigné au
  journal inter-équipes (cascade scopée, cache glyphe,
  `relayout_subtree` dominant) ; le test
  `profile_click_sync_heavy` (`#[ignore]`) mesure le chemin
  bout à bout.

- **Étape 53f — `run_pending`/`sync_dom` hors verrou**
  (`explorer-core`, résidu 53d / terrain mission 8) : un
  burst JS (`repros/relayout.html`, ~200 `appendChild`)
  figeait `/status` ~20 s — l'exécution + la sérialisation
  du miroir se faisaient sous `&mut Session`.
  `pump_timers_shared` suit le patron de `navigate_shared` :
  take/commit sous verrou court, moteur **extrait** de
  l'onglet (`js_detached(epoch)`) et travail long dans
  `spawn_blocking` (`JsEngine: Send`). À la réinsertion,
  une epoch supplantée jette le moteur sans sync (jamais de
  miroir caduc) ; les réponses `fetch`/XHR et les pushes
  d'attributs (`value`, `checked`) survenus pendant le run
  sont tamponnés puis livrés/rejoués **avant** la
  re-sérialisation — la saisie utilisateur n'est jamais
  écrasée. Boucle daemon migrée (`pump_timers_shared` +
  `drain_navs_shared` hors verrou). Dégradation assumée et
  documentée : un événement DOM dispatché en plein run
  trouve `js` absent → handler inerte, input traité côté
  hôte — comme une page sans script. Régressions :
  `pump_shared_keeps_lock_free_during_js_burst` (verrou
  acquisissable pendant `run_pending` bloqué),
  `pump_shared_applies_dom_mutations` (rendu identique à la
  pompe synchrone).

- **Étape 55e — `@keyframes` cadencés en session**
  (`explorer-core`, adoption R-10) : les animations CSS ne
  produisaient qu'une frame figée (repro `anim.html` — deux
  frames à 400 ms identiques). `sync_animations` enregistre
  les déclarations `animation*` du style calculé brut par
  identité `(ordinal d'élément, index de la liste)` — une
  déclaration inchangée conserve son `start` (le re-style
  ne redémarre pas), changée elle repart.
  `apply_anim_overrides` intercale la couche keyframes
  **sous** les transitions (`keyframe_style` + interpolation
  partagée R-10, `update_transitions` ne mute plus `styles`
  — le diff reste sur le brut et la surcharge « current
  transition » gagne toujours). Cadence : `advance_animations`
  partagé par `tick_animations` (pull `/frame`) **et**
  `pump_timers_all` (battement hôte ~50 ms — l'animation ne
  reste plus figée entre deux pulls). Repli : une lecture
  terminée sans `fill` produit un dernier reflow pour
  replier le style de base (drapeau `applied`) ; `forwards`
  fige sans reticker. Régressions :
  `keyframes_animation_advances_and_finishes`,
  `keyframes_absent_means_no_animation_ticks`.

- **R-22 — décorations inline par fragment + flux en-ligne
  réel** (`explorer-css`, `explorer-layout`,
  `explorer-render`) : bug terrain lobste.rs — titres en
  « pastilles sombres », interlignes ~180 px.
  (1) **Shrink-to-fit §10.3.9** des atomes `inline-block`
  à `width:auto` (`clamp(dispo, min-content, max-content)`
  via `intrinsic::own_sides`/`min_content_px`/
  `max_content_px`) : `ul.tags` 978 px → ~87 px.
  (2) **Fond/bordure d'inline peints par fragment**
  (`paint_inline_deco`) — l'union des fragments wrappés
  produisait un blob couvrant le texte voisin ;
  approximation `box-decoration-break: clone` documentée.
  (3) **`border-*-color` absent → `currentColor`** (spec)
  au lieu de noir — les pastilles `a.tag` dont les `var()`
  vivent derrière `@import` non résolue retombent sur
  `color`.
  (4) **§9.2.1.1 complet** : `hoist_blocks` éclate un
  inline pur autour d'un bloc en flux niché (récursif,
  `deep` R-24), et un `inline-block`/`inline-flex`…
  atomique anonymise ses enfants comme un conteneur bloc —
  `details.caches > summary` ne compte plus une ligne par
  blanc (byline lobste.rs ~93 px → ~23 px, story ~158 px →
  ~81-89 px).
  (5) **Hors-flux exclus de l'itemisation inline** :
  `absolute`/`fixed` sautés de `size_atomics`/
  `collect_child`/`assign_*` (série `aid` synchrone) mais
  conservés dans l'arbre pour `resolve_absolutes` ; tests
  de positionnement/float lus sur le **style calculé**
  (`LayoutBox::out_of_flow`, kind-aware : le style cloné
  d'un `Text`/`AnonymousBlock` n'est pas une position
  d'élément — corrige le double placement du texte d'un
  absolu qui a cassé puis réparé `flexbox_align-items-
  flexstart`). `ComputedStyle` gagne `PartialEq` (style du
  fragment ≠ style du conteneur = discriminant de
  décoration). Régressions : `tests/repros.rs` (fixture
  lobste.rs figée, 25 stories), `tests/boxtree.rs` +
  `tests/inline.rs` (split bloc-dans-inline, absolu niché,
  atome + bloc), `tests/display.rs` (déco par fragment,
  `currentColor`). Cliquet WPT toujours 40/58. Reste :
  `@import` non résolue (fetch = couture NAVIGATEUR —
  journal `interface-equipes.md`).

- **Étape 55g — resynchro grand document : trois gardes de
  coût** (`explorer-core`) : le gel terrain mission 12
  (mutation JS par clic sur ~3 000 nœuds → gel > 5 min) —
  mesure par phases (`profile_resync_phases`, `#[ignore]`d)
  : `relayout+repaint` **28,6 s → ~17 s** debug.
  (1) **Cull raster** : les items-feuilles entièrement
  hors de la zone raster réelle (post-cap `MAX_FRAME_DIM`,
  jamais sous `PushTransform` dont les bornes brutes ne
  sont pas fiables) sont jetés avant `rasterize` —
  `paint_text` ne re-trace plus chaque glyphe du document
  hors-champ : raster ~18,5 s → ~6,5 s sur le repro.
  (2) **Diff dirty-rects borné** (`DIRTY_DIFF_MAX_ITEMS` =
  2 000 items) : `merge_rects` O(k²) coûtait ~2 s sur
  ~8 500 rects avant de s'effondrer en « pleine frame » —
  on la produit directement au-delà du seuil.
  (3) **PNG paresseux** : `repaint` n'encode plus —
  `frame_png(&mut)` encode+cache à la demande, un burst de
  repaints entre deux lectures n'encode qu'une fois (~4 s
  sortis du chemin de resynchro). `freeze`/`try_bfcache`
  jugent désormais le produit peint sur `frame` (la frame
  raster est l'autorité, le PNG un cache). Restes logués
  pour PIPE dans `docs/interface-equipes.md` : fusion de
  rects linéaire, cache glyphe→outline, layout/cascade
  scopés. Régressions : `cull_keeps_only_in_frame_items`
  + suite session 111/111.

- **R-23 — valeur de contrôle peinte à sa position**
  (`explorer-layout`) : résidu de R-21 — le glyphe tapé
  apparaissait en haut à gauche du document (repro
  `repros/form.html`, mission 7). Le fragment atomique
  recevait son rect final mais son sous-arbre posé en
  (0,0) n'était jamais translaté. `assign_atomic_rects`
  applique désormais `translate_subtree` (delta fragment −
  rect pré-layout) et s'ordonne avant `assign_child_rects`,
  qui saute les atomes. Couvre aussi le texte d'un
  `inline-block` et le repli d'`object`. Régression :
  `controle_input_valeur_posee_sous_le_controle`
  (`tests/display.rs`).

- **R-24 — profondeur d'arbre sans stack overflow**
  (`explorer-common`, `explorer-dom`, `explorer-css`,
  `explorer-layout`, `explorer-render`) : bug terrain
  `repros/host-nest*.html` — ~300 `<div>` imbriqués tuaient
  le worker tokio (`dump` DOM seul survivait : la récursion
  était en aval — boxtree, layout, display list, hit-test,
  cascade). Livré : `explorer_common::deep` =
  `stacker::maybe_grow` (zone rouge 128 Kio, segment tas
  1 Mio — patron JS-C41) appliqué à **chaque arête
  récursive ∝ profondeur DOM** : `collect_text`,
  `compute_subtree`, `match_selector_part`, boxtree,
  `layout_block` (hub unique — couvre flow, flex, grid,
  table, inline, relayout, absolus), `translate_subtree`,
  atomiques inline, `content_px`, tables, `hit_test`,
  `paint_box`, `svg::serialize_into`, `dump`. Plus drop
  **itératif** de `LayoutBox` (le drop récursif par défaut
  aurait débordé à lui seul). Régression :
  `tests/deep.rs` — pipeline complet + hit-test + dump sous
  une pile de 384–512 Kio.

- **R-13 (en cours) — compatibilité Acid2, première
  salve** (`explorer-css`, `explorer-layout`,
  `explorer-render`) : score informatif 23,3 % → 22,0 %
  (la géométrie se rapproche mais le comptage de pixels
  reste sous la cible 60 % — étape non terminée).
  Livrés dans cette salve :
  - `::before`/`::after` — sélecteur (`::` moderne et `:`
    legacy, inconnus rejetés), `StyleMap.pseudos`, boîtes
    générées dans l'ordre spec (before → enfants → after) ;
    `content: ''` produit des boîtes visibles (bordures
    peintes — sourcils du nez Acid2).
  - URLs `data:` (`dataurl.rs`) — décodage pur borné
    (`percent-decode` avant base64 — Acid2 écrit `%2F`),
    `collect_data_images` remplit `PaintImages.replaced`/
    `backgrounds` depuis `<img>`, `<object>` et
    `background-image`.
  - `<object>` repli/remplacé — ressource résolue → feuille
    remplacée intrinsèque (atomique en ligne, série `aid`
    pré-ordre dans `inline.rs`) ; sinon les enfants se
    substituent (cascade imbriquée Acid2).
  - `background` shorthand réinitialise les longhands non
    mentionnées (`background: none` efface la couleur —
    feuille liée `data:text/css` d'Acid2) ;
    `background-position` (mots-clés/%, longueurs) et
    `background-attachment: fixed` (origine viewport)
    propagés jusqu'à `DisplayItem::BackgroundImage`.
  - Tables : rangs/cellules **anonymes** (§17.2.1 — cellules
    nues emballées), tables imbriquées isolées, `height` de
    table vide honoré, hauteur de cellule étirée au rang
    (§17.5.3), marges de table prises en compte.
  - Positionnement : CB absolu = **padding box** de
    l'ancêtre positionné (§10.1), `em` résolus contre la
    fonte propre de chaque boîte, **shrink-to-fit** pour
    `width:auto` absolu (§10.3.7).
  - Marges : collapsing à travers les boîtes vides.
  Régressions : `pseudo_*` (selector/block), `dataurl`,
  `object_resolu_est_remplace_sinon_repli_enfants`,
  `background_position_*`, cellules anonymes.

- **R-13 (en cours) — items flex/grid + corpus WPT
  étendu** (`explorer-css`, `explorer-layout`,
  `explorer-render`) :
  - Conteneurs flex/grid : les enfants directs ne sont
    plus fusionnés en un bloc anonyme unique — chaque
    enfant devient un item (blockifié §9.7), seules les
    séries de texte nu se groupent.
  - Boîtes en-ligne posées comme conteneurs
    (`inline-block`, item flex/grid, repli `<object>`) :
    leur contenu coule désormais via `layout_inline` — le
    texte d'un `inline-block` item flex recevait un rect
    0×0 et n'était jamais peint.
  - Marges des items flex : `margin-top/bottom` sur l'axe
    principal en `column`, marges cross comptées dans la
    taille de ligne (§9.4) et appliquées à la pose
    (`stretch` remplit la margin box).
  - Shorthands cascade : `flex-flow` →
    `flex-direction`/`flex-wrap`, `flex` →
    grow/shrink/basis, `gap` → `row-gap`/`column-gap`.
  - Corpus WPT : +19 reftests (flexbox + grid), harnais
    `.xhtml` normalisé (`<t/>` → `<t></t>` — servi en XML
    par WPT, le parsing HTML ignorerait `/>` hors vides) ;
    cliquet `WPT_BASELINE` 38 → 40/58.
  Reste connu : `column wrap` multi-colonnes, marges
  `auto` verticales en column, écarts fins justify/align.

- **R-25/R-26 — clôture Acid2** (`explorer-render`) :
  `R-25` vérifié résolu par la salve 3 de R-13 — l'objet
  remplacé **inline** peint sa ressource (≈240 px verts
  mesurés dans la zone yeux au harnais Acid2, après
  `content_rect` + ordre Appendice E). `R-26` : le compteur
  de diff `wpt_reftests` ne renvoie plus `usize::MAX`
  (`18446744073709551615 px durs`) quand les dimensions
  divergent — la zone hors recouvrement compte « dure » en
  entier, chiffres réels pour le triage (ex. 59 156 px).

- **R-13 — compatibilité Acid2 ≥ 60 % atteinte (86,1 %)**
  (`explorer-css`, `explorer-layout`, `explorer-render`) :
  score informatif 22 % → 86,1 %, étape terminée.
  Livrés dans cette salve :
  - Layout : collapse des marges par chaîne complète
    (§8.3.1 — `MarginRun` = max positif + min négatif à
    travers les éléments transparents, plus de pairwise),
    flottants posés au bord de la **margin box** au
    flow-position (le `margin-top:-2em` du nez remonte),
    `top`/`bottom` absolus ancrant la margin box,
    shrink-to-fit absolu incluant padding/bordures,
    colonne grid `auto` plafonnée au free space (§12.6),
    marges d'items grid appliquées en margin box.
  - Rendu : ordre de peinture Appendice E (blocs en flux →
    flottants → inlines reportés au contexte de pilement ;
    positionnés `z-index` triés) — les yeux Acid2 se
    superposent dans le bon ordre ; jointures de bordure
    diagonales (trapèzes — losange du nez) ; largeurs de
    bordure `em` résolues à la font-size de la boîte ;
    images remplacées peintes en content box.
  - CSS : `\c` en tête de token = ident (`\}` ne ferme plus
    un bloc — `background:yellow` Acid2 survit) ; `!` non
    suivi d'`important` invalide la déclaration
    (`! error`) ; `background: red pink` droppé (une valeur
    par composante) ; validation par propriété
    `value_is_valid_for` (`width: 200` sans unité droppé ;
    `var(`/`calc(`/`attr(` et mots-clés CSS-wide exemptés) ;
    hash `#000` re-sérialisé sans échappement de digit ;
    identifiants échappés préservés dans les préludes
    re-parsés (`[class=second\ two]` → `float:right`
    appliqué à l'adresse jaune).
  - Corpus WPT : cliquet `WPT_BASELINE` 40 → **41/58**
    (`floats-placement-005`, `grid-margins-no-collapse-001`,
    `grid-layout-basic` réparés).
  Régressions : `ordre_appendice_e_bloc_flottant_inline`,
  `longueur_sans_unite_dropee`,
  `background_invalide_drope_pas_remplace`,
  `echappement_dans_nom_de_propriete`,
  `bang_non_important_invalide_la_declaration`,
  `accolade_echappee_ne_ferme_pas_le_bloc`,
  `backslash_echappement_en_tete_de_token`.
  Reste connu : hairline/`p.bad` fixed hors du visage
  rasterisé (ancrage viewport, pas de scroll — par
  conception du harnais), anneaux des yeux et rangées
  parser/`ul` à l'affûtage pixel près.

- **Étape 56f — images `data:` peintes en session**
  (`explorer-core`) : le défaut terrain mission 12
  (`img.html`, `b1`-`b4` — `<img src=data:>`,
  `<object data=data:>` et `background-image: url(data:)`
  peignaient 0 pixel). `collect_data_images` (R-13) est
  désormais appelé dans `relayout_dom` après la cascade —
  `replaced`/`backgrounds` décodés inline (zéro réseau) et
  fusionnés aux corps fetchés, injectés **avant**
  `image_sizes` pour servir le sizing intrinsèque. Le
  planificateur de sous-ressources n'émet plus de job fetch
  pour les URL non-http(s) — un `data:` ne partait de toute
  façon nulle part (échec inutile). Débloque aussi les yeux
  Acid2 (avec R-25 côté PIPE). Régression :
  `data_url_images_paint_inline` — `<img>` et
  `background-image` `data:` == bloc 2×2 rouge de
  référence, 3 requêtes document, 0 sous-ressource.

- **Étape 56d — `#fragment` : navigation + scroll vers la
  cible** (`explorer-core`) : le défaut terrain mission 11
  (`anchor.html`, footnotes Wikipedia — clic `href="#id"` :
  `navigated` + entrée d'historique émis mais `scroll_y`
  restait 0). `try_fragment_nav` traite le cas
  **même-document** (GET, URL hors fragment identique à la
  page commise) **sans re-fetch** : push historique,
  `Navigated` émis, scroll vers la cible — `id` d'abord,
  `<a name>` ensuite (ordre navigateur), `#` nu → haut de
  page. Position = haut de la première boîte posée portant
  le nœud (`box_y_of_node`, `min` itératif R-24), bornée
  `[0, doc_h − viewport]` via `scroll_to_y`. Les fragments
  **cross-document** (`url#frag` autre page) sont appliqués
  post-commit dans `sub_commit`. Bonus : `back`/`forward`
  vers une entrée historiée avec fragment rescrolle
  naturellement (la requête porte le fragment). Cible
  introuvable/non rendue → no-op, jamais d'erreur.
  Régression : `fragment_nav_scrolls_without_refetch`.

- **Étape 56b — `/frame` 204 persistant résolu : snapshots
  sans produit peint** (`explorer-core`) : le défaut terrain
  mission 11 (première nav `World_Wide_Web` — `navigated` +
  `frame_invalidated` émis mais `/frame` 204, hit-test mort,
  scroll bloqué à 0, seul un `back` ramenant une frame).
  Cause : `Tab::freeze` drainait l'état vivant **pendant la
  fenêtre `nav_apply` → `sub_commit`** — un `try_bfcache`
  concurrent (verrou libre pendant le fetch des
  sous-ressources) capturait un snapshot avec DOM commis
  mais `tree`/`frame`/`png` encore absents ; sa
  restauration servait alors un cadre vide à vie —
  `restore` ne repeint jamais par conception. Deux
  garde-fous : `freeze` refuse une page sans produit peint
  (`dom` **et** `png` exigés — la position sera re-fletchée
  plus tard plutôt qu'empoisonnée) ; `try_bfcache` repeint
  depuis le DOM restauré quand le snapshot ramené n'a pas
  de produit (répare aussi les éventuels snapshots
  empoisonnés déjà présents). Régression :
  `paintless_snapshot_repairs_on_restore`.

- **Étape 56c — état hybride résolu : peinture spéculative
  révertée + `navigate` forward = restauration bfcache**
  (`explorer-core`) : deux mécanismes derrière le repro
  terrain mission 11 (A→B→back→`navigate B` — l'onglet
  affichait B, hit-test et `/status` restaient sur A).
  (1) `Tab::spec_paint` marque toute frame/arbre issus
  d'un document **en vol** (peinture progressive 53d) ;
  `unspec` repeint depuis le DOM commis — ou jette
  arbre/liste/frame/png si rien n'est commis — aux points
  de gel bfcache, d'abandon (epoch supplantée), d'échec
  et de commit, et `repaint` ré-arme le drapeau après
  chaque peinture non spéculative. Le snapshot gelé ne
  peut plus embarquer une frame qui ne correspond pas à
  son DOM. (2) `try_forward_nav` réécrit un `navigate`
  GET sans corps vers l'URL **exacte** de l'entrée
  forward en `try_bfcache` — commit complet de l'entrée
  (snapshot restauré, `hist_idx` avancé, événements
  `Navigated`/`TitleChanged` émis) au lieu d'un fetch
  dont la peinture en vol recréerait l'hybride ; POST ou
  URL différente naviguent normalement. Correctif
  robustesse au passage : la cadence de peinture
  progressive ne se déclenche plus sur un morceau sans
  texte décodé (fenêtre de sniffing 1 024 octets du
  `StreamDecoder` — peindre un DOM vide était un no-op
  `EmptyDocument` par chunk). Régressions :
  `superseded_nav_reverts_speculative_paint`,
  `navigate_to_forward_url_restores_bfcache_entry`.

- **Étape 55f — `relayout_subtree` adopté en session**
  (`explorer-core`, `explorer-js`, `dillo-js` — adoption
  R-12) : le miroir suit le **sous-arbre sale**
  (`Dom::dirty_node`, escalade à l'ancêtre commun, cible
  rapportée en chemin `(tag, index)` via `take_dirty_path` —
  survit au re-parse contrairement aux `NodeId` d'arène) ;
  `JsEngine::sync_dom_path` la transporte (contrat interne
  NAV étendu, `DomSyncPayload`/`DomPath` aliases).
  `apply_dom_sync` tente `try_sync_incremental` : remap des
  `NodeId` des boîtes conservées par chemin d'élément
  **avant** `relayout_subtree` (la sérialisation n'est pas
  structure-identique — les indices d'arène ne coincident
  pas entre parses), puis relayout local. Repli
  conservateur sur `NeedsFullLayout` (flottants, inline,
  positionnés), cible sans boîte ou id irrésolvable —
  l'arbre mixte est toujours jeté et reconstruit, jamais
  servi. Dette corrigée au passage : un `dirty_node`
  résiduel `#document` à l'installation empoisonnait
  l'ancêtre commun (chemin vide à vie) ; une mutation sur
  nœud détaché ne fixe plus la cible (couverte par la marque
  du parent à l'attachement). Compteur diagnostic
  `incremental_syncs`. Régressions :
  `dom_sync_uses_incremental_relayout`,
  `dom_sync_falls_back_when_floats_present`,
  `dirty_path_tracks_mutated_subtree`.

- **Étape 55d — `FrameInvalidated` rapporte le delta, pas
  l'historique** (`explorer-core`) : `Tab::dirty` garde
  l'accumulation complète pour le pull FFI (`take_dirty`,
  inchangé) et un watermark `emitted` borne ce que chaque
  émission rapporte — le client SSE reçoit les régions
  changées depuis l'événement précédent au lieu de
  l'historique à vie (qui collapsait en « plein cadre »
  au-delà de 32 repaints — `relayout.html` terrain
  mission 8). Watermark, pas drain : les deux
  consommateurs sont indépendants (le handle FFI garde un
  receveur inerte du bus — drainer l'affamerait). Gelé
  dans le bfcache. Limite connue documentée : après un
  `scroll` les régions couvrent le document entier — le
  défilement est cuit dans le raster, un pull partiel de
  scroll n'a pas de sens. Régression :
  `frame_invalidated_reports_delta_not_history`.

- **Étape 55b — `<svg>` inline câblé en session**
  (`explorer-core`, adoption R-9) : `Tab::images` devient la
  table `PaintImages` complète — `svg::svg_sizes` fournit
  les tailles intrinsèques (`width`/`height`/`viewBox`,
  défaut remplacé 300×150) à `build_box_tree_sized`, et
  `svg::collect_svg` rasterise les racines `<svg>` du DOM
  vers `PaintImages.svgs` une fois par reflow (borné
  `image_max_px`, sources >4 Mio ignorées — boîte
  remplacée vide en dégradé). `repaint` consomme
  `build_display_list_full`. Le bfcache gèle la table
  entière — retour arrière sans re-raster. Régression :
  `inline_svg_is_rasterized_into_frame` (pixels rouges
  mesurés dans la frame).

- **Étape 55c — Miroir→DOM : la source `<script>` ne fuit
  plus en texte peint** (`dillo-js`) : la recherche du
  `</tag` fermant un élément rawtext passait par
  `rest.to_lowercase().find()` — index en **octets** d'une
  chaîne dont la casse change la longueur de certaines
  runes (`ﬀ`→`ff`, `İ`→`i̇`…) ; la coupe désalignée ratait
  le `</script>` et le reste du document était parsé comme
  texte **hors** de l'élément (JSON clientpref/RLSTATE
  peints sur wikipedia — terrain missions 6/7). Le scan est
  désormais insensible à la casse directement dans le flux
  de caractères. Fidélité round-trip en bonus : les
  enfants texte de `script`/`style` sont sérialisés
  **verbatim** (l'échappement doublait à chaque
  resynchronisation ; repli échappé si la source contient
  son propre `</tag`), et `title`/`textarea` décodent leurs
  entités à la parse (RCDATA, comme le tokenizer réel).
  Régressions : `rawtext_close_survives_length_shifting_
  runes`, `rawtext_serialization_is_verbatim`.

- **Étape 56 — Profil persisté complet + bfcache**
  (`explorer-core` + `explorer-net` + `explorer-daemon`) :
  `cookies.json` rejoué au démarrage via
  `ProfileStore::load_cookies`/`save_cookies` (cookies de
  session exclus — sémantique navigateur ; `CookieJar::
  export`/`insert_stored` net) ; cache HTTP **disque borné**
  `cache/<fnv64>.json|.bin` implémentant le nouveau port
  `CacheStore` de `CachingTransport` (mémoire ou disque
  interchangeables, éviction FIFO `max_total_bytes`) ;
  **bfcache minimal** : la page quittée est gelée
  (`PageSnapshot` : DOM, arbre posé, frame, scroll, moteur
  JS figé) à sa position d'historique — `go_back`/
  `go_forward` restaurent sans réseau, borné
  `SessionConfig::bfcache_pages` (FIFO, défaut 4) ;
  **sémantique POST** : l'historique stocke la requête
  complète — un POST historié est rejoué tel quel au
  déplacement et au `reload` (limite « re-GET » levée).
  Régressions : `bfcache_restores_back_forward_without_
  refetch`, `bfcache_evicts_oldest_page`, `cookies_persist_
  across_restart`, `post_history_entry_replays_request`,
  `store_cache` (5 tests daemon).

- **Étape 55 — Dirty-rects transportés** (`explorer-core` +
  API/FFI) : `repaint` diffe la nouvelle display list contre
  la précédente (`DisplayList::changed_rects`, R-7) et
  accumule les régions invalides ; `CoreEvent::
  FrameInvalidated` porte désormais `rects` (px physiques —
  le `scale` est appliqué côté core ; vide = repeindre tout,
  conservateur). Pulls partiels : REST `GET
  /api/tabs/{id}/frame?x=&y=&w=&h=` (PNG de la région),
  `Session::frame_region`/`frame_png_region`/`take_dirty`,
  FFI `explorer_frame_copy_region` + `explorer_dirty_rects`
  (quadruplets `f32`, drain). Accumulation plafonnée à
  32 régions puis effondrement « pleine frame » — un client
  qui ne draine jamais retombe sur le comportement antérieur.
  Arbitrage `ExternalTexture` GPU reporté : à mesurer sur
  l'app avec de vraies frames avant de réviser ADR-0004/0018.

- **R-12 — Layout incrémental** (`explorer-layout`, tiers
  `DirtyLayout` d'ADR-0012) : `relayout_subtree` reboîte +
  re-pose un sous-arbre sale en place (`LayoutBox.slot`
  mémorise le contexte de pose), propagation de hauteur aux
  ancêtres ; repli `NeedsFullLayout` conservateur (flottants,
  chemin absolu/fixe, flux inline). Bench `relayout` : ~33 ms
  reflow → ~1.9 ms relayout feuille (page 200 blocs, ~17×).

- **R-11 — Raster `<canvas>` 2D** (`explorer-render` +
  `explorer-layout`) : `CanvasOp`/`CanvasTable` (couture
  NAV↔PIPELINE — `dillo-js` JS-C28 enregistrera les appels
  `getContext("2d")`), playback `raster_ops` vers tiny-skia
  (rects, chemins avec arc approximé en cubiques, fill/stroke/
  clip, transforms, `globalAlpha`, `lineWidth`, save/restore),
  `PaintImages.canvases` → `DisplayItem::Image` ; `<canvas>`
  remplacé intrinsèque `width`/`height` (défaut 300×150),
  UA `inline-block`.

- **R-21 — Valeur des contrôles peinte** (`explorer-layout`) :
  le `Text` synthétique d'`<input>`/`<select>` est enveloppé
  dans un `AnonymousBlock` — avant, enfant inline nu d'une
  boîte atomique, il n'entrait jamais dans `lines` (le texte
  tapé restait invisible). `DisplayItem::Text` émis.

- **R-20 — Jamais-rendus robustes** (`explorer-layout`) :
  `script|style|template|head|meta|link|base|title` ne
  produisent aucune boîte même sans style calculé —
  injection DOM post-cascade couverte (bug wikipedia :
  JSON de `<script>` inline peint).

- **R-19 — Hit-test tolérant aux enfants échappés**
  (`explorer-layout`, module `hit`) : `explorer_layout::
  hit_test` remonte le hit-test dans le domaine — élagage
  sur `paint_bounds` (union des rects du sous-arbre) au lieu
  du border-box : un enfant peint hors du parent
  (`position:absolute`, marges négatives, overflow visible)
  reste cliquable — bug terrain lite.ddg (formulaire posé
  hors de `<center>`). Même contrat « plus imbriqué contenant
  gagne » ; `explorer-core::session::hit_test` délègue
  (carve-out consommateur, journalé).

- **R-10 — `@keyframes`** (`explorer-css`, module
  `animation`) : collecte `StyleEngine.keyframes` (offsets
  `from`/`to`/`N%` éclatés et triés, fusion même-offset),
  `animation_decls` (shorthand `animation:` + longhands par
  index — duration, delay, timing, `infinite`, direction,
  fill-mode), `AnimationDecl::phase(elapsed)` → progression
  [0,1] avec délai/itérations/alternate/fill, et
  `keyframe_style` — interpolation **via le moteur de
  transitions** (étape 45), frames 0 %/100 % implicites prises
  sur le style calculé. Cadenceur inchangé côté session.

- **R-9 — `<svg>` inline** (`explorer-render` +
  `explorer-layout` + `explorer-css`) : le sous-arbre DOM
  étranger est re-sérialisé en source SVG (échappement XML,
  `xmlns` injecté, borne `SVG_MAX_SOURCE`) puis rasterisé par
  `usvg`+`resvg` — même chemin que `<img src=*.svg>`. Nouveau
  module `svg` : `collect_svg` (`NodeId` → bitmap, `max_px`)
  et `svg_sizes` (intrinsèque attrs/`viewBox`/300×150) ;
  `PaintImages.svgs` (additif). Le `<svg>` devient boîte
  remplacée atomique (sous-arbre jamais boxifié HTML) via
  `build_box_tree_sized` ; UA passe `svg` en `inline-block`.
  `foreignObject` dégradé vide (refus usvg documenté).

- **R-7 — Dirty-rects produits** (`explorer-render`) :
  `DisplayList::changed_rects(prev)` rapporte les régions
  modifiées entre deux frames — bounds résolus sous la pile
  transform/clip (ordre `pre_concat` du raster), zones
  ancienne+nouvelle à chaque item différent ou déplacé,
  fusion par intersection. `[]` = frame propre. API
  additive ; débloque le transport FFI dirty-rects (NAV-55).

- **R-6 — `position: sticky` tous côtés** (`explorer-render`)
  : `sticky_offset` généralise les quatre seuils — `bottom`
  épingle au bas du viewport, `right` au bord droit,
  sur-contrainte spec (`top` gagne `bottom`, `left` gagne
  `right` en LTR), borne = border box du parent sur les deux
  axes. Nouvelle API additive `build_display_list_scrolled`
  (scroll `(x, y)`) — les builders existants délèguent à
  `x = 0`, signature inchangée. Les conteneurs `overflow`
  internes ne scrollent pas encore (référence = viewport
  racine, documenté).

- **R-5 — Grid MVP+** (`explorer-layout`) :
  `grid-template-areas` + `grid-area` (nom → bounding box,
  ou forme numérique `r/c/r2/c2`), lignes nommées
  `[start] 100px [mid]` résolues dans `grid-column`/`grid-row`,
  `grid-auto-flow: column` (colonnes implicites `auto`)
  et `dense` (backfill depuis l'origine), `minmax(min, max)`
  à deux bornes — `fr` borné par le min à la distribution.
  Écarts documentés : `fr` vertical ≈ `auto`, `auto-fill`/
  `auto-fit`, indices négatifs, spans dans la mesure auto.

- **R-4 — `flex-wrap` multi-lignes** (`explorer-layout`) :
  découpe des items en lignes par taille hypothétique
  (`flex-wrap: wrap`/`wrap-reverse`), facteurs flex,
  marges `auto` et `justify-content` résolus **par ligne**
  (spec §9.5-9.7), `row-gap` inter-lignes, `align-content`
  complet (7 valeurs ; `stretch` répartit le libre sur les
  crosses de ligne) quand la hauteur du conteneur est
  définie, miroir cross pour `wrap-reverse`. Les items
  `order`és franchissent les lignes dans l'ordre trié.
  Hors scope documenté : `wrap` en `column`.

- **R-3 — Tailles intrinsèques réelles** (`explorer-layout`,
  nouveau module `intrinsic`) : `min-content`/`max-content`/
  `fit-content` sont désormais **mesurés sur le contenu
  réel** — texte pesé par la fonte injectée (mot le plus
  long vs ligne insécable), sous-arbres récursés, lignes de
  table sommées — et résolus sur `width`, `min|max-width`
  et `flex-basis`. Les bases flex `auto` et les colonnes
  grid `auto` consomment le max-content réel (fin de
  l'approximation « texte direct »). `margin: auto` arrive
  sur les items flex : axe principal = part égale du libre
  **avant** `justify-content` (spec §8.1), cross = reliquat
  (centrage `margin: auto` vertical/horizontal en column).
  `align-items: stretch` étire le rect cross quand la
  taille cross est `auto` (approximation documentée : le
  contenu interne n'est pas re-flouté). R-8 marquée livrée
  par R-15 (foreign-content = sous-chantier 1 de
  html5lib 1592/1592).

- **R-18 — `color` des éléments inline jusqu'aux runs**
  (`explorer-layout`/`explorer-render`, repro terrain
  `color.html`) : `Fragment` porte désormais le
  `ComputedStyle` de son item (l'élément inline le plus
  proche du texte, héritage fait) — le raster peint encre,
  fonte et taille **par fragment** au lieu de la boîte des
  lignes. `span{color}`/`a[href]` colorent enfin leurs
  glyphes (symptôme « liens non stylés » résolu).

- **R-17 — `font-size` : valeur calculée absolue**
  (`explorer-css`, repro terrain info.cern.ch/wikipedia).
  La cascade stockait la déclaration brute (« 2em ») et
  l'héritage la propageait — les styles clonés des boîtes
  anonymes/texte re-multipliaient le ratio contre le
  font-size parent déjà résolu (`2em` → 128px au lieu de
  32, facteur em³). `font-size` est désormais normalisé en
  px absolus à la compute (spec : héritage de la valeur
  calculée) : `em`/`ex`/`ch`/`%`/`larger`/`smaller` contre
  le parent, `rem` et mots-clés absolus contre le nouveau
  `StyleEnv::font_size_base` (défaut 16). `calc()` reste
  brut (dégradation documentée). `2em` ≡ `32px` vérifié.

- **Étape 54 — Formulaires v2 côté produit** (`explorer-core`,
  `explorer-js`, `dillo-js`, ADR-0014 révisée) : le cycle
  `<select>` MVP devient un **popup natif peint moteur** —
  overlay chrome en fin de display list (`FillRect`/`Border`/
  lignes `Placeholder`), options figées à l'ouverture,
  `<select>` retenu par chemin `(tag,index)` (survit aux
  re-parses de synchro), `Arrow↑/↓` surlignent, `Enter`
  choisit, `Escape` referme, clic hors popup = fermeture
  consommée, `input`+`change` dispatchés au choix et miroir
  `selected` synchronisé. **Validation de contraintes HTML**
  avant tout submit (`required`, `minlength`, `maxlength`,
  `pattern` en correspondance entière, `min`/`max`/`step`
  numériques, contrôles « barred » exclus) : `invalid`
  dispatché sur chaque fautif, soumission bloquée,
  `novalidate`/`formnovalidate` honorés, le bouton par défaut
  devient `submitter` de la soumission implicite `Enter`.
  API JS complète côté miroir (`validity`,
  `validationMessage`, `willValidate`, `checkValidity`,
  `reportValidity`, `setCustomValidity` — canal miroir→DOM
  par attribut `data-explorer-cv`) et **`form.submit()` /
  `requestSubmit()`** qui émettent `NavRequest::FormSubmit`
  (champs collectés miroir) drainé vers `navigate_request` —
  `submit()` saute l'événement comme l'exige la spec,
  `requestSubmit()` valide puis tire `submit`.
  **`enctype=multipart/form-data`** en `POST` (RFC 7578,
  frontière pseudo-unique horodatée) et `input[type=file]`
  successful (fichier vide `filename=` — pas de picker).
  Déviations documentées : `:valid`/`:invalid` CSS consigné
  en `interface-equipes.md` (PIPE), `invalid` propage
  (spec : non-bullant), `reportValidity` = `checkValidity`
  (pas de bulle UI), types `date`/`email`/`url` sans
  sémantique dédiée. Tests : 16 session (popup ×4,
  blocages ×6, Enter, novalidate, customValidity,
  multipart, submits JS ×2, helper) + 8 `dillo-js`
  (drapeaux, check/report, custom, `invalid`, radios,
  hooks submit/requestSubmit).

- **Étape 52b — défauts event-loop vus en vrai** (`dillo-js`,
  `explorer-core`, QA daemon `cfa0514`) : `document.title=`
  n'était qu'un setter muet (`Ok(false)`) — implémenté (remplace
  le `<title>` existant, sinon en crée un sous `<head>`) ;
  `emit_navigated` annonce le titre **vivant** de l'onglet au
  lieu du titre extrait au parse — les scripts qui le mutent
  ne se font plus ré-émettre par l'ancienne valeur.
  `TitleChanged` et `frame_invalidated` suivent désormais un
  `setTimeout` différé sans aucun input (le silence QA venait
  de la racine `FrameTooBig`, levée plus haut). Reste : le
  texte d'un `<input>` n'a jamais peint (bug PIPE
  `explorer-layout` consigné `interface-equipes.md` — enfant
  `Text` jamais mis en `lines`). Tests : timer différé +
  titres/frames émis, frappe `input` (repro en attente PIPE).

- **JS-C41 résidu — destruction itérative de l'AST**
  (`dillo-js`, terrain `gh/landing-pages-*.js`) : les chaînes
  d'opérandes `1+1+…`/`a&&a&&…` se parsent itérativement et
  produisent une AST à 15 K+ maillons que la borne `nest` ne
  voit jamais — leur destruction récursive débordait la pile
  via `module_specifiers` (parse + drop, aucun `eval`). Le
  `Drop` d'`Expr` vide désormais les enfants `Box`/`Vec` dans
  une file explicite — plus de descente récursive, plus de
  cas non couvert. `eval` reste borné par `expr_depth`
  (erreur typée).

- **Documents géants : `FrameTooBig` devient une frame
  plafonnée** (`explorer-core`, décision roadmap NAV —
  terrain github/MDN/stallman) : `Session::repaint` tronque
  `DisplayList.height` à `16 384/scale` px avant `rasterize`
  au lieu de propager l'erreur — le PNG couvre le sommet du
  document et `paint_progress` émet `frame_invalidated`
  pendant le téléchargement même sur pages > cap (le bug de
  silence QA est levé). Limite assumée : le bas du document
  au-delà du cap n'est pas rasterisable — besoin PIPE
  consigné en `interface-equipes.md` (fenêtre de raster).
  `FrameTooBig` reste l'erreur des cas vraiment hors bornes
  (largeur). Smoke 10 sites réels : **10/10**.

- **R-16 — Tables réelles et HTML legacy** (`explorer-css`/
  `explorer-layout`, repro terrain news.ycombinator.com) :
  `<center>` et les balises bloc historiques (`dir`, `menu`,
  `hgroup`, `search`, `listing`, `xmp`, `plaintext`,
  `details`, `summary`, `dialog`, `marquee`, `frameset`)
  rejoignent la feuille UA ; `[hidden]`, `frame`, `details`
  fermé et `dialog` fermé sont masqués ; `noscript` reste
  visible (scripting off). `text-align: -webkit-center`
  centre aussi les descendants **bloc** (lignes inline =
  `center`, boîtes filles étroites translatées au centre —
  comportement navigateur historique dont dépendent les
  tables à `width` en %). Hints présentationnels étendus :
  `bgcolor`→`background-color`, `align`→`text-align`,
  `valign`→`vertical-align`, `cellspacing`→`border-spacing`
  (défaut 2px en hint), `cellpadding`→`padding` des cellules
  via la table ancêtre, `border`→filets `outset`/`inset`,
  `body[text]`→`color`. Mesuré : table HN `width=85%`
  centrée à x=60 sur viewport 800.

- **R-15 — Tree-builder : 100 % du corpus tree-construction**
  (`explorer-html`/`explorer-dom`, html5lib-tests
  **1592/1592** exécutables, 200 fragment/script-on hors
  périmètre — cliquet `BASELINE_PASSED` 953→1592). Tous les
  clusters du triage R-14 fermés :
  - **Contenu étranger** : namespaces `Svg`/`MathMl` sur les
    éléments du DOM, tables d'ajustement SVG (casse tags/attrs,
    `definitionURL`, `xlink:*`/`xml:*`/`xmlns`), points
    d'intégration MathML/HTML, breakout, CDATA (`]]]>`),
    end tags étrangers à la marche spec (HTML → retraitement,
    sinon pop jusqu'au match), scoping namespace-aware
    (élément cherché HTML, bornes étrangères par nom local).
  - **`<template>`** : fragment `content`, pile de modes
    template, redirection d'insertion, foster-parenting vers
    le template le plus récent, EOF avec template ouvert.
  - **Adoption agency réelle** (§13.2.6.4.7) : boucle
    externe ×8, boucle interne avec compteur et retrait AFE
    >3, bookmark, clones — remplace la version « lite ».
    `nobr` spécial (AAA implicite + reconstruction), règle
    « 3 éléments » comparant tag **et** attributs.
  - **Tokenizer** : états script escaped/double-escaped,
    `--!>`, `<!` à EOF → commentaire vide, `\0` en étranger
    → U+FFFD sans toucher `frameset_ok`, casse préservée
    dans les faux end tags raw-text, `&#xD;` → `\r` littéral
    (normalisation `\r\n` déplacée du DOM au prétraitement
    d'entrée — spec), `MAX_NAME` 64 Kio (attributs longs),
    **table d'entités officielle complète** (2125 entrées,
    106 legacy — `entities.json` upstream).
  - **Modes résiduels** : in-select + in-select-in-table,
    frameset/after-frameset/after-after-body (ws en in-body,
    reste ignoré sauf body présent), `InHeadNoscript`,
    `search`/`keygen`/`</button>`/`</h1>-</h6>` premier
    heading rencontré, `hr`/`input`/`param`/`source`/`track`,
    `select` **borne de scope** (WHATWG PR 10557 — `</font>`
    sous select ignoré, `</i>` intérieur → AAA), select
    permissif (éléments étrangers, formatage AFE dans
    option), **miroir `selectedcontent`** (clone profond des
    enfants de l'option sélectionnée à la fermeture).
  - **DOM** (`explorer-dom`) : `Namespace` sur `ElementData`,
    `set_text` tolère `Text|Comment`, `create_text` stocke
    verbatim ; dump html5lib namespacé (`<svg svg>`, attrs
    `xml lang`, PI `?target:data`, `template>content`).
- **JS-C41 — Récursion bornée, stack overflow site réel**
  (`dillo-js`, bug terrain, repro : github.com — violation
  « jamais crash ») : le parseur et l'évaluateur ne peuvent
  plus épuiser la pile native. Deux mécanismes combinés :
  `stacker::maybe_grow` (nouvelle dépendance, pur Rust `psm`)
  alloue un segment de pile de 2 Mio sur tas quand il reste
  moins de 256 Kio de pile native, aux points de récursion de
  `Parser` (`stmt`, `expr`, `assign`, `binary`, `unary`,
  `new_callee`, `pattern`, `class_lit`), du `Lexer`
  (templates `` `${…}` `` ré-entrants, nouveau compteur
  `tpl_nest` propagé à travers `lex_in`) et de l'`Engine`
  (`eval_expr`, corps `Fun::Script`, `exec_stmt`) ; les
  plafonds logiques deviennent configurables —
  `Engine::set_call_depth_limit` / `set_nest_limit`
  (défauts 256/512, l'adaptateur `explorer-js` monte à
  1024 sur son thread moteur à pile de 32 Mio). Trous
  fermés : `x=>x=>`, `a?b:c?d`, `**`, `if(`, `new new`,
  `[[[[`, `((((`, `${`${`, `class extends class`,
  `await await`. Tests : 4 régressions (`c41_*`) — 9 cas
  hostiles ×50 000 → erreur typée, récursion directe et
  croisée → `RangeError`, imbrication légitime préservée.
- **Étape 53c — Tolérance TLS `close_notify` absent**
  (`explorer-net`, bug terrain, repro : craigslist.org —
  `TLS unexpected-eof`) : `TolerantEof` wrappe le `TlsStream`
  — un `UnexpectedEof` en lecture devient une fin de flot
  propre, le cadrage HTTP tranche ensuite : `Content-Length`
  ou chunk incomplet → `TruncatedBody` conservé ; corps
  délimité par la coupure → complet. Les autres erreurs
  (`ConnectionReset`…) ne sont pas masquées. Cohérent en
  plus : EOF toléré dans les trailers chunked finaux
  (`read_line_opt`/`stream_line_opt`, corps complet dès
  `0\r\n` — chemins bufferisé et streamé). Tests : 2
  unitaires (mapping UnexpectedEof→EOF, erreurs autres
  intactes) + 2 intégration (chunk `0` suivi de coupure,
  buffered + stream).
- **Étape 53b — Charset des sous-ressources** (bug terrain
  `2693959`, repro example.com) : scripts, modules et feuilles
  liées ne passent plus par le sniffer **HTML** — son repli
  `windows-1252` corrompait les littéraux UTF-8 des scripts
  réels (mojibake « Lâ€™usage » via `insertAdjacentHTML` /
  `textContent`). Ordre implémenté : classiques = attribut
  `<script charset>` → BOM → UTF-8 ; modules = UTF-8 strict
  (le `charset` de la balise est ignoré, spec) ; CSS =
  `charset` du `Content-Type` transport → BOM → `@charset`
  en tête → UTF-8. `decode_subresource`/`decode_stylesheet`/
  `label_encoding`/`transport_charset`/`css_charset` +
  `ScriptItem.charset` collecté. Tests : 8 unitaires
  (décodeurs, labels, BOM, `@charset` tardif ignoré) + 3
  intégration PNG (littéral `’` préservé, `charset`
  Latin-1 honoré, module UTF-8 malgré l'attribut).
- **Étape 53 — Session incrémentale** (`explorer-net`,
  `explorer-core`) : la page se peint pendant le
  téléchargement. `HttpTransport::fetch_stream` renvoie
  `StreamedResponse` (statut + en-têtes dès leur réception,
  corps par morceaux déframés sur un canal) ; défaut bufferisé
  un-morceau pour les fakes. `H1Client` streame réellement :
  la lecture corps tourne en tâche (`BodyTask`), chaque
  segment CL/chunked/EOF part immédiatement, la connexion
  rejoint le pool keep-alive à la fin si réutilisable ;
  `AutoClient` streame h2 (`DATA` → canal, fenêtre de
  flow-control libérée au fil de l'eau) et le repli ALPN
  `http/1.1` conserve sa connexion. Côté session,
  `navigate_request` alimente `TreeBuilder::push` au fil des
  morceaux (R-1 : arbre cohérent à frontière de token),
  `StreamDecoder` sniffe WHATWG sur 1 Ko puis décode
  incrémentalement (séquence UTF-8/surrogate UTF-16 coupée en
  frontière de morceau conservée en queue, pas de `U+FFFD`
  parasite), et `paint_progress` relayout+raster cadencé par
  `SessionConfig::paint_chunk_bytes` (32 Ko) ou
  `paint_interval_ms` (100 ms) — chaque peinture émet
  `FrameInvalidated` (SSE/UI repeignent en direct). Tests : 6
  scénarios session (cadence, frontières UTF-8/1252, égalité
  streamé/bufferisé, erreur typée) + 3 tests h1 réels
  (morceaux, chunked, remise au pool).
- **Étape 52 — Boucle d'événements v2 + tick timers**
  (`explorer-core`, `explorer-js`, `dillo-js`,
  `explorer-daemon`, `explorer-api`, `explorer-cli`,
  `explorer-ffi`) : la surface événementielle s'étend au-delà
  du clic — `mouseover`/`mouseout`/`mousemove` (survol suivi
  par `Tab.hovered`, coalescé naturellement : un événement
  par `move`), `keydown`/`keyup` (`InputEvent::KeyUp`,
  `keydown` annulable avant édition/soumission `Enter`),
  `input` après édition, `focus`/`blur` sur les transitions
  de champ (ordre navigateur), `wheel` annulable
  (`preventDefault` supprime le scroll) puis `scroll` niveau
  document après défilement effectif. Dispatch par
  **chemin** `(tag, index)` partout où un handler peut
  re-parser le document — `focused`/`hovered` stockés en
  chemins survivent aux re-parse (corrigé : `element_path`
  comptait tous les frères de même tag au lieu des
  précédents — cible fausse dès N≥2 frères ; `find_by_id`
  ne descendait jamais sous `#document`). Checkbox/radio :
  activation **avant** `click` (le handler lit `checked`
  post-bascule via `sync_mirror_attr` — `value`/`checked`
  poussés au miroir pour les mutations hors-script),
  `input` puis `change`, revert sur `preventDefault`.
  **Tick timers** : `install_timers` câblé au spawn moteur
  (`setTimeout`/`setInterval`/`requestAnimationFrame` —
  jamais installés auparavant), `JsEngine::run_pending_bounded`
  borne les callbacks par pompe (`max_timer_jobs`, défaut
  128 — ADR-0019), `next_timer_deadline` expose la proche
  échéance ; le daemon dort `min(échéance, battement)` et
  pompe net + timers + intentions de navigation, le CLI
  termine sa grâce sur timers échus, FFI `explorer_tick`
  pompe timers + drain nav. `MutationObserver` (JS-C26)
  livré au même microtask checkpoint que les timers. API :
  `"keyup"` ; FFI : `kind 4` = `KeyUp`. 16 tests session +
  5 adaptateur.
- **R-14 — Triage des échecs tree-construction** (`explorer-html`) :
  le harnais html5lib rapporte désormais les échecs complets avec
  comptage par fichier `.dat` (au lieu de 20 tronqués) — code de
  lecture de cas + dump normalisé extrait en `tests/common/`
  partagé. Inventaire des **639 échecs** par algorithme fautif :
  foreign content ~330, `<template>` contents 110, adoption
  agency ~65, `<noscript>` ~45, tokenizer ~40, `<select>`-en-table
  ~25, doctype/quirks ~21, frameset ~10, catégories ~8,
  after-body ~5, button-scope ~3. **Décision : le chantier
  tree-builder devient R-15 et préempte R-3/R-4** — un DOM mal
  construit empoisonne tout l'aval (layout, rendu, miroir JS).
- **JS-C26 — `MutationObserver`** (`dillo-js`) : le `Dom` miroir
  enregistre des observateurs par cible (`MutObsReg`, options
  `childList`/`attributes`/`characterData`, `subtree`,
  `attributeFilter`, `attributeOldValue`,
  `characterDataOldValue` — `attributes`/`characterData`
  implicites quand le `*Filter`/`*OldValue` associé est posé)
  et produit des `MutationRecord` (`type`, `target`,
  `addedNodes`/`removedNodes`, `attributeName`, `oldValue`)
  sur tout mutateurs instrumentés : `appendChild`,
  `insertBefore`, `removeChild`, `replaceChild`,
  `textContent`/`innerHTML`/`nodeValue`/`data`,
  `setAttribute`/`removeAttribute`. Côté moteur
  (`src/mutation.rs`) : constructeur `MutationObserver(cb)`,
  `observe`/`disconnect`/`takeRecords`, file de records par
  observer délivrée au **microtask checkpoint**
  (`promise.rs` — jamais en synchrone depuis le mutateur,
  batching en une livraison par checkpoint, livraison
  bouclée bornée pour les mutations ré-entrantes émises par
  le callback, records purgés par `disconnect`). Comportement
  confiné au DOM miroir — aucune fuite vers `explorer-dom`.
  Débloque l'étape 52.
- **Étape 51 — Câblage XHR/fetch/storage/cookie** (`explorer-core`,
  `explorer-daemon`, `explorer-cli`, `explorer-js`) : les
  `NetRequest` du moteur sont drainés aux points de synchro et
  exécutés en tâches tokio (`Session::drain_script_net`) —
  jamais de callback ré-entrant ; les réponses reviennent par
  canal et sont livrées par `Session::pump_net` (daemon :
  battement `NET_PUMP_MS` ; CLI : grâce bornée `net_pending()`
  ; points internes : `poll_net` synchrone après événements).
  **Politique ADR-0019 implémentée** : quota
  `max_script_net` = 16 requêtes en vol par onglet (au-delà :
  échec `status 0` → `error`), `Origin` canonique posé sur
  toute requête CORS (`origin_header_value`, port défaut
  omis), `Referer`/`Origin` via `initiator_headers`,
  `Cookie` injecté same-origin uniquement, `Set-Cookie`
  capturé same-origin uniquement. **CORS minimal** :
  cross-origin limité aux requêtes simples (verbes
  `GET`/`HEAD`/`POST` + en-têtes safelistés — le preflight
  `OPTIONS` est hors MVP, requête bloquée) ; réponse lisible
  seulement si `Access-Control-Allow-Origin` vaut `*` ou
  l'origine exacte, sinon opaque (`status 0` → `error`).
  En-têtes interdits au script filtrés (`sec-*`, `proxy-*`,
  `Cookie`, `Referer`…). `document.cookie` : écritures
  drainées vers le `CookieJar` partagé (`HttpOnly` filtré —
  non posable par script), snapshot installé au spawn ;
  le daemon partage le même jar entre `CookieTransport`
  (sous `RedirectTransport` — cookies re-calculés par hop)
  et session. `localStorage` : map par origine drainée aux
  points de synchro, persistée via `ProfileStore`
  (`load_storage`/`save_storage` — implémenté par
  `JsonFileStore` en `storage-<slug>.json` par origine) ;
  `sessionStorage` frais par contexte. `NetRequest::Abort`
  tue la tâche corrélée ; la fermeture d'onglet annule les
  siennes. Clôt l'étape 44 de ROADMAP.md.
- **JS-C27 — Traverse proxy net/storage/cookie** (`explorer-js`) :
  les surfaces `dillo-js` déjà livrées traversent le canal
  `DilloEngine` — pattern `drain_nav_requests` généralisé.
  `JsEngine` : `drain_net_requests()` (`NetRequest::Fetch` /
  `Abort`, `XhrTransport` côté worker empile, jamais d'I/O
  depuis le moteur) + `deliver_net_response()` (miroir
  sérialisable de `XhrResponse` — `readyState`/promesses et
  handlers tirent en synchrone moteur). `install_storage` /
  `drain_storage` : `localStorage` partagé par snapshot
  `BTreeMap` (persistance par origine côté session),
  `sessionStorage` frais par contexte. `set_cookie_snapshot` /
  `drain_cookie_writes` : vue `document.cookie` posée par la
  session (le vrai jar garde les politiques
  HttpOnly/Secure/SameSite), écritures brutes drainées avec
  miroir local de lecture. Prérequis interne de l'étape 51.
- **R-1 — Peinture progressive : le domaine tolère un document
  en construction** (`explorer-dom`, `explorer-html`,
  `explorer-layout`, `explorer-render`) : conduite incrémentale
  haut niveau `TreeBuilder::push(&str)`/`finish()` — tokenizer
  interne possédé par le builder, `dom()` observable entre les
  chunks à frontière de token (le couple bas niveau
  `feed`/`process` reste exposé ; `TreeBuilder::parse` réécrit
  par-dessus, chemin de référence unique). `Dom::document_element()`
  — `None` tant que le document en construction n'a pas
  d'élément racine (préambule : doctype, commentaires), pour
  l'usage du consommateur incrémental. Contrat rustdoc posé :
  DOM partiel = entrée supportée de `build_box_tree_*` /
  `build_display_list_*` ; aucun `NodeId` inséré n'est collecté
  tant que le builder vit (hit-test, images des frames
  intermédiaires). `Tokenizer::is_finished()`. Tests :
  `incremental.rs` (convergence à **toute** coupure octet,
  `NodeId` vivants, flux clos) et `progressive.rs` (frame
  cohérente à chaque push, contenu progressif, dernière frame
  pixel-identique au parse complet). Débloque la session
  incrémentale côté NAVIGATEUR (étape 53).
- **R-2 — WPT floats/block-formatting : 38/38 (2 exclus
  documentés)** (`explorer-css`, `explorer-layout`,
  `explorer-render`) : reftests CSS2 normal-flow de 27/40 à
  **38 cas jouables tous verts** (cliquet `WPT_BASELINE`
  27 → 38). Layout : les blocs anonymes sont du flux pur
  (n'héritent ni BFC ni float — corrige les placements
  bandés à zéro) ; les flottants s'ancrent au **haut de la
  ligne courante** ; les bandes en-ligne mesurent
  l'étendue complète (`band_over`) et honorent
  `direction: rtl`. Cascade : attributs de présentation
  HTML `width`/`height` injectés en préséance auteur
  minimale (CSS l'emporte) ; feuille UA corrigée
  (`thead` → `table-header-group`, `tfoot` →
  `table-footer-group` — ordre visuel de la grille).
  Tailles : `height:%` résout contre la hauteur du bloc
  conteneur (§10.5) propagée dans toutes les voies de
  layout ; la hauteur de table réserve ses propres
  bordures/padding ; `\u{A0}` n'est plus un blanc
  collapsible (`text::is_css_ws` — `&nbsp;` tient sa
  ligne). Tables : `border-collapse: collapse` MVP
  horizontal — arête partagée = max des candidats, rangs
  décalés, géométrie réelle des groupes de rangs peignant
  les lignes. Rendu : canvas blanche (fond de document) ;
  `background-image: url()` peinte en tuiles
  (`PaintImages` + `build_display_list_full`, additif) ;
  clip `overflow` arrêté à la grille device (bords AA
  cohérents avec les fill/blit) ; bordure `solid` sans
  largeur peint `medium` = 3 px ; **peinture du texte
  réparée** — ordre de composition fonte→plume
  (`post_concat` applique `self` puis `other`) qui
  collapsait tout glyphe à ~1 px, puis point de plume
  arrêté sur la grille device. Harnais WPT : résolution
  relative des `src`/`url()`, décodage des PNG
  `support/` (`ImageTable`, `build_box_tree_sized`).
  Skips `wpt_skip.txt` : `bfc-011` (reftest dépendant
  des métriques d'une fonte proportionnelle — le harnais
  n'a qu'Ahem : « Filler Text » = 176 px > 100 px wrappe
  dans la ref seule) et `block-in-inline-append-001`
  (`flags: dom` — la ref exige l'exécution JS, harnais
  statique).
- **Étape 50 — Scripts externes + scheduler de sous-ressources
  parallèle** (`explorer-core`, `explorer-net`, `explorer-js`,
  `dillo-js`) : `<script src>` fetché via `HttpTransport` —
  scheduler parallèle par onglet (`FuturesUnordered` borné à
  `SessionConfig::max_inflight` = 6), émission par priorité
  `document > CSS > script > img`, ordre document préservé par
  slots de réception indexés. `defer` = file post-parse,
  `async` = évalué à réception (classique) ou bufferisé avant
  la file post-parse (module) ; `type="module"` : graphe
  statique prefetché en BFS bornée (`max_modules`), instancié
  par le `ModuleLoader` existant (JS-C5) via
  `JsEngine::eval_module` + `MapLoader` — `type` non-JS,
  `nomodule` et fetches en échec ignorés proprement.
  `Referer`/`Origin` calculés à l'émission
  (`policy::initiator_headers`, strict-origin-when-cross-origin)
  et mixed-content filtré ; borne `max_scripts`. Insertion
  dynamique de scripts : hooks à définir avec JS-C35.
- **Étape 49 — CSS2 floats/BFC/whitespace : reftests WPT
  17/40 → 27/40 (67 %)** (`explorer-css`, `explorer-layout`,
  `explorer-render`, `explorer-cli`) : attribut `style`
  inline dans la cascade (priorité auteur maximale,
  `!important` inline dominant) + tolérance `<![CDATA[`/
  `]]>` du tokenizer (fixtures XHTML) — la majorité des
  échecs venait de là. `FloatCtx` partagé : flottants posés
  à leur position de flux (règle du « parent anonyme »,
  descente tant que la bande ne les contient pas, règle 5
  `max_top`), `clear` n'avance plus le flux, `band_over`
  (intersection des bandes sur toute la hauteur) pour BFC
  (`layout_bfc_child`) et atomes inline-block, containment
  §10.6.7, `float` → niveau bloc (§9.7). Inline : marges
  h/v des atomes dans la ligne, whitespace collapsing +
  espaces de bord de ligne supprimées (`trail_w`) — fini
  les carrés `.notdef` Ahem et les décalages de
  `text-align`. Rendu : boxes synthétiques (anonymous,
  text) sans fond/bordure clonés ; remplissages pleins
  pixel-snappés (coutures AA supprimées). Cliquet
  `WPT_BASELINE` 17 → **27** ; utilitaire `debug_dump`.
- **Étape 48 — Interop mesurée : reftests WPT, Acid2, sites
  réels** (`explorer-render`, `explorer-cli`, `scripts/`) :
  harnais reftest pixel-exact `tests/wpt_reftests.rs` —
  40 cas `css/CSS2/floats` + `normal-flow` extraits par
  `scripts/fetch_wpt.ps1` avec fonte Ahem du WPT, références
  `rel="match|mismatch"` résolues, tolérance AA documentée
  (0 pixel « dur » Δ>16, ≤512 « doux »), cliquet
  `WPT_BASELINE` = **17/40 (42 %)**, skip-list versionnée
  `wpt_skip.txt` (vide : aucun échec masqué). Jalon Acid2 :
  `tests/acid2.rs` + `scripts/fetch_acid.ps1` — cliquet
  robustesse + score d'alignement informatif **29,8 %** vs
  `reference.png` officielle. `explorer-cli shot <url>
  <png>` + `scripts/site_smoke.ps1` : **5/5 sites réels**
  rendus (exemple, info.cern.ch, duckduckgo, wikipedia, MDN).
- **Frontière défensive tiny-skia dans `raster.rs`** (bug
  amont trouvé par le smoke sites réels — duckduckgo.com) :
  `fill_guarded` borne la géométrie au domaine fixed-point
  24.8 (`SAFE_COORD` = 30 000 px — au-delà, `fdot16`
  déborde → panic `hairline_aa`) et route les « hairlines
  pures » (bords sub-pixel couvrant toute la largeur,
  `debug_assert` tiny-skia 0.11) par `fill_path` — AA
  préservée, jamais de crash. Glyphes bornés à la plume,
  clips clampés au domaine, `draw_pixmap` affine gardé.

- **Étape 47 — Mode in-process `explorer-ffi` + frames RGBA
  via `dart:ffi`** (`explorer-ffi` nouveau crate `cdylib`,
  `app/`, ADR-0018) : ABI C volontairement étroite — handle
  opaque + `open_tab`/`close_tab`/`navigate`/`reload`/
  `go_back`/`go_forward`/`set_viewport`/`input` (codes
  discrets)/`tick`/`frame_size`/`frame_copy` RGBA brut/
  `tab_url`/`last_error` thread-local ; runtime tokio privé
  `current_thread`, pile réseau identique au démon.
  `Session::frame_pixels` expose la frame empruntée.
  **Dérogation unique** : seul crate du workspace en `unsafe`
  (frontière C — validation pointeurs + copies bornées, lint
  `unsafe_code` recopié en `allow` dans son manifeste seul).
  Côté Flutter : `FfiEngine implements BrowserRepository`
  (dart:ffi), `fetchFrameImage` → `ui.decodeImageFromPixels`
  → `RawImage` (zéro PNG/socket), `tryLoad` → repli REST ;
  `EXPLORER_ENGINE=auto|rest|ffi`, `EXPLORER_ENGINE_LIB`.
  Pas de SSE → `Timer` 50 ms `tick` + pull post-action.
  Tests : 4 Rust (pipeline ABI navigate→RGBA pixel, bornes
  buffer, handle nul, tick) + 2 Dart (repli, ABI réel si DLL).

- **Étape 45 (partie moteur) — SVG statique + transitions
  CSS** (`explorer-render`, `explorer-css`, `explorer-core`,
  `explorer-api`, ADR-0017) : `decode_image` détecte le SVG
  par sniff (`<` + `<svg` prolog) et le rasterise via
  `resvg`/`usvg` 0.45 à taille naturelle réduite au budget
  `max_px` (viewBox hostile plafonné, pixmap prémultiplié →
  RGBA droit) — `<img src=*.svg>` complet ; SMIL/scripts
  hors périmètre, `<svg>` inline en déviation documentée
  (foreign-content tokenizer requis). `transition.rs` :
  parsing `transition`/longhands, `linear`/`ease*`/
  `cubic-bezier` (Newton+bissection), interpolation nombres/
  longueurs (même unité ou absolues→px)/couleurs RGBA, le
  reste discret à t=1. Session : diff sur valeurs calculées
  indexé par **ordinal d'élément** (survit au re-parse de la
  synchro DOM — `NodeId` régénéré), surcharge post-cascade
  de priorité max, `MAX_TRANSITIONS` = 256, retarget depuis
  la valeur courante, mort à la navigation ;
  `tick_animations` appelé par `GET /frame` — l'animation
  est cadencée par le pull de l'UI. 12 tests : transition
  (6), svg (4), session tick (2).

- **Étape 44 (partie moteur) — `location`/`history` →
  navigation** (`explorer-js`, `explorer-core`) :
  `JsEngine::drain_nav_requests` + `NavRequest`
  (`Navigate(url résolue)` / `HistoryGo(delta)`) —
  `location.href`/`assign`/`replace`/`reload` et
  `history.back`/`forward`/`go` de `dillo-js` deviennent des
  intentions en file, jamais de callback ré-entrant dans le
  pipeline. `navigate_request` reste feuille du graphe
  d'appels : l'entrée d'historique est poussée **avant** le
  drain script (un `history.go` en vol voit l'index réel,
  une redirection pousse sa propre entrée), `Navigated`
  n'est émis que si le document final est celui demandé.
  `drain_script_nav` itératif borné `MAX_NAV_HOPS` = 8 —
  appliqué après chargement, événement DOM, jobs en attente
  et soumission de formulaire ; `history.go(0)` =
  rechargement (spec HTML). 4 tests session
  (`location.href`, URL invalide ignorée, `history.back`,
  soumission) + drain/lecture URL adaptateur.

- **Étape 46 — Webfonts + sous-ressources CSS**
  (`explorer-css`, `explorer-render`, `explorer-core`,
  ADR-0016) : `@font-face` conservé par `StyleEngine` avec
  pile `@media` — `font_faces(env)` sert les descripteurs
  (`font-family`, `src` `url()`/`local()`, weight, style,
  `font-display`) ; feuilles `<link rel=stylesheet>`
  fetchées borné (`max_sheets`) et jointes à la cascade en
  origine auteur ; `woff.rs` — WOFF1 complet (tables
  zlib/brutes → sfnt avec checksums) + WOFF2 à transform
  nulle (flot brotli, glyf transformé refusé proprement,
  `ttcf` refusé) — bornes 64 tables / 32 Mio table /
  64 Mio sortie, décompression plafonnée ; `WebFontSource`
  composite — faces web (famille normalisée + meilleure
  graisse/italique, validées `ttf-parser` à l'inscription,
  registre `max_fonts` purgé par navigation) devant la
  source locale de repli ; `font-display` parsé,
  comportement `block` synchrone appliqué (une frame = un
  état cohérent). 14 tests : descripteurs+media (4), woff
  (5), WebFontSource (3), pipeline session TTF/WOFF/feuille
  liée/cap (3).

- **Étape 42 — Layout moderne MVP** (`explorer-css`,
  `explorer-layout`, `explorer-render`, ADR-0015) : custom
  properties `--*` héritées + `var()` substitué au temps de
  valeur calculée (fallbacks imbriqués, cycles/IACVT
  bornés) ; `calc()` `+ − × ÷` + parenthèses/`calc` imbriqué
  résolu dans `css_px` (toutes les propriétés de longueur) ;
  flexbox une ligne — `flex-direction` row/column ±reverse,
  `order`, `flex-grow`/`shrink`/`basis` + shorthand `flex`,
  `justify-content`, `align-items`/`align-self`, `gap` ;
  grid — `grid-template-columns/rows` (`px`/`%`/`fr`/`auto`,
  `repeat`, `minmax`→max), `grid-column`/`grid-row` + `span`,
  auto-placement row-major, `justify-items`/`align-items` ;
  `transform` 2D (matrix, translate, scale, rotate, skew) via
  `PushTransform`/`PopTransform` — composée `dpi · pile` au
  raster sur rects, bordures, glyphes et images (nearest
  axis-aligned, `draw_pixmap` affine sinon) ;
  `transform-origin` px/% ; `position: sticky` (seuil `top`)
  contraint par translation de paint bornée au parent.
  Écarts MVP documentés en modules : pas de `flex-wrap`,
  `margin:auto`, areas grid, max-content intégral,
  `stretch` cross, `bottom` sticky, clip transformé en bbox.
  26 tests : var (6), calc (4), flex (4), grid (3),
  transforms display+raster+sticky (9).

- **Étape 41 — HTTP/2** (`explorer-net`, `explorer-daemon`,
  `explorer-cli`) : `AutoClient` route le trafic `https` par
  ALPN (RFC 7301) — `h2` → connexion multiplexée partagée par
  origine (`SendRequest` cloné par requête, pilote de
  connexion en tâche, corps requête/réponse bornés par
  `max_body_bytes` + fenêtres de flow-control respectées),
  `http/1.1` ou ALPN absent → repli h1 **sur la connexion
  déjà négociée** (`Connector::connect_negotiated` rapporte
  l'ALPN, `H1Client::fetch_stream` la réutilise). `http:`
  clair reste h1 (`h2c` hors périmètre), push server non
  implémenté (déprécié). `TlsConnector` offre `h2` +
  `http/1.1`. Framing/multiplexage délégué au crate `h2`
  (pur Rust, équipe tokio — infrastructure, ADR-0002) +
  types `http`/`bytes`. `NetError::Http2` pour les erreurs
  de couche. 4 tests : h2 corps+en-têtes, POST echo, repli
  h1 sur même connexion, `http:` h1.

### Changed

- **Étape 53d — navigation éclatée, API réactive pendant la
  charge** (`explorer-core`, `explorer-api`, `explorer-daemon`,
  bloquant produit — QA daemon @ `cfa0514`) : le
  `Mutex<Session>` unique était tenu pendant tout le
  `fetch`/parse/sub-ressources d'une navigation — nav > 8
  min ⇒ `/status`, `/tabs`, `/frame`, l'input et le SSE
  timeoutaient tous. La navigation est désormais découpée
  en phases : `nav_begin`/`nav_apply`/`sub_commit` sous
  verrou court ; `NavJob::run` (fetch + parse incrémental)
  et `SubJob::run` (vague CSS/scripts/images, `@font-face`,
  prefetch des graphes de modules — les `eval` restent
  synchrones au commit) tournent **hors verrou** sur des
  `Arc` du transport. Nouveaux drivers publics
  `navigate_shared`/`drain_navs_shared`/`history_move_shared`
  pour les hôtes à session partagée ; les intentions de
  navigation input (`clic lien`, `Enter`/submit) et script
  (`location`, `history`, `form.submit()`) convergent dans
  `pending_nav` — `handle_input` n'attend plus le réseau
  non plus. Epoch par onglet : une navigation supplantée
  (double-clic, redirect plus récent) est abandonnée avant
  tout commit. Peinture progressive par `try_lock` (tick
  manqué = repeint au suivant). Handlers API
  `navigate`/`reload`/`back`/`forward`/`input` et pompe
  daemon basculés ; `navigate`/`reload`/`go_*`/`handle_input`/
  `drain_script_nav` restent en compat `&mut self` pour
  CLI/FFI/tests. Régression couverte : la session partagée
  répond en <500 ms pendant un corps de document en vol.

- **Organisation deux équipes** (réorganisation de
  pilotage, code inchangé) : périmètres disjoints actés sur
  l'axe clean architecture — équipe **PIPELINE** (domaine
  pur : `common`, `dom`, `html`, `css`, `layout`, `render`,
  zéro I/O) et équipe **NAVIGATEUR** (plateforme produit :
  `net`, `core`, `api`, `daemon`, `cli`, `ffi`,
  `test-support`, `app/`, `scripts/`, `fuzz/`, **+ toute la
  surface JS** — `explorer-js` et `dillo-js`). Le trait
  `JsEngine` devient contrat **interne** à NAVIGATEUR ; la
  couture inter-équipes = les API publiques des crates
  PIPELINE, loguées dans `docs/interface-equipes.md`. Le
  rôle « spécialiste JS » vit au sein de NAVIGATEUR (piste
  JS-C26+). `ROADMAP.md` devient l'index maître ; le détail
  vivant migre vers `docs/roadmap-equipe-pipeline.md`
  (étapes R-1+, activateurs cross-équipe en tête) et
  `docs/roadmap-equipe-navigateur.md` (étapes 50+ + piste
  JS). `explorer-test-support` classé crate **partagé**
  (consommé en dev-deps par les deux). Workflow trunk-based
  sur `main` (pas de branches longues), `CHANGELOG.md` en
  `merge=union` (`Cargo.lock` : régénéré, jamais fusionné à
  la main), `.github/CODEOWNERS` créé, état des lieux mesuré
  dans `docs/etat-des-lieux-2026-10-19.md` (AGENTS.md §10).
- **ADR-0019 — réévaluation du modèle de menace JS
  in-process** : solde l'exigence d'ADR-0011 (« réévaluation
  obligatoire en phase 8 ») — thread dédié + budget d'ops +
  miroir DOM + `forbid(unsafe_code)` couvrent les vecteurs
  actuels ; prochains points fixés au câblage XHR (étape 51),
  aux workers (JS-C29) et à un éventuel défaut FFI.
- `app/pubspec.yaml` : description projet remplacée
  (placeholder Flutter). `examples_dbg/` vide supprimé.

### Fixed

- **`<br>` et sauts de ligne forcés perdus (régression
  étape 49)** (`explorer-layout`) : le collapsing
  §16.6.1 transformait le `"\n"` synthétique du `<br>`
  en espace avant l'itemisation — plus de coupure dure,
  tout coulait sur une seule ligne (champs de formulaire
  intraçables au hit-test). Le `<br>` porte désormais un
  texte sentinelle **U+2028 LINE SEPARATOR** — pas un
  blanc CSS, il survit au collapsing et reste une
  coupure dure UAX #14. Les pièces réduites à leur
  coupure (`strip_break_chars` vide le texte) étaient
  sautées avant l'évaluation de `hard_after` : elles
  poussent maintenant une sentinelle atomique
  zéro-largeur + `new_line!` — une ligne vide est
  réémise (`<br>` seul, lignes vides de `pre`). Bonus :
  `pre`/`pre-wrap` préservent les espaces de tête de
  ligne. Détecté par le gate `verify_all` au point
  d'équipe — session 44/44, WPT 27/40 tenu.

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
- **JS-C12 — Construction + API ES2025** (`dillo-js`) :
  - `new.target` (`Expr::NewTarget` → liaison cachée
    `\u{f120}new.target` posée par `call_value_impl` :
    ctor cible sous `new`, `undefined` en appel ordinaire,
    propagée à travers `super()` et `call_value_ctor`,
    héritée lexicalement par les flèches — pas de binding
    propre sur `is_arrow`) ;
  - `Reflect.construct(target, args, newTarget)` — proto de
    l'instance pris sur `newTarget.prototype` (spec
    `OrdinaryCreateFromConstructor`) ; la lecture de
    `prototype` passe par `get_prop_value` (traps `get` de
    proxy + getters traversés — corrige `new` sur proxy sans
    trap `construct`) ;
  - méthodes `Set` ES2025 : `union`, `intersection`,
    `difference`, `symmetricDifference`, `isSubsetOf`,
    `isSupersetOf`, `isDisjointFrom` — tout itérable accepté ;
  - `String.prototype.isWellFormed`/`toWellFormed` (nos
    `char` sont toujours des scalaires — `isWellFormed` vaut
    `true`, `toWellFormed` identité) ;
  - `RegExp.escape` (ES2025) : `SyntaxCharacter`s préfixés,
    ASCII non sûr en tête → `\xNN`/`\uNNNN` (`a…` → `\x61…`) ;
  - `ArrayBuffer` redimensionnable : `resize`/`resizable`/
    `maxByteLength`, `transfer`/`transferToFixedLength`
    (détachement — les vues sur buffer détaché ou trop court
    lèvent `TypeError` au lieu de paniquer) ;
  - test262 : 859/859 conservé ; 4 tests de régression
    (`new_target_binding`, `set_composition_es2025`,
    `string_well_formed_and_regexp_escape`,
    `arraybuffer_resize_transfer`).
- **JS-C20 — `Intl` minimal, `Array.from` complet,
  invariants Proxy proto, `getOrInsert`,
  `AsyncDisposableStack`** (`dillo-js`) :
  - `Array.from(src, mapFn?, thisArg?)` conforme spec :
    `@@iterator` → itérateur, sinon lecture array-like
    `length`-driven (`{length:2}` ne rend plus `[]` —
    bug corrigé), `mapFn(v, i)` appelée avec `thisArg`,
    `TypeError` si `mapFn` non-callable ;
  - invariants Proxy des traps de niveau proto (spec
    §9.5) : `getPrototypeOf` (résultat `===` proto cible
    si non-extensible), `setPrototypeOf` (proto demandé
    identique à celui de la cible scellée),
    `isExtensible` (booléen exact de la cible),
    `preventExtensions` (`true` exige une cible déjà
    non-extensible) — violation → `TypeError` ;
  - `Intl` — sous-ensemble `en-US`/UTC fixe sans données
    CLDR (déviation documentée) : `getCanonicalLocales`/
    `supportedLocalesOf` (canonicalisation BCP-47 :
    `en-us` → `en-US`, `sr-cyrl-rs` → `sr-Cyrl-RS`,
    `RangeError` sur tag invalide) ; `Intl.DateTimeFormat`
    (`dateStyle`/`timeStyle`/composants — mois numérique
    → `MM/DD/YYYY`, nommé → `Month D, YYYY`, `timeStyle`
    seul sans date, minute/second paddées —,
    `formatToParts`, `resolvedOptions`, `formatRange`
    approximé, `format` bound à l'instance comme spec) ;
    `Intl.NumberFormat` (styles decimal/currency/percent/
    unit, `minimumFractionDigits`/`maximumFractionDigits`/
    `minimumIntegerDigits`, `useGrouping` dont `min2`,
    `signDisplay`, `notation` compact/scientific,
    `formatToParts` segmenté integer/group/fraction/
    currency/percentSign/exponent, NaN/`∞`) ; `Intl.
    Collator` (`numeric` collation chiffres,
    `sensitivity` base → fold case, `ignorePunctuation`,
    `compare` bound) ; `Intl.PluralRules` (cardinal
    `one`/`other` + ordinal `en` st/nd/rd/th,
    `selectRange` → `other`) ; `Intl.ListFormat`
    (conjunction `a, b, and c`, disjunction `or`, unit,
    `formatToParts` element/literal) ; `Intl.
    RelativeTimeFormat` (`in N units`/`N units ago`,
    `numeric: auto` → `yesterday`/`today`/`tomorrow`/
    `last|this|next unit`/`now`) ; `Intl.Locale`
    (accesseurs `baseName`/`language`/`script`/`region`/
    `calendar`/`numberingSystem`, options qui
    surchargent le tag, `maximize`/`minimize` →
    nouvelles `Locale`) ; `Intl.DisplayNames` (tables
    `en` régions/langues/scripts/devises + repli sur le
    code, spec `fallback: 'code'`) ; `@@toStringTag
    "Intl"` ; `supportedLocalesOf` statique sur chaque
    constructeur (spec) ;
  - `Map.prototype.getOrInsert`/`getOrInsertComputed`
    (ES2026 — `cb` appelée seulement en absence ;
    héritées par `WeakMap` via la chaîne proto) ;
  - `AsyncDisposableStack` (ES2026) : `use` prend
    `v[@@asyncDispose] ?? v[@@dispose]`, `adopt`/`defer`/
    `move`/`disposed`, `disposeAsync()` dépile LIFO en
    awaitant chaque retour (await eager — déviation
    documentée) et rend une promesse, erreurs
    enroulées en `SuppressedError` (spec),
    `@@asyncDispose` aliasé, `@@toStringTag` ;
  - test262 : 859/859 ; 8 tests de régression.

- **JS-C21 — `splice`, `using`/`await using`,
  `@@hasInstance`, ES2026** (`dillo-js`) :
  - `Array.prototype.splice` enfin implémenté (gap
    critique — absent depuis toujours) : `start` clampé
    dans `[0, len]` (négatifs relatifs à la fin),
    `deleteCount` par défaut = fin du tableau, un seul
    arg supprime tout, insertion des args restants,
    retour du tableau des éléments supprimés,
    `length` mis à jour — l'array n'est réécrit qu'en
    fin d'opération (un throw de conversion le laisse
    intact) ;
  - gestion explicite de ressources (ES2026) :
    `using x = e` / `await using x = e` — parsing
    contextuel (`using` reste un identifiant valide :
    `let using=5` fonctionne, `for (using of y)` =
    variable), frames de disposeurs empilées par corps
    (bloc, fonction, programme, itération `for-of`),
    dépilement **LIFO** à toute sortie — normal,
    `return`, `throw`, `break` (spec
    `DisposeResources`) ; `v[@@dispose]()` /
    `v[@@asyncDispose]()` avec await eager (déviation
    documentée — pas de suspension) ; `null`/`undefined`
    passent sans disposeur ; objet sans méthode ou
    primitive → `TypeError` (spec
    `AddDisposableResource`) ; double erreur →
    `SuppressedError` ; `for (using r of it)` dispose la
    ressource à chaque itération ;
  - `Function.prototype[Symbol.hasInstance]` exposé —
    `OrdinaryHasInstance` complet : fonction bound
    délègue à sa cible via `InstanceofOperator` (spec),
    `prototype` non-objet → `TypeError` ; refactor
    `instance_of`/`ordinary_has_instance` partagé entre
    `instanceof` et le hook ;
  - `Error.isError` (ES2026) — approximation
    documentée : chaîne de prototypes atteignant
    `Error.prototype` (couvre `TypeError`,
    `AggregateError`, sous-classes) ;
  - `Math.sumPrecise(iterable)` (ES2026) — somme
    compensée Neumaier/double-double (`[1e20,1,-1e20]`
    → `1`, pas `0`) ;
  - `Symbol.metadata` (well-known ES2026 decorators,
    posé au démarrage comme spec) ;
  - `JSON.rawJSON(text)` / `JSON.isRawJSON(v)` (ES2026)
    — objet gelé à proto nul portant `rawJSON`, texte
    re-parsé (conteneur ou JSON invalide →
    `SyntaxError`), émission **brute** dans
    `stringify` ;
  - test262 : 859/859 ; 5 tests de régression.

- **JS-C22 — `Temporal` tranche initiale + gaps**
  (`dillo-js`, nouveau `src/temporal.rs`) :
  - `Temporal.Instant` — nanosecondes `i128`, ctor
    `BigInt` requis (spec), `fromEpochMilliseconds`/
    `fromEpochNanoseconds`, `toString` ISO `Z`,
    offsets `±HH:MM` appliqués, `since`/`until`
    (largestUnit temps uniquement — `days` →
    `RangeError` spec) ;
  - `PlainDate`/`PlainTime`/`PlainDateTime` — props
    complètes (`dayOfWeek` lundi=1, `dayOfYear`,
    `weekOfYear`/`yearOfWeek` ISO-8601, `daysInMonth`/
    `daysInYear`, `monthsInYear`, `inLeapYear`,
    `monthCode`, `calendarId`), `from` chaîne ISO ou
    objet-props (`monthCode` accepté), conversions
    `toPlainDateTime`/`toPlainDate`/`toPlainTime`/
    `toPlainYearMonth`/`toPlainMonthDay`, `with`,
    `equals`, `compare` ;
  - arithmétique calendaire réelle : `add`/`subtract`
    avec clamp du jour (`Jan 31 + 1 mois → Feb 29`),
    `since`/`until` selon `largestUnit`
    (`days`/`weeks`/`months`/`years` calendaires,
    temps en nanosecondes `i128`) — `Jan 15 → Mar 20`
    en mois = `P2M5D` conforme ;
  - `PlainYearMonth`/`PlainMonthDay` (réf. iso jour 1 /
    année 1972) ;
  - `Temporal.Duration` — 10 champs, signe uniforme
    exigé (`RangeError` sinon), parse/format ISO
    `P…`, `neg`/`abs`/`add`/`subtract`/`with`,
    `total(unit)` et `compare` avec `relativeTo`
    obligatoire pour les unités date (spec) ;
  - `Temporal.Now` — horloge UTC (`instant`,
    `plainDateTimeISO`, `plainDateISO`,
    `plainTimeISO`, `timeZoneId` → `"UTC"`) ;
  - `valueOf` lève `TypeError` sur tout Temporal
    (spec — conversion implicite interdite), `toJSON`,
    `@@toStringTag` `Temporal.X` ;
  - déviations documentées (tête de module) :
    calendrier `iso8601` unique, fuseau UTC seul —
    `ZonedDateTime`/`Now.zonedDateTimeISO` absents,
    props publiques en data non-énumérables au lieu
    d'accesseurs de prototype, `PlainDate.add` ignore
    les champs sub-jour, fraction de durée limitée
    aux secondes/minutes ;
  - compléments : `Atomics.pause` (hint spin — no-op
    observable), `Iterator.concat` (eager, déviation
    laziness documentée), propriétés restreintes
    `caller`/`arguments` sur les fonctions
    strictes/générateurs/async (spec
    `AddRestrictedFunctionProperties` — sloppy :
    absents, déviation vs le `null` legacy de V8) ;
  - test262 : 859/859 ; 6 tests de régression.

- **Étape 43 cochée — JS vers ES6+** : les 4 livrables nommés
  sont livrés dans `dillo-js` (modules JS-C5, `Proxy` JS-C4+,
  `Intl` minimal JS-C20, GC ADR-0008 JS-A) ; couverture effective
  JS-C1→JS-C24, test262 859/859. Caveats documentés dans
  `ROADMAP.md` (générateurs lazy, `Intl` CLDR, `Temporal`
  complet, pont générationnel `explorer-dom` ↔ miroir) —
  échappatoire `boa_engine` non requise.

- **JS-C25 — ordre `[[OwnPropertyKeys]]`, tableaux
  épars, Symbol `+`, string-box, `\p{…}`**
  (`dillo-js`) :
  - `spec_key_order` — indices entiers croissants,
    puis strings par insertion, puis symboles —
    pour `Object.keys`/`getOwnPropertyNames`/
    `for-in`/`JSON.stringify` ;
  - `Value::Hole` : `Array(n)`/littéral/`delete`/
    grow produisent de vrais trous ; `in`/
    `hasOwnProperty`/descripteurs voient l'absence,
    toute lecture rend `undefined` ; méthodes
    `HasProperty` sautent (`forEach`/`map`/
    `filter`/`reduce`/`find*`/`every`/`some`/
    `indexOf`/`flat*`), `includes` voit
    `undefined`, `with`/`toReversed`/`toSorted`/
    `toSpliced` densifient, `slice`/`concat`
    préservent ;
  - `ToPrimitive(symbole)` → symbole : `sym+x`,
    `sym<n`, `` `${sym}` `` → `TypeError` (spec) ;
    `String(sym)` explicite reste légal ;
  - string-box : `hasOwnProperty`/`Object.keys`/
    `getOwnPropertyDescriptor`/`getOwnPropertyNames`
    fonctionnent sur primitif ; `new String(s)`
    gagne `length` non-writable ;
  - `\p{Name}`/`\p{Name=Value}` acceptés en regex
    `u` (pass-through fancy-regex, erreurs spec) ;
  - test262 : 859/859 ; 6 tests de régression.

- **JS-C24 — `arguments` mappé, `bind` props,
  protos Weak*, `fn.toString`** (`dillo-js`) :
  - objet `arguments` mappé (spec
    `CreateMappedArgumentsObject`) — nouveau champ
    `Object::args_map` `(EnvRef, Rc<Vec<Option<name>>>)` :
    pour les fonctions **sloppy** à liste de
    paramètres simple, `arguments[i]` est aliasé sur
    le slot env du paramètre `i` — lecture et
    écriture bidirectionnelles (`a=9 → arguments[0]`,
    `arguments[0]=9 → a`), `eval` traverse, noms
    dupliqués → dernier slot, `delete` démappe puis
    supprime, non-mappé en strict / rest / defaults /
    destructuring ; `arguments.length` configurable
    (spec) et GC trace l'env capturé. Déviation
    documentée : les mutators (`sort`, `splice`…)
    écrivent les éléments sans remonter l'alias ;
  - `Function.prototype.bind` — props `length`
    (`max(0, target.length − boundArgs)`) et `name`
    (`"bound "+target.name`), `{w:false, c:true}`
    (spec `BoundFunctionCreate`) ;
  - `WeakMap.prototype`/`WeakSet.prototype`
    reparentés à `Object.prototype` (spec) — plus
    d'`@@iterator`/`keys`/`values`/`size` hérités de
    Map/Set ; méthodes spec seules exposées +
    `getOrInsert`/`getOrInsertComputed` (ES2026,
    présents sur WeakMap aussi) ;
  - `Function.prototype.toString` — paramètres
    reconstruits depuis les patterns
    (`function f(a, b) { [code] }`,
    `(x) => { [code] }`, `function*`/`async function`
    /`async function*`, `[native code]` pour
    natives et bound — approximation documentée du
    texte source ;
  - test262 : 859/859 ; 4 tests de régression.

- **JS-C23 — sémantique UTF-16 + intégrité des
  éléments de tableau** (`dillo-js`) :
  - APIs String en code units UTF-16 (spec) :
    `.length`, indexation `s[i]`, `charAt`/
    `charCodeAt`/`at`/`codePointAt` (paire
    surrogate → code point complet, unit isolé →
    le unit), `indexOf`/`lastIndexOf` avec
    position `from` (défaut : fin pour
    `lastIndexOf` — régression corrigée),
    `slice`/`substring`/`substr`, `split('')`,
    `includes`/`startsWith`/`endsWith` avec
    `position`, `padStart`/`padEnd`,
    `propertyIsEnumerable` sur indice ;
  - `String.fromCharCode` reconstitue les paires
    (`(0xD83D,0xDE00)` → `\u{1F600}`) ;
    `fromCodePoint` inchangé (code points) ;
  - déviations documentées : surrogate isolé →
    `U+FFFD` (`String::from_utf16_lossy` — nos
    strings sont UTF-8), `match.index` et offsets
    `replace` restés en chars, littéral
    `\ud800` → `U+FFFD` au lexing ;
  - bug hostile corrigé : `startsWith(pos)`/
    `endsWith(pos)`/`includes(pos)` tranchaient
    `s[pos..]` en octets → panic sur frontière
    non-`char` — recherche désormais sur la vue
    `encode_utf16` ;
  - `Object.seal`/`Object.freeze` sur éléments
    de tableau — nouveau champ
    `Object::elem_integrity` (bits `1` sealed,
    `2` frozen, `4` `length` non-writable) :
    écritures d'éléments refusées (`TypeError`
    strict / silence sloppy via `write_refused`),
    `delete` refusé, descripteurs
    `getOwnPropertyDescriptor` corrects, ajout
    hors bornes bloqué, `isSealed`/`isFrozen` ;
  - mutators conformes `Set`/`Delete` en mode
    throw — `put_elems_guarded` (longueur) +
    checks upfront par intention d'écriture :
    `push`/`pop`/`shift`/`unshift`/`splice`
    (delete/insert), `sort`/`reverse`/`fill`/
    `copyWithin` (plages vides → no-op admis) →
    `TypeError` même en sloppy ;
  - `seal`/`freeze` sur `TypedArray` non vide →
    `TypeError` (spec `SetIntegrityLevel`) ;
  - `Object.defineProperty` sur indice de
    tableau (valeur posée, descripteur
    incompatible avec l'intégrité → `TypeError`,
    accesseur refusé) et sur `length` (resize,
    verrou `writable:false`) ;
  - test262 : 859/859 ; 6 tests de régression.

- **JS-C19 — Date complet, `eval` direct, Annex B
  RegExp, JSON, invariants Proxy** (`dillo-js`) :
  - `Date` : les 15 setters (`setFullYear`…`setTime` +
    variantes `UTC` — fuseau unique), `toString`/
    `toDateString`/`toTimeString`/`toLocaleString`/
    `toLocaleDateString`/`toLocaleTimeString` au format
    spec (UTC fixe, `en-US` pour les locale*),
    `toJSON` (dispatch `toISOString`, `null` si NaN) ;
  - `Date.parse`/`new Date(str)` : formats legacy Annex
    B (`Jan 15 2000`, `January 15, 2000`, `15 Jan 2000`
    — ne lit plus « 15 » comme année 1915 —, `MM/DD/YYYY`,
    `YYYY/M/D`, `h:m[:s[.ms]]`, jours de semaine,
    ordinaires `1st`, `GMT`/`UTC`, tokens inconnus
    tolérés) + garde ISO strict ; `days_from_civil`
    normalise les mois hors 1-12 (`setMonth(13)`, ctor) ;
  - `arguments` : `callee` sur l'objet sloppy (strict :
    absent — déviation vs poisoned accessor ; pas
    d'aliasing param/arguments, documenté) ;
  - `eval` direct (spec `PerformEval`) : `eval(...)`
    écrit tel quel s'exécute dans l'env + `this` de
    l'appelant — `var`/fonctions y persistent, bindings
    locaux visibles ; `(0,eval)`/`e=eval`/`Function()`
    restent globaux ; identité vérifiée par valeur (un
    homonyme local `eval` n'est pas un appel direct) ;
  - Annex B : `String.prototype.trimLeft`/`trimRight`
    (alias `trimStart`/`trimEnd`) ; statiques `RegExp`
    — `$1`…`$9`, `input`/`$_`, `lastMatch`/`$&`,
    `lastParen`/`$+`, `` $` ``/`$'`/`leftContext`/
    `rightContext` alimentées par `UpdateRegExpStatics`
    sur `exec`/`test`/`match`/`search`/`replace`/
    `matchAll` (échec → statiques vidées, groupe non
    participé → `""`) ;
  - `JSON.stringify` : `PropertyList` (whitelist +
    ordre, dédup), replacer fonction (`holder` en
    `this`, clé `""` à la racine), `toJSON` dispatché,
    symboles élidés (racine `undefined`, valeur omise,
    élément de tableau → `null`), wrappers
    `Number`/`String` en `space` ;
  - `Number.prototype.toString(radix)` : partie
    fractionnaire en base (`(0.5).toString(2)` →
    `"0.1"`, borne 53 digits mantisse) ;
  - `@@toStringTag` : protos+ctors `WeakMap`/`WeakSet`
    dédiés (`WeakMap !== Map`, `[object WeakMap]`),
    `ArrayBuffer` ; `new ArrayBuffer()`/`new
    SharedArrayBuffer()` sans arg → buffer vide (spec
    `ToIndex` court-circuité, était `RangeError`) ;
  - `Array.prototype` est un tableau exotique (spec) :
    `Array.isArray(Array.prototype)` → `true` et
    `length` a son descripteur (`{w:true,e:false,
    c:false}`, valeur 0) ;
  - invariants Proxy (spec §9.5) vérifiés sur les traps
    `get`/`set`/`has`/`deleteProperty`/
    `getOwnPropertyDescriptor`/`ownKeys`/`defineProperty`
    — violations sur props non-configurables ou cibles
    non-extensibles → `TypeError` (ex. `get` ne peut
    plus mentir sur une prop gelée) ;
  - test262 : 859/859 conservé ; 10 tests de régression
    (`c19_date_setters_and_formats`,
    `c19_date_parse_legacy`, `c19_arguments_callee`,
    `c19_direct_eval`, `c19_annexb_regexp_statics`,
    `c19_annexb_string_trim`, `c19_json_stringify`,
    `c19_number_radix_fraction`,
    `c19_tags_and_array_proto`, `c19_proxy_invariants`).
- **JS-C18 — Annex B complet, `SharedArrayBuffer`,
  `Atomics`** (`dillo-js`) :
  - méthodes HTML `String.prototype` (Annex B) :
    `anchor`/`big`/`blink`/`bold`/`fixed`/`fontcolor`/
    `fontsize`/`italics`/`link`/`small`/`strike`/`sub`/
    `sup` — `CreateHTML`, `"` → `&quot;` dans l'attribut ;
  - `Date.prototype.getYear`/`setYear` (Annex B : années
    0-99 → 1900+n, mois/jour conservés) ;
  - `Number.EPSILON`, `Number.MIN_VALUE` (5e-324),
    `Number.isSafeInteger` ; `BigInt.prototype.
    toLocaleString` (repli `toString` — pas de CLDR) ;
  - `AggregateError.prototype.name` propre (spec — était
    hérité `"Error"`) ;
  - accesseurs `ArrayBuffer.prototype` spec :
    `resizable`/`maxByteLength`/`detached` (`TypeError`
    sur non-buffer — les instances gardent des props
    propres, déviation mineure `hasOwnProperty`) ;
  - `SharedArrayBuffer` : ctor `{maxByteLength}` →
    `growable`/`grow`, `ByteBuf` partageable par les vues,
    `@@toStringTag` ;
  - `Atomics` complet (mono-thread — le partage inter-
    worker est indiscernable sauf `wait`) : `load`/`store`/
    `add`/`sub`/`and`/`or`/`xor`/`exchange`/
    `compareExchange` (ancienne valeur retournée, wrap à
    la largeur d'élément, kinds BigInt en opérandes
    `BigInt` stricts), `isLockFree` (1/2/4/8), `wait` →
    `TypeError` (spec navigateur main thread), `notify` →
    0, `waitAsync` → `{async:false, value:"not-equal"|
    "timed-out"}` ; `Uint8ClampedArray` et floats exclus
    (spec `ValidateIntegerTA`), hors-bornes → `RangeError` ;
  - test262 : 859/859 conservé ; 6 tests de régression
    (`c18_string_html_annex_b`, `c18_number_consts`,
    `c18_date_year`, `c18_aggregate_and_accessors`,
    `c18_shared_array_buffer`, `c18_atomics`).
- **JS-C17 — Float16, base64/hex, Annex B, `Iterator`,
  `crypto`, `DisposableStack`** (`dillo-js`) :
  - `Float16Array` : `TaKind::F16` bout en bout (ctor,
    indexation, `BYTES_PER_ELEMENT=2`, `@@toStringTag`) —
    codec IEEE-754 binary16 natif `f64_to_f16`/`f16_to_f64`
    (round-to-nearest-even, subnormaux, ±0/±∞/NaN) ;
    `Math.f16round` ; `DataView.prototype.getFloat16`/
    `setFloat16` endian-aware ;
  - `Uint8Array` ES2025 : statiques `fromBase64` (alphabets
    standard + urlsafe, whitespace ignoré) / `fromHex`
    (`SyntaxError` — longueur impaire ou caractère
    invalide) ; proto `toBase64`/`toHex`/
    `setFromBase64`/`setFromHex` → `{read, written}`
    (`TypeError` si débordement) ;
  - `Array.prototype.copyWithin` — sémantique memmove :
    snapshot de la plage source avant la pose (indices
    relatifs, clamp spec) ;
  - Annex B `Object.prototype` : accesseur `__proto__`
    (get/set sur objets ; get sur primitives via
    `read_prop` — les getters hérités sont désormais
    invoqués avec `this` = la primitive), `__defineGetter__`/
    `__defineSetter__`/`__lookupGetter__`/`__lookupSetter__`
    (énumérable+configurable, recherche dans la chaîne),
    `propertyIsEnumerable` ;
  - global `Iterator` : ctor abstrait (`new` → TypeError),
    `Iterator.prototype` = proto des itérateurs existants,
    `Iterator.from` (itérateur passé tel quel, itérable via
    `@@iterator`, `next` callable suffit) ;
  - `crypto` (WHATWG subset) : `getRandomValues` (quota
    65 536 octets) et `randomUUID` (v4 RFC 4122) — sur le
    PRNG xorshift64 du moteur, **pas cryptographique**
    (déviation documentée) ;
  - `DisposableStack` (ES2026) : `ObjKind::DispStack` —
    pile LIFO de disposeurs `(fn, this, arg)` ; `use`
    enregistre `v[@@dispose]` (invoqué `this = v`),
    `adopt`/`defer`, `dispose` (première erreur propagée,
    les suivantes enroulées en `SuppressedError`), `move`,
    accesseur `disposed`, `@@dispose`, `@@toStringTag` ;
    `SuppressedError` ctor (`error`/`suppressed`/`message`) ;
  - reportés : syntaxe `using`, `AsyncDisposableStack`,
    `Intl`, `Temporal`, `Atomics`/`SharedArrayBuffer` ;
  - test262 : 859/859 conservé ; 7 tests de régression
    (`c17_float16`, `c17_u8_base64_hex`, `c17_copy_within`,
    `c17_annex_b`, `c17_iterator_global`, `c17_crypto`,
    `c17_disposable_stack`).
- **JS-C16 — locale-fallbacks, `URL`, `URLSearchParams`**
  (`dillo-js`) :
  - `Array.prototype.reduceRight` ;
  - fallbacks locale (déviation documentée — pas de CLDR) :
    `localeCompare` (ordre point de code),
    `toLocaleLowerCase`/`toLocaleUpperCase`,
    `toLocaleString` sur `Array`/`Number`/`Date`/`String`
    (repli `join`/`toString`) ;
  - `URL` WHATWG subset : parsing absolu `scheme://authority
    /path?query#frag`, user/pass, ports par défaut éliminés
    (http:80, https:443, ws:80, wss:443, ftp:21), `origin`
    pour les schémas spéciaux, résolution relative
    (`//host`, `/path`, `?q`, `#f`, relatif dossier avec
    normalisation `.`/`..`), `file:` avec authority vide —
    pas d'IDNA ni de validation d'hôte stricte ;
  - `URLSearchParams` : `get`/`getAll`/`has`/`set`/`append`/
    `delete`/`sort`/`forEach`/`keys`/`values`/`entries`/
    `@@iterator`, encodage `x-www-form-urlencoded` (`+`/
    `%XX`), init string/record/itérable/copie ; `u.
    searchParams` peuplé depuis la query ;
  - test262 : 859/859 conservé ; 3 tests de régression
    (`c16_reduce_right_locale`, `c16_url`,
    `c16_url_search_params`).
- **JS-C15 — dispatch `@@*`, `Array.of`, `@@species`,
  `captureStackTrace`** (`dillo-js`) :
  - `str.match/matchAll/replace/replaceAll/search/split`
    dispatchent via `GetMethod(pat, @@x)` (spec) —
    `RegExp.prototype` porte `@@match`, `@@matchAll`,
    `@@replace`, `@@search`, `@@split` ; un objet custom
    `{[Symbol.replace](s,r){…}}` intercepte l'appel ; le
    check `replaceAll`/`matchAll` RegExp-sans-`g` reste
    `TypeError` avant le hook ;
  - `Array.of(…items)` — `this` ctor → `new this(len)`
    (sous-classes), sinon Array ordinaire ;
  - `@@species` (`ArraySpeciesCreate`) dans `map`/`filter`/
    `slice`/`concat` — `this.constructor[@@species]` définit
    le ctor du résultat ; `null`/`undefined` → Array
    ordinaire ;
  - `@@isConcatSpreadable` dans `concat` — la prop prime sur
    `Array.isArray`, spread via `length` + `Get` par indice
    (sous-classes/array-likes/proxies) ;
  - `Error.captureStackTrace(obj)` — extension V8 : own prop
    `stack` non-énumérable `Name: message` (déviation
    documentée : pas de frames) ;
  - test262 : 859/859 conservé ; 4 tests de régression
    (`c15_array_of`, `c15_sym_method_dispatch`,
    `c15_species_concat_spreadable`,
    `c15_capture_stack_trace`).
- **JS-C14 — `@@toStringTag`, chaîne générateur, reviver,
  `stack`, `normalize`** (`dillo-js`) :
  - `@@toStringTag` installé sur les prototypes concrets et
    les namespaces (`Map`/`Set`/`Promise`/`Date`/`Math`/
    `JSON`/`ArrayBuffer`/`DataView`/`globalThis`…), lu par
    `obj[Symbol.toStringTag]` et `Object.prototype.toString`
    — `[object Map]`/`[object Math]` conformes ; tag
    synthétique par kind sur les vues typées (`Uint8Array`,
    `BigInt64Array` — lecture d'indice non-numérique ne
    court-circuite plus `find_own`) ;
  - chaîne prototype générateurs conforme : ctors cachés
    `%GeneratorFunction%`/`%AsyncGeneratorFunction%` (pas de
    global, spec), `%GeneratorFunction.prototype%` →
    `%Generator%` → `%IteratorPrototype%` ; `function*` a
    pour proto `%GeneratorFunction.prototype%` et son
    `.prototype` hérite `%Generator%` →
    `getPrototypeOf(g()) === g.prototype`,
    `toString.call(g())` = `"[object Generator]"`,
    `constructor.name` = `"GeneratorFunction"` ;
  - `JSON.parse(text, reviver)` — `InternalizeJSONProperty`
    en post-ordre (enfants avant parents), `undefined` →
    suppression de la prop (arrays + objets) ;
  - `Error.stack` — own prop non-énumérable `Name: message`
    (déviation documentée : pas d'introspection de la pile
    d'appel) ;
  - `String.prototype.normalize` — NFC (défaut)/NFD/NFKC/
    NFKD via `unicode-normalization` (nouvelle dépendance :
    tables Unicode réelles, pur Rust ; forme invalide →
    `RangeError`) ;
  - test262 : 859/859 conservé ; 5 tests de régression
    (`c14_tostring_tags`, `c14_generator_function_chain`,
    `c14_json_reviver`, `c14_error_stack`,
    `c14_string_normalize`).
- **JS-C13 — Symboles réels, tagged templates, `WeakRef`**
  (`dillo-js`) :
  - clés-symboles uniques : `ObjKind::Symbol{desc,key}` porte
    sa clé-propriété (`\u{f130}sym{N}` anonyme, sentinelle
    `Symbol.<name>` pour les well-knowns) — corrige la
    collision `o[Symbol('x')] === o[Symbol('x')]` (deux
    symboles distincts = deux clés) ; `Symbol.*` sont de
    vraies valeurs symbole (`typeof Symbol.iterator` =
    `"symbol"`), `Symbol.dispose`/`asyncDispose` exposés ;
    `getOwnPropertySymbols` reverse-lookup `sym_index` avec
    vérification de vivacité post-GC ;
  - tagged templates `` tag`c${e}…` `` : le lexer garde les
    chunks `(cooked, raw)`, appel `tag(strings, …exprs)` avec
    `strings.raw` figée et `this` lié pour `` o.m`…` `` ;
    `?.` + template → `SyntaxError` (spec) ; `String.raw`
    implémenté (lit `.raw`/`length` — array-likes acceptés) ;
  - `fn.name` synthétique — `lit.name`, nom natif,
    `bound <cible>` pour `.bind()`, `Function.prototype.name
    === ""` ; une propre prop `name` (configurable) prime ;
  - `WeakRef`/`deref` et `FinalizationRegistry` —
    `register`/`unregister` : les cibles ne sont pas tracées
    par le GC (vraie faiblesse), `deref()` → `undefined`
    après collecte, cleanup `cb(held)` enfilé en microtask
    post-collecte ;
  - test262 : 859/859 conservé ; 4 tests de régression
    (`tagged_templates`, `symbol_keys_unique`,
    `function_names`, `weakref_finreg`).
- **JS-C11 — Syntaxe moderne** (`dillo-js`) :
  - séparateurs numériques `1_000` / `0xFF_FF` / `0b1010_10` /
    `0o7_7` / `1.5_5` / `1e1_0` / `1_000n` — validés « entre
    deux chiffres » (`1_`, `_1`, `1__0`, `1._5`, `0x_FF`
    rejetés) ;
  - statements labellés `id:` + `break`/`continue label` :
    `Flow::Break(Option<String>)`/`Continue(..)`, labels
    empilés via `pending_labels`, early-errors (label
    inconnu, `continue` vers non-boucle, label hors fonction) ;
  - champs de classe `x = e` / `x;` / `[k] = e` — l'init
    devient un thunk clos sur le scope de classe exécuté par
    `install_class_fields` : classe de base à l'entrée du
    `[[Construct]]`, dérivée juste après `super()`
    (`FLD_FIELDS`/`FLD_FIELDS_D` sur le ctor, ordre
    parent→dérivé respecté, `this` lié à l'instance) ;
  - champs statiques `static x = e` et blocs `static {}` —
    exécutés à la définition de classe avec `this` = ctor ;
  - noms privés `#x` : résolution lexicale au parse vers
    clés sentinelles `\u{f131}privN.x` (pile `priv_scopes`
    = `PrivateNameEnvironment` simulée, classes imbriquées
    voient l'englobante, `#x` non déclaré → `SyntaxError`),
    champs/méthodes/accesseurs privés, `#x in obj` ;
  - `C()` sans `new` → `TypeError` (marque `CLASS_CTOR`,
    `call_value` vs `call_value_ctor`) ; double `super()` →
    `TypeError` (marque `FLD_DONE` sur `this`) ;
  - clés internes `\u{f131}…` filtrées de
    `getOwnPropertyNames`/`getOwnPropertyDescriptors`/
    `Reflect.ownKeys` ;
  - test262 : 859/859 conservé ; 5 tests de régression.
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

