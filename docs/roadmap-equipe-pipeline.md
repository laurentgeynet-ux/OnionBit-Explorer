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
- [x] **R-31. Acid2 → 100 %** (lead) : corpus à
  **99,5 %** (1323/1329, alignement (72,108) avec émulation
  du scroll de capture) — refermée : le reste est le
  plancher de bruit AA (tiny-skia vs renderer 2005), pas
  de la conformité. Soldés : strut §10.8.1 (menton),
  `;` orphelin avalant la règle qualifiée (Syntax §5.4.4 —
  `.parser` 1em), marge basse échappée §8.3.1 (`<ul>`),
  ordre `min|max` §10.7 (barre fixe 12 px), CB des inlines
  `relative` §10.1, phase de tuile des fonds `fixed` ancrée
  viewport (bug dormant scroll>0), `line-height: normal`
  aux métriques réelles `hhea` (le 1.2em codé en dur
  décalait tout le visage de 0.2 px — la vraie cause
  « AA » des yeux et du nez). Plancher documenté : 6
  échantillons = mélanges AA sur les mitres 45° du losange
  (96,36,0 / 219,159,0 — couverture sub-pixel, jamais une
  couleur franche). Méthode éprouvée : crop par région,
  diff pixel, un commit par cause.
- [x] **R-32. Élargir le corpus WPT** (lead) : tranche
  `margin-padding-clear` + `floats-clear` + `normal-flow`
  (margin-collapsing systématique) extraite via
  `fetch_wpt.ps1` — 154 fichiers dont 36 sans référence.
  Score mesuré **115/152 exécutés (6 skips)**, cliquet
  `WPT_BASELINE` reposé à 115. La boucle échec→cause a
  produit : collapse-through des chaînes hautes pré-layout
  (`chain_transparent`/`candidate_chain` purement
  stylistiques), marges de la racine jamais collapsées
  (`is_root`), position statique des absolus ancrée au
  `flow_bottom` du frère précédent (§10.6.4), marges des
  flottants jamais collapsées, `table-layout:fixed` +
  règles UA `col`/`colgroup` et somme des colonnes
  spécifiées pour `width:auto`, `border-width` mots-clés
  dans les extras de mesure de cellules, propagation du
  fond de `body` au canvas (§14.2), `background-position`
  en `em`, reset `line-height`/`font-*` du shorthand
  `font`, ordre de pile Appendix E (portée inline fermée
  par conteneur bloc + positionnés `z≥0` différés au
  contexte de pilement), translation du sous-arbre pour
  `position:relative`, marges horizontales des éléments
  inline (marqueurs de portée). Feuille UA `body {
  margin: 8px }` ajoutée (compat réelle — refs partagées
  WPT la supposent) avec neutralisation `margin:0` dans
  les fixtures downstream écrites pour une marge nulle
  (carve-out §10.3). Skip documenté :
  `margin-collapse-through-percentage-padding.html`
  exige mutation JS + relayout — piste NAVIGATEUR, hors
  périmètre PIPELINE.
- [x] **R-33. Perf — coût unitaire de l'apply**
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
  **Attribution mesurée** (harnais phases sur storm.html,
  debug) : parse 13 ms, cascade 69 ms, boxtree 12 ms,
  layout complet 74 ms — et `relayout_subtree` **154 ms,
  soit ~2× le layout complet**. Cause : `step` appelait
  `relay` = `layout_block` entier sur **chaque ancêtre**
  dont la hauteur bougeait — sur un document plat (#cont
  + 600 `<p>` frères), chaque append reposait `body` puis
  `html` en entier (O(profondeur) × layout global).
  **Fix** : propagation par translation — sans flottant
  (garanti par `has_float`), une hausse Δ du fils décale
  ses frères suivants de Δ, leurs layouts étant invariants
  (§8.3.1 : marges inchangées). Gardes conservatives :
  fils hors-flux, transition vide ↔ non vide (casse la
  chaîne de collapse), conteneur flex/grid/table ou
  `min|max-height` → `relay` classique. **Résultat :
  154 → 9,7 ms (×16)**, arbre strictement égal au reflow
  complet (2 tests permanents `relayout.rs`, comparaison
  positions à 0,01). La cascade globale (`styles_payload`,
  ~69 ms) devient le poste suivant — scopage non requis :
  elle est bornée O(n), pas anormale ×10.
  **Mur frère mesuré (mission 18)** : la nav initiale de
  `heavy.html` (~3 000 nœuds) gèle l'API ~4–6,5 min —
  `nav_apply`/`sub_commit` tournent encore sous verrou
  session. Même famille que le coût unitaire (layout +
  cascade), responsabilité partagée : NAV étudiera la
  découpe du commit sous verrou ; le coût pur du pipeline
  relève d'ici.
  **R-33b — part pipeline du mur frère soldée.** Attribution
  (harnais `profile_resync_phases`, ~9 000 nœuds, debug) :
  `layout_tree` = **27,7 s**, dont la quasi-totalité fonte —
  chaque run de texte re-facturait `font_data` (rebuild
  fixture en test / clone ~200 Ko en prod) + `Face::parse`
  ×2 + shaping `rustybuzz`, ~10 appels/item, **zéro cache**
  dans tout le pipeline fonte. **Fix** : `FontCache`
  (`font.rs`) borné à une passe — octets par `FontRequest`
  (miss figé), métriques par (fonte, taille), shaping par
  (fonte, taille, texte, dir) borné à 8 192 entrées ;
  `BlockCtx::font_cache` côté layout, cache propre à
  `rasterize` côté render — `request_of` mutualise la
  résolution style → requête (avant : triple copie).
  **Encode PNG** : `Frame::encode_png` passe de tiny-skia
  (clone + démultiplication + deflate `Balanced`) à
  `image::PngEncoder` `Compression::Fast`/`NoFilter`
  (fdeflate) — la démultiplication est conservée à
  l'identique. **Résultats** : layout 27,7 → **1,09 s**
  debug (×25) ; `relayout total` 24,8 → **3,78 s** debug,
  **289 ms** release ; `encode_png` 4,3 s → **125 ms**
  release. 3 tests permanents `text.rs` (dédupe source,
  cohérence métriques/shape, fonte invalide jamais panic).
  WPT 57/57, Acid2 99,5 % inchangés. **Réception terrain
  (même protocole que M21-F, worktree fieldwork à
  `c2488c6`, debug)** : wall `heavy.html` **706 → 38,3 s
  (×18)** — sous la barre de la minute ; gel API **réduit
  mais présent** : le verrou session reste tenu pour toute
  la nav (polls `/api/status` tenus ~35 s pendant le
  commit, ~1,4 ms après) — `commit.apply_queued` ~31 µs
  sur storm (hand-off détaché 55h quasi libre), API ~2 ms
  pendant les mutations. Décomposition du reliquat 38 s :
  ~6-10 `relayout_dom` progressifs (~3,4 s debug chacun,
  cadence 32 Kio/100 ms) + miroir DOM→JS de `spawn_engine`
  sur ~9 000 nœuds — **file NAV** (options notées :
  miroir détaché façon 55h, cadence adaptative, verrou
  par onglet). Mesures debug — en release le gel serait
  ~3-5 s, la *structure* du verrou reste à corriger.
- [x] **R-34. Médias vrais remplacés boxtree** — livré :
  `video`/`audio` sont des feuilles remplacées **natives**
  du boxtree — taille intrinsèque de l'affiche décodée
  (le poster n'est plus étiré dans un 300×150 forcé —
  test bout en bout : `<video poster>` ≡ `<img>` du même
  bitmap), gabarit standard sinon (vidéo 300×150, audio
  300×54) ; le contenu de repli n'est jamais boxifié.
  `object-fit` minimal à la peinture (`fill`/`contain`/
  `scale-down`/`cover`/`none` + `object-position`
  mots-clés/`%`/`px` ; les modes débordants sont clipés à
  la content box). `collect_data_images` couvre
  `<video poster="data:…">` → complément NAV
  `poster_data_images` retiré ; hints `width`/`height`
  (déjà présents pour `video`, axe par axe). UA :
  `video` redevient remplacé en-ligne, `audio:not
  ([controls])` = `display:none` côté domaine ; le
  complément UA session ne garde que le look produit
  (fond noir, chrome audio). Tests : feuilles natives
  boxtree, rect/clip `object-fit`, collecte poster
  `data:`, équivalence intrinsèque bout en bout.
