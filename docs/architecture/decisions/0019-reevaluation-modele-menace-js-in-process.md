# ADR-0019 — Réévaluation du modèle de menace : JS hostile in-process

Statut : Acceptée (2026-10-19) — réévaluation exigée par
**ADR-0011** (« réévaluation obligatoire en phase 8 »), écrite
*a posteriori* : la phase 8 (étapes 34–36) est livrée, `dillo-js`
exécute du JavaScript réel dans le processus du navigateur. Cette
ADR solde la dette documentaire et fixe les prochains points de
réévaluation. **Soldée (2026-10-25)** — tous les points de
réévaluation événementiels sont traités (§ Clôture) ; la question
d'isolation vit désormais dans ADR-0026.

## Contexte

ADR-0011 (phase 0) posait : budgets + cloisonnement de session
actifs, isolation processus différée, **réévaluation obligatoire
avant de brancher `dillo-js`** — exécuter du code arbitraire
change le modèle de menace : le script n'est plus une donnée
malformée mais un programme hostile.

Ce qui a été livré depuis, et qui constitue la réponse
technique :

- `dillo-js` s'exécute sur un **thread dédié par contexte**
  (`DilloEngine` = proxy canal, étape 34) — son tas `Rc`/`RefCell`
  est `!Send` et ne quitte jamais ce thread ; la session derrière
  le mutex API reste `Send` et ne peut pas être suspendue par un
  script.
- **Budget d'opérations borné** (`set_op_limit`,
  `SessionConfig::js_ops` = 5M) — une boucle infinie est tuée,
  jamais le pipeline.
- **Aucun `unsafe`** dans le code livré (`forbid(unsafe_code)`
  workspace ; seul `explorer-ffi` déroge à sa frontière C,
  validée et bornée — ADR-0018).
- **Miroir DOM** (ADR-0013) : le script ne voit jamais
  `explorer-dom` — il mute un `Rc` copié qui est resérialisé et
  re-parsé ; pas de `NodeId` exposé, pas de handle pendant
  possible, pas de corruption croisée d'arènes.
- **GC + rooting ADR-0008** : un `ObjId` réclamé résout `None`
  (dégradation, pas de dangling).
- `NullEngine` par défaut : le navigateur fonctionne sans JS.
- Les appels JS → hôte sont des **intentions drainées**
  (`NavRequest`, futur `drain_net_requests`) — jamais de callback
  ré-entrant dans le pipeline.

## Évaluation

| Vecteur | État | Résidu |
|---|---|---|
| Boucle infinie / DoS CPU | budget ops borné | un script peut allouer avant d'épuiser le budget — borne mémoire heap à instrumenter (JS-C37) |
| Mutation DOM hostile | confinée au miroir `Rc` | coût de resynchro O(document) par synchro — DoS par churn documenté, point d'extension diff prévu (ADR-0013) |
| Timers en bombe | drainés aux points de synchro + tick dédié | bornés : `max_timer_jobs` = 128 callbacks timer par pompe (étape 52) — le surplus reste en file pour la pompe suivante |
| XHR/fetch réseau depuis JS | câblé à l'étape 51 derrière la politique minimale ci-dessous | SSRF résiduel : une page hostile peut fetcher des cibles internes — mitigé par CORS minimal (réponse opaque sans ACAO) + quota, pas par un allowlist réseau |
| Code arbitraire → mémoire | éliminé côté maison par `forbid(unsafe_code)` | reste la surface des dépendances (`regex`, `fancy-regex` borné par `BACKTRACK_LIMIT`, `num-bigint`) |
| Crash du moteur JS | `JsError::Dead`, onglet dégradé | le thread est joint au `Drop` — un panic worker n'empoisonne pas la session |
| Mode FFI (`explorer-ffi`) | le moteur vit **dans le processus UI** — un crash logique résiduel peut emporter l'app (déviation assumée ADR-0018) | REST reste la référence ; le mode FFI est opt-in via `EXPLORER_ENGINE` |

## Décision

- **L'isolation processus reste différée** — justifiée : la
  classe dominante (memory-safety) est couverte par
  `forbid(unsafe_code)` ; le vecteur résiduel principal est le
  DoS, couvert par budgets + thread dédié + drain d'intentions.
  Site-isolation sur-dimensionnerait le palier actuel.
- **Prochains points de réévaluation fixes** (ne pas attendre une
  phase entière cette fois) :
  1. **au câblage XHR/fetch (étape 51)** — le JS touche le réseau
     pour la première fois : same-origin, `Referer`/`Origin`,
     mixed-content et quotas de requêtes script-initiées ;
  2. **au palier 3 / workers (JS-C29)** — exécution multi-thread
     de code hostile : réévaluer l'absence de sandbox ;
  3. **si le mode FFI devient le défaut** — la dégradation
     d'isolation d'ADR-0018 deviendrait le chemin principal.
- **Borne mémoire heap JS** à instrumenter (compteur d'objets/
  octets par contexte, kill doux au-delà) — étape JS-C37.
- Le daemon ne bind **jamais** hors loopback (inchangé,
  ADR-0003) ; la clé d'API reste la seule autorisation.

## Conséquences

- L'exigence d'ADR-0011 est formellement soldée ; les points de
  réévaluation sont désormais **événementiels** (câblage réseau
  JS, workers, défaut FFI), plus calendaires.
- L'étape 51 ne peut pas être cochée sans que la politique
  same-origin/CORS minimale soit implémentée et testée.
- `docs/etat-des-lieux-2026-10-19.md` §5 consignait cette dette ;
  elle est résolue par la présente ADR.

