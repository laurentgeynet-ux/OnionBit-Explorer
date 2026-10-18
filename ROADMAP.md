# Roadmap — Explorer

Source de vérité de l'avancement. Une étape est cochée quand :
implémentée + testée + documentée (CHANGELOG, ADR si décision) +
commitée (AGENTS.md §5).

Légende : `[ ]` à faire · `[~]` en cours · `[x]` terminée.

Contrainte structurante : **moteur d'abord**. L'app Flutter démarre
en squelette (phase 0) et n'est alimentée que via l'API — chaque
phase backend débloque les évolutions UI correspondantes.

## Jalons (repères de motivation, non bloquants)

| Jalon | Définition | Phase |
|---|---|---|
| **J0** | Dépôt cadré + workspace compilant | 0–1 |
| **J1** | `explorer-cli fetch <url>` télécharge une page réelle décodée | 1 |
| **J2** | `explorer-cli dump <url>` affiche le texte d'une page réelle — premier artefact visible | 2 |
| **J3** | Première page stylée servie en PNG (`GET /frame`) | 5 |
| **J4** | Navigation réelle dans l'app Flutter (liens, scroll, back) | 7 |
| **J5** | Release 0.1 packagée | 9 |

J2 compte double : il rend visible le pipeline bien avant le moindre
pixel, et c'est le premier vrai test bout-en-bout net+dom+html.

## Phase 0 — Cadrage du dépôt

- [x] **Étape 0. Fondations documentaires.** `AGENTS.md`, `ROADMAP.md`,
  `CHANGELOG.md`, licences MIT/Apache-2.0, ADR initiaux
  (`docs/architecture/decisions/`), `docs/architecture.md`.
- [x] **Étape 1. Workspace cargo.** 13 crates `explorer-*`
  squelettes (compilent, `clippy -D warnings`/`fmt --check`/`test`
  passent), `scripts/verify_all.ps1`, CI locale documentée.
  *Bonus : `crates/dillo-js` migre dans le workspace — version
  canonique, licence MIT OR Apache-2.0, équipe JS dédiée.*
