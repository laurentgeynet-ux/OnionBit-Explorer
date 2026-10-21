# Roadmap — Équipe NAVIGATEUR

**Périmètre :** la plateforme produit — `explorer-net`,
`explorer-core` (session/pipeline), `explorer-api`,
`explorer-daemon`, `explorer-cli`, `explorer-ffi`,
**toute la surface JS** (`explorer-js` + `explorer-jscore`),
`app/` Flutter, `scripts/`, `fuzz/`.
**Ne touche pas :** les crates domaine pur (`explorer-common`,
`explorer-dom`, `explorer-html`, `explorer-css`,
`explorer-layout`, `explorer-render`) — propriété PIPELINE,
consommées via leurs API publiques.
**Partagé :** `explorer-test-support` (fixtures/stubs additifs
libres, structure via le journal — AGENTS.md §10).

**Deux pistes de numérotation** dans ce fichier :
- **Piste produit** : étapes **50+** (suite de la séquence
  historique phases 0–10).
- **Piste JS** : **JS-C26+** (suite de la série `explorer-jscore`).

**Definition of done** (inchangée) : implémenté + testé +
CHANGELOG + case cochée + `verify_all` vert + commit dédié.
**Spécialiste JS** : la piste JS est tenue par la personne qui
connaît `explorer-jscore` — rôle nommé au sein de l'équipe, pas une
équipe séparée. `JsEngine` est un contrat **interne** à
l'équipe (plus de sign-off croisé, extensions additives
recommandées mais non bloquées par l'autre équipe).

Légende : `[ ]` à faire · `[~]` en cours · `[x]` terminée.
Les besoins dans le domaine pur passent par
`docs/interface-equipes.md` — marqués **↔ PIPE**.

---

## Piste produit (étapes 50+)

### Palier N-A — Le navigateur charge vraiment les pages

Objectif : les blocages du web réel sautent. Sortie de palier :
`site_smoke.ps1` étendu à 10 sites, **8/10 fonctionnels**
(navigation + liens + formulaires + scripts de base actifs).
**Mesuré : 10/10 à `ef8ee76`** — chaque site : nav OK, lien
réel cliqué par hit-test moteur, back restauré, dispatch de
scripts OK ; `form:absent` honnête partout (aucun champ
éditable dans les bandes sondées — corpus quasi sans
formulaires en premier viewport). **Re-mesuré : 10/10 à
`76e3f34`** après la vague JS-C36→C52 / R-43→R-53 /
étapes 91-94 — la mesure a d'abord lu 5/10 par épuisement
des ports éphémères Windows côté sondes (~16 000 TIME_WAIT,
`WSAEADDRINUSE`) ; `site_smoke` passe alors sur client HTTP
poolé keep-alive, la valeur réelle remonte.

Shortlist candidate (reconnaissance terrain @ `2693959`,
panel de 8) : **mesurables** lobste.rs, wikipedia ·
**régression** example.com, info.cern.ch · **stress** HN.
Écartés : duckduckgo (blanc « par design » —
`body{display:none}` anti-FOUC honoré, script de déblocage
non exécuté), old.reddit (shadow DOM/custom elements, hors
portée). **MDN débloqué par la décision FrameTooBig**
(rend complet 562 Ko) — candidat smoke recevable ;
lemonde/bbc rendent aussi (lemonde tronqué au cap).

Balayage robustesse (14 sites, même session) : **vitrine
`text.npr.org` quasi-parfaite**, weather.gov ~75 % (images
NOAA peintes), csszengarden/git-scm/npmjs/go.dev/arstechnica/
rust-lang rendent — **ajouter les deux premiers au smoke**.
Écartés en plus : github (crash explorer-jscore → JS-C41 livré),
stackoverflow (403 bot-block, honnête), craigslist (TLS
`close_notify` → étape 53c livrée), **lemonde/bbc (classés :
`<noscript>` dans `<head>` avalait le document — R-15 livre
le mode `InHeadNoscript`, à re-vérifier sur nouveau build)**,
lite.ddg (= R-16, `<center>` inline → corrigé en vol).

**Décision tranchée — borne `FrameTooBig`** : option (a),
plafond côté NAV — `Session::repaint` tronque
`DisplayList.height` à `16 384/scale` px avant `rasterize` :
un document géant produit une frame du **sommet** plutôt
qu'une erreur sèche, et `paint_progress` émet enfin
`frame_invalidated` pendant le téléchargement sur pages
> 16 384 px (github, MDN — le bug de silence QA `cfa0514`
est levé). Limite assumée : au-delà du cap le bas du
document n'est pas rasterisable — le scroll n'y montre rien
(besoin PIPE consigné `interface-equipes.md` : fenêtre de
raster). Rejetées : (b) tuiler le PNG — complexité de
transport pour un usage debug ; (c) relever le cap —
l'encode streamé déplace la borne sans la supprimer, et le
cap est aussi anti-hostile (mémoire frame = w×h×4). Smoke
10 sites : **10/10** (`github` 6,4 Mo, `stallman` 7,5 Mo —
frames tronquées au sommet, MDN 562 Ko complet). Reste le
coût quadratique du pipeline complet par palier — vraie
sortie : R-12 (layout incrémental) ou cadencement temporel.

- [x] **Étape 50. Scripts externes + scheduler de
  sous-ressources** (ADR-0012 complété côté session).
  `<script src>` fetché via `HttpTransport` : ordre document,
  `defer` (post-parse, ordre garanti), `async` (à réception),
  `type=module` → `ModuleLoader` de `explorer-jscore` (existe, JS-C5).
  Scheduler **parallèle** : files bornées par onglet
  (`SessionConfig`), priorités `document > CSS > script > img`,
  N fetches en vol (`FuturesUnordered`), `Referer`/`Origin` et
  mixed-content déjà calculés à l'émission. Insertion dynamique
  de scripts : hooks à définir avec JS-C35.
  *Pourquoi en premier : trou le plus bloquant du web réel.*

- [x] **Étape 51. Câblage Web APIs — clôt l'étape 44 de
  ROADMAP.md.** Intégralement interne à l'équipe :
  - `JsEngine` : traverse proxy des surfaces déjà livrées dans
    `explorer-jscore` — `XhrTransport`/`deliver_xhr_response`,
    `install_storage`, `CookieJar` (pattern `drain_nav_requests`
    généralisé : `drain_net_requests()` + `deliver_net_response()`).
    **Livré par JS-C27** (commit `2b73089`).
  - Session : exécute le fetch en tâche tokio, livre à un point
    de synchro — jamais de callback ré-entrant.
    `drain_script_net` → spawn tokio → canal → `pump_net`
    (daemon : battement 50 ms ; CLI : grâce 1 s ; interne :
    `poll_net` post-événements).
  - `localStorage` : map par origine persistée dans le profil ;
    `sessionStorage` par onglet (mémoire). `ProfileStore`
    étendu (`load_storage`/`save_storage`, implémenté par
    `JsonFileStore` — `storage-<slug>.json` par origine).
  - `document.cookie` : bridge vers le jar `explorer-net`
    (politiques Secure/SameSite héritées) — `CookieTransport`
    daemon partagé avec la session, `HttpOnly` filtré des
    écritures script.
  - **Réévaluation ADR-0019 faite** : same-origin vs CORS
    minimal (requêtes simples + ACAO `*`|origine exacte,
    preflight hors MVP), `Origin` sur CORS, quota
    `max_script_net` = 16/onglet, `Set-Cookie` capturé
    same-origin seul — déviations documentées dans
    l'amendement de l'ADR. Tests : 8 scénarios (succès,
    opaque, ACAO, preflight-bloqué, quota, jar, Set-Cookie,
    storage).

- [x] **Étape 52. Boucle d'événements v2 + tick timers.**
  `dispatch_event(ty, path)` existe — étendre les types :
  `mouseover`/`mouseout`/`mousemove` (coalesced),
  `keydown`/`keyup`/`input`, `focus`/`blur` (liés au focus champ
  existant), `wheel`/`scroll`. **Tick dédié** :
  `next_timer_deadline` de `explorer-jscore` devient un délai armé par
  le daemon (et `tick` FFI) — `setTimeout`/`setInterval`/`rAF`
  tirent sans attendre un input ; borne de jobs par tick
  (ADR-0019). `MutationObserver` = JS-C26, drainé aux mêmes
  sync points.

