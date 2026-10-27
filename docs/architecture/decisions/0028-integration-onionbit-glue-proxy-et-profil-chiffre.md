# ADR-0028 — Intégration OnionBit : topologie deux daemons, glue proxy loopback, profil chiffré

Statut : Proposée — trajectoire d'intégration figée, implémentation
par étapes déclenchées (même méthode qu'ADR-0022 §6). Complète
ADR-0026 (qui fixe *vers quoi* aller pour l'isolation) en nommant
*où vit le navigateur* dans le produit réel, et ADR-0022 dont
la variante d'égress est ici précisée.

## Contexte

Le produit cible est un **mix bidirectionnel, portable dans tous
les cas** :

- **Explorer vanilla** — le navigateur portable seul, égress
  direct ou proxy utilisateur.
- **OnionBit-Explorer** — le même binaire navigateur embarqué
  dans le produit OnionBit (remplacement du Dillo historique),
  égress via la tuyauterie OnionBit (tunnels IPv8, Tor/arti,
  stealth anticensure, messagerie double-adressage, zones
  `public\`/`private\`).

La plateforme OnionBit (bundle 1.2.0 déployé, source plus ancienne
dans `D:\Projet\OnionBit`) a déjà livré, vérifié sur artefacts :

- `onionbit-daemon` supervisé par l'UI (spawn détaché, sonde
  alive), `dist\` portable, `state\` à côté des exécutables ;
- `onionbit-browser.exe` = le Dillo, **processus indépendant
  « glué »** : profil `state\browser\run-<hash>\.dillo\dillorc`
  régénéré par lancement, `http_proxy="http://127.0.0.1:<port>/"`
  + `http_proxyuser` — un **proxy HTTP CONNECT loopback**, pas du
  SOCKS5 ;
- la tuyauterie complète : circuits IPv8, `Tor` via **arti**
  in-process, stealth transport (datagrammes morphés, bridges
  `onionbit-bridge://` hors-bande, cover traffic), messagerie
  décentralisée à double adresse (IPv8 + `.onion`), store
  `data\private\` à **fichiers chiffrés individuellement**
  (`manifest.obm` = ciphertext), zone `data\public\` claire
  explicite ;
- `onionbit-network-policy` : crate feuille de gardes pures —
  `IpPolicy` anti-SSRF, `KillSwitch` scopé, `exit_policy`,
  `proxy_guard`.

Constats structurants :

1. **La colle la plus simple existe déjà** : un fichier de
   config régénéré + un proxy HTTP loopback + un état par
   profil. `explorer-net` a **déjà** `ProxyConnector`
   (absolute-form `http:`, tunnel `CONNECT` `https:`,
   `http_proxy`/`no_proxy` honorés — étape 8). Le contrat de
   glue Dillo est satisfait nativement par Explorer.
2. **Le slot navigateur est un processus séparé** aujourd'hui
   (`onionbit-browser.exe`) — la topologie à deux daemons est
   en production, supervisée par l'UI.
3. **FFI in-process est exclu du produit mix** : le renderer
   dans le processus qui détient les clés de tunnel et
   l'identité = pire cas (ADR-0026 le borne déjà à « contenu de
   confiance »). Le navigateur vit en daemon séparé **dans tous
   les produits**.
4. **`windows-sys` FFI borné est déjà en usage** dans
   `onionbit-daemon` (`instance.rs`, mutex CreateMutexW) —
   l'échelle d'enveloppe syscall est prouvée.
5. Licences : OnionBit = GPL-3.0, Explorer = MIT/Apache-2.0.
   La distribution « OnionBit-Explorer » combinée est GPL par
   inclusion ; **Explorer vanilla ne lie rien de GPL** — la
   frontière propre est le protocole (HTTP proxy), pas un
   `path`-dep.

## Décision

### T1 — Topologie : deux daemons, jamais de fusion d'adressage

- `explorer-daemon` = le navigateur (processus confiné), prend
  le slot de `onionbit-browser.exe` : même supervision
  (spawn, sonde alive, respawn), même état par profil
  `state\browser\<profil>\`.
- `onionbit-daemon` = la tuyauterie (tunnels, arti, SOCKS5/HTTP
  proxy, messagerie, DB) — processus privilégié réseau.
- « OnionBit dans Explorer » et « Explorer dans OnionBit » =
  **même trio de processus, deux skins de produit** — la
  direction de l'intégration est une question de packaging/UX,
  jamais de partage d'espace mémoire entre le renderer et la
  tuyauterie.
- À terme (ADR-0026 P2), la tuyauterie est la couche égress du
  **broker** — contenu (onglets) sans socket par construction.

### T2 — Glue contract : proxy HTTP CONNECT loopback

- Égress navigateur en mode OnionBit = **`HttpProxy` loopback
  uniquement** : le connecteur `Direct` n'est **pas construit**
  (capacité, pas flag — la thèse ADR-0022 : le code ne peut
  pas fuir ce à quoi il n'a pas accès). Kill switch structurel
  en bonus : proxy mort = toute connexion échoue.
- Le glue = config de profil régénérée par lancement
  (`http_proxy` + credentials + `state\browser\<profil>\`),
  comme `dillorc` aujourd'hui. `ProxyConfig` doit se charger
  **depuis le profil/config**, pas seulement l'environnement.
- SOCKS5 reste une variante possible (`EgressConnector::Socks5`)
  — non requise : le proxy HTTP couvre déjà `.onion` (prouvé
  terrain TPB via Dillo). `arti` direct dans `explorer-net`
  n'est pas nécessaire pour le mix (l'arbre est déjà payé côté
  OnionBit ; ADR-0022 garde la variante pour vanilla/Tor).

### T3 — Confinement par rôle

- **`explorer-daemon`** : `explorer-sandbox` (crate interne
  `allow(unsafe_code)` borné sur l'enveloppe syscall — précédent
  ADR-0018 §5, nommé ADR-0026) — palier 1 :
  `SetProcessMitigationPolicy` (win32k off — daemon headless —
  no-child-process, block-remote-image-loads, dynamic-code) +
  Job Object auto-assigné (mémoire, CPU, kill-on-close, UI).
  Palier 2 : AppContainer **sans `internetClient`** — zéro socket
  internet structurel ; même package SID que `onionbit-daemon`
  ⇒ le loopback proxy reste ouvert entre eux (l'inbound
  cross-container est le seul point dur Windows ; le canal API
  REST peut migrer pipes/UDS si besoin — AF_UNIX existe sous
  Windows et tokio).
- **`onionbit-daemon`** : profil « réseau ouvert, tout le reste
  fermé » — FS limité à `state\`/`data\`, spawn refusé,
  mémoire bornée. Il émet : pas de coupure réseau possible.
- **UI** : non confinée (processus de confiance) — mais aucun
  contenu hostile ne s'y exécute jamais (ADR-0003).
- Profils d'isolation externe (`.wsb`, VM, utilisateur dédié +
  ACLs + pare-feu `netsh` par programme) : documentés pour
  navigation hostile — **attention portable** : tout levier
  admin (ACLs, firewall, `.wsb` Windows Home absent) est hors
  portable ; FAT32/exFAT rend les ACLs inopérantes → le
  confinement auto-appliqué (Job/mitigations/AC) est le seul
  garanti en portable.

### T4 — Profil chiffré par fichier, passphrase ou éphémère

- **`ProfileVault`** : port de stockage dans `explorer-core` —
  `seal(path, bytes)`/`open(path)` AEAD (ChaCha20-Poly1305,
  primitives pur-Rust déjà validées côté OnionBit) appliqué à
  `profile_store`, `CookieJar` persistant, cache disque,
  méta downloads — adaptateur derrière les points d'écriture
  existants, aucun changement d'archi.
- **Session user** : `passphrase → Argon2id → seed → HKDF` →
  clé profil (+ même racine pour l'identité nœud côté OnionBit).
  Une passphrase = profil + identité + adresses de messagerie.
- **Mode invité** : clé `getrandom` en RAM, **jamais persistée**
  (`persist_keys: bool` au profil) — persona éphémère complète :
  keypair, adresses, fichiers, contacts. Fermeture = drop clé +
  delete best-effort → **crypto-shredding** : ce qui reste est
  du bruit irrécupérable, y compris sur FAT32 (supérieur à
  l'effacement de fichiers, indépendant du filesystem).
  GC au démarrage des répertoires `run-*`/`guest-*` orphelins.
- **Export** : `private\` → `public\` (ou copie externe) =
  franchissement explicite de la frontière, commun aux deux
  modes ; `public\` est la seule zone claire par construction.
- Résidus assumés : clé en RAM peut toucher le pagefile
  (mitigation `VirtualLock` dans `explorer-sandbox`),
  passphrase oubliée = profil mort, mode invité couvre
  l'at-rest pas la session en cours.

### T5 — Gardes reprises d'OnionBit (ports conceptuels)

- **`IpPolicy` anti-SSRF** : refus loopback/privé/link-local sur
  les requêtes **initiées par le contenu** (scripts, sous-
  ressources) — placée *avant* le proxy (empêche `CONNECT
  192.168.x.x` via le tunnel). Port conceptuel : réécriture,
  pas copie (frontière de licence).
- **`KillSwitch` scopé** : égress navigateur conditionné aux
  signaux tunnels (`circuits ready`, stealth actif) — exposés
  par l'API diagnostics d'OnionBit ; le navigateur se tait
  avant bootstrap, il ne décide pas de l'état.
- **Checklist étapes** : toute fonctionnalité à effet réseau ou
  ressource déclare son mode (`Direct`/`Proxy`) et son côté
  broker/contenu — prolonge ADR-0022/ADR-0026.

## Séquencement (déclencheurs)

1. **Docs libres immédiates** : profil d'isolation externe
   (`.wsb`/VM/firejail) + mode invité décrit.
2. **Étape NAV — `explorer-sandbox` P1-v1** : mitigations +
   Job auto-assign (~200 lignes FFI) — ouvrable dès maintenant ;
   devient requise avant distribution (déclencheur ADR-0026).
   L'ADR d'implémentation arbitre `win32job`/`rappct` vs crate
   borné + qualification `windows-sys` sous « pur Rust ».
3. **Étape NAV — glue profil** : `ProxyConfig` depuis profil +
   mode proxy-only (pas de `Direct` construit) + credentials +
   `state\browser\<profil>\`.
4. **Étape NAV — `ProfileVault`** : AEAD par fichier,
   passphrase/invité, export private→public, gc orphelins.
5. **Étape NAV — gardes** : `IpPolicy` contenu + kill-switch
   scopes consommés de l'API tunnels.
6. **Étape NAV — AppContainer** : même package SID, sans
   `internetClient` côté navigateur (+ pipes/UDS si requis).
7. P2/P3 d'ADR-0026 inchangés ; la tuyauterie OnionBit devient
   la couche égress du broker.
8. Distribution : code-signing du zip portable (SmartScreen) au
   premier packaging.

## Justification

- **Minimalité prouvée** : la glue Dillo (config régénérée +
  proxy loopback + état par profil) est en production et rend
  `.onion` — on ne peut pas faire plus simple ; SOCKS5/arti
  ajoutent un protocole ou un arbre pour zéro gain dans le mix.
- **Capacité > flag** : en mode OnionBit, le `Direct` n'existe
  pas dans la config — conforme au principe directeur ADR-0022
  « empêcher > détecter ».
- **Crypto-shredding > delete** : destruction d'une clé RAM =
  wipe complet indépendant du filesystem — la seule garantie
  qui tienne en portable FAT32.
- **Blast-radius** : renderer et tuyauterie sont deux surfaces
  hostiles distinctes (contenu web vs parsers UDP des tunnels)
  — les séparer en processus les confine l'une de l'autre ;
  fusionner les adressages doublerait l'impact dans les deux
  sens.
- **Le coût OS confinement est plafonné par ADR-0026** : P1
  d'abord (peu cher), P2 ensuite (le split `explorer-core`,
  ~22 500 lignes de session — le vrai coût, repoussé).

## Conséquences

- `explorer-net` gagne `ProxyConfig`-depuis-profil + mode
  proxy-only (additif, pas de casse).
- `explorer-core` gagne le port `ProfileVault` (additif) ;
  les écritures store/cache/cookies/downloads passent dessus.
- Nouveau crate `explorer-sandbox` (Windows d'abord) — seconde
  exception `allow(unsafe_code)` documentée après ADR-0018.
- ADR-0022 amendée de fait : la variante d'égress du mix est
  `HttpProxy` (pas `arti`); `arti`/Tor reste la branche vanilla.
- ADR-0026 confirmée : le navigateur = daemon séparé confiné
  dans tous les produits ; FFI hors produit mix.
- Les étapes NAV listées au § Séquencement sont à créer dans
  `docs/roadmap-equipe-navigateur.md` ; entrées correspondantes
  au journal `docs/interface-equipes.md`.
- Rien n'exige de changer les crates PIPELINE (`BlobFontSource`
  reste la seule extension prévue, via le journal, quand P2
  arrive).
