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
- [x] **AR-3. Inventaire `NodeId` + déclencheur** — NAV.
  **Livré Étape 208** : `dom_live_seeds` (`keymap` +
  `touched` + `frame_nodes` + `anim_births`), déclencheur
  watermark ×2 dans `apply_journal`, `retain_alive` sur
  `images.*`/`canvases`.
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

- [x] **AR-6. Bench `explorer-dom`** — PIPE.
  **Livré R-144** : `benches/arena.rs` (criterion 0.5,
  harness `relayout`) — baselines quick-run :
  `attribute` ≈ 27× `attributes` (68 vs 2,5 ns/lookup),
  `children` 4,2 µs/appel, `collect` 3,4 ms/10 k
  fragmentés, marquage profond linéaire 0,5 ms/10 k.
- [x] **AR-7. `Dom::heap_bytes()`** — PIPE, additif.
  **Livré R-143** : capacité `slots` (cases libres
  incluses) + tables + allocs internes aux nœuds →
  `MemReport.dom_bytes`/`bfcache_dom_bytes`.
- [x] **AR-8. Estimateur corrigé** — NAV, trivial.
  `tab_mem_inventory` : `live_count × ~120 o` + marche
  manuelle remplacés par `Dom::heap_bytes()` (R-143) ;
  `dom_deep_bytes` supprimé.

## Palier AR-P2 — Perf / mémoire (sur baseline AR-6)

- [x] **AR-9. `attribute_str` non-clonant** — PIPE additif +
  migration. **Livré R-145** : les 124 sites `src` migrés ;
  `find_by_attr` ne fait plus d'alloc par nœud ; `to_owned`
  uniquement aux sorties qui exigent `String` (FormEntry,
  IframeKind, URLs collectées).
- [x] **AR-10. Itérateurs de traversée** — PIPE additif.
  **Livré R-146** : `children_iter` (curseur sur liens
  intrusifs — une validation racine, un check par sibling,
  zéro `Vec`) + `descendants` (préordre itératif borné à la
  racine — pas de fuite par parent/sibling). Migration
  sélective : scans document `queue` → `descendants`,
  boucles directes → `children_iter` ; **conservés** :
  scans à arêtes latérales (shadow/`<template>`), gardes de
  frontière (`<form>` imbriqué, sous-arbre `<svg>`),
  tranches/réassignations du `Vec`, boucles mutantes.
  `compare_document_position` reste ouvert (hors livraison —
  `is_ancestor` couvre `contains` modulo `|| a == b`).
- [x] **AR-11. Index `id → NodeId`** — **Livré R-147** :
  `id_index` multivalué maintenu à `set_attribute`
  (rename = retire ancien + ajoute) / `remove_attribute` /
  purge `collect` ; `by_id` O(1) filtre vivant +
  atteignable-depuis-document (détaché/shadow/template
  exclus — spec `getElementById`) et arbitre les doublons
  par `doc_order_lt` **à la requête** (les `insert_before`
  ne réordonnent pas l'index). Adoption NAV : `find_by_id`
  tué (6 sites), `svg::find_by_id` tué, `find_by_attr`
  conservé pour les attributs génériques. `element_mut`
  documenté comme contournement de l'index.
- [x] **AR-12. Sweep `collect` optimisé** — **Livré R-148** :
  bitmap `Vec<u64>` sur scratch `mark_bits`/`mark_stack`
  conservé entre passes (zéro alloc/passe) ; balayage borné
  à `new_len` avec free-list **re-chaînée dans le même
  passage** en ordre croissant (allocs tassées en tête,
  queue tronçable) — aucun second parcours des cases ;
  `truncate` + `shrink` amorti (seuil 2×) → la RAM post-churn
  est rendue, `heap_bytes` la mesure. Sécurité générationnelle
  : plancher `tail_generation` sur les cases tronquées
  (repousse au-dessus de toute génération abandonnée) +
  `peak_len` monotonique distinguant indice tronqué
  (`StaleHandle`) d'indice forgé (`OutOfBounds`). Valeur
  d'index `id` singleton inline `IdSet` (purge sans saut de
  cache sur ids uniques). Banc : `collect` fragmenté 10 k
  **3,8 ms vs 4,5 ms baseline** (−15 %) en faisant plus de
  travail (reclaim + purges) ; profond 0,42 ms.
