# Changelog

Toutes les modifications notables de ce projet sont consignées ici.

Le format suit [Keep a Changelog](https://keepachangelog.com/fr/1.1.0/)
et le projet adhère au [Semantic Versioning](https://semver.org/lang/fr/).

## [Unreleased]

### Added

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

