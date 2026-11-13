# Plan perf archétypes — piste unique (dev.to / YouTube)

> Document de pilotage — 2026-11-13, rév. 1. **Source de
> vérité du plan** : « Plan perf archétypes » de
> `roadmap-equipe-pipeline.md` et « Sprint N5 » de
> `roadmap-equipe-navigateur.md` ne gardent que le pointeur —
> l'ordre, les fichiers et les verrous vivent ici.
> **Un seul opérateur, deux chapeaux séquentiels** : chaque
> étape/commit reste étiquetée par périmètre
> (`css`/`layout`/`render` = PIPE, `core`/`jscore`/`net` =
> NAV) ; le sign-off croisé des specs devient une revue de
> design écrite au journal **avant** code. Rédigé sur l'état
> mesuré `sync_profile` (audits chantiers 8-12ter, entrée
> d'assignation `interface-equipes.md` 2026-11-13).

## 0. Objectif et définition de « fait »

**Produit** : réduire mesurablement le wall des deux
archétypes réels — dev.to (~149-161 s, vs ~413 s avant
Étapes 232-235) et YouTube (~212,6 s, JS ~50 % du wall) —
en attaquant les postes **attribués par sonde**, jamais par
intuition.

**Condition de sortie par étape** : commit vert
(`verify_all`) + re-vol sur les **deux** archétypes
rapportant le delta du poste visé — jamais l'un sans
l'autre. Verdict visuel = binaire frais uniquement (§8
AGENTS — attendre `Finished`).

## 1. Postes mesurés (état au 2026-11-13)

| Poste | Mesure | Site | Levier |
|---|---|---|---|
| `incr.cascade` | 8,5-10,4 s — poste n°1 | dev.to | frontière (§2.2) |
| jambes JS agrégées | ~110-150 s (`commit_scripts` 57-66 s, `js_run_sync` ≤ 30 s/appel, `mirror_sync` ≤ 26 s) | YouTube | §2.6 |
| `group_us` résidu | ~9,9 s quand zone = cadre | YouTube | §2.5 |
| `incr.emit` | ~1,5 s | dev.to | `may_escape` (§2.4) |
| `draw_us` | ~1,0 s | dev.to | attribution d'abord (§2.7) |
| `incr.raster` fenêtre lourde | 13,4 s (`clips_real` ~1 898, ~46 %) | dev.to | **trou nommé — coût de couverture réel, aucun levier de cull** (§4) |

**Déjà solvés — ne pas refaire** : `resolved_bounds`
incrémental (~2 ms, innocent) ; `Mask` ∩ pixmap (impossible
via tiny-skia — pool livré Étape 235) ; atlas glyphes
(**R-85 actif** — `rendered` → `blit_glyph`, `draw_us`
mesuré avec) ; `explorer_html::entities` pub (R-98/99) ;
attribution `incr.repaint` (`raster.parts`).

## 2. Séquence ordonnée

L'ordre est imposé par les dépendances, pas par les
équipes. Chaque étape prend son numéro (PIPE : R-292+ ;
NAV : Étape 236+) au commit.

### 2.1 — L'arc entier vit sur `performance` (merge différé)

| Tâche | Fichiers | DoD |
|---|---|---|
| Décision opérateur : la suite de l'arc se fait **sur la branche** jusqu'à convergence (Étape 239) — `main` n'est mergé qu'en fin d'arc | — | branche verte à chaque commit ; dérive de `main` bornée |

- **`main` est figé pendant l'arc** — aucune autre piste ne
  lande ; la dérive est nulle par construction (état au
  2026-11-13 : branche +9 / −0). Si une piste reprend et
  touche `raster.rs`/`apply.rs`/`cascade.rs`, réintégrer
  `main` → `performance` aussitôt plutôt qu'un merge final
  massif.
- Le câblage §2.3 n'attend **que** la frontière sur la
  branche — `journal_style_scopes` y est déjà (Étape 234) ;
  le merge n'est plus une précondition.
- Les re-vols se font sur le **binaire de branche frais** —
  c'est l'état mesuré, la baseline post-235 vit ici.

### 2.2 — Frontière cascade (R-292) [PIPE `explorer-css`] — **LIVRÉ** (`570c18f5`)

