# AGENTS.md — Guide pour agents de codage et contributeurs

Explorer est un **navigateur web écrit from scratch** :
moteur 100 % Rust (URL, HTTP, TLS, HTML, CSS, layout, rendu, JS) et
interface **Flutter** desktop, reliées par une API de contrôle locale
**REST + SSE** (ADR-0003).
Ce fichier est le contrat de travail : le lire en entier avant de
toucher quoi que ce soit.

## 1. Mission et périmètre

- **Moteur écrit ici, pas importé** : chaque couche du pipeline
  (`fetch → parse → style → layout → render → paint`) est du code de
  ce dépôt. On peut utiliser des crates d'infrastructure pur Rust
  (tokio, rustls, httparse, tiny-skia…) ; on n'importe **pas** un
  moteur existant (html5ever, servo, kuchiki…) — voir ADR-0001.
- **Backend d'abord** : le moteur est prioritaire. L'app Flutter
  (`app/`) est un squelette qui évolue avec l'avancement — elle ne
  consomme que l'API, jamais le core (ADR-0003).
- **JS par couture** : le moteur JavaScript `dillo-js` vit dans ce
  workspace (`crates/dillo-js`) — chantier de l'équipe JS dédiée. Il
  sera intégré en phase 8 derrière le trait `JsEngine` (ADR-0007) ;
  son heap exposera les primitives du modèle mémoire ADR-0008.
  Rien d'autre ne lui est couplé.
- **Tolérant, jamais crash** : page malformée, réponse HTTP
  biscornue, CSS cassé → dégradation gracieuse, pas de panic.

## 2. Structure cible

```
crates/
  explorer-common    types/utilitaires partagés, erreurs racines
  explorer-net       URL, DNS, HTTP/1.1, TLS (rustls), cache,
                     cookies, MIME — seul crate qui ouvre des sockets
  explorer-dom       arène DOM + NodeId générationnels (ADR-0008)
  explorer-html      tokenizer + tree-builder HTML → explorer-dom
  explorer-css       tokenizer, parser, sélecteurs, cascade
  explorer-js        trait JsEngine + bindings DOM (couture moteur)
  dillo-js           moteur JavaScript autonome — équipe JS dédiée,
                     adapté derrière JsEngine en phase 8 (ADR-0007)
  explorer-layout    arbre de boîtes, block/inline, dimensionnement
  explorer-render    display list → framebuffer RGBA → PNG
  explorer-core      session : onglets, navigation, pipeline, events
  explorer-api       REST + SSE (axum) — couture unique vers l'UI
  explorer-cli       binaire de dev/debug (fetch, dump DOM, shot)
  explorer-daemon    binaire hôte : moteur + API loopback
  explorer-test-support  transports simulés, fixtures
app/                 app Flutter : lib/core (api, design tokens, di)
                     + lib/features/<feature>/ (clean architecture)
docs/architecture/   ADR (decisions/) + architecture.md
docs/                documents de référence
fuzz/                cibles cargo-fuzz (hors workspace)
scripts/             verify_all.ps1 + utilitaires
```

Sens de dépendance strictement descendant :
`common ← net|dom ← html|css ← layout ← render ← core ← api ← daemon`,
`common ← dom ← js ← core`, `dillo-js ← js`, `net ← core`. Jamais
de dépendance montante ni latérale ; le code partagé descend d'une
couche (ou ADR). `dillo-js` est autonome (aucune dépendance interne,
son mini-DOM est un harnais de test). `explorer-net` est le seul
crate autorisé à ouvrir des sockets.

## 3. Commandes (avant chaque commit)

```powershell
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo doc --workspace --no-deps   # zéro warning

# Si app/ touché :
cd app; flutter analyze; flutter test
```

Ou tout d'un coup : `pwsh -File scripts/verify_all.ps1`.

## 4. Règles dures

1. **`#![forbid(unsafe_code)]`** dans tous les crates `explorer-*`.
   Les dépendances qui utilisent unsafe en interne sont OK ; notre
   code non. Si une fonctionnalité OS n'a vraiment pas d'équivalent
   safe → ADR d'abord.
2. **Pas de nouvelle dépendance sans justification** (ADR ou message
   de commit). Crates publiées depuis ≥ 7 jours, épinglées en version
   mineure dans `[workspace.dependencies]`. **Pur Rust uniquement** —
   un crate qui lie du C (`*-sys` sans feature pure Rust) casse la
   mission.
3. **Pas de `unwrap()`/`expect()`/`panic!`/`todo!()` dans le code
   livré.** Toléré en tests et dans les binaires. Les bibliothèques
   renvoient `Result`/`Option` — un navigateur est un logiciel
   tolérant : dégrader, jamais planter. `debug_assert!` est permis
   sur les invariants internes purs (jamais sur du contenu externe
   ou hostile : là, on dégrade).
4. **`CHANGELOG.md` à jour** (Keep a Changelog) à chaque changement
   visible, dans le même commit.
5. **`ROADMAP.md` coché dans le même commit** que l'étape terminée.
   Un commit de code sans sa doc est incomplet.
6. **Nommage** : `snake_case` modules/fonctions, `CamelCase` types.
   Rustdoc sur les items publics ; invariants non triviaux documentés.
7. **Erreurs** : enums `thiserror` par crate, alias `Result<T>`.
   Jamais `Box<dyn Error>` dans les API publiques des bibliothèques.
8. **Async / modèle d'événements** : tout I/O sur tokio (runtime
   unique du workspace — ADR-0002 pour la pile réseau).
   Le layout et le DOM vivent sur un flux unique (single-writer) —
   jamais d'`Arc<Mutex>` sur l'arbre de widgets/DOM. Réseau et
   décodeurs tournent en tâches tokio et livrent des chunks bornés
   par canaux.
