// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Builtins JS — section extraite de `builtins.rs` (JS-C40, pur
//! reformatage mécanique : sémantique et tests inchangés).

#[allow(clippy::wildcard_imports)]
use super::*;

// ------------------------------------------------------------------
// JS-C15 — `Array.of`, dispatch `@@*`, `@@species`, captureStackTrace
// ------------------------------------------------------------------

/// `Array.of(…items)` — spec : si `this` est un ctor → `new this(len)`
/// (sous-classes `Array.of.call(C, …)`), sinon un Array ordinaire.
pub(crate) fn array_of(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let len = args.len();
    // Spec : `IsConstructor(this)` → `A = Construct(this, «len»)`,
    // sinon `ArrayCreate`. Éléments via `CreateDataPropertyOrThrow`
    // — les setters/accessors de la chaîne proto ne sont jamais
    // sollicités (`does-not-use-prototype-properties`).
    let a = if e.is_constructor(this.clone()) {
        e.construct(this, &[Value::Num(len as f64)])?
    } else {
        e.new_array(vec![Value::Undefined; len])
    };
    for (i, v) in args.iter().enumerate() {
        let ok = match a {
            Value::Obj(aid) => e.create_data_prop_spec_on(aid, &i.to_string(), v.clone())?,
            _ => true,
        };
        if !ok {
            return Err(e.type_error("Array.of: element is not creatable"));
        }
    }
    e.set_prop_value(a.clone(), "length", Value::Num(len as f64))?;
    Ok(a)
}

/// `GetMethod(obj, sym)` — dispatch spec `str.x(rx)` → `rx[@@x](…)`.
/// `Ok(None)` → chemin builtin (`sym` absent, `undefined` ou `null`).
pub(crate) fn sym_method(
    e: &mut Engine,
    obj: Value,
    sym: &str,
    args: &[Value],
) -> JsResult<Option<Value>> {
    if !e.is_spec_object(&obj) {
        return Ok(None);
    }
    // `GetMethod` (spec, JS-C618) : `Get` complet — accesseurs
    // invoqués et abrupts propagés (`get_prop` brut lisait la
    // donnée sans déclencher les getters → throws avalés).
    match e.get_prop_value(obj.clone(), sym)? {
        Value::Undefined | Value::Null => Ok(None),
        m if e.is_callable(m.clone()) => e.call_value(m, obj, args).map(Some),
        _ => Err(e.type_error("symbol method is not callable")),
    }
}

/// `get [Symbol.species]` (spec) — `return this`. Accesseur
/// `{e:false, c:true}` posé par [`install_species`] sur chaque
/// ctor species-aware (audit boa A5).
pub(crate) fn species_get(_e: &mut Engine, this: Value, _args: &[Value]) -> JsResult<Value> {
    Ok(this)
}

/// Pose `get [Symbol.species]` sur le ctor `ctor` (`Array`,
/// `ArrayBuffer`, `Map`, `Set`, `RegExp`, `SharedArrayBuffer`,
/// `TypedArray` + ses concrets — spec).
pub(crate) fn install_species(e: &mut Engine, ctor: &Value) {
    if let Value::Obj(cid) = ctor {
        let sg = e.native_fn("get [Symbol.species]", species_get);
        if let Some(o) = e.heap.obj_mut(*cid) {
            o.set_accessor(crate::interp::SYM_SPECIES, sg, Value::Undefined, false);
        }
    }
}

