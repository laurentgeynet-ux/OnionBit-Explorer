// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Builtins: global functions, constructors and prototype methods.
//!
//! No `eval`/`Function` ctor (parser isn't reentrant yet); `Date`
//! formatting stays at `getTime`. Regexes exist (`RegExp`, literals,
//! `match`/`replace`/`split`/`search`) via the pure-Rust `regex`
//! crate — lookahead/backrefs surface as `SyntaxError` (regexp.rs).
//! Everything installed non-enumerable so `for-in` stays clean.

use crate::heap::{JsResult, NativeFn, ObjKind, Object, Thrown};
use crate::value::{num_to_js_string, to_int32, to_uint32, ObjId, Value};
use crate::Engine;
use std::time::{SystemTime, UNIX_EPOCH};

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).copied().unwrap_or(Value::Undefined)
}

/// Wire `Ctor.prototype = proto` and `proto.constructor = Ctor`.
fn ctor(e: &mut Engine, name: &'static str, f: NativeFn, proto: ObjId) -> Value {
    let c = e.native_fn(name, f);
    e.set_own(c_id(c), "prototype", Value::Obj(proto), false);
    e.set_own(proto, "constructor", c, false);
    c
}

fn c_id(v: Value) -> ObjId {
    match v {
        Value::Obj(id) => id,
        _ => 0,
    }
}

/// Install the whole builtin surface on `e`'s global object.
pub(crate) fn install(e: &mut Engine) {
    let g = e.global;
    e.set_const(g, "undefined", Value::Undefined);
    e.set_const(g, "NaN", Value::Num(f64::NAN));
    e.set_const(g, "Infinity", Value::Num(f64::INFINITY));
    e.set_own(g, "window", Value::Obj(g), false);
    e.set_own(g, "self", Value::Obj(g), false);
    e.set_own(g, "globalThis", Value::Obj(g), false);

    // `@@toStringTag` (spec) — prototypes concrets, namespaces et
    // `globalThis`. Lus par `obj[Symbol.toStringTag]` et par
    // `Object.prototype.toString`. Les sous-classes d'`Error`
    // héritent le tag `"Error"` (spec).
    for (pid, tag) in [
        (e.protos.string, "String"),
        (e.protos.number, "Number"),
        (e.protos.boolean, "Boolean"),
        (e.protos.bigint, "BigInt"),
        (e.protos.symbol, "Symbol"),
        (e.protos.error, "Error"),
        (e.protos.date, "Date"),
        (e.protos.regexp, "RegExp"),
        (e.protos.promise, "Promise"),
        (e.protos.map, "Map"),
        (e.protos.set, "Set"),
        (e.protos.iterator, "Iterator"),
        (e.protos.array_buffer, "ArrayBuffer"),
        (e.protos.data_view, "DataView"),
        (e.protos.typed_array, "TypedArray"),
        (e.global, "globalThis"),
    ] {
        let t = e.str_val(tag);
        e.set_own(pid, crate::interp::SYM_TO_STRING_TAG, t, false);
    }

    e.add_method(g, "parseInt", parse_int);
    e.add_method(g, "parseFloat", parse_float);
    e.add_method(g, "isNaN", is_nan);
    e.add_method(g, "isFinite", is_finite);
    e.add_method(g, "escape", escape);
    e.add_method(g, "unescape", unescape);
    e.add_method(g, "encodeURIComponent", encode_uri_component);
    e.add_method(g, "decodeURIComponent", decode_uri_component);
    e.add_method(g, "btoa", btoa);
    e.add_method(g, "atob", atob);
    e.add_method(g, "encodeURI", encode_uri);
    e.add_method(g, "decodeURI", decode_uri);
    // `eval(src)` — global scope (indirect-eval semantics; the
    // caller's local scope is out of reach — documented deviation).
    e.add_method(g, "eval", eval_fn);
    e.add_method(g, "structuredClone", structured_clone);

    let (p_obj, p_arr, p_str, p_num, p_bool, p_err, p_terr, p_rerr, p_serr, p_date, p_fun) = {
        let p = &e.protos;
        (
            p.object,
            p.array,
            p.string,
            p.number,
            p.boolean,
            p.error,
            p.type_error,
            p.reference_error,
            p.syntax_error,
            p.date,
            p.function,
        )
    };
    let object = ctor(e, "Object", object_ctor, p_obj);
    let array = ctor(e, "Array", array_ctor, p_arr);
    let string = ctor(e, "String", string_ctor, p_str);
    let number = ctor(e, "Number", number_ctor, p_num);
    let boolean = ctor(e, "Boolean", boolean_ctor, p_bool);
    // Les prototypes sont eux-mêmes des wrappers spec :
    // `String.prototype` ≙ `new String("")`, etc.
    let empty = e.str_val("");
    e.set_own(p_str, PRIM, empty, false);
    e.set_own(p_num, PRIM, Value::Num(0.0), false);
    e.set_own(p_bool, PRIM, Value::Bool(false), false);
    let error = ctor(e, "Error", error_ctor, p_err);
    if let Value::Obj(eid) = error {
        e.add_method(eid, "captureStackTrace", error_capture_stack);
    }
    let t_err = ctor(e, "TypeError", type_error_ctor, p_terr);
    let r_err = ctor(e, "ReferenceError", reference_error_ctor, p_rerr);
    let s_err = ctor(e, "SyntaxError", syntax_error_ctor, p_serr);
    let a_err = ctor(
        e,
        "AggregateError",
        aggregate_error_ctor,
        e.protos.aggregate_error,
    );
    // Spec : `AggregateError.prototype.name` = "AggregateError"
    // (propre, pas hérité d'`Error.prototype`).
    {
        let an = e.str_val("AggregateError");
        e.set_own(e.protos.aggregate_error, "name", an, false);
    }
    let date = ctor(e, "Date", date_ctor, p_date);

    // `URL`/`URLSearchParams` (WHATWG subset — JS-C16) : parsing
    // absolu + résolution relative basique, pas d'IDNA/punycode.
    let url = ctor(e, "URL", url_ctor, e.protos.url);
    let usp = ctor(e, "URLSearchParams", usp_ctor, e.protos.url_params);
    e.set_own(g, "URL", url, false);
    e.set_own(g, "URLSearchParams", usp, false);
    // `Iterator` — ctor abstrait (spec : `new Iterator()` lève) ;
    // `Iterator.prototype` = protos.iterator, `Iterator.from`.
    {
        let iter = e.native_fn("Iterator", iterator_ctor);
        if let Value::Obj(iid) = iter {
            e.set_own(iid, "prototype", Value::Obj(e.protos.iterator), false);
            e.add_method(iid, "from", iterator_from);
        }
        e.set_own(e.protos.iterator, "constructor", iter, false);
        e.set_own(g, "Iterator", iter, false);
    }

    // `crypto` (WHATWG subset) — `getRandomValues`/`randomUUID`
    // sur le PRNG moteur (xorshift64 — pas cryptographique,
    // déviation documentée).
    {
        let crypto = e.new_object();
        e.add_method(crypto, "getRandomValues", crypto_get_random_values);
        e.add_method(crypto, "randomUUID", crypto_random_uuid);
        let tag = e.str_val("Crypto");
        e.set_own(crypto, crate::interp::SYM_TO_STRING_TAG, tag, false);
        e.set_own(g, "crypto", Value::Obj(crypto), false);
    }

    // `SuppressedError` (ES2026) — `error`/`suppressed`/`message`.
    {
        let se_p = e
            .heap
            .alloc_obj(crate::heap::Object::ordinary(Value::Obj(e.protos.error)));
        let se = ctor(e, "SuppressedError", suppressed_error_ctor, se_p);
        e.set_own(g, "SuppressedError", se, false);
    }

    // `DisposableStack` (ES2026) — gestion explicite de ressources
    // (`using` syntaxe non implémentée ; l'API impérative est
    // complète : `use`/`adopt`/`defer`/`dispose`/`move`/`disposed`
    // + `@@dispose`/`@@toStringTag`). `AsyncDisposableStack` —
    // reportée (nécessite le runtime async).
    {
        let ds_p = e.new_object();
        let ds = ctor(e, "DisposableStack", disposable_stack_ctor, ds_p);
        let dg = e.native_fn("get disposed", ds_disposed_get);
        if let Some(o) = e.heap.obj_mut(ds_p) {
            o.set_accessor("disposed", dg, Value::Undefined, false);
        }
        e.add_method(ds_p, "use", ds_use);
        e.add_method(ds_p, "adopt", ds_adopt);
        e.add_method(ds_p, "defer", ds_defer);
        e.add_method(ds_p, "dispose", ds_dispose);
        e.add_method(ds_p, "move", ds_move);
        e.add_method(ds_p, crate::interp::SYM_DISPOSE, ds_dispose);
        let tag = e.str_val("DisposableStack");
        e.set_own(ds_p, crate::interp::SYM_TO_STRING_TAG, tag, false);
        e.set_own(g, "DisposableStack", ds, false);
    }

    {
        let up = e.protos.url;
        e.add_method(up, "toString", url_href_get);
        e.add_method(up, "toJSON", url_href_get);
        let pp = e.protos.url_params;
        e.add_method(pp, "get", usp_get);
        e.add_method(pp, "getAll", usp_get_all);
        e.add_method(pp, "has", usp_has);
        e.add_method(pp, "set", usp_set);
        e.add_method(pp, "append", usp_append);
        e.add_method(pp, "delete", usp_delete);
        e.add_method(pp, "sort", usp_sort);
        e.add_method(pp, "forEach", usp_for_each);
        e.add_method(pp, "toString", usp_to_string);
        e.add_method(pp, "keys", usp_keys);
        e.add_method(pp, "values", usp_values);
        e.add_method(pp, "entries", usp_entries);
        let it = e.native_fn("entries", usp_entries);
        e.set_own(pp, crate::interp::SYM_ITERATOR, it, false);
        let tag = e.str_val("URLSearchParams");
        e.set_own(pp, crate::interp::SYM_TO_STRING_TAG, tag, false);
        let tag = e.str_val("URL");
        e.set_own(up, crate::interp::SYM_TO_STRING_TAG, tag, false);
    }
    e.set_own(g, "Object", object, false);
    e.set_own(g, "Array", array, false);
    e.set_own(g, "String", string, false);
    e.set_own(g, "Number", number, false);
    e.set_own(g, "Boolean", boolean, false);
    e.set_own(g, "Error", error, false);
    e.set_own(g, "TypeError", t_err, false);
    e.set_own(g, "ReferenceError", r_err, false);
    e.set_own(g, "SyntaxError", s_err, false);
    e.set_own(g, "AggregateError", a_err, false);
    e.set_own(g, "Date", date, false);

    // `WeakRef`/`FinalizationRegistry` (JS-C13) — vraies
    // références faibles : les cibles ne sont pas tracées par le
    // GC ; le cleanup part en microtask après collecte.
    let wrp = e.new_object();
    e.add_method(wrp, "deref", weakref_deref);
    let tag = e.str_val("WeakRef");
    e.set_own(wrp, crate::interp::SYM_TO_STRING_TAG, tag, false);
    let wr_ctor = ctor(e, "WeakRef", weakref_ctor, wrp);
    e.set_own(g, "WeakRef", wr_ctor, false);
    let frp = e.new_object();
    e.add_method(frp, "register", finreg_register);
    e.add_method(frp, "unregister", finreg_unregister);
    let tag = e.str_val("FinalizationRegistry");
    e.set_own(frp, crate::interp::SYM_TO_STRING_TAG, tag, false);
    let fr_ctor = ctor(e, "FinalizationRegistry", finreg_ctor, frp);
    e.set_own(g, "FinalizationRegistry", fr_ctor, false);

    // `BigInt` (JS-C7) — ctor fonctionnel (`new` → TypeError),
    // `asIntN`/`asUintN`, proto `toString`/`valueOf`. Le proto est
    // un wrapper de `0n` comme les autres primitives.
    let p_big = e.protos.bigint;
    let zero = e.big_val(num_bigint::BigInt::from(0));
    e.set_own(p_big, PRIM, zero, false);
    e.add_method(p_big, "toString", bigint_to_string);
    e.add_method(p_big, "valueOf", bigint_value_of);
    // Repli locale (déviation documentée — pas de CLDR).
    e.add_method(p_big, "toLocaleString", bigint_to_string);
    let big = ctor(e, "BigInt", bigint_ctor, p_big);
    if let Value::Obj(bid) = big {
        e.add_method(bid, "asIntN", bigint_as_int_n);
        e.add_method(bid, "asUintN", bigint_as_uint_n);
    }
    e.set_own(g, "BigInt", big, false);

    // `Function(p1,…,body)` ctor + `Symbol` + `Reflect` (JS-7f).
    let function_v = ctor(e, "Function", function_ctor, p_fun);
    e.set_own(g, "Function", function_v, false);
    // `%GeneratorFunction%`/`%AsyncGeneratorFunction%` — ctors
    // cachés (pas de global, spec) : own `constructor` des
    // générateurs, délèguent à `Function` en approximation.
    e.gen_ctor = e.native_fn("GeneratorFunction", function_ctor);
    e.async_gen_ctor = e.native_fn("AsyncGeneratorFunction", function_ctor);
    // Chaîne prototype (spec) :
    //   %GeneratorFunction%.prototype          = %GeneratorFunction.prototype% (gfp)
    //   gfp.constructor                        = %GeneratorFunction%
    //   gfp.prototype                          = %Generator% (gp)
    //   gfp.[[Prototype]]                      = %Function.prototype%
    //   gp.constructor                         = gfp
    //   gp.[[Prototype]]                       = %IteratorPrototype%
    //   gp[@@toStringTag]                      = "Generator"
    // Même forme pour `AsyncGeneratorFunction` ; `%AsyncGenerator%`
    // hérite `%IteratorPrototype%` par approximation (déviation —
    // pas d'`%AsyncIteratorPrototype%` distinct).
    let fn_proto = Value::Obj(e.protos.function);
    let it_proto = Value::Obj(e.protos.iterator);
    for (ctor_v, tag) in [
        (e.gen_ctor, "Generator"),
        (e.async_gen_ctor, "AsyncGenerator"),
    ] {
        let gfp_v = Value::Obj(e.heap.alloc_obj(crate::heap::Object::ordinary(fn_proto)));
        let gp_v = Value::Obj(e.heap.alloc_obj(crate::heap::Object::ordinary(it_proto)));
        if let (Value::Obj(gfp), Value::Obj(gp)) = (gfp_v, gp_v) {
            e.set_own(gfp, "constructor", ctor_v, false);
            e.set_own(gfp, "prototype", gp_v, false);
            e.set_own(gp, "constructor", gfp_v, false);
            let t = e.str_val(tag);
            e.set_own(gp, crate::interp::SYM_TO_STRING_TAG, t, false);
            let ft = e.str_val(format!("{tag}Function"));
            e.set_own(gfp, crate::interp::SYM_TO_STRING_TAG, ft, false);
            if let Value::Obj(cid) = ctor_v {
                e.set_own(cid, "prototype", gfp_v, false);
            }
            if tag == "Generator" {
                e.gen_fn_proto = gfp_v;
                e.gen_proto = gp_v;
            } else {
                e.async_gen_fn_proto = gfp_v;
                e.async_gen_proto = gp_v;
            }
        }
    }
    // `Symbol` — son propre prototype (`protos.symbol`) ; passer
    // `p_obj` écraserait `Object.prototype.constructor`.
    let sym = ctor(e, "Symbol", symbol_ctor, e.protos.symbol);
    if let Value::Obj(sid) = sym {
        // Well-known symbols : de vraies `ObjKind::Symbol` dont la
        // clé-propriété est la sentinelle `Symbol.<name>` — les
        // props symbolées posées par le moteur restent atteignables
        // via `o[Symbol.x]` (`typeof Symbol.x` = "symbol", spec).
        let ws = |name: &str, e: &mut Engine| {
            let key = format!("\u{f130}Symbol.{name}");
            let v = e.new_symbol_key(Some(format!("Symbol.{name}")), key);
            e.set_own(sid, name, v, false);
        };
        for name in [
            "iterator",
            "asyncIterator",
            "toPrimitive",
            "hasInstance",
            "toStringTag",
            "unscopables",
            "species",
            "match",
            "matchAll",
            "replace",
            "search",
            "split",
            "isConcatSpreadable",
            "dispose",
            "asyncDispose",
        ] {
            ws(name, e);
        }
        e.add_method(sid, "for", symbol_for);
        e.add_method(sid, "keyFor", symbol_key_for);
        e.add_method(e.protos.symbol, "toString", symbol_to_string);
        e.add_method(e.protos.symbol, "valueOf", symbol_value_of);
        let desc_fn = e.native_fn("get description", symbol_description);
        if let Some(o) = e.heap.obj_mut(e.protos.symbol) {
            o.set_accessor("description", desc_fn, Value::Undefined, false);
        }
    }
    e.set_own(g, "Symbol", sym, false);
    let reflect = e.new_object();
    e.add_method(reflect, "get", reflect_get);
    e.add_method(reflect, "set", reflect_set);
    e.add_method(reflect, "has", reflect_has);
    e.add_method(reflect, "ownKeys", reflect_own_keys);
    e.add_method(reflect, "apply", reflect_apply);
    e.add_method(reflect, "construct", reflect_construct);
    e.add_method(reflect, "defineProperty", reflect_define_property);
    e.add_method(reflect, "deleteProperty", reflect_delete_property);
    e.add_method(
        reflect,
        "getOwnPropertyDescriptor",
        reflect_get_own_property_descriptor,
    );
    e.add_method(reflect, "getPrototypeOf", reflect_get_prototype_of);
    e.add_method(reflect, "setPrototypeOf", reflect_set_prototype_of);
    e.add_method(reflect, "isExtensible", reflect_is_extensible);
    e.add_method(reflect, "preventExtensions", reflect_prevent_extensions);
    let tag = e.str_val("Reflect");
    e.set_own(reflect, crate::interp::SYM_TO_STRING_TAG, tag, false);
    e.set_own(g, "Reflect", Value::Obj(reflect), false);
    // `Proxy` — construct-only (`Proxy(t,h)` → TypeError) ; la
    // méthode statique `revocable` rend `{proxy, revoke}`.
    let proxy = ctor(e, "Proxy", proxy_ctor, e.protos.proxy);
    if let Value::Obj(pid) = proxy {
        e.add_method(pid, "revocable", proxy_revocable);
    }
    e.set_own(g, "Proxy", proxy, false);
    let performance = e.new_object();
    e.add_method(performance, "now", performance_now);
    e.set_own(g, "performance", Value::Obj(performance), false);

    // `TextEncoder`/`TextDecoder` — UTF-8 only. Chacun a SON
    // prototype (héritant `Object.prototype`) — partager `p_obj`
    // écraserait `Object.prototype.constructor`.
    let te_proto = e.new_object();
    let td_proto = e.new_object();
    let te = ctor(e, "TextEncoder", text_encoder_ctor, te_proto);
    let td = ctor(e, "TextDecoder", text_decoder_ctor, td_proto);
    e.set_own(g, "TextEncoder", te, false);
    e.set_own(g, "TextDecoder", td, false);

    // `console` — engine-wide (workers have it too); lines go to the
    // embedder's hook or vanish silently (headless).
    let console = e.new_object();
    e.add_method(console, "log", crate::dom_host::console_line);
    e.add_method(console, "info", crate::dom_host::console_line);
    e.add_method(console, "warn", crate::dom_host::console_line);
    e.add_method(console, "error", crate::dom_host::console_line);
    e.add_method(console, "debug", crate::dom_host::console_line);
    e.set_own(g, "console", Value::Obj(console), false);

    // Object ctor statics + prototype.
    if let Value::Obj(oid) = object {
        e.add_method(oid, "keys", object_keys);
        e.add_method(oid, "values", object_values);
        e.add_method(oid, "assign", object_assign);
        e.add_method(oid, "create", object_create);
        e.add_method(oid, "defineProperty", object_define_property);
        e.add_method(oid, "defineProperties", object_define_properties);
        e.add_method(
            oid,
            "getOwnPropertyDescriptor",
            object_get_own_property_descriptor,
        );
        e.add_method(
            oid,
            "getOwnPropertyDescriptors",
            object_get_own_property_descriptors,
        );
        e.add_method(oid, "getPrototypeOf", object_get_proto);
        e.add_method(oid, "setPrototypeOf", object_set_proto);
        e.add_method(oid, "getOwnPropertyNames", object_get_own_property_names);
        e.add_method(
            oid,
            "getOwnPropertySymbols",
            object_get_own_property_symbols,
        );
        e.add_method(oid, "entries", object_entries);
        e.add_method(oid, "fromEntries", object_from_entries);
        e.add_method(oid, "hasOwn", object_has_own);
        e.add_method(oid, "freeze", object_freeze);
        e.add_method(oid, "isFrozen", object_is_frozen);
        e.add_method(oid, "seal", object_seal);
        e.add_method(oid, "isSealed", object_is_sealed);
        e.add_method(oid, "preventExtensions", object_prevent_extensions);
        e.add_method(oid, "isExtensible", object_is_extensible);
        e.add_method(oid, "is", object_is);
        e.add_method(oid, "groupBy", object_group_by);
    }
    // `Number` ctor statics.
    if let Value::Obj(nid) = number {
        e.add_method(nid, "isNaN", number_is_nan);
        e.add_method(nid, "isFinite", number_is_finite);
        e.add_method(nid, "isInteger", number_is_integer);
        e.add_method(nid, "parseInt", parse_int);
        e.add_method(nid, "parseFloat", parse_float);
        e.set_const(nid, "MAX_SAFE_INTEGER", Value::Num(9_007_199_254_740_991.0));
        e.set_const(
            nid,
            "MIN_SAFE_INTEGER",
            Value::Num(-9_007_199_254_740_991.0),
        );
        e.set_const(nid, "MAX_VALUE", Value::Num(f64::MAX));
        e.set_const(nid, "MIN_VALUE", Value::Num(5e-324));
        e.set_const(nid, "EPSILON", Value::Num(f64::EPSILON));
        e.add_method(nid, "isSafeInteger", number_is_safe_integer);
        e.set_const(nid, "POSITIVE_INFINITY", Value::Num(f64::INFINITY));
        e.set_const(nid, "NEGATIVE_INFINITY", Value::Num(f64::NEG_INFINITY));
        e.set_const(nid, "NaN", Value::Num(f64::NAN));
    }
    let op = e.protos.object;
    e.add_method(op, "toString", object_to_string);
    e.add_method(op, "hasOwnProperty", has_own_property);
    e.add_method(op, "valueOf", object_value_of);
    e.add_method(op, "isPrototypeOf", object_is_prototype_of);
    e.add_method(op, "propertyIsEnumerable", object_property_is_enumerable);
    // Annex B — `__proto__` accesseur + `__defineGetter__`…
    e.add_method(op, "__defineGetter__", object_define_getter);
    e.add_method(op, "__defineSetter__", object_define_setter);
    e.add_method(op, "__lookupGetter__", object_lookup_getter);
    e.add_method(op, "__lookupSetter__", object_lookup_setter);
    {
        let pg = e.native_fn("get __proto__", proto_getter);
        let ps = e.native_fn("set __proto__", proto_setter);
        if let Some(o) = e.heap.obj_mut(op) {
            o.define_accessor(
                "__proto__",
                pg,
                ps,
                crate::heap::PropAttrs {
                    writable: false,
                    enumerable: false,
                    configurable: true,
                },
            );
        }
    }

    // Function.prototype.
    let fp = e.protos.function;
    e.add_method(fp, "call", function_call);
    e.add_method(fp, "apply", function_apply);
    e.add_method(fp, "bind", function_bind);
    e.add_method(fp, "toString", function_to_string);
    // `Function.prototype.name === ""` (spec — le proto est
    // ordinaire, le bras synthétique `Fun` ne le couvre pas).
    let empty_name = e.str_val("");
    e.set_own(fp, "name", empty_name, false);

    // Array.prototype.
    let ap = e.protos.array;
    e.add_method(ap, "push", array_push);
    e.add_method(ap, "pop", array_pop);
    e.add_method(ap, "shift", array_shift);
    e.add_method(ap, "unshift", array_unshift);
    e.add_method(ap, "join", array_join);
    e.add_method(ap, "toString", array_join);
    e.add_method(ap, "indexOf", array_index_of);
    e.add_method(ap, "lastIndexOf", array_last_index_of);
    e.add_method(ap, "slice", array_slice);
    e.add_method(ap, "concat", array_concat);
    e.add_method(ap, "reverse", array_reverse);
    e.add_method(ap, "sort", array_sort);
    e.add_method(ap, "forEach", array_for_each);
    e.add_method(ap, "map", array_map);
    e.add_method(ap, "filter", array_filter);
    e.add_method(ap, "reduce", array_reduce);
    e.add_method(ap, "reduceRight", array_reduce_right);
    e.add_method(ap, "copyWithin", array_copy_within);
    // Spec : joint `,` via `toLocaleString` des éléments — ici
    // `join` suffit (pas de locale, déviation documentée).
    e.add_method(ap, "toLocaleString", array_join);
    e.add_method(ap, "includes", array_includes);
    e.add_method(ap, "find", array_find);
    e.add_method(ap, "findIndex", array_find_index);
    e.add_method(ap, "findLast", array_find_last);
    e.add_method(ap, "findLastIndex", array_find_last_index);
    e.add_method(ap, "some", array_some);
    e.add_method(ap, "every", array_every);
    e.add_method(ap, "at", array_at);
    e.add_method(ap, "fill", array_fill);
    e.add_method(ap, "flat", array_flat);
    e.add_method(ap, "flatMap", array_flat_map);
    e.add_method(ap, "entries", array_entries);
    e.add_method(ap, "keys", array_keys);
    e.add_method(ap, "values", array_values);
    e.add_method(ap, "with", array_with);
    e.add_method(ap, "toReversed", array_to_reversed);
    e.add_method(ap, "toSorted", array_to_sorted);
    e.add_method(ap, "toSpliced", array_to_spliced);
    // `[Symbol.iterator]` ≙ `values` (spec).
    let values_fn = e.native_fn("values", array_values);
    e.set_own(ap, crate::interp::SYM_ITERATOR, values_fn, false);
    // `[Symbol.unscopables]` — méthodes exclues de la liaison `with`
    // (liste spec ES2015+ ; `with ([]) { find }` → scope externe).
    let unsc = e.new_object();
    for n in [
        "at",
        "copyWithin",
        "entries",
        "fill",
        "find",
        "findIndex",
        "findLast",
        "findLastIndex",
        "flat",
        "flatMap",
        "includes",
        "keys",
        "values",
        "with",
        "toReversed",
        "toSorted",
        "toSpliced",
    ] {
        e.set_own(unsc, n, Value::Bool(true), false);
    }
    e.set_own(ap, crate::interp::SYM_UNSCOPABLES, Value::Obj(unsc), false);
    // `Array.isArray` sur le ctor.
    if let Value::Obj(aid) = array {
        e.add_method(aid, "isArray", array_is_array);
        e.add_method(aid, "from", array_from);
        e.add_method(aid, "of", array_of);
        e.add_method(aid, "fromAsync", array_from_async);
    }

    // String.prototype + String.fromCharCode.
    let sp = e.protos.string;
    e.add_method(sp, "toString", string_to_string);
    e.add_method(sp, "valueOf", string_to_string);
    e.add_method(sp, "charAt", string_char_at);
    e.add_method(sp, "charCodeAt", string_char_code_at);
    e.add_method(sp, "indexOf", string_index_of);
    e.add_method(sp, "lastIndexOf", string_last_index_of);
    e.add_method(sp, "slice", string_slice);
    e.add_method(sp, "substring", string_substring);
    e.add_method(sp, "substr", string_substr);
    e.add_method(sp, "toLowerCase", string_to_lower);
    e.add_method(sp, "toUpperCase", string_to_upper);
    // Locale-fallbacks (déviation : pas de CLDR — tri par point de
    // code, casse Unicode simple, `toLocaleString` ≡ `toString`).
    e.add_method(sp, "toLocaleLowerCase", string_to_lower);
    e.add_method(sp, "toLocaleUpperCase", string_to_upper);
    e.add_method(sp, "toLocaleString", string_to_string);
    e.add_method(sp, "localeCompare", string_locale_compare);
    // Annex B — méthodes HTML legacy (`'x'.bold()` → `<b>x</b>`),
    // encore présentes sur nombre de vieux sites.
    for (n, tag, attr) in [
        ("anchor", "a", "name"),
        ("big", "big", ""),
        ("blink", "blink", ""),
        ("bold", "b", ""),
        ("fixed", "tt", ""),
        ("fontcolor", "font", "color"),
        ("fontsize", "font", "size"),
        ("italics", "i", ""),
        ("link", "a", "href"),
        ("small", "small", ""),
        ("strike", "strike", ""),
        ("sub", "sub", ""),
        ("sup", "sup", ""),
    ] {
        let tag_v = e.str_val(tag);
        let attr_v = e.str_val(attr);
        let f = e.bound_native(n, string_html_wrap, vec![tag_v, attr_v]);
        e.set_own(sp, n, f, false);
    }
    e.add_method(sp, "trim", string_trim);
    e.add_method(sp, "split", string_split);
    e.add_method(sp, "replace", string_replace);
    e.add_method(sp, "replaceAll", string_replace_all);
    // ES2024 well-formedness — nos chaînes sont des `char` Rust
    // (scalaires Unicode valides, jamais de lone surrogate).
    e.add_method(sp, "isWellFormed", string_is_well_formed);
    e.add_method(sp, "toWellFormed", string_to_well_formed);
    e.add_method(sp, "match", string_match);
    e.add_method(sp, "search", string_search);
    e.add_method(sp, "concat", string_concat);
    e.add_method(sp, "includes", string_includes);
    e.add_method(sp, "startsWith", string_starts_with);
    e.add_method(sp, "endsWith", string_ends_with);
    e.add_method(sp, "repeat", string_repeat);
    e.add_method(sp, "padStart", string_pad_start);
    e.add_method(sp, "padEnd", string_pad_end);
    e.add_method(sp, "trimStart", string_trim_start);
    e.add_method(sp, "trimEnd", string_trim_end);
    e.add_method(sp, "at", string_at);
    e.add_method(sp, "codePointAt", string_code_point_at);
    e.add_method(sp, "matchAll", string_match_all);
    e.add_method(sp, "normalize", string_normalize);
    if let Value::Obj(sid) = string {
        e.add_method(sid, "fromCharCode", string_from_char_code);
        e.add_method(sid, "fromCodePoint", string_from_code_point);
        e.add_method(sid, "raw", string_raw);
    }

    // Number.prototype.
    let np = e.protos.number;
    e.add_method(np, "toString", number_to_string);
    e.add_method(np, "valueOf", number_value_of);
    e.add_method(np, "toFixed", number_to_fixed);
    e.add_method(np, "toPrecision", number_to_precision);
    e.add_method(np, "toExponential", number_to_exponential);
    e.add_method(np, "toLocaleString", number_to_locale);

    // Boolean.prototype.
    let bp = e.protos.boolean;
    e.add_method(bp, "toString", boolean_to_string);
    e.add_method(bp, "valueOf", boolean_value_of);

    // Error.prototype.toString (inherited by Type/ReferenceError).
    let ep = e.protos.error;
    e.add_method(ep, "toString", error_to_string);
    let name_v = e.str_val("Error");
    e.set_own(ep, "name", name_v, false);
    let msg_v = e.str_val("");
    e.set_own(ep, "message", msg_v, false);

    // RegExp ctor + prototype methods.
    let rp = e.protos.regexp;
    e.add_method(rp, "exec", regexp_exec);
    e.add_method(rp, "test", regexp_test);
    e.add_method(rp, "toString", regexp_to_string);
    // `@@match`/`@@matchAll`/`@@replace`/`@@search`/`@@split`
    // (spec) — les méthodes String dispatchent vers ces hooks.
    e.add_method(rp, crate::interp::SYM_MATCH, re_sym_match);
    e.add_method(rp, crate::interp::SYM_MATCH_ALL, re_sym_match_all);
    e.add_method(rp, crate::interp::SYM_REPLACE, re_sym_replace);
    e.add_method(rp, crate::interp::SYM_SEARCH, re_sym_search);
    e.add_method(rp, crate::interp::SYM_SPLIT, re_sym_split);
    let regexp = ctor(e, "RegExp", regexp_ctor, rp);
    if let Value::Obj(rid) = regexp {
        e.add_method(rid, "escape", regexp_escape);
    }
    e.set_own(g, "RegExp", regexp, false);

    // Promise + queueMicrotask (JS-7c).
    crate::promise::install(e);

    // Date minimal : now() static, getTime/valueOf on instances.
    let dp = e.protos.date;
    e.add_method(dp, "getTime", date_get_time);
    e.add_method(dp, "valueOf", date_get_time);
    e.add_method(dp, "toString", date_to_string);
    e.add_method(dp, "toLocaleString", date_to_string);
    e.add_method(dp, "toLocaleDateString", date_to_string);
    e.add_method(dp, "toLocaleTimeString", date_to_string);
    // JS-7g — full-ish UTC Date.
    e.add_method(dp, "getFullYear", date_get_full_year);
    e.add_method(dp, "getUTCFullYear", date_get_full_year);
    e.add_method(dp, "getMonth", date_get_month);
    e.add_method(dp, "getUTCMonth", date_get_month);
    e.add_method(dp, "getDate", date_get_date);
    e.add_method(dp, "getUTCDate", date_get_date);
    e.add_method(dp, "getDay", date_get_day);
    e.add_method(dp, "getUTCDay", date_get_day);
    e.add_method(dp, "getHours", date_get_hours);
    e.add_method(dp, "getUTCHours", date_get_hours);
    e.add_method(dp, "getMinutes", date_get_minutes);
    e.add_method(dp, "getUTCMinutes", date_get_minutes);
    e.add_method(dp, "getSeconds", date_get_seconds);
    e.add_method(dp, "getUTCSeconds", date_get_seconds);
    e.add_method(dp, "getMilliseconds", date_get_milliseconds);
    e.add_method(dp, "getUTCMilliseconds", date_get_milliseconds);
    e.add_method(dp, "getTimezoneOffset", date_get_timezone_offset);
    // Annex B legacy — `getYear` = année - 1900, `setYear` 0-99 →
    // 1900+n (utilisés par du code pré-2000 encore vivant).
    e.add_method(dp, "getYear", date_get_year);
    e.add_method(dp, "setYear", date_set_year);
    e.add_method(dp, "toISOString", date_to_iso);
    e.add_method(dp, "toUTCString", date_to_utc_string);
    e.add_method(dp, "toGMTString", date_to_utc_string);
    if let Value::Obj(did) = date {
        e.add_method(did, "now", date_now);
        e.add_method(did, "parse", date_parse);
        e.add_method(did, "UTC", date_utc);
    }

    // Math object.
    let math = e.new_object();
    e.set_const(math, "PI", Value::Num(std::f64::consts::PI));
    e.set_const(math, "E", Value::Num(std::f64::consts::E));
    e.set_const(math, "LN2", Value::Num(std::f64::consts::LN_2));
    e.set_const(math, "LN10", Value::Num(std::f64::consts::LN_10));
    e.set_const(math, "SQRT2", Value::Num(std::f64::consts::SQRT_2));
    e.set_const(math, "SQRT1_2", Value::Num(std::f64::consts::FRAC_1_SQRT_2));
    e.set_const(math, "LOG2E", Value::Num(std::f64::consts::LOG2_E));
    e.set_const(math, "LOG10E", Value::Num(std::f64::consts::LOG10_E));
    e.add_method(math, "abs", math_abs);
    e.add_method(math, "floor", math_floor);
    e.add_method(math, "ceil", math_ceil);
    e.add_method(math, "round", math_round);
    e.add_method(math, "max", math_max);
    e.add_method(math, "min", math_min);
    e.add_method(math, "pow", math_pow);
    e.add_method(math, "sqrt", math_sqrt);
    e.add_method(math, "exp", math_exp);
    e.add_method(math, "log", math_log);
    e.add_method(math, "sin", math_sin);
    e.add_method(math, "cos", math_cos);
    e.add_method(math, "tan", math_tan);
    e.add_method(math, "atan", math_atan);
    e.add_method(math, "atan2", math_atan2);
    e.add_method(math, "random", math_random);
    e.add_method(math, "trunc", math_trunc);
    e.add_method(math, "sign", math_sign);
    e.add_method(math, "cbrt", math_cbrt);
    e.add_method(math, "log2", math_log2);
    e.add_method(math, "log10", math_log10);
    e.add_method(math, "log1p", math_log1p);
    e.add_method(math, "expm1", math_expm1);
    e.add_method(math, "fround", math_fround);
    e.add_method(math, "asin", math_asin);
    e.add_method(math, "acos", math_acos);
    e.add_method(math, "sinh", math_sinh);
    e.add_method(math, "cosh", math_cosh);
    e.add_method(math, "tanh", math_tanh);
    e.add_method(math, "hypot", math_hypot);
    e.add_method(math, "imul", math_imul);
    e.add_method(math, "clz32", math_clz32);
    e.add_method(math, "f16round", math_f16round);
    let tag = e.str_val("Math");
    e.set_own(math, crate::interp::SYM_TO_STRING_TAG, tag, false);
    e.set_own(g, "Math", Value::Obj(math), false);
}

