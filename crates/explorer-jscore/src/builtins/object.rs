// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Builtins JS — section extraite de `builtins.rs` (JS-C40, pur
//! reformatage mécanique : sémantique et tests inchangés).

#[allow(clippy::wildcard_imports)]
use super::*;

// ------------------------------------------------------------------
// Object
// ------------------------------------------------------------------

pub(crate) fn object_to_string(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let tag = match this {
        Value::Undefined | Value::Hole => "Undefined",
        Value::Null => "Null",
        Value::Num(_) => "Number",
        Value::Bool(_) => "Boolean",
        Value::Str(_) => "String",
        Value::Big(_) => "BigInt",
        // Arguments exotiques : `builtinTag = "Arguments"` (spec
        // `Object.prototype.toString` §19.1.3.6) — discriminé par
        // `args_map` (posé sur mapped ET unmapped).
        Value::Obj(id) if e.heap.obj(id).is_some_and(|o| o.args_map.is_some()) => "Arguments",
        Value::Obj(id) => match e.heap.obj(id).map(|o| &o.kind) {
            Some(ObjKind::Array(_)) => "Array",
            Some(ObjKind::Fun(_)) => "Function",
            Some(ObjKind::RegExp(_)) => "RegExp",
            Some(ObjKind::ByteBuf(_)) => "ArrayBuffer",
            Some(ObjKind::DataView { .. }) => "DataView",
            Some(ObjKind::TypedArray { ta, .. }) => ta.name(),
            Some(ObjKind::Host(h)) => return Ok(e.str_val(format!("[object {}]", h.class_name()))),
            _ => "Object",
        },
    };
    // `@@toStringTag` override (spec `Object.prototype.toString` —
    // les instances lisent leur tag propre ou hérité).
    if let Value::Obj(id) = this {
        if let Some(Value::Str(sid)) = e.get_prop(id, crate::interp::SYM_TO_STRING_TAG) {
            let s = sid.to_string();
            return Ok(e.str_val(format!("[object {s}]")));
        }
    }
    Ok(e.str_val(format!("[object {tag}]")))
}

pub(crate) fn has_own_property(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    // `ToPropertyKey` — un symbole indexe sa clé, pas une
    // conversion `ToString` (TypeError spuri — trigger réel :
    // `hasOwnProperty.call(t, Symbol())` dans catalyst/GitHub).
    let key = e.prop_key(arg(args, 0))?;
    // `StringGetOwnProperty` : l'index d'un wrapper
    // `new String(s)` (PRIM = Str) est own, hors `props`.
    let str_idx = match &this {
        Value::Obj(id) => {
            e.heap
                .obj(*id)
                .and_then(|o| match o.get_own(crate::builtins::ctors::PRIM) {
                    Some(Value::Str(sid)) => crate::interp::array_index(&key)
                        .filter(|i| *i < sid.u16_len())
                        .map(|_| true),
                    _ => None,
                })
        }
        _ => None,
    };
    let has = match str_idx {
        Some(true) => true,
        _ => match this {
            // Proxy : `hasOwnProperty` passe par la trap
            // `getOwnPropertyDescriptor` (spec `[[GetOwnProperty]]`).
            Value::Obj(id)
                if matches!(e.heap.obj(id).map(|o| &o.kind), Some(ObjKind::Proxy { .. })) =>
            {
                e.get_own_prop(id, &key)?.is_some()
            }
            Value::Obj(id) => match e.heap.obj(id) {
                Some(o) => match &o.kind {
                    ObjKind::Array(elems) => {
                        key == "length"
                            || crate::interp::array_index(&key)
                                .is_some_and(|i| elems.get(i).is_some_and(|v| !v.is_hole()))
                            || o.find(&key).is_some()
                    }
                    // Index canonique d'une vue typée : own ssi index
                    // **vivant** (`-0`/`-1`/`1.5`/`NaN` sont canoniques
                    // mais invalides → `false`) — borne lue sur la
                    // région réelle du buffer (détaché → vidé, RAB
                    // rétréci, spec `IsValidIntegerIndex`, JS-C606) ;
                    // clé non-canonique → prop ordinaire.
                    ObjKind::TypedArray {
                        ta,
                        buffer,
                        offset,
                        len,
                        auto,
                    } => match crate::interp::canonical_numeric_index(&key) {
                        Some(n) => crate::interp::ta_live_index(
                            &e.heap, *ta, buffer, *offset, *len, *auto, n,
                        )
                        .is_some(),
                        None => o.find(&key).is_some(),
                    },
                    _ => o.find(&key).is_some(),
                },
                None => false,
            },
            // Primitif string : `length` et les indices sont des
            // props propres du wrapper `ToObject` (spec).
            Value::Str(sid) => {
                key == "length"
                    || crate::interp::array_index(&key).is_some_and(|i| i < sid.u16_len())
            }
            _ => false,
        },
    };
    Ok(Value::Bool(has))
}

pub(crate) fn object_value_of(_e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    Ok(this)
}

/// `o.isPrototypeOf(v)` — `this` se trouve-t-il sur la chaîne de
/// prototypes de `v` (bornée à 100 sauts).
pub(crate) fn object_is_prototype_of(
    e: &mut Engine,
    this: Value,
    args: &[Value],
) -> JsResult<Value> {
    let (Value::Obj(pid), Value::Obj(vid)) = (this, arg(args, 0)) else {
        return Ok(Value::Bool(false));
    };
    let mut cur = e.heap.obj(vid).map(|o| o.proto.clone());
    let mut depth = 0;
    while let Some(Value::Obj(c)) = cur {
        if c == pid {
            return Ok(Value::Bool(true));
        }
        cur = e.heap.obj(c).map(|o| o.proto.clone());
        depth += 1;
        if depth > 100 {
            break;
        }
    }
    Ok(Value::Bool(false))
}

pub(crate) fn object_keys(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    // `Object.keys('ab')` → `['0','1']` — indices énumérables de
    // la string-box (spec `ToObject`).
    if let Value::Str(sid) = arg(args, 0) {
        let n = sid.u16_len();
        let vals: Vec<Value> = (0..n).map(|i| e.str_val(i.to_string())).collect();
        return Ok(arr_of(e, vals));
    }
    let Value::Obj(id) = arg(args, 0) else {
        return Ok(arr_of(e, Vec::new()));
    };
    let keys = e.own_enum_keys_trapped(id)?;
    // Spec : `Object.keys` = clés **string** énumérables — les
    // clés-symboles `\u{f130}…` énumérables ne doivent pas fuir
    // comme strings sentinelles (même classe que `Reflect.ownKeys`,
    // §B-57).
    let vals: Vec<Value> = keys
        .iter()
        .filter(|k| !crate::builtins::bigint::is_sentinel_key(k))
        .map(|k| e.str_val(k.clone()))
        .collect();
    Ok(arr_of(e, vals))
}

