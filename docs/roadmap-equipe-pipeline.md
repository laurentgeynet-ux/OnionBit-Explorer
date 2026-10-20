# Roadmap — Équipe PIPELINE

**Périmètre :** les crates du **domaine pur** — `explorer-common`,
`explorer-dom`, `explorer-html`, `explorer-css`, `explorer-layout`,
`explorer-render`. Zéro I/O, zéro async, zéro réseau : tout ce qui
transforme des octets en pixels.
**Ne touche pas :** `explorer-net`, `explorer-core`, `explorer-js`,
`explorer-jscore`, `api`/`daemon`/`cli`/`ffi`, `app/`, `scripts/`, `fuzz/`.
**Partagé :** `explorer-test-support` (fixtures/stubs additifs
libres, structure via le journal — AGENTS.md §10).

**Numérotation :** série propre — **R-1+**. Ordre réel : **la
file suit le déblocage** — un marqueur ↔ dont l'étape
consommatrice approche fait remonter l'étape fournisseur ; les
activateurs cross-équipe sont en tête par construction (R-1
débloque NAV-53, R-7 débloque NAV-55).

**Definition of done** (inchangée, AGENTS.md §5) : implémenté +
testé + CHANGELOG + case cochée ici + `verify_all` vert + commit
dédié `feat(<crate>): …`.

Légende : `[ ]` à faire · `[~]` en cours · `[x]` terminée.
Les besoins qui sortent du périmètre passent par
`docs/interface-equipes.md` — marqués **↔ NAV**.

## Correspondance avec l'ancienne numérotation

| Ancien plan (étape moteur) | Ici |
|---|---|
| 53 (moitié domaine — document en construction) | **R-1** |
| 54 (WPT floats/block-formatting) | **R-2** |
| 56 (tailles intrinsèques) | **R-3** |
| 57 (flex-wrap) | **R-4** |
| 58 (grid) | **R-5** |
| 59 (sticky tous côtés) | **R-6** |
| 63a (dirty rects produits) | **R-7** |
| 60a (foreign-content tokenizer) | **R-8** |
| 60b (`<svg>` inline) | **R-9** |
| 60c (`@keyframes`) | **R-10** |
| 62 (moitié raster — canvas op-list) | **R-11** |
| — (layout incrémental) | **R-12** |
| 61 (Acid2 + corpus) | **R-13** |
| — (triage échecs html5lib) | **R-14** |

---

## Palier R-A — Déblocage cross-équipe + conformité flux

Objectif : débloquer la session incrémentale NAVIGATEUR, puis
rendre le flux block/inline/float conforme au web réel.
Sortie de palier : **WPT reftests ≥ 35/40**.

- [x] **R-1. Peinture progressive — côté domaine** (ex-53).
  Le boxtree/display list tolère un document **en cours de
  construction** (tokenizer `push()` alimenté par chunks) —
  une frame = un état cohérent préservé ; la cadence relève de
  la session. **Débloque NAV étape 53** (palier N-A) — tenu en
  tête de file pour ça. Livré :
  `TreeBuilder::push`/`finish` + `Dom::document_element` +
  contrat rustdoc DOM partiel, tests `incremental`/`progressive`.
- [x] **R-14. Triage des échecs tree-construction** — diagnostic
  avant de poursuivre le layout. Corpus complet html5lib :
  **953/1592 (59 %)**, 639 échecs inventoriés par algorithme
  fautif : **foreign content ~330** (namespaces svg/math, mode
  « in foreign content », ajustements `definitionURL`/`xlink:`,
  breakout `foreignObject`/`annotation-xml`, CDATA) ·
  **`<template>` contents 110** (fragment `content` séparé) ·
  **adoption agency ~65** (`</a>`, `<nobr>`, reconstruction des
  éléments de formatage) · **`<noscript>` ~45** (mode
  « in head noscript », corps en éléments scripting-off) ·
  **tokenizer ~40** (script-data-escaped, `--!>`, `&#xD;`,
  casse `</S` dans raw-text) · **`<select>` en table ~25**
  · **doctype ids + quirks ~21** · frameset ~10 · catégories
  spec (`<search>/<main>/<menuitem>`, `input type=hidden`) ~8 ·
  after-body ~5 · button-scope ~3. **Décision : le chantier
  tree-builder préempte R-3/R-4 → R-15** (un DOM mal construit
  empoisonne layout, rendu et miroir JS).
  Livré : harnais ouvert (comptage par `.dat` + liste complète),
  code cas/dump partagé (`tests/common/`).
- [x] **R-15. Tree-builder : clusters du triage** — issue de la
  décision R-14, **préempte R-3/R-4**. Ordre par volume
  (inventaire R-14 : harnais ouvert, comptage par `.dat`,
  `tests/common/`) :
  1. foreign content (namespaces sur les éléments, insertion
     mode §13.2.6.5, tables d'ajustement attrs/tags, CDATA
     dans svg/math, points de breakout) — débloque aussi
     `<svg>` inline réel (R-9) ;
  2. `<template>` : fragment `content` propre (modèle DOM +
     règles de réinitialisation des modes) ;
  3. adoption agency + reconstruction des éléments de
     formatage actifs + fermeture `<nobr>` ;
  4. `<noscript>` (in-head-noscript + parsing d'éléments) ;
  5. tokenizer : script-data-escaped, `--!>`, `&#xD;` → CR,
     casse des balises de fin raw-text, table de charrefs
     complétée (`&ImaginaryI;`, `&NotEqualTilde;`…) ;
  6. modes résiduels : in-select-in-table, frameset
     (texte ignoré hors whitespace), button-scope,
     after/after-after-body, catégories spec.
  Cliquet `BASELINE_PASSED` monté au fil des sous-chantiers.
  **Livré : 1592/1592 — 100 % du corpus exécutable** (200 cas
  fragment/script-on hors périmètre). Contenu additionnel au
  plan : table d'entités complète `entities.json` (2125
  entrées vs ~250), normalisation `\r\n` déplacée du DOM au
  prétraitement d'entrée du tokenizer (spec), miroir
  `selectedcontent` du select personnalisable, `select`
  borne de scope (WHATWG PR 10557), scoping namespace-aware,
  foster-parenting vers `template::content`, frameset_ok
  préservé sur `\0` étranger, EOF/comment/CDATA tokenizer.