- [x] **Étape 53. Session incrémentale** (moitié orchestration de
  l'ex-53) : la session alimente `push(&str)` du tokenizer par
  chunks pendant le fetch, relayout cadencé (N ms ou N octets),
  `frame_invalidated` intermédiaire → peinture pendant le
  téléchargement. ↔ PIPE **R-1** fournit la tolérance « document
  en construction » côté domaine — tenu en tête de leur file
  pour débloquer cette étape.

- [x] **Étape 53b. Charset des sous-ressources** (bug terrain
  @ `2693959`, repro : example.com) : les corps de scripts
  (`session.rs` ~l.1562), modules (~l.1840) et feuilles CSS
  (~l.1560) passent par `decode_sniffed` — le sniffer **HTML**,
  dont le fallback Latin-1 corrompt les littéraux UTF-8
  (mojibake « Lâ€™usage » injecté par `insertAdjacentHTML`).
  Spec : script = `charset` attribut → BOM → UTF-8 ; CSS =
  `@charset`/transport → UTF-8 ; jamais le sniffer HTML hors
  document. Borné — régression vérifiée sur example.com.

- [~] **Étape 52b. Défauts event-loop vus en vrai** (QA daemon
  @ `cfa0514`, repros `D:\explorer-fieldwork\repros\`) :
  1. **timers ne tirent qu'à l'input** — `setTimeout(8s)` n'a
     couru qu'au scroll suivant : la pompe de fond du daemon
     (`main.rs` ~l.144-164) existe mais ne livre pas les
     échus ; `dispatch_path_event` pompe à chaque input
     (`session.rs` ~l.2556). Une page dont le contenu arrive
     par timer reste figée sans interaction.
     **Fait** : pompe vérifiée saine — le symptôme était la
     même racine que le silence `FrameTooBig` (`relayout`
     échouait → `apply_dom_sync` rendait tôt → zéro
     `frame_invalidated`), levé par `986f01f`. Couvert par
     test `delayed_timer_pumps_and_emits_events` (timer
     différé, mutation DOM, événements — sans aucun input) ;
  2. **texte tapé invisible** — en fait **jamais peint** :
     l'enfant `Text` synthétique du contrôle n'entre jamais
     en `lines` (atomique `inline-block` + bras
     `is_inline_level` de `flow_children`) — bug PIPE
     `explorer-layout`, consigné `interface-equipes.md` avec
     diagnostic complet (repro `typed_text_repaints_new_value`,
     ignoré tant que PIPE n'a pas livré) ;
  3. **`document.title` par JS** → aucun `title_changed` émis
     (le seul reçu portait l'ancienne valeur).
     **Fait** : `document.title=` posait `Ok(false)` — setter
     implémenté (remplace le `<title>` existant, sinon en crée
     un sous `<head>`) + `emit_navigated` annonce le titre
     **vivant** (les scripts tournent avant l'émission — le
     titre extrait au parse était ré-émis et écrasait la
     mutation).
- [x] **Étape 53d. API réactive pendant la navigation** (QA
  daemon @ `cfa0514`) : le mutex session unique gelait toute
  l'API pendant `navigate` — nav France > 8 min ⇒ `status`,
  `tabs`, `frame` timeoutaient tous. La navigation est
  désormais éclatée en phases : `nav_begin`/`nav_apply`/
  `sub_commit` sous verrou court, `NavJob::run` (fetch +
  parse incrémental) et `SubJob::run` (vague
  CSS/scripts/images, `@font-face`, prefetch des graphes
  modules) hors verrou sur clones `Arc`. Les drivers
  `navigate_shared`/`drain_navs_shared`/`history_move_shared`
  conduisent une `Mutex<Session>` sans await réseau sous
  verrou ; les intentions input/script convergent dans
  `pending_nav` (`handle_input_now` ne fetch plus) ; l'epoch
  par onglet invalide les navigations supplantées ; la
  peinture progressive passe par `try_lock` (un tick manqué
  est repeint au suivant). Régression couverte :
  `shared_session_stays_responsive_during_navigation`
  (lock < 500 ms pendant un corps en vol). API :
  `navigate`/`reload`/`back`/`forward`/`input` + pompe
  daemon basculés ; `navigate`/`handle_input`/`drain_script_nav`
  compat `&mut self` conservés (CLI, FFI, tests).
- [x] **Étape 53c. Tolérance TLS `close_notify` absent**
  (bug terrain, repro : craigslist.org — `TLS unexpected-eof`)
  : un pair qui ferme TCP sans `close_notify` doit terminer
  le corps proprement quand la longueur est connue (CL/CD
  atteinte) au lieu d'échouer la réponse. Tolérance rustls
  documentée — cas fréquent chez les vieux serveurs/CDN.

- [x] **Étape 53e. Charset transport honoré pour le
  document** (bug terrain mission 6, repro HN `â€“`) : la
  page déclare UTF-8 par en-tête `Content-Type`
  (`charset=utf-8`), sans `<meta>` — le `StreamDecoder`
  ne voyait que BOM+prescan → repli 1252. Livré : le
  décodeur reçoit `hint` (`transport_charset` +
  `label_encoding`), décision ordre WHATWG **BOM réel >
  charset transport > prescan `<meta>` > windows-1252** —
  titre et corps décodés UTF-8 quand le serveur le dit.
  Tests : hint > méta, BOM > hint, titre `Actualités –`
  via `Content-Type` seul.

- [x] **Étape 53f. `run_pending`/`sync_dom` hors verrou**
  (résidu 53d, terrain mission 8 : `repros/relayout.html` —
  200 `appendChild` triviaux → `/status` gelé ~20 s) :
  `pump_timers_shared` sort le moteur de l'onglet
  (`js_detached(epoch)`) et exécute `run_pending` +
  sérialisation miroir dans `spawn_blocking` — le verrou
  n'est tenu qu'au take/commit, garde epoch anti-commit
  concurrent, tampons réponses net + pushes d'attributs
  rejoués à la réinsertion. Boucle daemon migrée ;
  dégradation documentée : `dispatch_event` en plein run =
  handler inerte (input traité côté hôte). **Terrain mission
  14** : le run JS est bien libéré — l'API répond pendant
  ~5 min de travail post-commit. **Mais** la phase commit
  `sync_dom` (sérialisation miroir → re-parse → cascade →
  layout → raster) reste intégralement sous verrou :
  ~9-10 min de gel sur clic-mutation à 3 000 nœuds — le
  mur s'est déplacé, pas tombé → étape 55h.

- [x] **Étape 54. Formulaires v2 — côté produit** (ex-55, moitié
  session) : `multipart/form-data`, états `:valid`/`:invalid`
  exposés aux events, popup `<select>` via widget natif Flutter
  (arbitrage : rendu moteur ↔ PIPE ou widget app — décider en
  début d'étape).
  **Fait** : arbitrage tranché — popup `<select>` **rendu
  moteur** (overlay chrome peint en fin de display list, aucun
  contrat Flutter ajouté — ADR-0014 révisée) : options figées
  à l'ouverture, select retenu par chemin `(tag,index)`,
  clavier `Arrow↑/↓`/`Enter`/`Escape`, clic hors popup =
  fermeture consommée. Validation de contraintes HTML
  (`required`, `min/maxlength`, `pattern`, `min`/`max`/`step`)
  avant tout submit — `invalid` dispatché, `novalidate`/
  `formnovalidate` honorés, bouton par défaut = `submitter`
  de la soumission implicite `Enter`. Côté JS : `validity`,
  `validationMessage`, `willValidate`, `checkValidity`,
  `reportValidity`, `setCustomValidity` (canal miroir→DOM par
  `data-explorer-cv`), `form.submit()`/`requestSubmit()` via
  `NavRequest::FormSubmit`. `enctype=multipart/form-data` +
  entrée `File` (fichier vide — pas de picker). `:valid`/
  `:invalid` CSS → besoin consigné `interface-equipes.md`
  (PIPE). 20 tests (session + dom).

### Palier N-B — Produit, profil, release

- [x] **Étape 55. Dirty-rects transportés** (ex-63b) : régions
  produites par PIPE **R-7** (`DisplayList::changed_rects`) →
  `FrameInvalidated.rects` (px physiques ; vide = plein
  cadre), pull partiel REST `frame?x=&y=&w=&h=` +
  `Session::frame_region`/`frame_png_region`/`take_dirty`,
  FFI `explorer_frame_copy_region` + `explorer_dirty_rects`.
  Accumulation plafonnée à 32 régions puis « pleine frame ».
  **ExternalTexture GPU reporté** — à mesurer sur de vraies
  frames avant révision ADR-0004/0018. Régression :
  `dirty_rects_reported_and_region_served`.
- [x] **Étape 55b. `<svg>` branché en session** (terrain
  mission 7 — R-9 livré mais jamais câblé : espace réservé,
  rien peint ; repro `svg.html` + example.com + octicons
  github) : `Tab::images` promu `PaintImages` complet —
  `svg::svg_sizes` fusionné aux tailles `<img>` avant
  `build_box_tree_sized`, `svg::collect_svg` rasterise à
  chaque reflow vers `PaintImages.svgs` (borne
  `image_max_px`), `repaint` consomme
  `build_display_list_full`. Gelée dans le bfcache avec le
  reste de la table. `canvases` attendra JS-C28
  (enregistrement op-list côté `explorer-jscore`) — même couture
  déjà figée.
- [x] **Étape 55c. Miroir→DOM : source `<script>` injectée
  comme texte** (terrain mission 6+7, wikipedia — clientpref/
  RLSTATE JSON toujours peints après R-20) : le filet R-20 vit
  dans `build_box_tree_sized` (tag statique) mais le texte
  provient de la **resynchro miroir**. Cause trouvée côté
  parseur miroir (`parse_fragment_into`) : le `</tag` de
  fermeture rawtext était cherché via `to_lowercase().find`
  — index **octet** désaligné par les runes à casse
  variable (`ﬀ`, `İ`…) → `</script>` raté, suite du
  document parsée en texte hors élément. Scan insensible
  à la casse directement dans `cs` + sérialisation
  **verbatim** des enfants `script`/`style` (repli échappé
  si la source contient son propre `</tag`) + décodage
  RCDATA `title`/`textarea`. Jamais de `Text` portant la
  source hors de son élément au round-trip.
- [x] **Étape 55d. `dirty` drainé par événement SSE/REST**
  (défaut 55, terrain mission 8) : watermark `emitted` sur
  `Tab::dirty` — chaque `FrameInvalidated` rapporte
  `dirty[emitted..]` (le delta depuis l'émission
  précédente) puis avance la tête ; l'accumulation reste
  vivante pour `take_dirty` (pull FFI indépendant — le
  handle FFI garde un receveur inerte, un vrai drain
  l'affamerait). Limite documentée : après `scroll` les
  rects = document entier (défilement cuit dans le raster —
  inutilisable pour un pull partiel). **Vérifié terrain
  mission 11** : 38 deltas sur 41 événements, zéro collapse
  permanent au-delà de 32 — mais 2 pleins cadres
  sporadiques (~clic 16/32, périodicité à creuser : tick
  timer ou GC déclenchant un full relayout ?).
- [x] **Étape 55e. `@keyframes` cadencés en session**
  (adoption R-10, règle AGENTS §10.5b — confirmé non-câblé
  par `anim.html` : frames identiques à 400 ms) : **livré** —
  `ActiveAnimation` par `(ordinal, index)` + `start` conservé
  tant que la déclaration est identique ; surcharges
  keyframes sous transitions (`update_transitions` scindé
  diff/surcharge, couche spec) ; `advance_animations` partagé
  `tick_animations` (pull `/frame`) + `pump_timers_all`
  (battement hôte) — plus figée entre deux pulls ; repli
  style de base en fin de lecture via drapeau `applied`.
  **Terrain mission 14 — rouvert** : `anim.html` = 6 pulls
  `/frame` byte-identiques sur ~4 s — la cadence ne produit
  aucun raster en session. Piste : `pump_timers_all` a été
  remplacé par `pump_timers_shared` (53f) — le battement hôte
  `tick_animations` a-t-il survécu à la migration ?
  **Résolu** : la branche « pas de moteur » de
  `pump_timers_shared` rendait `0` sans avancer les
  animations — une page `@keyframes` purement CSS
  (`js: None`) ne battait jamais côté daemon, et l'UI v2 ne
  tire `/frame` que sur `FrameInvalidated` SSE (jamais émis)
  → figé total. Le `else` appelle désormais
  `advance_animations` sous verrou court (early-out ~gratuit
  sans anim en lecture) ; régression couverte par
  `shared_pump_advances_scriptless_animations`.
- [x] **Étape 55f. `relayout_subtree` adopté en session**
  (adoption R-12, §10.5b — mesuré : mutation DOM = re-parse
  + relayout global) : `apply_dom_sync`/`repaint` doivent
  viser le sous-arbre sale (`Relayout::Done`) et ne retomber
  en reflow que sur `NeedsFullLayout` — bench PIPE :
  ~33 ms → ~1,9 ms feuille, soit ~17× sur mutation locale.
  Livré : miroir → `dirty_node`/`take_dirty_path`, contrat
  `sync_dom_path` (`DomSyncPayload`), remap pré-relayout des
  boîtes conservées (ids non stables au re-parse), repli
  sûr sur `NeedsFullLayout`/id irrésolvable.
- [x] **Étape 55g. La resynchro reste globale — l'incrémental
  ne vise que le relayout** (terrain mission 12) : **livré
  côté NAV** — mesure par phases sur ~3 000 nœuds (test
  `profile_resync_phases`, `#[ignore]` à la demande) :
  `relayout+repaint` 28,6 s → **~17 s** debug. Trois gardes
  posées : (1) **cull raster** — les items-feuilles
  entièrement hors zone raster réelle (post-cap
  `MAX_FRAME_DIM`, jamais sous `PushTransform`) sont jetés
  avant `rasterize` : ~18,5 s → ~6,5 s sur le repro
  (`paint_text` ne re-trace plus chaque glyphe du document
  hors-champ) ; (2) **diff dirty-rects borné**
  (`DIRTY_DIFF_MAX_ITEMS` = 2 000 items) — `merge_rects`
  O(k²) coûtait ~2 s sur ~8 500 rects avant de s'effondrer
  en pleine frame : on la produit directement ; (3) **PNG
  paresseux** — `repaint` n'encode plus : `frame_png(&mut)`
  encode+cache à la demande (~4 s sortis du chemin de
  resynchro ; un burst de repaints n'encode qu'une fois).
  Reste en PIPE (journal `interface-equipes.md`) : fusion
  de rects linéaire ou seuil interne à `changed_rects`,
  cache glyphe→outline pour `paint_text`, layout/cascade
  scopés sous-arbre — les ~15 s résiduels du repro debug.
  **Terrain mission 13 — frontière instrumentée** :
  `profile_click_sync_heavy` (mod interne, `#[ignore]`)
  mesure le clic-mutation ~3 000 nœuds bout à bout :
  `dispatch_path_event` ~18 s debug **sous verrou** dont
  eval 0,5 ms + sérialisation miroir ~5 ms + re-parse ~38 ms
  + `try_sync_incremental` ~18 s — pas de repli, le
  sous-arbre couvre presque tout le `<body>` pour un append
  en tête de flux (`relayout_subtree` ~14 s, cascade ~0,3 s,
  repaint ~3,7 s). NAV sorti : remap des `NodeId` mémoïsé
  par parent (~3,4 s). Reste PIPE (journal
  `interface-equipes.md`) : `relayout_subtree` dominant,
  cascade scopée sous-arbre, cache glyphe→outline
  `paint_text`. La récidive `/frame` 204 post-gel est
  couverte par `ensure_painted` (56b). **Suite livrée** :
  (1) instrumentation de phase permanente sur la cible
  `sync_profile` (`RUST_LOG=sync_profile=info` — dispatch,
  run_pending, sérialisation miroir, re-parse, incrémental
  vs reflow, repaint, phases pompe off-lock/commit) pour
  attribuer le prochain gel à ses phases exactes ;
  (2) **coalescence `sync_floor`** — le mécanisme du trou
  ~7 min : une page qui mute en continu (`setInterval`)
  rendait le miroir sale à chaque battement → chaque pompe
  payait un apply complet **sous verrou** → verrou tenu
  ~100 %, API morte. Désormais le prochain apply pompe
  attend `coût_apply.clamp(paint_interval,
  resync_gap_max_ms)` (config, 4 s) — le miroir accumule,
  l'apply autorisé absorbe tout en une passe, l'API
  respire entre deux peintures. Les synchros d'événements
  restent immédiates ; la saisie (pushes d'attributs) est
  toujours rejouée. Régression :
  `pump_resync_coalesced_under_floor`. Reste PIPE : coût
  unitaire de l'apply (`relayout_subtree` dominant) —
  chaque fenêtre de coalescence agrandit le delta
  accumulé, mais la cadence reste bornée par le coût
  unitaire (journal `interface-equipes.md`).
- [x] **Étape 56. Profil persisté complet** (ex-64) : cookies
  (`cookies.json` via `ProfileStore`, session exclus),
  localStorage (cf. 51), **cache disque borné**
  (`DiskCache` daemon, port `CacheStore` net — FIFO
  `max_total_bytes`), historique — namespaces cloisonnés
  par répertoire de profil (`EXPLORER_PROFILE`, ADR-0011),
  répertoire déplaçable. + **bfcache minimal** (terrain
  mission 5 : back/forward re-fetche systématiquement — HN
  retour 40 s) : `PageSnapshot` gelé à la position
  d'historique (DOM, arbre, frame, scroll, moteur JS figé),
  restauration instantanée sans réseau, borné
  `bfcache_pages` (FIFO, 4). + **sémantique historique
  POST** (terrain mission 6) tranchée navigateur-réel :
  l'entrée est poussée **sans dédup** et la requête
  complète est rejouée au déplacement/`reload` — re-POST,
  plus de re-GET. **Vérifié terrain mission 9** : back
  ~40 s → **46 ms** zéro hit réseau, forward 52 ms, scroll
  restauré, FIFO exact (5ᵉ back sur page évincée = re-fetch),
  `back` sur un POST retombe sur la page résultats (61 ms —
  l'entrée est désormais poussée), forward re-gel instantané.
  **Vérifié terrain mission 11** : profil complet (cookie +
  localStorage + cache disque) survit à `taskkill /F` ×2 —
  nav post-kill = 0 hit serveur.
- [x] **Étape 56b. `/frame` → 204 persistant après commit
  — ROUVERT (reproduit mission 13), fermé par auto-guérison** :
  le fix « snapshot sans produit » couvrait bfcache, pas la
  classe entière — mission 13 : nav `World_Wide_Web`
  commitée → 204 persistant ; récidive après gel
  `heavy.html`. **Livré** : `ensure_painted` — un DOM
  commis sans frame re-peint paresseusement à la première
  demande (`frame_png`, `handle_input`), quel que soit le
  chemin qui a jeté le produit (`unspec` sur relayout
  impossible, fenêtre commit→repaint…) — l'état empoisonné
  devient transitoire au lieu de persistant ; un DOM
  réellement non-layable sert un 204 honnête. Régression :
  `committed_dom_repaints_on_frame_pull`.
- [x] **Étape 56c. État hybride bfcache après back+navigate**
  (terrain mission 11, repro A→B→back→`navigate B`) :
  **livré.** Deux mécanismes corrigés : (1) `spec_paint` +
  `unspec` — toute frame issue d'un document en vol est
  révertée au gel/abandon/échec/commit (repeint depuis le
  DOM commis, ou jette arbre/frame si rien de commis), le
  snapshot bfcache ne peut plus capturer l'hybride ;
  (2) `try_forward_nav` — `navigate` GET vers l'URL exacte
  de l'entrée forward = restauration bfcache (commit
  complet : snapshot + `hist_idx` + événements), jamais un
  fetch peint sans commit. Cadence progressive durcie :
  pas de tick sur morceau sans texte décodé (fenêtre de
  sniffing `StreamDecoder`). Régressions :
  `superseded_nav_reverts_speculative_paint`,
  `navigate_to_forward_url_restores_bfcache_entry`.
  **Vérifié terrain mission 13** : A→B→back→`navigate B` →
  url=frame=hit-test=B, `can_forward:false` — hybride mort.
- [x] **Étape 56d. `#fragment` = navigation complète, sans
  scroll** (terrain mission 11 — `anchor.html` + footnote
  wikipedia) : **livré.** `try_fragment_nav` traite le
  même-document **sans re-fetch** (push historique +
  `Navigated` + scroll vers `id` puis `<a name>`, `#` nu →
  haut) ; les fragments cross-document sont appliqués
  post-commit dans `sub_commit` (`scroll_to_fragment` →
  `box_y_of_node`, borné `scroll_to_y`). Back/forward vers
  une entrée fragmentée rescrolle — la requête historiée
  porte le fragment. Régression :
  `fragment_nav_scrolls_without_refetch`. **Vérifié terrain
  mission 13** : clic `#sec50` → `scroll_y:1433` sans
  re-fetch ; back→haut, forward→rescrolle ; `name=` idem.
- [x] **Étape 56e. `<img src>` jamais fetchée en session**
  (terrain mission 11 — `cache_server` `/page` avec
  `<img src=img.png>` : nav complète, zéro requête serveur
  pour l'image). **Vérifié — non-défaut moteur** : le
  scheduler émet `SubTag::Img` et fetch bien (régression
  `img_src_triggers_subresource_fetch` : document + image =
  2 hits transport). La panne venait du harness
  `cache_server.py` : variable `PAGE` morte — `do_GET` ne la
  servait jamais, la page réelle ne contenait pas d'`<img>`.
  Harness corrigé (`/page` sert `PAGE`).
- [x] **Étape 56f. Images `data:` invisibles en session**
  (terrain mission 12 — fixtures `img.html`, `b1`-`b4`) :
  **livré.** `collect_data_images` (R-13) câblé dans
  `relayout_dom` après la cascade — `replaced`/`backgrounds`
  décodés inline avant `image_sizes` (sizing intrinsèque
  servi), fusionnés aux corps fetchés. Planificateur :
  seuls http(s) partent en fetch — un `data:` ne produisait
  qu'un échec. Débloque img `data:` + backgrounds `data:` +
  les yeux Acid2 (avec R-25). Régression :
  `data_url_images_paint_inline`. **Vérifié terrain mission
  13** : b1–b4 = 1 800 px non-blancs (`<img>`/`<object>`/
  cascade `data:` peints) ; img http + data = 12 800 px.
- [x] **Étape 57. UI Flutter v2** (ex-65) : **thème clair** —
  variante `tokens.light.json` + `meta.scheme` (pilote
  `ColorScheme` et `ThemeMode` : clair/sombre/système, repli
  embarqué `fallbackLight`) ; **settings** persistés
  (`ui_settings.json` zéro-dep dart:io) — thème, moteur
  auto/REST/FFI (re-création du dépôt via provider), profil
  `EXPLORER_PROFILE` exporté avant chargement FFI ;
  **bibliothèque** — `GET/POST/DELETE /api/bookmarks` +
  `GET /api/history` consommés (étoile navbar togglée,
  panneaux historique/signets — FFI dégradé : ABI sans store,
  documenté) ; **arbitrage ADR-0021** : **zoom livré côté UI**
  — `setViewport(w/z, h/z, z·dpr)` = reflow réel + raster net,
  entrées ÷(z·dpr), Ctrl±/0 + menu — **find-in-page reclassé**
  (surface moteur requise → étape dédiée + journal).
  Régressions : signet toggle, historique provider, zoom
  borné, store réglages roundtrip, thème clair dérivé.
- [x] **Étape 57bis. Find-in-page** (reclassé depuis 57,
  ADR-0021) : livré **sans** attente PIPE — la surface
  nécessaire existait déjà (`LayoutBox.text`/`rect`/`node`
  publics, overlay display list du popup `<select>`).
  `session/find.rs` : `find_text_boxes` sur le boxtree
  (insensible à la casse, ordre document), `FindState` sur
  `Tab`/`PageSnapshot` (survit au bfcache, effacé à la
  navigation), surlignage `FillRect` translucide en fin de
  display list (jaune atténué / orange pour la courante),
  `find_step` boucle ±1, scroll-vers-occurrence tiers
  supérieur. `POST /api/tabs/{id}/find` `{query, dir}` →
  `{count, index, query}` ; FFI dégradé `FindResult.empty`
  (ABI non exportée). UI : barre `_FindBar` sous la navbar
  (Ctrl+F, Entrée = suivante, Maj+Entrée implicite via
  boutons, Échap ferme), compteur `i+1/n`, bouton navbar.
  **Granularité v1 : la boîte texte entière** — le
  surlignage au caractère près reste la demande PIPE
  (métriques de runs). Régression : comptage + boucle +
  FillRect orange dans la liste + effacement.
- [x] **Étape 58. `<video>`/`<audio>`** (ex-66) : arbitrage
  décodage pur Rust requis (ADR) — sinon stubs propres + affiche.
  **ADR-0023 : pas de décodeur** — aucun codec vidéo pur Rust
  viable (H.264/VP9/AV1 : openh264/dav1d lient du C, rav1d
  instable) et la sortie audio n'existe pas ; un décodeur
  from-scratch n'est pas une étape de palier. Livré :
  (1) **affiche** — `video[poster]` réutilise le canal image
  (collecte étendue, fetch borné `max_images`, bitmap remplacé
  peint dans la content-box — mécanisme déjà générique par
  `NodeId`) ; affiches `data:` via `poster_data_images`
  (complément du collecteur PIPE) ;
  (2) **boîtes de repli** — feuille UA complémentaire
  `MEDIA_UA_CSS` en `Origin::UserAgent` (`video` inline-block
  300×150 fond noir, `audio` masqué sans `controls`,
  `audio[controls]` barre 300×54 — l'auteur l'emporte) ;
  (3) **JS honnête** — stubs `HTMLMediaElement` dans
  `explorer-jscore` : `play()`/`setSinkId`/`setMediaKeys` rejettent
  `NotSupportedError` (chemin `.catch()` déjà géré par les
  sites), `canPlayType` avoue `""`, `paused`/`duration`(NaN)/
  `readyState`/`networkState` (NO_SOURCE si `src`) reflètent
  « rien à lire », `buffered`/`played`/`seekable` = TimeRanges
  vides, écritures IDL acceptées-ignorées — jamais de faux
  succès. Tests : 4 session (affiche peinte http+`data:`,
  boîte noire, audio masqué) + 2 explorer-jscore (props, rejet
  `play()`). Refinements PIPE consignés (`interface-equipes`) :
  vrais remplacés boxtree, `collect_data_images` sur poster,
  hints `width`/`height`.
- [x] **Étape 60. Éclater `session.rs`** (audit externe) :
  ~~3 500+~~ 7 307 lignes — le god module portait session +
  pipeline + scheduler + profil + hit-test + glue JS +
  décodage. Livré : `session.rs` → `session/` — 20 fichiers
  par responsabilité (`types`, `tab`, `navigate`, `net`,
  `js_glue`, `input`, `paint`, `frame`, `store`, `anim`,
  `popup`, `forms`, `scripts`, `dom_helpers`,
  `paint_helpers`, `nav_types`, `net_items`, `anim_items`,
  `decode`, `shared`, `tests`). Frontières : items internes
  en `pub(crate)` (`Session`/`Tab` gardent leurs champs
  privés dans `mod.rs` — les sous-modules sont des
  descendants, visibilité inchangée), API publique et
  ré-exports `lib.rs` strictement identiques (237 fns /
  49 items dénombrés identiques avant/après). Isolation :
  `session::decode` sort avec son propre `mod tests` —
  5 tests sur les fonctions pures (BOM, labels charset,
  morceaux UTF-8/UTF-16 coupés, ordre WHATWG feuilles)
  sans session ni réseau. Suite : l'éclatement des gros
  impl (`navigate`, `net`, `input` > 700 lignes) reste
  ouvert aux étapes suivantes — le découpage par
  responsabilité est posé, l'état `Session` unique
  demeure le point de cohésion (documenté dans
  `session/mod.rs`).
- [x] **Étape 59. Release 0.2** (ex-67) : packaging, smoke E2E
  étendu, doc. **Prérequis d'ouverture** : renommage
  `crates/dillo-js` → `explorer-jscore` (ADR-0020) — commit
  `chore:` dédié, à un moment où aucune piste ne touche le crate
  ; le nom de crate est surface publique de release.
  **Livré** : version workspace `0.1.0` → `0.2.0` (Cargo.lock
  régénéré), `package.ps1` → bundle `dist/explorer-0.2.0`,
  `smoke.ps1` étendu — serveur loopback routé par chemin,
  assertions sur les faits du cycle : `onclick=` → mutation
  rendue (hash PNG diffère), clic lien → nav + `back`,
  `scroll` → `scroll_y` (contrat 71d), `<video>` → offre
  `GET /media` (seam 71b), `find` → compte. CHANGELOG
  `[0.2.0] - 2026-10-20`. Prérequis déjà satisfait : le
  renommage est `a9a2d27`. Réceptions humaines ouvertes :
  vidéo réelle dans l'overlay (M25), œil dpr > 1 (82).

- [x] **Étape 55h. Apply de resynchro borné — verrou tenu
  en continu** (terrain missions 14+15 — la coalescence
  `sync_floor` borne la cadence, jamais la durée unitaire) :
  `storm.html` (`setInterval` 10 ms, ~600 nœuds) → ~85 s
  d'API morte sur un seul apply ; `sync_profile` en vrai :
  `pump.apply_dom_sync` = **157,1 s sous verrou** dont
  `incr.remap_relayout` 150,3 s et `incr.repaint` 6,7 s
  (`pump.offlock` 472 µs, `commit_lock` 810 µs). Idem
  mission 14 : clic-mutation sur 3 000 nœuds (synchro
  événement, exemptée du floor par design) = ~9-10 min de
  gel continu. Le mur n'est plus le run JS (53f l'a sorti)
  ni la cadence (55g-suite) : c'est **l'atomicité de
  l'apply** — sérialisation miroir → re-parse → cascade →
  remap/relayout → repaint, tout sous `&mut Session`.
  **Livré** : `session/apply.rs` — file `pending_applies`
  par onglet (une entrée coalescée par onglet, epoch-gardée)
  + `PaintPayload` détachable (`take_paint`/`put_paint`,
  les champs `Option` de `Tab` restent vides → dégradation
  naturelle : `/frame` 204 lazy-heal, hit-test mort,
  tick animation inerte). Tous les points de synchro
  (dispatch événement, pompe JS, livraisons XHR) enfilent
  au lieu d'appliquer ; la pipeline tourne en
  `spawn_blocking` via `drain_applies_shared` (daemon :
  pompe partagée + handler `/input` + battement net), le
  verrou n'est tenu qu'au take/commit. Hôtes `&mut`
  (FFI/CLI/tests) : `drain_applies` synchrone aux mêmes
  frontières — sémantique d'avant conservée. Deux
  rattrapages de fidélité : `freshen_apply` re-sérialise au
  drain si le miroir a remuté depuis l'enfilement (séquence
  bascule→`preventDefault`→restauration), et le commit
  repeint si le scroll a avancé pendant la fenêtre détachée.
  Le coût **unitaire** `remap_relayout` (150 s terrain vs
  ~14 s au profil test — écart ×10) reste la demande PIPE
  consignée dans `interface-equipes.md`.
- [x] **Étape 61. FFI : profil persistant + surface
  d'état** (terrain mission 15 — ABI exercée via ctypes,
  cycle complet OK mais deux gaps produit) : (1)
  `EXPLORER_PROFILE` exporté par l'app avant chargement FFI
  (étape 57) **jamais lu** par `explorer_create` → mode FFI
  = zéro persistance (le `ProfileStore` vit dans
  `explorer-daemon`) — lire l'env dans `explorer_create`
  ou ajouter `explorer_create_with_profile(path)` (ABI
  additive) ; (2) exports d'état manquants : `tab_title`,
  `tab_scroll` — le titre d'onglet n'a aucun canal FFI (la
  bibliothèque store est déjà documentée dégradée). Reste
  de l'ABI vérifié sain : nav réelle, `frame_copy`/régions
  clipées exactes, erreurs typées (`last_error`), UTF-8
  roundtrip, `dirty_rects`/`tick`/`close_tab`/`destroy`.
  **Livré** : `JsonFileStore` migré de `explorer-daemon` vers
  `explorer_core::profile_store` (les deux hôtes partagent
  l'impl — aucun crate plus bas ne peut voir les DTO sans
  dépendance montante ; les formes filaires `Raw*` serde
  restent privées, les DTO publiques neutres) ;
  `explorer_create` lit désormais `EXPLORER_PROFILE` +
  `explorer_create_with_profile(dir,len)` (ABI additive,
  chemin invalide = échec explicite) ; exports
  `explorer_tab_title` + `explorer_tab_scroll` (`-1` sans
  ambiguïté), câblés côté app avec garde `providesSymbol`
  — une dll antérieure garde le repli URL ; `sendInput`
  `scrolled` rapporte `scroll_y` comme le REST.
- [x] **Étape 62. Daemon : arguments CLI ignorés
  silencieusement** (terrain mission 15) : `--port`/
  `--profile` acceptés sans effet — toute la configuration
  passe par les variables `EXPLORER_*` (`EXPLORER_PORT`,
  `EXPLORER_PROFILE`, `EXPLORER_FONT_DIR`…). Les missions
  antérieures écrivaient dans le profil par défaut sans le
  savoir. Rejeter les args inconnus + `--help`, ou les
  honorer ; à minima documenter l'absence de CLI.
  **Livré** : `--port N`/`--profile DIR` (formes `--x v`
  et `--x=v`) sont honorés en surcharge des variables ;
  `--help` affiche l'usage ; tout flag inconnu ou valeur
  invalide sort 2 + usage — parser maison, zéro dépendance.
- [x] **Étape 63. Popup `<select>` : options peintes en
  vrai texte** (terrain mission 16, repro `repros/
  form.html`) : l'overlay s'ouvrait en fin de display list
  mais n'émettait que des `Placeholder` gris translucides —
  zéro glyphe, le surlignage flèches/Enter était sans
  retour visuel. Livré : `append_select_popup` shape
  chaque étiquette via `explorer_layout::font::shape` +
  `SharedFonts` (`sans-serif` ~¾ de ligne borné 9–16 px,
  encre `#202020`, blanc sur ligne sélectionnée), tronqué
  à la largeur avec `...` ; la baseline est centrée dans
  la ligne. Le clavier était logique-correct (choix et
  soumission déjà testés) — c'est l'absence de texte qui
  rendait le retour invisible ; `ArrowDown` repeint le
  bandeau de surlignage (test pixel). Reste de l'étape 54
  vérifié sain : `required` bloque la nav + `invalid`,
  `checkValidity`/`setCustomValidity`/`validationMessage`
  via JS, Enter implicite → POST urlencoded correct,
  multipart boundary cohérent.
- [x] **Étape 64. `navigated` rapporté au commit
  effectif** (terrain mission 16 — séquencement) : les
  drains `drain_script_nav`/`drain_navs_shared`
  comptabilisent les navigations **commises** (`is_ok`,
  y compris bfcache/fragment — un fetch mort n'en est
  pas une) ; `handle_input` et `POST /input` dégradent
  `Navigated` en `Edited` quand zéro nav n'a abouti —
  l'API ne rapporte plus une intention comme un état
  commis. `drain_navs_shared` retourne `usize` (appelants
  ignoraient le booléen) ; FFI `> 0`. Test : soumission
  vers cible 404 → `Edited`, URL inchangée.
- [x] **Étape 65. `doc_h` mesurée exposée (plafond
  raster distingué)** (terrain mission 16 — mesure) :
  `TabInfo` gagne `doc_h` (hauteur réelle posée, px CSS)
  et `doc_h_clamped` (`doc_h > MAX_FRAME_DIM / scale`) —
  additif ; `GET /tabs/{id}` sérialise les deux. La frame
  reste plafonnée 16 384 px mais le document réel a
  désormais un canal d'observation — `frame.height` n'est
  plus l'oracle de hauteur. Test : doc 20 000 px →
  `doc_h` > plafond, `clamped` vrai ; page courte → faux.
- [x] **Étape 66. `SameSite` appliqué à l'envoi** (terrain
  mission 17 — `cookies.rs`, `header_for`) : le flag était
  parsé/persisté mais `header_for` ne connaissait aucun
  contexte d'initiateur — un `SameSite=Strict` était
  renvoyé cross-site. Livré : `Request.initiator`
  (`Initiator::{TopLevelNav, NavFrom, Subresource}`,
  additif `Option` — sans contexte = permissif compat) ;
  `CookieJar::header_for_ctx` applique la politique
  (`Strict` : same-site `(scheme, registrable)` PSL seul ;
  `Lax` : + navigation top-level `GET`/`HEAD` ; `None` :
  libre, `Secure` exigé à la pose) ; `CookieTransport`
  passe par `_ctx` ; `nav_begin` (entonnoir `&mut` +
  `navigate_shared`) pose le défaut `NavFrom(page)` —
  `TopLevelNav` explicite aux navigations navigateur (API,
  `navigate`, reload, back/forward), `Subresource` aux
  sous-ressources/`fetch`/`XHR`/polices/modules,
  `NavFrom` aux formulaires. Le reste du jar vérifié sain :
  host-only filtré, `Domain=` rejeté sur suffixe public
  (`localhost` via PSL), `Max-Age=0`/`Expires` passé
  suppriment, `Secure` sur http rejeté sauf exemption
  localhost documentée.
