// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `ArrayBuffer`, `DataView` et les vues `%TypedArray%` (JS-C6) :
//! buffers d'octets fixes partagés entre vues, conversions par
//! [`TaKind`] (little-endian pour les vues typées, endianness
//! paramétrable pour `DataView`).
//!
//! Les indices canoniques, `length`/`byteLength`/`byteOffset`/
//! `buffer` et l'énumération des indices sont gérés côté
//! `interp.rs` (`find_own`/`set_prop`/`delete_prop`/`own_keys`/
//! `iter_values`). Déviations documentées : ces props sont des data
//! props propres (spec : accesseurs sur le prototype) et il n'y a
//! ni `BigInt64Array`/`BigUint64Array` ni `Atomics`/`resizable`/
//! `SharedArrayBuffer`.

use crate::heap::{JsResult, ObjKind, Object, Prop, TaKind};
use crate::interp::SYM_ITERATOR;
use crate::value::{ObjId, Value};
use crate::Engine;

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).copied().unwrap_or(Value::Undefined)
}

fn c_id(v: Value) -> ObjId {
    match v {
        Value::Obj(id) => id,
        _ => ObjId::MAX,
    }
}

/// `this` doit être une view typée → `(ta, buffer_id, offset, len)`.
fn ta_view(e: &mut Engine, this: Value) -> JsResult<(TaKind, ObjId, usize, usize)> {
    let Value::Obj(id) = this else {
        return Err(e.type_error("not a typed array"));
    };
    match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::TypedArray {
            ta,
            buffer: Value::Obj(bid),
            offset,
            len,
        }) => Ok((*ta, *bid, *offset, *len)),
        _ => Err(e.type_error("not a typed array")),
    }
}

/// `this` doit être un `DataView` → `(buffer_id, offset, len)`.
fn dv_view(e: &mut Engine, this: Value) -> JsResult<(ObjId, usize, usize)> {
    let Value::Obj(id) = this else {
        return Err(e.type_error("not a DataView"));
    };
    match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::DataView {
            buffer: Value::Obj(bid),
            offset,
            len,
        }) => Ok((*bid, *offset, *len)),
        _ => Err(e.type_error("not a DataView")),
    }
}

/// `this` doit être un `ArrayBuffer` → son `ObjId`.
fn ab_this(e: &mut Engine, this: Value) -> JsResult<ObjId> {
    let Value::Obj(id) = this else {
        return Err(e.type_error("not an ArrayBuffer"));
    };
    match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::ByteBuf(_)) => Ok(id),
        _ => Err(e.type_error("not an ArrayBuffer")),
    }
}

/// New `ByteBuf` object of `n` zeroed bytes — les flags spec
/// `resizable`/`maxByteLength`/`detached` sont portés par des
/// props propres non énumérables.
fn new_buffer(e: &mut Engine, n: usize) -> JsResult<ObjId> {
    // Garde d'alloc avant `vec![0; n]` : sans elle un `n` hostile
    // avorte le processus dans l'allocateur (JS-C55).
    e.alloc_guard(n)?;
    let proto = Value::Obj(e.protos.array_buffer);
    let id = e.heap.alloc_obj(Object {
        proto,
        props: Vec::new(),
        kind: ObjKind::ByteBuf(vec![0; n]),
        extensible: true,
        elem_integrity: 0,
        args_map: None,
    });
    mark_buffer(e, id, false, n);
    Ok(id)
}

/// Installe les props `resizable`/`maxByteLength`/`detached`
/// (non énumérables) d'un `ArrayBuffer`.
fn mark_buffer(e: &mut Engine, id: ObjId, resizable: bool, max: usize) {
    e.set_own(id, "resizable", Value::Bool(resizable), false);
    e.set_own(id, "maxByteLength", Value::Num(max as f64), false);
    e.set_own(id, "detached", Value::Bool(false), false);
}

/// `buf.detached === true` — lu sur la prop propre (posée par
/// `transfer`, déviation : prop mutable ordinaire).
fn is_detached(e: &Engine, id: ObjId) -> bool {
    matches!(
        e.heap.obj(id).and_then(|o| {
            o.props
                .iter()
                .find(|(k, _)| k == "detached")
                .map(|(_, p)| p)
        }),
        Some(Prop::Data {
            value: Value::Bool(true),
            ..
        })
    )
}

/// New typed view `ta` over `bid` (`offset` bytes, `len` éléments),
/// proto = le proto du ctor correspondant.
pub(crate) fn new_ta(e: &mut Engine, ta: TaKind, bid: ObjId, offset: usize, len: usize) -> Value {
    let proto = match e.get_prop(e.global, ta.name()).and_then(|c| match c {
        Value::Obj(cid) => e.get_prop(cid, "prototype"),
        _ => None,
    }) {
        Some(p) => p,
        None => Value::Obj(e.protos.typed_array),
    };
    Value::Obj(e.heap.alloc_obj(Object {
        proto,
        props: Vec::new(),
        kind: ObjKind::TypedArray {
            ta,
            buffer: Value::Obj(bid),
            offset,
            len,
        },
        extensible: true,
        elem_integrity: 0,
        args_map: None,
    }))
}

/// New `ArrayBuffer` possédant `bytes` — résultat de
/// `crypto.subtle.digest` (JS-C79 : la promesse résout un
/// `ArrayBuffer` brut, pas une vue).
pub(crate) fn new_array_buffer(e: &mut Engine, bytes: Vec<u8>) -> Value {
    let len = bytes.len();
    let bid = e.heap.alloc_obj(Object {
        proto: Value::Obj(e.protos.array_buffer),
        props: Vec::new(),
        kind: ObjKind::ByteBuf(bytes),
        extensible: true,
        elem_integrity: 0,
        args_map: None,
    });
    mark_buffer(e, bid, false, len);
    Value::Obj(bid)
}

/// New `Uint8Array` over `bytes` (own buffer) — partagé avec
/// `TextEncoder.encode`/`TextDecoder`/`crypto` côté builtins.
pub(crate) fn new_u8_array(e: &mut Engine, bytes: Vec<u8>) -> Value {
    let len = bytes.len();
    let bid = e.heap.alloc_obj(Object {
        proto: Value::Obj(e.protos.array_buffer),
        props: Vec::new(),
        kind: ObjKind::ByteBuf(bytes),
        extensible: true,
        elem_integrity: 0,
        args_map: None,
    });
    mark_buffer(e, bid, false, len);
    new_ta(e, TaKind::U8, bid, 0, len)
}

/// Raw bytes of a buffer-ish value — `ByteBuf`, `DataView`, view
/// typée (la région vue, pas le buffer entier). `None` sinon.
pub(crate) fn bytes_of_value(e: &Engine, v: Value) -> Option<Vec<u8>> {
    let Value::Obj(id) = v else { return None };
    match e.heap.obj(id).map(|o| &o.kind)? {
        ObjKind::ByteBuf(b) => Some(b.clone()),
        ObjKind::DataView {
            buffer: Value::Obj(bid),
            offset,
            len,
        } => buf_region(e, *bid, *offset, *len),
        ObjKind::TypedArray {
            ta,
            buffer: Value::Obj(bid),
            offset,
            len,
        } => buf_region(e, *bid, *offset, *len * ta.size()),
        _ => None,
    }
}

/// `bytes[offset..offset+len]` borné — `None` hors du buffer.
fn buf_region(e: &Engine, bid: ObjId, offset: usize, len: usize) -> Option<Vec<u8>> {
    match e.heap.obj(bid).map(|b| &b.kind)? {
        ObjKind::ByteBuf(bytes) => {
            let end = (offset + len).min(bytes.len());
            bytes.get(offset..end).map(<[u8]>::to_vec)
        }
        _ => None,
    }
}

/// Snapshot des éléments de `this` (`Vec<Value>` — `Num`, ou `Big`
/// pour les kinds big) lu dans la région `offset..offset+len*size`.
fn ta_vals(e: &mut Engine, this: Value) -> JsResult<(TaKind, Vec<Value>)> {
    let (ta, bid, off, len) = ta_view(e, this)?;
    Ok((ta, e.ta_elements(ta, bid, off, len)))
}

/// Alloue un buffer + view `ta` et écrit `vals` (conversions
/// `ToNumber`/`ToBigInt` selon le kind — les échecs se propagent).
fn ta_from_vals(e: &mut Engine, ta: TaKind, vals: &[Value]) -> JsResult<Value> {
    let bid = new_buffer(e, vals.len().saturating_mul(ta.size()))?;
    // Conversion d'abord (le `&mut` du buffer interdit `to_number`).
    let conv = conv_vals(e, ta, vals)?;
    if let Some(b) = e.heap.obj_mut(bid) {
        if let ObjKind::ByteBuf(bytes) = &mut b.kind {
            write_conv(ta, bytes, 0, &conv);
        }
    }
    Ok(new_ta(e, ta, bid, 0, vals.len()))
}

/// Élément converti prêt-à-écrire (conversion faite avant le
/// `&mut` du buffer — voir `ta_from_vals`).
enum Conv {
    N(f64),
    B(num_bigint::BigInt),
}

