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
  Test : `a` abs sans `top` dans `ul` abs à
  `top:-100px` → suit le conteneur (y≈−100),
  plus jamais 0. Terrain : les deux liens
  « Skip to main content »/« Skip to
  search » du MDN réel ont disparu du shot
  (diff pixel localisé y18-33).
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
