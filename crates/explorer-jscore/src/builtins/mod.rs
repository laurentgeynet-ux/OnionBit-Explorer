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

pub(crate) fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).copied().unwrap_or(Value::Undefined)
}

/// Wire `Ctor.prototype = proto` and `proto.constructor = Ctor`.
pub(crate) fn ctor(e: &mut Engine, name: &'static str, f: NativeFn, proto: ObjId) -> Value {
    let c = e.native_fn(name, f);
    e.set_own(c_id(c), "prototype", Value::Obj(proto), false);
    e.set_own(proto, "constructor", c, false);
    c
}

pub(crate) fn c_id(v: Value) -> ObjId {
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
    {
        let ev = e.native_fn("eval", eval_fn);
        e.set_own(g, "eval", ev, false);
        e.eval_fn = ev;
    }
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
        e.add_method(eid, "isError", error_is_error);
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
    // `EvalError`/`RangeError`/`URIError` — mêmes gabarits que
    // `TypeError` (dev.to : table d'intrinsèques `%EvalError%`,
    // JS-C56 vague 2).
    let ev_err = ctor(e, "EvalError", eval_error_ctor, e.protos.eval_error);
    let rg_err = ctor(e, "RangeError", range_error_ctor, e.protos.range_error);
    let ur_err = ctor(e, "URIError", uri_error_ctor, e.protos.uri_error);
    for (p, n) in [
        (e.protos.eval_error, "EvalError"),
        (e.protos.range_error, "RangeError"),
        (e.protos.uri_error, "URIError"),
    ] {
        let nv = e.str_val(n);
        e.set_own(p, "name", nv, false);
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
            e.add_method(iid, "concat", iterator_concat);
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

    // `AsyncDisposableStack` (ES2026) — disposeurs awaités au
    // dépile (eager await, déviation documentée).
    {
        let ads_p = e.new_object();
        let ads = ctor(
            e,
            "AsyncDisposableStack",
            async_disposable_stack_ctor,
            ads_p,
        );
        let dg = e.native_fn("get disposed", ds_disposed_get);
        if let Some(o) = e.heap.obj_mut(ads_p) {
            o.set_accessor("disposed", dg, Value::Undefined, false);
        }
        e.add_method(ads_p, "use", ads_use);
        e.add_method(ads_p, "adopt", ads_adopt);
        e.add_method(ads_p, "defer", ads_defer);
        e.add_method(ads_p, "disposeAsync", ads_dispose_async);
        e.add_method(ads_p, "move", ads_move);
        e.add_method(ads_p, crate::interp::SYM_ASYNC_DISPOSE, ads_dispose_async);
        let tag = e.str_val("AsyncDisposableStack");
        e.set_own(ads_p, crate::interp::SYM_TO_STRING_TAG, tag, false);
        e.set_own(g, "AsyncDisposableStack", ads, false);
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
    e.set_own(g, "EvalError", ev_err, false);
    e.set_own(g, "RangeError", rg_err, false);
    e.set_own(g, "URIError", ur_err, false);
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
            // `Symbol.metadata` (ES2026 decorators) — clé de
            // stockage `[Symbol.metadata]` des métadonnées de
            // décorateurs ; exposée même sans syntaxe `@dec` (la
            // spec la crée au démarrage).
            "metadata",
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
    // `mark`/`measure` — instrumentation : mesurées par la télémétrie
    // des pages réelles, jamais relues (getEntries* → `[]` honnêtement
    // vide, les marks ne sont pas stockés — ADR perf : pas de
    // timeline dans ce moteur).
    e.add_method(performance, "mark", performance_noop);
    e.add_method(performance, "measure", performance_noop);
    e.add_method(performance, "clearMarks", performance_noop);
    e.add_method(performance, "clearMeasures", performance_noop);
    e.add_method(performance, "getEntries", performance_empty_entries);
    e.add_method(performance, "getEntriesByName", performance_empty_entries);
    e.add_method(performance, "getEntriesByType", performance_empty_entries);
    // `timeOrigin` — wall-clock de création du moteur (≙
    // navigationStart ; `now()` reste monotique depuis `t0`).
    let origin_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64() * 1000.0);
    e.set_own(performance, "timeOrigin", Value::Num(origin_ms), true);
    // `performance.timing`/`navigation` — objets legacy lus par les
    // pages âgées : toutes les bornes = origine de navigation.
    let timing = e.new_object();
    for k in [
        "navigationStart",
        "fetchStart",
        "domainLookupStart",
        "domainLookupEnd",
        "connectStart",
        "connectEnd",
        "requestStart",
        "responseStart",
        "responseEnd",
        "domLoading",
        "domInteractive",
        "domContentLoadedEventStart",
        "domContentLoadedEventEnd",
        "domComplete",
        "loadEventStart",
        "loadEventEnd",
    ] {
        e.set_own(timing, k, Value::Num(origin_ms), true);
    }
    e.set_own(performance, "timing", Value::Obj(timing), true);
    let navigation = e.new_object();
    e.set_own(navigation, "type", Value::Num(0.0), true);
    e.set_own(navigation, "redirectCount", Value::Num(0.0), true);
    e.set_own(performance, "navigation", Value::Obj(navigation), true);
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
    // `Function.prototype[Symbol.hasInstance]` — `OrdinaryHasInstance`
    // (hook `instanceof` par défaut, spec).
    let hi = e.native_fn("[Symbol.hasInstance]", function_has_instance);
    e.set_own(fp, crate::interp::SYM_HAS_INSTANCE, hi, false);

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
    e.add_method(ap, "splice", array_splice);
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
    // Annex B — alias historiques encore présents sur les vieux sites.
    e.add_method(sp, "trimLeft", string_trim_start);
    e.add_method(sp, "trimRight", string_trim_end);
    e.add_method(sp, "at", string_at);
    e.add_method(sp, "codePointAt", string_code_point_at);
    e.add_method(sp, "matchAll", string_match_all);
    e.add_method(sp, "normalize", string_normalize);
    // `[Symbol.iterator]` — itérateur de code points (trigger
    // réel dev.to : appel direct `str[Symbol.iterator]()`).
    let siter_fn = e.native_fn("[Symbol.iterator]", string_iterator);
    e.set_own(sp, crate::interp::SYM_ITERATOR, siter_fn, false);
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
    e.re_ctor = regexp;
    e.set_own(g, "RegExp", regexp, false);

    // Promise + queueMicrotask (JS-7c).
    crate::promise::install(e);

    // Date minimal : now() static, getTime/valueOf on instances.
    let dp = e.protos.date;
    e.add_method(dp, "getTime", date_get_time);
    e.add_method(dp, "valueOf", date_get_time);
    e.add_method(dp, "setTime", date_set_time);
    e.add_method(dp, "toString", date_to_string);
    e.add_method(dp, "toDateString", date_to_date_string);
    e.add_method(dp, "toTimeString", date_to_time_string);
    e.add_method(dp, "toJSON", date_to_json);
    e.add_method(dp, "toLocaleString", date_to_locale_string);
    e.add_method(dp, "toLocaleDateString", date_to_locale_date);
    e.add_method(dp, "toLocaleTimeString", date_to_locale_time);
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
    // Setters — fuseau unique (UTC) : `setX` ≡ `setUTCX`.
    e.add_method(dp, "setFullYear", date_set_full_year);
    e.add_method(dp, "setUTCFullYear", date_set_full_year);
    e.add_method(dp, "setMonth", date_set_month);
    e.add_method(dp, "setUTCMonth", date_set_month);
    e.add_method(dp, "setDate", date_set_date);
    e.add_method(dp, "setUTCDate", date_set_date);
    e.add_method(dp, "setHours", date_set_hours);
    e.add_method(dp, "setUTCHours", date_set_hours);
    e.add_method(dp, "setMinutes", date_set_minutes);
    e.add_method(dp, "setUTCMinutes", date_set_minutes);
    e.add_method(dp, "setSeconds", date_set_seconds);
    e.add_method(dp, "setUTCSeconds", date_set_seconds);
    e.add_method(dp, "setMilliseconds", date_set_ms);
    e.add_method(dp, "setUTCMilliseconds", date_set_ms);
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
    e.add_method(math, "sumPrecise", math_sum_precise);
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

/// Variante de [`put_elems`] pour les *mutators* (`pop`, `sort`,
/// `splice`, `fill`…) : la spec les fait passer par `Set`/`Delete`
/// en mode *throw* — toute violation d'intégrité lève `TypeError`
/// même en sloppy. `orig` est le snapshot pris avant mutation :
/// - `frozen` : le moindre changement (valeur *ou* longueur) lève ;
/// - `sealed` : longueur figée (les éléments restent writable —
///   `sort`/`fill` passent, `pop`/`push`/`splice` lévent).
fn put_elems_guarded(e: &mut Engine, id: ObjId, elems: Vec<Value>, orig: &[Value]) -> JsResult<()> {
    let integ = e.heap.obj(id).map_or(0, |o| o.elem_integrity);
    if integ & 3 != 0 && orig.len() != elems.len() {
        return Err(e.type_error("cannot change length of a sealed array"));
    }
    put_elems(e, id, elems);
    Ok(())
}

fn arr_of(e: &mut Engine, elems: Vec<Value>) -> Value {
    e.new_array(elems)
}

/// Longueur en code units UTF-16 (spec `String.length`) — les
/// astraux comptent pour 2 (surrogate pairs). Nos strings sont
/// UTF-8 ; cette vue lazy est la base de la sémantique JS.
fn u16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// Vue code units UTF-16 de `s` (allouée à la demande).
fn u16_view(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// `u16s` → `String` : les surrogates orphelins (tranche qui coupe
/// une paire, `fromCharCode(0xD800)`) deviennent `U+FFFD` —
/// déviation documentée : nos strings UTF-8 ne peuvent pas porter
/// de surrogate isolé.
fn u16_to_string(u: &[u16]) -> String {
    String::from_utf16_lossy(u)
}

/// Le code unit à l'indice `i` sous forme de `String` (spec
/// `s[i]`/`charAt`) — BMP → le caractère ; surrogate → `U+FFFD`.
fn u16_at(s: &str, i: usize) -> Option<String> {
    s.encode_utf16().nth(i).map(|u| u16_to_string(&[u]))
}

fn chars_of(s: &str) -> Vec<char> {
    s.chars().collect()
}

fn now_millis() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_millis() as f64)
}

// ------------------------------------------------------------------
// JS-C40 — découpage par domaine (pur reformatage mécanique) :
// chaque fichier héberge une section, `install` ci-dessus les
// référence via les re-exports glob — aucune sémantique changée.
// ------------------------------------------------------------------

pub(crate) mod array;
pub(crate) mod bigint;
pub(crate) mod c17;
pub(crate) mod c18;
pub(crate) mod clone;
pub(crate) mod ctors;
pub(crate) mod date;
pub(crate) mod disposable;
pub(crate) mod eval_misc;
pub(crate) mod extras;
pub(crate) mod function;
pub(crate) mod globals;
pub(crate) mod math;
pub(crate) mod number;
pub(crate) mod object;
pub(crate) mod species;
pub(crate) mod string;
pub(crate) mod url;

pub(crate) use array::*;
pub(crate) use bigint::*;
pub(crate) use c17::*;
pub(crate) use c18::*;
pub(crate) use clone::*;
pub(crate) use ctors::*;
pub(crate) use date::*;
pub(crate) use disposable::*;
pub(crate) use eval_misc::*;
pub(crate) use extras::*;
pub(crate) use function::*;
pub(crate) use globals::*;
pub(crate) use math::*;
pub(crate) use number::*;
pub(crate) use object::*;
pub(crate) use species::*;
pub(crate) use string::*;
pub(crate) use url::*;