/// `vals` convertis selon `ta` — `ToBigInt` (propage `TypeError`)
/// pour les kinds big, `ToNumber` sinon.
fn conv_vals(e: &mut Engine, ta: TaKind, vals: &[Value]) -> JsResult<Vec<Conv>> {
    let mut out = Vec::with_capacity(vals.len());
    for v in vals {
        out.push(if ta.is_big() {
            Conv::B(e.to_bigint(*v)?)
        } else {
            // `ToNumber` propage : `new Int8Array([1n])` → TypeError.
            Conv::N(e.to_number(*v)?)
        });
    }
    Ok(out)
}

/// Écrit `conv` dans `bytes` (région de vue déjà extraite).
fn write_conv(ta: TaKind, region: &mut [u8], base: usize, conv: &[Conv]) {
    for (i, c) in conv.iter().enumerate() {
        match c {
            Conv::N(n) => ta.set(region, base + i, *n),
            Conv::B(b) => ta.set_big(region, base + i, b),
        }
    }
}

/// Réécrit `vals` dans la région `voff..` du buffer `bid` (in-place
/// : `reverse`, `sort`, `copyWithin`, `fill`, `set`). Conversion
/// avant le `&mut` du buffer.
fn ta_write_back(
    e: &mut Engine,
    ta: TaKind,
    bid: ObjId,
    voff: usize,
    base: usize,
    vals: &[Value],
) -> JsResult<()> {
    let conv = conv_vals(e, ta, vals)?;
    if let Some(b) = e.heap.obj_mut(bid) {
        if let ObjKind::ByteBuf(bytes) = &mut b.kind {
            if let Some(region) = bytes.get_mut(voff..) {
                write_conv(ta, region, base, &conv);
            }
        }
    }
    Ok(())
}

/// `src` lu comme liste de valeurs (view typée, tableau, iterable
/// ou array-like `{length}`). Les conversions arrivent à
/// l'écriture — `ToBigInt`/`ToNumber` selon le kind cible.
pub(crate) fn src_vals(e: &mut Engine, src: Value) -> JsResult<Vec<Value>> {
    if let Value::Obj(id) = src {
        // Vue typée → copie élément à élément (`Big` préservés).
        let view = match e.heap.obj(id).map(|o| &o.kind) {
            Some(ObjKind::TypedArray {
                ta,
                buffer: Value::Obj(bid),
                offset,
                len,
            }) => Some((*ta, *bid, *offset, *len)),
            _ => None,
        };
        if let Some((ta, bid, off, len)) = view {
            return Ok(e.ta_elements(ta, bid, off, len));
        }
        // Array / iterable / array-like.
        if let Ok(items) = e.iter_values(src) {
            return Ok(items);
        }
        if let Some(Value::Num(n)) = e.get_prop(id, "length") {
            e.alloc_guard((n as usize).saturating_mul(std::mem::size_of::<Value>()))?;
            let mut out = Vec::with_capacity(n as usize);
            for i in 0..(n as usize) {
                e.tick()?;
                out.push(e.get_prop(id, &i.to_string()).unwrap_or(Value::Undefined));
            }
            return Ok(out);
        }
    }
    if let Value::Num(n) = src {
        e.alloc_guard((n as usize).saturating_mul(std::mem::size_of::<Value>()))?;
        return Ok(vec![Value::Num(0.0); if n >= 0.0 { n as usize } else { 0 }]);
    }
    Ok(Vec::new())
}

/// `ToIndex` — entier ≥ 0 borné à `usize` (`RangeError` si négatif,
/// comme les vrais moteurs sur les tailles d'offset).
fn to_index(e: &mut Engine, v: Value) -> JsResult<usize> {
    let n = e.to_number(v)?;
    if n.is_nan() || n < 0.0 || n > usize::MAX as f64 {
        return Err(e.range_error("invalid index"));
    }
    Ok(n as usize)
}

/// Clamp spec `min(max(relative, 0), len)` (`begin`/`end` négatifs
/// comptent depuis la fin).
fn rel_bounds(e: &mut Engine, b: Value, en: Value, len: usize) -> JsResult<(usize, usize)> {
    let li = len as f64;
    let rel = |v: Value, e: &mut Engine| -> JsResult<f64> {
        let n = e.to_number(v)?;
        Ok(if n < 0.0 {
            (li + n).max(0.0)
        } else {
            n.min(li)
        })
    };
    let start = if matches!(b, Value::Undefined) {
        0.0
    } else {
        rel(b, e)?
    };
    let end = if matches!(en, Value::Undefined) {
        li
    } else {
        rel(en, e)?
    };
    Ok((start as usize, end.max(start) as usize))
}

/// Install `ArrayBuffer`, `DataView`, `%TypedArray%` et les 9
/// constructeurs typés sur `e`'s global.
pub(crate) fn install(e: &mut Engine) {
    let g = e.global;

    // -- ArrayBuffer --------------------------------------------------
    let abp = e.protos.array_buffer;
    e.add_method(abp, "slice", ab_slice);
    e.add_method(abp, "resize", ab_resize);
    e.add_method(abp, "transfer", ab_transfer);
    {
        let ab = e.native_fn("ArrayBuffer", ab_ctor);
        let id = c_id(ab);
        e.set_own(id, "prototype", Value::Obj(abp), false);
        e.set_own(abp, "constructor", ab, false);
        e.add_method(id, "isView", ab_is_view);
        e.set_own(g, "ArrayBuffer", ab, false);
    }
    // Accesseurs spec sur le proto — les instances portent les
    // flags en props propres (déviation : `hasOwnProperty` visible)
    // mais `ArrayBuffer.prototype.resizable` doit exister et lever
    // `TypeError` sur `this` non-buffer (spec `get resizable`).
    for (n, f, gname) in [
        (
            "resizable",
            ab_resizable_get as crate::heap::NativeFn,
            "get resizable",
        ),
        (
            "maxByteLength",
            ab_max_get as crate::heap::NativeFn,
            "get maxByteLength",
        ),
        (
            "detached",
            ab_detached_get as crate::heap::NativeFn,
            "get detached",
        ),
    ] {
        let gf = e.native_fn(gname, f);
        if let Some(o) = e.heap.obj_mut(abp) {
            o.set_accessor(n, gf, Value::Undefined, false);
        }
    }

    // -- SharedArrayBuffer / Atomics (JS-C18) -------------------------
    // Moteur mono-thread : le SAB est un `ByteBuf` ordinaire
    // (indiscernable observablement hors `Atomics.wait` qui lève
    // `TypeError` comme sur le main thread navigateur — spec).
    {
        let sabp = e.new_object();
        e.add_method(sabp, "grow", sab_grow);
        {
            let gg = e.native_fn("get growable", sab_growable_get);
            if let Some(o) = e.heap.obj_mut(sabp) {
                o.set_accessor("growable", gg, Value::Undefined, false);
            }
        }
        let tag = e.str_val("SharedArrayBuffer");
        e.set_own(sabp, crate::interp::SYM_TO_STRING_TAG, tag, false);
        let sab = e.native_fn("SharedArrayBuffer", sab_ctor);
        let sid = c_id(sab);
        e.set_own(sid, "prototype", Value::Obj(sabp), false);
        e.set_own(sabp, "constructor", sab, false);
        e.set_own(g, "SharedArrayBuffer", sab, false);
    }

    {
        let at = e.new_object();
        e.add_method(at, "load", atomics_load);
        e.add_method(at, "store", atomics_store);
        e.add_method(at, "add", atomics_add);
        e.add_method(at, "sub", atomics_sub);
        e.add_method(at, "and", atomics_and);
        e.add_method(at, "or", atomics_or);
        e.add_method(at, "xor", atomics_xor);
        e.add_method(at, "exchange", atomics_exchange);
        e.add_method(at, "compareExchange", atomics_compare_exchange);
        e.add_method(at, "isLockFree", atomics_is_lock_free);
        e.add_method(at, "pause", atomics_pause);
        e.add_method(at, "wait", atomics_wait);
        e.add_method(at, "notify", atomics_notify);
        e.add_method(at, "waitAsync", atomics_wait_async);
        let tag = e.str_val("Atomics");
        e.set_own(at, crate::interp::SYM_TO_STRING_TAG, tag, false);
        e.set_own(g, "Atomics", Value::Obj(at), false);
    }

    // -- DataView ------------------------------------------------------
    let dvp = e.protos.data_view;
    for (kind, gname, sname) in [
        (TaKind::I8, "getInt8", "setInt8"),
        (TaKind::U8, "getUint8", "setUint8"),
        (TaKind::I16, "getInt16", "setInt16"),
        (TaKind::U16, "getUint16", "setUint16"),
        (TaKind::I32, "getInt32", "setInt32"),
        (TaKind::U32, "getUint32", "setUint32"),
        (TaKind::F32, "getFloat32", "setFloat32"),
        (TaKind::F64, "getFloat64", "setFloat64"),
        (TaKind::I64, "getBigInt64", "setBigInt64"),
        (TaKind::U64, "getBigUint64", "setBigUint64"),
        (TaKind::F16, "getFloat16", "setFloat16"),
    ] {
        let gv = e.bound_native(gname, dv_get, vec![Value::Num(kind.index() as f64)]);
        e.set_own(dvp, gname, gv, false);
        let sv = e.bound_native(sname, dv_set, vec![Value::Num(kind.index() as f64)]);
        e.set_own(dvp, sname, sv, false);
    }
    {
        let dv = e.native_fn("DataView", dv_ctor);
        let id = c_id(dv);
        e.set_own(id, "prototype", Value::Obj(dvp), false);
        e.set_own(dvp, "constructor", dv, false);
        e.set_own(g, "DataView", dv, false);
    }

    // -- %TypedArray% proto partagé -----------------------------------
    let tap = e.protos.typed_array;
    e.add_method(tap, "set", ta_set_m);
    e.add_method(tap, "subarray", ta_subarray);
    e.add_method(tap, "fill", ta_fill);
    e.add_method(tap, "slice", ta_slice);
    e.add_method(tap, "map", ta_map);
    e.add_method(tap, "filter", ta_filter);
    e.add_method(tap, "forEach", ta_for_each);
    e.add_method(tap, "join", ta_join);
    e.add_method(tap, "indexOf", ta_index_of);
    e.add_method(tap, "lastIndexOf", ta_last_index_of);
    e.add_method(tap, "includes", ta_includes);
    e.add_method(tap, "every", ta_every);
    e.add_method(tap, "some", ta_some);
    e.add_method(tap, "find", ta_find);
    e.add_method(tap, "findIndex", ta_find_index);
    e.add_method(tap, "at", ta_at);
    e.add_method(tap, "reduce", ta_reduce);
    e.add_method(tap, "reduceRight", ta_reduce_right);
    e.add_method(tap, "reverse", ta_reverse);
    e.add_method(tap, "sort", ta_sort);
    e.add_method(tap, "copyWithin", ta_copy_within);
    e.add_method(tap, "keys", ta_keys);
    e.add_method(tap, "values", ta_values);
    e.add_method(tap, "entries", ta_entries);
    e.add_method(tap, "toString", ta_join);
    {
        let it = e.native_fn("values", ta_values);
        e.set_own(tap, SYM_ITERATOR, it, false);
    }

    // -- les 9 constructeurs typés ------------------------------------
    for ta in TaKind::ALL {
        let proto = e.heap.alloc_obj(Object::ordinary(Value::Obj(tap)));
        e.set_const(proto, "BYTES_PER_ELEMENT", Value::Num(ta.size() as f64));
        let ctor = e.bound_native(ta.name(), ta_ctor, vec![Value::Num(ta.index() as f64)]);
        let cid = c_id(ctor);
        e.set_own(cid, "prototype", Value::Obj(proto), false);
        e.set_own(proto, "constructor", ctor, false);
        e.set_const(cid, "BYTES_PER_ELEMENT", Value::Num(ta.size() as f64));
        let from = e.bound_native("from", ta_from, vec![Value::Num(ta.index() as f64)]);
        e.set_own(cid, "from", from, false);
        let of = e.bound_native("of", ta_of, vec![Value::Num(ta.index() as f64)]);
        e.set_own(cid, "of", of, false);
        e.set_own(g, ta.name(), ctor, false);
        // ES2025 `Uint8Array` — `fromBase64`/`fromHex` statiques et
        // `toBase64`/`toHex`/`setFromBase64`/`setFromHex` proto.
        if matches!(ta, TaKind::U8) {
            let fb = e.native_fn("fromBase64", u8_from_base64);
            e.set_own(cid, "fromBase64", fb, false);
            let fh = e.native_fn("fromHex", u8_from_hex);
            e.set_own(cid, "fromHex", fh, false);
            e.add_method(proto, "toBase64", u8_to_base64);
            e.add_method(proto, "toHex", u8_to_hex);
            e.add_method(proto, "setFromBase64", u8_set_from_base64);
            e.add_method(proto, "setFromHex", u8_set_from_hex);
        }
    }
}

