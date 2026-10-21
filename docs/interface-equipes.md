# Interface entre équipes — journal des coutures

Journal des changements de **contrat partagé** entre l'équipe
PIPELINE (domaine pur) et l'équipe NAVIGATEUR (plateforme
produit + surface JS). Trois lignes par entrée : date, contrat
touché, nature du changement.

Règles :

- **PIPELINE → NAVIGATEUR** : les API publiques des crates
  domaine (`parse`, `StyleEngine`, `build_box_tree`, display
  list, hit-test…) sont le contrat. Extensions additives de
  préférence ; toute casse de signature existante est loguée ici
  **avant** le code.
- **Couplage comportemental** : le contrat inclut les sorties
  observables consommées par les tests downstream (rendu,
  hit-test, sérialisation). Un changement de sortie légitime
  est logué ici et la mise à jour des tests en aval se fait
  dans le même commit (carve-out AGENTS.md §10.3) — la casse
  détectée par `verify_all` reste à la charge de l'équipe
  causale.
- **NAVIGATEUR → PIPELINE** : une demande de capacité dans le
  domaine pur se note ici (ex. « dirty rects », « document en
  construction ») puis devient une étape R-xx.
- **`JsEngine` est interne à NAVIGATEUR** (trait + `DilloEngine`
  + `dillo-js` dans le même périmètre) — ses évolutions ne
  passent plus par ce journal ; elles sont dans
  `docs/roadmap-equipe-navigateur.md` piste JS.

## Contrats existants (état au 2026-10-19)

| Contrat | Producteur | Consommateur | État |
|---|---|---|---|
| `TreeBuilder::parse` / `push(&str)` | `explorer-html` (PIPE) | `explorer-core` | ✅ prêt, incrémental non exploité (NAV-53 ↔ R-1, tête de file PIPE) |
| `StyleEngine`/`font_faces` | `explorer-css` (PIPE) | `explorer-core` | ✅ |
| `FontProvider`/`FontSource` port | `explorer-layout` (PIPE) définit | impl `fontdb` dans `explorer-core` | ✅ ADR-0009 |
| `build_box_tree_*`/`build_display_list_*` | `explorer-layout`/`render` (PIPE) | `explorer-core` | ✅ variants sized/imaged |
| hit-test, path `(tag,index)` | `explorer-layout` (PIPE) | `explorer-core` dispatch | ✅ |
| `decode_image`/`decode_font` | `explorer-render` (PIPE) | `explorer-core` | ✅ |
| `HttpTransport` trait | `explorer-net` (NAV) | `explorer-core`, stubs test-support | ✅ interne NAV |
| `JsEngine`/`DilloEngine` | `explorer-js` (NAV) | `explorer-core` | ✅ interne NAV |
| `XhrTransport`/`Storage`/`CookieJar`/`ModuleLoader` | `dillo-js` (NAV) | `explorer-core` via traverse | surfaces prêtes — traverse = étape 51/JS-C27 |
| Fixtures Ahem/TTF, transports simulés | `explorer-test-support` (**partagé**) | tests PIPE (Ahem) + tests NAV (transports) | ✅ ajouts additifs libres, structure via ce journal |

## Extensions demandées (en attente)