- [x] **Étape 67. URL post-redirection non commise**
  (terrain mission 17) : `/r301` → `/landing` suit bien la
  chaîne (contenu peint, historique fusionné, borne 8 hops
  + erreur typée, 303 → GET et 307 → POST conservé), mais
  `tab.url` reste l'URL de **requête** — jamais la cible
  finale. Livré : `resolved_url: Option<Url>` additif sur
  `Response` **et** `StreamedResponse` (le défaut
  `fetch_stream` propage) ; `RedirectTransport` pose la
  cible quand ≥1 hop est suivi (fragment `Location`
  inclus) ; `NavJob::run` commet l'URL résolue — barre
  d'adresse, `record_visit`, base des sous-ressources
  relatives et `scroll_to_fragment` (`/x#sec`) alignés ;
  `tail.req` garde la requête d'origine (replay POST
  historique inchangé). Régression :
  `redirect_commits_final_url_and_fragment`.
- [x] **Étape 68. Find : surlignage absent quand le scroll
  ne bouge pas** (terrain mission 17, `session/find.rs` +
  `scroll_to_y`) : `find_in_page` → `find_scroll` →
  `scroll_to_y` ; si l'occurrence est déjà dans le
  viewport le early-return saute repaint + invalidate —
  les `FillRect` de surlignage sont calculés mais jamais
  rasterisés (0 px sur page courte ; 2 470 px corrects
  dès que le scroll bouge). Fix : repeindre même à scroll
  inchangé — la surbrillance est un changement de frame
  indépendant du déplacement. Livré : `scroll_to_y` renvoie
  `bool` déplacé (repaint fait) ; `find_scroll` repeint +
  invalide quand la position n'a pas bougé. Régression :
  `find_highlight_repaints_without_scroll`.
