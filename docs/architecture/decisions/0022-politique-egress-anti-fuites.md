# ADR-0022 — Politique d'égress unique et détection de fuites (préparation Tor)

Statut : Acceptée (2026-10-20) — politique figée, implémentation
**différée** : chaque niveau a son déclencheur (§ Séquencement).

## Contexte

Explorer se prépare, à terme, un transport Tor via `arti-client`
(pur Rust — horizon produit, non introduit ici). AGENTS.md §9
délègue déjà la politique réseau (mandataire, filtrage, `.onion`)
à l'application qui embarque le moteur et garde les coutures
ouvertes (`HttpTransport` injectable, `http_proxy`/`no_proxy`
honorés — architecture.md « Coutures gardées ouvertes »).

Risque visé : la **fuite** — tout trafic qui contourne le chemin
prévu : résolution DNS système, socket directe, UDP (QUIC/HTTP3,
STUN/WebRTC, DoH), OCSP/CRL/AIA, prefetch/preconnect, `Alt-Svc`,
`report-uri`, PAC file, captive portal, IPv6/happy-eyeballs,
`<a ping>`, WebSocket. Un navigateur compte des dizaines de
vecteurs ; un mode Tor n'a de sens que si la liste est
*fermée*, pas seulement surveillée.

Atout existant : AGENTS.md §4.10 — sockets uniquement dans
`explorer-net` (ADR-0002). Tout l'I/O passe déjà par un point
unique ; cette ADR le rend contractuel et le prépare.

Principe directeur : **empêcher > détecter**. Un détecteur ne
mesure que ce qu'il observe — la conception doit rendre la
fuite inatteignable avant de la tracer.

## Décision

1. **Connecteur d'égress unique — capacité, pas flag.**
   `explorer-net` exposera un `EgressConnector` : seule fabrique
   de flux sortants (TCP+TLS aujourd'hui, datagrammes si l'UDP
   existait un jour). Variantes : `Direct` (dialer TCP/TLS
   actuel) et, le jour venu, `Tor` (client `arti-client`). En
   mode Tor **la variante ne contient aucun dialer direct** —
   le code ne peut pas fuir ce à quoi il n'a pas accès. Le
   reste du moteur ne manipule que des flux
   (`AsyncRead + AsyncWrite` ; `arti_client::DataStream` les
   implémente), jamais d'adresses.

2. **DNS en mode Tor : jamais local.** Le connecteur reçoit le
   *hostname* non résolu ; la résolution est déléguée au nœud de
   sortie (cellule `RESOLVE`). Aucun `lookup_host`,
   `ToSocketAddrs` ou `getaddrinfo` dans le chemin Tor — premier
   vecteur de fuite d'un navigateur.

3. **UDP : absence, pas contournement.** Arti est TCP-only ; en
   mode Tor aucune socket UDP n'est créée — QUIC/HTTP3,
   STUN/WebRTC et DoH sont **désactivés**, pas « routés ». Toute
   feature UDP future déclare son comportement par mode
   d'égress.

4. **Détection à trois niveaux.**
   - *Lint-time* (gate `verify_all`) : `scripts/check_egress.ps1`
     + `clippy.toml` `disallowed-methods` — `std::net::*`,
     `tokio::net::{TcpStream, UdpSocket, lookup_host}`,
     `ToSocketAddrs` interdits hors `explorer-net` ; `cargo-deny`
     `bans` : `reqwest`, `hyper`, `hickory-dns`/`trust-dns`,
     `quinn`, `native-tls`, `schannel` exclus de l'arbre (règle
     §4.2 — pur Rust déjà exigé).
   - *Runtime* : compteur de flux dans `explorer-net` — chaque
     flux sortant enregistré `(cible, transport: Direct|Tor,
     octets)`, exposé via l'API (`/debug/egress` ou champ d'un
     endpoint existant) pour les tests et l'UI.
   - *E2E canari* : serveur canari dans `explorer-test-support`
     (HTTP + DNS locaux ; plus tard domaine public dédié) +
     **pages piégées** couvrant le catalogue de vecteurs —
     `<img>`, `css url()`, favicon, `dns-prefetch`/`preconnect`/
     `prefetch`, `<a ping>`, `report-uri`/CSP, `Alt-Svc`,
     redirections, IPv6, WebSocket. Assertion : en mode Tor,
     zéro hit canari sans flux Tor corrélé (et à maturité, IP
     source ∈ liste publique des exits — CollecTor). Audit
     OS-level aux jalons : daemon en environnement n'autorisant
     que les guards (sandbox/VM/netns) — preuve par incapacité.

5. **Sémantique `no_proxy`/PAC en mode Tor.** `http_proxy` /
   `no_proxy` restent honorés en mode `Direct` ; en mode Tor,
   `no_proxy` est **ignoré** — le respecter contournerait Tor,
   c'est-à-dire fuir. Documenté dans la config du profil.

6. **Séquencement (déclencheurs).**
   - Lint + bans cargo-deny : libre dès maintenant — verrouille
     le périmètre avant Arti (étape NAVIGATEUR à ouvrir).
   - `EgressConnector` + compteur de flux : avec l'ADR
     d'introduction d'`arti-client` (arbre de dépendances
     conséquent — règle §4.2, ADR dédié à ce moment-là).
   - Canari e2e : après le connecteur ; fixtures dans
     `explorer-test-support` (crate partagé §10 → entrée
     `docs/interface-equipes.md`).
   - Isolation de circuits par onglet (`arti_client::isolation`)
     : évaluée au même moment — prolonge le cloisonnement de
     session d'ADR-0011.

## Justification

- La détection seule ne prouve que ce qu'elle mesure ; la
  structure par capacité (point 1) rend la fuite inatteignable
  par le code — les niveaux du point 4 servent de preuve et de
  filet contre les erreurs de conception, pas de ligne de
  défense principale.
- `explorer-net` est déjà le point d'étranglement (§4.10,
  ADR-0002) : formaliser `EgressConnector` prolonge une règle
  respectée au lieu d'inventer un mécanisme.
- La méthodologie canari + audits (celle du QA Tor) est l'état
  de l'art mesurable — compatible avec la culture corpus du
  projet (ADR-0010) : une fuite se rapporte « x hits directs /
  y vecteurs testés », jamais « ça a l'air étanche ».
- Ne rien changer au code aujourd'hui est assumé : le moteur
  est en plein développement ; l'ADR fige le contrat pour que
  les étapes futures n'aient pas à le redécouvrir.

## Conséquences

- Toute fonctionnalité à effet réseau (prefetch, `report-uri`,
  `Alt-Svc`, WebRTC, HTTP/3, PAC…) **déclare son comportement
  par mode d'égress** — ajout à la checklist des étapes réseau.
- `arti-client` reste à arbitrer dans son ADR dédié (taille de
  l'arbre de dépendances, `DataStream`, isolation par circuit,
  pluggable transports `ptmgr`) — cette ADR n'introduit aucune
  dépendance.
- Alternative écartée : proxy SOCKS5 externe (démon Tor) —
  reste possible via `HttpTransport` (couture déjà ouverte),
  mais l'intégration privilégiée est in-process.
- Audit OS-level non continu : `pktmon`/sandbox en jalon, pas
  dans `verify_all` — trop lourd pour un gate quotidien.
