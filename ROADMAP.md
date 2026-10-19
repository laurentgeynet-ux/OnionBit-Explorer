# Roadmap — Explorer

Source de vérité de l'avancement. Une étape est cochée quand :
implémentée + testée + documentée (CHANGELOG, ADR si décision) +
commitée (AGENTS.md §5).

Légende : `[ ]` à faire · `[~]` en cours · `[x]` terminée.

Contrainte structurante : **moteur d'abord**. L'app Flutter démarre
en squelette (phase 0) et n'est alimentée que via l'API — chaque
phase backend débloque les évolutions UI correspondantes.

## Jalons (repères de motivation, non bloquants)

| Jalon | Définition | Phase |
|---|---|---|
| **J0** | Dépôt cadré + workspace compilant | 0–1 |
| **J1** | `explorer-cli fetch <url>` télécharge une page réelle décodée | 1 |
| **J2** | `explorer-cli dump <url>` affiche le texte d'une page réelle — premier artefact visible | 2 |
| **J3** | Première page stylée servie en PNG (`GET /frame`) | 5 |
| **J4** | Navigation réelle dans l'app Flutter (liens, scroll, back) | 7 |
| **J5** | Release 0.1 packagée | 9 |

J2 compte double : il rend visible le pipeline bien avant le moindre
pixel, et c'est le premier vrai test bout-en-bout net+dom+html.

## Phase 0 — Cadrage du dépôt

- [x] **Étape 0. Fondations documentaires.** `AGENTS.md`, `ROADMAP.md`,
  `CHANGELOG.md`, licences MIT/Apache-2.0, ADR initiaux
  (`docs/architecture/decisions/`), `docs/architecture.md`.
- [x] **Étape 1. Workspace cargo.** 13 crates `explorer-*`
  squelettes (compilent, `clippy -D warnings`/`fmt --check`/`test`
  passent), `scripts/verify_all.ps1`, CI locale documentée.
  *Bonus : `crates/dillo-js` migre dans le workspace — version
  canonique, licence MIT OR Apache-2.0, équipe JS dédiée.*