// ------------------------------------------------------------------
// helpers used inside natives
// ------------------------------------------------------------------

/// `this` coerced to a string (throws on null/undefined).
fn this_str(e: &mut Engine, this: Value) -> JsResult<String> {
    match this {
        Value::Undefined | Value::Null => {
            Err(e.type_error("String method called on null/undefined"))
        }
        _ => e.to_js_string(this),
    }
}

/// `this` as a number (accepts primitives; objects via `ToNumber`).
fn this_num(e: &mut Engine, this: Value) -> JsResult<f64> {
    match this {
        Value::Undefined | Value::Null => {
            Err(e.type_error("Number method called on null/undefined"))
        }
        _ => e.to_number(this),
    }
}

/// `this` must be an array; returns its `ObjId`.
fn this_array(e: &mut Engine, this: Value) -> JsResult<ObjId> {
    match this {
        Value::Obj(id) if matches!(e.heap.obj(id).map(|o| &o.kind), Some(ObjKind::Array(_))) => {
            Ok(id)
        }
        _ => Err(e.type_error("array method called on non-array")),
    }
}

fn take_elems(e: &mut Engine, id: ObjId) -> Vec<Value> {
    match e.heap.obj_mut(id) {
        Some(o) => match &mut o.kind {
            ObjKind::Array(v) => std::mem::take(v),
            _ => Vec::new(),
        },
        None => Vec::new(),
    }
}

fn put_elems(e: &mut Engine, id: ObjId, elems: Vec<Value>) {
    if let Some(o) = e.heap.obj_mut(id) {
        if let ObjKind::Array(v) = &mut o.kind {
            *v = elems;
        }
    }
}

fn arr_of(e: &mut Engine, elems: Vec<Value>) -> Value {
    e.new_array(elems)
}

fn chars_of(s: &str) -> Vec<char> {
    s.chars().collect()
}

fn from_chars(e: &mut Engine, cs: &[char]) -> Value {
    e.str_val(cs.iter().collect::<String>())
}

fn now_millis() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_millis() as f64)
}

// ------------------------------------------------------------------
// global functions
// ------------------------------------------------------------------

fn parse_int(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let s = e.to_js_string(arg(args, 0))?;
    let mut radix = match arg(args, 1) {
        Value::Undefined => 0i32,
        v => e.to_number(v)? as i32,
    };
    let t = s.trim_start();
    let (sign, mut rest) = match t.strip_prefix('-') {
        Some(r) => (-1.0, r),
        None => (1.0, t.strip_prefix('+').unwrap_or(t)),
    };
    if radix == 0 || radix == 16 {
        if let Some(h) = rest.strip_prefix("0x").or_else(|| rest.strip_prefix("0X")) {
            rest = h;
            radix = 16;
        }
    }
    if radix == 0 {
        radix = 10;
    }
    if !(2..=36).contains(&radix) {
        return Ok(Value::Num(f64::NAN));
    }
    let mut v = 0f64;
    let mut any = false;
    for c in rest.chars() {
        match c.to_digit(radix as u32) {
            Some(d) => {
                v = v * f64::from(radix) + f64::from(d);
                any = true;
            }
            None => break,
        }
    }
    Ok(Value::Num(if any { sign * v } else { f64::NAN }))
}

fn parse_float(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let s = e.to_js_string(arg(args, 0))?;
    let t = s.trim_start();
    if t.starts_with("Infinity") || t.starts_with("+Infinity") {
        return Ok(Value::Num(f64::INFINITY));
    }
    if t.starts_with("-Infinity") {
        return Ok(Value::Num(f64::NEG_INFINITY));
    }
    let b = t.as_bytes();
    let mut i = 0usize;
    if matches!(b.first(), Some(b'+' | b'-')) {
        i = 1;
    }
    let mut mantissa_digits = 0usize;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
        mantissa_digits += 1;
    }
    if b.get(i) == Some(&b'.') {
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
            mantissa_digits += 1;
        }
    }
    let mut end = i;
    if mantissa_digits > 0 && matches!(b.get(i), Some(b'e' | b'E')) {
        let mut j = i + 1;
        if matches!(b.get(j), Some(b'+' | b'-')) {
            j += 1;
        }
        let ds = j;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j > ds {
            end = j;
        }
    }
    if mantissa_digits == 0 {
        return Ok(Value::Num(f64::NAN));
    }
    Ok(Value::Num(t[..end].parse::<f64>().unwrap_or(f64::NAN)))
}

fn is_nan(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    Ok(Value::Bool(e.to_number(arg(args, 0))?.is_nan()))
}

fn is_finite(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    Ok(Value::Bool(e.to_number(arg(args, 0))?.is_finite()))
}

fn escape(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let s = e.to_js_string(arg(args, 0))?;
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() || "@*_+-./".contains(c) {
            out.push(c);
        } else if (c as u32) < 256 {
            out.push_str(&format!("%{:02X}", c as u32));
        } else {
            out.push_str(&format!("%u{:04X}", c as u32));
        }
    }
    Ok(e.str_val(out))
}

fn unescape(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let s = e.to_js_string(arg(args, 0))?;
    let cs = chars_of(&s);
    let mut out = String::new();
    let mut i = 0;
    while i < cs.len() {
        if cs[i] == '%' {
            if cs.get(i + 1) == Some(&'u') && i + 6 <= cs.len() {
                if let Some(v) = hex_val(&cs[i + 2..i + 6]) {
                    if let Some(c) = char::from_u32(v) {
                        out.push(c);
                        i += 6;
                        continue;
                    }
                }
            }
            if i + 3 <= cs.len() {
                if let Some(v) = hex_val(&cs[i + 1..i + 3]) {
                    if let Some(c) = char::from_u32(v) {
                        out.push(c);
                        i += 3;
                        continue;
                    }
                }
            }
        }
        out.push(cs[i]);
        i += 1;
    }
    Ok(e.str_val(out))
}

fn hex_val(cs: &[char]) -> Option<u32> {
    if cs.is_empty() {
        return None;
    }
    let mut v = 0u32;
    for c in cs {
        v = v.checked_mul(16)?.checked_add(c.to_digit(16)?)?;
    }
    Some(v)
}

const URI_SAFE: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_.!~*'()";

