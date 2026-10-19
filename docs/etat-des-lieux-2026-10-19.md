# État des lieux — Explorer

**Date :** 2026-10-19 · **Auteur :** chef de projet entrant
**Statut :** recommandations adoptées et appliquées (§9) —
arbre de travail non commité, en attente de validation finale.

> Ce document est un constat objectif, mesuré sur le dépôt tel qu'il
> est au 19/10/2026 (`main`, 80 commits, arbre propre). Toutes les
> données chiffrées ont été vérifiées par exécution réelle, pas
> recopiées depuis la documentation.

---

## 1. Synthèse exécutive

- Le projet a livré sa **release 0.1.0 le jour même de ce rapport** :
  phases 0 à 9 intégralement terminées (étapes 0–40), phase 10
  (compatibilité web moderne) terminée à **8 étapes sur 9**.
- La suite de tests workspace est **intégralement verte** :
  **748 tests passent, 0 échec** (vérifié par `cargo test
  --workspace` le jour du rapport).
- La conformité est mesurée et cliquetée : html5lib ≥ 953 cas,
  test262 859/859 sur le sous-ensemble extrait, WPT reftests 27/40,
  Acid2 29,8 %, 5/5 sites réels rendus.
- La séparation de travail optimale **n'est pas** celle des
  silos technologiques (Rust vs JS) : `dillo-js` est en avance
  de phase sur le navigateur — une équipe JS dédiée serait
  sous-employée. Le découpage retenu suit la clean architecture :
  **domaine pur vs plateforme produit**, la surface JS entière
  rangée côté produit (§6).
- Le seul point de couplage réel entre les deux équipes devient
  les API publiques des crates domaine + les fichiers
  documentaires partagés (`CHANGELOG.md`, `ROADMAP.md`,
  `Cargo.toml` workspace). La couture `JsEngine` passe **en
  interne** d'une seule équipe — le point de coordination le
  plus fin disparaît.
