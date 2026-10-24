# ADR-0026 — Trajectoire sandbox OS : confinement du daemon, puis split broker/contenu

Statut : Proposée — trajectoire figée, implémentation
**différée par déclencheurs** (même méthode qu'ADR-0022 §6).
Complète ADR-0011 (isolation OS différée) et ADR-0019
(réévaluation JS in-process) : celles-ci fixaient *quand*
réévaluer, celle-ci fixe *vers quoi* — pour que la question
ne soit pas redérivée à chaque session.

## Contexte

ADR-0011 a différé l'isolation OS au MVP ; ADR-0019 a soldé
la réévaluation exigée par l'arrivée du JS in-process et
gardé l'isolation processus différée, avec des points de
réévaluation **événementiels** : workers multi-thread
(JS-C29), mode FFI devenant défaut. La « site isolation
multi-processus » figure au ROADMAP en « hors périmètre
assumé (documenté, non planifié) — candidat à un ADR
dédié ». Le présent document est cet ADR.

Mitigations actives aujourd'hui : `#![forbid(unsafe_code)]`
tous crates sauf la frontière bornée d'`explorer-ffi`
(ADR-0018 §5), budgets par onglet/page (ADR-0011),
thread dédié + proxy-canal JS avec miroir DOM sérialisé
(ADR-0013, ADR-0019), égress unique `explorer-net`
(ADR-0022), loopback + clé API (ADR-0003), daemon
redémarrable sans tuer l'UI.

Résidu qu'un sandbox viendrait plafonner : l'`unsafe`
**dans les dépendances** — `ring` (exception `*-sys`
consignée ADR-0002), `tiny-skia`, `image` (pur Rust mais
surface de décodage classique) — et les bugs logiques
(host functions JS, pipeline). Les effets de bord à
refuser : écriture disque hors profil, spawn de
processus, égress hors politique, lecture de fichiers
utilisateur.

## Décision

Trois paliers, ordonnés par coût/gain ; chacun a ses
déclencheurs (§ Séquencement). Les budgets restent la
première ligne de défense — le sandbox **plafonne le
blast-radius, il ne remplace rien**.

### P1 — Confinement du daemon entier (sans changement d'archi)

Le daemon est déjà un processus séparé ; on le spawne
**confiné, avant le premier octet hostile** :

- **Windows** : Job Object (limites mémoire/processus,
  kill-on-close, restrictions handles/UI) + token
  restreint / integrity level basse.
- **Linux** : seccomp-bpf + Landlock ; ou lanceur externe
  `bwrap`/`firejail`/`nsjail`.
- **macOS** : `sandbox-exec` avec profil seatbelt.
- **Lanceur externe, zéro code** : Windows Sandbox
  (`.wsb`), VM/WSL2, utilisateur dédié bas privilège +
  pare-feu — documentable dès maintenant comme
  « isolation mode v2 » pour navigation hostile.

Ordre de préférence pour l'`unsafe` (règle §4.1) :
crates à **API safe** (`win32job`, `seccompiler`,
`landlock` — unsafe interne toléré dans les
dépendances) > lanceur externe > crate dédié
`explorer-sandbox` à `allow(unsafe_code)` borné sur la
seule enveloppe syscall — précédent ADR-0018 §5, ADR
d'implémentation à ce moment. La qualification
`windows-sys`/`windows` (bindings DLL système, pas de C
compilé) sous la règle « pur Rust » est arbitrée dans
cette même ADR d'implémentation.

### P2 — Split broker/contenu (le vrai multi-processus)

- **Broker** = `explorer-api` + `explorer-net` (TLS
  incluse) + sessions/cookies/cache +
  `EgressConnector`. Processus privilégié.
- **Contenu** = `explorer-html/css/dom/js/jscore/layout/
  render` → framebuffer RGBA. Spawné **déjà confiné**
  (P1 à son maximum), **un par onglet** en v1.
- **Aucun socket côté contenu** : la règle §4.10 devient
  structurelle — `explorer-net` n'existe que dans le
  broker. Les fetchs passent par un
  `IpcTransport implements HttpTransport` tunnelé vers
  l'`EgressConnector` du broker : la couture injectable
  existe littéralement pour ça.
- **IPC** : un canal duplex typé unique (pipes nommés
  tokio / `interprocess`) — pas de bus généraliste
  façon Mojo.
- **Miroir DOM** : le format de traversée est la
  sérialisation déjà prévue pour la synchro JS
  (ADR-0013 ; journal de mutations ADR-0025 en vol).
- **Fontes** : `DirFontSource` lit le disque → variante
  « blob » alimentée par le broker avant/à travers le
  confinement (extension additive).
- **Frames** : RGBA brut par pipe d'abord ; shared-memory
  ensuite si le profiling l'exige (unsafe → ADR à ce
  moment — précédent ADR-0018 §2).
