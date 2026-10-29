// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Media Source Extensions — surface JS honnête **sans lecture**
//! (JS-C193, amorce de la piste MSE — ADR-0024 : la lecture est
//! un service d'hôte, le moteur ne décode pas).
//!
//! Ce que le palier livre : `MediaSource`, `SourceBuffer`,
//! `SourceBufferList`, `TimeRanges` réels (constructibles,
//! `instanceof`, `EventTarget` via le rail `__et` partagé), la
//! machine `readyState`, `URL.createObjectURL(mediaSource)` et
//! `appendBuffer` accepté-borné. Ce qu'il ne livre pas —
//! délibérément, honnêteté ADR-0023 :
//!
//! - `isTypeSupported` rend `""` pour **tout** type : aucun
//!   conteneur n'est jouable tant que le transport d'octets vers
//!   l'hôte n'existe pas. Un `""` honnête pousse les players vers
//!   leurs fallbacks (`<video src>` progressif/HLS, servis par le
//!   seam `MediaSurface` existant) plutôt que vers un appendBuffer
//!   sans fin derrière un spinner.
//! - `readyState` reste `"closed"` : rien n'attache le
//!   `MediaSource` à un élément (`blob:` → `<video>` = étape
//!   dédiée côté couture média). `addSourceBuffer`/`appendBuffer`/
//!   `endOfStream` sont néanmoins **permis** (déviation spec
//!   assumée : l'`InvalidStateError` strict tuerait les boots qui
//!   construisent leurs buffers avant l'attache — le stall
//!   « closed » est un comportement de flux bloqué, pas un crash).
//! - `SourceBuffer.buffered` reste vide : les octets sont
//!   comptabilisés, pas stockés — « rien de lisible bufferisé »
//!   est vrai tant que l'hôte ne peut pas les consommer.
//! - Les événements `updatestart`/`update`/`updateend` sont tirés
//!   **en séquence synchrone** dans `appendBuffer` (approximation
//!   documentée du « queue a task » spec — même économie que les
//!   livraisons `FileReader` au checkpoint).
//!
//! Budgets (§8 AGENTS — `appendBuffer` est un vecteur OOM
//! canonique) : cap d'octets par `SourceBuffer`, cap de buffers
//! par `MediaSource`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::heap::{Host, JsResult, ObjKind, Thrown};
use crate::value::{ObjId, Value};
use crate::Engine;

/// Octets retenus par `SourceBuffer` — `appendBuffer` au-delà
/// jette `QuotaExceededError` (budget hostile).
const SB_BYTE_CAP: usize = 64 * 1024 * 1024;

/// `SourceBuffer` vivants par `MediaSource` (un site hostile qui
/// alloue des buffers en boucle reste borné).
const MS_BUFFER_CAP: usize = 8;

/// État moteur MSE — protos paresseux + registre des URL objets
/// pointant un `MediaSource`. Document-local : swappé avec le
/// `BrowsingContext` comme `files`/`idb`.
#[derive(Default)]
pub(crate) struct MseState {
    media_source_proto: Option<ObjId>,
    source_buffer_proto: Option<ObjId>,
    buffer_list_proto: Option<ObjId>,
    time_ranges_proto: Option<ObjId>,
    /// `blob:<origin>/<n>` → wrapper `MediaSource`. L'entrée
    /// garde la source en vie (racine GC) jusqu'à
    /// `revokeObjectURL` ou la mort du document — consommée par
    /// la future couture d'offre `mse` (transport vers l'hôte).
    object_urls: HashMap<String, ObjId>,
}

impl MseState {
    /// Racines GC : protos installés + `MediaSource` référencés
    /// par une URL objet vivante.
    pub(crate) fn root_ids(&self, ids: &mut Vec<ObjId>) {
        for p in [
            self.media_source_proto,
            self.source_buffer_proto,
            self.buffer_list_proto,
            self.time_ranges_proto,
        ]
        .into_iter()
        .flatten()
        {
            ids.push(p);
        }
        ids.extend(self.object_urls.values().copied());
    }
}

// ------------------------------------------------------------------
// Données internes
// ------------------------------------------------------------------

