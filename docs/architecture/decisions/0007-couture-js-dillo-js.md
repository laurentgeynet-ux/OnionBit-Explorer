# ADR-0007 — Couture JavaScript : trait `JsEngine`, intégration `dillo-js` en phase 8

Statut : Acceptée (2026-10-18), point licence ouvert (cf. ADR-0006).

## Contexte

Le navigateur exécutera du JavaScript via le moteur **`dillo-js`** —
moteur ES3-subset écrit from scratch par l'utilisateur dans le
workspace DilloRust (`D:\Projet\dillorust-OnionBit-TorBrowser`,
crate `dillo-js`, ADR-0007 de ce projet — « extension satellite »
non couverte par la fidélité du port). Ce moteur est en cours de
codage ; son API n'est pas figée.

Question : comment intégrer un moteur externe encore mouvant sans
coupler le pipeline navigateur à ses choix internes ?

## Décision

- `explorer-js` définit le **trait `JsEngine`** (évaluation,
  contexte par onglet, budget d'exécution, bindings DOM) — contrat
  possédé par ce dépôt.
- `dillo-js` sera adapté derrière ce trait en phase 8, via
  dépendance `path`/`git` épinglée par `rev`.
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
- `NullEngine` permet de développer phases 0–7 sans dépendre d'un
  chantier externe — le backend prioritaire n'attend personne.
- Séparer event loop (core) et évaluateur (js) suit la pratique des
  moteurs réels et garde le single-writer du DOM.

## Conséquences

- **Licence à re-vérifier en phase 8** : `dillo-js` vit dans un
  dépôt GPL-3.0-or-later ; l'intégrer dans un dépôt MIT/Apache-2.0
  exige soit une relicence/exception du sous-projet, soit un
  fonctionnement en processus/plugin séparé — ADR de phase obligé
  (ADR-0006 §conséquences).
- ES3-subset ≠ web moderne : `document`/DOM basiques seulement — la
  dégradation sans JS est un comportement de premier ordre.
- Cross-repo : épingler par `rev` git quand dillo-js aura un
  upstream publié ; en dev local, `path` toléré et documenté.