/// `ArraySpeciesCreate(src, len)` (spec) : `new src.constructor
/// [@@species](len)` ; `undefined`/`null` → tableau ordinaire.
/// Lectures **observables** (`Get`) — `constructor` et `@@species`
/// invoquent leurs accesseurs (JS-C615 ; avant `get_prop`
/// data-only, les getters étaient invisibles).
///
/// Démotion cross-realm (spec étape 6.c) : si `C` est un ctor
/// d'un **autre** realm identique à **son** `%Array%`, `C` est
/// ramené à `undefined` — `@@species` n'est alors jamais lu
/// (`create-proto-from-ctor-realm-array` : `callCount` reste 0).
/// Déviation cataloguée : `IsArray` ne traverse pas les proxies.
pub(crate) fn species_array(e: &mut Engine, src: Value, len: usize) -> JsResult<Value> {
    // `ArrayCreate(len)` — le repli valide la borne spec
    // (`len > 2³²−1` → `RangeError`) et pose `length` ; la
    // représentation sparse ne densifie rien.
    let array_create = |e: &mut Engine| -> JsResult<Value> {
        if len > u32::MAX as usize {
            return Err(e.range_error("Invalid array length"));
        }
        Ok(e.new_array_len(len as u32))
    };
    let Value::Obj(id) = src else {
        return array_create(e);
    };
    if !matches!(e.heap.obj(id).map(|o| &o.kind), Some(ObjKind::Array(_))) {
        return array_create(e);
    }
    let mut c = e.get_prop_value(src.clone(), "constructor")?;
    if e.is_constructor(c.clone()) {
        if let Some(rc) = e.fn_realm(&c) {
            if rc != e.cur_ctx && e.realm_global_own(rc, "Array").is_some_and(|a| a == c) {
                c = Value::Undefined;
            }
        }
    }
    if e.is_spec_object(&c) {
        match e.get_prop_value(c, crate::interp::SYM_SPECIES)? {
            Value::Undefined | Value::Null => {}
            s => return e.construct(s, &[Value::Num(len as f64)]),
        }
    } else if !matches!(c, Value::Undefined) {
        // Spec étape 7 : `C` non-objet et ≠ undefined → TypeError
        // (`flatMap` `this-value-ctor-non-object`).
        return Err(e.type_error("constructor is not an object"));
    }
    array_create(e)
}

// ------------------------------------------------------------------
// JS-C618 — `RegExp.prototype[@@match]/[@@matchAll]/[@@replace]/
// [@@search]` spec : boucles `RegExpExec` dynamiques (exec custom
// et `lastIndex` observables) au lieu des stubs natifs `exec_all`.
// ------------------------------------------------------------------

/// `Set(O, P, V, Throw=true)` (spec) — `[[Set]]` rend le
/// booléen spec (`set_prop_recv`) ; un refus (prop non-writable,
/// trap proxy `false`) lève `TypeError` au lieu du silence
/// sloppy — sans lui un `lastIndex` gelé boucle à l'infini.
pub(crate) fn set_strict(e: &mut Engine, base: Value, key: &str, v: Value) -> JsResult<()> {
    if let Value::Obj(id) = base.clone() {
        if !e.set_prop_recv(id, base, key, v)? {
            return Err(e.type_error(format!("cannot assign to `{key}`")));
        }
        Ok(())
    } else {
        e.set_prop_value(base, key, v)
    }
}

/// `SameValue` spec — NaN ~ NaN mais `-0 ≠ +0` (le `==` de
/// Rust les confond ; `to_bits` porte le signe). Employé sur
/// `lastIndex` (@@search compare avant de restaurer).
fn same_value(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Num(x), Value::Num(y)) => x.to_bits() == y.to_bits() || (x.is_nan() && y.is_nan()),
        _ => a == b,
    }
}

/// `SpeciesConstructor(rx, %RegExp%)` — `constructor` puis
/// `@@species` lus par `Get` (accesseurs observables) ; défaut
/// `%RegExp%` du realm courant (JS-C617/618 partagé).
fn species_ctor_regexp(e: &mut Engine, this: &Value) -> JsResult<Value> {
    let c = e.get_prop_value(this.clone(), "constructor")?;
    if matches!(c, Value::Undefined) {
        return regexp_intrinsic(e);
    }
    if !e.is_spec_object(&c) {
        return Err(e.type_error("constructor is not an object"));
    }
    match e.get_prop_value(c, crate::interp::SYM_SPECIES)? {
        Value::Undefined | Value::Null => regexp_intrinsic(e),
        sc if e.is_constructor(sc.clone()) => Ok(sc),
        _ => Err(e.type_error("@@species is not a constructor")),
    }
}

/// `RegExpCreate(pattern, flags)` — `Construct(%RegExp%, «p, f»)`
/// du realm courant — repli `String.prototype.*` (JS-C618).
pub(crate) fn regexp_create(e: &mut Engine, pattern: Value, flags: Value) -> JsResult<Value> {
    let ctor = regexp_intrinsic(e)?;
    e.construct(ctor, &[pattern, flags])
}

/// `IsRegExp(v)` (spec) — `Symbol.match` observable ; à défaut
/// le slot `[[RegExpMatcher]]` tranche.
pub(crate) fn is_regexp_spec(e: &mut Engine, v: Value) -> JsResult<bool> {
    if !e.is_spec_object(&v) {
        return Ok(false);
    }
    let m = e.get_prop_value(v.clone(), crate::interp::SYM_MATCH)?;
    if !matches!(m, Value::Undefined) {
        return Ok(e.to_boolean(m));
    }
    Ok(matches!(v, Value::Obj(id)
        if matches!(e.heap.obj(id).map(|o| &o.kind), Some(ObjKind::RegExp(_)))))
}