- [x] **Étape 2. Squelette Flutter.** `flutter create` desktop dans
  `app/` (Windows d'abord), clean architecture `core`/`features`,
  client API minimal, fichier `assets/design/tokens.json` chargé.

## Phase 1 — Réseau (`explorer-net`)

Objectif : télécharger une page `http(s)` réelle, décodée, bornée.

- [x] **Étape 3. URL.** Résolution relative/absolue, normalisation
  WHATWG, **IDNA/punycode** (crate pur Rust à arbitrer), origine
  (schéma/hôte/port) pour la future politique same-origin, schémas
  `data:`/`about:` minimaux.
- [x] **Étape 4. HTTP/1.1 client.** Connexions tokio, keep-alive,
  requêtes pipelinables non — séquentielles d'abord ; parsing
  statut/en-têtes via `httparse`, corps `Content-Length` +
  `chunked`. Timeouts bornés.
- [x] **Étape 5. TLS.** `rustls` + `webpki-roots` ; erreurs
  certificat remontées typées (l'UI affichera une page d'erreur,
  pas un crash).
- [x] **Étape 6. Décodage.** `gzip`/`deflate`/`br` (crates pur
  Rust), content-length vs chunked résolus, charset déclaré exposé
  + **transcodage** (UTF-8, windows-1252 — `encoding_rs` ou tables
  maison, à arbitrer), `X-Content-Type-Options` honoré.
- [x] **Étape 7. Cookies + cache.** Jar mémoire avec politique
  réelle dès le départ : `Secure`/`HttpOnly`/`SameSite`,
  préfixes `__Host-`/`__Secure-`, domaines validés par **Public
  Suffix List** (crate pur Rust type `psl` — sans elle, un jar
  naïf accepte les supercookies `.co.uk`) ; cache mémoire borné
  (ETag/Last-Modified) — disque optionnel en phase ultérieure.
- [x] **Étape 8. Redirections & politique.** Suivi 3xx borné, store
  **HSTS** mémoire borné, `http_proxy`/`no_proxy` honorés
  (fonctionnalité standard — la couture reste ouverte à tout
  transport mandataire futur : proxy d'entreprise, Tor…) ;
  en-têtes `Referer`/`Origin` calculés par requête et politique
  **mixed-content** (sous-ressource `http` d'une page `https`
  bloquée ou dégradée) câblés dès le scheduler ADR-0012.

## Phase 2 — DOM et HTML (`explorer-dom`, `explorer-html`)

- [x] **Étape 9. DOM complet.** Arène + `NodeId` générationnels
  (pattern `slotmap` index+génération — modèle mémoire figé par
  **ADR-0008** : rooting explicite `add_root`/`remove_root`, cycles
  DOM↔JS par table de callbacks, invalidation de génération
  documentée sur chaque mutation), attributs ordonnés, navigation
  parent/enfant/siblings, `Document`/`Doctype`/`Comment`, texte
  normalisé (`\r\n`→`\n`, `normalize()` fusionne les `Text`
  adjacents), `remove` détache sans détruire, `collect` réclame
  les sous-arbres non rootés, budget `max_nodes` (ADR-0011).
  `text_content`, `insert_before`, garde anti-cycles. 13 tests.
- [x] **Étape 10. Tokenizer HTML5.** États de la spec WHATWG,
  entités nommées/numériques, DOCTYPE/quirks mode — entrée
  **incrémentale** (chunks bornés, ADR-0012) pré-décodée avec
  **sniffing d'encodage WHATWG** (BOM, `<meta>` dans les premiers
  octets, défaut `windows-1252`). Machine à états incrémentale
  (reprise de coupure au milieu de balise/entité/mot-clé),
  texte `Character` coalescé avec ordre du flux préservé,
  rawtext/RCDATA/script/PLAINTEXT pilotés par le tree-builder
  (`ContentKind` + « appropriate end tag »), CDATA en contenu
  étranger, entités longest-prefix + legacy sans `;` (règle de
  l'esperluette ambiguë en attribut), remap `&#80..9F;`
  windows-1252, bornes hostile (`MAX_NAME`/`MAX_VALUE`/flush).
  34 tests (25 tokenizer + 9 encodage).
- [x] **Étape 11. Tree-builder.** Modes d'insertion de la spec
  (Initial → AfterAfterBody, dont InTable/InTableBody/InRow/InCell/
  InCaption/InColumnGroup/InSelect/Text), `html/head/body`
  synthétisés, scopes (button/list/table/formatting), **AFE** avec
  règle de Noé (max 3) + reconstruction après fermeture de bloc +
  adoption agency basique, foster parenting (texte **et** éléments
  déplacés avant la table), `<tbody>`/`<tr>` implicites, cellules
  fermées en chaîne, pilotage du tokenizer (rawtext/RCDATA/script/
  PLAINTEXT, CDATA en `<svg>`/`<math>`), `quirks`/`limited-quirks`
  par DOCTYPE (liste WHATWG complète, règles « missing **or
  empty** »/IBM DTD), `\n` initial sauté après `<pre>`/`<textarea>`,
  contenu post-`</body>`/`</html>` retraité en InBody.
  `TreeBuilder::drive` intercale `next_token`/`process` — les états
  de contenu sont réglés avant la lecture de la suite.
  26 tests (tag-soup, tables, format, quirks, incrémental).
- [x] **Étape 12. Golden tests + premier artefact.** Harness
  **html5lib-tests** (tree-construction, ADR-0010) + corpus
  `tests/data/`, arbres attendus vs produits, % de conformité
  publié dans CHANGELOG ; fuzzing du tokenizer (`fuzz/`) ;
  `explorer-cli dump` (texte normalisé du DOM d'une page réelle —
  **jalon J2**) + spike ADR-0008 avec faux handles JS validant le
  protocole rooting/invalidation avant l'arrivée du vrai moteur.

## Phase 3 — CSS (`explorer-css`)

- [x] **Étape 13. Tokenizer/parser.** Règles, déclarations,
  at-rules (`@media` minimal), tolérance aux erreurs de la spec.
- [x] **Étape 14. Sélecteurs.** Tag/classe/id/descendant/enfant/
  attribut — matching sur le DOM.
- [x] **Étape 15. Cascade + héritage.** Spécificité, `!important`,
  valeurs initiales/héritées, `computed style` par nœud.
- [x] **Étape 16. Propriétés essentielles.** `display` (dont
  `list-item`, `none`), `margin`, `padding`, `border`,
  `width/height`, `color`, `background`, `font-*`, `text-align`,
  `list-style`, `white-space`, `position`, `z-index`, `overflow`,
  pseudo-classes dynamiques (`:hover`, `:focus`…) → restyle via
  l'invalidation ADR-0012 — liste tenue dans l'ADR de phase ;
  sous-ensembles **WPT** selectors/cascade mesurés (ADR-0010)
  *(mesure WPT : voir chantier conformance — à brancher)*.

## Phase 4 — Layout (`explorer-layout`)

- [x] **Étape 17. Arbre de boîtes.** Block/inline/anonymous,
  lien nœud DOM ↔ boîte, dump wireframe debug dans `explorer-cli`
  (feedback visuel immédiat des rectangles avant la phase 5).
- [x] **Étape 18. Stack texte et fontes** (ADR-0009).
  `fontdb`/`ttf-parser` (chargement, métriques), `rustybuzz`
  (shaping), `unicode-bidi`, `unicode-linebreak` ; runs de texte
  itemisés ; fonte fixture embarquée pour les tests headless —
  remplace toute mesure fixe approximative. `fontdb` fait de l'I/O
  (registre/filesystem) : `explorer-layout` n'en dépend pas — le
  port `FontProvider`/`FontSource` est défini côté layout,
  l'implémentation `fontdb` vit en couche infra injectée par
  `explorer-core` (ADR-0009).
- [x] **Étape 19. Block layout.** Flux normal, marges collapsées,
  dimensions auto/spécifiées.
- [x] **Étape 20. Inline layout.** Line boxes sur runs itemisés,
  word wrap (opportunités UAX #14), `text-align`, inline-block
  minimal.
- [x] **Étape 21. Tables, flottants et positionnement.**
  Tables : auto layout CSS 2.1 simplifié — largeurs
  min/pref de colonnes par contenu mesuré, distribution
  proportionnelle, rangées à hauteur max uniformisée.
  `float` posé au bord avec bandes réservées qui rétrécissent
  le flux, `clear` descend sous les flottants ; `position`
  relative (décalage post-flux), absolute/fixed hors flux
  résolus en second passage contre le padding box du plus
  proche ancêtre positionné / le viewport ; `z-index` et
  `overflow` (x/y combinés) en métadonnées usées sur les
  boîtes pour le rendu. Algorithmes bornés, 11 tests.

## Phase 5 — Rendu (`explorer-render`)

- [x] **Étape 22. Display list.** L'arbre de layout produit une
  liste de primitives (rects, texte, images placeholders) :
  `DisplayItem` (fond, bordure 4 côtés, run de glyphes,
  clip, placeholder) ordonnés — non-positionnés dans
  l'ordre de l'arbre puis positionnés triés par `z-index`,
  `PushClip`/`PopClip` autour des `overflow` ≠ visible.
  Fixture TTF remontée dans `explorer-test-support`
  (source unique).
- [x] **Étape 23. Rasterisation.** `tiny-skia` (ADR-0001) :
  couleurs, bordures, texte (contours `ttf-parser` → paths) ;
  framebuffer RGBA, `Mask` de clipping `overflow`, facteur
  d'échelle DPI (HiDPI) appliqué à la rasterisation, PNG via
  `Pixmap::encode_png`.
- [x] **Étape 24. Frame PNG servie.** `GET /api/tabs/{id}/frame`
  (ADR-0004) + événement `frame_invalidated` SSE — l'UI affiche
  sa première vraie page en pull PNG (MVP).

## Phase 6 — API et daemon (`explorer-api`, `explorer-daemon`)

- [x] **Étape 25. Contrat REST.** `/status`, `/tabs` CRUD,
  `navigate`, `reload`, `back`/`forward`, `/events` SSE — erreurs
  `{"error":{handled,message}}`.
- [x] **Étape 26. Entrées.** `POST /api/tabs/{id}/input` :
  clics/souris/clavier/scroll traduits en hit-test sur le layout ;
  **coalescing** des événements mouvement pour limiter les
  round-trips (ADR-0003).
- [x] **Étape 27. Daemon.** `explorer-daemon` : bind loopback,
  clé d'API optionnelle, arrêt propre, logs `tracing`.
- [x] **Étape 28. Historique + bookmarks.** Persistance fichier
  (répertoire de profil déplaçable — couture suite).

## Phase 7 — Interface Flutter (`app/`)

L'UI évolue **au fil des phases backend** ; cette phase consolide.

- [x] **Étape 29. Chrome navigateur.** Omnibox, onglets,
  boutons back/forward/reload, états de chargement via SSE.
- [x] **Étape 30. Viewport moteur.** Affichage frame PNG (MVP —
  pull sur `frame_invalidated`), scroll molette →
  `input scroll`, redimensionnement → `POST viewport`
  debouncé, survol/clic relayés. Le canal direct mémoire
  partagée / texture externe (ADR-0004, dirty-rects) reste
  planifié à l'étape 47 quand les frames l'exigeront.
- [x] **Étape 31. Interactions.** Hit-test via API : clic lien →
  navigation, hover → curseur (lien/champ), saisie champs
  `input`/`textarea` (focus au clic, `key` édite `value`,
  `<br>` = saut de ligne forcé).
- [x] **Étape 32. Design system.** Thème complet dérivé de
  `tokens.json` (ADR-0005) — section `metrics` ajoutée,
  tous les composants stylés ; barre de statut (href
  survolé / champ actif). Thème clair : variante future
  (le JSON accepte d'autres palettes).
- [x] **Étape 33. Pages d'erreur.** Échec réseau/TLS rendu en
  page stylée côté UI (`pageErrors` par onglet via
  `pipeline_error`/`navigate` en échec, effacée au
  `navigated` suivant).

## Phase 8 — JavaScript (`explorer-js`)

### Chantier `dillo-js` — piste parallèle de l'équipe JS dédiée

Le moteur (`crates/dillo-js`) avance sur sa propre cadence, en
parallèle des phases moteur — son ordre est dicté par le contrat
d'intégration, pas par la couverture :

- [x] **JS-A. Alignement heap → ADR-0008** : rooting explicite
  (`Engine::add_root`/`remove_root`, ref-counté), tombstones +
  freelist, passe GC mark-sweep itérative (`Engine::collect`) —
  racines semées : global/prototypes, env global (`let`/`const`),
  `Symbol.for`, timers, microtasks, XHR/fetch en vol, cache
  DOM↔JS + listeners de nœuds, hooks navigateur, complétion ;
  trace proto/props/accesseurs/tableaux/Map/Set/itérateurs/
  promesses/réactions/envs capturés via `Fun::Script` ; hook
  `Host::trace`/`Host::finalize`. Un `ObjId` réclamé résout
  `None` — dégradation tolérante, jamais de dangling. La pile
  d'appel n'est pas une racine : `collect` se pilote entre deux
  jobs (contrat documenté). 13 tests `tests/gc.rs`.
  **Prérequis de l'étape 34 levé** pour la partie heap — reste
  le branchement `explorer-js`/`explorer-dom` générationnel.
- [x] **JS-B. Harness test262** : sous-ensemble langage extrait
  (sparse-checkout @ `d53ebc8`, 1077 fichiers — `types`, `literals`,
  `comments`, `white-space`, `line-terminators`, `identifiers`,
  `hashbang` + `harness/`), frontmatter `flags`/`features`/
  `includes`/`negative` parsé, skips whitelistés et comptés,
  cliquet `BASELINE_PASSED`. **Score initial : 531/859 (62 %),
  218 skips** — échecs concentrés sur identifiants Unicode,
  ASI LS/PS, commentaires HTML. Premier bug trouvé par la suite :
  wrappers `String/Number/Boolean` sans `[[PrimitiveValue]]` →
  récursion infinie `ToPrimitive`→`toString` (corrigé via prop
  cachée `__prim` + `valueOf`/`toString` stricts).
- [x] **JS-C1. Lexique/parser conforme** : identifiants
  Unicode via `unicode-ident` (UAX #31 + `Other_ID_*` +
  ZWNJ/ZWJ + échappes `\u`), U+FEFF/LS/PS/CR, commentaires
  HTML Annex B, `\u{…}`/paires de surrogates dans les
  chaînes, octales legacy + `\8`/`\9` flaggés, regex validée
  au parse + traduction Annex B/`\u{…}`/surrogates/`\cX`,
  **mode strict** (directive prologue script + fonction,
  octales/`delete` nu/liaisons réservées en early error).
  **Score : 836/859 (97 %)** — résidu : attributs de
  propriétés, strict runtime (`ReferenceError`), backrefs,
  modificateurs regexp ES2025.
- [x] **JS-C2. Attributs de propriétés + strict runtime** :
  `PropAttrs` `{writable, enumerable, configurable}` sur
  data/accessor (`Prop::Data`/`Prop::Acc`), `[[Extensible]]`,
  `[[DefineOwnProperty]]` simplifié (fusion, rejet
  non-configurable), `[[Set]]`/`[[Delete]]` ordinaires respectant
  les flags. API surface : `Object.defineProperty` descripteur
  complet, `defineProperties`, `getOwnPropertyDescriptor(s)`,
  `create(proto, map)`, `setPrototypeOf` (+ rejet cycle),
  `freeze`/`seal`/`preventExtensions`/`is*`, `isPrototypeOf`.
  Constantes verrouillées (`undefined`, `NaN`, `Infinity`,
  `Math.*`, `Number.*`), `lastIndex` `{w,e¬,c¬}`, `var` global
  non-configurable. Strict runtime : `FnLit::strict`/`Program::
  strict` propagés (corps de classe toujours stricts),
  `ReferenceError` identifiant non déclaré, `TypeError` écriture
  refusée/delete non-configurable/assign `const`, `this` sloppy
  coercé en global — `eval` direct hérite du strict appelant
  (`parse_in`). Fix : `TextEncoder`/`TextDecoder`/`Symbol` ne
  partagent plus `Object.prototype` (`constructor` correct).
  **Score : 851/859 (99 %)** — résidu : backrefs/lookahead
  regexp (limite du crate `regex`), modificateurs ES2025,
  validations `u` profondes.
- [x] **JS-C3. RegExp complètes — `fancy-regex` + validateur
  ECMA-262** : backend `regex` → `fancy-regex` (backtracker pur
  Rust, `BACKTRACK_LIMIT` borné) — lookahead/lookbehind,
  backrefs `\1`/`\k<nom>` natives. `translate_pattern` devient
  un validateur/traducteur complet (fancy est *plus* permissif
  que la spec) : identifiants de groupes via les tables UAX #31
  du lexer, doublons rejetés, `\k<nom>` avant déclaration →
  match vide (21.2.2.9), `\k` identity seulement en sloppy sans
  groupe nommé, décimales Annex B (préfixe/octale/`\8`/`\9`),
  modificateurs `(?ims:…)` ES2025, quantificateurs `{n,m}` sans
  atome et `n>m` rejetés, `]`/`{`/`}` étendus stricts en `u`,
  ranges de classes `u` validées, `\xNN`/`\uHHHH`/`\u{…}`/
  surrogates fusionnées, `\0`+chiffre octale legacy. Fold `i`
  non-`u` émis à la main (`ToUppercase` simple — KELVIN ne
  matche plus 'k').
  **Score : 859/859 (100 %)** — corpus extrait entièrement
  vert.
- [x] **JS-C4. `Proxy` complet** : `ObjKind::Proxy {target,
  handler}` tracé GC, les 13 méthodes internes routées vers les
  traps (`get`/`set`/`has`/`deleteProperty`/`ownKeys`/
  `getOwnPropertyDescriptor`/`defineProperty`/`getPrototypeOf`/
  `setPrototypeOf`/`isExtensible`/`preventExtensions`/`apply`/
  `construct`), `Proxy.revocable` + `revoke` (handler nul →
  `TypeError` partout), construct-only, `typeof`/`instanceof`/
  `for-in`/`in`/`delete`/spreads/`Object.*`/`Reflect.*` trap-aware.
- [x] **JS-C5. Modules ES** : `import`/`export` complets (nommés,
  `default`, `* as ns`, `export {…} from`, `export *`, import
  d'effet de bord), **live bindings** via `EnvCell::Alias`
  (l'importeur lit l'env de l'exporteur — mutations visibles,
  écriture → `TypeError`), graphe d'instanciation DFS avec
  cycles (exports statiques connus avant récursion,
  `pending` résolus post-DFS), évaluation post-order,
  objet namespace exotique à getters live et proto nul,
  `import()` dynamique → promesse de namespace,
  `import.meta.url`, modules toujours stricts,
  `SyntaxError` précoce sur export manquant. Chargeur hôte
  via trait `ModuleLoader` injecté (`set_module_loader`).
  Approximations documentées : pas de TDZ, specifier =
  identité brute, pas d'ambiguïté `*` (premier gagne).
- [x] **JS-C6. `ArrayBuffer` / `DataView` / vues typées** :
  `ObjKind::ByteBuf` (octets fixes partagés), `DataView`,
  `TypedArray` — les vues référencent leur buffer (tracé GC),
  `TaKind` porte taille/nom/conversion des 9 kinds (wrap
  modulo entiers, saturation `Uint8Clamped`, IEEE floats).
  Indices canoniques + `length`/`byteLength`/`byteOffset`/
  `buffer`/`BYTES_PER_ELEMENT` synthétisés dans `find_own`/
  `set_prop`/`delete`/énumération/`iter_values` — la région
  `byteOffset` est respectée partout. Ctors complets
  (`len`/`buffer,offset,len`/iterable/array-like/view→view),
  `from`/`of`, `ArrayBuffer.slice`/`isView`, `DataView`
  get/set 8 types endianness paramétrable, méthodes
  `%TypedArray%.prototype` (`set`/`subarray`/`fill`/`slice`/
  `map`/`filter`/`sort` numérique/`copyWithin`/`keys`/`values`/
  `entries`/`[Symbol.iterator]`/`at`/reduce/find/every…).
  `TextEncoder.encode` rend un vrai `Uint8Array`,
  `TextDecoder.decode` lit vues/`DataView`/buffer.
  Déviations : props synthétiques = data props (spec :
  accesseurs proto), pas d'`Atomics`/resizable/
  `SharedArrayBuffer`.
- [x] **JS-C7. `BigInt` + vues big** : primitive
  `Value::Big(ObjId)` → `ObjKind::BigInt(num-bigint)`
  (tracée GC). Littéraux `123n`/radix (`0xFn`…) lexés, formes
  malformées rejetées (`08n`, `123n4`). Sémantique complète :
  `typeof` → `"bigint"`, arithmétique exacte (`/` tronquée,
  `**`, `-`/`~`/`++` unaires), bitwise (`>>>` interdit),
  `TypeError` sur mixte bigint/number, comparaisons mixtes
  (`1n < 2` ok), `==`/`===` spec, concat string, `ToBigInt`
  (chaîne → `SyntaxError`, number non-entier → `RangeError`).
  Builtins : `BigInt()` (pas de `new`), `asIntN`/`asUintN`,
  proto `toString(radix)`/`valueOf`, `Number(1n)`,
  `JSON.stringify` → `TypeError`, `[object BigInt]`.
  `TaKind::I64`/`U64` : `BigInt64Array`/`BigUint64Array`
  complètes (mod 2^64, complément à 2) — refactor
  `ta_nums`→`ta_vals` (éléments `Value`, `Big` préservés,
  `reverse`/`copyWithin` au niveau octet), `DataView`
  get/set `BigInt64`/`BigUint64` endianness paramétrable.
  `set_prop`/`find_own`/`get_prop`/`env_get` passés `JsResult`/
  `&mut` pour matérialiser les `Value::Big` et propager les
  `TypeError` de conversion (`u8[0]=1n`, `big[0]=1` lèvent).
  test262 reste 859/859.
- [x] **JS-C8. APIs modernes** : `AggregateError` (proto dédié,
  `errors` own prop), `Error(m, {cause})`, `Promise.withResolvers`/
  `try`/`any` raisons collectées. `structuredClone` — copie
  profonde spec `StructuredSerialize` : cycles préservés
  (table id→clone), objets/tableaux/`Map`/`Set`/buffers/vues
  typées/`RegExp`, `Fun`/`Symbol`/`Promise`/`Proxy`/`Iter`/
  host → `TypeError`. `Object.groupBy` (proto nul, clés
  `ToString`) / `Map.groupBy` (clés `SameValueZero`).
  `Array.fromAsync` — `await` par élément + mapfn, erreurs en
  promesse rejetée. RegExp : flag `d` → `hasIndices` +
  `match.indices`/`indices.groups` (unités char), lookbehind
  `(?<=)`/`(?<!)` natif fancy-regex, ordre canonique `dgimsuy`.
  `Symbol.asyncIterator` + `for await` — `next()` awaité par
  étape, fallback itérable sync (valeurs awaitées, spec
  `CreateAsyncFromSyncIterator`), `for await…in` →
  `SyntaxError`. Iterator helpers ES2025 (`map`/`filter`/`take`/
  `drop`/`flatMap`/`toArray`/`forEach`/`reduce`/`some`/`every`/
  `find`) — drain eager en snapshot `Iter` (déviation
  documentée, paresse spec → timing). `await_value` extrait
  d'`Expr::Await` (partagé `fromAsync`/`for await`). test262
  reste 859/859.
- [x] **JS-C9. Générateurs eager** : `function*`/`yield`/
  `yield*`, méthodes `*m(){}` objet+classe, `async function*`.
  Modèle : le corps tourne **à l'appel** en empilant un
  collecteur `gen_collect` (pile pour générateurs imbriqués) —
  pas de suspension (déviation documentée, cohérente avec
  `await` settled-only ; `next(v)` ignore `v` → l'expression
  `yield` vaut `undefined` ; les générateurs infinis butent
  sur le budget d'opérations). L'appel rend un `ObjKind::Iter`
  étendu (`ret` terminal → `{value, done:true}` du `return`),
  proto partagé + `gen.return(v)`/`gen.throw(e)`. Parser :
  `in_gen` save/restore (flèches/accessors reset), `yield`
  contextuel (identifiant sloppy, `SyntaxError` hors gen),
  pas de newline avant l'opérande, `for await` sur
  générateurs via fallback sync (valeurs awaitées, spec
  `AsyncFromSyncIterator`). test262 reste 859/859.
- [x] **JS-C10. Symboles bien connus + `with` + `Reflect`
  complet** : `@@toPrimitive` (hint `default`/`number`/`string`
  — `+` et `==` passent `"default"`, arithmétique `"number"`),
  `@@hasInstance` (hook lu **avant** l'exigence callable, spec
  `InstanceofOperator`), `@@toStringTag` dans
  `Object.prototype.toString` (tag propre ou hérité),
  `@@unscopables` sur `Array.prototype` (liste spec ES2015+).
  `with (o)` : `Stmt::With`, `EnvKind::With(ObjId)` — les noms
  se résolvent dans les props de `o` (chaîne proto incluse,
  filtre `@@unscopables`), l'écriture va à la prop existante
  sinon remonte au scope parent, `var` hoiste au scope fonction
  (records declarative : le `with` est transparent), primitives
  enrobées dans un wrapper jetable (`with ("abc") { length }`
  → 3), `null`/`undefined` → `TypeError`, early-error en strict.
  Énumération symbole-aware : `Object.getOwnPropertyNames`
  exclut les clés-sentinelles, `getOwnPropertySymbols` les rend.
  `Reflect` complété à 13 méthodes (`defineProperty`,
  `deleteProperty`, `getOwnPropertyDescriptor`, `getPrototypeOf`,
  `setPrototypeOf`, `isExtensible`, `preventExtensions` rendent
  `false` sur refus — déviation : une trap lancée surface aussi
  `false`). test262 reste 859/859 ; 7 tests de régression.
- [x] **JS-C11. Syntaxe moderne** : séparateurs numériques
  (`1_000`, `0xFF_FF`, `1n`, fractions/exposants — validation
  « entre deux chiffres »), statements labellés `id:` avec
  `break`/`continue label` (validation early-error : label
  inconnu, `continue` vers non-boucle ; `Flow::Break(Some)`/
  `Continue(Some)` propagés via `pending_labels` + `loop_flow`),
  champs de classe `x = e` / `x` / `[k] = e` (init = thunk clos
  sur le scope de classe, exécuté par `install_class_fields` :
  base → tête de `[[Construct]]`, dérivée → juste après
  `super()` via `FLD_FIELDS_D`, ordre parent→dérivé), champs
  statiques + `static {}` (exécutés à `eval_class`, `this` =
  ctor), noms privés `#x` — résolution lexicale au parse vers
  clés sentinelles `\u{f131}privN.x` (`PrivateNameEnvironment`
  simulé par pile `priv_scopes`, `#x` inconnu → `SyntaxError`),
  champs/méthodes/accesseurs privés, `#x in obj`, `C()` sans
  `new` → `TypeError` (marque `CLASS_CTOR`), double `super()`
  → `TypeError` (marque `FLD_DONE`). test262 : 859/859 ;
  5 tests de régression.
- [x] **JS-C12. Construction + API ES2025** : `new.target`
  (`Expr::NewTarget`, liaison cachée `\u{f120}new.target`
  posée par `call_value_impl` — héritée lexicalement par les
  flèches, propagée aux super-constructeurs, `undefined` en
  appel ordinaire), proto de `[[Construct]]` via
  `Get(newTarget, "prototype")` (traps/getters traversés —
  corrige `new (proxy)`), `Reflect.construct(f, args,
  newTarget)`, méthodes `Set` ES2025 (`union`, `intersection`,
  `difference`, `symmetricDifference`, `isSubsetOf`,
  `isSupersetOf`, `isDisjointFrom` — acceptent tout itérable
  set-like), `String.prototype.isWellFormed`/`toWellFormed`,
  `RegExp.escape` (ES2025 — `a…` → `\x61…` en tête),
  `ArrayBuffer` redimensionnable (`resize`, `resizable`,
  `maxByteLength`) + `transfer`/`transferToFixedLength`
  (buffer détaché → accès vue → `TypeError`, plus de panic).
  test262 : 859/859 ; 4 tests de régression.
- [x] **JS-C13. Symboles réels + tagged templates + WeakRef** :
  clés-symboles uniques (`ObjKind::Symbol{desc,key}` — bug de
  collision par description corrigé, `o[s1] ≠ o[s2]`),
  `Symbol.*` exposés comme vraies valeurs symbole (`typeof` =
  `"symbol"`, `Symbol.dispose`/`asyncDispose` ajoutés),
  `getOwnPropertySymbols` reverse-lookup via `sym_index`,
  tagged templates `` tag`…` `` (chunks `(cooked, raw)` au
  lexer, `this` lié pour `o.m`tpl``, `strings.raw`, `?.` +
  template → `SyntaxError`), `String.raw`, `fn.name`
  synthétique (`lit.name`, nom natif, `bound <cible>`,
  `Function.prototype.name === ""`), `WeakRef`/`deref` et
  `FinalizationRegistry` — références réellement faibles
  (cibles non tracées par le GC, `deref()` → `undefined`
  après collecte, cleanup `cb(held)` en microtask). test262 :
  859/859 ; 4 tests de régression.
- [x] **JS-C14. Tags, chaîne générateur, reviver, stack,
  normalize** : `@@toStringTag` posé sur les prototypes et
  namespaces (`Map`/`Set`/`Promise`/`Math`/`JSON`/typed
  arrays/`globalThis`… — `new Map()[Symbol.toStringTag]` =
  `"Map"`, `toString.call` → `"[object Map]"` ; tag par kind
  sur les vues typées BigInt corrigé), chaîne prototype
  `function*`/`async function*` conforme (`%GeneratorFunction%
  .prototype%` → `%Generator%` → `%IteratorPrototype%` :
  `getPrototypeOf(g()).constructor.name` = `"GeneratorFunction"`,
  `getPrototypeOf(g()) === g.prototype`, `[object Generator]`),
  `JSON.parse` reviver (`InternalizeJSONProperty` —
  post-ordre, `undefined` → suppression), `Error.prototype.
  stack` own (première ligne `Name: msg` — déviation : pas de
  frames), `String.prototype.normalize` NFC/NFD/NFKC/NFKD via
  `unicode-normalization`. test262 : 859/859 ; 5 tests de
  régression.
- [x] **JS-C15. Hooks `@@*` + `Array.of` +
  captureStackTrace** : dispatch spec `str.x(rx)` →
  `rx[@@x](str)` pour `match`/`matchAll`/`replace`/
  `replaceAll`/`search`/`split` (RegExp.prototype porte les 5
  hooks — objets custom interceptables), `Array.of` (`this`
  ctor → `new this(len)`), `@@species` (`ArraySpeciesCreate`
  pour `map`/`filter`/`slice`/`concat` — `null`/`undefined` →
  Array ordinaire), `@@isConcatSpreadable` dans `concat`
  (spread via `length` + `Get` — array-likes et proxies
  traversés), `Error.captureStackTrace` (own `stack`
  `Name: message` — V8-compat, pas de frames). test262 :
  859/859 ; 4 tests de régression.
- [x] **JS-C16. Locale-fallbacks + `URL`/`URLSearchParams`** :
  `reduceRight`, `localeCompare` (ordre point de code — pas de
  CLDR), `toLocale{Lower,Upper}Case`, `toLocaleString`
  (Array/Number/Date/String — replis documentés), `URL`
  WHATWG subset (schéma/authority/path/query/fragment, ports
  par défaut, `origin`, résolution relative `//h` `/p` `?q`
  `#f` `./..`, `file:` — pas d'IDNA/punycode),
  `URLSearchParams` complet (`get`/`getAll`/`has`/`set`/
  `append`/`delete`/`sort`/`forEach`/`keys`/`values`/
  `entries`/`@@iterator`, encodage
  `application/x-www-form-urlencoded`, init string/record/
  itérable/copie). test262 : 859/859 ; 3 tests de régression.
- [x] **JS-C17. ES2024-2026 — Float16, base64/hex,
  Annex B, `Iterator`, `crypto`, `DisposableStack`** :
  `Float16Array` (`TaKind::F16` — codec IEEE-754 binary16
  natif : round-to-nearest-even, subnormaux, ±0/±∞),
  `Math.f16round`, `DataView` `getFloat16`/`setFloat16`
  (endianness), `Uint8Array` `fromBase64`/`toBase64`/
  `fromHex`/`toHex`/`setFromBase64`/`setFromHex` (alphabet
  standard+urlsafe, `SyntaxError` strict sur hex,
  `{read,written}`), `Array.prototype.copyWithin`
  (sémantique memmove — snapshot de la plage source),
  Annex B : `__proto__` accesseur (objets + primitives via
  `read_prop` — getters hérités invoqués avec `this` =
  primitive), `__defineGetter__`/`__defineSetter__`/
  `__lookupGetter__`/`__lookupSetter__`,
  `propertyIsEnumerable` (own + flag ; indices de string
  énumérables), global `Iterator` (ctor abstrait,
  `Iterator.from` — `GetIteratorFlattenable`),
  `crypto.getRandomValues`/`randomUUID` (PRNG xorshift64 —
  PAS cryptographique, déviation documentée, quota 65 536
  octets), `DisposableStack` complet (`use`/`adopt`/`defer`/
  `dispose`/`move`/`disposed`/`@@dispose`/`@@toStringTag` —
  LIFO, erreurs enroulées `SuppressedError`) +
  `SuppressedError`. `using`/`AsyncDisposableStack`/`Intl`/
  `Temporal` reportés. test262 : 859/859 ; 7 tests de
  régression.
- [x] **JS-C18. Annex B complet + `SharedArrayBuffer`/
  `Atomics`** : méthodes HTML String (`anchor`/`big`/`blink`/
  `bold`/`fixed`/`fontcolor`/`fontsize`/`italics`/`link`/
  `small`/`strike`/`sub`/`sup` — `CreateHTML`, `"` →
  `&quot;`), `Date.prototype.getYear`/`setYear` (année
  0-99 → 1900+n), `Number.EPSILON`/`MIN_VALUE`/
  `isSafeInteger`, `AggregateError.prototype.name` propre,
  accesseurs `ArrayBuffer.prototype` (`resizable`/
  `maxByteLength`/`detached` — `TypeError` sur non-buffer,
  spec), `BigInt.prototype.toLocaleString`,
  `SharedArrayBuffer` (`ByteBuf` partagé, `growable`/
  `grow`), `Atomics` complet : `load`/`store`/`add`/`sub`/
  `and`/`or`/`xor`/`exchange`/`compareExchange` (retour
  ancienne valeur, wrap à la largeur, BigInt64/Uint64 —
  opérandes BigInt obligatoires), `isLockFree`,
  `wait` → `TypeError` (main thread, spec navigateur),
  `notify` → 0, `waitAsync` → `{async:false,value}`.
  Mono-thread : aucun partage inter-worker — indiscernable
  sauf `wait`/`waitAsync`, déviation documentée.
  test262 : 859/859 ; 6 tests de régression.
- [x] **JS-C19. Date complet, `eval` direct, Annex B
  RegExp, JSON, invariants Proxy** : les 15 setters
  `Date` (`setFullYear`/`setMonth`/`setDate`/`setHours`/
  `setMinutes`/`setSeconds`/`setMilliseconds`/`setTime` +
  variantes UTC — fuseau unique), formats spec
  `toString`/`toDateString`/`toTimeString`/`toLocale*`/
  `toJSON` (fuseau UTC fixe, `en-US` pour les locale*),
  `Date.parse` legacy (`Jan 15 2000`, `January 15, 2000`,
  `15 Jan 2000` — bug année 1915 corrigé —, `MM/DD/YYYY`,
  `YYYY/M/D`, heures, suffixes `GMT`), `days_from_civil`
  normalise les mois hors 1-12 (`setMonth(13)` → an+1).
  `arguments.callee` sloppy (strict : absent, déviation
  vs poisoned accessor — pas d'aliasing param/arguments).
  `eval(...)` direct : exécuté dans l'env + `this` de
  l'appelant (`PerformEval` chemin local — `var`/
  fonctions persistent dans la fonction, bindings
  visibles, `this.v` accessible ; indirect `e()`/
  `(0,eval)`/`Function()` reste global ; déviation :
  `let`/`const` de l'eval vivent dans l'env appelant,
  la spec les isole). Annex B : `trimLeft`/`trimRight`,
  statiques `RegExp` (`$1`-`$9`, `input`/`$_`,
  `lastMatch`/`$&`, `lastParen`/`$+`, `` $` ``/`$'`/
  `leftContext`/`rightContext` — `UpdateRegExpStatics`
  sur exec/match/search/replace/matchAll). `JSON.stringify` :
  `PropertyList` (filtre + ordre), replacer fonction
  (holder `this`, clé `""` à la racine), symboles élidés
  (valeur + racine), `toJSON` dispatché, wrappers
  `Number`/`String` en `space`. `Number.prototype.
  toString(radix)` : fraction en base (mantisse 53 bits
  max) — `(0.5).toString(2)` → `"0.1"`. `@@toStringTag`
  : `WeakMap`/`WeakSet` protos+ctors dédiés (≠ `Map`/
  `Set`), `ArrayBuffer` tagé ; `new ArrayBuffer()` 0-arg
  → buffer vide (spec `ToIndex`), idem `SharedArrayBuffer`.
  `Array.prototype` est un vrai tableau exotique
  (`Array.isArray` → `true`, descripteur `length`
  `{w:true,c:false}`). Invariants Proxy (spec §9.5) :
  `get`/`set` sur props non-config/non-writable, `has`
  sur prop non-config ou cible non-extensible,
  `deleteProperty`, `getOwnPropertyDescriptor`
  (absent vs non-config, non-config rapporté sur prop
  absente/configurable, ajout sur non-extensible),
  `ownKeys` (clés non-config requises, liste exacte sur
  non-extensible), `defineProperty` (ajout sur
  non-extensible) → `TypeError`. test262 : 859/859 ;
  10 tests de régression.
- [x] **JS-C20. `Intl` minimal, `Array.from` complet,
  invariants Proxy proto, `getOrInsert`,
  `AsyncDisposableStack`** : `Array.from` est spec
  (`@@iterator` → itérateur, sinon array-like `length`
  — `{length:2}` ne rend plus `[]` —, `mapFn` + `thisArg`
  honorés, `TypeError` sur mapFn non-callable). Invariants
  Proxy des traps proto (spec §9.5) : `getPrototypeOf`
  (résultat = proto cible si non-extensible),
  `setPrototypeOf` (proto demandé identique si cible
  scellée), `isExtensible` (booléen conforme à la cible),
  `preventExtensions` (true exige cible non-extensible) →
  `TypeError`. `Intl` (en-US/UTC fixe, pas de données CLDR
  — déviation documentée) : `getCanonicalLocales`/
  `supportedLocalesOf` (canonicalisation BCP-47), les 8
  constructeurs `DateTimeFormat` (dateStyle/timeStyle/
  composants, formatToParts, resolvedOptions, `format`
  bound spec), `NumberFormat` (currency/percent/unit,
  grouping, signDisplay, notation compact/scientific,
  formatToParts), `Collator` (numeric, sensitivity),
  `PluralRules` (cardinal/ordinal en), `ListFormat`
  (conjunction/disjunction/unit), `RelativeTimeFormat`
  (numeric auto → tomorrow/yesterday/now), `Locale`
  (accesseurs + overrides options, maximize/minimize
  retournent des Locale), `DisplayNames` (tables régions/
  langues/scripts/devises + repli code). `Map`/`WeakMap`
  `.getOrInsert`/`.getOrInsertComputed` (ES2026, hérités
  WeakMap via chaîne proto). `AsyncDisposableStack`
  complet (`use` → `@@asyncDispose` ?? `@@dispose`,
  `adopt`/`defer`/`move`/`disposed`, `disposeAsync`
  awaité eager → promesse, `SuppressedError` enroulé).
  test262 : 859/859 ; 8 tests de régression.
- [x] **JS-C21. `splice`, `using`/`await using`,
  `@@hasInstance`, ES2026** : `Array.prototype.splice`
  mutable (start/deleteCount clampés spec, insertion,
  retour des supprimés — gap critique corrigé). Gestion
  explicite de ressources (ES2026) : `using x = e` et
  `await using x = e` — lexing contextuel (`using` reste
  un ident valide), frames de disposeurs par corps,
  dépilement LIFO sur toute sortie (normal, `return`,
  `throw`, `break`), `@@dispose` / `@@asyncDispose` +
  await eager, `null`/`undefined` tolérés, objet sans
  disposeur ou primitive → `TypeError`,
  `for (using r of it)` dispose à chaque itération,
  erreurs enroulées en `SuppressedError` (spec
  `DisposeResources`). `Function.prototype
  [Symbol.hasInstance]` = `OrdinaryHasInstance` (bound →
  cible via `InstanceofOperator`, `prototype` non-objet
  → `TypeError`) — refactor `instance_of`/
  `ordinary_has_instance` partagé. `Error.isError`
  (chaîne proto → `Error.prototype`, approximation
  documentée du slot `[[ErrorData]]`), `Math.sumPrecise`
  (somme compensée Neumaier), `Symbol.metadata`,
  `JSON.rawJSON`/`isRawJSON` (objet gelé à proto nul,
  émission brute dans `stringify`). test262 : 859/859 ;
  5 tests de régression.
- [x] **JS-C22. `Temporal` tranche initiale + gaps** :
  `src/temporal.rs` — `Instant` (epoch ns `i128`,
  `fromEpochMilliseconds`/`fromEpochNanoseconds`,
  offsets ISO appliqués), `PlainDate`/`PlainTime`/
  `PlainDateTime` (props complètes : `dayOfWeek`,
  `dayOfYear`, `weekOfYear`/`yearOfWeek` ISO-8601,
  `daysInMonth`, `inLeapYear`…), `PlainYearMonth`/
  `PlainMonthDay`, `Duration` (signe uniforme exigé,
  `neg`/`abs`/`total`/`compare` avec `relativeTo`),
  `Temporal.Now` (`instant`/`plainDateTimeISO`/
  `plainDateISO`/`plainTimeISO`/`timeZoneId` → `UTC`),
  `from`/`compare`/`with`/`equals`/`add`/`subtract`/
  `since`/`until` avec `largestUnit`, parse+format ISO
  (`P…` durées, fraction 3/6/9 chiffres),
  `valueOf` → `TypeError` (spec). Déviations
  documentées : calendrier `iso8601` unique, fuseau
  UTC (pas de `ZonedDateTime`), props en data
  non-énumérables au lieu d'accesseurs proto, champs
  sub-jour ignorés par `PlainDate.add`. Compléments :
  `Atomics.pause` (hint no-op), `Iterator.concat`
  (eager), propriétés restreintes `caller`/`arguments`
  (`AddRestrictedFunctionProperties` : accesseurs
  poison `TypeError` sur fonctions strictes/gen/async).
  test262 : 859/859 ; 6 tests de régression.
- [x] **JS-C23. Sémantique UTF-16 + intégrité des
  éléments de tableau** : les APIs String comptent
  désormais en code units UTF-16 (spec) — `.length`,
  `s[i]`, `charAt`/`charCodeAt`/`at`/`codePointAt`
  (paire → code point, surrogate isolé → le unit),
  `indexOf`/`lastIndexOf` et `from`, `slice`/
  `substring`/`substr`, `split('')`, positions de
  `includes`/`startsWith`/`endsWith`, `padStart`/
  `padEnd`, `fromCharCode` (paire reconstituée)/
  `fromCodePoint`, `propertyIsEnumerable`. Déviations
  documentées : surrogate isolé → `U+FFFD` (strings
  UTF-8 natives, `String::from_utf16_lossy`), indices
  `match.index`/regex restés en chars. Bug hostile
  corrigé : `startsWith(pos)` tranchait `s[pos..]` sur
  une frontière non-`char` (panic). Intégrité
  `Object.seal`/`freeze` sur les éléments
  (`Object::elem_integrity` bit-flags 1/2/4) : éléments
  non-configurables (`seal`), non-writable (`freeze`),
  `length` verrouillée (`freeze` ou
  `defineProperty(len,{writable:false})`), descripteurs
  corrects, mutators via `Set`/`Delete` en mode throw
  (`put_elems_guarded` + checks upfront : `push`/`pop`/
  `sort`/`splice`/`fill`/`copyWithin`/`reverse`/
  `unshift` → `TypeError` même en sloppy, `fill`/`sort`
  no-op admis), `seal`/`freeze` sur vue typée non vide
  → `TypeError` (spec), `defineProperty` sur indice et
  sur `length` (resize, verrou). test262 : 859/859 ;
  6 tests de régression.
- [x] **JS-C24. `arguments` mappé, `bind` props,
  protos Weak*, `fn.toString`** : objet `arguments`
  mappé (`Object::args_map` = `(EnvRef, Rc<map>)`,
  spec `CreateMappedArgumentsObject`) — fonctions
  sloppy à liste de params simple : lecture/écriture
  d'indice aliasée sur le slot env du paramètre
  (`find_own`/`set_prop`/`delete_prop` interceptés,
  `delete` démappe), noms dupliqués → dernier slot,
  `eval` traverse, `arguments.length` configurable,
  GC trace l'env capturé. Déviation documentée : les
  mutators de tableau (`sort`…) écrivent les
  éléments sans remonter l'alias. `bind` : `length` =
  max(0, target.length − nArgs liés) et `name` =
  `"bound "+target.name` (spec `BoundFunctionCreate`,
  props `{w:false,c:true}`). `WeakMap`/`WeakSet` :
  protos reparentés à `Object.prototype` (spec — fin
  de la fuite `@@iterator`/`keys`/`size` hérités de
  Map/Set), seuls `get`/`set`/`has`/`delete`/`add` +
  upserts ES2026 exposés. `Function.prototype
  .toString` reconstruit les paramètres depuis les
  patterns (`function f(a, b) { [code] }`,
  `(x) => { [code] }`, `function*`/`async function`,
  `[native code]` pour natives/bound — approximation
  du texte source). test262 : 859/859 ; 4 tests de
  régression.
- [x] **JS-C25. `[[OwnPropertyKeys]]` ordre spec,
  tableaux épars, Symbol `+`/ToString TypeError,
  props propres des string-box, `new String`
  `.length`, `\p{…}` regex `u`** : ordre
  `[[OwnPropertyKeys]]` (`spec_key_order` —
  indices entiers croissants, puis strings par
  insertion, puis symboles) appliqué à
  `own_keys`/`own_enum_keys`/`enum_keys` (for-in),
  `Object.keys`/`getOwnPropertyNames`/`JSON.stringify`.
  `Value::Hole` : `Array(n)` crée des trous (spec
  `ArrayCreate`), ainsi que littéral `[1,,3]`,
  `delete a[i]`, grow `length`/`arr[i]` ; `find_own`
  rend `None` (`in`/`hasOwnProperty`/descripteurs
  voient l'absence), chemins génériques Hole ≡
  `undefined` (coercions, `strict_eq`/`loose_eq`,
  `typeof`, JSON→null, itérateurs/`pop`/`at`→
  `unhole`). Méthodes `HasProperty` sautées
  (`forEach`/`filter`/`find*`/`some`/`every`/
  `indexOf`/`flat*`, `map` préserve le trou,
  `reduce` acc = premier présent) ; `includes`
  =undefined (SameValueZero), `join`→"",
  `slice`/`concat` préservent, `with`/`toReversed`/
  `toSorted`/`toSpliced` densifient (spec `Get`).
  `ToPrimitive(symbole)` → le symbole (spec
  `@@toPrimitive`) — `sym+x`/`sym<n`/template →
  `TypeError` via `to_number`/`to_js_string`,
  `String(sym)` explicite reste légal. String-box :
  `hasOwnProperty`/`Object.keys`/`getOwnPropertyDescriptor`/
  `getOwnPropertyNames` sur primitif (`length` +
  indices), `new String(s)` gagne `length` propre
  non-writable (spec `StringCreate`). `\p{Name}`/
  `\p{Name=Value}` passent à fancy-regex en mode
  `u` (validation + erreurs spec). test262 :
  859/859 ; 6 tests de régression.
- [ ] **JS-C. Couverture par besoin** : `Temporal`
  (`ZonedDateTime`, calendriers non-iso, `round`),
  `Intl` multi-locales, décorateurs `@dec` — ordre
  piloté par les pages réelles.
- [ ] **JS-D. Médias** : `canvas` 2D (pont tiny-skia), `Audio`/
  `Video` — phase 10, dépend du pipeline rendu.

Étapes navigateur (intégration) :

- [x] **Étape 34. Intégration `dillo-js` + alignement GC.** Le
  moteur vit dans ce workspace (`crates/dillo-js`, équipe JS
  dédiée, MIT OR Apache-2.0 — ADR-0007). Prérequis levés :
  (a) heap aligné sur ADR-0008 (JS-A livré) ; (b) ADR-0011
  écrit. Adaptation livrée : `JsEngine` (trait `Send`) dans
  `explorer-js` — `eval` → complétion décrite, budget
  d'ops (`set_op_limit`), `run_pending` ; `DilloEngine`
  = proxy canal → **thread dédié par contexte** (le tas
  `Rc` ne quitte pas son thread, la session reste `Send`,
  le pipeline ne peut pas être suspendu) ; `NullEngine`
  défaut. `Session.set_js_factory` + `Tab.js` : contexte
  neuf par document, `<script>` inline évalués dans
  l'ordre après parse, échecs → `pipeline_error` sans
  faire échouer la page. Couverture au branchement :
  noyau ES3/ES5 + subset ES2015 (test262 851/859, 99 % sur
  le sous-ensemble extrait) ; la couverture grandit en
  phase 10 (étape 43).
- [x] **Étape 35. Bindings DOM.** Livré par **miroir
  synchronisé** (pas de re-implémentation) : la surface DOM
  complète de `dillo-js` (`document`, `navigator`, `window`,
  `getElementById`, `querySelector`, `createElement`,
  `appendChild`, `textContent`, `innerHTML`, attributs,
  collections live…) est installée via `Engine::install_dom`
  sur un `dillo_js::Dom` parsé depuis le HTML source, côté
  thread JS. `JsEngine` gagne `install_dom(html)` +
  `sync_dom()` : le dirty bit du miroir est sondé après les
  scripts ; s'il est levé, `serialize_document()` est
  **re-parsé dans `explorer-dom`** → le relayout qui suit
  rend les mutations à l'écran (invalidation = re-parse
  borné + relayout existants — handles `NodeId` jamais
  exposés au JS, la génération reste interne). Limites
  assumées documentées : le miroir re-parse le HTML source
  (les corrections du tree-builder HTML5 peuvent diverger
  légèrement), `document.title=` n'est pas un setter en
  amont, et la synchro n'est déclenchée qu'en fin de
  navigation (pas encore par événement — étape 36). Tests :
  4 adaptateur (global `document`, dirty+serialize,
  lecture seule, noop `NullEngine`) + 2 session bout en
  bout (`textContent`/appendChild mutés → PNG identique à
  la page statique équivalente).
- [x] **Étape 36. Boucle d'événements.** `click` dispatché
  dans le miroir via `JsEngine::dispatch_event` : le core
  calcule le chemin `(tag, index parmi les frères de même
  tag)` de la cible hit-testée, le worker le rejoue sur le
  `dillo_js::Dom` (saut tolérant des wrappers implicites
  `html`/`body`/`head`/`tbody*` absents du parse littéral)
  puis `Engine::dom_event_on` — propagation
  capture→cible→bulle complète, listeners
  `addEventListener` **et** `onclick=` propres, microtasks
  drainées par dispatch, `defaultPrevented` remonté (un
  clic `preventDefault` sur `<a>` ne navigue pas).
  `run_pending` après chaque dispatch draine les timers
  dus — `setTimeout(…, 0)` pendant le chargement mute le
  document et le rendu final (timers différés : tirent au
  prochain point de synchro — input/navigation —, un tick
  dédié reste une amélioration documentée). Mutations de
  listener → `sync_dom` → re-parse + relayout +
  `frame_invalidated` ; erreur de listener →
  `pipeline_error`, jamais de blocage du pipeline.
  Limites assumées : `mouseover`/`keydown`/`focus`/`blur`
  non dispatchés (clic seul à cette étape), l'action par
  défaut champ-lien suit l'événement (spéc DOM). Tests :
  5 adaptateur (listener atteint, saut de wrappers,
  preventDefault, chemin fautif no-op, sans miroir) + 3
  session bout en bout (click→PNG muté ≡ statique,
  preventDefault→`Ignored`+URL inchangée, timer 0 rendu).

## Phase 9 — Consolidation

- [x] **Étape 37. Images.** `<img>` = boîte remplacée à taille
  intrinsèque : fetch sous-ressource via `HttpTransport` injecté
  (borné `max_images`/`image_max_px`), décodage pur Rust `image`
  (png/jpeg/gif/webp), blit nearest-neighbor + alpha dans le
  raster, repli `alt` sur tout échec. Décodage progressif
  (incrémental) reporté — hors scope du palier.
- [x] **Étape 38. Formes.** `<form>` GET/POST : champs natifs
  rendus par le moteur (ADR-0014) — valeur en enfant `Text`
  synthétique, marque `checked`, `<select>` cycle au clic,
  soumission urlencodée via `HttpTransport`, événement
  `submit` JS + `preventDefault` honoré.
- [x] **Étape 39. Perf + fuzz.** Benchs pipeline, bornes
  mémoire/temps, corpus fuzz tokenizer/parser.
- [x] **Étape 40. Release 0.1.** Packaging Windows
  (daemon + app), smoke test bout en bout, documentation.

## Phase 10 — Compatibilité web moderne (cap long terme)

Objectif explicite : **naviguer le web réel — tous les sites**,
comme direction asymptotique (ADR-0001) : aucun moteur hors des
trois grands n'y parvient ; la progression se pilote par paliers
mesurés sur corpus publics (ADR-0010), pas par promesse. Pour
garantir des résultats mesurables sans épuisement, la compatibilité
progresse en trois paliers structurants :

- **Palier 1 — Web documentaire (statique)** : Wikipedia (mode HTML),
  spécifications W3C/RFC, blogs, sites de documentation. Layout flux normal
  robuste, fontes précises, liens fonctionnels, zéro crash.
  *Honnêteté de périmètre : une part croissante de ces sites utilise
  flexbox — sans l'étape 42 leurs pages dégradent en flux block ;
  c'est assumé et mesuré sur corpus, pas masqué.*
- **Palier 2 — Web 2.0 & interactif** : Formulaires complets, CSS2.1
  (tables, flottants), sessions/cookies, forums, JS basique (ES3/ES5).
- **Palier 3 — Applications modernes (SPA)** : Flexbox, Grid, ES6+, Web APIs,
  sites riches temps réel.

Étapes de la phase 10 :

- [x] **Étape 41. HTTP/2 puis HTTP/3.** Multiplexage h2 sur
  `tokio`+`rustls` ; QUIC quand l'écosystème pur Rust le permet.
  *h2 livré (ALPN, multiplexage, corps bornés) — QUIC/h3
  reporté : `quinn` pur Rust viable mais h3 nécessite un
  effort dédié hors proportion du bénéfice à ce stade.*
- [x] **Étape 42. Layout moderne.** `flexbox`, `grid`,
  `position: sticky`, transforms, `calc()`, custom properties
  (`--var`).
  *Livré en MVP borné (ADR-0015) : `var()`/fallback/IACVT,
  `calc()` `+−×÷` imbriqué, flex une ligne (order, grow/
  shrink/basis, justify/align, gap), grid template explicite
  + spans + auto-placement, transforms 2D au raster,
  sticky `top` par translation de paint. Écarts documentés
  en tête de modules (`flex-wrap`, areas, max-content…).*
- [x] **Étape 43. JS vers ES6+.** Les 4 livrables nommés sont
  livrés dans `dillo-js` : modules `import`/`export` + `import()`
  dynamique + live bindings (JS-C5), `Proxy` complet avec
  invariants (JS-C4 + C19/C20), `Intl` (JS-C20 — subset
  `en-US`/UTC fixes, pas de CLDR : déviation documentée rangée
  sous « JS-C couverture par besoin »), GC mark-sweep + rooting
  ADR-0008 (JS-A). Couverture effective bien au-delà du périmètre
  nommé : JS-C1→JS-C24 (UTF-16, arguments mappé, Temporal
  subset, `using`, TypedArrays, Annex B…), 214 tests langage,
  test262 859/859 sur le sous-ensemble extrait — échappatoire
  `boa_engine` non requise. *Restes structurels documentés
  (hors scope, pilotés par les pages réelles) : générateurs
  lazy/`await` sur pending, décorateurs, `Intl` multi-locales,
  `Temporal` complet, branchement `explorer-dom` générationnel
  ↔ miroir JS.*
- [ ] **Étape 44. Web APIs.** `fetch`/`XHR`, `localStorage`/
  `sessionStorage`, History API, `MutationObserver`, workers
  (bornés). *En cours — côté moteur livré : `location.*` et
  `history.*` deviennent des intentions `NavRequest`
  drainées par la session (navigation, historique, `go(0)` =
  reload, borne 8 sauts) ; `fetch`/`XHR`/storage livrés
  côté `dillo-js` par l'équipe JS (JS-C16+).*
- [x] **Étape 45. Médias et graphiques.** *Côté moteur livré
  (ADR-0017) : `<img src=*.svg>` rasterisé via `resvg`/`usvg`
  (sniff + borne `max_px`, SMIL/scripts hors périmètre) ;
  transitions CSS — `transition`/longhands, `cubic-bezier`,
  interpolation nombres/longueurs/couleurs, diff sur valeurs
  calculées indexé par **ordinal d'élément** (survit au
  re-parse), tick piloté par le pull `/frame`. `rAF` déjà
  livré côté `dillo-js`. Déviations : `<svg>` inline (exige
  foreign-content tokenizer), `<canvas>` 2D (couture JS —
  équipe dédiée), `<video>`/`<audio>` (décodage à arbitrer),
  `@keyframes`.*
- [x] **Étape 46. Webfonts + charsets.** `@font-face` (inline et
  feuilles `<link>` — premières sous-ressources CSS), fetch borné,
  WOFF1 complet + WOFF2 transform nulle (glyf transformé refusé
  proprement), `font-display` parsé (comportement `block`
  appliqué), matching famille/graisse/style — ADR-0016.
  Encodages legacy : déjà livrés à l'étape 5 (windows-1252,
  BOM, meta sniff).
- [x] **Étape 47. Streaming UI haute performance.** Mode
  **in-process** `explorer-ffi` (cdylib, ADR-0018 — tranche la
  tension documentée dans ADR-0004) : ABI C étroite
  (`create`/`navigate`/`input`/`frame_size`/`frame_copy` RGBA
  brut/`tick`/`last_error`), `dart:ffi` `FfiEngine implements
  BrowserRepository` → `ui.Image` via `decodeImageFromPixels` +
  `RawImage` — zéro codec PNG ni socket sur le chemin frame.
  Repli REST transparent (`EXPLORER_ENGINE=auto`), sondage
  `tick` 50 ms remplace le SSE. Déviations : isolation
  processus dégradée en mode FFI (REST reste la référence),
  titre/URL de survol non exportés, pas d'`ExternalTexture`
  GPU (plugin C++ reporté).
- [x] **Étape 48. Interop continue.** Corpus **WPT** ciblés +
  sites réels, jalons Acid2/Acid3 informatifs (ADR-0010) ; les
  écarts constatés deviennent des étapes.
  Reftests pixel-exacts `tests/wpt_reftests.rs` : 40 cas
  `css/CSS2/floats` + `normal-flow` extraits par
  `scripts/fetch_wpt.ps1` (+ fonte Ahem), `rel="match|mismatch"`
  résolu, tolérance AA documentée (0 px dur, ≤512 px doux),
  cliquet `WPT_BASELINE` = **17/40 (42 %)** mesuré — échecs
  réels listés : placement `flow-root`, bands float gauche/
  droite, `clear`, margin-collapse BFC. Jalon **Acid2**
  (`tests/acid2.rs` + `scripts/fetch_acid.ps1`) : cliquet
  robustesse (pas de panic) + alignement informatif
  **29,8 %** vs `reference.png` officielle. Sites réels :
  `explorer-cli shot` + `scripts/site_smoke.ps1` →
  **5/5** (example.com, info.cern.ch, duckduckgo, wikipedia,
  MDN 211 Ko) — le smoke a révélé puis corrigé un **panic
  tiny-skia** (`hairline_aa` sur bordure 1 px fractionnaire +
  géométrie hors domaine 24.8) : frontière défensive
  `fill_guarded`/`SAFE_COORD` dans `raster.rs`. Acid3 reste
  informatif côté équipe JS (test262 859/859). Prochaine
  étape déduite du rapport : `display: flow-root` +
  affinement des bands float (`float_band`).
- [x] **Étape 49. CSS2 floats/BFC/whitespace — WPT 17→27/40.**
  Étape déduite du rapport de l'étape 48. `explorer-css` :
  attribut `style` inline dans la cascade (priorité au-dessus
  de tout sélecteur auteur, `!important` inline > `!important`
  sélecteur), tolérance `<![CDATA[`/`]]>` dans le tokenizer
  (fixtures XHTML). `explorer-layout` : `FloatCtx` partagé
  par `&mut` (registre des flottants du BFC — `band`,
  `band_over`, `next_band_change`, `first_bottom_within`,
  `max_top` règle 5, `cleared`) ; les flottants se posent à
  leur position de flux (règle du « parent anonyme »),
  descendent tant que la bande ne les contient pas,
  n'avancent jamais le curseur ; `clear` ne déplace que la
  boîte concernée ; non-BFC pleine largeur (leurs lignes se
  rétrécissent dans la bande), BFC posée dans la bande
  intersectée sur toute sa hauteur (`layout_bfc_child`),
  containment des flottants §10.6.7 ; boxtree §9.7 — `float`
  force le niveau bloc. `inline` : bandes par ligne,
  marges horizontales **et** verticales des atomes
  (inline-block = BFC posé sur son étendue), whitespace
  collapsing CSS (newline → espace, `.notdef` Ahem évité,
  espaces en tête/fin de ligne supprimées — `trail_w`).
  `explorer-render` : anonymous/text boxes ne peignent plus
  les fond/bordure clonés (non hérités en CSS), pixel-snap
  des arêtes de remplissage plein (coutures AA supprimées
  + hairline fractionnaire saine). Score mesuré :
  **27/40 (67 %)**, cliquet `WPT_BASELINE` = 27 — reste :
  placement 004/006/007/008, `wrap-top-below-inline-001r`,
  bfc-overflow, zero-height, block-formatting 001/003/004/
  008/015.

## Hors périmètre assumé (documenté, non planifié)

Fonctionnalités d'un navigateur complet volontairement hors
roadmap — candidats à un ADR dédié si un besoin réel émerge :

- `<iframe>` et navigation multi-contextes, shadow DOM ;
- accessibilité native (arbre a11y, lecteurs d'écran) ;
- gestionnaire de téléchargements, impression, zoom page,
  recherche en page ;
- SVG animé (SMIL, `<animate>`…) — `<svg>` statique reste prévu
  en phase 10 (étape 45) ;
- extensions, DevTools complet (`explorer-cli` couvre le debug
  moteur) ;
- site isolation multi-processus (modèle de menace : ADR-0011).