- [x] **AR-13. Interning atomes** — **Livré R-149** :
  `tag_name`/`Attribute.name` : `String` → `Atom` — nom
  interné **process-wide** (`&'static str` dédupliquée
  `thread_local`, zéro verrou — style `AtomicString`).
  *Écart au brief « u32 sur table au `Dom` »* : une table
  par-`Dom` cassait `PartialEq` cross-`Dom` (`attrs_eq`
  compare deux arènes) et imposait `&Dom` à chaque lecture —
  l'atome global garde l'égalité saine partout (fast-path
  pointeur + égalité textuelle). Shims `PartialEq<str>` /
  `Deref` / `Display` / `as_str()` → lectures quasi
  inchangées (2 corrections mesurées workspace). Bonus :
  `NodeData::Doctype` boxé (unique → ne plafonne plus
  `NodeData` à 72 o). `Slot` 144→128 o, −1 alloc/élément,
  −1 alloc/nom d'attribut, noms dedup'd ; `collect` 3,2 ms
  (baseline 4,5), profond 0,19 ms. Casse loguée au journal
  **avant** code (§10.2).
- [x] **AR-14. Texte mutualisé** — PIPE, **casse de variante
  pub** journalée avant code (même procédure qu'AR-13).
  `NodeData::Text`/`Comment` : `String` (24 o + alloc) →
  `TextRange{start:u32,len:u32}` (8 o) dans `Dom::text_buffer`
  — **une seule allocation pour tout le texte du document**.
  Sémantique : buffer append-only entre passes (`set_text`/
  fusion `normalize` poussent une plage, l'ancienne meurt)
  puis **compacté par `collect`** — plages vivantes réécrites
  contiguës, octets morts rendus (`heap_bytes` le mesure ;
  `String::new()` pour que la capacité se tasse au vivant).
  `dom.text`/`set_text`/`create_text`/`text_content` inchangés
  en signature (`&str`) ; nouveau `dom.text_at(TextRange)`
  pour les payloads destructurés (~10 sites migrés :
  apply, tree, dom_helpers, debug, svg, tests). `PartialEq`
  `NodeData` compare désormais les plages. Cap buffer
  `u32::MAX` → `BudgetExceeded` typé. Mesuré : −1 alloc/nœud
  texte, ~40 o/nœud net, `collect` fragmenté **2,7 ms**
  (vs 4,5 baseline), profond 0,20 ms ; verrou
  `collect_compacte_le_buffer_texte` (heap ÷4 + survie du
  texte + plage forgée → `""`).

## Palier AR-P3 — Redesign sémantique (dernier, sur preuve)

- [x] **AR-15. Liens internes — mesuré, livré la moitié
  sûre, `u32` nu refusé sur preuve** — PIPE. Mesure
  `Option<NodeId>` = **12 o** (4 o de discriminant —
  `NodeId{u32,u32}` sans niche) → 5 liens intrusifs =
  **60 o/nœud** (roadmap cohérente). **Livré** : sentinelle
  `NodeId::NIL` (`index = u32::MAX`, `generation = 0`) —
  liens stockés en `NodeId` nu → **60 → 40 o/nœud**,
  `Node` 120 → **96 o**, `Slot` 128 → **104 o** (−19 %) ;
  **génération toujours vérifiée à la résolution** —
  invariant ADR-0008 strictement inchangé, champs
  `pub(crate)`, zéro casse API, zéro fixup. Banc :
  `collect` fragmenté **2,3 ms** (baseline 4,5 → −48 %
  cumulé AR-12→15), profond 0,18 ms. **`u32` intégral
  refusé sur analyse** : le marquage couvre les liens
  descendants, mais `parent` et les siblings frontière
  (sous-arbre vivant par root/seed dont le parent meurt)
  pointeraient vers des cases réattribuées → **mauvais nœud
  résolu en silence** dans `is_ancestor`/`children_iter` —
  classe de bug correction, pas crash. Le fixup de délink
  exigerait une passe O(vivants) au `collect` + régime
  hybride `u32`/`NodeId` pour ~20 o/nœud de plus : gain
  réel (~19 % slot) jugé insuffisant au regard de la classe
  de risque sur un navigateur. `NonZeroU32` sur
  `generation` refusé aussi : `from_parts(idx, 0)` forgé
  clamé pourrait résoudre une vraie case gen-1.

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