/// `JsStr` → domaine encodé du haystack regex (inverse de
/// `re_str_val` : les `units` redeviennent sentinelles).
fn enc_js(s: &JsStr) -> String {
    if s.has_units() {
        crate::regexp::re_encode(&s.units_view())
    } else {
        s.as_str().to_string()
    }
}

/// `ToString(v)` → domaine encodé (cf. [`enc_js`]).
fn enc_val(e: &mut Engine, v: Value) -> JsResult<String> {
    Ok(enc_js(&e.to_js_jstr(v)?))
}

/// `GetSubstitution(matched, S, position, captures, named,
/// template)` (spec 22.1.3.19.1, JS-C618) — tout est en domaine
/// encodé (chars) ; `captures` déjà `ToString`ées à l'appel.
/// `named` = objet `groups` (`ToObject`é) ou `undefined`.
/// `$$` `$&` `` $` `` `$'` `$n`/`$nn` `$<name>` — un `$` non
/// résolu est émis littéralement (spec `refReplacement = ref`).
#[allow(clippy::too_many_arguments)]
fn get_substitution(
    e: &mut Engine,
    matched: &str,
    hay: &[char],
    position: usize,
    captures: &[Value],
    named: &Value,
    rep: &str,
) -> JsResult<String> {
    let size = hay.len();
    let rc: Vec<char> = rep.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < rc.len() {
        if rc[i] == '$' && i + 1 < rc.len() {
            match rc[i + 1] {
                '$' => {
                    out.push('$');
                    i += 2;
                    continue;
                }
                '&' => {
                    out.push_str(matched);
                    i += 2;
                    continue;
                }
                '`' => {
                    let t: String = hay[..position].iter().collect();
                    out.push_str(&t);
                    i += 2;
                    continue;
                }
                '\'' => {
                    // `tailPosition` peut dépasser `size` avec un
                    // exec custom (spec : borné à stringLength).
                    let tail = (position + matched.chars().count()).min(size);
                    let t: String = hay[tail..].iter().collect();
                    out.push_str(&t);
                    i += 2;
                    continue;
                }
                '<' => {
                    // `$<name>` — `named` requis (sinon ref
                    // littéral) ; la valeur est lue par `Get`
                    // (accesseurs observables) puis `ToString`.
                    if let Some(end) = rc[i + 2..].iter().position(|&c| c == '>') {
                        if e.is_spec_object(named) {
                            let name: String = rc[i + 2..i + 2 + end].iter().collect();
                            let g = e.get_prop_value(named.clone(), &name)?;
                            if !matches!(g, Value::Undefined) {
                                out.push_str(&enc_val(e, g)?);
                            }
                            i += end + 3;
                            continue;
                        }
                    }
                }
                c if c.is_ascii_digit() => {
                    let d1 = rc[i + 1].to_digit(10).unwrap_or(0) as usize;
                    let d2 = rc
                        .get(i + 2)
                        .and_then(|c| c.to_digit(10))
                        .map(|d| d as usize);
                    // `$nn` compte si n ≤ nb captures ; sinon
                    // repli `$n` suivi d'un chiffre littéral.
                    let (n, step) = match d2 {
                        Some(d2) if d1 * 10 + d2 >= 1 && d1 * 10 + d2 <= captures.len() => {
                            (d1 * 10 + d2, 3)
                        }
                        _ => (d1, 2),
                    };
                    if n >= 1 && n <= captures.len() {
                        if let Value::Str(cs) = &captures[n - 1] {
                            out.push_str(&enc_js(cs));
                        }
                        i += step;
                        continue;
                    }
                }
                _ => {}
            }
        }
        out.push(rc[i]);
        i += 1;
    }
    Ok(out)
}