fn encode_uri_component(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let s = e.to_js_string(arg(args, 0))?;
    let mut out = String::new();
    for b in s.bytes() {
        let c = b as char;
        if c.is_ascii() && URI_SAFE.contains(c) {
            out.push(c);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    Ok(e.str_val(out))
}

fn decode_uri_component(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let s = e.to_js_string(arg(args, 0))?;
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let h = (bytes[i + 1] as char).to_digit(16);
            let l = (bytes[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (h, l) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    if let Ok(s) = String::from_utf8(out) {
        Ok(e.str_val(s))
    } else {
        let p = e.protos.error;
        Err(Thrown(e.error_value(p, "URIError", "malformed URI")))
    }
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn btoa(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let s = e.to_js_string(arg(args, 0))?;
    if s.chars().any(|c| c as u32 > 0xff) {
        return Err(e.type_error("btoa: non-latin1 input"));
    }
    let b: Vec<u8> = s.chars().map(|c| c as u8).collect();
    let b = b.as_slice();
    let mut out = String::new();
    for chunk in b.chunks(3) {
        let n = chunk.iter().fold(0u32, |a, &c| (a << 8) | u32::from(c));
        let n = n << (8 * (3 - chunk.len()));
        out.push(B64[(n >> 18 & 63) as usize] as char);
        out.push(B64[(n >> 12 & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            B64[(n >> 6 & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            B64[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    Ok(e.str_val(out))
}

fn atob(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let s = e.to_js_string(arg(args, 0))?;
    let mut bits = 0u32;
    let mut nbits = 0u32;
    let mut out = Vec::new();
    for c in s.chars() {
        if c == '=' {
            break;
        }
        let v = match B64.iter().position(|&b| b == c as u8) {
            Some(v) => v as u32,
            None if c.is_whitespace() => continue,
            None => return Err(e.type_error("atob: invalid character")),
        };
        bits = (bits << 6) | v;
        nbits += 6;
        if nbits >= 8 {
            nbits -= 8;
            out.push((bits >> nbits) as u8);
        }
    }
    match String::from_utf8(out) {
        Ok(s) => Ok(e.str_val(s)),
        Err(_) => Err(e.type_error("atob: invalid byte sequence")),
    }
}

// ------------------------------------------------------------------
// constructors
// ------------------------------------------------------------------

fn object_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    match arg(args, 0) {
        v @ Value::Obj(_) => Ok(v),
        _ => Ok(Value::Obj(e.new_object())),
    }
}

fn array_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    if args.len() == 1 {
        if let Value::Num(n) = args[0] {
            if n >= 0.0 && n.fract() == 0.0 {
                return Ok(arr_of(e, vec![Value::Undefined; n as usize]));
            }
            return Err(e.range_error("invalid array length"));
        }
    }
    Ok(arr_of(e, args.to_vec()))
}

/// Propriété cachée portant `[[PrimitiveValue]]` des wrappers
/// `new String/Number/Boolean` — sans elle, `valueOf`/`toString`
/// retomberaient sur `ToPrimitive(this)` → boucle infinie.
const PRIM: &str = "__prim";

/// `this` est-il l'objet frais d'un `new` sur le proto donné ?
/// (Les natifs ne voient pas le drapeau construct — l'heuristique
/// est le proto exact, assez pour `String/Number/Boolean`.)
fn constructed_this(e: &Engine, this: Value, proto: ObjId) -> Option<ObjId> {
    let Value::Obj(id) = this else {
        return None;
    };
    match e.heap.obj(id).map(|o| o.proto) {
        Some(Value::Obj(p)) if p == proto => Some(id),
        _ => None,
    }
}

/// Pose `__prim` sur `this` et retourne `this` si `this` est un
/// wrapper construit sur `proto` ; sinon retourne `prim` (appel
/// fonctionnel — `String(x)` produit une primitive).
fn wrap_or_prim(e: &mut Engine, this: Value, proto: ObjId, prim: Value) -> Value {
    match constructed_this(e, this, proto) {
        Some(id) => {
            e.set_own(id, PRIM, prim, false);
            this
        }
        None => prim,
    }
}

fn string_ctor(e: &mut Engine, t: Value, args: &[Value]) -> JsResult<Value> {
    match arg(args, 0) {
        Value::Undefined => {
            let prim = e.str_val("");
            Ok(wrap_or_prim(e, t, e.protos.string, prim))
        }
        v => {
            let s = e.to_js_string(v)?;
            let prim = e.str_val(s);
            Ok(wrap_or_prim(e, t, e.protos.string, prim))
        }
    }
}

fn number_ctor(e: &mut Engine, t: Value, args: &[Value]) -> JsResult<Value> {
    match arg(args, 0) {
        Value::Undefined => Ok(wrap_or_prim(e, t, e.protos.number, Value::Num(0.0))),
        // `Number(1n)` — `ToNumeric` autorise le bigint (troncation
        // `to_f64` documentée pour |b| > 2^53).
        Value::Big(_) => {
            let n = e
                .bigint_of(arg(args, 0))
                .and_then(|b| num_traits::ToPrimitive::to_f64(&b))
                .unwrap_or(f64::NAN);
            Ok(wrap_or_prim(e, t, e.protos.number, Value::Num(n)))
        }
        v => {
            let n = e.to_number(v)?;
            Ok(wrap_or_prim(e, t, e.protos.number, Value::Num(n)))
        }
    }
}

fn boolean_ctor(e: &mut Engine, t: Value, args: &[Value]) -> JsResult<Value> {
    let b = Value::Bool(e.to_boolean(arg(args, 0)));
    Ok(wrap_or_prim(e, t, e.protos.boolean, b))
}

/// `this` en tant que primitive String — wrapper `new String` via
/// `__prim`, primitive directe, sinon `ToString(this)`. Ne boucle
/// jamais : les méthodes strictes (`toString`/`valueOf`) lisent
/// `__prim` ou jettent.
fn this_prim(e: &Engine, this: Value) -> Option<Value> {
    match this {
        Value::Obj(id) => e.heap.obj(id).and_then(|o| o.get_own(PRIM)),
        v => Some(v),
    }
}

/// `this` strict pour `String.prototype.{toString,valueOf}` :
/// primitive string ou wrapper — `TypeError` sinon (spec
/// `thisStringValue`, évite aussi la récursion ToString→toString).
fn this_string_strict(e: &mut Engine, this: Value) -> JsResult<Value> {
    match this_prim(e, this) {
        Some(v @ Value::Str(_)) => Ok(v),
        _ => Err(e.type_error("String.prototype method called on incompatible receiver")),
    }
}

fn make_named_error(e: &mut Engine, proto: ObjId, name: &str, args: &[Value]) -> JsResult<Value> {
    let msg = match arg(args, 0) {
        Value::Undefined => String::new(),
        v => e.to_js_string(v)?,
    };
    let err = e.error_value(proto, name, &msg);
    // `new Error(msg, {cause})` → `err.cause` (own, non-énumérable).
    if let (Value::Obj(eid), Value::Obj(opt)) = (err, arg(args, 1)) {
        if let Some(c) = e.get_prop(opt, "cause") {
            e.set_own(eid, "cause", c, false);
        }
    }
    Ok(err)
}

/// `AggregateError(errors, message?, options?)` — iterable `errors`
/// matérialisé en tableau propre (spec `CreateArrayFromList`-like).
fn aggregate_error_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let errs = match arg(args, 0) {
        Value::Undefined | Value::Null => Vec::new(),
        v => e.iter_values(v)?,
    };
    let list = e.new_array(errs);
    let err = make_named_error(e, e.protos.aggregate_error, "AggregateError", &args[1..])?;
    if let Value::Obj(eid) = err {
        e.set_own(eid, "errors", list, false);
    }
    Ok(err)
}

fn error_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let p = e.protos.error;
    make_named_error(e, p, "Error", args)
}

fn type_error_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let p = e.protos.type_error;
    make_named_error(e, p, "TypeError", args)
}

fn reference_error_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let p = e.protos.reference_error;
    make_named_error(e, p, "ReferenceError", args)
}

fn syntax_error_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let p = e.protos.syntax_error;
    make_named_error(e, p, "SyntaxError", args)
}

fn date_now(_e: &mut Engine, _t: Value, _args: &[Value]) -> JsResult<Value> {
    Ok(Value::Num(now_millis()))
}

fn date_get_time(e: &mut Engine, this: Value, _args: &[Value]) -> JsResult<Value> {
    match this {
        Value::Obj(id) => Ok(e.get_prop(id, "__time").unwrap_or(Value::Num(f64::NAN))),
        _ => Err(e.type_error("Date method on non-Date")),
    }
}

fn date_to_string(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let ms = date_get_time(e, this, args)?;
    let n = match ms {
        Value::Num(n) => n,
        _ => f64::NAN,
    };
    Ok(e.str_val(format!("Date({})", num_to_js_string(n))))
}

// ------------------------------------------------------------------
// Object
// ------------------------------------------------------------------

fn object_to_string(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let tag = match this {
        Value::Undefined => "Undefined",
        Value::Null => "Null",
        Value::Num(_) => "Number",
        Value::Bool(_) => "Boolean",
        Value::Str(_) => "String",
        Value::Big(_) => "BigInt",
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
            let s = e.heap.string(sid).unwrap_or_default();
            return Ok(e.str_val(format!("[object {s}]")));
        }
    }
    Ok(e.str_val(format!("[object {tag}]")))
}

fn has_own_property(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let key = match arg(args, 0) {
        Value::Num(n) => num_to_js_string(n),
        v => e.to_js_string(v)?,
    };
    let has = match this {
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
                        || crate::interp::array_index(&key).is_some_and(|i| i < elems.len())
                        || o.get_own(&key).is_some()
                }
                ObjKind::Fun(crate::heap::Fun::Script { .. }) => {
                    key == "length" || o.get_own(&key).is_some()
                }
                _ => o.get_own(&key).is_some(),
            },
            None => false,
        },
        _ => false,
    };
    Ok(Value::Bool(has))
}

fn object_value_of(_e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    Ok(this)
}

/// `o.isPrototypeOf(v)` — `this` se trouve-t-il sur la chaîne de
/// prototypes de `v` (bornée à 100 sauts).
fn object_is_prototype_of(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (Value::Obj(pid), Value::Obj(vid)) = (this, arg(args, 0)) else {
        return Ok(Value::Bool(false));
    };
    let mut cur = e.heap.obj(vid).map(|o| o.proto);
    let mut depth = 0;
    while let Some(Value::Obj(c)) = cur {
        if c == pid {
            return Ok(Value::Bool(true));
        }
        cur = e.heap.obj(c).map(|o| o.proto);
        depth += 1;
        if depth > 100 {
            break;
        }
    }
    Ok(Value::Bool(false))
}

fn object_keys(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = arg(args, 0) else {
        return Ok(arr_of(e, Vec::new()));
    };
    let keys = e.own_enum_keys_trapped(id)?;
    let vals: Vec<Value> = keys.iter().map(|k| e.str_val(k.clone())).collect();
    Ok(arr_of(e, vals))
}

fn object_get_proto(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    match arg(args, 0) {
        // Proxy : trap `getPrototypeOf` (objet ou null requis),
        // sinon proto de la cible.
        Value::Obj(id) => {
            if let Some((tid, target, handler)) = e.proxy_th(id)? {
                let trap = e.proxy_trap(handler, "getPrototypeOf")?;
                if matches!(trap, Value::Obj(_)) {
                    let r = e.call_value(trap, handler, &[target])?;
                    return match r {
                        Value::Obj(_) | Value::Null => Ok(r),
                        _ => Err(
                            e.type_error("proxy getPrototypeOf trap must return an object or null")
                        ),
                    };
                }
                return Ok(e.heap.obj(tid).map_or(Value::Null, |o| o.proto));
            }
            Ok(e.heap.obj(id).map_or(Value::Null, |o| o.proto))
        }
        _ => Ok(Value::Null),
    }
}

/// `Object.setPrototypeOf(o, p)` — `p` objet ou `null` ; refus sur
/// cycle de prototype et sur objet non-extensible (spec). Sur un
/// proxy : trap `setPrototypeOf` → `false` = `TypeError`.
fn object_set_proto(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let target = arg(args, 0);
    let proto = match arg(args, 1) {
        v @ Value::Obj(_) => v,
        Value::Null | Value::Undefined => Value::Null,
        _ => return Err(e.type_error("setPrototypeOf: proto must be an object or null")),
    };
    let Value::Obj(id) = target else {
        return Ok(target); // primitives are ignored, per spec
    };
    if let Some((tid, tv, handler)) = e.proxy_th(id)? {
        let trap = e.proxy_trap(handler, "setPrototypeOf")?;
        if matches!(trap, Value::Obj(_)) {
            let ok = e.call_value(trap, handler, &[tv, proto])?;
            return if e.to_boolean(ok) {
                Ok(target)
            } else {
                Err(e.type_error("proxy setPrototypeOf trap returned false"))
            };
        }
        return object_set_proto(e, Value::Undefined, &[Value::Obj(tid), proto]);
    }
    if !e.heap.obj(id).is_some_and(|o| o.extensible) {
        return Err(e.type_error("cannot set prototype of non-extensible object"));
    }
    // Cycle check : remonter la chaîne de `proto` jusqu'à `id`.
    let mut cur = proto;
    while let Value::Obj(cid) = cur {
        if cid == id {
            return Err(e.type_error("cyclic prototype value"));
        }
        cur = e.heap.obj(cid).map_or(Value::Null, |o| o.proto);
    }
    if let Some(o) = e.heap.obj_mut(id) {
        o.proto = proto;
    }
    Ok(target)
}

fn object_values(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = arg(args, 0) else {
        return Ok(arr_of(e, Vec::new()));
    };
    let keys = e.own_enum_keys_trapped(id)?;
    let mut vals = Vec::with_capacity(keys.len());
    for k in keys {
        // Getter-aware read (class accessors are own props).
        vals.push(e.get_prop_value(Value::Obj(id), &k)?);
    }
    Ok(arr_of(e, vals))
}

/// `Object.assign(t, …srcs)` — enumerable-own copy, accessor-aware
/// read and write (setters on the target fire, per spec).
fn object_assign(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let target = arg(args, 0);
    for src in &args[1.min(args.len())..] {
        if let Value::Obj(sid) = src {
            for k in e.own_enum_keys_trapped(*sid)? {
                let v = e.get_prop_value(*src, &k)?;
                e.set_prop_value(target, &k, v)?;
            }
        }
    }
    Ok(target)
}

/// `Object.create(proto[, propsMap])` — proto must be an object or
/// `null`; the optional map applies `defineProperties` semantics.
fn object_create(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let proto = match arg(args, 0) {
        v @ Value::Obj(_) => v,
        Value::Null | Value::Undefined => Value::Null,
        _ => return Err(e.type_error("Object.create: proto must be an object")),
    };
    let id = e.heap.alloc_obj(Object::ordinary(proto));
    if let Value::Obj(_) = arg(args, 1) {
        define_props_from_map(e, id, arg(args, 1))?;
    }
    Ok(Value::Obj(id))
}

/// `Object.defineProperty(o, k, desc)` — descripteur complet :
/// `value`/`writable`/`enumerable`/`configurable`/`get`/`set`.
/// Règles simplifiées de `[[DefineOwnProperty]]` (heap.rs) :
/// non-configurable → rejet `TypeError`.
fn object_define_property(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let target @ Value::Obj(id) = arg(args, 0) else {
        return Err(e.type_error("Object.defineProperty on non-object"));
    };
    let key = match arg(args, 1) {
        Value::Num(n) => num_to_js_string(n),
        v => e.to_js_string(v)?,
    };
    let Value::Obj(desc_id) = arg(args, 2) else {
        return Err(e.type_error("property descriptor must be an object"));
    };
    define_one_prop(e, id, &key, desc_id)?;
    Ok(target)
}

/// Corps de `[[DefineOwnProperty]]` piloté par un objet descripteur —
/// partagé par `defineProperty`/`defineProperties`/`Object.create`.
/// Sur un proxy : trap `defineProperty` ou délégation à la cible.
fn define_one_prop(e: &mut Engine, id: ObjId, key: &str, desc_id: ObjId) -> JsResult<()> {
    if let Some((tid, target, handler)) = e.proxy_th(id)? {
        let trap = e.proxy_trap(handler, "defineProperty")?;
        if matches!(trap, Value::Obj(_)) {
            let kv = e.str_val(key);
            let r = e.call_value(trap, handler, &[target, kv, Value::Obj(desc_id)])?;
            return if e.to_boolean(r) {
                Ok(())
            } else {
                Err(e.type_error(format!("cannot define property `{key}`")))
            };
        }
        return define_one_prop(e, tid, key, desc_id);
    }
    let desc = Value::Obj(desc_id);
    let (has_value, has_writable, has_enumerable, has_configurable, has_get, has_set) = {
        let o = e.heap.obj(desc_id);
        let h = |k: &str| o.is_some_and(|o| o.find(k).is_some());
        (
            h("value"),
            h("writable"),
            h("enumerable"),
            h("configurable"),
            h("get"),
            h("set"),
        )
    };
    let cur = e.find_own(id, key);
    // Flags absents du descripteur → valeur de la prop existante,
    // `false` sinon (spec `[[DefineOwnProperty]]`).
    let enumerable = if has_enumerable {
        let v = e.get_prop_value(desc, "enumerable")?;
        e.to_boolean(v)
    } else {
        cur.is_some_and(|p| p.enumerable())
    };
    let configurable = if has_configurable {
        let v = e.get_prop_value(desc, "configurable")?;
        e.to_boolean(v)
    } else {
        cur.is_some_and(|p| p.configurable())
    };
    let writable = if has_writable {
        let v = e.get_prop_value(desc, "writable")?;
        e.to_boolean(v)
    } else {
        cur.is_some_and(|p| matches!(p, crate::heap::Prop::Data { attrs, .. } if attrs.writable))
    };
    let get = e.get_prop_value(desc, "get")?;
    let set = e.get_prop_value(desc, "set")?;
    let has_acc = has_get || has_set;
    let has_data = has_value || has_writable;
    if has_acc && has_data {
        return Err(e.type_error("descriptor cannot be both data and accessor"));
    }
    if has_acc && !matches!(get, Value::Undefined) && !e.is_callable(get)
        || has_acc && !matches!(set, Value::Undefined) && !e.is_callable(set)
    {
        return Err(e.type_error("accessor descriptor `get`/`set` must be callable"));
    }
    let attrs = crate::heap::PropAttrs {
        writable,
        enumerable,
        configurable,
    };
    let v = if has_value {
        e.get_prop_value(desc, "value")?
    } else {
        cur.and_then(|p| p.data_value()).unwrap_or(Value::Undefined)
    };
    // Côté absent : getter/setter existant conservé.
    let (g, s) = match cur {
        Some(crate::heap::Prop::Acc { get: g, set: s, .. }) => {
            (if has_get { get } else { g }, if has_set { set } else { s })
        }
        _ => (get, set),
    };
    let ok = e.heap.obj_mut(id).is_some_and(|o| {
        if has_acc {
            o.define_accessor(key, g, s, attrs)
        } else {
            o.define_data(key, v, attrs)
        }
    });
    if !ok {
        return Err(e.type_error(format!("cannot define property `{key}`")));
    }
    Ok(())
}

/// `Object.defineProperties(o, propsMap)` — chaque propre prop
/// énumérable de la map est un descripteur appliqué à `o`.
fn object_define_properties(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let target @ Value::Obj(id) = arg(args, 0) else {
        return Err(e.type_error("Object.defineProperties on non-object"));
    };
    define_props_from_map(e, id, arg(args, 1))?;
    Ok(target)
}

/// Map de descripteurs → définitions (ordre d'insertion conservé).
fn define_props_from_map(e: &mut Engine, id: ObjId, map: Value) -> JsResult<()> {
    let Value::Obj(mid) = map else {
        return Err(e.type_error("property map must be an object"));
    };
    for k in e.own_enum_keys_trapped(mid)? {
        let dv = e.get_prop_value(map, &k)?;
        let Value::Obj(did) = dv else {
            return Err(e.type_error(format!("descriptor for `{k}` must be an object")));
        };
        define_one_prop(e, id, &k, did)?;
    }
    Ok(())
}

/// `Object.getOwnPropertyDescriptor(o, k)` — `{value, writable,
/// enumerable, configurable}` ou `{get, set, enumerable,
/// configurable}`, `undefined` quand la prop est absente.
fn object_get_own_property_descriptor(
    e: &mut Engine,
    _t: Value,
    args: &[Value],
) -> JsResult<Value> {
    let Value::Obj(id) = arg(args, 0) else {
        return Ok(Value::Undefined);
    };
    let key = match arg(args, 1) {
        Value::Num(n) => num_to_js_string(n),
        v => e.to_js_string(v)?,
    };
    let Some(p) = e.get_own_prop(id, &key)? else {
        return Ok(Value::Undefined);
    };
    Ok(prop_desc_object(e, p))
}

/// Property descriptor object from a `Prop` — shared by
/// `Object.getOwnPropertyDescriptor` and `Reflect.*`.
fn prop_desc_object(e: &mut Engine, p: crate::heap::Prop) -> Value {
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
fn object_get_own_property_descriptors(
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
                    let mut ks: Vec<String> = o.props.iter().map(|(k, _)| k.clone()).collect();
                    if let ObjKind::Array(elems) = &o.kind {
                        ks.splice(0..0, (0..elems.len()).map(|i| i.to_string()));
                        ks.insert(elems.len(), "length".into());
                    }
                    ks
                }
                None => Vec::new(),
            }
        };
    for k in keys {
        // Clés internes `\u{f131}…` (privés, métadonnées classe) :
        // jamais exposées comme descripteurs.
        if k.starts_with('\u{f131}') {
            continue;
        }
        let kv = e.str_val(k.clone());
        let d = object_get_own_property_descriptor(e, Value::Undefined, &[arg(args, 0), kv])?;
        e.set_own(out, &k, d, true);
    }
    Ok(Value::Obj(out))
}

// ------------------------------------------------------------------
// Function
// ------------------------------------------------------------------

fn function_call(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    e.call_value(this, arg(args, 0), &args[1.min(args.len())..])
}

fn function_apply(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let this_arg = arg(args, 0);
    let arr_args = match arg(args, 1) {
        Value::Obj(id) => match e.heap.obj(id).map(|o| &o.kind) {
            Some(ObjKind::Array(elems)) => elems.clone(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    };
    e.call_value(this, this_arg, &arr_args)
}

fn function_to_string(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let name = match this {
        Value::Obj(id) => match e.heap.obj(id).map(|o| &o.kind) {
            Some(ObjKind::Fun(crate::heap::Fun::Script { lit, .. })) => {
                lit.name.clone().unwrap_or_default()
            }
            Some(ObjKind::Fun(
                crate::heap::Fun::Native { name, .. } | crate::heap::Fun::BoundNative { name, .. },
            )) => (*name).to_string(),
            _ => String::new(),
        },
        _ => String::new(),
    };
    Ok(e.str_val(format!("function {name}() {{ [code] }}")))
}

// ------------------------------------------------------------------
// Array
// ------------------------------------------------------------------

fn array_push(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this)?;
    let n = if let Some(o) = e.heap.obj_mut(id) {
        if let ObjKind::Array(v) = &mut o.kind {
            v.extend_from_slice(args);
            v.len()
        } else {
            0
        }
    } else {
        0
    };
    Ok(Value::Num(n as f64))
}

fn array_pop(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this)?;
    let mut elems = take_elems(e, id);
    let v = elems.pop().unwrap_or(Value::Undefined);
    put_elems(e, id, elems);
    Ok(v)
}

fn array_shift(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this)?;
    let mut elems = take_elems(e, id);
    let v = if elems.is_empty() {
        Value::Undefined
    } else {
        elems.remove(0)
    };
    put_elems(e, id, elems);
    Ok(v)
}

fn array_unshift(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this)?;
    let mut elems = take_elems(e, id);
    let mut new: Vec<Value> = args.to_vec();
    new.append(&mut elems);
    let n = new.len();
    put_elems(e, id, new);
    Ok(Value::Num(n as f64))
}

fn array_join(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this)?;
    let sep = match arg(args, 0) {
        Value::Undefined => ",".to_string(),
        v => e.to_js_string(v)?,
    };
    let elems = take_elems(e, id);
    let mut parts = Vec::with_capacity(elems.len());
    for v in &elems {
        parts.push(match v {
            Value::Undefined | Value::Null => String::new(),
            _ => e.to_js_string(*v)?,
        });
    }
    put_elems(e, id, elems);
    Ok(e.str_val(parts.join(&sep)))
}

fn array_index_of(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this)?;
    let needle = arg(args, 0);
    let elems = take_elems(e, id);
    let mut start = match arg(args, 1) {
        Value::Undefined => 0i64,
        v => e.to_number(v)? as i64,
    };
    if start < 0 {
        start = (elems.len() as i64 + start).max(0);
    }
    let mut found = -1i64;
    for (i, v) in elems.iter().enumerate().skip(start as usize) {
        if e.strict_eq(*v, needle) {
            found = i as i64;
            break;
        }
    }
    put_elems(e, id, elems);
    Ok(Value::Num(found as f64))
}

fn array_last_index_of(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this)?;
    let needle = arg(args, 0);
    let elems = take_elems(e, id);
    let mut found = -1i64;
    for (i, v) in elems.iter().enumerate().rev() {
        if e.strict_eq(*v, needle) {
            found = i as i64;
            break;
        }
    }
    put_elems(e, id, elems);
    Ok(Value::Num(found as f64))
}

fn norm_idx(i: i64, len: i64) -> usize {
    if i < 0 {
        (len + i).max(0) as usize
    } else {
        (i.min(len)) as usize
    }
}

fn array_slice(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this)?;
    let elems = take_elems(e, id);
    let len = elems.len() as i64;
    let start = match arg(args, 0) {
        Value::Undefined => 0,
        v => e.to_number(v)? as i64,
    };
    let end = match arg(args, 1) {
        Value::Undefined => len,
        v => e.to_number(v)? as i64,
    };
    let (a, b) = (norm_idx(start, len), norm_idx(end, len));
    let out: Vec<Value> = if a < b {
        elems[a..b].to_vec()
    } else {
        Vec::new()
    };
    put_elems(e, id, elems);
    // `ArraySpeciesCreate(this, out.len)` — sous-classes.
    let a_v = species_array(e, this, out.len())?;
    for (i, v) in out.iter().enumerate() {
        e.set_prop_value(a_v, &i.to_string(), *v)?;
    }
    Ok(a_v)
}

fn array_concat(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this)?;
    let mut out = take_elems(e, id);
    put_elems(e, id, out.clone());
    for a in args {
        // `IsConcatSpreadable` (spec) : `arg[@@isConcatSpreadable]`
        // prime, sinon le défaut est `Array.isArray(arg)`.
        let spread = match a {
            Value::Obj(aid) => match e.get_prop(*aid, crate::interp::SYM_IS_CONCAT_SPREADABLE) {
                Some(v) if !matches!(v, Value::Undefined) => e.to_boolean(v),
                _ => matches!(e.heap.obj(*aid).map(|o| &o.kind), Some(ObjKind::Array(_))),
            },
            _ => false,
        };
        if spread {
            // Lecture via `length` + `Get` par indice (sous-classes,
            // array-likes et proxies traversés, spec).
            let len = match e.get_prop_value(*a, "length")? {
                Value::Num(n) => n.max(0.0) as usize,
                _ => 0,
            };
            for i in 0..len {
                let v = e.get_prop_value(*a, &i.to_string())?;
                out.push(v);
            }
        } else {
            out.push(*a);
        }
    }
    let a_v = species_array(e, this, out.len())?;
    for (i, v) in out.iter().enumerate() {
        e.set_prop_value(a_v, &i.to_string(), *v)?;
    }
    Ok(a_v)
}

fn array_reverse(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this)?;
    if let Some(o) = e.heap.obj_mut(id) {
        if let ObjKind::Array(v) = &mut o.kind {
            v.reverse();
        }
    }
    Ok(this)
}

fn array_sort(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this)?;
    let cmp = arg(args, 0);
    let use_cmp = e.is_callable(cmp);
    let mut elems = take_elems(e, id);
    // Insertion sort — page scripts sort small arrays, and this keeps
    // the fallible comparator simple.
    for i in 1..elems.len() {
        let mut j = i;
        while j > 0 {
            let swap = if use_cmp {
                let r = e.call_value(cmp, Value::Undefined, &[elems[j - 1], elems[j]])?;
                e.to_number(r)? > 0.0
            } else {
                let a = e.to_js_string(elems[j - 1])?;
                let b = e.to_js_string(elems[j])?;
                a > b
            };
            if !swap {
                break;
            }
            elems.swap(j - 1, j);
            j -= 1;
        }
    }
    put_elems(e, id, elems);
    Ok(this)
}

fn array_for_each(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this)?;
    let cb = arg(args, 0);
    if !e.is_callable(cb) {
        return Err(e.type_error("forEach needs a function"));
    }
    let this_arg = arg(args, 1);
    let elems = take_elems(e, id);
    for (i, v) in elems.iter().enumerate() {
        e.call_value(cb, this_arg, &[*v, Value::Num(i as f64), this])?;
    }
    put_elems(e, id, elems);
    Ok(Value::Undefined)
}

fn array_map(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this)?;
    let cb = arg(args, 0);
    if !e.is_callable(cb) {
        return Err(e.type_error("map needs a function"));
    }
    let this_arg = arg(args, 1);
    let elems = take_elems(e, id);
    let mut out = Vec::with_capacity(elems.len());
    for (i, v) in elems.iter().enumerate() {
        out.push(e.call_value(cb, this_arg, &[*v, Value::Num(i as f64), this])?);
    }
    put_elems(e, id, elems);
    let a_v = species_array(e, this, out.len())?;
    for (i, v) in out.iter().enumerate() {
        e.set_prop_value(a_v, &i.to_string(), *v)?;
    }
    Ok(a_v)
}

fn array_filter(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this)?;
    let cb = arg(args, 0);
    if !e.is_callable(cb) {
        return Err(e.type_error("filter needs a function"));
    }
    let this_arg = arg(args, 1);
    let elems = take_elems(e, id);
    let mut out = Vec::new();
    for (i, v) in elems.iter().enumerate() {
        let keep = e.call_value(cb, this_arg, &[*v, Value::Num(i as f64), this])?;
        if e.to_boolean(keep) {
            out.push(*v);
        }
    }
    put_elems(e, id, elems);
    let a_v = species_array(e, this, out.len())?;
    for (i, v) in out.iter().enumerate() {
        e.set_prop_value(a_v, &i.to_string(), *v)?;
    }
    Ok(a_v)
}

fn array_reduce(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this)?;
    let cb = arg(args, 0);
    if !e.is_callable(cb) {
        return Err(e.type_error("reduce needs a function"));
    }
    let elems = take_elems(e, id);
    let (mut acc, start) = match args.get(1) {
        Some(v) => (*v, 0),
        None if elems.is_empty() => {
            put_elems(e, id, elems);
            return Err(e.type_error("reduce of empty array"));
        }
        None => (elems[0], 1),
    };
    for (i, v) in elems.iter().enumerate().skip(start) {
        acc = e.call_value(cb, Value::Undefined, &[acc, *v, Value::Num(i as f64), this])?;
    }
    put_elems(e, id, elems);
    Ok(acc)
}

// ------------------------------------------------------------------
// String
// ------------------------------------------------------------------

fn string_to_string(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    this_string_strict(e, this)
}

fn string_char_at(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = this_str(e, this)?;
    let i = match arg(args, 0) {
        Value::Undefined => 0.0,
        v => e.to_number(v)?,
    };
    let c = if i >= 0.0 {
        chars_of(&s).get(i as usize).copied()
    } else {
        None
    };
    Ok(match c {
        Some(c) => e.str_val(c.to_string()),
        None => e.str_val(""),
    })
}

fn string_char_code_at(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = this_str(e, this)?;
    let i = match arg(args, 0) {
        Value::Undefined => 0.0,
        v => e.to_number(v)?,
    };
    let c = if i >= 0.0 {
        chars_of(&s).get(i as usize).copied()
    } else {
        None
    };
    Ok(Value::Num(match c {
        Some(c) => f64::from(c as u32),
        None => f64::NAN,
    }))
}

fn str_index_of(e: &mut Engine, this: Value, args: &[Value], last: bool) -> JsResult<Value> {
    let s = this_str(e, this)?;
    let needle = e.to_js_string(arg(args, 0))?;
    let byte_pos = if last {
        s.rfind(&needle)
    } else {
        let from = match arg(args, 1) {
            Value::Undefined => 0usize,
            v => (e.to_number(v)?.max(0.0)) as usize,
        };
        // `from` is in chars; convert.
        let cs = chars_of(&s);
        if from >= cs.len() {
            None
        } else {
            let byte_from = s.char_indices().nth(from).map_or(s.len(), |(i, _)| i);
            s[byte_from..].find(&needle).map(|p| p + byte_from)
        }
    };
    let char_pos = byte_pos.map_or(-1.0, |b| s[..b].chars().count() as f64);
    Ok(Value::Num(char_pos))
}

fn string_index_of(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    str_index_of(e, this, args, false)
}

fn string_last_index_of(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    str_index_of(e, this, args, true)
}

fn str_slice(e: &mut Engine, s: &str, start: i64, end: Option<i64>) -> Value {
    let cs = chars_of(s);
    let len = cs.len() as i64;
    let a = norm_idx(start, len);
    let b = end.map_or(cs.len(), |e2| norm_idx(e2, len));
    if a >= b {
        return e.str_val("");
    }
    from_chars(e, &cs[a..b])
}

fn string_slice(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = this_str(e, this)?;
    let start = match arg(args, 0) {
        Value::Undefined => 0,
        v => e.to_number(v)? as i64,
    };
    let end = match arg(args, 1) {
        Value::Undefined => None,
        v => Some(e.to_number(v)? as i64),
    };
    Ok(str_slice(e, &s, start, end))
}

fn string_substring(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = this_str(e, this)?;
    let cs = chars_of(&s);
    let len = cs.len() as i64;
    let a = match arg(args, 0) {
        Value::Undefined => 0,
        v => (e.to_number(v)? as i64).clamp(0, len),
    };
    let b = match arg(args, 1) {
        Value::Undefined => len,
        v => (e.to_number(v)? as i64).clamp(0, len),
    };
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    Ok(from_chars(e, &cs[lo as usize..hi as usize]))
}

fn string_substr(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = this_str(e, this)?;
    let cs = chars_of(&s);
    let len = cs.len() as i64;
    let mut start = match arg(args, 0) {
        Value::Undefined => 0,
        v => e.to_number(v)? as i64,
    };
    if start < 0 {
        start = (len + start).max(0);
    }
    let count = match arg(args, 1) {
        Value::Undefined => len - start,
        v => (e.to_number(v)? as i64).max(0),
    };
    let end = (start + count).min(len);
    if start >= end {
        return Ok(e.str_val(""));
    }
    Ok(from_chars(e, &cs[start as usize..end as usize]))
}

fn string_to_lower(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let s = this_str(e, this)?;
    Ok(e.str_val(s.to_lowercase()))
}

fn string_to_upper(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let s = this_str(e, this)?;
    Ok(e.str_val(s.to_uppercase()))
}

fn string_trim(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let s = this_str(e, this)?;
    Ok(e.str_val(s.trim().to_string()))
}

fn string_split(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = this_str(e, this)?;
    // `sep[Symbol.split](this, limit)` (spec `GetMethod`).
    if let Some(v) = sym_method(
        e,
        arg(args, 0),
        crate::interp::SYM_SPLIT,
        &[this, arg(args, 1)],
    )? {
        return Ok(v);
    }
    let limit = match arg(args, 1) {
        Value::Undefined => usize::MAX,
        v => e.to_number(v)? as usize,
    };
    let mut parts: Vec<String> = match arg(args, 0) {
        Value::Undefined => vec![s],
        v => {
            if let Some(re) = re_of(e, v) {
                // Regex split inserts capture groups between pieces.
                if limit == 0 {
                    return Ok(arr_of(e, Vec::new()));
                }
                re.split(&s)
                    .into_iter()
                    .take(limit)
                    .map(str::to_string)
                    .collect()
            } else {
                let sep = e.to_js_string(v)?;
                if sep.is_empty() {
                    s.chars().map(|c| c.to_string()).collect()
                } else {
                    s.split(&sep).map(str::to_string).collect()
                }
            }
        }
    };
    parts.truncate(limit);
    let vals: Vec<Value> = parts.iter().map(|p| e.str_val(p.clone())).collect();
    Ok(arr_of(e, vals))
}