- [x] **R-35. Métriques de runs pour find** — livré :
  `fragment_sub_rect` (PIPE, additif) donne le sous-rect
  d'une tranche d'octets d'un fragment posé — union des
  cellules des glyphes dont le `cluster` tombe dans la
  tranche (ordre visuel → LTR/RTL, ligatures), bande =
  hauteur de ligne du fragment ; repli proportionnel aux
  octets sans run shapé, `None` sur tranche vide/atome.
  `session/find.rs` recherche sur le texte **affiché**
  (fragments concaténés par boîte — les matches enjambent
  fragments, styles internes et retours à la ligne) et
  groupe les sous-rects par occurrence (`Vec<Vec<Rect>>` :
  compte/navigation à l'occurrence, comme Chrome). Repli
  boîte-entière conservé pour les `Text` hors lignes
  (valeurs de contrôle), dédupliqué via collecte globale
  des nœuds couverts. Tests : sous-rect exact au glyphe
  (fixture 0.5em), replis, bande peinte 3 glyphes sur 10,
  dédup match niché + groupement transversal.
- [x] **R-36. Cache glyphes `paint_text`** — livré :
  `GlyphCache` persistant (module `glyph_cache`,
  `Send` — `Arc`/`OwnedFace`, voyage dans `PaintPayload`
  détachée) : faces `ttf_parser` possédées + contours de
  glyphes en unités fonte (upem) réutilisés d'un repaint
  à l'autre — `Path::transform` applique le même
  `map_point` par point que l'ancien sink transformé
  (équivalence pixel assertée). Variantes additives
  `rasterize_cached` / `rasterize_dirty_cached` ; les
  signatures historiques délèguent à cache jetable.
  Invalidation : `FontSource::revision` (méthode de
  trait additive, `0` figé par défaut) — bumpée par
  `WebFontSource::register`/`clear`, relue à chaque
  passe ; une révision changée vide tout (pointeurs de
  faces recyclables). `PaintPayload.font_cache` câblé.
  **Mesure honnête (`sync_profile`)** : ~0 % de gain —
  165.5 ms/passe neuf *vs* chaud sur page dense en texte,
  1.22 s sur la fixture 3 000 nœuds. Le poste dominant
  du raster est le **fill scanline** de tiny-skia
  (`fill_path`), pas `outline_glyph`/`Face::parse` —
  la prochaine étape perf texte n'est pas un cache de
  chemins mais un **atlas bitmap de glyphes** (blit de
  masques pré-rendus — exige une équivalence de blend
  prémultiplié ±0 LSB ou une garde d'approximation
  documentée, travail séparé).
- [x] **R-37. Invalidation bornée display-list / raster**
  — sous-entrée « raster par dirty-rect » **livrée** :
  `rasterize_dirty` repeint uniquement les fenêtres de
  `changed_rects` par-dessus la frame précédente (calque
  blanc par fenêtre = fond UA, inflation 2 px CSS pour
  l'AA/overhang, fusion des recouvrements, repli plein
  cadre si dims/scale/borne ≠). `repaint_payload`
  consomme les régions — avant, elles n'alimentaient que
  les events SSE. **Équivalence d'abord** : 9 tests
  permanents `dirty_raster` assertent le strict égal
  pixel « régional == plein cadre » (couleur, déplacement,
  ajout/retrait texte, translate, overflow-clip,
  sous-contenu chevauchant, dirty vide, fallback, fusion
  disjointe-y) — la méthode a attrapé un vrai bug au
  premier atterrissage (tri des fenêtres qui avalait une
  région disjointe → `checkbox_toggles_then_submits`
  session rouge, régression verrouillée par test dédié).
  Mesure : doc 18 001 items, mutation localisée — plein
  cadre 2,10 s → régional **8,2 ms** (×256, pixels
  identiques). MediaSurface inchangée : les rects média
  restent relus du boxtree après relayout complet — le
  dirty ne borne que le raster, jamais le layout.
  — sous-entrée « **PNG streamé/borné** » **livrée** :
  `Frame::encode_png` réécrit sur `png::StreamWriter`
  (crate `png` 0.18, backend fdeflate identique à
  l'`CompressionType::Fast` d'`image` — R-33b préservé).
  La démultiplication prémultipliée se fait par bandes
  de 256 lignes dans un scratch borné (~`width*4*256`
  octets) — fini le clone RGBA monolithique (~50 Mio
  sur une page de 16 384 px). Les `IDAT` partent par
  chunks de ~256 Kio. API additive `encode_png_to(W:
  Write)` : le flux peut écrire directement vers une
  réponse HTTP/SSE sans matérialiser le `Vec`.
  Tests : équivalence bytes `encode_png`/`encode_png_to`,
  décodage pixel-exact en travers de la frontière de
  bande, démultiplication (a=0 → noir, 128/128 → 255).
  — sous-entrée « **validation `DAMAGE_INFLATE_CSS`** »
  **livrée** (option « bounds d'encre réelles » de la
  dette ci-dessous) : `ShapedRun.ink` = union des
  `ttf_parser::glyph_bounding_box` décalées par position
  de plume (calculée au shaping, `Arc`-partagée) ;
  `item_bounds` des items `Text` utilise l'encre réelle
  au lieu de la boîte de métriques — l'overhang glyphe
  (italique, accents, fontes plein-em type Ahem) n'a plus
  besoin d'inflation. `DAMAGE_INFLATE_CSS = 2` reste pour
  le seul AA sous-pixel et les encre non-glyphiques.
  Fixture `fixture_ttf_overhang` (test-support, additif) :
  encre mesurée débordant de 0.10/0.45/0.25em — à 20 px,
  9 px à droite et 5 px en haut > l'inflation : le test
  mutationnel `dirty_raster` (texte ↔ vide) passerait en
  reliques sans les bounds d'encre ; il est vert avec.
  Résidu de la dette : items non-texte sans bounds
  exactes (aucune encre débordante implémentée — pas de
  box-shadow/text-shadow/outline dans le moteur actuel).
  — sous-entrée « **display-list incrémentale** »
  **livrée** : `build_display_list_incremental(prev,
  tree_epoch)` copie les plages d'items des sous-arbres
  prouvés équivalents au lieu de les ré-émettre. Index
  boîte→plage : `DisplayList.spans` (`SpanMap` chemin
  d'enfants → `SpanRec{start,end,rect,off,sig,flags,
  bx_ptr,lines_ptr,kids_ptr,kids_len,escaped}`).
  Correspondance par **chemin d'index** (les `NodeId`
  sont remappés au reparse — instables) ; une plage ne
  se copie que si signature de contenu `box_sig`
  (kind/position/float/overflow/z-index/checked/
  intrinsic/texte/style/fragments) + taille identiques
  et **Δ uniforme récursif** (un descendant divergent —
  `fixed`, sale, restructuré — fait descendre le parent).
  `F_DEFERS` + `escaped` : les porteurs inline reportés
  restent drainés par l'ancêtre de portée (copie plate
  interdite, re-push des chemins). `off` séparé de
  `rect` → le scroll copie translaté et `fixed` garde
  son ancre viewport (`effective_off`).
  **Fast-path `tree_epoch`** : le relayout incrémental
  session mute `p.tree` **en place** — epoch inchangée ⇒
  même objet ⇒ identité par pointeurs `{bx_ptr,
  kids_ptr, lines_ptr, kids_len}` sans signature (les
  `*bx = fresh` de `relayout_subtree` allouent avant de
  libérer → adresses disjointes garanties ; feuille =
  ptrs dangling → signature toujours). Contrat
  auto-testé : en `debug_assertions` la signature reste
  vérifiée quand le fast-path tire — il a attrapé que
  `translate_y` déplace `frag.rect` in-place (géométrie
  retirée de `box_sig` — dérivée, couverte par
  `s.rect`+Δ). `FontCache::shape` → `Arc<ShapedRun>`
  (runs partagés — mémoire dédupliquée) ; `box_sig`
  hache le **contenu** des glyphes (fonte chargée entre
  builds ⇒ glyphes différents ⇒ détecté).
  Côté session : `p.list` est stockée **non cullée** —
  `cull_display_list` purgeait les spans à chaque frame
  sur page longue (indices décalés) ; la cullée est une
  copie jetable réservée au plein cadre, le diff se fait
  non cullé (plus précis : plus de churn à la frontière).
  `tree_epoch` propagé `Tab`↔`PaintPayload`↔`PageSnapshot`
  (bfcache : même arbre ⇒ même epoch, ptrs valides).
  **Équivalence d'abord** : 15 tests `incremental_list`
  — dump item-par-item + frame pixel-à-pixel (identique,
  texte, restyle sans géométrie, translation rigide,
  non-rigide, scroll, fixed-sous-scroll, sticky, clip
  overflow, floats/positionnés, porteur inline+drain,
  image, insertion de frère = chemins décalés, frame
  session). La méthode a attrapé deux vrais bugs au
  premier atterrissage : drains inline perdus (span
  enregistré avant le drain → `F_DEFERS`/`escaped`) et
  `fixed` translaté sous scroll (`effective_off`).
  Mesure (release, doc 84 000 items / 60 002 spans) :
  scroll **74 ms vs 197 ms (×2,7)**, restyle locale
  142 vs 196 (×1,4), shift 161 vs 206 (×1,3) — le
  fast-path pointeur évite la signature ; la signature-
  contenu matche désormais **entre arbres** (glyphes,
  pas pointeurs).
- [x] **R-38. Mode quirks → cascade** (résidu terrain,
  triage `site_smoke`) — **livré** : les titres de
  news.ycombinator.com se rendaient centrés dans leur
  cellule au lieu d'alignés à gauche après le rang —
  page sans DOCTYPE (quirks) enveloppée de `<center>`,
  dont le `text-align: -webkit-center` UA traversait
  l'héritage jusqu'aux `td`. La spec (HTML Standard §
  rendering, « In quirks mode ») exige le reset
  `table { font-weight/font-style/font-variant/
  font-size/line-height/white-space/text-align:
  initial }`. Livré : `DocumentMode` (`NoQuirks`/
  `LimitedQuirks`/`Quirks`) porté par `Dom`
  (`document_mode`/`set_document_mode`, additif),
  synchronisé par le tree-builder dès la décision du
  DOCTYPE — la cascade le lit aussi sur arbre partiel
  (peinture progressive) ; `QUIRKS_UA_CSS` parsée en
  rang UA dans `StyleEngine::new`, ne candidatant que
  si `dom.document_mode() == Quirks` (zéro changement
  de signature — le mode voyage par le `Dom`). Tests :
  4 cas cascade (`text-align`, `font-weight`, `th`
  exempt du reset, `font-size` initiale, contrôle
  no-quirks) + 2 cas géométrie `inline` (fragment
  cellule à gauche en quirks vs centré avec DOCTYPE).
- [x] **R-39. `em`/`rem` dans les media queries** (résidu
  terrain, triage `site_smoke`) — **livré** : rust-lang.org
  se rendait comme une page mobile empilée à 1024 px —
  Tachyons (sa feuille) met **tout** le CSS desktop derrière
  `(min-width: 30em)`/`(min-width: 60em)` et l'app derrière
  `(min-width: 64rem)`, mais `condition_matches` ne
  résolvait que les `px` nus : toute feature non-px →
  fausse → colonnes/flex/grilles desktop morts. Spec MQ4 :
  `em`/`rem` en media query se résolvent contre la
  font-size **initiale** (jamais le `:root` calculé) —
  `env.font_size_base` (16 par défaut). Livré : résolution
  `px`/`em`/`rem` sur les six conditions `*-width`/`*-height`.
  Vérif terrain : `shot` rust-lang.org après fix = footer
  en 3 colonnes, liens orange, hiérarchie restaurée.
  Reste sur ce site (écarts séparés, triage suivant) :
  superposition hero/footer (hauteur `vh` ?) et une bande
  blanche verticale (SVG/icône à gabarit intrinsèque
  hostile). Test : `media_query_em_rem_resolus` (30em=480
  ✓, 64rem=1024 ✓, 100em=1600 ✗, seuil qui suit la base).
- [x] **R-40. Ratio intrinsèque des remplacés (§10.6.2)**
  (résidu terrain, triage `site_smoke`) — **livré** : la
  bande blanche verticale de rust-lang.org était ses 4
  icônes sociales SVG empilées — `img{max-width:100%}`
  (Normalize/Tachyons) bridait la largeur à celle du
  `<a>` flex-shrinké (~70 px) mais la hauteur restait
  à l'intrinsèque (511 px) : chaque icône s'étirait en
  un blob blanc ~70×500, empilées ≈ 1800 px de bande.
  Spec §10.6.2 : un axe `auto` d'un remplacé dérive de
  l'axe utilisé via le ratio intrinsèque. Livré dans
  `layout_block` : `h_css` conservé sans le repli
  intrinsèque ; `width:auto` + `height` spécifiée →
  `w = h × iw/ih` ; `height:auto` → `h = w_utilisée ×
  ih/iw` calculée **après** le bridage `min|max-width`
  (le clamp entre dans la dérivation, comme la spec).
  Vérif terrain : `shot` rust-lang.org → icônes carrées
  proportionnées, bande disparue. Reste sur ce site :
  superposition hero/footer (écart séparé) et taille/
  disposition des icônes (probablement liée au flex
  shrink des `<a>`). Tests : 4 cas `block` (max-width
  bridant → h dérivée, height → w dérivée, deux axes
  explicites inchangés, forme réelle `<a><img>` atomique
  inline portrait 480×511 → ~215×229 au lieu de 215×511).
- [x] **R-41. Flex colonne : base `%` et `grow` en hauteur
  indéfinie** (résidu terrain, triage `site_smoke`) —
  **livré** : le pied de rust-lang.org se peignait **sur**
  le héros — `body{display:flex;flex-direction:column;
  min-height:100vh}` + `main{flex:1}` : le shorthand
  pose `flex-basis:0%`, qui contre une taille main
  indéfinie (`min-height` non résolu → `height:auto`)
  résolvait `0` au lieu de `content` (§9.2) ; l'espace
  libre indéfini ne distribuait rien (§9.9.2 : items à
  `grow` → max-content). `main` contribuait donc 0 au
  `pen_y` → footer posé à l'origine de main = overlap
  complet du héros. Livré : `item_flex` reçoit `cb_def`
  (taille main définie) — base `flex-basis`/`height` en
  `%` contre indéfini → `content` ; colonne `h_def` absent
  → `grow>0` → hauteur mesurée (passe 0). Vérif terrain :
  dump boxtree → footer y=5226 (après main) vs 316
  (sur main) avant ; `shot` = page complète enchaînée
  nav→héros→sections→pied. Tests : 3 cas `modern`
  (`flex:1` indéfini, `flex-basis:50%` indéfini,
  `flex:1 1 0px` indéfini). Reste sur ce site : unités
  `vh`/`vw` jamais résolues (`min-height:100vh` ignoré —
  invisible ici car contenu > viewport), marges `<p>`
  qui collapse à travers l'item flex, icônes sociales
  empilées.
- [x] **R-42. Unités viewport `vw`/`vh`/`vmin`/`vmax`**
  (résidu terrain, triage `site_smoke`) — **livré** :
  l'enum `Unit` les parsait mais `px_factor` renvoyait
  `None` → toute longueur viewport s'évaporait
  (`min-height:100vh` du sticky-footer rust-lang.org,
  `max-width:90vw`, héros `height:100vh` — famille de
  R-39). Livré : `BlockCtx.viewport_w` (additif,
  défaut 800 — `viewport_h`=600 existait) ; résolution
  dans `raw_length_px` ET `eval_calc` (`calc(100vh-60px)`
  marche) ; media queries `condition_matches` résout les
  4 unités contre `env.width_px`/`height_px` (MQ4 §6.4) ;
  `font-size:Nvw` résolu en cascade via `env` (valeur
  calculée absolue). Carve-out aval : `explorer-core`
  câble `viewport_w`/`viewport_h` depuis `ctx.viewport`
  dans les deux `BlockCtx` (avant : `viewport_h` restait
  au défaut 600 — `position:fixed`/ICB aussi plus justes).
  Tests : 2 cas cascade (`(min-width:50vw)`, `font-size:2vw`)
  + 2 cas modern (4 unités 800×600, `calc(100vh-60px)`=540).
- [x] **R-43. `box-sizing` (css-sizing-3 §4)**
  (résidu terrain, triage M-H1) — **livré** : le
  bouton Connexion de google.fr rendait ~60 px
  au lieu de 40 — `min-height:40px` +
  `padding:10px 0` + `box-sizing:border-box`
  (universel `*{}`) : toutes les tailles
  spécifiées étaient traitées en content-box.
  Livré : `box_size_to_content` convertit
  `width`/`height`/`min-*`/`max-*` border-box →
  contenu (plancher 0) aux 4 sites de
  `layout_block`, `abs_geo`, bases `item_flex`
  (châssis résolu avant), mesure de piste grid
  `auto`, `width` explicite dans `content_px`.
  Write-backs flex compensés (`sizes` internes
  = contenu → écriture border-box équivalente,
  `layout_block` reconvertit et retombe sur la
  cible). Tables inchangées (`width` y est déjà
  border-box, §17.5.2). Vérif terrain :
  `shot` google.fr → bouton ~40 px.
  Tests : 4 cas `block` (width, min-height
  châssis>valeur → border box 40, height<
  padding, max clampés) + 2 cas `modern`
  (`flex:none`+width bb, `flex:1` grow bb).
- [x] **R-44. `width` explicite d'un remplacé
  fait foi dans la mesure intrinsèque (§5.1)**
  (résidu terrain, triage `site_smoke`) —
  **livré** : les 4 icônes sociales de
  rust-lang.org s'empilaient une par ligne
  dans le pied. `footer img{width:40px}` :
  `content_px` retournait l'intrinsèque (512)
  **avant** la `width` spécifiée → les `<a>`
  flex mesuraient 512 de base, chacun >
  conteneur (~215) → `flex-wrap` pose une
  ligne par item (§9.5 : la ligne se décide
  sur la base hypothétique, **avant** le
  shrink qui les aurait fait tenir). Livré :
  `width` non-% d'abord, intrinsèque ensuite ;
  `min|max-width` non-% bornent la source
  retenue (les `%` indéfinis retombent sur le
  contenu, §5.2). Généralise à toutes les
  mesures : bases flex, pistes grid `auto`,
  flottants shrink-to-fit, cellules de table.
  Vérif terrain : `shot` rust-lang.org → les
  4 icônes en ligne horizontale. Tests : 2 cas
  `block` (4×`<a><img width:40>` 512² dans un
  wrap 200 → une ligne ; `max-width:30px` px
  borne l'intrinsèque).
- [x] **R-45. `float:right` translate le
  sous-arbre entier** (résidu terrain,
  thumbs Wikipedia) — **livré** : les
  vignettes ITN/OTD de en.wikipedia.org
  se peintaient au bord **gauche** de la
  colonne, le texte par-dessus. Cause :
  `place_float` corrigeait `child.rect.x`
  après le layout interne sans translater
  les descendants — la border box partait
  à droite, son contenu restait posé au
  bord gauche de la bande. Même motif que
  l'ancrage `right`/`bottom` des absolus :
  correction par `translate_subtree(child,
  delta, 0)` après placement. Le registre
  des bandes était sain (texte contracté
  correctement dans les repros) — seule la
  position finale du sous-arbre manquait.
  Vérif terrain : `shot` en.wikipedia.org
  → vignettes ITN/OTD/Help-us-improve à
  droite, légendes sous l'image, texte
  qui s'enroule. Tests : 2 cas `block`
  (`float:right` → descendant collé au
  bord droit ; `float:left` inchangé).
- [x] **R-46. Item flex/grid = contexte de
  formatage indépendant (§4.1 /
  §8.3.1)** (résidu terrain) — **livré** :
  les marges des descendants d'un item
  flex **s'évaporaient** — elles
  traversaient la boîte item (pas de BFC
  propre sur son style) sans être ni
  posées dedans ni comptées dans sa
  hauteur. Repro : `<p>` `margin:16px`
  premier fils d'un item column → posé à
  `y=0` (marge perdue), item mesuré à
  19.2 au lieu de 51.2. Terrain : marge
  `1em` du `<p>` d'un item poussait le
  flex dans la fixture rust-lang
  (`main.y` décalé). Livré : champ
  `LayoutBox.fc_root` posé par
  `layout_item` (couvre flex **et** grid,
  même point d'entrée) — `is_fc_root` =
  `establishes_bfc(style) || fc_root`
  aux quatre gardes §8.3.1
  (`top/bottom_adjoining`,
  `margin_transparent`,
  `transparent_candidate`), à
  `own_bfc` (flottants englobés, §9.4.1)
  et à `child_bfc` (défensif). Champ et
  non paramètre : survit au relayout
  incrémental via `slot`. Tests : 2 cas
  `block` (marges contenues + item
  suivant après la margin box ; cross
  `row` englobe les marges verticales).
- [x] **R-47. `colspan`/`rowspan` — grille
  de table à occupation** (résidu terrain
  HN) — **livré** : les cellules étaient
  comptées 1×1 — la 2ᵉ rangée de
  `itemlist` HN (`<td colspan=2></td>`
  vide puis `subtext`) posait le
  sous-texte sous la colonne rang au
  lieu du titre. Livré : hints
  `-x-colspan`/`-x-rowspan` posés par la
  cascade (`td`/`th`, entier ≥ 1 plafonné
  1000 — `0` et invalides ignorés) ;
  `place_cells` assigne les colonnes en
  ordre visuel en sautant les colonnes
  retenues par les `rowspan` (HTML table
  model simplifié) ; la mesure d'une
  cellule à `colspan` se répartit en
  **déficit égal** sur les colonnes
  couvertes (§17.5.2.2, mono-colonnes
  d'abord) ; la boîte posée couvre la
  somme des colonnes/rangs traversés
  (+ arêtes `collapse` internes). Effet
  de bord réparé : le correcteur de
  résidu de la distribution ne retire
  plus jamais sous `min` — un span
  massif ne peut plus annuler une
  colonne voisine (la table déborde).
  Hors périmètre : distribution de la
  hauteur d'un `rowspan` entre ses rangs,
  `colspan=0`, `span` des `col`.
  Terrain : subtext HN aligné sous le
  titre. Tests : 4 cas `positioning`
  (forme HN, répartition de mesure,
  rétention `rowspan`, valeurs
  malformées).
- [x] **Écarts terrain — vol humain M-H1
  (22/10)** : trois constats rendu du
  premier run humain — **triage terminé** :
  - ~~**résultats DDG tronqués au bord
    droit**~~ → **non reproduit** : captures
    `shot` à 800/1024/1440 px sur les versions
    `html.` et `lite.` — les tables résultats
    reflow correctement, aucun clip. Le rendu
    moteur (`ddg-lite-r.png`) est quasi
    identique au frame utilisateur. Constat
    probablement transitoire ou propre à la
    version JS (redirigée vers `html.` chez
    nous) — non promu, à ré-observer au
    prochain vol.
  - ~~**entités littérales sur
    google.fr**~~ → **trié : bug NAV
    (jscore), pas PIPE** — entrée
    journal 2026-10-20. Le trou :
    `parse_fragment_into` n'appelle
    `decode_entities` que sur
    attributs + RCDATA, jamais sur le
    texte courant → tout markup
    injecté (`innerHTML`,
    `document.write`, parse miroir
    initial) sérialise `&amp;nbsp;`
    → littéral au re-parse. Mini-table
    sans `&copy;`. Repro prouvée.
    « Lornaeniaiae » : même famille
    (corruption charset/JS), non revu au
    re-shot — suivi côté NAV.
  - ~~**bouton Connexion
    surdimensionné**~~ → **confirmé PIPE** :
    `box-sizing` ignoré → promu **R-43**
    (livré ci-dessus).
  - ~~**frame décalée haut-gauche au
    survol d'un lien**~~ → **non confirmé
    PIPE** : l'hypothèse « restyle `:hover` »
    est éliminée — `matches_with_state`
    n'est appelée par aucun chemin session.
    Repros session propres : mutation
    `onmouseover` (texte, `setAttribute(
    style)`, overlay `absolute`, page
    scrollée) → repaint correct, aucune
    translation d'origine. Le symptôme
    réel relève probablement d'une
    mutation JS plus lourde (re-layout
    global) ou du slicing côté app —
    non promu, à ré-observer avec
    capture.
- [x] **Re-run `site_smoke` post-R-39→R-46
  (20/10)** : corpus 10 sites — **10/10
  OK, zéro régression**. Gains confirmés
  à l'œil : titres HN désormais alignés à
  gauche (R-38), vignettes Wikipedia en
  place (R-45), icônes rust-lang en ligne
  (R-44). Hauteurs document quasi
  stables ; octets PNG ↑ (contenu
  réellement peint, pas du blanc).
  **Nouveaux résidus notés** (non promus,
  à reproduire) : ~~(a) HN — la cellule
  `subtext` tombe sous la colonne rang au
  lieu d'aligner sous le titre (attribution
  de grille `colspan` sur la 2ᵉ rangée —
  candidat tables)~~ → **livré R-47** ;
  (b) ~~example.com —
  le `<svg>` injecté par `insertAdjacent
  HTML` (`/s.js`, logo 44×44) n'est pas
  peint alors que les `<p>` injectés
  oui~~ → **trié : bug NAV, PIPE
  exonéré** (re-triage 22/10).
  `navigator.languages` est absent du
  miroir → `undefined.some` → TypeError
  tue `/s.js` entre les deux
  `insertAdjacentHTML` : le premier
  (`afterend`, `<p>` multilingues)
  passe, le second (`afterbegin`,
  style+logo) ne s'exécute jamais —
  le svg n'entre pas dans le DOM.
  Chaîne vérifiée bout-en-bout :
  sérialisation miroir saine,
  re-parse `explorer-html` ajuste
  `viewbox`→`viewBox`, `collect_svg`
  rasterise le markup exact (usvg
  accepte `<use href>`), boxtree+
  display list émettent l'`Image`.
  Fix d'une ligne côté NAV
  (`languages` = array gelé) —
  entrée journal 2026-10-21.
- [x] **R-48. Piste `auto` grid :
  limite par PISTE, pas par item**
  (résidu terrain, second palier
  BBC) — **livré** : une grille
  sans template crée une colonne
  `Auto` implicite ; sous
  `auto-flow: row` plusieurs
  items y cohabitent (rangées
  distinctes). `col_widths`
  collectait une entrée par
  **item** : le libre était
  divisé par le nombre
  d'enfants et `w[col]` écrasé
  à chaque tour (dernier
  gagnant) — carte BBC 180px /
  3 enfants → piste 60px →
  titres/descriptions/métas en
  rivières de 60px. Livré :
  regroupement par colonne —
  limite = **max** des bases
  des items (§12.6), le libre
  se distribue entre pistes ;
  `w[col]` ne descend plus sous
  le plancher `lo` d'un
  `minmax(lo, auto)` dans les
  deux branches. Effet de bord
  bonus : la branche « libre ≤
  0 » prend elle aussi le max
  au lieu d'écraser. Terrain :
  dump BBC — `h2` 60×308 →
  **180×132**, `p` 60×468 →
  **180×144**, métadonnées en
  ligne (avant : empilées
  vertical-rivière). Test :
  `grid_colonne_auto_multi_items`
  (modern) — trois items en
  colonne implicite doivent tous
  mesurer la largeur conteneur.
  Reste noté (non promu) :
  d'autres zones BBC gardent du
  texte étroit (colonne
  droite/en-têtes de sections —
  structure différente, à
  diagnostiquer à part) ; deux
  `StyleEnv::default()` hors
  périmètre PIPE journalisés
  (`styles_payload` évalue les
  media à 1024×768 fixe —
  entrée journal NAV).
- [x] **R-49. `visibility:hidden`/`collapse`
  honoré à l'émission** (résidu
  terrain, second palier BBC) —
  **livré** : la propriété était
  en cascade (héritée, init
  `visible`) mais jamais
  consommée — le motif
  « visually hidden »
  (`visibility:hidden` + `width:1px`
  + `overflow:hidden` — accordéon
  `LanguageDropdown` BBC, sr-only)
  se peignait en rivières de 1px.
  Livré : `paint_box` interdit
  l'émission propre (`paint_self` +
  `paint_replaced`) aux boîtes
  `hidden`/`collapse`, **sans
  court-circuiter** transform/
  clip/descente — §11.2, un
  descendant `visibility:visible`
  explicite se repeint (le style
  calculé porte l'héritage →
  vérification locale par boîte).
  Incrémental couvert sans
  travail : `visibility` est
  déjà dans `box_sig` → un flip
  invalide le ré-usage. `collapse`
  ≡ `hidden` partout (le retrait
  de rangée de table reste hors
  périmètre). Terrain : le
  contenu du dropdown BBC
  (h3/h4 + ~50 liens langues)
  n'émet plus d'item ; tests
  `display.rs` — hidden supprime
  texte+fond en gardant la place,
  re-show descendant, collapse.
  Reste noté (non promu) : le
  hit-test ne filtre pas
  `visibility` (éléments cachés
  restent ciblables — inertes
  dans le cas BBC
  `pointer-events:none`) ;
  rivières résiduelles BBC =
  rangée de 12 IndexCards à 49px
  (carousel `auto-columns` ?) et
  `LondonMedia` 8.5px — causes
  distinctes à diagnostiquer.
- [x] **R-50. Blockification des items
  flex/grid (§9.7 / CSS Display
  §2.7)** (résidu terrain BBC —
  les deux « causes distinctes »
  de R-49, même racine) —
  **livré** : `styled-components`
  émet `display:initial` sur ses
  `GridItem` — la cascade résout
  le mot-clé CSS-wide en `inline`
  (valeur initiale de `display`),
  puis `wrap_children` appliquait
  `hoist_blocks` **avant** de
  savoir que le parent est un
  conteneur flex/grid : l'item
  inline contenant un bloc était
  scindé et son contenu hissé
  comme item auto-placé à 1 piste
  (`jqvHMb` LondonMedia = 15.5px
  = 1 col de `repeat(24,1fr)`
  sous `bKikLw` ; les 12 cartes
  manchester à 49px = items
  explosés). Livré : pour un
  parent `flex`/`inline-flex`/
  `grid`/`inline-grid`,
  `wrap_children` blockifie
  chaque enfant (`blockify_item`
  — kind `Inline`→`Block`,
  enfants repassés par
  `wrap_children` : hoist +
  anonymisation normale, ou
  blockification récursive pour
  `inline-flex`/`inline-grid`)
  au lieu de hoister — l'item
  garde sa cellule et son
  contenu. Terrain : 48 → 5
  boîtes étroites (les 5 restantes
  = le dropdown `visibility:hidden`
  à 1px, layout correct), carte
  london 15.5→488px, rangée
  manchester/hero/sections 4-col
  conformes au shot. Tests
  `modern.rs` : item inline→bloc
  en grid (cellule + contenu
  préservés) et en flex.
  Reste noté (non promu) :
  `display:contents` tombe encore
  sur le fallback `Block` — BBC
  s'en sort visuellement ici,
  support complet = flatten des
  enfants comme items (feature
  propre si un site réel le
  montre).
- [x] **R-51. Remplacé `width:%`
  hors mesure intrinsèque
  (§4.1/§5.1)** (résidu BBC —
  le « débordement carte LIVE
  sous le titre héros » noté en
  R-50, racine trouvée par
  instrumentation session) —
  **livré** : `content_px`
  retombait sur `bx.intrinsic`
  quand `width` était un
  pourcentage ; en mesure
  min/max-content le bloc
  contenant est indéfini (§4.1),
  donc un `img{width:100%}`
  imposait son bitmap entier aux
  ancêtres shrink-to-fit —
  `min-width:fit-content`
  d'`entAKy` = 480px dès
  l'arrivée du bitmap `src 480w`,
  débordant sa piste 236px et
  déployant le titre h2 sur
  ~430px par-dessus le héros.
  Latent : invisible sans tailles
  intrinsèques réelles (scratch
  propre, session cassée —
  d'où diagnostic par
  instrumentation
  `build_display_list` + chaîne
  d'ancêtres). Livré : un
  remplacé à `width:%`
  contribue 0 à la mesure
  intrinsèque ; bornes
  `min|max-width` non-%
  préservées ; non-remplacé
  inchangé (retombe sur la
  mesure du contenu).
  Terrain : titre LIVE et
  description wrappés dans la
  piste, héros propre au shot.
  Tests `modern.rs` :
  `img{width:100%}` ne gonfle
  plus `min-width:fit-content`,
  contrôle `width:auto` =
  l'intrinsèque mesure toujours.
  Reste noté : le premier mot
  du h2 est masqué par l'icône
  `InlineIconWrapper` (badge
  LIVE) — arrangement inline
  icon+titre à affiner (petit,
  non promu sans repro).
- [x] **Triage `site_smoke` post-R-48→R-51
  (batch BBC livré)** — corpus 10 sites
  re-tiré, défauts classés par famille
  avec mécanisme confirmé :
  - **`max_sheets: 8` (borne NAV) =
    défaut dominant du corpus** —
    GitHub : `marketing-header.*.module
    .css` est la 28ᵉ/30 feuilles liées
    (les 8 premières = variantes de
    thème `prefers-color-scheme`,
    filtrables par `media`) → nav
    mobile peinte dépliée, h2
    `.visuallyHidden` visible ;
    MDN : `styles-a11y-menu` (13ᵉ/20),
    `styles-navigation` (14ᵉ),
    `styles-menu` (16ᵉ) non fetchées
    → skip-links visibles, nav en
    colonne, boutons bordés. Repro
    hors-doute : index des feuilles
    dans le HTML servi. PIPE exonéré —
    entrée `interface-equipes.md`
    (filtrer `<link media>` via
    l'éval mq existante + revoir la
    borne). Lobste.rs : la barre rouge
    `var(--color-bg-accent)` vit dans
    `system-system.css` via `@import`
    — non résolu (couture fetch NAV) —
    même famille « complétude des
    sous-ressources CSS ».
  - **`mask-image` → fond plein =
    candidat R-52** : l'icône loupe
    Wikipedia (`cdx-text-input__start
    -icon`) = `mask-image:url(svg)` +
    `background-color:#202122` — la
    couleur devait passer par le mask
    ; sans support du mask le fond
    remplit la boîte entière → carrés
    noirs (loupe des deux recherches,
    avatar avant « Donate »). Motif
    universel des icon systems
    (Codex/Primer/Material) — la
    chaîne mask est absente du code
    (`mask-image`, `-webkit-mask-*`
    jamais lus). Réversible par un
    mask d'alpha tiny-skia au paint.
  - **« Appearance » Wikipedia =
    PIPE exonéré** : sur le DOM
    statique + vector.css réel, le
    landmark calcule `display:none`
    (unit-test scratch supprimé —
    règle `.vector-feature-appearance
    -pinned-clientpref-1 .vector-user
    -links .vector-appearance
    -landmark` + `.client-nojs`).
    Divergence session probable :
    `client-nojs`→`client-js` par le
    script startup MediaWiki et/ou
    relocation du menu « pinned »
    par JS — à confirmer par dump
    DOM de session si repro.
  - **`clip-path`/`clip` absents,
    impact borné** : le motif
    `.visuallyHidden` moderne
    (`clip-path:inset(50%)` + 1×1
    `overflow:hidden`) est déjà
    masqué par notre clip overflow
    (repro : PushClip 1×1 entoure
    bien le texte, raster honore les
    `Mask`) — `clip-path` ne change
    le rendu que sur les variantes
    sans dimensionnement+overflow.
    Candidat mineur.
  - **Triage soldé** (re-shots
    post-R-53/54/55 + Étape 96 NAV
    `<link media>`/`@import`/`max_
    sheets`) : weather.gov rendu
    quasi juste (héros, carte
    alertes, légende, mosaïque —
    le débord du titre était une
    conséquence des défauts
    corrigés) ; lobste.rs header
    correct (le « carré rouge » est
    le vrai logo — la barre ombrée
    n'existe que sous 480px) ; MDN
    lisible. Limite connue :
    `display:contents` reste repli
    `Block` (R-50) ; `clip-path:
    inset()` livré en R-58 (le
    skip-link MDN relevait en fait
    d'un bug de position statique
    absolue — R-60).
- [x] **R-52. `mask-image` borne le
  fond (motif icône)** (issu du
  triage — loupe Wikipedia, avatar
  « Donate », carrés noirs) —
  **livré** : le motif universel
  `mask-image:url(svg)` +
  `background-color` (Codex/Primer/
  Material) peignait un carré plein
  sans mask. Chaîne additive :
  `PaintImages.masks` (table
  additive, `box_sig` déjà couvrant
  via le style haché),
  `collect_data_images` lit
  `mask-image`/`mask` en `url(data:)`,
  nouveau `DisplayItem::MaskedFill`
  (fill rect + couleur + bitmap
  mask) émis par `paint_self` à la
  place du `FillRect` quand un mask
  décodé existe, raster = mask
  d'alpha tiny-skia échantillonné
  nearest dans la zone device,
  composé `min` avec le clip
  `overflow` actif. Repli tolérant :
  mask absent/indécodable →
  `FillRect` plein conservé.
  **`url_arg` décode désormais les
  échappements CSS** (`\"` dans les
  data-URL `utf8` sérialisées) —
  répare aussi les
  `background-image` échappées.
  Hors portée documentée :
  `mask-size`/`mask-position`
  (le motif icône dimensionne la
  boîte à l'icône), masks multi-
  couches, `mask-mode:luminance`.
  Tests `raster.rs` : alpha du mask
  borne le fond (pixels dedans/
  dehors), repli fond plein sur
  SVG indécodable.
- [x] **R-53. `width:%` d'un flottant
  résolu sur le bloc conteneur** (§10.3,
  §10.3.5) (triage weather.gov — colonne
  droite à ~1 car/ligne) — **livré** :
  `place_float` passait la **bande**
  résiduelle (`fl.band`) comme base `cb_w`
  à `layout_block` — un `float; width:100%`
  résolvait son % sur l'espace laissé par
  les voisins au lieu du bloc contenant
  (squelette `.div-full`/`.full-width-*`
  de weather.gov effondré à 0/300px).
  Livré : `layout_block_in_band` — `cb_w`
  = conteneur (base %, marges, bornes
  `min|max-width`, cohérent avec les
  marges pré-résolues de `place_float`),
  `band_w` ne borne que le shrink-to-fit
  `width:auto`. Terrain : carte d'alertes
  US pleine largeur, légende en 4
  colonnes lisibles, insets Alaska/
  Hawaï — la colonne écrasée disparaît.
  Tests `block.rs` : `width:100%` = 400
  du conteneur (pas 300 de bande) et
  descente sous le voisin ; `width:50%`
  = 200, reste en ligne à x=100.
- [x] **R-54. `grid-column:<ident>`
  résout `-start`/`-end` implicites**
  (CSS Grid §8.5) (triage MDN — cartes
  featured à ~115px) — **livré** :
  `placement` faisait un match exact
  des noms de lignes — `grid-column:
  content` avec `[content-start]`/
  `[content-end]` définis retombait en
  auto-placement (le squelette MDN
  `grid-template-columns:var(--layout-*)`
  + `.homepage > *{grid-column:content}`
  plaçait chaque enfant en piste 0).
  Livré : ident nu → repli `<ident>
  -start` côté début / `<ident>-end`
  côté fin ; ident seul → span bornée
  par `<ident>-end` si présente.
  Terrain : cartes featured + latest
  news + contributions en pleine
  colonne `content` (minmax(0,72rem))
  — lisibles. Tests `modern.rs` :
  ident nu `content` couvre la piste
  minmax, forme `a / b` sur
  `a-start`/`b-end`.
- [x] **R-55. `overflow` clippe les lignes propres
  de la boîte** (App. E — le clip recouvre le contenu,
  pas seulement les enfants) (triage lobste.rs —
  wordmark « Lobsters » peint sur la nav) — **livré** :
  un élément `display:inline` positionné
  (`absolute`/`fixed`) ou flottant est racine de son
  propre contexte inline — ses fragments vivent sur
  `bx.lines`, émis par `paint_self` AVANT le
  `PushClip` → le texte `.sr-only`
  (`position:absolute;w/h:1px;overflow:hidden`)
  fuyait sa boîte 1×1 et se peignait par-dessus le
  header. Livré : émission extraite en `paint_lines`,
  appelée dans la zone clippée ; l'ouverture du clip
  compte aussi `bx.lines` (une boîte sans enfants
  mais à lignes propres est bornée). Terrain :
  shot lobste.rs — wordmark masqué, seul le carré
  logo 1rem visible ; le fond de barre rouge reste
  une entrée NAV (`@import` non résolu — déjà au
  journal). Tests `display.rs` : texte `32px` d'un
  inline absolu `overflow:hidden` entre `PushClip`
  1px et `PopClip` ; variante `inline-block`
  (contraire — déjà juste, verrouillée).
- [x] **R-56. Media queries range MQ4
  `(width <= N)`** (Media Queries §6.4)
  (triage MDN — nav brut visible, tout le
  conditionnel tombé) — **livré** : une condition
  sans `:` comme `(width <= 1044px)` devenait un
  NOM de condition inconnu → query fausse — tout
  l'habillage range de MDN sautait (`display:none`
  du popup nav ignoré → menu brut peint ; styles
  mobile/desktop absents). Livré :
  `parse_range_condition` normalise en conditions
  `min-`/`max-`/`gt-`/`lt-` (`width >= N` ≡
  `min-width`, `width <= N` ≡ `max-width`,
  stricts `>`/`<` = bornes exclusives) — formes
  `(name op val)`, `(val op name)`, double borne
  `(v1 <= name <= v2)`, ops espacés ou collés.
  `condition_matches` gagne `gt-|lt-width|height`.
  Terrain : shot MDN — popup nav masqué
  (`data-open=false`), cartes en grille 2
  colonnes, footer 4 colonnes. Tests `cascade.rs` :
  `<=`/`>`/borne exacte, collage sans espaces,
  double borne, `and`.
- [x] **R-57. Custom properties `--*` en table
  `Arc` partagée** (demande NAV→PIPE au journal —
  ~1,1 Go de rétention commise mesurés Étape 95)
  — **livré** : `ComputedStyle` porte désormais
  `props` (HashMap plat, inchangé) + `vars:
  Arc<HashMap>` — un nœud sans `--*` propre
  référence la table effective du parent
  (héritage O(1) par bump refcount au lieu
  d'une copie O(vars) : ~200 tokens `:root` ×
  19k nœuds Wikipedia) ; un nœud qui déclare
  paie un clone-on-write unique
  (`Arc::make_mut` — la table parente est
  intouchée). `Arc` et non `Rc` : le payload
  de peinture traverse `spawn_blocking` —
  `Send` exigé. La substitution `var()`
  résout via un lookup « propres puis
  héritées » — plus de snapshot complet des
  vars par nœud. `set("--x")` = CoW isolé.
  Sémantique identique : `get`/`iter`/
  `PartialEq` exposent l'ensemble effectif.
  Mesure sonde `tab_mem_inventory`
  (Wikipedia « Rust », même page avant/après)
  : **RSS live 1249 → 205 Mio (−1,04 Go)**,
  drop tree 850→58 Mio, drop prev_styles
  244→21 Mio ; tailles `LayoutBox`/`Computed
  Style` +8 o. Tests : externes sémantiques
  (héritage/masquage/isolation `set`) +
  internes `ptr_eq` (chaîne sans déclaration
  = une seule allocation, déclaration =
  divergence CoW).
- [x] **R-58. `clip-path: inset()` minimal**
  (motif `.visuallyHidden` moderne — GitHub,
  skip-links MDN/Wikipedia) — **livré** :
  `clip-path` est parsé en cascade (propriété
  connue) et `paint_box` enveloppe désormais
  `paint_self` + tout le sous-arbre d'un
  `PushClip`/`PopClip` quand la valeur est
  `inset(<length-percentage>{1,4})` — syntaxe
  1→4 côtés façon margin, `%` contre la
  border box. Réutilise la mécanique
  `Mask` du raster (tiny-skia) — transform
  appliquée au clip comme à l'overflow.
  `inset(50%)` sur une boîte 20 px = clip
  0×0 (10 px retirés par côté). Hors scope
  v1 : `circle()`, `polygon()`, `path()`,
  `url()` — valeurs inconnues ignorées
  tolérantes, jamais de clip. Tests :
  display-list (`inset(25%)` → clip
  centré 10×10, `inset(50%)` → 0×0,
  tolérance `circle()`/`url()`),
  raster pixel (encre confinée, fond
  opaque intact hors clip). Terrain :
  le skip-link MDN n'était en fait **pas**
  un cas `clip-path` — voir R-60 ; le
  motif `.visuallyHidden` reste couvert
  pour les variantes sans 1×1+overflow.
- [x] **R-60. Position statique des absolus
  en-ligne** (trouvé en validation terrain
  R-58 — skip-link MDN visible) — **livré** :
  un enfant inline `position:absolute|fixed`
  n'émettait aucun fragment dans le flux
  en-ligne → `assign_child_rects` ne lui
  écrivait jamais de rect → `resolve_
  absolutes` lisait `rect.y=0` et `top:auto`
  ancrait en haut de page (§10.6.4 violé :
  sur MDN, les deux `<a>` de `.a11y-menu`
  peignaient superposés à y≈0 alors que le
  `ul` absolu vivait à −304 px). Livré :
  `assign_child_rects` pose désormais le
  rect hypothétique des enfants hors-flux —
  fin du dernier fragment d'un owner
  précédent, sinon début de première ligne,
  sinon origine du conteneur ; et dans
  `layout_block`, un enfant inline-level
  direct hors-flux ne consomme plus de ligne
  (placeholder statique comme les blocs).
- [x] **R-61. `merge_rects` linéaire**
  (dette journal NAV 55g — diff dégénéré
  ~8 500 rects ≈ 2 s) — **livré** : balayage
  trié par `x` — un rect ne se compare
  qu'aux composantes encore ouvertes
  horizontalement (compaction amortie des
  fermées) — puis consolidation bornée en
  point fixe sur les C ≪ k survivantes
  (une union qui a grossi vers la gauche
  peut rejoindre une voisine déjà fermée).
  O(k log k + k·ouvertes) typique vs O(k²)
  avant ; 5 000 rects chaînés < 1 s en
  debug. Bonus corrigé : l'ancienne passe
  unique n'atteignait jamais le point fixe
  qu'elle annonçait (union tardive pouvant
  rejoindre un rect déjà passé) — la
  consolidation le réalise désormais
  vraiment ; couverture identique, la
  partition rapportée peut fusionner plus.
- [x] **R-62. `rasterize_window`** (demande
  NAV repriorisée — cap 16 384 px bloquait
  le scroll sur github/MDN/stallman) —
  **livré** : `rasterize_window(list,
  fonts, scale, y_css, height_css)` et sa
  variante `_cached` (GlyphCache R-36)
  rasterisent la tranche verticale dans
  une frame `list.width × height_css` —
  le cap s'applique à la fenêtre, plus à
  la hauteur du document. Réutilise la
  mécanique `rasterize_dirty` : pixmap
  bornée, transform `p_css·scale − y0`,
  filtre « item × fenêtre » sur
  `resolved_bounds` (pile clip/transform
  intégrale rejouée). `y_css` négatif
  clampé à 0 ; hors document → blanc.
  Tests raster : équivalence ligne-à-
  ligne avec le plein cadre, tranche
  peinte sous un document 20 000 px que
  `rasterize` refuse.
- [x] **R-63. Cascade scopée au sous-arbre**
  (dette journal NAV 55g-suite — le coût
  unitaire d'un apply re-stylait tout le
  document) — **livré** :
  `compute_scoped(dom, root, env, parent)`
  + variante `_with_state` re-stylent le
  seul sous-arbre de `root`, semé sur le
  `ComputedStyle` du parent (héritage et
  `--*` traversent la frontière) ; les
  sélecteurs matchent contre le DOM réel
  complet (ancêtres/frères hors scope
  restent visibles aux `descendant`, `~`,
  `+`, `:nth-*`). `StyleMap::apply_scoped`
  fusionne dans la map stockée (autorité
  par nœud couvert — pseudos non reproduits
  purgés) ; `StyleMap::remove` pour les
  nœuds détachés. Contrat documenté : la
  portée du `root` est au choix de
  l'appelant (frères `:nth-*`/`~` touchés
  → root élargi). Tests : scoped ≡ global
  sur le sous-arbre, `--*` + sélecteur
  ancêtre traversant le scope, splice +
  purge pseudo + `remove`.