/// `MediaSource` interne.
struct MsData {
    /// `"closed"` | `"open"` | `"ended"` — reste `"closed"` en
    /// C173 : aucune attache `blob:` → élément n'existe encore.
    ready_state: &'static str,
    /// Wrappers `SourceBuffer` (ordre d'ajout — spec).
    buffers: Vec<Value>,
    /// Wrapper `SourceBufferList` caché (`sourceBuffers ===
    /// sourceBuffers` reste vrai entre deux lectures).
    list: Option<Value>,
    /// `duration` — `NaN` tant que jamais fixée (spec).
    duration: f64,
    /// `setLiveSeekableRange` retenu (`None` = `clear`).
    live_range: Option<(f64, f64)>,
}

/// `SourceBuffer` interne.
struct SbData {
    /// Back-ref vers le wrapper `MediaSource` (tracé).
    ms: Value,
    /// MIME passé à `addSourceBuffer`/`changeType` — retenu
    /// pour le futur transport (le seam d'offre en aura
    /// besoin ; aucun getter spec ne l'expose).
    #[allow(dead_code)]
    mime: String,
    updating: bool,
    /// `"segments"` (défaut) | `"sequence"`.
    mode: String,
    timestamp_offset: f64,
    append_start: f64,
    append_end: f64,
    /// Octets acceptés — comptabilisés, jamais stockés (C173 :
    /// pas de transport, `buffered` reste vide).
    bytes: usize,
}

struct MsHost {
    d: Rc<RefCell<MsData>>,
}

struct SbHost {
    d: Rc<RefCell<SbData>>,
}

/// `SourceBufferList` — vue vivante sur `MsData.buffers`.
struct SbListHost {
    d: Rc<RefCell<MsData>>,
}

/// `TimeRanges` — snapshot figé de plages `(start, end)`.
struct TrHost {
    ranges: Rc<Vec<(f64, f64)>>,
}

fn ms_data(e: &Engine, v: Value) -> Option<Rc<RefCell<MsData>>> {
    let Value::Obj(id) = v else { return None };
    match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::Host(h)) => h
            .clone()
            .as_any()
            .downcast_ref::<MsHost>()
            .map(|r| r.d.clone()),
        _ => None,
    }
}

fn sb_data(e: &Engine, v: Value) -> Option<Rc<RefCell<SbData>>> {
    let Value::Obj(id) = v else { return None };
    match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::Host(h)) => h
            .clone()
            .as_any()
            .downcast_ref::<SbHost>()
            .map(|r| r.d.clone()),
        _ => None,
    }
}

fn tr_ranges(e: &Engine, v: Value) -> Option<Rc<Vec<(f64, f64)>>> {
    let Value::Obj(id) = v else { return None };
    match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::Host(h)) => h
            .clone()
            .as_any()
            .downcast_ref::<TrHost>()
            .map(|r| r.ranges.clone()),
        _ => None,
    }
}

/// `v` est un wrapper `MediaSource` (pour `createObjectURL`).
pub(crate) fn is_media_source(e: &Engine, v: Value) -> bool {
    ms_data(e, v).is_some()
}

/// Erreur DOM nommée jetée — `{name, message}` (approximation
/// documentée du `DOMException`, même convention qu'IDB/crypto).
fn mse_err(e: &mut Engine, name: &str, msg: &str) -> Thrown {
    let o = e.new_object();
    let n = e.str_val(name);
    e.set_own(o, "name", n, true);
    let m = e.str_val(msg);
    e.set_own(o, "message", m, true);
    Thrown(Value::Obj(o))
}

/// Event minimal `{type, target, currentTarget}`.
fn mse_event(e: &mut Engine, ty: &str, target: Value) -> Value {
    let o = e.new_object();
    let t = e.str_val(ty);
    e.set_own(o, "type", t, true);
    e.set_own(o, "target", target, true);
    e.set_own(o, "currentTarget", target, true);
    Value::Obj(o)
}

