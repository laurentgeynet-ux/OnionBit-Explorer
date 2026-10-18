# ADR-0007 — Couture JavaScript : trait `JsEngine`, intégration `dillo-js` en phase 8

Statut : Acceptée (2026-10-18), licence tranchée MIT OR Apache-2.0.

## Contexte

Le navigateur exécutera du JavaScript via le moteur **`dillo-js`** —
moteur écrit from scratch par l'utilisateur (Laurent Geynet) dans le
workspace DilloRust (`D:\Projet\dillorust-OnionBit-TorBrowser`, crate
`dillo-js`). Ce moteur est en cours de codage, démarré sur un subset ES3 ;
son API n'est pas figée et sa couverture du langage grandira
progressivement (ES5 → ES6+) pour suivre l'objectif du navigateur :
le web réel.

Question : comment intégrer un moteur externe encore mouvant sans
coupler le pipeline navigateur à ses choix internes, et comment gérer
les cycles de vie croisés entre le Garbage Collector JS et l'arène DOM ?

## Décision

- `explorer-js` définit le **trait `JsEngine`** (évaluation,
  contexte par onglet, budget d'exécution, bindings DOM) — contrat
  possédé par ce dépôt.
- `dillo-js` sera adapté derrière ce trait en phase 8, via
  dépendance `path`/`git` épinglée par `rev`.
- **Licence** : l'auteur unique de `dillo-js` étant Laurent Geynet,
  le crate `dillo-js` est formellement fourni sous double licence
  `MIT OR Apache-2.0` pour être intégré en dépendance directe dans
  `explorer-js` sans conflit GPL.
- **Cycle de vie DOM ↔ GC** : l'arène DOM (`explorer-dom`) utilise des
  **identifiants générationnels** (`NodeId` index + génération, type
  `slotmap`) et un mécanisme de *rooting* (handles JS vers DOM). Si un
  script détache un nœud (`node.remove()`), l'accès ultérieur via JS
  ne produit jamais d'accès mémoire invalide ou d'aliasing (détection de
  génération obsolète).
- `NullEngine` (toute évaluation → `JsError::Unavailable`) est le
  défaut : le navigateur fonctionne sans JS, conforme à la mission
  « tolérant ».
- Exécution bornée : budget instructions/temps par script, jamais de
  blocage du pipeline (contenu hostile = norme, AGENTS.md §8).
- L'event loop JS (timers, handlers DOM) reste dans `explorer-core` ;
  le moteur ne connaît que `eval` + bindings.

## Justification

- Le trait isole l'instabilité de l'API dillo-js : seul
  `explorer-js` absorbe ses changements.
- La double licence par l'auteur garantit une intégration fluide
  en crate standard sans recourir à un IPC de contournement.
- Les identifiants générationnels éliminent le risque de pointeurs
  pendants (*dangling IDs*) lorsque le GC JS référence des nœuds
  détachés de l'arbre DOM actif.
- `NullEngine` permet de développer phases 0–7 sans dépendre d'un
  chantier externe — le backend prioritaire n'attend personne.
- Séparer event loop (core) et évaluateur (js) suit la pratique des
  moteurs réels et garde le single-writer du DOM.

## Conséquences

- **Licence sécurisée** : aucune contamination GPL dans le binaire
  OnionBit-Explorer grâce à la double licence accordée par l'auteur.

- **Couverture du langage** : le subset ES3 actuel est un point de
  départ, pas un plafond — le trait `JsEngine` n'en dépend pas et le
  moteur devra couvrir ES6+ (classes, promises, modules, `fetch`…)
  au fil des phases de compatibilité ; si `dillo-js` ne peut pas
  suivre, le trait permet de le remplacer ou de le compléter sans
  toucher au core.
- La dégradation sans JS reste un comportement de premier ordre
  (pages consultables même si le script échoue).
- Cross-repo : épingler par `rev` git quand dillo-js aura un
  upstream publié ; en dev local, `path` toléré et documenté.
