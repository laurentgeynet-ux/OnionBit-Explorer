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
  `pump_resync_coalesced_under_floor`. Coût unitaire
  **attribué → R-65** (`FontCache` froid par apply —
  `with_font_cache`/`take_font_cache` livrés). Reste :
  transplantation NAV bloquée — `FontCache` est `!Send`
  (`Rc` dans `data`) et `PaintPayload` migre en
  `spawn_blocking` ; demande `Rc`→`Arc` au journal
  (précédent `GlyphCache`/`Arc<OwnedFace>`). Re-mesure
  `sync_profile` à l'atterrissage ; reliquat éventuel :
  `abs_geometry`→`shrink`.
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
- [x] **Étape 71. Seam `MediaSurface` — lecture média par
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
  - [x] **71e — variante FFI spécifiée + câblée** : le
    mode in-process (premier rang P4) porte le cycle
    complet. Sérialisation du contrat descendue dans
    `explorer_core` (`media_event_json`, `media_offer_json`,
    `media_report_from_json`, `media_codecs_from_json` —
    api et ffi la partagent, charges identiques SSE/FFI).
    ABI pull : `explorer_media_events` (NDJSON
    `{event,data}`, jamais de ligne tronquée —
    `media_pending` retient le reliquat),
    `explorer_media_list` (fiches = DTO `GET …/media`),
    `claim`/`release`/`state` (corps JSON du REST, codes
    `0`/`-400`/`-409`/`-410` = statuts du contrat). App :
    ops worker 13-17, `listMedia`/`claimMedia`/
    `releaseMedia`/`mediaState` réels (fin des
    `UnsupportedError`), drain média dans le RPC `tick`
    → `_onEvent` (même dispatch que le SSE : claim →
    lecteur → commandes JS → rapports ≤ 4 Hz) ; dll
    antérieure → stubs honnêtes. Test
    `media_surface_ffi_cycle` (offered → claim → -409 →
    state → -400/-409 → release → media_gone à close).
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
  rejouée : `dark-normal-b6128952.css`
  fetchée en vol (0 erreur).

- [x] **Étape 99. Fenêtre raster au
  scroll** (consommation R-62 —
  `rasterize_window_cached` livré) :
  doc > `16 384/scale` px → la frame
  couvre la tranche viewport
  `[0, vh]` (items déjà ancrés
  `-scroll_y`) au lieu d'un pixmap
  `w×16384` de ~52 Mio re-rasterisé
  à chaque scroll — le cap passe du
  document à la fenêtre. `doc_h` reste
  réel (clamp scroll), `doc_h_clamped`
  marque le mode pour l'API ; dirty
  remplacé par un rect fenêtre pleine
  (le scroll re-translate tout —
  `full_cover` quasi systématique).
  Chemins sync et détaché couverts
  (`repaint_payload` unique). Test :
  `tall_document_scrolls_past_raster_cap`
  — doc 20 000 px, scroll 18 000 →
  frame `800×600` peignant la bande
  profonde, aller-retour haut/fond.
  Dernier blocage fonctionnel connu
  du corpus levé.

- [x] **Étape 100. `FontCache` layout
  persistant entre applys** (consom-
  mation R-65 + **R-67** — le type
  est `Send` depuis la conversion
  `Rc`→`Arc` de `data`, demande
  journalisée) : le cache vit dans
  `PaintPayload`/`Tab` (`layout_font_
  cache`, analogue `font_cache`
  `GlyphCache` R-36) et voyage en
  `spawn_blocking` avec la payload
  détachée — le chemin dominant
  `drain_applies_shared` est couvert.
  `layout_ctx` amorce chaque
  `BlockCtx` sur le cache sorti,
  `restitute_font_cache` le reprend
  sur tous les chemins — succès
  comme repli incrémental. Re-mesure
  `sync_profile` (repro mission 13,
  `profile_click_sync_heavy` ~3 000
  nœuds) : **dispatch total ~21 s →
  0,53 s** — le mur `relayout_subtree`
  était le shaping re-facturé, pas la
  structure ; verdict conforme à
  l'attribution PIPE (le suspect
  suivant `abs_geometry`→`shrink`
  n'est pas atteint). Zéro casse :
  cache froid = même passe qu'avant.
  Test : `relayout_reuses_persistent_
  font_cache` — compteur `font_data`,
  2e relayout = 0 résolution.