pub(crate) fn object_get_proto(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    match arg(args, 0) {
        // Proxy : trap `getPrototypeOf` (objet ou null requis),
        // sinon délégation à la cible — boucle bornée : un proxy
        // sans trap transfère `[[GetPrototypeOf]]` à sa cible,
        // qui peut elle-même proxifier (J2, chaîne alpine).
        Value::Obj(mut id) => {
            let mut depth = 0;
            while let Some((tid, target, handler)) = e.proxy_th(id)? {
                let trap = e.proxy_trap(handler.clone(), "getPrototypeOf")?;
                if matches!(trap, Value::Obj(_)) {
                    let r = e.call_value(trap, handler, &[target])?;
                    match r {
                        Value::Obj(_) | Value::Null => {
                            // Spec : cible non-extensible → le trap
                            // doit rendre le proto de la cible.
                            if !e.target_extensible(tid) {
                                let tp = e.heap.obj(tid).map_or(Value::Null, |o| o.proto.clone());
                                if !e.strict_eq(&r, &tp) {
                                    return Err(e.type_error(
                                        "proxy getPrototypeOf trap must return the target's prototype",
                                    ));
                                }
                            }
                            return Ok(r);
                        }
                        _ => {
                            return Err(e.type_error(
                                "proxy getPrototypeOf trap must return an object or null",
                            ));
                        }
                    }
                }
                id = tid;
                e.deleg_hop(&mut depth)?;
            }
            Ok(e.heap.obj(id).map_or(Value::Null, |o| o.proto.clone()))
        }
        // Spec `ToObject` : `null`/`undefined` → `TypeError` ; les
        // primitives sont wrappées et remontent leur proto (github
        // `behaviors.js` — `getPrototypeOf(elt.delegate)` avec
        // `delegate` null : l'erreur doit surgir ici, pas au
        // `.requestErrored` en aval).
        Value::Null | Value::Undefined | Value::Hole => {
            Err(e.type_error("Cannot convert undefined or null to object"))
        }
        Value::Num(_) => Ok(Value::Obj(e.protos.number)),
        Value::Str(_) => Ok(Value::Obj(e.protos.string)),
        Value::Bool(_) => Ok(Value::Obj(e.protos.boolean)),
        Value::Big(_) => Ok(Value::Obj(e.protos.bigint)),
    }
}

/// `Object.setPrototypeOf(o, p)` — `p` objet ou `null` ; refus sur
/// cycle de prototype et sur objet non-extensible (spec). Sur un
/// proxy : trap `setPrototypeOf` → `false` = `TypeError`.
pub(crate) fn object_set_proto(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let target = arg(args, 0);
    let proto = match arg(args, 1) {
        v @ Value::Obj(_) => v,
        Value::Null | Value::Undefined => Value::Null,
        _ => return Err(e.type_error("setPrototypeOf: proto must be an object or null")),
    };
    let Value::Obj(mut id) = target else {
        return Ok(target); // primitives are ignored, per spec
    };
    // Délégation bornée — chaîne `Proxy(proxy)×N` sans trap
    // récurait ici hors `call_depth` (J2, classe alpine).
    let mut depth = 0;
    while let Some((tid, tv, handler)) = e.proxy_th(id)? {
        let trap = e.proxy_trap(handler.clone(), "setPrototypeOf")?;
        if matches!(trap, Value::Obj(_)) {
            let ok = e.call_value(trap, handler, &[tv, proto.clone()])?;
            if !e.to_boolean(ok) {
                return Err(e.type_error("proxy setPrototypeOf trap returned false"));
            }
            // Spec : cible non-extensible → le proto demandé doit
            // être identique à celui de la cible.
            if !e.target_extensible(tid) {
                let tp = e.heap.obj(tid).map_or(Value::Null, |o| o.proto.clone());
                if !e.strict_eq(&proto, &tp) {
                    return Err(e.type_error(
                        "proxy setPrototypeOf trap returned true without changing the prototype",
                    ));
                }
            }
            return Ok(target);
        }
        id = tid;
        e.deleg_hop(&mut depth)?;
    }
    if !e.heap.obj(id).is_some_and(|o| o.extensible) {
        return Err(e.type_error("cannot set prototype of non-extensible object"));
    }
    // Cycle check : remonter la chaîne de `proto` jusqu'à `id`.
    let mut cur = proto.clone();
    while let Value::Obj(cid) = cur {
        if cid == id {
            return Err(e.type_error("cyclic prototype value"));
        }
        cur = e.heap.obj(cid).map_or(Value::Null, |o| o.proto.clone());
    }
    // Sonde cran 47 : `b.__proto__ = ShadowRoot.prototype`
    // (wc_sd `shadyUpgradeFragment`) re-prototype l'objet. Si le
    // receveur porte déjà des champs de contrôleur PolySi
    // (`hostElement`/`__dataEnabled`), c'est le `this` de
    // `new X` qui passe en fragment d'upgrade — signature du
    // `zc` creux (`urp@absent`).
    if crate::builtins::extras::has_own(e, id, "hostElement")
        || crate::builtins::extras::has_own(e, id, "__dataEnabled")
    {
        let host = e
            .get_prop_value(Value::Obj(id), "hostElement")
            .ok()
            .and_then(|h| e.get_prop_value(h, "localName").ok())
            .and_then(|l| e.to_js_string(l).ok())
            .unwrap_or_else(|| "?".to_string());
        // Cran 53c — distinguer fragment stampé (`templateInfo`/
        // `nodeInfoList`/`$` own) du contrôleur (`root`/`attached`
        // /`urp` dans la chaîne AVANT le swap — mesure la perte
        // réelle de la chaîne de classe).
        let is_frag = crate::builtins::extras::has_own(e, id, "templateInfo")
            || crate::builtins::extras::has_own(e, id, "nodeInfoList")
            || crate::builtins::extras::has_own(e, id, "$");
        let roteq =
            matches!(e.get_prop_value(Value::Obj(id), "root"), Ok(Value::Obj(r)) if r == id);
        let urpc = e.find_prop(id, "updateReduxProperties").is_some();
        let urpo = crate::builtins::extras::has_own(e, id, "updateReduxProperties");
        // Cran 54 — nommer le proto **remplacé** : en Chrome un
        // fragment stampé hérite `DocumentFragment.prototype` ;
        // si le nôtre porte `d.prototype` (proto de classe
        // Polymer, repérable par `is`/`ctor`), la déviation est
        // l'héritage du fragment, pas le swap.
        let pname = e
            .heap
            .obj(id)
            .and_then(|o| match o.proto {
                Value::Obj(pid) => Some(pid),
                _ => None,
            })
            .map_or_else(
                || "null".to_string(),
                |pid| {
                    let is = e
                        .get_prop_value(Value::Obj(pid), "is")
                        .ok()
                        .and_then(|v| e.to_js_string(v).ok())
                        .filter(|s| !s.is_empty());
                    let ctor = e
                        .get_prop_value(Value::Obj(pid), "constructor")
                        .ok()
                        .and_then(|c| e.get_prop_value(c, "name").ok())
                        .and_then(|n| e.to_js_string(n).ok())
                        .filter(|s| !s.is_empty());
                    match (is, ctor) {
                        (Some(i), Some(c)) => format!("{i}/{c}"),
                        (Some(i), None) => i,
                        (None, Some(c)) => c,
                        (None, None) => "?".to_string(),
                    }
                },
            );
        // Cran 58 — `nodeType` own (11 = vrai fragment) et
        // `hpc` : `b` est-il le `polymerController` de son
        // hôte ? (contrôleur swappé → méthodes de classe
        // perdues → `loadPage` meurt).
        let nt = e
            .get_prop_value(Value::Obj(id), "nodeType")
            .ok()
            .and_then(|v| e.to_js_string(v).ok())
            .unwrap_or_else(|| "?".to_string());
        let hpc = e
            .get_prop_value(Value::Obj(id), "hostElement")
            .ok()
            .and_then(|h| e.get_prop_value(h, "polymerController").ok())
            .is_some_and(|pc| e.strict_eq(&pc, &Value::Obj(id)));
        let stack: String = e
            .stack_text("", "")
            .lines()
            .map(str::trim)
            .filter(|l| l.starts_with("at "))
            .take(12)
            .map(|l| {
                // Les URLs kevlar (~200 chars) noient le
                // `:line:col` final — tag du script + queue.
                let tag = if l.contains("kevlar") {
                    "kb:"
                } else if l.contains("webcomponents") {
                    "wc:"
                } else {
                    "ot:"
                };
                if l.len() > 60 {
                    format!("{tag}{}", &l[l.len().saturating_sub(14)..])
                } else {
                    format!("{tag}{l}")
                }
            })
            .collect::<Vec<_>>()
            .join(" <- ");
        tracing::info!(target: "dom_probe",
            site = "proto.swap", host = %host,
            frag = is_frag, roteq, urpc, urpo, pname = %pname,
            ext = e.heap.obj(id).is_some_and(|o| o.extensible),
            nt = %nt, hpc,
            stack = %stack.chars().take(400).collect::<String>());
    }
    if let Some(o) = e.heap.obj_mut(id) {
        o.set_proto(proto);
    }
    Ok(target)
}