- [x] **R-16. Layout : tables réelles** (bug terrain @
  `2693959`, repro : news.ycombinator.com) — DOM complet et
  correct (158 `.titleline`) mais table **explosée**.
  **Diagnostic** (repro `r16-center.html`, 1 ligne) :
  `<center>` **absent de `UA_CSS`** → rendu `inline` → une
  table dans un contexte inline perd son containing-block
  → boîte 0×0 + cellules pleine largeur. Livré :
  - `center{display:block;text-align:-webkit-center}` +
    audit complet : `dir`, `menu`, `hgroup`, `search`,
    `listing`, `xmp`, `plaintext`, `details`, `summary`,
    `dialog`, `marquee`, `frameset` blocs ; `[hidden]`,
    `frame`, `details` fermé, `dialog` fermé masqués ;
    `noscript`/`noembed`/`noframes` visibles (scripting
    off — leur contenu EST le repli).
  - `text-align: -webkit-center` en layout : lignes
    inline = `center` ; descendants **bloc** plus étroits
    centrés via `translate_subtree` (hérité → la table
    de contenu HN se centre dans sa cellule).
  - Hints présentationnels (spécificité 0, `or_insert`) :
    `bgcolor`→`background-color`, `align`→`text-align`
    (`middle`=`center`), `valign`→`vertical-align`,
    `cellspacing`→`border-spacing` (défaut 2px émis en
    hint — une règle UA écraserait l'attribut),
    `cellpadding`→`padding` des `td`/`th` via la table
    ancêtre, `border>0`→`border outset` (filet `1px
    inset` cellules), `body[text]`→`color`.
  - Mesuré : `r16.html` → `block<center>` 800px, table
    85% = 680px à x=60 ; `r16-center` table étroite à
    x=396 ; `r16-pct` 680px ; `r16-nowidth` pleine
    largeur ; `r16-nocenter` à x=0. Tests :
    `cascade.rs` (UA legacy + hints), `block.rs`
    (center/table%).
- [x] **R-17. Texte : `em` résolu deux fois** (bug terrain
  @ `2693959`, repro : info.cern.ch, wikipedia). **Diagnostic**
  (repro `r17b.html`) : `font-size:2em` → run 448px au lieu
  de 112 (×4 mesuré — em³ en fait : div→32, anon→64, item→128),
  `1.5em` → ×2,25 ; `32px` correct. **Racine** : le style
  gardait `font-size` brut (« 2em ») et l'héritage propageait
  la déclaration relative — le style cloné des boîtes
  anonymes/texte re-résolvait le ratio contre le font-size
  parent **déjà résolu**. **Fix** (spec : font-size hérite
  la valeur calculée) : la cascade normalise `font-size` en
  px absolus en fin de calcul — `em`/`ex`/`ch`/`%`,
  `larger`/`smaller` contre la taille calculée du parent,
  `rem`/mots-clés absolus contre `StyleEnv::font_size_base`
  (nouveau champ, défaut 16). `calc()` et syntaxes
  exotiques restent bruts (dégradation documentée). Les
  `em` des autres propriétés (marges, letter-spacing…)
  sont inchangés : leur base = le font-size propre, déjà
  absolu. Mesuré : `2em` ≡ `32px` au fragment près ;
  `h1`/`h2` UA corrects ; 36/36 suites PIPELINE vertes
  (WPT reftests inclus).
- [x] **R-18. `color` n'atteint pas les runs de texte inline**
  (bug terrain, repro `color.html`) : `div{color:red}` peint
  ✓, `span`/`a{color}` restaient noirs. **Racine** : le
  raster lisait `color`/`font`/`font-size` sur la boîte des
  lignes (`bx.style`), jamais par fragment — le style de
  l'élément inline propriétaire se perdait au passage
  boîte→run. **Fix** : `Fragment` porte désormais le
  `ComputedStyle` de son item (l'élément inline le plus
  proche — héritage fait) ; `display.rs` peint encre,
  fonte et taille par fragment. Mesuré : `span` → bleu,
  `a[href]` → `#00e` UA, texte direct → couleur du bloc —
  test `color_inline_atteint_le_run`.
- [x] **R-19. Rects parents non-bondants → hit-test mort**
  (**préempte** — une page qui peint mais ne se clique pas
  est le pire mode d'échec ; lite.ddg entier est mort)
  (bug terrain mission 5, journal `interface-equipes.md`,
  repro : lite.ddg entier + zones wikipedia/example.com) :
  `hit_test` élague le sous-arbre quand le point sort du
  rect parent — or `<center>` pose son `<form>` HORS de son
  propre rect (`[0 256 1024×192]` vs enfant `y=76.8`) → tout
  le peint devient incliquable. **Arbitré** : l'élagage
  porte sur `paint_bounds` (union des rects du sous-arbre —
  conservative, inclut les enfants échappés), pas sur le
  border-box. `hit_test` remonté en `explorer-layout`
  (contrat déjà listé) ; `session.rs` délègue — même
  sémantique « plus imbriqué contenant gagne ». Reste ouvert
  (hypothèse connexe) : contenu injecté JS non rafraîchi —
  côté `apply_dom_sync`, périmètre session.
- [x] **R-20. `<script>` peint en texte** (bug terrain
  mission 5, wikipedia — clientpref/RLSTATE JSON visibles
  vers 300 px) : `script{display:none}` existe dans
  `UA_CSS` mais des scripts inline entrent au box tree.
  **Fix** : filet de robustesse dans `build_box_tree_sized` —
  `script|style|template|head|meta|link|base|title` ne
  produisent **aucune** boîte, même si la cascade a manqué
  le nœud (injection DOM post-compute = le cas wikipedia ;
  le test reproduit l'injection sans style). `noscript`
  exclus : scripting off ⇒ contenu rendu (UA).
- [x] **R-21. Texte synthétique des contrôles hors ligne**
  (bug terrain mission 5, journal `interface-equipes.md`,
  repro : `typed_text_repaints_new_value` ignoré) : la valeur
  des `<input>`/`<select>` est synthétisée en enfant `Text`,
  mais le contrôle `inline-block` est traité atomiquement —
  `flow_children`/`layout_block` avance le curseur sur le
  `Text` nu sans l'ajouter aux `lines` → aucun
  `DisplayItem::Text` → le glyphe tapé reste invisible alors
  que `edited`/`frame_invalidated` partent. **Fix** : enfant
  `Text` enveloppé dans un `AnonymousBlock` dans
  `build_box_tree_sized` (chemin standard du flux en-ligne) —
  le texte est posé en ligne et émis en `DisplayItem::Text`
  (test `controle_input_peint_sa_valeur` côté render).
  Réactivation du test NAV = à leur commit (fichier WIP).
- [x] **R-22. Décoration de lien peinte par fragment +
  flux en-ligne réel** (bug terrain mission 6, lobste.rs) :
  trois défauts confondus sur la page réelle. **Livré** :
  (1) **shrink-to-fit §10.3.9** des atomes `inline-block`
  (`ul.tags` : 978 px conteneur → ~87 px — les interlignes
  ~180 px venaient de là) ; (2) **fond/bordure inline
  peints par fragment** (`paint_inline_deco`) au lieu de
  l'union des fragments wrappés (blob sur le texte voisin) ;
  (3) **`border-*-color` absent → `currentColor`** (spec),
  pas noir — les `var(--color-tag-*)` derrière `@import`
  non résolue retombent proprement ; (4) **§9.2.1.1** : un
  bloc en flux niché dans un inline pur éclate la boîte
  (`hoist_blocks` — `details.caches > summary` : ligne de
  byline ~93 px → ~23 px) et un `inline-block` établit son
  contexte bloc (enfants anonymisés — les blancs ne
  comptent plus une ligne chacun) ; (5) **hors-flux hors
  de la ligne** : `absolute`/`fixed` sautés de
  l'itemisation inline tout en restant nichés pour
  `resolve_absolutes`, avec garde **kind-aware**
  (`LayoutBox::out_of_flow` — le style cloné d'un `Text`/
  `AnonymousBlock` n'est pas une position d'élément, sinon
  le texte d'un absolu était posé deux fois).
  Régressions : `tests/repros.rs` (fixture lobste.rs figée,
  25 stories), `boxtree.rs`/`inline.rs` (split, absolu
  niché, `inline-block` + bloc), `display.rs` (déco par
  fragment, `currentColor`). Reste : `@import` jamais
  fetchée — les couleurs `--color-tag-*` restent non
  résolues (couture NAVIGATEUR, journal). **Vérifié terrain
  mission 13** : lobste.rs transformé — titres propres,
  seules les vraies pastilles de tags gardent leurs
  bordures (`currentColor`).
- [x] **R-23. Valeur de contrôle peinte à (0,0)** (résidu
  R-21, terrain mission 7 — fixture `repros/form.html` : le
  glyphe tapé apparaît mais chevauche le `<h1>` en haut à
  gauche) : le fragment atomique recevait son rect final
  (`assign_atomic_rects`) mais son **sous-arbre** restait
  posé en (0,0) — `size_atomics` y avait appelé
  `layout_block` pour le dimensionner. **Fix** :
  `translate_subtree` (delta fragment − rect pré-layout) sur
  l'atome, ordonné **avant** `assign_child_rects` (l'union
  `owner` aurait sinon écrasé le rect source → delta nul) ;
  les atomes sont sautés dans `assign_child_rects` (leurs
  indices `owner` internes ne parlent pas des lignes du
  parent). Couvre tout contenu d'atome : valeur de
  contrôle, texte d'`inline-block`, repli d'`object`.
  Test : `controle_input_valeur_posee_sous_le_controle`.
  **Vérifié terrain mission 13** : « test » peint dans
  l'input, plus à (0,0).
- [x] **R-24. Profondeur d'arbre bornée — stack overflow**
  (**préempte** — 2ᵉ violation « jamais crash » après
  JS-C41 ; terrain mission 10, repro `repros/host-nest.html`
  : **~300 `<div>` imbriqués tuent le process**,
  `stack overflow` worker tokio. `dump` DOM seul survit →
  récursion en aval : `build_box_tree`/`layout_block`/
  `build_display_list`/`hit_test` sont tous récursifs sur la
  profondeur DOM). Arbitrer : borne de profondeur avec
  dégradation documentée (sous-arbre aplati/tronqué à N),
  ou traversées itératives — le patron `stacker::maybe_grow`
  de JS-C41 existe déjà comme précédent. **Livré** :
  `explorer_common::deep` (`stacker::maybe_grow`, zone rouge
  128 Kio / segment tas 1 Mio) sur chaque arête récursive ∝
  profondeur DOM — `Dom::collect_text`, `compute_subtree`,
  `match_selector_part`, boxtree, `layout_block` (hub :
  flow/flex/grid/table/inline/relayout/absolus),
  `translate_subtree`, `size_atomics_walk`, `assign_*_rects`,
  `content_px`, `walk_children` (tables), `paint_bounds` +
  `hit_test`, `relayout::step`/`contains`/`has_float`,
  `paint_box`, `svg::serialize_into`, `dump` — + **drop
  itératif** de `LayoutBox` (le drop récursif par défaut
  déborderait à lui seul). Régression : `tests/deep.rs`
  (pipeline complet sous pile 512 Kio, 4000 niveaux
  demandés).
- [x] **R-25. `<object>` résolu inline ne peint pas sa
  ressource** (terrain mission 12 — fixtures `b2`/`b4`) :
  la cascade `<object>` descend correctement (repli
  fonctionnel — texte `ERROR` au bout), mais même dans le
  harnais acid2 qui appelle `collect_data_images`, les yeux
  restent absents — suspect : la feuille remplacée sous
  `display:inline` (`#eyes-a object`) reçoit une taille
  nulle ou n'est pas atomisée → 0 px vert mesuré malgré
  l'image décodée. Second maillon après 56e (câblage
  session) — les deux sont nécessaires pour les yeux.
  **Précisé mission 13** : 56f livré — `data:` peint en
  session et `<object data>` **bloc** peint ; le trou
  résiduel est la feuille remplacée **inline** (l'objet
  résolu sous `display:inline` ne peint pas sa ressource).
  Dernier maillon avant les yeux. **Résolu par la salve 3
  de R-13** — les sprites verts des yeux peignent (≈240 px
  verts mesurés x125-186 / y2691-2710 au harnais Acid2) :
  l'objet remplacé inline est atomisé et peint en content
  box (`content_rect` — la border box l'étirait sur les
  bordures/padding, l'ordre App. E le fait passer au-dessus
  des flottants).
- [x] **R-26. Compteur de diff WPT : underflow `usize`**
  (terrain mission 12 — triage du corpus) : un échec
  `justify-content` rapporte `18446744073709551615 px durs`
  — sentinelle `usize::MAX` renvoyée quand les dimensions
  diffèrent. `diff_stats` compare désormais la zone de
  recouvrement et compte l'excédent comme « dur »
  (`…-002.xhtml` : 59 156 px réels au lieu de la sentinelle).
  Cliquet inchangé : 41/58.
- [x] **R-27. Direction d'axe flex — `row-reverse`/`column`**
  (triage mission 12 — premier lot des 17 échecs WPT
  résiduels, la fondation qui empoisonne les mesures
  justify/align/wrap en aval). Trois fixes orthogonaux :
  **(a)** `row-reverse` pose les items dans l'ordre logique
  inversé (avant : ordre DOM + décalage de groupe — les
  libellés tombaient dans le désordre) et le `lead` miroir
  inclut l'espacement `between` (§8.1 — `justify-content`
  mesuré depuis main-start) ; **(b)** `text-align` mesure
  la ligne sur les **margin boxes** (`Fragment::marg_l/r`
  — §16.2 : les marges d'un atome comptent dans la largeur
  de ligne — la réf `text-align:right` débordait à +112 px
  hors conteneur) ; **(c)** la chaîne de marges basse du
  dernier fils en flux reste **dans** la hauteur de contenu
  quand un séparateur existe — bordure/padding bas, BFC,
  hauteur fixe (§8.3.1 — les réf `column`/`column-reverse`
  à `border:1px` perdaient le `mb` final : 191 px au lieu
  de 207). Cliquet WPT 41 → **44/58** (les trois
  `flex-direction-*` passent, zéro régression). Reste
  documenté : `justify` sous `column-reverse` ancrait déjà
  haut (MVP).
- [x] **R-28. Distribution d'espace flex** (7 échecs WPT) :
  `justify-content` horiz-001a/002/003/004,
  `align-items` center/flexend/stretch. Livré — plusieurs
  causes racines plutôt qu'une seule géométrie : **(a)**
  alias `left`/`right`/`end` (css-align-3 §4.1 — MVP
  horizontal LTR) ; **(b)** lead **négatif** conservé pour
  `flex-end`/`right`/`end`/`center` en débordement (§9.5 —
  l'overflow part vers main-start ; les `space-*` replient
  en flex-start) ; **(c)** ligne mono-ligne = taille cross
  interne du conteneur à hauteur définie (§9.4 — pas la
  hauteur max des items) ; **(d)** `flex-basis`/`width` =
  **content-box** — padding + bordures s'ajoutent à la taille
  externe (l'item `a` de `-004` : 45 pas 35) ; **(e)** la
  cible shrink/grow remplace le `width` déclaré à la pose
  (§9.7) ; **(f)** `vertical-align: top/bottom/middle` des
  atomes inline-block — margin box ancrée aux bords de ligne
  (§10.8 — les réf `…-002/-004` à `vertical-align:top` en
  dépendent) ; **(g)** marge haute du premier fils en flux
  ne traverse pas un **BFC** (`adjoining_top` symétrique à
  R-27 — les wrappers inline-block des réf gardaient leur
  `mt` interne : 20 px au lieu de 22) ; **(h)** positionné
  `bottom`/`right` translate **tout le sous-arbre** (les
  enfants des spans absolus `bottom:0` restaient à la
  position statique) ; **(i)** marges hautes échappées des
  descendants agrégées dans `run` + règles flottantes CSS
  2.1 n°3 (descendre sous le flottant adverse si la bande
  est insuffisante) et n°7 (descendre sous le flottant
  même-côté si le bord du bloc conteneur est dépassé).
  Cliquet WPT 44 → **53/57 (3 ignorés)** — `horiz-004` entre
  dans `wpt_skip.txt` : fixture défectueuse documentée
  (bug Mozilla 1954193 — ses rangées `left`/`right` codent
  une sémantique qu'aucun navigateur n'implémente ; le diff
  est normalement sous le viewport 600 px).
- [x] **R-29. Wrap horizontal + `column/row-gap` flex**
  (2 échecs WPT) : `flexbox-flex-wrap-horiz-002`,
  `flexbox-column-row-gap-001` — dépend de R-28.
  Livré : (a) flottant `width:auto` shrink-to-fit —
  `min|max-width` bornent la boîte de **contenu** avant la
  pose des enfants (§10.3.5/§10.4) ; (b) enfants flottants
  comptés en **somme** dans le max-content d'un parent
  shrink-to-fit (une ligne, pas un empilement) ;
  (c) `column wrap` complet — découpe en colonnes sur la
  hauteur définie, tailles flex grow/shrink sur `height`,
  marges `auto` main, `justify-content` par colonne,
  `align-content` entre colonnes sur la largeur cross,
  `column-gap`/`row-gap` sur les bons axes, `wrap-reverse`
  en miroir cross ; (d) marge cross `auto` figée à 0 pendant
  la mesure (résolue sur la colonne, pas le conteneur).
  Cliquet WPT 53 → **55/57 (3 ignorés)** — les 2 échecs
  restants sont le lot grid R-30.
- [x] **R-30. Unité `fr` grid + `display:grid` résiduels**
  (2 échecs WPT) : `fr-unit`, `grid-display-grid-001` —
  lot séparé, algos disjoints de flex.
  Livré : (a) pistes `fr` verticales distribuées contre la
  hauteur définie du conteneur — libre = hauteur − pistes
  non-flexibles − gaps, plancher `minmax` (§12.7) ;
  (b) `height:%` des items grid résolu contre l'aire de
  grille — la cellule EST le bloc conteneur (§6.5) via
  `cb_h` passé à `layout_item` ; (c) position statique des
  absolus = border-box en flux — la marge en attente du
  frère précédent est consommée (§10.6.4) ; (d) table :
  `width` de cellule cloue sa colonne, reste aux colonnes
  auto (§17.5.2.2) ; `height` définie distribuée aux rangs
  auto (§17.5.3) ; (e) peinture des positionnés `z-index`
  négatif **avant** les blocs en flux (ordre de peinture
  §E.2 — fond `.error` sous la grille).
  Cliquet WPT 55 → **57/57 (100 %, 3 ignorés)**.
  **Le sous-ensemble WPT extrait est soldé** — plus aucun
  échec connu à cliqueter ; la suite se joue sur Acid2
  (R-31) et un corpus élargi (R-32).
- [ ] **R-31. Acid2 → 100 %** (lead) : corpus à
  **96,5 %** (1282/1329, alignement (72,108) avec émulation
  du scroll de capture). Soldés : strut §10.8.1 (menton),
  `;` orphelin avalant la règle qualifiée (Syntax §5.4.4 —
  `.parser` 1em), marge basse échappée §8.3.1 (`<ul>`),
  ordre `min|max` §10.7 (barre fixe 12 px), CB des inlines
  `relative` §10.1. Reste ~47 px : bords AA du PNG œil et
  du damier `fixed` sous échantillonnage 4 px, arête du nez
  (~5 px). Méthode éprouvée : crop par région, diff pixel,
  un commit par cause.
- [ ] **R-32. Élargir le corpus WPT** (lead) : le
  cliquet actuel (57/57) ne mesure plus rien — extraire une
  tranche nouvelle via le script existant (candidats :
  `vertical-align` complet, `overflow`, `position`,
  margin-collapsing systématique, tables avancées,
  `display` résiduels). Nouveau baseline honnête
  « x/y exécutés (z skips) sur sous-ensemble S' élargi » +
  cliquet reposé. Objectif : réactiver la boucle
  échec→cause racine qui a produit R-27→30.
- [ ] **R-33. Perf — coût unitaire de l'apply**
  (demande consignée `interface-equipes.md`, terrain
  mission 15) : `incr.remap_relayout` = **150,3 s** sur
  `storm.html` (~600 nœuds + mutations accumulées) vs
  ~14 s au profil test `profile_resync_phases` — écart ×10
  à expliquer (saleté accumulée absorbée d'un coup ? remap
  mémoïsé dégénéré ? cascade globale re-stylée au lieu du
  sous-arbre sale ?). Livrable : attribution précise via
  `sync_profile` + fix du poste dominant + cascade scopée
  au sous-arbre sale si elle y contribue. 55h (NAV) a sorti
  l'apply du verrou — le coût restant est ici.
  **Mur frère mesuré (mission 18)** : la nav initiale de
  `heavy.html` (~3 000 nœuds) gèle l'API ~4–6,5 min —
  `nav_apply`/`sub_commit` tournent encore sous verrou
  session. Même famille que le coût unitaire (layout +
  cascade), responsabilité partagée : NAV étudiera la
  découpe du commit sous verrou ; le coût pur du pipeline
  relève d'ici.
- [ ] **R-34. Médias vrais remplacés boxtree**
  (résidu étape 58, demande consignée) : `video`/`audio`
  comme remplacés natifs — taille intrinsèque de l'affiche
  décodée (le poster n'est plus étiré dans un 300×150
  forcé), `object-fit` minimal, `video[poster]` couvert par
  `collect_data_images` (le complément NAV
  `poster_data_images` se retire alors), hints
  présentationnels `width`/`height`.
- [ ] **R-35. Métriques de runs pour find**
  (résidu 57bis, demande consignée) : API de métriques —
  sous-rect du match dans une boîte `Text` (offsets glyphe
  → `Rect`), remplace le surlignage boîte-entière actuel.
  API publique additive, consommée par `session/find.rs`.
- [ ] **R-36. Cache glyphes `paint_text`** (demande
  consignée, perf raster) : le repaint global re-shape
  chaque run — cache par (fonte, taille, texte) ou
  run-level, borné. À instrumenter `sync_profile` avant/
  après pour chiffrer le gain.
- [x] **R-2. WPT floats/block-formatting — finir les 13 cas**
  (ex-54). Échecs listés après l'étape 49 : placement
  `004/006/007/008`, `wrap-top-below-inline-001r`, `bfc-overflow`,
  `zero-height`, `block-formatting-001/003/004/008/015`.
  Cliquet `WPT_BASELINE` monté dans le même commit — objectif
  ≥ 35/40 → **38/38 mesurés (2 exclus documentés)**.
  Livré : blocs anonymes = flux pur (jamais BFC/float),
  ancre des flottants au haut de ligne, bandes `band_over`
  + `direction: rtl`, attributs présentationnels
  `width`/`height` en cascade, `%height` contre le bloc
  conteneur, hauteur border-box des tables, `\u{A0}` =
  blanc non-collapsible (`is_css_ws`), `border-collapse`
  horizontal MVP + ordre visuel thead/tbody/tfoot, canvas
  blanche, `background-image` peinte (`PaintImages` +
  `build_display_list_full`), snap du clip à la grille
  device, peinture texte réparée (ordre de composition
  fonte→plume + pen arrêté au pixel), harnais WPT étendu
  (images `support/` + `url()`), bordure `medium` 3 px par
  défaut. Skips : `bfc-011` (métrique fonte Ahem seule),
  `block-in-inline-append-001` (`flags: dom` ⇒ JS requis).
- [x] **R-3. Tailles intrinsèques réelles** (ex-56).
  `min-content`/`max-content` mesurés par contenu (plus
  d'approximation), `margin:auto` (block horizontal + axes flex),
  `stretch` cross — fondation requise pour R-4/R-5.
  Livré : module `intrinsic` (mesure récursive du sous-arbre —
  mot le plus long pour min-content, ligne insécable pour
  max-content, `fit-content` borné) branchée sur `width` /
  `min|max-width` / `flex-basis` ; bases flex `auto` et
  colonnes grid `auto` mesurées en max-content réel ;
  `margin:auto` sur items flex (axe principal avant
  `justify-content`, cross = reliquat) ; `stretch` étire le
  rect cross quand la taille est `auto`.
- [x] **R-4. `flex-wrap` multi-lignes** (ex-57) :
  coupures de lignes flex, `align-content`, `order` cross-line.
  Livré : découpe en lignes par taille hypothétique
  (base+marges), flex/marges `auto`/`justify-content`
  calculés **par ligne**, `row-gap`, `align-content`
  (7 valeurs + `stretch` qui grandit les lignes) quand la
  hauteur du conteneur est définie, `wrap-reverse` miroir
  l'axe cross. Hors scope documenté : wrap en `column`,
  `height` en % sans base définie.
- [x] **R-5. Grid complet MVP+** (ex-58) :
  `grid-template-areas`, placement nommé, `auto-flow`
  dense/column, `minmax`/`fr` mixtes résolus ; `subgrid`
  documenté hors scope. Livré : `[noms]` de lignes,
  `grid-area` (nom d'area ou `r/c/r2/c2`), `auto-flow:
  column` (colonnes implicites) et `dense` (backfill),
  `minmax(min, max)` deux bornes (fr borné par min).
  Écarts : `fr` vertical ≈ auto, `auto-fill|fit` ignorés,
  spans multi-pistes hors mesure auto, indices négatifs.
- [x] **R-6. `position: sticky` tous côtés** (ex-59) :
  `right`/`bottom`/`left`, conteneur de scroll correct,
  référence `overflow` non-visible. Livré : `sticky_offset`
  4 côtés (top gagne sur bottom, left sur right en LTR),
  borne = border box du parent, scrollport = viewport —
  `build_display_list_scrolled` ajoute `scroll_x` (les
  anciens builders délèguent à x=0). Écart documenté : les
  conteneurs `overflow` internes ne scrollent pas encore —
  la référence reste le viewport racine.
- [x] **R-7. Dirty-rects produits** (ex-63a) : la display list
  rapporte les régions modifiées par frame. **Doit être livré
  avant que NAVIGATEUR n'aborde l'étape 55** (palier N-B) —
  remonte automatiquement si NAV accélère. Livré :
  `DisplayList::changed_rects(prev)` — bounds résolus dans le
  contexte transform/clip (même ordre `pre_concat` que le
  raster), zones ancienne+nouvelle rapportées, fusion par
  intersection, `[]` si frame identique. API additive, aucun
  changement de signature — débloque NAV-55.

## Palier R-B — Contenu étranger, animation, canvas

- [x] **R-8. Foreign-content dans `explorer-html`** (ex-60a) :
  tokenizer/tree-builder SVG et MathML inline, points de breakout
  de la spec, ajustements d'attributs/casse — prérequis de tout
  `<svg>` inline réel. **Livré par R-15** (sous-chantier 1 :
  namespaces, points d'intégration, breakout, `CDATA`,
  ajustements d'attributs étrangers — 1592/1592 html5lib).
- [x] **R-9. `<svg>` inline** (ex-60b) : sous-arbre SVG rendu
  (via `usvg` sur le sous-arbre sérialisé, ou display items
  dédiés — arbitrer en début d'étape), `<foreignObject>`
  documenté hors scope. Arbitrage : **sérialisation + usvg**
  (réutilise le moteur conforme d'étape 45 au lieu de
  dupliquer un parseur SVG). Livré : `svg::collect_svg` +
  `svg::svg_sizes` (sérialisation XML échappée, bornée
  `SVG_MAX_SOURCE` ; tailles attrs/`viewBox`/300×150),
  `PaintImages.svgs`, bras atomique dans `build_box_tree_sized`
  (sous-arbre jamais boxifié HTML), `svg` en `inline-block`
  UA. Hors scope documenté : `foreignObject` refusé par
  usvg → zone vide ; bitmap à taille naturelle (re-raster
  DPI exact = chantier ultérieur).
- [x] **R-10. `@keyframes`** (ex-60c) : parsing dans
  `explorer-css`, timeline partagée avec le moteur de
  transitions de l'étape 45 — le cadenceur reste côté session
  (consommé via `tick_animations` existant). Livré : module
  `animation` — collecte `StyleEngine.keyframes` (multi-offsets
  éclatés, same-offset fusionné, trié), `AnimationDecl` +
  `animation_decls` (shorthand + 7 longhands alignés par index,
  `infinite`, `direction`, `fill-mode`), `phase(elapsed)` →
  [0,1] (délai/itérations/alternate/forwards-backwards),
  `keyframe_style` interpole via `transition::interpolate`
  (timeline partagée), frames 0 %/100 % implicites depuis le
  style calculé. Cadencement session = adoption NAV.
- [x] **R-11. Raster `<canvas>` 2D** (ex-62, moitié rendu) :
  playback de l'**op-list** enregistrée par `explorer-jscore` (JS-C28 —
  ↔ NAV, surface JS) vers tiny-skia → pixmap de l'élément →
  `DisplayItem::Image`. **Couture figée** (`interface-equipes.md`)
  : `canvas::CanvasOp`/`CanvasTable` + `PaintImages.canvases` ;
  playback `raster_ops` (rects, chemins, fill/stroke/clip,
  transforms, alpha, lineWidth, save/restore — texte/dégradés/
  drawImage hors MVP, variantes additives) ; `<canvas>` =
  remplacé intrinsèque `width`/`height` (défaut 300×150),
  UA `display:inline-block`.

## Palier R-C — Incrémental interne, mesure, robustesse

- [x] **R-12. Layout incrémental** (tiers `DirtyLayout`
  d'ADR-0012 réalisé) : `relayout_subtree` reboîte le sous-arbre
  sale (`build_box_tree_sized`), le re-pose à son `BoxSlot`
  enregistré et propage la hauteur aux ancêtres — repli
  `NeedsFullLayout` sur flottants dans le document, chemin
  absolu/fixe ou inline/anonyme. Mesuré
  (`benches/relayout.rs`, page 200 blocs) : ~33 ms reflow
  complet → ~1.9 ms relayout feuille (~17×).
- [x] **R-13. Acid2 ≥ 60 %** (ex-61) + extension du corpus
  WPT reftests : extraction `css/css-flexbox` + `css/css-grid`
  ciblés, baselines mesurées, skip-list versionnée — les écarts
  deviennent des étapes R-xx. **Baseline terrain mission 9**
  (shot `repros/acid2/` vs page de référence) : ~15 % du
  visage — `.intro` bordée, liens bleus et « Hello World! »
  peints ; `.picture p` fixed ≈ au bon endroit. **Écart n°1**
  résolu : le bloc conteneur positionné rapportait les boîtes
  du visage à la border box — padding box désormais, et les
  `em` se résolvent par boîte. **Salve livrée** : pseudo-
  éléments `::before`/`::after`, `data:` URLs (décodage +
  collecte), `<object>` repli/remplacé atomique en ligne,
  `background` reset + position + `fixed`, cellules/rangs de
  table anonymes + stretch §17.5.3, shrink-to-fit §10.3.7,
  collapse à travers les vides. Score informatif 23,3 % →
  22,0 % (géométrie plus juste, comptage pixel encore bas) —
  la structure du visage tient (front, nez, sourire, `ul` en
  cellules 12 px) mais le compactage vertical et les
  alignements fins restent à moudre. **Correction terrain
  mission 11** : contrairement à la lecture initiale, les
  yeux verts sont **absents** (0 px vert mesuré — la chaîne
  `<object>`→`data:` PNG ne peint toujours pas) et le visage
  reste ~y2650 au lieu de ~y1400 (compactage vertical).
  **Métrique** : le score pixel global punit la convergence
  (un offset corrigé réduit le recouvrement avant de
  l'augmenter) — compléter par un tableau par composant
  (front/yeux/nez/sourire/fond : présent/partiel/absent)
  dans le harnais `tests/acid2.rs`.
  **Salve 2** : items flex/grid blockifiés (plus de bloc anonyme
  unique), contenu en-ligne des `inline-block` items coulé via
  `layout_inline` (le texte restait 0×0), marges items flex
  (axe principal + cross, §9.4), shorthands `flex-flow`/`flex`/
  `gap`, harnais `.xhtml` normalisé (`<t/>` auto-fermant),
  corpus +19 reftests flexbox/grid, cliquet 38 → 40/58.
  Reste : `column wrap` multi-colonnes, `margin:auto` vertical
  en column, écarts fins `justify-content`/`align-items`,
  `fr`/`grid` margins — chacun devient une étape si le triage
  le justifie. **Triage mission 12** (18 échecs, tous dans le
  nouveau corpus — le socle floats tient) : direction d'axe
  flex (row-reverse/column), wrap horizontal, `column/row-gap`,
  justify ×4, align-items ×3, grid `fr`/display/marges ×3 —
  familles candidates : *direction axe flex*, *justify/align
  fins*, *unité fr grid*, *marges grid*. **Salve 3 — Acid2
  22 % → 86,1 %, cap ≥ 60 % atteint** : collapse des marges
  par chaîne complète §8.3.1 (`MarginRun` — max positif +
  min négatif à travers les éléments transparents), flottants
  posés au bord de la **margin box** (le `margin-top:-2em`
  du nez remonte le bloc), `top`/`bottom` absolus ancrant la
  margin box + shrink-to-fit incluant padding/bordures,
  ordre de peinture Appendice E (blocs en flux → flottants →
  inlines reportés au contexte de pilement, positionnés
  triés `z-index`), jointures de bordure diagonales (losange
  du nez via trapèzes), largeurs de bordure `em` résolues à
  la font-size de la boîte en peinture, images remplacées
  peintes en content box. Côté CSS : `\x` en tête de token =
  ident (`\}` ne ferme plus le bloc — `background:yellow`
  d'Acid2 survit), `!` parasite invalide la déclaration
  (`border: 5em solid red ! error`), `background: red pink`
  droppé (une valeur par composante), `width: 200` sans
  unité droppé via `value_is_valid_for` (validation par
  propriété, `var(`/`calc(`/`attr(` et CSS-wide exemptés),
  hash `#000` re-sérialisé sans échappement de digit,
  idents échappés préservés dans les préludes re-parsés
  (`[class=second\ two]`). Cliquet WPT 40 → **41/58**
  (`floats-placement-005`, `grid-margins-no-collapse-001`,
  `grid-layout-basic` réparés : colonne `auto` plafonnée au
  free space §12.6, marges d'items grid en margin box).
  Reste connu : la hairline fixed et `p.bad` restent hors
  du visage rasterisé (pas de scroll — ancrage viewport par
  conception), anneaux des yeux et rangées parser/`ul` à
  l'affûtage pixel près.

## Tâches continues

- **Triage rendu** des échecs `site_smoke.ps1` : les écarts
  visuels classés « pipeline » au triage deviennent des
  cas de test (`tests/` du crate fautif) puis des R-xx.
- **Corpus vivant** : tout bug site réel → test minimal avant fix
  (méthode des étapes 48/49 — elle a fait ses preuves).
- **Fixture partagée** : la fonte Ahem/TTF vit dans
  `explorer-test-support` (crate partagé — ajouts additifs
  libres, structure via interface-equipes.md).

## Interfaces avec l'équipe NAVIGATEUR (récapitulatif)

| Contrat (pub API des crates PIPELINE) | Consommateur | État |
|---|---|---|
| `TreeBuilder::parse` / `push(&str)` | `explorer-core` session | ✅ — incrémental prêt, non exploité (NAV-53 ↔ R-1) |
| `StyleEngine`/`font_faces(env)` | `explorer-core` | ✅ |
| `FontProvider`/`FontSource` port | impl `fontdb` en infra core | ✅ (ADR-0009) |
| `build_box_tree_*`/`build_display_list_*` | `explorer-core` | ✅ — variants sized/imaged |
| hit-test / path `(tag,index)` | `explorer-core` dispatch events | ✅ |
| `DisplayItem` + dirty rects | transport FFI (NAV-55) | R-7 à faire |
| op-list canvas → `DisplayItem::Image` | `explorer-jscore` canvas (JS-C28) | R-11 à faire ↔ NAV |

Règle : les API publiques des crates PIPELINE sont le contrat —
extensions additives de préférence ; toute casse de signature
existante = entrée dans `docs/interface-equipes.md` avant code.
