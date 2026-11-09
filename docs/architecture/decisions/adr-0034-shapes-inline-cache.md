# ADR-0034 — Shapes + cache d'inline global `(shape, clé) → slot`

- **Statut** : proposé (V-4a) · **Date** : 2026-11-10 · **Périmètre** : `explorer-jscore` (interne, aucune casse publique)

## Contexte

`o.k` paie à **chaque** accès : marche proto complète
(`find_prop` jusqu'à `MAX_PROTO_DEPTH` = 512), `find_own`
par niveau — `array_index(key)` reparsé **avant** le lookup
+ dispatch `ObjKind` + `prop_pos` linéaire (`Vec<(Rc<str>,
Prop)>`, seuil 24). Aucun cache de forme : un site
monomorphe refait le chemin entier à chaque itération.

Mesures du banc V-1 (`benches/eval.rs`, criterion) :

| Banc | Mesure |
|---|---|
| V-1a monomorphe `o.k` même shape | **~1.10 µs/lookup** |
| V-1b polymorphe 2-3 shapes | ~1.74 µs/lookup (**+58 %**) |
| V-1c profondeur proto +3 niveaux | ~1.23 µs/lookup (+12 %) |
| V-1d accès variable closure prof. 2+1 | ~1.00 µs/iter |
| V-1e appel `f()` petit corps | ~4.93 µs/appel (dominant) |

Faits structurels vérifiés au code :

- Clés internées (`intern()`, TLS `HashSet<Rc<str>>`) —
  `Expr::Member(.., Rc<str>)` porte déjà l'Rc : `ptr_eq`
  tirera sans précondition, `memcmp` gracieux sinon.
- `push_prop` = **seul writer** des `props` (J15 — revue
  grep : aucun `swap_remove`/`props.insert` hors rail) :
  point d'injection unique pour la table de transitions.
- Zéro cache de parse : un `Program` meurt après éval →
  **les IC par site meurent avec** → le cache retenu est
  **global** `(shape, clé) → slot`, jamais ancré nœud
  (les IC par site reviendront avec l'AST persistant —
  V-5c/VM, hors de cette décision).

## Décision

1. **`ShapeId` sur `JsObj`** + table de transitions dans
   `push_prop` (injection unique) — `shape` = ordre
   d'insertion des clés ; marquage GC des tables.
2. **Lookup** : `find_own` consulte `(shape, key)` →
   `slot` (index dans `props`) → hit → lecture directe
   O(1) ; miss → chemin actuel inchangé → fill du cache.
3. **Mode dictionnaire** : `delete`, proto-swap, accessor
   props, ou dérive structurelle → l'objet bascule hors
   shapes (`shape_epoch` global invalide le cache en une
   génération — jamais de purge table par table).
4. **Verrous obligatoires (V-4e)** : accessors, `delete`,
   swap proto mid-loop, transition dictionnaire,
   alternance polymorphe, suites entières vertes.
5. **Juge** : banc V-1 rejoué (V-4f) — `o.k` monomorphe
   doit approcher O(1) ; le chiffre va en CHANGELOG, pas
   de livraison sans mesure (discipline de la piste).

## Conséquences

- Hot path `get_prop`/`set_own` gagne O(1) sur les sites
  monomorphes (la forme dominante : objets de données,
  littéraux, exports de modules) ; polymorphe dégrade au
  chemin actuel, jamais pire.
- Invariants à tenir : tout accès par index `slot` suppose
  `props[i].0 == key` — les mutations (`delete`, proto)
  passent par `shape_epoch`/dictionnaire, jamais par
  rafistolage de slot.
- `probe_lookup` §B-97 reste après le hit (V-2i a déjà
  gaté son coût hors `dom_probe=info`).

## Alternatives écartées

- **IC par site** : nécessite AST persistant (cache parse
  V-3c puis émission bytecode — horizon V-5c). Le cache
  global garde 90 % du bénéfice pour une fraction du coût.
- **Bytecode/VM** : horizon marathon, hors périmètre —
  cette décision est la dernière optimisation structurelle
  du moteur AST-walking.