- **In-process conservé** : les mêmes traits permettent
  un mode broker+contenu fusionnés en loopback —
  `verify_all` et la culture test in-process ne changent
  pas.
- **Résidu nommé** : le broker termine TLS → `ring`
  parse des octets hostiles dans le processus
  *privilégié*. Même acceptation que la network-service
  Chromium. Options d'endurcissement (troisième
  processus net, TLS côté contenu) laissées à l'ADR
  d'implémentation.

### P3 — Site isolation par origine

Séparation cross-origin stricte (processus par site,
OOPIF). Hors périmètre — ADR propre **après** P2
atterri.

## Séquencement (déclencheurs)

- **Profil d'isolation externe documenté** (`.wsb`, VM,
  firejail + pare-feu) : libre immédiatement — docs
  seulement, aucun code.
- **P1** : étape NAVIGATEUR ouvrable dès maintenant ;
  devient **requise** avant toute distribution hors
  poste de développement ou avant que la navigation par
  défaut vise du contenu non fiable.
- **P2** : pas avant la phase 10+. Réévaluation
  obligatoire aux déclencheurs restants d'ADR-0019 —
  workers câblés (JS-C29 : code hostile multi-thread),
  FFI devenant le mode défaut — plus deux nouveaux :
  décodeurs média/image réels sur octets réseau (la
  classe d'exploit historique ; aujourd'hui stubs
  ADR-0023 + `image` pur Rust), et toute fonctionnalité
  exigeant du contenu un accès fichier/processus.
- **FFI (ADR-0018)** reste documenté « contenu de
  confiance » : in-process et confinement par onglet
  sont incompatibles par construction ; un défaut FFI
  est un déclencheur P2, pas un état durable.
- **P3** : ADR dédiée post-P2.

## Justification

- **Proportionnalité** : `forbid(unsafe_code)` couvre la
  classe dominante (corruption mémoire dans le code
  maison) ; le sandbox cappe le résidu (deps `unsafe`,
  bugs logiques). Viser plus fort aujourd'hui serait
  du sur-dimensionnement — l'argument d'ADR-0011 tient
  encore.
- **P2 maintenant figerait un protocole IPC au-dessus
  d'un pipeline qui bouge** : la synchro DOM elle-même
  migre du re-parse au journal (ADR-0025, pilote
  scopé). Figer la traversée sur le format d'aujourd'hui
  serait à refaire.
- **Les coutures ont été dessinées dans ce sens** :
  transport injectable, moteur JS proxy-canal,
  single-writer par onglet, sortie framebuffer, net
  unique. Documenter la trajectoire empêche de la
  redériver — le motif exact d'ADR-0022.
- **P1 est presque gratuit** : le mode daemon séparé
  est déjà la référence (ADR-0003) ; y ajouter un
  Job Object ou un lanceur confiné couvre l'essentiel
  du blast-radius pour une fraction du coût P2.
- Alternative écartée : exécuter `explorer-jscore` sous
  wasmtime (sandbox by construction) — arbre de
  dépendances et coût CPU disproportionnés ; option de
  recherche, non planifiée.

## Conséquences

- **Checklist étapes** : toute fonctionnalité touchant
  une ressource (fichier, fonte, socket, processus)
  déclare désormais **son côté de la couture
  broker/contenu** — exigence symétrique à la
  déclaration d'égress par mode d'ADR-0022.
- `DirFontSource` → variante blob identifiée comme
  extension future (additif, contrat PIPELINE —
  passage par `docs/interface-equipes.md` le jour venu).
- Le broker P2 reste exposé aux octets TLS hostiles via
  `ring` — résidu assumé et nommé, pas découvert.
- Le ROADMAP « site isolation multi-processus » pointe
  désormais sur cette ADR : le statut passe de « non
  planifié » à « trajectoire figée, déclenchée par
  conditions ».
- Aucun code modifié par cette ADR ; `forbid(unsafe_code)`
  et la règle pur Rust inchangés.