/// `rx[Symbol.match](str)` — spec 22.2.6.8 (JS-C618) : `flags`
/// lu par `Get` ; sans `g` → un seul `RegExpExec` ; avec `g` →
/// `lastIndex` = 0 puis boucle `RegExpExec` collectant
/// `Get(result,"0")`, `AdvanceStringIndex` sur match vide,
/// `null` si aucun match.
pub(crate) fn re_sym_match(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    if !e.is_spec_object(&this) {
        return Err(e.type_error("Symbol.match receiver is not an object"));
    }
    let s = e.to_js_jstr(arg(args, 0))?;
    // `flags` lu par `Get`+`ToString` (observable) ; `global` et
    // `unicode` lus comme **propriétés** (algo corpus — un
    // getter ou shadow `global` est observé).
    let flags_v = e.get_prop_value(this.clone(), "flags")?;
    let _flags = e.to_js_string(flags_v)?;
    let global = {
        let g = e.get_prop_value(this.clone(), "global")?;
        e.to_boolean(g)
    };
    let unicode = {
        let u = e.get_prop_value(this.clone(), "unicode")?;
        e.to_boolean(u)
    };
    if !global {
        return reg_exp_exec(e, this, Value::Str(s));
    }
    set_strict(e, this.clone(), "lastIndex", Value::Num(0.0))?;
    let a = e.new_array(Vec::new());
    let Value::Obj(aid) = a.clone() else {
        return Ok(a);
    };
    let mut n = 0usize;
    loop {
        let result = reg_exp_exec(e, this.clone(), Value::Str(s.clone()))?;
        if matches!(result, Value::Null) {
            return Ok(if n == 0 { Value::Null } else { a });
        }
        let m0 = e.get_prop_value(result, "0")?;
        // `matchStr` reste un `JsStr` (units préservées) — le
        // passage par `String` remplaçait les surrogates isolés
        // par U+FFFD (non-`u` : `.` matche **1 code unit**, cf.
        // `match/regexp-prototype-match-v-u-flag`).
        let match_str = e.to_js_jstr(m0)?;
        if !e.create_data_prop_spec_on(aid, &n.to_string(), Value::Str(match_str.clone()))? {
            return Err(e.type_error("match: element is not creatable"));
        }
        if match_str.is_empty() {
            // `AdvanceStringIndex` sur `lastIndex` relu (spec
            // `ToLength`) — +2 units sous `unicode` sur paire.
            let idx_v = e.get_prop_value(this.clone(), "lastIndex")?;
            let idx = e
                .to_number(idx_v)?
                .trunc()
                .clamp(0.0, 9_007_199_254_740_991.0);
            let next = advance_str_index(&s, idx, unicode);
            set_strict(e, this.clone(), "lastIndex", Value::Num(next))?;
        }
        n += 1;
    }
}

/// `AdvanceStringIndex(S, index, unicode)` (spec) — `index` en
/// code units UTF-16 ; sous `unicode`, une paire surrogate est
/// franchie d'un coup (+2 units).
pub(crate) fn advance_str_index(s: &JsStr, index: f64, unicode: bool) -> f64 {
    let i = index as usize;
    if unicode {
        let u = s.units_view();
        if i + 1 < u.len()
            && (0xD800..0xDC00).contains(&u[i])
            && (0xDC00..0xE000).contains(&u[i + 1])
        {
            return index + 2.0;
        }
    }
    index + 1.0
}

/// `rx[Symbol.matchAll](str)` — spec 22.2.6.9 (JS-C618) :
/// `SpeciesConstructor` → `Construct(C, «rx, flags»)` →
/// `lastIndex` copié → `%RegExpStringIterator%` paresseux
/// (`IterOp::ReMatchAll` — exec dynamique à chaque `next`).
pub(crate) fn re_sym_match_all(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    if !e.is_spec_object(&this) {
        return Err(e.type_error("Symbol.matchAll receiver is not an object"));
    }
    let s = e.to_js_jstr(arg(args, 0))?;
    let ctor = species_ctor_regexp(e, &this)?;
    let flags_v = e.get_prop_value(this.clone(), "flags")?;
    let flags = e.to_js_string(flags_v)?;
    let flags_arg = e.str_val(flags.clone());
    let matcher = e.construct(ctor, &[this.clone(), flags_arg])?;
    let li = e.get_prop_value(this, "lastIndex")?;
    let li_n = e.to_number(li)?.max(0.0);
    set_strict(e, matcher.clone(), "lastIndex", Value::Num(li_n))?;
    let global = flags.contains('g');
    let unicode = flags.contains('u') || flags.contains('v');
    Ok(crate::collections::re_match_all_iter(
        e,
        matcher,
        Value::Str(s),
        global,
        unicode,
    ))
}