/// `on<ty>` own-prop + listeners `addEventListener` (rail `__et`
/// partagé — même helper que `fire_idb`).
fn fire_mse(e: &mut Engine, target: ObjId, ty: &str) -> JsResult<()> {
    let ev = mse_event(e, ty, Value::Obj(target));
    let key = format!("on{ty}");
    if let Some(f) = e.get_prop(target, &key) {
        if e.is_callable(f) {
            e.call_value(f, Value::Obj(target), &[ev])?;
        }
    }
    if let Some(Value::Obj(map)) = e.get_prop(target, "__et") {
        if let Some(Value::Obj(l)) = e.get_prop(map, ty) {
            let len = match e.get_prop(l, "length") {
                Some(Value::Num(n)) => n as usize,
                _ => 0,
            };
            for i in 0..len {
                if let Some(cb) = e.get_prop(l, &i.to_string()) {
                    if e.is_callable(cb) {
                        e.call_value(cb, Value::Obj(target), &[ev])?;
                    }
                }
            }
        }
    }
    Ok(())
}

// ------------------------------------------------------------------
// Protos paresseux
// ------------------------------------------------------------------

fn tagged_proto(e: &mut Engine, tag: &str) -> ObjId {
    let p = e.new_object();
    let t = e.str_val(tag);
    e.set_own(p, crate::interp::SYM_TO_STRING_TAG, t, false);
    p
}

/// Méthodes `EventTarget` communes (`__et` partagé).
fn install_et_methods(e: &mut Engine, p: ObjId) {
    e.add_method(p, "addEventListener", crate::dom_host::et_add_listener);
    e.add_method(
        p,
        "removeEventListener",
        crate::dom_host::et_remove_listener,
    );
    e.add_method(p, "dispatchEvent", crate::dom_host::et_dispatch_event);
}

fn ms_proto(e: &mut Engine) -> ObjId {
    if let Some(p) = e.mse.media_source_proto {
        return p;
    }
    let p = tagged_proto(e, "MediaSource");
    install_et_methods(e, p);
    e.add_method(p, "addSourceBuffer", ms_add_source_buffer);
    e.add_method(p, "removeSourceBuffer", ms_remove_source_buffer);
    e.add_method(p, "endOfStream", ms_end_of_stream);
    e.add_method(p, "setLiveSeekableRange", ms_set_live_range);
    e.add_method(p, "clearLiveSeekableRange", ms_clear_live_range);
    e.mse.media_source_proto = Some(p);
    p
}

fn sb_proto(e: &mut Engine) -> ObjId {
    if let Some(p) = e.mse.source_buffer_proto {
        return p;
    }
    let p = tagged_proto(e, "SourceBuffer");
    install_et_methods(e, p);
    e.add_method(p, "appendBuffer", sb_append_buffer);
    e.add_method(p, "abort", sb_abort);
    e.add_method(p, "remove", sb_remove);
    e.add_method(p, "changeType", sb_change_type);
    e.mse.source_buffer_proto = Some(p);
    p
}

fn sbl_proto(e: &mut Engine) -> ObjId {
    if let Some(p) = e.mse.buffer_list_proto {
        return p;
    }
    let p = tagged_proto(e, "SourceBufferList");
    install_et_methods(e, p);
    e.mse.buffer_list_proto = Some(p);
    p
}

fn tr_proto(e: &mut Engine) -> ObjId {
    if let Some(p) = e.mse.time_ranges_proto {
        return p;
    }
    let p = tagged_proto(e, "TimeRanges");
    e.add_method(p, "start", tr_start);
    e.add_method(p, "end", tr_end);
    e.mse.time_ranges_proto = Some(p);
    p
}

/// Wrapper `TimeRanges` figé sur un snapshot de plages.
fn time_ranges(e: &mut Engine, ranges: Vec<(f64, f64)>) -> Value {
    let tp = tr_proto(e);
    let v = e.new_host_object(Rc::new(TrHost {
        ranges: Rc::new(ranges),
    }));
    if let Value::Obj(id) = v {
        if let Some(o) = e.heap.obj_mut(id) {
            o.proto = Value::Obj(tp);
        }
    }
    v
}

/// Wrapper `SourceBufferList` caché sur le `MediaSource`.
fn buffer_list(e: &mut Engine, d: &Rc<RefCell<MsData>>) -> Value {
    if let Some(l) = d.borrow().list {
        return l;
    }
    let lp = sbl_proto(e);
    let v = e.new_host_object(Rc::new(SbListHost { d: d.clone() }));
    if let Value::Obj(id) = v {
        if let Some(o) = e.heap.obj_mut(id) {
            o.proto = Value::Obj(lp);
        }
        d.borrow_mut().list = Some(v);
    }
    v
}