- [x] **Étape 2. Squelette Flutter.** `flutter create` desktop dans
  `app/` (Windows d'abord), clean architecture `core`/`features`,
  client API minimal, fichier `assets/design/tokens.json` chargé.

## Phase 1 — Réseau (`explorer-net`)

Objectif : télécharger une page `http(s)` réelle, décodée, bornée.

- [x] **Étape 3. URL.** Résolution relative/absolue, normalisation
  WHATWG, **IDNA/punycode** (crate pur Rust à arbitrer), origine
  (schéma/hôte/port) pour la future politique same-origin, schémas
  `data:`/`about:` minimaux.
- [ ] **Étape 4. HTTP/1.1 client.** Connexions tokio, keep-alive,
  requêtes pipelinables non — séquentielles d'abord ; parsing
  statut/en-têtes via `httparse`, corps `Content-Length` +
  `chunked`. Timeouts bornés.
- [ ] **Étape 5. TLS.** `rustls` + `webpki-roots` ; erreurs
  certificat remontées typées (l'UI affichera une page d'erreur,
  pas un crash).
- [ ] **Étape 6. Décodage.** `gzip`/`deflate`/`br` (crates pur
  Rust), content-length vs chunked résolus, charset déclaré exposé
  + **transcodage** (UTF-8, windows-1252 — `encoding_rs` ou tables
  maison, à arbitrer), `X-Content-Type-Options` honoré.
- [ ] **Étape 7. Cookies + cache.** Jar mémoire avec politique
  réelle dès le départ : `Secure`/`HttpOnly`/`SameSite`,
  préfixes `__Host-`/`__Secure-`, domaines validés par **Public
  Suffix List** (crate pur Rust type `psl` — sans elle, un jar
  naïf accepte les supercookies `.co.uk`) ; cache mémoire borné
  (ETag/Last-Modified) — disque optionnel en phase ultérieure.
- [ ] **Étape 8. Redirections & politique.** Suivi 3xx borné, store
  **HSTS** mémoire borné, `http_proxy`/`no_proxy` honorés
  (fonctionnalité standard — la couture reste ouverte à tout
  transport mandataire futur : proxy d'entreprise, Tor…) ;
  en-têtes `Referer`/`Origin` calculés par requête et politique
  **mixed-content** (sous-ressource `http` d'une page `https`
  bloquée ou dégradée) câblés dès le scheduler ADR-0012.

## Phase 2 — DOM et HTML (`explorer-dom`, `explorer-html`)

- [ ] **Étape 9. DOM complet.** Arène + `NodeId` générationnels
  (pattern `slotmap` index+génération — modèle mémoire figé par
  **ADR-0008** : rooting explicite, cycles DOM↔JS par table de
  callbacks, invalidation de génération documentée sur chaque
  mutation), attributs, navigation parent/enfant/siblings,
  `Document`/`Doctype`/`Comment`, texte normalisé.
- [ ] **Étape 10. Tokenizer HTML5.** États de la spec WHATWG,
  entités nommées/numériques, DOCTYPE/quirks mode — entrée
  **incrémentale** (chunks bornés, ADR-0012) pré-décodée avec
  **sniffing d'encodage WHATWG** (BOM, `<meta>` dans les premiers
  octets, défaut `windows-1252`).
- [ ] **Étape 11. Tree-builder.** Insertion modes, fermetures
  implicites, `<table>`/foster parenting, éléments `html/head/body`
  synthétisés — dégradation tag-soup tolérante.
- [ ] **Étape 12. Golden tests + premier artefact.** Harness
  **html5lib-tests** (tree-construction, ADR-0010) + corpus
  `tests/data/`, arbres attendus vs produits, % de conformité
  publié dans CHANGELOG ; fuzzing du tokenizer (`fuzz/`) ;
  `explorer-cli dump` (texte normalisé du DOM d'une page réelle —
  **jalon J2**) + spike ADR-0008 avec faux handles JS validant le
  protocole rooting/invalidation avant l'arrivée du vrai moteur.

## Phase 3 — CSS (`explorer-css`)

- [ ] **Étape 13. Tokenizer/parser.** Règles, déclarations,
  at-rules (`@media` minimal), tolérance aux erreurs de la spec.
- [ ] **Étape 14. Sélecteurs.** Tag/classe/id/descendant/enfant/
  attribut — matching sur le DOM.
- [ ] **Étape 15. Cascade + héritage.** Spécificité, `!important`,
  valeurs initiales/héritées, `computed style` par nœud.
- [ ] **Étape 16. Propriétés essentielles.** `display` (dont
  `list-item`, `none`), `margin`, `padding`, `border`,
  `width/height`, `color`, `background`, `font-*`, `text-align`,
  `list-style`, `white-space`, `position`, `z-index`, `overflow`,
  pseudo-classes dynamiques (`:hover`, `:focus`…) → restyle via
  l'invalidation ADR-0012 — liste tenue dans l'ADR de phase ;
  sous-ensembles **WPT** selectors/cascade mesurés (ADR-0010).

## Phase 4 — Layout (`explorer-layout`)

- [ ] **Étape 17. Arbre de boîtes.** Block/inline/anonymous,
  lien nœud DOM ↔ boîte, dump wireframe debug dans `explorer-cli`
  (feedback visuel immédiat des rectangles avant la phase 5).
- [ ] **Étape 18. Stack texte et fontes** (ADR-0009).
  `fontdb`/`ttf-parser` (chargement, métriques), `rustybuzz`
  (shaping), `unicode-bidi`, `unicode-linebreak` ; runs de texte
  itemisés ; fonte fixture embarquée pour les tests headless —
  remplace toute mesure fixe approximative. `fontdb` fait de l'I/O
  (registre/filesystem) : `explorer-layout` n'en dépend pas — le
  port `FontProvider`/`FontSource` est défini côté layout,
  l'implémentation `fontdb` vit en couche infra injectée par
  `explorer-core` (ADR-0009).
- [ ] **Étape 19. Block layout.** Flux normal, marges collapsées,
  dimensions auto/spécifiées.
- [ ] **Étape 20. Inline layout.** Line boxes sur runs itemisés,
  word wrap (opportunités UAX #14), `text-align`, inline-block
  minimal.
- [ ] **Étape 21. Tables, flottants et positionnement.**
  Dimensionnement de tables, `float`, `clear` ; `position`
  (abs/rel/fixed), `z-index` et contextes d'empilement,
  `overflow` (clipping, scroll interne) — omniprésents même sur
  les sites documentaires du palier 1 — algorithmes bornés.

## Phase 5 — Rendu (`explorer-render`)

- [ ] **Étape 22. Display list.** L'arbre de layout produit une
  liste de primitives (rects, texte, images placeholders).
- [ ] **Étape 23. Rasterisation.** `tiny-skia` (ADR-0001) :
  couleurs, bordures, texte ; framebuffer RGBA, facteur d'échelle
  DPI (HiDPI) appliqué à la rasterisation.
- [ ] **Étape 24. Frame PNG servie.** `GET /api/tabs/{id}/frame`
  (ADR-0004) + événement `frame_invalidated` SSE — l'UI affiche
  sa première vraie page en pull PNG (MVP).

## Phase 6 — API et daemon (`explorer-api`, `explorer-daemon`)

- [ ] **Étape 25. Contrat REST.** `/status`, `/tabs` CRUD,
  `navigate`, `reload`, `back`/`forward`, `/events` SSE — erreurs
  `{"error":{handled,message}}`.
- [ ] **Étape 26. Entrées.** `POST /api/tabs/{id}/input` :
  clics/souris/clavier/scroll traduits en hit-test sur le layout ;
  **coalescing** des événements mouvement pour limiter les
  round-trips (ADR-0003).
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

### Chantier `dillo-js` — piste parallèle de l'équipe JS dédiée

Le moteur (`crates/dillo-js`) avance sur sa propre cadence, en
parallèle des phases moteur — son ordre est dicté par le contrat
d'intégration, pas par la couverture :

- [ ] **JS-A. Alignement heap → ADR-0008** : rooting explicite,
  trace des handles, finalisation sur l'arène `ObjId` ; passe GC
  mark-sweep réelle (l'arène qui « meurt avec la page » ne tient
  pas pour une SPA longue durée — palier 3). **Prérequis de
  l'étape 34** — chaque feature ajoutée avant ce retrofit le rend
  plus cher.
- [ ] **JS-B. Harness test262** : extraction par script vers
  `tests/conformance/`, skip list versionnée, % publié dans
  CHANGELOG (ADR-0010) — la couverture devient un signal objectif.
- [ ] **JS-C. Couverture par besoin** : modules ES
  (`import`/`export` + graphe de chargeur), `Proxy`, `Intl`,
  regex lookahead/backrefs — ordre piloté par les pages réelles.
- [ ] **JS-D. Médias** : `canvas` 2D (pont tiny-skia), `Audio`/
  `Video` — phase 10, dépend du pipeline rendu.

Étapes navigateur (intégration) :

- [ ] **Étape 34. Intégration `dillo-js` + alignement GC.** Le
  moteur vit dans ce workspace (`crates/dillo-js`, équipe JS
  dédiée, MIT OR Apache-2.0 — ADR-0007). Deux prérequis avant
  branchement : (a) **aligner le heap `ObjId` sur ADR-0008** —
  rooting explicite, trace, finalisation des handles (le spike de
  l'étape 12 a validé le protocole avec de faux handles) ;
  (b) réévaluation du modèle de menace (**ADR-0011**). Ensuite :
  adaptation derrière `JsEngine` — `eval`, contexte par onglet,
  budget d'exécution. Couverture au branchement : noyau ES3/ES5 +
  subset ES2015 déjà présent ; la couverture grandit en phase 10
  (étape 43), mesurée sur extraits test262 (ADR-0010).
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

Objectif explicite : **naviguer le web réel — tous les sites**,
comme direction asymptotique (ADR-0001) : aucun moteur hors des
trois grands n'y parvient ; la progression se pilote par paliers
mesurés sur corpus publics (ADR-0010), pas par promesse. Pour
garantir des résultats mesurables sans épuisement, la compatibilité
progresse en trois paliers structurants :

- **Palier 1 — Web documentaire (statique)** : Wikipedia (mode HTML),
  spécifications W3C/RFC, blogs, sites de documentation. Layout flux normal
  robuste, fontes précises, liens fonctionnels, zéro crash.
  *Honnêteté de périmètre : une part croissante de ces sites utilise
  flexbox — sans l'étape 42 leurs pages dégradent en flux block ;
  c'est assumé et mesuré sur corpus, pas masqué.*
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
- [ ] **Étape 43. JS vers ES6+.** Étendre `dillo-js` dans ce
  workspace (modules `import`/`export`, `Proxy`, `Intl`, GC
  complet ADR-0008…) — échappatoire documentée : `boa_engine`
  derrière le même trait si le moteur maison ne suit pas
  (ADR-0007).
- [ ] **Étape 44. Web APIs.** `fetch`/`XHR`, `localStorage`/
  `sessionStorage`, History API, `MutationObserver`, workers
  (bornés).
- [ ] **Étape 45. Médias et graphiques.** `<canvas>` 2D,
  `<video>`/`<audio>` (décodage à arbitrer), `requestAnimationFrame`,
  animations/transitions CSS, **`<svg>` statique** (sous-ensemble —
  `resvg`/crate équivalent ; SMIL et animations SVG restent hors
  périmètre).
- [ ] **Étape 46. Webfonts + charsets.** WOFF/WOFF2, `font-display`,
  encodages legacy (windows-1252, ISO-8859-*).
- [ ] **Étape 47. Streaming UI haute performance.** Texture externe
  Flutter via FFI (upgrade d'ADR-0004) pour les framerates élevés et
  médias animés.
- [ ] **Étape 48. Interop continue.** Corpus **WPT** ciblés +
  sites réels, jalons Acid2/Acid3 informatifs (ADR-0010) ; les
  écarts constatés deviennent des étapes.

## Hors périmètre assumé (documenté, non planifié)

Fonctionnalités d'un navigateur complet volontairement hors
roadmap — candidats à un ADR dédié si un besoin réel émerge :

- `<iframe>` et navigation multi-contextes, shadow DOM ;
- accessibilité native (arbre a11y, lecteurs d'écran) ;
- gestionnaire de téléchargements, impression, zoom page,
  recherche en page ;
- SVG animé (SMIL, `<animate>`…) — `<svg>` statique reste prévu
  en phase 10 (étape 45) ;
- extensions, DevTools complet (`explorer-cli` couvre le debug
  moteur) ;
- site isolation multi-processus (modèle de menace : ADR-0011).