// -- ArrayBuffer --------------------------------------------------------

/// `new ArrayBuffer(len, {maxByteLength?})` — `len` bytes,
/// zero-filled. `maxByteLength` ≥ len rend le buffer `resizable`.
fn ab_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    // Spec : arg absent → `ToIndex(undefined)` = 0 (`new
    // ArrayBuffer()` est un buffer vide — `to_index` pur lève
    // RangeError sur NaN, il faut court-circuiter l'absence).
    let n = match arg(args, 0) {
        Value::Undefined => 0,
        v => to_index(e, v)?,
    };
    let id = new_buffer(e, n)?;
    if let Value::Obj(opts) = arg(args, 1) {
        if let Some(mv) = e.get_prop(opts, "maxByteLength") {
            if !matches!(mv, Value::Undefined) {
                let max = to_index(e, mv)?;
                if max < n {
                    return Err(e.range_error("maxByteLength smaller than length"));
                }
                e.set_own(id, "resizable", Value::Bool(true), false);
                e.set_own(id, "maxByteLength", Value::Num(max as f64), false);
            }
        }
    }
    Ok(Value::Obj(id))
}

/// `buf.resize(newLen)` — ES2024, buffers `resizable` only.
fn ab_resize(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = ab_this(e, this)?;
    if is_detached(e, id) {
        return Err(e.type_error("detached ArrayBuffer"));
    }
    let resizable = matches!(e.get_prop(id, "resizable"), Some(Value::Bool(true)));
    if !resizable {
        return Err(e.type_error("ArrayBuffer is not resizable"));
    }
    let max = match e.get_prop(id, "maxByteLength") {
        Some(Value::Num(n)) => n as usize,
        _ => usize::MAX,
    };
    let n = to_index(e, arg(args, 0))?;
    if n > max {
        return Err(e.range_error("new length exceeds maxByteLength"));
    }
    e.alloc_guard(n)?;
    if let Some(b) = e.heap.obj_mut(id) {
        if let ObjKind::ByteBuf(bytes) = &mut b.kind {
            bytes.resize(n, 0);
        }
    }
    Ok(Value::Undefined)
}

/// `buf.transfer(len?)` — ES2024 : copie les données dans un
/// nouveau buffer et détache l'ancien (`detached` + vidé).
fn ab_transfer(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = ab_this(e, this)?;
    if is_detached(e, id) {
        return Err(e.type_error("detached ArrayBuffer"));
    }
    // Props lues d'abord (`get_prop` prend `&mut`).
    let (max_p, res_p) = (e.get_prop(id, "maxByteLength"), e.get_prop(id, "resizable"));
    let bytes = match e.heap.obj(id).map(|b| &b.kind) {
        Some(ObjKind::ByteBuf(b)) => b.clone(),
        _ => Vec::new(),
    };
    let resizable = matches!(res_p, Some(Value::Bool(true)));
    let max = match max_p {
        Some(Value::Num(n)) => n as usize,
        _ => bytes.len(),
    };
    let n = if matches!(arg(args, 0), Value::Undefined) {
        bytes.len()
    } else {
        to_index(e, arg(args, 0))?
    };
    let nid = new_buffer(e, n.min(bytes.len()))?;
    if let Some(b) = e.heap.obj_mut(nid) {
        if let ObjKind::ByteBuf(nb) = &mut b.kind {
            *nb = bytes[..n.min(bytes.len())].to_vec();
        }
    }
    e.set_own(nid, "resizable", Value::Bool(resizable), false);
    e.set_own(nid, "maxByteLength", Value::Num(max as f64), false);
    if let Some(b) = e.heap.obj_mut(id) {
        if let ObjKind::ByteBuf(nb) = &mut b.kind {
            nb.clear();
        }
    }
    e.set_own(id, "detached", Value::Bool(true), false);
    Ok(Value::Obj(nid))
}

/// `buf.slice(begin?, end?)` — copie dans un nouveau buffer.
fn ab_slice(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = ab_this(e, this)?;
    if is_detached(e, id) {
        return Err(e.type_error("detached ArrayBuffer"));
    }
    let len = match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::ByteBuf(b)) => b.len(),
        _ => 0,
    };
    let (s, en) = rel_bounds(e, arg(args, 0), arg(args, 1), len)?;
    let data = match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::ByteBuf(b)) => b[s..en].to_vec(),
        _ => Vec::new(),
    };
    let out = new_buffer(e, data.len())?;
    if let Some(o) = e.heap.obj_mut(out) {
        if let ObjKind::ByteBuf(b) = &mut o.kind {
            *b = data;
        }
    }
    Ok(Value::Obj(out))
}

/// `ArrayBuffer.isView(v)` — `true` pour `DataView`/vues typées.
fn ab_is_view(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let ok = match arg(args, 0) {
        Value::Obj(id) => matches!(
            e.heap.obj(id).map(|o| &o.kind),
            Some(ObjKind::TypedArray { .. } | ObjKind::DataView { .. })
        ),
        _ => false,
    };
    Ok(Value::Bool(ok))
}

// -- DataView ------------------------------------------------------------

/// `new DataView(buffer, byteOffset?, byteLength?)`.
fn dv_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(bid) = arg(args, 0) else {
        return Err(e.type_error("DataView expects an ArrayBuffer"));
    };
    let blen = match e.heap.obj(bid).map(|o| &o.kind) {
        Some(ObjKind::ByteBuf(b)) => b.len(),
        _ => return Err(e.type_error("DataView expects an ArrayBuffer")),
    };
    let off = if matches!(arg(args, 1), Value::Undefined) {
        0
    } else {
        to_index(e, arg(args, 1))?
    };
    if off > blen {
        return Err(e.range_error("DataView offset out of range"));
    }
    let len = if matches!(arg(args, 2), Value::Undefined) {
        blen - off
    } else {
        let n = to_index(e, arg(args, 2))?;
        if off + n > blen {
            return Err(e.range_error("DataView length out of range"));
        }
        n
    };
    let proto = Value::Obj(e.protos.data_view);
    Ok(Value::Obj(e.heap.alloc_obj(Object {
        proto,
        props: Vec::new(),
        kind: ObjKind::DataView {
            buffer: Value::Obj(bid),
            offset: off,
            len,
        },
        extensible: true,
        elem_integrity: 0,
        args_map: None,
    })))
}