// ------------------------------------------------------------------
// Constructeurs
// ------------------------------------------------------------------

/// `new MediaSource()` — source `closed` vide.
fn ms_ctor(e: &mut Engine, _t: Value, _args: &[Value]) -> JsResult<Value> {
    let mp = ms_proto(e);
    let d = Rc::new(RefCell::new(MsData {
        ready_state: "closed",
        buffers: Vec::new(),
        list: None,
        duration: f64::NAN,
        live_range: None,
    }));
    let v = e.new_host_object(Rc::new(MsHost { d }));
    if let Value::Obj(id) = v {
        if let Some(o) = e.heap.obj_mut(id) {
            o.proto = Value::Obj(mp);
        }
    }
    Ok(v)
}

/// `SourceBuffer`/`SourceBufferList`/`TimeRanges` — interfaces
/// non constructibles (spec : `Illegal constructor`) mais à
/// prototype réel pour `instanceof`/feature-detect.
fn sb_illegal_ctor(e: &mut Engine, _t: Value, _args: &[Value]) -> JsResult<Value> {
    Err(e.type_error("SourceBuffer: Illegal constructor"))
}

fn sbl_illegal_ctor(e: &mut Engine, _t: Value, _args: &[Value]) -> JsResult<Value> {
    Err(e.type_error("SourceBufferList: Illegal constructor"))
}

fn tr_illegal_ctor(e: &mut Engine, _t: Value, _args: &[Value]) -> JsResult<Value> {
    Err(e.type_error("TimeRanges: Illegal constructor"))
}

/// `MediaSource.isTypeSupported(mime)` → `""` **pour tout type**
/// (C173 : aucun conteneur jouable sans transport vers l'hôte —
/// réponse honnête qui pousse les players vers leurs fallbacks,
/// jamais un faux succès — ADR-0023).
fn ms_is_type_supported(e: &mut Engine, _t: Value, _args: &[Value]) -> JsResult<Value> {
    Ok(e.str_val(""))
}

// ------------------------------------------------------------------
// Méthodes MediaSource
// ------------------------------------------------------------------

/// `ms.addSourceBuffer(mime)` — buffer neuf, poussé dans
/// `sourceBuffers` (ordre conservé), `addsourcebuffer` tiré sur
/// la liste. Permis quel que soit `readyState` (déviation
/// documentée — voir en-tête de module).
fn ms_add_source_buffer(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Some(d) = ms_data(e, this) else {
        return Err(e.type_error("MediaSource.addSourceBuffer: Illegal invocation"));
    };
    let Some(&v) = args.first() else {
        return Err(e.type_error("MediaSource.addSourceBuffer: 1 argument required"));
    };
    let mime = e.to_js_string(v)?;
    if d.borrow().buffers.len() >= MS_BUFFER_CAP {
        return Err(mse_err(
            e,
            "QuotaExceededError",
            "MediaSource: source buffer quota",
        ));
    }
    let sp = sb_proto(e);
    let sb = Rc::new(RefCell::new(SbData {
        ms: this,
        mime,
        updating: false,
        mode: "segments".to_string(),
        timestamp_offset: 0.0,
        append_start: 0.0,
        append_end: f64::INFINITY,
        bytes: 0,
    }));
    let v = e.new_host_object(Rc::new(SbHost { d: sb }));
    if let Value::Obj(id) = v {
        if let Some(o) = e.heap.obj_mut(id) {
            o.proto = Value::Obj(sp);
        }
        d.borrow_mut().buffers.push(v);
    }
    // `addsourcebuffer` sur la liste (spec) — la liste est
    // matérialisée pour porter l'événement même si le site ne
    // l'a jamais lue.
    if let Value::Obj(lid) = buffer_list(e, &d) {
        fire_mse(e, lid, "addsourcebuffer")?;
    }
    Ok(v)
}

