# Audit différentiel jscore ↔ Boa — inventaire des gaps

**Date** : 2026-11-17 — re-mesure après les évolutions du projet
(jscore ~120 k lignes, JS-C575). Première version de l'audit
(2026-11-04, baseline BOA-02/03) obsolète : la vague regexp
(JS-C43/C48/C36), les positions UTF-16 (JS-C263), `@@species`,
`Math.acosh`/`asinh`/`atanh`, les TypedArrays ont depuis été
livrés.

**Méthode** (ADR-0010 — jamais de % sans dénominateur) :
énumération par évaluation — `getOwnPropertyNames` +
`getOwnPropertySymbols` + `getOwnPropertyDescriptor` sur
`globalThis` → ctors → protos (profondeur 2), même script
(`_scratch/surface.js`) dans les deux moteurs.

- jscore : `explorer-cli eval` (moteur **nu** — les WebAPIs
  `install_*` non installées) — **22 736 chemins**
- boa 0.22-dev : `boa surface.js` — **5 399 chemins**
- communs : **4 796** ; boa-seuls : **591** ; jscore-seuls :
  **17 940** (WebAPIs + miroir DOM — attendu, boa n'a rien de
  comparable)

Sondes sémantiques : `_scratch/probe.js` (24 cas) exécuté dans
les deux moteurs — sorties `surf_*.txt`/`only_*.txt`/
`flagdiff.txt` dans `_scratch/`.

## Re-mesure finale (2026-11-17, après A1–A8 + retouches)

- jscore : **26 540 chemins** (+3 800 — `name` own, `@@species`,
  descripteurs matérialisés) ; boa inchangé 5 399.
- **boa-only : 591 → 176** — dont ~164 surfaces host non
  installées en eval nu (`fetch`, `Headers`, timers…, installées
  par `install_*` au runtime) + bruit du script (`fd`/`walk`/
  `out`). **Réels restants : 2 — tous deux des déviations boa** :
  `Error.prototype.stack` (accesseur V8-style non-spec : `stack`
  est une data propre d'*instance*, pas d'accesseur de proto) et
  `RegExp.prototype.lastIndex` (boa modèle le proto RegExp en
  exotique ES5 — ES6+ le rend ordinaire, sans `lastIndex` propre).
- **flagdiff : ~200 → 17** — tous spec-favorables à jscore :
  13× `URL.prototype.*`/`URLSearchParams.size` `011` (WebIDL veut
  des attributs **énumérables** — boa `001` se trompe) ; 3×
  `console` `111` (global navigateur énumérable) ; 1×
  `%TypedArray%.prototype[@@toStringTag]` `getset` (spec : get+set
  — boa n'a que get).
- **Bilan : zéro divergence sémantique restante sur les chemins
  communs** — tout écart mesuré est soit spec-correct côté jscore,
  soit une surface host volontairement non installée en eval nu.

---

## A. Gaps confirmés jscore (à porter)

### A1. `name` en propriété propre des fonctions natives — **328 chemins** ✅ *fait (2026-11-17)*

`native_fn` pose `length` (attributs spec : non-writable,
non-enum, configurable) mais **pas `name`** — le nom vit dans
`Fun::Native`, invisible à `getOwnPropertyNames`. `.name` se lit
quand même correctement (`[].map.name === 'map'`), seul le
*descripteur* manque → `hasOwnProperty('name')`/`getOwnPropertyDescriptor`
/`Object.keys` divergent.

**Fix ponctuel** : `set_own_attrs(id, "name", str_val(name), {w:false, e:false, c:true})` dans `native_fn` (+
`bound_native`) — une ligne, ~330 chemins soldés. Reste ensuite
l'**arité `length`** : tout est 0 ; la spec fixe des arités
réelles pour nombre de builtins (ex. `Array.prototype.push.length
=== 1`) — passe d'arité à prévoir par enregistrement.

### A2. `Ctor.prototype` verrouillé — **103 chemins** ✅ *fait (2026-11-17)*

Spec : la propriété `prototype` du constructeur est
`{writable:false, enumerable:false, configurable:false}`.
jscore l'expose `{writable:true, configurable:true}` →
`Array.prototype = {}` réussit chez nous, jette chez V8/JSC/boa.

Même classe pour les `Symbol.*` well-known (**30 chemins** :
`Symbol.iterator` etc. `{000}` attendu, `{101}` actuel) et
`Function.prototype.@@hasInstance` (`{000}` attendu, `{101}`).

### A3. `@@toStringTag` non-writable/non-enum — **26 chemins** ✅ *fait (2026-11-17)*

Spec `{writable:false, enumerable:false, configurable:true}` ;
jscore `{writable:true, enumerable:true, configurable:true}` —
**l'énumérable est le pire** (`Object.keys(Map.prototype)` le
renvoie). Touche Map/Set/Promise/ArrayBuffer/BigInt/DataView/
FinalizationRegistry/Intl/JSON/Math/Reflect/Temporal/Atomics/
Iterator/Symbol/SharedArrayBuffer/WeakMap…

