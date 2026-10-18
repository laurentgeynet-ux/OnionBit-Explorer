# Changelog

Toutes les modifications notables de ce projet sont consignées ici.

Le format suit [Keep a Changelog](https://keepachangelog.com/fr/1.1.0/)
et le projet adhère au [Semantic Versioning](https://semver.org/lang/fr/).

## [Unreleased]

### Added

- **Étape 9 — `explorer-dom` : arène DOM** : `NodeId`
  générationnels (index+génération, handles périmés →
  `DomError::StaleHandle`), nœuds `Document`/`Doctype`/`Element`/
  `Text`/`Comment`, attributs ordonnés, navigation complète,
  `append_child`/`insert_before`/`remove` avec gardes de
  hiérarchie (cycles, parent sans capacité), `remove` détache sans
  détruire, table de **rooting** explicite (`add_root`/`remove_root`,
  cycles DOM↔JS par table — ADR-0008), `collect` mark-sweep réclamant
  les sous-arbres non rootés, `normalize()` fusionnant les `Text`
  adjacents (les nœuds rootés bloquent la fusion), newlines
  `\r\n`→`\n`, `text_content`, budget `max_nodes` borné (ADR-0011).
  13 tests verts.
- **Étape 3 — `explorer-net` : URL WHATWG** (`url.rs`) : parseur
  hiérarchique (`http`/`https`/`file`) + opaque (`data:`/`about:`),
  hôtes IDNA UTS-46 via la crate `idna` (arbitrage consigné
  ADR-0002), IPv4 aux formes WHATWG (octal/hex/réduites `127.1`),
  IPv6 `[…]`, userinfo, ports par défaut élidés, segments `.`/`..`
  normalisés, percent-encoding des jeux path/query/fragment/
  userinfo, résolution relative RFC 3986 §5.2, `Origin` tuple/
  opaque pour la politique same-origin, `path_query` (origin-form).
  27 tests de conformité sous-ensemble WHATWG.
- **Étape 2 — squelette Flutter `app/`** : `flutter create` Windows,
  clean architecture `core/` (`api/` client REST + SSE `dart:io`
  zéro-dépendance avec reconnexion, `design/` tokens + thème dérivé,
  `config/` `AppConfig`, `di/` providers Riverpod) + `features/
  browser/` (entité `BrowserTab`, `BrowserRepository` sur le contrat
  REST ADR-0003, `BrowserPage` coquille omnibox). `assets/design/
  tokens.json` chargé au démarrage avec repli embarqué (ADR-0005) ;
  `ExplorerApp` attend les tokens puis dérive `ThemeData`.
- **Étape 1 — workspace cargo** : 13 crates `explorer-*` squelettes
  (lib/binaire, `#![forbid(unsafe_code)]`, lint clippy pedantic,
  rustdoc), `[workspace]` Cargo.toml, `scripts/verify_all.ps1`
  (fmt/clippy/test/doc + flutter conditionnel).
- **`dillo-js` dans le workspace** : le moteur JavaScript est devenu
  `crates/dillo-js` — **version canonique et unique**, licence
  `MIT OR Apache-2.0`, chantier de l'équipe JS dédiée (ADR-0007
  révisé). Le crate a été **retiré du workspace du port voisin** —
  son `dillo-browser` consomme désormais le crate d'ici via la dep
  `path` `../OnionBit-Explorer/crates/dillo-js` (jonction locale
  `D:\Projet\OnionBit-Explorer` → `C:\OnionBit-Explorer`), en
  attendant la bascule `git`+`rev` à la publication d'Explorer.
- **ADR-0012** — pipeline incrémental : scheduler de
  sous-ressources dans `core`, entrée tokenizer par chunks, modèle
  d'invalidation style/layout/paint en trois niveaux.
- **Jalons J0–J5** dans `ROADMAP.md` : repères de motivation, dont
  J2 (`explorer-cli dump` — premier artefact visible dès la fin de
  phase 2, avant tout pixel).
- **Chantier `dillo-js`** (piste parallèle équipe JS, ROADMAP
  phase 8) : JS-A alignement heap → ADR-0008 + GC mark-sweep
  (prérequis de l'étape 34), JS-B harness test262, JS-C couverture
  (modules ES, `Proxy`, `Intl`), JS-D médias.

### Changed

- **Rebranding** : le projet s'appelle désormais « Explorer » (nom
  de travail — le répertoire physique sera renommé à la fermeture
  de l'IDE). Tous les documents ont été débarrassés des références
  à l'ancien écosystème ; les coutures (proxy, profil déplaçable,
  transport injectable) restent ouvertes sous justification
  propre.
- **Roadmap durcie** : politique cookies complète (`SameSite`,
  `__Host-`, Public Suffix List) en étape 7 ; `Referer`/`Origin`/
  mixed-content en étape 8 ; entrée tokenizer incrémentale (étape
  10) ; `position`/`z-index`/`overflow`/stacking/`list-item`/
  `white-space` + pseudo-classes dynamiques (étapes 16/21) ;
  `FontProvider` comme port injecté (étape 18, ADR-0009) ; spike
  ADR-0008 à faux handles + alignement heap `dillo-js` comme
  prérequis de l'étape 34 ; `<svg>` statique en étape 45 ;
  dégradation flexbox du palier 1 assumée et mesurée ; exclusions :
  SVG animé (SMIL) ajouté.
- **ADR corrigés** : référence `tiny-skia` (ADR-0001 : 0005 →
  présente décision) ; ADR-0003 réécrite (justifications propres,
  compromis `?key=` documenté) ; ADR-0004 (glue FFI côté Flutter
  pour la mémoire partagée) ; ADR-0006 (justification licence sans
  lien amont) ; ADR-0007 (moteur en workspace, échappatoire
  `boa_engine` nommée) ; ADR-0008 (contrat imposé à `dillo-js`) ;
  ADR-0009 (frontière I/O `fontdb`/`FontProvider`) ; ADR-0011
  (profils cloisonnés génériques).
- **Règle §4.3 assouplie** : `debug_assert!` permis sur invariants
  internes purs (jamais sur contenu externe).
- `.gitignore` : `app/pubspec.lock` n'est plus ignoré — une
  application Dart commit son lockfile (builds reproductibles).

### Added (fondations, étape 0)

- Fondations documentaires du dépôt (étape 0) : `AGENTS.md`
  (contrat de travail), `ROADMAP.md` (11 phases, étapes 0–48),
  `CHANGELOG.md`, `docs/architecture.md`.
- ADR initiaux dans `docs/architecture/decisions/` :
  - ADR-0001 — périmètre : moteur from scratch, dépendances
    d'infrastructure pur Rust autorisées, moteurs existants exclus ;
  - ADR-0002 — pile réseau : HTTP/1.1 maison sur tokio + rustls,
    `httparse` pour les en-têtes ;
  - ADR-0003 — couture UI : API de contrôle REST + SSE sur loopback,
    l'app Flutter ne touche jamais le core ;
  - ADR-0004 — transport des frames : PNG tiré sur invalidation SSE
    (MVP), WebSocket/texture FFI en voies d'upgrade ;
  - ADR-0005 — design tokens : `app/assets/design/tokens.json`
    source de vérité unique du design system ;
  - ADR-0006 — double licence MIT OR Apache-2.0 ;
  - ADR-0007 — couture JavaScript : trait `JsEngine`, moteur
    `dillo-js` intégré en phase 8 ;
  - ADR-0008 — modèle mémoire DOM ↔ GC figé avant la phase 2 :
    `NodeId` générationnels, rooting explicite, cycles par table de
    callbacks, invalidation de génération documentée ;
  - ADR-0009 — stack texte/fontes : primitives pur Rust (`fontdb`,
    `ttf-parser`, `rustybuzz`, `unicode-bidi`, `unicode-linebreak`),
    runs itemisés, fonte fixture embarquée ;
  - ADR-0010 — corpus de conformité publics (html5lib-tests, WPT,
    test262, Acid2/Acid3), % tracké par phase ;
  - ADR-0011 — modèle de menace : budgets + cloisonnement de session
    actifs, isolation processus différée, réévaluation obligatoire
    en phase 8.

### Changed

- Cap du projet rétabli : **compatibilité avec le web moderne** —
  le but d'un navigateur est d'accéder à tous les sites. Une version
  initiale des documents plafonnait par erreur le périmètre à un
  « navigateur documentaire » (ES3, pas de flexbox/grid) ; la roadmap
  porte désormais une phase 10 « Compatibilité web moderne »
  (HTTP/2+, flexbox/grid, ES6+, Web APIs, médias, webfonts) et les
  ADR 0001/0004/0007 ont été corrigés en conséquence.
- Optimisations d'architecture et de roadmap (ADR-0004, 0006, 0007,
  `ROADMAP.md`, `architecture.md`) :
  - **Licence `dillo-js` tranchée** : double licence `MIT OR Apache-2.0`
    formalisée par son auteur unique (Laurent Geynet), éliminant tout
    risque de contamination GPL-3.0 dans `explorer-js`.
  - **Gestion mémoire DOM ↔ GC** : `NodeId` générationnels (`slotmap`)
    et système de handles sécurisés pour prévenir les *dangling IDs*
    lors du détachement de nœuds par du script.
  - **Transport de frames accéléré** : intégration dès la phase 6/7
    d'un buffer brut de pixels décompressés en mémoire partagée /
    named pipe local, cantonnant le pull PNG au MVP strict.
  - **Outillage visuel layout précoce** : export et dump wireframe
    dans `explorer-cli` dès la phase 4 pour calibrer le layout sans
    attendre le pipeline de rendu complet.
  - **Paliers de compatibilité web** : découpage formalisé en 3
    paliers structurants (Documentaire → Web 2.0/SSR → Applications riches SPA).
