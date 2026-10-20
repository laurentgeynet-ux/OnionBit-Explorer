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