- [x] **Étape 69. Pages d'erreur navigateur** (lead —
  trou produit, terrain missions 16/17) : aujourd'hui une
  nav qui échoue (`HttpStatus` 404/5xx, `Fetch` connexion
  refusée/DNS, `EmptyDocument`) ne produit aucune surface —
  l'onglet garde la page précédente ou un vide, sans mot à
  l'utilisateur. Livrable : page d'erreur **peinte par le
  moteur** (document généré côté core, style minimal — pas
  de dépendance Flutter) : statut/code, URL, cause lisible,
  `can_back` conservé ; les erreurs typées existent
  (`CoreError`) — il manque la surface. Bonus : même canal
  pour `pipeline_error` SSE → la frame montre l'échec au
  lieu de figer l'ancienne page.
  Livré : `error_document` (dom_helpers) génère le markup
  (URL + cause échappées, cause lisible par variante
  `CoreError`, styles inline minimaux) ; `paint_error_page`
  (paint.rs) le commet via `relayout_dom` — URL tentée en
  barre, titre « Erreur de chargement », `last_req=None`
  (reload = retente GET), historique intact → `can_back`
  préservé, `Navigated` émis (commit effectif, §64) +
  `PipelineError` inchangé. Câblé sur les deux chemins
  d'échec `NavJob::run` (`navigate_request` +
  `navigate_shared`). Régressions :
  `failed_nav_commits_error_page`,
  `failed_nav_shared_commits_error_page`.
- [x] **Étape 70. Téléchargements** (lead — trou produit,
  terrain mission 17-D) : `application/octet-stream`,
  `Content-Disposition: attachment`, type inconnu en
  navigation top-level → aujourd'hui dump binaire mojibake
  peint en texte. Comportement navigateur : enregistrement
  fichier + progression. Livrable : events SSE
  `DownloadOffered`/`DownloadProgress`/`DownloadDone` +
  endpoint REST d'acceptation (`POST /api/downloads/{id}`
  `{path}` ou répertoire de téléchargement du profil) ;
  bornes de taille réseau respectées ; UI Flutter = entrée
  de téléchargement. Le dump-texte actuel reste la
  dégradation si le téléchargement est refusé.
  Livré : `download_meta` (net_items) détecte attachment /
  octet-stream / types non-documentaires ; `NavJob::run`
  renvoie `NavOutcome::Download(PendingDownload)` — corps
  bufferisé borné (`max_download_bytes`, défaut 64 Mio) +
  `DownloadProgress` par morceau. `Session::offer_download`
  enregistre sans commettre (page courante conservée) ;
  `accept_download` écrit (chemin explicite ou
  `download_dir` + nom sanitisé/unifié, `DownloadDone`),
  `decline_download` rejoue le corps par le pipeline
  document complet (dump texte historique). REST :
  `GET /api/tabs/{id}/downloads`,
  `POST …/downloads/{dl}/accept|decline` ; daemon :
  `--download-dir`/`EXPLORER_DOWNLOAD_DIR` (défaut
  `<profil>/downloads`) ; Flutter : dialogue
  « Enregistrer / Ouvrir comme texte ». FFI : stubs
  dégradés (`UnsupportedError`, pas de canal
  d'acceptation). Régressions :
  `download_offered_buffers_and_keeps_page`,
  `decline_download_replays_as_document`,
  `document_types_never_download`, REST
  `download_accept_writes_body`.
- [ ] **Étape 71. Seam `MediaSurface` — lecture média par
  l'hôte** (lead + ADR-0024 — le moteur ne décode pas, la
  lecture est un service d'hôte contracté) : les stubs
  honnêtes de l'étape 58 restent la norme *sans* hôte ;
  quand un hôte est attaché, `MediaOffered` (SSE : `tab`,
  `media`, `rect` de la boîte remplacée, `url` résolue,
  `mime`, `controls`) est émis pour chaque média lisible ;
  endpoints REST `claim`/`release`/`state` + `GET` ; l'état
  hôte remonte en events → les props JS deviennent réelles
  (`play()` résout à la 1re frame,
  `currentTime`/`duration`/`buffered` vivants,
  `canPlayType` reflète les codecs déclarés de l'hôte) —
  jamais de faux succès (hôte muet = stub ADR-0023).
  `MediaMoved` coalescé à la cadence de peinture quand
  relayout/resize déplace la boîte. Côté app : la
  surface vidéo est un overlay au-dessus du rect — la
  pattern `AbstractPlayerEngine` (backends `fvp`/FFmpeg
  desktop, ExoPlayer Android) de l'écosystème adjacent est
  le consommateur de référence. **Vigilance licence** :
  vérifier la config FFmpeg/libmpv de `fvp` (LGPL dynamique
  OK, GPL statique non). Prérequis doc : R-34 donne au
  seam une géométrie fiable (non bloquant).
  - [x] **71a — contrat** : seam rédigée dans
    `docs/interface-equipes.md` (directions des flux,
    identité `media: u64`, events SSE + REST, vocabulaire
    JS claimé vs stubs, FFI proposé). Énums/DTO figés à
    l'adoption ; routes REST et vocabulaire JS ajustables
    au premier consommateur réel.
  - [x] **71b — session + REST** : détection
    `<video>`/`<audio>` lisibles sur le couple DOM/boxtree
    posé (scan DOM — l'`audio` sans `controls` est
    `display:none` UA et doit être offert à 0×0), ids
    `media: u64` alloués à l'offre, retenue positionnelle
    (`kind`+`url`) sur apply JS, `MediaGone` à la mort de
    page (gel bfcache inclus — ré-offre = ids neufs) et à
    la fermeture d'onglet, `MediaMoved` émis uniquement au
    refresh post-repaint (borne « 1 par frame » tenue par
    construction). REST `GET /api/tabs/{id}/media[/{mid}]`,
    `POST …/claim` (409/410), `…/release`, `…/state`
    (`MediaReport` stocké, exige un claim). `MediaCommand`
    figé dans le DTO, non encore émis. Régressions :
    `media_offered_scan_kinds_and_rects`,
    `media_gone_on_close_and_navigation`,
    `media_claim_first_wins_and_state`,
    `media_ids_fresh_after_freeze_restore`,
    `media_moved_on_relayout_only_when_rect_changes`,
    REST `media_claim_state_and_gone`.
  - [x] **71c — délégation JS** : surface claimée →
    `MediaCommand` relayé à l'hôte (`play`/`pause`/`seek`/
    `volume`/`muted`/`rate`), `play()` résout au premier
    `state:"playing"` (rejetée sur `error`/release), props
    refletées depuis `MediaReport` (`paused`/`currentTime`/
    `duration`/`buffered`/`played`/`seekable`/`readyState`/
    `networkState`/`volume`/`playbackRate`/`error`) et events
    DOM non-bubblants tirés (`loadedmetadata`, `durationchange`,
    `play`+`playing`, `pause`, `seeked`, `timeupdate`, `ended`,
    `error`, `volumechange`, `ratechange`, `seeking`) ; non
    claimée = stubs ADR-0023 inchangés. Identité : le
    `media: u64` session transmis au claim est rappelé dans
    les commandes drainées (robuste aux re-paths). Ajout
    contractuel v1 : `MediaCmd::Muted` (distinct de
    `volume=0` — le dé-mute restaure). Tests :
    `media_js_commands_only_when_claimed`,
    `media_js_report_resolves_play_and_reflects_props`.
  - [x] **71d — composite Flutter** : overlay de lecture
    positionné `rect − scroll_y` × zoom (rects espace
    document, `scroll_y` transporté par `frame_invalidated`
    — champ additif du contrat, atomique avec
    l'invalidation). Consommateur de référence :
    `video_player` + `fvp` (backend MDK/FFmpeg — licence
    LGPL vérifiée : linking dynamique, binaire
    `mdk.dll`/`ffmpeg` non statique). `media_offered` →
    `claim` REST (`mediaCodecs` en `AppConfig`) →
    `MediaPlayer` natif ; `media_command` routé
    (`play`/`pause`/`seek`/`volume`/`muted`/`rate`),
    `media_moved`/`gone` mettent à jour/purgent ;
    `tab_closed`/`navigated` disposent. Rapports hôte
    `POST …/state` bornés ≤ 4 Hz (`mediaReportInterval`),
    saut de cadence au changement d'état. Audio 0×0 :
    joue sans overlay. `controls` → tap = play/pause
    (UI hôte minimale v1). FFI : méthodes stub
    `UnsupportedError` — variante in-process à spécifier
    au premier hôte FFI (journal). Tests :
    `media_offered → claim + lecteur natif`,
    `media_command route vers le lecteur`,
    `rapport state borné à 4 Hz`, `tab_closed dispose`,
    `frame_invalidated porte scroll_y`.
- [x] **Étape 72. Résolveur sécurisé (DoH) — capacité
  d'égress optionnelle** (lead — anti-blocage FAI « DNS
  menteur », même patron que le resolver custom de l'app
  IPTV ; prolonge ADR-0022) : `explorer-net` gagne un
  résolveur DoH (`application/dns-message` en GET/POST
  HTTPS vers un résolveur configuré — Cloudflare/Quad9/
  serveur perso, **aucune valeur en dur**, `ResolverConfig`
  ouvert en config session) comme stratégie de résolution
  optionnelle derrière `HttpTransport` : `system |
  doh(url)` au niveau du connecteur, jamais en silencieux.
  Réutilise la pile TLS+H1 existante (pas de crate lourde
  — ou `hickory-proto` pur Rust si ADR de dépendance le
  préfère). Ne contourne pas le blocage IP/SNI — le proxy
  loopback/Tor reste la réponse pour ce niveau. Contrat :
  le mode choisi est observable (`GET /api/egress` ou
  équivalent) et `no_proxy` inchangé hors mode Tor.
  Livré : trait `resolver::Resolver` injecté dans
  `h1::ResolvingConnector` sous `TlsConnector`
  (`web_roots_over`) — `SystemResolver` ≡ `TcpConnector`
  (`lookup_host`), `DohResolver` POSTe un
  `application/dns-message` (RFC 8484, format filaire
  embarqué : requêtes `A`/`AAAA`, réponses compressées par
  pointeurs, aucune dépendance) via une pile TLS+H1
  ordinaire — la résolution de l'endpoint reste système,
  pas de récursion DoH. Cache par hôte borné par TTL RR
  (clamp 10–300 s). IP littérales court-circuitées,
  candidates tentées dans l'ordre. Daemon : `--doh`/
  `EXPLORER_DOH` + `EXPLORER_DOH_TIMEOUT_MS` ; le mode est
  observable (`GET /api/egress` → `{resolver}`) et ne
  bascule jamais en cours de route.
- [x] **Étape 73. `/input` : outcome rendu à
  l'enfilement, pas au commit de l'apply** (ticket
  terrain mission 18-A — mesuré : `POST /input`
  `await`e `drain_applies_shared` → le `spawn_blocking` ;
  la réponse HTTP du clic est retenue toute la durée de
  l'apply, >30 s sur `heavy.html`, alors que l'API
  respire à ~1,4 ms pendant). L'UI perçoit une latence
  qui n'existe plus. Arbitrage : retourner l'outcome
  dès l'enfilement (202-like) et laisser `Edited`/
  `Navigated` arriver par SSE — vérifier la sémantique
  des appelants `&mut` (FFI/CLI) qui lisent l'outcome
  synchrone ; le cas « apply supplanté par epoch » doit
  rester observable.
  Livré : les drains (`drain_applies_shared` puis
  `drain_navs_shared`, ordre conservé) tournent en
  `tokio::spawn` détaché — la réponse rend l'outcome
  d'enfilement (`navigated`/`edited`/`focused` =
  **intention**) ; le commit effectif reste porté par
  `navigated`/`pipeline_error`/`frame_invalidated` SSE,
  et un apply supplanté par epoch est jeté par la garde
  (observable via les events du gagnant). `handle_input`
  `&mut` (FFI/CLI) inchangé : drain synchrone + outcome
  de commit réel (étape 64 intacte côté lib).
- [x] **Étape 74. `constructor.name` des éléments média**
  (ticket terrain mission 18-B, cosmétique) :
  `v.constructor.name` vaut `Object` au lieu de
  `HTMLVideoElement`/`HTMLAudioElement` — la
  `class_name` host de l'étape 58 n'irrigue pas
  `constructor`. Surface : `explorer-jscore` bindings média.
  Livré : `NodeHost::get("constructor")` renvoie un objet
  allégé `{name: class_name}` — `v.constructor.name` vaut
  `HTMLVideoElement`/`HTMLAudioElement`/`HTMLCanvasElement`
  (`HTMLElement` ailleurs) ; non constructible (déviation
  v1 documentée). Régression :
  `media_constructor_name_is_class_name`.
- [x] **Étape 75. Page d'erreur : purger `find` AVANT le
  relayout** (ticket terrain mission 19-D, pixel) :
  `paint_error_page` pose `e.find = None` **après**
  `relayout_dom` (paint.rs) — les `FillRect` du find stale
  de la page précédente sont cuits dans la frame d'erreur
  (bande orange observée sous le h1 du 404, disparue à la
  nav suivante). Fix = inverser l'ordre (purge d'état
  interactif avant pose). Couvrir aussi la réserve
  mesurée : les erreurs de `nav_apply`/`sub_commit`
  (chemin `Err` post-fetch) partent à `emit_error`
  **sans** page d'erreur — trou de couverture 69.
  Livré : la purge interactive (find, popup, focus, hover,
  spec_paint) précède `relayout_dom` — régression mesurée
  à **624 pixels orange cuits** avant le fix, 0 après.
  Les erreurs `Err` de `nav_apply`/`sub_commit` peignent
  la page d'erreur aussi (URL effectivement naviguée),
  sur les chemins direct et partagé — la couverture 69
  est complète.