- Optimisation documentaire suite à revue critique (ADR-0001,
  0002, 0003, 0004, 0007, `ROADMAP.md`, `architecture.md`,
  `AGENTS.md`, `README.md`) :
  - **Cap reformulé** : « tous les sites » devient une direction
    asymptotique pilotée par paliers mesurés sur corpus (ADR-0001,
    ADR-0010).
  - **Réseau** : IDNA/punycode, schémas `data:`/`about:`, HSTS,
    `X-Content-Type-Options`, transcodage charset + sniffing WHATWG
    — `windows-1252` avancé aux phases 1–2 (défaut réel de la spec
    HTML), plus phase 10 (ADR-0002).
  - **Entrées** : coalescing des événements mouvement documenté
    (ADR-0003) ; tension texture GPU ↔ isolation de processus
    explicitée pour l'étape 47 (ADR-0004).
  - **JS** : modèle mémoire DOM ↔ GC délégué à ADR-0008, mesure
    test262, échappatoire ES6+ formalisée (ADR-0007).
  - **Roadmap** : étapes 3, 6, 8–10, 12, 16, 18, 20, 23, 26, 34,
    48 enrichies ; référence tiny-skia corrigée (ADR-0001, pas
    ADR-0005) ; section « Hors périmètre assumé » ajoutée.