- [x] **R-64. `not` en parens MQ4**
  (trigger terrain Étape 97 — lobste.rs
  `dark-normal.css` filtrée à tort) —
  **livré** : `(not (f:v))`/`(not f:v)`
  → condition `not-<feature>` évaluée
  niée dans `condition_matches` ; le
  parse est factorisé en
  `parse_media_condition` partagé. Corps
  `not` multi-conditions (double borne
  range, `and` interne) non exprimable en
  AND joint → nom inconnu → « feature
  absente » → nié vrai (tolérance
  documentée) ; `or` imbriqué reste
  conditionné, jamais panic. Le shim NAV
  `media_attr_matches` hérite du fix via
  `media_matches` — `dark-normal.css`
  se charge désormais en env Dark.
  Tests : repro lobste.rs (dark applique,
  light saute), feature connue niée dans
  les deux sens, double borne tolérante.
  Test : `a` abs sans `top` dans `ul` abs à
  `top:-100px` → suit le conteneur (y≈−100),
  plus jamais 0. Terrain : les deux liens
  « Skip to main content »/« Skip to
  search » du MDN réel ont disparu du shot
  (diff pixel localisé y18-33).
- [x] **R-65. `FontCache` persistant
  entre passes** (attribution du mur apply
  55g-suite — `relayout_subtree` ~14 s /
  150 s mesurés NAV) — **livré** :
  `apply.rs` reconstruisait
  `BlockCtx::default()` par apply →
  `font_cache` froid → re-shaping de tout
  le texte posé à chaque tick (~5 ms/nœud
  texte debug — R-33b mesurait ~28 s/
  9 000 nœuds à froid, même magnitude).
  Couture : `BlockCtx::with_font_cache`/
  `take_font_cache` (transplant du cache,
  analogue au `GlyphCache` persisté dans
  le payload paint côté NAV). `FontCache`
  gagne la synchro `FontSource::revision`
  (contrat `GlyphCache` R-36) — un
  `@font-face` inscrit entre passes
  invalide octets+métriques+runs au lieu
  de figer la famille « inconnue ».
  Tests : compteur de résolutions —
  transplant = 0 appel supplémentaire,
  ctx froid = re-facture ; bump de
  révision invalide puis re-fige.
