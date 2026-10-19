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
  workspace (`crates/dillo-js`), intégré derrière le trait
  `JsEngine` (ADR-0007) — périmètre de l'équipe NAVIGATEUR (§10),
  rôle « spécialiste JS ». Rien d'autre ne lui est couplé.
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
  explorer-js        trait JsEngine + adaptateur DilloEngine +
                     NullEngine — équipe NAVIGATEUR (§10)
  dillo-js           moteur JavaScript autonome — équipe
                     NAVIGATEUR, spécialiste JS (ADR-0007)
  explorer-layout    arbre de boîtes, block/inline, dimensionnement
  explorer-render    display list → framebuffer RGBA → PNG
  explorer-core      session : onglets, navigation, pipeline, events
  explorer-api       REST + SSE (axum) — couture unique vers l'UI
  explorer-cli       binaire de dev/debug (fetch, dump DOM, shot)
  explorer-daemon    binaire hôte : moteur + API loopback
  explorer-ffi       mode in-process cdylib (ADR-0018)
  explorer-test-support  transports simulés, fixtures — partagé (§10)
app/                 app Flutter : lib/core (api, design tokens, di)
                     + lib/features/<feature>/ (clean architecture)
docs/architecture/   ADR (decisions/) + architecture.md
docs/                documents de référence, roadmaps par équipe
                     (pipeline / navigateur), interface-equipes.md
                     (journal des coutures)
fuzz/                cibles cargo-fuzz (hors workspace)
scripts/             verify_all.ps1 + utilitaires
.github/CODEOWNERS   périmètres des deux équipes (§10)
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
   visible, dans le même commit. Chaque entrée porte le numéro
   d'étape de sa piste (`Étape NN` navigateur, `R-N` pipeline,
   `JS-CNN` piste JS) — le fichier est union-mergé
   (`.gitattributes`).
5. **La roadmap de l'équipe cochée dans le même commit** que
   l'étape terminée (`docs/roadmap-equipe-pipeline.md` ou
   `docs/roadmap-equipe-navigateur.md` — §10). `ROADMAP.md` reste
   l'index maître, tenu par le lead aux jalons. Un commit de code
   sans sa doc est incomplet.
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

1. **Choisir une étape** dans la roadmap de son équipe
   (`docs/roadmap-equipe-pipeline.md` ou
   `docs/roadmap-equipe-navigateur.md`), dans l'ordre des paliers.
2. **Implémenter** dans le crate cible (API d'abord, docstring avant
   le code non trivial).
3. **Tester** : tests unitaires + tests d'intégration in-process ;
   fixtures dans `crates/<crate>/tests/data/`. Golden tests de rendu
   via framebuffer headless (PNG attendu vs produit). Corpus de
   conformité publics (html5lib-tests, WPT, test262) extraits par
   script vers `tests/conformance/`, jamais édités à la main —
   skip list versionnée, % tracké dans CHANGELOG (ADR-0010).
4. **Documenter** : case cochée dans la roadmap de l'équipe,
   `CHANGELOG.md`, ADR si décision d'architecture — même commit.
5. **Commit** : `feat(<crate>): <sujet>` / `fix(<crate>): …` /
   `docs: …` / `chore: …` — une étape = un commit dédié.
6. **Intégrer** : `verify_all` vert → `git pull --rebase` → push
   sur `main` (trunk-based, §10). Une étape multi-jours peut
   vivre en branche courte `pipeline/R-N` ou `nav/etape-NN`.

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
  Et **jamais un % sans son dénominateur** : un score se rapporte
  « x/y exécutés (z skips) sur sous-ensemble S » — test262 extrait
  ≠ test262, un dossier WPT ≠ WPT. Le cliquet verrouille une
  régression, il ne mesure pas une couverture globale.

## 9. Ce qui n'est PAS ce projet

- Pas un port de Dillo — `dillo-js` a une histoire dans un chantier
  de port voisin mais aucune architecture n'en est importée :
  l'adaptation passe par le trait `JsEngine` (ADR-0007). Provenance
  figée dans ADR-0020 ; renommage en `explorer-jscore` décidé,
  séquencé avant release 0.2.
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

## 10. Organisation deux équipes (depuis 2026-10-19)

Le projet est piloté par **deux équipes à taille comparable**,
découpées sur l'axe de la clean architecture (§6) — le domaine
pur d'un côté, la plateforme produit de l'autre.

> « Équipe » = une **piste de travail parallèle** (session/agent
> dédié) travaillant dans le même arbre — pas un groupe de
> personnes. Les règles ci-dessous gouvernent le *code* : qui
> touche quoi, dans quel ordre, sous quels contrats — c'est du
> contrôle de concurrence, pas de l'organisation humaine. Pas de
> réunion : le journal `docs/interface-equipes.md` et le
> `CHANGELOG` **sont** la synchronisation, asynchrone ; la revue
> se fait au rythme de l'opérateur.

