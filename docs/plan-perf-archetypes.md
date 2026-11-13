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

### 2.2 — Frontière cascade (R-292) [PIPE `explorer-css`]

| Tâche | Fichiers | DoD |
|---|---|---|
| `compute_scoped_frontier(root, frontier: &[NodeId])` — extension **additive** de `compute_scoped`/`apply_scoped` : ancêtres + frères précédents des touchés évalués en sus du sous-arbre, borné héritage (restyle du parent propage aux descendants — la frontière ne peut pas réduire sous la ligne de parenté) | `crates/explorer-css/src/cascade.rs` | `incr.cascade` < ~2 s au re-vol dev.to sur les mutations à parent géant (~4,7-5,3k nœuds) |

Spec = proposition journalisée (chantier 12ter). **Revue
de design écrite au journal avant code** (remplace le
sign-off croisé en piste unique) : couverture des cas
`~`/`+`/`:nth-*` frères, héritage transitif, repli quand
la frontière ≈ le sous-arbre.

### 2.3 — Câblage frontière + re-vol [NAV `explorer-core`]

| Tâche | Fichiers | DoD |
|---|---|---|
| Dériver `frontier` (ancêtres + frères précédents des touchés) depuis `journal_style_scopes` ; appeler `compute_scoped_frontier` quand dispo, repli `compute_scoped` | `crates/explorer-core/src/session/apply.rs` | re-vol dev.to : `scope_nodes` géants évités, `incr.cascade` tranché |

Dépend de §2.2.

### 2.4 — `may_escape` (R-293) [PIPE `explorer-layout` + `explorer-render`]

| Tâche | Fichiers | DoD |
|---|---|---|
| Champ `may_escape: bool` sur `LayoutBox` — post-ordre au build ET `relayout_subtree` (vrai si la boîte ou un descendant : `position ≠ static` — `pos_defer`/`fixed_defer` inclus —, transformé, flottant, `overflow:visible` à débord ; conservateur = `true`) ; culle O(1) dans `paint_box` : sauter quand `rect ⊕ marge-ombres ∩ bande = ∅ && !may_escape` — **voie incrémentale seule** | `crates/explorer-layout/src/` (champ, calcul), `crates/explorer-render/src/display.rs` (`paint_box`) | `incr.emit` < ~0,5 s au re-vol dev.to ; `rasterize` plein cadre inchangé (verrou différentiel) |

Spec = proposition journalisée (même revue de design que
§2.2).

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