pub(crate) fn object_values(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = arg(args, 0) else {
        return Ok(arr_of(e, Vec::new()));
    };
    let keys = e.own_enum_keys_trapped(id)?;
    let mut vals = Vec::with_capacity(keys.len());
    for k in keys {
        // Spec `Object.values` : valeurs des props **string**
        // énumérables — les clés-symboles sont exclues.
        if crate::builtins::bigint::is_sentinel_key(&k) {
            continue;
        }
        // Getter-aware read (class accessors are own props).
        vals.push(e.get_prop_value(Value::Obj(id), &k)?);
    }
    Ok(arr_of(e, vals))
}

/// `Object.assign(t, …srcs)` — enumerable-own copy, accessor-aware
/// read and write (setters on the target fire, per spec).
pub(crate) fn object_assign(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let target = arg(args, 0);
    for src in &args[1.min(args.len())..] {
        if let Value::Obj(sid) = src {
            // diag sonde (même mur que `{...}` : getters
            // proto perdus — spec, terrain TanStack).
            if crate::heap::proto_has_accessor(&e.heap, *sid) {
                e.diag("Object.assign dropped proto getters");
            }
            for k in e.own_enum_keys_trapped(*sid)? {
                let v = match e.get_prop_value(src.clone(), &k) {
                    Ok(v) => v,
                    Err(t) => {
                        crate::defprop_note(format!(
                            "assign #{sid} read `{k}` → {}",
                            defprop_err_summary(e, t.0.clone())
                        ));
                        return Err(t);
                    }
                };
                if let Err(t) = e.set_prop_value(target.clone(), &k, v) {
                    crate::defprop_note(format!(
                        "assign tgt={target:?} set `{k}` → {}",
                        defprop_err_summary(e, t.0.clone())
                    ));
                    return Err(t);
                }
            }
        }
    }
    Ok(target)
}

/// `Object.create(proto[, propsMap])` — proto must be an object or
/// `null`; the optional map applies `defineProperties` semantics.
pub(crate) fn object_create(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let proto = match arg(args, 0) {
        // Cellule Symbol : primitive malgré l'`ObjId` — pas un
        // proto valide (spec `ObjectCreate` exige Object|null ;
        // `15.2.3.5-1-1` — `undefined`/`Symbol` → `TypeError`).
        v @ Value::Obj(pid) => {
            if let Some(ObjKind::Symbol { .. }) = e.heap.obj(pid).map(|o| &o.kind) {
                return Err(e.type_error("Object.create: proto must be an object"));
            }
            v
        }
        Value::Null => Value::Null,
        _ => return Err(e.type_error("Object.create: proto must be an object")),
    };
    let id = e.heap.alloc_obj(Object::ordinary(proto));
    // `Properties === undefined` → skip ; TOUT le reste passe
    // `ToObject` (spec `ObjectDefineProperties`) — `null` →
    // TypeError (`15.2.3.5-4-3`), primitive → wrapper vide
    // (no-op) sauf string non-vide (indices énumérables →
    // descripteurs non-objets → TypeError).
    match arg(args, 1) {
        Value::Undefined | Value::Hole => {}
        map => define_props_from_map(e, id, map)?,
    }
    Ok(Value::Obj(id))
}

/// `Object.defineProperty(o, k, desc)` — descripteur complet :
/// `value`/`writable`/`enumerable`/`configurable`/`get`/`set`.
/// Règles simplifiées de `[[DefineOwnProperty]]` (heap.rs) :
/// non-configurable → rejet `TypeError`.
pub(crate) fn object_define_property(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let target @ Value::Obj(id) = arg(args, 0) else {
        return Err(e.type_error("Object.defineProperty on non-object"));
    };
    let key = e.prop_key(arg(args, 1))?;
    let Value::Obj(desc_id) = arg(args, 2) else {
        return Err(e.type_error("property descriptor must be an object"));
    };
    // Les cellules Symbol vivent en `ObjKind::Symbol` — un symbole
    // n'est PAS un objet descripteur (spec — `property-description
    // -must-be-an-object-not-symbol`).
    if let Some(ObjKind::Symbol { .. }) = e.heap.obj(desc_id).map(|o| &o.kind) {
        return Err(e.type_error("property descriptor must be an object"));
    }
    if !define_one_prop(e, id, &key, desc_id)? {
        return Err(e.type_error(format!("cannot define property `{key}`")));
    }
    Ok(target)
}