9. **Tracing** : crate `tracing`, pas de `println!` hors
   binaires/tests.
10. **Sockets uniquement dans `explorer-net`**. Cache, cookies, DNS,
    TLS sont injectables pour que les tests tournent in-process.
11. **En-tête de licence sur chaque fichier source** (RS/Dart/PS1) :
    `This file is part of Explorer.` + `Copyright (C) 2026
    Laurent Geynet` + `SPDX-License-Identifier: MIT OR Apache-2.0`.
12. **Code et docs en français** pour les documents internes ; les
    identifiants et le code restent en anglais.
13. **Aucune valeur en dur** : seuils, timeouts, ports → structs de
    config.
14. **Scripts `.ps1`** : UTF-8 **avec BOM** ; invoquer `pwsh` (7+),
    jamais `powershell` dans les appels imbriqués. Ne pas réécrire de
    fichiers via `Set-Content`/`-replace` depuis PowerShell.

## 5. Workflow par étape

1. **Choisir une étape** de `ROADMAP.md` dans l'ordre des phases.
2. **Implémenter** dans le crate cible (API d'abord, docstring avant
   le code non trivial).
3. **Tester** : tests unitaires + tests d'intégration in-process ;
   fixtures dans `crates/<crate>/tests/data/`. Golden tests de rendu
   via framebuffer headless (PNG attendu vs produit). Corpus de
   conformité publics (html5lib-tests, WPT, test262) extraits par
   script vers `tests/conformance/`, jamais édités à la main —
   skip list versionnée, % tracké dans CHANGELOG (ADR-0010).
4. **Documenter** : `ROADMAP.md` checkbox, `CHANGELOG.md`, ADR si
   décision d'architecture — même commit.
5. **Commit** : `feat(<crate>): <sujet>` / `fix(<crate>): …` /
   `docs: …` / `chore: …` — une étape = un commit dédié.

## 6. Clean architecture

- **Domaine** (`dom`, `css`, `layout`) : pur, aucun I/O, aucune
  dépendance async — testable à la nanoseconde.
- **Application** (`core`) : cas d'usage (navigate, new_tab…) et
  ports (`HttpTransport`, `JsEngine`) — orchestre, ne sait pas
  comment on ouvre une socket.
- **Infrastructure** (`net`, `render`, `js` impl) : implémentations
  des ports — tokio, rustls, tiny-skia.
- **Interface** (`api`, `daemon`, `cli`, `app/`) : adaptateurs —
  REST/SSE côté moteur, widgets côté Flutter.
- Règle de dépendance : les couches internes ne connaissent jamais
  les couches externes. Les DTO de l'API ne fuient pas dans le core.

## 7. Côté Flutter (`app/`)

- Clean architecture en features : `lib/features/<f>/` =
  `domain` (entités, use-cases), `data` (repos, sources API),
  `presentation` (widgets, providers Riverpod).
- `lib/core/` : `api/` (client REST + SSE), `design/` (tokens +
  thème), `config/`, `di/`, `router/`, `theme/`.
- **Design tokens** : source de vérité = `app/assets/design/
  tokens.json` (ADR-0005), chargé au démarrage — aucune couleur,
  espacement ou rayon en dur dans les widgets.
- L'app ne connaît que le contrat REST/SSE/PNG — si un besoin exige
  plus (perf frames), ADR avant tout canal additionnel.

## 8. Pièges connus

- **Le web est malformé** : HTML tag-soup, CSS cassé, en-têtes
  HTTP fautifs — toujours dégrader, jamais `unreachable!`.
- **Rendu incrémental** : le pipeline doit accepter des données
  partielles (page qui se peint pendant le téléchargement) —
  prévoir les points d'invalidation dès le départ.
- **Charset ≠ transfer-encoding** : `gzip`/`chunked` (transport) ne
  se confondent pas avec UTF-8/windows-1252 (document).
- **Une frame = un état cohérent** : ne jamais rasteriser un arbre
  de layout en cours de mutation.
- **Budgets** : scripts, parseurs et layout reçoivent des bornes
  (temps, mémoire, profondeur) — le contenu hostile est la norme.
- **Texte = iceberg** : shaping, bidi, line-breaking structurent
  l'inline — porter des runs itemisés, jamais des chaînes plates
  (ADR-0009).
- **DOM ↔ GC** : chaque mutation de l'arène documente les
  générations invalidées ; un handle JS obsolète = erreur typée,
  jamais d'accès invalide (ADR-0008).
- **Corpus > intuition** : la compat se mesure en % sur les suites
  publiques trackées (ADR-0010), pas sur « ça a l'air de marcher ».

## 9. Ce qui n'est PAS ce projet

- Pas un port de Dillo — `dillo-js` a une histoire dans un chantier
  de port voisin mais aucune architecture n'en est importée :
  l'adaptation passe par le trait `JsEngine` (ADR-0007).
- Pas un navigateur Tor ni proxy : la politique réseau (mandataire,
  filtrage, `.onion`) est déléguée à l'application qui embarque le
  moteur — garder les coutures ouvertes (`HttpTransport` injectable,
  `http_proxy`/`no_proxy` honorés, profil déplaçable).
- Pas de moteur tiers (ADR-0001) — le cap est la compatibilité avec
  le web moderne : toutes les couches de la roadmap convergent vers
  la navigation réelle. Cap exprimé en **direction asymptotique** :
  « tous les sites » est un horizon (aucun moteur hors des trois
  grands n'y parvient), piloté par paliers mesurés sur corpus
  (ADR-0010), pas une promesse.