- [x] **R-66. `cols`/`rows`/`size` →
  dimensions en caractères** (trigger
  terrain — sonde thread HN : le
  `<textarea rows=6 cols=60>` de la
  page commentaire rendu ~135×64 px
  génériques) — **livré** : hints
  présentationnels `cols` → `width`
  en `ch`, `rows` → `height` ≈
  lignes ×1.2em, `size` → `width` en
  `ch` (input). Le dimensionnement
  sort de la feuille UA (`10em`/`4em`)
  vers les hints : une règle UA
  primerait l'attribut (hints =
  `or_insert` après les règles) —
  défauts émis en hint, visuel par
  défaut identique (10em ≈ 20ch).
  `input[type=checkbox]` 13px et
  `submit` `auto` restent des règles
  UA qui priment le hint. Test :
  `hint_cols_rows_size_en_caracteres`
  (attr, défauts, priorité règle UA,
  écrasement auteur) ; sonde HN :
  textarea ~540×130 px conforme.
- [x] **R-67. `FontCache` `Send`**
  (demande NAV journal 2026-11-08 —
  la recette R-65 bute sur le
  transport `PaintPayload` en
  `spawn_blocking`) — **livré** :
  `FontCache.data` `Rc<Vec<u8>>` →
  `Arc<Vec<u8>>` (tampons immuables,
  `Arc::as_ptr` conserve l'identité
  des clés `metrics`/`shape`) ;
  `font_data_of` idem. Le cache
  devient `Send`+`Sync` — stockable
  dans la payload migrée ; sert
  toujours sous `RefCell` dans
  `BlockCtx` (single-writer).
  Précédent : `Arc` retenu pour les
  `--*` en R-57 — même raison.
