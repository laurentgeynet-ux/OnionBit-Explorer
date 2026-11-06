# Roadmap — Chantier ARÈNE (piste AR)

Piste dédiée à la **santé des arènes** — `explorer-dom` au
premier chef (réclamation, marquage, empreinte, accès), plus
les deux arènes adjacentes exposées au même défaut de classe
(heap `explorer-jscore`, DOM miroir). Issue de l'audit arène
→ contre-expertise → arbitrage (2026-11-06).

**Désignation `AR-N`** — distincte de `R-N` (PIPE), `Étape NN`
et `JS-CNN` (NAV), `PR-N` (profil), pour éviter toute
collision : le chantier enjambe les deux équipes. L'étape
est re-numérotée dans la roadmap de l'équipe qui la prend au
moment de la prise (AGENTS.md §5 : une étape = un commit + sa
case cochée dans *sa* roadmap).

**Périmètres.** Le socle (`explorer-dom`) est **PIPE** ;
le câblage session (seeds, déclencheur, adoption des index)
est **NAV** ; le heap et le miroir relèvent du **spécialiste
JS**. Les API publiques d'`explorer-dom` sont le contrat
inter-équipes — additif de préférence, toute casse = entrée
`docs/interface-equipes.md` **avant** code.

Légende : `[ ]` à faire · `[~]` en cours · `[x]` terminée.

## Faits de terrain (arbitrés — vérifiés au code)

| Constat | Preuve |
|---|---|
| `Slot` = **144 o** mesuré (`size_of`) : `Node` 136 dont **60 o de liens** (`5 × Option<NodeId>` à 12 o), `NodeData` 72 (discriminant niche-packé). 1 M nœuds ≈ **144 Mio** + tas. L'estimateur « ~120 o » de `session/tests.rs:2556` est faux. | sonde compilée, déclarations réelles |
| `Dom::collect` + toute l'API rooting (`add_root`/`remove_root`/`root_target`/`root_count`/`is_root_target`) = **morts en prod**. `free()` n'a que deux atteignants (`normalize`, `collect`) → `slots`/`live` **monotone-croissants** : fuite structurelle. | greps workspace ; `dom.rs:798-902` |
| `Remove` du journal purge déjà la `keymap` du sous-arbre entier — les nœuds retirés sont **orphelins purs**. | `session/apply.rs:1446-1464` |
| Le marquage ne suit que `first_child`/`next_sibling` — `template_contents` et les tables `shadow_roots`/`shadow_hosts` ignorés **et** jamais purgés. Câbler `collect` aujourd'hui balaie des arbres vivants. | `dom.rs:875-908` |
| La vivacité réelle n'est **pas** dans le `NodeCache` JS (il retient des ids **miroir**) : les détenteurs de `NodeId` réels = `keymap` + tables session persistantes (`PaintPayload.images.*`, transitions, focus/caret…). | `apply.rs`, `dom_host.rs:553` |
| Boxtree + fragments portent `Option<NodeId>` — une collecte en vol de frame laisserait des handles périmés dans les consommateurs. | `boxtree.rs:115`, `inline.rs:39` |
| `Dom::normalize` récursif sans `explorer_common::deep` — mort en prod ; l'homologue **miroir est itératif** (work-stack). | `dom.rs:784`, `jscore/dom.rs:3299-3332` |
| ADR-0008 promet « le nombre de roots est borné » — `add_root` est sans borne. | `dom.rs:826-832` |
| `ObjId` jscore = `u32` nu réutilisé ; `NodeId` **miroir** idem (`free_nodes`) — `dead_blocked` atténue le rejeu journal, pas les autres détenteurs. | `value.rs:9`, `jscore/dom.rs:23,693,2024` |
| `find_by_id` = parcours préfixe **avec un `String` alloué par nœud visité** ; `by_id` miroir = scan O(n) — aucun index `id` nulle part. | `dom_helpers.rs:1291`, `jscore/dom.rs:3636` |
| `.children(` ×68 en `src` (core 56) → un `Vec` par appel ; `.attribute(` ×124 → un `String` cloné par lookup. | comptages unitaires `src/` |

## Règles transverses