| Tâche | Fichiers | DoD |
|---|---|---|
| `compute_scoped_frontier(root, frontier: &[NodeId])` — extension **additive** de `compute_scoped`/`apply_scoped` : ancêtres + frères précédents des touchés évalués en sus du sous-arbre, borné héritage (restyle du parent propage aux descendants — la frontière ne peut pas réduire sous la ligne de parenté) | `crates/explorer-css/src/cascade.rs` | `incr.cascade` < ~2 s au re-vol dev.to sur les mutations à parent géant (~4,7-5,3k nœuds) |

Spec = proposition journalisée (chantier 12ter). **Revue
de design écrite au journal avant code** (remplace le
sign-off croisé en piste unique) : couverture des cas
`~`/`+`/`:nth-*` frères, héritage transitif, repli quand
la frontière ≈ le sous-arbre. **Livré** : API 3 niveaux
additifs, 9 tests `cascade_frontier` verts (invariant
`base ⊕ frontier ≡ compute_tree`), canal silencieux
couvert par flag `deep_relational`.

### 2.3 — Câblage frontière + re-vol [NAV `explorer-core`] — **LIVRÉ** (`63043aef`, Étape 236)

| Tâche | Fichiers | DoD |
|---|---|---|
| Dériver `frontier` (ancêtres + frères précédents des touchés) depuis `journal_style_scopes` ; appeler `compute_scoped_frontier` quand dispo, repli `compute_scoped` | `crates/explorer-core/src/session/apply.rs` | re-vol dev.to : `scope_nodes` géants évités, `incr.cascade` tranché |

Dépend de §2.2. **Livré** : `style_touched` (nœud réel
`SetAttr`, parent `Remove`/`Insert`/`SetText`) →
`compute_scoped_frontier` quand `frontier_ok`, repli
multi-scope 234 sinon. **Mesuré** (`vol-236-devto-dlv.log`,
sondes `frontier_ok`/`n_frontier` armées) : passes petits
touchés → `incr.cascade` **0,31-0,34 s ≈ ×30** vs scope
~4 800 nœuds (~10 s) ; mur résiduel hors frontière :
`SetAttr` sur conteneur géant (`descend_all` requis, 7,66 s
— levier « descend-on-change si l'attribut n'est dans
aucun composé ancêtre » **livré en R-297**, voir ci-dessous)
et `has_dep_touched` → cascade complète 8,74 s (garde 234,
trou (e)).

**R-297 — frontière à clé de mutation + fan-out borné
(livré, mesuré ×80)** : `frontier` typée `(NodeId,
Option<&str>)` — `Some(attr)` pour `SetAttr`, `None`
structurel/texte. `AncestorKeys` (clés `[attr]`/pseudos
dynamiques/`.class`/`#id` en position non-sujet, traversée
`:not`/`:is`/`:has`/`:host*`) : sous `Some(a)` la descente
exige `attr_descends ∨ computed ≠ base` — un `data-*` hors
sélecteurs élaque le sous-arbre, `A[b] .c` force la
descente. **Raffinement fan-out** nommé par la sonde
`FrontierProbe` (`visited`) : frères précédents non visités
sous frontière 100 % `Some` (`:has` gardé amont,
`:nth-last-*` = comptage) ; frères suivants visités ssi
`None` ∨ `attr_descends`. Verrous : 13/13
`cascade_frontier` (+4 tests — élagage, descente forcée,
frères non visités `visited=3`, bascule `+`-sur-attr),
`base ⊕ frontier ≡ compute_tree`. Casse de signature
journalisée — commit atomique css + core. **Mesuré
(`vol-297c`)** : `visited` 4 700 → 11, `incr.cascade`
**6,41 s → 79,5 ms ≈ ×80** sur `data-is-billboard-visible` ;
résidu = repli `has_dep_touched` 8,8 s (trou (e)).

