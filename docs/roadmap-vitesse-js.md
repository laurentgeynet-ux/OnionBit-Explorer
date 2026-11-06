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

- [x] **V-1a. Boucle lecture de prop monomorphe** — `o.k`
  même shape ×N : coût de la marche proto + `prop_pos` par
  accès (la cible V-4). → **~1.10 µs/lookup**
- [x] **V-1b. Polymorphe** — 2-3 shapes alternées sur le même
  site : dégradation méga-morphique (borne de V-4).
  → **~1.74 µs/lookup (+58 %)**
- [x] **V-1c. Profondeur proto** — `o.k` résolu à 4-6
  niveaux de chaîne (`Array.prototype` → `Object.prototype`) :
  coût par niveau. → **~1.23 µs/lookup (+12 % / +3 niveaux)**
- [x] **V-1d. Accès variable closure** — `Ident` à 3+
  profondeurs de scope : `env_get_seen` marché.
  → **~1.00 µs/iter (2 reads, prof. 2+1)**
- [x] **V-1e. Taux d'appel** — `f()` petit corps ×N : coût
  frame/env par `call_value` (alloc scope incluse).
  → **~4.93 µs/appel — 4.5× un lookup, dominant**
- [x] **V-1f. Parse seul** — `parse_in` `pub(crate)` → proxy
  eval parse-dominé (`0;` ×50k, exécution ~nulle) :
  **~1.08 µs/stmt**, tare `eval("0")` = 1.8 µs.
- [x] **V-1g. Mix composite** — mini-workload « page »
  (objets littéraux + map + accès + appels) : **28.6 µs/iter**
  (~5 calls/iter → call-dominé, cohérent avec V-1e).
  + **bonus `array_read`** (`a[i]`/`length`) : ~1.17 µs/iter.
- **Signal V-1** : `call_rate` ≈ 4.9 µs domine les lookups
  (~1.1 µs) — V-2 doit attribuer frame/env vs résolution
  avant tout fix structurel (la porte peut nommer les
  appels, pas les props).

## V-2 — Compteurs de résolution J17

Nouveaux compteurs sur le mécanisme existant (coût nul
désarmé) — le partage *réel* avant d'optimiser quoi que ce
soit. Dump périodique (à la collect, gabarit `sched.tally`)
ou en fin d'eval.

- [x] **V-2a.** `J17_PROP_LOOKUP` — appels `find_own` totaux.
- [x] **V-2b.** `J17_PROTO_LVL` — somme des niveaux proto
  parcourus (ratio = profondeur moyenne par lookup).
- [x] **V-2c.** `J17_PROP_SCAN` — comparaisons de clés en
  branche linéaire (<24, sans index) — le coût exact de la
  branche V-3a.
- [x] **V-2d.** `J17_PROP_IDX_HIT/MISS/BUILD` — index
  paresseux (≥24 props).