/// Read `kind` at byte offset `off` (`le` → little-endian, spec
/// default big-endian).
fn dv_num(kind: TaKind, bytes: &[u8], off: usize, le: bool) -> Option<f64> {
    let b = bytes.get(off..off + kind.size())?;
    let v = match kind {
        TaKind::I8 => b[0] as i8 as f64,
        TaKind::U8 | TaKind::U8C => f64::from(b[0]),
        TaKind::I16 => {
            let a = [b[0], b[1]];
            (if le {
                i16::from_le_bytes(a)
            } else {
                i16::from_be_bytes(a)
            }) as f64
        }
        TaKind::U16 => {
            let a = [b[0], b[1]];
            (if le {
                u16::from_le_bytes(a)
            } else {
                u16::from_be_bytes(a)
            }) as f64
        }
        TaKind::I32 => {
            let a = [b[0], b[1], b[2], b[3]];
            (if le {
                i32::from_le_bytes(a)
            } else {
                i32::from_be_bytes(a)
            }) as f64
        }
        TaKind::U32 => {
            let a = [b[0], b[1], b[2], b[3]];
            (if le {
                u32::from_le_bytes(a)
            } else {
                u32::from_be_bytes(a)
            }) as f64
        }
        TaKind::F32 => {
            let a = [b[0], b[1], b[2], b[3]];
            (if le {
                f32::from_le_bytes(a)
            } else {
                f32::from_be_bytes(a)
            }) as f64
        }
        TaKind::F64 => {
            let a = [b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]];
            if le {
                f64::from_le_bytes(a)
            } else {
                f64::from_be_bytes(a)
            }
        }
        TaKind::F16 => {
            let a = [b[0], b[1]];
            crate::heap::f16_to_f64(if le {
                u16::from_le_bytes(a)
            } else {
                u16::from_be_bytes(a)
            })
        }
        // Les accesseurs `getBigInt64`/`getBigUint64` ne passent pas
        // par `dv_num` — voir `dv_get` (retour `Value::Big`).
        TaKind::I64 => {
            let a = [b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]];
            (if le {
                i64::from_le_bytes(a)
            } else {
                i64::from_be_bytes(a)
            }) as f64
        }
        TaKind::U64 => {
            let a = [b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]];
            (if le {
                u64::from_le_bytes(a)
            } else {
                u64::from_be_bytes(a)
            }) as f64
        }
    };
    Some(v)
}

/// `dv.getXxx(byteOffset, littleEndian?)` — bound arg = kind idx.
/// `RangeError` si la lecture déborde de la vue (spec `GetViewValue`).
fn dv_get(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let kind = TaKind::from_index(match arg(args, 0) {
        Value::Num(n) => n as usize,
        _ => 0,
    })
    .unwrap_or(TaKind::F64);
    let (bid, base, vlen) = dv_view(e, this)?;
    let off = to_index(e, arg(args, 1))?;
    let le = e.to_boolean(arg(args, 2));
    if off + kind.size() > vlen {
        return Err(e.range_error("offset out of DataView bounds"));
    }
    let bytes = match e.heap.obj(bid).map(|b| &b.kind) {
        Some(ObjKind::ByteBuf(b)) => b.clone(),
        _ => return Err(e.type_error("detached DataView")),
    };
    // Kinds big → `Value::Big` (spec `getBigInt64`/`getBigUint64`).
    if kind.is_big() {
        let Some(s) = bytes.get(base + off..base + off + 8) else {
            return Err(e.range_error("offset out of DataView bounds"));
        };
        let a: [u8; 8] = <[u8; 8]>::try_from(s).unwrap_or([0; 8]);
        let bi = match kind {
            TaKind::U64 => num_bigint::BigInt::from(if le {
                u64::from_le_bytes(a)
            } else {
                u64::from_be_bytes(a)
            }),
            _ => num_bigint::BigInt::from(if le {
                i64::from_le_bytes(a)
            } else {
                i64::from_be_bytes(a)
            }),
        };
        return Ok(e.big_val(bi));
    }
    match dv_num(kind, &bytes, base + off, le) {
        Some(n) => Ok(Value::Num(n)),
        None => Err(e.range_error("offset out of DataView bounds")),
    }
}

/// `dv.setXxx(byteOffset, value, littleEndian?)` — bound arg = kind.
fn dv_set(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let kind = TaKind::from_index(match arg(args, 0) {
        Value::Num(n) => n as usize,
        _ => 0,
    })
    .unwrap_or(TaKind::F64);
    let (bid, base, vlen) = dv_view(e, this)?;
    let off = to_index(e, arg(args, 1))?;
    let le = e.to_boolean(arg(args, 3));
    if off + kind.size() > vlen {
        return Err(e.range_error("offset out of DataView bounds"));
    }
    let o = base + off;
    // Kinds big — `ToBigInt` puis `v mod 2^64`, endianness au choix
    // (spec `setBigInt64`/`setBigUint64`).
    if kind.is_big() {
        use num_traits::ToPrimitive;
        let b = e.to_bigint(arg(args, 2))?;
        let m = num_bigint::BigInt::from(1) << 64u32;
        let mut r = &b % &m;
        if r.sign() == num_bigint::Sign::Minus {
            r += &m;
        }
        let bits = r.to_u64().unwrap_or(0);
        let arr = if le {
            bits.to_le_bytes()
        } else {
            bits.to_be_bytes()
        };
        let Some(bo) = e.heap.obj_mut(bid) else {
            return Err(e.type_error("detached DataView"));
        };
        let ObjKind::ByteBuf(bytes) = &mut bo.kind else {
            return Err(e.type_error("detached DataView"));
        };
        if bytes.len() < o + 8 {
            return Err(e.range_error("offset out of DataView bounds"));
        }
        bytes[o..o + 8].copy_from_slice(&arr);
        return Ok(Value::Undefined);
    }
    let v = e.to_number(arg(args, 2))?;
    let Some(b) = e.heap.obj_mut(bid) else {
        return Err(e.type_error("detached DataView"));
    };
    let ObjKind::ByteBuf(bytes) = &mut b.kind else {
        return Err(e.type_error("detached DataView"));
    };
    if o + kind.size() > bytes.len() {
        return Err(e.range_error("offset out of DataView bounds"));
    }
    match kind {
        TaKind::I8 => bytes[o] = (crate::value::to_int32(v) as i8) as u8,
        TaKind::U8 | TaKind::U8C => bytes[o] = crate::value::to_uint32(v) as u8,
        _ => {
            let n = o + kind.size();
            match kind {
                TaKind::I16 => {
                    let a = crate::value::to_int32(v) as i16;
                    let b = if le { a.to_le_bytes() } else { a.to_be_bytes() };
                    bytes[o..n].copy_from_slice(&b);
                }
                TaKind::U16 => {
                    let a = crate::value::to_uint32(v) as u16;
                    let b = if le { a.to_le_bytes() } else { a.to_be_bytes() };
                    bytes[o..n].copy_from_slice(&b);
                }
                TaKind::I32 => {
                    let a = crate::value::to_int32(v);
                    let b = if le { a.to_le_bytes() } else { a.to_be_bytes() };
                    bytes[o..n].copy_from_slice(&b);
                }
                TaKind::U32 => {
                    let a = crate::value::to_uint32(v);
                    let b = if le { a.to_le_bytes() } else { a.to_be_bytes() };
                    bytes[o..n].copy_from_slice(&b);
                }
                TaKind::F32 => {
                    let a = v as f32;
                    let b = if le { a.to_le_bytes() } else { a.to_be_bytes() };
                    bytes[o..n].copy_from_slice(&b);
                }
                TaKind::F16 => {
                    let a = crate::heap::f64_to_f16(v);
                    let b = if le { a.to_le_bytes() } else { a.to_be_bytes() };
                    bytes[o..n].copy_from_slice(&b);
                }
                _ => {
                    let b = if le { v.to_le_bytes() } else { v.to_be_bytes() };
                    bytes[o..n].copy_from_slice(&b);
                }
            }
        }
    }
    Ok(Value::Undefined)
}

// -- constructeur %TypedArray% ------------------------------------------

