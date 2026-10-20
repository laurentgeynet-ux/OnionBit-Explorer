# Roadmap — Équipe NAVIGATEUR

**Périmètre :** la plateforme produit — `explorer-net`,
`explorer-core` (session/pipeline), `explorer-api`,
`explorer-daemon`, `explorer-cli`, `explorer-ffi`,
**toute la surface JS** (`explorer-js` + `dillo-js`),
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
- **Piste JS** : **JS-C26+** (suite de la série `dillo-js`).

**Definition of done** (inchangée) : implémenté + testé +
CHANGELOG + case cochée + `verify_all` vert + commit dédié.
**Spécialiste JS** : la piste JS est tenue par la personne qui
connaît `dillo-js` — rôle nommé au sein de l'équipe, pas une
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
Écartés en plus : github (crash dillo-js → JS-C41 livré),
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
  `type=module` → `ModuleLoader` de `dillo-js` (existe, JS-C5).
  Scheduler **parallèle** : files bornées par onglet
  (`SessionConfig`), priorités `document > CSS > script > img`,
  N fetches en vol (`FuturesUnordered`), `Referer`/`Origin` et
  mixed-content déjà calculés à l'émission. Insertion dynamique
  de scripts : hooks à définir avec JS-C35.
  *Pourquoi en premier : trou le plus bloquant du web réel.*

- [x] **Étape 51. Câblage Web APIs — clôt l'étape 44 de
  ROADMAP.md.** Intégralement interne à l'équipe :
  - `JsEngine` : traverse proxy des surfaces déjà livrées dans
    `dillo-js` — `XhrTransport`/`deliver_xhr_response`,
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
  `next_timer_deadline` de `dillo-js` devient un délai armé par
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

