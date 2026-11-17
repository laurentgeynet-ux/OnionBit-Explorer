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

### A7. Lone surrogates ✅ *fait (JS-C579)*

La représentation interne reste `String` UTF-8, **doublée** :
`StrBox { s, units: Option<Vec<u16>> }` — `s` = rendu lossy
(U+FFFD) pour affichage/`as_str`, `units` = vue UTF-16 exacte
quand la chaîne porte un surrogate isolé. Tous les chemins
spec lisent les units : `charCodeAt`/`codePointAt`/`at`/`[]`,
`length`, slices, concat `+`/`concat`, indexation, égalité,
matching (`indexOf`/`includes`/`starts`/`ends`/`replace`),
itération (`@@iterator`, spread `[...s]`, `with`),
`split('')`, `pad*`/`repeat`/`trim*`/`toUpper`/`toLower`/
`normalize`, `JSON.parse`/`stringify`, comparaison `<` (ordre
**UTF-16**, pas code point : `'\uFFFF' < '\u{10000}'` = false),
`fromCharCode`/`fromCodePoint`, `getOwnPropertyDescriptor`.
Lexer : escape `\uXXXX` isolé → sentinelle `U+F0000+` décodée
par `str_val_lit` ; chars réels de la zone doublés par `F0800`
(collision fermée). **Mesure : corpus oracle 233/233
concordants, 0 divergence.** Design détaillé : § « A7 —
design choisi » plus bas.

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
| 8 | A7 lone surrogates (repr JsString) | **L** | JSON/`.charCodeAt` — chantier représentation | ✅ fait — `units` paresseux + sentinelle lexer (JS-C579) |
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
- **`boa-diff`** : corpus `src/probes.rs` (~185 sondes —
  littéraux, conversions, UTF-16, tableaux, descripteurs,
  classes, erreurs, regexp, itérateurs, Map/Set, JSON, Math,
  Proxy, Symbol, promesses sync, TypedArrays, divers langage,
  énumération, sort, BigInt, reviver, destructuring).
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

### Mesure de référence (JS-C584, boa 0.22-dev + `annex-b`)

