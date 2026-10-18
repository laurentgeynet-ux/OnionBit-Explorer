# ADR-0008 — Modèle mémoire DOM ↔ GC : identifiants générationnels et rooting explicite

Statut : Acceptée (2026-10-19) — figée **avant** l'écriture de l'arène
`explorer-dom` (phase 2).

## Contexte

La phase 2 construit `explorer-dom` (arène + `NodeId`). La phase 8
branchera un moteur JS à garbage collector dont les objets
référenceront des nœuds vivants. Deux problèmes classiques se
présentent :

1. **Pointeurs pendants** : un script détache un nœud
   (`node.remove()`) alors qu'un handle JS le pointe encore.
2. **Cycles DOM ↔ JS** : un nœud porte un listener/expando vers un
   objet JS qui retient le nœud — aucun ramasse-miettes isolé ne
   peut libérer ce cycle (leçon historique : refcount + GC mixtes =
   fuites ; Servo l'a payé cher).

Le modèle mémoire structure l'API de l'arène DOM : les signatures de
mutation (`remove`, `adopt`, `clone`) dépendent de la politique
d'invalidation. Le trancher après coup, en phase 8, imposerait une
refonte du domaine pur.

## Décision

- **`NodeId` générationnels** (index + génération, pattern
  `slotmap`) confirmés comme fondation — détection des nœuds
  détachés sans risque d'aliasing ni de réutilisation silencieuse.
- **Handles JS → DOM** : tout objet JS référençant un nœud passe par
  un handle inscrit dans la table de roots du contexte JS
  (`explorer-js`). Un accès via une génération obsolète retourne une
  erreur typée — jamais d'accès invalide.
- **Pas de référence DOM → JS directe** : listeners et expandos sont
  des entrées d'une table de callbacks indexée par `NodeId`, tenue
  par `explorer-js` ; le DOM ne connaît que des identifiants
  opaques. Le cycle traverse une frontière que le GC peut tracer —
  collecte par traçage côté JS, jamais par refcount croisé.
- **Invariant** : « le DOM survit sans JS ; le JS ne survit pas sans
  handles valides. » Chaque mutation de l'arène documente dans sa
  rustdoc quelles générations elle invalide.
- Un sous-arbre détaché est recyclable par l'arène quand plus aucun
  handle ne le roote — l'arène reste seule autorité sur la durée de
  vie de ses nœuds.

## Justification

- Les identifiants générationnels rendent la détection quasi
  gratuite — aucun refcount sur l'arbre, aucun `Rc` croisé.
- Séparer « graphe DOM pur » et « graphe d'objets JS » par une table
  de roots suit la pratique des moteurs réels (V8/SpiderMonkey
  tracent des handles, jamais des pointeurs bruts).
- Figer le protocole avant l'arène évite la refonte que cette
  question a coûtée aux moteurs qui l'ont découverte tard.

## Conséquences

- `explorer-dom` expose des mutations typées dont la rustdoc précise
  l'invalidation de génération — testable dès la phase 2 avec de
  faux handles, sans attendre le moteur.
- `explorer-js` possède la table de roots et la politique de
  tracing ; `explorer-core` ne manipule que des `NodeId`.
- **Ce modèle est le contrat imposé à `dillo-js`** (crate de ce
  workspace, équipe JS dédiée — ADR-0007) : son heap `ObjId` devra
  exposer rooting/trace/finalization avant le branchement de
  phase 8. Le spike de l'étape 12 valide le protocole avec de faux
  handles ; la revue de conformité du heap réel est un prérequis
  explicite de l'étape 34.
- Le nombre de roots est borné (contenu hostile = norme) : une page
  qui roote l'arbre entier dégrade en erreur typée, elle ne déborde
  pas.