/// `rx[Symbol.replace](str, rep)` — spec 22.2.6.11 (JS-C618) :
/// `RegExpExec` dynamique collecté **avant** toute substitution
/// (ordre observable) ; `position`/`index`/captures/`groups`
/// lus par `Get` ; remplacement fonction appelé avec
/// `« matched, caps…, position, string [, groups] »`, sinon
/// `GetSubstitution`. `flags`/`global`/`lastIndex` observables.
pub(crate) fn re_sym_replace(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    if !e.is_spec_object(&this) {
        return Err(e.type_error("Symbol.replace receiver is not an object"));
    }
    let s = e.to_js_jstr(arg(args, 0))?;
    let rep = arg(args, 1);
    let functional = e.is_callable(rep.clone());
    // `rep_str` dans le domaine encodé (comme `hay`/`matched`)
    // — un template portant des units passe par `enc_js`.
    let rep_str = if functional {
        String::new()
    } else {
        enc_js(&e.to_js_jstr(rep.clone())?)
    };
    // `flags` lu par `Get`+`ToString` (observable) ; `global` et
    // `unicode` lus comme **propriétés** (algo corpus —
    // `get-global-err`/`coerce-global`/`get-unicode-error`).
    let flags_v = e.get_prop_value(this.clone(), "flags")?;
    let _flags = e.to_js_string(flags_v)?;
    let global = {
        let g = e.get_prop_value(this.clone(), "global")?;
        e.to_boolean(g)
    };
    let unicode = {
        let u = e.get_prop_value(this.clone(), "unicode")?;
        e.to_boolean(u)
    };
    if global {
        set_strict(e, this.clone(), "lastIndex", Value::Num(0.0))?;
    }
    // Collecte des résultats — TOUS les `exec` avant la
    // première lecture de capture (ordre spec observable).
    let mut results: Vec<Value> = Vec::new();
    loop {
        let result = reg_exp_exec(e, this.clone(), Value::Str(s.clone()))?;
        if matches!(result, Value::Null) {
            break;
        }
        let empty = {
            let m0 = e.get_prop_value(result.clone(), "0")?;
            e.to_js_string(m0)?.is_empty()
        };
        results.push(result);
        if !global {
            break;
        }
        if empty {
            let idx_v = e.get_prop_value(this.clone(), "lastIndex")?;
            let idx = e
                .to_number(idx_v)?
                .trunc()
                .clamp(0.0, 9_007_199_254_740_991.0);
            let next = advance_str_index(&s, idx, unicode);
            set_strict(e, this.clone(), "lastIndex", Value::Num(next))?;
        }
    }
    if results.is_empty() {
        return Ok(Value::Str(s));
    }
    // Domaine encodé quand `s` porte des `units` **ou des
    // astrals** — pièces et captures ré-encodées, décodage
    // unique en sortie. JS-C622 : non-`u` = domaine units
    // éclatés (positions `index`/`position` cohérentes avec
    // `enc_hay` du matching — mid-paire découparable) ; `u` =
    // domaine sentinelle joint.
    let has_surrogate_units = s.has_units() || s.as_str().chars().any(|c| (c as u32) >= 0x1_0000);
    let (hay_cow, dec): (std::borrow::Cow<'_, str>, bool) = if !unicode {
        if has_surrogate_units {
            (
                std::borrow::Cow::Owned(crate::regexp::re_encode_split(&s.units_view())),
                true,
            )
        } else {
            (std::borrow::Cow::Borrowed(s.as_str()), false)
        }
    } else if s.has_units() {
        (
            std::borrow::Cow::Owned(crate::regexp::re_encode(&s.units_view())),
            true,
        )
    } else {
        (std::borrow::Cow::Borrowed(s.as_str()), false)
    };
    let hay: Vec<char> = hay_cow.chars().collect();
    // JS-C622 : `position`/`nextSourcePosition` sont en **code
    // units** (spec) ; le découpage de `hay` travaille en chars
    // — `map` convertit (`None` = identité, pure BMP).
    let map = crate::regexp::UnitMap::of(&hay_cow);
    let u2c = |u: usize| map.as_ref().map_or(u, |m| m.u2c(u));
    let size = hay.len();
    let size_u = map.as_ref().map_or(size, |m| m.c2u(size));
    let mut acc = String::new();
    let mut next_pos = 0usize;
    for result in results {
        let lv = e.get_prop_value(result.clone(), "length")?;
        let n_caps = (e.to_number(lv)?.max(0.0) as usize).saturating_sub(1);
        let m0 = e.get_prop_value(result.clone(), "0")?;
        // `matched_js` = l'argument JS du remplacement fonction
        // (units conservés) ; `matched` = sa forme encodée pour
        // l'arithmétique de positions dans `hay`.
        let matched_js = e.to_js_jstr(m0)?;
        let matched = enc_js(&matched_js);
        let match_len_u = matched_js.u16_len();
        let iv = e.get_prop_value(result.clone(), "index")?;
        // `pos` en units (arg `position` du replacer) ;
        // `pos_c` en chars pour `hay`/`get_substitution`.
        let pos = (e.to_number(iv)?.trunc().max(0.0) as usize).min(size_u);
        let pos_c = u2c(pos);
        // Captures ToStringées (spec : `Get` puis `ToString`
        // pour chaque non-undefined — observable).
        let mut caps: Vec<Value> = Vec::with_capacity(n_caps);
        for ci in 1..=n_caps {
            let c = e.get_prop_value(result.clone(), &ci.to_string())?;
            caps.push(if matches!(c, Value::Undefined) {
                c
            } else {
                Value::Str(e.to_js_jstr(c)?)
            });
        }
        let named = e.get_prop_value(result, "groups")?;
        let rep_out = if functional {
            let mut call_args = Vec::with_capacity(n_caps + 4);
            call_args.push(Value::Str(matched_js));
            call_args.extend(caps.iter().cloned());
            call_args.push(Value::Num(pos as f64));
            call_args.push(Value::Str(s.clone()));
            if !matches!(named, Value::Undefined) {
                call_args.push(named.clone());
            }
            let r = e.call_value(rep.clone(), Value::Undefined, &call_args)?;
            enc_val(e, r)?
        } else {
            // `namedCaptures` : `undefined` saute `ToObject` ;
            // `null` → `ToObject` → `TypeError` (spec — un
            // `groups:null` retourné par un exec custom
            // doit jeter) ; primitives → boxing.
            let named_obj = if matches!(named, Value::Undefined) || e.is_spec_object(&named) {
                named
            } else if matches!(named, Value::Null) {
                return Err(e.type_error("cannot convert null to object"));
            } else {
                e.box_primitive(named)
            };
            get_substitution(e, &matched, &hay, pos_c, &caps, &named_obj, &rep_str)?
        };
        if pos >= next_pos {
            // `position` reculant : la substitution est ignorée
            // (spec — exec custom malveillant).
            let piece: String = hay[u2c(next_pos)..pos_c].iter().collect();
            acc.push_str(&piece);
            acc.push_str(&rep_out);
            next_pos = pos + match_len_u;
        }
        if next_pos >= size_u {
            return Ok(re_str_val(e, &acc, dec));
        }
    }
    let tail: String = hay[u2c(next_pos)..size].iter().collect();
    acc.push_str(&tail);
    Ok(re_str_val(e, &acc, dec))
}

