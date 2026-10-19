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
| 2026-10-19 | display list `explorer-render` | dirty rects rapportés par frame | NAV (étape 55) | proposé → **R-7** |
| 2026-10-19 | `explorer-render` | playback op-list canvas → `DisplayItem::Image` | NAV (JS-C28) | proposé → **R-11** |
| 2026-10-19 | domaine (boxtree/display list) | tolérance « document en construction » pour peinture progressive | NAV (étape 53) | livré → **R-1** (`TreeBuilder::push`/`finish`, `Dom::document_element`) |
| 2026-10-19 | `dillo-js` (interne NAV) | `MutationObserver` delivery microtask | NAV (étape 52) | proposé → JS-C26 |
| 2026-10-19 | `explorer-css` sélecteurs | pseudo-classes `:valid` / `:invalid` (`:user-invalid`) appuyées sur l'état de validation formulaires (hôte : `Dom::element` + règles §4.10.21.2) | NAV (étape 54) | proposé |
| 2026-10-19 | `explorer-render` raster | fenêtre de raster (`rasterize_window(list, y, h)` ou `DisplayList` crop) — aujourd'hui NAV plafonne la hauteur à 16 384/scale px en entrée : les documents plus hauts ne sont jamais rasterisables sous le cap, scroll impossible | NAV (terrain : github/MDN/stallman) | proposé |

## Journal

| Date | Contrat | Changement | Commit |
|---|---|---|---|
| 2026-10-19 | — | Création du journal (réorg deux équipes — découpage PIPELINE/NAVIGATEUR) | — |
| 2026-10-19 | entrée incrémentale `explorer-html` + `explorer-dom` | **Additif** : `TreeBuilder::push(&str)`/`finish()` (tokenizer interne), `Dom::document_element()`, `Tokenizer::is_finished()` — aucune casse de signature ; `parse` délègue au nouveau chemin | R-1 |
| 2026-10-19 | `explorer-render` display list + `explorer-layout` texte | **Additif** : `PaintImages` (replaced + backgrounds), `build_display_list_full`, `DisplayItem::BackgroundImage`, `text::is_css_ws`/`trim_css_ws` (`\u{A0}` non-collapsible) — aucune casse ; `rasterize` peint désormais la **canvas blanche** (fond de document) — sortie modifiée, tests du crate ajustés dans le commit | R-2 |