/// Corps de `[[DefineOwnProperty]]` piloté par un objet descripteur —
/// partagé par `defineProperty`/`defineProperties`/`Object.create`.
/// Sur un proxy : trap `defineProperty` ou délégation à la cible.
/// Enveloppe sonde JS-C313 : tout rejet est journalisé (cible, clé,
/// prop courante, raison) dans le log borné `defprop_log`.
pub(crate) fn define_one_prop(
    e: &mut Engine,
    id: ObjId,
    key: &str,
    desc_id: ObjId,
) -> JsResult<bool> {
    // Sonde kevlar §cran-27 : `polymerController` peut être
    // posé par `Object.defineProperty` (acc `get/set` ou
    // valeur) sans passer `set_prop_value` — la sonde `pc.set`
    // de l'interp ne le voit pas ; celle-ci le nomme (classe
    // hôte + forme du descripteur + pile JS bornée).
    if key == "polymerController" && tracing::enabled!(target: "dom_probe", tracing::Level::INFO) {
        let cls = e.host_of(id).map_or("<plain>", |h| h.class_name());
        let dv = e
            .get_prop(desc_id, "value")
            .map_or_else(|| "<acc/none>".into(), |v| e.describe(v));
        let stack: String = e
            .stack_text("", "")
            .lines()
            .map(str::trim)
            .filter(|l| l.starts_with("at "))
            .take(8)
            .collect::<Vec<_>>()
            .join(" <- ");
        let stack: String = stack.chars().take(600).collect();
        tracing::info!(target: "dom_probe",
            site = "pc.def", host = %cls,
            v = %dv.chars().take(120).collect::<String>(),
            stack = %stack);
    }
    let r = define_one_prop_impl(e, id, key, desc_id);
    if let Err(t) = &r {
        crate::defprop_note(defprop_err_desc(e, id, key, t));
    }
    r
}

/// Résumé « type de prop » pour le log sonde (`u`=absent,
/// `f`=callable/obj, `v`=valeur).
fn vtag(v: Value) -> char {
    match v {
        Value::Undefined => 'u',
        Value::Obj(_) => 'f',
        _ => 'v',
    }
}

/// Drapeaux compacts d'une prop existante (`D(w,e,c)` / `A(g,s,e,c)`).
fn prop_flags(p: crate::heap::Prop) -> String {
    match p {
        crate::heap::Prop::Data { attrs, .. } => format!(
            "D(w{}e{}c{})",
            attrs.writable as u8, attrs.enumerable as u8, attrs.configurable as u8
        ),
        crate::heap::Prop::Acc { get, set, attrs } => format!(
            "A(g{}s{}e{}c{})",
            vtag(get),
            vtag(set),
            attrs.enumerable as u8,
            attrs.configurable as u8
        ),
    }
}

/// Nom du `constructor` own-prop du proto ciblé — identifie QUEL
/// proto de la chaîne (y1/Y/P/…) reçoit le `defineProperty`.
fn proto_ctor_name(e: &Engine, pid: ObjId) -> Option<String> {
    let p = e.heap.obj(pid)?;
    let cv = p.find("constructor").and_then(|pr| pr.data_value())?;
    let Value::Obj(cid) = cv else {
        return None;
    };
    let c = e.heap.obj(cid)?;
    let nv = c.find("name").and_then(|pr| pr.data_value())?;
    Engine::heap_string_of(&nv)
}

/// Résumé d'un `Thrown` — `stack`/`name`/`message` propres (lectures
/// heap seulement : jamais d'éval utilisateur, borne à 200 chars).
fn defprop_err_summary(e: &Engine, v: Value) -> String {
    let Value::Obj(id) = v else {
        return match v {
            Value::Str(sid) => sid.to_string(),
            _ => format!("{v:?}"),
        };
    };
    let Some(o) = e.heap.obj(id) else {
        return "<err?>".into();
    };
    let own = |k: &str| {
        o.find(k)
            .and_then(|p| p.data_value())
            .as_ref()
            .and_then(Engine::heap_string_of)
    };
    let mut out = own("stack")
        .filter(|s| !s.is_empty())
        .or_else(|| match (own("name"), own("message")) {
            (Some(n), Some(m)) => Some(format!("{n}: {m}")),
            (_, m @ Some(_)) => m,
            (n @ Some(_), None) => n,
            _ => None,
        })
        .unwrap_or_else(|| "<thrown>".into());
    out.truncate(200);
    out
}

/// Sonde JS-C313 — « cible | clé | prop courante | raison » d'un
/// rejet `defineProperty` : le receveur nommé (kind, own, proto
/// ctor) + `cur` (non-configurable ? accesseur ? absent) + le
/// message/stack de l'erreur identifient le site exact du drop.
fn defprop_err_desc(e: &Engine, id: ObjId, key: &str, t: &crate::heap::Thrown) -> String {
    let (target, cur) = e.heap.obj(id).map_or_else(
        || (format!("?#{id}"), "-".into()),
        |o| {
            let kind = match &o.kind {
                ObjKind::Ordinary => "Ord",
                ObjKind::Array(_) => "Arr",
                ObjKind::Fun(_) => "Fun",
                ObjKind::Host(_) => "Host",
                ObjKind::Map(_) => "Map",
                ObjKind::Set(_) => "Set",
                _ => "other",
            };
            let proto = match o.proto {
                Value::Obj(pid) => proto_ctor_name(e, pid)
                    .map_or_else(|| format!("{pid}"), |n| format!("{n}#{pid}")),
                _ => "null".into(),
            };
            let cur = o.find(key).map_or_else(|| "-".into(), prop_flags);
            (
                format!(
                    "{kind}#{id}(own={},ext={},proto={proto})",
                    o.props.len(),
                    o.extensible as u8
                ),
                cur,
            )
        },
    );
    format!(
        "defprop {target} `{key}` cur={cur} → {}",
        defprop_err_summary(e, t.0.clone())
    )
}

