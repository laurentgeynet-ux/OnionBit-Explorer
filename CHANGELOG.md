# Changelog

Toutes les modifications notables de ce projet sont consignées ici.

Le format suit [Keep a Changelog](https://keepachangelog.com/fr/1.1.0/)
et le projet adhère au [Semantic Versioning](https://semver.org/lang/fr/).

## [Unreleased]

### Added

- **Étape 18 — `explorer-layout` : stack texte et fontes
  (ADR-0009)** (`text.rs` + `font.rs`) : port `FontSource`
  (données de fontes — `fontdb` reste en infra, injecté par
  `explorer-core`), `FontRequest`/`FontMetrics`/`Glyph`/
  `ShapedRun`. Métriques réelles via `ttf-parser`, shaping
  via `rustybuzz` (`shape`/`measure`, direction RTL).
  `itemize` : segmentation bidi UAX #9 par niveau
  (`unicode-bidi`) + opportunités de coupure UAX #14
  (`unicode-linebreak`) recalées au run. `break_lines` :
  greedy avec blancs de fin hors mesure, débordement de mot
  géant sans coupure forcée (`overflow-wrap: normal`),
  progression stricte garantie. **Fonte fixture embarquée**
  (`tests/common`) : TTF minimal construit en code
  (`unitsPerEm` 1000, ascender 800/descender -200, cmap
  format 4) — métriques réelles en headless, aucune mesure
  fixe approximative. 11 tests.

- **Étape 17 — `explorer-layout` : arbre de boîtes**
  (`boxtree.rs`) : `LayoutBox`/`BoxKind`
  (block/inline/list-item/texte/anonyme) + lien
  `node: Option<NodeId>` ↔ boîte, `Rect` (géométrie remplie
  par les étapes 19+). `build_box_tree` : `display:none`
  saute le sous-arbre, texte porte le style du parent,
  enveloppes anonymes CSS 2.1 §9.2.1.1 avec troncature des
  blancs purs en bord de run (l'espace entre inlines
  survit). `dump` wireframe + commande `explorer-cli layout`
  (pipeline fetch → parse → style → boîtes, `<style>`
  embarqués collectés — feuilles liées hors périmètre).
  8 tests.

- **JS-B — harness test262 (ADR-0010)** : extrait versionné de
  la suite ECMAScript officielle @ `d53ebc8` (1077 fichiers,
  sous-ensembles `test/language/{types,literals,comments,
  white-space,line-terminators,identifiers,hashbang}` +
  `harness/`), frontmatter `flags`/`features`/`includes`/
  `negative` exécuté, skips whitelistés comptés, cliquet
  `BASELINE_PASSED = 531` — **score initial 531/859 (62 %)**,
  218 skips (BigInt, champs privés, séparateurs numériques,
  hashbang, modules…), 328 échecs documentés.
- **fix(dillo-js)** : wrappers `new String/Number/Boolean` sans
  `[[PrimitiveValue]]` — `ToPrimitive`→`toString`→`ToString`
  bouclait à l'infini (trouvé par test262 `S8.7_A4`). Ajout de
  la prop cachée `__prim`, `valueOf`/`toString` stricts
  (`TypeError` sur receiver incompatible, spec) et `__prim` sur
  les prototypes eux-mêmes.
- **JS-A — GC mark-sweep `dillo-js` (ADR-0008)** : l'arène
  `ObjId` ne « meurt plus avec la page » —
  - `Heap` : tombstones + freelist (réutilisation de slots),
    rooting explicite ref-counté (`add_root`/`remove_root`),
    `is_alive`/`is_rooted`/`live_count` ;
  - `Engine::collect` : mark itératif depuis toutes les racines
    (global, prototypes, env global, `Symbol.for`, timers,
    microtasks, XHR/fetch en vol, cache DOM↔JS + listeners,
    hooks, complétion) puis sweep + `Host::finalize` ;
  - trace : proto, props data/accesseurs, éléments de tableau,
    `Map`/`Set`/itérateurs, promesses + réactions + jobs,
    envs capturés par `Fun::Script` (visited-set anti-cycle),
    hooks `Host::trace`/`finalize` pour les objets hôtes ;
  - contrat : `collect` entre deux jobs — la pile d'appel JS
    n'est pas une racine ; les handles hôtes stables passent
    par les racines explicites, un `ObjId` obsolète résout
    `None` (jamais d'accès dangling) ;
  - 13 tests `tests/gc.rs` : cycles, fermetures, timers,
    promesses, cache DOM, racines explicites, idempotence.

- **Étape 16 — `explorer-css` : propriétés typées +
  shorthands + pseudo-classes dynamiques** (`value.rs`,
  ~560 lignes) : `Length`/`Unit` (px + absolus avec facteur,
  relatives `em`/`rem`/`ex`/`ch` + viewport `vw`/`vh`/`vmin`/
  `vmax` différées à la used value), `Color` RGBA (`#rgb`,
  `#rrggbb[a/a]`, `rgb()/rgba()`, ~50 noms + `transparent`),
  exposants `2e3` gérés à la découpe nombre/unité. Expansion
  des shorthands à la cascade : `margin`/`padding` (1-4
  valeurs), `border`/`border-<side>` (ordre libre
  width/style/color), `border-width|style|color`, `overflow`,
  `background` (couleur + image), `font` (style/variant/
  weight/size[/line-height]/family), `list-style`. Accesseurs
  typés sur `ComputedStyle` : `length`/`color`/`number`
  (`currentcolor` résolu via `color`). Pseudo-classes
  dynamiques (`:hover`, `:focus*`, `:checked`, `:disabled`,
  `:target`…) : `matches_with_state`/`compute_with_state`/
  `compute_tree_with_state` pilotées par une fonction d'état
  externe — couture restyle ADR-0012 ; `:link` structurel sur
  `a[href]`/`area[href]`, `:visited` toujours faux (pas de
  tracking, conformité vie privée). 10 tests + corrections
  d'attentes (`:hover` désormais valide, alpha rgba arrondi).
  *Mesure WPT selectors/cascade : reste à brancher dans le
  chantier conformance (ADR-0010).*

- **Étape 15 — `explorer-css` : cascade + héritage +
  computed style** (`cascade.rs`, ~500 lignes) : ordre de la
  cascade **UA normal < auteur normal < auteur `!important` <
  UA `!important`**, à rang égal spécificité puis ordre source ;
  `StyleEngine` accumule les feuilles (`add_css`/`add_stylesheet`,
  origine typée) avec sélecteurs pré-parsés. Mots-clés CSS-wide
  `inherit`/`initial`/`unset`/`revert` ; table des propriétés
  héritées (custom `--*` incluses) + table `initial_value`
  couvrant les propriétés de l'étape 16. Media queries
  évaluées sur `StyleEnv` (type, `min/max-width`, `-height`,
  `orientation`, négation, imbrication AND) — une règle sous
  `@media` non vérifié est sautée. **Feuille UA intégrée**
  (`UA_CSS`, défauts HTML Rendering) écrite en CSS et parsée
  par notre propre pipeline (dogfooding) ; `compute` (nœud
  seul) et `compute_tree` (mémoïsation parent→enfant, O(n)).
  16 tests : origines, `!important`, héritage explicite,
  `unset`, media queries, handles obsolètes.

- **Étape 14 — `explorer-css` : sélecteurs + matching DOM**
  (`selector.rs`, ~700 lignes) : parsing depuis le flux de
  tokens — type, `*`, `.classe`, `#id`, `[a]`/`[a=v]`/`~=`/
  `|=`/`^=`/`$=`/`*=` + drapeau `i`, combinateurs descendant,
  `>`, `+`, `~`, listes `,` ; pseudo-classes structurelles
  `:root`, `:empty`, `:first/last/only-child`, `:nth-child()`,
  `:nth-last-child()`, les quatre `*-of-type`, `:not()`, `:is()`,
  `:where()` — dynamiques (`:hover`…) et pseudo-éléments
  réservés à l'étape 16 (présence → sélecteur invalide, règle
  ignorée). `an+b` complet (`odd`/`even`/`-n+3`). Matching
  droite→gauche sur `explorer-dom` via ses accesseurs
  `Result` (handle obsolète → non-match, jamais de panic) ;
  combinateurs frères ignorant les nœuds texte. Spécificité
  `(id, classe, type)` exposée pour la cascade. 17 tests :
  parsing, ops attributs, nth, combinateurs, matching réel,
  handles obsolètes.

- **Étape 13 — `explorer-css` : tokenizer + parser**
  (`tokenizer.rs`/`parser.rs`, ~1 600 lignes) : tokens CSS
  Syntax (ident, function, nombre/dimension/pourcentage,
  chaîne, url, hash, CDO/CDC, delim) avec échappements résolus
  et `BadString`/`BadUrl` en dégradé ; buffer d'avance
  `VecDeque` (look-ahead 2-3 chars) ; borne `MAX_TOKENS`
  anti-contenu-hostile. Parser tolérant : feuille de style →
  règles qualifiées + déclarations ordonnées, `!important`
  détecté/retiré, récupération spec (saut à `;`/`}` avec
  profondeur, blocs non fermés à EOF avalés, garde
  anti-progression) ; at-rules `@charset`, `@import`,
  `@font-face`, `@keyframes` (règles imbriquées), at-rules
  inconnues avalées sainement ; `@media` minimal : type,
  conditions `(min-width: 600px)`, négation `not`, listes `,`.
  32 tests (14 tokenizer + 18 parser) couvrant nombres
  signés/exposants, `url()` brut vs `url("…")`, `calc(`,
  déclarations invalides sautées sans perte des suivantes.

- **Étape 12 — golden tests + premier artefact** :
  - Corpus **html5lib-tests tree-construction** (57 fichiers
    `.dat`, ~1 600 cas) extrait @ `9329e64694` vers
    `tests/data/tree-construction/` — harness
    `tests/tree_construction.rs` avec cliquet
    `BASELINE_PASSED = 953` (**59 %**, 200 cas fragment/
    script-on ignorés) ; échecs concentrés sur l'adoption
    agency complète, conformément aux écarts documentés.
  - `explorer-cli` : `dump <fichier|url>` (dump DOM format
    html5lib, sniffing d'encodage, pile `explorer-net`
    TLS+redirections+décodage) et `fetch <url>` — jalon J2.
  - `explorer-html::debug::dump` : sérialiseur partagé.
  - Spike **ADR-0008** (`explorer-dom/tests/rooting_spike.rs`) :
    faux heap JS validant rooting explicite, réclamation
    `collect`, `StaleHandle` sur slot réalloué, détachement
    sans destruction.
  - `fuzz/` : cible cargo-fuzz du tokenizer + tree-builder
    (chunks de 7 octets, hors workspace).

- **Étape 11 — `explorer-html` : tree-builder HTML5**
  (`tree.rs`, ~2 100 lignes) : pipeline token → `explorer-dom::Dom`
  avec les modes d'insertion de la spec — Initial, BeforeHtml,
  BeforeHead, InHead, AfterHead, InBody, Text, InTable,
  InCaption, InColumnGroup, InTableBody, InRow, InCell, InSelect,
  AfterBody, AfterAfterBody. `html`/`head`/`body` synthétisés,
  attributs `<html>`/`<body>` fusionnés aux nœuds existants.
  Implicites : `<p>` fermé par les blocs (`P_CLOSERS` — `table`,
  `form`, `hr`, `pre` traités par leurs règles dédiées), `li`,
  `dt`/`dd`, `h1`–`h6`, `option`/`optgroup`, `rt`/`rp` (scope
  `ruby`). Scopes : button, list-item, table, formatting (marqueurs
  `<applet>`/`marquee`/`object`/`template`). **AFE** : règle de Noé
  (3 entrées identiques max), reconstruction après fermeture de
  bloc, adoption agency basique (`<b><i>x</b>y</i>` → `y` ré-enrobé
  dans un `<i>` frère). Tables : `<tbody>`/`<tr>` synthétisés,
  cellules `td`/`th` fermées en chaîne, **foster parenting** du
  texte et des éléments mal placés (insérés avant `<table>` dans
  son parent, repli sous-table). Contenu spécial : `script`/
  `style`/`noframes`/`xmp`/`iframe` (rawtext/ScriptData),
  `title`/`textarea` (RCDATA), `plaintext`, `\n` initial sauté
  après `pre`/`textarea`/`listing`, CDATA autorisé en `<svg>`/
  `<math>` via `set_cdata_allowed`. DOCTYPE → `quirks`/
  `limited-quirks` : liste de préfixes WHATWG complète, cas
  « missing **or empty** » vs « neither missing nor empty »
  distingués, DTD IBM XHTML reconnu, `strict 4.01` sans system
  reste `NoQuirks`. Tolérance : contenu post-`</body>`/`</html>`
  retraité en InBody, `</p>` sans `<p>` → `<p>` vide, `</br>` →
  `<br>`. **API** : `TreeBuilder::process` token par token,
  `drive` intercalant `next_token`/`process` (les états de contenu
  sont réglés *avant* la lecture de la suite — `Tokenizer::run`
  ne doit pas précéder `process`), `parse` raccourci, accès
  `dom`/`into_dom`/`quirks_mode`. 26 tests : synthèse, fermetures
  implicites, foster parenting, rawtext/RCDATA/script, quirks,
  tag-soup sans panic, découpage incrémental équivalent au
  one-shot.
- **Étape 10 — `explorer-html` : encodage, entités et tokenizer
  incrémental** (`encoding.rs`, `entities.rs`, `token.rs`,
  `tokenizer.rs`) : sniffing WHATWG — BOM UTF-8/UTF-16LE/BE,
  `<meta charset>` prescanné dans les 1024 premiers octets (case
  et quoting tolérants, hors-balise rejeté), défaut
  `windows-1252` — puis transcodage tolérant (`U+FFFD`, BOM
  consommé). Tokenizer machine-à-états incrémental : chunks
  bornés via `feed`/`next_token`, reprise de coupure au milieu
  d'une balise, d'une entité ou d'un mot-clé (`doctype`,
  `public`, `system`, `[CDATA[` — matching incrémental), texte
  `Character` coalescé avec ordre du flux préservé (token retenu
  pendant la purge), états rawtext/RCDATA/script/PLAINTEXT
  pilotés par le tree-builder (`ContentKind`, « appropriate end
  tag » via `last_start`), CDATA en contenu étranger, `Eof` après
  résolution des constructions suspendues (orphelins `<`/`</`/
  `<!`/`]]` réinjectés en texte). Entités : table nommée
  longest-prefix (`;` d'abord, legacy sans `;` — règle de
  l'esperluette ambiguë en attribut), numériques décimales/hexa
  avec remap `0x80..0x9F` windows-1252 et `U+FFFD` sur
  surrogates/dépassements. Bornes contenu hostile : noms
  1 Kio, valeurs/commentaires 64 Kio, purge texte 16 Kio
  (ADR-0011). 34 tests dont coupures de chunk exhaustives.
- **Étape 8 — `explorer-net` : redirections & politique**
  (`policy.rs`, `proxy.rs`) : `RedirectTransport` — suivi 3xx borné
  (`TooManyRedirects`), `Location` résolu contre la base, 303 → GET
  (sauf HEAD), 301/302 POST → GET, 307/308 préservés intégralement.
  `HstsStore` mémoire borné — `max-age`/`includeSubDomains`,
  `max-age=0` révoque, upgrade `http→https` appliqué par
  `HstsTransport` sur le fil, header capturé sur réponses https.
  `ProxyConfig` env (`http_proxy`/`https_proxy`/`no_proxy`,
  suffixes `.dom`/`*.dom`) + `ProxyConnector` — absolute-form pour
  HTTP, tunnel `CONNECT` pour HTTPS (TLS posé sur le tunnel via le
  `inner` injectable de `TlsConnector`). `RefererTransport` —
  URL complète same-origin, origine seule cross-origin, droppé en
  `https→http` ; `Origin` posé sur les requêtes mutantes
  cross-origin. `PolicyTransport` — mixed-content `http` sous page
  `https` bloqué (`BlockedMixedContent`), localhost/`data:`/`about:`
  exempts. `Connector` gagne `absolute_form()`. 18 tests.
- **Étape 7 — `explorer-net` : cookies + cache** (`cookies.rs`,
  `cache.rs`) : `CookieJar` RFC 6265 durci — `Domain=` validé par
  **Public Suffix List** (`psl`, supercookies `.co.uk` rejetés),
  `Secure` https-only (localhost toléré), `HttpOnly` filtré de la
  vue `document.cookie`, `SameSite`, préfixes `__Host-`/`__Secure-`
  requis, `Max-Age=0` = suppression, chemins RFC 6265 §5.1.4,
  budgets `CookieConfig` (total/par-domaine, éviction). `MemoryCache`
  — GET only, `no-store`/`Vary: *` exclus, statuts cachables RFC
  9111, fraîcheur `max-age`/`Expires`, revalidation `If-None-Match`/
  `If-Modified-Since` sur 304 (corps caché servi, en-têtes
  fusionnés), éviction FIFO bornée. Décorateurs `CookieTransport`
  (Cookie: injecté + Set-Cookie capturé) et `CachingTransport`
  composables. 17 tests dont wire end-to-end.
- **Étape 6 — `explorer-net` : décodage des corps** (`decode.rs`)
  : `DecodingTransport` décorateur — `Accept-Encoding: gzip,
  deflate, br` injecté, `Content-Encoding` multi-codages décodés
  en ordre inverse, zlib **et** raw deflate (le web réel mélange),
  `br` via `brotli`, budget `max_decoded_bytes` anti-bombe zip →
  `NetError::DecodedTooLarge`, flux corrompus/codages inconnus →
  `NetError::ContentDecode`, `Content-Length` ré-écrit à la taille
  décodée. `Response` gagne `content_type()`/`charset()`/
  `nosniff()` (`X-Content-Type-Options` exposé) et `text()` —
  transcodage WHATWG par `encoding_rs` (défaut `windows-1252`).
  Deps : `flate2`/`brotli`/`encoding_rs` (pur Rust). 13 tests.
- **Étape 5 — `explorer-net` : TLS** (`tls.rs`) : `TlsConnector`
  implémente la couture `Connector` — TCP clair pour `http:`,
  handshake `rustls` pour `https:` (SNI/ALPN par `tokio-rustls`),
  racines Mozilla `webpki-roots` ou store injecté
  (`with_roots` — tests/enterprise), erreurs certificat/SNI/
  handshake en `NetError::Tls` typées. Provider crypto **`ring`** :
  exception `*-sys*` consignée ADR-0002 (pas de pile TLS viable
  100 % Rust ; `ring` build Windows sans cmake, `aws-lc-rs` exclu).
  3 tests loopback avec cert `rcgen` : handshake OK, cert non
  approuvé → `Tls`, mauvais hostname → `Tls`.
- **Étape 4 — `explorer-net` : client HTTP/1.1** (`http.rs`,
  `h1.rs`, `error.rs`) : `Request`/`Response`/`Method`, trait
  `HttpTransport` injectable (ADR-0002), `H1Client` sur `Connector`
  pluggable (`TcpConnector` clair, TLS à l'étape 5 par la même
  couture `IoStream`), en-têtes via `httparse`, corps
  `Content-Length`/`chunked` (trailers)/EOF HTTP/1.0, pool
  keep-alive par origine borné + retry unique sur méthode
  idempotente, budgets `HttpConfig` (timeouts connect/io, tailles
  tête/corps, `User-Agent`, cap pool), erreurs `NetError` typées
  (DNS/connect/timeout/InvalidResponse/TruncatedBody/InvalidChunked/
  BodyTooLarge/HeadTooLarge/UnsupportedScheme). 14 tests
  d'intégration sur `TcpListener` loopback (aucun hôte externe).
  Dépendances consignées ADR-0002 : `tokio`, `httparse`,
  `async-trait`.
- **Étape 9 — `explorer-dom` : arène DOM** : `NodeId`
  générationnels (index+génération, handles périmés →
  `DomError::StaleHandle`), nœuds `Document`/`Doctype`/`Element`/
  `Text`/`Comment`, attributs ordonnés, navigation complète,
  `append_child`/`insert_before`/`remove` avec gardes de
  hiérarchie (cycles, parent sans capacité), `remove` détache sans
  détruire, table de **rooting** explicite (`add_root`/`remove_root`,
  cycles DOM↔JS par table — ADR-0008), `collect` mark-sweep réclamant
  les sous-arbres non rootés, `normalize()` fusionnant les `Text`
  adjacents (les nœuds rootés bloquent la fusion), newlines
  `\r\n`→`\n`, `text_content`, budget `max_nodes` borné (ADR-0011).
  13 tests verts.
- **Étape 3 — `explorer-net` : URL WHATWG** (`url.rs`) : parseur
  hiérarchique (`http`/`https`/`file`) + opaque (`data:`/`about:`),
  hôtes IDNA UTS-46 via la crate `idna` (arbitrage consigné
  ADR-0002), IPv4 aux formes WHATWG (octal/hex/réduites `127.1`),
  IPv6 `[…]`, userinfo, ports par défaut élidés, segments `.`/`..`
  normalisés, percent-encoding des jeux path/query/fragment/
  userinfo, résolution relative RFC 3986 §5.2, `Origin` tuple/
  opaque pour la politique same-origin, `path_query` (origin-form).
  27 tests de conformité sous-ensemble WHATWG.
- **Étape 2 — squelette Flutter `app/`** : `flutter create` Windows,
  clean architecture `core/` (`api/` client REST + SSE `dart:io`
  zéro-dépendance avec reconnexion, `design/` tokens + thème dérivé,
  `config/` `AppConfig`, `di/` providers Riverpod) + `features/
  browser/` (entité `BrowserTab`, `BrowserRepository` sur le contrat
  REST ADR-0003, `BrowserPage` coquille omnibox). `assets/design/
  tokens.json` chargé au démarrage avec repli embarqué (ADR-0005) ;
  `ExplorerApp` attend les tokens puis dérive `ThemeData`.
- **Étape 1 — workspace cargo** : 13 crates `explorer-*` squelettes
  (lib/binaire, `#![forbid(unsafe_code)]`, lint clippy pedantic,
  rustdoc), `[workspace]` Cargo.toml, `scripts/verify_all.ps1`
  (fmt/clippy/test/doc + flutter conditionnel).
- **`dillo-js` dans le workspace** : le moteur JavaScript est devenu
  `crates/dillo-js` — **version canonique et unique**, licence
  `MIT OR Apache-2.0`, chantier de l'équipe JS dédiée (ADR-0007
  révisé). Le crate a été **retiré du workspace du port voisin** —
  son `dillo-browser` consomme désormais le crate d'ici via la dep
  `path` `../OnionBit-Explorer/crates/dillo-js` (jonction locale
  `D:\Projet\OnionBit-Explorer` → `C:\OnionBit-Explorer`), en
  attendant la bascule `git`+`rev` à la publication d'Explorer.
- **ADR-0012** — pipeline incrémental : scheduler de
  sous-ressources dans `core`, entrée tokenizer par chunks, modèle
  d'invalidation style/layout/paint en trois niveaux.
- **Jalons J0–J5** dans `ROADMAP.md` : repères de motivation, dont
  J2 (`explorer-cli dump` — premier artefact visible dès la fin de
  phase 2, avant tout pixel).
- **Chantier `dillo-js`** (piste parallèle équipe JS, ROADMAP
  phase 8) : JS-A alignement heap → ADR-0008 + GC mark-sweep
  (prérequis de l'étape 34), JS-B harness test262, JS-C couverture
  (modules ES, `Proxy`, `Intl`), JS-D médias.

### Changed

- **Rebranding** : le projet s'appelle désormais « Explorer » (nom
  de travail — le répertoire physique sera renommé à la fermeture
  de l'IDE). Tous les documents ont été débarrassés des références
  à l'ancien écosystème ; les coutures (proxy, profil déplaçable,
  transport injectable) restent ouvertes sous justification
  propre.
- **Roadmap durcie** : politique cookies complète (`SameSite`,
  `__Host-`, Public Suffix List) en étape 7 ; `Referer`/`Origin`/
  mixed-content en étape 8 ; entrée tokenizer incrémentale (étape
  10) ; `position`/`z-index`/`overflow`/stacking/`list-item`/
  `white-space` + pseudo-classes dynamiques (étapes 16/21) ;
  `FontProvider` comme port injecté (étape 18, ADR-0009) ; spike
  ADR-0008 à faux handles + alignement heap `dillo-js` comme
  prérequis de l'étape 34 ; `<svg>` statique en étape 45 ;
  dégradation flexbox du palier 1 assumée et mesurée ; exclusions :
  SVG animé (SMIL) ajouté.
- **ADR corrigés** : référence `tiny-skia` (ADR-0001 : 0005 →
  présente décision) ; ADR-0003 réécrite (justifications propres,
  compromis `?key=` documenté) ; ADR-0004 (glue FFI côté Flutter
  pour la mémoire partagée) ; ADR-0006 (justification licence sans
  lien amont) ; ADR-0007 (moteur en workspace, échappatoire
  `boa_engine` nommée) ; ADR-0008 (contrat imposé à `dillo-js`) ;
  ADR-0009 (frontière I/O `fontdb`/`FontProvider`) ; ADR-0011
  (profils cloisonnés génériques).
- **Règle §4.3 assouplie** : `debug_assert!` permis sur invariants
  internes purs (jamais sur contenu externe).
- `.gitignore` : `app/pubspec.lock` n'est plus ignoré — une
  application Dart commit son lockfile (builds reproductibles).

### Added (fondations, étape 0)

- Fondations documentaires du dépôt (étape 0) : `AGENTS.md`
  (contrat de travail), `ROADMAP.md` (11 phases, étapes 0–48),
  `CHANGELOG.md`, `docs/architecture.md`.
- ADR initiaux dans `docs/architecture/decisions/` :
  - ADR-0001 — périmètre : moteur from scratch, dépendances
    d'infrastructure pur Rust autorisées, moteurs existants exclus ;
  - ADR-0002 — pile réseau : HTTP/1.1 maison sur tokio + rustls,
    `httparse` pour les en-têtes ;
  - ADR-0003 — couture UI : API de contrôle REST + SSE sur loopback,
    l'app Flutter ne touche jamais le core ;
  - ADR-0004 — transport des frames : PNG tiré sur invalidation SSE
    (MVP), WebSocket/texture FFI en voies d'upgrade ;
  - ADR-0005 — design tokens : `app/assets/design/tokens.json`
    source de vérité unique du design system ;
  - ADR-0006 — double licence MIT OR Apache-2.0 ;
  - ADR-0007 — couture JavaScript : trait `JsEngine`, moteur
    `dillo-js` intégré en phase 8 ;
  - ADR-0008 — modèle mémoire DOM ↔ GC figé avant la phase 2 :
    `NodeId` générationnels, rooting explicite, cycles par table de
    callbacks, invalidation de génération documentée ;
  - ADR-0009 — stack texte/fontes : primitives pur Rust (`fontdb`,
    `ttf-parser`, `rustybuzz`, `unicode-bidi`, `unicode-linebreak`),
    runs itemisés, fonte fixture embarquée ;
  - ADR-0010 — corpus de conformité publics (html5lib-tests, WPT,
    test262, Acid2/Acid3), % tracké par phase ;
  - ADR-0011 — modèle de menace : budgets + cloisonnement de session
    actifs, isolation processus différée, réévaluation obligatoire
    en phase 8.

### Changed

- Cap du projet rétabli : **compatibilité avec le web moderne** —
  le but d'un navigateur est d'accéder à tous les sites. Une version
  initiale des documents plafonnait par erreur le périmètre à un
  « navigateur documentaire » (ES3, pas de flexbox/grid) ; la roadmap
  porte désormais une phase 10 « Compatibilité web moderne »
  (HTTP/2+, flexbox/grid, ES6+, Web APIs, médias, webfonts) et les
  ADR 0001/0004/0007 ont été corrigés en conséquence.
- Optimisations d'architecture et de roadmap (ADR-0004, 0006, 0007,
  `ROADMAP.md`, `architecture.md`) :
  - **Licence `dillo-js` tranchée** : double licence `MIT OR Apache-2.0`
    formalisée par son auteur unique (Laurent Geynet), éliminant tout
    risque de contamination GPL-3.0 dans `explorer-js`.
  - **Gestion mémoire DOM ↔ GC** : `NodeId` générationnels (`slotmap`)
    et système de handles sécurisés pour prévenir les *dangling IDs*
    lors du détachement de nœuds par du script.
  - **Transport de frames accéléré** : intégration dès la phase 6/7
    d'un buffer brut de pixels décompressés en mémoire partagée /
    named pipe local, cantonnant le pull PNG au MVP strict.
  - **Outillage visuel layout précoce** : export et dump wireframe
    dans `explorer-cli` dès la phase 4 pour calibrer le layout sans
    attendre le pipeline de rendu complet.
  - **Paliers de compatibilité web** : découpage formalisé en 3
    paliers structurants (Documentaire → Web 2.0/SSR → Applications riches SPA).