- [x] **Étape 101. Deadline total de
  requête** (trigger vol humain —
  fnac.fr : 403 anti-bot affiché
  après ~60 s) : `connect_timeout`
  et `io_timeout` ne bornent que
  l'établissement et chaque lecture
  — un serveur « tarpit » gouttant
  sa réponse sous le seuil par-ops
  n'était jamais coupé. `HttpConfig.
  request_timeout` (45 s) borne la
  phase en-têtes (et le corps
  bufferisé) h1 **et** h2 ; le corps
  streamé reçoit un budget neuf de
  même taille dans sa tâche détachée
  (`IoCtx.deadline` / idem h2) — un
  serveur lent sur les en-têtes
  n'affame pas un corps légitime.
  `NetError::Timeout` phase
  `request`. Repros :
  `tarpit_headers_hit_request_
  timeout` (gouttes < `io_timeout`,
  total coupé à la borne) et
  `tarpit_stream_body_hit_request_
  timeout` (Timeout livré sur le
  canal corps). Le 403 reste le
  serveur — c'est sa latence qui
  est désormais bornée.

- [x] **Étape 102. `fetch_stream`
  réel sur toute la chaîne de
  décorateurs** (même trigger
  fnac.fr, attribution creusée) :
  `NavJob::run` décide sur le
  statut dès les en-têtes — mais
  `Redirect`/`Caching`/`Cookie`/
  `Decoding` n'implémentaient pas
  `fetch_stream` → le repli
  `fetch` bufferisait le corps
  **complet à chaque hop et sur
  la réponse terminale**. Un 403
  à corps servi au goutte-à-
  goutte retenait donc la page
  d'erreur derrière le corps
  entier (borné Étape 101, encore
  ~45 s) — et la peinture
  progressive de l'étape 53 était
  morte en production sur **toute**
  navigation. Fix : chaque
  décorateur stream — `Redirect`
  suit les hops sans attendre
  leur corps (drop → la tâche
  de corps meurt, délai nul) et
  rend les en-têtes de la cible
  dès réception ; `Decoding`
  décode en vol via `spawn_
  blocking` + `ChanReader`
  (`deflate` zlib/raw choisi par
  signature 2 octets, borne
  `max_decoded` conservée) ;
  `Cookie` capture `Set-Cookie`
  sur les en-têtes streamés ;
  `Caching` court-circuite franc
  (intention documentée — le
  store travaille sur corps
  entiers) ; `Hsts`/`Referer`/
  `Policy` passent à travers avec
  leurs mutations de requête.
  Effet mesuré : statut d'erreur
  rendu dès les en-têtes (<1 s,
  borne 5 s en test — vs corps
  complet requis avant), et la
  navigation re-peint pendant le
  téléchargement. Vérifs terrain :
  nav réelle `www.fnac.com` →
  403 en 0,46 s, `lobste.rs` 200
  streamé+décodé end-to-end →
  PNG produit. Repros :
  `stream_error_status_headers_
  early_stalled_body` (hop 301 +
  403 à corps retenus — en-têtes
  <5 s), `stream_follows_redirect_
  and_delivers_body`, `stream_
  gzip/raw_deflate_decoded_in_
  flight`, `stream_identity_
  passthrough`. Limite honnête :
  `fnac.fr` nu est géo-bloqué au
  connect depuis cette machine —
  la mesure du ~60 s exact reste
  celle du vol humain ; la repro
  in-process prouve le mécanisme.

- [x] **Étape 103. Props
  `KeyboardEvent` plateforme**
  (trigger JS nommé — `key`/`code`/
  `keyCode`/`which` absents des
  events dispatchés, les handlers
  clavier réels lisaient
  `undefined`) : `EventProp`
  (`Str`/`Num`/`Bool`) transporte
  les props plateforme de
  `dispatch_key` → `dispatch_
  path_event_props` →
  `JsEngine::dispatch_event_props`
  → `Cmd::Dispatch.props` →
  `dom_event(_on)_props` →
  `EventState.props`, résolus par
  `EventHost::get` après les
  champs natifs (type/target/
  bubbles/defaultPrevented…).
  Table `key_code_of` (UI Events) :
  lettres ASCII → `KeyX` +
  keyCode ASCII majuscule,
  chiffres → `DigitN`, touches
  nommées (Enter 13, flèches
  37-40, modifieurs, F1-F12),
  ponctuation OEM US ; inconnue →
  `code` "" / keyCode 0, jamais
  de panic. `which` = `keyCode`
  (alias hérité jQuery). Le ctor
  `KeyboardEvent` copie aussi
  `key`/`code`/`keyCode`/`which`/
  `charCode` + modifieurs depuis
  l'init dict (events synthétiques
  couverts). Trait défaillant par
  défaut → props perdues sans
  casse. Repros jscore :
  `dom_event_props_reach_
  listeners` (élément ciblé),
  `dom_event_props_on_document_
  level` (document sans focus),
  `keyboard_event_ctor_copies_
  init_keys`. Réutilisable tel
  quel pour `MouseEvent.clientX`…

- [x] **Étape 104. CSS parsé par
  feuille** (suite de R-68 — option
  structurelle notée par PIPE au
  journal, reprise) : la session
  concaténait toutes les feuilles
  en un blob avant `add_css` —
  chaque `<style>` inline et
  chaque `<link>` est désormais
  parsé isolément. Gains :
  isolation d'erreur (une accolade
  non fermée n'avale plus la
  feuille suivante), chaque
  feuille reste sous `MAX_TOKENS`
  (85 K max mesuré sur github.com
  vs 384 K le blob), `@import`/
  `@charset` en tête de feuille
  redeviennent valides (au milieu
  du blob : rejetés). `linked_css:
  String` → `linked_sheets:
  Vec<String>` sur `Tab`,
  `PaintPayload`, snapshot
  bfcache ; `doc_css` →
  `doc_sheets` (SubJob) ;
  `collect_style_css` →
  `collect_style_sheets`. Ordre de
  cascade inchangé. Scans
  `@font-face`/backgrounds `url()`
  par feuille. Test :
  `broken_sheet_does_not_swallow_
  next`.

- [x] **Étape 105. Modifieurs
  clavier** (ticket adjacent de
  l'étape 103 — `ctrlKey`/
  `shiftKey` lisaient
  `undefined`) : `InputEvent::
  Key`/`KeyUp` portent `mods:
  KeyMods` (ctrl/shift/alt/meta)
  → les quatre booleans du
  `KeyboardEvent`. Un accord
  Ctrl/Alt/Meta est livré à la
  page mais saute l'action par
  défaut (édition, submit
  `Enter`) ; `shift` seul reste
  de la frappe. REST
  `key`/`keyup` : `ctrl`/`shift`/
  `alt`/`meta` optionnels. FFI
  `explorer_input` : `mods: u8`
  (bitmask — ABI in-process avec
  l'app) ; Dart envoie l'état
  `HardwareKeyboard` réel.
  Tests : `keydown_event_carries_
  modifier_props` (frame),
  `keydown_chord_suppresses_
  default_edit` (`q=Ab`),
  `keymods_from_bits_decode`.

- [x] **Étape 106. Fuite mémoire
  frames (~10 Go, vol humain
  22/10)** : l'app grossissait sans
  plafond (~270 Mo retenus **par
  pull de frame**). Bissection par
  sondes live (fetch seul /
  décode+dispose / stockage) +
  profil heap via la VM service :
  cause racine = **payloads dans
  `BrowserState` épinglés par
  l'arbre de widgets** (états
  obsolètes retenus via
  `DebugCreator`/listeners
  d'animations — `widget.state`
  gardait ~20-250 Mo chacun) +
  **`ImageCache` accumulant les
  octets sources** (`MemoryImage`
  à clé-identité unique par pull).
  `ui.Image.dispose()` était
  innocent. Correctif : frames
  hors état → `_frameImages` map
  privée mutée en place
  (`frameTick` pilote le rebuild),
  chemin unique `ui.Image` +
  `RawImage` REST/FFI, purge à
  `tab_closed` ;
  `getBytes` : `BytesBuilder`
  (`Uint8List` unique ~25 Mo au
  lieu d'une `List<int>` growable
  ~256 Mo) typé jusqu'au widget.
  Mesure : 339→353 Mo après 110
  changements de viewport forcés.

- [x] **Étape 107. Worker isolate
  FFI** (vol 22/10 constat (1) —
  « FFI inutilisable sous charge ») :
  les appels natifs `explorer_*` sont
  synchrones et bloquants (`block_on`
  Rust — `navigate` peut tenir des
  dizaines de secondes sous
  `request_timeout`) et tournaient
  dans l'isolate UI → latences +
  freezes francs. Un isolate dédié
  possède désormais `DynamicLibrary`
  + le handle — séquentialité
  préservée (ABI non thread-safe),
  UI jamais bloquée. RPC
  `[seq,op,arg]` → `{ok,data|err}`
  par ports ; `navigate`/`back`/
  `forward`/`reload` retournent
  `{url,title}` relus dans le même
  aller-retour (redirects vus) ;
  frames en `TransferableTypedData`
  (zéro-copie de port) ;
  `tryLoad` reste synchrone — sonde
  `open`+`lookup` légère, repli REST
  inchangé ; `tick` devient
  `Future<bool>` (seul call-site
  retouché). Limite honnête : un
  appel natif coincé n'est pas
  interruptible — le kill de
  l'isolate diffère à la sortie du
  code natif ; l'UI reste
  responsive (le contrat). Sonde
  live vs DLL release : ready →
  openTab → navigate → frame
  1024×768 → input mods → shutdown.
  Fichiers : `ffi_worker.dart`
  (nouveau — typedefs + dispatch),
  `ffi_engine.dart` réécrit en
  client RPC.

- [x] **Étape 108. `<img>`/fonds
  fetchés survivent aux synchros JS**
  (attribution PIPE mesurée vol 22/10,
  journal 22/10 — logo central du
  portail Wikipedia absent) :
  `apply_payload` RAZ `p.images` à
  chaque apply — `NodeId` régénérés
  au re-parse, corps `<img>`/`poster`
  consommés une fois, `bg_bodies`
  drainé sans restitution → tout
  média http(s) mourait au premier
  apply post-fetch. Caches décodés
  persistants `img_decoded`
  (`src`/`poster` → image) et
  `bg_decoded` (URL résolue → image)
  sur `Tab` + `PaintPayload`
  (take/put, purgés à la nav) ;
  `reinsert_imgs_by_src` re-matche
  `collect_images(new_dom)` à chaque
  apply (reflow + incrémental) ;
  `insert_fetched_backgrounds`
  idempotent. Tests
  `img_survives_js_dom_resync` +
  `fetched_background_survives_js_dom_
  resync` (mutation `setTimeout`
  post-nav, page flottante → reflow
  complet ; contrôle négatif : le
  premier échoue sans le fix).
  Terrain : globe du portail
  Wikipedia peint après synchro
  `jsl10n`.

- [x] **Étape 109. Consommateur
  `rects` + traduction locale de
  frame** (audit 2 — la plomberie
  `frame?x&y&w&h` + `rects`/
  `frame_copy_region`/`dirty_rects`
  existait côté moteur depuis
  l'étape 55, personne ne la
  lisait) : `scroll` lit le
  `scroll_y` autoritaire de la
  réponse `input` (REST JSON +
  FFI joint par le worker) →
  `paintedScrollY` vs `scrollY`
  pilote `Transform.translate` de
  la `RawImage` retenue — retour
  visuel molette immédiat, la
  bande exposée attend le pull.
  `frame_invalidated` parse
  `rects` (px physiques) + plan :
  même position → accumulation →
  pull **partiel** (`fetchFrame`
  région → `PictureRecorder`
  composite sur l'image retenue) ;
  scroll, `rects` vide/absent
  (contrat 55), volume borné
  (`frameRegionMaxRects`×16,
  `frameRegionMaxArea`×½) ou
  échec → plein auto-réparateur.
  FFI : `tick` enrichi rend
  `scroll_y` + `dirty_rects`
  drainés dans le même RPC —
  même plan, exports absents →
  `null` → plein (dll antérieure
  tolérée). `paintedScrollY` posé
  à l'**install** (conservateur :
  retarde plutôt que survole),
  purgé à `navigated`/`tab_closed`.
  Clip aux bornes frame (sonde
  live : lobste.rs émet `y:-375`
  — un rect hors-fenêtre ferait
  400 REST et coulerait le
  partiel ; tout-dehors → skip).
  Tests : traduction locale
  scroll → zéro au pull livré,
  pull partiel rects, `rects: []`
  → plein, hors-frame → clip/skip,
  coalescence étape 89 intacte.

- [x] **Étape 110. `SharedFonts`
  délègue `font_face` +
  `fallback_for`** (demande PIPE
  journalisée 08/11 — activation
  session de R-69) : le port
  `FontSource` a gagné deux méthodes
  pour le repli par glyphe, mais
  `SharedFonts` (le `WebFontSource`
  de session sous `Mutex`) ne
  déléguait que `font_data` +
  `revision` — les défauts du trait
  re-servaient `font_data` à
  l'index 0 (faces `.ttc` >0
  inaccessibles) et `None` pour le
  repli → R-69 prouvé en sonde mais
  **dormant en produit**. Deux
  délégations sous verrou, même
  borne de rétention que
  `font_data` (recherche + clone
  `Arc`, pas de copie d'octets).
  Test `shared_fonts_delegates_face_
  and_fallback` : face `@font-face`
  enregistrée → `font_face`/`fallback_
  for` la servent (avant : défaut
  `None` → l'`expect` paniquait).

- [x] **Étape 111. Pilote ADR-0025 —
  synchro miroir→DOM par journal de
  mutations** (revue externe P1,
  escalade JS-C39 actée) : chaque
  `apply` de synchro JS re-parsait le
  HTML complet → `NodeId` tous
  régénérés → cause racine d'Étape
  108 et des re-key images/focus/
  find/canvas. Désormais :
  - *Miroir* (`explorer-jscore`) :
    `journal` capture **inconditionnelle**
    des mutations attachées (avant le
    filtre `MutationObserver`), `serialize_
    keyed` livre `ids` pré-ordre, empreinte
    structurelle FNV-1a (tags, hors
    texte, `<template>` via `content`),
    `poison_journal` sur chemins non
    tracés (`template.content`), journal
    clôt à `install_dom`/baseline.
  - *Couture* (`explorer-js`) :
    `DomMutation`/`DomJournal`/
    `DomSyncKeyed`, `drain_dom_mutations`
    (classifieur : parent `<table>`/
    `<select>`/`<template>`/`html`,
    contenu `<script>`/tableaux →
    re-parse), `sync_dom_keyed`,
    `resync_dom_keyed` (re-sérialise
    même dirty consommé — repli post-
    rejet).
  - *Session* (`explorer-core`) :
    `keymap` miroir→`NodeId` sur
    `PaintPayload`, `apply_journal`
    rejoue **en place** (inserts greffés
    par parse de fragment + zip ids,
    removes purgent la table, attrs/
    texte en place), empreinte vérifiée
    post-rejeu ; rejet → re-enfilage
    baseline (`NoBaseline`/`UnmappedKey`/
    `Fingerprint`/`TooManyOps`/`Dom`/
    `NoDom`) — auto-réparant, jamais
    de divergence silencieuse. Compteurs
    `journal_applies`/`journal_fallbacks`
    (observabilité du taux de succès).
    Borne `SessionConfig::dom_journal_max_ops`
    (512). Comportement identique chemin
    synchrone et `drain_applies_shared`.
  - *Bugs trouvés par les tests* :
    records du parse initial restaient
    dans le journal (`Insert` sous
    `html` → tout classé risqué →
    journal jamais engagé) — `clear_
    journal` à l'install ; `resync_
    dom_keyed` ajouté car le drain
    consomme `dirty` (le repli aurait
    re-parsé un document vide) ;
    `engine_preorder` saute doctype/
    commentaires (le miroir n'en a
    pas — zip aligné sur pages réelles).
  - *Supplanté* : le relayout
    incrémental 55f sur le chemin JS
    (le rejeu en place est la bonne
    granulate ; 55f reste pour les
    autres applis) — test 55f porté
    sur `journal_applies`.
  - Tests : 6 couture `dom_sync.rs`
    (journal sans observer, ops,
    baseline, parents risqués, poison
    template) + 2 session (rejeu en
    place avec bootstrap NoBaseline,
    borne d'ops → repli) ; verts.

- [x] **Étape 112. Mort silencieuse
  sur articles géants — deux bugs de
  classe mesurés, fixés, prouvés**
  (constat vol élargi — `jprobe`/
  `shot` sur wikipedia/{France,
  Brasil} exit 1, zéro stderr après
  ~300-900 s CPU). Instrumentation
  `info!` aux frontières (nav,
  apply, phases pipeline, evals) —
  la mort localisée au run suivant :
  **`fetch : timeout request après
  0 ms`**, pas un crash.
  - *Cause racine* (`explorer-net`) :
    `BodyTask.deadline` absolue
    (`now + request_timeout`) —
    pendant qu'une peinture
    progressive synchrone (jusqu'à
    ~40 s/passe) monopolise le
    runtime `current_thread`,
    l'attente consommateur sur
    `tx.send` consommait la borne →
    timeout à budget nul. **Fix** :
    l'attente `send` est recréditée
    dans `deadline` (`IoCtx::credit`,
    `send_piece`) — l'attente
    consommateur n'est pas un stall
    serveur ; la borne anti-trickle
    est préservée (test tarpit
    intact).
  - *Amplificateur* (`explorer-core`) :
    peinture progressive = pipeline
    complet par chunk → coût
    quadratique sur DOM croissant
    (31+ passes, ~600 s). **Fix** :
    plancher adaptatif — une passe
    coûtant T exige ≥ T
    d'accumulation avant la suivante
    (`paint_floor` = max(intervalle,
    coût), duty ≤ ~50 %) — inchangé
    sur petites pages, ~2 passes sur
    Brasil.
  - *Validation terrain* : `shot`
    Brasil **complet** — nav commit
    à 11,7 s, frame 1,26 Mo écrite
    (avant : mort à ~10 min, zéro
    sortie). La lourdeur restante
    (apply post-script ~250 s :
    cascade + layout géant) est le
    coût unitaire PIPE documenté,
    pas une régression.
  - Tests : régression
    `consumer_wait_is_credited_
    against_body_deadline` — reproduit
    `Timeout{phase:"request",ms:0}`
    exact sans le fix, vert avec.

- [x] **Étape 113. Cycle de vie
  document : `DOMContentLoaded` +
  `load` dispatchés au commit**
  (issue #11 — lobste.rs : timestamps
  ISO bruts, journal `0/0/0`).
  Qualification mesurée : le script
  `application.js` (`type="module"`)
  s'évaluait **proprement** — son
  transform `time[data-at-unix]` vit
  entier dans
  `document.addEventListener
  ("DOMContentLoaded",…)`, et la
  session ne dispatchait **jamais**
  `DOMContentLoaded`/`load` (seuls
  les événements d'input :
  click/submit/input/scroll/wheel).
  Listener enregistré, jamais
  invoqué → aucune mutation →
  journal `0/0/0` = signature exacte
  du constat.
  - *Fix* (`sub_commit_tail`) :
    `DOMContentLoaded` dispatché au
    commit (scripts différés/modules
    déjà évalués en
    `sub_commit_head`), `load` en fin
    de queue (sous-ressources +
    historique commis). Mutations des
    listeners enfilées → appliquées
    au prochain drain comme toute
    mutation script — pas de drain
    forcé sous verrou hôte partagé.
    Listeners `document`/`window`
    stockés sur la racine du miroir :
    la propagation depuis `<body>`
    les atteint sans travail moteur.
  - *Test* :
    `lifecycle_domcontentloaded_and_
    load_dispatch` — titre chaîné
    `dcl+loaded` (le listener `load`
    relit le titre posé par DCL —
    la coalescence d'applies rend le
    passage de DCL observable) +
    `Navigated` titre `'orig'`
    inchangé.
  - *Couverture héritée* :
    `dataset`/`innerText`/sélecteurs
    d'attribut (`JS-C54`) — les
    primitives du handler lobste.rs
    étaient déjà là, il ne manquait
    que le déclenchement.

- [x] **Étape 114. Import maps :
  collecte + prefetch + eval**
  (issue #6 — github.com : la page
  sert la map React complète mais
  `import 'react'` mourrait en
  `cannot load` : jamais collectée,
  jamais poussée à
  `set_import_map`, et le prefetch
  résolvait le bare en relatif →
  `github.com/react` 404). Sonde
  3 scénarios : bare sans map servi
  exécutait **silencieusement** le
  relatif ; map + cible seule
  échouait. Fix : `collect_import_
  maps` (ordre document) →
  `merge_import_maps` ; `SubJob`
  porte raws + `imports` fusionné ;
  `module_deps` réécrit via
  `rewrite_bare_specifier` (pure,
  extraite de `resolve_bare` — une
  règle partagée prefetch/eval) ;
  maps poussées au moteur avant le
  premier `eval_module` ; bare non
  mappé = « irrésolvable » propre
  (plus de relatif déguisé) ; alias
  par spec **brut** (un module
  bare-importé est `referrer` de
  ses propres imports — trou
  révélé par le vol github réel).
  `jprobe` affiche les
  `PipelineError`. Approximation :
  toutes les maps s'appliquent à
  tous les modules. *Terrain* :
  erreurs `react` soldées sur
  github.com — résidus = gaps
  moteur distincts (Symbol,
  syntaxe, `composedPath`).
  *Régressions* :
  `importmap_reecrit_bare_au_
  prefetch_et_eval` +
  `importmap_dep_bare_resout_ses_
  propres_imports`.

- [x] **Étape 115. Affichage frame :
  `OverflowBox` + barre de scroll**
  (terrain 23/10 — « colonne
  étroite » persistante depuis les
  frames hauteur-document). Cause
  racine mesurée au SDK :
  `RenderImage._sizeForConstraints`
  réduit sa boîte au ratio de
  l'image (`constrainSizeAnd
  AttemptToPreserveAspectRatio`)
  — frame 1826×2876 dans une boîte
  1217×543 → enfant **345×543**
  centré → `paintImage` fit:none
  aligne au centre → on voyait une
  **tranche centrale à 1:1** de la
  page (le zoom « réparait » : la
  colonne mobile tenait dans la
  tranche). Fix : `OverflowBox`
  (maxW/H `infinity`, topLeft)
  autour du `RawImage` — taille
  naturelle, déborde, le
  `Container` clippe. Bonus
  robustesse mesurés en route :
  `setViewport` marque
  `_pendingFull` (les `rects`
  post-resize vivent dans les
  coords de la nouvelle frame —
  composite partiel sur ancienne
  base = corruption), et un patch
  de taille ≠ demandée (daemon a
  clippé à une frame d'une autre
  taille) rebascule en pull plein.
  **Barre de scroll verticale**
  ajoutée : piste toujours
  visible, thumb dimensionné
  `viewH/docH` (doc_h déduit de
  `raw.height/(zoom·dpr)`),
  position `scrollY/maxScroll` —
  indicateur pur, pas de drag.

- [x] **Étape 116. Omnibox synchronisée
  sur la navigation par lien**
  (terrain 23/10 — www.wikipedia.com
  → clic « Français » →
  `list_tabs` rapportait
  fr.wikipedia.org pendant que la
  barre affichait l'ancienne URL).
  Cause : le listener omnibox ne
  sélectionnait que `s.activeId` —
  une navigation interne met à jour
  `active.url` sans changer
  `activeId`. Fix : la sélection
  porte sur `s.active?.url`
  (couvre changement d'onglet **et**
  navigation interne) + garde
  `!_omniboxFocus.hasFocus` — le
  commentaire promettait « sauf
  édition en cours », le code ne le
  faisait pas. Test widget :
  `navigated` SSE injecté → le
  champ affiche la nouvelle URL.
  Harnais : fakes dépôt/événements/
  lecteur extraits de
  `browser_controller_test.dart`
  vers `test/browser_test_fakes.dart`
  (partagé avec `widget_test`).

- [x] **Étape 117. `pending_nav` purgé
  au commit** (terrain 23/10 —
  « reload sur wikipedia rechargeait
  google.com »). Le commit de
  `nav_apply` purgeait `js`,
  `deferred_inputs`, `popup`… mais
  pas la file d'intentions de
  navigation — `restore()` la
  purgeait déjà (tab.rs:111), pas
  le commit. Une intention posée
  par le document mort (clic lien
  via `handle_input_now` jamais
  drainé par l'hôte, reste au-delà
  de `MAX_NAV_HOPS`, drain
  interrompu) survivait à la
  navigation suivante et tirait au
  prochain drain — dont celui de
  `reload` (navigate.rs:107) : la
  page affichée naviguait vers la
  cible périmée. Repro mesuré :
  clic lien enfilé + nav omnibox →
  l'onglet atterrissait sur la
  cible du lien mort au lieu de
  l'URL tapée. Fix : `pending_nav.
  clear()` au commit (page réussie
  ET page d'erreur) — les
  intentions sont à vie document.
  Tests : `commit_kills_pending_nav
  _of_previous_document` (rouge→vert
  vérifié) + `reload_after_bfcache
  _restore_refetches_restored_page`
  (couvre la piste `last_req`
  initialement soupçonnée — le
  snapshot la porte déjà).

- [x] **Étape 118. `Accept-Language`
  de bout en bout** (terrain 23/10 —
  les portails servaient l'anglais).
  `LangTransport` (explorer-net) :
  décorateur à cellule partagée
  `Arc<RwLock<Option<String>>>` qui
  coiffe toute la pile transport —
  navigations, sous-ressources et
  XHR annoncent la même valeur ;
  un en-tête explicite de l'appelant
  reste prioritaire (fetch API).
  `Session::set_accept_language`
  fait évoluer la cellule à chaud ;
  `""`/`None` lève l'en-tête.
  Surface : `POST /api/accept-
  language` (`{"lang":""}` lève),
  `/api/status.accept_language`
  (observabilité), export FFI
  `explorer_set_accept_language`
  (additif — worker Dart lookup
  optionnel, dll antérieure = no-op
  documenté). App : réglage
  `AppSettings.language` + champ
  « Langue » dans les réglages —
  `''` = locale OS, sinon étiquette
  (`fr-FR`) ou liste qualifiée ;
  `_effectiveLanguage` forme
  `fr-FR,fr;q=0.9,en;q=0.8`, poussé
  au bootstrap et à chaque
  changement. Tests :
  `accept_language_reaches_requests
  _and_updates_live` (FakeTransport
  `last_header` : posé, propagé,
  levé par `""` et blancs) +
  `accept_language_roundtrip_and_
  clear` (API : set, reflet status,
  clear, rejet `lang:42`).

- [x] **Étape 119. Piège
  `engineMode=auto` rendu visible**
  (terrain : une `explorer_ffi.dll`
  périmée adjacente à l'exe gagnait
  silencieusement la sonde
  `tryLoad` — latence 20 s + gel
  attribués à tort au daemon).
  `BrowserRepository.engineLabel`
  → `REST <baseUrl>` ; `FfiEngine`
  retient le chemin candidat
  gagnant → `FFI <chemin dll>` ;
  `engineLabelProvider` lu par la
  barre de statut — désormais
  **toujours rendue** (moteur à
  droite, ellipsis RTL : la fin du
  chemin — nom du fichier — reste
  visible). `package.ps1` :
  `cargo build --release -p
  explorer-ffi` ajouté et la dll
  fraîche copiée dans `dist/app/`
  à côté de l'exe — le bundle ne
  repose plus sur une dll
  préexistante. Régression :
  `barre de statut affiche le
  moteur actif` (40/40 verts).

- [x] **Étape 120. Sondes document
  vivant + triage consent Google**
  (terrain 23/10 — le clic « Tout
  accepter » du mur consent répond
  `ignored` alors que DOM et paint
  sont justes). Outils livrés :
  `Session::dom_dump` /
  `Session::layout_dump` (DOM post-JS
  et arbre de layout du tab vivant) ;
  routes `GET /api/tabs/{id}/dom`
  et `/layout` (404 honnêtes sur
  onglet vierge) ; CLI `explorer-cli
  dom <url>` (fetch + JS + dump
  hors daemon). Triage consent :
  `/search` → `consent.google.com/ml`
  (mur UE réel, pas une boucle) ;
  DOM du mur sain (vrais
  `form[method=POST]` +
  `input[type=submit]`), jar à
  cookies câblé ; clic `ignored`
  avec `hit_tag="form"` — défaut
  layout mesuré (input `inline-flex`
  posé au rect du sibling) dispatché
  PIPE au journal avec repro minimal
  (`repro_c.html`, `is_atomic` +
  `assign_child_rects` owner-index).
  Reliquat `Error: b` = piste JS.
  Test : `dom_and_layout_dumps_after_
  navigate` (404 vierge + contenu
  après nav).

- [x] **Étape 121. Fetch `mask-image:
  url(http…)`** (demande de couture
  PIPE R-86 — icônes MDN invisibles
  `.color-theme__button`/`.external`).
  Chaîne symétrique des fonds étape
  98 : `collect_decl_urls` généralisé
  (`mask-image`/`-webkit-mask-image`
  — `mask` non expansé par la
  cascade, hors scope honnête) ;
  fetch dans la même boucle que les
  fonds (budget `max_images` partagé,
  dédup par résolue, résolution sur
  la base feuille/document,
  `data:` exclus — déjà collectés
  render-side) ; alias `mask_aliases`
  brut→résolue via `SubJob` →
  `SubCommitTail` → `Tab` →
  `PaintPayload` (chemins sync +
  détaché) ; corps `bg_bodies` et
  cache `bg_decoded` communs
  (`image_max_px`). `insert_fetched_
  images` (ex-`insert_fetched_
  backgrounds`) enregistre
  `register_mask` sous clé `url(...)`
  brute à chaque cascade — requis :
  `masks_url` meurt au RAZ
  `PaintImages::default()` de chaque
  apply ; pseudos sans `NodeId`
  couverts de fait. **Trou
  colatéral réparé** : `data_imgs.
  masks_url` jamais étendu à l'apply
  → `mask-image` `data:` aveugle en
  session depuis R-52. **Mesuré
  MDN** (page `mask-image`, 1366 px)
  : `declared=24 registered=24` —
  les spans vides `icon icon-baseline`
  (étoile badge) et `icon icon-chevron`
  (chevron accordéon) peignent leurs
  glyphes : le motif n'est plus
  structurellement invisible.
  Reliquats honnêtes : `.external`
  hors frame capturée (fragment
  scroll inerte au shot) ;
  `.color-theme__button` absent du
  DOM post-JS (hydratation — hors
  scope mask). Tests :
  `mask_image_fetched_and_rendered`
  (base feuille), `webkit_mask_image_
  fetched_and_rendered` (style
  inline + préfixe), `mask_on_
  pseudo_fetched` (motif `::before`
  MDN), `mask_image_missed_fetch_
  is_silent` (404 → fond plein),
  `data_mask_paints_in_session`,
  `fetched_mask_survives_js_dom_
  resync` (RAZ).

- [x] **Étape 122. `root_font_size`
  sur `relayout_subtree`** (reliquat
  R-88 consigné au journal) : la
  voie incrémentale ne passe pas
  par `layout_tree` →
  `BlockCtx.root_font_size` restait
  `None` → tout `rem` d'un
  sous-arbre resynchronisé
  retombait sur l'initiale (16 px)
  alors que le reflow global
  lisait la font-size calculée de
  `documentElement`. Fix côté
  appelant (`apply.rs`, aucune
  retouche PIPE) : helper
  `root_font_size_px` parse le
  `font-size` calculé de la racine
  dans la `StyleMap` fraîche
  (forme `"{px}px"` normalisée
  R-17 — même parse que
  `StyleEngine::root_font_size`) ;
  `block.root_font_size.set(...)`
  avant `relayout_subtree`, cache
  fonte restitué sur tous les
  chemins. Test :
  `rem_basis_survives_incremental_
  relayout` — `setAttribute` sur
  `<table>` (classé « re-parse »
  ADR-0025 → `dirty` intact →
  synchro cléée réelle),
  `incremental_syncs` prouve la
  voie, frame identique au
  statique 40 px (32 px avant le
  fix — régression vérifiée par
  inversion).
- [x] **Étape 123. Borne heap JS
  exposée — outillage diagnostic**
  (dispatch post-census github) :
  le challenge PoW `SG_SS` de
  Google consomme >512 Mio de
  travail attendu — la protection
  JS-C65 tue le script **avant**
  d'atteindre l'espace d'exécution
  où `Error: b` pourrait se
  prononcer. Chaîne :
  `JsEngine::set_mem_limit(bytes)`
  (trait additif, défaut no-op —
  `Cmd::SetMemLimit` feu-et-oublie
  avant `install_dom`) →
  `SessionConfig.js_mem_limit_mib`
  (défaut **512**, `0` = illimité
  explicite, saturé ×Mio) appliqué
  à chaque contexte →
  `EXPLORER_JS_MEM_MIB` lu par
  daemon (`session_config`, valeur
  tracée `info!` — la borne en
  vigueur est citée, jamais
  supposée) et CLI
  (`session_from_env`, `eprintln`).
  **Réglage diagnostic, pas
  produit** : la borne relevée ne
  rend pas la recherche Google
  fonctionnelle — le PoW reste
  ~100× trop lent sous
  interpréteur (mesuré hors de
  portée à 2 Gio d'ops) ; la
  valeur est d'atteindre l'espace
  où l'exception se prononce.
  Tests :
  `js_heap_limit_reaches_context`
  (1 Mio → alloc ~4 Mio refusée,
  titre `err:`) /
  `js_heap_limit_zero_disarms`
  (`0` → titre `ok`) — couture
  bidirectionnelle prouvée.
- [x] **Étape 124. Budget d'ops JS
  exposé — `EXPLORER_JS_OPS`**
  (suite de la mesure de vol
  `ef087e1` : borne 2048 Mio →
  le PoW `SG_SS` meurt au
  **budget ops** `5 M` au lieu
  du heap — le réglage 123 a
  déplacé le mur, pas franchi
  l'espace). `EXPLORER_JS_OPS`
  (ops absolues) lu par daemon
  (`session_config`, tracé
  `info!`) et CLI
  (`session_from_env`) ;
  `0` = illimité explicite mappé
  `u64::MAX` (`op_limit=0`
  tuerait le premier `tick` — la
  sémantique diverge de la borne
  mémoire, le mapping est local
  à l'env). La couture
  `js_ops` → `set_budget` →
  `set_op_limit` existait
  (étape ~50) — l'ajout est
  purement l'exposition env ;
  même cave « diagnostic, pas
  produit » : un `while(1)`
  gèle le contexte à 0, réglage
  de vol uniquement.

- [x] **Étape 125. En-têtes de
  métadonnées `Sec-Fetch-*`**
  (cap « web réel sécurisé » —
  worktree `nav/cap-g1`) : les
  endpoints d'auth et CDN modernes
  servent des flux différents
  selon les fetch metadata
  headers, absents des nôtres.
  `Request` gagne `fetch_dest`
  (`FetchDest` : `Empty`/`Document`/
  `Script`/`Style`/`Image`/`Font`),
  `fetch_mode` (`FetchMode` :
  `NoCors`/`Cors`/`SameOrigin` —
  `navigate` est **dérivé de
  l'initiateur** à l'émission, un
  appelant ne le pose jamais) et
  `fetch_user` — tous additifs,
  défauts `Empty`/`NoCors`/`false`
  (comportement inchangé pour les
  outils sans page). Helper
  `policy::sec_fetch_headers`
  calcule `Sec-Fetch-Site`
  (`none`/`same-origin`/`same-site`/
  `cross-site` — même grille que
  `same_site_allows` : registrable
  + schéma), `-Mode`, `-Dest`
  et `Sec-Fetch-User: ?1` (geste
  utilisateur réel uniquement) ;
  ignoré hors http(s). Câblage
  session : navigations et
  formulaires → `dest:document`
  + `?1` au geste (clic lien,
  submit implicite ; `form.submit()`
  scripté non marqué) ; `script_
  request` (`fetch()`/XHR) →
  `mode:cors`, `dest:empty` ;
  sous-ressources (`sub_request`)
  typées `script`/`style`/`image`/
  `font` par contexte d'appel ;
  modules ES → `dest:script`.
  `sec-*` déjà interdit aux
  scripts (`forbidden_request_
  header`) — les champs ne sont
  posés que par la plateforme.
  Socle `mode`/`dest` consommé
  par l'enforcement CORS (126).
  Régressions : `sec_fetch_*`
  (6 tests policy).

- [x] **Étape 126. Enforcement
  CORS** (suite cap « web réel
  sécurisé ») : `Origin` était
  émis mais `Access-Control-*`
  jamais lu sur la requête — le
  MVP bloquait tout cross-origin
  non safelisté à l'émission.
  `Request::cors: Option<
  CorsRequest>` (origin
  sérialisée, `FetchCredentials`
  `Omit`/`SameOrigin`/`Include`,
  `unsafe_headers` auteur) posé
  par `script_request` sur les
  cross `mode:cors` — la requête
  n'est plus refusée, elle est
  soumise au protocole.
  `CorsTransport` (`policy.rs`,
  wrappé sous `LangTransport` à
  la session — `fetch_stream`
  délégué pour préserver le
  rendu progressif) : preflight
  `OPTIONS` quand verbe non
  safelisté / `unsafe_headers`
  non vide / `Content-Type`
  hors types « form » →
  `Origin` + `ACRM` + `ACRH`,
  validation statut 2xx +
  `ACAO` (`*` seulement sans
  `include`) + `ACAM` + `ACAH` ;
  réponse finale vérifiée
  `ACAO`/`ACAC` (`ACAC: true`
  exigé sous `include`).
  Échec → `NetError::CorsBlocked`
  (opaque `status: 0` au script,
  même surface qu'un échec
  réseau). Cookies : `Cookie`
  injecté same-origin ou sous
  `include` (jar filtre
  `SameSite`), `Set-Cookie`
  capturé sauf `omit` /
  cross-origin seulement sous
  `include`. Pas de cache
  preflight (MVP). Couture
  jscore : `Request.fetch_mode`/
  `cors.credentials` existent —
  `fetch(mode,credentials)` peut
  s'y brancher (journal).
  Régressions `cors_*` (9 tests
  policy) ; carve-out
  `xhr_cross_origin_custom_
  header_blocked` (preflight
  réel : doc+OPTIONS = 2 appels,
  même `error` livrée).

- [x] **Étape 127. Cookies
  `Partitioned` (CHIPS)** : clé
  de jar `(partition, domaine,
  chemin, nom)` — `partition` =
  `"{scheme}://{registrable}"`
  du site top-level ([`partition_
  key`/`request_partition`]).
  Un `Set-Cookie: …; Partitioned`
  exige `Secure` (rejet sinon)
  et se range sous le seau de la
  page initiatrice ; l'envoi
  n'admet le cookie que dans son
  seau propre — la partition
  remplace la politique
  `SameSite` (spec CHIPS : le
  cloisonnement supplée la
  restriction). `CookieTransport`,
  capture `Set-Cookie` XHR
  (`NetDone.partition`) et
  `document.cookie` passent la
  partition ; `StoredCookie`/
  `RawCookie` (`serde(default)`,
  profils antérieurs relus en
  non-partitionné) la
  persistent. Les homonymes
  cohabitent par seau.
  5 tests `chips_*`.

- [x] **Étape 128. Contextes de
  navigation imbriqués
  (`<iframe>`) — socle** :
  `<iframe src>` = fetch de
  sous-contexte sous
  l'initiateur parent
  (`Subresource` → `SameSite`/
  CHIPS corrects —
  `sec-fetch-dest: iframe` +
  `sec-fetch-mode: navigate`,
  `FetchDest::Iframe`/
  `FetchMode::Navigate`
  additifs), `Set-Cookie`
  capturé sous le seau
  top-level de la page
  (`SubJob.cookies`, même
  calcul que `CookieTransport`),
  corps décodé et **parsé en
  `Dom` propre** (`SubFrame`
  sur `Tab` — ordre document,
  bfcache restauré, borné
  `max_frames: 8`, profondeur
  1 : pas de descente dans
  l'enfant). Exposé :
  `tab_frames` /
  `frame_dom_dump`.
  Régressions `iframe_*`
  (6 tests session).

- [x] **Étape 131. Sous-contextes
  `<iframe>` — rendu
  composite** (renumérotée : 129
  = radar pris par NAV, 130 =
  empreinte) : chaque
  `SubFrame` est rasterisée
  dans sa surface (pipeline
  complète : styles → boîtes
  → layout → display list →
  `rasterize_window_cached`,
  viewport = content box,
  `image_max_px` borné) puis
  blittée en `images.replaced`
  à la clé `NodeId` de sa
  boîte — `paint_replaced`
  émet l'`Image` clippé sans
  extension PIPE. La boîte
  est atomique (gabarit
  300×150 / attributs px —
  `intrinsic` → pas de repli
  RAWTEXT) même sans contexte
  (`src` absent, 404, cadre
  vide blanc). Surfaces
  cachées `(url,w,h)` bornées
  — jamais par `NodeId`
  (réattribution au re-parse) ;
  purge `images.replaced`
  **avant** les inserts de la
  passe pour la même raison.
  Relayout/resize : réutilise
  les surfaces inchangées ;
  bfcache : DOM enfants
  restaurés, surfaces
  régénérées. 4 tests
  `iframe_composite_*`.

- [x] **Étape 134. Câblage
  `SubFrame` → contexte JS**
  (renumérotée — la partie
  « ponts » de l'ex-131 ;
  JS-C78 a livré le moteur
  multi-contextes) : un realm
  par `<iframe>` fetché, créé
  au `sub_commit_head`
  **avant** les scripts du
  parent (`frames[n]`/
  `contentWindow` résolvent un
  vrai contexte en cours de
  parse). Trait `JsEngine`
  étendu additif : `context_new`
  (`frame_ord` → `frame_node`
  miroir résolu côté worker),
  `context_destroy`,
  `install_dom_ctx` (miroir
  propre + timers + XHR/fetch +
  `crypto.subtle` — **pas** de
  hooks nav ni `form.submit` :
  `location.href=` enfant est
  inerte plutôt qu'une nav sous
  identité parente),
  `eval_ctx`/`dispatch_event_ctx`/
  `sync_dom_ctx`/`drain_sinks_ctx`
  — chaque commande adressée
  fait `context_switch` puis
  **restaure** le contexte
  précédent. Worker : registres
  `ctx_doms`/`ctx_jars`/
  `ctx_stores`, compteurs de
  handles XHR/crypto **partagés**
  (Abort non ambigu sans tag).
  `SubFrame.html` retenu pour
  `install_dom_ctx`. Scripts
  **inline** de l'enfant évalués
  en ordre document + `DCL`
  enfant au head, `load` enfant
  puis `onload` de l'élément
  `<iframe>` (realm parent) au
  tail. `NetRequest::Fetch.ctx`
  : `drain_script_net` émet sous
  l'initiateur/`page_url`
  **de l'enfant** (ctx inconnu
  → échec livré, jamais la
  mauvaise identité) ; sinks
  enfant (cookies/storage)
  drainés sous origine enfant,
  partition du site top-level.
  Synchro : miroir enfant sale
  → re-parse en `SubFrame.dom`,
  bitmaps `(url,*)` purgées,
  relayout. Cycle de vie :
  `frame_ctxs` voyage dans
  `PageSnapshot` (bfcache
  conserve tout l'arbre),
  `context_destroy` explicite
  hors bfcache. Timer enfant
  tiré par la pompe cross-ctx
  (C78). Reliquat : les
  sous-ressources **du cadre**
  (ses `src=` externes) ne sont
  pas fetchées à cette tranche —
  scripts inline seuls. 9 tests
  adaptateur `frame_ctxs.rs` +
  9 tests session `iframe_*`
  (mutation, isolation, fetch
  tagué, cookie/storage enfant,
  bfcache, destroy, onload,
  timer).

- [x] **Étape 129. Radar corpus
  ≥1200 px — `doc_h`/`scroll_y`
  réels au nightly** (verdict
  ordonné post-clôture
  `Error: b` — la boucle a besoin
  que chaque livraison soit
  mesurée sous conditions réelles,
  pas seulement en unitaire).
  **`EXPLORER_VIEWPORT=WxH`**
  (ex. `1366x900`) lu par daemon
  (`DaemonConfig.viewport` →
  `session_config`, `info!` de la
  valeur active — borne citée,
  jamais supposée) et CLI
  (`session_from_env`,
  `eprintln`) — même format
  `WxH` que l'argument `LxH` de
  `shot`, défaut inchangé
  1024×768, invalide → repli
  silencieux. **`site_smoke.ps1`
  phase 2** : viewport 1366×900
  posé par onglet (shots phase 1
  idem via l'env), réponse
  `navigate` lue pour `doc_h`/
  `doc_h_clamped` (`!` au
  verdict), grille de sondes
  élargie à 1366×900 (x≤1300,
  y≤864), et **mesure de scroll
  réel** : `dy=doc_h` doit
  atteindre `doc_h−900` sinon
  `scroll:<y>/<max>:FIGÉ` — la
  suspicion `scroll_y=0.0` du
  re-census github se prouve ou
  s'innocente site par site.
  **Vérifié terrain** : Wikipedia
  1366×900 → `doc_h=4085`,
  `scroll_y=3185.29` (= doc_h−900
  exact), retour 0 propre — la
  mécanique est saine, le constat
  github serait page-spécifique.

- [x] **Étape 130. Empreinte
  mémoire par réservoir** (file
  NAV — constat « station
  ~1,4 Gio » sous sites réels).
  `Session::mem_report(tab) →
  Option<MemReport>` mesure la
  page vivante **et les pages
  gelées bfcache** : `dom_nodes`
  (`live_count`), `layout_boxes`,
  `display_items`, `frame`/`png`
  octets exacts, bitmaps
  `DecodedImage` **dé-dupliquées
  par `Arc::as_ptr`** (même image
  dans `images.*` + display list
  + caches = une fois), corps
  `bg_bodies`, et `heap_stats`
  du moteur vivant **comme des
  gelés** — le premier réservoir
  suspecté : `bfcache_pages` = 4
  → 5 états de page par onglet,
  chaque snapshot gardant un
  heap jusqu'à `js_mem_limit_mib`.
  Surface : `GET /api/tabs/{id}/
  mem` (JSON sérialisé) +
  `explorer-cli mem <url>`
  (tableau post-nav + grâce).
  **Honnêteté** : `_bytes` =
  mesuré, `_nodes`/`_boxes` =
  comptes (pas d'accesseur de
  capacité PIPE — aucune touche
  domaine) ; non mesurés
  documentés : caches de fontes,
  `prev_styles`, miroir DOM
  jscore, registres
  `idb`/`caches` (C72 — `Rc`
  injectables hors heap).
  Régression
  `mem_report_attributes_live_and
  _bfcache` (rapport peuplé,
  snapshot compté, onglet
  inconnu → `None`).

- [x] **Étape 132. Barre de scroll
  interactive** (app Flutter —
  la piste/thumb de l'étape ~115
  était « indicateur pur »).
  `_vScrollbar` gagne les gestes :
  **drag du thumb** → delta px
  logique → px CSS via la course
  réelle (`maxScroll / travel`,
  `ctrl.scroll` relatif, clamp
  moteur) ; **tap sur la piste**
  au-dessus/sous le thumb → page
  ±`viewH` (convention
  navigateur) ; curseur `grab`,
  poignée assombrie pendant le
  geste (`_scrollDrag`). Gestes
  inertes si `doc_h ≤ viewH`.
  Réutilise `ctrl.scroll` (input
  `scroll` relatif → `scroll_y`
  autoritaire → traduction
  locale étape 109) — zéro
  canal nouveau. `flutter
  analyze`/`test` verts.

- [x] **Étape 133. Discard des
  bitmaps des snapshots
  bfcache** — le multiplicateur
  mesuré par l'étape 130 (~144
  Mio de bitmaps github × 4
  pages gelées/onglet) est le
  réservoir dominant du constat
  « ~1,4 Gio ». Au `freeze` :
  les tables `images.*`
  (`replaced`/`backgrounds`/
  `svgs`/`masks`/`masks_url`)
  sont vidées et `snap.list`
  lâchée — tous les `Arc` de
  `DecodedImage` meurent ; les
  op-lists `<canvas>` survivent.
  Les **corps compressés** sont
  retenus dans `img_bodies`
  (clé URL résolue, borné
  `MAX_IMG_BODY_STORE` = 32
  Mio — `<img>`/poster,
  fonds, masques) + alias
  `bg_aliases`/`mask_aliases`.
  Au retour (`history_move`),
  `Session::restore_images`
  re-décode **sans réseau** :
  http depuis `img_bodies`,
  `data:` depuis l'attribut,
  fonds/masques via
  `prev_styles` (ordinal→NodeId
  par `element_ordinals`), svg
  inline re-rasterisés par
  `collect_svg`. `mem_report`
  gagne `img_bodies_bytes`,
  `bfcache_img_bodies_bytes` et
  le diagnostic
  `bfcache_images_bytes`
  (attendu 0) — CLI `mem`
  affiché. Tests : bitmaps
  gelés libérés + corps
  retenus + `back()` re-décode
  à l'identique sans re-fetch.

- [x] **Étape 125. En-têtes de
  métadonnées `Sec-Fetch-*`**
  (cap « web réel sécurisé » —
  worktree `nav/cap-g1`) : les
  endpoints d'auth et CDN modernes
  servent des flux différents
  selon les fetch metadata
  headers, absents des nôtres.
  `Request` gagne `fetch_dest`
  (`FetchDest` : `Empty`/`Document`/
  `Script`/`Style`/`Image`/`Font`),
  `fetch_mode` (`FetchMode` :
  `NoCors`/`Cors`/`SameOrigin` —
  `navigate` est **dérivé de
  l'initiateur** à l'émission, un
  appelant ne le pose jamais) et
  `fetch_user` — tous additifs,
  défauts `Empty`/`NoCors`/`false`
  (comportement inchangé pour les
  outils sans page). Helper
  `policy::sec_fetch_headers`
  calcule `Sec-Fetch-Site`
  (`none`/`same-origin`/`same-site`/
  `cross-site` — même grille que
  `same_site_allows` : registrable
  + schéma), `-Mode`, `-Dest`
  et `Sec-Fetch-User: ?1` (geste
  utilisateur réel uniquement) ;
  ignoré hors http(s). Câblage
  session : navigations et
  formulaires → `dest:document`
  + `?1` au geste (clic lien,
  submit implicite ; `form.submit()`
  scripté non marqué) ; `script_
  request` (`fetch()`/XHR) →
  `mode:cors`, `dest:empty` ;
  sous-ressources (`sub_request`)
  typées `script`/`style`/`image`/
  `font` par contexte d'appel ;
  modules ES → `dest:script`.
  `sec-*` déjà interdit aux
  scripts (`forbidden_request_
  header`) — les champs ne sont
  posés que par la plateforme.
  Socle `mode`/`dest` consommé
  par l'enforcement CORS (126).
  Régressions : `sec_fetch_*`
  (6 tests policy).

- [x] **Étape 126. Enforcement
  CORS** (suite cap « web réel
  sécurisé ») : `Origin` était
  émis mais `Access-Control-*`
  jamais lu sur la requête — le
  MVP bloquait tout cross-origin
  non safelisté à l'émission.
  `Request::cors: Option<
  CorsRequest>` (origin
  sérialisée, `FetchCredentials`
  `Omit`/`SameOrigin`/`Include`,
  `unsafe_headers` auteur) posé
  par `script_request` sur les
  cross `mode:cors` — la requête
  n'est plus refusée, elle est
  soumise au protocole.
  `CorsTransport` (`policy.rs`,
  wrappé sous `LangTransport` à
  la session — `fetch_stream`
  délégué pour préserver le
  rendu progressif) : preflight
  `OPTIONS` quand verbe non
  safelisté / `unsafe_headers`
  non vide / `Content-Type`
  hors types « form » →
  `Origin` + `ACRM` + `ACRH`,
  validation statut 2xx +
  `ACAO` (`*` seulement sans
  `include`) + `ACAM` + `ACAH` ;
  réponse finale vérifiée
  `ACAO`/`ACAC` (`ACAC: true`
  exigé sous `include`).
  Échec → `NetError::CorsBlocked`
  (opaque `status: 0` au script,
  même surface qu'un échec
  réseau). Cookies : `Cookie`
  injecté same-origin ou sous
  `include` (jar filtre
  `SameSite`), `Set-Cookie`
  capturé sauf `omit` /
  cross-origin seulement sous
  `include`. Pas de cache
  preflight (MVP). Couture
  jscore : `Request.fetch_mode`/
  `cors.credentials` existent —
  `fetch(mode,credentials)` peut
  s'y brancher (journal).
  Régressions `cors_*` (9 tests
  policy) ; carve-out
  `xhr_cross_origin_custom_
  header_blocked` (preflight
  réel : doc+OPTIONS = 2 appels,
  même `error` livrée).

- [x] **Étape 127. Cookies
  `Partitioned` (CHIPS)** : clé
  de jar `(partition, domaine,
  chemin, nom)` — `partition` =
  `"{scheme}://{registrable}"`
  du site top-level ([`partition_
  key`/`request_partition`]).
  Un `Set-Cookie: …; Partitioned`
  exige `Secure` (rejet sinon)
  et se range sous le seau de la
  page initiatrice ; l'envoi
  n'admet le cookie que dans son
  seau propre — la partition
  remplace la politique
  `SameSite` (spec CHIPS : le
  cloisonnement supplée la
  restriction). `CookieTransport`,
  capture `Set-Cookie` XHR
  (`NetDone.partition`) et
  `document.cookie` passent la
  partition ; `StoredCookie`/
  `RawCookie` (`serde(default)`,
  profils antérieurs relus en
  non-partitionné) la
  persistent. Les homonymes
  cohabitent par seau.
  5 tests `chips_*`.

- [x] **Étape 128. Contextes de
  navigation imbriqués
  (`<iframe>`) — socle** :
  `<iframe src>` = fetch de
  sous-contexte sous
  l'initiateur parent
  (`Subresource` → `SameSite`/
  CHIPS corrects —
  `sec-fetch-dest: iframe` +
  `sec-fetch-mode: navigate`,
  `FetchDest::Iframe`/
  `FetchMode::Navigate`
  additifs), `Set-Cookie`
  capturé sous le seau
  top-level de la page
  (`SubJob.cookies`, même
  calcul que `CookieTransport`),
  corps décodé et **parsé en
  `Dom` propre** (`SubFrame`
  sur `Tab` — ordre document,
  bfcache restauré, borné
  `max_frames: 8`, profondeur
  1 : pas de descente dans
  l'enfant). Exposé :
  `tab_frames` /
  `frame_dom_dump`.
  Régressions `iframe_*`
  (6 tests session).

- [x] **Étape 129. Sous-contextes
  `<iframe>` — rendu
  composite** : chaque
  `SubFrame` est rasterisée
  dans sa surface (pipeline
  complète : styles → boîtes
  → layout → display list →
  `rasterize_window_cached`,
  viewport = content box,
  `image_max_px` borné) puis
  blittée en `images.replaced`
  à la clé `NodeId` de sa
  boîte — `paint_replaced`
  émet l'`Image` clippé sans
  extension PIPE. La boîte
  est atomique (gabarit
  300×150 / attributs px —
  `intrinsic` → pas de repli
  RAWTEXT) même sans contexte
  (`src` absent, 404, cadre
  vide blanc). Surfaces
  cachées `(url,w,h)` bornées
  — jamais par `NodeId`
  (réattribution au re-parse) ;
  purge `images.replaced`
  **avant** les inserts de la
  passe pour la même raison.
  Relayout/resize : réutilise
  les surfaces inchangées ;
  bfcache : DOM enfants
  restaurés, surfaces
  régénérées. 4 tests
  `iframe_composite_*`.

- [x] **Étape 130. Ponts JS des
  sous-contextes `<iframe>`** :
  `JsEngine` par contexte
  enfant (JS-C78 + étape 134 +
  CAP-171/175), `window.parent`/
  `top`/`self` résolus
  (same-origin = objets réels,
  cross-origin = `WindowProxy`
  opaque), `postMessage`
  cross-contexte borné
  (`WireValue`, `targetOrigin`
  exigé, `'*'` spec). **Fix de
  livraison** : les scripts
  inline du cadre s'évaluant
  avant ceux du parent, une
  task `postMessage` drainée
  à la frontière de job de
  l'émetteur livrait à un
  document sans listener —
  perte sèche mesurée.
  `CtxMeta.defer_msgs` (armé
  par `set_ready_state(…,
  "loading")`, désarmé à
  `interactive`/`complete`)
  retient les messages entrants
  pendant le parse du document
  cible — FIFO préservé, drop
  silencieux sur contexte mort.
  Défaut `false` hors session
  (harnais inchangés). 4 tests
  session `iframe_postmessage_*/
  iframe_window_refs_*`.

- [x] **Étape 131. Cibles de
  navigation (`target`) et
  `rel`** : `target=_blank`
  ou nom non réservé sur
  `<a>`/`<form>` → `DrainAct::
  NewTab` — le drain ouvre un
  onglet (`TabOpened` émis) et
  y conduit la requête ;
  `_self`/`_parent`/`_top`/
  absent → onglet courant.
  `rel=noreferrer` →
  `Request::no_referer`
  (additif net) supprime
  l'injection `Referer` dans
  `RefererTransport` —
  `Origin` préservé (pas une
  info de référent) ;
  `noopener` implicite sans
  objet tant que
  `window.opener` n'a pas de
  pont JS. Bornes : contextes
  nommés non réutilisés
  (chaque clic = onglet frais),
  `form.submit()` miroir ne
  porte pas `target` (lien
  DOM réel et submit hôte
  seulement). 4 tests
  `target_*`/`form_target_*`
  session + 1 test
  `no_referer` policy.

- [x] **Étape 132. `Referrer-Policy`
  et émission du `Referer` de
  navigation** : `ReferrerPolicy`
  (net, 8 tokens, parse
  insensible à la casse,
  inconnu → défaut
  `strict-origin-when-cross-origin`)
  sur `Request::referrer_policy` —
  `RefererTransport` calcule
  URL/origine/rien selon politique
  × same-origin × downgrade
  (https→http interdit sauf
  `unsafe-url`), normal **et**
  stream, prévol CORS inclus.
  Les **navigations émettent
  désormais `Referer`** (trou
  réel : seuls sub/XHR le
  faisaient) via la politique
  de la page **source**. Politique
  de page : en-tête
  `Referrer-Policy` (dernier
  valide gagne) puis
  `<meta name=referrer>` — vit
  sur `Tab`/`PageSnapshot`
  (bfcache la restaure).
  Attribut `referrerpolicy` sur
  `<a>`/`<form>`/`<iframe>` =
  override par requête ;
  sous-ressources/script/fetch/
  XHR suivent la politique
  courante. `Origin` toujours
  indépendant du `Referer`.
  6 tests session + 3 policy.

- [x] **Étape 133. `Origin` sur
  les requêtes non-safe** :
  spec Fetch — toute méthode
  ni `GET` ni `HEAD` porte
  `Origin`, **same-origin
  inclus** (les endpoints
  CSRF/auth l'exigent ; le
  gate était cross-origin
  seulement). `origin_value`
  (net) élargie, origine
  opaque → `Origin: null`.
  `nav_begin` stamp `Origin`
  sur les navigations POST
  initiées par la page —
  trou mesuré : les submit
  de formulaire n'en
  portaient aucun. Fetch/
  XHR same-origin POST
  couverts via
  `initiator_headers_policy`.
  3 tests session + 2 policy.

- [x] **Étape 134. En-têtes de
  sécurité de réponse** :
  `nosniff` (dest `script` →
  MIME JavaScript, `style` →
  `text/css`, `Content-Type`
  absent = échec) ; CORP
  `same-origin`/`same-site`
  sur les sous-ressources
  `no-cors` ; `X-Frame-Options`
  `deny`/`sameorigin` + CSP
  `frame-ancestors`
  (`'none'`/`'self'`/scheme/
  host-source, `*` — CSP
  l'emporte sur XFO) sur les
  navigations `<iframe>`.
  Enforcement aux points
  d'admission : vague de sous-
  ressources, `@import`,
  modules, fonts, fonds ;
  réponse refusée = ressource
  abandonnée (même surface
  qu'un échec réseau).
  8 tests session + 3 policy.

- [x] **CAP-170. `document.readyState` +
  `readystatechange`** — *(numéro officiel attribué
  à l'intégration)* : le cycle de vie document est
  réel (`loading` → `interactive` → `complete`),
  fini le `readyState` codé `"complete"` en dur —
  les pages qui attendent `interactive`/`complete`
  (bootstrap de Docs/Gmail, mesures de perf,
  defer maison) lisent l'état vrai et reçoivent
  `readystatechange` à chaque transition (ordre
  spec : `interactive` avant `DOMContentLoaded`,
  `complete` avant `load`). `hooks.ready_state`
  document-local garé par `BrowsingContext` —
  chaque `<iframe>` suit son propre cycle.
  `JsEngine::set_ready_state` additif (défaut
  no-op) ; le `readystatechange` suit la voie
  `dispatch_event_ctx`/`dispatch_doc_event`
  (mutations des listeners resynchronisées).
  Bornes documentées : les scripts `defer` courent
  avant la transition `interactive` (ils lisent
  `loading`, écart spec assumé — notre commit
  n'a pas encore de frontière parse/defer
  observable) ; `readystatechange` d'un `<iframe>`
  ne se propage pas au parent. 2 tests session
  (`document_ready_state_*`).

- [x] **CAP-193. Canal diag clone-
  getters (mur `TanStack` x-web)** —
  `set_diag_hook` + `e.diag` : émis
  quand `{...}`/`Object.assign` perd
  les getters proto de la source
  (`proto_has_accessor`, `Prop::Acc`).
  Spéc-correct mais signature exacte
  du mur `Invariant failed` :
  `e.id` → `undefined` ×2 →
  collision `i['undefined']` au 2ᵉ
  nœud. La sonde volante tranche les
  hypothèses. Sondes `probe_tanstack`
  (namespace getter proto intact,
  spread/assign déprotoise,
  `setPrototypeOf` restaure).

- [x] **JS-C34. Suspension réelle —
  générateurs lazy + `await` pendante
  (mur n°1 moteur)** — `function*`
  produit `ObjKind::GenLive` : `GExec`
  suspendable (pile de contrôle
  `Vec<GFrame>` — `Seq`/`While`/`For`/
  `ForIter`/`Try`), `yield`/`await`
  hissés en init de statement par
  `lower_suspend_body`. `next(v)`/
  `throw`/`return` réels ; `for..of`,
  spread, `yield*`, boucles+try
  verts. **`async fn`** : même
  `GExec` — `await` pendante →
  réaction `then` boundée au
  `GenLive` (`BoundNative`, bound=
  `[gen, outer]`), `resume_await`
  reprend, `settle_outer` settle la
  promesse rendue. Fixtures `c34_*`
  dé-ignorées vertes. Borne :
  `await`/`yield` en sous-expression
  paresseuse (`&&`/`?:`/`?.`/`??`)
  évaluée éager — hoist non porté.

- [x] **JS-C35b. TLA de module** —
  `await` top-level → corps module
  sur `GExec` (même infra JS-C34).
  Borne : importeurs n'attendent pas
  (exports lus paresseusement via
  `nsget`) — entrées feuilles
  couvertes. Test `tla_module_awaits_
  pending`.

- [x] **CAP-197. Shadow DOM T3+T5
  (miroir)** — T3 répartition flat :
  `slots_in`/`slot_assigned`/
  `assigned_slot` (`<slot>` DFS,
  enfants hôte dont `slot` égale
  `name`, textes → défaut, racines
  exclues) ; `el.slot=` reflété.
  T5 `<template shadowrootmode>` —
  fragment `content` = racine.
  Journal : flat-tree rendu + parse
  déclaratif réel = entrées PIPE
  futures. Tests `shadow_t3`.

- [x] **CAP-188. `document.evaluate` :
  sous-ensemble `XPath` borné (sonde)** —
  `evaluate(expr, ctx, resolver, type,
  result)` → `XPathResult` — chemins
  `//tag`, `/tag`, `.//`, `.`/`..`,
  `[@a]`/`[@a='v']`/`[n]`, `id('x')`,
  `count(...)` — syntaxe non portée →
  `TypeError` honnête. Types scalaires
  1/2/3 + listes 4/5/7/9 (`iterateNext`,
  `snapshotItem`, `singleNodeValue`) —
  liste matérialisée (borné : pas
  d'itérateur live). Réparation :
  extractions `spawn_dyn_script_src`/
  `drain_net_done`/`deliver_dyn_done`
  + `eval_module_graph` par ref
  (`net.rs` >100 lignes post
  CAP-169/186/JS-C94). Test jscore
  `cap188_xpath_subset`. **La file
  jscore mesurée est épuisée** —
  reste : `canvas.toDataURL`/
  `captureStream` (pont raster session),
  artefacts spec-corrects.

- [x] **CAP-187. Web Animations borné :
  `animate`/`getAnimations`/
  `startViewTransition` (sonde)** —
  machine à états honnête sans
  compositeur : `playState`/`play`/
  `pause`/`finish`/`cancel`/`reverse`,
  promesses `finished`/`ready`,
  `onfinish`/`oncancel`, registre vivant
  `misc.anims` (identité préservée) ;
  `ViewTransition` `{updateCallbackDone,
  ready, finished, skipTransition}` —
  callback immédiat, promesses
  résolues (borné : pas de transition
  visuelle). Test jscore
  `cap187_web_animations_bounded`.
  Reste mesuré : `evaluate`,
  `canvas.toDataURL`/`captureStream`
  (arcs P2/P3).

- [x] **CAP-183. Surfaces honnêtes :
  `createNodeIterator`, `SharedWorker`,
  `CSSStyleSheet`, ctors spec-fidèles
  (sonde)** — `createNodeIterator` :
  itérateur doc-order incluant la
  racine (spec ≠ TreeWalker),
  `previousNode` via `tw_prev` ;
  helpers `tw_*` factorisés partagés.
  `new SharedWorker` : ctor défini →
  `TypeError` gate sans loader,
  `{port}` délégué avec (borné : pas
  de partage inter-contextes).
  `new CSSStyleSheet` : constructible
  borné — `{cssRules, insertRule,
  deleteRule, replace, replaceSync}`
  (règles stockées, cascade =
  chantier session). `TreeWalker`/
  `NodeIterator`/`MutationRecord`/
  `IdleDeadline` → `illegal_ctor`
  (`TypeError` spec, pas
  `ReferenceError`). Réparation :
  `build_session` extrait (`main`
  106/100 lignes post-JS-C94 — équipe
  causale répare). Test jscore
  `cap183_honest_surfaces`. Reste
  mesuré : `evaluate`/animate/canvas
  (arcs P2/P3 — XPath, Web Animations,
  pont raster).

- [x] **CAP-184. `document.currentScript`
  par séquence figée (sonde x.com)** — *(numéro
  officiel attribué à l'intégration)* : le
  lookup ordinal `by_tag("script").get(ord)`
  se décalait dès qu'un script s'auto-
  supprimait — stream `$_TSR` : chaque chunk
  fait `currentScript.remove()` → ordinaires
  suivants pointaient le script d'après puis
  hors-bornes → `TypeError` sur chaque chunk
  (4 erreurs sonde). Tampon de création figé
  `Dom::script_seqs` (ord → nœud) posé au
  parse, lookup insensible aux `remove()` —
  un script retiré avant éval reste
  identifiable (spec : `currentScript` =
  l'élément, attaché ou non). Test
  `document_current_script_survives_self_
  remove`. Sonde x.com : 4 erreurs → **0**,
  bootstrap `$_TSR` hydraté.

- [x] **CAP-185. `load`/`error` des `<link
  rel=stylesheet>` statiques (sonde x.com)** —
  *(numéro officiel attribué à l'intégration)* :
  les feuilles du parse initial fethaient mais
  ne tiraient jamais l'événement — or
  `document.addEventListener('load', f, true)`
  est la gâchette `startEntry` de `x-web`
  (injection `entry-client-*.js` `type=module`).
  `Dom::link_seqs` (ordinal parmi **tous** les
  `<link>`, même convention `collect_scripts`),
  `el_ord` dans `SheetLink`/`SheetFetch`, la
  vague enregistre `(ord, ok)` sur toutes les
  issues (fetch refusé, non-2xx, sécurité
  sous-ressource, SRI) ; le commit livre
  `load`/`error` après les scripts, avant
  `after_script`. Trait
  `JsEngine::deliver_static_link` + helper
  moteur `fire_el_load_error` (factorise
  dyn_img/dyn_link). Bornes : liens filtrés
  pré-fetch sans événement, autres `rel` non
  livrés. Tests jscore `cap185_static_link_
  load_error` + session `static_stylesheet_
  fires_load_and_error`. Sonde x.com : mutation
  classée (l'`appendChild` de `startEntry`).

- [x] **CAP-190. `Intl.Segmenter` borné
  (sonde x.com)** — `new Intl.Segmenter` =
  `undefined` → ctor non-appelable dans
  `entry-client` (diagnostic : le callee est
  désormais nommé dans `new X`/`construct`
  non-appelables). Ctor `{granularity}` +
  `segment()` itérable (`containing`,
  items `{segment,index,isWordLike,input}`
  indices UTF-16) + `resolvedOptions` —
  frontières honnêtes sans CLDR (grapheme :
  surrogates/étendues/ZWJ/RI ; word :
  transitions alnum ; sentence : `[.!?]`).
  Répare `e.truthy`→`to_boolean` (code
  CAP-187 en vol). Test
  `intl_segmenter_bounded`. **Verdict :
  `entry-client` franchit les gaps d'API
  — mur = `Error: Invariant failed`,
  invariante applicative X elle-même.**

- [x] **CAP-189. `}`-bloc → regex +
  lvalues `super.*` (sonde x.com)** — deux murs
  de parse/assign de `entry-client` : (a)
  `ends_operand` classait tout `}` en fin
  d'operande → `/` après `}` de **bloc**
  (corps de fonction) lu division au lieu de
  regex (`/(?:^|;\s*)gt=/` réel) ; suivi
  `brace_kinds` parallèle à `clause_parens`
  classifie `{` bloc vs littéral objet par
  le token précédent (bornes : `{` après
  `:`/`class`/`import`/`export` = operande).
  (b) `is_lvalue` ignorait
  `Expr::SuperMember`/`SuperIndex` →
  `super.x = v`/`super[k] = v` rejetés ;
  `read_lvalue`/`write_lvalue` les livrent
  via `super_base` + `read_prop`/`write_prop`
  (receiver `this`, setter cherché sur le
  proto — spec [[Set]]). Tests :
  `regex_after_block_brace`,
  `super_prop_assignment`. Verdict sonde :
  `entry-client` + `startup-vendor`
  **parsent et s'évaluent** — mur passé
  en runtime (`TypeError: constructor is
  not a function`, API hôte manquante).

- [x] **CAP-186. `<script type="module">`
  dynamique (sonde x.com)** — `deliver_dyn_script`
  évaluait tout insertion en classique :
  l'`entry-client-*.js` de `x-web` (module
  ES) y était `SyntaxError` avalée. Le drain
  lit `type` (`DynScriptReq.module`) ;
  `src` → le corps vérifié ensemence un
  `ModuleGraph` dont les deps sont fetchées
  hors session (`spawn_dyn_module_graph`,
  `NetDone.dyn_module`), `inline` →
  `inline:{seq}` même chemin ; la livraison
  `deliver_dyn_module` évalue par
  `eval_module` puis tire `load`/`error`
  (helper `eval_mod` sans `&self` pour le
  replay `js_pending_mod` sous détachement).
  Borne : bare specifier hors import map =
  « unresolvable ». Test session
  `dynamic_module_script_evals_graph`
  (entrée + `import` réel + `load`).

- [x] **CAP-182. Arc édition : `Range`
  vivant + `TreeWalker` + `execCommand`
  (sonde)** — `new Range`/`createRange`
  constructibles (host à bornes mutables :
  `setStart*`, `selectNode*`, `clone`/
  `extract`/`deleteContents`, `insertNode`,
  `toString`, rect = ancêtre commun borné) ;
  `SelData.ranges` → `Rc<RefCell<>>`
  partagé (`getRangeAt` vivant,
  `addRange` downcast). `createTreeWalker`
  — successeur document-order borné root
  (`tw_step_out` oncles) + `NodeFilter`
  `SHOW_*`/`FILTER_*`. `execCommand` borné :
  `copy`/`cut`→hook clipboard, `selectAll`→
  range body, sinon `false`. Bornes :
  identité `getRangeAt` par appel,
  navigation inverse/fratrie → `null`.
  Test jscore `cap182_editing_arc`.

- [x] **CAP-181. `navigator.clipboard`
  par hook embedder (sonde)** —
  `writeText`/`readText` via
  `BrowserHooks::clipboard_*` +
  `set_clipboard_hooks` (engine-global) ;
  sans hook → `NotAllowedError` honnête.
  Bornes : pas de permission prompt
  (embedder arbitre), clipboard-item
  hors surface. Test jscore
  `cap181_clipboard_hook`.

- [x] **CAP-180. Hit-test inverse +
  `getClientRects` (sonde)** — même
  `GeometryMap` i#19 que `gBCR` :
  `elementFromPoint`/`elementsFromPoint`/
  `caretPositionFromPoint` (topmost-first
  borné — profondeur puis DFS desc,
  z-index non porté) ; `caretPosition`
  `{offsetNode, offset:0}` (pas de
  shaping) ; `getClientRects` → `[union]`
  ou `[]` (multi-fragments aplati,
  documenté). `Dom::node_at_path` —
  chemin inverse `element_path`. Sans
  table → `null`/`[]` honnêtes. Test
  jscore `cap180_inverse_geometry`.

- [x] **CAP-179. Machine à états
  fullscreen + popover (sonde)** —
  `requestFullscreen`/`exitFullscreen`/
  `fullscreenElement`/`fullscreen`/
  `fullscreenEnabled` (promesse résolue,
  détaché → rejet `TypeError` spec) ;
  `showPopover`/`hidePopover`/`togglePopover`
  gatés `popover` attr (`InvalidStateError`
  spec), état interne `popover_open`,
  `beforetoggle`/`toggle` tirés
  (beforetoggle non-annulable — borné).
  Slots document-local garés au swap.
  `typeof` honnête : popover seulement
  si l'attribut est présent. Test
  jscore `cap179_fullscreen_popover`.

- [x] **CAP-178. Batch surface mesurée
  (sonde `_census_calls`)** — *(numéro
  officiel attribué à l'intégration)*
  : `new Audio`/`new Video` alias
  `createElement` ; `el.getHTML`/
  `setHTMLUnsafe` (rails outer/inner,
  helper `set_inner_html`) ; `el.
  checkVisibility` (attaché + !hidden +
  !display:none — borné inline seul) ;
  stubs fenêtre honnêtes (`find`→false,
  `moveTo`/`resizeTo`/`openDialog`/
  `showModalDialog` no-op) ; `XMLSerializer`
  + `serializeToString` → `outer_html` ;
  ctors `MessageEvent`/`StorageEvent`/
  `CloseEvent` + allowlist init étendue.
  `Worker` diag : refus honnête (pas de
  chargeur installé), pas un trou. Test
  jscore `cap178_surface_batch`.

- [x] **CAP-177. Batch primitives DOM/document
  mesurées** — *(numéro officiel attribué à
  l'intégration ; trous `_census_calls`)*
  : `insertAdjacentElement` (rail
  `insert_at_position` factorisé),
  `replaceChildren`, `form.reset` (événement
  `reset` annulable), `select`/`setSelectionRange`/
  `setRangeText` + `selection*` sur
  `input`/`textarea` (slots `data-explorer-sel-*`),
  `img.decode` → promesse, `document.importNode`
  + `adoptNode`. Feature-detect per-tag étendu
  (`has_method` gaté form/field/img) ;
  `createComment` retiré de la table (non
  livré). `MutationObserver.observe` hors
  périmètre — TypeError spec-correct (artefact
  sonde). Test jscore `cap177_dom_primitives_batch`.

- [x] **CAP-176. `<dialog>` : `showModal`/`show`/
  `close` + `open`/`returnValue`** — *(numéro
  officiel attribué à l'intégration ; trou
  mesuré par sonde empirique `_census_calls`)*
  : modales sign-in — `showModal()` pose `open`
  + flag modal, `show()` pose `open`, déjà
  ouvert → `InvalidStateError` (spec) ; `close()`
  retire `open` + flag, pose `returnValue` si
  arg et tire `close` ; `close()` sur fermé →
  no-op. `open` reflété bool (`<dialog>` +
  `<details>`), `returnValue` slot spec string.
  Slots document-local garés au `switch_to`.
  Feature-detect per-tag : `typeof dlg.showModal`
  = `'function'`, `typeof div.showModal` =
  `'undefined'` — `Host::methods()` devient
  `has_method(name)` (check dépendant du
  receveur). Bornes : `cancel` (Esc) hors borne
  miroir ; `form method="dialog"` non câblé ;
  top-layer visuel = arc rendu. Test jscore
  `dialog_show_modal_close_bounded`.

- [x] **CAP-175. `postMessage '*'` spec +
  méthodes fenêtre no-op bornées** — *(numéro
  officiel attribué à l'intégration ; trous
  mesurés par sonde empirique `_census_*`)* :
  `postMessage(x, '*')` — le refus `'*'`→SyntaxError
  était une sur-restriction (spec : `'*'` = livrer
  sans check d'origine, `origin`/`source` honnêtes —
  le patron le plus fréquent des embeds réels).
  `window.scrollTo`/`scrollBy`/`scroll`/`focus`/`blur`/
  `close` — fonctions no-op bornées sur le global
  (même motif documenté que `scrollIntoView` JS-C74 ;
  géométrie/scroll réel = arc #19/JS-8, `scrollX`/
  `scrollY` restent 0 = état honnête). Tests
  `c78_post_message_origin_required_*` +
  `c78_window_scroll_focus_noop`.

- [x] **CAP-174. Feature-detect : méthodes host
  lisibles en propriété fonctionnelle** —
  *(numéro officiel attribué à l'intégration ;
  trou mesuré par sonde empirique `_census_*`)* :
  les méthodes host étaient invoke-only — `o.m()`
  dispatchait via `Host::invoke` mais `o.m` lisait
  `undefined` → feature-detects (`typeof el.m ===
  'function'`, `o = el.matches || el.msMatchesSelector`,
  `.call`/`.bind`) cassés **en masse** alors que les
  appels marchaient — les sites prenaient les
  mauvaises branches (polyfill/dégradé/bail).
  `Host::methods()` (trait additif, défaut `&[]`)
  liste les noms dispatchés ; `get_prop_value`
  enveloppe `bound_native` le nom → `'function'`
  lisible + re-dispatch `invoke` à l'appel (même
  pile que `el.m()` — `this` = receveur). Tables :
  `NODE_METHODS` (~55 méthodes élément) +
  `DOCUMENT_METHODS` (~23 document). Shadowing
  préservé (own-prop scriptée prime).
  Bornes : surface bornée aux tables
  (extension `impl methods()` par host au fil) ;
  les constructeurs restent sans `.prototype`
  réel (arc distinct). Test jscore
  `host_methods_read_as_functions` (typeof×12,
  shim, `.call` re-dispatch, shadowing).

- [x] **CAP-173. `<link rel=stylesheet>` inséré
  dynamiquement** — *(numéro officiel attribué à
  l'intégration)* : `document.createElement('link')`
  + `rel=stylesheet` + `href` + insertion — et
  `href=`/`rel=` modifié sur lien attaché — déclenche
  le cycle réel : fetch → feuille appliquée à la
  cascade → `load`/`error` — le patron CSS-in-JS
  et theme switcher (React/Vue injectent leurs
  feuilles par JS, jamais au parse). Miroir :
  `note_link` enfile les liens demandés
  (`pending_dyn_links`, `dyn_link_seen` — dedup
  par clé `rel|href`, dernier non drainé gagne ;
  **attaché requis**, spec « link processing » —
  un lien détaché ne charge pas, contrairement aux
  images). `drain_dyn_links` émet des `DynLinkReq`
  (`ctx` émetteur, `seq`, `href`, `integrity`,
  `media`, `crossorigin` parsé `FetchCredentials`,
  `referrerpolicy`) pompés par `drain_script_net`
  hors réentrance : résolution base émetteur +
  CSP `style-src` + mixed-content + `sub_request`
  (`Sec-Fetch-Dest: style`, CORS si `crossorigin`).
  La réponse passe statut 2xx +
  `subresp_security_allows` (`nosniff`/CORP) + SRI
  (`integrity_allows`, CAP-163) puis
  `decode_stylesheet` rejoint `linked_sheets` — la
  cascade la reprend au prochain apply (même pile
  que les feuilles statiques). `media` non
  matching : fetch maintenu (spec) mais
  application gatée par `media_attr_matches`.
  `load`/`error` non-bullant sur l'élément.
  Bonus livré : `l.media` propriété réfléchie
  (`MAPPED_ATTRS` — `HTMLLinkElement`/`HTMLStyle`/
  `HTMLSource`). Bornes : dernier `href` non
  pompé gagne (coalescence — pas d'annulation
  de tâche en vol) ; `disabled`/alternate
  stylesheets non honorés (la feuille s'applique
  toujours — borne documentée) ;
  `rel=preload`/`modulepreload` non couverts.
  5 tests session (`dyn_link_*`).

- [x] **CAP-172. `<img>` inséré dynamiquement** —
  *(numéro officiel attribué à l'intégration)* :
  `document.createElement('img')` + `src`/`srcset`/
  `sizes` + insertion — et `img.src=` sur élément
  **détaché** (`new Image().src=` = prefetch, spec
  « update the image data ») — déclenche le cycle
  réel : fetch → décodage → `load`/`error` —
  le patron des galeries lazy, avatars et threads
  Gmail (leurs images sont injectées par JS,
  jamais au parse). Miroir : `note_img` enfile les
  sources demandées (`pending_dyn_imgs`,
  `dyn_img_seen` — dedup par clé
  `srcset|sizes|src`, dernier non drainé gagne ;
  insertion à source inchangée ≠ re-fetch).
  `drain_dyn_imgs` émet des `DynImgReq` (`ctx`
  émetteur, `seq`, `src`, `srcset`, `sizes`,
  `crossorigin` parsé `FetchCredentials`,
  `referrerpolicy`) pompés par
  `drain_script_net` hors réentrance : `srcset` →
  candidat choisi par `resolve_sizes`/`pick_srcset`
  (mêmes helpers `pub(crate)` que `collect_images`
  — viewport/dpr de session) ; `data:` décodé
  **sans requête** ; sinon résolution base
  émetteur + CSP `img-src` + mixed-content +
  `sub_request` (`Sec-Fetch-Dest: image`, CORS si
  `crossorigin`). La réponse passe 2xx +
  `subresp_security_allows` (`nosniff`/CORP) puis
  `commit_dyn_img_body` range le décodé dans
  `img_decoded` (clé chaîne source — la resynchro
  rematche `collect_images` sur le vrai DOM) +
  `img_bodies` (URL résolue — re-décodage bfcache)
  avant `load`/`error` non-bullant sur l'élément
  — détaché inclus (prefetch). Bonus livrés :
  `m.srcset`/`m.sizes` propriétés réfléchies
  (`MAPPED_ATTRS`). Bornes : dernier `src` non
  pompé gagne (coalescence spec — pas
  d'annulation de tâche en vol) ; `loading=lazy`
  ignoré (eager, borne documentée) ;
  `<source>`/`<picture>` exclus à cette tranche.
  7 tests session (`dyn_img_*`).

- [x] **CAP-171. `<iframe>` inséré dynamiquement** —
  *(numéro officiel attribué à l'intégration)* :
  `document.createElement('iframe')` + insertion
  (ou `src=`/`srcdoc=` sur cadre attaché —
  re-navigation spec) déclenche le cycle réel :
  fetch → sous-contexte → `load`/`error` — le
  patron Gmail/One Tap/reCAPTCHA (leurs cadres
  sont injectés par JS, jamais au parse). Miroir :
  les mutateurs `Dom` enfilent les `<iframe>`
  attachés (`pending_dyn_frames`, `dyn_frame_seen`
  — contrairement aux scripts, `innerHTML`
  **charge** : pas d'« already started » pour les
  cadres, spec) ; insertion-détachée avant drain
  = annulation ; `src`/`srcdoc` modifié sur cadre
  attaché re-enfile (dedup). `drain_dyn_frames`
  émet des `DynFrameReq` (`ctx` émetteur, `seq`,
  `ord`, `src`, `srcdoc`, `sandbox`,
  `referrerpolicy`) pompés par
  `drain_script_net` hors réentrance :
  `srcdoc` → `SubFrame` inline synchrone ; `src`
  absent/vide → `about:blank` synchrone (spec «
  process the iframe attributes » — `contentWindow`
  non-null + `load` dès l'insertion) ; `src` →
  résolution base émetteur + CSP `frame-src` +
  mixed-content + `frame_request` (Sec-Fetch nav
  + `referrerpolicy` + partition cookies). La
  réponse passe `collect_frame` (XFO/`frame-ancestors`
  /`Set-Cookie` — même pile que l'étape 128) ;
  `install_dyn_frame_ctx` crée le realm via
  `JsEngine::context_new_in` (additif — rattaché
  au **ctx émetteur**, cadre imbriqué correct),
  installe le miroir, évalue les scripts inline
  (CSP enfant + `sandbox`), tire le cycle
  `readyState` complet (ordre spec) puis
  `load`/`error` sur l'élément du miroir parent.
  `FrameCtx.load_fired` empêche le double cycle
  quand le cadre est installé pendant les scripts
  du commit. Bonus spec livrés : origine `about:*`
  héritée du parent (`contentWindow`/`contentDocument`
  same-origin) ; `f.srcdoc`/`sandbox`/
  `referrerPolicy` propriétés réfléchies ;
  `document.readyState` lu par ctx **propriétaire**
  du miroir. Bornes : `max_frames` partagé avec le
  plan statique ; un `src=` en vol n'est pas
  annulé au re-set (dernier émis gagne) ; le
  `load` d'un cadre dynamique ne retarde pas le
  `load` parent. 6 tests session
  (`dyn_frame_*`).

- [x] **CAP-169. `<script>` inséré dynamiquement** —
  *(numéro officiel attribué à l'intégration)* :
  `document.createElement('script')` + insertion
  (`appendChild`/`insertBefore`/`replaceChild`,
  `before`/`after`/`prepend`/`append`,
  `insertAdjacentElement`, `document.write`)
  déclenche le cycle réel : fetch réseau → éval →
  `load`/`error` sur l'élément — le patron dominant
  des loaders (tag managers, reCAPTCHA, bundlers).
  Miroir : les mutateurs `Dom` marquent chaque
  `<script>` inséré connecté (`dyn_pending`) ;
  `innerHTML`/fragment détaché → inerte (spec
  « already started »). `drain_dyn_scripts` émet
  des `DynScriptReq` (`ctx`, `seq`, `src`, `text`,
  `nonce`, `crossorigin`, `referrerpolicy`, base)
  que la session pompe **hors** réentrance
  (`drain_script_net`) : inline → contrôle CSP
  `script-src` (nonce/hash compris) puis livraison ;
  `src` → même pile que les classiques (`sub_request`
  CORS/`Origin`/`Referer`/`Sec-Fetch-*` + `SubFetch`
  SRI — `NetDone.script` route la réponse vers
  `deliver_dyn_script`, jamais d'éval d'un corps
  refusé). Livraison dans le ctx émetteur avec
  `currentScript` posé ; `error` non-bullant.
  Bornes documentées : un fetch non retourné avant
  navigation est abandonné (tampon `js_pending_dyn`
  purgé) ; `async`/`defer`/`type` dynamiques non
  ordonnancés (le web réel n'ordonne que `src`).
  4 tests session (`dyn_script_*`).

- [x] **CAP-168. `document.currentScript`** —
  *(numéro officiel attribué à l'intégration)* :
  l'élément `<script>` en cours d'évaluation est
  reflété pendant son exécution (`null` hors éval,
  spec) — les loaders/bundles UMD et polyfills s'en
  servent pour se retrouver (Google tag, webpack
  chunks). `ScriptItem.el_ord` = ordinal parmi
  **tous** les `<script>` du document (ordre
  document, CSP-refusés/`nomodule`/types inconnus
  compris — l'index reste stable quel que soit le
  filtre de collecte) → `JsEngine::eval_script_el`
  (additif, défaut → `eval_ctx`) →
  `Cmd::EvalScriptEl` → `Engine::eval_script_el` :
  le nœud est retrouvé dans le miroir par ordinal
  (`by_tag` itératif), posé sur
  `hooks.current_script` autour de l'éval. Parse,
  `defer`/`Post` et `async`/arrivée couverts.
  Bornes documentées : modules → `null` (spec) ;
  scripts d'iframe évalués via `eval_ctx` sans
  ordinal (cas marginal — leurs loaders passent par
  `install_dom_ctx`) ; élément introuvable (miroir
  divergé par `document.write`) → `null`, éval
  normale. 1 test dom + 1 session.

- [x] **CAP-167. `crossorigin` sur
  `<script src>`/`<link rel=stylesheet>`** —
  *(numéro officiel attribué à l'intégration)* :
  l'attribut bascule la ressource cross-origin en
  `fetch_mode: cors` — `req.cors` + `Origin:` émis,
  `CorsTransport` exige `Access-Control-Allow-Origin`
  (et `Access-Control-Allow-Credentials: true` en
  `use-credentials`, `*` refusé). Mapping spec :
  absent/vide/`anonymous`/inconnu →
  `FetchCredentials::SameOrigin`, `use-credentials` →
  `Include`. Sans l'attribut la ressource reste
  `no-cors` (comportement historique conservé) ;
  same-origin avec `crossorigin` ne force pas de
  CORS inutile. SRI (CAP-163) inchangé — le condensat
  est vérifié après acceptation CORS. Bornes
  documentées : `@import`, fonds CSS, images, iframes
  et prefetch de graphe module restent `no-cors` ;
  l'origine opaque sandbox suit le chemin
  `Origin: null` existant. 5 tests session
  (ACAO absent/exact, `use-credentials` + `*`
  rejeté, stylesheet appliquée/rejetée, no-cors
  inchangé).

- [x] **CAP-166. `history.scrollRestoration`** —
  *(numéro officiel attribué à l'intégration)* : enum
  reflété `"auto"`/`"manual"` document-local —
  `BrowserHooks.scroll_restore_manual` swappé au
  `switch_to` (miroir `BrowsingContext`). Les routeurs
  SPA posent `'manual'` avant de restaurer le scroll
  eux-mêmes ; absent, l'écriture tombait sur un expando
  ou un throw selon le mode strict. Valeur hors enum →
  `TypeError` (WebIDL), état inchangé. Borne
  documentée : la persistance de scroll par entrée
  n'existant pas, l'effet `auto` reste le comportement
  actuel dans tous les cas — le drapeau est honnête
  (stocké/relu, pas un faux repli). 1 test dom.

- [x] **CAP-165. `requestIdleCallback`/`cancelIdleCallback`**
  — *(numéro officiel attribué à l'intégration)* : React
  scheduler et les libs riches en dépendent
  (`window.requestIdleCallback || shim` — sans lui le
  repli `setTimeout` dégrade le scheduling). Planifié sur
  la `TimerQueue` : sans `{timeout}` → 0 ms (la prochaine
  pompe *est* le moment creux — même déviation bornée que
  `requestAnimationFrame` ≙ `setTimeout(16)`), avec
  `{timeout: ms}` → tir à l'échéance et `didTimeout:
  true` (le timer *est* l'échéance — `{timeout:
  undefined}`/`{}` = pas d'échéance). Le callback reçoit
  un `IdleDeadline` ordinaire : `didTimeout` data-prop,
  `timeRemaining()` = native liée portant l'instant
  d'ordonnancement — reliquat d'une tranche de 50 ms,
  plancher 0 (spec : ≤ 50). Callback non-fonction →
  `TypeError` (WebIDL). `cancelIdleCallback` ≙
  `clearTimeout`. Installé via `install_timers` — live
  sur les vraies pages (explorer-js). 3 tests dom.

- [x] **CAP-164. API Node de complétude (miroir)** —
  *(numéro officiel attribué à l'intégration)* : les
  libs réelles lisent ces primitives pour la délégation
  d'événements et la comparaison d'arbres (React lit
  `getRootNode`, selector-observer `isConnected`).
  Ajouts `explorer-jscore::Dom` — tous itératifs,
  bornés par la taille d'arène (JS-C55 : profondeur
  contrôlée par le script) : `root_of` (`getRootNode` —
  sommet d'arbre, `{composed:true}` inerte : pas de
  shadow DOM), `is_equal_node` (kind/tag/attrs-
  ensemble/texte/enfants pairwise), `compare_position`
  (bitmask `DOCUMENT_POSITION_*` spec : contains→
  `8|4`, contained→`16|4`, ordre par rang préordre DFS,
  arbres disjoints→`1|32`+ordre stable d'arène),
  `normalize` (fusion `Text` adjacents + suppression
  des vides via `detach` → records `childList` émis).
  Côté hôte : `isConnected` (getter, = `is_attached`),
  `hasChildNodes`, `isSameNode`, `compareDocumentPosition`
  (TypeError sur non-nœud), `normalize`, `getRootNode`
  (racine `#document` → global `document` — identité
  `===` comme `ownerDocument`). 3 tests dom.

- [x] **CAP-163. `integrity` (SRI) sur scripts et
  feuilles externes** — *(numéro officiel attribué à
  l'intégration)* : `<script src integrity>` et
  `<link rel=stylesheet integrity>` vérifient le corps
  téléchargé contre le condensat déclaré avant
  acceptation — mismatch = ressource abandonnée, même
  surface qu'un échec réseau (jamais évaluée ni
  appliquée, spec SRI §3.3.5). `explorer-net::crypto`
  gagne `integrity_allows(meta, body)` : jetons
  `algo-base64` séparés d'espaces, `sha256`/`sha384`/
  `sha512` (options `?` ignorées, spec), le plus fort
  des algorithmes reconnus décide, aucun reconnu →
  admis (dégradation spec). Métadonnée filée :
  `collect_stylesheet_links` → triple
  `(href, media, integrity)`, `sheet_urls` →
  `Vec<(Url, Option<String>)>`, `ScriptItem.integrity`,
  garde `sri_allows` aux deux bras `Css`/`Script` du
  `fetch_wave`. Bornes : `@import` interne non
  couvert (le sous-fetch hérite la politique du
  parent, pas d'`integrity` par règle), `<img>`/module
  scripts non couverts à cette tranche. 3 tests net
  (vecteurs + match/mismatch) + 2 session (exécution
  script, pixel feuille).

- [x] **CAP-162. Focus de document (miroir)** —
  *(numéro officiel attribué à l'intégration)* :
  `el.focus()`/`el.blur()` ne faisaient que dispatcher
  un événement — aucun état, `document.activeElement`
  absent, `autofocus` ignoré. Désormais :
  `Dom.focused` (`Cell<Option<NodeId>>`, meurt avec le
  document) tenu par « focus update steps » bornées —
  `focus()` sur un élément **focalisable**
  (`input`/`select`/`textarea`/`button`/`summary`/
  `iframe`, `a`/`area`[href], `[tabindex]`,
  `[contenteditable]`, `disabled` exclus, détachés
  exclus) déplace le focus — quad `blur`/`focusout` →
  `focus`/`focusin` (ordre Chrome) ; `blur()` retire
  si détenteur ; `focus()` sur le focalisé = no-op.
  `document.activeElement` = focalisé, repli `body`,
  `null` sans corps ; nœud détaché perd le focus
  (repli immédiat). `autofocus` pose le focus au
  `install_dom` (premier élément focalisable). Flags
  plateforme corrigés : `focus`/`blur` ne **bullent**
  plus (`focusin`/`focusout` si — spec UI Events) ;
  `platform_event_flags` gagne la colonne `bubbles`
  (lire par `propagate_event`, `EventFlags.bubbles`).
  Bornes : pas de `focus-visible`, pas de
  `preventScroll`, focus non synchronisé avec le
  `focused` de session (miroir-local), `tabIndex`
  reflection non ajoutée. 2 tests dom + 1 session.

- [x] **CAP-161. `matchMedia` — évaluateur borné + env
  de session** — *(numéro officiel attribué à
  l'intégration)* : `window.matchMedia` sort du stub
  `matches:false` permanent. Nouveau module
  `explorer-jscore::mediaq` — sous-ensemble réel des
  media queries : `,` (ou), `not`/`only`, type
  `all`/`screen` vs inapplicables (`print`… → faux),
  conditions `and`/`or` (mixte = invalide, spec MQ4),
  `width`/`height` `px`/`em`/`rem` + `min-`/`max-`,
  `aspect-ratio`, `orientation`, `prefers-color-scheme`,
  `prefers-reduced-motion`, `hover`/`pointer` (et
  `any-*`), `color`/`color-gamut`/`resolution`/`grid`/
  `monochrome`/`update`/`scripting`/`display-mode`/
  `forced-colors` ; feature nue = « non zéro »,
  inconnue = clause fausse. Env poussé au spawn :
  `JsEngine::set_media_env` (`MediaEnv` —
  dark/reduced-motion/hover/pointer) +
  `JsEngine::set_viewport` **avant** `install_dom` —
  répare au passage `innerWidth`/`screen.*` jamais
  câblés (lisaient `(0,0)`) ; `Session::set_viewport`
  repousse aux moteurs vivants (re-lecture à la
  demande — pas de `change`, borné). Doublon assumé
  et borné de `explorer-css::media_matches` : jscore
  est autonome (pas de dépendance PIPE possible).
  1 test dom jscore (17 queries) + 1 test session
  (env clair vs sombre).

- [x] **CAP-160. `hashchange` + setters `location.*`** —
  *(numéro officiel attribué à l'intégration)* : deux trous
  liés des routeurs SPA. (a) `location.hash=`/`search=`/
  `pathname=`/`host`/`hostname`/`port`/`protocol` étaient
  des écritures silencieusement inertes — chaque setter
  recompose l'URL courante (`url_parts`) et émet le hook
  `navigate` (spec « Location » : tout setter de composant
  navigue ; `''` retire `hash`/`search`, `/` préfixé à
  `pathname`, `#`/`?` normalisés). (b) La navigation
  fragment et la traversée entre fragments ne dispatçaient
  pas `hashchange` : `try_fragment_nav` tire désormais
  `{type:"hashchange", oldURL, newURL}` quand le fragment
  diffère réellement (`#a`→`#a` silencieux), et la
  traversée same-document le tire **après** `popstate`
  (ordre spec). Plomberie : `JsEngine::fire_hashchange`
  additif (défaut no-op) → `Cmd::FireHashchange` →
  `Engine::fire_hashchange` (switch/restore si ctx non
  courant) → `fire_event_listeners` ; mutations des
  listeners resynchronisées miroir→DOM
  (`sync_dom_keyed`/`apply_dom_sync`/`after_script`).
  3 tests session + 1 test dom jscore (triplet d'URLs
  recomposées).
- [x] **CAP-159. Contextes nommés — `window.open(u,"x")`/
  `target="x"`/`form target="x"` réutilisent l'onglet
  porteur** — *(numéro officiel attribué à l'intégration)* :
  HTML « selecting a browsing context ». Un `target` non
  réservé (ni `_blank`/`_self`/`_parent`/`_top`) désigne un
  contexte **nommé** : le drain le retrouve par nom
  (`Session::named_or_new_tab`) et le re-navigue au lieu
  d'empiler des onglets — le cas des popups OAuth
  (`open(u,"auth")`) et des ouvertures nommées de Docs.
  `""`/`_blank` = toujours neuf sans nom ; `_self`/
  `_parent`/`_top` = contexte émetteur (repli top-level).
  `window.name` est installé via `JsEngine::set_ctx_name`
  → `BrowserHooks.win_name` (document-local, swappé au
  `switch_to`) et survit à la navigation du contexte
  (attaché au **contexte**, pas au document — spec).
  Plomberie : `OpenHook` sig `(url, name)` ; `NavRequest::
  NewTab{url,name}` + `FormSubmit.target` ;
  `ScriptNav::NewTab(Url,String)`/`Submit(Box<Request>,
  String)` ; `DrainAct::NewTab(Box<Request>,String)` ;
  `resolved_target_name` remplace le booléen
  `target_new_context` (le nom voyage jusqu'au drain) ;
  `formtarget` du submitter prime `target` du `<form>`
  (CAP-150 en version moteur). **Réparation adjacente** :
  `next_script_nav` drainait tout le lot moteur puis
  détruisait les intentions après le premier `return` —
  tampon `Tab.pending_nav_reqs` (deux `window.open` du
  même script survivent tous deux). Borne : `window.name`
  écrit par script n'est pas re-remonté à la session (le
  nom de ciblage reste celui de l'ouverture). 6 tests
  session + 1 test dom jscore (couple `(url, name)`).
- [x] **CAP-158. `history.state`/`popstate`** —
  *(numéro officiel attribué à l'intégration)* :
  `pushState(state)`/`replaceState(state)` attachent
  l'état à l'entrée d'historique — `args[0]` est
  sérialisé `JSON.stringify` côté moteur (borne
  documentée : structured clone hors portée —
  fonctions, cycles, `Map`/`Set` dégradent en
  `None` → `history.state` = `null`), transporté
  par `NavRequest::PushState.state` jusqu'à
  `HistEntry.state`. `pushState(state)` **sans**
  URL crée une entrée à même adresse (spec).
  `history.state` getter désérialise par
  `json::parse` (`null` absent/indésérialisable) ;
  la traversée same-document restitue l'état de
  l'entrée cible **et** dispatche `popstate` —
  `{type:"popstate", state}` via
  `fire_event_listeners` (cible window, listeners
  `onpopstate` + `addEventListener` ; les
  mutations des handlers sont resynchronisées
  DOM→rendu). Entrée fragment hérite l'état
  courant (spec). Plomberie : alias
  `PushStateHook`, `BrowserHooks`/`BrowsingContext`
  `.history_state` swappé au `switch_to`,
  `JsEngine::sync_ctx_state` additif (défaut →
  `sync_ctx_url`) → `Cmd::SyncCtxState` →
  `Engine::sync_ctx_state` (popstate `false` au
  push/replace — spec). 5 tests session
  (`history_state_*`, `popstate_*`,
  `push_state_without_url`, `replace_state_*`).
- [x] **CAP-157. `<img srcset>`/`sizes`** — *(numéro
  officiel attribué à l'intégration)* : sélection de
  la candidate par `pick_srcset` au collect — `x` →
  plus petit ratio `≥ dpr` de session (`scale`),
  `w` → plus petit `w ≥ sizes×dpr` (`sizes` résolu :
  premier terme non média, `px`/`vw`/`em`/`rem`,
  défaut `100vw`) ; `srcset` résoluble **prime**
  `src`, `src` reste le repli quand aucun candidat
  n'est utilisable. La candidate choisie devient la
  clé `src` de bout en bout (fetch, décodage, cache
  `img_decoded`, réinsertion post re-parse) — un
  seul point de décision, zéro désynchronisation.
  Plomberie : `collect_images(dom, img_w, dpr)` —
  `img_w` = viewport CSS px, `dpr` = `scale` —
  propagée aux quatre sites (plan de fetch,
  insertion, réinsertion, payload détaché via
  `ApplyCtx`). Bornes documentées : candidats
  `data:` et descripteurs `h` ignorés, mélange
  `x`/`w` invalide → repli `src`, termes `sizes`
  à condition média sautés, `<picture><source>` et
  `imagesrcset` de `<link>` hors portée.
  4 tests session (sélection DPR, `w`+`sizes`,
  sans `src`, primauté + repli).
- [x] **CAP-156. `document.referrer`** — *(numéro
  officiel attribué à l'intégration)* : la propriété
  reflète le `Referer` effectivement envoyé par la
  navigation du document — lu depuis les en-têtes
  finaux de `tail.req` au commit (politique
  `Referrer-Policy`/`rel=noreferrer`/`no_referer`
  déjà appliqués — le DOM reflète le fil réel, pas
  un recalcul approximatif) ; `""` hors navigation
  référée (omnibox, `about:blank`, noreferrer).
  Plomberie additive : `JsEngine::set_page_referrer`
  (défaut no-op) → `Cmd::SetPageReferrer` →
  `BrowserHooks.page_referrer` → getter
  `document.referrer` ; `spawn_engine` porte le
  paramètre. Borne documentée : les contextes
  `<iframe>` partagent `hooks` — leur
  `document.referrer` retourne le referrer du
  document top (idéalement celui du parent).
- [x] **CAP-155. `<noscript>` sous scripting** —
  *(numéro officiel attribué à l'intégration)* :
  « scripting flag » de la spec (§13.2.3.5) posé
  additivement sur `TreeBuilder` (`set_scripting`
  + `parse_scripting`, défaut `false` = parse
  historique) — avec JS actif, `<noscript>` est
  du **rawtext** en `<head>` **et** en body : le
  fallback ne devient jamais des éléments (aucun
  fetch ni rendu des `<img>`/`<link>`/`meta`
  enfouis — tout site sérieux en porte, les
  dupliquer était un bug de compat réel). Filé
  par contexte : `NavJob.scripting` (top-level,
  `js_factory.is_some()`), `frame_scripting` =
  parent **et** `allow-scripts` du sandbox
  (`store_frame`/`store_srcdoc_frame`),
  `ApplyCtx.scripting` + re-parse des miroirs
  (`apply`, `sync_frame_mirrors`, composite) —
  sinon le fallback réapparaissait à chaque sync
  miroir. Un cadre `sandbox` sans `allow-scripts`
  affiche correctement son fallback.
- [x] **CAP-154. Navigation `javascript:` évaluée** —
  *(numéro officiel attribué à l'intégration)* : une
  intention vers une URL `javascript:` n'est plus
  fetchée — le chemin opaque **percent-décodé**
  (spec HTML) est évalué dans le contexte top-level
  (`DrainAct::Req` intercepté au drain, session
  exclusive + partagée — couvre clic lien,
  `location=`, `form action`, `open`). `void(0)` =
  no-op dominant (résultat non-string ; borne : un
  string ne remplace pas encore le document) ; CSP
  `script-src` s'applique (`'unsafe-inline'` requis,
  nonce/hash n'ouvrent pas `javascript:`). Miroir
  muté → `sync_dom_keyed` + `apply_dom_sync` +
  `after_script` (cookies/storage/XHR émis par le
  code évalué). `has_javascript_urls` installe le
  moteur au commit pour une page sans `<script>`
  porteuse d'un `href`/`action`/`formaction`
  `javascript:` — sinon l'évaluation serait lettre
  morte. `explorer_net::url::percent_decode` exposé
  (API additive).
- [x] **CAP-153. Activation behavior du clic scripté +
  `window.open`** — *(numéro officiel attribué à
  l'intégration)* : `el.click()` exécute désormais
  l'action par défaut après dispatch si non annulée
  (spec « synthetic click » — les apps JS riches
  pilotent la navigation par clics programmatiques) :
  `<a href>` navigue (`target` ≠ `_self` → hook
  `open` → `NavRequest::NewTab` → `DrainAct::NewTab`,
  même voie que le clic plateforme `_blank`),
  `input`/`button` submit+image soumettent le
  `<form>` ancêtre via `submit_via` (factorisé de
  `requestSubmit` — validation + event `submit` +
  submitter dans les champs), `checkbox`/`radio`
  basculent `checked` (+`input`/`change`, radio
  décoche son groupe `name`+form), `<label>`
  délègue à `for=`/premier descendant contrôle
  (`first_control`), `<summary>` bascule `open`
  de `<details>`. `disabled` = aucune activation.
  `window.open(url)` câblé (`set_open_hook` →
  `NewTab`). Bornes : `type=reset`/file/color
  inertes, `window.open` lit seulement `url`
  (retour `Null`, target/features ignorés).
- [x] **CAP-152. `location.replace`/`location.reload`** —
  *(numéro officiel attribué à l'intégration)* : les
  sémantiques `historyHandling` de `location.*`
  (utilisées par les flows auth — une étape de
  sign-in ne doit pas rester dans l'historique).
  Hooks additifs `navigate_replace`/`navigate_reload`
  (jscore) → variantes `NavRequest::Replace`/`Reload`
  (explorer-js) → `ScriptNav`/`DrainAct` dédiés :
  **replace** = fetch réel puis réécriture de
  `hist[hist_idx]` par la requête commise (`back`
  saute la page remplacée ; `same_doc` retombe à
  faux — document neuf, tête de run) ; **reload**
  = rejeu de `last_req` (méthode + corps — POST
  re-POSTe) à la position courante + règle
  d'erreur de `reload` public (succès → entrée
  redevient saine, étape 76). Deux drains couverts
  (`drain_script_nav` exclusif +
  `drain_navs_shared` sous verrou). Borne :
  `location.assign(même URL)` reste un push
  (spec `navigate` — Chrome diffère peut-être).
- [x] **CAP-151. History API `pushState`/`replaceState`** —
  *(numéro officiel attribué à l'intégration)* :
  `history.pushState`/`replaceState` remontent à la
  session via `NavRequest::PushState` (additif,
  drain existant) — résolution contre la **base
  effective** du document (`<base href>` honoré,
  `nav_base`), **same-origin exigée** (refus
  silencieux côté moteur *et* session — spec
  `SecurityError`, tolérant). `push` = nouvelle
  entrée, `replace` = réécriture de l'entrée
  courante ; pendant `hist_pending`, les mutations
  sont différées (`pending_push`/`pending_replace`)
  et drainées par `sub_commit_tail` au-dessus de
  l'entrée de navigation. `entry.url`, `last_req`
  (reload/back) et le contexte JS suivent dès le
  drain — `location.href` reflète la résolution
  dès le même script (`set_page_url` local), puis
  `JsEngine::sync_ctx_url` (trait additif) recanonise
  et aligne `history.length`. Entrées marquées
  `same_doc` : elles partagent le document commis
  de la tête de « run » — `back`/`forward` à
  l'intérieur d'un run = **swap d'URL, zéro
  réseau** (modèle Chrome ; entrée hors run →
  bfcache/rejeu classique). Navigation fragment
  marquée `same_doc` de même. Bornes : `popstate`
  non dispatché, état objet non stocké, run
  quitté puis rejoint → rejeu de la requête
  historiée (le document n'est pas persisté par
  entrée).
- [x] **CAP-150. Overrides du submitter** —
  `formaction`/`formmethod`/`formenctype`/
  `formtarget` *(numéro officiel attribué à
  l'intégration)* : le bouton cliqué
  (`<input type=submit>`/`<button>`) prime
  les attributs du `<form>` (HTML §4.10.21 —
  formulaires multi-actions : « Enregistrer »
  vs « Supprimer »). Chaîne de cible :
  `formtarget` > `target` du form > `<base
  target>` (`target_name_new_context` factorisé
  depuis `target_new_context`).
  `formmethod="dialog"` = pas de soumission
  (fermeture de `<dialog>` — hors périmètre,
  intention morte). `formnovalidate` sans
  effet (pas de validation à cette tranche).
  Tests : 5 session (`submitter_*`).

- [x] **CAP-149. `<iframe sandbox>`** —
  restrictions de contexte *(numéro officiel
  attribué à l'intégration)* : attribut
  `sandbox` collecté à la vague (`collect_iframes`
  → `SubPlan` → `SubTag::Frame` indexé →
  `SubFrame::sandbox` → `FrameCtx`) — `None` =
  plein droit, présent = tout jeton absent est
  une capacité **refusée** (défaut spec).
  `allow-scripts` : sans lui le realm enfant
  existe (miroir, `contentWindow`) mais aucun
  script n'est évalué. `allow-same-origin` :
  sans lui le contexte vit en **origine
  opaque** — `Initiator::Opaque` (net,
  additif) → `Sec-Fetch-Site: cross-site`,
  `Origin: null` (sérialisation spec), cookies
  `Lax`/`Strict` retenus et `document.cookie`
  muet, `localStorage` sous clé éphémère
  `opaque:{ctx}` jamais persistée ; `'self'`
  CSP n'étant pas une URL http(s), évalué
  contre `about:blank`. `srcdoc` sandboxé
  couvert. `allow-popups`/`allow-forms`/
  `allow-top-navigation` retenus sur le
  drapeau (bornes : aucun chemin popup/form/
  top-nav depuis un cadre à cette tranche).
  Tests : 6 session (`iframe_sandbox_*`,
  `iframe_srcdoc_sandbox_*`, témoin
  `iframe_plain_fetch_same_origin_baseline`).

- [x] **CAP-148. `<a download>`** —
  téléchargement forcé *(numéro
  officiel attribué à
  l'intégration)* : attribut
  `download` sur `<a>` — clic
  dispatché → `Request::
  download_hint` (additif,
  valeur = suggestion de nom,
  `""` = aucune) → `NavJob`
  force la voie `PendingDownload`
  même pour un type affichable
  (`text/html` téléchargé, pas
  peint). Garde spec : honoré
  seulement **same-origin** avec
  le document — cross-origin =
  attribut ignoré (navigation
  normale). `download` prime
  `target` (pas d'onglet pour
  un fichier). Priorité du nom
  (spec « downloading
  resources ») : `filename=` du
  `Content-Disposition` >
  attribut > segment d'URL >
  `download` — sanitisé comme
  toujours. Borne : `<a
  download>` depuis un script
  (`a.click()`) suit le drain
  `ScriptNav::To` qui ne porte
  pas le hint — piste JS quand
  elle implémentera
  `HTMLAnchorElement::click`.
  Tests : 4 session
  (`a_download_*`).

- [x] **CAP-147. `Clear-Site-Data`
  — cookies** *(numéro officiel
  attribué à l'intégration)* :
  en-tête de réponse de logout —
  `"cookies"` et `"*"` vident le
  jar du **site** (domaine
  registrable de l'URL résolue,
  sous-domaines inclus) au
  commit de navigation, après
  le `Set-Cookie` éventuel de la
  même réponse (spec : tout
  part, même le fraîchement
  posé). Nouveau `CookieJar::
  clear_site(host)` : cookies
  host/domaine **et** seaux
  CHIPS — WICG §cookies. Le
  `CookieTransport` capture les
  `Set-Cookie` par hop avant le
  vidage : la page de logout
  tourne sans identifiants, les
  fetches suivants (page servie)
  n'envoient plus rien. Bornes
  documentées : `"cache"`,
  `"storage"`, `"execution-
  Contexts"` non implémentés
  (cache disque côté pile hôte,
  storage JS absent — traités
  comme inconnus = ignorés).
  Tests : 3 session
  (`clear_site_data_*`).

- [x] **CAP-146. Client Hints
  basse-entropie** *(numéro
  officiel attribué à
  l'intégration)* :
  `policy::ClientHints` +
  `ClientHintsTransport`
  (décorateur, patron
  `AcceptTransport`) émettent
  `Sec-CH-UA` (`"Explorer";v="0",
  "Not/A)Brand";v="24"` —
  branding honnête + marque
  GREASE), `Sec-CH-UA-Mobile`
  (`?0`/`?1`) et
  `Sec-CH-UA-Platform` (OS
  hôte mappé, omis hors liste
  connue) sur **chaque**
  requête http(s) — chaque
  en-tête posé seulement si
  absent (valeur auteur ou
  branding hôte interne
  préservés), rien hors
  http(s). Câblé dans
  `Session::with_config` sous
  `Cors` (preflights couverts),
  au-dessus d'`Accept` —
  configurable côté hôte sans
  changement core (les siens
  priment). Hints haute
  entropie (`-Platform-Version`,
  `-Arch`, `-Bitness`, `-Model`,
  `-Full-Version-List`) non
  implémentés — jamais émis
  non sollicités. Surface JS
  `navigator.userAgentData`
  restée à la piste JS.
  Tests : 3 policy + 1 session
  (`client_hints_*`).

- [x] **CAP-145. Content-
  Security-Policy —
  enforcement** *(numéro
  officiel attribué à
  l'intégration)* : nouveau
  module `explorer-net::csp`
  — parseur `Content-
  Security-Policy` (toutes
  les valeurs d'en-tête +
  `<meta http-equiv>` :
  chaque politique s'applique,
  la plus restrictive réduit ;
  `frame-ancestors`/`sandbox`/
  `report-*` écartées en méta
  par spec) + évaluateurs.
  Directives : `default-src`,
  `script-src`, `style-src`,
  `img-src`, `font-src`,
  `connect-src`, `frame-src`
  (repli `child-src`),
  `form-action`, `base-uri`
  (sans repli), `upgrade-
  insecure-requests` (`http:`
  → `https:`, `ws:` → `wss:`).
  Enforcement à tous les
  points : plan (scripts,
  feuilles, `<img>`, cadres —
  `srcdoc` testé comme origine
  héritée `'self'`), inline
  (`<script>`/`<style>` :
  `'unsafe-inline'` + `nonce`,
  bypass nonce/hash spec),
  fetchs tardifs (`@import`
  récursif, fonds/masques,
  fonts, dépendances de
  modules), `fetch()`/`XHR`
  (`connect-src`), soumissions
  de formulaire (`form-action`,
  DOM + script), `<base href>`
  (base refusée = ignorée).
  CSP portée : `NavDone` →
  `Tab` → `PaintPayload` (re-
  filtrage `<style>` à chaque
  cascade) → `SubJob` (fetchs
  tardifs) → `PageSnapshot`
  (gel/restauration bfcache).
  Bornes documentées :
  `Report-Only` ignoré,
  `sha*-` non-matching (pas de
  SHA-2), `*-src-elem`/`-attr`
  et `strict-dynamic` non
  implémentés (fail-closed),
  `object-src`/`media-src`/
  `worker-src`/`navigate-to`
  hors périmètre (pas de
  fetch `<object>`/média/
  worker en moteur).
  Tests : 18 `tests/csp.rs`
  (parseur/matching/inline/
  upgrade) + 14 session
  (`csp_*` — scripts, styles,
  img, fonte, cadres, srcdoc,
  form-action, base-uri,
  multi-en-têtes, méta,
  `@import`, `connect-src`).

- [x] **CAP-144. `Refresh:`
  en-tête de réponse**
  *(numéro officiel attribué
  à l'intégration)* : le
  pragma HTTP alimente la
  même échéance que la méta
  (`NavDone.refresh`) —
  **traité avant le
  document** (spec : la
  réponse prime) ; première
  valeur valide gagne, une
  invalide retombe sur la
  méta. Grammaire partagée
  `parse_refresh_content`
  passée `pub(crate)`.
  Tests : `refresh_header_*`
  (3 : tir, priorité sur la
  méta, invalide → repli
  méta).

- [x] **CAP-143. `<meta
  http-equiv=refresh>`**
  *(numéro officiel attribué
  à l'intégration)* :
  navigation **programmée par
  le document**, hors-JS —
  première méta valide en
  ordre document gagne (une
  morte n'occupe pas le
  slot) ; grammaire `N[;,]
  [url=]U` tolérante (casse,
  espaces, quotes), cible
  résolue sur la base, sans
  `url` = rechargement.
  `NavDone.refresh` porte
  `(délai, cible)` →
  `Tab.refresh_at` recalé au
  commit ; le tick
  (`pump_timers`, avant le
  gate moteur) transforme
  l'échéance échue en
  `DrainAct::Req` — drain
  habituel. bfcache : le gel
  conserve le **délai
  restant**, la restauration
  recale l'échéance. Rejeu
  download programmé idem.
  Tests : `meta_refresh_*`
  (5 : tir, rechargement,
  non-échue, quotes/casse,
  méta morte sautée).

- [x] **CAP-142. `<iframe
  srcdoc>`** *(numéro officiel
  attribué à l'intégration —
  convention CAP-xxx)* :
  document **inline**
  prioritaire sur `src` quand
  présent, même vide (spec
  HTML) — aucun fetch, le HTML
  de l'attribut est parsé en
  `SubFrame` dont l'URL est
  `about:srcdoc` (origine
  opaque) ; dédup par contenu
  (deux cadres au même HTML =
  un seul contexte), borne
  `max_frames` partagée avec
  les `src`. Association
  boîte ↔ contexte par
  contenu côté composite
  (clé de cache = hash du
  HTML — tous les
  `about:srcdoc` partagent
  leur URL) ; l'enfant hérite
  la base du parent.
  Tests : `iframe_srcdoc_*`
  (4 : peinture, priorité sur
  `src`, vide → blanc opaque,
  dédup).

- [x] **CAP-141. `Accept` par
  destination** *(numéro
  officiel attribué à
  l'intégration — compteur
  gelé, convention CAP-xxx)* :
  `AcceptTransport` (net,
  pile session tout au fond —
  preflights `OPTIONS`
  inclus) pose `Accept` par
  `fetch_dest` quand absent :
  `document`/`iframe` →
  `text/html,application/
  xhtml+xml,application/
  xml;q=0.9,*/*;q=0.8`,
  `style` → `text/css,*/*;
  q=0.1`, `image` →
  `image/png,image/svg+xml,
  image/*;q=0.8,*/*;q=0.5`
  (liste **honnête** — pas
  d'avif/webp non décodés),
  reste → `*/*`. Jamais
  d'écrasement d'un `Accept`
  auteur (`fetch(...,
  {headers:{Accept}})`) ;
  hors http(s) rien posé.
  Les CDN/serveurs à
  négociation de contenu
  répondent différemment sans
  cet en-tête. Tests :
  `accept_value_table` +
  `accept_stamped_per_dest_
  and_author_kept` (policy),
  `accept_header_reaches_
  wire_per_destination`
  (session).

- [x] **Étape 135. `<base
  href>`/`<base target>`** :
  premier `<base href>` en
  ordre document fixe la base
  de résolution des URLs
  relatives — sous-ressources
  (`link`/`img`/`script`/
  `<iframe>`/fonts/fonds/
  `mask-image`/specificateurs
  de modules inline), clic
  `<a>`, `<form action>`,
  `location.href` et
  `form.submit()` scriptés,
  `fetch`/`XHR`, médias
  `<video>`/`<audio>`, images
  de la passe de peinture
  détachée. `<base target>`
  fixe la cible par défaut
  des `<a>`/`<form>` sans
  `target` propre. Base
  invalide → repli sur l'URL
  du document ; second
  `<base>` sans effet.
  Distinction conservée :
  `url` = page (initiateur,
  `Referer`, `Origin`,
  mixed-content, historique),
  `base_url` = résolution
  seule — gelée/restaurée
  avec le bfcache, portée par
  `Tab`/`PageSnapshot`/
  `PaintPayload`/`SubJob.
  doc_base`. `NavOutcome`
  boxé (Box<NavDone/NavTail/
  PendingDownload> — croît
  avec les métadonnées de
  document). 8 tests session.

- [x] **Étape 136. Onglet délégué
  WebView2 pour domaines DRM
  (ADR-0027) — livrée 2026-10-25,
  piste agent « media »** : bascule de
  l'onglet Flutter sur une
  WebView2 native Windows pour
  une allowlist de domaines DRM
  (Netflix, myCANAL, Disney+,
  Prime…) — le chrome (onglets,
  omnibox, tokens, historique)
  reste Explorer, le moteur
  n'est jamais contacté sur ces
  domaines (`navigate` REST et
  FFI court-circuités).
  Chantier `app/` pur, aucune
  couture PIPE ; la seam
  `MediaSurface` (étape 71)
  reste la voie du non-DRM.
  Sous-étapes :
  - [x] **136a — plugin retenu :
    `webview_win_floating`
    (BSD-3, HWND natif)** —
    constat : les flux DRM ne
    sont pas capturables → les
    plugins texture
    (`webview_windows`) rendent
    noir ; HWND overlay requis.
    Amendement ADR-0027 + PoC
    `lib/poc_delegated.dart`
    (build windows vert).
  - [x] **136b — allowlist DRM en
    config** :
    `AppConfig.drmDelegateHosts`
    (5 domaines seed arbitrés,
    `amazon.tld` exclu — host
    entier trop large) ; matcher
    `DrmAllowlist` domaine :
    host exact + sous-domaines,
    http/https, URL malformée
    refusée. 6 tests.
  - [x] **136c — interception
    navigate + `TabKind`** :
    kind décidé par URL —
    `delegated` court-circuite
    `_repo.navigate` (REST et
    FFI), re-bascule engine sur
    URL normale ; `DelegatedWebView`
    monté à la place de la frame,
    registre contrôleurs, routage
    back/forward/reload WebView2,
    gardes pull/close. 3 tests.
  - [x] **136d — surface
    persistante + user-data-folder** :
    webviews toutes montées (état
    conservé entre onglets),
    bascule `setVisibility` ;
    `userDataFolder` =
    `<profileDir>/webview2` —
    sessions hors profil Explorer.
  - [x] **136e — resync
    navigation** : délégué WebView2
    → `BrowserTab` (loading, url,
    titre, canBack/canForward via
    `canGoBack`/`canGoForward`) ;
    routage back/forward/reload
    déjà posé en 136c. 1 test.
  - [x] **136f — indicateur
    chrome** : icône `live_tv` +
    tooltip « rendu WebView2
    natif » sur l'onglet délégué.
  - [x] **136g — repli honnête
    hors-Windows** : gate
    `Platform.isWindows`
    (override test) — allowlist
    matchée sans WebView2 →
    moteur normal, échec DRM du
    site. 1 test.
  - [x] **136h — gate + doc** :
    10 tests (allowlist 6,
    contrôleur 4) ; `flutter
    analyze` + `flutter test`
    suite complète verte (51) +
    `flutter build windows` vert ;
    ADR-0027 « Acceptée — livrée ».
  Hors périmètre de l'étape :
  MSE/EME moteur — murs nommés
  ADR-0024/0027.

- [ ] **Étape 137. Reliquat ADR-0025 —
  journal de mutations au-delà du
  pilote** (ouverte 2026-10-25,
  § Ouverture de l'ADR) : le journal
  est le chemin nominal depuis
  l'étape 111 ; ce qui reste :
  - [x] **137a — ciblage relayout par
    les ops** : `apply_journal` paie
    encore `relayout_dom_payload`
    complet alors que les ops portent
    la cible exacte → `relayout_
    subtree` direct, sans
    `take_dirty_path` ni remap ;
    unifier avec `job.dirty` (R-12)
    ou le supplanter.
  - [x] **137b — `DomMutation::
    AttachShadow`** : op de couture
    `(host, mode, sous-arbre)` —
    l'attache `attachShadow`
    programmatique voyage encore par
    re-parse (borne R-96) ;
    `Dom::attach_shadow_root` existe,
    consommation pure périmètre NAV.
  - [x] **137c — seuil de repli
    calibré** : mesurer
    `journal_fallbacks` sur vols
    terrain réels, fixer le seuil
    d'investigation de l'ADR.
  - [ ] **137d — simplification
    `img_decoded`/`bg_decoded`
    (optionnelle)** : survie par clé
    devenue inutile sous journal
    nominal ; les caches restent en
    dédup de décodage — confort.

- [x] **Étape 138. Piles élargies +
  capture de crash (mort silencieuse
  daemon x.com, prioritaire)** —
  seul trou non nommé de la file :
  mort daemon intermittente post-clic
  (~25 min de churn JS), zéro trace
  panic, non reproduit en ~55 inputs
  de campagne — signature stack
  overflow probable (récursion
  dispatch/JS ré-entrant) ou kill
  externe. **Deux tiers, tous deux
  safe** : (a) `tokio::Builder::
  thread_stack_size` + `std::thread::
  Builder::stack_size` pour le
  thread moteur (~8→32 Mo, ~15
  lignes) — fix candidat ET test
  d'hypothèse : mort récurrente →
  pile innocente, plus de mort →
  mécanisme confirmé ; (b) **WER
  LocalDumps** — capture externe OS
  (`HKLM\SOFTWARE\Microsoft\Windows\
  Windows Error Reporting\LocalDumps\
  explorer-daemon.exe`, admin une
  fois, note `scripts/vol-agent.md`)
  — tout crash futur écrit un `.dmp`
  complet, survit aux fautes où un
  handler in-process ne tourne plus.
  **Écarté** : SEH/minidump
  in-process — `forbid(unsafe)` +
  fiabilité douteuse sur overflow
  (le handler court sur la pile
  épuisée) ; WER le remplace
  strictement. Régression : test de
  récursion profonde qui survit.

- [x] **Étape 139. Chaîne média —
  même câblage que la référence
  `IptvMediacenter-Rust` (ADR-0024
  amendée)** : porter le pattern
  éprouvé — tout flux de lecture
  passe par un proxy local, puis
  le lecteur hôte. Référence :
  `iptv-infra::network::local_proxy`
  (Axum, streaming borné, headers/
  UA injectés), `hls_transcoder`
  (cast ffprobe→HLS), `MdkPlayerEngine`.
  Sous-étapes :
  - [x] 139a — `LocalMediaProxy`
    posé + testé (`media_proxy.rs` :
    `GET /m?u=` streamé zéro-copie
    `fetch_stream`, Range/headers
    propagés, port éphémère
    daemon+FFI ; 2 tests flux)
  - [x] **139b — seam branchée** :
    `Session` spawn `MediaProxy`
    en tokio dès `with_config`
    (daemon/CLI/FFI), `url`
    d'offre réécrite via
    `proxy.url()` — repli URL
    directe si proxy absent.
  - [ ] 139c — cast HLS (garée) :
    port `hls_transcoder` —
    ffprobe → remux|encode → HLS,
    endpoint `/hls/v1/` ; ouvrir au
    premier besoin cast réel.
  - [ ] 139d — dossier externe
    (garée, ADR-0024 amendée) :
    source fichier via
    `MediaPlayerFactory.file` +
    dossier configurable — même
    seam, entrée non-réseau.
  - [x] 139e — tests flux +
    doc : `proxy_relaye_stream_
    headers_range_et_ua` (chunks
    relayés, Range propagé, UA
    injectée) + `proxy_refuse_
    sans_u` ; clippy net+core
    propre ; ADR-0024 et
    CHANGELOG à jour.

- [x] **Étape 143. Pont
  `adoptedStyleSheets` → cascade**
  (`explorer-core::session` +
  `explorer-jscore` + `explorer-js`)
  — feuilles construites
  (`new CSSStyleSheet` + `insertRule`
  + `adoptedStyleSheets`) cascadées
  en feuilles auteur document :
  `ObjId` au `set` → `adopted_sheets
  _css()` (cssText par feuille) →
  trait `JsEngine` → `drain_script
  _net` → `Tab`/`PageSnapshot`/
  `PaintPayload` → `add_css(Author)`
  après `linked_sheets` (ordre spec).
  Fix inclus : `insertRule` sans
  index → fin (spec, était tête).
  Reliquat : `:defined` matcher
  (PIPE). Test `adopted_sheets_css_
  extracts_rule_texts`.

- [x] **Étape 142. `animationstart`
  synthétique** (`explorer-core::
  session`) — naissances `@keyframes`
  collectées à `sync_animations`,
  transportées par `PaintPayload`
  (détaché) vers `pending_anim_
  births` borné, dispatchées
  `animationstart {animationName,
  elapsedTime, pseudoElement}` par
  `drain_anim_births` aux deux
  battements ; flags spec
  animation/transition (bullants,
  non-annulables, non-composés).
  Découverte lazy-CE github
  (`x(e)` → `m.get(animationName)`)
  couverte pour les insertions
  post-load. Test `animationstart_
  fires_for_new_keyframe_animation`.
  Reliquat : `adoptedStyleSheets`/
  `insertRule` non pontés vers la
  cascade (feuilles JS-pures).

- [x] **Étape 141. Drain dyn-subresources
  dé-famé** (`explorer-core::session`)
  — `drain_script_net` ne tournait que
  via `after_script` (gate `jobs>0`) :
  les `<link>`/`<img>`/`import()`
  émis par la dernière rafale n'étaient
  jamais fetchés, `load`/`error` jamais
  livrés — famine liveness mesurée en
  vol (`<link>` injecté → `load` jamais
  ; post-fix → `LOAD`). Le drain court
  à chaque battement des deux pompes.
  Terrain github : `n.f.css` suspendait
  `c.e` sur `link.onload`. Reliquat :
  define lazy absent — mur côté
  fabrique d'entrée `s(id)`. Test
  `dyn_link_drains_without_pending_jobs`.

- [x] **Étape 140. Cycle de vie doc
  rejoué après pompe JS détachée**
  (`explorer-core::session`) — race
  mesurée au vol github : le tail de
  `sub_commit` pose `readyState
  interactive`/`complete` +
  `readystatechange`/`DOMContent
  Loaded`/`load` pendant que la
  pompe détachée détient le moteur
  (`entry.js=None`) — appels tombés
  en silencieux, jamais rejoués →
  `readyState` figé `"loading"`,
  gate `ready` du lazy-registry
  github pendue, zéro `import()`,
  mort sans erreur. File bornée
  `pending_doc_lifecycle` sur `Tab`
  (pattern `js_pending_*`), rejouée
  à `commit_js_pump`, purgée nav/
  epoch/snapshot. Mesuré :
  `readyState "complete"` tenu, gate
  levée, 24 `import()` lazy (2
  avant), livraisons ok. Reliquat :
  `customElements.define` lazy
  encore absent — entrées `s(id)`
  propres, mur plus profond (chaîne
  `x` catalyst). Test
  `doc_lifecycle_replays_after_
  detached_pump`.

- [ ] **Écarts terrain — vol humain
  22/10 (release `39ab821`, FFI)** :
  second run humain — trois constats
  au journal `interface-equipes`
  (22/10) :
  - ~~**RSS ~10 Go**~~ → **résolu,
    Étape 106** (cause racine au
    changelog — pas `dispose()` :
    `BrowserState` épinglés +
    `ImageCache` sources).
  - ~~**FFI inutilisable sous charge**~~
    → **résolu, Étape 107** : worker
    isolate dédié possède lib+handle —
    les appels natifs synchrones
    (`navigate` jusqu'aux timeouts)
    sortent de l'isolate UI. RPC par
    ports, frames zéro-copie
    (`TransferableTypedData`),
    `tryLoad` inchangé (sonde sync
    + repli REST).
  - ~~**frame ≠ fenêtre**~~ →
    **attribué transitoire** : frame
    figée à sa taille intrinsèque
    (`fit:none`) pendant le cycle
    `set_viewport`→relayout→pull —
    mesuré ~0,8 s release / ~10 s
    debug sur wikipedia.
    **`fit:width` implémenté** : la frame
    retenue est tirée à la largeur
    fenêtre pendant le transit
    (`Transform.scale` sur
    `size.width×dpr/raw.width`, s'annule
    seul à la frame fraîche ;
    `frameShift` + overlays média
    suivent `fZoom`). À re-valider en
    vol humain release (ressenti
    resize).
  - ~~**`Image`/`HTMLImageElement`
    manquant**~~ → **résolu,
    JS-C57** : `new Image()`,
    `new Image(w,h)` + interface
    `HTMLImageElement` (srcset/
    sizes CAP-172) — tests
    wikipedia 22/10.

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

### J-A2 — Inventaire API : gaps énumérables (décidé 2026-10-23)

Contrairement aux trous de sémantique (JS-C59→C65 — dormants,
seul un bundle réel les révèle), les gaps **d'interface** sont
listables à l'avance : chaque global absent est un
`ReferenceError` garanti sur les sites qui l'utilisent.
Inventaire établi sur l'existant `install_dom` + bundles
github fetchés (`ReferenceError` mesurés en sonde) —
**les items J-A2 ne sont pas trigger-conditionnés** : ils
sont commandables dès que la file JS est vide, dans l'ordre
ci-dessous (fréquence observée sur le corpus réel).

- [x] **JS-C66. `fetch` — surface complète** : `Request`,
  `Response`, `Headers` constructibles (aujourd'hui `fetch`
  existe mais `new Headers()` = `ReferenceError` — mesuré en
  sonde github). Mixin `Body` (`text`/`json`/`blob`/
  `arrayBuffer`/`formData`), `Response` natif lisible
  (`status`/`ok`/`headers`), `Request` en entrée de `fetch`.
  **Livré** : trois classes à prototype réel (`instanceof`,
  `for-of` — `@@iterator` sur `Headers.prototype`) ; `Headers`
  = own-props non énumérables (combinaison `", "` en stockage,
  `set-cookie` isolé en prop sentinelle + `getSetCookie`) ;
  `new Request` valide méthode/`GET`+corps/URL résolue contre
  `location` ; `new Response` + `error`/`redirect`/`json`
  statiques ; mixin partagé avec `bodyUsed` (rejet promesse au
  second usage) ; `fetch` accepte `Request` et `Headers` en
  `opts`, `response.url` renseignée. `FetchResponse` →
  `FetchBody` (corps seul). Reliquats assumés : `body` =
  `null` (streams JS-C71), `formData()`/`blob()` minimaux
  (JS-C67/C70), guards `Headers` non appliqués, `credentials`
  etc. stockés non appliqués. Régressions `c66_*` (6 tests).
- [x] **JS-C67. `FormData`** (mesuré `ReferenceError` en
  sonde) + `URLSearchParams` bidirectionnel — formulaires et
  uploads des sites réels (google recherche, github login).
  **Livré** : prototype réel (`instanceof`, `@@iterator`,
  `[object FormData]`), paires `[[k, v], …]` en prop sentinelle
  (doublons + ordre), API complète (`append`/`set`/`delete`/
  `get`/`getAll`/`has`/`forEach`/`entries`/`keys`/`values`).
  `new FormData(form, submitter?)` = champs « successful » du
  miroir (règles de `form.submit()`) ; `new URLSearchParams(fd)`
  via `@@iterator`. `BodyInit` unifié sur `fetch`/`Request`/
  `Response`/`xhr.send` : `FormData` et `URLSearchParams` →
  urlencodé + `Content-Type` (`charset=UTF-8` pour USP).
  `res.formData()` → vrai `FormData` (proto paresseux, valide
  sans DOM). Reliquats assumés : `multipart/form-data` hors
  scope transport `String` (déviation documentée), `File`/`Blob`
  à JS-C70. Régressions `c67_*` (7 tests).
- [x] **JS-C68. `customElements` machinery** : `define` avec
  upgrade des éléments existants, callbacks `connected
  Callback`/`disconnectedCallback`/`attributeChangedCallback`,
  `whenDefined` résolutrice. **Débloque le résidu github
  `turbo-frame.delegate`** (seule erreur honnête restante du
  census — la turbo-frame est un custom element).
  **Livré** : registre `Dom.ce` (defs/upgraded/failed/
  `whenDefined` pendantes) — `define` validé (nom réservé,
  ctor, `prototype`, doublons), `observedAttributes` figé ;
  upgrade préservant le wrapper (`call_value_ctor` sur
  l'élément, slot `ce_upgrade_slot` pour le `super()` de
  `extends HTMLElement` — `HTMLElement` = vraie classe de
  base), état « failed » honnête. Réactions synchrones
  drainées après chaque dispatch host : `connected`/
  `disconnected`/`attributeChanged` (observés + tir initial),
  upgrades à `define`, `createElement(NS)`, `cloneNode`,
  insertion, `innerHTML`, `customElements.upgrade` ;
  `whenDefined` → `undefined`, `get`/`getName` réels, tout
  raciné GC. Reliquat assumé : chaîne `HTMLElement` non
  vérifiée à `define` (échec « failed » à l'upgrade au
  lieu du `NotSupportedError` spec). Régressions `c68_*`
  (7 tests).
- [x] **JS-C69. `IntersectionObserver`/`ResizeObserver`
  fonctionnels** : les stubs actuels sont constructibles mais
  le callback n'est jamais livré (documenté « no-op honnête »
  JS-C56) — les sites qui attendent un premier appel
  (lazy-load, infinite scroll) restent muets. Entrées
  minimales : un `IntersectionObserverEntry` initial +
  déclenchement à l'`observe`.
  **Livré** : `observers.rs` (parallèle `mutation.rs`) —
  `observe` enfile la notification initiale livrée au
  checkpoint microtask (ordre de création des observers) ;
  `unobserve`/`disconnect`/`takeRecords` réels, re-observe
  dédupliqué, `cb(entries, observer)`. Entries spec-shape :
  IOE (`isIntersecting` ≙ `is_attached`, `intersectionRatio`
  1|0, `time`, `boundingClientRect`/`intersectionRect`/
  `rootBounds`) et ROE (`contentRect`, `*BoxSize[]`) —
  géométrie honnête règle `clientWidth` (viewport pour
  `documentElement`/`body`, 0 sinon). Options validées et
  lisibles (`thresholds`, `root`, `rootMargin`, `box`) mais
  non appliquées — pas de géométrie réelle à filtrer.
  Callbacks/wrappers racinés GC. Régressions `c69_*`
  (7 tests).
- [x] **JS-C70. `Blob`/`File`/`FileReader` + `URL.
  createObjectURL`** — uploads et traitement client-side.
  `Blob` (parts string/AB/TA/DV/Blob, `size`/`type`/`slice`
  partagé, `text`/`arrayBuffer` promesses) ; `File` (`name`,
  `lastModified`, proto sous `Blob`) ; `FileReader` complet
  (`readAsText/DataURL/ArrayBuffer/BinaryString`, events
  `loadstart→progress→load→loadend` au checkpoint microtask,
  `abort`+`loadend`, ProgressEvent minimal) ; `URL.create
  ObjectURL`/`revokeObjectURL` — registre `blob:<origin>/<n>`
  résolu par `fetch` en local. Corps `Blob`/`File` dans
  fetch/Request/Response/XHR (octets→String lossy, transport
  `String` — limite documentée), `res.blob()` réel,
  `FormData` filename/3e arg, `structuredClone`. GC :
  protos/URL/readers racinés. Régressions `c70_*` (7 tests).
- [x] **JS-C71. `ReadableStream`/`WritableStream`/`Transform
  Stream`** — requis par `Response.body` moderne et les
  bundlers (webpack dev-server, SSR-streaming). Queues
  mémoire + `underlyingSource`/`underlyingSink`/`transformer`
  (`start` synchrone, `pull`/`transform`/`flush` appelés) ;
  `read()`/`write()`/`close()`/`abort()` en promesses
  `{value,done}`/`undefined`, pendantes livrées au checkpoint
  microtask ; `[Symbol.asyncIterator]`, `tee`, `pipeTo`/
  `pipeThrough` (drain synchrone — pas de backpressure,
  documenté), `locked`/`desiredSize`, controllers spec.
  `res.body`/`req.body`/`blob.stream()` = stream d'un chunk
  `Uint8Array` (transport `String`). Régressions `c71_*`
  (6 tests).
- [x] **JS-C72. `indexedDB` + `caches` (Cache API)** — storage
  persistant des apps (Service Worker hors scope : la
  persistence suffit à la plupart des fallbacks gracieux).
  Registres `Rc<RefCell>` injectables (`install_idb`/
  `install_caches`, même couture que `localStorage`) : `open`
  → `IDBOpenDBRequest` (`upgradeneeded` → `success`,
  checkpoint microtask), `deleteDatabase`/`databases`/`cmp` ;
  txns `readonly`/`readwrite`/`versionchange` + `oncomplete` ;
  stores `put`/`add`/`get`/`getAll`/`delete`/`clear`/`count`/
  `openCursor` (valeurs structured-clonées) ; `keyPath`,
  `autoIncrement`, index `unique`/`multiEntry` + `IDBKeyRange` ;
  curseurs `continue`/`advance`/`update`/`delete` (request
  re-tirée — spec). `caches` : `CacheStorage` `open`/`has`/
  `delete`/`keys`/`match` + `Cache` `put`/`match`/`matchAll`/
  `keys`/`delete`/`add`/`addAll` (via transport fetch — rejet
  honnête sans). Fix promesse au passage : adoption pendante
  résolvait `undefined` (résolveurs de la mauvaise promesse).
  Régressions `c72_*` (8 tests).
- [x] **JS-C73. `WebSocket` stub honnête** — `src/websocket.rs`
  : handshake refusé en async au checkpoint microtask
  (`error` puis `close` `{code: 1006, wasClean: false}` — la
  séquence observable d'un vrai refus, qui arme les fallbacks
  long-polling ; aucune socket ouverte). Surface spec :
  `readyState`, `url` (`ws`/`wss` exigés), `protocol`/
  `extensions`, `bufferedAmount`, `binaryType` validé,
  `send` (`InvalidStateError` en `CONNECTING`),
  `close(code, reason)` validé, `on*` + `addEventListener`/
  `removeEventListener`/`dispatchEvent`, constantes ctor +
  proto, `protocols` validé (tokens/doublons →
  `SyntaxError`). Régressions `c73_*` (5 tests).
- [x] **JS-C74. `Notification`/`BroadcastChannel`/
  `navigator.sendBeacon`/`getSelection`/`reportError`/
  `scrollIntoView`** — `src/misc_apis.rs` : BroadcastChannel
  **réel** intra-contexte (clone par destinataire, jamais
  soi-même, `close` → `InvalidStateError`) ; `Notification`
  stub honnête (`permission` `'default'`,
  `requestPermission` → `'denied'`, instances inertes) ;
  `getSelection` singleton réel (ranges, `selectAllChildren`,
  `toString` = `textContent`, `getRangeAt`) ; `reportError`
  réel (`window.onerror` legacy + `ErrorEvent` propagé +
  `console` embedder `Uncaught:`) ; `scrollIntoView` no-op
  honnête sur `Element.prototype` ; `sendBeacon` existait
  déjà. Régressions `c74_*` (6 tests).
- [~] **JS-C75. `attachShadow`/Shadow DOM** — le trou le
  plus structurel (scoped CSS + slots) : old.reddit et MDN
  l'utilisent. **Découpé en tranches T1→T6** (vol 2026-10-25,
  répartition actée dans `docs/interface-equipes.md`) :
  **T1** surface sans composition — livré (CAP-191/192) ;
  **T2** scoping CSS → PIPE (demande posée au journal) ;
  **T3** slots/distribution + **T5** `shadowrootmode` →
  piste JS — **livré (CAP-197, miroir flat)** ;
  **T4** event retargeting + **T6** (stubs
  terrain `adoptedStyleSheets`/`StyleHost position`) → NAV —
  **T4 livré (CAP-198)**, **T6 livré (CAP-195/196)**.
  *Coût architectural, pas un stub.*

### J-B — Couverture langue par besoin (réactive)

**Règle du palier — aucune de ces étapes ne s'ouvre sans un
déclencheur documenté** : échec `site_smoke`, bug d'un site
réel, ou besoin d'une étape produit. Le % de couverture n'est
jamais un déclencheur (état des lieux §2 — limites des
mesures).

**Déclencheurs accumulés** : ~~lobste.rs — `TypeError: value
is not a function` ×4~~ — **soldé** (JS-C54, reproduit propre
par sonde le 22/10 : 0 PipelineError) ; ~~example.com —
`TypeError` sur `s.js`~~ — **soldé** (JS-C51, sonde 22/10 :
0 erreur, 1 sync miroir = injection svg réussie) ;
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
- [x] **JS-C34. Générateurs lazy + `await` pending** : refacto
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
  `#[ignore]` documentés. **Livré (vol 2026-10-25)** :
  suspension réelle `GExec`/`GenLive` (entrée piste
  produit ci-dessus), fixtures `c34_*` dé-ignorées vertes.
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
  - **dev.to (vague 3)** : la sonde pendait >10 min.
    Chaîne de diagnostic : fetches tous finis à +1 s
    (pas réseau), ~1 cœur CPU stable (boucle de calcul,
    pas attente), `lex` 35 ms / `parse` gelé en census
    isolé — **le parseur**, pas l'eval ni les timers.
    Cause : la spéculation `(params) =>` re-parsait
    chaque `(` en entier — la chaîne `n=(n=(n=…` de
    `strftime` (~60 parens, base.js) l'explosait
    exponentiellement. Fix : `close_of`, table O(n) des
    délimiteurs appariés — la spéculation vérifie `=>`
    après le `)` en O(1) sans re-parse. base.js :
    >10 min → **41 ms** ; sonde dev.to complète
    (nav + pompes + survol) : **12,3 s**, 0 crash.
    Après-delta : `%EvalError%`/`%Window%` absents →
    `EvalError`/`RangeError`/`URIError` constructeurs
    complets (proto chaîné `Error.prototype`), `Window`
    stub inconstructible ; `str[Symbol.iterator]()`
    appelé directement (homePage) — `iter_values`
    fast-pathait les primitives sans exposer la méthode :
    `String.prototype[Symbol.iterator]` ajouté,
    itérateur de code points.
  - **Résidu honnête** : `react` bare specifier — le
    registry GitHub résout `react` vers
    `assets/react` (React réel, hors périmètre miroir) ;
    `value is not a function` à
    `app-foundation` (`useSyncExternalStore` — aval du
    `react` manquant) ; dev.to : `paginate` →
    `userData()` (symbole cross-bundle inline absent),
    `then` of Undefined + `object is not a function`
    en tête d'`application.js`/`followButtons` (aval
    webpack runtime, attribution non isolée) ;
    `/keys` : 404 hors session authentifiée.
  Tests : `mutation_observer_document_target`,
  `dom_interface_constructors`, `custom_elements_registry`,
  `regexp_quantified_lookahead`, `class_empty_elements`,
  `template_literals` (quotes en texte brut),
  `symbol_cles_propres_to_property_key`,
  `stack_attribue_la_source_par_frame`,
  `document_fragment_splice_et_serialise`,
  `document_location_egale_window`,
  `hex_literals_geants_convertissent_en_f64`,
  `chaine_parenthesee_strftime_ne_reexplose_pas`,
  `sous_classes_erreur_completes`,
  `string_symbol_iterator_appel_direct`.

- [x] **JS-C57. `Image`/`HTMLImageElement`** (trigger vol
  humain 22/10 — `ReferenceError: Image is not defined` sur
  `www.wikipedia.org`, `index-*.js` `doWhenReady` qui
  précharge via `new Image()`) : `Image` devient un
  constructeur réel — alias de `document.createElement
  ('img')` (nœud détaché du miroir, insertable,
  `parentNode` `null`) + attributs `width`/`height`
  posés depuis les args optionnels (numériques ou
  chaînes) ; `HTMLImageElement` rejoint le registre de
  stubs inconstructibles pour la détection. Sans
  `document` installé : `TypeError` honnête. Mesure
  terrain : sonde `LOBSTE_PROBE_URL` sur
  `www.wikipedia.org` — **0 PipelineError** (nav +
  survol + clavier, le bundle `index-*.js` évalue
  `doWhenReady` en entier, 2 syncs miroir 244 Kio)
  vs `ReferenceError` observé en vol. Test :
  `image_constructor_creates_detached_img` (tagName
  IMG, parent null, attrs 100x50, insertable, stub
  inconstructible).

- [x] **JS-C58. Census post-Étape 108 — six trous mini-DOM
  mesurés** (sonde `LOBSTE_PROBE_URL`, batterie
  wikipedia/github/dev.to/bbc — wikipedia reste à 0,
  bbc quasi-propre (1 throw défensif du site :
  `Load full version of piano SDK`, détection
  feature, pas un gap moteur)) :
  - **`?.` → `Host::invoke`** (dev.to `paginate` :
    `localStorage?.getItem("current_feed")` levait
    `TypeError: value is not a function`) — `eval_chain`
    lisait la prop membre sans consulter `invoke` ;
    le step `Call` suit désormais l'ordre de
    `Callee::Member` (host invoke d'abord, valeur
    lue ensuite), clé membre suivie via `recv_key`.
    Couvre `h?.m()`, `h?.x.m()`, `h?.[k]()` pour
    *tous* les hosts (Storage, location, elements…).
  - **`document` hérite `Document.prototype`** (dev.to
    `application.js` : `Document.prototype.ready =
    new Promise(…)` puis `document.ready.then` →
    `cannot read 'then' of Undefined` — le patch du
    proto tombait dans un objet orphelin) — le proto
    du host `document` pointe le `prototype` du stub
    ; `document instanceof Document` devient vrai.
  - **Famille `HTML*Element` complète** (github.com :
    `ReferenceError: HTMLButtonElement is not defined`
    dans `t2-*.js`) — les ~70 interfaces spec rejoignent
    le registre de stubs inconstructibles (fin du
    whack-a-mole membre par membre).
  - **`createElementNS`** (github.com `z8-*.js` :
    `` `createElementNS` is not a function (on
    HTMLDocument) `` — les bundles SVG l'atteignent
    dès le déblocage des stubs) — même chemin que
    `createElement`, `localName` conservé en
    minuscules, `tagName` uppercase (convention moteur).
  - **`Array.prototype` génériques** (github.com
    `selector-observer` : `this.indexes =
    Object.create(proto.indexes)` puis `.slice(0)`
    sur un objet array-*like* héritant d'un vrai
    tableau → `array method called on non-array`) —
    `this_array_like` : `length` + `Get` par indice à
    travers la chaîne proto (spec : `slice`/`concat`
    sont intentionnellement génériques), borne
    `length` 1 M alignée sur `array_concat`, `tick`
    budgété. `concat` préserve les *trous* (spec
    `FlattenIntoArray` via `HasProperty` — ne
    densifie pas en `undefined`).
  - **`<template>.content`** (github.com
    `github-elements` : `h.content.appendChild(m)` →
    `cannot read 'appendChild' of Undefined`) —
    `Dom::template_contents` : fragment paresseux
    par élément template ; la lecture `.content`
    *déplace* les enfants parsés dans le fragment
    (spec `template contents` — `childNodes` de
    l'élément passe à 0), `innerHTML` lit/écrit le
    fragment, le sérialiseur descend dans le
    contenu. `NodeKind::Fragment` existant réutilisé
    (splice `append`/`insertBefore` intact).
  Mesure terrain : dev.to **3 → 1 PipelineError**
  (`paginate` et `document.ready` morts, syncs miroir
  2→3 — plus de JS s'exécute ; le résidu `U8` est le
  shim `createContext` react-dom — aval React réel,
  backlog documenté), github **5 → 2**
  (`HTMLButtonElement`, `createElementNS`,
  `selector-observer` et `template.content` morts ;
  il ne reste que les deux `react` bare specifier —
  résidu documenté, React réel hors périmètre).
  Repro `census` offline : `application.js` 678 Kio
  évalué **en entier** (vs échec d'init). Tests :
  `storage_opt_chain_method_dispatch` (`?.` sur
  méthodes host, chaînes mixtes, base nullish),
  `document_inherits_document_prototype` (patch
  visible, `instanceof`, priorité own > proto),
  `html_element_interfaces_detectable` (famille,
  inconstructibles), `create_element_ns_creates_
  local_name`, `array_slice_concat_generic_on_array_
  like` (héritage proto, `slice`/`concat`/`filter`),
  `template_content_is_fragment` (fragment, déplace
  enfants, identité stable, markup parsé).

- [x] **JS-C59. Surface des events dispatchés — la
  seconde implémentation d'`Event`** (trou remonté
  par github.com après Étape 114 : la résolution
  import-map réparée laisse les modules React
  s'évaluer assez loin pour frapper
  `TypeError: composedPath is not a function (on
  Event)` ×3 dans `ser-*.js` — EventHost dispatché
  n'avait que 3 méthodes là où `Event.prototype`
  des events script-créés en a 5 ; même pattern
  « deux chemins, deux surfaces » que la saga
  fonte) :
  - **`composedPath` réel** — chaîne
    `target → ancêtres → #document` via
    `Dom::ancestors` (ordre spec, `[0]` = cible) ;
    `[]` hors dispatch (`currentTarget` null).
  - **`eventPhase` réel** — `Cell` posé par
    `propagate_event` : 1 capture / 2 at-target /
    3 bubble / 0 hors dispatch ; `currentTarget`
    repart à `null` en fin de marche (spec).
  - **`stopPropagation` rectifié au passage** —
    il tuait aussi les listeners *frères* du
    nœud courant ; désormais seul
    **`stopImmediatePropagation`** (nouveau,
    flag `immediate` distinct) le fait, spec.
  - **`preventDefault` honoré par
    `cancelable`** — le dict `EventInit` (déjà
    lu au constructeur) est relu à
    `event_state_of` ; no-op spec sans flag.
    `dispatchEvent` rend `!defaultPrevented`.
  - **`isTrusted` distingué** — plateforme
    (`dom_event*`, media) vrai, script
    (`dispatchEvent`, `el.click()`) faux.
  - **`timeStamp`** = `Instant` depuis la
    construction de l'état (ms, pas epoch).
  - **`composed`/`cancelable` lisibles** +
    `detail` recopié (CustomEvent) ;
    `cancelBubble = true` ≡ `stopPropagation`,
    `returnValue = false` ≡ `preventDefault`
    (legacy DOM0, `Host::set`).
  - Convention moteur confirmée par sonde :
    méthodes host = `invoke`-only, `typeof` →
    `undefined` partout (`addEventListener`…)
    — la discrimination se mesure à l'*appel*.
  - **`Element.prototype` porte les méthodes en
    *valeur*** — au re-vol github, les ×3
    `composedPath` disparus laissent `call of
    Undefined` dans `selector-observer` (`o =
    Element.prototype.matches || …Selector`
    → `o.call`) : le shim lit la *référence*
    sur le proto, pas l'appel — or les méthodes
    host invoke-only sont invisibles à la
    lecture. `matches`/`closest` posés en
    natives receveur-élément (`TypeError` sur
    receveur non-Element, spec) + alias
    `matchesSelector`/`webkit`/`moz`/`o`/`ms`
    au bras invoke des éléments.
  Mesure sonde `zz_gap_probe` : `4|main` chemin
  complet, frère +10 survit à `stopPropagation`
  (11) mais meurt à `stopImmediatePropagation`
  (1), `preventDefault` `false|true` selon
  `cancelable`. Régression
  `event_dispatche_surface_honnete` (4 blocs).
  Résidu noté : le proto `Event` script-créé
  ignore `cancelable` dans `preventDefault`
  (déviation pré-existante, autre chemin).

- [x] **JS-C60. `/` après `)` de clause =
  regex** (trou remonté par github.com après
  Étape 114 : `SyntaxError at 2:56320`
  affichée `module «landing-pages-*.js»` —
  bissection réelle : le coupable est la dep
  `ns-*.js`, `if (o<128)
  /[!#$&-;=?-Z_a-z~]/.test(…)` ; le nom de
  l'entrée masquait la source fautive) :
  - **Lexeur** : pile `clause_parens` — le
    token précédant chaque `(` est mémorisé ;
    au `)`, `if`/`while`/`for`/`switch`/
    `with`/`catch` réarment `expect_operand`
    → `/` = regex. Spec : la clause fermée,
    ce qui suit est un statement, la division
    y est impossible. `return (x)/2`,
    `(expr)/n`, appels `f(x)/n` restent
    divisions (parens de groupe — `return`
    & co. hors `clause_kw`).
  - **Diagnostic** : `instantiate` suffixe
    `SyntaxError` du `spec` fautif
    (`— «ns-*.js»`) — `js_glue` préfixe le
    nom de l'*entrée* ; sans suffixe, la
    source réelle est introuvable dans un
    graphe de ~50 chunks webpack.
  - Déniais de mesure : `(0,o.m)()`
    innocenté, fetch byte-identique, fichier
    100 % ASCII, source saine parsée hors
    session — le défaut était dans la
    disjonction lexicale, pas le transport.
  Régression `regex_after_clause_paren`
  (lang.rs, 5 cas). Vol github : la
  `SyntaxError` disparaît — résidus
  `sym1` (catalyst, receveur undefined en
  frontière native) + `set is not a
  function (on Object)` (nouveau, modules
  allant plus loin).

- [x] **JS-C61. Portée de bloc réelle —
  `EnvKind::Block`** (une seule cause
  racine pour les DEUX résidus de
  JS-C60 — mesure du graphe réel
  `behaviors-*.js` + deps sur disque,
  stack complète `P@catalyst ←
  c@runtime-helpers` = `tslib.__decorate`) :
  - **Chaîne de causalité mesurée** —
    `let`/`const` sans portée de bloc
    liaient dans l'env *fonction* :
    1ᵉʳ appel `_(A)`, `let e = A[O]` =
    `undefined` **écrasait le paramètre
    `e`** → `o.set(undefined, Set)` →
    map `O` à clé `undefined` ; 2ᵉ
    appel `_(B)` héritant la map →
    `for (let [t,n] of e)` **écrasait
    le paramètre `t`** → `t = undefined`
    → `let o = t[O]` → `TypeError:
    cannot read 'sym2' of Undefined`.
    Instrumentation du bundle par
    try/catch étiquetés (`READ3`)
    jusqu'au statement exact.
  - **`set is not a function` mort
    de la même cause** — `4u-*.js`
    évalue proprement : une liaison de
    boucle écrasait le détenteur du
    `Map`/`Set`.
  - **`EnvKind::Block`** — record
    déclaratif identique à `Local`
    pour `let`/`const`/lectures,
    traversé par `var`/déclarations de
    fonction (`env_declare` monte,
    spec VarDeclaredNames) ; `with`
    inchangé. `Env::block(parent)`.
  - **Sites couverts** — `Stmt::Block`,
    `try`/`finally`, `switch` (UN env
    partagé entre `case`, spec),
    `catch` (param en liaison lexicale
    — `bind_pat Var` traverserait le
    bloc), têtes `for`/`for-in`/`for-of`
    lexicales et `using`.
  - **`for (let i;;)` — env par
    itération, copie post-corps** :
    cellules fraîches recréées entre
    corps et `update` (spec
    CreatePerIterationEnvironment) —
    les closures capturent LEUR `i`
    (`0,1,2`, pas `1,2,3` : `i++`
    mute la cellule du tour suivant).
  - **`history.*` en *valeur*** —
    `get` rend les natives
    `pushState`/`replaceState`/`back`/
    `forward`/`go` (le wrap turbo
    `e.call(history,…)` de github lit
    la référence, invoke-only la rend
    invisible) — logique partagée
    avec `invoke`, `this` sans effet.
  Régression `block_scoping` (lang.rs —
  shadowing param, fuites de bloc,
  `var` traversant, closures par
  itération, motif catalyst réduit) +
  cas `.call(history,…)` dans
  `window_history` (dom.rs). Sonde
  réelle : le graphe `behaviors`
  traverse `__decorate`/`P`/`_` et
  meurt sur `client-env` absent —
  donnée applicative de la page
  réelle (artefact de fixture,
  borne honnête du miroir).

- [x] **JS-C62. Census github.com, reliquat aval —
  trois gaps mesurés** (`jprobe` réel, repro isolé
  avant fix — 2 erreurs → 1) : `o.m?.()` préserve
  le récepteur (membre plié dans la tête `Chain`
  migré au premier `?.` — polyfill popover
  `e.matches?.()`) ; **prototypes itérateurs
  concrets** `%Array/Map/Set/String/RegExpString/
  URLSearchParams Iterator%` + `%IteratorHelper
  Prototype%` (cause racine du `get` of Undefined
  zod — le polyfill ser.js résolvait
  `Iterator.prototype` en `Object.prototype` et y
  installait `map`/`filter` énumérables via
  `Object.assign`, poison `for in` global) ;
  `Object.getPrototypeOf` `ToObject` spec
  (nullish → `TypeError`, primitives → proto).
  Résidu honnête : `turbo-frame.delegate` —
  machinery custom-elements manquante (backlog).
  Tests : `iterator_concrete_protos`, `?.()`
  récepteur (lang + dom), `getPrototypeOf`
  `ToObject`.

- [x] **JS-C63. Census github.com, nom propre
  des class expressions** (`jprobe` réel, repro
  isolé avant fix — 2 erreurs → 1) : la liaison
  immuable `NomClasse → ctor` du scope de
  classe (spec `ClassDefinitionEvaluation`)
  n'existait pas — la déclaration survivait via
  son binding externe, l'expression `var a =
  class e {}` résolvait `e` ailleurs →
  `constructor is not a function` mesuré dans
  `primer-react-*.js` (param `e` exports des
  factories webpack masqué). `env_put(&cenv,
  name, ctor)` : visible des méthodes/
  statiques/champs/blocs statiques, masque
  toute réassignation externe, immuable
  (`TypeError` strict), sans fuite. Résidu
  honnête inchangé : `turbo-frame.delegate`
  (machinery custom-elements, backlog).
  Test : `class_expr_self_name`.

- [x] **JS-C64. `import.meta.url` par module —
  liaison lexicale `IMPORT_META`** (journal
  interface « reliquat `Invalid base URL` » —
  trigger confirmé sur `wp-runtime-*.js` :
  `new URL("./", import.meta.url)`, la
  détection `publicPath` standard de webpack) :
  - **Avant** : `import.meta.url` lisait
    `current_module` global — chaque dep voyait
    l'URL de l'*entrée*, et une microtâche/
    un callback différé voyait `""`
    (`eval_module` le restaure avant
    `run_microtasks`) → `new URL(x, "")` =
    « Invalid base URL » ; `import()` prenait
    aussi `current_module` en référent, les
    deps profondes résolvaient `./x` contre un
    spécificateur relatif brut.
  - **Maintenant** : `instantiate` pose la
    liaison cachée `IMPORT_META` = spec résolu
    dans l'env du module — résolution
    *lexicale* (fonctions, closures, timers)
    comme `SUPER_BIND`/`HOME_BIND` ;
    `ModuleLoader::resolve` (méthode additive,
    défaut `None`) laisse l'hôte rapporter
    l'URL absolue — `MapLoader` (session) la
    rend → `import.meta.url` absolu par module,
    `new URL("./", meta.url)` fonctionne ;
    `import()` reçoit le référent lexical ;
    `evaluate_module` aligne `current_module`
    sur `Module.spec` ; le référent des deps
    est l'URL *résolue* du parent.
  Mesure sonde : dep/callback/lazy-chunk lisent
  `https://site/app/{dep,sub}.js` au lieu de
  l'entrée ; `new URL('a.png', dep.url)` OK.
  Terrain github.com : résidu inchangé
  (turbo-frame honnête). Régression
  `mod_import_meta_url_per_module`.

- [x] **JS-C65. Borne heap armée par défaut
  (ADR-0019)** — crash mesuré : le challenge
  « enablejs » de Google (`/search` →
  `/httpservice/retry/enablejs`, preuve-de-
  travail à flux aplatie `for(D=46;D!=83;)`)
  + `js_ops` relevé à 2 G pour mesure =
  heap en Gio → OOM station. Cause : 3
  couches — `mem_limit` jamais armé
  (`set_mem_limit` appelé nulle part),
  pas de GC mid-eval (par design), ops
  seul frein indirect. `Heap::new` arme
  désormais `DEFAULT_MEM_LIMIT` = 512 Mio
  — kill doux `RangeError` au `tick`, le
  sweep post-kill relève le drapeau et le
  moteur survit ; `set_mem_limit(0)` reste
  le désarmement explicite. Le challenge
  Google reste une lacune honnête (PoW
  taillée pour V8 — insatisfiable sous un
  interpréteur, même classe que le tarpit
  Fnac) : **l'item `Error: b` reste ouvert**.

- [x] **JS-C76. Collections DOM itérables
  — `Error: b` google.com** : le throw
  `Error("b`" + a)` du helper closure
  `makeIterator` (tout `for-of`/spread des
  bundles compilés) cible une valeur sans
  `Symbol.iterator` **ni** `length`
  numérique. Balayage des surfaces host :
  `el.attributes`/`form.elements` =
  `undefined` (throw), `style.length` =
  `""` (throw), `classList`/`children`… =
  `length` OK mais indexés absents (items
  `undefined` silencieux). Livré :
  `Host::iter_items` + `Engine::new_
  iterable_host` posant `Symbol.iterator`
  en own-prop (native partagée resnapshottant
  à l'appel) ; `el.attributes` =
  `NamedNodeMap` vivante (`length`/indexés/
  accès nommé/`item`/`get|set|removeNamedItem`
  — `*NS` = alias nom local) ; `form.
  elements`/`fieldset.elements` = contrôles
  listés + `namedItem` + accès nommé ;
  named getter `form.username` (multi-match
  → collection) ; `select.options`/
  `selectedOptions`/`selectedIndex` ;
  `input.files` (FileList vide / `null`) ;
  `document.getElementsByName` ; accès nommé
  `document.forms/links/images/scripts` ;
  `classList` indexé/`value`/`item`/`forEach`/
  `entries`/`keys`/`values`/`replace`/
  `toggle(force)`/`supports` ; `style.length`
  numérique + `style[i]`/`item(i)` ;
  `item()` (null hors-bornes) sur toutes les
  collections tableaux. Déviations : `Attr`
  = objet plat snapshot, collections =
  snapshots d'accès, `namedItem` multi-match
  → premier (pas de `RadioNodeList`). Non
  reproduit à l'identique (variante `/ml`
  servie par token `escs` périmé) — fix sur
  la classe de faute balayée. Régressions
  `c76_*` (3 tests).

- [x] **JS-C77. `fetch(mode,
  credentials)` → transport** —
  couture réservée de l'étape 126
  (journal), consommée : le chantier
  « web réel sécurisé » (125/126)
  n'était exercé que par la
  plateforme, le script peut
  désormais piloter `Request.mode`
  et `Request.credentials`.
  `XhrRequest` + `NetRequest::Fetch`
  gagnent `mode`/`credentials`
  (enums `FetchMode`/`FetchCredentials`
  jscore, ré-exportés par
  `explorer-js`, mappés aux enums
  net dans `script_request`).
  `fetch` : lecture `Request` input
  puis `init`, validation
  `TypeError` en rejet (`new Request`
  : synchrone) ; `same-origin` +
  cross → rejet avant émission ;
  `no-cors` → verbe simple exigé +
  safelist à l'émission + réponse
  **opaque** livrée (`type:'opaque'`,
  statut 0, corps/en-têtes vides —
  `ACAO` non exigé) ; `cors` cross
  → `type:'cors'`. `xhr.withCredentials`
  réel → `credentials:include` ;
  `sendBeacon` → `no-cors`+`include` ;
  `cache.add/addAll` → `include`
  (spec). Cookies : `omit` pose/capture
  rien, `include` envoie sous
  `SameSite` et capture `Set-Cookie`
  — beacon `no-cors` cross inclus.
  Défense session : strip no-cors
  redoublé dans `script_request`,
  `same-origin` cross → bloquée.
  Régressions `c77_*` (10 tests
  jscore) + 4 session.
- [x] **JS-C78. `JsEngine` par
  contexte — ponts `window.parent`/
  `top`/`postMessage` borné** —
  *déclenché : étape 131 produit
  (renumérotée — ex-129 iframe,
  journal « piste JS notifiée »
  de l'étape 128 socle).* Le
  socle 128 parse chaque
  `<iframe src>` en `Dom` propre
  (`SubFrame` sur `Tab`, bfcache
  restauré) — la suite exige un
  miroir par contexte : un
  `Engine` = un document
  aujourd'hui, l'enfant a besoin
  du sien. Surface attendue
  (spec + sites : login widgets,
  captchas, boutons tiers) :
  getters cross-contexte
  `window.parent`/`top`/`frames[n]`,
  `iframe.contentWindow`/
  `contentDocument` (sous
  same-origin), `postMessage`
  structuré-cloné **borné par
  origine** (`targetOrigin`
  exigé, `*` dégrade honnête),
  `MessageEvent` livré au
  destinataire. Chantier
  architectural (comme C75) —
  **périmètre v1 arrêté** :
  `contentDocument` same-origin
  uniquement (cross → `null`,
  spec) ; `contentWindow` cross →
  `WindowProxy` **opaque
  whitelisté** (spec réelle —
  `null` rendrait `postMessage`
  cross injoignable : la surface
  whitelistée n'est ni un stub ni
  une fuite d'objet natif) ;
  `postMessage` clone `WireValue`
  (C29), `targetOrigin` **exigé**
  — absent/`undefined`/`*` =
  `SyntaxError` honnête ;
  `MessageEvent.origin`/`source`
  peuplés, livraison après les
  microtasks de l'émetteur.
  Forme retenue : **un heap, N
  globals** (modèle agent spec —
  zéro réentrance, `content
  Document` = vrai miroir
  enfant) — arrêt couture au
  journal. Livré : `context_new`/
  `context_switch`/`current_
  context`/`context_destroy`
  publics, timers/microtasks/XHR
  taggés par contexte, racines
  GC des garés, intrinsics frais
  par realm — 17 tests `c78_*`.
  La couture `SubFrame`→contexte
  reste côté session (étape 131).
- [x] **JS-C79. `crypto.subtle`
  (couture embedder)** — *réservé à la
  piste JS par l'annonce cap-g1
  (journal 2026-10-23), reliquat
  non consommé du cap « web réel
  sécurisé ».* Déclencheur :
  login OAuth2 réel — PKCE
  `code_challenge` S256 exige
  `crypto.subtle.digest('SHA-256',
  …)` côté script (SPAs modernes),
  `sign`/`verify`/`encrypt` pour
  JWT et WebAuthn-adjacent.
  `getRandomValues` existe déjà
  (JS-C26, arc4random seedé —
  vérifier borne `QuotaExceededError`
  64 Kio). Surface : `crypto.subtle`
  `CryptoKey`/`SubtleCrypto` —
  digest SHA-256/384/512 minimum,
  HMAC/AES-GCM/RSA-PSS selon
  besoin site. Point d'architecture
  : `explorer-jscore` est
  autonome sans dépendance —
  **arbitrage tranché : couture
  embedder** — `explorer-jscore`
  reste autonome-zéro-dépendance
  (promesse mission) : requête
  abstraite `Crypto{algo,op,data}`
  → plateforme exécute (RustCrypto
  côté net/core, deps permises) →
  résultat en `Promise`. Bonus :
  la même couture sert le FFI
  Flutter. **Livré** : `digest`
  `SHA-256`/`SHA-384`/`SHA-512`
  (`AlgorithmIdentifier` normalisé
  — `{name}` et casse acceptés) ;
  `CryptoTransport`/`CryptoRequest`/
  `CryptoResponse` publics,
  `install_crypto` engine-global
  (les sous-contextes C78 en
  héritent), pending document-local
  livré dans le realm émetteur ;
  `NetRequest::Crypto` →
  `spawn_blocking` → canal
  `CryptoDone` → `poll_net` ;
  `NotSupportedError` (algo
  inconnu, transport absent),
  `OperationError` (échec
  plateforme), `TypeError` sync
  (`WebIDL`). Fournisseur = `ring`
  (dep directe `explorer-net` —
  déjà provider rustls ADR-0002,
  zéro nouvelle entrée lockfile) ;
  `getRandomValues` borne 64 Kio →
  `QuotaExceededError` (était
  `TypeError`). Ops à clé
  (`importKey`/`sign`/`verify`/
  `encrypt`/`CryptoKey`/`derive*`) :
  absents, au besoin site — la
  couture est prête (`CryptoOp`
  extensible). Régressions `c79_*`
  (11) + vecteurs FIPS `net::
  crypto` + 2 tests session
  end-to-end.

- [x] **JS-C80. Compat frameworks —
  attributs énumérés + méthodes
  lisibles + handlers `on*`** — sonde
  React 18 UMD prod (`createRoot`,
  `useState`, `useEffect`, input
  contrôlé). Déclencheur : champ
  contrôlé React jamais tiré —
  chaîne racine mesurée :
  `input.type` rendait `""`
  (sniff `isTextInputElement` rejeté) ;
  `document.createElement` invoke-only
  (`canUseDOM` faux → polyfill IE8
  `propertychange` de
  `ChangeEventPlugin`) ; `"oninput" in
  document` faux (sniff `Lj`). Livré :
  table `enum_attr_default` (défauts
  missing/invalid spec, set = reflet
  brut via `enum_attr_write_name`,
  tag-scopé — hors `MAPPED_ATTRS` qui
  masquait le named getter `<form>`,
  c76) ; méthodes lisibles
  `DOC_METHODS`/`NODE_METHODS` via
  trampoline `bound_native` →
  `Host::invoke` ; `on_handler_get`
  (propre > attribut compilé JS-C45 >
  `null`). **onChange contrôlé
  fonctionne en session.**
- [x] **JS-C81. `MessageChannel`/
  `MessagePort`** — voie rapide des
  schedulers (React préfère le canal
  à `setTimeout`) : `postMessage` →
  `WireValue` cloné (DataCloneError
  sur non-clonable), livraison
  `message` au checkpoint de pompe
  (`drain_port_msgs`, même frontière
  que `postMessage`/workers) ;
  `onmessage` + `addEventListener`,
  `close` défausse la file, `start`
  no-op (démarrés à la création) ;
  GC-traqué (`peer` + listeners).
  Régressions `message_channel_*`.
- [x] **JS-C82. Accesseurs IDL +
  `constructor` typé** —
  `el.constructor` rend un objet
  fonction nommé par classe
  (`HTMLInputElement`…) avec
  `.prototype` porteur de `Prop::Acc`
  pour les props réfléchies
  (`value`/`checked`/`type`/…, table
  par interface) →
  `Object.getOwnPropertyDescriptor(
  ctor.prototype,'value')` rend
  `{get,set}` réel ; `defineProperty`
  accesseur own sur élément intercepte
  get/set (précédence own-Acc dans
  `NodeHost::get`/`set` ; les
  accesseurs proto lisent/écrivent
  l'attribut **brut**, jamais via
  `Host::get` — récursion sinon).
  Chaîne `inputValueTracking` armée
  end-to-end. Ctor non constructible
  (`illegal constructor` spec).
  Régression
  `idl_accessors_descriptor_surface`.

- [x] **JS-C83. Sonde « vraie app
  React » + `MouseEventInit`** — React
  19 bundlé esbuild (IIFE, 1,2 Mo) :
  `createRoot`/`useState`/`useEffect`,
  `fetch`→JSON→re-render, champ
  contrôlé (chaîne `inputValueTracking`
  complète — setter natif intercepté),
  `createPortal` (modal dans
  `#portal-root`), `useTransition`,
  `history.pushState`/`location`.
  **Trou corrigé** : `MouseEventInit`
  défauts (`button`/`buttons`/coords
  = 0, modifieurs = `false`) au ctor
  + recopie au dispatch littéral —
  `event.button === 0` est le garde
  des routers ; mesuré
  react-router-dom 7 `<Link>` →
  `route:/x` end-to-end. Effet de bord
  constaté : le repaint post-effets
  passifs (gap noté JS-C80) est
  résolu — `MessageChannel` livre le
  travail React avant la frame.
  Régression
  `mouse_event_defaults_reach_dispatch`.

- [x] **JS-C84. Événements React
  étendus** — sonde React 19 :
  `onSubmit` (`requestSubmit` →
  `preventDefault` → reste SPA),
  `onKeyDown` (`key`/`code`),
  `onFocus`/`onBlur` via
  `focusin`/`focusout` délégués,
  `onPointerDown`, `onWheel` — tous
  vivants. **Trou corrigé** :
  `WheelEventInit.delta*` et les
  inits `PointerEvent`/`FocusEvent`/
  `InputEvent`/`TouchEvent`
  n'étaient ni copiés au ctor ni
  recopiés au dispatch littéral
  (`e.deltaY` = 0) — listes
  étendues. Régression
  `specialized_event_init_reaches_dispatch`.

- [x] **JS-C85. `useSyncExternalStore`
  + `lazy`/`Suspense`** — sonde React 19 :
  sains sans fix (subscribe→bump→
  re-render ; lazy→promesse→fallback
  remplacé) — la pompe microtasks +
  `MessageChannel` couvrent la sémantique.

- [x] **JS-C87. Census terrain
  react.dev** — Next.js hydraté :
  page montée, chunks JSONP dynamiques
  fetchés (CAP-169), gtag.js évalué.
  **Trou corrigé** : `a.pathname`
  rendait `undefined` (le parse-via-
  ancre de toute la télémétrie) →
  décomposition URL `a`/`area` depuis
  `href` (`""` sans href). Bonus :
  attribution des piles inline/dyn.
  Reste mesuré : 1 repli fingerprint
  journal (différentiel→re-parse).

- [x] **JS-C86. `hydrateRoot` /
  hydration** — markup SSR + hydrate
  sur sonde React 19 : React
  réconcilie l'existant (pas de
  recréation), délégation attachée,
  effets lancés — clic et useEffect
  vivants de bout en bout. Aucun trou
  moteur ; l'hydration réelle de
  react.dev (JS-C87) le confirme.

- [x] **JS-C88. Build React dev** —
  `react-dom.development.js` sur la
  sonde UMD : chemins `__DEV__` sains
  (warnings console, mêmes résultats
  que prod — count=1, echo:hello).

- [x] **JS-C89. Audit React 19 (1/2) —
  `document.head` auto-généré** :
  `mountHoistable` crashait sur doc
  sans `<head>` (`insertBefore of
  Null`) — le hoisting métadonnée
  exige `head` toujours présent. Fix :
  création paresseuse + insertion
  avant `body`. Reste mesuré : render
  de l'app d'audit silencieusement
  vide — bisection des hooks R19
  (`use`/`useOptimistic`/`useActionState`)
  à poursuivre.

- [x] **JS-C92. `EventSource` réel
  (Server-Sent Events)** — surface spec
  (`readyState`, `url`, `withCredentials`,
  `close()`→`Abort`, `on*`+`addEventListener`,
  constantes) + couture `EsTransport`
  engine-global → `NetRequest::EventSource`
  streamée (`fetch_stream`, `Accept:
  text/event-stream`, `connect-src` comme
  XHR, quota `net_tasks`, `close()` =
  `Abort` handle) → livraisons
  `open`/`message`/`error` multi-coups
  par `deliver_es_event` (realm émetteur,
  tour post-script complet, buffer
  détaché). Parseur incrémental
  `explorer_net::sse` (lignes `\n`/`\r\n`/
  `\r`, `\r\n` coupé entre chunks, BOM
  tête, `data` jointe `\n`, `id` persistant,
  événement sans `data` ignoré, queue
  incomplète abandonnée — spec §9.2.6).
  **Bornes** : pas de reconnexion
  `Last-Event-ID` ni `retry:` à cette
  tranche — `error` puis CLOSED, le
  script ré-ouvre ; `ports`/`source`
  absents du `MessageEvent` minimal ;
  sans transport injecté → refus différé
  `error` au checkpoint (feature-detect
  + fallback, motif `WebSocket`).
  Régressions : `eventsource_streams_
  messages`, `eventsource_error_on_non_
  sse` (2 session) + 6 unitaires parseur.

- [x] **JS-C90. `Array.prototype` générique** —
  `take_elems`/`put_elems`/`this_array` array-like
  (`[[Get]]`/`[[Set]]`/`[[Delete]]`/`length`) : jQuery
  complet ($(fn), append, on(click), text, addClass,
  hide). Régression `array_methods_generic_on_array_like`.
- [x] **JS-C91. Chaîne htmx (partie)** —
  `overrideMimeType`, `classList` méthodes lisibles,
  `xhr.upload` stable, `DOMParser` constructible.
  `htmx.ajax` part. Reste : heap-limit au swap
  (`hx-swap`), Alpine `x-data`.

- [x] **JS-C93. `crypto.subtle` à clé +
  `navigator.credentials` honnête** —
  `importKey`/`sign`/`verify` (HMAC,
  ECDSA-P256, Ed25519 — `ring`),
  `CryptoKey` opaque ; `credentials`
  `create`/`get` → `NotSupportedError`.
  Reste : `jwk`, `generateKey`,
  `derive*`, `encrypt`, passkeys
  plateforme (délégation embedder).

- [x] **JS-C94. `WebSocket` réel** —
  codec RFC 6455 `explorer_net::ws`
  (masquage, fragmentation, ping/pong,
  close) + `WsDial`/`RealWsDial` (socket
  dédiée hors pool) + couture
  `WsTransport` → `NetRequest::WsOpen`/
  `WsSend`/`WsClose` → livraisons
  `open`/`message`/`error`/`close`
  multi-coups par realm (spec, spec
  « fail » = error+close 1006).
  Bornes : pas de CONNECT-proxy ni
  deflate, `Blob` approximé.
  Régressions : 2 session + 1 jscore +
  6 codec + 2 loopback `RealWsDial`
  (socket TCP réelle — handshake,
  echo, pong constaté, close 2 sens).

- [x] **CAP-194. `AbortSignal` réel
  borné (sonde x.com)** — `AbortSignal
  is not defined` au bootstrap : le nom
  existait en `illegal_ctor` sans
  surface. Statics `abort`/`timeout`/
  `any` + proto `aborted`/`reason`/
  `throwIfAborted`/`onabort`/`addEvent
  Listener` + ctor Illegal spec ;
  `AbortController.abort` bascule
  `aborted`/`reason` + listeners.
  Test `cap194_abort_signal_surface`.
  Mur suivant mesuré offline :
  `Invariant failed` = duplicate
  route-id TanStack (`undefined/*` —
  parent root jamais inité, deux objets
  root distincts — identité tranchée,
  **JS-C95**).

- [x] **JS-C95. Identité de module =
  URL résolue** — le cache de
  `eval_module` était clé par le
  spécificateur brut : `./assets/x.js`
  importé depuis la racine et `./x.js`
  depuis `assets/` instanciaient le
  même fichier **deux fois** — corps
  ré-exécuté, arbres top-level
  dupliqués. Mesuré en terrain x-web :
  `startup-dynamic` instancié ×2 →
  deux `routeTree` TanStack parallèles
  → les closures `getParentRoute`
  lisaient l'arbre non marché →
  `_id` `undefined/<path>` (~30
  routes) → invariant `routesById`
  (`Invariant failed`, stack
  `lu←s←s←sd`). Fix : registre
  `module_urls` `url résolue → idx`
  (canonique, spec), le cache `spec`
  conservé pour les cycles. Reproduit
  en local : graphe réel + scripts
  inline `[5]`/`[6]` de la page —
  `dynEvals=2→1`, `hAtLp===Q===sdarg`,
  zéro collision — le mur franchit
  jusqu'au `setTimeout` du harnais.
  Régression `mod_dedup_resolved_url`.

- [x] **CAP-191. Shadow DOM tranche 1 :
  surface sans composition** — `attachShadow`
  spec-fidèle (`NotSupportedError` au
  re-attach), `shadowRoot.host` via
  `Dom::shadow_hosts`, `getRootNode({composed:
  true})` traversant à `document`,
  `assignedSlot`/`assignedElements`/
  `assignedNodes` honnêtes (pas de
  répartition), `mode:"closed"` +
  `ElementInternals`. Reste : scoping
  CSS (T2 → PIPE), event retargeting
  (T4) ; slots T3 + `shadowrootmode`
  T5 livrés en CAP-197. Test
  `cap191_shadow_surface`.

- [x] **CAP-192. `customElements` lisible
  et patchable** — census shadow DOM
  (reddit `patchDefine`) :
  `has_method` sur `CustomElementsHost`
  (méthodes lisibles `typeof ===
  "function"`) ; dispatch `obj.m(…)`
  cède à l'own-prop scriptée avant
  `Host::invoke` (spec `[[Get]]` —
  patchs de shim effectifs) ; le
  trampoline capturé `n.call(t)` joint
  l'original sans récursion. Tests
  `custom_elements_methods_readable_
  and_patchable`, `custom_elements_registry`.

- [x] **CAP-195. `document.adoptedStyleSheets`
  settable `[]`** — mur mesuré github :
  le polyfill Popover meurt à
  `[C, ...e.adoptedStyleSheets]`
  (`value is not iterable`). Tableau
  `[]` paresseux stable `===`,
  assignation acceptée ; feuilles non
  appliquées (scoping = T2). Test
  `cap195_adopted_style_sheets_surface`.

- [x] **CAP-196. `getComputedStyle
  (el).position` ← géométrie** — mur
  mesuré github : garde `"fixed"===
  position` lisait l'inline → `""` →
  `--marketing-header-fixed-top` jamais
  posée. `StyleHost.computed` distingue
  `el.style` (pur inline, spec) ;
  `position` absent de l'inline →
  `"fixed"` quand `ElementGeom.fixed`
  est publié (i#19). `getPropertyValue`
  idem. Test `cap196_computed_position_
  from_geometry`.

- [x] **CAP-198. Shadow DOM tranche 4 :
  event retargeting** — propagation
  shadow-aware dans `propagate_event` :
  `composed` traverse racine → hôte →
  ancêtres (imbriqué, borné 64), non-
  composé tronqué à la racine (fragment
  détaché → `ancestors` suffisait).
  `EventState` gagne `adjusted` /
  `related_node` / `adjusted_related` /
  `path` : `target` et `relatedTarget`
  ré-étiquetés par listener (spec
  « retarget » — visible depuis la
  racine du `currentTarget`),
  `composedPath()` = chemin réel de
  dispatch, `[]` hors dispatch. Le
  ctor cesse d'écraser `relatedTarget`
  d'init à `null`. Borne : chemin DOM,
  pas l'arbre aplat (T3 CAP-197).
  Test `cap198_shadow_event_retarget`.

- [x] **JS-C96. `await` hors lowering
  — toutes les positions couvertes
  (mur n°1 x.com, vol `ebb5168`)** —
  borne documentée de JS-C34b mesurée
  sur site réel (`TypeError "await on
  a pending promise is unsupported"`).
  **Correction du diagnostic** : les
  sous-expressions paresseuses
  (`&&`/`?:`/`?.`/`??`) étaient déjà
  hissées — les vrais trous étaient
  les **têtes de statements** et les
  **motifs**. Livré : `for` init
  (var/let/expr), `case` de `switch`,
  `with`, `using`/`await using`,
  `for-in var_init`, défauts de motifs
  (`{a = …}`/`[… = …]` — déclarations,
  `catch`, cibles `for-of`),
  `for await` (nouveau frame
  `FKind::ForAwait` — `it.next()`
  awaited par pas, `done`/rejet spec),
  clés calculées d'objets et de
  classes, `class extends`, `export
  default`/decl, et `lower_suspend_fn`
  — paramètres `a = await e` déplacés
  dans le corps en `if (a ===
  undefined) a = e` (paresse spec
  préservée). Déviations assumées
  documentées : suspends hissés **une
  fois, éagerment** pour `case`/`catch`
  /défauts de motifs et têtes de boucle.
  Régressions `c96_*` (5 tests, 13+
  positions — sonde exhaustive
  pré-fix exécutée).

- [x] **JS-C97. Palier 2 geom —
  `offsetParent`/`offsetTop`/
  `offsetLeft` sur `ElementGeom`
  (verrou github, vol `73820bf`)** —
  la garde `--marketing-header-fixed-top`
  lisait `a.offsetTop` (CAP-196
  servi pour `position`). Livré :
  `ElementGeom.positioned` (computed
  `position` ≠ `static`, publié à la
  collecte `collect_geometry`), la
  marche `offsetParent` côté miroir —
  plus proche ancêtre positionné ou
  `td`/`th`/`table`/`body`, `null`
  pour `fixed`/racine/sans boîte
  (spec CSSOM) ; `offsetTop`/
  `offsetLeft` = `rect − parent.rect`
  en coords **document** (insensible
  au scroll, union ≙ border box) ;
  `fixed` → `rect` viewport.
  Régression `c97_offset_parent_
  top_left`.

- [x] **JS-C98. Complétude de surface
  — census des globaux web (147 →
  ~12 trous réels)** — recensement
  `typeof` des globaux ES/web sur
  moteur + DOM installé : 147
  absences mesurées, ramenées aux
  seules **absences honnêtes**
  (valeurs `NaN`/`undefined`,
  APIs injectées par la session —
  timers/fetch/storage — niches
  volontairement absentes). Livré :
  ~70 stubs « illegal constructor »
  (Web Animations, Performance,
  ServiceWorker, WebRTC, File System
  Access, XPath…) + ctors événement
  (`ProgressEvent`, `PageTransitionEvent`,
  `BeforeUnloadEvent`, `FormDataEvent`,
  `MediaQueryListEvent`,
  `NotificationEvent`) + ctors réels
  bornés (`Option`, `DOMPoint`,
  `DOMMatrix` math 2D complète,
  `ImageData`, `Path2D` enregistreur,
  `OffscreenCanvas` ctx enregistré,
  `FontFace`, `createImageBitmap`) +
  props/méthodes window (`status`,
  `opener`, `frameElement`, `external`,
  `isSecureContext`, `crossOriginIsolated`,
  `origin`, `visualViewport`,
  `scheduler`, `cookieStore`,
  `stop`/`print`/`find`, `reportError`,
  pickers → `SecurityError`,
  `WebAssembly` honnête,
  `trustedTypes` passthrough) +
  navigator (`mediaDevices` rejets
  `NotFoundError`, `userActivation`,
  `permissions.query → 'denied'`) +
  `speechSynthesis` jar vide +
  `document.{has,request}StorageAccess`.
  Régressions `c98_*` (9 tests).

- [x] **JS-C99. Sous-ensemble
  sélecteur du miroir — `:not()` +
  `:defined` + listes `a, b`
  (verrou Shoelace, vol 2026-11-10)** —
  `shoelace-autoloader` `discover(root)`
  faisait `querySelectorAll(":not(
  :defined)")` → `[]` silencieux →
  `customElements.define` jamais
  appelé → `attachShadow` jamais
  atteint → 29 `<sl-*>` rendus en
  light DOM brut. Livré : parse de
  compound étendu (`:` arm +
  `MAX_PSEUDO_DEPTH`), réparation du
  `*` universel (jamais parsé avant),
  coupe top-level `,`/espace
  respectant `()`/`[` (`[a="x y"]`
  réparé au passage), pseudo-classes
  `:defined` (lit `ce.defs`/`upgraded`),
  `:not`/`:is`/`:where` (listes de
  compounds), structurelles
  (`:first/last/only-child`,
  `:*-of-type`, `:nth-*(an+b)`,
  `:empty`, `:root`, `:scope`),
  attributaires (`:checked`,
  `:disabled`/`:enabled`,
  `:required`/`:optional`,
  `:open`/`:closed`, `:indeterminate`,
  `:placeholder-shown`, `:default`,
  `:link`/`:any-link`,
  `:read-only`/`:read-write`),
  héritées (`:dir`, `:lang`), focus
  (`:focus`, `:focus-within`),
  états jamais vrais (`:visited`,
  `:hover`, `:active`, `:target`,
  `:fullscreen` → `Never` honnête).
  Listes `a, b` en ordre document
  dédoublonné sur `query`/`matches`/
  `closest`. Régressions `c99_*`
  (8 tests — le chemin Shoelace
  `discover→define` complet couvert).
  Reste borné : `:has`, `:state`,
  `:host*`, `::part`, `:popover-open`,
  `:modal`, `:valid`/`:invalid` —
  parse-fail honnête (`[]`).
- [x] **JS-C100. `import()` dynamique
  servi à la demande**
  (`explorer-jscore` + `explorer-js` +
  `explorer-core`) — verrou Shoelace
  mesuré : `MapLoader` ne sert que le
  graphe **préfetché** — les specs
  construits à la volée (`${tag}.js`)
  rejetaient → `customElements.define`
  jamais atteint. Livré : miss
  `load_source` → promesse `import()`
  **pending** + `DynImportReq` enfilée
  (`drain_dyn_imports`, même motif
  `DynScriptReq`) ; la session fetch
  l'URL (CSP `script-src`, mixed-
  content) → `deliver_dyn_import`
  → `instantiate`/`evaluate` →
  `resolve_promise` — adoption
  thenable gratuite (eval.rs wrap).
  Sans drain branché : rejet comme
  avant (pas de defer fantôme).
  Tests `c100_*` (3).
- [x] **JS-C101. Graphe rempli à la
  livraison d'`import()`**
  (`explorer-jscore` + `explorer-js` +
  `explorer-core`) — verrou Shoelace
  **complet** : le fichier livré
  (`sl-button.js`) importe `lit`
  **statiquement** → `load_source`
  manquait le préfetch → `instantiate`
  rejetait → `.catch` avalé →
  `define` jamais atteint. Livré :
  `spawn_dyn_import` fetch l'entrée
  puis **`fetch_module_sources`**
  (même chemin `type=module`, import-
  specs de la page portées dans
  `DynImportReq`) → `deliver_dyn_
  import(ctx, seq, entry, deps)` →
  `seed_module_source` × deps dans
  `Engine::module_sources` (cache
  par URL résolue, consulté avant
  le chargeur dans `load_source`)
  → `instantiate` trouve `lit`.
  Tests `c101_*` (1 — dep statique
  seedée résout `ns:42`).

- [x] **JS-C103. Drain de microtasks
  borné (jobs + rounds)** —
  mur reddit mesuré (`21e7507`) :
  file auto-régénérante ou
  livraisons perpétuelles (stream
  `read()` perpétuel, observers
  ré-armés — `delivered` vrai à
  chaque round **sans tick de
  budget ops**) → `run_microtasks`
  infini → `rx.recv()` sous
  `sess.lock()` → daemon gelé.
  Bornes 65 536 jobs + 128 rounds ;
  reliquat repris au drain suivant.
  Test `c103_*`.

- [x] **JS-C114. Parseur miroir :
  `</head>` implicite + `PAIRED_CLOSE`**
  (`explorer-jscore::dom`) — gBCR
  0×0 global w3.org attribué à tort
  au « post-layout » : la sonde
  `EXPLORER_GEOM_TRACE` a classé la
  vraie cause — le miroir nichait
  `body` sous `head` (HTML sans
  `</head>` littéral) → tous les
  chemins `element_path` décalés.
  Règle « in head » (`HEAD_TAGS` +
  `<body>` implicite) + paires
  cross-tag (`dd`/`dt`/`li`,
  `tr`/`td`/`th`, sections de table)
  dépilant jusqu'à la cible.
  Mesuré : 0 % → **87,6 %**
  rects non-nuls ; résidu honnête.
  Tests `implicit_head_close_keeps_
  body_sibling` + `implicit_close_
  dt_dd_pairs`.

- [x] **JS-C115. `FunctionDecl`
  top-level sans ré-instanciation**
  (`explorer-jscore::interp::exec`)
  — `registerId is not a function`
  vaadin : l'exécution de la
  déclaration recréait la fonction
  en objet neuf, jetant l'hoistée
  qui portait les props posées tôt.
  Skip quand le binding contient
  déjà la fonction de cette
  déclaration (`Rc::ptr_eq` AST) —
  Annex-B bloc et closures de
  boucle préservés. `registerId`
  disparu de la console vaadin.
  Test `iife_hoisted_fn_shadows_
  and_receives_props`.

- [x] **JS-C116. `document.
  styleSheets` itérable**
  (`explorer-jscore::dom_host`) —
  fixup lumo vaadin `new Set(
  e.styleSheets)` tuait l'upgrade
  CE (`object is not iterable`).
  StyleSheetList bornée : `<style>`
  + `<link rel=stylesheet>` (rel
  token-list case-insensitive) en
  ordre document, objets
  `css_sheet_ctor` + `ownerNode`/
  `href`, tableau frais par get.
  Mesuré : 52 feuilles, CE
  `appendChild` survit à l'init
  lumoInjector. Test `document_
  style_sheets_iterable`.

- [x] **JS-C107. `push`/`pop`
  array-like O(1) — wedge gov.uk
  tuée** (`explorer-jscore::builtins`)
  — la voie générique matérialisait
  `len` (cap 4M) via `take_elems`/
  `put_elems` : ~8M d'ops natives
  non tickées, spin de minutes
  (`Array.prototype.push.call` sur
  `{length: 2**32}` — queue webpack/
  GOVUK). Spec : `Set(len+i, arg)` +
  `Set("length")` — zéro
  matérialisation ; `pop` O(1) idem.
  Repro 100 % wedge → 2,12 s,
  matrice 26/26. Test
  `array_push_generic_array_like_
  no_materialize`. Reliquat :
  take/put faillibles + tick pour
  les mutateurs O(len) (`shift`,
  `splice`, `sort`).

- [x] **JS-C113. Kill mémoire sur
  vivant** (`explorer-jscore`::tick) —
  mur hubspot (`heap memory limit
  exceeded` sur page réelle) : le
  drapeau `over_limit` estimé tuait
  sur déchets. `tick` → `collect()`
  → re-test — kill sur vivant seul.
  `c37_mem_limit_soft_kill` refondu.

- [x] **JS-C119. Promesses `import()`
  racinées + microtasks dé-famées**
  (`explorer-jscore`) — mur final
  lazy-CE github mesuré en vol :
  `pending_imports` non tracé au GC
  (promesse balayée → `resolve` no-op
  → `.then` morts, `reactions=999`
  instrument) ; `run_microtasks` sans
  ré-armement `ops_left` (drains
  affamés mangeaient un job par
  pompe). Fix : promise en racine +
  budget neuf par drain + re-queue
  sur échec `tick`. Terrain :
  `react-partial`/`select-panel-
  experimental`/`global-banner`/
  `active-global-banners` définis.
  Test `import_then_fires_after_
  deliver`.

- [x] **JS-C118. `define` jette
  `DOMException` typées**
  (`explorer-jscore::dom_host`) —
  catalyst github rejette le
  `TypeError` sur re-define (tolère
  `NotSupportedError` par `e.name`).
  Spec : nom invalide → `SyntaxError`,
  doublon nom/ctor → `NotSupportedError`,
  ctor non-appelable → `TypeError`.
  Tests `define_duplicate_throws_
  domexception_not_supported` +
  `custom_elements_registry`.

- [x] **JS-C112. Garde cycle DOM**
  (`explorer-jscore` dom+dom_host) —
  `appendChild`/`insertBefore`/
  `replaceChild` d'un ancêtre/soi
  créait un cycle → walks DFS
  bouclaient. `is_ancestor_or_self`
  → `HierarchyRequestError` (spec) ;
  arbre acyclique par construction.
  Test `append_child_ancestor_cycle_
  rejected`.

- [x] **JS-C111. Regex/commentaires
  dans `${…}` template** (`explorer-jscore
  ::lexer`) — mur apple (`unterminated
  string in template` globalheader) :
  `'`/`"` dans un regex littéral ou
  commentaire ouvrait un scan chaîne
  jusqu'à EOF. `/` géré : `//`/`/*…*/`
  sautés, regex vs division par
  contexte opérande, `skip_regex`
  (`\\`/`[…]`). Test
  `template_substitution_regex_and_comments`.

- [x] **JS-C110. `EventTarget` /
  `DOMException` constructibles**
  (`explorer-jscore::dom_host`) —
  mur css-tricks (`illegal
  constructor` tuait wp-polyfill
  au detect) : `new EventTarget()`
  → instance proto-liée + listeners
  `__et` (add/remove/dispatch,
  `target`/`currentTarget`,
  `preventDefault`) ; `new
  DOMException(msg, name)` →
  `{name, message}`. Test
  `event_target_and_domexception_
  constructible`.

- [x] **JS-C109. Nœuds `Comment`
  réels** (`explorer-jscore` dom +
  dom_host) — mur Lit mesuré vaadin
  (`createComment is not a function`
  → 0 custom element) : `NodeKind::
  Comment` complet — `nodeType` 8,
  `#comment`, `data`/`nodeValue`/
  `textContent`, `<!--…-->` round-trip
  `innerHTML` (marqueurs `?lit$N$`
  vivants), TreeWalker `SHOW_COMMENT`.
  `nodeName` majuscule = éléments
  seuls (spec). Test
  `create_comment_node`.

- [x] **JS-C108. `take_elems`/`put_elems`
  faillibles + tickés** (reliquat
  JS-C107 — `explorer-jscore::builtins`) :
  les mutateurs O(len) encore
  matérialisants (`shift`, `splice`,
  `sort`, `unshift`, `fill`, `copyWithin`)
  bouclaient 4M de `get`/`set`/`delete`
  indices non tickés — `{length: 2**32}`
  gelaient hors budget. Helpers devenus
  `JsResult` + `tick` par indice (67 call
  sites) : mort au budget, erreur nommée ;
  tableaux natifs inchangés. Test
  `array_mutators_array_like_bounded_by_ops`.

- [x] **JS-C106. Borne
  d'imbrication 512→4096**
  (`explorer-jscore::parser`) —
  `nest_max`/`expr_depth` vs
  bundles minifiés (caniuse
  `RangeError: expression too
  deeply nested` — terrain).
  `maybe_grow` protège la
  pile — la borne reste
  hostile-only. Test
  `deeply_nested_expression_parses`.

- [x] **JS-C104. Virgule finale
  ES2017 en args d'appel**
  (`explorer-jscore::parser`) —
  `f(a,)`/`new X(a,)`/`x?.(a,)`
  acceptés (arrays/objets/params
  l'étaient déjà — seul trou) ;
  `f(,)`/`f(a,,)` restent
  illégaux. Mur spectrum
  ak.js:280 levé — terrain :
  0 erreur console, imports
  dynamiques livrés.
  Test `trailing_comma_call_args`.

- [x] **JS-C102. `el.constructor` =
  ctor CE sur élément upgradé**
  (`explorer-jscore`) — verrou
  Shoelace **bout-en-bout** : le
  host `get("constructor")` court-
  circuitait vers l'interface
  (`dom_ctor`) → `this.constructor.
  elementProperties` LitElement =
  `undefined` → `.keys()` jette →
  `ce.failed` → `attachShadow`
  jamais. Fix : `ce.upgraded`/`defs`
  prime — `el.constructor` rend le
  ctor enregistré. **Mesuré
  terrain** : `sl-button` CDN
  `sr=SR`. Test `c102_*`.

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
