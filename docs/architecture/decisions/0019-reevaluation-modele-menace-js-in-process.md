# ADR-0019 — Réévaluation du modèle de menace : JS hostile in-process

Statut : Acceptée (2026-10-19) — réévaluation exigée par
**ADR-0011** (« réévaluation obligatoire en phase 8 »), écrite
*a posteriori* : la phase 8 (étapes 34–36) est livrée, `dillo-js`
exécute du JavaScript réel dans le processus du navigateur. Cette
ADR solde la dette documentaire et fixe les prochains points de
réévaluation.

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
| Timers en bombe | drainés aux points de synchro | le tick dédié (étape 52) doit borner le nombre de jobs par tick |
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