- [x] **Étape 76. Nav échouée : entrée d'historique**
  (ticket terrain mission 19-D, sémantique) : la nav
  échouée ne pousse pas d'entrée — repro A→B→refus :
  `back` depuis la page d'erreur atterrit sur **A** (B
  sauté), `fwd` ressort l'erreur gelée dans le slot
  bfcache de B (snapshot écrasé) → B inaccessible par
  traversée. Chrome/Firefox poussent la nav échouée en
  entrée (back revient à B, forward réessaie). Arbitrage
  : pousser l'entrée avec marqueur « échec » (retry à la
  traversée) vs ne pas pousser mais préserver le slot —
  décision + gestion bfcache documentées.
  Livré : `Tab.hist` passe de `Vec<Request>` à
  `Vec<HistEntry { req, failed }>` — `paint_error_page`
  pousse l'entrée marquée `failed` (method+body préservés,
  replay POST couvert par construction). La traversée
  (`history_move`, chemins direct et partagé) retente la
  requête d'origine sur entrée échouée : succès = nav
  normale, échec = page d'erreur repeinte à la même
  position (pas de doublon d'entrée). `try_bfcache`
  ignore les entrées `failed` — jamais de page d'erreur
  gelée ; une page d'erreur courante n'est pas
  snapshotée. Repro validé : A→B→refus, `back` revient à
  B, `fwd` retente et repeint l'erreur, `back` revient
  encore à B.
- [x] **Étape 77. Daemon : `axum::serve` en échec visible
  + sortie gouvernée** (ticket terrain mission 19,
  explique la mort « silencieuse » de mission 18) : en
  vol le daemon a quitté `exit(1)` — `axum::serve` a
  renvoyé `Err` et le filtre `RUST_LOG=sync_profile` a
  avalé le `tracing::error!`. Deux fixes : (1) le message
  de sortie fatale doit survivre à tout filtre (stderr
  direct ou niveau toujours émis) ; (2) investiguer la
  cause de l'`Err` serveur (accept-loop, bind perdu ?) —
  un daemon navigateur ne meurt pas sans laisser de
  raison.
  Livré : les deux sorties fatales (`TcpListener::bind` en
  échec, `axum::serve` terminé en `Err`) écrivent la
  raison sur **stderr direct** (`eprintln!`) en plus du
  `tracing::error!` structuré — aucun filtre
  `RUST_LOG` ne peut plus avaler la cause de mort. La
  cause de l'`Err` devient observable par construction
  (`Err` io de l'accept-loop rapporté tel quel).
- [x] **Étape 78. DoH : noms d'usage spécial court-circuités**
  (ticket terrain mission 20-B) : en mode `--doh`,
  `localhost` (et la famille RFC 6761 — `.localhost`,
  `.local`, `.test`, `.invalid`, `.example`, adresses
  de documentation) est routé **par le résolveur DoH**
  comme un nom ordinaire → l'endpoint public répond
  « aucune adresse » et les fixtures/pages locales sont
  mortes en mode DoH. Chrome/Firefox gardent ces noms
  sur la résolution système quoi qu'il arrive. Les
  littéraux IP court-circuitent déjà correctement
  (`127.0.0.1` bypass). Correctif attendu : liste
  special-use dans `DohResolver` (ou le trait
  `Resolver`) → délégation système, avec test de
  régression. Comportement DoH par ailleurs validé
  terrain : `/api/egress` observable, `http://` refusé
  au démarrage, endpoint mort → page d'erreur honnête
  sans repli système silencieux.
  Livré : `is_special_use` (RFC 6761 + `.local` mDNS
  RFC 6762, casse et point final normalisés) court-
  circuite `DohResolver::resolve` **avant** cache et
  requête — délégation [`SystemResolver`]. Régression :
  `localhost` → loopback sans toucher le transport,
  huit noms spéciaux jamais requêtés, un nom ordinaire
  passe bien par `DoH`. Fixture des tests rebasée sur
  `webfix.dev` (`example.com` est désormais
  special-use).
- [x] **Étape 82. FrameView : frame affichée sans
  clip `dpr`** (prérequis validation overlay média
  71d — positionner `rect − scroll_y` sur une frame
  tronquée ne mesure rien) : le raster moteur fait
  `w_css × zoom·dpr` px physiques ; `Image.memory` /
  `RawImage` à `scale` 1.0 (défaut) l'affichaient en
  px logiques = px physiques → `dpr×` trop grands,
  `Clip.hardEdge` tronquait `1−1/dpr` (~33 % à
  dpr 1,5, ~43 % à 1,75 — invisible à dpr 1,0, d'où
  le passage à travers toute la chaîne de tests).
  Correctif : `scale: dpr` (1 px image = 1 px
  physique, netteté conservée) + diviseur d'entrée
  `zoom·dpr` → `zoom` — il n'était cohérent qu'avec
  l'affichage surdimensionné. `flutter analyze` +
  `flutter test` verts (18/18). Réception humaine :
  vérifier visuellement sur affichage dpr > 1 que la
  frame remplit le viewport et que les clics
  touchent les éléments visés.

- [x] **Étape 81. `daemonOnline` : récupération
  automatique** (mensonge visible quotidien — la
  bannière « daemon injoignable » survivait à un
  daemon revenu, SSE reconnecté en silence) :
  `_bootstrap` échoué figeait l'état offline
  jusqu'au « Réessayer » manuel. Correctif :
  `AppConfig.daemonRetryDelay` (3 s) + timer de
  re-bootstrap armé sur échec bootstrap et sur
  tout `ApiException` à `statusCode` nul (échec
  de connexion — une erreur HTTP prouve le
  daemon vivant et ne pose pas offline) ;
  annulé au premier `listTabs` réussi. Couvre
  le démarrage app-avant-daemon **et** la mort
  + renaissance mid-session. Test : récupération
  automatique sans geste (repo simulé offline→
  online, timer 10 ms). `analyze` + `test` verts.
- [x] **Étape 80. Icônes trailing de la navbar
  non déterministes** (rapport M22 : étoile,
  historique, signets, find, zoom, réglages
  absents en run mission — confirmé en capture
  foreground — mais visibles en session manuelle) :
  le repro a démenti l'hypothèse icon-font —
  **`Row` fixe overflow** : sous ~560 px les 9
  `IconButton` (48 dp mini) poussent le trailing
  hors de la frame — silencieux en release,
  bandes jaune/noir en debug, donc « absentes »
  en capture et présentes en session large.
  Correctif : `LayoutBuilder` — sous le seuil
  `metrics.navCompact` (560, tokens les deux
  variantes) la barre passe en mode compact :
  navigation + omnibox + menu « ⋮ » regroupant
  signet, historique, signets, recherche, zoom
  ±/100 %, réglages (modèle Chrome). Régression
  : widget test à 380 px — zéro overflow,
  actions joignables via le menu. Les overrides
  tokens synchrones rendent le test
  déterministe (le chargement `rootBundle`
  réel est flaky sous charge de suite).
  `analyze` + `test` verts (20/20).

- [x] **Étape 83. Cache : revalidation 304
  gzip vide + partiels tronqués jamais
  stockés** (`explorer-net`) — déclencheur
  `site_smoke` : wikipedia en échec
  **déterministe** via daemon (5/5,
  `gzip: unexpected end of file`) alors que
  le cli passait — la frontière était le
  cache disque. (a) Un 304 porte
  `Content-Encoding: gzip` (en-têtes de la
  ressource) et un corps **vide** :
  `DecodingTransport` — couche interne de la
  pile `Cache → Decoding → h1` — l'échouait
  avant que le cache ne serve l'entrée
  périmée ; `decode_response` saute les
  statuts sans corps (1xx/204/304) et la
  fusion 304 ne propage plus
  `content-encoding` vers l'entrée (le corps
  stocké est décodé). (b) Un flux compressé
  tronqué (h2 coupé tôt) rendait une erreur
  sèche : partiel décodé conservé (Chrome/
  Firefox pareil) mais marqué `no-store` —
  le cache disque ne doit jamais bénir un
  corps incomplet. Repro verrouillé :
  `stale_304_through_decoding_layer` sur la
  chaîne réelle, 4 tests décodeur en plus.
  Wikipedia 5/5 via daemon sur profil
  empoisonné après fix.

- [x] **Étape 84. `back` pendant navigation
  en vol** (`explorer-core`) — déclencheur
  `site_smoke` : « précédent » pendant la
  vague de sous-ressources répondait 502
  `EmptyDocument` (iana ~10 s de fenêtre,
  github ~100 s). L'entrée hist n'était
  poussée qu'en `sub_commit`, après
  `sub.run()` — `hist_target` comptait alors
  depuis `hist_idx` (page quittée) et
  échouait. `hist_pending` marque la fenêtre
  `nav_apply`→commit : la position logique
  devient `hist_idx + 1` pour
  `hist_target`/`history_move`/`can_back`/
  `can_forward`/`try_forward_nav`. La page
  en vol n'est pas gelée (pas de slot —
  empoisonnait le snapshot précédent) ;
  `restore` bump `nav_epoch` → la nav en
  vol est supplantée, son commit tardif
  jeté. Sémantique Chrome : back annule et
  restaure (bfcache ou rejeu). Terrain :
  clic→iana, back en vol = 3 ms, A restauré.
  2 tests de régression.

- [x] **Étape 85. Gestes chrome — test
  humain** (`app/`) : les raccourcis
  attendus au premier usage, liés au
  niveau **page** (`CallbackShortcuts` —
  avant, zoom/find n'existaient que sous
  focus frame ; désormais actifs aussi
  depuis omnibox/barre find). Clavier :
  `Ctrl+L` (focus+sélection omnibox),
  `Ctrl+T`/`Ctrl+W`, `Ctrl+R`/`F5`,
  `Alt+←`/`→` + touches `browserBack`/
  `Forward`, `Ctrl+Tab`/`Ctrl+Shift+Tab`,
  `Ctrl+1..8`/`Ctrl+9`, zoom `Ctrl`+`+`/
  `−`/`0`, `Ctrl+F`, `Échap`. Souris :
  `X1`/`X2` = précédent/suivant,
  clic-molette sur lien = **nouvel onglet
  en arrière-plan** (`openInNewTab`),
  clic-molette sur onglet = fermer. Le
  relais moteur n'envoie que des frappes
  nues (un combo modifié n'est plus tapé
  dans le champ — `Ctrl+V` envoyait `v`).
  Suite : garde AltGr (`altRight` exempté —
  Windows le synthétise `Ctrl+Alt`) et
  relais du `character` produit plutôt que
  `keyLabel` — `@€{}[]~`, shift, accents
  par touches mortes atteignent le champ ;
  repli `keyLabel` pour les touches
  nommées. Tests contrôleur : cyclage,
  index, arrière-plan, échec de création.
  `analyze` + `test` verts (28/28).