Corpus : **647 concordants / 0 divergences / 0 internal** —
`KNOWN` **vide**. Les 9 divergences historiques ont été soldées
plutôt que documentées : (1) `re_source_str` était en fait un
**bug jscore** — `.source` rendait le pattern verbatim sans
`EscapeRegExpPattern` (spec : `/` → `\/`, fins de ligne échappées,
`\` verbatim ; `/a\/b/.source` = `a\\/b`) — corrigé par
`escaped_source` ; (2) `str_substr_sub` — `substr` (Annex B.2.2)
existait dans le source boa derrière `#[cfg(feature="annex-b")]` :
feature activée sur la path-dep, déviation soldée sans toucher
boa ; (3) les 7 sondes host-API (`structuredClone`×4, `Intl`×2,
`Error.captureStackTrace`) concordent via **`BOA_PRELUDE`** —
implémentations de référence spec-following évaluées dans chaque
contexte boa (clone structuré réel avec table de cycles, Intl
minimal sur la surface probée) : le shim sert d'oracle quand boa
n'en est pas un. Bugs boa mesurés hors corpus : `JSON.parse`
imbriqué → stack overflow natif dès ~100 niveaux ; **parseur
récursif** qui stack-overflow sur les `else if` chaînés imbriqués
(prelude écrit en `if` séquentiels plats).

Quatre vagues d'élargissement ont mesuré **20 bugs jscore réels**,
tous corrigés (+9 déviations ci-dessus + 1 crash boa documenté) :

- JS-C580 (~65 sondes) : `Number('0b11'/'0o17')` (préfixes
  binaire/octal absents du String→Number), `Math.sign(-0)` →
  `+1` (`f64::signum` perdait le signe du zéro), **4 divergences
  regex surrogates** (moteur `fancy-regex` sur le `s` lossy :
  `/\uD800/` ne matchait pas un surrogate isolé, `/\uFFFD/` le
  matchait à tort) → **domaine sentinelle regex** (§ ci-dessous).
- JS-C581 (~45 sondes, Proxy/Reflect/générateurs/URI/
  destructuring) : **`gen.return(v)` ignorait les `finally`**
  (`ret_at` forçait `done` — spec `GeneratorResumeAbrupt` :
  déroulement par flag `ret_unwind`, catch sautée, `yield` en
  finally ré-émet) ; **`class extends null`/`undefined`
  rejeté** en TypeError (spec : proto null/Object.proto,
  ctorParent `%Function.prototype%`) et `super.x` sur base
  null → `ReferenceError` ; **`range_error`/`uri_error` sur
  `protos.error`** (`constructor.name` → "Error" — protos
  corrigés) ; **`encodeURIComponent(surrogate)`** encodait le
  FFFD au lieu de `URIError` ; **`array_includes` ignorant
  `fromIndex`**.
- JS-C582 (~100 sondes, async/Atomics/DataView/TA/JSON/dates/
  Reflect/destructuring) : **`delete a?.b.c` ne supprimait pas**
  (l'opérande `Expr::Chain` tombait sur le bras `_` → éval +
  `true` — nouveau `delete_chain` marche jusqu'à la base du
  dernier pas, court-circuit nullish → `true`) ; **`eval` strict
  fuyait les `var`** (spec `PerformEval` : env déclaratif enfant
  quand `prog.strict` — `exec_program`/`eval_source_in`
  paramétrés).
- JS-C583 (~160 sondes, for-await/itérateurs ES2025/Set/Temporal/
  `__proto__`/computed keys/for-in/`arguments`/`Function.toString`/
  lookbehind/`toExponential`/getters hérités) : **`for-in` sur
  `String`** ignorait les primitives (index `"0"`,`"1"`… non
  énumérés) ; **`arguments.callee` strict** retournait la fonction
  au lieu du `%ThrowTypeError%` empoisonné ; **`trim*` perdait
  `U+FEFF`** (le fast-path `str::trim` Rust ignore `FEFF` —
  whitespace JS ≠ `White_Space` Unicode → predicate `js_ws`) ;
  **`lastIndexOf` ignorait `fromIndex` négatif** ; **`catch(e)
  {let e}`** échouait en `ReferenceError` runtime au lieu du
  `SyntaxError` early (param ∩ déclarations du bloc) ; **clés
  `WeakMap`/`WeakSet` primitives acceptées** (`map_set` partagé
  sans check `Value::Obj`).

Surface : **26 441 chemins jscore / 4 203 boa**, `boa-only`
réduit à 2 déviations boa (`Error.prototype.stack` accesseur
non-spec, `RegExp.prototype.lastIndex` modèle ES5) + bruit du
script ; **0 flagdiff** après correction du setter
`%TypedArray%.prototype[@@toStringTag]` (spec : get seul).
Arités : table `builtins/arities.rs` de 190 chemins générée par
`boa-surface --dump boa`, posée par `install_arities` après
tous les installs.

---

## A7 — lone surrogates : design **implémenté** (JS-C579)

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

**Points d'entrée rebranchés** : `String.fromCharCode`,
`charCodeAt`/`codePointAt`/`at`/`[]`/`indexOf`/`includes`/
`startsWith`/`endsWith`/`replace` (match exact en units —
`U+FFFD` n'égale pas un surrogate), `slice`/`substring`/`substr`,
concat `+`/`concat`, itération code point (`@@iterator`, spread,
`with`), `split('')`, `pad*`/`repeat`/`trim*`/`toUpper`/`toLower`,
`normalize` (par runs de scalaires, surrogate = borne), comparaison
`<`/`>` (ordre UTF-16 via `cmp_js`), `getOwnPropertyDescriptor`,
`JSON.parse`/`stringify` (isolé → `\uXXXX`), lexer `\uXXXX` +
sentinelle `U+F0000..F07FF` (zone réelle échappée par `F0800`),
`str_val_lit`/`str_val_units`, `to_js_jstr`/`this_jstr`
(conservation du `JsStr` au lieu du `String` lossy).

**Corpus de verrous** : 26 sondes `str_surr_*`/`str_fffd_*`/
`json_surr_*`/`lit_sentinel_zone`/`str_u16_order` dans
`probes.rs` — **toutes concordantes** (cliquet `ratchet.rs`,
`KNOWN` vide).

## Regex — domaine sentinelle (JS-C580)

`fancy-regex` opère sur `&str` UTF-8 : un surrogate isolé n'y
existe pas — le `s` lossy rendait `\uD800` invisible et `\uFFFD`
collisionnait. Même schéma sentinelle que le lexer, appliqué
au **pattern et au haystack** quand l'un des deux en porte :

- `re_encode(units)` : paires astrales fusionnées (`decode_utf16`),
  surrogate isolé → `U+F0000+off`, char réel `F0000..=F0800`
  doublé par `F0800`. `re_decode` inverse pour les textes
  produits (match, groupes, pièces de split, splice).
- `JsRegExp::enc_hay(JsStr)` : encodage si `has_sentinels`
  (pattern) **ou** `has_units` (sujet) — sinon passthrough,
  aucun coût sur le chemin commun.
- `translate_pattern` émet les sentinelles **au lieu** des
  `(?!)`/clamps historiques : atome isolé → `\x{f00xx}` +
  (non-`u`) astrals dont le lead est l'atome (`[\x{f003d}
  \x{1f400}-\x{1f7ff}]` — sémantique units : un lead couvre
  l'astral qu'il préfixe) ; classe `[\uD800-\uDFFF]` → union
  `[\x{f0000}-\x{f07ff}\x{10000}-\x{10ffff}]` ; paires fusionnées
  et idiom `\uD8hi[\uDClo]` inchangés.
- `builtins` : `s: &str` → `&JsStr` dans `re_match_exec`/
  `re_match_all_impl`/`re_replace_impl`/`re_sym_*`/`regexp_exec`/
  `string_match`/`search`/`split`/`replace`/`matchAll` —
  `to_js_jstr`/`this_jstr` préservent les units ;
  `new RegExp('\uD800')` encodé via `make_regexp_jstr`.
- `has_sentinels` = char `F0000..=F0800` dans le pattern traduit.

**Limites résiduelles du modèle char** (déjà celles du moteur :
un élément ≈ 1 unit) : un **trail** atom (`/\uDCA5/`) ne peut
pas matcher la moitié basse d'un astral (non-`u` seul affecté),
les indices `d`/`lastIndex` comptent en chars, `.source` d'un
regex construit depuis un `JsStr` à units rend la sentinelle.