fn string_replace(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    replace_impl(e, this, args, false)
}

fn string_replace_all(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    replace_impl(e, this, args, true)
}

fn replace_impl(e: &mut Engine, this: Value, args: &[Value], all: bool) -> JsResult<Value> {
    let s = this_str(e, this)?;
    let pat = arg(args, 0);
    let rep = arg(args, 1);
    // Spec `replaceAll` : un pattern RegExp sans `g` est rejeté
    // AVANT le dispatch `@@replace` (`IsRegExp` + flags).
    if all {
        if let Some(re) = re_of(e, pat) {
            if !re.global {
                return Err(e.type_error("replaceAll requires a global regex"));
            }
        }
    }
    // `pat[Symbol.replace](this, rep)` (spec `GetMethod`).
    if let Some(v) = sym_method(e, pat, crate::interp::SYM_REPLACE, &[this, rep])? {
        return Ok(v);
    }
    if let Some(re) = re_of(e, pat) {
        return re_replace_impl(e, &re, &s, rep);
    }
    // String pattern — first occurrence (or all for replaceAll).
    let pat = e.to_js_string(pat)?;
    if pat.is_empty() {
        return Ok(e.str_val(s));
    }
    let positions: Vec<usize> = if all {
        s.match_indices(&pat).map(|(i, _)| i).collect()
    } else {
        s.find(&pat).into_iter().collect()
    };
    if positions.is_empty() {
        return Ok(e.str_val(s));
    }
    let mut out = String::with_capacity(s.len());
    let mut cur = 0usize;
    for pos in positions {
        out.push_str(&s[cur..pos]);
        out.push_str(&expand_rep(
            e,
            rep,
            &[],
            &[],
            &s[..pos],
            &s[pos + pat.len()..],
            &pat,
        )?);
        cur = pos + pat.len();
    }
    out.push_str(&s[cur..]);
    Ok(e.str_val(out))
}

/// Coerce a `replace`/`match`/`split`/`search` argument to a
/// `JsRegExp` — `RegExp` objects pass through, strings compile
/// (`str.match("x+")` ≡ `new RegExp("x+")`).
fn re_of(e: &mut Engine, v: Value) -> Option<std::rc::Rc<crate::regexp::JsRegExp>> {
    let Value::Obj(id) = v else { return None };
    match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::RegExp(r)) => Some(r.clone()),
        _ => None,
    }
}

/// `RegExp` object + the value it must coerce to one.
fn as_regexp(e: &mut Engine, v: Value) -> JsResult<(std::rc::Rc<crate::regexp::JsRegExp>, ObjId)> {
    if let Some(r) = re_of(e, v) {
        let Value::Obj(id) = v else {
            return Err(e.type_error("not a RegExp"));
        };
        return Ok((r, id));
    }
    let pat = e.to_js_string(v)?;
    let obj = e.make_regexp(&pat, "")?;
    let Value::Obj(id) = obj else {
        return Err(e.type_error("not a RegExp"));
    };
    match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::RegExp(r)) => Ok((r.clone(), id)),
        _ => Err(e.type_error("not a RegExp")),
    }
}

/// Read `rx.lastIndex` (char units).
fn re_last_index(e: &mut Engine, id: ObjId) -> JsResult<usize> {
    let v = e.get_prop(id, "lastIndex").unwrap_or(Value::Num(0.0));
    let n = e.to_number(v)?;
    Ok(if n.is_sign_negative() || n.is_nan() {
        0
    } else {
        n as usize
    })
}

/// Expand `$$`, `$&`, `` $` ``, `$'`, `$n`/`$nn`, `$<name>` in a
/// string replacement. Function replacements are handled by caller.
fn expand_rep(
    e: &mut Engine,
    rep: Value,
    groups: &[Option<String>],
    named: &[(String, String)],
    before: &str,
    after: &str,
    matched: &str,
) -> JsResult<String> {
    let rep = e.to_js_string(rep)?;
    let rc = chars_of(&rep);
    let mut out = String::with_capacity(rep.len());
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
                    out.push_str(before);
                    i += 2;
                    continue;
                }
                '\'' => {
                    out.push_str(after);
                    i += 2;
                    continue;
                }
                '<' => {
                    if let Some(end) = rc[i + 2..].iter().position(|&c| c == '>') {
                        let name: String = rc[i + 2..i + 2 + end].iter().collect();
                        if let Some((_, v)) = named.iter().find(|(k, _)| *k == name) {
                            out.push_str(v);
                        }
                        i += end + 3;
                        continue;
                    }
                }
                c if c.is_ascii_digit() => {
                    let d1 = rc[i + 1].to_digit(10).unwrap_or(0) as usize;
                    let d2 = rc
                        .get(i + 2)
                        .and_then(|c| c.to_digit(10))
                        .map(|d| d as usize);
                    // `$nn` counts only if group nn exists.
                    let (n, step) = match d2 {
                        Some(d2) if d1 * 10 + d2 > 0 && d1 * 10 + d2 < groups.len() => {
                            (d1 * 10 + d2, 3)
                        }
                        _ => (d1, 2),
                    };
                    if n > 0 && n < groups.len() {
                        if let Some(g) = &groups[n] {
                            out.push_str(g);
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

/// Apply `matches` against `s` with string-or-function replacement.
fn splice_matches(
    e: &mut Engine,
    s: &str,
    matches: &[crate::regexp::ExecMatch],
    rep: Value,
) -> JsResult<Value> {
    let use_fn = e.is_callable(rep);
    let mut out = String::with_capacity(s.len() + matches.len() * 8);
    let mut cur = 0usize; // byte offset
    for m in matches {
        out.push_str(&s[cur..m.start_b]);
        if use_fn {
            // f(match, g1…gn, index, string [, groups])
            let mut call_args = Vec::with_capacity(m.groups.len() + 3);
            call_args.push(e.str_val(m.text.clone()));
            for g in &m.groups[1..] {
                call_args.push(match g {
                    Some(t) => e.str_val(t.clone()),
                    None => Value::Undefined,
                });
            }
            call_args.push(Value::Num(m.index as f64));
            call_args.push(e.str_val(s.to_string()));
            if !m.named.is_empty() {
                let gobj = e.new_object();
                for (k, v) in &m.named {
                    let vv = e.str_val(v.clone());
                    e.set_own(gobj, k, vv, true);
                }
                call_args.push(Value::Obj(gobj));
            }
            let r = e.call_value(rep, Value::Undefined, &call_args)?;
            out.push_str(&e.to_js_string(r)?);
        } else {
            out.push_str(&expand_rep(
                e,
                rep,
                &m.groups,
                &m.named,
                &s[..m.start_b],
                &s[m.end_b..],
                &m.text,
            )?);
        }
        cur = m.end_b;
    }
    out.push_str(&s[cur..]);
    Ok(e.str_val(out))
}

/// `exec`/`test` result array: `[m0, m1…].index/.input/.groups`.
fn match_result(
    e: &mut Engine,
    m: &crate::regexp::ExecMatch,
    re: &crate::regexp::JsRegExp,
    input: &str,
) -> Value {
    let elems: Vec<Value> = m
        .groups
        .iter()
        .map(|g| match g {
            Some(t) => e.str_val(t.clone()),
            None => Value::Undefined,
        })
        .collect();
    let Value::Obj(arr) = e.new_array(elems) else {
        return Value::Null;
    };
    e.set_own(arr, "index", Value::Num(m.index as f64), false);
    let input_v = e.str_val(input.to_string());
    e.set_own(arr, "input", input_v, false);
    let groups_v = if m.named.is_empty() {
        Value::Undefined
    } else {
        let gobj = e.new_object();
        for (k, v) in &m.named {
            let vv = e.str_val(v.clone());
            e.set_own(gobj, k, vv, true);
        }
        Value::Obj(gobj)
    };
    e.set_own(arr, "groups", groups_v, false);
    // `d` flag : `indices` = [[start, end] | undefined] par groupe
    // (+ `indices.groups` miroir des groupes nommés).
    if re.has_indices {
        let idx: Vec<Value> = m
            .group_indices
            .iter()
            .map(|r| match r {
                Some((s, t)) => e.new_array(vec![Value::Num(*s as f64), Value::Num(*t as f64)]),
                None => Value::Undefined,
            })
            .collect();
        let Value::Obj(ia) = e.new_array(idx) else {
            return Value::Obj(arr);
        };
        if !m.named_indices.is_empty() {
            let gobj = e.new_object();
            for (k, r) in &m.named_indices {
                let v = match r {
                    Some((s, t)) => e.new_array(vec![Value::Num(*s as f64), Value::Num(*t as f64)]),
                    None => Value::Undefined,
                };
                e.set_own(gobj, k, v, true);
            }
            e.set_own(ia, "groups", Value::Obj(gobj), false);
        }
        e.set_own(arr, "indices", Value::Obj(ia), false);
    }
    Value::Obj(arr)
}

fn regexp_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    // `new RegExp(re)` clones the pattern; `RegExp(str, flags)`.
    let (pat, flags) = match arg(args, 0) {
        v @ Value::Obj(_) => {
            if let Some(r) = re_of(e, v) {
                let flags = match arg(args, 1) {
                    Value::Undefined => r.flag_string(),
                    f => e.to_js_string(f)?,
                };
                (r.source.clone(), flags)
            } else {
                let p = e.to_js_string(v)?;
                (p, String::new())
            }
        }
        Value::Undefined => (String::new(), String::new()),
        v => {
            let p = e.to_js_string(v)?;
            (p, String::new())
        }
    };
    let flags = if flags.is_empty() {
        match arg(args, 1) {
            Value::Undefined => flags,
            f => e.to_js_string(f)?,
        }
    } else {
        flags
    };
    e.make_regexp(&pat, &flags)
}

fn regexp_exec(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (re, id) = as_regexp(e, this)?;
    let s = e.to_js_string(arg(args, 0))?;
    let use_last = re.global || re.sticky;
    let last = if use_last { re_last_index(e, id)? } else { 0 };
    if let Some((m, new_last)) = re.exec_from(&s, last) {
        if use_last {
            let _ = e.set_prop(id, "lastIndex", Value::Num(new_last as f64));
        }
        Ok(match_result(e, &m, &re, &s))
    } else {
        if use_last {
            let _ = e.set_prop(id, "lastIndex", Value::Num(0.0));
        }
        Ok(Value::Null)
    }
}

fn regexp_test(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let r = regexp_exec(e, this, args)?;
    Ok(Value::Bool(!matches!(r, Value::Null)))
}

/// `RegExp.escape(s)` — ES2025 : échappe `s` pour insertion
/// littérale dans un pattern. `SyntaxCharacter`s/`/` → `\c` ;
/// en tête, tout autre caractère ASCII non sûr (ponctuation,
/// chiffres, lettres, espace) → `\xNN` ou `\uNNNN`.
fn regexp_escape(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    const SYNTAX: &str = "^$\\.*+?()[]{}|/";
    let src = e.to_js_string(arg(args, 0))?;
    let mut out = String::with_capacity(src.len() * 2);
    for (i, c) in src.chars().enumerate() {
        let esc_hex = i == 0 && (c.is_ascii_alphanumeric() || c.is_ascii_punctuation() || c == ' ');
        if SYNTAX.contains(c) {
            out.push('\\');
            out.push(c);
        } else if esc_hex {
            let cp = c as u32;
            if cp <= 0xff {
                out.push_str(&format!("\\x{cp:02x}"));
            } else {
                out.push_str(&format!("\\u{cp:04x}"));
            }
        } else {
            out.push(c);
        }
    }
    Ok(e.str_val(out))
}

/// `s.isWellFormed()` — nos `char` sont toujours des scalaires
/// valides : `true` (les surrogates orphelins dégradent en
/// U+FFFD au lexing).
fn string_is_well_formed(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    this_str(e, this)?;
    Ok(Value::Bool(true))
}

/// `s.toWellFormed()` — identité (même raison).
fn string_to_well_formed(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let s = this_str(e, this)?;
    Ok(e.str_val(s))
}

fn regexp_to_string(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let (re, _id) = as_regexp(e, this)?;
    let src = if re.source.is_empty() {
        "(?:)"
    } else {
        re.source.as_str()
    };
    Ok(e.str_val(format!("/{src}/{}", re.flag_string())))
}

fn string_match(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = this_str(e, this)?;
    // `rx[Symbol.match](this)` (spec `GetMethod`).
    if let Some(v) = sym_method(e, arg(args, 0), crate::interp::SYM_MATCH, &[this])? {
        return Ok(v);
    }
    let (re, id) = as_regexp(e, arg(args, 0))?;
    re_match_exec(e, &re, id, &s)
}

fn string_search(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = this_str(e, this)?;
    // `rx[Symbol.search](this)` (spec `GetMethod`).
    if let Some(v) = sym_method(e, arg(args, 0), crate::interp::SYM_SEARCH, &[this])? {
        return Ok(v);
    }
    let (re, _id) = as_regexp(e, arg(args, 0))?;
    match re.exec(&s, 0) {
        Some(m) => Ok(Value::Num(m.index as f64)),
        None => Ok(Value::Num(-1.0)),
    }
}

fn string_concat(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let mut s = this_str(e, this)?;
    for a in args {
        s.push_str(&e.to_js_string(*a)?);
    }
    Ok(e.str_val(s))
}

fn string_from_char_code(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let mut out = String::new();
    for a in args {
        let n = e.to_number(*a)? as u32;
        out.push(char::from_u32(n).unwrap_or('\u{fffd}'));
    }
    Ok(e.str_val(out))
}

// ------------------------------------------------------------------
// Number / Boolean / Error prototypes
// ------------------------------------------------------------------

fn number_to_string(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let n = this_num(e, this)?;
    let radix = match arg(args, 0) {
        Value::Undefined => 10,
        v => e.to_number(v)? as u32,
    };
    if radix == 10 || !(2..=36).contains(&radix) {
        return Ok(e.str_val(num_to_js_string(n)));
    }
    if !n.is_finite() {
        return Ok(e.str_val(num_to_js_string(n)));
    }
    let neg = n < 0.0;
    let mut int = n.abs().trunc() as u64;
    let mut digits = Vec::new();
    loop {
        digits.push(std::char::from_digit((int % u64::from(radix)) as u32, radix).unwrap_or('0'));
        int /= u64::from(radix);
        if int == 0 {
            break;
        }
    }
    if neg {
        digits.push('-');
    }
    Ok(e.str_val(digits.iter().rev().collect::<String>()))
}

fn number_value_of(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    match this_prim(e, this) {
        Some(v @ Value::Num(_)) => Ok(v),
        _ => Err(e.type_error("Number.prototype.valueOf on incompatible receiver")),
    }
}

fn number_to_fixed(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let n = this_num(e, this)?;
    let d = match arg(args, 0) {
        Value::Undefined => 0usize,
        v => (e.to_number(v)? as i64).clamp(0, 20) as usize,
    };
    if !n.is_finite() {
        return Ok(e.str_val(num_to_js_string(n)));
    }
    Ok(e.str_val(format!("{n:.d$}")))
}

fn boolean_to_string(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    match this {
        Value::Bool(b) => Ok(e.str_val(if b { "true" } else { "false" })),
        _ => Err(e.type_error("Boolean method on non-boolean")),
    }
}

fn boolean_value_of(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    match this_prim(e, this) {
        Some(v @ Value::Bool(_)) => Ok(v),
        _ => Err(e.type_error("Boolean.prototype.valueOf on incompatible receiver")),
    }
}

fn error_to_string(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let (name, msg) = match this {
        Value::Obj(id) => {
            let n = e.get_prop(id, "name").map_or_else(
                || "Error".into(),
                |v| e.to_js_string(v).unwrap_or_else(|_| "Error".into()),
            );
            let m = e
                .get_prop(id, "message")
                .map(|v| e.to_js_string(v).unwrap_or_default())
                .unwrap_or_default();
            (n, m)
        }
        _ => ("Error".into(), String::new()),
    };
    Ok(e.str_val(if msg.is_empty() {
        name
    } else {
        format!("{name}: {msg}")
    }))
}

// ------------------------------------------------------------------
// Math
// ------------------------------------------------------------------

macro_rules! math1 {
    ($name:ident, $f:expr) => {
        fn $name(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
            let f: fn(f64) -> f64 = $f;
            Ok(Value::Num(f(e.to_number(arg(args, 0))?)))
        }
    };
}

math1!(math_abs, f64::abs);
math1!(math_floor, f64::floor);
math1!(math_ceil, f64::ceil);
math1!(math_sqrt, f64::sqrt);
math1!(math_exp, f64::exp);
math1!(math_log, f64::ln);
math1!(math_sin, f64::sin);
math1!(math_cos, f64::cos);
math1!(math_tan, f64::tan);
math1!(math_atan, f64::atan);

fn math_round(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    // JS rounds half up (toward +∞).
    Ok(Value::Num((e.to_number(arg(args, 0))? + 0.5).floor()))
}

fn math_max(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let mut m = f64::NEG_INFINITY;
    for a in args {
        let n = e.to_number(*a)?;
        if n.is_nan() {
            return Ok(Value::Num(f64::NAN));
        }
        m = m.max(n);
    }
    Ok(Value::Num(if args.is_empty() {
        f64::NEG_INFINITY
    } else {
        m
    }))
}

fn math_min(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let mut m = f64::INFINITY;
    for a in args {
        let n = e.to_number(*a)?;
        if n.is_nan() {
            return Ok(Value::Num(f64::NAN));
        }
        m = m.min(n);
    }
    Ok(Value::Num(if args.is_empty() { f64::INFINITY } else { m }))
}

fn math_pow(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let b = e.to_number(arg(args, 0))?;
    let x = e.to_number(arg(args, 1))?;
    Ok(Value::Num(b.powf(x)))
}

fn math_atan2(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let y = e.to_number(arg(args, 0))?;
    let x = e.to_number(arg(args, 1))?;
    Ok(Value::Num(y.atan2(x)))
}

fn math_random(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    // xorshift64* — fine for page scripts, not crypto.
    e.rng ^= e.rng >> 12;
    e.rng ^= e.rng << 25;
    e.rng ^= e.rng >> 27;
    let v = e.rng.wrapping_mul(0x2545_f491_4f6c_dd1d);
    Ok(Value::Num((v >> 11) as f64 / 9_007_199_254_740_992.0))
}

// ------------------------------------------------------------------
// JS-7f — eval/Function/bind, Symbol, Reflect, misc builtins
// ------------------------------------------------------------------

/// `eval(src)` — *indirect* eval semantics: the source runs in the
/// global scope (direct eval reaching the caller's locals is a
/// documented deviation — the tree-walker's env chain isn't
/// re-enterable mid-call). Non-string args return as-is (spec).
fn eval_fn(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    match arg(args, 0) {
        Value::Str(id) => {
            let src = e.heap.string(id).unwrap_or_default();
            e.eval_source(&src)
        }
        v => Ok(v),
    }
}

/// `Function("a","b","body")` — parses `function anonymous(a,b){body}`
/// and binds it to the *global* env (spec: `Function` functions are
/// never closures).
fn function_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let body = if args.is_empty() {
        String::new()
    } else {
        e.to_js_string(args[args.len() - 1])?
    };
    let mut params = String::new();
    for p in &args[..args.len().saturating_sub(1)] {
        if !params.is_empty() {
            params.push(',');
        }
        params.push_str(&e.to_js_string(*p)?);
    }
    let src = format!("function anonymous({params}) {{\n{body}\n}}");
    let prog = crate::parser::parse(&src).map_err(|err| {
        crate::heap::Thrown(e.error_value(
            e.protos.syntax_error,
            "SyntaxError",
            &format!("Function: {err}"),
        ))
    })?;
    match prog.body.first() {
        Some(crate::ast::Stmt::FunctionDecl(lit)) => {
            let env = e.global_env.clone();
            Ok(e.make_function(lit, &env, Value::Undefined))
        }
        _ => Err(e.syntax_error("Function: unparseable source")),
    }
}

/// `fn.bind(thisArg, ...bound)` → `Fun::Bound` wrapper.
fn function_bind(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    if !e.is_callable(this) {
        return Err(e.type_error("bind called on non-callable"));
    }
    let bound_this = arg(args, 0);
    let bound = args.get(1..).unwrap_or(&[]).to_vec();
    let proto = Value::Obj(e.protos.function);
    Ok(Value::Obj(e.heap.alloc_obj(crate::heap::Object::fun(
        crate::heap::Fun::Bound {
            target: this,
            this: bound_this,
            bound,
        },
        proto,
    ))))
}

// -- Symbol ---------------------------------------------------------

/// `Symbol(desc?)` — unique `ObjKind::Symbol` per call (identity by
/// `ObjId` ≙ spec identity semantics; prop keys stay strings, so
/// well-known symbols are the existing sentinels).
fn symbol_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let desc = match arg(args, 0) {
        Value::Undefined => None,
        v => Some(e.to_js_string(v)?),
    };
    Ok(e.new_symbol(desc))
}

// ---- BigInt (JS-C7) --------------------------------------------------

/// `this` d'une méthode `BigInt.prototype` : primitive `Value::Big`
/// ou wrapper `Object(1n)`/`new` portant `__prim`.
fn bigint_this(e: &mut Engine, this: Value) -> JsResult<num_bigint::BigInt> {
    let prim = match this {
        v @ Value::Big(_) => v,
        Value::Obj(id) => {
            let own = e.heap.obj(id).and_then(|o| o.get_own(PRIM));
            match own {
                Some(p @ Value::Big(_)) => p,
                _ => {
                    return Err(
                        e.type_error("BigInt.prototype method called on incompatible receiver")
                    );
                }
            }
        }
        _ => {
            return Err(e.type_error("BigInt.prototype method called on incompatible receiver"));
        }
    };
    match e.bigint_of(prim) {
        Some(b) => Ok(b),
        None => Err(e.type_error("invalid BigInt value")),
    }
}

/// `BigInt(x)` — fonctionnel uniquement (`new` → `TypeError`, spec
/// `isConstructor` interdit). `x` numérique doit être entier.
fn bigint_ctor(e: &mut Engine, t: Value, args: &[Value]) -> JsResult<Value> {
    if constructed_this(e, t, e.protos.bigint).is_some() {
        return Err(e.type_error("BigInt is not a constructor"));
    }
    let v = arg(args, 0);
    if let Value::Num(n) = v {
        if !n.is_finite() || n.fract() != 0.0 {
            return Err(e.range_error("the number is not a safe integer"));
        }
        let b = num_traits::FromPrimitive::from_f64(n).unwrap_or_else(num_bigint::BigInt::default);
        return Ok(e.big_val(b));
    }
    let prim = if let Value::Obj(_) = v {
        e.to_primitive(v, crate::interp::PrimHint::Number)?
    } else {
        v
    };
    // `to_bigint` produit le SyntaxError spec pour les chaînes
    // non-parsables (`StringToBigInt`).
    let b = e.to_bigint(prim)?;
    Ok(e.big_val(b))
}

fn bigint_to_string(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let b = bigint_this(e, this)?;
    let radix = match arg(args, 0) {
        Value::Undefined => 10,
        v => {
            let n = e.to_number(v)?;
            if !(2.0..=36.0).contains(&n) || n.fract() != 0.0 {
                return Err(e.range_error("toString() radix must be between 2 and 36"));
            }
            n as u32
        }
    };
    Ok(e.str_val(b.to_str_radix(radix)))
}

fn bigint_value_of(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let b = bigint_this(e, this)?;
    Ok(e.big_val(b))
}

/// `ToIndex` simplifié : entier ≥ 0 (borné à 2^53-1 implicite f64).
fn to_bits(e: &mut Engine, v: Value) -> JsResult<u32> {
    let n = e.to_number(v)?;
    if !n.is_finite() || n < 0.0 || n.fract() != 0.0 {
        return Err(e.range_error("invalid bigint bit width"));
    }
    Ok(n as u32)
}

fn bigint_as_int_n(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let bits = to_bits(e, arg(args, 0))?;
    let b = e.to_bigint(arg(args, 1))?;
    if bits == 0 {
        return Ok(e.big_val(num_bigint::BigInt::from(0)));
    }
    let m = num_bigint::BigInt::from(1) << bits;
    let mut r = &b % &m;
    if r.sign() == num_bigint::Sign::Minus {
        r += &m;
    }
    let half = &m >> 1u32;
    if r >= half {
        r -= &m;
    }
    Ok(e.big_val(r))
}

fn bigint_as_uint_n(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let bits = to_bits(e, arg(args, 0))?;
    let b = e.to_bigint(arg(args, 1))?;
    if bits == 0 {
        return Ok(e.big_val(num_bigint::BigInt::from(0)));
    }
    let m = num_bigint::BigInt::from(1) << bits;
    let mut r = &b % &m;
    if r.sign() == num_bigint::Sign::Minus {
        r += &m;
    }
    Ok(e.big_val(r))
}

/// `Symbol.for(key)` — global registry on the engine (survives
/// heap traffic; `===` on a `for` symbol is identity-stable).
fn symbol_for(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let key = e.to_js_string(arg(args, 0))?;
    if let Some(v) = e.symbol_reg.get(&key) {
        return Ok(*v);
    }
    // `Symbol.for` symbols carry their key as description (spec).
    let sym = symbol_ctor(e, Value::Undefined, args)?;
    e.symbol_reg.insert(key, sym);
    Ok(sym)
}

/// `Symbol.keyFor(sym)` — reverse lookup over the registry.
fn symbol_key_for(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let sym = arg(args, 0);
    let hit = e
        .symbol_reg
        .iter()
        .find(|(_, v)| e.strict_eq(**v, sym))
        .map(|(k, _)| k.clone());
    Ok(hit.map_or(Value::Undefined, |k| e.str_val(k)))
}

fn sym_desc(e: &Engine, this: Value) -> Option<String> {
    let Value::Obj(id) = this else { return None };
    match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::Symbol { desc, .. }) => Some(desc.clone().unwrap_or_default()),
        _ => None,
    }
}

fn symbol_to_string(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let d =
        sym_desc(e, this).ok_or_else(|| e.type_error("Symbol.prototype.toString on non-symbol"))?;
    Ok(e.str_val(if d.is_empty() {
        "Symbol()".to_string()
    } else {
        format!("Symbol({d})")
    }))
}

fn symbol_value_of(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    if sym_desc(e, this).is_none() {
        return Err(e.type_error("Symbol.prototype.valueOf on non-symbol"));
    }
    Ok(this)
}

/// `Symbol.prototype.description` getter.
fn symbol_description(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    match sym_desc(e, this) {
        Some(d) if !d.is_empty() => Ok(e.str_val(d)),
        Some(_) => Ok(Value::Undefined),
        None => Err(e.type_error("description on non-symbol")),
    }
}

// -- Reflect ---------------------------------------------------------

