# Architecture — Explorer

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
│                   (sous-ressources + invalidation,  │
│                   ADR-0012)                         │
├──────────────┬──────────────┬───────────────────────┤
│ explorer-net │ explorer-js  │  ports (traits)       │
│ HTTP/TLS/DNS │ trait        │  injectés dans core   │
│ cache/cookie │ JsEngine     │                       │
│              │ dillo-js     │  moteur JS autonome — │
│              │ (workspace)  │  équipe NAVIGATEUR    │
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
| `explorer-dom` | arène DOM + `NodeId` générationnels (slotmap), mutations | 2 |
| `explorer-html` | tokenizer + tree-builder WHATWG | 2 |
| `explorer-css` | tokenizer, sélecteurs, cascade, computed style | 3 |
| `explorer-layout` | arbre de boîtes, block/inline, tables, floats, positionnement ; fontes via port `FontProvider` injecté | 4 |
| `explorer-render` | display list → framebuffer RGBA → PNG / buffer brut | 5 |
| `explorer-js` | trait `JsEngine` + bindings DOM + table de roots (ADR-0008) | 8 |
| `dillo-js` | moteur JS autonome — équipe NAVIGATEUR (spécialiste JS), version canonique en workspace, licence MIT/Apache | 8 |
| `explorer-core` | session : onglets, pipeline, bus d'événements | 0→ |
| `explorer-api` | REST + SSE axum, auth par clé | 6 |
| `explorer-cli` | binaire dev (fetch, dump DOM, shot, debug layout) | 0→ |
| `explorer-daemon` | binaire hôte API loopback | 6 |
| `explorer-test-support` | transports simulés, fixtures | 0→ |

## Contrat de couture UI (ADR-0003/0004)

- `GET  /api/status` — état du moteur (diagnostic).
- `GET/POST /api/tabs`, `DELETE /api/tabs/{id}` — cycle de vie.
- `POST /api/tabs/{id}/navigate|reload|back|forward` — navigation.
- `GET  /api/tabs/{id}/frame` — PNG du framebuffer (pull MVP).
- Trajectoire d'upgrade : canal direct de pixels bruts décompressés
  (mémoire partagée ou Named Pipe sous Windows) dès la phase 6/7 pour
  éliminer la surcharge CPU d'encodage/décodage PNG.
- `POST /api/tabs/{id}/viewport` — taille du viewport (resize UI)
  et facteur d'échelle DPI (HiDPI).
- `POST /api/tabs/{id}/input` — souris/clavier/scroll (hit-test
  layout) ; événements mouvement **coalescés** côté client pour
  limiter les round-trips loopback (ADR-0003).
- `GET  /api/events` — SSE : `tab_state`, `frame_invalidated` (+ dirty rects),
  `title_changed`, `events_lost`.
- Erreurs : `{"error": {"handled": true, "message": "…"}}`.
- Bind `127.0.0.1` ; clé d'API optionnelle (`x-api-key` ou `?key=`).

## Décisions structurantes anticipées

- Modèle mémoire DOM ↔ GC figé avant la phase 2 (ADR-0008) —
  rooting explicite, cycles par table de callbacks ; c'est le
  **contrat imposé à `dillo-js`** (rooting/trace/finalization) —
  périmètre équipe NAVIGATEUR dans le workspace.
- Stack texte/fontes arbitrée avant l'étape 18 (ADR-0009) —
  `fontdb`/`ttf-parser` + `rustybuzz` + `unicode-bidi` +
  `unicode-linebreak`, runs de texte itemisés ; `fontdb` fait de
  l'I/O → port `FontProvider` dans layout, impl `fontdb` en
  infra injectée par `core`.
- Pipeline incrémental et sous-ressources (ADR-0012) : chargement
  parallèle `<link>`/`<img>`/`<script>`, entrée par chunks,
  invalidation style/layout sur mutation DOM, CSS tardive,
  pseudo-classes — jamais de rasterisation d'un arbre en mutation.
- Compatibilité mesurée sur corpus publics (ADR-0010) —
  html5lib-tests, WPT ciblés, test262, Acid2/Acid3, % tracké par
  phase.
- Modèle de menace : budgets + cloisonnement de session actifs,
  isolation processus différée, réévaluation obligatoire en
  phase 8 (ADR-0011).

## Coutures gardées ouvertes

- `HttpTransport` injectable → un futur transport mandataire
  (proxy d'entreprise, Tor…) est une implémentation, pas une
  réécriture.
- `http_proxy`/`no_proxy` honorés par l'environnement.
- `dillo-js` vit dans ce workspace sous `MIT OR Apache-2.0` — le
  trait `JsEngine` le rend remplaçable (`boa_engine` en
  échappatoire documentée, ADR-0007).
- Répertoire de profil déplaçable (cookies, historique,
  bookmarks) — contextes cloisonnés par session (ADR-0011).