| Date | Contrat | Extension | Demandeur | État |
|---|---|---|---|---|
| 2026-10-19 | display list `explorer-render` | dirty rects rapportés par frame | NAV (étape 55) | livré → **R-7** (`DisplayList::changed_rects`) |
| 2026-10-19 | `explorer-render` | playback op-list canvas → `DisplayItem::Image` | NAV (JS-C28) | livré → **R-11** — couture figée : `canvas::CanvasOp`/`CanvasTable` (`NodeId` → `Vec<CanvasOp>`), champ `PaintImages.canvases`, `<canvas>` remplacé intrinsèque 300×150 (attrs `width`/`height`) ; **consommé** — JS-C28 enregistre en `CanvasOpRecord` miroir → `canvas_ops()` → `CanvasTable` |
| 2026-10-19 | domaine (boxtree/display list) | tolérance « document en construction » pour peinture progressive | NAV (étape 53) | livré → **R-1** (`TreeBuilder::push`/`finish`, `Dom::document_element`) |
| 2026-10-19 | `dillo-js` (interne NAV) | `MutationObserver` delivery microtask | NAV (étape 52) | proposé → JS-C26 |
| 2026-10-19 | `explorer-css` sélecteurs | pseudo-classes `:valid` / `:invalid` (`:user-invalid`) appuyées sur l'état de validation formulaires (hôte : `Dom::element` + règles §4.10.21.2) | NAV (étape 54) | proposé |
| 2026-10-19 | `explorer-render` raster | fenêtre de raster (`rasterize_window(list, y, h)` ou `DisplayList` crop) — aujourd'hui NAV plafonne la hauteur à 16 384/scale px en entrée : les documents plus hauts ne sont jamais rasterisables sous le cap, scroll impossible | NAV (terrain : github/MDN/stallman) | proposé |
| 2026-10-19 | `explorer-layout` rects / hit-test | **bug** : `hit_test` n'entre dans un enfant que si le point est dans son rect — or des parents **non-bondants** existent (`<center>` prouvé : rect `[0 256 1024×192]` alors que son `<form>` vit à `y=76.8`) → tout le contenu peint devient incliquable. Bloque lite.ddg entier, zones de wikipedia et example.com (contenu injecté). Fix candidat : rect d'une boîte = union de ses enfants à la pose, ou hit-test sans élagage — arbitrer côté layout (perf). | NAV (QA mission 5, repro lite.ddg) | livré → **R-19** (`explorer_layout::hit_test`, élagage sur paint-bounds) |
| 2026-10-19 | `explorer-layout` boxtree/layout | **bug** : le texte d'un `<input>`/`<select>` ne peint jamais. Diagnostic (repro NAV `tests/session.rs::typed_text_repaints_new_value`) : `control_paint` synthétise la valeur en enfant `BoxKind::Text` ; mais le contrôle est `inline-block` (UA) → fragment atomique → la boîte est posée par `flow_children`/`layout_block`, où l'enfant `Text` (niveau inline, sans enveloppe `AnonymousBlock`) tombe dans le bras `is_inline_level → cursor += line_h; continue` — jamais mis en `lines`, donc aucun `DisplayItem::Text`. Fix probable : envelopper l'enfant `Text` du contrôle dans un `AnonymousBlock` (boxtree), ou faire que `layout_block` mette en ligne les enfants inline nus. | NAV (QA 52b, repro `repros/form.html`) | livré → **R-21** (`AnonymousBlock` autour du `Text` de contrôle) |
| 2026-10-19 | `explorer-render` dirty rects + raster | **perf (55g)** : deux goulots mesurés sur ~3 000 nœuds après les gardes NAV. (1) `merge_rects` est O(k²) en nombre de rects — un diff dégénéré (~8 500 rects) coûtait ~2 s avant de s'effondrer en « pleine frame » de toute façon ; NAV borne désormais en amont (`DIRTY_DIFF_MAX_ITEMS` = 2 000 items) mais le bon fix est une fusion linéaire (sweep/balayage) ou un seuil interne à `changed_rects`. (2) `paint_text` re-parse la fonte et re-trace chaque glyphe sans cache ni cull — ~6,4 s debug sur la fenêtre raster déjà cullée NAV ; un cache glyphe→outline par (fonte, taille) transformerait le profil. | NAV (étape 55g, repro mission 12) | proposé |
| 2026-10-19 | pipeline feuilles de style | **`@import` + `<link rel=stylesheet>` jamais fetchées** : `explorer-css` collecte les `ImportRule` mais le domaine n'a pas d'I/O — seul NAV peut résoudre les URL et injecter les feuilles dans `StyleEngine::add_css`. Sur lobste.rs (R-22), `application.css` importe `system-system.css` qui importe les sheets de couleurs : les `var(--color-*)` restent non résolues et tout retombe sur `currentColor`/noir. Besoin : après parse, boucle de résolution des `@import` (resp. média) + récupération `<link>` avant `compute_tree`. | PIPE → NAV (R-22, repro `repros/lobste`) | proposé |
| 2026-10-19 | `explorer-layout` relayout | **mesure (55g-suite, mission 13)** : le trou du clic-mutation ~3 000 nœuds est instrumenté — `dispatch_path_event` total ~18 s debug dont **`relayout_subtree` ~14 s** (un `appendChild` en tête de flux re-pose structurellement presque tout le `<body>` — le sous-arbre ≈ le document), cascade complète ~0,3 s, remap NAV mémoïsé (~3,4 s sortis), repaint global ~3,7 s (glyphes — cf. demande cache `paint_text` ci-dessus). Demande jointe : cascade **scopée** au sous-arbre sale (re-styler tout le document pour un append local est le deuxième poste) — `compute_styles` global + `relayout_subtree` restent les goulots du chemin incrémental R-12. **Suite mission 13** : le trou ~7 min = applies en série sous verrou (miroir re-salit à chaque battement sur page à `setInterval` mutateur) — NAV a livré la coalescence `sync_floor` (fenêtre `coût_apply.clamp(paint_interval, resync_gap_max)` — l'API respire entre deux peintures) + la cible tracing `sync_profile` pour attribution de phase. Le coût **unitaire** de l'apply reste PIPE — la coalescence borne la cadence, pas la fenêtre unitaire. | NAV (étape 55g-suite, repro `repros/heavy.html`) | proposé |
| 2026-11-07 | `explorer-render` display list / hit-test | **find-in-page (étape 57bis, ADR-0021)** : recherche textuelle dans le DOM → surlignage des occurrences + scroll-vers-occurrence. ~~Demande à détailler~~ **v1 livrée NAV-side** sur la surface existante (`LayoutBox.text`/`rect`/`node` + `FillRect` overlay — `session/find.rs`, `POST /api/tabs/{id}/find`). Demande **réduite** persistante : granularité caractère — une API de métriques de runs (sous-rect du match dans une boîte `Text`, p.ex. offsets glyphe → `Rect`) remplacerait le surlignage boîte-entière actuel. | NAV (étape 57bis) | v1 livré NAV — résidu : métriques de runs |
| 2026-10-20 | `explorer-layout` relayout + cascade | **mesure (mission 15, repro `storm.html` — `setInterval` mutateur)** : un apply `sync_dom` = **157,1 s** sous verrou, attribution `sync_profile` : `incr.remap_relayout` **150,3 s**, `incr.repaint` 6,7 s — vs ~14 s au profil test `profile_resync_phases` (écart ×10 : saleté accumulée absorbée d'un coup ? remap mémoïsé qui dégénère ? cascade globale ?). NAV traite l'**atomicité** de l'apply côté orchestration (étape 55h : checkpoints/budget) ; le **coût unitaire** reste PIPE — `relayout_subtree` dominant + cascade scopée au sous-arbre (demandes déjà consignées ci-dessus, ici avec l'écart terrain/test à expliquer). | NAV (étape 55h) | **consommé côté NAV** — atomicité traitée : apply en `spawn_blocking` sur `PaintPayload` détachée (`session/apply.rs`) ; la demande coût-unitaire reste ouverte pour PIPE |
| 2026-10-20 | `explorer-layout` flottants | **constat transitoire, résolu** : pendant le chantier float (`intrinsic.rs`/`block.rs`, livré R-28 `3cb6a98`), le test NAV `dom_sync_falls_back_when_floats_present` cassait — `hit_test` sur la boîte flottante ratait `(10,10)` → retombait sur la racine `<html>`. Repasse au vert avec la version commitée ; rien à faire côté NAV. | PIPE | résolu |
| 2026-11-07 | `explorer-layout` boxtree + `explorer-render` `collect_data_images` | **médias (étape 58, ADR-0023)** : v1 livré NAV — `video[poster]` réutilise le canal image (fetch+décodage+générique `images.replaced`), boîte de repli via feuille UA complémentaire `MEDIA_UA_CSS` en `Origin::UserAgent` (`video` 300×150, `audio` masqué sans `controls`). Refinements demandés, non bloquants : (1) `video`/`audio` en vrais remplacés `boxtree` — taille intrinsèque de l'affiche décodée (aujourd'hui la boîte est forcée 300×150 même si le poster fait 640×360 ; l'affiche est étirée, `object-fit` absent) ; (2) `video[poster]` couvert par `collect_data_images` (complément NAV `poster_data_images` à retirer alors) ; (3) présentational hints `width`/`height` des médias. | NAV (étape 58) | v1 livré NAV — résidu : remplacé boxtree + data-collector |

## Seam `MediaSurface` (étape 71 — ADR-0024)

**Statut : adopté** — session et REST implémentés à
l'étape 71b selon ce texte ; les énums et DTO ci-dessous
sont **figés champ par champ** ; les routes REST et le
vocabulaire JS restent **ajustables** au premier
consommateur réel (hôte `AbstractPlayerEngine`/`fvp`).

Principe (ADR-0024) : le moteur ne décode pas — il émet
l'**offre** d'une surface (élément `<video>`/`<audio>`
lisible, rect de la boîte remplacée, source résolue) ;
l'hôte qui veut jouer la **claim** puis pilote la
lecture. Le moteur peint toujours (poster, boîte UA) ;
l'hôte composite sa surface vidéo **au-dessus** du rect
déclaré. DRM (Widevine/FairPlay) et MSE hors scope v1.

### Directions des flux

- **Moteur → hôte (SSE)** : cycle de vie et géométrie
  (`MediaOffered`/`MediaMoved`/`MediaGone`) + commandes
  initiées par la page JS (`MediaCommand`).
- **Hôte → moteur (REST)** : `claim`/`release` et état de
  lecture (`state`) — la vérité de lecture vit chez
  l'hôte, le moteur la reflète.

### Identité et cycle de vie

- `media: u64` — alloué par le moteur à l'émission de
  l'offre, unique par onglet et par instance d'élément ;
  toute ré-offre (bfcache restauré, élément ré-inséré)
  produit un **nouvel** id — l'hôte re-claim.
- `MediaGone` sur : retrait DOM de l'élément, navigation,
  fermeture d'onglet. Un claim/state sur id mort →
  `410 Gone`.
- Claim : premier arrivé gagne — un second claim reçoit
  `409 Conflict`. Le claim déclare `codecs` (capacités
  hôte) : elles alimentent `canPlayType`/`error`.

### Événements SSE (moteur → hôte)

- `MediaOffered { tab: u64, media: u64, kind: "video"|"audio", rect: {x,y,w,h}, url: String, mime: Option<String>, controls: bool, autoplay: bool }`
  — `rect` en **px CSS document** ; l'hôte positionne
  son overlay à `rect.y − scroll_y` (voir transport
  `scroll_y` ci-dessous). `mime` = attribut `type`
  ou suffixe d'URL, `None` si inconnu ; `controls`
  indique que l'hôte doit afficher son UI native.
  **`audio` sans `controls`** : offre à `rect` 0×0 —
  rien à positionner, l'hôte n'attache que le pipeline
  de lecture (pas d'overlay).
- **`scroll_y` est transporté par `frame_invalidated`**
  (champ additif du DTO, émis à chaque repaint/scroll) —
  l'overlay recalcule sa position au même instant où la
  frame bouge : pas de course scroll/rect, pas de poll.
  `scroll_x` n'existe pas : le moteur ne scrolle que
  verticalement (champ additif futur si horizontal un
  jour). Côté FFI : `explorer_scroll_y(tab)`.
- `MediaMoved { tab: u64, media: u64, rect: {x,y,w,h} }`
  — relayout, scroll, resize. **Coalescence à la cadence
  de peinture** : au plus un `MediaMoved` par frame
  produite par onglet et par surface (le dernier rect
  connu gagne) — un scroll fluide n'émet pas un delta
  par événement d'entrée. Asymétrie voulue : le sens
  inverse (hôte → moteur `state`) est déjà borné ≤ 4 Hz.
- `MediaGone { tab: u64, media: u64 }` — fin de vie.
- `MediaCommand { tab: u64, media: u64, cmd }` — ordre
  initié par la page JS : `cmd` ∈ `play` | `pause` |
  `seek {ms: u64}` | `volume {v: f64}` |
  `muted {on: bool}` | `rate {r: f64}` — `muted` distinct
  de `volume=0` : le dé-mute restaure le volume courant
  (ajout 71c, pré-consommateur).

### REST (hôte → moteur)

- `POST /api/tabs/{tab}/media/{id}/claim`
  `{codecs: Vec<String>}` → `200` (claim acquis) |
  `409` (déjà claimé) | `410` (élément mort).
- `POST …/release` — l'hôte se détache ; la surface
  retombe sur les stubs ADR-0023.
- `POST …/state`
  `{position_ms, duration_ms, buffered_ms, state, error?}`
  avec `state` ∈ `"playing"|"paused"|"ended"|"error"` —
  rapport après chaque transition + battement
  `timeupdate` borné (≤ 4 Hz) ; le moteur le reflète
  dans les props JS et tire les events DOM.
- `GET /api/tabs/{id}/media` — liste des offres vivantes
  (debug, parité FFI).

### Vocabulaire JS (éléments média)

- **Claimé** : `play()` résout à la première frame hôte
  (premier `state:"playing"` rapporté), `pause()`
  résout au `paused` rapporté ; `currentTime` =
  `position_ms/1000`, `duration`, `buffered`
  (`TimeRanges` `[0, buffered]`), `paused`, `ended`,
  `volume`, `muted`, `playbackRate`, `readyState = 4`
  dès métadonnées connues ; `canPlayType` reflète les
  `codecs` du claim. Events DOM tirés : `play`,
  `playing`, `pause`, `seeking`, `seeked`,
  `timeupdate`, `durationchange`, `loadedmetadata`,
  `volumechange`, `ended`, `error`.
- **Non claimé** : stubs ADR-0023 **inchangés** —
  `play()`/`setSinkId` → `NotSupportedError`,
  `canPlayType` → `""`, props figées. **Clause
  honnêteté** : hôte absent, muet ou claim refusé =
  stub ; jamais de faux succès.

### Transport FFI (ADR-0018)

La seam ci-dessus est SSE+REST ; le mode in-process n'a
ni l'un ni l'autre — variante `explorer_media_*`
(polling ou callbacks) à spécifier au premier hôte FFI.
**Proposé**, non requis pour la v1 REST.

## Journal

| Date | Contrat | Changement | Commit |
|---|---|---|---|
| 2026-10-19 | — | Création du journal (réorg deux équipes — découpage PIPELINE/NAVIGATEUR) | — |
| 2026-10-19 | entrée incrémentale `explorer-html` + `explorer-dom` | **Additif** : `TreeBuilder::push(&str)`/`finish()` (tokenizer interne), `Dom::document_element()`, `Tokenizer::is_finished()` — aucune casse de signature ; `parse` délègue au nouveau chemin | R-1 |
| 2026-10-19 | `explorer-render` display list + `explorer-layout` texte | **Additif** : `PaintImages` (replaced + backgrounds), `build_display_list_full`, `DisplayItem::BackgroundImage`, `text::is_css_ws`/`trim_css_ws` (`\u{A0}` non-collapsible) — aucune casse ; `rasterize` peint désormais la **canvas blanche** (fond de document) — sortie modifiée, tests du crate ajustés dans le commit | R-2 |
| 2026-10-19 | `explorer-layout` hit-test | **Additif** : `explorer_layout::hit_test` remonte en PIPELINE (contrat déjà listé) — élagage sur `paint_bounds` (union du sous-arbre) au lieu du border-box ; `session.rs` délègue (carve-out : le consommateur était patché par le bug) | R-19 |
| 2026-10-19 | `<canvas>` 2D | **Additif** : `explorer_render::canvas` — `CanvasOp` (enum d'ops 2D : rects, chemins, fill/stroke/clip, transforms, alpha, lineWidth, save/restore), `CanvasTable` = `HashMap<NodeId, Vec<CanvasOp>>`, `raster_ops(ops,w,h) -> DecodedImage`, champ `PaintImages.canvases`, `<canvas>` traité comme remplacé (intrinsic attrs `width`/`height`, défaut 300×150, UA `display:inline-block`). NAV produit les ops (JS-C28) ; le playback reste côté PIPELINE | R-11 |
| 2026-10-19 | `explorer-layout` relayout | **Additif** : `relayout_subtree(root, dom, styles, sizes, target, ctx) -> Relayout` + champ `LayoutBox.slot: BoxSlot` (contexte de pose enregistré) — tiers `DirtyLayout` d'ADR-0012 côté domaine ; `Relayout::NeedsFullLayout` = repli `layout_tree` (flottants dans le doc, chemin absolu/fixe ou inline). Mutation DOM → NAV passe le `NodeId` sale | R-12 |
| 2026-10-19 | `explorer-css`/`explorer-layout` | **Additif** : `ComputedStyle` dérive `PartialEq`/`Eq` (comparaison style fragment/conteneur à la peinture) ; `LayoutBox::out_of_flow()` — position `absolute`/`fixed` lue sur le style calculé, restreinte aux boîtes d'élément (le style cloné d'un `Text`/`AnonymousBlock` n'est pas une position) ; `intrinsic::own_sides` → `pub(crate)` | R-22 |
| 2026-10-19 | `explorer-common` pile | **Additif** : `explorer_common::deep(f)` = `stacker::maybe_grow` (zone rouge 128 Kio / segment tas 1 Mio — patron JS-C41) — à appliquer sur toute arête récursive ∝ profondeur DOM (R-24 : ~300 `<div>` nichaient un stack overflow sur worker tokio). Toutes les traversées domaine PIPELINE sont couvertes + drop itératif de `LayoutBox`. NAV : si un marcheur récursif existe côté session (collecte DOM, sérialisation), utiliser `deep` — `explorer-common` est désormais une dep de dom/css/layout/render | R-24 |
| 2026-11-08 | seam `MediaSurface` (session/SSE/REST/JS) | **Contrat rédigé — proposé** : section dédiée ci-dessus (étape 71, ADR-0024). Directions des flux, identité `media: u64`, events `MediaOffered`/`MediaMoved`/`MediaGone`/`MediaCommand`, REST `claim`/`release`/`state`/`GET`, vocabulaire JS claimé vs stubs ADR-0023 (clause honnêteté préservée), variante FFI à spécifier au premier hôte. À l'adoption : énums/DTO figés champ par champ ; routes REST et vocabulaire JS ajustables au premier consommateur réel. Code en attente du verdict mission 21. | NAV (étape 71a) |
| 2026-11-08 | seam `MediaSurface` — implémentation session | **Implémenté (71b)** : détection DOM+boxtree (`audio` sans `controls` → offre 0×0), ids `media: u64`, retenue positionnelle sur apply, `MediaGone` à mort de page/onglet, `MediaMoved` au refresh post-repaint seul ; REST `GET media`, `claim`/`release`/`state` (409/410). `MediaCommand` figé dans le DTO, non encore émis (71c). Consommateurs `explorer-core` uniquement — aucune API PIPELINE touchée. | NAV (étape 71b) |
| 2026-11-08 | seam `MediaSurface` — délégation JS | **Implémenté (71c)** : `claim`/`release`/`state` poussés au miroir JS via `JsEngine::set_media_claim`/`deliver_media_state` ; ordres page (`play`/`pause`/`seek`/`volume`/`muted`/`rate`) drainés → `CoreEvent::MediaCommand` SSE. Identité : `media: u64` session transmis au claim, rappelé dans les commandes (robuste aux re-paths). `play()` résout au premier `playing`, rejette `error`/release ; props `HTMLMediaElement` refletées + events DOM non-bubblants. **Ajustement contractuel v1** (clause « vocabulaire au premier consommateur ») : variante `MediaCmd::Muted{on}` ajoutée — distincte de `volume=0` (le dé-mute restaure le volume courant) ; `Volume`/`Rate` passés en `f64` côté core (alignement JS natif). Non claimée = stubs ADR-0023 inchangés. Consommateurs NAV uniquement (`explorer-jscore`/`explorer-js`/`explorer-core`). | NAV (étape 71c) |
| 2026-11-08 | **Process — isolation des index** | **Worktree par équipe** (deux incidents en deux jours : staged-but-uncommitted capturé par le commit concurrent — CHANGELOG de NAV dans `4cfd052` puis étape 78 dans `dc3e627`) : chaque équipe travaille dans son propre arbre avec son propre index — NAV : `D:\explorer-nav` sur `nav/main`, PIPE : `C:\OnionBit-Explorer` sur `main`. Atterrissage NAV : `git rebase main` dans le worktree NAV, puis `git -C C:/OnionBit-Explorer merge --ff-only nav/main` (refus net si le worktree main est sale). Filet de secours : `git commit -o -- <paths>` (jamais d'index tenu entre deux commandes). `CARGO_TARGET_DIR` distinct par worktree (ex. `D:\cargo-target\explorer-nav`) — sinon les builds sérialisent sur le lock du target-dir et les `.exe` en cours se verrouillent mutuellement. | NAV (adopté lead) |
| 2026-11-08 | `explorer-render` R-37 — dette d'heuristique | **Observation pour PIPE** (à ranger dans la roadmap R-37 restante) : `DAMAGE_INFLATE_CSS = 2.0` est une constante jamais validée par mutation terrain — les 9 tests d'équivalence comparent raster régional vs plein cadre sur les **mêmes** `changed_rects`, donc aucun n'exerce un overhang > 2 px (italique fort, `outline` dépassant le border-box, `box-shadow`/`text-shadow` à blur étalé, caret). Un dépassement se manifesterait en **relique de pixels** au bord d'une fenêtre — défaut silencieux, pas un échec de test. Suggestion : fixture terrain avec glyphe italique à fort débord + outline 4 px, ou inflation dérivée des métriques d'item (`paint_bounds`) plutôt que constante. | NAV (relecture) |
| 2026-11-08 | seam `MediaSurface` — transport `scroll_y` | **Additif (71d)** : `frame_invalidated` gagne `scroll_y: f32` (px CSS de la frame peinte) — trou contractuel colmaté : l'offre promettait « l'hôte soustrait `scroll_y` » mais rien ne le transportait à jour (scroll moteur par ancre/`scrollTo`/clavier = repaint sans `POST /input` → overlay dérivant). Champ co-émis avec la frame → recalcul atomique, pas de course scroll/rect. `scroll_x` absent volontairement : pas de scroll horizontal dans le moteur — sera additif le jour où il existera. FFI : `explorer_scroll_y(tab)` déjà exposé. Consommateurs NAV uniquement. | NAV (étape 71d) |
| 2026-11-08 | seam `MediaSurface` — premier consommateur | **Implémenté (71d)** : `app/` consomme le contrat de bout en bout — `media_offered` → `POST …/claim` (`AppConfig.mediaCodecs`, containers MDK) → `MediaPlayer` natif (`video_player` + `fvp`, backend MDK/FFmpeg en linking dynamique — licence LGPL vérifiée) ; `media_command` routé (`play`/`pause`/`seek`/`volume`/`muted`/`rate`), `POST …/state` borné ≤ 4 Hz (`AppConfig.mediaReportInterval`, saut de cadence au changement d'état). Overlay `Positioned` à `rect − scroll_y` × zoom ; audio 0×0 joue sans overlay ; `controls` → tap = play/pause (UI hôte minimale v1). Cycle de vie : `media_gone`/`tab_closed`/`navigated` → `dispose` ; réconciliation `GET …/media` au bootstrap (offres pré-reconnexion SSE). **FFI** : `listMedia` → `[]`, `claim`/`release`/`state` → `UnsupportedError` — la variante in-process reste à spécifier au premier hôte FFI (les events média ne traversent pas l'ABI aujourd'hui). Consommateurs `app/` uniquement — aucune API moteur touchée. | NAV (étape 71d) |
| 2026-10-20 | `explorer-render`/`explorer-layout` — build incrémental | **Additif + casse interne** : `build_display_list_incremental(bx,w,h,scroll,images,prev,tree_epoch)` ; `DisplayList` gagne `spans: SpanMap` (index chemin-d'enfants → `SpanRec`) + `tree_epoch` ; exports `SpanMap`/`SpanRec`. **Casses** (aucun consommateur externe trouvé — `grep` workspace) : `FontCache::shape` → `Option<Arc<ShapedRun>>`, `Fragment::shaped` et `DisplayItem::Text.run` → `Arc<ShapedRun>` (runs partagés — la même pièce = même allocation). **Contrat `tree_epoch`** : la session ne bump l'epoch qu'au remplacement de `p.tree` — inchangée ⇒ même objet ⇒ le fast-path pointeur est valide ; les mutations in-place autorisées = `{rect, node (remap), lines (relay), slot, géométrie de fragments (translate_y)}` — asserté en `debug_assertions` côté render. `repaint_payload` stocke désormais la liste **non cullée** (spans préservées) ; la cullée n'est qu'une copie jetable du plein cadre — comportement observable : `frame_invalidated` peut rapporter des régions hors viewport (diff plus précis). Carve-out : `explorer-core` câblé dans le même commit (consommateur de la nouvelle signature). | PIPE (R-37) |
| 2026-10-20 | médias `<video>`/`<audio>` — remplacés natifs | **Additif + retrait interne (prévu par la roadmap R-34)** : `collect_data_images` couvre désormais `<video poster="data:…">` (`PaintImages.replaced` clé sur le nœud `video` — même contrat qu'`<img>` : sizing intrinsèque + peinture). `<video>`/`<audio>` deviennent feuilles remplacées natives du boxtree (`intrinsic` = affiche décodée, sinon 300×150 / 300×54 — boîte **atomique**, jamais d'enfants). `object-fit`/`object-position` lus à la peinture (`fill` inchangé par défaut ; `cover`/`none` débordants → `PushClip` sur la content box). UA_CSS : `video` sort de la liste `display:block` (redevient inline remplacé), `audio:not([controls])` = `display:none` — sorties modifiées, tests session ajustés dans le commit (carve-out : fixtures `width`/`height` présentationnels). **Retraits NAV** (internes, `pub(crate)` — aucune API publique) : `poster_data_images` (couvert par le collecteur PIPE) ; `MEDIA_UA_CSS` réduit au « look produit » — fond noir vidéo, chrome `audio[controls]` (les règles de taille/display sont désormais domaine : remplacé natif + UA_CSS). | PIPE (R-34) |
| 2026-10-21 | `explorer-layout` métriques de runs | **Additif** : `explorer_layout::fragment_sub_rect(frag, start, end) -> Option<Rect>` — sous-rect d'une tranche d'octets d'un `Fragment` posé (union des cellules des glyphes dont le `cluster` tombe dans la tranche, ordre visuel → LTR/RTL, ligatures entières ; bande = hauteur de ligne ; repli proportionnel sans `shaped`, `None` sur tranche vide/atome). Consommé par `session/find.rs` : le surlignage find est désormais au glyphe près — occurrences groupées (`Vec<Vec<Rect>>` interne NAV, aucune API), recherche sur le texte affiché concaténé par boîte (matches trans-fragments/trans-lignes), repli boîte-entière pour les `Text` hors lignes. Aucune casse de signature. | PIPE (R-35) |
| 2026-10-21 | `explorer-render`/`explorer-layout` — cache glyphes cross-passe | **Additif + nouvelle dep** : `explorer_render::GlyphCache` (faces `OwnedFace` + contours upem persistés entre repaints, `Send`), `rasterize_cached`, `rasterize_dirty_cached` — signatures historiques inchangées (délèguent à cache jetable). `FontSource::revision() -> u64` = méthode de trait **additive à défaut `0`** (source figée) ; `WebFontSource` la bumpe à `register`/`clear` — les caches sync-avant-passe vident tout sur révision changée (miss figés sinon éternels). `SharedFonts` (core) délègue sous verrou. `PaintPayload.font_cache` = interne NAV. **Dep** : `owned_ttf_parser 0.25` (wrapper officiel de `ttf-parser`, pur Rust — `Face` possédée, `unsafe` interne au crate permis). Mesure consignée : ~0 % (fill tiny-skia domine) — atlas bitmap = levier suivant, non fait ici. | PIPE (R-36) |
| 2026-10-21 | `explorer-render` — encodage PNG streamé | **Additif + nouvelle dep** : `Frame::encode_png_to<W: Write>(&self, w) -> bool` — encodage PNG par bandes de 256 lignes sur `png::StreamWriter` (scratch démultiplié borné, `IDAT` par chunks ~256 Kio, fini le clone RGBA monolithique) ; `encode_png()` inchangé en signature (délègue à `encode_png_to`). NAV : un endpoint `shot`/SSE peut désormais streamer la réponse sans matérialiser le `Vec`. **Dep** : `png 0.18` en direct (déjà tirée par `image` — même version, backend fdeflate identique à `CompressionType::Fast`). | PIPE (R-37) |
| 2026-10-21 | `explorer-layout`/`explorer-render` — bounds d'encre réelles | **Additif (champ pub sur struct pub)** : `ShapedRun.ink: Option<Rect>` — union des `glyph_bounding_box` positionnées, px CSS relatifs à l'origine du run (baseline=0) ; `item_bounds` des items `Text` l'utilise au lieu de la boîte de métriques → les dirty rects couvrent l'overhang glyphe sans inflation. `FontCache::shape`/`shape()` signatures inchangées ; `ShapedRun` gagne un champ → construction littérale cassée le cas échéant (aucun site hors `shape()` dans le workspace). `explorer_test_support::fixture_ttf_overhang` = fixture **additive** partagée (encre débordant 0.10/0.45/0.25em mesuré). NAV : `changed_rects` rapporte désormais des régions ajustées à l'encre (plus précises — un SSE consumer qui se fiait à la boîte métrique voit des fenêtres texte plus justes). | PIPE (R-37) |
| 2026-11-08 | `explorer-dom`/`explorer-css` — mode quirks → cascade | **Additif (champs et accesseurs pub, aucune casse)** : `explorer_dom::DocumentMode` (`NoQuirks`/`LimitedQuirks`/`Quirks`, `Default=NoQuirks`) ; `Dom::document_mode()` / `Dom::set_document_mode()` — le tree-builder le synchronise dès le DOCTYPE tranché (`TreeBuilder::parse`/`into_dom`/`push` intermédiaires), la cascade le lit via `dom` (zéro signature modifiée). `StyleEngine::new` parse désormais une feuille UA **quirks-only** (`QUIRKS_UA_CSS`, interne) : en quirks, `table` réinitialise les propriétés texte héritées (`text-align`→`start`, `font-*`→initiales) — `<center>` ne déteint plus dans les cellules (terrain : news.ycombinator.com). Comportement observable NAV : un document sans DOCTYPE peut produire des computed styles et des boîtes différents d'avant (texte cellules à gauche). | PIPE (R-38) |
| 2026-11-08 | `explorer-css` — `em`/`rem` en media queries | **Correctif comportemental, aucune API touchée** : `condition_matches` résout désormais `px`/`em`/`rem` contre `env.font_size_base` (font-size initiale, spec MQ4) — avant, toute feature non-px (`(min-width: 30em)`, `64rem`…) évaluait faux. Observable NAV : les feuilles derrière des seuils em/rem s'appliquent enfin — computed styles et boîtes changent sur les sites responsive (terrain : rust-lang.org/Tachyons, CSS desktop entièrement réactivé). | PIPE (R-39) |
| 2026-11-08 | `explorer-layout` — ratio intrinsèque des remplacés | **Correctif comportemental, aucune API touchée** : §10.6.2 — l'axe `auto` d'un élément remplacé dérive désormais de l'axe utilisé via le ratio intrinsèque (`width:auto`+`height` → `w = h × iw/ih` ; `height:auto` → `h = w_utilisée × ih/iw`, calculée **après** le bridage `min|max-width`). Avant : `h` restait à l'intrinsèque même quand `w` était bridée → tout `img{max-width:100%}` contraint s'étirait verticalement (terrain : icônes sociales rust-lang.org en bande blanche ~70×511). Observable NAV : les boîtes remplacées bridées changent de taille (plus petites, proportionnées) — appliqué à `img`/`video`/atomiques inline uniformément. | PIPE (R-40) |
| 2026-10-20 | `explorer-layout` — flex colonne en hauteur indéfinie | **Correctif comportemental, aucune API touchée** : deux trous spec comblés dans la branche `column` du flex — (1) base `flex-basis`/`height` en `%` contre une taille main indéfinie résout `content` (§9.2, avant : `0`) ; (2) espace libre indéfini + `flex-grow` → l'item prend son max-content mesuré (§9.9.2, avant : gardait sa base — un `flex:1`/`basis:0` s'effondrait à 0 et l'item suivant le **recouvrait**). Observable NAV : les conteneurs `display:flex;flex-direction:column` sans `height` (`body{min-height:100vh}` sticky-footer classique) posent leurs items empilés sans chevauchement — terrain : pied de rust-lang.org peint sur le héros, corrigé. | PIPE (R-41) |
| 2026-10-20 | `explorer-layout`/`explorer-css`/`explorer-core` — unités viewport | **Additif + carve-out aval** : `BlockCtx.viewport_w: f32` (champ pub additif, défaut 800) — `vw`/`vh`/`vmin`/`vmax` désormais résolues dans `raw_length_px`/`eval_calc` (propriétés de boîte), dans `condition_matches` des media queries (MQ4 §6.4) et dans `font-size` (cascade, contre `env.width_px`/`height_px`). Avant : parsées puis `px_factor`→`None` → ignorées partout. **Carve-out `explorer-core` (même commit)** : les deux `BlockCtx` de `apply.rs` reçoivent `viewport_w`/`viewport_h` depuis `ctx.viewport` — `viewport_h` restait au défaut 600 ; `position:fixed`/ICB y gagnent aussi en justesse. Observable : tailles/positions changent sur les pages utilisant des unités viewport (sticky footers `min-height:100vh`, héros `100vh`, modales `90vw`). | PIPE (R-42) |
| 2026-10-20 | `explorer-jscore` — entités non décodées à l'injection (triage M-H1) | **Verdict triage PIPE → item NAV (JS-Cxx), pas R-xx** : `&nbsp;`/`&copy;` littéraux sur google.fr — la chaîne est entièrement côté JS mirror : `Dom::parse_fragment_into` (`crates/explorer-jscore/src/dom.rs` ~l.1394) accumule le texte courant **brut** — `decode_entities` n'est appelée que sur les valeurs d'attribut (~l.1508) et le texte RCDATA `title`/`textarea` (~l.1567). Tout markup injecté (`innerHTML`, `document.write`, parse initial du miroir) conserve `&nbsp;` littéral → sérialisation sync `escape_text` → `&amp;nbsp;` → re-parse Dom réel → **affiche l'entité en clair** (toute page scriotée corrompt en gros). Bonus : `textContent` JS lit aussi le littéral (corruption côté scripts). Repro prouvée : `innerHTML='a&nbsp;b&copy;'` → `textContent`=`"a&nbsp;b&copy;"`, resérialise `"x&amp;nbsp;y"`. **Fix suggéré** : `decode_entities(&text)` dans `flush_text!` + élargir la mini-table (`&copy;` absent — top ~30 entités du web réel, ou table complète si jscore peut l'embarquer sans casser l'autonomie). | PIPE → NAV |
| 2026-11-08 | `explorer-layout` — `box-sizing` | **Correctif comportemental, aucune API touchée** : `box-sizing:border-box` (sélecteur universel quasi-systématique du web réel) est désormais honoré — `width`/`height`/`min-*`/`max-*` spécifiées s'appliquent à la border box, converties en contenu (châssis padding+bordure retranché, plancher 0) dans `layout_block`, `abs_geo`, bases `item_flex`, mesure de piste grid `auto` et `content_px`. Avant : tout était lu content-box → toute boîte `border-box` avec padding/bordure rendait trop grande (terrain : bouton Connexion google.fr ~60 px au lieu de 40). Observable NAV : les boîtes `border-box` rétrécissent à leur taille réelle — layouts de pages modernes affectés positivement. Tables inchangées (`width` y est déjà border-box, §17.5.2). | PIPE (R-43) |
| 2026-11-08 | `explorer-cli` — `shot` viewport optionnel | **Additif, outillage** : `shot <url> <out.png> [LxH]` — `session.set_viewport(w,h,1.0)` avant navigation (défaut inchangé 1024×768). Fait par PIPE pour le triage M-H1 (comparaisons 800/1024/1440) — fichier périmètre NAV, changement strictement additif ; NAV peut reprendre/polir librement. | PIPE (triage M-H1) |
| 2026-11-08 | `explorer-layout` — `width` remplacé vs mesure intrinsèque | **Correctif comportemental, aucune API touchée** : la mesure min/max-content honorait l'intrinsèque avant le `width` spécifié — un `img{width:40px}` contribuait 512. §5.1 : la contribution d'un remplacé = sa taille utilisée. Désormais `width` non-% d'abord, intrinsèque ensuite, `min|max-width` non-% bornant les deux. Observable NAV : les conteneurs shrink-to-fit/flex/grid de remplacés à `width` spécifiée se mesurent plus petits — corrige les icônes empilées une-par-ligne (terrain : pied de rust-lang.org). | PIPE (R-44) |
| 2026-11-08 | `explorer-layout` — `float:right` translate le sous-arbre | **Correctif comportemental, aucune API touchée** : `place_float` corrigeait `child.rect.x` après le layout interne sans translater les descendants — la border box du flottant partait au bord droit, son contenu restait posé au bord gauche de la bande (vignettes Wikipedia ITN/OTD peintes à gauche, texte par-dessus). Désormais `translate_subtree(child, delta, 0)` — même motif que l'ancrage `right`/`bottom` des absolus. Observable NAV : les sous-arbres des flottants `right` corrigés horizontalement sont à leur vraie position ; les régions de dégât incrémental suivent (l'ancienne position devenait un trou de repeinture). | PIPE (R-45) |
| 2026-10-20 | `explorer-layout` — item flex/grid = FC indépendant | **Additif (champ pub) + correctif comportemental** : `LayoutBox.fc_root` (défaut `false`, posé par `layout_item`) — Flexbox §4.1 : un item établit un contexte de formatage indépendant. Avant : les marges des descendants traversaient l'item et s'évaporaient (un `<p>` en tête d'item perdait sa marge ; mesure flex trop petite). Désormais marges contenues + flottants englobés (`is_fc_root` = `establishes_bfc || fc_root` aux gardes §8.3.1/§9.4.1). Observable NAV : items contenant des enfants marginés grandissent d'autant — layouts flex de sites réels (sections à `<p>` marginés) reprennent leur espacement. Note terrain : `&mdash;` littéral observé sur rust-lang.org via le chemin JS — même cause que l'item NAV `parse_fragment_into` consigné au-dessus (le parse miroir initial laisse aussi le texte brut — rayon de blast = toute page scriotée), aucun trou PIPE côté `explorer-html` (table WHATWG complète, `dump` décode). | PIPE (R-46) |
| 2026-10-21 | `explorer-jscore` — entités (réponse NAV) | **Résolu côté NAV (JS-C46)** : `decode_entities` appelé au `flush_text!` + table élargie au jeu HTML 4.01 (~200 noms) + `&#x…;` + remap 1252 — repro `innerHTML='a&nbsp;b&copy;'` → `textContent` correct. **Demande résiduelle ouverte** : exposer `explorer_html::entities` (`lookup_named` pub ou un `decode_str`) ferait bénéficier le miroir de la table WHATWG complète + règles legacy sans `;`/esperluette ambiguë — non bloquant, le jeu courant couvre le web réel. | NAV (JS-C46) |
| 2026-10-21 | `explorer-css`+`explorer-layout` — `colspan`/`rowspan` | **Correctif comportemental, aucune API touchée** : les attributs `colspan`/`rowspan` arrivent au layout table via des clés internes `-x-colspan`/`-x-rowspan` posées en hints `td`/`th` (même couture que `cellpadding`/`align` — pas de nouvelle API). Grille à occupation : les `rowspan` retiennent leurs colonnes sur les rangs suivants ; mesure des spans répartie en déficit égal (§17.5.2.2) ; boîte peinte étendue sur les rangs traversés. Observable NAV : les tables réelles à `colspan`/`rowspan` (HN `itemlist`, infoboxes) s'alignent correctement — la 2ᵉ rangée HN pose `subtext` sous le titre. Limite : la hauteur d'un `rowspan` reste dans son rang d'origine. | PIPE (R-47) |
| 2026-10-21 | `explorer-jscore` — rapatriement `dillo-js` | **Rapatriement complet de la divergence vendored Dillo (JS-C48)** : la copie `dillo-js` avait dérivé ~1 200 lignes bidirectionnelles — tout ce qui existait seulement côté Dillo est désormais dans le canonique (`window` EventTarget/`getComputedStyle`/`matchMedia`/`CSS`/`Node`, `document.fonts`/`defaultView`/`hasFocus`/dims honnêtes, `sendBeacon`, `pushState`/`replaceState`, `on*`=false→`preventDefault`, `AbortController`/`AbortSignal`+rejet fetch, TypeError « `x` is not a function (on recv) », regexp lazy `*?`/bornes échappées/surrogates clampés, `performance` complet en stubs honnêtes, re-baseline test262 1007/1046=96 %, outil `census.rs`). Sens inverse vérifié : JS-C45/46 et JS-C26+ étaient déjà Explorer-only — rien perdu. **Signal aux équipes** : la copie vendored Dillo est remplacée par une dépendance `path` vers ce crate — tout futur travail JS se fait ici (piste JS-Cxx, spécialiste NAV). | NAV (JS-C48) |
| 2026-10-21 | `explorer-jscore` — `error.stack` = pile d'appels | **Correctif comportemental, aucune API touchée** : `Engine.call_stack` (Rc<FnLit> par appel script, natives exclues) + `FnLit.pos` (ligne/col du jeton ouvrant, 10 sites parseur). `error.stack` rend le format V8 `at f (l:c)` borné à 10 frames ; `describe` le préfère → `JsError::Uncaught`/`PipelineError` deviennent **multi-lignes** (en-tête + frames). Observable pour tout consommateur de logs d'exception JS (session, app, triage PIPE) : `Error: b` cite désormais la fonction fautive et son site de définition. Dillo : surface `stack` enrichie, aucune casse. | NAV (JS-C47) |
| 2026-10-21 | `explorer-jscore` — `navigator.languages` absent (triage résidu example.com) | **Verdict triage PIPE → demande NAV (JS-Cxx)** : le `<svg>` injecté par `/s.js` n'est jamais peint non pas par la chaîne miroir/re-parse mais parce que **le script meurt avant l'injection** — `navigator.languages.some(...)` → `undefined.some` → TypeError, tuant le script entre le premier `insertAdjacentHTML("afterend", <p>…)` (paragraphes visibles ✓) et le second (`"afterbegin"`, style+logo — jamais inséré). Vérifications bout-en-bout : la sérialisation miroir du markup exact (`<svg viewbox=…><use href="#q">`…) est saine, le re-parse `explorer-html` ajuste `viewbox`→`viewBox`, `collect_svg` rasterise (usvg accepte `use href`), boxtree+display list émettent l'`Image` — **versant PIPE intégralement exonéré**. **Fix suggéré** : `navigator.languages` = tableau gelé `["en-US","en"]` (dérivé de `language` existant, low-fingerprint) — une ligne côté `install_dom`, restaure l'injection `afterbegin` entière. | PIPE → NAV |
| 2026-10-21 | `explorer-jscore`/`explorer-js` — import maps minimales | **Additif, contrat `JsEngine` étendu sans casse** : `JsEngine::set_import_map(json)` (défaut noop — NAV-interne) + `Engine::set_import_map` côté jscore (JS-C44). La session doit pousser le contenu des `<script type="importmap">` (dans l'ordre du document, avant le premier `eval_module`) pour que les bare specifiers (`import 'react'`) résolvent — `load_source` réécrit exact + préfixe `/` le plus long avant le `ModuleLoader`. Sans push amont : comportement inchangé (erreur typée `cannot load module`). `census` rejoue déjà les importmaps. | NAV (JS-C44) |
| 2026-10-21 | `explorer-layout` — piste `auto` multi-items (R-48) | **Correctif comportemental, aucune API touchée** : `col_widths` regroupe désormais les bases par **colonne** (max des items de la piste) avant de distribuer le libre — plusieurs items en `auto-flow: row` partageant une piste `auto` ne divisent plus l'espace entre eux (cartes BBC : 180px/3 enfants → 60px devenu 180px). Observable NAV : toute grille sans template (`grid-template-columns` absent → colonne implicite) à plusieurs enfants reprend la largeur conteneur. | PIPE (R-48) |
| 2026-10-21 | `explorer-core` — `StyleEnv::default()` pour media queries | **Constat terrain PIPE → demande NAV** : `styles_payload` (`session/apply.rs:399`) évalue les media queries avec `StyleEnv::default()` = **1024×768 fixe**, quel que soit le viewport session — à 800px, `max-width:1007` est faux alors qu'il devrait être vrai (BBC `grid-column:1/span 8` perdu → `span 6` de base). Idem `cli layout` (`main.rs:186`, env 1024 vs layout 800). **Fix suggéré** : construire le `StyleEnv` depuis le viewport réel de la session/payload — additif, aucun contrat touché ; la cascade lit déjà un `&StyleEnv` paramètre. | PIPE → NAV |
| 2026-10-21 | `explorer-render` — `visibility:hidden`/`collapse` (R-49) | **Correctif comportemental, aucune API touchée** : les boîtes `visibility:hidden`/`collapse` n'émettent plus d'items (`paint_self`/`paint_replaced` sautés, descente conservée pour les descendants `visible` explicites — §11.2). Observable NAV : les motifs « visually hidden » (dropdowns, sr-only, accordéons) disparaissent de la display list — BBC `LanguageDropdown` ne se peint plus. **Limite connue** : le hit-test ne filtre pas `visibility` — un élément caché reste ciblable par clic (inerte si `pointer-events:none`). | PIPE (R-49) |
| 2026-10-21 | `explorer-jscore` — `navigator.languages` (réponse NAV) | **Résolu côté NAV (JS-C51)** : tableau gelé `["en-US","en"]` dérivé de `language` installé dans `install_dom` — `languages.some(...)` ne jette plus, `/s.js` example.com atteint son second `insertAdjacentHTML` → le `<svg>` logo entre dans le miroir (chaîne PIPE aval inchangée). Test : `navigator_languages_frozen_array` (valeur, gel, identité). | NAV (JS-C51) |