/// `ms.removeSourceBuffer(sb)` — splice + `removesourcebuffer`.
fn ms_remove_source_buffer(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Some(d) = ms_data(e, this) else {
        return Err(e.type_error("MediaSource.removeSourceBuffer: Illegal invocation"));
    };
    let v = args.first().copied().unwrap_or(Value::Undefined);
    if sb_data(e, v).is_none() {
        return Err(e.type_error("MediaSource.removeSourceBuffer: not a SourceBuffer"));
    }
    let pos = {
        let mut md = d.borrow_mut();
        let pos = md.buffers.iter().position(|b| *b == v);
        if let Some(i) = pos {
            md.buffers.remove(i);
        }
        pos
    };
    if pos.is_none() {
        return Err(mse_err(
            e,
            "NotFoundError",
            "MediaSource.removeSourceBuffer: buffer not in sourceBuffers",
        ));
    }
    if let Value::Obj(lid) = buffer_list(e, &d) {
        fire_mse(e, lid, "removesourcebuffer")?;
    }
    Ok(Value::Undefined)
}

/// `ms.endOfStream(err?)` — `readyState` → `"ended"` +
/// `sourceended`. Permis hors `"open"` (déviation documentée).
fn ms_end_of_stream(e: &mut Engine, this: Value, _args: &[Value]) -> JsResult<Value> {
    let Some(d) = ms_data(e, this) else {
        return Err(e.type_error("MediaSource.endOfStream: Illegal invocation"));
    };
    d.borrow_mut().ready_state = "ended";
    let Value::Obj(mid) = this else {
        return Ok(Value::Undefined);
    };
    fire_mse(e, mid, "sourceended")?;
    Ok(Value::Undefined)
}

/// `ms.setLiveSeekableRange(start, end)` — plage live retenue,
/// relue par `liveSeekableRange`.
fn ms_set_live_range(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Some(d) = ms_data(e, this) else {
        return Err(e.type_error("MediaSource.setLiveSeekableRange: Illegal invocation"));
    };
    let s = e.to_number(args.first().copied().unwrap_or(Value::Undefined))?;
    let t = e.to_number(args.get(1).copied().unwrap_or(Value::Undefined))?;
    if !(0.0..=t).contains(&s) {
        return Err(e.type_error("MediaSource.setLiveSeekableRange: bad range"));
    }
    d.borrow_mut().live_range = Some((s, t));
    Ok(Value::Undefined)
}

/// `ms.clearLiveSeekableRange()` — plage live vidée.
fn ms_clear_live_range(e: &mut Engine, this: Value, _args: &[Value]) -> JsResult<Value> {
    let Some(d) = ms_data(e, this) else {
        return Err(e.type_error("MediaSource.clearLiveSeekableRange: Illegal invocation"));
    };
    d.borrow_mut().live_range = None;
    Ok(Value::Undefined)
}

// ------------------------------------------------------------------
// Méthodes SourceBuffer
// ------------------------------------------------------------------

/// `sb.appendBuffer(buf)` — octets acceptés-**comptabilisés**
/// (jamais stockés : pas de transport en C173). Séquence
/// synchrone `updatestart` → `update` → `updateend` ; le cap
/// d'octets jette `QuotaExceededError` (budget hostile).
fn sb_append_buffer(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Some(d) = sb_data(e, this) else {
        return Err(e.type_error("SourceBuffer.appendBuffer: Illegal invocation"));
    };
    let v = args.first().copied().unwrap_or(Value::Undefined);
    let Some(bytes) = crate::typed_array::bytes_of_value(e, v) else {
        return Err(
            e.type_error("SourceBuffer.appendBuffer: argument is not an ArrayBuffer or view")
        );
    };
    {
        let mut sd = d.borrow_mut();
        let next = sd.bytes.saturating_add(bytes.len());
        if next > SB_BYTE_CAP {
            return Err(mse_err(
                e,
                "QuotaExceededError",
                "SourceBuffer: byte quota exceeded",
            ));
        }
        sd.bytes = next;
        sd.updating = true;
    }
    let Value::Obj(sid) = this else {
        return Ok(Value::Undefined);
    };
    // Approximation documentée : les trois événements partent en
    // séquence synchrone (spec : « queue a task ») — l'ordre vu
    // par les listeners est correct.
    fire_mse(e, sid, "updatestart")?;
    fire_mse(e, sid, "update")?;
    d.borrow_mut().updating = false;
    fire_mse(e, sid, "updateend")?;
    Ok(Value::Undefined)
}