## Réévaluation à l'étape 51 (soldée)

Point de réévaluation prévu « au câblage XHR/fetch » — fait.
Politique livrée dans `explorer-core` (`script_request`,
`poll_net`, `after_script`) :

- **Same-origin** : requêtes libres — `Cookie` injecté à
  l'émission depuis le `CookieJar` partagé, `Set-Cookie` des
  réponses capturé dans le même jar (le `CookieTransport` du
  daemon partage le jar avec la session — navigations, XHR et
  `document.cookie` voient un état unique).
- **Cross-origin — CORS minimal** : limité aux *simple
  requests* (verbes `GET`/`HEAD`/`POST`, en-têtes safelistés
  `accept`, `accept-language`, `content-language`,
  `content-type`, `range`) ; `Origin` canonique posé à
  l'émission. Réponse **opaque** (`status: 0` → événement
  `error`) sauf `Access-Control-Allow-Origin: *` ou origine
  exacte. **Déviations MVP documentées** : pas de preflight
  `OPTIONS` (requête non-simple bloquée à l'émission), pas de
  `credentials` cross-origin (ni `Cookie` ni `Set-Cookie`), la
  valeur `content-type` n'est pas restreinte aux MIME
  safelistés.
- **Quota** : `SessionConfig::max_script_net` = 16 requêtes
  script-initiées en vol par onglet — au-delà, échec
  immédiat `status: 0` (page vivante, règle budgets).
- **En-têtes interdits au script** filtrés (`sec-*`,
  `proxy-*`, `Host`, `Cookie`, `Referer`, `Origin`…) — le
  navigateur garde le contrôle des en-têtes initiatés.
- **Mixed-content** filtré, `Referer`/`Origin` calculés à
  l'émission (`initiator_headers`), `Abort` moteur → annulation
  tokio, fermeture d'onglet → annulation de ses tâches.
- **Livraison** par intentions drainées + canal → points de
  synchro (`pump_net`), jamais de callback ré-entrant — la
  session reste `Send` et le pipeline ne peut pas être
  suspendu par un script.

Résidu assumé : le fetch cross-origin simple est **émis**
même si la réponse sera opaque (comportement navigateur
réel — l'exposition réseau reste ; l'info ne revient pas au
script). Tests : 8 scénarios d'intégration session
(`xhr_same_origin_fetches_and_renders`, opaque sans ACAO,
ACAO `*`, en-tête bloqué, quota, jar partagé, `Set-Cookie`
capturé, `localStorage` inter-documents).

## Amendement 2026-10-24 — tick timers (étape 52)

La ligne « Timers en bombe » est levée : la boucle
d'événements v2 arme un vrai battement (le daemon dort
jusqu'à `min(échéance timer, battement net)`, FFI expose le
tick), et le nombre de callbacks timer exécutés par pompe
est borné par `SessionConfig::max_timer_jobs` (défaut 128)
via `JsEngine::run_pending_bounded` — une page qui arme des
milliers de `setTimeout` échus ne monopolise pas un tick ;
le surplus reste en file, ordre conservé, pour la pompe
suivante. Les microtasks et livraisons `MutationObserver`
restent drainées intégralement au même point de synchro —
leur volume est borné par la sémantique (un job ne peut en
créer qu'un nombre fini par checkpoint). `MutationObserver`
est ainsi drainé aux mêmes sync points que les timers, comme
exigé par l'étape 52.

## Clôture 2026-10-25 — les deux derniers points de réévaluation

**Workers (JS-C29, livré)** — le déclencheur « exécution
multi-thread de code hostile » est tiré, et la surface livrée
est **bornée par construction** : un `Engine` par thread (le
tas `!Send` ne quitte jamais son thread — même schéma que le
contexte principal), `postMessage` en clone fil `WireValue`
(aucun handle de heap ne traverse), pool borné (`max_workers`
= 8, `queue_len` en back-pressure), budget ops propagé à
chaque worker, pas de `SharedArrayBuffer` inter-thread réel
(buffer posté = copié), `new Worker` sans fabrique =
`TypeError` honnête. Pas d'état partagé mutable inter-thread
→ aucune classe d'exploit nouvelle ; les budgets + le
confinement par thread couvrent le résidu comme pour le
contexte principal. Réévaluation : **maintien du différé** —
le confinement OS reste la trajectoire ADR-0026, qui liste
déjà « workers câblés » comme déclencheur P2 tiré.

**FFI devenu le chemin principal** — tiré : `engineMode:
'auto'` charge `explorer_ffi` dès que la DLL est présente, et
le journal 08/11 a acté « FFI premier rang » (canal push +
worker isolate, étape 107). Le coût isolationnel a été pesé
au moment de la bascule (2ᵉ audit externe, même journal) :
verdict amendé — FFI premier rang **pour la performance**,
daemon conservé comme **mode d'isolation documenté** pour la
navigation hostile, pas simple couture debug. La dégradation
d'isolation d'ADR-0018 devient donc le chemin principal **par
décision documentée**, avec une réponse nommée (le mode
daemon), pas par accident de défaut. ADR-0026 liste « FFI
devenant défaut » parmi les déclencheurs P2 — tiré.

**Bilan** : les trois points événementiels sont clos (étape
51 § ci-dessus, workers et FFI ici) et la borne heap JS-C37
est livrée. Cette ADR n'a plus de déclencheur propre : la
réévaluation d'isolation est définitivement déléguée à
ADR-0026 (P1 requis avant distribution ; P2 déclenché par
conditions — deux déjà tirées, le frein restant est le
séquencement « phase 10+ » du lead).