fn reflect_get(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = arg(args, 0) else {
        return Err(e.type_error("Reflect.get on non-object"));
    };
    let k = e.prop_key(arg(args, 1))?;
    e.get_prop_value(Value::Obj(id), &k)
}

fn reflect_set(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = arg(args, 0) else {
        return Err(e.type_error("Reflect.set on non-object"));
    };
    let k = e.prop_key(arg(args, 1))?;
    e.set_prop_value(Value::Obj(id), &k, arg(args, 2))?;
    Ok(Value::Bool(true))
}

fn reflect_has(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = arg(args, 0) else {
        return Ok(Value::Bool(false));
    };
    let k = e.prop_key(arg(args, 1))?;
    Ok(Value::Bool(e.has_prop(id, &k)?))
}

fn reflect_own_keys(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = arg(args, 0) else {
        return Ok(e.new_array(Vec::new()));
    };
    let keys = e.own_keys_trapped(id)?;
    // Clés-symboles `\u{f130}…` rendues ; internes `\u{f131}…` filtrées.
    let vals = keys
        .iter()
        .filter(|k| !k.starts_with('\u{f131}'))
        .map(|k| e.str_val(k.clone()))
        .collect();
    Ok(e.new_array(vals))
}

fn reflect_apply(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let f = arg(args, 0);
    let this = arg(args, 1);
    let list = match arg(args, 2) {
        Value::Obj(id) => e.iter_values(Value::Obj(id))?,
        _ => Vec::new(),
    };
    e.call_value(f, this, &list)
}

fn reflect_construct(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let f = arg(args, 0);
    let list = match arg(args, 1) {
        Value::Obj(id) => e.iter_values(Value::Obj(id))?,
        _ => Vec::new(),
    };
    // `newTarget` optionnel — `new.target` vu par `f` (spec :
    // défaut = la cible).
    let nt = match arg(args, 2) {
        Value::Undefined | Value::Null => f,
        v => v,
    };
    e.construct_nt(f, &list, nt)
}

/// `Reflect.defineProperty(t, k, desc)` — mêmes règles que
/// `Object.defineProperty` mais rend `false` sur échec au lieu de
/// jeter (spec `Reflect.defineProperty` — les erreurs internes de
/// `[[DefineOwnProperty]]` surface en `false`, déviation documentée :
/// un getter de descripteur qui jette est aussi rendu `false`).
fn reflect_define_property(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = arg(args, 0) else {
        return Err(e.type_error("Reflect.defineProperty on non-object"));
    };
    let k = e.prop_key(arg(args, 1))?;
    let Value::Obj(desc) = arg(args, 2) else {
        return Err(e.type_error("Reflect.defineProperty: descriptor is not an object"));
    };
    Ok(Value::Bool(define_one_prop(e, id, &k, desc).is_ok()))
}

/// `Reflect.deleteProperty(t, k)` — `false` si la prop refuse,
/// jamais de `TypeError` de suppression (contrairement au `delete`
/// strict).
fn reflect_delete_property(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = arg(args, 0) else {
        return Err(e.type_error("Reflect.deleteProperty on non-object"));
    };
    let k = e.prop_key(arg(args, 1))?;
    // `delete_result` jette en mode strict sur refus — pour
    // `Reflect` tout échec se rabat sur `false` (déviation
    // documentée : une trap `deleteProperty` qui jette est aussi
    // rendue `false`).
    Ok(Value::Bool(
        e.delete_result(id, &k).is_ok_and(|v| e.to_boolean(v)),
    ))
}

/// `Reflect.getOwnPropertyDescriptor(t, k)` — objet descripteur ou
/// `undefined` ; passe par la trap `getOwnPropertyDescriptor`.
fn reflect_get_own_property_descriptor(
    e: &mut Engine,
    _t: Value,
    args: &[Value],
) -> JsResult<Value> {
    let Value::Obj(id) = arg(args, 0) else {
        return Err(e.type_error("Reflect.getOwnPropertyDescriptor on non-object"));
    };
    let k = e.prop_key(arg(args, 1))?;
    let Some(p) = e.get_own_prop(id, &k)? else {
        return Ok(Value::Undefined);
    };
    Ok(prop_desc_object(e, p))
}

/// `Reflect.getPrototypeOf(t)` — `TypeError` sur primitive
/// (contrairement à `Object.getPrototypeOf` qui ignore).
fn reflect_get_prototype_of(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    match arg(args, 0) {
        v @ Value::Obj(_) => object_get_proto(e, Value::Undefined, &[v]),
        _ => Err(e.type_error("Reflect.getPrototypeOf on non-object")),
    }
}

/// `Reflect.setPrototypeOf(t, p)` — `false` sur refus (cycle,
/// non-extensible, trap `false`), `TypeError` sur args invalides.
fn reflect_set_prototype_of(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let target = arg(args, 0);
    if !matches!(target, Value::Obj(_)) {
        return Err(e.type_error("Reflect.setPrototypeOf on non-object"));
    }
    if !matches!(arg(args, 1), Value::Obj(_) | Value::Null) {
        return Err(e.type_error("Reflect.setPrototypeOf: proto must be an object or null"));
    }
    match object_set_proto(e, Value::Undefined, &[target, arg(args, 1)]) {
        Ok(_) => Ok(Value::Bool(true)),
        Err(_) => Ok(Value::Bool(false)),
    }
}

/// `Reflect.isExtensible(t)` — `TypeError` sur primitive.
fn reflect_is_extensible(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    match arg(args, 0) {
        v @ Value::Obj(_) => object_is_extensible(e, Value::Undefined, &[v]),
        _ => Err(e.type_error("Reflect.isExtensible on non-object")),
    }
}

/// `Reflect.preventExtensions(t)` — `true` si l'opération a réussi,
/// `false` sur refus (trap `false` ou cible déjà scellée).
fn reflect_prevent_extensions(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    match arg(args, 0) {
        v @ Value::Obj(_) => Ok(Value::Bool(
            object_prevent_extensions(e, Value::Undefined, &[v]).is_ok(),
        )),
        _ => Err(e.type_error("Reflect.preventExtensions on non-object")),
    }
}

// -- Proxy ------------------------------------------------------------

/// Alloue `new Proxy(target, handler)` — les deux arguments doivent
/// être des objets (spec `Proxy` step 1-2).
fn new_proxy(e: &mut Engine, target: Value, handler: Value) -> JsResult<Value> {
    if !matches!(target, Value::Obj(_)) || !matches!(handler, Value::Obj(_)) {
        return Err(e.type_error("Proxy target and handler must be objects"));
    }
    // Proto apparent du proxy = celui de la cible (approximation
    // pour les marches `for-in`/`instanceof` qui ne passent pas
    // par la trap `getPrototypeOf`).
    let proto = match target {
        Value::Obj(tid) => e.heap.obj(tid).map_or(Value::Null, |o| o.proto),
        _ => Value::Null,
    };
    Ok(Value::Obj(e.heap.alloc_obj(Object {
        proto,
        props: Vec::new(),
        kind: ObjKind::Proxy {
            target,
            handler: Some(handler),
        },
        extensible: true,
    })))
}

/// `new Proxy(t, h)` — construct-only : l'appel fonctionnel lève
/// `TypeError` (heuristique `this` = objet frais sur
/// `Proxy.prototype`, même technique que `String`/`Number`).
fn proxy_ctor(e: &mut Engine, t: Value, args: &[Value]) -> JsResult<Value> {
    if constructed_this(e, t, e.protos.proxy).is_none() {
        return Err(e.type_error("Proxy constructor must be called with `new`"));
    }
    new_proxy(e, arg(args, 0), arg(args, 1))
}

/// `Proxy.revocable(t, h)` → `{proxy, revoke}` — `revoke` est une
/// native liée au proxy (`BoundNative`) qui nulle son handler.
fn proxy_revocable(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let p = new_proxy(e, arg(args, 0), arg(args, 1))?;
    let revoke = e.bound_native("revoke", proxy_revoke, vec![p]);
    let out = e.new_object();
    e.set_own(out, "proxy", p, true);
    e.set_own(out, "revoke", revoke, true);
    Ok(Value::Obj(out))
}

/// `revoke()` — l'objet proxy est passé en arg lié (`bound[0]`) ;
/// après appel, toute opération sur le proxy lève `TypeError`.
fn proxy_revoke(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    if let Some(Value::Obj(id)) = args.first() {
        if let Some(o) = e.heap.obj_mut(*id) {
            if let ObjKind::Proxy { handler, .. } = &mut o.kind {
                *handler = None;
            }
        }
    }
    Ok(Value::Undefined)
}

// -- performance ------------------------------------------------------

/// `performance.now()` — ms since engine creation (monotonic).
fn performance_now(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    Ok(Value::Num(e.t0.elapsed().as_secs_f64() * 1000.0))
}

// -- encodeURI / decodeURI -------------------------------------------

/// RFC 3986 reserved + URI-safe set kept verbatim by `encodeURI`
/// (component version escapes `; / ? : @ & = + $ , #` too).
const URI_SAFE_FULL: &str =
    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_.!~*'();/?:@&=+$,#";

fn uri_escape(s: &str, safe: &str) -> String {
    let mut out = String::new();
    for &b in s.as_bytes() {
        let c = b as char;
        if c.is_ascii() && safe.contains(c) {
            out.push(c);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn uri_unescape(e: &mut Engine, s: &str) -> JsResult<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            if i + 2 >= b.len() {
                return Err(e.uri_error("malformed URI escape"));
            }
            let hex = std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("");
            let Ok(v) = u8::from_str_radix(hex, 16) else {
                return Err(e.uri_error("malformed URI escape"));
            };
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| e.uri_error("invalid utf-8 in URI"))
}

fn encode_uri(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let s = e.to_js_string(arg(args, 0))?;
    Ok(e.str_val(uri_escape(&s, URI_SAFE_FULL)))
}

fn decode_uri(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let s = e.to_js_string(arg(args, 0))?;
    let out = uri_unescape(e, &s)?;
    Ok(e.str_val(out))
}

// -- Array extras ------------------------------------------------------

/// `Array.isArray(v)`.
fn array_is_array(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    Ok(Value::Bool(matches!(arg(args, 0), Value::Obj(id)
        if matches!(e.heap.obj(id).map(|o| &o.kind), Some(ObjKind::Array(_))))))
}

/// `Array.from(iterable|arrayLike)` — iterator protocol, then a
/// `length`-indexed fallback; strings iterate by char.
fn array_from(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let v = arg(args, 0);
    let items = match v {
        Value::Str(id) => {
            let s = e.heap.string(id).unwrap_or_default();
            s.chars()
                .map(|c| e.str_val(c.to_string()))
                .collect::<Vec<_>>()
        }
        Value::Obj(_) => e.iter_values(v).unwrap_or_default(),
        _ => Vec::new(),
    };
    Ok(e.new_array(items))
}

fn array_includes(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this)?;
    let needle = arg(args, 0);
    let elems = take_elems(e, id);
    // SameValueZero (NaN matches itself here).
    let found = elems.iter().any(|&v| {
        e.strict_eq(v, needle)
            || matches!((v, needle), (Value::Num(a), Value::Num(b)) if a.is_nan() && b.is_nan())
    });
    put_elems(e, id, elems);
    Ok(Value::Bool(found))
}

/// Shared `find`/`findIndex`/`some`/`every` loop — `cb(v, i, arr)`.
fn array_iter_find(
    e: &mut Engine,
    this: Value,
    args: &[Value],
) -> JsResult<(Vec<Value>, Vec<bool>, ObjId)> {
    let id = this_array(e, this)?;
    let cb = arg(args, 0);
    if !e.is_callable(cb) {
        return Err(e.type_error("callback is not a function"));
    }
    let elems = take_elems(e, id);
    let mut hits = Vec::with_capacity(elems.len());
    for (i, &v) in elems.iter().enumerate() {
        let r = e.call_value(cb, arg(args, 1), &[v, Value::Num(i as f64), this])?;
        hits.push(e.to_boolean(r));
    }
    Ok((elems, hits, id))
}

fn array_find(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (elems, hits, id) = array_iter_find(e, this, args)?;
    let out = elems
        .iter()
        .zip(&hits)
        .find_map(|(&v, &h)| h.then_some(v))
        .unwrap_or(Value::Undefined);
    put_elems(e, id, elems);
    Ok(out)
}

fn array_find_index(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (elems, hits, id) = array_iter_find(e, this, args)?;
    let out = hits.iter().position(|&h| h).map_or(-1.0, |i| i as f64);
    put_elems(e, id, elems);
    Ok(Value::Num(out))
}

fn array_some(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (elems, hits, id) = array_iter_find(e, this, args)?;
    let out = hits.iter().any(|&h| h);
    put_elems(e, id, elems);
    Ok(Value::Bool(out))
}

fn array_every(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (elems, hits, id) = array_iter_find(e, this, args)?;
    let out = hits.iter().all(|&h| h);
    put_elems(e, id, elems);
    Ok(Value::Bool(out))
}

// -- String extras -----------------------------------------------------

fn string_includes(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = this_str(e, this)?;
    let needle = e.to_js_string(arg(args, 0))?;
    Ok(Value::Bool(s.contains(&needle)))
}

fn string_starts_with(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = this_str(e, this)?;
    let needle = e.to_js_string(arg(args, 0))?;
    let start = match arg(args, 1) {
        Value::Undefined => 0usize,
        v => (e.to_number(v)? as usize).min(s.len()),
    };
    Ok(Value::Bool(s[start..].starts_with(&needle)))
}

fn string_ends_with(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = this_str(e, this)?;
    let needle = e.to_js_string(arg(args, 0))?;
    let end = match arg(args, 1) {
        Value::Undefined => s.len(),
        v => (e.to_number(v)? as usize).min(s.len()),
    };
    Ok(Value::Bool(s[..end].ends_with(&needle)))
}

fn string_repeat(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = this_str(e, this)?;
    let n = e.to_number(arg(args, 0))?;
    if n < 0.0 || !n.is_finite() {
        return Err(e.range_error("repeat count out of range"));
    }
    Ok(e.str_val(s.repeat(n as usize)))
}

fn pad(e: &mut Engine, s: &str, args: &[Value], right: bool) -> JsResult<String> {
    let target = e.to_number(arg(args, 0))?.max(0.0) as usize;
    let fill = match arg(args, 1) {
        Value::Undefined => " ".to_string(),
        v => e.to_js_string(v)?,
    };
    if s.len() >= target || fill.is_empty() {
        return Ok(s.to_string());
    }
    let need = target - s.len();
    let mut p = fill.repeat(need / fill.len() + 1);
    p.truncate(need);
    Ok(if right {
        format!("{s}{p}")
    } else {
        format!("{p}{s}")
    })
}

fn string_pad_start(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = this_str(e, this)?;
    let out = pad(e, &s, args, false)?;
    Ok(e.str_val(out))
}

fn string_pad_end(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = this_str(e, this)?;
    let out = pad(e, &s, args, true)?;
    Ok(e.str_val(out))
}

fn string_trim_start(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let s = this_str(e, this)?;
    Ok(e.str_val(s.trim_start().to_string()))
}

fn string_trim_end(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let s = this_str(e, this)?;
    Ok(e.str_val(s.trim_end().to_string()))
}

// -- Number statics ------------------------------------------------------

fn number_is_nan(_e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    Ok(Value::Bool(
        matches!(arg(args, 0), Value::Num(n) if n.is_nan()),
    ))
}

fn number_is_finite(_e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    Ok(Value::Bool(
        matches!(arg(args, 0), Value::Num(n) if n.is_finite()),
    ))
}

fn number_is_integer(_e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    Ok(Value::Bool(
        matches!(arg(args, 0), Value::Num(n) if n.is_finite() && n.fract() == 0.0),
    ))
}

// -- Object extras -------------------------------------------------------

/// `Object.getOwnPropertyNames` — all own props (enum + non-enum).
/// Préfixe sentinelle des clés-symboles (`\u{f130}Symbol.*`).
fn object_get_own_property_names(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let keys = match arg(args, 0) {
        Value::Obj(id) => e.own_keys_trapped(id)?,
        _ => Vec::new(),
    };
    // Spec : `getOwnPropertyNames` ne rend que les clés string —
    // les clés-symboles sentinelles `\u{f130}…` et les internes
    // `\u{f131}…` (noms privés, métadonnées de classe) sont exclues.
    let vals = keys
        .iter()
        .filter(|k| !is_sentinel_key(k))
        .map(|k| e.str_val(k.clone()))
        .collect();
    Ok(e.new_array(vals))
}

/// `true` pour les clés sentinelles internes `\u{f120}…\u{f13f}`
/// (symboles, noms privés, métadonnées de classe) — invisibles
/// aux API d'énumération de noms.
fn is_sentinel_key(k: &str) -> bool {
    k.chars()
        .next()
        .is_some_and(|c| ('\u{f120}'..='\u{f13f}').contains(&c))
}

/// `Object.getOwnPropertySymbols` — les `ObjKind::Symbol`
/// correspondant aux clés sentinelles `\u{f130}…` (reverse-lookup
/// `sym_index` ; une entrée périmée par GC est ignorée — le slot
/// ne peut pas être réutilisé par un symbole à même clé).
fn object_get_own_property_symbols(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let keys = match arg(args, 0) {
        Value::Obj(id) => e.own_keys_trapped(id)?,
        _ => Vec::new(),
    };
    let mut vals = Vec::new();
    for k in &keys {
        // Toute clé sentinelle `\u{f130}…` — `Symbol.<name>` des
        // well-knowns ET `sym{N}`/`sym.for.<k>` des symboles
        // utilisateurs.
        if !k.starts_with('\u{f130}') {
            continue;
        }
        let Some(sid) = e.sym_index.get(k).copied() else {
            continue;
        };
        let live = matches!(
            e.heap.obj(sid).map(|o| &o.kind),
            Some(ObjKind::Symbol { key, .. }) if key == k
        );
        if live {
            vals.push(Value::Obj(sid));
        }
    }
    Ok(e.new_array(vals))
}

/// `Object.preventExtensions` — `[[Extensible]] = false`. Sur un
/// proxy : trap `preventExtensions` ou délégation à la cible.
fn object_prevent_extensions(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    if let Value::Obj(id) = arg(args, 0) {
        if let Some((tid, target, handler)) = e.proxy_th(id)? {
            let trap = e.proxy_trap(handler, "preventExtensions")?;
            if matches!(trap, Value::Obj(_)) {
                let ok = e.call_value(trap, handler, &[target])?;
                if !e.to_boolean(ok) {
                    return Err(e.type_error("proxy preventExtensions trap returned false"));
                }
                return Ok(arg(args, 0));
            }
            return object_prevent_extensions(e, Value::Undefined, &[Value::Obj(tid)]);
        }
        if let Some(o) = e.heap.obj_mut(id) {
            o.extensible = false;
        }
    }
    Ok(arg(args, 0))
}

fn object_is_extensible(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    if let Value::Obj(id) = arg(args, 0) {
        if let Some((tid, target, handler)) = e.proxy_th(id)? {
            let trap = e.proxy_trap(handler, "isExtensible")?;
            if matches!(trap, Value::Obj(_)) {
                let r = e.call_value(trap, handler, &[target])?;
                return Ok(Value::Bool(e.to_boolean(r)));
            }
            return object_is_extensible(e, Value::Undefined, &[Value::Obj(tid)]);
        }
        return Ok(Value::Bool(e.heap.obj(id).is_some_and(|o| o.extensible)));
    }
    Ok(Value::Bool(false))
}

/// `Object.seal` — non-extensible + toutes props non-configurables.
/// Sur un proxy : la trap `preventExtensions` si présente, sinon la
/// cible (approximation — la spec définit chaque prop via la trap
/// `defineProperty`, surface jugée trop lourde ici).
fn object_seal(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    if let Value::Obj(id) = arg(args, 0) {
        if matches!(e.heap.obj(id).map(|o| &o.kind), Some(ObjKind::Proxy { .. })) {
            return object_prevent_extensions(e, Value::Undefined, &[arg(args, 0)]);
        }
        if let Some(o) = e.heap.obj_mut(id) {
            o.extensible = false;
            for (_, p) in &mut o.props {
                match p {
                    crate::heap::Prop::Data { attrs, .. }
                    | crate::heap::Prop::Acc { attrs, .. } => {
                        attrs.configurable = false;
                    }
                }
            }
        }
    }
    Ok(arg(args, 0))
}

fn object_is_sealed(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    Ok(Value::Bool(match arg(args, 0) {
        Value::Obj(id) => e
            .heap
            .obj(id)
            .is_some_and(|o| !o.extensible && o.props.iter().all(|(_, p)| !p.configurable())),
        _ => true,
    }))
}

/// `Object.freeze` — seal + toutes data props non-writable.
/// Sur un proxy : même approximation que `seal` (trap
/// `preventExtensions` ou cible).
fn object_freeze(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    if let Value::Obj(id) = arg(args, 0) {
        if matches!(e.heap.obj(id).map(|o| &o.kind), Some(ObjKind::Proxy { .. })) {
            return object_prevent_extensions(e, Value::Undefined, &[arg(args, 0)]);
        }
        if let Some(o) = e.heap.obj_mut(id) {
            o.extensible = false;
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

fn object_is_frozen(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    Ok(Value::Bool(match arg(args, 0) {
        Value::Obj(id) => e.heap.obj(id).is_some_and(|o| {
            !o.extensible
                && o.props
                    .iter()
                    .all(|(_, p)| !p.configurable() && !p.writable())
        }),
        _ => true, // primitives are frozen by nature
    }))
}

/// `Object.is` — `SameValue` (`===` plus NaN and ±0 distinction).
fn object_is(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let (a, b) = (arg(args, 0), arg(args, 1));
    if let (Value::Num(x), Value::Num(y)) = (a, b) {
        if x.is_nan() && y.is_nan() {
            return Ok(Value::Bool(true));
        }
        if x == 0.0 && y == 0.0 {
            return Ok(Value::Bool(x.is_sign_negative() == y.is_sign_negative()));
        }
    }
    Ok(Value::Bool(e.strict_eq(a, b)))
}

// ------------------------------------------------------------------
// JS-7g — coverage extras: Object.entries/fromEntries/hasOwn, Array
// at/flat/fill/findLast/iterators/ES2023 copy variants, String
// at/codePointAt/matchAll/fromCodePoint, WeakMap/WeakSet,
// TextEncoder/TextDecoder, full-ish UTC Date, Math extras,
// Number.toPrecision/toExponential.
// ------------------------------------------------------------------

/// Own-prop test shared by `Object.hasOwn`/`hasOwnProperty` — data
/// prop, accessor or array index.
fn has_own(e: &Engine, id: ObjId, key: &str) -> bool {
    match e.heap.obj(id) {
        Some(o) => match &o.kind {
            ObjKind::Array(elems) => {
                key == "length"
                    || crate::interp::array_index(key).is_some_and(|i| i < elems.len())
                    || o.find(key).is_some()
            }
            _ => o.find(key).is_some(),
        },
        None => false,
    }
}

/// `Object.entries(o)` → `[[k, v], …]` (own enumerable).
fn object_entries(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = arg(args, 0) else {
        return Ok(e.new_array(Vec::new()));
    };
    let keys = e.own_enum_keys_trapped(id)?;
    let mut out = Vec::with_capacity(keys.len());
    for k in keys {
        // Lecture `[[Get]]` (getters et traps proxy déclenchés).
        let v = e.get_prop_value(Value::Obj(id), &k)?;
        let ks = e.str_val(k);
        let kv = e.new_array(vec![ks, v]);
        out.push(kv);
    }
    Ok(e.new_array(out))
}

/// `Object.fromEntries(iterable)` — `[k, v]` pairs → object.
fn object_from_entries(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let obj = e.new_object();
    let pairs = match arg(args, 0) {
        Value::Obj(id) => e.iter_values(Value::Obj(id))?,
        _ => Vec::new(),
    };
    for p in pairs {
        let pair = match p {
            Value::Obj(id) => e.iter_values(Value::Obj(id))?,
            _ => Vec::new(),
        };
        if pair.len() >= 2 {
            let k = e.prop_key(pair[0])?;
            e.set_prop_value(Value::Obj(obj), &k, pair[1])?;
        }
    }
    Ok(Value::Obj(obj))
}

/// `Object.hasOwn(o, k)` — own prop, no proto walk.
fn object_has_own(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let key = e.prop_key(arg(args, 1))?;
    Ok(Value::Bool(match arg(args, 0) {
        Value::Obj(id)
            if matches!(e.heap.obj(id).map(|o| &o.kind), Some(ObjKind::Proxy { .. })) =>
        {
            e.get_own_prop(id, &key)?.is_some()
        }
        Value::Obj(id) => has_own(e, id, &key),
        _ => false,
    }))
}

// -- Array: index helpers --------------------------------------------

/// `at`/index args → element index (negative wraps from the end).
fn rel_index(e: &mut Engine, v: Value, len: usize) -> JsResult<Option<usize>> {
    let i = e.to_number(v)? as isize;
    let j = if i < 0 { len as isize + i } else { i };
    Ok((j >= 0 && (j as usize) < len).then_some(j as usize))
}

fn array_at(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this)?;
    let elems = take_elems(e, id);
    let i = rel_index(e, arg(args, 0), elems.len())?;
    let v = i.map_or(Value::Undefined, |i| elems[i]);
    put_elems(e, id, elems);
    Ok(v)
}

fn array_fill(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this)?;
    let mut elems = take_elems(e, id);
    let len = elems.len();
    let v = arg(args, 0);
    let start = rel_index(e, arg(args, 1), len)?.unwrap_or(0);
    let end = match arg(args, 2) {
        Value::Undefined => len,
        v => rel_index(e, v, len)?.unwrap_or(len),
    };
    for slot in &mut elems[start..end.min(len)] {
        *slot = v;
    }
    put_elems(e, id, elems);
    Ok(this)
}

/// Recursive `flat` — spec depth param, default 1.
fn flat_into(e: &mut Engine, v: Value, depth: u32, out: &mut Vec<Value>) -> JsResult<()> {
    if depth > 0 {
        if let Value::Obj(id) = v {
            if matches!(e.heap.obj(id).map(|o| &o.kind), Some(ObjKind::Array(_))) {
                for el in take_elems(e, id) {
                    flat_into(e, el, depth - 1, out)?;
                }
                return Ok(());
            }
        }
    }
    out.push(v);
    Ok(())
}

fn array_flat(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this)?;
    let depth = match arg(args, 0) {
        Value::Undefined => 1,
        v => e.to_number(v)? as u32,
    };
    let elems = take_elems(e, id);
    let mut out = Vec::new();
    for v in elems.iter().copied() {
        flat_into(e, v, depth, &mut out)?;
    }
    put_elems(e, id, elems);
    Ok(e.new_array(out))
}