fn define_one_prop_impl(e: &mut Engine, id: ObjId, key: &str, desc_id: ObjId) -> JsResult<bool> {
    // Délégation bornée — même motif que `object_set_proto` (J2).
    let mut id = id;
    let mut depth = 0;
    while let Some((tid, target, handler)) = e.proxy_th(id)? {
        let trap = e.proxy_trap(handler.clone(), "defineProperty")?;
        if matches!(trap, Value::Obj(_)) {
            let kv = e.str_val(key);
            let r = e.call_value(trap, handler, &[target, kv, Value::Obj(desc_id)])?;
            return if e.to_boolean(r) {
                // Invariants spec `[[DefineOwnProperty]]` : nouvelle
                // prop sur cible non-extensible → TypeError ; prop
                // non-config ou absente rapportée configurable:false.
                let tgt_exists = e.find_own(tid, key).is_some();
                let tgt_ext = e.target_extensible(tid);
                if !tgt_exists && !tgt_ext {
                    return Err(e.type_error(
                        "proxy defineProperty: cannot add prop on non-extensible target",
                    ));
                }
                Ok(true)
            } else {
                // Trap `defineProperty` → false = refus : `false`
                // pour `Reflect.defineProperty`, `TypeError` pour
                // `Object.defineProperty` (traduit par l'appelant).
                Ok(false)
            };
        }
        id = tid;
        e.deleg_hop(&mut depth)?;
    }
    let desc = Value::Obj(desc_id);
    // `HasProperty` sur la chaîne entière (spec
    // `ToPropertyDescriptor` — un champ hérité du proto du
    // descripteur compte : `Object/create/15.2.3.5-4-102`,
    // `descObj` hérite `configurable:true` de son proto).
    let (has_value, has_writable, has_enumerable, has_configurable, has_get, has_set) = (
        e.has_prop(desc_id, "value")?,
        e.has_prop(desc_id, "writable")?,
        e.has_prop(desc_id, "enumerable")?,
        e.has_prop(desc_id, "configurable")?,
        e.has_prop(desc_id, "get")?,
        e.has_prop(desc_id, "set")?,
    );
    let cur = e.find_own(id, key);
    // Flags absents du descripteur → valeur de la prop existante,
    // `false` sinon (spec `[[DefineOwnProperty]]`).
    let enumerable = if has_enumerable {
        let v = e.get_prop_value(desc.clone(), "enumerable")?;
        e.to_boolean(v)
    } else {
        cur.as_ref()
            .is_some_and(super::super::heap::Prop::enumerable)
    };
    let configurable = if has_configurable {
        let v = e.get_prop_value(desc.clone(), "configurable")?;
        e.to_boolean(v)
    } else {
        cur.as_ref()
            .is_some_and(super::super::heap::Prop::configurable)
    };
    let writable = if has_writable {
        let v = e.get_prop_value(desc.clone(), "writable")?;
        e.to_boolean(v)
    } else {
        cur.as_ref()
            .is_some_and(|p| matches!(p, crate::heap::Prop::Data { attrs, .. } if attrs.writable))
    };
    let get = e.get_prop_value(desc.clone(), "get")?;
    let set = e.get_prop_value(desc.clone(), "set")?;
    let has_acc = has_get || has_set;
    let has_data = has_value || has_writable;
    if has_acc && has_data {
        return Err(e.type_error("descriptor cannot be both data and accessor"));
    }
    if has_acc && !matches!(get, Value::Undefined) && !e.is_callable(get.clone())
        || has_acc && !matches!(set, Value::Undefined) && !e.is_callable(set.clone())
    {
        return Err(e.type_error("accessor descriptor `get`/`set` must be callable"));
    }
    // `IntegerIndexedDefineOwnProperty` (JS-C606) — un index
    // canonique sur une vue typée n'accepte que le descripteur
    // intrinsèque `{w:true,e:true,c:true}` : accesseur, flag
    // contraire (`configurable:false`/`enumerable:false`/
    // `writable:false` demandés) ou index invalide **vivant**
    // (`-0`, `-1`, `1.5`, `NaN`, hors-bornes, buffer détaché ou
    // région rétrécie) → refus `false`. `[[Value]]` présent →
    // écriture d'élément via `set_prop` (coercition
    // `ToNumber`/`ToBigInt` incluse — abrupts propagés).
    let ta_v = match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::TypedArray {
            ta,
            buffer,
            offset,
            len,
            auto,
        }) => Some((*ta, buffer.clone(), *offset, *len, *auto)),
        _ => None,
    };
    if let Some((ta, buffer, offset, len, auto)) = ta_v {
        if let Some(n) = crate::interp::canonical_numeric_index(key) {
            if has_acc
                || (has_configurable && !configurable)
                || (has_enumerable && !enumerable)
                || crate::interp::ta_live_index(&e.heap, ta, &buffer, offset, len, auto, n)
                    .is_none()
                || (has_writable && !writable)
            {
                return Ok(false);
            }
            if has_value {
                let v = e.get_prop_value(desc.clone(), "value")?;
                e.set_prop(id, key, v)?;
            }
            return Ok(true);
        }
    }
    let attrs = crate::heap::PropAttrs {
        writable,
        enumerable,
        configurable,
    };
    let v = if has_value {
        e.get_prop_value(desc.clone(), "value")?
    } else {
        cur.clone()
            .and_then(|p| p.data_value())
            .unwrap_or(Value::Undefined)
    };
    // Côté absent : getter/setter existant conservé.
    let (g, s) = match &cur {
        Some(crate::heap::Prop::Acc { get: g, set: s, .. }) => (
            if has_get { get } else { g.clone() },
            if has_set { set } else { s.clone() },
        ),
        _ => (get, set),
    };
    // Indice de tableau : l'élément vit dans `ObjKind::Array`,
    // pas dans `props` — route dédiée. Les attributs posés sont
    // contraints par l'intégrité (seal/freeze) : un descripteur
    // incompatible est un `TypeError` (spec [[DefineOwnProperty]]).
    // `defineProperty(arr, 'length', …)` — `length` est synthétisée,
    // pas dans `props` : route dédiée (valeur → resize, verrou
    // `writable:false` → bit 4, le reste est fixe).
    if key == "length" {
        let is_arr = matches!(e.heap.obj(id).map(|o| &o.kind), Some(ObjKind::Array(_)));
        if is_arr {
            // Spec `ArraySetLength` : `[[Value]]` est coercée/
            // validée AVANT toute vérification de compatibilité
            // du descripteur — `valueOf` s'exécute deux fois
            // (`ToUint32` puis `ToNumber`), `TypeError` propage,
            // divergence → `RangeError`.
            let new_len = if has_value {
                let u = crate::value::to_uint32(e.to_number(v.clone())?);
                let num = e.to_number(v.clone())?;
                #[allow(clippy::float_cmp)]
                if u as f64 != num {
                    return Err(e.range_error("Invalid array length"));
                }
                Some(u)
            } else {
                None
            };
            // `oldLenDesc` est lu APRÈS la coercition : `valueOf`
            // peut avoir muté l'array (verrouillé `length`,
            // scellé, redimensionné) — l'état post-coercion
            // gouverne les refus (`define-own-prop-length-
            // coercion-order.js`).
            let (integ, len) = e.heap.obj(id).map_or((0, 0), |o| {
                let l = match &o.kind {
                    ObjKind::Array(elems) => elems.len() as usize,
                    _ => 0,
                };
                (o.elem_integrity, l)
            });
            if integ & 4 != 0 {
                // `length` verrouillée : valeur modifiée,
                // `writable:true`, accessor, `e`/`c` → refus.
                if has_writable && writable
                    || has_acc
                    || (has_enumerable && enumerable)
                    || (has_configurable && configurable)
                    || new_len.is_some_and(|n| n as usize != len)
                {
                    return Ok(false);
                }
                return Ok(true);
            }
            // Compatibilité du descripteur : `length` est data
            // `{¬e,¬c}` — accessor, `enumerable`/`configurable`
            // `true` → refus (`Ok(false)` → `TypeError`).
            if has_acc || (has_enumerable && enumerable) || (has_configurable && configurable) {
                return Ok(false);
            }
            if let Some(n) = new_len {
                // `writable:false` s'applique AVANT la suppression
                // d'éléments (spec `ArraySetLength` : le verrou
                // survit à l'échec partiel —
                // `defineProperties/15.2.3.7-6-a-164/176`).
                if has_writable && !writable {
                    if let Some(o) = e.heap.obj_mut(id) {
                        o.elem_integrity |= 4;
                    }
                }
                let mut bl = n;
                let blocked;
                if (n as usize) < len {
                    // Suppression décroissante des index `>= n`,
                    // arrêt au premier non-supprimable (accesseur
                    // indexé ou élément `configurable:false` —
                    // `4-168`, `4-174`) ; `length` retombe sur
                    // blocker+1 puis `TypeError`. Les trous se
                    // suppriment librement.
                    let (bl2, doomed, blk) = e.heap.obj(id).map_or((n, Vec::new(), false), |o| {
                        let bl2 = n;
                        let mut doomed = Vec::new();
                        let mut i = len as u32;
                        while i > n {
                            i -= 1;
                            if o.elem_integrity & 8 != 0 {
                                if let Some(p) = o.find(&i.to_string()) {
                                    if p.configurable() {
                                        doomed.push(i.to_string());
                                        continue;
                                    }
                                    return (i + 1, doomed, true);
                                }
                            }
                            if let ObjKind::Array(elems) = &o.kind {
                                if elems.has(i) {
                                    let ok = elems.attrs_of(i).is_none_or(|a| a.configurable)
                                        && o.elem_integrity.trailing_zeros() >= 2;
                                    if ok {
                                        continue;
                                    }
                                    return (i + 1, doomed, true);
                                }
                            }
                        }
                        (bl2, doomed, false)
                    });
                    bl = bl2;
                    blocked = blk;
                    if let Some(o) = e.heap.obj_mut(id) {
                        for k in &doomed {
                            o.remove_own(k);
                        }
                        if let Some((_, map)) = o.args_map.as_mut() {
                            if let Some(m) = std::rc::Rc::get_mut(map) {
                                for i in bl..len as u32 {
                                    if let Some(entry) = m.get_mut(i as usize) {
                                        *entry = None;
                                    }
                                }
                            }
                        }
                        if let ObjKind::Array(elems) = &mut o.kind {
                            elems.set_len(bl);
                        }
                    }
                    if blocked {
                        return Ok(false);
                    }
                } else {
                    // Croissance sparse : aucune densification —
                    // `len` est découplée du backing (JS-C613).
                    if let Some(ObjKind::Array(elems)) = e.heap.obj_mut(id).map(|o| &mut o.kind) {
                        elems.set_len(bl);
                    }
                }
            }
            if has_writable && !writable {
                if let Some(o) = e.heap.obj_mut(id) {
                    o.elem_integrity |= 4;
                }
            }
            return Ok(true);
        }
    }
    if let Some(i) = crate::interp::array_index(key) {
        let is_arr = matches!(e.heap.obj(id).map(|o| &o.kind), Some(ObjKind::Array(_)));
        if is_arr {
            let (integ, len, ext, cur_v) = e
                .heap
                .obj(id)
                .map(|o| {
                    if let ObjKind::Array(elems) = &o.kind {
                        (
                            o.elem_integrity,
                            elems.len() as usize,
                            o.extensible,
                            elems.get(i).cloned(),
                        )
                    } else {
                        (0, 0, false, None)
                    }
                })
                .unwrap_or_default();
            // Descripteur « générique » (ni `value`/`writable` ni
            // `get`/`set`) sur un accesseur d'index existant : le
            // type est préservé — traité comme un descripteur
            // d'accesseur (spec `[[DefineOwnProperty]]` —
            // `4-271` : `{enumerable:false}` sur un set-accessor
            // d'index garde le setter).
            let treat_as_acc =
                has_acc || (matches!(cur, Some(crate::heap::Prop::Acc { .. })) && !has_data);
            if treat_as_acc {
                // Spec `[[DefineOwnProperty]]` : un accesseur est
                // légal sur un index d'array — `ArraySetLength`
                // ne s'applique qu'aux data. Sous seal/freeze les
                // éléments sont non-configurables et la cible
                // non-extensible → refus ; sinon l'accesseur vit
                // dans `props`, ombre l'élément via le bit 8 de
                // `elem_integrity` (JS-C627, `array-key-get-error`).
                // Type-switch data→acc sur un index
                // non-configurable → refus (spec — `4-241`,
                // `4-339`) ; la compat acc-sur-acc (flags figés,
                // get/set `SameValue`) est tranchée par
                // `define_accessor_strict`.
                if integ & 3 != 0
                    || matches!(cur, Some(crate::heap::Prop::Data { ref attrs, .. }) if !attrs.configurable)
                {
                    return Ok(false);
                }
                return Ok(e.heap.obj_mut(id).is_some_and(|o| {
                    // Spec `ArrayDefineOwnProperty` — index
                    // `>= oldLen` sur `length` non-writable →
                    // refus, même pour un accesseur.
                    if i >= len && (!o.extensible || o.elem_integrity & 4 != 0) {
                        return false;
                    }
                    if !o.define_accessor_strict(key, g, s, attrs) {
                        return false;
                    }
                    if let ObjKind::Array(elems) = &mut o.kind {
                        // Une seule own-prop : l'élément disparaît.
                        // `i >= len` → `length` devient `i + 1`
                        // (spec — l'accesseur est alors atteint
                        // par l'itération, `array-key-get-error`).
                        elems.delete(i as u32);
                        if i >= len {
                            elems.set_len(i as u32 + 1);
                        }
                    }
                    // `arguments` mappé : un accesseur sur l'index
                    // SUPPRIME le mapping paramètre (spec 10.6
                    // [[DefineOwnProperty]] 5.a.i — `4-325-1`).
                    if let Some((_, map)) = o.args_map.as_mut() {
                        if let Some(m) = std::rc::Rc::get_mut(map).and_then(|m| m.get_mut(i)) {
                            *m = None;
                        }
                    }
                    o.elem_integrity |= 8;
                    true
                }));
            }
            // JS-C644 — compatibilité `[[DefineOwnProperty]]` sur
            // élément. `cur` porte les attributs effectifs
            // (table `Arr::attrs_of` sous plancher d'intégrité —
            // `find_own_ai`) : non-configurable ⇒ flags figés ;
            // `writable:false` ⇒ la valeur doit être `SameValue`
            // (`NaN` égale `NaN`, `-0` diffère de `+0` —
            // `Object/create/15.2.3.5-4-102`, `…6-4-215`).
            match &cur {
                Some(crate::heap::Prop::Data { attrs: ca, .. }) => {
                    if !ca.configurable
                        && ((has_configurable && configurable)
                            || (has_enumerable && enumerable != ca.enumerable)
                            || (has_writable && writable && !ca.writable)
                            || (!ca.writable
                                && has_value
                                && !crate::interp::same_value(
                                    &v,
                                    cur_v.as_ref().unwrap_or(&Value::Undefined),
                                    &e.heap,
                                )))
                    {
                        return Ok(false);
                    }
                }
                Some(crate::heap::Prop::Acc { attrs: ca, .. }) => {
                    // Ombre-acc (bit 8) + descripteur data explicite
                    // : le switch de type exige un accesseur
                    // configurable (spec), puis l'élément revit.
                    if !ca.configurable {
                        return Ok(false);
                    }
                    if let Some(o) = e.heap.obj_mut(id) {
                        o.remove_own(key);
                    }
                }
                _ => {
                    // Index absent (hors-bornes ou trou) : cible
                    // non-extensible ou `length` verrouillée au-delà
                    // → refus (`ArraySetLength`, spec).
                    if !ext || (i >= len && integ & 4 != 0) {
                        return Ok(false);
                    }
                }
            }
            // `[[DefineOwnProperty]]` — écriture own directe de
            // l'élément : jamais `[[Set]]` (les accesseurs de la
            // chaîne proto sont invisibles ici, JS-C615). Valeur
            // posée quand le descripteur la porte OU quand la prop
            // est créée (absente ⇒ `undefined`, spec) — `v` a déjà
            // été fusionné (valeur courante sinon `undefined`), la
            // réécriture d'une valeur conservée est sans effet.
            // Pas de garde d'alloc (JS-C613) : les indices hauts
            // vivent dans la map sparse de `Arr`. Les attributs
            // fusionnés sont persistés dans la table par index.
            // `arguments` mappé : le descripteur data écrit À
            // TRAVERS le mapping — le paramètre formel prend la
            // valeur (spec 10.6 5.a.i — `4-333-11` : `obj[0]` lit
            // `2010` en passant par l'env). Le mapping survit.
            let mapped = e.heap.obj(id).and_then(|o| {
                o.args_map.as_ref().and_then(|(env, map)| {
                    map.get(i).and_then(|n| n.clone().map(|n| (env.clone(), n)))
                })
            });
            // `writable:false` demandé sur un index mappé → le
            // mapping est SUPPRIMÉ (spec 10.6 — `4-293-3` :
            // `a` garde la valeur écrite, le paramètre se
            // désynchronise de `arguments[i]`).
            let drop_mapping = mapped.is_some() && has_writable && !writable;
            if let Some(o) = e.heap.obj_mut(id) {
                if let ObjKind::Array(elems) = &mut o.kind {
                    elems.set(i as u32, v.clone());
                    elems.set_attrs(i as u32, attrs);
                }
                if drop_mapping {
                    if let Some((_, map)) = o.args_map.as_mut() {
                        if let Some(m) = std::rc::Rc::get_mut(map).and_then(|m| m.get_mut(i)) {
                            *m = None;
                        }
                    }
                }
            }
            if let Some((env, name)) = mapped {
                if has_value {
                    e.env_set(&env, &name, v)?;
                }
            }
            return Ok(true);
        }
    }
    // Descripteur générique (ni `value`/`writable` ni `get`/`set`)
    // sur un accessor existant : la spec ne change que les flags
    // présents — le type accesseur est préservé, pas converti en
    // data `Undefined`.
    let want_acc = has_acc || (!has_data && matches!(cur, Some(crate::heap::Prop::Acc { .. })));
    let ok = e.heap.obj_mut(id).is_some_and(|o| {
        if want_acc {
            // `g`/`s` sont les valeurs FINALES (le merge côté-absent
            // est fait plus haut) — `Undefined` vide ce côté (spec
            // `[[DefineOwnProperty]]`), pas « garder » : route
            // stricte, sinon un `P.get = void 0` suivi de
            // `defineProperty` laisserait survivre le vieux getter
            // (décorateur `@computed` Polymer — JS-C313, §B-4(c)).
            o.define_accessor_strict(key, g, s, attrs)
        } else {
            o.define_data(key, v, attrs)
        }
    });
    if !ok {
        return Ok(false);
    }
    Ok(true)
}