/// `sb.abort()` — `updating` retombé + `abort`/`updateend`.
fn sb_abort(e: &mut Engine, this: Value, _args: &[Value]) -> JsResult<Value> {
    let Some(d) = sb_data(e, this) else {
        return Err(e.type_error("SourceBuffer.abort: Illegal invocation"));
    };
    d.borrow_mut().updating = false;
    let Value::Obj(sid) = this else {
        return Ok(Value::Undefined);
    };
    fire_mse(e, sid, "abort")?;
    fire_mse(e, sid, "updateend")?;
    Ok(Value::Undefined)
}

/// `sb.remove(start, end)` — accepté : `buffered` est vide, il
/// n'y a rien à évincer ; la séquence d'événements est tirée.
fn sb_remove(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Some(d) = sb_data(e, this) else {
        return Err(e.type_error("SourceBuffer.remove: Illegal invocation"));
    };
    let s = e.to_number(args.first().copied().unwrap_or(Value::Undefined))?;
    let t = e.to_number(args.get(1).copied().unwrap_or(Value::Undefined))?;
    if !(s.is_finite() && s >= 0.0) || t <= s {
        return Err(e.type_error("SourceBuffer.remove: bad range"));
    }
    d.borrow_mut().updating = true;
    let Value::Obj(sid) = this else {
        return Ok(Value::Undefined);
    };
    fire_mse(e, sid, "updatestart")?;
    fire_mse(e, sid, "update")?;
    d.borrow_mut().updating = false;
    fire_mse(e, sid, "updateend")?;
    Ok(Value::Undefined)
}

/// `sb.changeType(mime)` — mime retenu (spec : re-parse du
/// type supporté — le support reste `""` en C173, on accepte
/// et on retient pour le futur transport).
fn sb_change_type(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Some(d) = sb_data(e, this) else {
        return Err(e.type_error("SourceBuffer.changeType: Illegal invocation"));
    };
    let mime = e.to_js_string(args.first().copied().unwrap_or(Value::Undefined))?;
    d.borrow_mut().mime = mime;
    Ok(Value::Undefined)
}

// ------------------------------------------------------------------
// Méthodes TimeRanges
// ------------------------------------------------------------------

/// `tr.start(i)` / `tr.end(i)` — borne de la i-ème plage, erreur
/// hors rang.
fn tr_bound(e: &mut Engine, this: Value, args: &[Value], start: bool) -> JsResult<Value> {
    let Some(ranges) = tr_ranges(e, this) else {
        return Err(e.type_error("TimeRanges: Illegal invocation"));
    };
    let i = e.to_number(args.first().copied().unwrap_or(Value::Undefined))?;
    if !(i >= 0.0 && i.fract() == 0.0) {
        return Err(e.type_error("TimeRanges: index out of range"));
    }
    let Some(&(s, t)) = ranges.get(i as usize) else {
        return Err(e.type_error("TimeRanges: index out of range"));
    };
    Ok(Value::Num(if start { s } else { t }))
}

fn tr_start(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    tr_bound(e, this, args, true)
}

fn tr_end(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    tr_bound(e, this, args, false)
}

// ------------------------------------------------------------------
// `URL.createObjectURL(mediaSource)`
// ------------------------------------------------------------------

/// `createObjectURL` sur un `MediaSource` — enregistre
/// `blob:<origin>/<n>` → wrapper dans `object_urls` et rend
/// l'URL. `None` si `v` n'est pas un `MediaSource` (le chemin
/// `Blob` prend le relais dans `files.rs`).
pub(crate) fn object_url_for(e: &mut Engine, v: Value) -> Option<String> {
    let Value::Obj(mid) = v else { return None };
    if !is_media_source(e, v) {
        return None;
    }
    let url = crate::files::alloc_object_url(e);
    e.mse.object_urls.insert(url.clone(), mid);
    Some(url)
}

/// `revokeObjectURL` — purge aussi le registre MSE (appelé
/// depuis `files.rs` ; retourne `true` si une entrée partait).
pub(crate) fn revoke_object_url(e: &mut Engine, url: &str) -> bool {
    e.mse.object_urls.remove(url).is_some()
}