/// `rx[Symbol.search](str)` — spec 22.2.6.12 (JS-C618) :
/// `lastIndex` sauvegardé, remis à 0, puis **restauré** après
/// `RegExpExec` dynamique — `global`/`lastIndex` n'influent
/// pas la recherche ; `index` relu par `Get`.
pub(crate) fn re_sym_search(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    if !e.is_spec_object(&this) {
        return Err(e.type_error("Symbol.search receiver is not an object"));
    }
    let s = e.to_js_jstr(arg(args, 0))?;
    let prev = e.get_prop_value(this.clone(), "lastIndex")?;
    if !same_value(&prev, &Value::Num(0.0)) {
        set_strict(e, this.clone(), "lastIndex", Value::Num(0.0))?;
    }
    let result = reg_exp_exec(e, this.clone(), Value::Str(s))?;
    let cur = e.get_prop_value(this.clone(), "lastIndex")?;
    if !same_value(&cur, &prev) {
        set_strict(e, this, "lastIndex", prev)?;
    }
    if matches!(result, Value::Null) {
        return Ok(Value::Num(-1.0));
    }
    e.get_prop_value(result, "index")
}
/// `RegExpExec(R, S)` (spec 22.2.7.1, JS-C617) : `exec`
/// callable (lu par `Get` — observable) → `Call(exec, R, «S»)`,
/// résultat non-objet non-null → `TypeError` ; non-callable →
/// `RegExpBuiltinExec` — le slot `[[RegExpMatcher]]` est requis
/// (un objet sans slot → `TypeError`).
pub(crate) fn reg_exp_exec(e: &mut Engine, rx: Value, s: Value) -> JsResult<Value> {
    let exec = e.get_prop_value(rx.clone(), "exec")?;
    if e.is_callable(exec.clone()) {
        let r = e.call_value(exec, rx, &[s])?;
        if !e.is_spec_object(&r) && !matches!(r, Value::Null) {
            return Err(e.type_error("RegExp exec result must be an Object or null"));
        }
        return Ok(r);
    }
    if matches!(rx, Value::Obj(id)
        if matches!(e.heap.obj(id).map(|o| &o.kind), Some(ObjKind::RegExp(_))))
    {
        return regexp_exec(e, rx, &[s]);
    }
    Err(e.type_error("not a RegExp"))
}

