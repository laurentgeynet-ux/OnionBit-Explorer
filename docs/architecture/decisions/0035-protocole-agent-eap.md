# ADR-0035 — Surface agent : protocole dédié bidirectionnel à handles (EAP)

- **Statut** : proposé (Étape 242 — design, aucune implémentation)
- **Date** : 2026-11-15
- **Périmètre** : `explorer-agent` (crate nouveau, NAV), `explorer-daemon`,
  `explorer-core` (additif), `explorer-api` (inchangé), `explorer-cli`
  (adaptateur)

## Contexte

La couture ADR-0003 (REST + SSE loopback) est dimensionnée pour **l'UI** :
pull de frames, événements broadcast, commandes ponctuelles sans état. Elle
sert déjà de porte aux vols agents (`docs/vol-agent.md`, `explorer-cli mcp`
≈ 10 verbes wrappant le REST) — mais à contretemps :

- **pas d'éléments adressables** : l'agent manipule des coordonnées brutes
  recalculées à chaque appel — la moindre mutation layout invalide le
  geste en vol ;
- **pas d'attente native** : « page posée », « élément visible »,
  « navigation commise » se reconstruisent par polling (`/dom`, `/eval`,
  SSE réécouté à la main) ;
- **deux canaux pour une conversation** : commandes en POST, vérité en
  SSE — aucune corrélation commande↔commit ;
- **pas de perception de forme modèle** : le dump DOM texte est un outil
  de triage humain, coûteux en tokens et sans sémantique d'action
  (rôle, nom, état, référence stable).

La référence du domaine (Playwright/CDP) montre la forme : canal
bidirectionnel unique, handles d'éléments conservés côté serveur,
actionabilité vérifiée à l'action, auto-wait intégré. Explorer peut le
faire **en natif et mieux** — trois atouts qu'un retrofit sur Chromium
n'a pas :

- le moteur possède toute la pile : actionabilité, hit-test et état de
  layout sont lisibles *dans le process*, sans aller-retour ;
- le single-writer (§6 AGENTS) **voit chaque commit** (nav, layout,
  paint, mutation DOM) — un wait accroché aux commits est strictement
  plus fiable qu'un polling distant ;
- les `NodeId` générationnels (ADR-0008) sont déjà des handles
  anti-obsolète — le contrat JS les utilise, l'agent hérite de la même
  discipline.

Contrainte de coexistence : l'API REST+SSE reste la couture UI — elle
n'est ni remplacée ni étendue. La surface agent est un **deuxième
client de `Session`**, pas une seconde couture moteur : tout passe par
les mêmes accès `Session` sous le même verrou tokio.

## Décision

### 1. Crate `explorer-agent` — protocole EAP v1

Un crate NAV nouveau porte le protocole **EAP** (*Explorer Agent
Protocol*) : types de messages, table de handles, moteur
d'actionabilité, serveur. `explorer-daemon` le montre sur le même
listener loopback que l'API — **un port, un daemon** ; `explorer-api`
est inchangée.

### 2. Transport canonique : WebSocket, JSON + binaire multiplexé

- Upgrade WS sur `GET /agent` via la feature `ws` d'axum 0.8
  (tokio-tungstenite en transitif — pur Rust, justifiée ici, règle §4.2).
- Messages texte **JSON-RPC 2.0** : requêtes/réponses corrélées par
  `id`, notifications serveur pour les événements souscrits — un seul
  socket porte la conversation entière (commandes + vérité), la
  corrélation commande↔commit est native.
- Messages **binaires** pour les charges lourdes : frames PNG, futures
  diffs raster — zéro base64.
- Sous-protocole `eap/1` annoncé en `Sec-WebSocket-Protocol` : la
  version se négocie dans le handshake HTTP, pas dans les messages.

### 3. Handshake à capacités — la clause d'évolution

Première requête obligatoire `session.open` : le client déclare
`protocol` + `capabilities` voulues ; le serveur répond version +
`capabilities` **livrées**. Un verbe non implémenté répond
`unsupported` (erreur typée), jamais un crash ni un comportement muet.
Une implémentation partielle est **légale par contrat** — le protocole
v1 est complet dès le jour 1, son remplissage suit la maturité moteur.

`capabilities` porte aussi la **sécurité** : la session agent reçoit un
profil (`perception` seule, `input` autorisé, `tabs` scopés, `eval`
armé). Bind loopback par défaut ; exposition externe = flag explicite
+ token obligatoire. Un agent qui clique est une surface d'exécution :
l'auth fait partie du protocole, pas d'une option ulterieure.

### 4. Handles d'éléments serveur — le cœur de la forme

- `snapshot`/`query` retournent des **refs opaques** (`"el:12"`)
  indexées dans une table par session agent :
  `ref → (TabId, NodeId, generation, doc_epoch)`.