- [x] **V-2e.** `J17_ENV_LVL` — niveaux de scope marchés par
  `env_get_seen`. *Env churn annexe (revue)* : `Env` frais +
  `BindingMap::Small(Vec)` par appel — la création est
  gratuite (`Vec::new()` n'alloue pas), le coût est à la
  **première liaison** (push → alloc + croissance amortie) ;
  scope sans liaison = gratuit, scope avec liaisons = 1
  alloc. Candidat sérieux au top-3, `stats_alloc` le
  nommera.
- [x] **V-2f.** `J17_ARRIDX_TRY` — `array_index(key)` tentés
  (**multiplicité** par `find_own` × niveau, ×2 par niveau
  Array — cible V-3b : hoister, pas garder).
- [x] **V-2g.** `J17_CALL` / `J17_CALL_NAT` — `call_value`
  par classe (script = call − nat) — pèse V-4 vs frame-alloc.
- [x] **V-2h.** **Classe ALLOC/clone (revue)** — `stats_alloc`
  existe déjà (`explorer-common`) : l'attribuer aux phases
  eval. Suspects nommés : `heap.string()` clone chaque
  chaîne lue (`heap.rs:275` → `J17_STR_READ` ajouté),
  `find_own` retourne `Prop` **par valeur** à chaque niveau
  proto.
- [x] **V-2i.** **Coût `probe_lookup` sur le hot path —
  corrigé (revue NAV #2)** : le chemin « armé » était
  **structurel** — `dom_ctx.is_some()` sur toute page
  réelle, `format!` + `HashSet::insert` + `borrow_mut`
  payés par read surveillé *hors* vols (et `format!`
  précédait la dédup → alloc même sur clés vues).
  **Fix livré** : travail gaté par
  `tracing::enabled!(dom_probe, INFO)` sur les deux sites
  (`probe_lookup` + `probe_store` — son `site: String`
  était aussi alloué avant le gate) + `try_borrow_mut`
  (panic réentrant impossible → sonde ratée, jamais de
  crash) + `probe_tagged` ne se peuple que sous sonde
  (état de sonde, pas de moteur). Désarmé = filtres seuls
  (~2 ns, ~0.2 % d'un lookup). `J17_PROBE_HIT` mesure les
  hits structurels en vol → biais des métriques chiffrable.
- [x] **V-2j.** Verrou : `tests/j17_counters.rs` ×4 —
  chemins nommés, index paresseux, snapshot, attribution
  `call_rate` (env_lvl=10.2k / 1.1k calls ≈ 10 niveaux/appel,
  prop_scan=21.7k ≈ 15 cmp/lookup).

**Lien marathon noté (revue NAV)** : le rejet récurrent
`Error: Wa` à `kevlar_base:6892:177` (~65 s, vol 669)
ressemble à un watchdog applicatif — si l'eval est assez
lent pour expirer les timeouts de la page, la vitesse n'est
pas orthogonale à §B-97. L'attribution V-2 **sur page
réelle** (snapshot lu en vol) tranche.

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

- [x] **V-3a. `ptr_eq` fast-path** — `prop_pos` et
  `BindingMap::Small` get/get_mut : `std::ptr::eq` sur
  `&str` (`&raw const`, clippy) puis `memcmp` — clés
  statiques internées via lexer, dynamiques → fallback.
  **Mesuré** : `env_depth` **−8.6 %** (30.1 → 27.5 ms —
  liaisons internées hit `ptr_eq` systématiquement), props
  ~±1 % (objets <5 props, scans déjà courts).
- [x] **V-3b. Hoister `array_index`** — le bail existe
  déjà (`ops.rs:148`) ; le coût était pire que la revue :
  appel par niveau proto + **×3** par `find_own`
  (`args_map`, éléments Array, BigInt `TypedArray`).
  `find_own` → wrapper déléguant à `find_own_ai(id, key,
  ai)` ; hoists aux 4 marches proto (read `find_prop`,
  writes `write_prop` + `prim_set_intercept`,
  `host_method_shadowed`). `read_prop` épargné
  volontairement — sa boucle proxy ne tourne pas dans le
  cas commun, hoister y taxerait chaque read.
- [x] **V-3c. Cache `Program` parse — DÉCISION : non
  retenu** à cette date (JS-C374). V-1f mesure le *coût*
  du parse (~1.08 µs/stmt), pas la *répétition* — or
  chaque `<script>` ne parse qu'une fois par page : un
  cache `hash→Program` ne rapporterait que sur
  `eval()`/`Function()` de source identique (rare hors
  code hostile/bancs). **Sondes posées** pour une
  décision réouvrable sur preuve : `parse_call`/
  `parse_dup` (hash FNV-1a source **+ mode**, TLS borné
  par `j17_on()`, ~1 ns désarmé) aux entonnoirs
  `parse_in`/`parse_module_in` — le ratio mesuré sur
  vols réels tranchera. Si réouvert : clé = (hash, mode)
  + cap borné + eviction ; préconditions inchangées —
  audit `lazy.rs`/`OnceCell` avant partage cross-eval
  (cache idempotent OK, état run-spécifique NON).

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
  `remove_own` `props.remove(i)` (`heap.rs:1761`, shift —
  l'ordre relatif survit → layout post-delete
  déterministe pour `S'`, cohérent avec « delete =
  transition ») ; **5 sites `get_mut`** (`define_data`
  1549, `define_accessor` 1595, `define_accessor_strict`
  1658, `set_own` 1714, `set_accessor` 1732).
  **La direction du flip compte** : dangereux = 3
  (`define_accessor`/`define_accessor_strict`/`set_accessor`
  — Data→Acc sur prop *cachée comme data*) ; bénin = 2
  (`define_data` Data→Data : slot juste, valeur fraîche ;
  `set_own` Acc→Data : jamais cachée, miss → slow path).
  **Arbitrage retenu (revue)** : **garde au read** —
  le hit caché re-vérifie `props[slot].1` kind et bail
  vers le slow path si `Acc` — plutôt que flip=transition
  aux 5 sites : une seule garde au point de lecture vs 5
  bumps répartis, et tout site futur oublié **dégrade**
  vers le slow path au lieu d'un slot fantôme (même
  philosophie que NIL/AR-15 : échec typé, jamais muet).
  Aucun `swap_remove`/`props.insert` trouvé (grep vérifié).
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