/// `new XxxArray(len | iterable | buffer, byteOffset?, length?)` —
/// bound arg = `TaKind::index`.
fn ta_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let ta = TaKind::from_index(match arg(args, 0) {
        Value::Num(n) => n as usize,
        _ => 0,
    })
    .unwrap_or(TaKind::U8);
    let size = ta.size();
    match arg(args, 1) {
        // `new TA(n)` — nouveau buffer de n éléments.
        Value::Num(n) => {
            let len = if n >= 0.0 && n <= usize::MAX as f64 {
                n as usize
            } else {
                return Err(e.range_error("invalid typed array length"));
            };
            let bid = new_buffer(e, len.saturating_mul(size))?;
            Ok(new_ta(e, ta, bid, 0, len))
        }
        // `new TA(buffer, byteOffset?, length?)` — vue sur buffer.
        Value::Obj(id) if matches!(e.heap.obj(id).map(|o| &o.kind), Some(ObjKind::ByteBuf(_))) => {
            let blen = match e.heap.obj(id).map(|o| &o.kind) {
                Some(ObjKind::ByteBuf(b)) => b.len(),
                _ => 0,
            };
            let off = if matches!(arg(args, 2), Value::Undefined) {
                0
            } else {
                to_index(e, arg(args, 2))?
            };
            if off % size != 0 || off > blen {
                return Err(e.range_error("byteOffset out of bounds"));
            }
            let len = if matches!(arg(args, 3), Value::Undefined) {
                if (blen - off) % size != 0 {
                    return Err(e.range_error("buffer length not a multiple of element size"));
                }
                (blen - off) / size
            } else {
                let n = to_index(e, arg(args, 3))?;
                if off + n * size > blen {
                    return Err(e.range_error("typed array length out of bounds"));
                }
                n
            };
            Ok(new_ta(e, ta, id, off, len))
        }
        // `new TA(src)` — iterable / array-like / autre view.
        Value::Obj(_) => {
            let vals = src_vals(e, arg(args, 1))?;
            ta_from_vals(e, ta, &vals)
        }
        // `new TA()` / primitifs non-numériques → vide.
        _ => ta_from_vals(e, ta, &[]),
    }
}

/// `XxxArray.from(src, mapfn?, thisArg?)` — bound arg = kind.
fn ta_from(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let ta = TaKind::from_index(match arg(args, 0) {
        Value::Num(n) => n as usize,
        _ => 0,
    })
    .unwrap_or(TaKind::U8);
    let mut vals = src_vals(e, arg(args, 1))?;
    if let Value::Obj(_) = arg(args, 2) {
        let f = arg(args, 2);
        if !e.is_callable(f) {
            return Err(e.type_error("mapfn is not callable"));
        }
        let this_arg = arg(args, 3);
        for (i, v) in vals.iter_mut().enumerate() {
            // Le résultat du mapfn est stocké brut — la conversion
            // `ToNumber`/`ToBigInt` se fait dans `ta_from_vals`.
            *v = e.call_value(f, this_arg, &[*v, Value::Num(i as f64)])?;
        }
    }
    ta_from_vals(e, ta, &vals)
}

/// `XxxArray.of(...items)` — bound arg = kind.
fn ta_of(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let ta = TaKind::from_index(match arg(args, 0) {
        Value::Num(n) => n as usize,
        _ => 0,
    })
    .unwrap_or(TaKind::U8);
    let vals: Vec<Value> = args[1..].to_vec();
    ta_from_vals(e, ta, &vals)
}

// -- méthodes %TypedArray% -----------------------------------------------

/// `ta.set(src, offset?)` — copie convertie dans la région de vue
/// du buffer. `TypeError` si `src` n'est pas un objet iterable /
/// array-like (spec : `ToObject` sur la source).
fn ta_set_m(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (ta, bid, voff, len) = ta_view(e, this)?;
    if !matches!(arg(args, 0), Value::Obj(_)) {
        return Err(e.type_error("set source is not an object"));
    }
    let vals = src_vals(e, arg(args, 0))?;
    let off = if matches!(arg(args, 1), Value::Undefined) {
        0
    } else {
        to_index(e, arg(args, 1))?
    };
    if off + vals.len() > len {
        return Err(e.range_error("source too large for target"));
    }
    // Conversion avant le `&mut` du buffer — `ToBigInt` peut lever
    // (`set` d'un Number dans une vue big → TypeError, spec).
    let conv = conv_vals(e, ta, &vals)?;
    let Some(b) = e.heap.obj_mut(bid) else {
        return Err(e.type_error("detached typed array"));
    };
    let ObjKind::ByteBuf(bytes) = &mut b.kind else {
        return Err(e.type_error("detached typed array"));
    };
    if let Some(region) = bytes.get_mut(voff..) {
        write_conv(ta, region, off, &conv);
    }
    Ok(Value::Undefined)
}

/// `ta.subarray(begin?, end?)` — nouvelle vue sur le même buffer.
fn ta_subarray(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = this else {
        return Err(e.type_error("not a typed array"));
    };
    let (ta, bid, off, len) = ta_view(e, this)?;
    let (s, en) = rel_bounds(e, arg(args, 0), arg(args, 1), len)?;
    let proto = e.heap.obj(id).map_or(Value::Null, |o| o.proto);
    Ok(Value::Obj(e.heap.alloc_obj(Object {
        proto,
        props: Vec::new(),
        kind: ObjKind::TypedArray {
            ta,
            buffer: Value::Obj(bid),
            offset: off + s * ta.size(),
            len: en - s,
        },
        extensible: true,
        elem_integrity: 0,
        args_map: None,
    })))
}

/// `ta.fill(v, start?, end?)`.
fn ta_fill(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (ta, bid, voff, len) = ta_view(e, this)?;
    let v = arg(args, 0);
    let (s, en) = rel_bounds(e, arg(args, 1), arg(args, 2), len)?;
    ta_write_back(e, ta, bid, voff, s, &vec![v; en - s])?;
    Ok(this)
}

/// `ta.slice(begin?, end?)` — nouvelle view du même kind, copie.
fn ta_slice(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (ta, vals) = ta_vals(e, this)?;
    let (s, en) = rel_bounds(e, arg(args, 0), arg(args, 1), vals.len())?;
    let out = vals[s..en].to_vec();
    ta_from_vals(e, ta, &out)
}

/// `ta.map(fn)` — nouvelle view du même kind.
fn ta_map(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (ta, vals) = ta_vals(e, this)?;
    let f = arg(args, 0);
    if !e.is_callable(f) {
        return Err(e.type_error("callback is not callable"));
    }
    let t = arg(args, 1);
    let mut out = Vec::with_capacity(vals.len());
    for (i, v) in vals.iter().enumerate() {
        out.push(e.call_value(f, t, &[*v, Value::Num(i as f64), this])?);
    }
    ta_from_vals(e, ta, &out)
}

/// `ta.filter(fn)` — nouvelle view du même kind.
fn ta_filter(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (ta, vals) = ta_vals(e, this)?;
    let f = arg(args, 0);
    if !e.is_callable(f) {
        return Err(e.type_error("callback is not callable"));
    }
    let t = arg(args, 1);
    let mut out = Vec::new();
    for (i, v) in vals.iter().enumerate() {
        let r = e.call_value(f, t, &[*v, Value::Num(i as f64), this])?;
        if e.to_boolean(r) {
            out.push(*v);
        }
    }
    ta_from_vals(e, ta, &out)
}

/// `ta.forEach(fn)`.
fn ta_for_each(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (_, vals) = ta_vals(e, this)?;
    let f = arg(args, 0);
    if !e.is_callable(f) {
        return Err(e.type_error("callback is not callable"));
    }
    let t = arg(args, 1);
    for (i, v) in vals.iter().enumerate() {
        e.call_value(f, t, &[*v, Value::Num(i as f64), this])?;
    }
    Ok(Value::Undefined)
}

/// `ta.join(sep?)` — aussi `toString`.
fn ta_join(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (_, vals) = ta_vals(e, this)?;
    let sep = match arg(args, 0) {
        Value::Undefined => ",".to_string(),
        v => e.to_js_string(v)?,
    };
    let mut out = String::new();
    for (i, v) in vals.iter().enumerate() {
        if i > 0 {
            out.push_str(&sep);
        }
        let s = e.to_js_string(*v)?;
        out.push_str(&s);
    }
    Ok(e.str_val(out))
}

/// `ta.indexOf(v, from?)` — `StrictEqual` : pas de conversion de
/// `needle` (`u8.indexOf(1n)` → -1, `big.indexOf(1)` → -1).
fn ta_index_of(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (_, vals) = ta_vals(e, this)?;
    let needle = arg(args, 0);
    let from = match arg(args, 1) {
        Value::Undefined => 0.0,
        v => e.to_number(v)?,
    };
    let start = if from < 0.0 {
        (vals.len() as f64 + from).max(0.0) as usize
    } else {
        from as usize
    };
    for (i, v) in vals.iter().enumerate().skip(start) {
        if e.strict_eq(*v, needle) {
            return Ok(Value::Num(i as f64));
        }
    }
    Ok(Value::Num(-1.0))
}

/// `ta.lastIndexOf(v, from?)`.
fn ta_last_index_of(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (_, vals) = ta_vals(e, this)?;
    let needle = arg(args, 0);
    let from = match arg(args, 1) {
        Value::Undefined => vals.len() as f64 - 1.0,
        v => e.to_number(v)?,
    };
    let start = if from < 0.0 {
        vals.len() as f64 + from
    } else {
        from.min(vals.len() as f64 - 1.0)
    };
    if start >= 0.0 {
        for i in (0..=start as usize).rev() {
            if e.strict_eq(vals[i], needle) {
                return Ok(Value::Num(i as f64));
            }
        }
    }
    Ok(Value::Num(-1.0))
}