/// `Object.defineProperties(o, propsMap)` — chaque propre prop
/// énumérable de la map est un descripteur appliqué à `o`.
pub(crate) fn object_define_properties(
    e: &mut Engine,
    _t: Value,
    args: &[Value],
) -> JsResult<Value> {
    let target @ Value::Obj(id) = arg(args, 0) else {
        return Err(e.type_error("Object.defineProperties on non-object"));
    };
    define_props_from_map(e, id, arg(args, 1))?;
    Ok(target)
}

/// Map de descripteurs → définitions (ordre d'insertion conservé).
/// Chaque phase qui jette est journalisée (sonde JS-C313) — la
/// composition de mixin se coupe ici à mi-itération.
pub(crate) fn define_props_from_map(e: &mut Engine, id: ObjId, map: Value) -> JsResult<()> {
    // `ToObject(Properties)` (spec `ObjectDefineProperties`) :
    // `defineProperties(o, false)` → wrapper Boolean frais —
    // aucune clé propre énumérable → no-op silencieux
    // (`Object/defineProperties/15.2.3.7-2-3`, JS-C644).
    let mid = match map {
        Value::Obj(mid) => mid,
        Value::Null | Value::Undefined | Value::Hole => {
            return Err(e.type_error("Cannot convert undefined or null to object"));
        }
        prim => {
            let proto = match &prim {
                Value::Num(_) => e.protos.number,
                Value::Str(_) => e.protos.string,
                Value::Bool(_) => e.protos.boolean,
                Value::Big(_) => e.protos.bigint,
                _ => return Err(e.type_error("property map must be an object")),
            };
            let wid = e.new_object();
            if let Some(o) = e.heap.obj_mut(wid) {
                o.proto = Value::Obj(proto);
            }
            e.set_own(wid, crate::builtins::ctors::PRIM, prim, false);
            wid
        }
    };
    for k in e.own_enum_keys_trapped(mid)? {
        let dv = match e.get_prop_value(Value::Obj(mid), &k) {
            Ok(dv) => dv,
            Err(t) => {
                crate::defprop_note(format!(
                    "defprops #{id} `{k}` read-desc → {}",
                    defprop_err_summary(e, t.0.clone())
                ));
                return Err(t);
            }
        };
        let Value::Obj(did) = dv else {
            crate::defprop_note(format!("defprops #{id} `{k}` desc non-object"));
            return Err(e.type_error(format!("descriptor for `{k}` must be an object")));
        };
        if let Some(ObjKind::Symbol { .. }) = e.heap.obj(did).map(|o| &o.kind) {
            return Err(e.type_error(format!("descriptor for `{k}` must be an object")));
        }
        if !define_one_prop(e, id, &k, did)? {
            return Err(e.type_error(format!("cannot define property `{k}`")));
        }
    }
    Ok(())
}