- Optimisation documentaire suite à revue critique (ADR-0001,
  0002, 0003, 0004, 0007, `ROADMAP.md`, `architecture.md`,
  `AGENTS.md`, `README.md`) :
  - **Cap reformulé** : « tous les sites » devient une direction
    asymptotique pilotée par paliers mesurés sur corpus (ADR-0001,
    ADR-0010).
  - **Réseau** : IDNA/punycode, schémas `data:`/`about:`, HSTS,
    `X-Content-Type-Options`, transcodage charset + sniffing WHATWG
    — `windows-1252` avancé aux phases 1–2 (défaut réel de la spec
    HTML), plus phase 10 (ADR-0002).
  - **Entrées** : coalescing des événements mouvement documenté
    (ADR-0003) ; tension texture GPU ↔ isolation de processus
    explicitée pour l'étape 47 (ADR-0004).
  - **JS** : modèle mémoire DOM ↔ GC délégué à ADR-0008, mesure
    test262, échappatoire ES6+ formalisée (ADR-0007).
  - **Roadmap** : étapes 3, 6, 8–10, 12, 16, 18, 20, 23, 26, 34,
    48 enrichies ; référence tiny-skia corrigée (ADR-0001, pas
    ADR-0005) ; section « Hors périmètre assumé » ajoutée.

