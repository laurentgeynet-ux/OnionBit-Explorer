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
  1046/1046, 31 skips).

---

## Synthèse — file de portage restante

| Rang | Gap | Effort | Impact |
|---|---|---|---|
| 1 | A1 `name` own-prop (fix `native_fn`) | **XS** | ~330 chemins, test262 `name` checks |
| 2 | A2 verrous `Ctor.prototype`/`Symbol.*`/`@@hasInstance` | S | ~135 chemins, invariants |
| 3 | A3 `@@toStringTag` flags | S | ~26 chemins |
| 4 | A5 `@@species` par ctor | S | ~38 chemins, subclassing |
| 5 | A4 exposer `TypedArray` global + statiques | M | ~49 chemins, `instanceof` libs |
| 6 | A6 `message` protos erreurs | XS | 8 chemins |
| 7 | A8 divers (Iterator ctor accessor, `Function.prototype` callable, proto `length`, Annex B, WebIDL enum) | S–M | ~15 chemins + typeof |
| 8 | A7 lone surrogates (repr JsString) | **L** | JSON/`.charCodeAt` — chantier représentation |
| 9 | Oracle différentiel (BOA-01..04 à recréer) | M | mesure continue |
| 10 | Harness test262 (BOA-10..15 à recréer) | M | corpus élargi |

Items 1–7 = **~600 chemins de gap réel**, la plupart par fixes
systémiques en un point chacun — c'est le « low-hanging fruit »
avant tout chantier de représentation. L'ordre suggéré : 1→7
(quick wins mesurables) → 9 (oracle pour piloter) → 8 (si
nécessité site réel).
