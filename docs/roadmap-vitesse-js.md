# Roadmap — Chantier VITESSE-JS (piste V)

Piste dédiée à la **vitesse de l'interpréteur**
`explorer-jscore` — les chemins de résolution (propriétés,
liaisons, parse) qui taxent *chaque* tâche de *chaque* site,
contrairement aux murs de compat nommés par les vols (piste
terrain, marathon séparé). Issue de l'audit post-ARÈNE
(2026-11-06) : l'arène est rapide, l'eval reste le
multiplicateur silencieux.

**Désignation `V-N`** — distincte de `R-N` (PIPE), `Étape NN`,
`JS-CNN` (NAV), `AR-N` (arène). L'étape est re-numérotée dans
la roadmap NAV au moment de la prise (JS-Cxx si c'est le
spécialiste JS qui la livre).

**Périmètre** : `explorer-jscore` (autonome, aucune dépendance
interne — AGENTS.md). Aucune casse publique attendue : tout
est interne au moteur. Discipline héritée d'ARÈNE : **mesure
avant chaque optimisation** — un palier qui ne mesure pas
son gain n'est pas livré ; le chantier peut s'arrêter après
V-2 si les compteurs disent « la résolution n'est pas le mur ».

Légende : `[ ]` à faire · `[~]` en cours · `[x]` terminée.

## Faits de terrain (vérifiés au code)

| Constat | Preuve |
|---|---|
| `o.k` = marche proto complète à chaque accès : `find_prop` boucle jusqu'à `MAX_PROTO_DEPTH` (512), `find_own` à chaque niveau → `array_index(key)` parsé **avant** le lookup + dispatch `ObjKind` + `prop_pos`. Aucun cache de forme — un site monomorphe refait le chemin à chaque itération. **Bonus revue** : `probe_lookup` (sonde §B-97) tourne à chaque `get_prop` — instrumentation vivante sur le hot path (compare `id == self.global` + scan `probe_tagged`). | `interp/mod.rs:5773-5799`, `:5490-5530`, `:5829` |
| `props: Vec<(Rc<str>, Prop)>` — clés internées (`intern()`, TLS `HashSet<Rc<str>>`) ; `prop_pos` linéaire <24 props (`&**k == key`), index `HashMap` paresseux au-delà. Le lexer interne chaque `Tok::Ident` → `Expr::Member(.., Rc<str>)` porte déjà l'Rc de la table : `ptr_eq` tirera **sans précondition** (clés dynamiques `o[computed]` non internées → miss ptr_eq → fallback `memcmp` gracieux). | `heap.rs:949`, `:1459-1513`, `lib.rs:111`, `lexer.rs:616` |
| **Correction revue** : `array_index` bails déjà au premier byte non-digit (`all(is_ascii_digit)`) — le coût réel n'est pas le parse mais la **multiplicité** : appelé par niveau proto dans `find_own` (avant même le check `ObjKind::Array`) et **deux fois** par niveau Array. La grammaire est déjà bonne (leading-zero refusé). | `interp/ops.rs:146-155`, `interp/mod.rs:5493,5523` |
| Liaisons : `BindingMap::Small(Vec) <16` linéaire → `Map` au seuil ; `env_get` → `env_get_seen` marche la chaîne de scopes à chaque accès variable. Kevlar = closure-heavy. | `heap.rs:1975-2043`, `interp/mod.rs:4322` |
| `eval`/`eval_named` re-parse à **chaque** appel (`parse_in` + `run_program` sur AST) — zéro cache. Moteur par document → `base.js` re-parsé à chaque navigation. Parse déjà chronométré (`note_parse`, `emit_eval_checkpoint`) — mesurable sans ajout. | `interp/mod.rs:2923-2946`, `:3041` |
| AST : `Expr::Member(Box<Expr>, Rc<str>)` = site `o.prop` ; `Expr::Index` dynamique ; `Expr::Ident` accès variable. **Sans cache de parse, les IC par site meurent avec chaque `Program`** → le design IC retenu est global `(shape, clé) → slot`, pas d'ancrage nœud. | `ast.rs:570-572` |
| Instrumentation `J17_*` existante (allocs par site, `j17_tick`, désarmée par défaut, `EXPLORER_J17_PROF=1`) — gabarit prêt pour les compteurs de résolution. | `AGENTS.md` §3 |

## V-1 — Banc de vérité

`crates/explorer-jscore/benches/eval.rs`, criterion 0.5
(gabarit `explorer-dom/benches/arena.rs` ; nouvelle
dépendance → justification dans le commit, règle 2 —
version publiée >7 jours). Chaque scénario mesure ns/op ;
les baselines sont enregistrées en commentaire dans le
banc — c'est le juge de V-3/V-4.

- [ ] **V-1a. Boucle lecture de prop monomorphe** — `o.k`
  même shape ×N : coût de la marche proto + `prop_pos` par
  accès (la cible V-4).
