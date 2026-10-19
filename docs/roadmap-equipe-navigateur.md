# Roadmap — Équipe NAVIGATEUR

**Périmètre :** la plateforme produit — `explorer-net`,
`explorer-core` (session/pipeline), `explorer-api`,
`explorer-daemon`, `explorer-cli`, `explorer-ffi`,
**toute la surface JS** (`explorer-js` + `dillo-js`),
`app/` Flutter, `scripts/`, `fuzz/`.
**Ne touche pas :** les crates domaine pur (`explorer-common`,
`explorer-dom`, `explorer-html`, `explorer-css`,
`explorer-layout`, `explorer-render`) — propriété PIPELINE,
consommées via leurs API publiques.
**Partagé :** `explorer-test-support` (fixtures/stubs additifs
libres, structure via le journal — AGENTS.md §10).

**Deux pistes de numérotation** dans ce fichier :
- **Piste produit** : étapes **50+** (suite de la séquence
  historique phases 0–10).
- **Piste JS** : **JS-C26+** (suite de la série `dillo-js`).

**Definition of done** (inchangée) : implémenté + testé +
CHANGELOG + case cochée + `verify_all` vert + commit dédié.
**Spécialiste JS** : la piste JS est tenue par la personne qui
connaît `dillo-js` — rôle nommé au sein de l'équipe, pas une
équipe séparée. `JsEngine` est un contrat **interne** à
l'équipe (plus de sign-off croisé, extensions additives
recommandées mais non bloquées par l'autre équipe).

Légende : `[ ]` à faire · `[~]` en cours · `[x]` terminée.
Les besoins dans le domaine pur passent par
`docs/interface-equipes.md` — marqués **↔ PIPE**.

---

## Piste produit (étapes 50+)

### Palier N-A — Le navigateur charge vraiment les pages

Objectif : les blocages du web réel sautent. Sortie de palier :
`site_smoke.ps1` étendu à 10 sites, **8/10 fonctionnels**
(navigation + liens + formulaires + scripts de base actifs).

- [x] **Étape 50. Scripts externes + scheduler de
  sous-ressources** (ADR-0012 complété côté session).
  `<script src>` fetché via `HttpTransport` : ordre document,
  `defer` (post-parse, ordre garanti), `async` (à réception),
  `type=module` → `ModuleLoader` de `dillo-js` (existe, JS-C5).
  Scheduler **parallèle** : files bornées par onglet
  (`SessionConfig`), priorités `document > CSS > script > img`,
  N fetches en vol (`FuturesUnordered`), `Referer`/`Origin` et
  mixed-content déjà calculés à l'émission. Insertion dynamique
  de scripts : hooks à définir avec JS-C35.
  *Pourquoi en premier : trou le plus bloquant du web réel.*

- [x] **Étape 51. Câblage Web APIs — clôt l'étape 44 de
  ROADMAP.md.** Intégralement interne à l'équipe :
  - `JsEngine` : traverse proxy des surfaces déjà livrées dans
    `dillo-js` — `XhrTransport`/`deliver_xhr_response`,
    `install_storage`, `CookieJar` (pattern `drain_nav_requests`
    généralisé : `drain_net_requests()` + `deliver_net_response()`).
    **Livré par JS-C27** (commit `2b73089`).
  - Session : exécute le fetch en tâche tokio, livre à un point
    de synchro — jamais de callback ré-entrant.
    `drain_script_net` → spawn tokio → canal → `pump_net`
    (daemon : battement 50 ms ; CLI : grâce 1 s ; interne :
    `poll_net` post-événements).
  - `localStorage` : map par origine persistée dans le profil ;
    `sessionStorage` par onglet (mémoire). `ProfileStore`
    étendu (`load_storage`/`save_storage`, implémenté par
    `JsonFileStore` — `storage-<slug>.json` par origine).
  - `document.cookie` : bridge vers le jar `explorer-net`
    (politiques Secure/SameSite héritées) — `CookieTransport`
    daemon partagé avec la session, `HttpOnly` filtré des
    écritures script.
  - **Réévaluation ADR-0019 faite** : same-origin vs CORS
    minimal (requêtes simples + ACAO `*`|origine exacte,
    preflight hors MVP), `Origin` sur CORS, quota
    `max_script_net` = 16/onglet, `Set-Cookie` capturé
    same-origin seul — déviations documentées dans
    l'amendement de l'ADR. Tests : 8 scénarios (succès,
    opaque, ACAO, preflight-bloqué, quota, jar, Set-Cookie,
    storage).