/// `Object.getOwnPropertyDescriptor(o, k)` — `{value, writable,
/// enumerable, configurable}` ou `{get, set, enumerable,
/// configurable}`, `undefined` quand la prop est absente.
pub(crate) fn object_get_own_property_descriptor(
    e: &mut Engine,
    _t: Value,
    args: &[Value],
) -> JsResult<Value> {
    let target = arg(args, 0);
    let key = e.prop_key(arg(args, 1))?;
    // Descripteurs propres de la string primitive (spec
    // `ToObject` + `StringGetOwnProperty`).
    if let Value::Str(sid) = target {
        let n = sid.u16_len();
        let d = e.new_object();
        if key == "length" {
            e.set_own(d, "value", Value::Num(n as f64), true);
            e.set_own(d, "writable", Value::Bool(false), true);
            e.set_own(d, "enumerable", Value::Bool(false), true);
            e.set_own(d, "configurable", Value::Bool(false), true);
            return Ok(Value::Obj(d));
        }
        if let Some(i) = crate::interp::array_index(&key) {
            if i < n {
                let cv = match sid.u16_at(i) {
                    Some(w) => e.str_val_units(vec![w]),
                    None => e.str_val(""),
                };
                e.set_own(d, "value", cv, true);
                e.set_own(d, "writable", Value::Bool(false), true);
                e.set_own(d, "enumerable", Value::Bool(true), true);
                e.set_own(d, "configurable", Value::Bool(false), true);
                return Ok(Value::Obj(d));
            }
        }
        return Ok(Value::Undefined);
    }
    let Value::Obj(id) = target else {
        return Ok(Value::Undefined);
    };
    let Some(p) = e.get_own_prop(id, &key)? else {
        return Ok(Value::Undefined);
    };
    Ok(prop_desc_object(e, p))
}