- [ ] **V-1b. Polymorphe** — 2-3 shapes alternées sur le même
  site : dégradation méga-morphique (borne de V-4).
- [ ] **V-1c. Profondeur proto** — `o.k` résolu à 4-6
  niveaux de chaîne (`Array.prototype` → `Object.prototype`) :
  coût par niveau.
- [ ] **V-1d. Accès variable closure** — `Ident` à 3+
  profondeurs de scope : `env_get_seen` marché.
- [ ] **V-1e. Taux d'appel** — `f()` petit corps ×N : coût
  frame/env par `call_value` (alloc scope incluse).
- [ ] **V-1f. Parse seul** — `parse_in` sur source
  synthétique représentatif (~100 Ko-1 Mo de code objet
  dense) : mesure si V-3c est justifié (seuil de décision :
  parse > ~5 % d'un eval réaliste).
- [ ] **V-1g. Mix composite** — mini-workload « page »
  (objets littéraux + map + accès + appels) : le nombre
  d'ordre global, point de comparaison final.

## V-2 — Compteurs de résolution J17

Nouveaux compteurs sur le mécanisme existant (coût nul
désarmé) — le partage *réel* avant d'optimiser quoi que ce
soit. Dump périodique (à la collect, gabarit `sched.tally`)
ou en fin d'eval.

- [ ] **V-2a.** `J17_PROP_LOOKUP` — appels `find_own` totaux.
- [ ] **V-2b.** `J17_PROTO_LVL` — somme des niveaux proto
  parcourus (ratio = profondeur moyenne par lookup).
- [ ] **V-2c.** `J17_PROP_SCAN` — comparaisons de clés en
  branche linéaire (<24, sans index) — le coût exact de la
  branche V-3a.
- [ ] **V-2d.** `J17_PROP_IDX` — hits/miss de l'index
  paresseux (≥24 props).
- [ ] **V-2e.** `J17_ENV_LVL` — niveaux de scope marchés par
  `env_get_seen`. *Env churn annexe (revue)* : `Env` frais +
  `BindingMap::Small(Vec)` par appel — même `Small` alloue ;
  candidat sérieux au top-3, `stats_alloc` le nommera.
- [ ] **V-2f.** `J17_ARRIDX` — `array_index(key)` tentés
  (**multiplicité** par `find_own` × niveau, ×2 par niveau
  Array — cible V-3b : hoister, pas garder).
- [ ] **V-2g.** `J17_CALL` — `call_value` par classe
  (script/natif) — pèse V-4 vs frame-alloc.
- [ ] **V-2h.** **Classe ALLOC/clone (revue)** — `stats_alloc`
  existe déjà (`explorer-common`) : l'attribuer aux phases
  eval. Suspects nommés : `heap.string()` clone chaque
  chaîne lue (`heap.rs:275`), `find_own` retourne `Prop`
  **par valeur** à chaque niveau proto.
- [ ] **V-2i.** **Coût `probe_lookup` sur le hot path** —
  l'instrumentation §B-97 s'exécute à chaque `get_prop`
  (`interp/mod.rs:5798`) : mesurer son coût désarmé
  (compare `id == self.global` + `probe_tagged.iter().find`)
  — si non négligeable, la garder derrière le flag J17.
- [ ] **V-2j.** Verrou : `EXPLORER_J17_PROF=1` sur micro-script
  → compteurs non nuls ; suite `cargo test` verte (désarmé =
  comportement identique).

**Porte de décision** (formulation de la revue) : si
`PROP_*` + `ENV_*` + parse dépassent **~30-40 %** de l'eval
réel page, le chantier est le plus gros trou *actionnable*
du moteur et V-3/V-4 se justifient ; sinon pivot vers ce
que les compteurs nomment (allocs/natives — la classe
V-2h existe pour ça) — coût du cas défavorable : deux
étapes de mesure. Discipline AR-15.

## V-3 — Micro-wins sans risque

Chaque sous-étape = un commit indépendant avec banc
avant/après dans le message. Ordre = risque croissant.

- [ ] **V-3a. `ptr_eq` fast-path** — `prop_pos` et
  `BindingMap::Small` : `std::ptr::eq(&**k, key) ||
  &**k == key`. **Zéro précondition** (revue) : clés de
  requête statiques déjà internées via lexer → `Member` ;
  clés dynamiques → miss → `memcmp` fallback. Signature
  inchangée (`&str` des deux côtés).
- [ ] **V-3b. Hoister `array_index`** — le bail existe
  déjà (`ops.rs:148`) ; le coût = appel par niveau proto
  + ×2 par niveau Array dans `find_own`. Calculer **une
  fois** par `find_own` (ou le passer en paramètre) —
  micro-fix réel, plus petit que prévu.
- [ ] **V-3c. Cache `Program` parse** — *seulement si V-1f
  le justifie* : table TLS `hash(src) → Rc<Program>`
  (gabarit `intern()`/`Atom`), cap borné + eviction.
  **Revue — deux conditions** : (1) le mode
  script/module (`parse_in(src, false, …)`) entre dans la
  **clé**, pas juste `hash(src)` ; (2) audit
  **immutabilité AST au run** — `lazy.rs` existe : si
  `FnLit` porte un `OnceCell` de compilation paresseuse,
  le partage cross-eval doit être idempotent (cache OK,
  état run-spécifique NON).

## V-4 — Shapes + cache global `(shape, clé) → slot`

Le fix structurel. **ADR-0034 avant code** — le design est
arbitré ici :

- **Shapes cachées** : `JsObj.shape: ShapeId` ; `push_prop`
  transitionne dans une table globale `shape → (clé →
  prochain shape + slot)` (gabarit hidden-classes).
- **Cache global** `HashMap<(ShapeId, Rc<str>), Slot>` —
  *pas* d'IC par site : les shapes étant immuables, une
  entrée `(S,k)→slot` est valide pour toujours → **zéro
  invalidation de structure**. Pas d'ancrage AST requis
  (résout le problème « IC par site meurt avec le
  `Program` »).
- **Mode dictionnaire** : objets volatils (add/delete
  fréquents, `defineProperty` dynamique) → transition capée
  puis `shape = DICTIONARY` → skip cache, repli `prop_pos`
  actuel (dégradation honnête = V8).
- **Invalidations restantes** : proto-mutation
  (`__proto__`/`setPrototypeOf`) → bump `shape_epoch` global
  (les entrées référencent l'époque → miss → refill) ;
  props `Acc` → jamais cachées (miss permanent) ; `delete`
  → transition shape (les entrées de l'ancien shape
  deviennent inaccessibles naturellement).
- **Surface de mutation `props` — inventaire exigé dans
  ADR-0034 (revue)** : le cache `(shape,key)→slot` suppose
  l'index `Vec` stable par shape ; `push_prop` est seul
  append-only (J15 `heap.rs:1527`) mais **les autres
  chemins mutent longueur/ordre/kind** — surface vérifiée :
  `remove_own` `props.remove(i)` (`heap.rs:1761`, décale
  toutes positions > i → transition shape obligatoire),
  **5 sites `get_mut`** retournant le kind in-place
  (`heap.rs:1549/1595/1658/1714/1732` — conversion
  data→accessor **post-cache** : soit flip = transition,
  soit le read caché re-vérifie `props[slot].1` kind —
  « accesseurs jamais cachés » ne couvre pas une prop
  *devenue* accesseur). Aucun `swap_remove`/`props.insert`
  trouvé (grep vérifié) — un seul site oublié = slot
  fantôme.
- [ ] **V-4a.** ADR-0034 : design + mesures V-1/V-2 citées.
- [ ] **V-4b.** `ShapeId` sur `JsObj` + table de transitions
  dans `push_prop` (J15 : le seul writer — point d'injection
  unique) + marquage GC des tables.
- [ ] **V-4c.** Lookup : `find_own` consulte le cache à
  `(shape, key)` → miss → chemin actuel → fill.
- [ ] **V-4d.** Mode dictionnaire + `shape_epoch`.
- [ ] **V-4e.** Verrous : accessor props, `delete`, proto
  swap mid-loop, dictionary transition, alternance
  polymorphe, suites entières vertes.
- [ ] **V-4f.** Banc V-1 rejoué : `o.k` monomorphe doit
  approcher O(1) — chiffre en CHANGELOG.

## Ouvertures (hors file)

- **V-5. Bytecode/VM** — l'horizon : stack machine, IC par
  site persistants, registre allocation. Marathon, hors de
  ce chantier.
- **IC par site (V-4 bis)** — si un jour le cache `Program`
  (V-3c) rend les AST persistants, les IC par site redeviennent
  viables et gagnent le hash du cache global.

## Traçabilité constat → étape

| Constat | Étape(s) |
|---|---|
| Marche proto + lookup par accès, zéro cache de forme | V-1a/c, V-2a/b, V-4 |
| `probe_lookup` §B-97 sur le hot path à chaque `get_prop` | V-2i |
| Branche linéaire <24 compare chaînes (interning inutilisé) | V-2c, V-3a |
| `array_index` multi-évalué par `find_own` (×niveau, ×2 Array) | V-2f, V-3b |
| Re-parse par eval, zéro cache | V-1f, V-3c |
| Scopes marchés + `BindingMap::Small` allouée par appel | V-1d, V-2e |
| Frame/env alloc par appel | V-1e, V-2g |
| Clones/allocs intra-lookup (`heap.string`, `Prop` par valeur) | V-2h |
| Mutations `props` hors `push_prop` (remove/get_mut) | V-4 (ADR-0034) |
| IC par site impossible sans AST persistant | V-4 (cache global) |
