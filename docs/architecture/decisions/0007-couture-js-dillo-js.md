# ADR-0007 — Couture JavaScript : trait `JsEngine`, moteur `dillo-js` en workspace

Statut : Acceptée (2026-10-18), révisée (2026-10-19) — `dillo-js`
est devenu un crate de ce workspace ; l'équipe JS dédiée y poursuit
son développement.

## Contexte

Le navigateur exécutera du JavaScript via le moteur **`dillo-js`** —
moteur écrit from scratch par Laurent Geynet. Il est né dans un
chantier voisin comme extension satellite (aucune contrepartie amont)
et a été **migré dans ce workspace à l'étape 1** : sa version
canonique est `crates/dillo-js`, sous la même double licence
`MIT OR Apache-2.0` que le reste du projet (l'auteur unique l'a
relicenciée à la migration).

État au moment de la migration : interpréteur tree-walk, arène
`ObjId` (pas encore de passe GC — l'arène meurt avec la page),
couverture ES3/ES5 + subset ES2015 (`let`/`const`, classes,
`Promise` + microtasks, `eval`/`Function` en scope global, regex via
crate `regex`), mini-DOM autonome comme harnais de test, transports
injectables (`XhrTransport`, `CookieJar`).

Questions : comment intégrer un moteur encore mouvant sans coupler
le pipeline navigateur à ses choix internes, et comment gérer les
cycles de vie croisés entre son heap (futur GC) et l'arène DOM ?

## Décision

- `explorer-js` définit le **trait `JsEngine`** (évaluation,
  contexte par onglet, budget d'exécution, bindings DOM) — contrat
  possédé par ce dépôt.
- `dillo-js` reste un crate **autonome** (aucune dépendance interne)
  développé par l'équipe JS dédiée ; il sera adapté derrière le
  trait en phase 8. Son mini-DOM (`dom`/`dom_host`) — initialement
  prévu comme harnais de test — sert finalement de **miroir DOM
  script-facing synchronisé** avec `explorer-dom` (ADR-0013,
  étape 35).
- **Licence** : `MIT OR Apache-2.0`, héritée du workspace — la
  migration a consumé la dépendance externe et le risque copyleft
  avec.
- **Cycle de vie DOM ↔ GC** : le modèle mémoire d'ADR-0008
  (identifiants générationnels, rooting explicite, cycles par table
  de callbacks) est le **contrat imposé à `dillo-js`** : son heap
  devra exposer rooting/trace/finalization. Prérequis de phase 8 :
  alignement validé par un spike — le protocole est déjà testable en
  phase 2 avec de faux handles (étape 12).
- `NullEngine` (toute évaluation → `JsError::Unavailable`) est le
  défaut : le navigateur fonctionne sans JS, conforme à la mission
  « tolérant ».
- Exécution bornée : budget instructions/temps par script, jamais de
  blocage du pipeline (contenu hostile = norme, AGENTS.md §8) —
  `set_op_limit` existe déjà dans `dillo-js`.
- L'event loop JS (timers, handlers DOM) reste dans `explorer-core` ;
  le moteur ne connaît que `eval` + bindings. `dillo-js` expose déjà
  `run_due_timers`/`next_timer_deadline` pompés par l'embedder —
  compatible avec ce partage.
- **Échappatoire nommée** : si `dillo-js` ne suit pas la couverture
  ES6+ au palier 3, `boa_engine` (moteur ECMAScript pur Rust,
  test262 tracké, MIT/Apache) est le remplaçant candidat derrière le
  même trait — ADR d'arbitrage à rédiger à l'approche du palier 3.

## Justification

- Le trait isole l'instabilité de l'API dillo-js : seul
  `explorer-js` absorbe ses changements.
- Co-localiser le moteur avec son contrat (trait + ADR-0008 +
  corpus test262) supprime le risque de dérive inter-dépôts et rend
  l'équipe JS autonome dans un périmètre propre.
- Les identifiants générationnels éliminent le risque de pointeurs
  pendants (*dangling IDs*) lorsque le GC JS référence des nœuds
  détachés de l'arbre DOM actif.
- `NullEngine` permet de développer phases 0–7 sans dépendre de la
  phase 8 — le backend prioritaire n'attend personne.
- Séparer event loop (core) et évaluateur (js) suit la pratique des
  moteurs réels et garde le single-writer du DOM.

## Conséquences

- La couverture du langage grandit en phase 10 (étape 43), mesurée
  sur extraits **test262** (ADR-0010).
- La dégradation sans JS reste un comportement de premier ordre
  (pages consultables même si le script échoue).
- L'équipe JS dédiée travaille dans ce workspace : ses changements
  passent par la même CI (`scripts/verify_all.ps1`), les mêmes
  règles (`forbid(unsafe_code)`, `Result`/`Option`, rustdoc) et les
  mêmes checkpoints documentaires que le reste du moteur.