- [ ] **Étape 52. Boucle d'événements v2 + tick timers.**
  `dispatch_event(ty, path)` existe — étendre les types :
  `mouseover`/`mouseout`/`mousemove` (coalesced),
  `keydown`/`keyup`/`input`, `focus`/`blur` (liés au focus champ
  existant), `wheel`/`scroll`. **Tick dédié** :
  `next_timer_deadline` de `dillo-js` devient un délai armé par
  le daemon (et `tick` FFI) — `setTimeout`/`setInterval`/`rAF`
  tirent sans attendre un input ; borne de jobs par tick
  (ADR-0019). `MutationObserver` = JS-C26, drainé aux mêmes
  sync points.

- [ ] **Étape 53. Session incrémentale** (moitié orchestration de
  l'ex-53) : la session alimente `push(&str)` du tokenizer par
  chunks pendant le fetch, relayout cadencé (N ms ou N octets),
  `frame_invalidated` intermédiaire → peinture pendant le
  téléchargement. ↔ PIPE **R-1** fournit la tolérance « document
  en construction » côté domaine — tenu en tête de leur file
  pour débloquer cette étape.

- [ ] **Étape 54. Formulaires v2 — côté produit** (ex-55, moitié
  session) : `multipart/form-data`, états `:valid`/`:invalid`
  exposés aux events, popup `<select>` via widget natif Flutter
  (arbitrage : rendu moteur ↔ PIPE ou widget app — décider en
  début d'étape).

### Palier N-B — Produit, profil, release

- [ ] **Étape 55. Dirty-rects transportés** (ex-63b) : régions
  produites par PIPE **R-7** → `frame_copy` FFI partiel + SSE
  `frame_invalidated` enrichi ; arbitrage ExternalTexture GPU si
  benchs (révision ADR-0004/0018).
- [ ] **Étape 56. Profil persisté complet** (ex-64) : cookies,
  localStorage (cf. 51), cache disque borné, historique —
  namespaces de session cloisonnés (ADR-0011), répertoire
  déplaçable.
- [ ] **Étape 57. UI Flutter v2** (ex-65) : thème clair
  (`tokens.json` variante), settings (moteur REST/FFI, thème,
  profil), UI historique + bookmarks (store core existant),
  arbitrage find-in-page / zoom (petit ADR si reclassés).
- [ ] **Étape 58. `<video>`/`<audio>`** (ex-66) : arbitrage
  décodage pur Rust requis (ADR) — sinon stubs propres + affiche.
- [ ] **Étape 59. Release 0.2** (ex-67) : packaging, smoke E2E
  étendu, doc. **Prérequis d'ouverture** : renommage
  `crates/dillo-js` → `explorer-jscore` (ADR-0020) — commit
  `chore:` dédié, à un moment où aucune piste ne touche le crate
  ; le nom de crate est surface publique de release.

## Piste JS (JS-C26+) — spécialiste `dillo-js`

Ordre dicté par les besoins de la piste produit d'abord, puis
couverture **strictement réactive** (un bug site → un cas
`corpus.rs`/test262 → un fix — fini la couverture pour la
couverture).

### J-A — APIs navigateur attendues par la session

- [x] **JS-C26. `MutationObserver`** : queue de mutations sur le
  `Dom` miroir (`childList`/`attributes`/`characterData`,
  `subtree`, `attributeFilter`, `oldValue`), delivery en
  microtask checkpoint, `disconnect`/`takeRecords`. Débloque
  l'étape 52.
- [x] **JS-C27. Traverse proxy net/storage** : `XhrTransport`,
  `install_storage`, `CookieJar` traversent le canal
  `DilloEngine` (`drain_net_requests`/`deliver_net_response`/
  `set_storage`/`set_cookie_jar`) — prérequis interne de
  l'étape 51.
- [ ] **JS-C28. `<canvas>` 2D surface** : `getContext('2d')` +
  `CanvasRenderingContext2D` complet niveau API (chemins,
  fills/strokes, transforms, `drawImage`, `fillText`, gradients,
  `getImageData`/`putImageData`) enregistrant une **op-list**
  exposée via `drain_canvas_ops` — raster = PIPE **R-11**.
- [ ] **JS-C29. `Worker` bornés** : Engine par thread (patron
  `DilloEngine`), `postMessage` structuredClone (livré),
  `onmessage`, `importScripts` minimal, `terminate`, pool
  borné (`max_workers`, budget ops/worker). Documenté : pas de
  `SharedArrayBuffer` inter-thread réel.

### J-B — Couverture langue par besoin (réactive)

**Règle du palier — aucune de ces étapes ne s'ouvre sans un
déclencheur documenté** : échec `site_smoke`, bug d'un site
réel, ou besoin d'une étape produit. Le % de couverture n'est
jamais un déclencheur (état des lieux §2 — limites des
mesures).

- [ ] **JS-C30. Extension corpus test262** : extraire les
  répertoires non couverts (`builtins/` complet,
  `language/statements|expressions`) → baseline mesurée, skips
  whitelistés, remontée par échecs groupés.
  *Déclencheur : aussi l'élargissement honnête du chiffre
  test262 publié — à faire quand le reporting l'exige.*
- [ ] **JS-C31. `Temporal` complet** : `ZonedDateTime`
  (arbitrage fuseaux), `round`/`smallestUnit`, calendriers.
  *Déclencheur : site réel.*
- [ ] **JS-C32. `Intl` multi-locale** : arbitrage CLDR
  (`icu4x` pur Rust — ADR dédié, dépendance lourde).
  *Déclencheur : site réel + ADR de dépendance.*
- [ ] **JS-C33. Décorateurs ES2026** — *déclencheur : site
  réel.*
- [ ] **JS-C34. Générateurs lazy + `await` pending** : refacto
  structurel, **uniquement si des sites réels bloquent**
  (l'eager documenté tient le corpus actuel).
- [ ] **JS-C35. `document.write` complet + insertion dynamique**
  — ordre cohérent avec le scheduler de l'étape 50.
  *Déclencheur : insertion dynamique constatée sur sites
  réels.*

### J-C — Mesure, perf, robustesse

- [ ] **JS-C36. Acid3 subset JS mesuré** (harnais informatif).
- [ ] **JS-C37. Perf + borne mémoire heap** : compteurs d'ops,
  seuils GC, **borne mémoire par contexte** (exigence ADR-0019),
  coût `serialize_document` instrumenté.
- [ ] **JS-C38. Fuzz lexer/parser `dillo-js`** : cible
  cargo-fuzz (extension de `fuzz/` — périmètre équipe).
- [ ] **JS-C39. Sync DOM incrémentale — instrumenter puis
  drainer les records** (dette ADR-0013 budgétisée tôt) :
  d'abord **mesurer** le coût réel de la resérialisation
  `sync_dom` — compteurs ms/octets par synchro dans la
  session, visibles au profilage (étape 57) ; si le coût le
  justifie, `JsEngine` gagne un `drain_dom_mutations()`
  **additif** qui réutilise les `MutationRecord` du miroir
  (mécanique JS-C26) comme diff d'arbre au lieu du HTML
  complet. Nouveau point de cohérence → tests croisés
  miroir ↔ `explorer-dom` exigés (règle de tri « corrige
  le côté fautif »).

## Tâches continues

- **Spécialiste JS** : divergence miroir ↔ `explorer-dom` quand
  le parse miroir est fautif → fix `dillo-js` + cas `corpus.rs`.
- **Conformance** : `explorer-test-support`, corpus extraction,
  outillage smoke — la culture « % mesurés » (ADR-0010).
- **Triage produit** : échecs `site_smoke` classés « navigateur »
  (session, net, JS, UI) → tests → étapes.

## Interfaces vers l'équipe PIPELINE (récapitulatif)

| Besoin | Fourni par | État |
|---|---|---|
| Parse incrémental `push()` | `explorer-html` | ✅ prêt — à exploiter (étape 53 ↔ **R-1**, tête de leur file) |
| Styles/fontes/layout/render | API `explorer-css`/`layout`/`render` | ✅ stables |
| Dirty rects | display list `explorer-render` | **R-7** à faire → étape 55 |
| Raster canvas op-list | `explorer-render` | **R-11** à faire ↔ JS-C28 |
| `<svg>` inline, `@keyframes`, flex/grid | domaine | R-6→R-8, R-3→R-4 |

Règle : les demandes d'API au domaine pur transitent par
`docs/interface-equipes.md` — jamais de commit direct dans
un crate PIPELINE.
