# ADR-0033 — Vivacité de l'arène miroir `explorer-jscore` : complétion conservatrice, pas de génération

- **Statut** : accepté — 2026-11-06
- **Contexte** : chantier ARÈNE, item AR-16 (audit + ADR avant impl).

## Contexte

Le miroir DOM de `explorer-jscore` stocke ses nœuds dans une
arène `nodes: Vec<Node>` indexée par `NodeId = u32` **sans
génération**. Depuis le §B-59, `collect_garbage` libère les
nœuds morts (non attachés, non retenus) dans `free_nodes`,
que `alloc` réutilise — un `NodeId` obsolète peut donc
résoudre **un autre nœud** que celui visé, en silence.

De même, `Heap::alloc` réutilise les tombstones via
`free.pop()` : un `ObjId` obsolète résout l'objet réaffecté
(`Option` partout → jamais de dangling mémoire, mais
confusion sémantique possible hors traçage).

L'invariant documenté (`Dom::extra_kept_nodes`) :
*« un slot réutilisé ne doit jamais aliasser une référence
encore stockée »* — la complétion est assurée par
énumération des tables retenant des ids (`extra_kept_nodes`,
`journal_kept_nodes` → `dead_blocked`, seeds `gc_seed_ids`).

## Audit (AR-16)

Tables `NodeId` non couvertes par l'énumération — écrites
sous l'ancien invariant « `NodeId` jamais réutilisé »
(doc `collect_garbage` devenue fausse) :

- `script_seqs`, `link_seqs` : tampons ordinaux **figés au
  parse** — `currentScript`/`deliver_static_link` résolvent
  un `NodeId` sans vérifier le nœud → mauvais élément /
  `load`·`error` sur mauvaise cible.
- `parse_dyn` : flag `defer_order` lu par `contains` — un
  nœud réutilisé hérite du flag d'un mort.
- `focused`, `dirty_node` : `Cell<Option<NodeId>>` —
  `:focus`/`activeElement`/cible de relayout sur mauvais
  nœud.
- Liens `parent` internes : la fermeture `keep` est
  descendante (descendants) — un nœud retenu (wrapper JS)
  dont le parent détaché meurt voit `parent` pointer vers
  un slot réattribué → `parentNode`/`ancestors`/`closest`
  résolvent le mauvais nœud.

Côté heap : `adopted_sheets: Vec<ObjId>` (DOM) n'est pas
semé dans `gc_seed_ids` → `cssRules`/`cssText` lus sur
l'objet réaffecté.

## Décision

**Compléter l'invariant conservateur, pas de génération.**

1. `extra_kept_nodes` += `script_seqs`/`link_seqs` (values)
   — le design les veut « figés au parse, identifiables » :
   la rétention préserve la sémantique, bornée par le
   nombre de scripts/links statiques.
2. Post-sweep dans `collect_garbage` :
   - **délink parent** — nœud retenu dont le parent est
     mort → `parent = None` (racine de forêt honnête,
     jamais mauvais nœud) ;
   - **purge des flags transitoires** — `parse_dyn`,
     `focused`, `dirty_node` : les entrées pointant des
     slots libérés sont retirées (dégradation honnête
     plutôt que fausse association).
3. `gc_seed_ids` += `adopted_sheets` — les `ObjId` des
   feuilles adoptées sont des racines tant que le document
   vit.
4. Correction du doc `collect_garbage` (`NodeId` **est**
   réutilisé — `dead_blocked` en dépend).

**Refusé** : génération sur `NodeId` miroir / `ObjId`.
Changer la forme des ids touche le journal `DomSync`,
`serde`, et des milliers de sites — rayon d'explosion
démensuré pour un harnais. Le `u32` nu reste acceptable
parce que les lectures dégradent (`Option`/bounds/`free`)
et que l'invariant est maintenu par complétion des seeds —
même arbitrage qu'AR-15 : la sécurité vient de la
vérification à la résolution + la complétion du marquage,
pas de la forme de l'id.

**Règle durable** : toute nouvelle table/champ retenant un
`NodeId` ou `ObjId` DOIT déclarer sa politique de rétention
(`extra_kept_nodes`, `journal_kept_nodes`, `gc_seed_ids`,
ou purge au sweep) — l'audit se relit à chaque ajout.

## Conséquences

- Les trois classes de bug « résolution silencieuse » sont
  fermées sans toucher aux protocoles de sérialisation.
- Le coût est marginal : rétention bornée aux nœuds
  effectivement cités ; une passe `O(kept)` de délink.
- `AR-17` (index `id` miroir) devra respecter la règle :
  la table `id→NodeId` est une référence stockée → soit
  protégée par `extra_kept_nodes`, soit purgée au sweep.