### A4. `TypedArray` — le global abstrait n'est pas exposé — **49 chemins** ✅ *fait (2026-11-17)*

`%TypedArray%.prototype` existe en interne (`protos.typed_array`,
proto partagé des 9 protos) mais le **constructeur global
`TypedArray` manque** (`typeof TypedArray === 'undefined'`), avec
`TypedArray.from`/`of`/`@@species` et l'accès aux méthodes
partagées via le proto. Les détects réels : `Object.getPrototypeOf(new
Uint8Array().__proto__)`, `x instanceof TypedArray` écrit par les
libs.

### A5. `@@species` sur les constructeurs — **38 chemins** ✅ *fait (2026-11-17)*

`species.rs` existe et `Promise[Symbol.species]` est posé, mais
manquent : `Array`, `ArrayBuffer`, `Map`, `Set`, `RegExp`,
`SharedArrayBuffer`, `TypedArray` + les 9 concrets
(`Uint8Array`…). Accesseur `{get}` `{c:true}` — fix ponctuel
par constructeur.

### A6. Erreurs — `message` propre des protos natifs — **8 chemins** ✅ *fait (2026-11-17)*

Spec : chaque `XxxError.prototype` porte `message: ""` propre
(et `name`). jscore ne le pose pas sur `AggregateError`/
`EvalError`/`RangeError`/`ReferenceError`/`SyntaxError`/
`TypeError`/`URIError` (ni `name` sur ces protos — vérifier).

### A7. Lone surrogates — résidu UTF-16 (positions corrigées)

La représentation interne reste `String` UTF-8 → un surrogate
isolé devient `U+FFFD` (« déviation documentée » à
`string_from_char_code`). Manifestations mesurées :

| Sonde | jscore | spec/boa |
|---|---|---|
| `String.fromCharCode(0xD800).charCodeAt(0)` | 65533 | **55296** |
| `JSON.stringify('\uD800')` | `"�"` | `"\ud800"` |

`length`/indexation/`charCodeAt` des astraux sont déjà
UTF-16-corrects (JS-C263). Résoudre = représentation
`JsString`-style (latin1/UTF-16) — chantier à part entière,
bénéfice réel plus faible qu'en BOA-02.

### A8. Divers ponctuels ✅ *fait (2026-11-17)*

- `Function.prototype` : `typeof` doit être `'function'`
  (objet appelable) — jscore rend `'object'` ; flags `{000}`.
- `String.prototype.length` et `Array.prototype.length` :
  prop `length` own = 0 absente.
- `RegExp.prototype.compile` (Annex B dépréciée, sites legacy)
  et `RegExp.prototype.lastIndex` (own data prop du proto, boa
  l'a — jscore l'a déjà sur instances, correct).
- `Iterator.prototype.constructor` : spec = accesseur get/set —
  jscore l'expose en data prop. `Iterator.prototype.@@toStringTag`
  idem (accesseur).
- `Temporal.Now.zonedDateTimeISO`, `Date.prototype.@@toPrimitive`,
  `Symbol.prototype.@@toPrimitive` absents.
- `Array.prototype.@@unscopables` flags `{001}` (jscore `101`).
- Globals WebIDL (`URL`, `TextEncoder`, `TextDecoder`,
  `URLSearchParams`, ~24) : WebIDL veut `enumerable:true` sur le
  global — jscore `101` → `111`. **Attention** : pour les builtins
  ECMA (`Object`, `Array`…) c'est jscore `101` qui est **spec-correct**
  et boa `111` qui se trompe — ne corriger que la famille WebIDL.
- `console.exception`, `console @@toStringTag` (boa_runtime).

### A9. Fausses pistes écartées par la mesure

- **Fetch-family** (~154 chemins `fetch`/`Headers`/`Request`/
  `Response`/`AbortController`/`AbortSignal`/`TextEncoder`/
  `URL.createObjectUrl`/`setTimeout`…) : installés par
  `install_dom`/`install_xhr`/`install_timers`/`install_crypto` —
  absents du run **nu**, pas du moteur. Non-gaps.
- `Error.prototype.stack` : boa le porte en data prop proto ;
  jscore le pose own sur instances (comportement navigateur
  standard) — non-gap.
- `URL.prototype.*` accesseurs `{011}` vs boa `{001}` : WebIDL
  veut enumerable:true — **jscore correct**, boa fautif.
