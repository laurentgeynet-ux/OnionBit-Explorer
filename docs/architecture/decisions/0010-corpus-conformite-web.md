# ADR-0010 — Corpus de conformité : html5lib-tests, WPT, test262 comme mesure

Statut : Acceptée (2026-10-19).

## Contexte

« Compatible web » sans corpus standard est subjectif : un corpus
maison mesure surtout ce qu'on a pensé à tester. L'industrie dispose
de suites publiques éprouvées :

- **html5lib-tests** — fixtures de construction d'arbre HTML
  (format `.dat`, licence MIT) : la référence des tree-builders ;
- **WPT** (web-platform-tests, BSD) — la suite partagée par tous les
  navigateurs, largement pilotée par JS ;
- **test262** — conformité ECMAScript officielle (s'applique au
  moteur `dillo-js` — crate de ce workspace, équipe JS dédiée ;
  les bindings DOM peuvent en consommer des extraits ciblés) ;
- **Acid2/Acid3** — jalons historiques lisibles, utiles pour
  communiquer un palier.

## Décision

Brancher chaque corpus à sa phase, avec un pourcentage de réussite
publié dans `CHANGELOG.md` :

| Phase | Corpus | Usage |
|---|---|---|
| 2 (étape 12) | html5lib-tests (tree-construction) | golden tests du tree-builder |
| 3–4 (étapes 15–20) | sous-ensembles WPT taggés (selectors, cascade, box) | mesure ciblée, jamais le WPT entier |
| 8 (étapes 34–36) | extraits test262 + tests DOM/bindings | informatif |
| 10 (étape 48) | WPT ciblés + Acid2/Acid3 | jalons de palier, non bloquants |

- Les corpus externes sont extraits par script vers
  `crates/<crate>/tests/conformance/` — **jamais modifiés à la
  main** : une fixture divergente entre dans une skip list
  versionnée et raisonnée, elle n'est pas éditée.
- Licences vérifiées à l'import (html5lib MIT, WPT BSD, test262 —
  compatibles).
- Les paliers de la roadmap deviennent mesurables : « x % de
  html5lib-tests passent », pas « le parseur a l'air de marcher ».

## Justification

- html5lib-tests capture les cas tordus (foster parenting, adoption
  agency, tables) qu'aucun corpus maison n'anticipe — c'est le
  moyen le plus rapide de valider un tree-builder réel.
- Un % tracké transforme la motivation long-terme en signal
  objectif et détecte les régressions à chaque commit.
- Le harness `.dat` est trivial (sections séparées par lignes) —
  coût faible, levier énorme.

## Conséquences

- WPT étant massivement JS-dépendant, seuls des sous-ensembles
  taggés servent avant la phase 8 — pas d'objectif global WPT avant
  le palier 3.
- Les fixtures volumineuses ne sont pas commitées brutes : script
  d'extraction reproductible + skip list versionnée.
- Un écart corpus = un bug corrigé ou un « dégradé assumé » tracé —
  la skip list est elle-même une métrique du palier.
