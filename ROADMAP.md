# Roadmap — OnionBit-Explorer

Source de vérité de l'avancement. Une étape est cochée quand :
implémentée + testée + documentée (CHANGELOG, ADR si décision) +
commitée (AGENTS.md §5).

Légende : `[ ]` à faire · `[~]` en cours · `[x]` terminée.

Contrainte structurante : **moteur d'abord**. L'app Flutter démarre
en squelette (phase 0) et n'est alimentée que via l'API — chaque
phase backend débloque les évolutions UI correspondantes.

## Phase 0 — Cadrage du dépôt

- [x] **Étape 0. Fondations documentaires.** `AGENTS.md`, `ROADMAP.md`,
  `CHANGELOG.md`, licences MIT/Apache-2.0, ADR initiaux
  (`docs/architecture/decisions/`), `docs/architecture.md`.
- [ ] **Étape 1. Workspace cargo.** 13 crates `explorer-*`
  squelettes (compilent, `clippy -D warnings`/`fmt --check`/`test`
  passent), `scripts/verify_all.ps1`, CI locale documentée.
- [ ] **Étape 2. Squelette Flutter.** `flutter create` desktop dans
  `app/` (Windows d'abord), clean architecture `core`/`features`,
  client API minimal, fichier `assets/design/tokens.json` chargé.

## Phase 1 — Réseau (`explorer-net`)

Objectif : télécharger une page `http(s)` réelle, décodée, bornée.

- [ ] **Étape 3. URL.** Résolution relative/absolue, normalisation,
  origine (schéma/hôte/port) pour la future politique same-origin.
- [ ] **Étape 4. HTTP/1.1 client.** Connexions tokio, keep-alive,
  requêtes pipelinables non — séquentielles d'abord ; parsing
  statut/en-têtes via `httparse`, corps `Content-Length` +
  `chunked`. Timeouts bornés.
- [ ] **Étape 5. TLS.** `rustls` + `webpki-roots` ; erreurs
  certificat remontées typées (l'UI affichera une page d'erreur,
  pas un crash).
- [ ] **Étape 6. Décodage.** `gzip`/`deflate`/`br` (crates pur
  Rust), content-length vs chunked résolus, charset déclaré exposé.
- [ ] **Étape 7. Cookies + cache.** Jar mémoire (domaine/path/
  secure), cache mémoire borné (ETag/Last-Modified) — disque
  optionnel en phase ultérieure.
- [ ] **Étape 8. Redirections & politique.** Suivi 3xx borné,
  `http_proxy`/`no_proxy` honorés (couture suite OnionBit — le
  navigateur ne sait pas ce qu'est Tor, il obéit au proxy).

## Phase 2 — DOM et HTML (`explorer-dom`, `explorer-html`)

- [ ] **Étape 9. DOM complet.** Arène + `NodeId` générationnels
  (pattern `slotmap` index+génération pour compatibilité GC JS et
  détection sans risque d'éléments détachés), attributs, navigation
  parent/enfant/siblings, `Document`/`Doctype`/`Comment`, texte normalisé.
- [ ] **Étape 10. Tokenizer HTML5.** États de la spec WHATWG,
  entités nommées/numériques, DOCTYPE/quirks mode.
- [ ] **Étape 11. Tree-builder.** Insertion modes, fermetures
  implicites, `<table>`/foster parenting, éléments `html/head/body`
  synthétisés — dégradation tag-soup tolérante.
- [ ] **Étape 12. Golden tests.** Corpus de pages de test
  (`tests/data/`), arbres attendus vs produits ; fuzzing du
  tokenizer (`fuzz/`).

## Phase 3 — CSS (`explorer-css`)

- [ ] **Étape 13. Tokenizer/parser.** Règles, déclarations,
  at-rules (`@media` minimal), tolérance aux erreurs de la spec.
- [ ] **Étape 14. Sélecteurs.** Tag/classe/id/descendant/enfant/
  attribut — matching sur le DOM.
- [ ] **Étape 15. Cascade + héritage.** Spécificité, `!important`,
  valeurs initiales/héritées, `computed style` par nœud.
- [ ] **Étape 16. Propriétés essentielles.** `display`, `margin`,
  `padding`, `border`, `width/height`, `color`, `background`,
  `font-*`, `text-align` — liste tenue dans l'ADR de phase.

## Phase 4 — Layout (`explorer-layout`)

- [ ] **Étape 17. Arbre de boîtes.** Block/inline/anonymous,
  lien nœud DOM ↔ boîte, dump wireframe debug dans `explorer-cli`
  (feedback visuel immédiat des rectangles avant la phase 5).
- [ ] **Étape 18. Fontes.** Stack de fontes pur Rust (mesure
  réelle des glyphes, line-height, métriques) — remplace toute
  mesure fixe approximative.
- [ ] **Étape 19. Block layout.** Flux normal, marges collapsées,
  dimensions auto/spécifiées.
- [ ] **Étape 20. Inline layout.** Line boxes, word wrap,
  `text-align`, inline-block minimal.
- [ ] **Étape 21. Tables + flottants.** Dimensionnement de tables,
  `float`, `clear` — algorithmes bornés.

## Phase 5 — Rendu (`explorer-render`)

- [ ] **Étape 22. Display list.** L'arbre de layout produit une
  liste de primitives (rects, texte, images placeholders).
- [ ] **Étape 23. Rasterisation.** `tiny-skia` (ADR-0005) :
  couleurs, bordures, texte ; framebuffer RGBA.
- [ ] **Étape 24. Frame PNG servie.** `GET /api/tabs/{id}/frame`
  (ADR-0004) + événement `frame_invalidated` SSE — l'UI affiche
  sa première vraie page en pull PNG (MVP).

## Phase 6 — API et daemon (`explorer-api`, `explorer-daemon`)

- [ ] **Étape 25. Contrat REST.** `/status`, `/tabs` CRUD,
  `navigate`, `reload`, `back`/`forward`, `/events` SSE — erreurs
  `{"error":{handled,message}}`.
- [ ] **Étape 26. Entrées.** `POST /api/tabs/{id}/input` :
  clics/souris/clavier/scroll traduits en hit-test sur le layout.
- [ ] **Étape 27. Daemon.** `explorer-daemon` : bind loopback,
  clé d'API optionnelle, arrêt propre, logs `tracing`.
- [ ] **Étape 28. Historique + bookmarks.** Persistance fichier
  (répertoire de profil déplaçable — couture suite).

## Phase 7 — Interface Flutter (`app/`)

L'UI évolue **au fil des phases backend** ; cette phase consolide.

- [ ] **Étape 29. Chrome navigateur.** Omnibox, onglets,
  boutons back/forward/reload, états de chargement via SSE.
- [ ] **Étape 30. Viewport & transport direct.** Affichage frame PNG
  (MVP) puis canal direct mémoire partagée / buffer brut (ADR-0004),
  scroll, invalidation temps réel avec dirty-rects, redimensionnement →
  viewport moteur.
- [ ] **Étape 31. Interactions.** Hit-test via API : clic lien →
  navigation, hover → curseur, saisie champs.
- [ ] **Étape 32. Design system.** Thème complet depuis
  `tokens.json`, dark/light, polish visuel.
- [ ] **Étape 33. Pages d'erreur.** Échec réseau/TLS rendu en
  page stylée côté UI (pas de frame moteur).

## Phase 8 — JavaScript (`explorer-js`)

- [ ] **Étape 34. Intégration dillo-js.** Adapter le moteur du
  projet DilloRust (Laurent Geynet, double licence MIT/Apache-2.0,
  ADR-0007) derrière `JsEngine` : `eval`, contexte par onglet, budget
  d'exécution. Subset ES3 au branchement — sa couverture grandira en
  phase 10.
- [ ] **Étape 35. Bindings DOM.** `document`, lecture/écriture
  d'attributs, `getElementById` & co avec handles sécurisés
  (générationnels) — invalidation layout.
- [ ] **Étape 36. Boucle d'événements.** `onclick`, timers
  bornés, `setTimeout` minimal — jamais de blocage du pipeline.

## Phase 9 — Consolidation

- [ ] **Étape 37. Images.** Décodeurs pur Rust (png/jpeg/gif/webp)
  derrière l'abstraction d'image, décodage progressif.
- [ ] **Étape 38. Formes.** `<form>`, GET/POST, champs natifs
  rendus moteur ou délégués UI (ADR de phase).
- [ ] **Étape 39. Perf + fuzz.** Benchs pipeline, bornes
  mémoire/temps, corpus fuzz tokenizer/parser.
- [ ] **Étape 40. Release 0.1.** Packaging Windows
  (daemon + app), smoke test bout en bout, documentation.

## Phase 10 — Compatibilité web moderne (cap long terme)

Objectif explicite : **naviguer le web réel — tous les sites**.
Pour garantir des résultats mesurables sans épuisement, la compatibilité
progresse en trois paliers structurants :

- **Palier 1 — Web documentaire (statique)** : Wikipedia (mode HTML),
  spécifications W3C/RFC, blogs, sites de documentation. Layout flux normal
  robuste, fontes précises, liens fonctionnels, zéro crash.
- **Palier 2 — Web 2.0 & interactif** : Formulaires complets, CSS2.1
  (tables, flottants), sessions/cookies, forums, JS basique (ES3/ES5).
- **Palier 3 — Applications modernes (SPA)** : Flexbox, Grid, ES6+, Web APIs,
  sites riches temps réel.

Étapes de la phase 10 :

- [ ] **Étape 41. HTTP/2 puis HTTP/3.** Multiplexage h2 sur
  `tokio`+`rustls` ; QUIC quand l'écosystème pur Rust le permet.
- [ ] **Étape 42. Layout moderne.** `flexbox`, `grid`,
  `position: sticky`, transforms, `calc()`, custom properties
  (`--var`).
- [ ] **Étape 43. JS vers ES6+.** Étendre `dillo-js` (classes,
  `let/const`, promesses, modules) — ou compléter/remplacer
  l'implémentation derrière `JsEngine` si le moteur ne suit pas.
- [ ] **Étape 44. Web APIs.** `fetch`/`XHR`, `localStorage`/
  `sessionStorage`, History API, `MutationObserver`, workers
  (bornés).
- [ ] **Étape 45. Médias et graphiques.** `<canvas>` 2D,
  `<video>`/`<audio>` (décodage à arbitrer), `requestAnimationFrame`,
  animations/transitions CSS.
- [ ] **Étape 46. Webfonts + charsets.** WOFF/WOFF2, `font-display`,
  encodages legacy (windows-1252, ISO-8859-*).
- [ ] **Étape 47. Streaming UI haute performance.** Texture externe
  Flutter via FFI (upgrade d'ADR-0004) pour les framerates élevés et
  médias animés.
- [ ] **Étape 48. Interop continue.** Corpus de compat sur sites
  réels ; les écarts constatés deviennent des étapes.