- [ ] **Étape 53e. Charset du `<title>` / `title_changed`**
  (bug terrain mission 6, repro : `news.ycombinator.com/
  item?id=…` — titre `â€“` = UTF-8 relu en Latin-1/1252) :
  vérifier d'abord si la page déclare son charset tardivement
  (prescan trop borné ?) ou pas du tout — dans ce dernier cas
  le fallback windows-1252 est conforme WHATWG et le défaut
  est côté extraction/émission du titre.

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
  (enregistrement op-list côté `dillo-js`) — même couture
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
  `dillo-js` : `play()`/`setSinkId`/`setMediaKeys` rejettent
  `NotSupportedError` (chemin `.catch()` déjà géré par les
  sites), `canPlayType` avoue `""`, `paused`/`duration`(NaN)/
  `readyState`/`networkState` (NO_SOURCE si `src`) reflètent
  « rien à lire », `buffered`/`played`/`seekable` = TimeRanges
  vides, écritures IDL acceptées-ignorées — jamais de faux
  succès. Tests : 4 session (affiche peinte http+`data:`,
  boîte noire, audio masqué) + 2 dillo-js (props, rejet
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
- [ ] **Étape 59. Release 0.2** (ex-67) : packaging, smoke E2E
  étendu, doc. **Prérequis d'ouverture** : renommage
  `crates/dillo-js` → `explorer-jscore` (ADR-0020) — commit
  `chore:` dédié, à un moment où aucune piste ne touche le crate
  ; le nom de crate est surface publique de release.

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
- [ ] **Étape 64. `navigated` rapporté avant commit de
  l'URL** (terrain mission 16 — séquencement) : premier
  `Enter` dans un input → événement `navigated` émis alors
  que `tab_url` reste sur l'ancienne page ; le commit
  n'apparaît qu'au second Enter. L'API rapporte une
  intention comme un état commis — l'UI afficherait une
  URL jamais atteinte si la nav échoue ensuite (observé :
  fetch mort derrière → on reste sur `/`). Séquencer :
  émettre `navigated` au commit effectif, ou typer le
  statut « en vol ».
- [ ] **Étape 65. `doc_h` clampé à 2¹⁴ masque la hauteur
  réelle** (terrain mission 16 — mesure) : sur un document
  plus haut que 16 384 px (`heavy.html`), `doc_h` rapporte
  le plafond au lieu de la hauteur posée — la hauteur
  réelle du document n'a pas de canal. Observation/API :
  exposer la hauteur mesurée vs le plafond raster
  (`MAX_FRAME_DIM`), ou typer la réponse
  (`doc_h: 16384, doc_h_clamped: true`).
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
- [ ] **Étape 69. Pages d'erreur navigateur** (lead —
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
- [ ] **Étape 70. Téléchargements** (lead — trou produit,
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

## Piste JS (JS-C26+) — spécialiste `dillo-js`

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
- [ ] **JS-C29. `Worker` bornés** : Engine par thread (patron
  `DilloEngine`), `postMessage` structuredClone (livré),
  `onmessage`, `importScripts` minimal, `terminate`, pool
  borné (`max_workers`, budget ops/worker). Documenté : pas de
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
- [ ] **JS-C35. `document.write` complet + insertion dynamique**
  — ordre cohérent avec le scheduler de l'étape 50.
  *Déclencheur : insertion dynamique constatée sur sites
  réels.*

### J-C — Mesure, perf, robustesse

- [ ] **JS-C36. Acid3 subset JS mesuré** (harnais informatif).
- [ ] **JS-C37. Perf + borne mémoire heap** : compteurs d'ops,
  seuils GC, **borne mémoire par contexte** (exigence ADR-0019),
  coût `serialize_document` instrumenté.
- [ ] **JS-C38. Fuzz lexer/parser `dillo-js`** : cible
  cargo-fuzz (extension de `fuzz/` — périmètre équipe).
- [ ] **JS-C40. Découpage interne `dillo-js`** (audit externe) :
  `builtins.rs` 269 Ko / `interp.rs` 222 Ko — extraire par
  domaine (builtins → `builtins/{object,array,string,…}.rs`,
  interp → dispatch + groupes d'opcodes). Pur reformatage
  mécanique, tests intacts — le tree-walker reste ; cette étape
  ne change **pas** la sémantique, elle rend le code revoyable
  à plusieurs. Écarts sémantiques assumés (générateurs eager,
  `await` pending) = JS-C34, déclenché site réel — pas ici.
- [ ] **JS-C39. Sync DOM incrémentale — instrumenter puis
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
- [x] **JS-C41. Récursion bornée — stack overflow site réel**
  (bug terrain, repro : github.com) : `dillo-js` fait un
  stack overflow sur le bundle réel de github (`dump`/`layout`
  OK, `shot` 💥 ; `EXPLORER_NO_JS=1` → erreur propre →
  confirme le moteur JS). **Violation de la règle « jamais
  crash »** — prioritaire dans J-C : profondeur
  d'appel/récursion bornée par budget → `JsError` typée,
  jamais d'overflow natif. Repro minimal : reproduire la
  pile en extrayant le pattern récursif du bundle.
- [ ] **JS-C42. `Drop` itératif de l'AST** (résidu mesuré de
  JS-C41, repro `repros/` : `x=1+1+…`/`a&&a&&…` à ~15 000
  opérandes via `eval` **et** `module_specifiers`) : le parse
  est borné (`input too deeply nested` propre) mais la
  **destruction récursive** de la chaîne `Box<Expr>` déborde
  — `maybe_grow` ne couvre pas `Drop`. Fix : `Drop` itératif
  sur les nœuds unaires/binaires chaînés. Coin extrême — le
  bundle réel github passe — mais du code généré peut
  produire de telles chaînes.
- [ ] **JS-C43. Parseur regex — motifs courants rejetés**
  (déclencheur terrain mission 6 : githubstatus.com,
  github.com — scripts réels morts au parse, erreurs typées,
  zéro crash) : `(.*?)` lazy → `quantifier without atom` ;
  `[.*+?^${}()|[\]\\]` → `Invalid character class` ;
  `[\0-\x1f]` → `empty class range` ; `\uD800` →
  `Invalid codepoint`. Chaque motif = un cas `corpus.rs`
  minimal avant fix — l'ordre est celui du site réel.
- [ ] **JS-C44. Specificateurs « bare » non résolus**
  (déclencheur terrain mission 6 : `cannot load module
  'react'`) : `import 'react'` hors import map — arbitrer :
  import maps minimal vs écart documenté (les bundlers
  résolvent normalement, mais des sites servent du bare en
  prod). Les `ReferenceError` `hljs`/`$`/`Event`/`SP`
  observés sont des **conséquences** (scripts amont morts),
  pas des tickets.
- [ ] **JS-C45. Attributs `on*` jamais compilés**
  (déclencheur terrain mission 10 — fixture `jsmut.html` :
  `onclick="…"` mort, seul `addEventListener` tire ;
  `fire_listeners` ne lit que `listeners` + prop
  `el.onclick`). Le web réel est couvert de handlers inline
  (`onclick`, `onload`, `onsubmit`…) — HTML spec : à la
  construction du miroir, chaque attribut `on*` est compilé
  en event handler (avec le bon `this`/scope).

## Tâches continues

- **Spécialiste JS** : divergence miroir ↔ `explorer-dom` quand
  le parse miroir est fautif → fix `dillo-js` + cas `corpus.rs`.
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