- [ ] **Veille — mort silencieuse du
  daemon / `RADAR_PRE_LEAK_64`** : le
  22/10, un daemon de sondage a disparu
  en plein `site_smoke` sur wikipedia
  (page 10 Mo, tempête de `mousemove`
  dispatchant des events DOM au moteur
  JS) — **jamais reproduit** : 630 sondes
  + cycle lobste→clic→back→wikipedia en
  repro manuel, RSS plat à ~330 Mo, aucun
  événement crash Windows (pas d'abort
  1000 — la piste restante est une sortie
  silencieuse ou un OOM). **Exonéré pour
  la 2e disparition** : daemon relancé
  (PID 13176) mort en même temps qu'un
  **crash du poste de dev** (reboot) —
  rien ne survit à un crash machine,
  logs vides et aucun event 1000
  attendus : disparition externe, pas
  interne. Durcissement livré au
  passage : daemons relancés via WMI
  (`Win32_Process.Create`, parent = hôte
  WMI hors job de session — survivent
  aux crashes de session/agent, pas aux
  crashes machine). Reste ouvert pour la
  1re disparition (sondage wikipedia,
  pwsh encore vivant à ce moment) et
  l'indice résiduel :
  `RADAR_PRE_LEAK_64` du 19/10 avait
  flagué `explorer-daemon` comme suspect
  de **fuite mémoire** (peut dater
  d'avant les fix 55h/83/84). Protocole
  proposé
  pour trancher : RSS-watch long —
  script qui polle le RSS du daemon
  pendant un cycle répété
  (nav grosse page → sondes → back →
  DELETE → nouvel onglet), avec stderr
  capturé ; si le RSS croît sans borne
  ou si le process meurt, on a la
  repro. Le script `site_smoke` capture
  déjà stderr et détecte la mort du
  daemon (étape « robustesse » de la
  phase fonctionnelle). *Critère de
  clôture : repro avec RSS croissant OU
  exonération sur fenêtre longue.*
  Instrument livré :
  `scripts/daemon_watch.ps1` — cycles
  répétés sur daemon dédié + CSV RSS.
  **Attribution probable (3e mort)** :
  le `cmd.exe` parent WMI ouvrait une
  fenêtre console **visible** — sa
  fermeture manuelle tue tout l'arbre
  sans stderr ni event (signature
  `^C` retrouvée dans stderr = event
  console propagé). Durcissement :
  `scripts/spawn_daemon.ps1` — spawn
  `SW_HIDE` + watchdog `cmd /v:on`
  qui écrit `EXIT:!ERRORLEVEL!` dans
  `daemon-<port>.exit.log` (kill
  externe → `-1`, crash → code
  exception, absence = kill d'arbre).
  La prochaine disparition laissera
  une signature — ou n'arrivera plus.

- [x] **Étape 86. Resync `fieldFocused` —
  outcome `unfocused`** (core + api + ffi
  + `app/`) : désync trouvée en relisant
  la chaîne de saisie — l'app posait
  `fieldFocused` au clic et ne le
  lâchait **jamais** : après
  `Enter`/`Escape` (focus effacé par le
  moteur) ou la mort du nœud sous les
  mains d'un handler, chaque frappe
  repartait en `ignored` muet — champ
  qui semble focalisé, texte avalé.
  Nouveau `InputOutcome::Unfocused`
  (« aucun champ focalisé à l'arrivée
  ou à l'issue ») — distinct
  d'`Ignored`, qui reste réservé au
  `keydown` consommé par
  `preventDefault` (le champ **reste**
  focalisé). API `"unfocused"`, FFI
  code `6`. App : `key()` lit son
  résultat, `navigated` et `selectTab`
  remettent le flag à zéro. Tests :
  `enter_escape_unfocus_reports_
  unfocused` + assertions existantes
  rebasées + contrôleur (resync +
  reset). Gate entier vert.

- [x] **Étape 87. Caret du champ
  focalisé** (core) — constat M-H1 #1 :
  un champ focalisé n'affichait rien
  avant la 1re frappe. Overlay de la
  display list façon find/popup, coords
  **frame** (`scroll_y` retiré) : barre
  1 px à l'encre du champ, fin du
  dernier fragment posé (`x+width+
  trail_w`) ou origine du contenu si
  valeur vide (le `placeholder` peint
  n'avance pas le point d'insertion).
  `PaintPayload.focused` = snapshot
  copié à la sortie (jamais ré-écrit
  sur `Tab`) ; `repaint_payload`
  reçoit `dom_hint` (le `dom` sorti
  pendant le relayout) — sinon le
  caret tombait à `dom: None` après
  chaque frappe. Corrigé au passage :
  focus par clic / perte par clic
  hors champ ou `Escape` repeignent +
  invalident — avant, rien ne
  déclenchait de peinture au focus.
  MVP : pas de clignotement, pas de
  point interne (`edit_field` n'insère
  qu'en fin). Test :
  `caret_paints_moves_and_clears_
  with_focus` + test d'événements
  rebasé (`Escape` avant comparaison
  PNG). Écart documenté : find/popup
  restent en coords document sous
  scroll (latence pré-existante).

- [x] **Étape 88. Quick wins M-H1
  (#2 #3 #4)** (app + core) —
  1. snackbar `ApiException` :
  `lastError` purgé au bootstrap
  réussi + snackbar visible masqué
  à l'effacement — l'erreur
  « injoignable » survivait à la
  reconnexion et masquait la
  récidive identique (select sans
  changement). 2. `Ctrl`+molette →
  `zoomProvider` (avant : dy
  toujours scrollé). 3. Home :
  icône barre large + entrée ⋮
  compact → `about:blank`, servi
  côté moteur par interception
  dans `NavJob::run` avant fetch
  (squelette tree-builder commis ;
  autres `about:*` → page
  d'erreur). Omnibox : `about:`
  non préfixé.

- [x] **Étape 89. Flux de frames
  app borné** (app) — constat M-H1
  #5 (`explorer_app` +80 Mio/min) :
  pulls coalescés « dernière
  gagne » (1 en vol + 1 en attente
  par onglet — avant : un fetch+
  decode par `frame_invalidated`,
  rafales non bornées) + chemin
  REST décodé une fois en
  `ui.Image`/`rawFrames` (avant :
  `Image.memory` par frame = clé
  de cache par identité + copie
  externe par rebuild). Test :
  gate FakeRepo, 3 invalidations
  en vol → 2 fetches.

- [x] **Étape 90. Rendu de commit
  nav détaché** (core) — constat M-H1
  #6 : `sub_commit` tenait
  `sess.lock()` pendant la cascade
  + layout + raster du document
  (famine ~85 s sur terrain, API
  gelée : `/status`, `/frame`,
  input, autres onglets). Livré :
  `sub_commit` = `sub_commit_head`
  (verrou : epoch, CSS, fontes, JS,
  drains) → rendu hors verrou en
  `spawn_blocking` sur payload
  (`insert_images_payload` +
  `relayout_dom_payload`,
  mécanique `paint_detached` de
  55h : `take_nav_paint`/`Busy`/
  `commit_nav_paint`/`rearm_nav_paint`)
  → `sub_commit_tail` (verrou :
  historique, `Navigated`, fragment,
  `mark_hist_ok`). Chemin `&mut self`
  (CLI/FFI/tests) inchangé. Réserve
  honnête : la **durée** de la cascade
  est inchangée (perf = chantier
  PIPE) — l'API reste serviable
  pendant ; les applies enfilés par
  `pump_net` drainent encore sous
  verrou (livraisons bornées).

- [x] **Étape 91. Entrées différées
  pendant la peinture détachée**
  (core) — triage jonglerie focus
  google (M-H1) : Étape 90 a rendu
  la fenêtre `paint_detached`
  possible — la tâche d'apply
  détient `dom`/`tree`/`frame`
  (`take_paint`), donc toute entrée
  pendant la fenêtre se résolvait
  sur un onglet sans DOM : `unfocused`
  mensonger (le focus vit sur `Tab`)
  et un clic y tuait le chemin de
  focus pour de bon. Livré : file
  `deferred_inputs` sur `Tab`
  (bornée 32, `Scroll` additionnés
  et `MouseMove` dernière l'emporte
  — même coalescence client
  ADR-0003) ; `handle_input_now`
  renvoie `Ignored` quand la
  fenêtre est ouverte **ou la file
  non vide** (FIFO strict — une
  entrée directe ne dépasse jamais
  une différée, sinon texte
  interverti, mesuré terrain :
  « explorer » tapé rendait
  « erx ») ; drain à la
  réinsertion de payload (apply
  synchrone et détachée, commit
  nav) ; purge à la nav/erreur/
  rearm. Terrain : les 6 touches
  différées d'une frappe google
  rejouent à la livraison.

- [x] **Étape 92. Focus ré-ancré par
  identité quand le chemin meurt**
  (core) — suite du triage jonglerie
  google : après Étape 91 les touches
  rejouaient mais tombaient encore
  — le listener `input` (suggest)
  mute le DOM, l'apply re-parse et
  le chemin `(tag, index)` du champ
  focalisé meurt ou résout sur un
  voisin (un sibling de même tag
  inséré avant). Livré :
  `focused_key = (attr, value)` —
  `id` puis `name` — capturée à
  `set_focus` ; `focused_node`
  vérifie l'ancre sur le nœud
  résolu et, mort ou usurpé,
  retrouve l'élément par
  `find_by_attr` puis ré-écrit le
  chemin — le focus survit au
  déplacement, comme un vrai
  navigateur ; l'ancre voyage dans
  le `PageSnapshot` (bfcache).
  Terrain : « explorer » tapé dans
  la recherche google rend
  « explorer| » exact, Enter
  soumet le formulaire. Test :
  `focus_reattaches_by_identity`
  (chemin mort, chemin usurpé,
  élément réellement supprimé).

- [x] **Étape 93. Reload POST : verrou
  `resubmit_confirm`** (core, api,
  app) — le endpoint REST `…/reload`
  court-circuitait `Session::reload`
  en reconstruisant un `GET` nu :
  tout reload applicatif était un
  GET vide, y compris sur une page
  POST avec `last_req` présent
  (constat M-H1). Livré :
  `Session::reload_request` expose
  la requête de rejeu — `last_req`
  (méthode + corps), la requête
  `failed` de l'entrée courante
  (page d'erreur : retenter la
  requête tombée, plus de GET vide),
  sinon `GET` top-level ; le
  endpoint la rejoue, mais une
  méthode ≠ `GET`/`HEAD` exige
  `{"confirm":true}` — sinon statut
  typé `resubmit_confirm`, zéro
  resoumission silencieuse. L'app
  affiche le dialogue « Renvoyer le
  formulaire ? » (Renvoyer →
  `confirm`, Annuler → rien) ; FFI
  in-process rejoue directement.
  Tests : `reload_request_on_failed_post_replays_request`,
  `post_reload_requires_confirm`,
  contrôleur `resubmit_confirm`.
- [x] **Étape 94. Media queries sur le
  viewport réel** (demande PIPE→NAV,
  journal) — `styles_payload`
  évaluait la cascade avec
  `StyleEnv::default()` = 1024×768
  fixe quel que soit le viewport
  session : à 800px, `max-width:1007`
  restait faux (BBC `span 8` perdu).
  Livré : `styles_payload` reçoit
  `ctx.viewport` → `StyleEnv` réel
  (reflow complet + relayout
  incrémental + helper test) ;
  `SubJob.viewport` recopié à la
  construction → `@font-face`/`media=`
  évalués sur le même env ;
  `cli layout` aligné sur son 800px.
  Test : `media_query_uses_session_viewport`
  (raster : le bloc `max-width:900`
  masqué à 800px).
- [x] **Étape 95. Attribution du pic
  mémoire Wikipedia** (veille M-H1)
  — le RSS ~1,5–1,9 Go de la nav
  « Rust » mesuré puis attribué :
  allocateur compteur (live = 916 Mio
  post-commit, ≠ transitoire),
  bissection par drop champ à champ —
  `tree` ~850 Mio, `prev_styles`
  ~256 Mio : la cascade duplique les
  `--*` héritées dans chaque
  `ComputedStyle` (3,26M+1,89M
  entrées). Livré : trait
  `JsEngine::heap_stats` (additif,
  défaut `None`, `HeapStats`
  réexporté), commande moteur +
  adaptateur `JscoreEngine`, sonde
  `tab_mem_inventory` `#[ignore]`
  (inventaire par champ + bissection
  RSS in-process). Correctif
  `ComputedStyle` = demande PIPE
  déposée au journal (partage `Rc`
  des vars héritées).
- [x] **Étape 96. Sous-ressources CSS :
  `<link media>` + `@import` + borne
  revue** (demande triage `site_smoke`
  PIPE→NAV, journal) —
  `plan_subresources` évalue `media=`
  contre `viewport_env(self.viewport)`
  **avant** la borne : hors-media →
  ni fetchée ni comptée ; `max_sheets`
  8→32 (corpus : GitHub ~22
  effectives, MDN ~20).
  `SubJob::fetch_imports` résout la
  chaîne `@import` : re-parse des
  feuilles reçues, imports expansés
  récursivement et préfixés au slot
  (ordre cascade) ; dédup URL +
  `MAX_IMPORT_DEPTH` + `max_imports`
  bornent boucles/dégénérescence ;
  avant `fetch_fonts` → `@font-face`
  des importées éligible.
  `media_attr_matches` = shim
  `@media` synthétique via
  `parse_stylesheet` (aucune API PIPE
  nouvelle ; inclassable → conservé).
  Tests : filtre media + non-fetch,
  tolérance `(((`, borne post-filtre,
  expansion + media d'import,
  borne/dédup. Solde aussi la
  demande R-22 (`@import` lobste.rs).
  Suite terrain : dédup des `<link>`
  par URL résolue (GitHub répète
  ~7 hrefs dans ses blocs de
  liens générés).

- [x] **Étape 98. `background-image`
  HTTP fetchés** — les `url(http…)`
  de fond n'étaient jamais fetchés
  (`collect_data_images` ne couvrait
  que `data:`) : logo lobste.rs,
  icônes CSS du corpus.
  `SubJob::fetch_backgrounds` scanne
  `background`/`background-image`
  des feuilles (imports expansées
  comprises, `@media` imbriqués
  filtrés sur l'env viewport) +
  `<style>` inline, résout contre
  la base de **sa** feuille, dédupe
  par URL résolue, fetches sous
  `max_images` partagé avec `<img>`.
  `bg_aliases` conserve le lien
  texte-brut→résolue perdu à la
  cascade ; `insert_fetched_backgrounds`
  (relayout, les deux chemins sync
  et détaché) retrouve le `NodeId`
  par la valeur calculée, décode
  dédupliqué → `images.backgrounds`
  (canal existant). Tests : fetch
  + base feuille + pixels rouges
  peints, 404 silencieux, borne
  partagée (32/40 fetchés). Limite
  documentée : `style=` inline non
  collecté (pas de DOM dans `SubJob`).

- [x] **Étape 97. Moitié NAV du thème
  sombre** (débloquée par R-59 —
  `StyleEnv.color_scheme` livré) :
  `SessionConfig.color_scheme` →
  `Session` → `viewport_env` →
  `SubJob`/`ApplyCtx` — media queries
  `prefers-color-scheme`, filtre
  `media=` des sous-ressources,
  défauts UA sombres de la cascade.
  `set_color_scheme` bascule en vol
  (re-render re-cascade). Canaux :
  REST `POST …/color-scheme`,
  daemon `EXPLORER_COLOR_SCHEME`,
  FFI `explorer_set_color_scheme`
  (additif), app `AppSettings.theme`
  (`system` résolu brightness OS +
  suivi des changements). Validé :
  lobste.rs simulé fetch
  `dark-normal`/filtre `light-normal`
  en env dark ; bascule runtime peint
  `prefers-color-scheme:dark`.
  **Gap révélé puis comblé** : `not`
  imbriqué MQ4 (`(cond) and (not (c2))`
  des `@import` lobste.rs) — journal →
  PIPE, livré R-64 ; sonde réelle dark
  à rejouer pour voir `dark-normal`
  fetchée en vol.

- [ ] **Écarts terrain — vol humain M-H1
  (22/10)** : premier run humain sur
  daemon `99782ac`. Validé : AZERTY
  complet (`@\|[]{}#é"(èç)=^`), submit
  Enter+clic → résultats DDG réels,
  back/forward, historique, zoom
  manuel, reconnexion auto. Chaque
  écart ci-dessous est une mesure
  terrain — promotion en Étape/JS-C au
  triage :
  - ~~**caret absent**~~ → **résolu,
    Étape 87** : overlay display list
    (coords frame), barre 1 px en fin
    de texte ou à l'origine du contenu
    si champ vide.
  - ~~**snackbar `ApiException`
    persistant**~~ → **résolu, Étape
    88** : `lastError` purgé au
    bootstrap réussi + snackbar
    visible masqué à l'effacement.
  - ~~**`Ctrl`+molette sans zoom**~~
    → **résolu, Étape 88** :
    `isControlPressed` aiguille vers
    `zoomProvider`.
  - ~~**badge/menu Home absent**~~
    → **résolu, Étape 88** : icône
    `⌂` + entrée ⋮ → `about:blank`,
    servi sans fetch par le moteur
    (`NavJob::run`).
  - ~~**RSS `explorer_app` 3,7→4,5 Go
    en ~10 min**~~ → **résolu, mesuré
    Étape 89+93** : pulls coalescés +
    `ui.Image` décodée une fois —
    veille 10 min pilotant l'onglet
    de l'app (4 sites réels, tempêtes
    mousemove+scroll, back) : app
    **plate 288→290 Mio** (`priv`
    324→326), daemon borné
    ~370-450 Mio — ~~pic transitoire
    ~1,9 Go + nav >120 s~~ →
    **attribué Étape 95** : ~1,1 Go de
    rétention commise dans le `Tab`
    (`tree` ~850 Mio + `prev_styles`
    ~256 Mio) par copie plate des
    `--*` héritées dans chaque
    `ComputedStyle` — ~200 vars
    `:root` × 19k nœuds. Correctif
    côté cascade (partage `Rc`) =
    demande PIPE déposée au journal.
  - **daemon injoignable sous charge
    machine, retour seul** : pas un
    crash (reconnexion auto OK) —
    famine `sess.lock()` ~85 s sur
    github. **Atténué, Étape 90** :
    le rendu de commit nav tourne
    hors verrou — l'API reste
    serviable pendant la cascade.
    La durée reste au chantier
    perf cache-cascade (PIPE).
  - ~~**google « oignon » → page stub
    « Cliquez ici si… pas été
    redirigé » + `exception js :
    Error: b`**~~ → **résolu, chaîne
    JS-C46→C52 + Étapes 91-92** :
    quantifieurs lazy (JS-C48), entités
    miroir (JS-C46), pile d'appel
    réelle (JS-C47), `eval_hook`
    (JS-C50), ordre LHS/RHS
    (JS-C52 — la cause racine du
    `vG is not a function`), entrées
    différées pendant paint détachée
    (91) + focus ré-ancré par identité
    (92). Vol de bout en bout :
    consent → homepage → « explorer »
    8/8 ordonné → Enter → navigated.
  - ~~**reload sur page résultats POST
    → formulaire vide**~~ → **résolu,
    Étape 93** : l'endpoint REST
    court-circuitait `Session::reload`
    (GET nu) ; `reload_request`
    rejoue la requête d'origine,
    méthode non sûre gated par
    `{"confirm":true}` — dialogue
    « Renvoyer le formulaire ? »
    côté app.

## Piste JS (JS-C26+) — spécialiste `explorer-jscore`

Ordre dicté par les besoins de la piste produit d'abord, puis
couverture **strictement réactive** (un bug site → un cas
`corpus.rs`/test262 → un fix — fini la couverture pour la
couverture).

Priorité terrain (lead, mission 14/16) : **JS-C45** en tête
(`onclick=` mort = casse-site réel mesuré), puis JS-C35
(`document.write`), JS-C44 (bare specifiers), JS-C29
(`Worker`) — le reste de la série reste réactif aux sites.

### J-A — APIs navigateur attendues par la session

- [x] **JS-C26. `MutationObserver`** : queue de mutations sur le
  `Dom` miroir (`childList`/`attributes`/`characterData`,
  `subtree`, `attributeFilter`, `oldValue`), delivery en
  microtask checkpoint, `disconnect`/`takeRecords`. Débloque
  l'étape 52.
- [x] **JS-C27. Traverse proxy net/storage** : `XhrTransport`,
  `install_storage`, `CookieJar` traversent le canal
  `DilloEngine` (`drain_net_requests`/`deliver_net_response`/
  `set_storage`/`set_cookie_jar`) — prérequis interne de
  l'étape 51.
- [x] **JS-C28. `<canvas>` 2D surface** : `getContext('2d')` +
  `CanvasRenderingContext2D` enregistrant une **op-list**
  (`CanvasOpRecord` miroir → `CanvasTable` R-11, snapshot
  `canvas_ops()` cumulé). Couverture = MVP R-11 : rects,
  chemins, fill/stroke/clip, transforms, alpha, lineWidth,
  save/restore, couleurs ; `width=`/`height=` effacent
  (spec) ; APIs hors MVP (`drawImage`, `fillText`,
  gradients, `getImageData`) absorbées en no-op/retour
  plausible — jamais de `TypeError` au milieu d'une
  séquence. Contexte raciné GC, borne `canvas_op_limit`,
  dessin = mutation de rendu → relayout ciblé du sous-arbre.
  Raster = PIPE **R-11** (livré).
- [x] **JS-C29. `Worker` bornés** : Engine par thread (patron
  `JscoreEngine`), `postMessage` structuredClone fil (livré),
  `onmessage`/`onerror`, `importScripts` minimal,
  `terminate`/`close()`, pool borné (`max_workers`,
  `queue_len`, budget ops/worker hérité). Documenté : pas de
  `SharedArrayBuffer` inter-thread réel.

### J-B — Couverture langue par besoin (réactive)

**Règle du palier — aucune de ces étapes ne s'ouvre sans un
déclencheur documenté** : échec `site_smoke`, bug d'un site
réel, ou besoin d'une étape produit. Le % de couverture n'est
jamais un déclencheur (état des lieux §2 — limites des
mesures).

**Déclencheurs accumulés** : lobste.rs — `TypeError: value is
not a function` ×4 (mission 5, API manquante à identifier) ;
example.com — `TypeError` sur `s.js` (injection partielle) ;
github.com — bundle > plafonds (JS-C41 livré le cadre).

- [ ] **JS-C30. Extension corpus test262** : extraire les
  répertoires non couverts (`builtins/` complet,
  `language/statements|expressions`) → baseline mesurée, skips
  whitelistés, remontée par échecs groupés.
  *Déclencheur : aussi l'élargissement honnête du chiffre
  test262 publié — à faire quand le reporting l'exige.*
- [ ] **JS-C31. `Temporal` complet** : `ZonedDateTime`
  (arbitrage fuseaux), `round`/`smallestUnit`, calendriers.
  *Déclencheur : site réel.*
- [ ] **JS-C32. `Intl` multi-locale** : arbitrage CLDR
  (`icu4x` pur Rust — ADR dédié, dépendance lourde).
  *Déclencheur : site réel + ADR de dépendance.*
- [ ] **JS-C33. Décorateurs ES2026** — *déclencheur : site
  réel.*
- [ ] **JS-C34. Générateurs lazy + `await` pending** : refacto
  structurel, **uniquement si des sites réels bloquent**
  (l'eager documenté tient le corpus actuel). **Rendre le
  déclencheur observable** : un fixture minimal volontairement
  en échec (plus petit programme démontrant la déviation,
  `#[ignore]` documenté) — sinon la panne silencieuse sur un
  site n'est pas attribuable à cet écart et le trigger ne
  peut pas tirer.
  Déclencheur rendu observable (livrable partiel) :
  `c34_generator_lazy_spec_fixture` et
  `c34_await_pending_spec_fixture` dans `tests/lang.rs` —
  encodent le comportement spec, échouent aujourd'hui
  (`"body,call"` vs `"call"` ; `"unset"` vs `"7"`),
  `#[ignore]` documentés. La case reste ouverte : la refacto
  lazy/pending n'est pas engagée, en attente du site réel.
- [x] **JS-C35. `document.write` complet + insertion dynamique**
  (miroir) — **livré** : `write`/`writeln` variadiques
  (concat de tous les args, `\n` pour `writeln`),
  `document.open()`/`close()` et le flag `Dom.doc_open` —
  ouvert pendant le chargement, fermé par `close()` ou la
  distribution de `DOMContentLoaded`/`load` ; `write` sur
  document fermé = `open()` implicite (contenu effacé
  d'abord — spec `document open steps`). Correctif en
  route : `Dom::by_id` n'accepte que les nœuds rattachés —
  `remove`/`open` ne ressortent plus par `getElementById`.
  Écart : `<script>` écrits inertes dans le miroir —
  l'ordonnancement revient au scheduler amont (étape 50),
  qui les découvre à la re-sérialisation.
  *Déclencheur : insertion dynamique constatée sur sites
  réels.*

### J-C — Mesure, perf, robustesse

- [x] **JS-C36. Acid3 subset JS mesuré** (harnais informatif) :
  `tests/conformance/acid3-js-subset.js` extrait les 16
  sous-tests ECMAScript purs (81-96) de `wpt/acid/acid3/test.html`
  (corps verbatim, DOM/CSS/HTTP exclus) ; `tests/acid3.rs`
  rapporte le score et cliquette la régression. Mesure
  initiale **16/16** — les 3 écarts trouvés sont fixés dans le
  même commit : `-0` sans signe dans `toFixed`/`toExponential`/
  `toPrecision`, backref `\N` vers un groupe pas encore fermé =
  match vide (spec 21.2.2.9), nom de `FunctionExpression`
  ReadOnly sloppy-silencieux (Binding.sloppy_silent).
- [x] **JS-C37. Perf + borne mémoire heap** : compteurs d'ops,
  seuils GC, **borne mémoire par contexte** (exigence ADR-0019),
  coût `serialize_document` instrumenté.
  Livré : `Heap` compte les octets estimés (`cell_bytes` à
  l'alloc, recal exact au sweep — croissance des props incluse),
  `Engine::set_mem_limit` borne par contexte avec kill doux
  (`RangeError` au `tick` suivant, `maybe_collect` offre un
  sweep avant de re-tuer), `set_gc_threshold` déclenche le GC
  aux frontières de jobs (`run_microtasks`), `heap_stats()` +
  `ops_used()` exposent les compteurs, `Dom::serialize_stats`
  compte appels/octets de `serialize_document`. Tests `c37_*`
  + `serialize_stats_counts_calls_and_bytes`.
- [x] **JS-C38. Fuzz lexer/parser `explorer-jscore`** : cible
  cargo-fuzz (extension de `fuzz/` — périmètre équipe).
  Livré : `fuzz/fuzz_targets/js.rs` (`cargo fuzz run js`)
  alimente `eval`/`eval_module`/`module_specifiers` en octets
  arbitraires, budgets serrés (50 K ops, 32 Mio heap) —
  contrat « dégrader, jamais paniquer » sous contenu hostile.
  Smoke déterministe hors nightly : `c38_fuzz_smoke_no_panic`
  (128 sources hostiles générées par LCG).
- [x] **JS-C40. Découpage interne `explorer-jscore`** (audit externe) :
  `builtins.rs` 269 Ko / `interp.rs` 222 Ko — extraire par
  domaine (builtins → `builtins/{object,array,string,…}.rs`,
  interp → dispatch + groupes d'opcodes). Pur reformatage
  mécanique, tests intacts — le tree-walker reste ; cette étape
  ne change **pas** la sémantique, elle rend le code revoyable
  à plusieurs. Écarts sémantiques assumés (générateurs eager,
  `await` pending) = JS-C34, déclenché site réel — pas ici.
  Livré : `builtins.rs` (7 528 lignes) → `builtins/` 20 fichiers
  par domaine (`mod.rs` garde l'installation et les helpers
  `arg`/`ctor`/`c_id` ; `object.rs`, `array.rs`, `string.rs`,
  `date.rs`, `url.rs`, `clone.rs`… ≤ 1 047 lignes) ; `interp.rs`
  (5 718 lignes) → `interp/` 4 fichiers (`mod.rs` : état Engine,
  envs, classes, accès props, coercions, appels, hoisting ;
  `exec.rs` : exécution de statements ; `eval.rs` : évaluation
  d'expressions et opérateurs ; `ops.rs` : fonctions libres
  `strict_eq`/`typeof_str`/`expr_to_pat`…). Méthodes d'`impl
  Engine` réparties passées `pub(crate)` — visibilité API externe
  inchangée, sémantique strictement identique (251 tests `lang`
  + test262 intacts au vert).
- [x] **JS-C39. Sync DOM incrémentale — instrumenter puis
  drainer les records** (dette ADR-0013 budgétisée tôt ; le
  pont `dom_host.rs` = 69 Ko, son empreinte se mesure ici).
  **Point de décision, pas juste mesure** — trois issues
  possibles, critères explicites :
  1. *mesure* : compteurs ms/octets par synchro `sync_dom`
     dans la session (visibles au profilage, étape 57) ;
  2. *si le coût domine* (synchro > ~10 % d'un frame budget
     ou churn DoS démontré sur un cas réel) : `JsEngine`
     gagne `drain_dom_mutations()` **additif** réutilisant
     les `MutationRecord` (mécanique JS-C26) — diff d'arbre
     au lieu du HTML complet. **Le critère exige un
     instrument** : `benches/pipeline.rs` ne benche que
     `navigate`/`scroll_repaint` globaux — la branche 1
     inclut donc un bench `mirror_sync` dédié (ou les
     compteurs suffisent s'ils donnent ms/octets par synchro
     isolée). « DoS démontré » reste un critère par
     observation, sans instrument ;
  3. *si le problème est le modèle* (divergences miroir ↔
     `explorer-dom` récurrentes, état non sérialisable,
     fidélité): escalade = **ADR amendant ADR-0013** (partage
     réel ou diff natif `explorer-dom`-à-`explorer-dom`) —
     décision tranchée à l'écrit, pas calculée au fil.
  Nouveau point de cohérence → tests croisés miroir ↔
  `explorer-dom` exigés (règle de tri « corrige le côté
  fautif »).
  Livré (branche 1, mesure) : `Dom::serialize_stats`
  (`SerializeStats { calls, bytes, micros }`) instrumente
  `serialize_document` côté moteur ; `JsEngine::dom_sync_stats`
  (additif, défaut `None`) expose `DomSyncStats
  { syncs, bytes, micros }` à la session — octets/micros par
  synchro mesurables sans bench dédié. Tests croisés :
  `dom_sync.rs` re-parse le HTML du miroir via
  `explorer_html::TreeBuilder` et vérifie la fidélité des
  mutations dans `explorer-dom`.
  **Décision écrite** : statu quo `serialize_document` — la
  synchro n'a jamais été mesurée dominante ; les compteurs
  existent désormais pour détecter le seuil (~10 % frame
  budget) ou un churn DoS → escalader alors vers
  `drain_dom_mutations` (records JS-C26 déjà produits).
- [x] **JS-C41. Récursion bornée — stack overflow site réel**
  (bug terrain, repro : github.com) : `explorer-jscore` fait un
  stack overflow sur le bundle réel de github (`dump`/`layout`
  OK, `shot` 💥 ; `EXPLORER_NO_JS=1` → erreur propre →
  confirme le moteur JS). **Violation de la règle « jamais
  crash »** — prioritaire dans J-C : profondeur
  d'appel/récursion bornée par budget → `JsError` typée,
  jamais d'overflow natif. Repro minimal : reproduire la
  pile en extrayant le pattern récursif du bundle.
- [x] **JS-C42. `Drop` itératif de l'AST** (résidu mesuré de
  JS-C41, repro `repros/` : `x=1+1+…`/`a&&a&&…` à ~15 000
  opérandes via `eval` **et** `module_specifiers`) : le parse
  est borné (`input too deeply nested` propre) mais la
  **destruction récursive** de la chaîne `Box<Expr>` déborde
  — `maybe_grow` ne couvre pas `Drop`. Livré (commit
  `5f19372`, rapatrié par JS-C48) : `Expr::take_children`
  + `impl Drop for Expr` — file explicite, plus de descente
  récursive native ; test
  `c41_deep_operand_chain_drops_iteratively` (15 K maillons,
  `eval` et `module_specifiers`).
- [x] **JS-C43. Parseur regex — motifs courants rejetés**
  (déclencheur terrain mission 6 : githubstatus.com,
  github.com — scripts réels morts au parse, erreurs typées,
  zéro crash) : `(.*?)` lazy → `quantifier without atom` ;
  `[.*+?^${}()|[\]\\]` → `Invalid character class` ;
  `[\0-\x1f]` → `empty class range` ; `\uD800` →
  `Invalid codepoint`. Trois motifs étaient déjà colmatés
  par le rapatriement JS-C48 (lazy `*?`, bornes de plage
  échappées, surrogates clampés) ; le dernier trou — les
  littéraux `[`/`&`/`~` en classe, qui ont une syntaxe de
  classe côté backend — est échappé via `class_char`
  (`\x{…}`), y compris en borne haute brute de plage.
  Test : `regexp_class_syntax_chars_escaped`.
- [x] **JS-C44. Specificateurs « bare » non résolus**
  (déclencheur terrain mission 6 : `cannot load module
  'react'`) : arbitrage = **import maps minimales** —
  `Engine::set_import_map` lit la clé `imports` du JSON
  (`scopes` non lu, valeurs non-string sautées, map
  invalide ignorée — tolérance), `load_source` réécrit les
  bare specifiers (exact puis préfixe le plus long en `/`,
  reste conservé) avant le `ModuleLoader` ; première clé
  gagnante sur fusion. Trait `JsEngine::set_import_map`
  (défaut noop) — la session pousse le contenu des
  `<script type="importmap">` avant le premier module ;
  `census` le fait déjà. Bare hors map = erreur typée
  inchangée. Les `ReferenceError` `hljs`/`$`/`Event`/`SP`
  observés sont des **conséquences** (scripts amont morts),
  pas des tickets.
- [x] **JS-C45. Attributs `on*` jamais compilés**
  (déclencheur terrain mission 10 — fixture `jsmut.html` :
  `onclick="…"` mort, seul `addEventListener` tire ;
  `fire_listeners` ne lit que `listeners` + prop
  `el.onclick`). Le web réel est couvert de handlers inline
  (`onclick`, `onload`, `onsubmit`…) — HTML spec : à la
  construction du miroir, chaque attribut `on*` est compilé
  en event handler (avec le bon `this`/scope). **Deux trous
  colmatés** : `fire_listeners` compile l'attribut
  paresseusement au dispatch (cache `(attr, src, fn)` sur le
  nœud, invalidé par la source — `setAttribute` recompile),
  wrapper spec `with(document)/with(this.form||{})/
  with(this)` ; `env_get`/`env_set` des scopes `with` sont
  host-aware (props réfléchies `id`, `class`… visibles en
  accès nu). **Et** le moteur est spawné pour les pages à
  `on*` sans `<script>` (`has_inline_handlers` — sinon clic
  muet : le `want` ne regardait que les balises script).
  Test session : `onclick=` → mutation → PNG identique à la
  page statique cible.
- [x] **JS-C46. Entités non décodées dans le miroir**
  (verdict PIPE→NAV au triage M-H1 — `&nbsp;`/`&copy;`
  littéraux sur google.fr) : `parse_fragment_into`
  n'appelait `decode_entities` que sur attributs et RCDATA
  (`title`/`textarea`) — le texte courant passait brut →
  `escape_text` ré-échappait `&` → `&amp;nbsp;` affiché en
  clair après re-parse, et `textContent` lisait le littéral.
  Livré : `decode_entities` au `flush_text!` + table élargie
  (jeu HTML 4.01 ~200 noms, `&#x…;` hex, remap windows-1252
  des `0x80..=0x9F`, `U+FFFD` spec) — jscore reste autonome,
  pas de dep sur `explorer-html`. Écart connu documenté :
  entités legacy sans `;` non décodées (règle de
  l'esperluette ambiguë = table complète), demande de
  couture `explorer_html::entities` consignée au journal.
  Bonus diagnostic (même commit) : `PipelineError` porte
  l'en-tête de la source/module fautif — « Error: b »
  localise enfin son `<script>`.
- [x] **JS-C47. Diagnostic d'exception — pile d'appel** :
  **livré**. `Engine.call_stack` pousse le `Rc<FnLit>` à
  l'entrée de chaque appel `Fun::Script` (après la borne
  `call_depth`, dépilé sur les trois sorties : bind params,
  corps, fin) — natives absentes par construction.
  `FnLit.pos` = `(ligne, colonne)` 1-based du jeton ouvrant
  le littéral, capturée aux 10 sites du parseur (fonctions,
  flèches, `async`, méthodes objet/classe, blocs `static`) ;
  `(0,0)` pour les synthèses (ctor de classe par défaut).
  `stack_text` rend le format V8 `Name: msg\n    at f
  (l:c)` borné à 10 frames (`stackTraceLimit`) — utilisé par
  `error_value` (capture à la construction, spec) et
  `Error.captureStackTrace` (l'élision `ctor` n'est pas
  honorée, déviation). `describe` préfère la prop `stack` →
  `JsError::Uncaught` hérite les frames : `Error: b` cite
  désormais la fonction fautive + son site de définition.
  Écart résiduel : position du **site d'appel** inconnue
  (l'AST ne porte pas de span par statement — la définition
  suffit à localiser dans le script cité par JS-C46).
  Prochain vol google pour confirmation consent.
  Tests : 6 cas `lang` (ordre frames, anonymes, capture à
  la construction, `captureStackTrace`, natives, Uncaught).
- [x] **JS-C48. Rapatriement des avancées Dillo `dillo-js`** :
  la copie vendored avait dérivé (~1 200 lignes de delta
  bidirectionnel sur 6 fichiers). Port-back complet dans le
  canonique : `window.addEventListener`/`removeEventListener`/
  `dispatchEvent` (sommet de propagation), `getComputedStyle`
  (inline `StyleHost`), `matchMedia` (`MediaQueryList`
  `matches:false`), `CSS.supports`/`escape`/`registerProperty`,
  constructeur `Node` + constantes, `document.fonts`
  (`FontFaceSet`), `defaultView`/`visibilityState`/`hidden`/
  `nodeType`/`nodeName`/`hasFocus`, `top`/`parent`/`frames`,
  `scrollX/Y`/`pageX/YOffset`, `parentElement` ≠ `parentNode`
  (spec — `html.parentElement` = `null`), dimensions
  client/scroll/offset (racine ≙ viewport, sinon 0),
  `history.pushState`/`replaceState` (URL sans rechargement),
  `navigator.sendBeacon`, handler `on*` retournant `false` →
  `preventDefault`, `AbortController`/`AbortSignal` (+ rejet
  `fetch` sur signal déjà aborted), diagnostic TypeError
  « `x` is not a function (on recv) », regexp : quantificateurs
  lazy `*?`/`+?`/`??`/`{n,m}?`, bornes de classe échappées
  décodées (`\xHH`, `\uHHHH`, `\u{…}`, `\0`, `\cX`, octales
  Annex B) et surrogates omis/clampés (classe vidée →
  `(?!)`/`[\s\S]`), `performance.mark`/`measure`/`timeOrigin`/
  `timing`/`navigation` (stubs honnêtes), re-baseline test262
  **1007/1046 exécutés (96 %)** avec whitelist re-vérifiée, et
  l'outil `examples/census.rs` (compte-rendu des erreurs de
  pages réelles). Sens inverse vérifié : JS-C46 (entités),
  JS-C45 et la vague JS-C26+ étaient déjà Explorer-only —
  `src/dom.rs` reste canonique.
- [x] **JS-C49. Hit-test de clic logué + validation terrain
  consent** (`explorer-core`) : `node_at` émet un
  `tracing::debug!` du tag frappé par le clic — premier
  réflexe de triage des « clics ignorés » (la carte du
  consent google est un patchwork `input`/`form`/`div`/∅ —
  comprendre où le point atterrit explique chaque
  `Ignored`). Validation terrain live (avec les
  quantifieurs lazy de JS-C48) : consent rendu, scripts
  sains, `location` redirect JS effectif → google.fr —
  **la chaîne consent→accueil google marche de bout en
  bout** (M-H1 #7 débloqué). Reste observé : jonglerie de
  focus sur `keydown` (un `key` sur deux rapporte
  `unfocused` — la machinerie suggest re-focus entre les
  events ; constat consigné, à trier côté miroir).
- [x] **JS-C50. `eval_hook` — observabilité des sources
  évaluées dynamiquement** (`explorer-jscore`) : le code
  obfusqué des pages réelles (bootstrap Google) vit dans
  `eval`/`Function`/timer-string, pas dans le `<script>`
  parent — les frames d'erreur (JS-C47) pointaient des
  positions dans des sources invisibles. Livré : hook
  `Engine::set_eval_hook(Option<EvalHook>)` additif,
  invoqué **avant parse** sur `eval_source`,
  `eval_source_in` et le constructeur `Function` ;
  `census.rs --dump-scripts` l'exploite pour écrire
  `eval_<n>.js` — le bundle Closure complet de Google
  (`eval_0.js`, 1861 lignes) est devenu rejouable en
  isolation, ce qui a permis de bissecter le bug
  d'ordre d'évaluation corrigé en JS-C52.
- [x] **JS-C51. `navigator.languages` absent** (demande
  PIPE→NAV au journal 2026-10-21 — triage résidu
  example.com : `navigator.languages.some(...)` →
  `TypeError` tuait `/s.js` entre ses deux
  `insertAdjacentHTML`, le `<svg>` logo n'entrait jamais
  dans le miroir) : tableau **gelé** `["en-US","en"]`
  dérivé de `language` installé dans `install_dom`
  (low-fingerprint, même instance à chaque lecture).
- [x] **JS-C52. Ordre d'évaluation des affectations —
  LHS avant RHS** (`explorer-jscore`) : le bootstrap
  Google mourait `TypeError: vG is not a function` —
  `Expr::Assign` évaluait le RHS **avant** la référence
  LHS, alors que le code Closure installe l'accesseur
  dans le LHS : `(..., SL(kW,…), K).Wn = kW.vG()`.
  Livré : `LValueRef` réifiée — `resolve_lvalue`
  évalue `base`/`key` (effets de bord inclus) avant le
  RHS, `read_lvalue_ref`/`store_lvalue` consomment la
  référence ; bonus spec : `a[i()] op= x` n'évalue
  plus `i()` deux fois. Bonus diagnostic : `recv_desc`
  affine — receveur callable → `function
  <anon>@l:c` (pos JS-C47) ou `.name`, objet
  ordinaire → `constructor.name` — « `x` is not a
  function (on `D7`) » localise la classe dans du
  minifié. Terrain : `eval_0.js` rejoue `ok`, la page
  résultats google complète émet
  `[navigate] https://www.google.com/?sei=…` — le
  redirect `location.replace` tire. Tests :
  `assign_lhs_evalue_avant_rhs`,
  `assign_index_key_evalue_avant_rhs`,
  `assign_compound_key_evaluee_une_fois`.
- [x] **JS-C53. Re-baseline test262 + solde des 39 échecs**
  (`explorer-jscore`, piste JS-Cxx post-vague regexp) : la
  référence 1007/1046 datait d'avant C43/C48/C36 —
  re-mesurée **inchangée** (la vague n'a rien régressé ni
  débloqué dans l'extrait), puis les trois familles
  résiduelles soldées — score final **1046/1046 exécutés,
  31 skips, 0 échec** sur le sous-ensemble extrait (1077
  fichiers) :
  - *Séparateurs numériques à zéro en tête* (8 négatifs) :
    `0_0`, `08_0`, `0_8`, `0_0123456789`… — ES2021 interdit
    `_` dans `LegacyOctalLikeDecimalIntegerLiteral` et
    `NonOctalDecimalIntegerLiteral` ; `Lexer::number`
    rejette désormais `_` dans toute forme `0…` (les
    préfixes `0x`/`0o`/`0b` gardent le séparateur).
  - *`put-value-prop-base-primitive`* : `[[Set]]` dont la
    base est primitive — la chaîne proto doit intercepter
    (setter hérité, proxy `set` amarré par
    `Object.setPrototypeOf`). `set_prop_value` route les
    vraies primitives (`prim_set_intercept`, refus sans
    interception — jamais de prop propre) ; et `write_prop`
    consultait la trap `set` des proxies **uniquement en
    tête** — les maillons intermédiaires étaient ignorés
    (cas `Symbol().x`, cellule-objet). `proxy_set_step`
    factorise trap + invariants spec, appelé en tête *et*
    à chaque maillon.
  - *`#\uXXXX` — noms privés échappés* (30 cas, un seul
    mécanisme) : `private_name` n'acceptait pas
    `\uXXXX`/`\u{…}` — les tests « Unicode identifiers » du
    corpus sont tous des `#…` échappés (les tables
    `unicode-ident` suffisaient).
  Tests : `primitive_base_write_reaches_chain_interceptors`,
  `mid_chain_proxy_set_trap` ; cliquet `BASELINE_PASSED`
  remonté à 1046. Terrain restant : TypeError lobste.rs
  ×4 → rejouer `eval_hook` (JS-C50) quand les pages
  sauvegardées arrivent.
- [x] **JS-C54. Sélecteurs d'attribut `[a]`/`[a=v]` au
  mini-DOM** (`explorer-jscore`, re-mesure terrain) : le
  replay lobste.rs montrait le module `application.js`
  propre — les TypeError ×4 historiques ne se reproduisent
  plus — mais `qSA('time[data-at-unix]')` retournait une
  liste **vide** : le sous-ensemble `querySelector` ne
  connaissait que tag/`#id`/`.class`. Fix : `SimpleSel`
  étendu aux filtres attribut (`=`/`~=`/`|=`/`^=`/`$=`/`*=`,
  valeurs nues ou quotées, flag `i`/`s`, multi-classe
  `.a.b` au passage). Résultat mesuré : le listener
  `DOMContentLoaded` réécrit désormais les `<time>` en
  texte relatif (« 12 hours ago ») — les ×4 étaient
  masqués par le no-op, les APIs (`dataset`, `innerText`,
  `addEventListener`, NodeList itérable) fonctionnent.
  Test : `query_selector_attr_operators`.
- [x] **JS-C55. Garde d'allocation hostile `alloc_guard`**
  (`explorer-jscore`, audit borne/panic) — **livré** :
  l'audit a mesuré que `vec![x; n]`/`repeat`/`resize` sur un
  `n` tiré du script **aborte le processus** dans l'allocateur
  Rust — le `mem_limit` soft n'est relu qu'au `tick` suivant,
  trop tard (mesure : `new Float64Array(1e10)` ≈ 160 Gio →
  `STATUS_STACK_BUFFER_OVERRUN` ; `a[4294967295]=1` densifie
  ~4 Gio×16 ≈ 68 Gio → station saturée). Fix : `Engine::
  alloc_guard(bytes)` refuse en `RangeError` au-delà de
  `min(mem_limit, 256 Mio)` — appliqué à `Array(n)`,
  écriture index/`length` (chemin `[[Set]]` + `defineProperty`),
  `ArrayBuffer`/`SharedArrayBuffer` (ctor, `resize`, `grow`),
  `String.repeat`, `padStart/End`, ctors `TA(n)`/array-like.
  Bonus borne : `big_binary` pow et `to_bits` (`asIntN`)
  plafonnés à 1 M de bits — `10n ** 1e9n`/`asIntN(1e30)`
  refusent au lieu d'allouer des Gio de limbs. Suite audit :
  la pile Rust débordait sur récursion **native** —
  `join`/`toString`/`to_primitive` chaînés traversaient
  `call_value` sans compter `call_depth` ni passer par
  `stacker::maybe_grow` (`a.join('')` sur 300 k niveaux →
  `STATUS_STACK_OVERFLOW`, mesuré). Fix : `Engine::call_native`
  borne les natifs comme les appels Script ; `flat_into`
  devient itératif (worklist + tick + alloc_guard) ;
  `clone_deep` plafonne à 256 niveaux (`DataCloneError` typé).
  Passe DOM : descentes `collect_*`/`serialize`/`clone_node`
  itératives (pile Rust épargnée) ; **parse profond O(d²)**
  corrigé — drapeau `attached` par nœud (O(1) pour
  `is_attached`/`mark_dirty_at`) et `common_ancestor` en
  HashSet (le 200 k niveaux `innerHTML` pendait >60 s,
  maintenant ~4 s) ; combinatoire descendant plafonné à
  64 parties. Passe `builtins/array` : `sort` O(n²)→O(n log n)
  (merge itératif + clés matérialisées), `join` gardé sur
  taille cumulée native, `concat`/`forEach`/`map`/`filter`/
  `reduce` bornés (cap `length` 1e6 + tick). Tests :
  `tests/hostile.rs` — 12 batteries par module
  (string, array, regexp, typed array, json/eval,
  temporal/intl, bigint/math, proxy, async, DOM, profondeur)
  × appels pathologiques — contrat unique « Ok ou Err,
  jamais panic ».
- [x] **JS-C56. Census terrain github.com / bbc.com — les gaps
  réels** (`explorer-jscore`, sonde `LOBSTE_PROBE_URL`) — la
  vague census a produit des erreurs mesurées, chacune
  reproduite en isolé puis fixée :
  - **BBC** : « unterminated string in template » sur les
    chunks Turbopack/styled-components — `skip_nested_template`
    traitait les `'`/`"` du *texte brut* d'un template
    imbriqué comme des délimiteurs de chaîne (`don't` en CSS
    injecté) → scan jusqu'à EOF. Fix : `skip_quoted` réservé
    au contenu `${…}` ; le texte brut n'est plus quoté.
    4/4 chunks fautifs passent.
  - **BBC** : `invalid regex … Target of repeat operator` —
    lookahead **quantifié** `(?=comodo_)?dragon` (détection
    UA) : JS légal (Annex B), fancy-regex refuse. Fix :
    pile `look_pos` parallèle à `open_stack` dans
    `translate_pattern` — à la fermeture d'un lookahead
    quantifié, le quantificateur est absorbé : min=0 →
    `(?:(?=…)|)`, min≥1 → forme nue (assertion zéro-largeur
    idempotente — `(?:(?=a)|)+` aurait tort de matcher sur
    'b'). Mode `u` : SyntaxError, conforme Annex B — cliquet
    test262 `u-invalid-*-lookahead` intact.
  - **GitHub** : `ReferenceError: HTMLElement/Event` puis
    `TypeError: illegal constructor` — les interfaces DOM
    n'existaient pas (`ariaNotify` polyfill) ou n'étaient
    pas constructibles. Fix : constructeurs-stubs
    inconstructibles (`Element`, `HTMLElement`, `Document`,
    `DOMException`… — « illegal constructor » comme `Node`)
    pour la détection `typeof`/`in`/`instanceof` ; `Event` +
    famille (`CustomEvent`/`UIEvent`/`MouseEvent`/
    `KeyboardEvent`…) **constructibles** — objets ordinaires
    dispatchables (`type`/`bubbles`/`cancelable`/`composed`,
    payload `detail`/`error`/`promise` copiés, proto chaîné
    sur `Event.prototype`, `preventDefault`/`stopPropagation`/
    `initEvent`/`composedPath`, constantes `*_PHASE`) ;
    `AbortController` minimal (`signal.aborted`, `abort()`,
    `onabort`) ; `IntersectionObserver`/`ResizeObserver`
    constructibles — callback requis, `observe`/`unobserve`/
    `disconnect`/`takeRecords` no-op honnêtes (le miroir ne
    génère ni intersection ni resize — jamais de tir
    artificiel du callback).
  - **GitHub** : `TypeError: observe: nœud requis` —
    `MutationObserver.observe(document)` rejeté : `node_for`
    ne reconnaissait que `NodeHost`. Fix : `DocumentHost`
    résout sur la racine du miroir.
  - **GitHub** : `customElements is not defined` (bloc
    polyfill rejoué en isolé) — `CustomElementsHost` :
    `define` (TypeError si ctor non-appelable ou nom pris),
    `get`, `whenDefined` (promesse tenue pour nom
    enregistré, pendante sinon — spec), `upgrade` no-op ;
    `trace` GC sur les ctors retenus.
  - **GitHub** : `syntax error — expected property name` —
    `class X { a(){};b(){} }` rejeté : l'élément de classe
    vide `;` (ClassElement de la grammaire) n'était pas
    accepté. Fix : consommé dans la boucle de membres.
  - **GitHub** : `TypeError: cannot convert a Symbol to
    string` — attribution par frame d'abord, coupable
    ensuite : `FnLit.src` porte le nom de la source
    (`eval_module` → URL du bundle, `eval`/`Function` →
    étiquette générique) et `stack_text` le rend
    `at f (bundle.js:l:c)`. Les stacks citent alors chaque
    maillon webpack — le Symbol sautait dans
    `catalyst-*.js` (`hasOwnProperty.call(t, O)` avec `O`
    symbole, callback registry des custom elements).
    Cause : `hasOwnProperty`, `Object.defineProperty` et
    `getOwnPropertyDescriptor` convertissaient leur clé par
    `to_js_string` au lieu de `prop_key` — `ToString` au
    lieu de `ToPropertyKey` (spec). Fix : `prop_key`
    partout — un symbole indexe sa clé unique.
    `JsEngine::eval_named` (méthode additive, défaut
    `eval`) + `Cmd::Eval.name` : la session passe l'URL du
    `<script src>` classique — les frames citent aussi les
    scripts non-module.
  - **arstechnica.com (vague 2)** : `DocumentFragment`
    implémenté — `NodeKind::Fragment` (nodeType 11),
    insertion = splice des enfants + vidage du fragment
    (spec), sérialisation sans balise englobante ;
    `document.location` ≙ `window.location` ;
    `document.implementation` — `createHTMLDocument(title)`
    construit un document détaché complet (html/head/title/
    body) — **jQuery 3.7.1 tourne en entier**
    (`jQuery.fn.jquery` vérifié) ; littéraux `0x` géants
    (obfuscateur AWS WAF, ~100 chiffres) — `u64` débordait
    en « malformed numeric literal », repli f64.
  - **Résidu honnête** : `react` bare specifier — le
    registry GitHub résout `react` vers
    `assets/react` (React réel, hors périmètre miroir) ;
    `value is not a function` à
    `app-foundation` (`useSyncExternalStore` — aval du
    `react` manquant) ; dev.to : sonde pendue >10 min
    (tempête de timers ou fetch gelé — à investiguer) ;
    `/keys` : 404 hors session authentifiée.
  Tests : `mutation_observer_document_target`,
  `dom_interface_constructors`, `custom_elements_registry`,
  `regexp_quantified_lookahead`, `class_empty_elements`,
  `template_literals` (quotes en texte brut),
  `symbol_cles_propres_to_property_key`,
  `stack_attribue_la_source_par_frame`,
  `document_fragment_splice_et_serialise`,
  `document_location_egale_window`,
  `hex_literals_geants_convertissent_en_f64`.

## Tâches continues

- **Spécialiste JS** : divergence miroir ↔ `explorer-dom` quand
  le parse miroir est fautif → fix `explorer-jscore` + cas `corpus.rs`.
- **Conformance** : `explorer-test-support`, corpus extraction,
  outillage smoke — la culture « % mesurés » (ADR-0010).
- **Triage produit** : échecs `site_smoke` classés « navigateur »
  (session, net, JS, UI) → tests → étapes.

## Interfaces vers l'équipe PIPELINE (récapitulatif)

| Besoin | Fourni par | État |
|---|---|---|
| Parse incrémental `push()` | `explorer-html` | ✅ prêt — à exploiter (étape 53 ↔ **R-1**, tête de leur file) |
| Styles/fontes/layout/render | API `explorer-css`/`layout`/`render` | ✅ stables |
| Dirty rects | display list `explorer-render` | **R-7** à faire → étape 55 |
| Raster canvas op-list | `explorer-render` | **R-11** à faire ↔ JS-C28 |
| `<svg>` inline, `@keyframes`, flex/grid | domaine | R-6→R-8, R-3→R-4 |

Règle : les demandes d'API au domaine pur transitent par
`docs/interface-equipes.md` — jamais de commit direct dans
un crate PIPELINE.