/// Property descriptor object from a `Prop` — shared by
/// `Object.getOwnPropertyDescriptor` and `Reflect.*`.
pub(crate) fn prop_desc_object(e: &mut Engine, p: crate::heap::Prop) -> Value {
    let d = e.new_object();
    match p {
        crate::heap::Prop::Data { value, attrs } => {
            e.set_own(d, "value", value, true);
            e.set_own(d, "writable", Value::Bool(attrs.writable), true);
            e.set_own(d, "enumerable", Value::Bool(attrs.enumerable), true);
            e.set_own(d, "configurable", Value::Bool(attrs.configurable), true);
        }
        crate::heap::Prop::Acc { get, set, attrs } => {
            e.set_own(d, "get", get, true);
            e.set_own(d, "set", set, true);
            e.set_own(d, "enumerable", Value::Bool(attrs.enumerable), true);
            e.set_own(d, "configurable", Value::Bool(attrs.configurable), true);
        }
    }
    Value::Obj(d)
}

/// `Object.getOwnPropertyDescriptors(o)` — map `key → descriptor`
/// pour chaque propre prop (ordre d'insertion conservé).
pub(crate) fn object_get_own_property_descriptors(
    e: &mut Engine,
    _t: Value,
    args: &[Value],
) -> JsResult<Value> {
    let out = e.new_object();
    let Value::Obj(id) = arg(args, 0) else {
        return Ok(Value::Obj(out));
    };
    let keys: Vec<String> =
        if matches!(e.heap.obj(id).map(|o| &o.kind), Some(ObjKind::Proxy { .. })) {
            e.own_keys_trapped(id)?
        } else {
            match e.heap.obj(id) {
                Some(o) => {
                    let mut ks: Vec<String> = o.props.iter().map(|(k, _)| k.to_string()).collect();
                    if let ObjKind::Array(elems) = &o.kind {
                        // Éléments *présents* seuls — les trous ne
                        // sont pas des clés propres (JS-C613 :
                        // jamais `0..len` sur un backing sparse).
                        let nk = elems.present().count();
                        ks.splice(0..0, elems.present().map(|(i, _)| i.to_string()));
                        ks.insert(nk, "length".into());
                    }
                    ks
                }
                None => Vec::new(),
            }
        };
    for k in keys {
        // Clés internes/symboles sentinelles (`\u{f120}…\u{f13f}` —
        // privés, métadonnées classe, `PRIM`) : jamais exposées
        // comme descripteurs.
        if crate::builtins::bigint::is_sentinel_key(&k) {
            continue;
        }
        let kv = e.str_val(k.clone());
        let d = object_get_own_property_descriptor(e, Value::Undefined, &[arg(args, 0), kv])?;
        e.set_own(out, &k, d, true);
    }
    Ok(Value::Obj(out))
}

/// `Object.freeze` — seal + toutes data props non-writable.
/// Sur un proxy : même approximation que `seal` (trap
/// `preventExtensions` ou cible).
pub(crate) fn object_freeze(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    if let Value::Obj(id) = arg(args, 0) {
        if matches!(e.heap.obj(id).map(|o| &o.kind), Some(ObjKind::Proxy { .. })) {
            return object_prevent_extensions(e, Value::Undefined, &[arg(args, 0)]);
        }
        if let Some(o) = e.heap.obj_mut(id) {
            // `freeze` sur vue typée non vide → TypeError (spec).
            if let ObjKind::TypedArray { len, .. } = &o.kind {
                if *len > 0 {
                    let err = e.type_error("cannot freeze a typed array");
                    return Err(err);
                }
            }
            o.extensible = false;
            o.elem_integrity |= 6;
            for (_, p) in &mut o.props {
                match p {
                    crate::heap::Prop::Data { attrs, .. } => {
                        attrs.configurable = false;
                        attrs.writable = false;
                    }
                    crate::heap::Prop::Acc { attrs, .. } => attrs.configurable = false,
                }
            }
        }
    }
    Ok(arg(args, 0))
}

pub(crate) fn object_is_frozen(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    Ok(Value::Bool(match arg(args, 0) {
        Value::Obj(id) => e.heap.obj(id).is_some_and(|o| {
            !o.extensible
                && o.props
                    .iter()
                    .all(|(_, p)| !p.configurable() && !p.writable())
                && match &o.kind {
                    ObjKind::Array(elems) => elems.is_empty() || o.elem_integrity & 6 == 6,
                    // Les éléments d'une vue typée restent
                    // `writable:true` → jamais frozen non vide
                    // (spec `TestIntegrityLevel`, JS-C606).
                    ObjKind::TypedArray { len, .. } => *len == 0,
                    _ => true,
                }
        }),
        _ => true, // primitives are frozen by nature
    }))
}

/// `Object.is` — `SameValue` (`===` plus NaN and ±0 distinction).
pub(crate) fn object_is(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let (a, b) = (arg(args, 0), arg(args, 1));
    if let (Value::Num(x), Value::Num(y)) = (&a, &b) {
        if x.is_nan() && y.is_nan() {
            return Ok(Value::Bool(true));
        }
        if *x == 0.0 && *y == 0.0 {
            return Ok(Value::Bool(x.is_sign_negative() == y.is_sign_negative()));
        }
    }
    Ok(Value::Bool(e.strict_eq(&a, &b)))
}
