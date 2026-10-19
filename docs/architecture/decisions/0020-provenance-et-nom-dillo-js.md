# ADR-0020 — Provenance et nom de `dillo-js`

Statut : Acceptée (2026-10-19) — la provenance est figée ici ;
le renommage est décidé et séquencé avant la release 0.2.

## Contexte

Le crate s'appelle `dillo-js`. Le nom suggère un lien avec
Dillo (navigateur GPL-2.0) —
un port, une importation, un héritage de code. C'est un signal
faux : première question de tout relecteur, et AGENTS.md §9 doit
déjà la désamorcer. On paie en interrogation un nom qui ne décrit
pas la réalité.

## Provenance (figée)

- `dillo-js` est un **moteur JavaScript original** écrit par
  Laurent Geynet — **né dans un projet JS-Rust personnel
  antérieur** (le « chantier voisin » d'AGENTS.md §9), repris
  dans ce workspace — ~32 000 lignes sous `crates/dillo-js`,
  licence `MIT OR Apache-2.0` (ADR-0006, en-têtes de
  fichiers). Cette antériorité explique son avance de phase :
  capital hérité, pas effort mal placé.
- **Aucune ligne issue de Dillo** — ni code, ni architecture.
  Le nom est une **ironie de l'auteur** : Dillo est le
  navigateur qui refuse JavaScript par principe — baptiser
  ainsi le moteur JS de son projet était le clin d'œil.
  Le développement continu JS-C1 → JS-C25+ est traçable dans
  `CHANGELOG.md`.
- Le `Dom` interne de `dillo-js` est un **harnais/miroir**
  (ADR-0013), pas un DOM importé ; l'intégration au navigateur
  passe par le trait `JsEngine` d'`explorer-js` (ADR-0007).
- Conformité ADR-0001 : aucun moteur tiers n'est importé dans
  Explorer, sous quelque forme que ce soit.

Cette ADR est la réponse de référence permanente à « est-ce un
port de Dillo ? » : non, et ce paragraphe ne sera pas réécrit.

## Décision

1. **Provenance documentée ici** — clos aujourd'hui, coût nul.
2. **Renommage décidé** : `crates/dillo-js` → `explorer-jscore`
   (convention `explorer-*`, écho assumé à JavaScriptCore,
   distinct de l'adaptateur `explorer-js`).
3. **Séquencé avant l'étape 59 (release 0.2)** — le nom du crate
   est surface publique de packaging ; le faire après la mise en
   paquet serait pire. **Pas maintenant** : du travail est en vol
   dans `crates/dillo-js` ; renommer un répertoire de 32 000
   lignes sous des modifications non commitées dégrade le working
   tree pour zéro gain fonctionnel.
4. Exécution mécanique : renommer le répertoire, `Cargo.toml`
   (membre + `package.name`), déclarations de dépendance,
   `use dillo_js::`, pointeurs docs (`AGENTS.md` §2/§10,
   CODEOWNERS, roadmaps, fuzz). Historique préservé par
   détection de renommage git — un commit dédié `chore:`,
   rien d'autre dedans.

## Conséquences

- Jusqu'au renommage, `dillo-js` reste le nom officiel ;
  AGENTS.md §9 pointe sur cette ADR.
- Les mentions historiques (`CHANGELOG`, `ROADMAP`, ADR-0007
  & co.) ne sont pas réécrites — elles datent d'avant le
  renommage, comme tout nom historique.
- Si un meilleur nom qu'`explorer-jscore` émerge d'ici
  l'exécution, l'amender ici par une note — pas de débat au
  moment du renommage.
- Conséquence structurelle : le moteur a été conçu pour son
  projet d'origine, **pas autour d'un DOM navigateur réel** —
  le miroir synchronisé par sérialisation (ADR-0013) est le
  contournement historique de cette hypothèse d'origine.
  Toute évolution vers un partage réel part de là (JS-C39).
