# ADR-0002 — Pile réseau : HTTP/1.1 maison sur tokio + rustls

Statut : Acceptée (2026-10-18).

## Contexte

Le moteur a besoin d'un transport HTTP. Options : `reqwest` (mature,
mais c'est un client HTTP complet — contraire à l'ambition « moteur
écrit ici »), `hyper` (serveur/client générique, moitié du chemin
déjà fait), ou client HTTP/1.1 écrit dans `explorer-net`.

## Décision

Client HTTP/1.1 **maison** dans `explorer-net` : connexions `tokio`,
TLS `rustls` + `webpki-roots`, parsing de statut/en-têtes via
`httparse` (crate borné, zéro-allocation — jamais de parseur maison
sur les en-têtes), corps `Content-Length` + `chunked`, décodage
`gzip`/`deflate`/`br` par crates pur Rust (`flate2` backend
`miniz_oxide` — pas de `zlib-sys`).

- HTTP/2, HTTP/3, WebSocket : pas dans l'implémentation initiale —
  étapes de la phase 10 « compatibilité web moderne » (ROADMAP.md),
  chacune avec son ADR.
- `http_proxy`/`no_proxy` honorés — fonctionnalité standard des
  navigateurs ; la couture reste ouverte à tout transport
  mandataire futur (proxy d'entreprise, Tor…) sans réécriture.
- Trait `HttpTransport` injectable : tests in-process, transport
  proxy futur sans réécriture.
- URL : normalisation WHATWG, **IDNA/punycode** — arbitré en faveur
  de la crate `idna` (pur Rust, UTS-46, étape 3),
  schémas `data:` et `about:` minimaux en plus de `http(s)`.
- Politique document : store **HSTS** mémoire borné,
  `X-Content-Type-Options` honoré, charset résolu par sniffing
  WHATWG (BOM → `<meta>` → défaut `windows-1252`), transcodage par
  crate pur Rust type `encoding_rs` ou tables maison — à arbitrer
  en phase 1.
- Politique cookies dès le jar initial : `Secure`/`HttpOnly`/
  `SameSite`, préfixes `__Host-`/`__Secure-`, et domaines validés
  par **Public Suffix List** — sans PSL, un jar naïf accepte des
  supercookies sur `.co.uk` (ROADMAP étape 7).

## Justification

- HTTP/1.1 côté client est un protocole à la portée d'un moteur
  maison (requêtes séquentielles, keep-alive simple) ; l'écrire est
  cohérent avec ADR-0001.
- `reqwest`/`hyper` cacheraient exactement les mécaniques que ce
  projet veut posséder (connexions, codages, redirections).
- `httparse` est du wire-format, pas du moteur : l'utiliser ne
  diminue pas la mission et élimine une classe de bugs. *(Arbitré
  étape 4 : `tokio` sockets/timers, `httparse`, `async-trait` pour
  les traits dyn-compatibles `HttpTransport`/`Connector`.)*
- Étape 5 : `rustls` + `tokio-rustls` + `webpki-roots` épinglés.
  **Exception `*-sys*` consignée** : le provider crypto est `ring`
  (C/assembleur audité). Aucune pile TLS viable n'est pure Rust ;
  `ring` se build Windows sans cmake (contrairement à
  `aws-lc-rs`, exclu de l'arbre). C'est la seule dépendance à
  liaison C du workspace — justifiée par l'infrastructure, pas le
  moteur.

## Conséquences

- Interop réelle à valider par tests d'intégration loopback (serveur
  axum local) dès la phase 1 — pas de dépendance à un hôte externe
  en CI.
- Une page nécessitant HTTP/2-only échouera proprement (erreur
  typée remontée à l'UI), pas de contournement silencieux.
- `windows-1252` arrive dès la phase 1/2 : c'est le défaut de la
  spec HTML pour la plupart des locales — le palier 1
  « documentaire » en dépend, pas la phase 10.
- Étape 8 : la politique réseau (redirections, HSTS, Referer/Origin,
  mixed-content) est implémentée en **décorateurs `HttpTransport`
  empilables** — la couture injectable suffit, aucun canal
  additionnel n'est nécessaire. Le proxy s'insère plus bas, au
  niveau `Connector` : `Connector::absolute_form()` décide la
  request-target origin-form vs absolute-form (HTTP via
  mandataire), et `TlsConnector` accepte un `inner` injectable —
  un tunnel `CONNECT` `ProxyConnector` y est composé pour poser
  TLS **sur** le tunnel, en réutilisant le même chemin H1.