- `Promise.allKeyed`/`allSettledKeyed`/`try`/`withResolvers`,
  `Math.f16round`/`sumPrecise`, `Object.groupBy`,
  `Atomics.waitAsync`/`pause`, `String.raw`, `Reflect.ownKeys`,
  `Number.parseFloat` : **présents** (seul le `.name` A1 manque).

### Bugs boa mesurés (oracle discriminant)

| Sonde | boa | spec |
|---|---|---|
| `JSON.parse('"\\ud800"')` | jette SyntaxError | légal → `"\ud800"` |
| `'\u{1F600}'[0] === '\uD800'` | `false` | **`true`** |

(boa 0.22-dev — à ne pas « porter ».)

---

## B. Couvert par le moteur depuis l'ancien audit

Regexp (tous getters flags + `@@match`/`@@matchAll`/`@@replace`/
`@@search`/`@@split`, `exec`/`lastIndex`/`test`/`toString`),
`Math.acosh|asinh|atanh|cbrt|clz32|expm1|f16round|fround|hypot|
imul|log10|log1p|log2|sign|sinh|sumPrecise|tanh|trunc`,
`Object.*` complet, `Reflect.*` complet, `Atomics.*` complet,
`Promise.*` (incl. `try`/`withResolvers`/`allKeyed`),
`Intl.*` (7 ctors + getCanonicalLocales), `Temporal.*` (9 protos),
positions/indexation UTF-16, `TypedArray` concrets ×9,
crypto.subtle étendu, streams async, `DisposableStack`/
`SuppressedError` (TC39 — **devance boa**).

## C. Ce que l'audit ne mesure pas

- Sémantique **d'exécution** (coercions, IteratorClose sur les
  chemins, trous vs undefined dans les algos, conversion nombre↔
  chaîne en profondeur) — la surface énumère l'existence, pas le
  comportement. L'oracle évaluatif (`Outcome` + corpus de sondes)
  reste l'outil pour ça — son squelette a disparu avec le reset
  git ; à recréer si on reprend le portage.
- `name`/`length` sur **méthodes** des protos WebAPI
  install-context (XHR, IDB…) — non couverts par le run nu.
- test262 complet (le corpus extrait reste lexer/scanner :
  **2045/2045 passes-mode** post-refonte, 31 skips — harness
  reconstruit 2026-11-17 : frontmatter YAML, double passe
  strict/sloppy, `async`/`module`, `negative.type` typé,
  cliquet par répertoire).

---

## Synthèse — file de portage restante

| Rang | Gap | Effort | Impact | État |
|---|---|---|---|---|
| 1 | A1 `name` own-prop (fix `native_fn`) | **XS** | ~330 chemins | ✅ fait |
| 2 | A2 verrous `Ctor.prototype`/`Symbol.*`/`@@hasInstance` | S | ~135 chemins | ✅ fait |
| 3 | A3 `@@toStringTag` flags | S | ~26 chemins | ✅ fait |
| 4 | A5 `@@species` par ctor | S | ~38 chemins | ✅ fait |
| 5 | A4 exposer `TypedArray` global + statiques | M | ~49 chemins | ✅ fait |
| 6 | A6 `message` protos erreurs | XS | 8 chemins | ✅ fait |
| 7 | A8 divers (Iterator ctor accessor, `Function.prototype` callable, proto `length`, Annex B, WebIDL enum) | S–M | ~15 chemins | ✅ fait |
| 8 | A7 lone surrogates (repr JsString) | **L** | JSON/`.charCodeAt` — chantier représentation | ouvert — design choisi, cf. § ci-dessous |
| 9 | Oracle différentiel (BOA-01..04 à recréer) | M | mesure continue | ✅ reconstruit (`explorer-boa-diff`) |
| 10 | Harness test262 | M | corpus élargi | ✅ reconstruit (sans oracle boa) |
| 11 | Arités `length` des builtins (toutes 0 vs spec) | M | `fn.length === 1` checks | ✅ fait — table `arities.rs` générée depuis boa (190 chemins), 0 flagdiff |
| 12 | `Number(undefined)` → 0 au lieu de NaN | XS | conversion | ✅ corrigé (arité, pas valeur) |
| 13 | `(0.5).toFixed(0)` → 0 au lieu de 1 | XS | demi vers le haut | ✅ corrigé (`floor(v+.5)` + `1e21`) |

Items 1–7 soldés : **~600 chemins de gap descripteur fermés**
(8 commits, `0e7731e`→`552199d`). Item 9 livré ci-dessous ; la
mesure de l'oracle a immédiatement produit les items 12–13
(preuve d'utilité). Reste : 11 (quick win) → 8 (si nécessité
site réel — seul chantier structurel).

---

## Oracle différentiel `explorer-boa-diff` (reconstruit)

Crate de conformité, **jamais en production** : `boa_engine` en
path-dep sur le checkout local (`D:/Projet/SourceRust/boa`),
features par défaut sans `intl`.