- [x] **R-68. `MAX_TOKENS` 256 K → 1 M**
  (trigger vol humain `dab313b` —
  nav mobile GitHub peinte dépliée) —
  **livré** : attribution mesurée —
  suspects éliminés un à un (fetch
  20/20 feuilles OK, cascade isolée
  sur DOM réel = `hidden`, paint
  `visibility` sain) jusqu'à la
  reproduction minimale : le blob
  concaténé (2,45 Mo / **384 K
  tokens**) dépassait la borne
  tokenizer ADR-0011 (256 K) →
  troncature silencieuse tuant la
  19ᵉ feuille sur 20 —
  `marketing-header.module.css` et
  sa règle `visibility:hidden` sur
  `.dropdown__jv_u56` absentes.
  Plafond relevé à 1 M tokens
  (marge ×2,7, ~40 Mo `Vec<Token>`
  au pire — borne conservée). Test
  `blob_geant_regle_apres_borne` +
  shot github.com : menu fermé.
  Note : la concaténation multi-
  feuilles est un choix session —
  le parsing par feuille garderait
  chaque feuille sous borne (max
  mesuré : 85 K tokens) — décision
  NAV si reprise. **Même vol,
  trigger (2) dev.to colonne ~15 % :
  vérifié sain** — irréproduit au
  commit du vol `f0ba062` comme à
  `main` sur les 5 breakpoints
  (768→1600, pipeline complet
  JS+net, shots byte-identiques) ;
  blob CSS 188 K tokens sous borne ;
  la chaîne `grid-template-columns:
  var(--layout)` → var() imbriquées
  résout. Suspect résiduel : frame
  transitoire avant `crayons.css`
  (406 Ko) sur le chemin série
  pré-102 — sous surveillance.
- [x] **R-59. `prefers-color-scheme` +
  défauts UA sombres** (moitié PIPE du
  thème sombre — journal d'abord,
  `StyleEnv` est le contrat NAV) —
  **livré** : `StyleEnv` gagne
  `color_scheme: ColorScheme` (énum
  publique — `Light` défaut, `Dark`,
  champ additif non cassant ; NAV
  branchera le réglage OS/app dans
  `viewport_env`). `condition_matches`
  évalue `prefers-color-scheme`
  (`light`/`dark`, feature nue → vrai)
  — le shim `media_attr_matches` NAV
  (étape 96) en hérite gratuitement
  pour `<link media>`/`@import`.
  `color-scheme` sur la racine
  (propriété héritée, valeur libre
  tolérée) arbitre les défauts UA
  sombres : schéma `dark` utilisé
  (env dark OU page qui force sans
  `light` déclaré ; `light`/`only
  light` = opt-out) → `background-color:
  #141414` + `color: #e6e6e6` posés à
  rang UA là où la page n'a rien
  déclaré — canvas propagé §14.2,
  texte hérité. Hors scope v1 :
  `light-dark()`, `<meta name="color-
  scheme">`, couleurs de liens dark.
  Tests cascade : MQ light/dark/nue,
  défauts posés/masqués/opt-out,
  héritage descendant.
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
- [x] **R-69. Fallback de fonte par glyphe** (trigger
  terrain vol 22/10 — tofus `□□□` sur les labels CJK du
  portail Wikipedia : 日本語/記事 → carrés). Mesuré —
  **deux gaps dans la chaîne** : (1) le scan fontdb
  (`explorer-render/src/fonts.rs:54`) ne retient que
  `ttf|otf` — quasi toutes les fontes CJK Windows sont des
  collections `.ttc` (`msjh.ttc`, `YuGoth*.ttc`,
  `simsun.ttc`, `msgothic.ttc`) → invisibles à la fontdb ;
  (2) `resolve` choisit **une** fonte par requête — glyphe
  absent → `glyph_id 0` → `.notdef` → carré : aucune
  tentative de repli sur une autre fonte de la db.
  **Livré** — segmentation au *shaping* (mieux que la
  piste « un item par segment » : les mesures restent
  cohérentes) : `.ttc` multi-face scannées à leur index,
  `FontSource::fallback_for` + `ShapedRun.fallback_spans`,
  raster via `GlyphCache::face_at` — sonde terrain :
  日本語/Русский/العربية peints sous fontes Windows
  réelles. **Activation session** : `SharedFonts` doit
  déléguer `font_face`+`fallback_for` (6 lignes, demande
  journalisée → NAV).
- [x] **R-72. Panic `selector` — découpe byte-index sur UTF-8**
  (priorité lead — **crash site réel**, dépasse R-69 ;
  repro mesurée vol terrain post-108 : `cli shot
  https://en.wikipedia.org/wiki/Main_Page` → panic
  `selector.rs:869` « start byte index 32 is not a char
  boundary; it is inside 'õ' » → site_smoke 9 OK / 1 FAIL).
  `attr_matches` découpe `actual` en **octets** :
  `&actual[..expected.len()]` (`DashMatch` :861,
  `Prefix` :865) et `&actual[len - expected.len()..]`
  (`Suffix` :869) — un attribut commençant/finissant par
  un caractère multi-octets fait paniquer le matching.
  **Livré** — `actual.get(..)` aux trois sites (`None` =
  `false`, sémantique inchangée) ; régression
  `match_attributs_utf8_ne_panique_pas` (16 combinaisons
  `^=`/`$=`/`|=`/`~=`/`*=`/`=` sur valeurs `ç…õ`). Validé
  terrain : shot en.wikipedia.org/wiki/Main_Page produit
  la frame complète (9,6 Mo) au lieu du panic.
- [x] **R-70. Texte coupé au bord droit de la frame**
  (trigger shot `cli shot www.wikipedia.org` post-Étape 108,
  1024 px — « Sear[ch] » tronqué en plein glyphe, « Read
  Wikipedia in your langua[ge] » poursuit hors viewport ;
  même symptôme relevé en vol 21/10 sur github.com). Sonde
  d'abord : mesurer si la boîte déborde le viewport (layout —
  largeur intrinsèque du rang input+bouton+lien > conteneur,
  overflow non clippé) ou si le texte déborde sa boîte (paint
  — `white-space`/`text-overflow`/`overflow:hidden` non
  honoré). Promouvoir en étape nommée après attribution
  boxtree vs display.
  **Livré** — deux défauts distincts attribués :
  (a) `paint_lines` émettait déco+texte entrelacés par
  fragment — le rect d'un fragment tardif, gonflé de
  `padding`, recouvrait l'encre du texte d'un frère
  (« langua[ge] » sous le fond du fragment espace du
  bouton). Deux passes : toutes les décos de la ligne
  puis tous les textes, et pièce **par élément** (union
  des fragments par `owner` — modèle `slice`, un fond +
  une bordure continus au lieu de carrés par fragment).
  (b) `text-indent` cascadait mais n'était jamais
  consommé — la technique image-remplacement
  (`.sprite{text-indent:-9999px}`) laissait le texte
  blanc peindre sous/hors son bouton bleu (« Sear[ch] »
  blanc-sur-blanc). Implémenté : première ligne seule,
  négatif admis ; `align_lines` recale sur le bord
  indenté (`ebx + indent`) pour ne pas neutraliser les
  valeurs négatives. Régressions
  `deco_inline_peint_sous_le_texte_de_la_ligne` (render)
  et `text_indent_premiere_ligne_seule` (layout).
  Validé terrain : shot portail — bouton unique bordé,
  texte complet, « Search » hors-écran. Résidus
  documentés : `em`/`rem` du padding inline ≈ font-size
  de l'élément (pas de base racine), pièces multi-ligne
  par union de ligne (pas par fragment).
- [x] **R-71. Sprites `background-image` non peints**
  (même trigger — les deux cases autour de « Read Wikipedia
  in your language » sont des rectangles vides sur le portail
  Wikipedia). **Cause** : les `.sprite` Wikipedia déclarent
  `background-image:linear-gradient(…),url(…)` — le garde
  `starts_with("url(")` de `paint_self` rejetait toute valeur
  multi-couche → aucun `DisplayItem::BackgroundImage` émis.
  Le reste de la chaîne était déjà sain : `css_url_args` NAV
  extrait le `url()` où qu'il soit (fetch/alias OK depuis
  l'Étape 98/108), `bg_position` accepte les offsets négatifs
  et `paint_tiled` échantillonne à l'origine décalée → la
  tranche d'atlas `0 -667px` est correcte dès l'émission.
  **Fix** : `bg_has_url` (scan `url(` insensible à la casse,
  même tolérance que l'extracteur de collecte) remplace
  `starts_with` aux 3 sites (peinture boîte, propagation
  canvas, `has_bg`). Sonde portail : 19 `BackgroundImage`
  émis pour 21 nœuds `.sprite`, origines = rect − position
  (`svg-search-icon` 957.92−747=210.92). Test régression :
  `background_image_multicouche_peint_la_couche_url`.
  Résidus : `linear-gradient` n'est pas rendu (couche
  ignorée — ici transparente, sans effet) ; `background-size`
  reste hors portée (atlas scalé ≠ 1:1 non géré) ; le shot
  PNG terrain attend que `explorer-core` recompile (WIP NAV
  `PendingApply.ids/journal` en vol au moment du fix).
- [x] **R-73. `linear-gradient` peint + shorthand `/size`**
  (issue #10 — vol comparatif : barre tricolore NPR et
  bandeau lobste.rs absents ; fixture lead confirmant deux
  trous : `background:linear-gradient(…)` et `background:red
  50% / contain no-repeat` **entièrement droppés** au parse).
  Deux causes au même endroit (`expand_background`) :
  (a) tokenisation `split_whitespace` éclatait
  `linear-gradient(to right, red, blue)` sur ses espaces
  internes → token inconnu → déclaration rejetée ; seul
  `url(`/`none` était admis en slot image — **fix** :
  tokenisation paren-consciente `bg_tokens` + slot
  `<bg-image>` élargi aux fonctions image (`*gradient(`,
  `image(`, `image-set(`) ;
  (b) aucune grammaire `<bg-size>` : `/` tombait en token
  inconnu → la déclaration `pos / size` était droppée, la
  couleur déclarée perdue — **fix** : `split_top_slash`
  (`/` collé ou séparé) + `bg_size_token`
  (`auto|cover|contain|length|%`, ≤2 valeurs), émission
  `background-size` + `background-origin`/`-clip` (slots
  `<box>`). Peinture : module `gradient` —
  `decode_linear_gradient` rasterise le dégradé à la taille
  de la zone de positionnement (pas de taille intrinsèque,
  §images-3) en un `DecodedImage` empruntant la machinerie
  `BackgroundImage` (position/repeat/clip) ; directions
  `to <side>`/coins, `<angle>` (deg/grad/rad/turn), stops
  `<color> [pos]` avec distribution spec des positions
  absentes + monotonie, `repeating-linear-gradient` par
  repliement du paramètre, borne 4096 px (hospitalité).
  `emit_bg_layers` mutualise l'émission par couches — ordre
  spec : la dernière couche déclarée peint d'abord (fond),
  la première en dernier (dessus). Tests : expand
  (`/size` séparé+collé, gradient à espaces, taille→reclasse
  normal, rejets préservés) + display (gradient `to right`
  pixels rouge→bleu, `to bottom` défaut via shorthand,
  `/size` → fond rouge peint, ordre multi-couches).
  Résidus documentés : `radial-`/`conic-gradient` parsés
  non peints ; `background-size` cascadé mais non appliqué
  au bitmap (déclaration préservée, image taille naturelle) ;
  `background-origin`/`-clip` émis non consommés ; stops à
  transitions dures (`color pos pos`) non gérés.
- [x] **R-74. Feuille UA listes + sélection de graisse**
  (issue #9 — vol comparatif : puces `ul` absentes, `h1` non
  gras, corps sans-serif au lieu de serif). Trois trous :
  (a) les puces n'étaient **jamais implémentées** — nouveau
  variant `DisplayItem::Marker { rect, kind, color }` +
  `emit_list_marker` (glyphe ≈ em/3 dans la gouttière,
  baseline de la première ligne ou repli font-size) ;
  `disc`/`circle`/`square` rasterisés en primitives
  tiny-skia (ovale plein/anneau 8 %, carré via
  `fill_guarded`) — les marqueurs textuels (`decimal`,
  alpha, roman) restent hors portée : la display list ne
  shape pas. Feuille UA : défauts spec §lists (`ul`→`disc`,
  `ul ul`→`circle`, `ul³`→`square`, `ol`→`decimal` — qui
  évite un disc trompeur sur listes numérotées) ;
  (b) `h1 { font-weight:bold }` était cascadé **mais
  ignoré** : `DirFontSource::resolve` matchait par famille
  seule, première face scannée. `face_meta` extrait
  désormais `usWeightClass` (OS/2) + italique
  (fsSelection/macStyle), `resolve`/`fallback_for` scorent
  style-puis-graisse (même formule que `WebFontSource`) ;
  `map_generics` retient le **nom** de famille (ses variantes
  de graisse restent candidates — `serif`+bold → Times Bold).
  Fixture `fixture_ttf_face` (test-support, additif) pose
  une table OS/2 v0 minimale (78 octets) ;
  (c) corps sans-serif : les prefs génériques élargies
  (times/georgia/cambria/dejavu…). Tests : faces par graisse
  (600→700, 500→400, italique), 6 tests marqueurs display
  (disc positionné gouttière, circle/square/none, ol sans
  disc trompeur, nested `ul ul`→circle, li vide → puce,
  pixels rouges à la couleur du li). Carve-out : bras
  `Marker` ajouté à `display_item_bounds` (explorer-core,
  match exhaustif — même commit).
  Résidus documentés : marqueurs textuels (`decimal`…) non
  peints (shaping requis) ; `list-style-position: inside`
  ne décale pas le texte.
  **Suite (shot `text.npr.org` de validation)** — la barre
  tricolore `.hr-line:after` absente : deux trous chaînés,
  (a) `box_kind_of` sans blockification §9.7 —
  `display:inline` + `position:absolute|fixed` → `Block`
  (`list-item` épargné : absent de la table) ;
  (b) garde `canvas_bg_src != bx.node` sautait le fond de
  toute boîte sans nœud — `None != None` → faux pour les
  pseudos. Shot : barre tricolore + puces `disc` visibles.

