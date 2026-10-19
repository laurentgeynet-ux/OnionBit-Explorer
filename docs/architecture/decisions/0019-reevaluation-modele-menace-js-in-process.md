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
| XHR/fetch réseau depuis JS | **non encore câblé** — risque SSRF depuis le daemon loopback au moment du câblage | same-origin + CORS minimal exigés à l'étape 51 ; le transport JS hérite des politiques `HttpTransport` |
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