/// `ta.includes(v, from?)` — `SameValueZero` : `===` + cas NaN.
fn ta_includes(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (_, vals) = ta_vals(e, this)?;
    let needle = arg(args, 0);
    let from = match arg(args, 1) {
        Value::Undefined => 0.0,
        v => e.to_number(v)?,
    };
    let start = if from < 0.0 {
        (vals.len() as f64 + from).max(0.0) as usize
    } else {
        from as usize
    };
    for v in vals.iter().skip(start) {
        let nan =
            matches!((*v, needle), (Value::Num(a), Value::Num(b)) if a.is_nan() && b.is_nan());
        if nan || e.strict_eq(*v, needle) {
            return Ok(Value::Bool(true));
        }
    }
    Ok(Value::Bool(false))
}

/// `ta.every(fn)` — tous les éléments passent le prédicat.
fn ta_every(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (_, vals) = ta_vals(e, this)?;
    let f = arg(args, 0);
    if !e.is_callable(f) {
        return Err(e.type_error("callback is not callable"));
    }
    let t = arg(args, 1);
    for (i, v) in vals.iter().enumerate() {
        let r = e.call_value(f, t, &[*v, Value::Num(i as f64), this])?;
        if !e.to_boolean(r) {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(true))
}

/// `ta.some(fn)`.
fn ta_some(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (_, vals) = ta_vals(e, this)?;
    let f = arg(args, 0);
    if !e.is_callable(f) {
        return Err(e.type_error("callback is not callable"));
    }
    let t = arg(args, 1);
    for (i, v) in vals.iter().enumerate() {
        let r = e.call_value(f, t, &[*v, Value::Num(i as f64), this])?;
        if e.to_boolean(r) {
            return Ok(Value::Bool(true));
        }
    }
    Ok(Value::Bool(false))
}

/// `ta.find(fn)` — premier élément passant le prédicat.
fn ta_find(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (_, vals) = ta_vals(e, this)?;
    let f = arg(args, 0);
    if !e.is_callable(f) {
        return Err(e.type_error("callback is not callable"));
    }
    let t = arg(args, 1);
    for (i, v) in vals.iter().enumerate() {
        let r = e.call_value(f, t, &[*v, Value::Num(i as f64), this])?;
        if e.to_boolean(r) {
            return Ok(*v);
        }
    }
    Ok(Value::Undefined)
}

/// `ta.findIndex(fn)`.
fn ta_find_index(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (_, vals) = ta_vals(e, this)?;
    let f = arg(args, 0);
    if !e.is_callable(f) {
        return Err(e.type_error("callback is not callable"));
    }
    let t = arg(args, 1);
    for (i, v) in vals.iter().enumerate() {
        let r = e.call_value(f, t, &[*v, Value::Num(i as f64), this])?;
        if e.to_boolean(r) {
            return Ok(Value::Num(i as f64));
        }
    }
    Ok(Value::Num(-1.0))
}

/// `ta.at(i)` — index négatif depuis la fin.
fn ta_at(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (_, vals) = ta_vals(e, this)?;
    let n = e.to_number(arg(args, 0))?;
    let i = if n < 0.0 { vals.len() as f64 + n } else { n };
    if i >= 0.0 && i < vals.len() as f64 {
        Ok(vals[i as usize])
    } else {
        Ok(Value::Undefined)
    }
}

/// `ta.reduce(fn, init?)` — `TypeError` sur vide sans init (spec).
fn ta_reduce(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (_, vals) = ta_vals(e, this)?;
    let f = arg(args, 0);
    if !e.is_callable(f) {
        return Err(e.type_error("callback is not callable"));
    }
    let (mut acc, start) = if matches!(arg(args, 1), Value::Undefined) {
        if vals.is_empty() {
            return Err(e.type_error("reduce of empty typed array"));
        }
        (vals[0], 1)
    } else {
        (arg(args, 1), 0)
    };
    for (i, v) in vals.iter().enumerate().skip(start) {
        acc = e.call_value(f, Value::Undefined, &[acc, *v, Value::Num(i as f64), this])?;
    }
    Ok(acc)
}

/// `ta.reduceRight(fn, init?)`.
fn ta_reduce_right(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (_, vals) = ta_vals(e, this)?;
    let f = arg(args, 0);
    if !e.is_callable(f) {
        return Err(e.type_error("callback is not callable"));
    }
    let (mut acc, start) = if matches!(arg(args, 1), Value::Undefined) {
        if vals.is_empty() {
            return Err(e.type_error("reduce of empty typed array"));
        }
        (vals[vals.len() - 1], vals.len() - 1)
    } else {
        (arg(args, 1), vals.len())
    };
    for i in (0..start).rev() {
        acc = e.call_value(
            f,
            Value::Undefined,
            &[acc, vals[i], Value::Num(i as f64), this],
        )?;
    }
    Ok(acc)
}

/// `ta.reverse()` — in-place, au niveau octet de la région de vue
/// (indifférent au kind, `Big` inclus).
fn ta_reverse(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let _ = args;
    let (ta, bid, voff, len) = ta_view(e, this)?;
    let size = ta.size();
    if let Some(b) = e.heap.obj_mut(bid) {
        if let ObjKind::ByteBuf(bytes) = &mut b.kind {
            if let Some(region) = bytes.get_mut(voff..voff + len * size) {
                for i in 0..len / 2 {
                    let (a, c) = (i * size, (len - 1 - i) * size);
                    for k in 0..size {
                        region.swap(a + k, c + k);
                    }
                }
            }
        }
    }
    Ok(this)
}

/// `ta.sort(cmp?)` — tri *numérique* par défaut (contrairement à
/// `Array.sort` ; les kinds big comparent des `BigInt`),
/// comparateur optionnel. In-place.
fn ta_sort(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (ta, bid, voff, len) = ta_view(e, this)?;
    let cmp = arg(args, 0);
    let mut vals = e.ta_elements(ta, bid, voff, len);
    if matches!(cmp, Value::Undefined) {
        vals.sort_by(|a, b| match (*a, *b) {
            (Value::Num(x), Value::Num(y)) => {
                x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal)
            }
            _ => match (e.bigint_of(*a), e.bigint_of(*b)) {
                (Some(x), Some(y)) => x.cmp(&y),
                _ => std::cmp::Ordering::Equal,
            },
        });
    } else {
        if !e.is_callable(cmp) {
            return Err(e.type_error("comparator is not callable"));
        }
        // Tri par comparateur JS — insertion sort (éléments ≤ 64k).
        for i in 1..vals.len() {
            let mut j = i;
            while j > 0 {
                let r = e.call_value(cmp, Value::Undefined, &[vals[j - 1], vals[j]])?;
                if e.to_number(r).unwrap_or(0.0) > 0.0 {
                    vals.swap(j - 1, j);
                    j -= 1;
                } else {
                    break;
                }
            }
        }
    }
    ta_write_back(e, ta, bid, voff, 0, &vals)?;
    Ok(this)
}

/// `ta.copyWithin(target, start, end?)` — copie d'éléments à
/// l'intérieur de la région de vue — `copy_within` au niveau octet
/// (gère le chevauchement nativement, indifférent au kind).
fn ta_copy_within(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (ta, bid, voff, len) = ta_view(e, this)?;
    let to = e.to_number(arg(args, 0))?;
    let from = match arg(args, 1) {
        Value::Undefined => 0.0,
        v => e.to_number(v)?,
    };
    let end = match arg(args, 2) {
        Value::Undefined => len as f64,
        v => e.to_number(v)?,
    };
    let li = len as f64;
    let rel = |n: f64| if n < 0.0 { (li + n).max(0.0) } else { n.min(li) } as usize;
    let (t, f, en) = (rel(to), rel(from), rel(end));
    let size = ta.size();
    let count = en.saturating_sub(f).min(len.saturating_sub(t));
    if let Some(b) = e.heap.obj_mut(bid) {
        if let ObjKind::ByteBuf(bytes) = &mut b.kind {
            if let Some(region) = bytes.get_mut(voff..voff + len * size) {
                region.copy_within(f * size..(f + count) * size, t * size);
            }
        }
    }
    Ok(this)
}

/// `ta.keys()` — itérateur d'indices.
fn ta_keys(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let (_, _, _, len) = ta_view(e, this)?;
    let items = (0..len).map(|i| Value::Num(i as f64)).collect();
    Ok(crate::collections::iter_obj(e, items, e.protos.array_iter))
}

/// `ta.values()` / `[Symbol.iterator]` — itérateur d'éléments.
fn ta_values(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let (_, vals) = ta_vals(e, this)?;
    Ok(crate::collections::iter_obj(e, vals, e.protos.array_iter))
}

/// `ta.entries()` — itérateur `[i, v]`.
fn ta_entries(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let (_, vals) = ta_vals(e, this)?;
    let mut items = Vec::with_capacity(vals.len());
    for (i, v) in vals.iter().enumerate() {
        items.push(e.new_array(vec![Value::Num(i as f64), *v]));
    }
    Ok(crate::collections::iter_obj(e, items, e.protos.array_iter))
}

// ------------------------------------------------------------------
// JS-C17 — `Uint8Array` base64/hex (ES2025)
// ------------------------------------------------------------------

const B64: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn b64_val(c: u8) -> Option<u8> {
    match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' | b'-' => Some(62),
        b'/' | b'_' => Some(63),
        _ => None,
    }
}

/// `Uint8Array.fromBase64(str)` — alphabet standard + urlsafe
/// (acceptés ensemble, tolérance navigateur), whitespace ignoré.
fn u8_from_base64(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let s = e.to_js_string(arg(args, 0))?;
    match b64_decode(&s) {
        Some(bytes) => Ok(new_u8_array(e, bytes)),
        None => Err(e.syntax_error("invalid base64")),
    }
}