fn array_flat_map(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this)?;
    let cb = arg(args, 0);
    if !e.is_callable(cb) {
        return Err(e.type_error("flatMap needs a function"));
    }
    let elems = take_elems(e, id);
    let mut out = Vec::new();
    for (i, &v) in elems.iter().enumerate() {
        let r = e.call_value(cb, arg(args, 1), &[v, Value::Num(i as f64), this])?;
        flat_into(e, r, 1, &mut out)?;
    }
    put_elems(e, id, elems);
    Ok(e.new_array(out))
}

fn array_find_last(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (elems, hits, id) = array_iter_find(e, this, args)?;
    let out = elems
        .iter()
        .zip(&hits)
        .rev()
        .find_map(|(&v, &h)| h.then_some(v))
        .unwrap_or(Value::Undefined);
    put_elems(e, id, elems);
    Ok(out)
}

fn array_find_last_index(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (elems, hits, id) = array_iter_find(e, this, args)?;
    let out = hits.iter().rposition(|&h| h).map_or(-1.0, |i| i as f64);
    put_elems(e, id, elems);
    Ok(Value::Num(out))
}

// Array iterators — snapshot `Iter` objects (same family as
// Map/Set's; live-cursor semantics documented as deviation).
fn array_values(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this)?;
    let elems = take_elems(e, id);
    put_elems(e, id, elems.clone());
    Ok(crate::collections::iter_obj(e, elems))
}

fn array_keys(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this)?;
    let n = take_elems(e, id).len();
    Ok(crate::collections::iter_obj(
        e,
        (0..n).map(|i| Value::Num(i as f64)).collect(),
    ))
}

fn array_entries(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this)?;
    let elems = take_elems(e, id);
    let mut items = Vec::with_capacity(elems.len());
    for (i, &v) in elems.iter().enumerate() {
        items.push(e.new_array(vec![Value::Num(i as f64), v]));
    }
    put_elems(e, id, elems);
    Ok(crate::collections::iter_obj(e, items))
}

// ES2023 non-mutating variants.
fn array_with(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this)?;
    let elems = take_elems(e, id);
    let len = elems.len();
    if let Some(i) = rel_index(e, arg(args, 0), len)? {
        let mut out_v = elems.clone();
        out_v[i] = arg(args, 1);
        let out = e.new_array(out_v);
        put_elems(e, id, elems);
        Ok(out)
    } else {
        put_elems(e, id, elems);
        Err(e.range_error("with: index out of range"))
    }
}

fn array_to_reversed(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this)?;
    let elems = take_elems(e, id);
    let mut out_v = elems.clone();
    out_v.reverse();
    let out = e.new_array(out_v);
    put_elems(e, id, elems);
    let _ = args;
    Ok(out)
}

fn array_to_spliced(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this)?;
    let elems = take_elems(e, id);
    let len = elems.len();
    let start = match arg(args, 0) {
        Value::Undefined => 0,
        v => rel_index(e, v, len)?.unwrap_or(len),
    };
    let del = match arg(args, 1) {
        Value::Undefined => len - start,
        v => (e.to_number(v)?.max(0.0) as usize).min(len - start),
    };
    let ins: Vec<Value> = args.get(2..).unwrap_or(&[]).to_vec();
    let mut out_v = Vec::with_capacity(len - del + ins.len());
    out_v.extend_from_slice(&elems[..start]);
    out_v.extend(ins);
    out_v.extend_from_slice(&elems[start + del..]);
    let out = e.new_array(out_v);
    put_elems(e, id, elems);
    Ok(out)
}

fn array_to_sorted(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    // Copy → sort via the shared comparator path → new array
    // (`sort` mutates `this`; `toSorted` must not).
    let id = this_array(e, this)?;
    let elems = take_elems(e, id);
    let copy = e.new_array(elems.clone());
    put_elems(e, id, elems);
    array_sort(e, copy, args)?;
    Ok(copy)
}

// -- String extras ----------------------------------------------------

fn string_at(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = this_str(e, this)?;
    let chars: Vec<char> = s.chars().collect();
    match rel_index(e, arg(args, 0), chars.len())? {
        Some(i) => Ok(e.str_val(chars[i].to_string())),
        None => Ok(Value::Undefined),
    }
}

/// `codePointAt(i)` — full scalar value (UTF-16 surrogate pairs
/// collapse through Rust's `char` since our strings are UTF-8).
fn string_code_point_at(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = this_str(e, this)?;
    let i = e.to_number(arg(args, 0))?;
    let v = s.chars().nth(i.max(0.0) as usize).map(|c| c as u32);
    Ok(v.map_or(Value::Undefined, |c| Value::Num(f64::from(c))))
}

/// `matchAll(re)` — requires the `g` flag (spec `TypeError`); yields
/// an iterator of match arrays.
fn string_match_all(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = this_str(e, this)?;
    // `rx[Symbol.matchAll](this)` (spec `GetMethod`).
    if let Some(v) = sym_method(e, arg(args, 0), crate::interp::SYM_MATCH_ALL, &[this])? {
        return Ok(v);
    }
    let (re, _id) = as_regexp(e, arg(args, 0))?;
    re_match_all_impl(e, &re, &s)
}

/// `String.fromCodePoint(...)` — scalar values; lone surrogates
/// degrade to U+FFFD (our strings are UTF-8, not UTF-16).
fn string_from_code_point(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let mut s = String::new();
    for &a in args {
        let n = e.to_number(a)?;
        if !(0.0..=1_114_111.0).contains(&n) || n.fract() != 0.0 {
            return Err(e.range_error("fromCodePoint: invalid code point"));
        }
        s.push(char::from_u32(n as u32).unwrap_or('\u{FFFD}'));
    }
    Ok(e.str_val(s))
}

/// `String.raw(strings, …subs)` — join `strings.raw[i]` with the
/// substitutions (spec `GetTemplateObject` inverse — reads `.raw`
/// and `.length`, so array-likes work too).
fn string_raw(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let raw = e.get_prop_value(arg(args, 0), "raw")?;
    let n = match e.get_prop_value(raw, "length")? {
        Value::Num(n) => n.max(0.0) as usize,
        _ => 0,
    };
    let mut out = String::new();
    for i in 0..n {
        let seg = e.get_prop_value(raw, &i.to_string())?;
        out.push_str(&e.to_js_string(seg)?);
        if let Some(v) = args.get(i + 1) {
            out.push_str(&e.to_js_string(*v)?);
        }
    }
    Ok(e.str_val(out))
}

// -- TextEncoder / TextDecoder (UTF-8 only) ----------------------------

/// `new TextEncoder()` — UTF-8 only (labeled encodings are out of
/// scope for the mirror engine; `encode` yields a real `Uint8Array`).
fn text_encoder_ctor(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    let id = e.new_object();
    e.add_method(id, "encode", text_encoder_encode);
    let enc = e.str_val("utf-8");
    e.set_own(id, "encoding", enc, true);
    Ok(Value::Obj(id))
}

fn text_encoder_encode(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let s = e.to_js_string(arg(args, 0))?;
    Ok(crate::typed_array::new_u8_array(e, s.into_bytes()))
}

/// `new TextDecoder(label?)` — UTF-8 (labels parsed but ignored;
/// non-UTF-8 requested → `RangeError` like real engines for
/// unsupported labels).
fn text_decoder_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let label = match arg(args, 0) {
        Value::Undefined => "utf-8".to_string(),
        v => e.to_js_string(v)?.to_lowercase(),
    };
    if !matches!(label.as_str(), "utf-8" | "utf8" | "unicode-1-1-utf-8") {
        return Err(e.range_error("unsupported encoding"));
    }
    let id = e.new_object();
    e.add_method(id, "decode", text_decoder_decode);
    let enc = e.str_val("utf-8");
    e.set_own(id, "encoding", enc, true);
    Ok(Value::Obj(id))
}

fn text_decoder_decode(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let bytes: Vec<u8> = match arg(args, 0) {
        // Vue typée / `DataView` / `ArrayBuffer` — ses octets bruts.
        Value::Obj(id) => match crate::typed_array::bytes_of_value(e, Value::Obj(id)) {
            Some(b) => b,
            // iterable/array-like de nombres (`decode([72,105])`).
            None => e
                .iter_values(Value::Obj(id))?
                .iter()
                .filter_map(|v| match v {
                    Value::Num(n) => Some(*n as u8),
                    _ => None,
                })
                .collect(),
        },
        _ => Vec::new(),
    };
    Ok(e.str_val(String::from_utf8_lossy(&bytes).into_owned()))
}

// -- Math extras --------------------------------------------------------

macro_rules! math1 {
    ($name:ident, $op:expr) => {
        fn $name(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
            let x = e.to_number(arg(args, 0))?;
            let f: fn(f64) -> f64 = $op;
            Ok(Value::Num(f(x)))
        }
    };
}

math1!(math_trunc, f64::trunc);
math1!(math_sign, f64::signum);
math1!(math_cbrt, f64::cbrt);
math1!(math_log2, f64::log2);
math1!(math_log10, f64::log10);
math1!(math_log1p, f64::ln_1p);
math1!(math_expm1, f64::exp_m1);
math1!(math_fround, |x| x as f32 as f64);
math1!(math_asin, f64::asin);
math1!(math_acos, f64::acos);
math1!(math_sinh, f64::sinh);
math1!(math_cosh, f64::cosh);
math1!(math_tanh, f64::tanh);

fn math_hypot(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let mut acc = 0.0_f64;
    for &a in args {
        let x = e.to_number(a)?;
        acc = acc.hypot(x);
    }
    Ok(Value::Num(acc))
}

fn math_imul(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let a = to_int32(e.to_number(arg(args, 0))?);
    let b = to_int32(e.to_number(arg(args, 1))?);
    Ok(Value::Num(f64::from(a.wrapping_mul(b))))
}

fn math_clz32(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let x = to_uint32(e.to_number(arg(args, 0))?);
    Ok(Value::Num(f64::from(x.leading_zeros())))
}

// -- Number extras --------------------------------------------------------

/// `{:.*e}` gives `d.ddd e±exp` — JS wants the sign always and
/// two-digit minimum exponent is optional; normalize the exponent.
fn js_exp(mut s: String) -> String {
    if let Some(i) = s.rfind('e') {
        let exp = &s[i + 1..];
        if !exp.starts_with(['-', '+']) {
            s.insert(i + 1, '+');
        }
    }
    s
}

fn number_to_precision(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let n = this_num(e, this)?;
    let p = match arg(args, 0) {
        Value::Undefined => return Ok(e.str_val(num_to_js_string(n))),
        v => e.to_number(v)? as usize,
    };
    if !(1..=100).contains(&p) {
        return Err(e.range_error("toPrecision: digits out of range"));
    }
    if !n.is_finite() || n == 0.0 {
        return Ok(e.str_val(num_to_js_string(n)));
    }
    let exp = n.abs().log10().floor() as i32;
    if exp < -6 || exp >= p as i32 {
        Ok(e.str_val(js_exp(format!("{n:.prec$e}", prec = p - 1))))
    } else {
        let dec = (p as i32 - 1 - exp).max(0) as usize;
        let mut s = format!("{n:.dec$}");
        while s.contains('.') && s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
        Ok(e.str_val(s))
    }
}

fn number_to_exponential(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let n = this_num(e, this)?;
    let d = match arg(args, 0) {
        Value::Undefined => {
            // As many digits as needed to represent uniquely — Rust's
            // `{e}` gives the shortest, close enough for our subset.
            return Ok(e.str_val(js_exp(format!("{n:e}"))));
        }
        v => e.to_number(v)? as usize,
    };
    if d > 100 {
        return Err(e.range_error("toExponential: digits out of range"));
    }
    if !n.is_finite() {
        return Ok(e.str_val(num_to_js_string(n)));
    }
    Ok(e.str_val(js_exp(format!("{n:.d$e}"))))
}

// ------------------------------------------------------------------
// JS-7g — full-ish UTC `Date`. Time zone is always UTC (the engine
// has no tz db; `getTimezoneOffset` = 0, `getUTC*` == `get*`).
// Civil ↔ days algorithms are Hinnant's proleptic-Gregorian ones.
// ------------------------------------------------------------------

/// Days since epoch for `y-m-d` (month is 1-based here).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9).rem_euclid(12);
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `(y, m, d)` from days since epoch.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `ms` since epoch → `(y, mo, d, weekday, h, mi, s, ms)`.
fn ms_to_parts(ms: f64) -> (i64, u32, u32, u32, u32, u32, u32, u32) {
    if !ms.is_finite() {
        return (0, 0, 0, 0, 0, 0, 0, 0);
    }
    let days = ms.div_euclid(86_400_000.0) as i64;
    let rem = ms.rem_euclid(86_400_000.0) as i64;
    let (y, mo, d) = civil_from_days(days);
    let wd = (days.rem_euclid(7) + 4).rem_euclid(7) as u32;
    let h = (rem / 3_600_000) as u32;
    let mi = (rem % 3_600_000 / 60_000) as u32;
    let s = (rem % 60_000 / 1000) as u32;
    let mss = (rem % 1000) as u32;
    (y, mo, d, wd, h, mi, s, mss)
}

/// `y, mo(0-based), d, h, mi, s, ms` → epoch ms (UTC).
fn parts_to_ms(y: i64, mo: i64, d: i64, h: i64, mi: i64, s: i64, mss: i64) -> f64 {
    // `y` < 100 maps to 1900+y (spec quirk kept for parity).
    let y = if (0..=99).contains(&y) { 1900 + y } else { y };
    (days_from_civil(y, mo + 1, d) * 86_400_000 + h * 3_600_000 + mi * 60_000 + s * 1000 + mss)
        as f64
}

/// Lenient ISO-8601 parse: `YYYY-MM-DD[THH:mm[:ss[.sss]]][Z|±hh:mm]`.
/// Returns epoch ms (UTC applied on `Z`/offset, treated as UTC
/// without one — we have no tz). Non-ISO → `NaN`.
fn parse_iso_date(s: &str) -> Option<f64> {
    let s = s.trim();
    let (date, time) = s.split_once(['T', ' ']).map_or((s, ""), |(d, t)| (d, t));
    let mut dp = date.split('-');
    let y: i64 = dp.next()?.parse().ok()?;
    let mo: i64 = dp.next().map_or(1, |m| m.parse().unwrap_or(1));
    let d: i64 = dp.next().map_or(1, |d| d.parse().unwrap_or(1));
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    let (mut h, mut mi, mut sec, mut mss) = (0i64, 0i64, 0i64, 0i64);
    let mut offset_min = 0i64;
    if !time.is_empty() {
        let (t, off) = if let Some(t) = time.strip_suffix('Z') {
            (t, 0i64)
        } else if let Some(i) = time.rfind(['+', '-']).filter(|&i| i > 0) {
            let (h_off, m_off) = time[i + 1..]
                .split_once(':')
                .map_or((time[i + 1..].parse().unwrap_or(0), 0), |(h, m)| {
                    (h.parse().unwrap_or(0), m.parse().unwrap_or(0))
                });
            let sign = if time.as_bytes()[i] == b'-' { -1 } else { 1 };
            (&time[..i], sign * (h_off * 60 + m_off))
        } else {
            (time, 0i64)
        };
        let mut tp = t.split(':');
        h = tp.next().map_or(0, |v| v.parse().unwrap_or(0));
        mi = tp.next().map_or(0, |v| v.parse().unwrap_or(0));
        if let Some(ss) = tp.next() {
            let (s_i, ms_f) = ss.split_once('.').map_or((ss, "0"), |(a, b)| (a, b));
            sec = s_i.parse().unwrap_or(0);
            mss = format!("{ms_f:0<3}")[..3].parse().unwrap_or(0);
        }
        offset_min = off;
    }
    Some(parts_to_ms(y, mo - 1, d, h, mi, sec, mss) - (offset_min * 60_000) as f64)
}

/// `Date.parse(str)` → epoch ms or `NaN`.
fn date_parse(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let s = e.to_js_string(arg(args, 0))?;
    Ok(Value::Num(parse_iso_date(&s).unwrap_or(f64::NAN)))
}

/// `Date.UTC(y, mo, …)` → epoch ms.
fn date_utc(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let mut n = |i: usize| -> JsResult<i64> {
        Ok(match args.get(i) {
            Some(v) => e.to_number(*v)? as i64,
            None => 0,
        })
    };
    if args.is_empty() {
        return Ok(Value::Num(f64::NAN));
    }
    let y = n(0)?;
    let mo = n(1)?;
    let d = n(2)?.max(1);
    let (h, mi, s, ms) = (n(3)?, n(4)?, n(5)?, n(6)?);
    Ok(Value::Num(parts_to_ms(y, mo, d, h, mi, s, ms)))
}

/// `new Date()` — now | ms | ISO string | `(y, mo, d?, …)` UTC.
fn date_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let ms = match args.len() {
        0 => now_millis(),
        1 => match arg(args, 0) {
            Value::Str(id) => {
                let s = e.heap.string(id).unwrap_or_default();
                parse_iso_date(&s).unwrap_or(f64::NAN)
            }
            v => e.to_number(v)?,
        },
        _ => {
            let mut n = |i: usize, dflt: i64| -> JsResult<i64> {
                Ok(match args.get(i) {
                    Some(v) => e.to_number(*v)? as i64,
                    None => dflt,
                })
            };
            parts_to_ms(
                n(0, 0)?,
                n(1, 0)?,
                n(2, 1)?,
                n(3, 0)?,
                n(4, 0)?,
                n(5, 0)?,
                n(6, 0)?,
            )
        }
    };
    let id = e.new_object();
    if let Some(o) = e.heap.obj_mut(id) {
        o.proto = Value::Obj(e.protos.date);
        o.set_own("__time", Value::Num(ms), false);
    }
    Ok(Value::Obj(id))
}

fn date_ms(e: &mut Engine, this: Value) -> JsResult<f64> {
    match this {
        Value::Obj(id) => match e.get_prop(id, "__time") {
            Some(Value::Num(n)) => Ok(n),
            _ => Err(e.type_error("Date method on non-Date")),
        },
        _ => Err(e.type_error("Date method on non-Date")),
    }
}

/// `d.toISOString()` → `YYYY-MM-DDTHH:mm:ss.sssZ` (NaN → `RangeError`).
fn date_to_iso(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let ms = date_ms(e, this)?;
    if !ms.is_finite() {
        return Err(e.range_error("Invalid time value"));
    }
    let (y, mo, d, _wd, h, mi, s, mss) = ms_to_parts(ms);
    Ok(e.str_val(format!(
        "{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{mss:03}Z"
    )))
}

/// `d.toUTCString()` ≙ RFC-1123-ish `"Thu, 01 Jan 1970 00:00:00 GMT"`.
const WD_NAMES: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MO_NAMES: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

fn date_to_utc_string(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let ms = date_ms(e, this)?;
    if !ms.is_finite() {
        return Ok(e.str_val("Invalid Date"));
    }
    let (y, mo, d, wd, h, mi, s, _m) = ms_to_parts(ms);
    Ok(e.str_val(format!(
        "{}, {:02} {} {:04} {:02}:{:02}:{:02} GMT",
        WD_NAMES[wd as usize],
        d,
        MO_NAMES[(mo - 1) as usize],
        y,
        h,
        mi,
        s
    )))
}

/// Shared `getX` — field index into `ms_to_parts` (0=y,1=mo-1,2=d,
/// 3=wd,4=h,5=mi,6=s,7=ms — month is 0-based JS-style).
fn date_part(e: &mut Engine, this: Value, i: usize) -> JsResult<f64> {
    let ms = date_ms(e, this)?;
    if !ms.is_finite() {
        return Ok(f64::NAN);
    }
    let p = ms_to_parts(ms);
    Ok(match i {
        0 => p.0 as f64,
        1 => f64::from(p.1) - 1.0,
        2 => f64::from(p.2),
        3 => f64::from(p.3),
        4 => f64::from(p.4),
        5 => f64::from(p.5),
        6 => f64::from(p.6),
        _ => f64::from(p.7),
    })
}

macro_rules! date_get {
    ($name:ident, $i:expr) => {
        fn $name(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
            Ok(Value::Num(date_part(e, this, $i)?))
        }
    };
}

date_get!(date_get_full_year, 0);
date_get!(date_get_month, 1);
date_get!(date_get_date, 2);
date_get!(date_get_day, 3);
date_get!(date_get_hours, 4);
date_get!(date_get_minutes, 5);
date_get!(date_get_seconds, 6);
date_get!(date_get_milliseconds, 7);

fn date_get_timezone_offset(_e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    Ok(Value::Num(0.0)) // always UTC — no tz db in the engine
}

// ------------------------------------------------------------------
// JS-C8 — structuredClone / groupBy / fromAsync
// ------------------------------------------------------------------

/// `structuredClone(v)` — copie profonde spec `StructuredSerialize` :
/// cycles préservés (table id→clone), `Fun`/`Promise`/`Proxy`/`Host`/
/// `Symbol`/`Iter` non-clonables → `TypeError` (`DataCloneError`).
fn structured_clone(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let mut seen: Vec<(ObjId, ObjId)> = Vec::new();
    clone_deep(e, arg(args, 0), &mut seen)
}

/// Forme extraite du `ObjKind` (champs Copy — le borrow du heap
/// s'arrête avant toute allocation).
enum CloneShape {
    Arr,
    Map,
    Set,
    Buf(Vec<u8>),
    Dv(ObjId, usize, usize),
    Ta(crate::heap::TaKind, ObjId, usize, usize),
    Re(String, String),
    Ordinary,
    Err(&'static str),
}

fn clone_deep(e: &mut Engine, v: Value, seen: &mut Vec<(ObjId, ObjId)>) -> JsResult<Value> {
    let Value::Obj(id) = v else { return Ok(v) };
    if let Some((_, c)) = seen.iter().find(|(s, _)| *s == id) {
        return Ok(Value::Obj(*c));
    }
    let shape = match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::Array(_)) => CloneShape::Arr,
        Some(ObjKind::Map(_)) => CloneShape::Map,
        Some(ObjKind::Set(_)) => CloneShape::Set,
        Some(ObjKind::ByteBuf(b)) => CloneShape::Buf(b.clone()),
        Some(ObjKind::DataView {
            buffer: Value::Obj(bid),
            offset,
            len,
        }) => CloneShape::Dv(*bid, *offset, *len),
        Some(ObjKind::TypedArray {
            ta,
            buffer: Value::Obj(bid),
            offset,
            len,
        }) => CloneShape::Ta(*ta, *bid, *offset, *len),
        Some(ObjKind::RegExp(re)) => CloneShape::Re(re.source.clone(), re.flag_string()),
        Some(ObjKind::Fun(_)) => CloneShape::Err("function"),
        Some(ObjKind::Promise(_)) => CloneShape::Err("Promise"),
        Some(ObjKind::Host(_)) => CloneShape::Err("host object"),
        Some(ObjKind::Symbol { .. }) => CloneShape::Err("Symbol"),
        Some(ObjKind::Proxy { .. }) => CloneShape::Err("Proxy"),
        Some(ObjKind::Iter { .. }) => CloneShape::Err("iterator"),
        Some(ObjKind::FetchResponse { .. }) => CloneShape::Err("Response"),
        _ => CloneShape::Ordinary,
    };
    match shape {
        CloneShape::Err(w) => Err(e.type_error(format!("structuredClone: cannot clone {w}"))),
        CloneShape::Buf(b) => Ok(Value::Obj(e.heap.alloc_obj(Object {
            proto: Value::Obj(e.protos.array_buffer),
            props: Vec::new(),
            kind: ObjKind::ByteBuf(b),
            extensible: true,
        }))),
        CloneShape::Dv(bid, offset, len) => {
            let nb = clone_deep(e, Value::Obj(bid), seen)?;
            Ok(Value::Obj(e.heap.alloc_obj(Object {
                proto: Value::Obj(e.protos.data_view),
                props: Vec::new(),
                kind: ObjKind::DataView {
                    buffer: nb,
                    offset,
                    len,
                },
                extensible: true,
            })))
        }
        CloneShape::Ta(ta, bid, offset, len) => {
            let nb = clone_deep(e, Value::Obj(bid), seen)?;
            let Value::Obj(nbid) = nb else {
                return Err(e.type_error("structuredClone: bad buffer"));
            };
            Ok(crate::typed_array::new_ta(e, ta, nbid, offset, len))
        }
        CloneShape::Re(src, flags) => e.make_regexp(&src, &flags),
        CloneShape::Arr => {
            let arr = e.new_array(Vec::new());
            let Value::Obj(cid) = arr else {
                return Err(e.type_error("structuredClone: bad array"));
            };
            seen.push((id, cid));
            let elems = match e.heap.obj(id).map(|o| &o.kind) {
                Some(ObjKind::Array(elems)) => elems.clone(),
                _ => Vec::new(),
            };
            let mut out = Vec::with_capacity(elems.len());
            for v in elems {
                out.push(clone_deep(e, v, seen)?);
            }
            if let Some(o) = e.heap.obj_mut(cid) {
                if let ObjKind::Array(el) = &mut o.kind {
                    *el = out;
                }
            }
            Ok(arr)
        }
        CloneShape::Map => {
            let cid = e.heap.alloc_obj(Object {
                proto: Value::Obj(e.protos.map),
                props: Vec::new(),
                kind: ObjKind::Map(Vec::new()),
                extensible: true,
            });
            seen.push((id, cid));
            let pairs = match e.heap.obj(id).map(|o| &o.kind) {
                Some(ObjKind::Map(m)) => m.clone(),
                _ => Vec::new(),
            };
            for (k, v) in pairs {
                let kc = clone_deep(e, k, seen)?;
                let vc = clone_deep(e, v, seen)?;
                crate::collections::map_set_entry(e, cid, kc, vc);
            }
            Ok(Value::Obj(cid))
        }
        CloneShape::Set => {
            let cid = e.heap.alloc_obj(Object {
                proto: Value::Obj(e.protos.set),
                props: Vec::new(),
                kind: ObjKind::Set(Vec::new()),
                extensible: true,
            });
            seen.push((id, cid));
            let items = match e.heap.obj(id).map(|o| &o.kind) {
                Some(ObjKind::Set(s)) => s.clone(),
                _ => Vec::new(),
            };
            for v in items {
                let vc = clone_deep(e, v, seen)?;
                crate::collections::set_add(e, cid, vc);
            }
            Ok(Value::Obj(cid))
        }
        CloneShape::Ordinary => {
            let (proto, props) = match e.heap.obj(id) {
                Some(o) => (o.proto, o.props.clone()),
                _ => (Value::Obj(e.protos.object), Vec::new()),
            };
            let cid = e.heap.alloc_obj(Object::ordinary(proto));
            seen.push((id, cid));
            for (key, prop) in props {
                // Spec : props énumérables propres, clonées en data.
                let crate::heap::Prop::Data { value, attrs } = prop else {
                    continue;
                };
                if !attrs.enumerable {
                    continue;
                }
                let cv = clone_deep(e, value, seen)?;
                e.set_own(cid, &key, cv, true);
            }
            Ok(Value::Obj(cid))
        }
    }
}