/// `blob:` → wrapper `MediaSource` enregistré — point d'entrée
/// de la future couture d'offre `mse` côté `explorer-core`
/// `url` → `ObjId` `MediaSource` enregistré (`createObjectURL`).
/// Consulté par le scan média core (MSE-2) et par
/// l'attache `src = blob:`.
pub(crate) fn object_url_media(e: &Engine, url: &str) -> Option<ObjId> {
    e.mse.object_urls.get(url).copied()
}

/// Attache tentative (MSE-2, ADR-0029) — appelée quand un
/// `<video>`/`<audio>` reçoit `src = blob:…` (prop directe ou
/// `setAttribute`) : `url` enregistrée `kind=mse` →
/// `readyState` `"closed"` → `"open"` + `sourceopen`
/// synchrone (spec : file d'événements — approximation
/// documentée, cohérente avec les autres événements du module).
/// Idempotent : une source déjà `open`/`ended` n'est pas
/// re-tirée — la spec ignore le second attachement.
pub(crate) fn maybe_attach(e: &mut Engine, url: &str) {
    let Some(mid) = object_url_media(e, url) else {
        return;
    };
    let Some(d) = ms_data(e, Value::Obj(mid)) else {
        return;
    };
    if d.borrow().ready_state != "closed" {
        return;
    }
    d.borrow_mut().ready_state = "open";
    let _ = fire_mse(e, mid, "sourceopen");
}

impl Engine {
    /// `true` si `url` est un object URL `MediaSource`
    /// enregistré — port `JsEngine` consulté par le scan
    /// média core pour marquer l'offre `mse` (ADR-0029).
    #[must_use]
    pub fn is_media_source_url(&self, url: &str) -> bool {
        self.mse.object_urls.contains_key(url)
    }
}

// ------------------------------------------------------------------
// impl Host
// ------------------------------------------------------------------

impl Host for MsHost {
    fn get(&self, e: &mut Engine, _t: Value, k: &str) -> JsResult<Option<Value>> {
        Ok(match k {
            "readyState" => Some(e.str_val(self.d.borrow().ready_state)),
            "sourceBuffers" | "activeSourceBuffers" => {
                // `buffer_list` emprunte le même `RefCell` —
                // relâcher le borrow avant d'y entrer.
                let dd = self.d.clone();
                Some(buffer_list(e, &dd))
            }
            "duration" => Some(Value::Num(self.d.borrow().duration)),
            "liveSeekableRange" => {
                let ranges = self
                    .d
                    .borrow()
                    .live_range
                    .map_or_else(Vec::new, |r| vec![r]);
                Some(time_ranges(e, ranges))
            }
            _ => None,
        })
    }

    fn set(&self, e: &mut Engine, _t: Value, k: &str, v: Value) -> JsResult<bool> {
        if k == "duration" {
            let n = e.to_number(v)?;
            // Spec : `NaN` ou négatif → `TypeError`.
            if n.is_nan() || n < 0.0 {
                return Err(e.type_error("MediaSource.duration: bad value"));
            }
            self.d.borrow_mut().duration = n;
            return Ok(true);
        }
        Ok(false)
    }

    fn trace(&self, out: &mut Vec<ObjId>) {
        let d = self.d.borrow();
        for b in &d.buffers {
            crate::heap::push_value(out, *b);
        }
        if let Some(l) = d.list {
            crate::heap::push_value(out, l);
        }
    }