/// `Uint8Array.fromHex(str)` — paires hex strictes, `SyntaxError`
/// sur caractère invalide ou longueur impaire (spec).
fn u8_from_hex(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let s = e.to_js_string(arg(args, 0))?;
    match hex_decode(&s) {
        Some(bytes) => Ok(new_u8_array(e, bytes)),
        None => Err(e.syntax_error("invalid hex string")),
    }
}

fn u8_to_base64(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let bytes = u8_bytes(e, this)?;
    Ok(e.str_val(b64_encode(&bytes)))
}

fn u8_to_hex(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let bytes = u8_bytes(e, this)?;
    Ok(e.str_val(hex_encode(&bytes)))
}

/// `setFromBase64(str)` → `{read, written}` ; `SyntaxError` si
/// invalide, `TypeError` si ça déborde la vue.
fn u8_set_from_base64(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = e.to_js_string(arg(args, 0))?;
    let Some(src) = b64_decode(&s) else {
        return Err(e.syntax_error("invalid base64"));
    };
    u8_set_from(e, this, &src)
}

fn u8_set_from_hex(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let s = e.to_js_string(arg(args, 0))?;
    let Some(src) = hex_decode(&s) else {
        return Err(e.syntax_error("invalid hex string"));
    };
    u8_set_from(e, this, &src)
}

/// Octets de la vue `this` (région `offset..offset+len` — `Uint8`
/// élément = 1 octet).
fn u8_bytes(e: &mut Engine, this: Value) -> JsResult<Vec<u8>> {
    let (ta, bid, off, len) = ta_view(e, this)?;
    if !matches!(ta, TaKind::U8 | TaKind::U8C) {
        return Err(e.type_error("method needs a Uint8Array"));
    }
    match e.heap.obj(bid).map(|b| &b.kind) {
        Some(ObjKind::ByteBuf(b)) => Ok(b[off..off + len].to_vec()),
        _ => Err(e.type_error("detached typed array")),
    }
}

fn u8_set_from(e: &mut Engine, this: Value, src: &[u8]) -> JsResult<Value> {
    let (ta, bid, off, len) = ta_view(e, this)?;
    if !matches!(ta, TaKind::U8 | TaKind::U8C) {
        return Err(e.type_error("method needs a Uint8Array"));
    }
    if src.len() > len {
        return Err(e.type_error("source too large for target view"));
    }
    let read = src.len();
    match e.heap.obj_mut(bid) {
        Some(b) => {
            if let ObjKind::ByteBuf(bytes) = &mut b.kind {
                bytes[off..off + src.len()].copy_from_slice(src);
            }
        }
        None => return Err(e.type_error("detached typed array")),
    }
    let o = e.new_object();
    e.set_own(o, "read", Value::Num(read as f64), true);
    e.set_own(o, "written", Value::Num(read as f64), true);
    Ok(Value::Obj(o))
}