**R-298 + Étape 238 — frontière à dépendances `:has`
(trou (e) fermé, mesuré)** : le repli `has_dep_touched` →
cascade complète 8,8 s existait pour un cas précis —
`E:has(x) .d` où la vérité `:has` de `E` bascule **sans**
changer son computed (la règle ne style que le composé
final `.d`) : `changed` seul rate `.d`. Correctif borné en
deux moitiés. *R-298 (PIPE)* : `TouchedDeps`
(attrs/classes/ids/structural/all effectivement mutés) +
index par passe `has_subject_index` = composés **non-
sujets** porteurs de `:has`, privés du pseudo + deps de
leurs args (`arg_deps`) + scope de la règle ;
`compute_scoped_frontier_deps(dom, root, env, base,
frontier, deps)` — sous `Some(d)` tout nœud visité
matchant un sujet plausible dont l'arg croise `d` obtient
`descend_all` (`has_subject_hit` via
`matches_with_state_cached`, même scope shadow), et la
couverture des frères précédents s'ouvre (sujets
`E:has(+ ~)` vivent à gauche du match). *Étape 238 (NAV)* :
`apply_journal_op` calcule le verdict dep **par op** avant
mutation (`journal_op_touches_has` ∪ `op_may_mutate_sheets`),
accumule les effets dans `tdeps` (SetAttr → attr +
class/id old∪new ; Insert → structural + noms dans
l'html ; Remove/AttachShadow/SetText → `all`/`structural`)
et arme `has_dep_touched` in-stream ; la gate devient
`has_block = eng_has_deps.is_none() && rel` (frontière
armée sous index complet), `scoped` reste interdit sous
dep touchée (pas de descente plausible → canal non
couvert), jeu de feuilles reconstruit → `deps.all`.
Clés de mutation **inchangées** (un attr de dep vit dans
un arg `:has` donc dans `AncestorKeys` → `Some` suffit).
API additive (R-298) journalisée. Verrous : 17/17
`cascade_frontier` (+4 tests canal ancêtre/frère,
non-touche élagué, `all` conservateur) + 2 tests NAV
(`tdeps` alimenté/non-dep, frontière armée `descend_has>0`
sous dep touchée) + verrous `falls_back_on_has_unanalyzable`
/`survives_has_untouched_dep` inchangés. Sonde
`FrontierProbe.descend_has` ajoutée.

**R-299 — delta de feuilles (poste résiduel nommé par le
vol R-298 : ~30 s/75 s de cascades complètes dev.to, ~12 s
YouTube — rebuild de feuilles → `n_frontier=0` →
`compute_tree`)** : `compute_sheet_delta(new, …, old)` diff
les tables de règles en zone médiane (préfixe/suffixe
communs exclus — `rule_semantic_eq` sans `order`) ; une
marche DOM seede les éléments matchant ≥1 sélecteur
changé (mini-`SelectorIndex` sur refs, média+scope, clé
sentinelle `\u{1}delta` = évalué, descente sur delta
computed seul) **et** tout nœud jamais stylé (streaming,
clé `None`), fusionne la frontière journal, délègue à
`frontier_ctx`. Gates → `compute_tree` : `namespaces`/
`@property`/`@container` différents (computed/matching
sans sélecteur), `MAX_DELTA_RULES`/`MAX_DELTA_SEEDS`.
`apply_scoped` propage `counter_styles`/`font_*` (correctif
de staleness). Le bras delta est **prioritaire** sur
`frontier`/`scoped` sous `eng_rebuilt` — ferme le trou
latent de R-298 (frontière seule sur rebuild). API
additive journalisée ; verrous 25/25 `cascade_frontier`
(+8) + `r299_delta_feuilles_restyle_borne`. Sondes
`delta_visited`/`delta_seeds`. **Mesuré release, protocole
fixe doublé** (`vol.sh` `--release`, dev.to 52,6 s /
YouTube 62,9 s, `76785796`) : cascade totale 30,8 s →
**2,86 s** dev.to (frontière 7,65 ms `visited=11` + delta
1,07 s `delta_visited=372, seeds=99` ; 2 `flow.cascade`
~0,9 s pleins — pas d'ancien moteur/nav, attendu) et
12,2 s → **3,08 s** YouTube (delta **180 ms**
`delta_visited=809, seeds=37`, frontière 156 ms
`descend_has=1`, scoped ~1,3 s). Pas de baseline release
pré-R-299 : la part exacte codegen-vs-delta n'est pas
séparable, mais les marches bornées (372/809 évals vs
arbres ~4 700) prouvent le mécanisme. Nouveau poste n°1
YouTube : **jambes JS 30 s = 48 % du wall en release**
(`pump.js_run_sync` 22,6 s + `commit_scripts` 7,5 s) —
coût réel, pas artefact debug (§2.6bis tranché).

### 2.4 — `may_escape` (R-293) [PIPE `explorer-layout` + `explorer-render`]

| Tâche | Fichiers | DoD |
|---|---|---|
| Champ `may_escape: bool` sur `LayoutBox` — post-ordre au build ET `relayout_subtree` (vrai si la boîte ou un descendant : `position ≠ static` — `pos_defer`/`fixed_defer` inclus —, transformé, flottant, `overflow:visible` à débord ; conservateur = `true`) ; culle O(1) dans `paint_box` : sauter quand `rect ⊕ marge-ombres ∩ bande = ∅ && !may_escape` — **voie incrémentale seule** | `crates/explorer-layout/src/` (champ, calcul), `crates/explorer-render/src/display.rs` (`paint_box`) | `incr.emit` < ~0,5 s au re-vol dev.to ; `rasterize` plein cadre inchangé (verrou différentiel) |

Spec = proposition journalisée (même revue de design que
§2.2).

**Statut — fait et mesuré (R-293, commits `2a4b72a4` +
`290d45b1`)** : champ + `compute_may_escape` (fin
`layout_tree`/`relayout_subtree`) + culle dans
`emit_scoped` (`cx.inc` seul, `shifted(rect,off) ⊕
ink_margin ∩ vp = ∅`) + raffinement `relative` à insets
`auto` non échappants. 7 tests verts, zéro régression.
**Verdict DoD : NON ATTEINT et levier infirmé** —
`incr.emit` dev.to ~1,48-1,53 s inchangé, `items`
identique → la culle n'a jamais tiré : `reuse_delta`/
`copy_span` absorbent déjà la traversée des sous-arbres
stables, le résidu = re-paint des mutés + copies de
spans. YouTube : `incr.emit` 0,07-0,60 s/passe (pas le
poste), wall 209,7 s (variance JS), aucune régression.
Mécanisme conservé (O(1), utile scroll/mutations
partielles) ; poste `incr.emit` ~1,5 s rouvert → levier
suivant = attribution interne de l'emit avant code
(cf. §2.7).

**Attribution livrée (`emit.parts`, commit `87788f6`)** :
`incr.emit` 1,47 s = émission réelle des ~443 boîtes
mutées (~3,3 ms/boîte debug) — copies+bookkeeping < 0,4 %,
`culled=0` (tout ce qui est visité est on-band). Levier (a)
choisi par l'utilisateur : **valeurs résolues cachées sur
`ComputedStyle`** — livré R-295 (`ResolvedFx`/`FxShadow`,
`OnceLock` paresseux partagé par clone, `PartialEq` manuel
inchangé) : `box-shadow`/`text-shadow`/`ink_margin` ne
re-parsent plus par item ; `bg_layers`/`corner_radii`
restent des cibles du même cache si la mesure les nomme.
Sonde `draw` par kind livrée dans `raster.parts`. Mesure
avant/après au re-vol court (`LOBSTE_PROBE_HOVER=0`).

**Vol court mesuré (85 s — boutons validés)** : `incr.emit`
inchangé (levier (a) infirmé — vol `--release` tranchera
debug vs réel) ; **`draw_shadow_us` ≈ 0,9 s = 88 % du
`draw_us`** — `paint_box_shadow` = 8 `fill_path`
concentriques/ombre, re-rasterisés par passe. **R-296
arrêté** : cache de couches d'ombres sur `Pixmap` borné +
`draw_pixmap` (pixels identiques) — prochain chantier
PIPE, avant §2.5 dont le poste ne se reproduit plus.

### 2.5 — Ops de groupe, résidu zone = cadre (R-294) [PIPE `explorer-render`]

| Tâche | Fichiers | DoD |
|---|---|---|
| (a) Ops **bandées** quand `zone_out` = cadre entier (le bornage zone ne peut rien quand la zone est le tout) ; (b) `group_mask_coverage` : `Mask::new(pw,ph)` aux dims parent + merge clip plein extent → borner à `zone_out` ; (c) chemin `filter=None` → `Unbounded` | `crates/explorer-render/src/raster.rs` | `group_us` < ~2 s sur frame à groupes couvrant le cadre (re-vol YouTube) ; différentiel fenêtré vs plein cadre vert |

`composite_blend`/`apply_filter_ops` sont **déjà** bornés
à la zone (Étape 235) — ce chantier vise uniquement le
résidu listé.

### 2.6 — YouTube `--release` + attribution JS + correctif [NAV]

| Tâche | Fichiers | DoD |
|---|---|---|
| (a) Vol `--release` : tranche si les jambes JS (~110-150 s) sont réelles ou gonflées debug ×10-20 — `cargo test -p explorer-core --lib --release lobste_pipeline_errors` (`LOBSTE_PROBE_URL`) ou daemon release + vol canonique (un seul binaire de test, pas le piège des ~50 jscore) ; (b) attribution interne méthode `raster.parts` sur `pump.js_run_sync`/`net.commit_scripts`/`event.mirror_sync` + heap GC (~570-690 k cells) — **référencer Sprint N3** (`prop_scan` 72 M → N4.1, `body_ns` ~426 s §B-114 — ne pas ré-instrumenter) ; (c) correctif sur le poste nommé | `crates/explorer-core/src/session/` (`js_glue.rs`, `net.rs`, `apply.rs`), `crates/explorer-jscore/` | verdict tranché consigné au journal ; si poste réel nommé → correctif + re-vol |

Le build release peut tourner en tâche de fond pendant
§2.2 — le verdict se lit quand il est prêt.

### 2.7 — Opportunistes (ordre libre)

| Tâche | Périmètre | DoD |
|---|---|---|
| Adoption `entities` : miroir jscore délègue à `explorer_html::entities` (`dom.rs` `decode_entities` + `map_numeric` dupliqués — résidu demande 2026-10-21) | NAV `explorer-jscore` | quasi gratuit — `textContent` identique sur jeu d'entités étendu |
| Sonde `draw` par kind d'item : `raster.parts` compte `draws=N` sans décomposition — attribuer glyphes/images/fills **avant** tout chantier `draw_us` | PIPE `explorer-render` | poste ~1 s attribué ou déclaré plancher |
| `content-visibility: auto` viewport-aware : reliquant Étape 230 — la pertinence doit être alimentée par le viewport (émission ou invalidation scroll ; le scroll ne re-layout pas — couture `explorer-core` à journaliser) | PIPE `explorer-layout` + couture NAV | design arrêté au journal, puis gain scroll mesuré |

### 2.8 — Étape 239 : convergence

Plan global consolidé ; chaque gain rapporté contre les
**deux** archétypes (dev.to et YouTube — jamais un seul).

## 3. Discipline d'exécution

- **Une étape = un commit** (`feat(<crate>)`), CHANGELOG +
  case roadmap cochée dans le même commit (règle §4.5) —
  les cases se cochent dans la roadmap de l'étape
  (R-… côté PIPE, Étape … côté NAV), pas ici.
- **Périmètre par commit** : un commit PIPE ne touche pas
  `core`/`jscore`/`net` ; un commit NAV ne touche pas
  `css`/`layout`/`render` — les coutures passent par le
  journal.
- **Re-vols doubles** après chaque livraison — le
  `raster.parts`/`sync_profile` des vols est la mesure de
  vérité.
- **Journal** : toute extension de contrat, mesure en vol
  et trou nouveau s'y écrit — c'est la synchro.
- **Pas de deuxième worktree** : tout vit sur
  `C:\OnionBit-Explorer-performance`.

## 4. Trou nommé et sortis

**Fenêtre lourde dev.to (hors plan)** : `clips_real`
~1 898 (~46 %) → `incr.raster` 13,4 s ; `Mask::fill_path`
~6 ms sur grand pixmap = coût de **couverture réel** (clips
vraiment partiels — non cullables, `Mask` aux dims du dst
exigé par tiny-skia). Poste n°2 dev.to après §2.2 —
mesurer avant de promettre un levier.

**Sortis** : `resolved_bounds` incrémental (2 ms),
`Mask` ∩ pixmap (impossible — pool livré), attribution
`incr.repaint` (faite), atlas glyphes (R-85), `entities`
pub (R-98/99).
