# ADR-0036 — `body_ns` : options pour le bond structurel de dispatch

- **Statut** : proposé (demande lead, campagne N5) · **Date** : 2026-11-15 · **Périmètre** : `explorer-jscore` (interne, aucune casse publique)

## Contexte

La campagne JS-C490→C554 a clos l'avenue des crans
*mesurables en vol* : annotations `(depth, slot)` (C490/502/
533/536), prop ICs (C500/526/528/531), mémos proto/own-absent
(C521b/540/544-546), sondes d'attribution (`idnt:`/`envg:`/
`pscan:`/`pidxm:`/C549b-550), `arguments` annoté (C553),
params au slot canonique (C554). Verdicts vol YouTube :

| Poste | État après crans |
|---|---|
| `env_ns` | ~1 s/vol — marches résiduelles sémantiquement requises (code `eval()`'d, scopes dynamiques) |
| `frame_ns` | ~2,65 µs/appel — prologue serré (LazyArgs, canon, flags cachés) |
| `prop_ns` | servi par ICs/mémos — le mur suivant est `ic_bail`/polymorphisme, borné |
| **`body_ns`** | **dominant et non attaqué** — le tissu interpréteur lui-même |

Mix mesuré (vol yt, ~40 M nœuds `eval:`/session) :
`ident` 39 % · `binary` 14 % · `member` 10 % · `call` 9 % ·
`unary` 6 % · `assign` 6 % · `num` 5 % · `cond` 2,5 %.
Chaque nœud paie : dispatch `eval_inner` (match ~40 variantes),
lookup résolu + clone `Value`, ticks J17 (désarmés : un load
atomique chacun). Le résidu par nœud ≈ **40-80 ns de mécanique
pure** — ~2-3 s cumulés/vol. C'est un coût de *structure*
(dispatch récursif sur AST), pas de chemin manquant : aucun
cran d'annotation ne le touche.

## Décision proposée

Ne pas engager de bond en cran de campagne. Trois options
mises en regard — coûts, mesures attendues, réversibilité :

### Option A — Fusion d'ops (quickening AST) ⬆ recommandée en premier

Au `resolve_lit` (le point où l'AST est déjà parcouru
statiquement), réécrire les motifs chauds en nœuds
**synthétiques** : `Call(Member(o,k), args)` → `MethodCall`,
`Binary(op, Ident, Ident)` → `BinII`, `Ident` seul →
`IdentSlot` (annotation inline dans le nœud, plus de
`resolved_map`). Le dispatch sert un nœud au lieu de trois.

- **Mesure attendue** : −3,5 M `member` + dispatchs sur
  `call`-méthode seule ; chaque fusion est isolément
  comptée (`eval:` étendu aux variantes fusionnées → le
  compteur *prouve* la couverture en vol).
- **Coût** : ~1-2 k lignes — nouvelles variantes `Expr`
  (ou table parallèle `fused_map` ptr→forme si on ne veut
  pas polluer l'AST), bras dans `eval_inner`, résolution
  des sites synthétiques.
- **Réversibilité** : **haute** — flag off par fusion,
  repli textuel possible. C'est la seule option qui
  *mesure ce qu'est le dispatch* avant de payer le bond B.
- **Risque** : sites `resolved_map` clés ptr — les nœuds
  synthétiques n'ont pas de ptr source : table latérale
  ou annotation portée par le nœud fusionné (à trancher au
  prototype). `resolve_verify` garde son rôle de filet.

### Option B — Bytecode / VM à registres

Compiler les corps `FnLit` en bytecode au premier appel
(le point de matérialisation existe déjà — V-5a). Boucle
de dispatch serrée, pile de valeurs explicite : gain
typique ×2-5 sur le tissu (littérature et leçons V-1e :
`call` petit corps ~5 µs dont ~40 % de dispatch AST).

- **Mesure attendue** : `body_ns` ÷2-3 si le dispatch est
  bien le dominant — **mais A doit d'abord prouver que
  c'est le cas** (si `body_ns` est dominé par la sémantique
  — clones `Value`, allocs heap — B ne sert à rien).
- **Coût** : ~10-20 k lignes — compileur, frames VM,
  dual-mode obligatoire pour `GExec` (gen/async
  suspendables suspendent la *pile Rust* aujourd'hui —
  un frame VM doit sérialiser son état, c'est le point
  dur), `eval`, `with`, `using`, destructeurs.
- **Réversibilité** : **quasi nulle** — deux moteurs à
  parité ou migration totale. Point de non-retour réel.

### Option C — Env-recycling (orthogonal, borné)

`Env::local` alloue `Rc<RefCell<Env>>` + `cells` par appel
(`env_new` ~1,9 M/vol) ; les envs sans capture survivante
pourraient être recyclés. Nécessite une **analyse
d'échappement** (une closure qui capture l'env l'interdit).

- **Mesure attendue** : fraction de `frame_ns` + `gc_ns`
  (~1,5 s/vol) — probablement quelques %, pas un bond.
- **Coût** : moyen (compteur de captures au resolve +
  pool borné).
- **Réversibilité** : haute, flag off.
- **Remarque** : faisable en cran de campagne — n'a pas
  besoin de cet ADR ; consigné ici car il ne touche pas
  `body_ns` mais `frame_ns`/`gc_ns`.

## Juge et chemin

1. **A d'abord** : 2-3 fusions pilotes (`MethodCall`,
   `BinII`, `IdentSlot`) → le compteur `eval:` mesure la
   part de dispatch éliminable. Si le gain dépasse ~10 %
   de `body_ns` → la thèse « dispatch dominant » tient et
   B devient défendable ; sinon B est tué *par la mesure*
   avant d'avoir coûté 20 k lignes.
2. C en cran parallèle si `frame_ns`/`gc_ns` restent hauts
   après les fusions.
3. B seulement sur preuve A + proto sur banc
   (`benches/eval.rs` — discipline V-4f : pas de livraison
   sans mesure, le chiffre va en CHANGELOG).

Verrous obligatoires quel que soit le chemin : `lang` 315,
`dom` 269, `j17` armés, `EXPLORER_RESOLVE_VERIFY=1` en vol,
console yt à signatures connues, jamais de régression
`cannot read`/`uncaught` nouvelle.

## Conséquences

- A engage ~2 k lignes réversibles et produit la **mesure
  qui décide B** — le papier avant le code, puis le code
  qui produit le papier suivant.
- B gelé jusqu'à preuve : un bond sans mesure de dispatch
  répéterait l'erreur de la sentinelle C552 à l'échelle
  d'un moteur.
- Rien n'exclut les crans restants : `ic_bail`
  (~2,5 M/vol), `arguments` résiduel `eval()`'d,
  `env_lvl` des marches requises — la campagne
  d'annotation continue en parallèle d'A.