/// `Object.groupBy(items, cb)` — objet à proto nul `key → [items]`
/// (ES2024 ; les clés passent par `ToString` comme `groupBy` spec).
fn object_group_by(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let items = e.iter_values(arg(args, 0))?;
    let f = arg(args, 1);
    if !e.is_callable(f) {
        return Err(e.type_error("groupBy callback is not callable"));
    }
    let out = e.heap.alloc_obj(Object::ordinary(Value::Null));
    for (i, v) in items.into_iter().enumerate() {
        let k = e.call_value(f, Value::Undefined, &[v, Value::Num(i as f64)])?;
        let key = e.to_js_string(k)?;
        let arr = if let Some(v @ Value::Obj(_)) = e.get_prop(out, &key) {
            v
        } else {
            let a = e.new_array(Vec::new());
            e.set_own(out, &key, a, true);
            a
        };
        if let Value::Obj(aid) = arr {
            if let Some(o) = e.heap.obj_mut(aid) {
                if let ObjKind::Array(elems) = &mut o.kind {
                    elems.push(v);
                }
            }
        }
    }
    Ok(Value::Obj(out))
}

/// `Array.fromAsync(src, mapfn?, thisArg?)` — `await` de chaque
/// élément puis du résultat de mapfn, promesse de tableau
/// (même déviation settled-only qu'`await` : pending → `TypeError`).
fn array_from_async(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let src = arg(args, 0);
    let f = arg(args, 1);
    let has_map = !matches!(f, Value::Undefined);
    if has_map && !e.is_callable(f) {
        return Err(e.type_error("fromAsync mapfn is not callable"));
    }
    let t = arg(args, 2);
    // Comme toute fonction async, `fromAsync` ne jette jamais
    // synchrone : les erreurs (itération, mapfn, rejet d'élément)
    // rejettent la promesse retournée.
    let mut collect = || -> JsResult<Vec<Value>> {
        let vals = crate::typed_array::src_vals(e, src)?;
        let mut out = Vec::with_capacity(vals.len());
        for (i, v) in vals.iter().enumerate() {
            let mut x = await_settled(e, *v)?;
            if has_map {
                x = e.call_value(f, t, &[x, Value::Num(i as f64)])?;
                x = await_settled(e, x)?;
            }
            out.push(x);
        }
        Ok(out)
    };
    let result = collect();
    let pid = e.new_promise();
    match result {
        Ok(out) => {
            let arr = e.new_array(out);
            e.resolve_promise(pid, arr);
        }
        Err(Thrown(v)) => e.reject_promise(pid, v),
    }
    Ok(Value::Obj(pid))
}

/// `await` limité aux promesses settled — déviation documentée
/// partagée avec `Expr::Await` (pending → `TypeError`).
fn await_settled(e: &mut Engine, v: Value) -> JsResult<Value> {
    e.await_value(v)
}

// -- WeakRef / FinalizationRegistry (JS-C13) --------------------------

/// `new WeakRef(target)` — la cible doit être un objet (spec).
/// `ObjKind::WeakRef` n'est pas tracé → `deref()` observe la
/// collecte réelle.
fn weakref_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(tid) = arg(args, 0) else {
        return Err(e.type_error("WeakRef target must be an object"));
    };
    let proto = match e.get_prop(e.global, "WeakRef") {
        Some(c) => e
            .get_prop_value(c, "prototype")
            .unwrap_or(Value::Obj(e.protos.object)),
        None => Value::Obj(e.protos.object),
    };
    Ok(Value::Obj(e.heap.alloc_obj(Object {
        proto,
        props: Vec::new(),
        kind: ObjKind::WeakRef(tid),
        extensible: true,
    })))
}

/// `wr.deref()` — `undefined` si la cible a été collectée.
fn weakref_deref(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = this else {
        return Err(e.type_error("WeakRef.prototype.deref on non-WeakRef"));
    };
    match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::WeakRef(tid)) => Ok(if e.heap.obj(*tid).is_some() {
            Value::Obj(*tid)
        } else {
            Value::Undefined
        }),
        _ => Err(e.type_error("WeakRef.prototype.deref on non-WeakRef")),
    }
}

/// `new FinalizationRegistry(cleanup)` — `cleanup` callable
/// requis (spec `TypeError` sinon).
fn finreg_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let cb = arg(args, 0);
    if !e.is_callable(cb) {
        return Err(e.type_error("FinalizationRegistry callback must be callable"));
    }
    let proto = match e.get_prop(e.global, "FinalizationRegistry") {
        Some(c) => e
            .get_prop_value(c, "prototype")
            .unwrap_or(Value::Obj(e.protos.object)),
        None => Value::Obj(e.protos.object),
    };
    let id = e.heap.alloc_obj(Object {
        proto,
        props: Vec::new(),
        kind: ObjKind::FinReg {
            cb,
            entries: Vec::new(),
        },
        extensible: true,
    });
    e.finregs.push(id);
    Ok(Value::Obj(id))
}

/// `fr.register(target, held, unregisterToken?)` — `target` objet
/// requis, `token !== held` (spec).
fn finreg_register(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = this else {
        return Err(e.type_error("FinalizationRegistry.register on non-registry"));
    };
    let Value::Obj(tid) = arg(args, 0) else {
        return Err(e.type_error("FinalizationRegistry target must be an object"));
    };
    let held = arg(args, 1);
    let token = arg(args, 2);
    if e.strict_eq(token, held) {
        return Err(e.type_error("unregister token must not equal held value"));
    }
    match e.heap.obj_mut(id).map(|o| &mut o.kind) {
        Some(ObjKind::FinReg { entries, .. }) => {
            entries.push(crate::heap::FinRegEntry {
                target: tid,
                held,
                token,
            });
            Ok(Value::Undefined)
        }
        _ => Err(e.type_error("FinalizationRegistry.register on non-registry")),
    }
}

/// `fr.unregister(token)` — retire les entrées à ce jeton, rend
/// `true` si au moins une a été trouvée (spec).
fn finreg_unregister(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = this else {
        return Err(e.type_error("FinalizationRegistry.unregister on non-registry"));
    };
    let token = arg(args, 0);
    let Some(entries) = (match e.heap.obj_mut(id).map(|o| &mut o.kind) {
        Some(ObjKind::FinReg { entries, .. }) => Some(std::mem::take(entries)),
        _ => None,
    }) else {
        return Err(e.type_error("FinalizationRegistry.unregister on non-registry"));
    };
    let before = entries.len();
    let mut keep = Vec::with_capacity(before);
    for en in entries {
        if !e.strict_eq(en.token, token) {
            keep.push(en);
        }
    }
    let removed = before - keep.len();
    if let Some(o) = e.heap.obj_mut(id) {
        if let ObjKind::FinReg { entries: es, .. } = &mut o.kind {
            *es = keep;
        }
    }
    Ok(Value::Bool(removed > 0))
}

/// Job microtask d'un cleanup `FinalizationRegistry` — bound =
/// `[cb, held]`, exécute `cb(held)` (`this` = undefined).
pub(crate) fn finreg_cleanup(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let cb = arg(args, 0);
    let held = arg(args, 1);
    e.call_value(cb, Value::Undefined, &[held])
}

/// `String.prototype.normalize(form?)` — NFC par défaut, NFD/NFKC/
/// NFKD explicites (unicode-normalization, tables Unicode réelles).
fn string_normalize(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    use unicode_normalization::UnicodeNormalization;
    let s = e.to_js_string(this)?;
    let form = match arg(args, 0) {
        Value::Undefined => "NFC".to_string(),
        v => e.to_js_string(v)?,
    };
    let out: String = match form.as_str() {
        "NFC" => s.nfc().collect(),
        "NFD" => s.nfd().collect(),
        "NFKC" => s.nfkc().collect(),
        "NFKD" => s.nfkd().collect(),
        _ => return Err(e.range_error("invalid normalization form")),
    };
    Ok(e.str_val(out))
}

// ------------------------------------------------------------------
// JS-C15 — `Array.of`, dispatch `@@*`, `@@species`, captureStackTrace
// ------------------------------------------------------------------

/// `Array.of(…items)` — spec : si `this` est un ctor → `new this(len)`
/// (sous-classes `Array.of.call(C, …)`), sinon un Array ordinaire.
fn array_of(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let len = args.len();
    let a = if e.is_callable(this) {
        e.construct(this, &[Value::Num(len as f64)])?
    } else {
        e.new_array(vec![Value::Undefined; len])
    };
    for (i, v) in args.iter().enumerate() {
        e.set_prop_value(a, &i.to_string(), *v)?;
    }
    e.set_prop_value(a, "length", Value::Num(len as f64))?;
    Ok(a)
}

/// `GetMethod(obj, sym)` — dispatch spec `str.x(rx)` → `rx[@@x](…)`.
/// `Ok(None)` → chemin builtin (`sym` absent, `undefined` ou `null`).
fn sym_method(e: &mut Engine, obj: Value, sym: &str, args: &[Value]) -> JsResult<Option<Value>> {
    let Value::Obj(id) = obj else { return Ok(None) };
    match e.get_prop(id, sym) {
        None | Some(Value::Undefined | Value::Null) => Ok(None),
        Some(m) if e.is_callable(m) => e.call_value(m, obj, args).map(Some),
        Some(_) => Err(e.type_error("symbol method is not callable")),
    }
}

/// `ArraySpeciesCreate(src, len)` (spec) : `new src.constructor
/// [@@species](len)` si la prop existe ; un `undefined`/`null` →
/// tableau ordinaire, autre chose → `TypeError` via `construct`.
fn species_array(e: &mut Engine, src: Value, len: usize) -> JsResult<Value> {
    if let Value::Obj(id) = src {
        if matches!(e.heap.obj(id).map(|o| &o.kind), Some(ObjKind::Array(_))) {
            let sp = e.get_prop(id, "constructor").and_then(|c| match c {
                Value::Obj(cid) => e.get_prop(cid, crate::interp::SYM_SPECIES),
                _ => None,
            });
            match sp {
                None | Some(Value::Undefined | Value::Null) => {}
                Some(ctor_v) => return e.construct(ctor_v, &[Value::Num(len as f64)]),
            }
        }
    }
    Ok(e.new_array(Vec::new()))
}

/// Algorithme `rx.exec`-résultat partagé `str.match`/`@@match`
/// (spec : global → liste de textes + `lastIndex` reset ; sinon
/// résultat d'un seul `exec`).
fn re_match_exec(
    e: &mut Engine,
    re: &std::rc::Rc<crate::regexp::JsRegExp>,
    id: crate::value::ObjId,
    s: &str,
) -> JsResult<Value> {
    if re.global {
        let texts: Vec<Value> = re
            .exec_all(s, 0)
            .into_iter()
            .map(|m| e.str_val(m.text))
            .collect();
        let _ = e.set_prop(id, "lastIndex", Value::Num(0.0));
        return Ok(if texts.is_empty() {
            Value::Null
        } else {
            arr_of(e, texts)
        });
    }
    match re.exec(s, 0) {
        Some(m) => Ok(match_result(e, &m, re, s)),
        None => Ok(Value::Null),
    }
}

fn re_match_all_impl(
    e: &mut Engine,
    re: &std::rc::Rc<crate::regexp::JsRegExp>,
    s: &str,
) -> JsResult<Value> {
    if !re.global {
        return Err(e.type_error("matchAll needs a global regex"));
    }
    let items: Vec<Value> = re
        .exec_all(s, 0)
        .iter()
        .map(|m| match_result(e, m, re, s))
        .collect();
    Ok(crate::collections::iter_obj(e, items))
}

/// Branche `RegExp` de `replace`/`@@replace` (global → toutes les
/// occurrences ; sinon la première).
fn re_replace_impl(
    e: &mut Engine,
    re: &std::rc::Rc<crate::regexp::JsRegExp>,
    s: &str,
    rep: Value,
) -> JsResult<Value> {
    let matches = if re.global {
        re.exec_all(s, 0)
    } else {
        re.exec(s, 0).into_iter().collect()
    };
    if matches.is_empty() {
        return Ok(e.str_val(s.to_string()));
    }
    splice_matches(e, s, &matches, rep)
}

/// `rx[Symbol.match](str)` — `RegExp.prototype[@@match]` (spec).
fn re_sym_match(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = e.to_js_string(arg(args, 0))?;
    let (re, id) = as_regexp(e, this)?;
    re_match_exec(e, &re, id, &s)
}

/// `rx[Symbol.matchAll](str)` — itérateur de résultats, `g` requis.
fn re_sym_match_all(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = e.to_js_string(arg(args, 0))?;
    let (re, _id) = as_regexp(e, this)?;
    re_match_all_impl(e, &re, &s)
}

/// `rx[Symbol.replace](str, rep)` — global ou première occurrence.
fn re_sym_replace(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = e.to_js_string(arg(args, 0))?;
    let rep = arg(args, 1);
    let (re, _id) = as_regexp(e, this)?;
    re_replace_impl(e, &re, &s, rep)
}

/// `rx[Symbol.search](str)` — indice du premier match, `-1` sinon.
fn re_sym_search(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = e.to_js_string(arg(args, 0))?;
    let (re, _id) = as_regexp(e, this)?;
    match re.exec(&s, 0) {
        Some(m) => Ok(Value::Num(m.index as f64)),
        None => Ok(Value::Num(-1.0)),
    }
}

/// `rx[Symbol.split](str, limit)` — captures insérées entre les
/// morceaux (délègue à `JsRegExp::split`).
fn re_sym_split(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = e.to_js_string(arg(args, 0))?;
    let limit = match arg(args, 1) {
        Value::Undefined => usize::MAX,
        v => e.to_number(v)? as usize,
    };
    let (re, _id) = as_regexp(e, this)?;
    if limit == 0 {
        return Ok(arr_of(e, Vec::new()));
    }
    let vals: Vec<Value> = re
        .split(&s)
        .into_iter()
        .take(limit)
        .map(|p| e.str_val(p))
        .collect();
    Ok(arr_of(e, vals))
}

/// `Error.captureStackTrace(obj, ctor?)` — extension V8 : pose la
/// own prop `stack` non-énumérable (même contenu `Name: message`
/// que les erreurs moteur — pas de frames, déviation documentée).
fn error_capture_stack(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
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
    let head = if msg.is_empty() {
        name
    } else {
        format!("{name}: {msg}")
    };
    let s = e.str_val(head);
    e.set_own(id, "stack", s, false);
    Ok(Value::Undefined)
}

// ------------------------------------------------------------------
// JS-C16 — reduceRight, locale-fallbacks, URL/URLSearchParams
// ------------------------------------------------------------------

fn array_reduce_right(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this)?;
    let cb = arg(args, 0);
    if !e.is_callable(cb) {
        return Err(e.type_error("reduceRight needs a function"));
    }
    let elems = take_elems(e, id);
    let (mut acc, start) = match args.get(1) {
        Some(v) => (*v, elems.len() as i64 - 1),
        None if elems.is_empty() => {
            put_elems(e, id, elems);
            return Err(e.type_error("reduceRight of empty array"));
        }
        None => (elems[elems.len() - 1], elems.len() as i64 - 2),
    };
    for i in (0..=start).rev() {
        let v = elems[i as usize];
        acc = e.call_value(cb, Value::Undefined, &[acc, v, Value::Num(i as f64), this])?;
    }
    put_elems(e, id, elems);
    Ok(acc)
}

/// `localeCompare` (déviation documentée : ordre par point de
/// code Unicode, pas de collation CLDR) — `-1/0/1`.
fn string_locale_compare(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = this_str(e, this)?;
    let o = e.to_js_string(arg(args, 0))?;
    let r = match s.cmp(&o) {
        std::cmp::Ordering::Less => -1.0,
        std::cmp::Ordering::Equal => 0.0,
        std::cmp::Ordering::Greater => 1.0,
    };
    Ok(Value::Num(r))
}

/// `Number.prototype.toLocaleString` — repli `toString` (pas de
/// locale, déviation documentée).
fn number_to_locale(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    number_to_string(e, this, args)
}

// -- URL (WHATWG subset) ---------------------------------------------

/// Parts décomposées d'une URL — pas d'IDNA ni de validation
/// d'hôte stricte (subset navigateur : schéma/authority/path/
/// query/fragment + résolution relative basique).
struct UrlParts {
    scheme: String,
    user: String,
    pass: String,
    host: String,
    port: String,
    path: String,
    query: String,
    frag: String,
}

const URL_DEFAULT_PORTS: &[(&str, &str)] = &[
    ("http", "80"),
    ("https", "443"),
    ("ws", "80"),
    ("wss", "443"),
    ("ftp", "21"),
];

fn url_has_origin(scheme: &str) -> bool {
    matches!(scheme, "http" | "https" | "ws" | "wss" | "ftp" | "file")
}

/// Découpe `user:pass@host:port` (authority après `//`).
fn url_split_authority(auth: &str, p: &mut UrlParts) {
    let (up, hp) = match auth.rsplit_once('@') {
        Some((u, h)) => (u, h),
        None => ("", auth),
    };
    if let Some((u, pw)) = up.split_once(':') {
        p.user = u.to_string();
        p.pass = pw.to_string();
    } else {
        p.user = up.to_string();
    }
    // Port = `:digits` en fin d'authority (host nu IPv6 `[…]` ok).
    if let Some(brk) = hp.rsplit_once(':') {
        if !brk.1.is_empty() && brk.1.chars().all(|c| c.is_ascii_digit()) && !brk.0.ends_with(']') {
            let default = URL_DEFAULT_PORTS
                .iter()
                .find(|(s, _)| s == &p.scheme.as_str())
                .map_or("", |(_, d)| *d);
            p.host = brk.0.to_string();
            p.port = if brk.1 == default {
                String::new()
            } else {
                brk.1.to_string()
            };
            return;
        }
    }
    p.host = hp.to_string();
}

/// Normalise les segments `.`/`..` d'un chemin.
fn url_norm_path(path: &str) -> String {
    let mut segs: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "." => {}
            ".." => {
                segs.pop();
            }
            _ => segs.push(seg),
        }
    }
    let out = segs.join("/");
    if path.starts_with('/') && !out.starts_with('/') {
        return format!("/{out}");
    }
    out
}

/// Parse absolu `scheme:…` — `None` si pas de schéma.
fn url_parse_absolute(s: &str) -> Option<UrlParts> {
    let colon = s.find(':')?;
    let scheme = &s[..colon];
    if scheme.is_empty()
        || !scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        || !scheme
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic())
    {
        return None;
    }
    let mut p = UrlParts {
        scheme: scheme.to_ascii_lowercase(),
        user: String::new(),
        pass: String::new(),
        host: String::new(),
        port: String::new(),
        path: String::new(),
        query: String::new(),
        frag: String::new(),
    };
    let mut rest = &s[colon + 1..];
    if let Some(a) = rest.strip_prefix("//") {
        rest = a;
        let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        url_split_authority(&rest[..end], &mut p);
        rest = &rest[end..];
    }
    let path_end = rest.find(['?', '#']).unwrap_or(rest.len());
    p.path = url_norm_path(&rest[..path_end]);
    rest = &rest[path_end..];
    if let Some(r) = rest.strip_prefix('?') {
        let qend = r.find('#').unwrap_or(r.len());
        p.query = r[..qend].to_string();
        rest = &r[qend..];
    }
    if let Some(f) = rest.strip_prefix('#') {
        p.frag = f.to_string();
    }
    Some(p)
}

/// Résolution relative basique (WHATWG `basic URL parser` subset) :
/// `//h`, `/p`, `?q`, `#f`, chemin relatif depuis le dossier de base.
fn url_parse(href: &str, base: Option<&UrlParts>) -> Option<UrlParts> {
    let s = href.trim();
    if let Some(p) = url_parse_absolute(s) {
        return Some(p);
    }
    let b = base?;
    let mut p = UrlParts {
        scheme: b.scheme.clone(),
        user: b.user.clone(),
        pass: b.pass.clone(),
        host: b.host.clone(),
        port: b.port.clone(),
        path: b.path.clone(),
        query: b.query.clone(),
        frag: String::new(),
    };
    if s.is_empty() {
        p.frag.clone_from(&b.frag);
        return Some(p);
    }
    if let Some(f) = s.strip_prefix('#') {
        p.frag = f.to_string();
        return Some(p);
    }
    p.frag.clear();
    if let Some(q) = s.strip_prefix('?') {
        p.query = q.to_string();
        return Some(p);
    }
    p.query.clear();
    if let Some(a) = s.strip_prefix("//") {
        let end = a.find(['/', '?', '#']).unwrap_or(a.len());
        url_split_authority(&a[..end], &mut p);
        let rest = &a[end..];
        let path_end = rest.find(['?', '#']).unwrap_or(rest.len());
        p.path = url_norm_path(&rest[..path_end]);
        let rest = &rest[path_end..];
        if let Some(r) = rest.strip_prefix('?') {
            let qend = r.find('#').unwrap_or(r.len());
            p.query = r[..qend].to_string();
            if let Some(f) = r[qend..].strip_prefix('#') {
                p.frag = f.to_string();
            }
        } else if let Some(f) = rest.strip_prefix('#') {
            p.frag = f.to_string();
        }
        return Some(p);
    }
    // Chemin : absolu (`/p`) ou relatif au dossier de base.
    let path_end = s.find(['?', '#']).unwrap_or(s.len());
    let raw = &s[..path_end];
    p.path = if raw.starts_with('/') {
        url_norm_path(raw)
    } else {
        let dir = match b.path.rfind('/') {
            Some(i) => &b.path[..=i],
            None => "/",
        };
        url_norm_path(&format!("{dir}{raw}"))
    };
    let rest = &s[path_end..];
    if let Some(r) = rest.strip_prefix('?') {
        let qend = r.find('#').unwrap_or(r.len());
        p.query = r[..qend].to_string();
        if let Some(f) = r[qend..].strip_prefix('#') {
            p.frag = f.to_string();
        }
    } else if let Some(f) = rest.strip_prefix('#') {
        p.frag = f.to_string();
    }
    Some(p)
}

fn url_href(p: &UrlParts) -> String {
    let mut out = format!("{}:", p.scheme);
    // `file:` garde son authority vide (`file:///x`) — spec.
    if !p.host.is_empty() || !p.user.is_empty() || p.scheme == "file" {
        out.push_str("//");
        if !p.user.is_empty() {
            out.push_str(&p.user);
            if !p.pass.is_empty() {
                out.push(':');
                out.push_str(&p.pass);
            }
            out.push('@');
        }
        out.push_str(&p.host);
        if !p.port.is_empty() {
            out.push(':');
            out.push_str(&p.port);
        }
    }
    if p.path.is_empty() && url_has_origin(&p.scheme) {
        out.push('/');
    } else {
        out.push_str(&p.path);
    }
    if !p.query.is_empty() {
        out.push('?');
        out.push_str(&p.query);
    }
    if !p.frag.is_empty() {
        out.push('#');
        out.push_str(&p.frag);
    }
    out
}

fn url_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let href = e.to_js_string(arg(args, 0))?;
    // `base` peut être une `URL` (lit son `href`) ou une string.
    let base_s = match arg(args, 1) {
        Value::Undefined => None,
        v => Some(e.to_js_string(v)?),
    };
    let base = base_s.as_deref().and_then(|b| url_parse_absolute(b.trim()));
    if base_s.is_some() && base.is_none() {
        return Err(e.type_error("Invalid base URL"));
    }
    let Some(p) = url_parse(&href, base.as_ref()) else {
        return Err(e.type_error("Invalid URL"));
    };
    let proto = Value::Obj(e.protos.url);
    let id = e.heap.alloc_obj(crate::heap::Object::ordinary(proto));
    let host = if p.port.is_empty() {
        p.host.clone()
    } else {
        format!("{}:{}", p.host, p.port)
    };
    let origin = if url_has_origin(&p.scheme) && p.scheme != "file" {
        let mut o = format!("{}://{}", p.scheme, p.host);
        if !p.port.is_empty() {
            o.push(':');
            o.push_str(&p.port);
        }
        o
    } else {
        "null".to_string()
    };
    let fields = [
        ("href", url_href(&p)),
        ("protocol", format!("{}:", p.scheme)),
        ("origin", origin),
        ("username", p.user.clone()),
        ("password", p.pass.clone()),
        ("host", host),
        ("hostname", p.host.clone()),
        ("port", p.port.clone()),
        (
            "pathname",
            if p.path.is_empty() && url_has_origin(&p.scheme) {
                "/".to_string()
            } else {
                p.path.clone()
            },
        ),
        (
            "search",
            if p.query.is_empty() {
                String::new()
            } else {
                format!("?{}", p.query)
            },
        ),
        (
            "hash",
            if p.frag.is_empty() {
                String::new()
            } else {
                format!("#{}", p.frag)
            },
        ),
    ];
    for (k, v) in fields {
        let sv = e.str_val(v);
        e.set_own(id, k, sv, true);
    }
    // `searchParams` lié à la query.
    let pairs = url_decode_pairs(&p.query);
    let sp = usp_new(e, pairs);
    e.set_own(id, "searchParams", sp, true);
    Ok(Value::Obj(id))
}

fn url_href_get(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = this else {
        return Err(e.type_error("URL method on non-URL"));
    };
    Ok(e.get_prop(id, "href").unwrap_or(Value::Undefined))
}

// -- URLSearchParams (x-www-form-urlencoded) --------------------------

/// Paire `(name, value)` encodée formulaire — `+` = espace.
fn url_decode_pairs(q: &str) -> Vec<(String, String)> {
    if q.is_empty() {
        return Vec::new();
    }
    q.split('&')
        .filter(|seg| !seg.is_empty())
        .map(|seg| {
            let (k, v) = seg.split_once('=').unwrap_or((seg, ""));
            (form_decode(k), form_decode(v))
        })
        .collect()
}

