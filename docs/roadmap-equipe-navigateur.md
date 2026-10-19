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

- [ ] **Étape 53f. `run_pending`/`sync_dom` hors verrou**
  (résidu 53d, terrain mission 8 : `repros/relayout.html` —
  200 `appendChild` triviaux → `/status` gelé ~20 s) : le
  fetch est libéré mais l'exécution JS + la synchro miroir
  au commit restent sous le verrou session — toute page
  réelle qui construit son DOM en JS (Gmail, Twitter…)
  gèlera pareil. Sortir la phase commit-script du verrou
  comme `NavJob`/`SubJob` l'ont fait pour le fetch.

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
- [ ] **Étape 55e. `@keyframes` cadencés en session**
  (adoption R-10, règle AGENTS §10.5b — confirmé non-câblé
  par `anim.html` : frames identiques à 400 ms) : zéro
  référence `keyframe`/`animation` hors `explorer-css` —
  `ActiveTransition` ne naît que de `transition-*` et
  `tick_animations` ne tourne qu'au pull de frame. Brancher
  `animation_decls`+`keyframe_style` au moteur d'animations
  + tick au pump de timers (sinon l'animation est figée
  entre deux pulls).
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
- [ ] **Étape 55g. La resynchro reste globale — l'incrémental
  ne vise que le relayout** (terrain mission 12) : `sync_dom`
  re-parse + re-cascade + re-boxtree le document **entier**
  avant que `relayout_subtree` ne vise le sous-arbre — le
  gain 55f n'atteint pas le goulot réel. Mesuré : mutation
  par clic sur doc de 3 000 nœuds → **gel > 5 min** puis
  `/frame` 204 (récidive 56b) ; petit doc = 468 ms + rects
  localisés corrects. Pistes : synchro partielle du sous-
  arbre seul, ou diff miroir→DOM sans re-parse global.
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
- [x] **Étape 56b. `/frame` → 204 transitoire après commit**
  (terrain mission 11) : **livré.** Cause : `freeze`
  drainait l'état vivant pendant la fenêtre
  `nav_apply`→`sub_commit` — un `try_bfcache` concurrent
  gelait un snapshot commis-mais-jamais-peint
  (`png`/`tree` absents), servi en 204 persistant à la
  restauration. Fix : `freeze` exige `dom` **et** `png`
  (pas de gel sans produit peint) + `try_bfcache` repeint
  depuis le DOM restauré si le snapshot n'a pas de produit
  (répare aussi l'existant). Régression :
  `paintless_snapshot_repairs_on_restore`.
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
- [ ] **Étape 56d. `#fragment` = navigation complète, sans
  scroll** (terrain mission 11 — `anchor.html` + footnote
  wikipedia) : clic sur `href="#id"` ou `name=` →
  `navigated` émis, nouvelle entrée d'historique,
  `frame_invalidated` plein document — mais `scroll_y`
  reste 0 : jamais de déplacement vers la cible. Fix :
  post-commit, résoudre le fragment (id puis `name`) et
  scroller à sa position — sans re-fetch si même document.
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
- [ ] **Étape 56f. Images `data:` invisibles en session**
  (terrain mission 12 — fixtures `img.html`, `b1`-`b4`) :
  `<img src=data:>`, `<object data=data:>` et
  `background-image: data:` peignent **0 pixel** — le fetch
  http `<img>` fonctionne (HIT + pixels mesurés), mais
  `collect_data_images`/`collect_images` n'est appelé que
  par les harnais de test, jamais en session
  (`insert_images` ne traite que les corps fetchés réseau).
  Câblage à faire : décoder les `data:` inline à
  `insert_images` (même entrée que les corps fetchés) —
  débloque d'un coup img `data:`, les yeux Acid2 (avec
  R-25 côté PIPE) et les backgrounds `data:`.
- [ ] **Étape 57. UI Flutter v2** (ex-65) : thème clair
  (`tokens.json` variante), settings (moteur REST/FFI, thème,
  profil), UI historique + bookmarks (store core existant),
  arbitrage find-in-page / zoom (petit ADR si reclassés).
- [ ] **Étape 58. `<video>`/`<audio>`** (ex-66) : arbitrage
  décodage pur Rust requis (ADR) — sinon stubs propres + affiche.
- [ ] **Étape 60. Éclater `session.rs`** (audit externe) :
  3 500+ lignes / 133 Ko — le god module porte session +
  pipeline + scheduler + profil + hit-test + glue JS + décodage
  images. **Découpler, pas juste déménager** : chaque
  sous-module sort avec sa frontière de responsabilité **et
  ses propres tests** (le scheduler de sous-ressources
  testable sans réseau ni DOM ; le net pump sans session
  complète) — sinon les 40+ champs de `Session` gardent le
  couplage sous un autre nom de fichier. Critère de sortie :
  au moins un module extrait testé en isolation. À faire
  entre deux étapes fonctionnelles, fichier au calme — pas
  pendant un chantier session en vol.
- [ ] **Étape 59. Release 0.2** (ex-67) : packaging, smoke E2E
  étendu, doc. **Prérequis d'ouverture** : renommage
  `crates/dillo-js` → `explorer-jscore` (ADR-0020) — commit
  `chore:` dédié, à un moment où aucune piste ne touche le crate
  ; le nom de crate est surface publique de release.

## Piste JS (JS-C26+) — spécialiste `dillo-js`

Ordre dicté par les besoins de la piste produit d'abord, puis
couverture **strictement réactive** (un bug site → un cas
`corpus.rs`/test262 → un fix — fini la couverture pour la
couverture).

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
- [ ] **JS-C28. `<canvas>` 2D surface** : `getContext('2d')` +
  `CanvasRenderingContext2D` complet niveau API (chemins,
  fills/strokes, transforms, `drawImage`, `fillText`, gradients,
  `getImageData`/`putImageData`) enregistrant une **op-list**
  exposée via `drain_canvas_ops` — raster = PIPE **R-11**.
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