- **`eval_jscore` / `eval_boa`** → `Outcome` canonique
  (`Value(ToString)` / `Throw(Nom: msg)` / `Syntax` / `Internal`) —
  budgets boa bornés (`loop_iteration = 2M`, piles par défaut).
- **`concordant`** : même variante ; `Value` exact ; `Throw` même
  classe (le message n'est pas spec) ; `Syntax` symétrique.
- **`boa-diff`** : corpus `src/probes.rs` (~120 sondes —
  littéraux, conversions, UTF-16, tableaux, descripteurs,
  classes, erreurs, regexp, itérateurs, Map/Set, JSON, Math,
  Proxy, Symbol, promesses sync, TypedArrays, divers langage).
  **Isolation par processus** : chaque sonde tourne dans un
  enfant `--worker` — un crash natif d'un moteur (observé :
  récursion boa 0.22-dev sur `for-of`+`break`) devient
  `Internal(crash)` au lieu de tuer le runner.
- **`boa-surface`** : matrice d'énumération (`surface.js` —
  `getOwnPropertyNames`/`Symbols`/descripteurs, profondeur 2)
  comparée chemin par chemin.
- **Cliquet** `tests/ratchet.rs` : rejoue le corpus en test ;
  divergences connues listeées par moteur fautif — toute
  nouvelle ou résolue échoue.

### Mesure de référence (2026-11-17, boa 0.22-dev)

Corpus : **191 concordants / 3 divergences / 0 internal** —
toutes documentées dans `KNOWN` (items 12–13 corrigés au
passage — l'oracle a produit ses deux premiers fixes) :

| Sonde | Fautif | Détail |
|---|---|---|
| `str_fromcc`, `str_lone_surrogate` | jscore | lone surrogate → U+FFFD (item 8) |
| `str_astral_idx` | boa | `'😀'[0] !== '\uD800'` — bug boa, jscore spec-correct |

Surface : **26 441 chemins jscore / 4 203 boa**, `boa-only`
réduit à 2 déviations boa (`Error.prototype.stack` accesseur
non-spec, `RegExp.prototype.lastIndex` modèle ES5) + bruit du
script ; **0 flagdiff** après correction du setter
`%TypedArray%.prototype[@@toStringTag]` (spec : get seul).
Arités : table `builtins/arities.rs` de 190 chemins générée par
`boa-surface --dump boa`, posée par `install_arities` après
tous les installs.

---

## A7 — lone surrogates : design choisi (avant implémentation)

**Contrainte** : `JsStr` enrobe `StrBox { s: String }` — UTF-8
Rust valide. Un surrogate isolé ne peut pas être stocké tel
quel (séquence invalide ; la voie WTF-8/CESU-8 exige `unsafe`,
interdit par `#![forbid(unsafe_code)]`).

**Voies écartées** :
- UTF-16 interne (`Vec<u16>`) : casse `as_str()`/`Deref<str>`/
  `try_append` utilisés dans tout le moteur — refactor majeur.
- Placeholder PUA (U+E000–U+E7FF ↔ D800–DFFF) : collision
  réelle avec les icon-fonts qui utilisent la zone private-use.

**Design retenu — double représentation paresseuse** :
`StrBox { s: String, units: Option<Vec<u16>> }` :
- `s` garde le rendu lossy (U+FFFD) pour affichage/DOM/`as_str()`.
- `units` porte la vue UTF-16 **exacte** quand la chaîne vient
  d'une source à surrogates (fromCharCode, JSON `\uD800`,
  slice coupant une paire).
- `u16_len`/`u16_at`/`slice_u16`/`index_of` lisent `units` si
  présent (sinon `encode_utf16` comme aujourd'hui).
- `PartialEq` : `s == s` rapide ; si l'un a `units`, comparer
  les vues u16 (`'a\uD800' !== 'a\uFFFD'`).
- `try_append` : concatène `s` et fusionne `units` si présents.

**Points d'entrée à rebrancher** : `String.fromCharCode`,
`charCodeAt`/`codePointAt`/`at`/`[]`, `slice_u16` (produit des
units → nouvelle JsStr), `JSON.parse` (`\u` escapes — `json.rs:92`),
`JSON.stringify` (ré-échappe surrogate isolé en `\uXXXX` au lieu
de FFFD), lexer `\uXXXX` (`lexer.rs:1329`), regex mode `u`,
`String.prototype.normalize`, `str_val` générique si source u16.

**Corpus de verrous** : `str_fromcc`, `str_lone_surrogate`
(`probes.rs` — déjà divergents, KNOWN), + `JSON.stringify` de
surrogate, `.length` de `'a\uD800'` (= 2), égalité vs U+FFFD.