- **Mesurer d'abord** (ADR-0010) : aucun item perf sans baseline
  `cargo bench -p explorer-dom` (AR-6) + `mem_report`
  avant/après sur page à churn (accueil youtube.com scrollé =
  le banc réel du chantier — la fuite s'y mesure en `dom_nodes`).
- **Seeds, pas état** : la vivacité est **recalculée à chaque
  passe** (`collect(&seeds)`) — un seed manqué échoue typé
  (`StaleHandle`, traçable) ; un `remove_root` oublié retiendrait
  à vie en silence. `roots` reste le contrat des futurs handles
  JS (ADR-0008), sans producteur aujourd'hui.
- **Jamais de frame incohérente** : la collecte ne court qu'à
  une frontière où aucun consommateur de `NodeId` n'est en vol
  (boxtree, display list, `paint_in_flight`) — sinon seedé ou
  relayout forcé au pas suivant (contrainte AR-3).
- **Tolérance** : contenu hostile = norme — toute borne
  nouvelle est configurable (AGENTS.md §4.13), tout échec est
  typé, jamais de panic.
- **Un commit par étape** : code + tests + CHANGELOG + case de
  la roadmap d'équipe ; entrées `interface-equipes` pour les
  signatures/semantic touches.

## Palier AR-P0 — La fuite (correctness, enchaînement strict)

Ordre imposé : **AR-1 → AR-2 → AR-3**. Inverser tuerait des
arbres vivants (AR-1 sans seeds) ou des nœuds encore référencés
(AR-3 sans inventaire).

- [x] **AR-1. Marquage complet de `Dom::collect`** — PIPE.
  **Livré R-139.**
  Edge-set = celui que `apply_journal_op`/`Remove` marche déjà
  (`apply.rs:1451-1460`) : `first_child`/`next_sibling` **+
  `template_contents` + `shadow_root`** ; seeds ajoutées : les
  racines des `shadow_roots` attachées ; au sweep, purge de
  `shadow_roots`/`shadow_hosts` des entrées à génération morte.
  Tests : racine shadow attachée + fragment `<template>`
  survivent à `collect` ; sous-arbre détaché non seedé
  réclamé ; entrées shadow d'un hôte balayé purgées ; handle
  de nœud balayé → `StaleHandle` typé.
- [x] **AR-2. `Dom::collect(&seeds)`** — PIPE (signature).
  **Livré R-142** : `collect` gagne le paramètre seeds semé
  avec document+roots — la shape est prouvée en interne :
  `jscore::Dom::collect_garbage(&HashSet<NodeId>)`
  (`jscore/dom.rs:1055`). `roots` inchangé (contrat JS futur).
  Journal : annoncé R-140, livré R-142.
- [ ] **AR-3. Inventaire `NodeId` + déclencheur** — NAV.
  `dom_live_seeds()` centralisé : `keymap.values()` + tables
  persistantes (`PaintPayload.images.*`, transitions `elem`,
  focus, caret, hit-test, rects médias) — **exhaustivité
  requise** : un oubli = `StaleHandle` latent, visible à
  l'usage post-sweep seulement. Déclencheur : frontière de job
  (modèle `maybe_collect`) ou seuil `live`, **fenêtre
  contrainte** : jamais pendant qu'un boxtree/display list
  transporte des `NodeId` — ou boxtree inclus dans les seeds.
  Vérif terrain : `GET /mem` → `dom_nodes` redescend après
  churn ; verrou : session youtube longue sans croissance.
- [x] **AR-4. `normalize` itératif** — PIPE, trivial,
  intercalable. **Livré R-140.** Porter la forme work-stack du miroir
  (`jscore/dom.rs:3299-3332`) — sans son clone de `children`
  (liens sibling suffisent). Sémantique inchangée : premier
  `Text` de la série absorbe, saut `is_root_target`,
  `unlink`+`free` (bump de génération). Mort en prod — à
  livrer avant que la piste n'acquière des appelants.
- [x] **AR-5. Borne `roots`** — PIPE, trivial, intercalable.
  **Livré R-141** : `DEFAULT_MAX_ROOTS` = 65 536 configurable
  via `Dom::with_limits` + `DomError::RootBudgetExceeded`
  — implémente la promesse ADR-0008 avant qu'un producteur
  n'arme le contrat.

## Palier AR-P1 — Mesure (préalable à tout item perf)

- [ ] **AR-6. Bench `explorer-dom`** — PIPE. `criterion 0.5`
  déjà en workspace + `[[bench]]` livré dans `explorer-layout`
  (même harness). Scénarios : build d'arbre N nœuds,
  traversée (`children`/siblings/`attribute`), `collect` sur
  arène fragmentée, `set_attribute`/lookup. Produit les
  baselines de AR-9 → AR-15.
- [ ] **AR-7. `Dom::heap_bytes()`** — PIPE, additif.
  `slots.capacity() × size_of::<Slot>()` + tas réel (textes,
  `attributes`, `roots`) — alimente `mem_report` (étape 130 :
  capacité « non inventée » à ce jour).
- [ ] **AR-8. Estimateur corrigé** — NAV, trivial.
  `session/tests.rs:2556` « ~120 o » → **144 o** (mesuré).

## Palier AR-P2 — Perf / mémoire (sur baseline AR-6)

- [ ] **AR-9. `attribute_str` non-clonant** — PIPE additif +
  migration. Getter `Option<&str>` à côté de l'existant ;
  migration prioritaire `find_by_attr` (`dom_helpers.rs:1291` —
  aujourd'hui O(arbre) **avec une alloc par nœud**), puis les
  ~124 sites `.attribute(` en `src` (poids : core 108).
- [ ] **AR-10. Itérateurs de traversée** — PIPE additif.
  `children_iter` (curseur prévalidé — une validation, plus un
  bound+gen-check par sibling), `descendants` préordre,
  `compare_document_position` (API spec manquante ;
  `is_ancestor` couvre déjà `contains` modulo `|| a == b`).
  Consommateurs : ~68 sites `.children(` — poids en core (56).
- [ ] **AR-11. Index `id → NodeId`** — PIPE (champ + accesseur)
  puis adoption NAV. Maintenu à `set_attribute("id")` /
  `remove_attribute` / collect ; `by_id` O(1) — attention
  sémantique : spec = **premier match en ordre document**,
  l'index rend le premier, pas un quelconque. Tue
  `find_by_id`/`find_by_attr("id")` et prépare `getElementById`
  réel.
- [ ] **AR-12. Sweep `collect` optimisé** — PIPE. Bitmap
  `Vec<u64>` (vs `vec![false]` = 1 o/case), scratch conservé
  entre passes, `truncate` des `Vacant` traînants (le `Vec`
  ne rétrécit jamais → watermark permanent post-churn).
- [ ] **AR-13. Interning atomes** — PIPE, **casse de champs
  pub** → journal + arbitrage. `tag_name: String` +
  `Attribute.name` → atome `u32` sur table internée au `Dom`
  (« div »/« class » dupliqués ×milliers ; comparaisons
  entières à la cascade). `ElementData` est `pub` à champs
  `pub` → casse de signature. À trancher **après** baseline
  AR-6 (coût allocs parse + matching).
- [ ] **AR-14. Texte mutualisé** — PIPE, **casse de variante
  pub** → journal + arbitrage. `NodeData::Text(String)` /
  `Comment(String)` → ranges `(off, len)` dans un buffer
  document unique — des milliers de petites allocs → une.
  Même procédure qu'AR-13.

## Palier AR-P3 — Redesign sémantique (dernier, sur preuve)

- [ ] **AR-15. Liens internes `u32`** — PIPE, redesign.
  60 o → 20 o de liens par nœud (`Slot` 144 → ~104 o).
  **Prérequis bloquant** : `free()` ne délinke pas —
  des survivants (roots, tables latérales) gardent des liens
  vers des cases libérées et c'est la **génération** qui rend
  leur suivi un échec typé aujourd'hui. En `u32` nu, une case
  réallouée résout silencieusement le mauvais nœud → exige un
  fixup de délink à la libération dans `collect` **avant**
  tout gain. Ne livrer que si AR-6/AR-12 mesurent le coût
  réel des `Option<NodeId>`.

## Piste parallèle JS (spécialiste JS — hors `explorer-dom`)

- [ ] **AR-16. Audit ids sans génération** — JS. `ObjId` =
  `u32` nu réutilisé (`heap.rs` alloc via `free.pop()`) et
  `NodeId` miroir idem (`free_nodes`) : un id obsolète peut
  résoudre un objet **réaffecté** en silence — `dead_blocked`
  ne couvre que le rejeu journal. Audit + ADR avant impl ;
  la mitigation « handle persistant = rooté » borne mais
  n'élimine pas la classe.
- [ ] **AR-17. Index `id` miroir** — JS. `by_id` = scan O(n)
  (`jscore/dom.rs:3636`) ; `getElementById` est chaud sur
  pages réelles — `HashMap<String, NodeId>` maintenu à
  attr/attach/detach.

## Ouvertures stratégiques (hors file — à promouvoir au besoin)

- **Séquence de mutation / dirty bits** exposés par l'arène —
  source unique pour le relayout ciblé (R-12 vit côté
  session, non alimenté par le domaine).
- **Convergence miroir ↔ arène réelle** — le journal ADR-0025
  duplique la shape ; « le miroir jscore n'est pas la shape
  DOM finale » (`interface-equipes.md:288`). Chantier ADR,
  pas un item de file.
- **`is_root_target` O(roots)** — index inversé `NodeId →
  count` ; pertinent seulement quand `roots` aura un
  producteur (AR-2 l'offre en alternative — alors cet index
  devient load-bearing).

## Traçabilité constat → étape

| Constat | Étape(s) | Équipe |
|---|---|---|
| `collect` jamais câblée → fuite monotone | AR-1 → AR-3 | PIPE + NAV |
| Marquage ignore template/shadow | AR-1 | PIPE |
| API rooting morte / `roots` non borné | AR-2, AR-5 | PIPE |
| `normalize` récursif | AR-4 | PIPE |
| « ~120 o » faux / pas de `heap_bytes` | AR-7, AR-8 | PIPE + NAV |
| `attribute()` clone ×124 / `children()` `Vec` ×68 | AR-9, AR-10 | PIPE + NAV |
| Aucun index `id` (réel ni miroir) | AR-11, AR-17 | PIPE + JS |
| Allocs strings/texte à la parse | AR-13, AR-14 | PIPE |
| Liens 60 o + sûreté générationnelle | AR-15 | PIPE |
| `ObjId`/`NodeId` miroir sans génération | AR-16 | JS |
