# Architecture — OnionBit-Explorer

Cartographie des couches et des crates cibles. Les décisions
structurantes sont dans `docs/architecture/decisions/` (ADR).

## Vue d'ensemble

```
┌─────────────────────────────────────────────────────┐
│ app/  Flutter desktop                               │
│   lib/core      api client (REST+SSE), tokens, di   │
│   lib/features  browser, settings, history…         │
└──────────────┬──────────────────────────────────────┘
               │ HTTP loopback : REST + SSE + PNG frames
┌──────────────▼──────────────────────────────────────┐
│ explorer-daemon   processus hôte (binaire)          │
│ explorer-api      axum : routes, DTO, SSE, auth     │
├─────────────────────────────────────────────────────┤
│ explorer-core     session, onglets, pipeline        │
│                   fetch→parse→style→layout→render   │
├──────────────┬──────────────┬───────────────────────┤
│ explorer-net │ explorer-js  │  ports (traits)       │
│ HTTP/TLS/DNS │ trait        │  injectés dans core   │
│ cache/cookie │ JsEngine     │                       │
├──────────────┴──────────────┴───────────────────────┤
│ explorer-dom · explorer-html · explorer-css         │
│ explorer-layout · explorer-render                   │
│              (domaine pur — zéro I/O)               │
├─────────────────────────────────────────────────────┤
│ explorer-common (types, erreurs, config)            │
└─────────────────────────────────────────────────────┘
```

## Clean architecture (règle de dépendance)

| Couche | Crates | Connaît |
|---|---|---|
| Domaine | `dom`, `html`, `css`, `layout` | rien d'externe — pur |
| Application | `core` | ports (`HttpTransport`, `JsEngine`) |
| Infrastructure | `net`, `render`, impl JS | tokio, rustls, tiny-skia |
| Interface | `api`, `daemon`, `cli`, `app/` | adaptateurs seulement |

Les couches internes ignorent les externes. Les DTO REST ne fuient
pas dans `core` ; le DOM ne connaît pas axum.

## Responsabilités des crates

| Crate | Rôle | Phase |
|---|---|---|
| `explorer-common` | types partagés, erreurs racines, version | 0 |
| `explorer-net` | URL, DNS, HTTP/1.1, TLS, décodage, cookies, cache | 1 |
| `explorer-dom` | arène DOM + `NodeId`, mutations | 2 |
| `explorer-html` | tokenizer + tree-builder WHATWG | 2 |
| `explorer-css` | tokenizer, sélecteurs, cascade, computed style | 3 |
| `explorer-layout` | arbre de boîtes, block/inline, tables, floats | 4 |
| `explorer-render` | display list → framebuffer RGBA → PNG | 5 |
| `explorer-js` | trait `JsEngine` + bindings DOM | 8 |
| `explorer-core` | session : onglets, pipeline, bus d'événements | 0→ |
| `explorer-api` | REST + SSE axum, auth par clé | 6 |
| `explorer-cli` | binaire dev (fetch, dump DOM, shot) | 0→ |
| `explorer-daemon` | binaire hôte API loopback | 6 |
| `explorer-test-support` | transports simulés, fixtures | 0→ |

## Contrat de couture UI (ADR-0003/0004)

- `GET  /api/status` — état du moteur (diagnostic).
- `GET/POST /api/tabs`, `DELETE /api/tabs/{id}` — cycle de vie.
- `POST /api/tabs/{id}/navigate|reload|back|forward` — navigation.
- `GET  /api/tabs/{id}/frame` — PNG du framebuffer (pull).
- `POST /api/tabs/{id}/input` — souris/clavier/scroll.
- `GET  /api/events` — SSE : `tab_state`, `frame_invalidated`,
  `title_changed`, `events_lost`.
- Erreurs : `{"error": {"handled": true, "message": "…"}}`.
- Bind `127.0.0.1` ; clé d'API optionnelle (`x-api-key` ou `?key=`).

## Coutures gardées ouvertes (suite OnionBit)

- `HttpTransport` injectable → un futur transport proxy Tor est une
  implémentation, pas une réécriture.
- `http_proxy`/`no_proxy` honorés par l'environnement.
- Répertoire de profil déplaçable (cookies, historique, bookmarks).
- Ces coutures sont un contexte, pas un ordre de travail — cf. la
  note équivalente côté DilloRust (`docs/onionbit-suite.md`).