/// `%RegExp%` du realm courant — défaut `SpeciesConstructor`.
fn regexp_intrinsic(e: &mut Engine) -> JsResult<Value> {
    e.realm_global_own(e.cur_ctx, "RegExp")
        .ok_or_else(|| e.type_error("missing %RegExp% intrinsic"))
}

/// `array[lengthA] = v` (`CreateDataPropertyOrThrow`) puis
/// `lengthA += 1` — `true` quand `lengthA` atteint `lim`.
fn emit_split(
    e: &mut Engine,
    aid: ObjId,
    len_a: &mut usize,
    lim: usize,
    v: Value,
) -> JsResult<bool> {
    if !e.create_data_prop_spec_on(aid, &len_a.to_string(), v)? {
        return Err(e.type_error("split: element is not creatable"));
    }
    *len_a += 1;
    Ok(*len_a >= lim)
}

/// `rx[Symbol.split](str, limit)` — spec 22.2.6.14 (JS-C617) :
/// `SpeciesConstructor` → `Construct(C, «rx, flags+y»)` → boucle
/// `RegExpExec` à `lastIndex` **observable** (le `exec` d'un
/// splitter custom peut le déplacer — `matchEnd` est relu après
/// l'appel, pas inféré du match). Captures intercalées, `lim`
/// borne `lengthA`. Avant : stub natif `JsRegExp::split` — ni
/// species, ni flags, ni `lastIndex`, ni `exec` custom
/// n'étaient observés.
pub(crate) fn re_sym_split(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    if !e.is_spec_object(&this) {
        return Err(e.type_error("Symbol.split receiver is not an object"));
    }
    let s = e.to_js_jstr(arg(args, 0))?;
    // `SpeciesConstructor(rx, %RegExp%)` : `constructor` puis
    // `@@species` lus par `Get` (accesseurs observables).
    let c = e.get_prop_value(this.clone(), "constructor")?;
    let ctor = if matches!(c, Value::Undefined) {
        regexp_intrinsic(e)?
    } else {
        if !e.is_spec_object(&c) {
            return Err(e.type_error("constructor is not an object"));
        }
        match e.get_prop_value(c, crate::interp::SYM_SPECIES)? {
            Value::Undefined | Value::Null => regexp_intrinsic(e)?,
            sc if e.is_constructor(sc.clone()) => sc,
            _ => return Err(e.type_error("@@species is not a constructor")),
        }
    };
    let flags_v = e.get_prop_value(this.clone(), "flags")?;
    let flags = e.to_js_string(flags_v)?;
    // `unicodeMatching` — sert `AdvanceStringIndex` ; +1 char
    // est correct des deux côtés dans notre domaine char.
    let unicode = flags.contains('u') || flags.contains('v');
    let new_flags = if flags.contains('y') {
        flags
    } else {
        format!("{flags}y")
    };
    let nf_v = e.str_val(new_flags);
    let splitter = e.construct(ctor, &[this, nf_v])?;
    let a = e.new_array(Vec::new());
    let Value::Obj(aid) = a.clone() else {
        return Ok(a);
    };
    let lim = match arg(args, 1) {
        Value::Undefined => u32::MAX as usize,
        v => to_uint32(e.to_number(v)?) as usize,
    };
    if lim == 0 {
        return Ok(a);
    }
    // Domaine encodé quand `s` porte des `units` **ou des
    // astrals** — même convention qu'`enc_hay` : JS-C622 non-`u`
    // = units éclatés (mid-paire découparable), `u` = domaine
    // sentinelle joint. Pièces décodées via `re_str_val`.
    let has_surrogate_units = s.has_units() || s.as_str().chars().any(|c| (c as u32) >= 0x1_0000);
    let (hay_cow, dec): (std::borrow::Cow<'_, str>, bool) = if !unicode {
        if has_surrogate_units {
            (
                std::borrow::Cow::Owned(crate::regexp::re_encode_split(&s.units_view())),
                true,
            )
        } else {
            (std::borrow::Cow::Borrowed(s.as_str()), false)
        }
    } else if s.has_units() {
        (
            std::borrow::Cow::Owned(crate::regexp::re_encode(&s.units_view())),
            true,
        )
    } else {
        (std::borrow::Cow::Borrowed(s.as_str()), false)
    };
    let hay: Vec<char> = hay_cow.chars().collect();
    let size = hay.len();
    // JS-C622 : `lastIndex` est en **code units** — les bornes
    // de la boucle vivent en chars de `hay`, conversion aux
    // écritures/lectures (`None` = identité, pure BMP).
    let map = crate::regexp::UnitMap::of(&hay_cow);
    let u2c = |u: usize| map.as_ref().map_or(u, |m| m.u2c(u));
    let c2u = |c: usize| map.as_ref().map_or(c, |m| m.c2u(c));
    let size_u = c2u(size);
    // `s` n'est plus emprunté passé `UnitMap::of` — `hay_cow`
    // est matérialisé dans `hay`/`map`.
    let s_arg = Value::Str(s);
    let mut length_a = 0usize;
    // Chaîne vide : un match → `[]`, sinon `[s]`.
    if size == 0 {
        let mr = reg_exp_exec(e, splitter, s_arg)?;
        if !matches!(mr, Value::Null) {
            return Ok(a);
        }
        let empty_v = re_str_val(e, "", dec);
        emit_split(e, aid, &mut length_a, lim, empty_v)?;
        return Ok(a);
    }
    let mut last_match_end = 0usize;
    let mut search_index = 0usize;
    while search_index < size {
        // `Set(splitter, "lastIndex", 𝔽(q), true)` — q en units.
        set_strict(
            e,
            splitter.clone(),
            "lastIndex",
            Value::Num(c2u(search_index) as f64),
        )?;
        let mr = reg_exp_exec(e, splitter.clone(), s_arg.clone())?;
        if matches!(mr, Value::Null) {
            // `AdvanceStringIndex` : +1 char — en unicode les
            // paires de surrogates sont déjà un char unique ;
            // non-unicode sur l'astral avance d'un char au lieu
            // d'un code unit (déviation cataloguée).
            search_index += 1;
            continue;
        }
        // `matchEnd = ToLength(Get(splitter,"lastIndex"))` —
        // relu *après* `exec` (units), borné à `size_u`, rendu
        // en chars pour le découpage.
        let end_v = e.get_prop_value(splitter.clone(), "lastIndex")?;
        let match_end_u = (e.to_number(end_v)?.max(0.0) as usize).min(size_u);
        let match_end = u2c(match_end_u).min(size);
        if match_end == last_match_end {
            search_index += 1;
            continue;
        }
        let piece: String = hay[last_match_end..search_index].iter().collect();
        let piece_v = re_str_val(e, &piece, dec);
        if emit_split(e, aid, &mut length_a, lim, piece_v)? {
            return Ok(a);
        }
        last_match_end = match_end;
        // Captures : `LengthOfArrayLike(matchResult) - 1` puis
        // `Get(matchResult, i)` — observable (un résultat custom
        // peut mentir sur ses propres indexes).
        let n_caps = {
            let lv = e.get_prop_value(mr.clone(), "length")?;
            (e.to_number(lv)?.max(0.0) as usize).saturating_sub(1)
        };
        for ci in 1..=n_caps {
            let cap = e.get_prop_value(mr.clone(), &ci.to_string())?;
            if emit_split(e, aid, &mut length_a, lim, cap)? {
                return Ok(a);
            }
        }
        search_index = last_match_end;
    }
    let piece: String = hay[last_match_end..size].iter().collect();
    let piece_v = re_str_val(e, &piece, dec);
    emit_split(e, aid, &mut length_a, lim, piece_v)?;
    Ok(a)
}

/// `Error.captureStackTrace(obj, ctor?)` — extension V8 : pose la
/// own prop `stack` non-énumérable (en-tête + frames réelles,
/// JS-C47 — l'argument `ctor` d'élision de frames n'est pas
/// honoré : toute la pile est rendue, déviation documentée).
pub(crate) fn error_capture_stack(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = arg(args, 0) else {
        return Err(e.type_error("captureStackTrace target must be an object"));
    };
    let name = e
        .get_prop(id, "name")
        .and_then(|v| e.to_js_string(v).ok())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "Error".to_string());
    let msg = e
        .get_prop(id, "message")
        .and_then(|v| e.to_js_string(v).ok())
        .unwrap_or_default();
    let s = e.str_val(e.stack_text(&name, &msg));
    e.set_own(id, "stack", s, false);
    Ok(Value::Undefined)
}