    fn class_name(&self) -> &'static str {
        "MediaSource"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl Host for SbHost {
    fn get(&self, e: &mut Engine, _t: Value, k: &str) -> JsResult<Option<Value>> {
        Ok(match k {
            "updating" => Some(Value::Bool(self.d.borrow().updating)),
            "mode" => Some(e.str_val(self.d.borrow().mode.clone())),
            "timestampOffset" => Some(Value::Num(self.d.borrow().timestamp_offset)),
            "appendWindowStart" => Some(Value::Num(self.d.borrow().append_start)),
            "appendWindowEnd" => Some(Value::Num(self.d.borrow().append_end)),
            "buffered" => Some(time_ranges(e, Vec::new())),
            "audioTracks" | "videoTracks" | "textTracks" => Some(e.new_array(Vec::new())),
            _ => None,
        })
    }

    fn set(&self, e: &mut Engine, _t: Value, k: &str, v: Value) -> JsResult<bool> {
        match k {
            "mode" => {
                let m = e.to_js_string(v)?;
                if m != "segments" && m != "sequence" {
                    return Err(e.type_error("SourceBuffer.mode: bad value"));
                }
                self.d.borrow_mut().mode = m;
                Ok(true)
            }
            "timestampOffset" => {
                let n = e.to_number(v)?;
                if !n.is_finite() {
                    return Err(e.type_error("SourceBuffer.timestampOffset: bad value"));
                }
                self.d.borrow_mut().timestamp_offset = n;
                Ok(true)
            }
            "appendWindowStart" | "appendWindowEnd" => {
                let n = e.to_number(v)?;
                if n.is_nan() {
                    return Err(e.type_error("SourceBuffer.appendWindow: bad value"));
                }
                let mut d = self.d.borrow_mut();
                if k == "appendWindowStart" {
                    if n < 0.0 {
                        return Err(e.type_error("SourceBuffer.appendWindowStart: bad value"));
                    }
                    d.append_start = n;
                } else {
                    d.append_end = n;
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn trace(&self, out: &mut Vec<ObjId>) {
        let d = self.d.borrow();
        crate::heap::push_value(out, d.ms);
    }

    fn class_name(&self) -> &'static str {
        "SourceBuffer"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl Host for SbListHost {
    fn get(&self, _e: &mut Engine, _t: Value, k: &str) -> JsResult<Option<Value>> {
        let d = self.d.borrow();
        if k == "length" {
            return Ok(Some(Value::Num(d.buffers.len() as f64)));
        }
        if let Ok(i) = k.parse::<usize>() {
            return Ok(d.buffers.get(i).copied());
        }
        Ok(None)
    }

    fn trace(&self, out: &mut Vec<ObjId>) {
        let d = self.d.borrow();
        for b in &d.buffers {
            crate::heap::push_value(out, *b);
        }
    }

    fn class_name(&self) -> &'static str {
        "SourceBufferList"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl Host for TrHost {
    fn get(&self, _e: &mut Engine, _t: Value, k: &str) -> JsResult<Option<Value>> {
        if k == "length" {
            return Ok(Some(Value::Num(self.ranges.len() as f64)));
        }
        Ok(None)
    }

    fn class_name(&self) -> &'static str {
        "TimeRanges"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

// ------------------------------------------------------------------
// Installation
// ------------------------------------------------------------------

/// Installe `MediaSource`/`SourceBuffer`/`SourceBufferList`/
/// `TimeRanges` sur le global — idempotent (appelé par
/// `install_dom`, donc par contexte iframe aussi).
pub(crate) fn install_mse(e: &mut Engine) {
    let g = e.global;
    if e.get_prop(g, "MediaSource").is_some() {
        return;
    }
    // `MediaSource` — vrai constructeur.
    let mp = ms_proto(e);
    let fv = e.native_fn("MediaSource", ms_ctor);
    if let Value::Obj(fid) = fv {
        e.set_own(fid, "prototype", Value::Obj(mp), false);
        if let Some(o) = e.heap.obj_mut(mp) {
            o.set_own("constructor", fv, false);
        }
        e.add_method(fid, "isTypeSupported", ms_is_type_supported);
    }
    e.set_own(g, "MediaSource", fv, false);
    // Interfaces non constructibles — ctor `Illegal constructor`
    // + vrai prototype (`instanceof`, feature-detect proto).
    for (name, ctor, proto) in [
        (
            "SourceBuffer",
            sb_illegal_ctor as crate::heap::NativeFn,
            sb_proto(e),
        ),
        ("SourceBufferList", sbl_illegal_ctor, sbl_proto(e)),
        ("TimeRanges", tr_illegal_ctor, tr_proto(e)),
    ] {
        let fv = e.native_fn(name, ctor);
        if let Value::Obj(fid) = fv {
            e.set_own(fid, "prototype", Value::Obj(proto), false);
        }
        e.set_own(g, name, fv, false);
    }
}