- [x] **R-75. Flottants/`margin:auto` — §10.4 rejoue
  §10.3.3** (issue #12 — vol comparatif : texte wikipedia
  « étranglé » en colonne étroite autour de l'image,
  colonne npr non centrée). Deux trous, même article de
  spec :
  (a) **intrinsèque non bornée** — `content_px` ne
  bridaient `min|max-width` que sur la branche « taille
  directe » (`width`/intrinsèque remplacé) ; la mesure
  d'enfants (flex, table-row, agrégat bloc) rendait le
  contenu brut. Sizing §5.1 : la contribution est bridée
  par les bornes propres — `thumbinner{max-width:170px}`
  du motif Wikipedia (`#mp-tfa-img` flottant shrink-to-fit,
  img 170 + légende ~350px max-content) gonflait le
  flottant père → bande résiduelle ~465px au lieu de ~615.
  `content_px` = wrapper borné sur `content_px_unclamped`
  (idempotent sur la branche directe) — boîtes `Text`
  exemptes (borne = celle du parent, appliquée à
  l'agrégat) ;
  (b) **`margin:auto` pré-clamp** — le slack allait aux
  marges `auto` sur la largeur tentative **avant**
  `min|max-width` ; §10.4 rejoue §10.3.3 avec la largeur
  bornée — `body{max-width:550px;margin:0 auto}` de
  text.npr.org restait à `x=0`. Distribution déplacée
  après bridage : partage égal / absorbante, slack
  négatif = débordement centré (comportement navigateur),
  `width:auto` sans borne → marges auto à 0.
  Validation terrain : shot `en.wikipedia.org/Main_Page`
  (texte coulant à pleine bande autour des 5 flottants
  réels — Priestfield, Menéndez, Hôtel de Ville,
  Metropolitan, candy store) + shot `text.npr.org`
  (colonne centrée). Piège noté : un `cargo build -p
  explorer-cli` interrompu avant link laisse un binaire
  périmé qui fait « échouer » une validation — vérifier
  `Finished` avant tout verdict terrain. 5 régressions
  dans `tests/block.rs` (centrage, absorbante, auto→0,
  bornes min/max du shrink). Résidu #9 soldé au passage :
  `BlockCtx::default_family` = `serif` (cohérent avec
  l'initiale cascade).

- [x] **R-76. `merge_rects` jetait les composantes
  fermées** (couture NAV, issue #11 bis — diagnostic
  complet et fix éprouvé fournis par NAV, re-numéroté
  R-76 après collision avec R-75). La compaction amortie
  `comps.retain` supprimait les composantes « fermées »
  — celles qu'aucun rect futur ne peut toucher (tri par
  `x`) — **sans les émettre** : sur un diff massif
  (~2 060 rects post re-parse lobste.rs), ~466 zones de
  damage disparaissaient → pixels modifiés jamais
  repeints, résidu `…:45:58` sous le timestamp relatif.
  Fix : accumulation dans `done: Vec<Rect>` puis
  réinjection avant la consolidation en point fixe
  (elles peuvent encore chevaucher une composante ayant
  grossi après leur fermeture — le point fixe les
  absorbe). Aucune API changée. Régression
  `merge_compaction_reemet_les_fermees` (70 fermées
  réémises, couverture préservée). Mesure NAV :
  `out` 39 → 505, `diff_px` raster 178737 → 0.
  Journal `AGENTS.md` §8 : nouveau piège noté — verdict
  terrain seulement après `Finished` de cargo build

- [x] **R-77. Coupure fantôme dans un flottant
  shrink-to-fit** (issue #13 — « PAST WEATHER » de la nav
  weather.gov coupé en deux lignes). Deux mécanismes
  d'ulp f32 chaînés : (a) la mesure par pièces du
  line-break dépasse le max-content intrinsèque (chaîne
  entière shapée une fois) de ~7e-6 → le flottant taillé
  pile voyait son texte « déborder » → `new_line` fantôme
  — tolérance `+ 0.01` aux tests de fit, déjà la convention
  du chemin atomique ; (b) `FloatCtx::band/band_over`
  rendaient `(cx+cw) - cx`, ulp perdu sur `cx`
  fractionnaire (origine négative `margin:auto`) — sans
  morsure effective, `cw` rendu intact. Régressions
  `float_shrink_to_fit_tolere_ulp_mesure_intrinseque` +
  `float_origine_negative_bande_rend_cw_exact`. Terrain :
  nav weather.gov sur une ligne. Résidu documenté dans
  l'issue : `letter-spacing` cascadé mais ignoré en
  layout → « ABOUT » retombe (comportement flottant
  conforme, support à faire). Les deux autres constats
  de #13 soldés sans fix : panneau `#myfcst` =
  `display:none` levé par jQuery (couture NAV/JS), logo
  NOAA = déjà peint (fond `body`).

- [x] **R-78. `letter-spacing` en layout** (§16.4, issue
  #13 suite — le résidu documenté de R-77). La propriété
  était cascadée/héritée mais ignorée de la mesure :
  `.topMenuNavList{letter-spacing:-.04em}` (~-45 px
  cumulés) laissait « ABOUT » retomber en ligne 2.
  `shape_face_ls` (additif — `shape_face`/`shape`/
  `measure` inchangés) ajoute `ls` à chaque avance de
  glyphe non nulle dans le signe de l'avance, **avant**
  la bbox d'encre ; `FontCache::shape` reçoit `ls` (clé
  étendue) ; `measure_fn`/`inline` le passent depuis le
  style via `letter_spacing_px` ; repli no-font borné
  ≥0. Régressions : clé de cache `ls` + avances signées
  (`tests/text.rs`), flottants resserrés/élargis
  (`tests/block.rs`). Terrain : les 9 items de la nav
  weather.gov tiennent sur une ligne, « ABOUT » compris.

- [x] **R-79. `DirFontSource` : génériques à nom espacé
  jamais matchés** (couture NAV→PIPE, issue #9 résidu).
  `map_generics` stockait le candidat brut (« times new
  roman ») alors que `f.families` est normalisé
  (`"timesnewroman"`) — `serif` tombait au repli
  (première face scannée = sans) → le corps CERN restait
  sans-serif malgré `default_family:"serif"` (R-75) ;
  `sans-serif`/`monospace` survivaient par accident
  (noms d'un mot). Fix NAV éprouvé en worktree :
  `norm(pref)` au stockage — une ligne. Régression
  `generic_a_nom_espace_resout_la_famille` (resolve +
  fallback_for). Terrain : corps `info.cern.ch` en
  serif — le résidu #9 est soldé côté PIPE.
  (binaire périmé = validation fantôme).
- [x] **R-80. `initial` sur `--*` = *guaranteed-invalid* +
  valeur `--*` vide + `content:/"alt"`** (issue #14 — trois
  trous d'un seul constat MDN). (1) css-variables-1 §3.2 :
  `initial`/`unset`/`revert`/`inherit` sur propriété custom →
  sentinelle GIV dans `vars` (masque l'héritage), filtrée au
  lookup `var()` → fallback / retrait — avant, `initial` →
  `""` rendait le toggle `light-dark` MDN « résoluble » en
  ` #fff` → texte blanc sur blanc (h3, contributions). (2)
  `--x: ;` : la valeur vide est valide (un whitespace seul) —
  le parseur la jetait → `[data-scheme=dark]` jamais armé →
  nav/héro sombres absents. (3) `content: v / "alt"` : le
  `/` de niveau 0 coupe le contenu peint de l'alt-text a11y —
  `pseudo_content` ne concatène plus `" (external)"`.
  Régressions : 5 cascade + 1 layout. Terrain : les 4
  symptômes soldés ; reliquat connu — `mask-image` non
  supporté (icône externe invisible au lieu d'un carré
  d'encre — dégradation discrète) + items du menu nav en
  masque (invisibles) → backlog fonctionnalité `mask-*`.
- [x] **R-81. `font_request` peinture = mesure** (demande
  NAV→PIPE, issue #9 4ᵉ défaut) : la display list repliait
  `font-family` absent sur `"sans-serif"` en dur (mesure :
  `ctx.default_family` = `serif`) et pondérait `bolder`/
  `lighter` en 800/300 (mesure : 700/400) → `<p>` nu peint
  sans sur mesure serif. Factorisation sans casse :
  `font::request_of_family` additif + `font::DEFAULT_FAMILY`
  (repli unique partagé), `display::font_request` délègue.
  Carve-out : `glyph_cache.rs` enregistre la face sous
  `serif`. Reliquat : propager `BlockCtx::default_family`
  surchargé à `PaintCx` reste ouvert (journal).
- [x] **R-82. Layout géant : faces rustybuzz mémoïsées +
  borne de runs relevée** (reliure « apply post-script
  ~250 s » du journal, étape 112 — coût unitaire PIPE sur
  article réel de 2,2 Mo / 32 803 nœuds). Le profil a
  localisé le mur : 115 776 appels `shape`, ~98 % du
  layout — et sa décomposition a renversé le diagnostic :
  le shaping pur ne pesait que ~10 s ; le reste =
  `hb_font_t::from_face` reconstruisant les digests de
  couverture GSUB/GPOS à **chaque** run unique (~2,6 ms ×
  ~20 k ≈ 50 s), le parse `ttf_parser` doublant la facture,
  et `SHAPE_CACHE_CAP=8 192` saturant en cours de document
  → ~33 k re-shapings parasites (~115 s de recalcul pur).
  Fix : carte `buzz` du `FontCache` — `(identité octets,
  index .ttc)` → `rustybuzz::Face<'static>` sur octets
  figés par `Box::leak` **sous** borne `BUZZ_FACE_CAP=64`
  (fuite assumée bornée ; son `Deref` vers
  `ttf_parser::Face` sert aussi métriques et bbox d'encre
  → parse unique par fonte) ; `SHAPE_CACHE_CAP` relevée à
  65 536 (~20 k runs uniques mesurés). Hors borne ou fonte
  hostile → repli au chemin par octets **identique**
  (couverture/fallback inchangés — borne de performance
  uniquement, régression
  `cap_buzz_preserve_la_couverture`). Résultat mesuré :
  `layout_tree` **181 s → ~17 s** (×10,7), pipeline total
  ~57 s. Sonde de profil conservée :
  `tests/zz_profile_giant.rs` (ignorée — fixture
  `target/probes/brasil.html` non versionnée, skip
  silencieux). Reste : `compute_styles` ~12 s et
  `raster`/`encode_png` ~13 s — prochains postes si le
  coût unitaire redevient le goulot.
- [x] **R-83. Styles géants : index de sélecteurs + filtre
  d'ancêtres Bloom + mémo de matching** (`explorer-css` —
  suite de la même reliure : le mur suivant mesuré par la
  sonde géante). `compute_styles` ~13,9 s = table de
  règles balayée par nœud (×3 avec pseudo). Livré :
  `SelectorIndex` (seaux id/classe/attr/tag/universel sur
  la clé du composé final ; `has_before`/`has_after`
  sautent le calcul pseudo quand aucune feuille n'en
  produit) → 189 k candidats essayés au lieu du balayage
  complet ; filtre de Bloom 128 bits des ancêtres (tag,
  id, classes, noms d'attributs — « ancestor filter » des
  moteurs, entretenu à la descente, aucun faux négatif) →
  lookups ancêtres 1,56 M → 791 k ; `MatchCache` partagé
  `(nœud, sélecteur, composé)` + rangs fraternels
  `:nth-*` mémoïsés (test composé 1,0 s → 0,47 s) ;
  attributs empruntés `attributes()` (fin des clones
  `attribute()` dans la boucle chaude), skip des
  sélecteurs dominés en spécificité, `&style` prêté aux
  enfants (clone parent/nœud supprimé), `var()` sauté
  sans `var(`, `deep` par niveau de composé au lieu de
  par pas d'ancêtre. Mesuré : **13,9 s → ~3,2 s** (×4,3),
  pipeline ~38 s (reliquat étape 112 ~250 s → ×6,6 avec
  R-82). Régressions
  `r83_filtre_ancetres_sans_faux_negatif` +
  `r83_saut_des_calculs_pseudo` ; invariants préservés :
  pureté DOM/`state` pendant la passe, cap du cache
  (dégradation perf seule), quirks sur table/index
  séparés. Reliquat documenté : héritage/mots-clés ~1 s
  (interning des noms de propriétés), `raster` ~6,7 s,
  `encode_png` ~4,2 s, `layout_tree` ~14,3 s — prochains
  postes par ordre de mur.
- [x] **R-84. Layout géant : bbox d'encre mémoïsée par
  (fonte, glyphe)** (`explorer-layout`, reliure étape 112,
  même sonde) : `glyph_bounding_box` re-parsait le contour
  glyf/CFF à chaque glyphe de chaque run (~2,4 s mesurés —
  « e » relu des milliers de fois) alors que la bbox est
  constante par (fonte, face, glyphe). Livré : carte
  `FontCache::ink` `(identité octets, face, glyphe) →
  Rect` brute (le scale est appliqué au cumul, pas stocké)
  bornée `INK_CAP=65 536`, vidée avec `faces` dans
  `sync()` (clés = pointeurs), partagée par le chemin
  primaire `shape_parsed` et les segments de repli
  (clé par segment — les fontes de fallback couvrent
  leurs glyphes sans re-parse). Hors borne → lecture
  directe, jamais de couverture perdue. Mesuré :
  `layout_tree` **14,3 s → 12,2 s**, pipeline ~35 s.
  Reliquat : `rustybuzz::shape` ~10 s = plancher debug
  séquentiel (la parallélisation exigerait l'ensemble des
  clés avant itemisation — chantier architectural
  séparé), `raster` ~6,4 s, `encode_png` ~4,0 s.
- [x] **R-85. Raster géant : atlas de glyphes rendus**
  (`explorer-render`, reliure étape 112 — quatrième mur
  mesuré) : décomposition par variante → **texte = ~95 %
  du raster** (76 k items) ; par glyphe : clone du contour
  + `transform` + `fill_path`, et sur frame > 8 191 px le
  `DrawTiler` de tiny-skia re-clone+transforme le path
  **par tuile** (les vides comprises — TODO amont). La
  plume étant snapée sur la grille device, la couverture
  est position-indépendante → `GlyphCache::rendered` :
  `(face, glyphe, échelle device, couleur) → RenderedGlyph`
  (pixmap couleur×couverture + offsets) rendue une fois
  via `Mask::fill_path` — même rasterizer, couverture
  bit-exacte — puis blit src-over prémultiplié par pixel
  (clip appliqué en facteur d'alpha ; `±1/255`, sous la
  tolérance reftest). `None` figé = pas d'encre ou hors
  gabarit (`RENDER_MAX_DIM=2048`) → repli `fill_path` ;
  `RENDER_CAP=8192` au-delà → idem (perf seule). Chemin
  non axis-aligned inchangé. Mesuré : `raster` **6,4 s →
  0,6-0,8 s** (×8-10) ; pipeline ~24 s (reliquat étape
  112 → ×10,5 cumulé). Régressions
  `atlas_meme_texte_meme_empreinte_a_deux_positions`
  (invariant de position) +
  `atlas_repaint_a_chaud_reproduit_la_frame`
  (bit-identique). Reliquat : `layout_tree` ~12 s (buzz
  ~10 s plancher debug), `encode_png` ~4,0 s,
  `compute_styles` ~3,0 s.
- [x] **R-86. `mask-*` : moitié domaine — pseudos,
  placement, couture fetch** (`explorer-render`, backlog
  issue #14 / reliquat R-80 — icônes MDN invisibles ; motif
  caractérisé sur `mdn.html` = trois trous cumulés :
  `mask-image:var(--icon)` → `url(/static/*.svg)` fetchée
  sur `::before` + `mask-size:cover`). Livré :
  `PaintImages.masks_url` (clé = `url(...)` brut du style
  calculé) + `register_mask(url, img)` additif (point
  d'entrée NAV symétrique d'`insert_fetched_backgrounds`,
  alias url brute↔résolue côté appelant) ; lookup à
  l'émission `masks[NodeId]` → repli `masks_url[url]` →
  boîtes pseudo couvertes ; `mask-size` (`cover`/`contain`/
  `<len>`/`%` — `auto` = étirement, déviation assumée :
  comportement R-52 préservé), `mask-position`
  (`bg_position` réutilisé), `mask-repeat` (tuilage
  `rem_euclid`, défaut `repeat`, `space`/`round` ≈
  repeat) ; `MaskedFill` porte `dst`+`repeat` ;
  `collect_data_images` couvre les styles pseudo
  (`StyleMap::pseudo`). Formulation honnête : le motif
  devient *possible* — MDN reste invisible tant que NAV ne
  wire pas le fetch des `url()` de masks (demande au
  journal des coutures). Régressions :
  `mask_sur_pseudo_via_url`,
  `mask_size_position_repeat_placent_la_tuile`,
  `mask_repeat_defaut_u_tile`,
  `mask_fetchem_enregistre_via_register_mask`.

> **Clôture de la reliure « apply post-script ~250 s »
> (étape 112) — arbitrage du 2026-10-23** : mesures release
> de la sonde géante — pipeline complet **~2,3 s**
> (`layout_tree` 389 ms, ×31 vs debug : le plancher restant
> était la taxe debug du shaping, pas l'algorithme) ;
> « apply simulé » à caches fonte chauds ≈ **1,9 s**
> (layout chaud 213 ms). **R-86 (pré-shaping parallèle) non
> retenu** — premier thread en crate domaine non justifié
> par 389 ms ; de plus les clés de shape sont des fragments
> post-itemisation, non collectables sans rejouer la moitié
> du travail. Reliquat par-apply : `StyleEngine` reconstruit
> à chaque apply (~3 s debug / ~0,3 s release de cascade) —
> premier poste fixe si le coût redevient le goulot.
> Détail au journal des coutures.

- [x] **R-87. Hauteur de document = région défilante** (terrain
  humain 2026-10-23 — fr.wikipedia.org en fenêtre ≥1120 px :
  document réduit à un viewport, scroll impossible, bas de
  page clippé). Mécanisme mesuré : `html,body{height:100%}`
  fige `body` à la hauteur ICB ; le contenu déborde en
  `overflow:visible` (légal) sans étirer `body`. En dessous
  de 1120 px le bas survivait par les flottants remontant au
  `FloatCtx` racine (`layout_tree` étend `rect.height` à
  leur profondeur) ; à 1120 px Vector-2022 bascule
  `.mw-page-container-inner` en `display:grid` — BFC qui
  capture ses flottants — et plus rien ne remonte. Correct :
  `doc_h` = `scrollable_overflow_bottom` (≈ `scrollHeight`) :
  max des bords inférieurs de toutes les boîtes dont le
  débord n'est pas clippé (`overflow` ≠ visible borne ses
  descendants à sa bordure ; `fixed` exclu — ancré viewport).
  La même borne nourrit `DisplayList.height` et le rect de
  canvas dans `display::build_list`. Régression :
  `doc_h_covers_overflow_past_fixed_height_body`
  (session.rs — hauteur + scroll effectif).

- [x] **R-88. `rem` = font-size calculée de `:root`**
  (terrain 23/10 — portail Wikipedia ~+74 % de hauteur
  vs Chrome ; cause racine : `rem` se résolvait contre
  `env.font_size_base`, l'initiale 16 px, pas contre la
  font-size calculée de `documentElement` — CSS Values
  §5.2). `html{font-size:62.5%}` (→ 10 px, motif répandu)
  gonflait chaque `rem` ×1.6 : polices, marges,
  `line-height:Nrem`, `width`, `calc(…rem)`. Correctif en
  deux couches : **cascade** — `resolve_font_size` reçoit
  `root_fs` (le rem de `font-size` sur la racine reste
  l'initiale, spec ; ses pseudos résolvent contre la fs
  calculée) ; threading `&mut root_fs` dans `compute_subtree`
  + `parent_computed`, seed via `documentElement` pour les
  entrées unitaire/scopée. **Layout** —
  `BlockCtx.root_font_size : Cell<Option<f32>>` posée par
  `layout_tree` ; `raw_length_px`/`eval_calc` lisent
  `rem_basis()` (repli base quand non posé — relayout
  scopé : statu quo, propagation NAV au journal des
  coutures). Mesures : portail live doc 1917 → **1088 px**
  (Chrome ~1102) ; fixture `html` 5038 → 2411 px.
  Régressions : `font_size_rem_contre_racine_calculee`
  (cascade), `rem_contre_font_size_racine` (layout).

- [x] **R-89. Multi-colonnes MVP — `column-*`** (terrain
  23/10 — « `.liste-horizontale` en colonne » ; diagnostic
  corrigé : la vraie fautive est `.colonnes` avec
  `column-width:10em;column-count:3` inline — la boîte
  d'accueil FR empilait ses liens au lieu de 3 colonnes).
  `flow_columns` dans `block.rs` : passe de mesure
  mono-colonne à `col_w` (`FloatCtx` jetable, origine
  scratch), descente des chaînes mono-enfant jusqu'au premier
  niveau ≥2 enfants (`columns_place` — origine contenu
  suivie par niveau, maillons recadrés en border-box réelle),
  répartition gloutonne par milieu d'unité vs `span/n`,
  `translate_subtree` par colonne (`partition_units`).
  `column-gap` (`normal`=1em), `n=min(count,
  ⌊(w+gap)/(width+gap)⌋)`, borné à `content_w` (tolérance).
  CSS : `expand_columns` (`columns:` → longhands, ordre
  libre). Déviations documentées : pas de fragmentation
  intra-boîte (texte seul = unité insécable), pas de
  `column-span`, `fill:auto`=`balance`, unités jamais coupées
  (`break-inside` couvert de fait). Terrain vérifié : accueil
  FR rend 3 colonnes équilibrées (identique Chrome).
  Régressions : `colonnes_distribuent_les_items`,
  `colonnes_degenerent_sans_niveau_multiple` (layout),
  `shorthand_columns` (css).
- [x] **R-90. Rects des inlines imbriqués + atomes
  `inline-*`** (journal piste PIPE — bloquait le
  consentement Google, triage Étape 120 : clic au bouton
  peint → `ignored`, `hit_tag="form"` ; repro
  `repro_c.html` : le 2ᵉ `<input style="display:
  inline-flex">` recevait le rect du 1ᵉʳ). Deux défauts
  chaînés : **(a)** `is_atomic` n'exigeait que
  `inline-block` — étendu à `inline-flex`/`inline-grid`/
  `inline-table` (CSS §9.2.4, atomes de niveau inline à
  FC propre — même liste que l'`atomic_inline` de
  `wrap_children`/`hoist_blocks`) : la série `aid` reste
  synchrone de fait entre `size_atomics`/`collect_child`/
  `assign_atomic_rects` ; l'input coule en atome
  dimensionné par `layout_block` (flex réel à
  l'intérieur). Bonus cohérents : `box_kind_of` mappe
  `inline-grid` → `BoxKind::Inline` (dernière valeur
  atomique manquante — jamais de niveau inline, écart
  spec corrigé) et `layout_block` route `inline-table`
  vers `layout_table` (l'atome garde l'algo table
  dedans). **(b)** `assign_child_rects` comparait
  `f.owner == i` en récursion — or `owner` n'indexe que
  les enfants **directs** de la boîte à lignes : tout
  inline niché d'index i copiait le rect du iᵉ enfant
  racine (invisible sur le texte peint par fragment,
  mortel sur les contrôles hit-testés). L'union reste au
  seul niveau racine ; en profondeur
  `assign_nested_rects` attribue les fragments via
  `f.node` ∈ sous-arbre (HashSet — exact, `Text`
  inclus) ; la position statique des hors-flux nichés
  s'ancre au dernier fragment des frères in-flow
  précédents (`seen`), sinon à l'origine du parent.
  Imprécision bornée documentée : fragments `node: None`
  (pseudos) et marqueurs de `split_piece` à `node`
  partagé. Vérifié terrain : `layout repro_c.html` →
  inputs côte à côte (x=8 / x=120.4), chacun portant son
  libellé ; hit-test du 2ᵉ bouton → son `input`.
  Régressions : `controle_inline_flex_niche_est_
  atomique`, `inline_niche_recupere_ses_propres_
  fragments`, `inline_grid_et_inline_table_sont_
  atomiques`.
- [x] **R-91. `@layer` + `%` des positionnés** (journal
  piste PIPE — hero github : texte coupé ~800 px, colonne
  droite hors viewport au shot 1216 px). Deux défauts
  chaînés mesurés par bissection sur le fragment réel
  (`gh_mini.html` — `.lp-IntroHero` rendait 1312 px dans
  un bloc conteneur de 1200/1216). **(a) `@layer` absent** :
  GitHub enveloppe **tout son reset Primer** dans
  `@layer primer-css-base{…}` — at-rule inconnu →
  `skip_block` avalait le bloc entier : `*{box-sizing:
  border-box}`, `body{margin:0}`, resets de formulaires…
  tous perdus. `AtKind::Layer` (prélude `nom`/`a.b`/
  anonyme, règles imbriquées sous la borne `MAX_NESTING`) ;
  `StyleEngine` tient une table chemin→rang à l'**ordre de
  première déclaration** (`@layer a, b;` déclare sans
  corps, `a.b` ≡ sous-couche, chaque préfixe déclaré au
  passage) et la clé `(rank, layer, spec, order)` s'insère
  entre origine et spécificité — hors-layer bat tous les
  layers en normal, `!important` renverse (premier layer
  gagnant, hors-layer dernier ; segments complémentés +
  borne `MAX` pour que les règles propres d'une couche
  restent après ses sous-couches). `revert-layer` garde
  son repli documenté. **(b) `%` re-résolu sur les
  positionnés** : `abs_geometry` calculait la border box
  (`width:%` + châssis = 1264) puis la repassait à
  `layout_block` comme `cb_w` — la résolution interne de
  `width` s'y re-appliquait (1264 → 1312, content-box).
  La base passée est désormais `target.width` quand
  `width` est non-`auto` (px idempotent, % → vrai CB ;
  bases de `min|max-width` et marges `auto` corrigées au
  passage — un absolu `width:600px;margin:0 auto` se
  centre désormais dans son CB). Vérifié terrain :
  `layout gh_probe.html` → `.lp-IntroHero` 1216, hero
  1168, grid 1120, CTA « Download GitHub Copilot app »
  x=777→1065 **dans la frame** ; `body{margin:0}` et
  `*{box-sizing:border-box}` appliqués (div sonde 1152).
  Régressions : `regle_en_layer_sapplique`,
  `hors_layer_bat_tous_les_layers`,
  `ordre_des_layers_prime_sur_specificite`,
  `ordre_des_layers_declare_par_statement`,
  `important_inverse_ordre_des_layers`,
  `layer_anonyme_et_imbrication`, `layer_sous_media`,
  `absolu_largeur_pourcent_content_box`,
  `absolu_largeur_pourcent_border_box`.
- [x] **R-92. Noms `--*` sensibles à la casse** (vol
  d'étalonnage corpus → F3, bissection du hero github).
  css-variables-1 §2.1 : les propriétés custom sont des
  identifiants **sensibles à la casse** — `--Foo` et `--foo`
  sont deux propriétés distinctes (les propriétés ordinaires,
  elles, sont ASCII-insensibles). La cascade normalisait tous
  les noms en minuscules à la déclaration : une `--*` à casse
  mixte était stockée minuscule et jamais retrouvée par un
  `var(--CasseExacte)` — tout `var()` à fallbacks chaînés
  s'effondrait au littéral terminal. Mesuré terrain : github
  `.lp-Spacer` lit `var(--Spacer-size1280, var(--Spacer-
  size1012, var(--Spacer-size768, var(--Spacer-size544,
  var(--Spacer-size, 156px)))))` avec les `--Spacer-size*`
  posées en `style=` (casse mixte + digits) — le spacer
  `.lp-IntroVisuals` s'écrasait à **156 px** au lieu de 660
  (≥1012 px) / 600 (≥768 px), le `.lp-Intro` sombre perdait
  ~500 px de hauteur (491 au lieu de ~755/839) et les sections
  en flux remontaient *sous* le contenu du hero `fixed` →
  superposition onglets/paragraphe du vol. Un seul site fautif
  (normalisation des noms en cascade) ; lecteurs (`get`,
  `var()`) déjà spec-corrects (adressage verbatim). Vérifié
  terrain : `layout gh_probe.html` → `.lp-IntroVisuals-spacer`
  156 → **600** (≥768 px), `.lp-Intro` 755 (=114+20+600+21).
  Régressions : `var_nom_custom_sensible_a_la_casse`,
  `var_custom_casse_mixte_dans_style_inline`,
  `var_custom_casse_mixte_heritage_et_lecture`. Effet de bord
  attendu et voulu : toute `--*` à casse mixte du web réel
  (design tokens `--Brand-*`, `--Font-*`…) fonctionne
  désormais — les vars minuscules (`--base-size-*`) sont
  inchangées.
- [x] **R-93. `(scripting)` MQ5 + `<noscript>` UA** (re-baseline
  post-train CAP → vol d'étalonnage, issue #22). CAP-155 a
  rendu `noscript` rawtext sous scripting — sa feuille UA
  n'avait aucun moyen de le masquer : le texte brut peignait
  (mesuré wikipedia : `CentralAutoLogin` en clair en bas de
  page ; github : 15 img ctfassets perdues — mais attribution
  dominante là = matcher CSP path-source, issue #21 NAV).
  MQ5 définit la media feature pour exactement ça : `StyleEnv`
  gagne `scripting: bool` (additif, défaut `false` = `none`
  = comportement historique — même motif que `color_scheme`
  en R-59). `condition_matches` évalue `scripting`
  (`enabled`/`none`/feature nue = « pas none ») ; `initial-only`
  jamais rapporté — nos scripts tournent aussi après le
  chargement. UA sheet : `@media (scripting) { noscript {
  display: none } }` — sous scripting l'arbre rawtext
  (unique enfant texte) ne peint plus ; sans scripting le
  sous-arbre reste rendu (le seul `noscript` utile). Filage
  NAV côté `viewport_env` (`apply.rs`) : `env.scripting =
  ctx.scripting` — une ligne, journalée. Régressions :
  `media_query_scripting` (enabled/none/nue/initial-only
  sur les deux mondes), `noscript_masque_sous_scripting`
  (UA ne masque pas sans scripting, masque sous).
- [x] **R-94. `background` multi-couches (première couche)**
  (vol terrain — reliquat « flèches HN »). Le shorthand était
  strictement mono-couche : `url(image), linear-gradient(décor)`
  — motif très courant (image + dégradé décoratif, HN
  `.votearrow` : `url("triangle.svg"), linear-gradient(…)`)
  — collait la virgule au token `url(` puis le second
  `<bg-image>` faisait **tomber la déclaration entière** →
  boîte 10×10 vide, flèches de vote invisibles (logo
  `<img>` voisin OK → symptôme isolé à la boîte vide, frame
  vs layout). Fix : `split_top_commas` (paren-aware — les
  virgules *dans* les fonctions protégées) éclate les
  couches, `parse_bg_layer` valide chacune par la grammaire
  d'avant — **une couche invalide invalide la déclaration
  entière (spec)** — et la **première** couche (la plus
  haute, spec) fournit les longhands. Borne honnête :
  première couche gagne — le stacking multi-couches au paint
  (toutes couches compositées) et la couleur de la dernière
  couche restent l'arc. Régressions :
  `background_multicouche_premiere_gagne` (url+gradient →
  image conservée), `background_multicouche_invalide_drope`
  (couche 2 invalide → déclaration droppée).
- [x] **R-95. `background-size` au raster** (vol terrain —
  suite de la bissection « flèches HN »). R-94 avait rendu la
  déclaration visible : fetch, décodage SVG, alias et
  matching `images.backgrounds` fonctionnaient tous — mais
  `DisplayItem::BackgroundImage` dessinait la tuile à
  **taille naturelle** (`background-size` hors portée). Le
  triangle HN (SVG 32×32 à motif centré) n'échantillonnait
  que le coin `(0,0)` — transparent — dans sa boîte 10×10 →
  peint mais invisible. Fix interne `display.rs` (zéro
  changement d'API) : `bg_size` résout `background-size`
  (`auto` = intrinsèque, `cover`/`contain`, `<len>`/`<%>`
  1-2 valeurs — même grammaire que `mask_size`, défaut
  différent), `scale_tile` produit la tuile nearest-neighbor
  une fois à l'émission — `paint_tiled`, `bg_position` (dont
  le `%` mesure désormais `zone − tuile réduite`, spec
  §images-3) et l'item sont inchangés. Borné : tuile >
  16 Mpx → naturelle conservée (jamais d'allocation hostile) ;
  `background-size` par couche non représenté (valeur unique
  — le modèle multi-couches est déjà borné R-94). Régressions :
  `background_size_met_la_tuile_a_l_echelle` (render — pixel
  central rouge avec `10px`, blanc sans), et bout-en-bout
  `background_size_scales_fetched_image` (session — motif HN
  exact : `url(svg), linear-gradient(…)` + `size:10px`).

- [x] **R-96. Shadow DOM tranche 2 — scoping CSS + DSD**
  (arc T2 du plan CAP-191 ; T1 = surface JS côté NAV).
  Chaîne complète octets→pixels pour les arbres shadow :
  `explorer-dom` porte les tables latérales hôte↔racine
  (`attach_shadow_root`/`shadow_root`/`shadow_host`/
  `is_shadow_root`/`containing_shadow_root` — la racine est
  le fragment `template_contents`, zéro nœud synthétique) ;
  `explorer-html` attache `<template shadowrootmode=open|
  closed>` à la fermeture (et au EOF) — énuméré HTML
  insensible à la casse, valeur exotique = template
  ordinaire, double racine refusée ; `explorer-css` gagne le
  confinement : `CollectedRule.scope`, `add_scoped_css`,
  matching `:host`/`:host(c)`/`:host-context(c)`/`::part(n)`
  (sujet `::part` = nœud `part` en shadow, les autres
  simples qualifient l'hôte — un seul boundary franchi),
  filtre d'arbre dans `collect_candidates` (document ↛
  shadow, racine ↛ autre racine ; `:host*` sujet = seule
  porte sortante, `::part` = seule porte entrante), marche
  de combinators qui ne franchit la frontière que vers un
  composé `:host`, héritage + `--*` qui traversent via le
  style de l'hôte ; `explorer-layout` peint le flat tree
  (`child_boxes` lit les enfants de la racine — light DOM
  non distribué non peint, `<slot>` = rien en attendant la
  distribution T3). Borné honnête : pas de `::slotted`, pas
  d'`exportparts`, distribution = T3, retargeting events =
  T4 (NAV). Régressions : 3× DSD html, 11× cascade scoping
  (`:host*`/`::part`/isolation/héritage), flat-tree boxtree,
  `shadow_root_tables` dom — + transport NAV↔PIPELINE :
  `serialize_keyed` miroir émet `<template shadowrootmode>`
  (canal synchro seul — `innerHTML` reste muet, spec),
  `collect_style_sheets` retourne `(scope, css)`.

- [x] **R-97. Shadow DOM tranche 3 (PIPE) — flat tree /
  distribution `<slot>` + `::slotted`** (répond au trigger
  « atterrissage T3 » de la note d'anticipation — T3 miroir
  livré par CAP-197). `explorer-dom` porte la relation
  d'assignation partagée : `slots_in` (préordre dans la
  racine), `slot_assigned` (enfants light dont `slot` égale
  `name`, défaut `""` + textes → slot défaut, template DSD
  consommé exclu — la spec le retire de l'arbre),
  `assigned_slot`. Layout : `child_boxes` distribue — les
  enfants light assignés sont les enfants boîte de leur
  `<slot>` (peints à sa position flat tree), le repli couvre
  l'absence d'assigné, l'orphelin `slot` inconnu ne peint
  pas. CSS : `::slotted(c)` — pseudo-élément frontier
  **sortant** (symétrique de `::part` — le sujet est
  l'enfant light assigné, les autres simples qualifient le
  slot, les combinators marchent dans la racine depuis le
  slot) ; confinement arbitré — le sujet doit être assigné
  à un slot de la racine DE LA RÈGLE (feuille document =
  `::slotted` muet). Héritage flat tree : `compute_subtree`
  calcule l'arbre shadow avant les enfants light — un
  assigné hérite du style de son `<slot>` (css-scoping §3.4),
  le non-assigné de l'hôte. Borné honnête : slots imbriqués
  non aplatis (`flatten` `assigned*()`), `<slot>` suit son
  `display` calculé (`display:contents` UA non porté),
  `exportparts`/`::part` imbriqué et `!important` inversé à
  la frontière absents, `slotchange`/`slot.assign()` =
  surface miroir NAV. Régressions : `slot_distribution`
  dom, 5× cascade (`::slotted`/qualifieur-slot/confinement/
  héritage-flat), 2× boxtree (distribution + repli), session
  bout-en-bout `dsd_slot_distribue_et_repli_couvert`.
- [x] **R-98. `explorer_html::entities` exposé** (demande
  résiduelle journal 2026-10-21 — le miroir jscore duplique
  la table) — **livré** : `EntityMatch`/`lookup_named`
  rendus `pub` + `decode_str(input, in_attribute)` = version
  « string entière » du décodage charref (nommées WHATWG
  complètes + legacy sans `;` + esperluette ambiguë en
  attribut, numériques `&#…`/`&#x…` remap `windows-1252`,
  orphelins littéraux). Test `decode_str_entites_nommees_
  et_numeriques`. Adoption miroir = commit NAV.
- [x] **R-99. `content_rect` promue publique** (audit file
  2026-10-25 — NAV la réplique en privé dans
  `session/composite.rs` pour le composite d'iframes, journal
  étape 129) — **livré** : `display::content_rect` exposée
  (`pub`, contrat « content box = border box − bordures −
  padding ») ; NAV adopte et retire sa copie à son commit.
- [x] **R-100. `:valid`/`:invalid` + pseudos d'état
  structurels** (demande NAV étape 54, seule extension
  « proposée » encore ouverte) — **livré** : module
  `state` d'`explorer-css` — les pseudos à valeur lue dans
  l'arbre s'évaluent désormais sur le DOM (plus sur `state`
  — qui restait `|_| false` en session : `:checked`,
  `:disabled`… étaient morts en production) : `:disabled`/
  `:enabled` (attribut + `fieldset[disabled]` ancêtre,
  exemption première `legend`), `:checked`,
  `:required`/`:optional`, `:read-only`/`:read-write`
  (`readonly` + `contenteditable`), `:placeholder-shown`,
  et le modèle §4.10.19 : candidats (input hors
  bouton/caché, select, textarea ; barrés disabled/
  readonly/`datalist`), contraintes `required`, `email`/
  `url`, `min`/`max`/`step` (number/range), `min`/`max`
  `length` (UTF-16) ; `form`/`fieldset` agrègent leurs
  candidats descendants. `:user-valid`/`:user-invalid`
  parsés : validité interne **et** `state(name)` (garde
  d'interaction = couture NAV, conservateur faux).
  `pattern` non évalué (RegExp JS — borné, documenté).
  7 tests `etat_*` (`tests/selector.rs`) : fieldset/legend,
  checked, required/optional, valid/invalid + agrégation
  form, remplissage, readonly/placeholder, gate user-*.
- [x] **R-101. Contrôle vide → boîte hauteur 0 → hit-test mort**
  (bug terrain dispatché 2026-10-22 — `searchInput` Wikipedia :
  54 sondes `mousemove`, zéro editable) — **livré** : la cause
  racine n'était ni l'atome `inline-*` (R-90) ni l'absolu,
  mais `control_paint` sans valeur/placeholder → **aucun
  enfant `Text`** → `height:auto` (ou `%` non résolvable)
  mesure 0 px → la boîte ne couvre pas la zone peinte par
  l'enveloppe `.cdx-text-input` (qui porte la bordure).
  Repro exact : `.w{position:relative}` + `input{height:100%}`
  → `rect.height = 0`, hit = `div`. Fix : jambe `Text`
  `\u{A0}` (insécable R-2, invisible) synthétisée pour tout
  contrôle textuel/select/bouton vide → hauteur de ligne ≈
  `1.2em` UA. Régressions `hit_test_input_*` ×4 (wrapper
  Codex, absolu `inset:0`, `height:100%` parent défini et
  indéfini). À re-mesurer terrain au prochain vol Wikipedia.
- [x] **R-102. Hit-test filtre `visibility:hidden`/`collapse`**
  (résidu R-49, catalogue file résiduelle) — **livré** :
  `hit_test`/`hit_test_scrolled` ne retournent plus la boîte
  d'un élément caché (style **calculé** — l'héritage est
  résolu : un descendant `visible` explicite reste ciblable,
  §11.2). Régression
  `hit_test_visibility_hidden_non_ciblable`.
- [x] **R-103. `display:contents` livré** (résidu catalogue —
  repli `Block` historique) : `child_boxes` aplatit l'élément
  sans boîte, ses enfants (et `::before`/`::after`) promus
  fils directs — items flex/grid, découpage en-ligne et slot
  UA héritent tous du mécanisme. Tests : structure + items
  flex.
- [x] **R-104. `radial-`/`conic-gradient` peints** (résidu
  catalogue — parsés, non rasterisés) : `decode_gradient`
  dispatche linear/radial/conic ; ellipse normalisée
  (shape/size/`at`, `√2` corner de css-images-3 §3.2, rayons
  px/%) et cône horaire (`from`, `at`, stops `%`/angle).
  `repeating-*` inclus. 3 tests pixels.
- [x] **R-105. `light-dark()` résolu à la valeur calculée**
  (résidu catalogue — hors scope R-59) : la cascade réécrit
  la branche claire/sombre selon le schéma usé de l'élément
  (`color-scheme` calculée ∩ `prefers-color-scheme` env),
  récursion paren-consciente, passe post-`var()`. Test
  `light_dark_suit_le_schema_use`.
- [x] **R-106. `background-origin`/`background-clip` au paint**
  (résidu catalogue — émis R-73, ignorés) : `emit_bg_layers`
  déduit les trois boîtes (border/padding/content via
  `content_rect` publique R-99) — `origin` choisit l'aire de
  positionnement, `clip` la zone de clip. `text`/`border-area`
  non portés. 2 tests.
- [x] **R-107. `background` shorthand multi-couches** (borne
  R-94 levée) : longhands jointes par couche (`background-image:
  url(a), linear-gradient(…)`), couleur réservée à la dernière
  couche (§3.10), paint par index de couche pour
  repeat/position/size/origin/clip. Carve-out du test R-94.
- [x] **R-108. Marqueurs textuels de liste** (résidu
  catalogue) : `marker_ordinal` sur `LayoutBox`
  (`ol[start]`/`reversed`, `li[value]`), `DisplayItem::
  MarkerText` shapé au raster via `FontSource` —
  `decimal`, `decimal-leading-zero`, alpha, roman ;
  inconnu → `decimal`. Test ordinals + types.
- [x] **R-109. `align-items:stretch` réel** (écart terrain
  github — dispatch journal) : `align-self:auto` hérite
  `align-items` (§8.3), `normal`→`stretch`, UA `input
  height`→`min-height` (un `height` UA bloquait la
  précondition `height:auto`), stretch planché au
  `min-height` item (§9.8). Test rect 42 + hit-test zone.
- [x] **R-112. `list-style-type: <string>`** (écart corpus
  datatracker) : la chaîne est le marqueur — `MarkerText`
  porte le littéral, aucun suffixe. 1 test.
- [x] **R-113. `ol[type]`/`li[type]`/`ul[type]` UA** (écart
  corpus datatracker, §14.3.9) : règles attribut-sélecteur
  `1/a/A/i/I`→decimal/alpha/roman — pas un hint (perdrait
  contre `ol{decimal}`). 1 test.
- [x] **R-137. Hit-test : rang de sous-arbre = max des rangs
  positionnés descendants** (`hit.rs`, §B-42 culture.gouv
  ×3 vols) — un overlay `position:fixed` sous wrapper
  statique était testé après les sous-arbres positionnés
  voisins → clic résolu sur l'ancre du mégamenu sous le
  bandeau. `subtree_rank` dans `hit_order` ; verrou
  `hit_test_overlay_fixed_sous_wrapper_statique_gagne`.
- [x] **R-110. `::part` chaîné + `exportparts` + slots nichés**
  (bornes Shadow) : `::part(a)::part(b)` marche par frontière,
  `exportparts` expose `i` sous `o` (§3.5) ; slots nichés
  couverts par récursion `slot_assigned` (test verrou).
  `match_part_chain` extrait.
- [x] **R-111. `!important` inversé à la frontière shadow**
  (css-scoping §6 — dernière borne) : `scope_rank` entre
  origine et layer — inner > outer normal, outer > inner
  `!important` ; `style=` au contexte du nœud. 3 sens testés.

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
| `DisplayItem` + dirty rects | transport FFI (NAV-55) | ✅ — livré R-7 (`changed_rects`), raffiné R-37/R-61/R-76 |
| op-list canvas → `DisplayItem::Image` | `explorer-jscore` canvas (JS-C28) | ✅ — livré R-11, consommé (`CanvasTable`) |

Règle : les API publiques des crates PIPELINE sont le contrat —
extensions additives de préférence ; toute casse de signature
existante = entrée dans `docs/interface-equipes.md` avant code.

## File résiduelle (vérifiée 2026-10-25)

Toutes les étapes R-1 → R-97 sont livrées. La file se nourrit
désormais des demandes journal et du triage terrain — état
vérifié au code de chaque reliquat (audit consigné dans
`interface-equipes.md`, même date) :

**Demandes ouvertes :**

- **`:valid` / `:invalid` / `:user-invalid`** — seule extension
  « proposée » non soldée. Parsés (`DynamicPseudo`) mais jamais
  évalués : la session appelle `compute_tree` avec `state =
  |_| false` et aucun modèle de validité §4.10.21.2 n'existe
  (`required`, `type=email|url`, `pattern`, `min|max`,
  `minlength|maxlength`, `step`). `:user-invalid` n'est pas
  parsé. Livrable PIPE : évaluation de la contrainte de
  validité d'un élément (`Dom` + règles spec) exposée pour le
  callback `state` — le filage session est la couture NAV.
- **`searchInput` Wikipedia — boîte vs surface peinte** :
  dispatché 2026-10-22, pas de fix dédié. R-90 (rects des
  inlines nichés + atomes `inline-flex`/`-grid`/`-table`) a
  traité la famille du motif `.cdx-text-input` — **à
  re-mesurer au prochain shot terrain avant de promouvoir**.
- **Exposer `explorer_html::entities`** (`lookup_named` pub ou
  `decode_str`) — demande résiduelle NAV (miroir jscore),
  non bloquante ; `lookup_named` est encore `pub(crate)`,
  jscore duplique `map_numeric`.
- **Promouvoir `content_rect` public** (`explorer-render`) —
  NAV le réplique en privé dans `session/composite.rs` pour
  le composite d'iframes ; promotion additif trivial, adoption
  NAV annoncée.

**Résidus fonctionnels documentés (à promouvoir en R-xx sur
trigger site réel) :**

- `display:contents` — repli `Block` ; flatten des enfants
  comme items flex/grid et UA sur `<slot>` non portés.
- `radial-`/`conic-gradient` — parsés, non peints.
- `background` shorthand multi-couches : première couche
  seulement (borne R-94) ; `background-origin`/`-clip` émis
  au parse, non consommés au paint. (Le multi-couches de
  `background-image` longhand est, lui, peint — R-73.)
- `light-dark()` — non implémenté (R-59 hors scope v1).
- Marqueurs textuels (`decimal`, alpha, roman) — non peints,
  la display list ne shape pas ; `list-style-position:
  inside` peint la puce sans décaler le texte.
- Hit-test sans filtre `visibility:hidden` — éléments cachés
  ciblables (inertes si `pointer-events:none`).
- Shadow DOM : `::part`/`exportparts` imbriqués, `!important`
  inversé à la frontière, aplatissement de slots nichés —
  bornes R-96/R-97.