pub(crate) fn b64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let (b0, b1, b2) = (
            c[0],
            c.get(1).copied().unwrap_or(0),
            c.get(2).copied().unwrap_or(0),
        );
        out.push(B64[(b0 >> 2) as usize] as char);
        out.push(B64[((b0 & 3) << 4 | b1 >> 4) as usize] as char);
        out.push(if c.len() > 1 {
            B64[((b1 & 0xf) << 2 | b2 >> 6) as usize] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            B64[(b2 & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    out
}

pub(crate) fn b64_decode(s: &str) -> Option<Vec<u8>> {
    let mut bits = 0u32;
    let mut nbits = 0u32;
    let mut out = Vec::new();
    let mut pad = false;
    for &c in s.as_bytes() {
        match c {
            b' ' | b'\t' | b'\n' | b'\r' => {}
            b'=' => pad = true,
            _ => {
                if pad {
                    return None; // données après `=`
                }
                let v = b64_val(c)?;
                bits = (bits << 6) | u32::from(v);
                nbits += 6;
                if nbits >= 8 {
                    nbits -= 8;
                    out.push((bits >> nbits) as u8);
                    bits &= (1 << nbits) - 1;
                }
            }
        }
    }
    // Bits résiduels non-nuls = troncature invalide.
    if nbits > 0 && bits != 0 {
        return None;
    }
    Some(out)
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    if b.len() % 2 != 0 {
        return None;
    }
    let dig = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let mut out = Vec::with_capacity(b.len() / 2);
    for c in b.chunks_exact(2) {
        match (dig(c[0]), dig(c[1])) {
            (Some(h), Some(l)) => out.push(h * 16 + l),
            _ => return None,
        }
    }
    Some(out)
}

// ------------------------------------------------------------------
// JS-C18 — `SharedArrayBuffer` + `Atomics` (mono-thread)
// ------------------------------------------------------------------

/// `new SharedArrayBuffer(len, {maxByteLength?})` — `ByteBuf` avec
/// `growable`/`maxByteLength`/`detached` en props propres.
fn sab_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let n = match arg(args, 0) {
        Value::Undefined => 0,
        v => to_index(e, v)?,
    };
    let proto = e
        .get_prop(e.global, "SharedArrayBuffer")
        .and_then(|c| match c {
            Value::Obj(cid) => e.get_prop(cid, "prototype"),
            _ => None,
        })
        .unwrap_or(Value::Obj(e.protos.array_buffer));
    e.alloc_guard(n)?; // Garde d'alloc — JS-C55.
    let id = e.heap.alloc_obj(Object {
        proto,
        props: Vec::new(),
        kind: ObjKind::ByteBuf(vec![0; n]),
        extensible: true,
        elem_integrity: 0,
        args_map: None,
    });
    e.set_own(id, "growable", Value::Bool(false), false);
    e.set_own(id, "maxByteLength", Value::Num(n as f64), false);
    e.set_own(id, "detached", Value::Bool(false), false);
    if let Value::Obj(opts) = arg(args, 1) {
        if let Some(mv) = e.get_prop(opts, "maxByteLength") {
            if !matches!(mv, Value::Undefined) {
                let max = to_index(e, mv)?;
                if max < n {
                    return Err(e.range_error("maxByteLength smaller than length"));
                }
                e.set_own(id, "growable", Value::Bool(true), false);
                e.set_own(id, "maxByteLength", Value::Num(max as f64), false);
            }
        }
    }
    Ok(Value::Obj(id))
}

/// `sab.grow(newLen)` — `resize` côté SAB (spec `growable`).
fn sab_grow(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let id = ab_this(e, this)?;
    let growable = matches!(e.get_prop(id, "growable"), Some(Value::Bool(true)));
    if !growable {
        return Err(e.type_error("SharedArrayBuffer is not growable"));
    }
    let max = match e.get_prop(id, "maxByteLength") {
        Some(Value::Num(m)) => m as usize,
        _ => usize::MAX,
    };
    let n = to_index(e, arg(args, 0))?;
    if n > max {
        return Err(e.range_error("new length exceeds maxByteLength"));
    }
    e.alloc_guard(n)?;
    if let Some(b) = e.heap.obj_mut(id) {
        if let ObjKind::ByteBuf(bytes) = &mut b.kind {
            bytes.resize(n, 0);
        }
    }
    Ok(Value::Undefined)
}

fn sab_growable_get(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let id = ab_this(e, this)?;
    Ok(e.get_prop(id, "growable").unwrap_or(Value::Bool(false)))
}

/// `get resizable` — own flag lu sur `this` (`TypeError` sur tout
/// non-`ByteBuf`, proto compris — spec).
fn ab_resizable_get(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let id = ab_this(e, this)?;
    Ok(e.get_prop(id, "resizable").unwrap_or(Value::Bool(false)))
}

fn ab_max_get(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let id = ab_this(e, this)?;
    Ok(e.get_prop(id, "maxByteLength").unwrap_or(Value::Num(0.0)))
}

fn ab_detached_get(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let id = ab_this(e, this)?;
    Ok(e.get_prop(id, "detached").unwrap_or(Value::Bool(false)))
}

/// Kinds Atomics (spec : hors `Uint8Clamped` et tout float).
fn atomic_ok(ta: TaKind) -> bool {
    matches!(
        ta,
        TaKind::I8
            | TaKind::U8
            | TaKind::I16
            | TaKind::U16
            | TaKind::I32
            | TaKind::U32
            | TaKind::I64
            | TaKind::U64
    )
}

/// Résout `(kind, buffer, octet)` pour `Atomics.op(ta, idx, …)` —
/// `TypeError` sur non-TA/kind non-atomique, `RangeError` hors
/// bornes (spec `ValidateAtomicAccess` + `ValidateIntegerTA`).
fn atomic_access(e: &mut Engine, ta_v: Value, idx_v: Value) -> JsResult<(TaKind, ObjId, usize)> {
    let (ta, bid, off, len) = ta_view(e, ta_v)?;
    if !atomic_ok(ta) {
        return Err(e.type_error("operation cannot be performed on this TypedArray type"));
    }
    let idx = to_index(e, idx_v)?;
    if idx >= len {
        return Err(e.range_error("index out of bounds"));
    }
    Ok((ta, bid, off + idx * ta.size()))
}

/// Lit l'élément atomique — `Value::Num` ou `Value::Big`.
fn atomic_read(e: &mut Engine, ta: TaKind, bid: ObjId, o: usize) -> Value {
    match e.heap.obj(bid).map(|b| &b.kind) {
        Some(ObjKind::ByteBuf(bytes)) if o + ta.size() <= bytes.len() => {
            let region = &bytes[o..];
            if ta.is_big() {
                e.big_val(ta.get_big(region, 0))
            } else {
                Value::Num(ta.get(region, 0))
            }
        }
        _ => Value::Num(f64::NAN),
    }
}

/// Écrit l'élément atomique (f64 ou `BigInt` selon le kind).
fn atomic_write(e: &mut Engine, ta: TaKind, bid: ObjId, o: usize, v: AtomicVal) {
    let Some(b) = e.heap.obj_mut(bid) else { return };
    let ObjKind::ByteBuf(bytes) = &mut b.kind else {
        return;
    };
    if o + ta.size() > bytes.len() {
        return;
    }
    let region = &mut bytes[o..];
    match v {
        AtomicVal::Num(n) => ta.set(region, 0, n),
        AtomicVal::Big(b) => ta.set_big(region, 0, &b),
    }
}

/// Valeur atomique lue/construite (nombre f64 ou `BigInt`).
#[derive(Clone)]
enum AtomicVal {
    Num(f64),
    Big(num_bigint::BigInt),
}

/// Convertit `v` selon le kind — `ToBigInt` sur les vues 64 bits
/// (spec : `TypeError` si Number sur vue `BigInt` et réciproquement).
fn atomic_operand(e: &mut Engine, ta: TaKind, v: Value) -> JsResult<AtomicVal> {
    if ta.is_big() {
        match v {
            Value::Big(_) => Ok(AtomicVal::Big(e.to_bigint(v)?)),
            _ => Err(e.type_error("cannot convert Number to BigInt")),
        }
    } else {
        Ok(AtomicVal::Num(e.to_number(v)?))
    }
}

fn atomic_num_val(v: &AtomicVal) -> f64 {
    match v {
        AtomicVal::Num(n) => *n,
        AtomicVal::Big(_) => 0.0,
    }
}

fn atomics_load(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let (ta, bid, o) = atomic_access(e, arg(args, 0), arg(args, 1))?;
    Ok(atomic_read(e, ta, bid, o))
}

/// `store` → retourne la valeur posée (spec).
fn atomics_store(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let (ta, bid, o) = atomic_access(e, arg(args, 0), arg(args, 1))?;
    let v = atomic_operand(e, ta, arg(args, 2))?;
    atomic_write(e, ta, bid, o, v.clone());
    Ok(match v {
        AtomicVal::Num(n) => Value::Num(n),
        AtomicVal::Big(b) => e.big_val(b),
    })
}

/// Coeur RMW non-big — `add`/`sub`/`and`/`or`/`xor` :
/// `(ta, idx, v)` → ancienne valeur. Arithmétique 32 bits wrapping
/// (les kinds < 32 bits re-wrapptent à l'écriture via `ta.set` —
/// `to_int32` mod 2^32 puis troncature à la largeur, spec).
fn atomics_rmw(e: &mut Engine, args: &[Value], op: fn(i64, i64) -> i64) -> JsResult<Value> {
    let (ta, bid, o) = atomic_access(e, arg(args, 0), arg(args, 1))?;
    let v = atomic_operand(e, ta, arg(args, 2))?;
    let old = atomic_read(e, ta, bid, o);
    let cur = match old {
        Value::Num(n) => n,
        _ => f64::NAN,
    };
    let new = AtomicVal::Num(op(
        i64::from(crate::value::to_int32(cur)),
        i64::from(crate::value::to_int32(atomic_num_val(&v))),
    ) as f64);
    atomic_write(e, ta, bid, o, new);
    Ok(old)
}

fn atomics_add(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let (ta, bid, o) = atomic_access(e, arg(args, 0), arg(args, 1))?;
    if ta.is_big() {
        let old = atomic_read(e, ta, bid, o);
        let a = e.to_bigint(old)?;
        let b = e.to_bigint(arg(args, 2))?;
        atomic_write(e, ta, bid, o, AtomicVal::Big(a + b));
        return Ok(old);
    }
    atomics_rmw(e, args, i64::wrapping_add)
}

fn atomics_sub(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let (ta, bid, o) = atomic_access(e, arg(args, 0), arg(args, 1))?;
    if ta.is_big() {
        let old = atomic_read(e, ta, bid, o);
        let a = e.to_bigint(old)?;
        let b = e.to_bigint(arg(args, 2))?;
        atomic_write(e, ta, bid, o, AtomicVal::Big(a - b));
        return Ok(old);
    }
    atomics_rmw(e, args, i64::wrapping_sub)
}

fn atomics_and(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let (ta, bid, o) = atomic_access(e, arg(args, 0), arg(args, 1))?;
    if ta.is_big() {
        let old = atomic_read(e, ta, bid, o);
        let a = e.to_bigint(old)?;
        let b = e.to_bigint(arg(args, 2))?;
        atomic_write(e, ta, bid, o, AtomicVal::Big(a & b));
        return Ok(old);
    }
    atomics_rmw(e, args, |a, b| a & b)
}

fn atomics_or(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let (ta, bid, o) = atomic_access(e, arg(args, 0), arg(args, 1))?;
    if ta.is_big() {
        let old = atomic_read(e, ta, bid, o);
        let a = e.to_bigint(old)?;
        let b = e.to_bigint(arg(args, 2))?;
        atomic_write(e, ta, bid, o, AtomicVal::Big(a | b));
        return Ok(old);
    }
    atomics_rmw(e, args, |a, b| a | b)
}

fn atomics_xor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let (ta, bid, o) = atomic_access(e, arg(args, 0), arg(args, 1))?;
    if ta.is_big() {
        let old = atomic_read(e, ta, bid, o);
        let a = e.to_bigint(old)?;
        let b = e.to_bigint(arg(args, 2))?;
        atomic_write(e, ta, bid, o, AtomicVal::Big(a ^ b));
        return Ok(old);
    }
    atomics_rmw(e, args, |a, b| a ^ b)
}

fn atomics_exchange(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let (ta, bid, o) = atomic_access(e, arg(args, 0), arg(args, 1))?;
    let v = atomic_operand(e, ta, arg(args, 2))?;
    let old = atomic_read(e, ta, bid, o);
    atomic_write(e, ta, bid, o, v);
    Ok(old)
}

/// `compareExchange(ta, idx, expected, replacement)` → ancienne
/// valeur (spec : comparaison sur la valeur, pas l'identité).
fn atomics_compare_exchange(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let (ta, bid, o) = atomic_access(e, arg(args, 0), arg(args, 1))?;
    let old = atomic_read(e, ta, bid, o);
    #[allow(clippy::float_cmp)] // égalité exacte spec `SameValue`.
    let eq = if ta.is_big() {
        e.to_bigint(old)? == e.to_bigint(arg(args, 2))?
    } else {
        matches!(old, Value::Num(n) if n == e.to_number(arg(args, 2))?)
    };
    if eq {
        let rep = atomic_operand(e, ta, arg(args, 3))?;
        atomic_write(e, ta, bid, o, rep);
    }
    Ok(old)
}

/// `isLockFree(size)` — atomique lock-free pour 1/2/4/8 octets
/// (convention V8/JSC ; impl-defined spec).
fn atomics_is_lock_free(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let n = e.to_number(arg(args, 0))?;
    Ok(Value::Bool(matches!(n as u32, 1 | 2 | 4 | 8)))
}

/// `Atomics.pause([n])` (ES2025) — hint de spin-wait : no-op
/// observable (`undefined`), le délai suggéré est ignoré.
fn atomics_pause(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    let _ = e;
    Ok(Value::Undefined)
}

/// `Atomics.wait` — `TypeError` systématique : mono-thread =
/// main thread navigateur où `wait` est interdit (spec navigateur).
fn atomics_wait(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    Err(e.type_error("Atomics.wait cannot be called in this context"))
}

/// `Atomics.notify` → nombre d'attentes réveillées (toujours 0 —
/// aucune attente possible sans worker).
fn atomics_notify(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    // Validation d'accès quand même (spec : vérifie ta/idx).
    atomic_access(e, arg(args, 0), arg(args, 1))?;
    Ok(Value::Num(0.0))
}

/// `Atomics.waitAsync` → `{async:false, value:"not-equal"}` si la
/// valeur diffère de l'attendue, `{async:false, value:"timed-out"}`
/// sinon (aucun notify ne viendra jamais — mono-thread).
fn atomics_wait_async(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let (ta, bid, o) = atomic_access(e, arg(args, 0), arg(args, 1))?;
    let cur = atomic_read(e, ta, bid, o);
    #[allow(clippy::float_cmp)] // égalité exacte spec `SameValue`.
    let eq = if ta.is_big() {
        e.to_bigint(cur)? == e.to_bigint(arg(args, 2))?
    } else {
        matches!(cur, Value::Num(n) if n == e.to_number(arg(args, 2))?)
    };
    let o2 = e.new_object();
    e.set_own(o2, "async", Value::Bool(false), true);
    let msg = if eq { "timed-out" } else { "not-equal" };
    let v = e.str_val(msg);
    e.set_own(o2, "value", v, true);
    Ok(Value::Obj(o2))
}