fn form_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                let hex = |b: u8| (b as char).to_digit(16);
                match (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                    (Some(h), Some(l)) => {
                        out.push((h * 16 + l) as u8);
                        i += 2;
                    }
                    _ => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn form_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'*' | b'-' | b'.' | b'_' => {
                out.push(b as char);
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

const USP_PAIRS: &str = "\\u{f131}urlp";

/// Lit les paires cachées `\u{f131}urlp` (array `[ [k,v],… ]`).
fn usp_pairs(e: &mut Engine, id: crate::value::ObjId) -> Vec<(String, String)> {
    let Some(Value::Obj(arr)) = e.get_prop(id, USP_PAIRS) else {
        return Vec::new();
    };
    let n = match e.get_prop(arr, "length") {
        Some(Value::Num(n)) => n.max(0.0) as usize,
        _ => 0,
    };
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        if let Some(Value::Obj(pair)) = e.get_prop(arr, &i.to_string()) {
            let k = e
                .get_prop(pair, "0")
                .map(|v| e.to_js_string(v).unwrap_or_default())
                .unwrap_or_default();
            let v = e
                .get_prop(pair, "1")
                .map(|v| e.to_js_string(v).unwrap_or_default())
                .unwrap_or_default();
            out.push((k, v));
        }
    }
    out
}

fn usp_write(e: &mut Engine, id: crate::value::ObjId, pairs: Vec<(String, String)>) {
    let vals: Vec<Value> = pairs
        .into_iter()
        .map(|(k, v)| {
            let k = e.str_val(k);
            let v = e.str_val(v);
            e.new_array(vec![k, v])
        })
        .collect();
    let arr = e.new_array(vals);
    e.set_own(id, USP_PAIRS, arr, false);
}

/// Nouvel objet `URLSearchParams` (proto `protos.url_params`).
fn usp_new(e: &mut Engine, pairs: Vec<(String, String)>) -> Value {
    let proto = Value::Obj(e.protos.url_params);
    let id = e.heap.alloc_obj(crate::heap::Object::ordinary(proto));
    usp_write(e, id, pairs);
    Value::Obj(id)
}

fn usp_this(e: &mut Engine, this: Value) -> JsResult<(crate::value::ObjId, Vec<(String, String)>)> {
    let Value::Obj(id) = this else {
        return Err(e.type_error("URLSearchParams method on non-object"));
    };
    if !matches!(e.get_prop(id, USP_PAIRS), Some(Value::Obj(_))) {
        return Err(e.type_error("URLSearchParams method on incompatible receiver"));
    }
    Ok((id, usp_pairs(e, id)))
}

fn usp_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let mut pairs = Vec::new();
    match arg(args, 0) {
        Value::Undefined | Value::Null => {}
        v @ Value::Obj(oid) => {
            // Iterable (`[[k,v],…]`, autre URLSearchParams via son
            // `[Symbol.iterator]`) → paires ; non-itérable → record
            // `{k: v}` dans l'ordre des props énumérables.
            match e.iter_values(v) {
                Ok(items) => {
                    for it in items {
                        let k = e.get_prop_value(it, "0")?;
                        let val = e.get_prop_value(it, "1")?;
                        pairs.push((e.to_js_string(k)?, e.to_js_string(val)?));
                    }
                }
                Err(_) => {
                    if matches!(e.get_prop(oid, USP_PAIRS), Some(Value::Obj(_))) {
                        // Copie directe si l'itérateur a été cassé.
                        pairs = usp_pairs(e, oid);
                    } else {
                        for k in e.own_enum_keys(oid) {
                            let val = e.get_prop_value(v, &k)?;
                            pairs.push((k, e.to_js_string(val)?));
                        }
                    }
                }
            }
        }
        v => {
            let s = e.to_js_string(v)?;
            let q = s.strip_prefix('?').unwrap_or(&s);
            pairs = url_decode_pairs(q);
        }
    }
    Ok(usp_new(e, pairs))
}

fn usp_get(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (_id, pairs) = usp_this(e, this)?;
    let name = e.to_js_string(arg(args, 0))?;
    match pairs.iter().find(|(k, _)| *k == name) {
        Some((_, v)) => Ok(e.str_val(v.clone())),
        None => Ok(Value::Null),
    }
}

fn usp_get_all(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (_id, pairs) = usp_this(e, this)?;
    let name = e.to_js_string(arg(args, 0))?;
    let vals: Vec<Value> = pairs
        .iter()
        .filter(|(k, _)| *k == name)
        .map(|(_, v)| e.str_val(v.clone()))
        .collect();
    Ok(e.new_array(vals))
}

fn usp_has(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (_id, pairs) = usp_this(e, this)?;
    let name = e.to_js_string(arg(args, 0))?;
    Ok(Value::Bool(pairs.iter().any(|(k, _)| *k == name)))
}

fn usp_set(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (id, mut pairs) = usp_this(e, this)?;
    let name = e.to_js_string(arg(args, 0))?;
    let val = e.to_js_string(arg(args, 1))?;
    let mut out = Vec::with_capacity(pairs.len());
    let mut inserted = false;
    for (k, v) in pairs.drain(..) {
        if k == name {
            if !inserted {
                out.push((k, val.clone()));
                inserted = true;
            }
        } else {
            out.push((k, v));
        }
    }
    if !inserted {
        out.push((name, val));
    }
    usp_write(e, id, out);
    Ok(Value::Undefined)
}

fn usp_append(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (id, mut pairs) = usp_this(e, this)?;
    let name = e.to_js_string(arg(args, 0))?;
    let val = e.to_js_string(arg(args, 1))?;
    pairs.push((name, val));
    usp_write(e, id, pairs);
    Ok(Value::Undefined)
}

fn usp_delete(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (id, pairs) = usp_this(e, this)?;
    let name = e.to_js_string(arg(args, 0))?;
    let val_filter = match arg(args, 1) {
        Value::Undefined => None,
        v => Some(e.to_js_string(v)?),
    };
    let keep: Vec<(String, String)> = pairs
        .into_iter()
        .filter(|(k, v)| !(k == &name && val_filter.as_ref().is_none_or(|f| f == v)))
        .collect();
    usp_write(e, id, keep);
    Ok(Value::Undefined)
}

fn usp_sort(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let (id, mut pairs) = usp_this(e, this)?;
    // Tri stable sur la clé (spec : code points).
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    usp_write(e, id, pairs);
    Ok(Value::Undefined)
}

fn usp_for_each(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (_id, pairs) = usp_this(e, this)?;
    let cb = arg(args, 0);
    if !e.is_callable(cb) {
        return Err(e.type_error("forEach needs a function"));
    }
    let this_arg = arg(args, 1);
    for (k, v) in pairs {
        let kv = e.str_val(k);
        let vv = e.str_val(v);
        e.call_value(cb, this_arg, &[vv, kv, this])?;
    }
    Ok(Value::Undefined)
}

fn usp_to_string(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let (_id, pairs) = usp_this(e, this)?;
    let out: Vec<String> = pairs
        .iter()
        .map(|(k, v)| format!("{}={}", form_encode(k), form_encode(v)))
        .collect();
    Ok(e.str_val(out.join("&")))
}

fn usp_keys(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let (_id, pairs) = usp_this(e, this)?;
    let vals: Vec<Value> = pairs.into_iter().map(|(k, _)| e.str_val(k)).collect();
    Ok(crate::collections::iter_obj(e, vals))
}

fn usp_values(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let (_id, pairs) = usp_this(e, this)?;
    let vals: Vec<Value> = pairs.into_iter().map(|(_, v)| e.str_val(v)).collect();
    Ok(crate::collections::iter_obj(e, vals))
}

fn usp_entries(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let (_id, pairs) = usp_this(e, this)?;
    let vals: Vec<Value> = pairs
        .into_iter()
        .map(|(k, v)| {
            let k = e.str_val(k);
            let v = e.str_val(v);
            e.new_array(vec![k, v])
        })
        .collect();
    Ok(crate::collections::iter_obj(e, vals))
}

// ------------------------------------------------------------------
// JS-C17 — Float16/copyWithin/crypto/Iterator/Annex-B/SuppressedError
// ------------------------------------------------------------------

/// `Array.prototype.copyWithin(target, start?, end?)` — copie
/// interne relative (indices négatifs = depuis la fin, spec).
fn array_copy_within(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = this_array(e, this)?;
    let mut elems = take_elems(e, id);
    let len = elems.len() as i64;
    let rel = |v: Value, e: &mut Engine| -> JsResult<i64> { Ok(e.to_number(v)? as i64) };
    let target = match rel(arg(args, 0), e)? {
        i if i < 0 => (len + i).max(0),
        i => i.min(len),
    };
    let start = match arg(args, 1) {
        Value::Undefined => 0,
        v => match rel(v, e)? {
            i if i < 0 => (len + i).max(0),
            i => i.min(len),
        },
    };
    let end = match arg(args, 2) {
        Value::Undefined => len,
        v => match rel(v, e)? {
            i if i < 0 => (len + i).max(0),
            i => i.min(len),
        },
    };
    // Snapshot de la plage source — `copyWithin` a une sémantique
    // memmove : les chevauchements copient les valeurs d'origine,
    // pas les valeurs déjà réécrites (spec `Array.prototype.
    // copyWithin` — copie d'abord, puis pose).
    let count = (end - start).max(0).min(len - target);
    if count > 0 {
        let chunk: Vec<Value> = elems[start as usize..(start + count) as usize].to_vec();
        for (k, v) in chunk.into_iter().enumerate() {
            elems[(target as usize) + k] = v;
        }
    }
    put_elems(e, id, elems);
    Ok(this)
}

/// `Math.f16round(v)` — arrondi IEEE-754 binary16 (ES2025).
fn math_f16round(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let v = e.to_number(arg(args, 0))?;
    Ok(Value::Num(crate::heap::f16_to_f64(
        crate::heap::f64_to_f16(v),
    )))
}

/// PRNG partagé `Math.random`/`crypto` — xorshift64 (même schéma
/// que `math_random` ; PAS cryptographique — déviation).
fn next_rand(e: &mut Engine) -> u64 {
    e.rng ^= e.rng >> 12;
    e.rng ^= e.rng << 25;
    e.rng ^= e.rng >> 27;
    e.rng.wrapping_mul(0x2545_f491_4f6c_dd1d)
}

/// `crypto.getRandomValues(ta)` — remplit la vue d'octets
/// pseudo-aléatoires (`TypeError` sur non-TypedArray, spec).
fn crypto_get_random_values(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let v = arg(args, 0);
    let Value::Obj(id) = v else {
        return Err(e.type_error("getRandomValues needs a TypedArray"));
    };
    let Some((ta, bid, offset, len)) = e.heap.obj(id).and_then(|o| match &o.kind {
        ObjKind::TypedArray {
            ta,
            buffer: Value::Obj(bid),
            offset,
            len,
        } => Some((*ta, *bid, *offset, *len)),
        _ => None,
    }) else {
        return Err(e.type_error("getRandomValues needs a TypedArray"));
    };
    // Quota spec : 65 536 octets max.
    if len.saturating_mul(ta.size()) > 65_536 {
        return Err(e.type_error("getRandomValues: quota exceeded"));
    }
    let n = len * ta.size();
    // Tirage hors emprunt du heap (le PRNG vit dans `e`) —
    // borné par le quota ci-dessus (≤ 64 Ko).
    let mut buf = vec![0u8; n];
    for chunk in buf.chunks_mut(8) {
        let r = next_rand(e);
        chunk.copy_from_slice(&r.to_le_bytes()[..chunk.len()]);
    }
    let Some(b) = e.heap.obj_mut(bid) else {
        return Err(e.type_error("detached typed array"));
    };
    let ObjKind::ByteBuf(bytes) = &mut b.kind else {
        return Err(e.type_error("detached typed array"));
    };
    let end = (offset + n).min(bytes.len());
    bytes[offset..end].copy_from_slice(&buf[..end - offset]);
    Ok(v)
}

/// `crypto.randomUUID()` — v4 via le PRNG moteur (déviation :
/// pas cryptographique).
fn crypto_random_uuid(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    let mut b = [0u8; 16];
    for chunk in b.chunks_mut(8) {
        let r = next_rand(e);
        chunk.copy_from_slice(&r.to_le_bytes()[..chunk.len()]);
    }
    b[6] = (b[6] & 0x0f) | 0x40; // version 4
    b[8] = (b[8] & 0x3f) | 0x80; // variant RFC 4122
    let hex = |v: u8| format!("{v:02x}");
    let s: String = b.iter().map(|&v| hex(v)).collect();
    Ok(e.str_val(format!(
        "{}-{}-{}-{}-{}",
        &s[0..8],
        &s[8..12],
        &s[12..16],
        &s[16..20],
        &s[20..32]
    )))
}

/// `new Iterator()` — constructeur abstrait (spec `TypeError`).
fn iterator_ctor(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    Err(e.type_error("The Iterator constructor is abstract"))
}

/// `Iterator.from(v)` — un itérateur passe tel quel (spec :
/// `GetIteratorFlattenable`), un itérable passe par `@@iterator`.
fn iterator_from(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let v = arg(args, 0);
    if let Value::Obj(id) = v {
        if e.get_prop(id, "next").is_some_and(|n| e.is_callable(n)) {
            return Ok(v);
        }
        if let Some(m) = e.get_prop(id, crate::interp::SYM_ITERATOR) {
            if e.is_callable(m) {
                return e.call_value(m, v, &[]);
            }
        }
        return Err(e.type_error("Iterator.from on non-iterable"));
    }
    if let Value::Str(_) = v {
        let items = e.iter_values(v)?;
        return Ok(crate::collections::iter_obj(e, items));
    }
    Err(e.type_error("Iterator.from on non-iterable"))
}

/// `SuppressedError(error, suppressed, message?)` — ES2026.
fn suppressed_error_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let msg = match arg(args, 2) {
        Value::Undefined => String::new(),
        v => e.to_js_string(v)?,
    };
    let se_p = e
        .get_prop(e.global, "SuppressedError")
        .and_then(|c| {
            e.get_prop(
                match c {
                    Value::Obj(id) => id,
                    _ => e.protos.error,
                },
                "prototype",
            )
        })
        .unwrap_or(Value::Obj(e.protos.error));
    let err = e.error_value(
        match se_p {
            Value::Obj(id) => id,
            _ => e.protos.error,
        },
        "SuppressedError",
        &msg,
    );
    if let Value::Obj(id) = err {
        e.set_own(id, "error", arg(args, 0), true);
        e.set_own(id, "suppressed", arg(args, 1), true);
    }
    Ok(err)
}

/// `Object.prototype.propertyIsEnumerable(k)` — own + flag
/// énumérable (spec ; indices de string wrappers = énumérables).
fn object_property_is_enumerable(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let key = e.prop_key(arg(args, 0))?;
    match this {
        Value::Obj(id) => {
            let Some(p) = e.get_own_prop(id, &key)? else {
                return Ok(Value::Bool(false));
            };
            Ok(Value::Bool(p.enumerable()))
        }
        Value::Str(sid) => {
            let s = e.heap.string(sid).unwrap_or_default();
            let ok = crate::interp::array_index(&key).is_some_and(|i| i < s.chars().count());
            Ok(Value::Bool(ok))
        }
        _ => Ok(Value::Bool(false)),
    }
}

/// `__defineGetter__(k, f)` — accesseur `{get: f}` énumérable +
/// configurable (Annex B).
fn object_define_getter(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    define_accessor_annex(e, this, arg(args, 0), arg(args, 1), true)
}

fn object_define_setter(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    define_accessor_annex(e, this, arg(args, 0), arg(args, 1), false)
}

fn define_accessor_annex(
    e: &mut Engine,
    this: Value,
    key_v: Value,
    f: Value,
    is_get: bool,
) -> JsResult<Value> {
    if !e.is_callable(f) {
        return Err(e.type_error("__defineGetter__/__defineSetter__ needs a function"));
    }
    let Value::Obj(id) = this else {
        return Err(e.type_error("can't define accessor on a primitive"));
    };
    let key = e.prop_key(key_v)?;
    let (get, set) = if is_get {
        (f, Value::Undefined)
    } else {
        (Value::Undefined, f)
    };
    if let Some(o) = e.heap.obj_mut(id) {
        o.define_accessor(
            &key,
            get,
            set,
            crate::heap::PropAttrs {
                writable: false,
                enumerable: true,
                configurable: true,
            },
        );
    }
    Ok(Value::Undefined)
}

/// `__lookupGetter__(k)` — premier `get` trouvé en remontant la
/// chaîne de prototypes (Annex B).
fn object_lookup_getter(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    lookup_accessor_annex(e, this, arg(args, 0), true)
}

fn object_lookup_setter(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    lookup_accessor_annex(e, this, arg(args, 0), false)
}

fn lookup_accessor_annex(
    e: &mut Engine,
    this: Value,
    key_v: Value,
    is_get: bool,
) -> JsResult<Value> {
    let key = e.prop_key(key_v)?;
    let Value::Obj(mut id) = this else {
        return Ok(Value::Undefined);
    };
    let mut hops = 0;
    loop {
        hops += 1;
        if hops > 64 {
            return Ok(Value::Undefined);
        }
        let Some(o) = e.heap.obj(id) else {
            return Ok(Value::Undefined);
        };
        if let Some(crate::heap::Prop::Acc { get, set, .. }) = o.find(&key) {
            return Ok(if is_get { get } else { set });
        }
        match o.proto {
            Value::Obj(next) => id = next,
            _ => return Ok(Value::Undefined),
        }
    }
}

/// `get __proto__` (Annex B) — `[[Prototype]]` du receiver ;
/// primitives → proto de leur wrapper (`(1).__proto__` =
/// `Number.prototype`).
fn proto_getter(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    match this {
        Value::Obj(id) => Ok(e.heap.obj(id).map_or(Value::Null, |o| o.proto)),
        Value::Str(_) => Ok(Value::Obj(e.protos.string)),
        Value::Num(_) => Ok(Value::Obj(e.protos.number)),
        Value::Bool(_) => Ok(Value::Obj(e.protos.boolean)),
        Value::Big(_) => Ok(Value::Obj(e.protos.bigint)),
        _ => Ok(Value::Null),
    }
}

/// `set __proto__` (Annex B) — `o.proto = v` si `v` est objet ou
/// `null` ; autres valeurs ignorées (spec).
fn proto_setter(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    if let Value::Obj(id) = this {
        if let v @ (Value::Obj(_) | Value::Null) = arg(args, 0) {
            if let Some(o) = e.heap.obj_mut(id) {
                o.proto = v;
            }
        }
    }
    Ok(Value::Undefined)
}

// ------------------------------------------------------------------
// JS-C17 — `DisposableStack` (ES2026)
// ------------------------------------------------------------------

/// Extrait la pile interne (vérifie `ObjKind::DispStack`).
fn ds_state(e: &mut Engine, this: Value) -> JsResult<ObjId> {
    match this {
        Value::Obj(id) => match e.heap.obj(id).map(|o| &o.kind) {
            Some(ObjKind::DispStack { .. }) => Ok(id),
            _ => Err(e.type_error("not a DisposableStack")),
        },
        _ => Err(e.type_error("not a DisposableStack")),
    }
}

/// `ReferenceError` sur pile déjà déposée (spec `dispose`/`use`).
fn ds_live(e: &mut Engine, id: ObjId) -> JsResult<()> {
    let disposed = match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::DispStack { disposed, .. }) => *disposed,
        _ => true,
    };
    if disposed {
        Err(e.ref_error("DisposableStack is already disposed"))
    } else {
        Ok(())
    }
}

fn ds_push(e: &mut Engine, id: ObjId, f: Value, t: Value, v: Value) {
    if let Some(o) = e.heap.obj_mut(id) {
        if let ObjKind::DispStack { stack, .. } = &mut o.kind {
            stack.push((f, t, v));
        }
    }
}

fn disposable_stack_ctor(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    let proto = e
        .get_prop(e.global, "DisposableStack")
        .and_then(|c| match c {
            Value::Obj(cid) => e.get_prop(cid, "prototype"),
            _ => None,
        })
        .unwrap_or(Value::Obj(e.protos.object));
    let mut o = crate::heap::Object::ordinary(proto);
    o.kind = ObjKind::DispStack {
        disposed: false,
        stack: Vec::new(),
    };
    Ok(Value::Obj(e.heap.alloc_obj(o)))
}

/// `get disposed` — `true` après `dispose()`/`move()`.
fn ds_disposed_get(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let id = ds_state(e, this)?;
    Ok(Value::Bool(ds_live(e, id).is_err()))
}

/// `use(v)` — enregistre `v[@@dispose]` ; `null`/`undefined`
/// passent sans enregistrement (spec).
fn ds_use(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = ds_state(e, this)?;
    ds_live(e, id)?;
    let v = arg(args, 0);
    if matches!(v, Value::Null | Value::Undefined) {
        return Ok(v);
    }
    let m = match v {
        Value::Obj(vid) => e.get_prop(vid, crate::interp::SYM_DISPOSE),
        _ => None,
    };
    match m {
        Some(f) if e.is_callable(f) => {
            // `v[@@dispose]()` — `this` = la ressource, pas d'arg.
            ds_push(e, id, f, v, Value::Undefined);
            Ok(v)
        }
        _ => Err(e.type_error("value has no [Symbol.dispose] method")),
    }
}

/// `adopt(v, f)` — disposeur arbitraire `f(v)`.
fn ds_adopt(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = ds_state(e, this)?;
    ds_live(e, id)?;
    let f = arg(args, 1);
    if !e.is_callable(f) {
        return Err(e.type_error("adopt onDispose is not callable"));
    }
    ds_push(e, id, f, Value::Undefined, arg(args, 0));
    Ok(arg(args, 0))
}

/// `defer(f)` — disposeur sans argument.
fn ds_defer(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = ds_state(e, this)?;
    ds_live(e, id)?;
    let f = arg(args, 0);
    if !e.is_callable(f) {
        return Err(e.type_error("defer callback is not callable"));
    }
    ds_push(e, id, f, Value::Undefined, Value::Undefined);
    Ok(Value::Undefined)
}

/// `dispose()` — LIFO ; première erreur propagée, les erreurs
/// suivantes enroulées en `SuppressedError(err, prev)` (spec).
fn ds_dispose(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let id = ds_state(e, this)?;
    ds_live(e, id)?;
    if let Some(o) = e.heap.obj_mut(id) {
        if let ObjKind::DispStack { disposed, .. } = &mut o.kind {
            *disposed = true;
        }
    }
    let mut pending: Option<Value> = None;
    loop {
        let next = match e.heap.obj_mut(id).map(|o| &mut o.kind) {
            Some(ObjKind::DispStack { stack, .. }) => stack.pop(),
            _ => None,
        };
        let Some((f, t, v)) = next else {
            break;
        };
        if let Err(Thrown(err)) = e.call_value(f, t, &[v]) {
            pending = Some(match pending {
                None => err,
                Some(prev) => make_suppressed(e, err, prev),
            });
        }
    }
    match pending {
        None => Ok(Value::Undefined),
        Some(err) => Err(Thrown(err)),
    }
}

/// `move()` — transfère la pile dans une nouvelle instance ;
/// l'originale est marquée déposée (vide).
fn ds_move(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let id = ds_state(e, this)?;
    ds_live(e, id)?;
    let fresh = disposable_stack_ctor(e, Value::Undefined, &[])?;
    let mut moved = Vec::new();
    if let Some(o) = e.heap.obj_mut(id) {
        if let ObjKind::DispStack { disposed, stack } = &mut o.kind {
            *disposed = true;
            moved = std::mem::take(stack);
        }
    }
    if let Value::Obj(nid) = fresh {
        if let Some(o) = e.heap.obj_mut(nid) {
            if let ObjKind::DispStack { stack, .. } = &mut o.kind {
                *stack = moved;
            }
        }
    }
    Ok(fresh)
}

/// Construit un `SuppressedError` (partagé `dispose` + ctor).
fn make_suppressed(e: &mut Engine, err: Value, prev: Value) -> Value {
    let proto = e
        .get_prop(e.global, "SuppressedError")
        .and_then(|c| match c {
            Value::Obj(cid) => e.get_prop(cid, "prototype"),
            _ => None,
        })
        .unwrap_or(Value::Obj(e.protos.error));
    let pid = match proto {
        Value::Obj(p) => p,
        _ => e.protos.error,
    };
    let v = e.error_value(pid, "SuppressedError", "");
    if let Value::Obj(id) = v {
        e.set_own(id, "error", err, true);
        e.set_own(id, "suppressed", prev, true);
    }
    v
}

// ------------------------------------------------------------------
// JS-C18 — Annex B complétion + Number/Atomics
// ------------------------------------------------------------------

/// `Number.isSafeInteger(v)` — entier dans ]-2^53, 2^53[.
fn number_is_safe_integer(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let ok = match arg(args, 0) {
        Value::Num(n) => n.is_finite() && n.fract() == 0.0 && n.abs() < 9_007_199_254_740_992.0,
        _ => false,
    };
    let _ = e;
    Ok(Value::Bool(ok))
}

/// `d.getYear()` (Annex B) — `getFullYear() - 1900`.
fn date_get_year(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let y = date_part(e, this, 0)?;
    Ok(Value::Num(if y.is_nan() { y } else { y - 1900.0 }))
}

/// `d.setYear(y)` (Annex B) — 0-99 → 1900+y ; conserve mois/jour/
/// heure ; retourne le nouveau timestamp (NaN → Invalid Date).
fn date_set_year(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = this else {
        return Err(e.type_error("Date method on non-Date"));
    };
    if e.get_prop(id, "__time").is_none() {
        return Err(e.type_error("Date method on non-Date"));
    }
    let y = e.to_number(arg(args, 0))?;
    let ms = e.get_prop(id, "__time").map_or(f64::NAN, |v| match v {
        Value::Num(n) => n,
        _ => f64::NAN,
    });
    let new_ms = if y.is_nan() {
        f64::NAN
    } else {
        let (oy, mo, d, _w, h, mi, s, mss) = if ms.is_finite() {
            ms_to_parts(ms)
        } else {
            (1970, 1, 1, 4, 0, 0, 0, 0) // date invalide → base epoch
        };
        let _ = oy;
        let yi = y.trunc() as i64;
        let full = if (0..=99).contains(&yi) {
            yi + 1900
        } else {
            yi
        };
        // `ms_to_parts` rend le mois 1-based ; `parts_to_ms`
        // attend le mois 0-based JS.
        parts_to_ms(
            full,
            i64::from(mo) - 1,
            i64::from(d),
            i64::from(h),
            i64::from(mi),
            i64::from(s),
            i64::from(mss),
        )
    };
    e.set_own(id, "__time", Value::Num(new_ms), false);
    Ok(Value::Num(new_ms))
}

/// `str.m()` Annex B — `<tag>m</tag>` ou `<tag attr="v">m</tag>`
/// (`"` échappé `&quot;` dans la valeur d'attribut, spec `CreateHTML`).
/// Bound args : `[tag, attr]` ; `attr` vide → pas d'attribut.
fn string_html_wrap(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = this_str(e, this)?;
    let tag = e.to_js_string(arg(args, 0))?;
    let attr = e.to_js_string(arg(args, 1))?;
    let out = if attr.is_empty() {
        format!("<{tag}>{s}</{tag}>")
    } else {
        let v = e.to_js_string(arg(args, 2))?.replace('"', "&quot;");
        format!("<{tag} {attr}=\"{v}\">{s}</{tag}>")
    };
    Ok(e.str_val(out))
}