- **Équipe PIPELINE** — le domaine pur (zéro I/O, zéro async) :
  `explorer-common`, `explorer-dom`, `explorer-html`,
  `explorer-css`, `explorer-layout`, `explorer-render`.
  Des octets aux pixels : parse, cascade, boîtes, display list,
  raster. Roadmap : `docs/roadmap-equipe-pipeline.md` — étapes
  **R-1+**.
- **Équipe NAVIGATEUR** — la plateforme produit (tout l'I/O et
  l'orchestration) : `explorer-net`, `explorer-core`,
  `explorer-api`, `explorer-daemon`, `explorer-cli`,
  `explorer-ffi`, `app/`, `scripts/`, `fuzz/`, **+ toute la
  surface JS** (`explorer-js`, `dillo-js`). Roadmap :
  `docs/roadmap-equipe-navigateur.md` — piste produit
  **étapes 50+**, piste JS **JS-C26+**.
- **`explorer-test-support` = partagé** : feuille à zéro
  dépendance consommée en `dev-dependencies` par les deux
  équipes (fixtures Ahem/TTF côté PIPELINE, transports simulés
  côté NAVIGATEUR). Ajouts de fixtures/stubs libres et
  **additifs** ; changement structurel via le journal.
- **Spécialiste JS** : rôle nommé au sein de NAVIGATEUR (pas une
  équipe séparée) — il tient la piste JS-Cxx. La profondeur de
  `dillo-js` (32 000 lignes) exige un détenteur du contexte ;
  sa file est ordonnée par les besoins produit, la couverture
  langue est strictement réactive aux sites réels.

Règles de la coexistence :

1. **Chaque équipe ne modifie que son périmètre** — un besoin
   dans la zone de l'autre passe par une entrée dans
   `docs/interface-equipes.md`, pas par un commit direct.
   Dépendance à sens unique : NAVIGATEUR consomme les API
   publiques de PIPELINE, jamais l'inverse.
2. **Contrats** : les API publiques des crates PIPELINE sont le
   contrat inter-équipes — extensions additives de préférence,
   toute casse de signature loguée dans
   `docs/interface-equipes.md` avant code. `JsEngine` est un
   contrat **interne** à NAVIGATEUR (plus de sign-off croisé).
3. **Trunk-based sur `main`** : pas de branches longues par
   équipe. Chacun pousse des commits d'étape verts après
   `pull --rebase`. Une étape multi-jours peut vivre en
   branche courte `pipeline/R-N` ou `nav/etape-NN`, fusionnée
   au vert. En interne d'équipe, liberté totale (branches
   partagées, worktrees) — les règles ne gouvernent que ce
   qui atterrit sur `main`.
   - **Gate entier avant push** : `verify_all` complet, pas
     seulement son crate — le couplage est asymétrique :
     PIPELINE (amont) peut casser des tests `explorer-core`
     en aval par un changement de sortie légitime ; l'inverse
     à la *compilation* est impossible (l'aval ne compile pas
     l'amont) — **sauf via `explorer-test-support` partagé**,
     où un changement NAV peut casser les tests PIPE : même
     règle, l'équipe causale répare ou revert. La suite
     downstream fait partie du contrat.
   - **Carve-out tests** : quand un changement légitime
     modifie une sortie consommée en aval (ou un stub/fixture
     partagé), l'équipe causale met à jour les tests
     downstream **dans le même commit** — exception
     documentée à « on ne touche que son périmètre », sinon
     tout se sérialise.
   - **Protocole main rouge** : celui qui casse `main`
     répare dans la journée **ou revert** — jamais de main
     rouge qui traîne.
   - **`Cargo.lock` en conflit** : jamais de fusion à la
     main — `cargo check`/`cargo update` régénère un
     lockfile cohérent.
4. **Fichiers partagés** : `CHANGELOG.md` est union-mergé ;
   `Cargo.lock` se régénère (règle §10.3) ;
   `[workspace.dependencies]` et `AGENTS.md`/ADR relèvent du
   lead ; `ROADMAP.md` = index maître (jalons, historique), le
   détail vivant est dans les roadmaps d'équipe.
5. **Numéros d'étapes immuables et uniques** — référence stable
   dans commits, bugs et conversations ; jamais réutilisés.
   Préfixes : `Étape NN` (navigateur), `R-N` (pipeline),
   `JS-CNN` (piste JS de navigateur).
6. **Triage transverse** : un bug à cheval se corrige du côté
   fautif (visuel/layout → PIPELINE ; session/réseau/JS/UI →
   NAVIGATEUR ; parse miroir fautif → piste JS).
   Le smoke sites réels (`scripts/site_smoke.ps1`) produit la
   liste, triée aux revues de l'opérateur.