- Toute action (`click`, `type`, `hover`, `scroll_into_view`,
  `get_rect`…) prend une `ref` — le serveur résout, vérifie la
  vivacité (génération + attachement, discipline ADR-0008/0033),
  mesure l'actionabilité, puis injecte l'`InputEvent` au même point
  que l'input réel de l'app.
- Expiration **honnête** : navigation, re-parse ou mort du nœud →
  résolution en erreur typée `stale_ref`, jamais d'action sur le
  mauvais élément.
- Politique de rétention documentée par commit (règle ADR-0033) : la
  table ne maintient rien en vie — cap borné + purge au sweep de doc.

### 5. Perception duale : carte sémantique + pixels

- `snapshot` = arbre sémantique compact : `ref`, rôle (mapping
  ARIA/tag borné v1), nom accessible, états (focus, disabled,
  checked…), rect. C'est la carte primaire de l'agent — dense en
  action, légère en tokens.
- `frame` = PNG binaire (même bitmap que `/frame` ADR-0004) pour les
  modèles vision.
- `snapshot{since: rev}` = **diff d'arbre** contre la révision connue
  — le coût dominant d'un agent IA est le token ; ne renvoyer que le
  sous-arbre changé est un levier que ni le DOM dump ni CDP n'offrent
  nativement.

### 6. Actionabilité + auto-wait natifs

Avant chaque action sur ref, le serveur vérifie **in-process** :
`visible` (style calculé + boîte posée non vide), `stable` (rect
identique sur deux commits layout consécutifs), `enabled`, `topmost`
(hit-test au centre de la boîte). L'action bloque jusqu'à actionnable
ou `timeout` (config, §4.13) — sémantique Playwright, évaluée sur le
box tree directement.

`wait_for(selector|nav|idle|frame)` s'accroche aux notifications de
commit du single-writer (point d'extension **additif** `explorer-core`
: hooks d'événements internes, pas de nouvelle émission I/O) — jamais
de polling.

### 7. Adaptateurs minces, protocole unique

Le schéma EAP v1 (JSON Schema versionné,
`crates/explorer-agent/schema/`) est la source de vérité — types Rust
générés, bindings Python/TS dérivés. Transports/adaptateurs, tous
minces :

- **WS** (canonique) — agents distants, SDK, boucles custom ;
- **stdio JSON-RPC** — mêmes trames sur stdin/stdout, zéro socket
  (spawn par un hôte local) ;
- **MCP** — `explorer-cli mcp` devient un adaptateur EAP→outils MCP
  (~25 verbes de convention Playwright-MCP : `navigate`,
  `snapshot`, `click_ref`, `type_ref`, `wait_for`, `screenshot`,
  `eval`…) ; MCP reste une porte interchangeable, jamais le protocole
  — il évolue trop vite pour être la base ;
- FFI : hors v1, le transport WS couvre le cas distant.

## Conséquences

**Positif** : forme de pilotage de niveau Playwright obtenue *mieux*
qu'en retrofit (handles réels, waits sur commits, actionabilité sans
round-trip) ; la couture UI n'est ni touchée ni détournée ; la
négociation de capacités rend tout remplissage incrémental non
cassant — c'est la réponse au « pas deux fois ».

**Coûts** : un schéma à versionner et des verrous de conformance à
écrire (tests golden de trames) ; la table de handles impose la même
discipline de rétention que l'arène miroir (ADR-0033) ; feature `ws`
d'axum = dépendance transitive nouvelle (tungstenite — pur Rust,
justifiée §2) ; `explorer-core` gagne un point d'extension additif
(hooks de commit) à journaliser dans `interface-equipes.md`.

**Hors périmètre v1** (déclarés `unsupported` à la négociation, pas
des trous du protocole) : interception/modification réseau, file
chooser, dialogues natifs (`alert`/`confirm`/`prompt`), PDF,
enregistrement vidéo, émulation device/géolocalisation — chacun est
une feature moteur indépendante, adressable ensuite sans casse de
contrat.

**Risque nommé** : l'actionabilité « stable » suppose deux commits
layout comparables — sur page à JS continu elle peut ne jamais se
stabiliser ; borne = timeout obligatoire + mode `force` explicite
(clic au dernier rect connu, verdict `degraded` rendu à l'agent).

## Séquencement

- **Étape 242** (cette ADR) : design — schéma, handles, capacités.
- E1 : crate + serveur WS + `session.open` + subset honnête
  (tabs/nav/snapshot-DOM/input/frame/wait-nav) — capacités déclarées.
- E2 : handles + actionabilité + auto-wait sur commits.
- E3 : snapshot sémantique + diff `{since}`.
- E4 : adaptateur MCP — les vols agents migrent d'`explorer-cli mcp`
  REST vers EAP.