- **Anomalie de processus relevée** : ADR-0011 exigeait une
  réévaluation formelle du modèle de menace « avant de brancher
  `dillo-js` » en phase 8. Le travail technique a été fait (thread
  dédié, budget d'ops) mais **aucun ADR de réévaluation n'a été
  écrit** — dette documentaire à solder.

---

## 2. Faits mesurés

| Indicateur | Valeur mesurée |
|---|---|
| Version | 0.1.0 (release packagée le 2026-10-19) |
| Crates workspace | 15 (14 `explorer-*` + `dillo-js`) |
| Lignes Rust `src/` | ≈ 55 900 (dont `dillo-js` ≈ 32 300, soit 58 %) |
| Tests `cargo test --workspace` | **748 verts, 0 échec** |
| App Flutter | 12 fichiers Dart, 1 feature (`browser`), 3 tests |
| Historique git | 80 commits, 1 auteur, 1 branche (`main`) |
| html5lib tree-construction (corpus **complet**) | **953/1592 exécutés (59 %)**, 200 ignorés (fragment/script-on), 639 échecs |
| test262 (sous-ensemble extrait, 1077 fichiers) | **859/859 testés (100 %)**, 218 skips whitelistés |
| WPT reftests (`css/CSS2/floats`+`normal-flow`) | **27/40 (67 %)**, cliquet 27 |
| Acid2 | 29,8 % (informatif, robustesse garantie) |
| Sites réels smoke | **5/5** (example.com, info.cern.ch, duckduckgo, wikipedia, MDN) |
| Bench pipeline (criterion) | navigate ≈ 63 ms, scroll repaint ≈ 19 ms |

**Limites des mesures** (à lire avec le tableau) : les scores de
corpus portent sur des **sous-ensembles extraits**, pas sur les
suites complètes — html5lib seul est complet (953/1592 = 59 %,
639 échecs réels) ; test262 : 1 077 fichiers lexicaux sur ~50 000
(littéraux, commentaires, identifiants, whitespace ; aucun
`builtins/` ni `statements/` — l'extension est JS-C30) ; WPT :
40 reftests de deux dossiers (`floats`, `normal-flow`). Le
cliquet verrouille une régression, il n'exprime pas une
couverture globale ; les chiffres ci-dessus ne peuvent pas être
lus comme des % de conformité aux suites entières. Le signal le
moins gonflable est le smoke sites réels.

## 3. État par module

| Module | État | Forces | Lacunes / restes |
|---|---|---|---|
| `explorer-net` | ✅ mature | URL WHATWG+IDNA, h1, h2 (ALPN), TLS, décodage, cookies PSL, HSTS, proxy env | pas de h3/QUIC (différé assumé), cache mémoire seul (pas de disque) |
| `explorer-dom` | ✅ mature | arène slotmap, rooting ADR-0008, budgets | stable |
| `explorer-html` | ✅ bon, avec dette de conformité | tokenizer incrémental `push()`, tree-builder modes complets, html5lib 953/1592 | pas de foreign-content (SVG/MathML inline) ; 639 cas tree-construction en échec (détail : `cargo test -p explorer-html -- --nocapture`) |
| `explorer-css` | ✅ avancé | cascade complète, `var()`, `calc()`, `@font-face`, transitions | sous-ensemble de propriétés ; `@keyframes` absent |
| `explorer-layout` | ✅ bon, en progression | block/inline/tables/floats/BFC, flex 1 ligne, grid MVP, sticky top, transforms | `flex-wrap`, `margin:auto`, areas, min/max-content, sticky autres côtés |
| `explorer-render` | ✅ bon | display list, tiny-skia, PNG, SVG statique, transforms, frontière défensive | pas de canvas, pas de dirty-rects |
| `explorer-core` | ⚠️ fonctionnel mais **synchrone et non incrémental** | pipeline complet, scheduler séquentiel de sous-ressources, session+historique+formulaires | **fetch complet → parse → puis** sous-ressources : pas de peinture pendant le téléchargement ; `<script src>` non fetché ; timers JS sans tick dédié ; événements `click` seuls |
| `explorer-js` | ✅ couture stable | trait `JsEngine` + proxy thread + miroir DOM (ADR-0013) | XHR/storage non câblés ; surface à étendre (`drain_net`, MutationObserver…) |
| `dillo-js` | ✅ **très avancé** | ES2026 large, GC, modules, Proxy, Temporal subset, DOM miroir, XHR/fetch/storage prêts côté moteur | MutationObserver absent, workers absents, `await`/générateurs eager (déviation), Intl mono-locale, canvas absent |
| `explorer-api`/`daemon` | ✅ stable | REST+SSE, erreurs typées, loopback | — |
| `explorer-ffi` | ✅ livré | mode in-process, RGBA via dart:ffi | isolation dégradée (documenté) |
| `explorer-cli` | ✅ outil dev | fetch/dump/shot/wireframe | — |
| `app/` Flutter | ⚠️ squelette fonctionnel | omnibox, onglets, frames FFI/REST, erreurs stylées, design tokens | thème clair absent, settings/history/bookmarks UI absents, pubspec encore « A new Flutter project » |
| `explorer-test-support` | ✅ | transports simulés, fixtures | — |
| `explorer-common` | ✅ | quasi vide (10 lignes) | — |
| `examples_dbg/` | 🗑 répertoire **vide** | — | à supprimer ou documenter |

## 4. Ce qui manque pour le web réel — inventaire priorisé

Classé par **impact sur les sites réels**, pas par ordre de code :

### Critique (bloque la majorité des sites modernes)

1. **`<script src>` externe** : seuls les scripts inline sont
   évalués. Aucun script externe n'est fetché ni ordonnancé
   (`defer`/`async`). C'est le trou le plus bloquant de la phase 10.
2. **Pipeline incrémental non exploité** : le tokenizer accepte
   `push(&str)` mais `Session::navigate_request` attend le corps
   complet avant de parser, puis fetch les sous-ressources
   **séquentiellement** (CSS → images → fontes). ADR-0012 posé,
   scheduler parallèle non réalisé.
3. **Web APIs côté navigateur** (étape 44 ouverte) : `fetch`/XHR,
   `localStorage`, `document.cookie` existent dans `dillo-js` mais
   ne sont **pas câblés** à `HttpTransport`/au profil. Sans eux,
   aucune app réelle ne fonctionne.
4. **Événements DOM** : seul `click` est dispatché. Pas de
   `mouseover`/`keydown`/`input`/`focus`/`blur`/`wheel`. Les
   timers différés ne tirent qu'aux points de synchro
   (input/navigation) — pas de tick dédié.
5. **MutationObserver** : absent de `dillo-js` — requis par une
   grande partie du web 2.0.

### Majeur (dégradation forte mais pages consultables)

6. Layout moderne : `flex-wrap`, `margin:auto`, areas nommées,
   `min/max-content` réels, `sticky` autres côtés que `top`.
7. `<svg>` inline (exige foreign-content tokenizer), `@keyframes`.
8. Reftests WPT restants : 13/40 (floats placement, BFC overflow,
   zero-height, block-formatting).
9. Formulaires : popup `<select>`, `multipart/form-data`, masque
   `password`, validation.

### Structurel / moyen terme

10. `canvas` 2D (couture dillo-js ↔ tiny-skia à définir).
11. Persistance profil : cookies/storage/cache sur disque.
12. Dirty-rects + transport frame optimisé (ExternalTexture GPU
    — arbitrage ADR-0004 à revisiter).
13. UI Flutter : thème clair, settings, historique/bookmarks UI.
14. `<video>`/`<audio>` (décodage à arbitrer), HTTP/3 (différé).
15. `iframe`/shadow DOM : hors périmètre assumé (à réexaminer un
    jour — beaucoup de sites réels en dépendent).

## 5. Dettes et déviations documentées (à ne pas « redécouvrir »)

- **Miroir DOM double-parse** : le miroir `dillo-js` re-parse le
  HTML source ; il peut diverger des corrections HTML5 du vrai
  tree-builder (ADR-0013 — assumé, point d'extension = diff).
- **Générateurs eager / `await` settled-only** : pas de suspension
  réelle dans `dillo-js` — déviation structurelle documentée.
- **Mono-thread** : `Atomics.wait`/`notify` approximés (conforme
  à part `waitAsync`), pas de SharedArrayBuffer inter-worker.
- **`Intl` en-US/UTC fixe**, pas de CLDR ; `crypto` = PRNG non
  cryptographique (déviation documentée — à corriger un jour).
- **WOFF2** : transform nulle seulement (glyf transformé refusé).
- **`reload` d'un POST refait GET** ; `document.title=` non
  setter en amont ; `password` non masqué.
- **Mode FFI = isolation processus dégradée** (ADR-0018, assumé).
- **Réévaluation ADR-0011 de phase 8 jamais écrite** (voir §1).

## 6. Répartition en 2 équipes — analyse

### 6.1 Pourquoi pas une équipe « JS » dédiée

ADR-0007 a conçu `dillo-js` comme chantier d'équipe dédiée, et
l'historique montre que les pistes `dillo-js` / `explorer-*` ne
se touchent quasiment jamais. Un découpage MOTEUR/JS serait le
choix le plus confortable — mais l'état des lieux le contredit :

- **`dillo-js` est en avance de phase** : couverture langue
  large (Promise, Proxy, Temporal partiel, modules — démontrée
  par ses tests internes ; le score test262 reste un sous-
  ensemble lexical extrait, cf. limites §2) — le moteur couvre
  un langage que le navigateur ne peut pas encore exploiter
  (pas de `<script src>`, pas d'événements complets, pas de
  XHR câblé).
- Son travail restant est réduit et **piloté par le navigateur** :
  MutationObserver, traverse proxy XHR/storage, canvas, workers —
  ~4 livrables d'intégration puis couverture réactive. Pas de quoi
  occuper une équipe entière sans gold-plating du test262.
- En revanche la **profondeur** du crate (32 000 lignes
  d'interpréteur) exige un détenteur de contexte — un **rôle**
  nommé, pas une équipe.

La bonne frontière est celle que la clean architecture (§6
d'AGENTS.md) a déjà tracée : **domaine pur vs plateforme** — elle
suit le sens des dépendances cargo, elle équilibre les charges,
et elle rend la couture JS interne à une équipe.

### 6.2 Découpage adopté

| | Équipe PIPELINE | Équipe NAVIGATEUR |
|---|---|---|
| Crates | `explorer-common`, `explorer-dom`, `explorer-html`, `explorer-css`, `explorer-layout`, `explorer-render` | `explorer-net`, `explorer-core`, `explorer-api`, `explorer-daemon`, `explorer-cli`, `explorer-ffi`, `explorer-js`, `dillo-js`, `app/`, `scripts/`, `fuzz/` — **`explorer-test-support` partagé** (feuille consommée en dev-deps par les deux : ajouts additifs libres) |
| Nature | domaine pur — zéro I/O, zéro async | plateforme produit — tout l'I/O, orchestration, JS, UI |
| Mission | conformité layout/CSS/rendu, corpus WPT, incrémental domaine | scheduler, session, Web APIs câblage, événements, profil, UI, intégration JS, release |
| Numérotation | R-1+ | étapes 50+ (piste produit) + JS-C26+ (piste JS) |

Effets structurels :

- **Le trait `JsEngine` devient interne** — `explorer-js`,
  `dillo-js` et `explorer-core` vivent dans la même équipe :
  fini le sign-off croisé sur la couture la plus fine du projet.
- **Dépendance à sens unique** : NAVIGATEUR consomme les API
  publiques de PIPELINE, jamais l'inverse — la règle de
  dépendance cargo IS le contrat de travail.
- **Charges équilibrées** : PIPELINE porte le goulot de la
  conformité (layout moderne, foreign content, WPT), NAVIGATEUR
  porte le goulot de l'intégration (scripts externes, XHR,
  événements, incrémental session) + la piste JS.
- **Spécialiste JS** : rôle au sein de NAVIGATEUR — il garde
  `dillo-js`, sa file est ordonnée par les étapes produit
  (C27→51, C26→52, C28→R-11 canvas) puis strictement réactive.

Règle de tri des bugs transverses : **on corrige du côté
fautif** — visuel/layout → PIPELINE ; session/réseau/JS/UI →
NAVIGATEUR ; parse miroir fautif → piste JS. Le smoke de sites
réels produit la liste, triée en sync hebdo.

### 6.3 Fichiers partagés — convention anti-conflit

| Fichier | Convention proposée |
|---|---|
| `ROADMAP.md` | devient **index maître** (jalons, phases, historique) — modifié par le lead aux jalons ; le détail vivant migre vers `docs/roadmap-equipe-*.md` |
| `docs/roadmap-equipe-pipeline.md` / `-navigateur.md` | un fichier **par équipe** — chacun ne touche que le sien → zéro conflit |
| `CHANGELOG.md` | `merge=union` dans `.gitattributes` + préfixe de piste dans les entrées (`**Étape 50**`, `**R-1**`, `**JS-C26**`) |
| `Cargo.toml` workspace | chaque équipe ne touche que la section de ses crates ; ajout de `[workspace.dependencies]` annoncé (règles §4.2 inchangées) |
| `Cargo.lock` | régénéré par cargo, jamais fusionné à la main |
| `AGENTS.md`, ADR | propriété du lead ; amendements via ADR |
| `.github/CODEOWNERS` (à créer) | formalise la table ci-dessus |

## 7. Stratégie Git — réponse aux questions

### Branche unique ou worktrees ?

**Worktree et branche ne sont pas la même décision.** Le worktree
est un mécanisme de checkout local ; la stratégie de branche est
le modèle d'intégration.

Recommandation : **trunk-based sur `main`** — les deux équipes
poussent sur `main`, avec pour chaque étape :

1. travail local (clone ou worktree propre à chacun) ;
2. `pwsh scripts/verify_all.ps1` vert (règle existante inchangée) ;
3. `git pull --rebase` puis push — les fichiers étant disjoints,
   les rebases sont quasi toujours triviaux ;
4. si une étape dépasse une journée de travail : branche courte
   `pipeline/R-N-…` ou `nav/etape-50-…`, fusionnée après gate
   vert —
   la convention existante « une étape = un commit » s'y prête
   naturellement (squash ou commit atomique).

**Déconseillé : deux branches longues par équipe.** Justification :

- les périmètres de fichiers sont disjoints → le gain d'isolation
  est faible, le coût de fusion (CHANGELOG/ROADMAP à chaque merge,
  gate workspace entier à rejouer) est élevé ;
- `cargo clippy --workspace` est un gate *entier* : plus une
  branche dérive, plus son merge est risqué. Le gate monolithique
  est un argument **pour** le trunk — la casse est détectée au
  push, par l'équipe causale, pas au merge des semaines plus tard ;
- un steward d'intégration deviendrait un goulot — le projet n'a
  pas la masse pour le justifier.

**Nuance technique — le couplage réel est comportemental, pas
compile-time.** Le risque sur `main` n'est pas « mon fichier vs
ton fichier » (périmètres disjoints) mais **la sortie observable
des API** : `cargo test --workspace` exécute les tests
`explorer-core` (NAVIGATEUR) contre le comportement de
`layout`/`render` (PIPELINE) — un fix légitime de whitespace
change la sortie → casse un test downstream. Asymétrie : l'aval
ne peut pas casser la compilation de l'amont. Garde-fous actés
(AGENTS.md §10.3) : gate entier pré-push, API additives,
mise à jour des tests downstream dans le même commit par
l'équipe causale (carve-out), protocole « main rouge » =
réparer ou revert dans la journée.

**Worktrees recommandés comme confort local** (Windows OK) :

- chaque dev garde un checkout de travail + optionnellement un
  checkout `main` propre pour les runs de gate :
  `git worktree add ..\explorer-main main` ;
- le lead peut ainsi vérifier/relire `main` pendant que les deux
  pistes avancent ;
- c'est aussi la réponse si une même machine héberge les deux
  équipes (environnements `target/` séparés → pas de contention).

### Comment se partagent-ils l'avancement ?

Le dépôt **est** le canal — les conventions existantes suffisent,
renforcées :

1. **Roadmaps par équipe** commitées dans git : `git pull` du
   matin = l'avancement de l'autre équipe.
2. **CHANGELOG unique** avec préfixe d'équipe — lisible en un
   coup d'œil, union-mergeable.
3. **`docs/interface-equipes.md`** (nouveau, petit) : journal des
   changements de couture — toute modif de `JsEngine`, de
   `[workspace.dependencies]` ou d'un ADR y est loguée en 3 lignes.
4. **Sync hebdo** (~30 min) : re-mesure des % de conformité,
   triage des échecs `site_smoke.ps1` en bugs pipeline vs
   navigateur, ré-allocation des étapes.
5. Les numéros d'étapes sont **uniques et immuables**
   (`Étape NN` navigateur 50+, `R-N` pipeline, `JS-CNN` piste
   JS) — référence stable dans les
   conversations, les commits et les bugs.

## 8. Risques identifiés

| Risque | Probabilité | Mitigation |
|---|---|---|
| Interface PIPELINE↔NAVIGATEUR plus épaisse que `JsEngine` (les API domaine sont nombreuses) | moyen | règle additive + journal des coutures ; les besoins sont déjà anticipés par ADR-0012 (ports `FontProvider`, incrémental) |
| Un chantier à cheval (canvas R-11↔C28, incrémental 53↔R-1) dérive | moyen | couture figée à deux *avant* de coder, marquée ↔ dans les roadmaps |
| Le spécialiste JS devient goulot dans NAVIGATEUR | faible | sa file est ordonnée par le produit ; la couverture langue est réactive, pas pro-active |
| Bus-factor `dillo-js` = 1 | assumé | seul détenteur du contexte des 32 000 lignes — mitigation : la piste JS documente chaque livrable dans `corpus.rs`/tests, le trait `JsEngine` borne la surface à connaître pour le reste de l'équipe |
| `Cargo.lock` cassé par une mauvaise fusion | faible | merge normal + convention « régénérer, jamais fusionner à la main » (§10.3) — l'union-merge a été écartée : stanzas `[[package]]` incohérents possibles |
| Dérive du miroir DOM vs vrai DOM | réel, déjà documenté | règle de tri « corrige le côté fautif » + tests de cohérence croisés |
| Conflits CHANGELOG à chaque commit | élevé si non traité | `merge=union` + préfixes de piste (§6.3) |
| Gate workspace cassé par l'autre équipe | faible | verify_all pré-push obligatoire ; branche courte pour le WIP risqué |
| `dillo-js` devient l'unique source de vérité DOM (dérive du miroir) | structurel | ADR-0013 assume ; si le coût de resynchro explose → point d'extension « diff d'arbre » déjà prévu |

## 9. Décisions — adoptées le 2026-10-19

Toutes les propositions de ce rapport ont été appliquées dans
l'arbre de travail (non commité à la rédaction) :

1. ✅ Découpage PIPELINE / NAVIGATEUR (§6.2) — la surface JS
   entière (`explorer-js` + `dillo-js`) rangée côté produit ;
   `JsEngine` = contrat interne ; rôle « spécialiste JS » au
   sein de NAVIGATEUR.
2. ✅ Workflow trunk-based sur `main` — `.gitattributes` :
   `CHANGELOG.md merge=union` ; `Cargo.lock` en merge normal,
   convention « régénéré, jamais fusionné à la main » ;
   `.github/CODEOWNERS` créé (roadmaps d'équipe assignées).
3. ✅ `ROADMAP.md` = index maître ; `docs/roadmap-equipe-*.md`
   vivants ; renvoi « travaux actifs » ajouté en fin de ROADMAP.
4. ✅ Dette ADR-0011 soldée → **ADR-0019** écrite
   (réévaluation modèle de menace JS in-process, prochains
   points événementiels).
5. ✅ `examples_dbg/` vide supprimé ; `pubspec.yaml` décrit.
6. ✅ Numérotation : navigateur 50+, pipeline R-1+, piste
   JS-C26+ — actée dans AGENTS.md §10.
7. ✅ `docs/interface-equipes.md` créé (journal des coutures,
   extensions en attente listées).

Les deux roadmaps détaillées : `docs/roadmap-equipe-pipeline.md`
et `docs/roadmap-equipe-navigateur.md`.
