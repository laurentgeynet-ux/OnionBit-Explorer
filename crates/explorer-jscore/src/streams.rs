// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `ReadableStream`/`WritableStream`/`TransformStream` (JS-C71).
//!
//! Modèle mémoire — pas d'I/O dans le miroir :
//!
//! - `ReadableStream` : queue de chunks + `underlyingSource`
//!   (`start(controller)` appelé au construct, `pull` invoqué
//!   quand `read()` trouve la queue vide, `cancel`).
//!   `getReader()` → `read()` promesses `{value, done}` : chunk
//!   dispo / `closed` / `error` → résolution immédiate ;
//!   queue vide après `pull` → promesse en attente, résolue au
//!   prochain `enqueue`/`close`/`error` (ou re-tentée au
//!   checkpoint microtask). `[Symbol.asyncIterator]` →
//!   `for await` (le moteur `await` chaque `next()`).
//! - `WritableStream` : sink `{start, write, close, abort}` ;
//!   `getWriter()` → `write(chunk)`/`close()`/`abort(r)` en
//!   promesses (la promesse du sink est chaînée — `await` sur
//!   pending → rejet honnête, même limite que `await_value`).
//! - `TransformStream` : `{readable, writable}` — `writable.write`
//!   appelle `transformer.transform(chunk, controller)` dont
//!   `enqueue` pousse dans le readable ; `writable.close`
//!   appelle `flush(controller)?` puis ferme le readable.
//! - `pipeTo`/`pipeThrough` : drain synchrone des chunks (pas
//!   de backpressure — approximation documentée, contenu en
//!   mémoire) ; `tee()` clone la queue restante dans deux
//!   branches (le stream source devient `locked`).

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use crate::heap::{Host, JsResult, ObjKind};
use crate::value::{ObjId, Value};
use crate::Engine;

/// État moteur des streams — protos paresseux + readers dont
/// la `read()` attend un chunk (re-tentés au checkpoint).
#[derive(Default)]
pub(crate) struct StreamState {
    /// `ReadableStream.prototype`.
    rs_proto: Option<ObjId>,
    /// `ReadableStreamDefaultReader.prototype`.
    reader_proto: Option<ObjId>,
    /// `ReadableStreamDefaultController.prototype`.
    rs_ctrl_proto: Option<ObjId>,
    /// `WritableStream.prototype`.
    ws_proto: Option<ObjId>,
    /// `WritableStreamDefaultWriter.prototype`.
    writer_proto: Option<ObjId>,
    /// `WritableStreamDefaultController.prototype`.
    ws_ctrl_proto: Option<ObjId>,
    /// `TransformStream.prototype` / `…DefaultController`.
    ts_proto: Option<ObjId>,
    ts_ctrl_proto: Option<ObjId>,
    /// Wrappers `ReadableStream` ayant des `read()` pendantes —
    /// drainés au checkpoint microtask (re-`pull`).
    pending_streams: Vec<ObjId>,
}

impl StreamState {
    /// Racines GC : protos + streams avec lectures pendantes
    /// (le `Host::trace` couvre ensuite queue/source/callbacks).
    pub(crate) fn root_ids(&self, ids: &mut Vec<ObjId>) {
        for p in [
            self.rs_proto,
            self.reader_proto,
            self.rs_ctrl_proto,
            self.ws_proto,
            self.writer_proto,
            self.ws_ctrl_proto,
            self.ts_proto,
            self.ts_ctrl_proto,
        ]
        .into_iter()
        .flatten()
        {
            ids.push(p);
        }
        ids.extend(self.pending_streams.iter().copied());
    }
}

/// Argument `args[i]` — `Undefined` hors rang (helper local).
fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).copied().unwrap_or(Value::Undefined)
}

// ------------------------------------------------------------------
// ReadableStream
// ------------------------------------------------------------------

/// État partagé d'un `ReadableStream` (stream ↔ controller ↔
/// reader ↔ branches `tee`).
struct RsData {
    /// Chunks en attente de lecture.
    queue: VecDeque<Value>,
    /// `close()` appelé — la queue se vide puis `done:true`.
    closed: bool,
    /// `error(e)` — le stream est en échec irréversible.
    errored: Option<Value>,
    /// `underlyingSource.pull` — appelé quand `read()` trouve
    /// la queue vide (backpressure simplifiée).
    pull: Option<Value>,
    /// `underlyingSource.cancel` — appelé par `rs.cancel`/
    /// `reader.cancel`.
    cancel: Option<Value>,
    /// `underlyingSource` (this de `pull`/`cancel`).
    source: Value,
    /// Reader verrouillé (`locked`).
    locked: bool,
    /// Promesses `read()` pendantes (queue vide post-`pull`) —
    /// résolues par `enqueue`/`close`/`error` ou re-tentées au
    /// checkpoint.
    pending_reads: Vec<ObjId>,
}

/// Host `ReadableStream` + `ReadableStreamDefaultReader` +
/// `ReadableStreamDefaultController` — même `Rc<RsData>`.
struct RsHost {
    d: Rc<RefCell<RsData>>,
}

/// `RsData` d'un `Value` stream/reader/controller — `None` sinon.
fn rs_data(e: &Engine, v: Value) -> Option<Rc<RefCell<RsData>>> {
    let Value::Obj(id) = v else { return None };
    match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::Host(h)) => h
            .clone()
            .as_any()
            .downcast_ref::<RsHost>()
            .map(|r| r.d.clone()),
        _ => None,
    }
}

/// Proto paresseux neuf — `Object.prototype` + `@@toStringTag`.
fn new_tagged_proto(e: &mut Engine, tag: &str) -> ObjId {
    let p = e.new_object();
    let t = e.str_val(tag);
    e.set_own(p, crate::interp::SYM_TO_STRING_TAG, t, false);
    p
}

fn rs_proto(e: &mut Engine) -> ObjId {
    if let Some(p) = e.streams.rs_proto {
        return p;
    }
    let p = new_tagged_proto(e, "ReadableStream");
    e.add_method(p, "getReader", rs_get_reader);
    e.add_method(p, "cancel", rs_cancel);
    e.add_method(p, "pipeThrough", rs_pipe_through);
    e.add_method(p, "pipeTo", rs_pipe_to);
    e.add_method(p, "tee", rs_tee);
    // `for await` — itérateur async adossé à un reader
    // (un `next()` par appel, spec async-iterable).
    let it = e.native_fn("[Symbol.asyncIterator]", rs_async_iter);
    e.set_own(p, crate::interp::SYM_ASYNC_ITERATOR, it, false);
    e.streams.rs_proto = Some(p);
    p
}

fn reader_proto(e: &mut Engine) -> ObjId {
    if let Some(p) = e.streams.reader_proto {
        return p;
    }
    let p = new_tagged_proto(e, "ReadableStreamDefaultReader");
    e.add_method(p, "read", reader_read);
    e.add_method(p, "cancel", reader_cancel);
    e.add_method(p, "releaseLock", reader_release);
    e.streams.reader_proto = Some(p);
    p
}

fn rs_ctrl_proto(e: &mut Engine) -> ObjId {
    if let Some(p) = e.streams.rs_ctrl_proto {
        return p;
    }
    let p = new_tagged_proto(e, "ReadableStreamDefaultController");
    e.add_method(p, "enqueue", ctrl_enqueue);
    e.add_method(p, "close", ctrl_close);
    e.add_method(p, "error", ctrl_error);
    e.streams.rs_ctrl_proto = Some(p);
    p
}

/// Alloue un host `RsHost` avec `proto` donné.
fn rs_obj(e: &mut Engine, d: &Rc<RefCell<RsData>>, proto: ObjId) -> Value {
    let v = e.new_host_object(Rc::new(RsHost { d: d.clone() }));
    if let Value::Obj(id) = v {
        if let Some(o) = e.heap.obj_mut(id) {
            o.proto = Value::Obj(proto);
        }
    }
    v
}

/// `new ReadableStream(source?, strategy?)` — `start(controller)`
/// appelé immédiatement (spec : enqueue en synchrone, cas
/// dominant du bundler/fill pattern).
fn readable_stream_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let source = arg(args, 0);
    let (mut pull, mut cancel, mut start) = (None, None, None);
    if let Value::Obj(sid) = source {
        pull = e.get_prop(sid, "pull");
        cancel = e.get_prop(sid, "cancel");
        start = e.get_prop(sid, "start");
    }
    let d = Rc::new(RefCell::new(RsData {
        queue: VecDeque::new(),
        closed: false,
        errored: None,
        pull: pull.filter(|&f| e.is_callable(f)),
        cancel: cancel.filter(|&f| e.is_callable(f)),
        source,
        locked: false,
        pending_reads: Vec::new(),
    }));
    let rp = rs_proto(e);
    let v = rs_obj(e, &d, rp);
    if let Some(s) = start.filter(|&f| e.is_callable(f)) {
        let cp = rs_ctrl_proto(e);
        let ctrl = rs_obj(e, &d, cp);
        // `start` peut throw → le stream part en échec (spec :
        // rejet reporté, ici propagé — pas de fetch en cours).
        if let Err(err) = e.call_value(s, source, &[ctrl]) {
            let crate::heap::Thrown(v) = err;
            d.borrow_mut().errored = Some(v);
        }
    }
    Ok(v)
}

/// `rs.locked` / `ctrl.desiredSize` — getters host.
impl RsHost {
    /// `desiredSize` spec = `hwm - queue.len` — `hwm` par défaut
    /// 1 (strategy non modélisée : file mémoire bornée).
    fn desired_size(d: &RsData) -> f64 {
        1.0 - d.queue.len() as f64
    }
}

/// `{value, done}` d'un résultat `read()` spec.
fn iter_result(e: &mut Engine, value: Value, done: bool) -> Value {
    let o = e.new_object();
    e.set_own(o, "value", value, true);
    e.set_own(o, "done", Value::Bool(done), true);
    Value::Obj(o)
}

/// Résout les `read()` pendantes d'un `RsData` tant que la
/// queue fournit (appelé après `enqueue`/`close`/`error` et au
/// checkpoint). Retourne `true` si des promesses restent
/// pendantes.
fn drain_pending(e: &mut Engine, d: &Rc<RefCell<RsData>>) -> bool {
    loop {
        let next = {
            let mut dd = d.borrow_mut();
            if dd.pending_reads.is_empty() {
                return false;
            }
            if let Some(err) = dd.errored {
                let pid = dd.pending_reads.remove(0);
                Some((pid, Err(err)))
            } else if let Some(v) = dd.queue.pop_front() {
                let pid = dd.pending_reads.remove(0);
                Some((pid, Ok(Some(v))))
            } else if dd.closed {
                let pid = dd.pending_reads.remove(0);
                Some((pid, Ok(None)))
            } else {
                return true;
            }
        };
        match next {
            Some((pid, Ok(Some(v)))) => {
                let r = iter_result(e, v, false);
                e.resolve_promise(pid, r);
            }
            Some((pid, Ok(None))) => {
                let r = iter_result(e, Value::Undefined, true);
                e.resolve_promise(pid, r);
            }
            Some((pid, Err(err))) => e.reject_promise(pid, err),
            None => return false,
        }
    }
}

/// `ctrl.enqueue(chunk)` — chunk en queue ou livré directement
/// à une `read()` pendante (ordre spec).
fn ctrl_enqueue(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Some(d) = rs_data(e, this) else {
        return Err(e.type_error("enqueue on incompatible receiver"));
    };
    {
        let dd = d.borrow();
        if dd.closed || dd.errored.is_some() {
            return Err(e.type_error("enqueue on a closed stream"));
        }
    }
    d.borrow_mut().queue.push_back(arg(args, 0));
    drain_pending(e, &d);
    Ok(Value::Undefined)
}

/// `ctrl.close()` — vide la queue puis `done`.
fn ctrl_close(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let Some(d) = rs_data(e, this) else {
        return Err(e.type_error("close on incompatible receiver"));
    };
    {
        let dd = d.borrow();
        if dd.closed || dd.errored.is_some() {
            return Ok(Value::Undefined);
        }
    }
    d.borrow_mut().closed = true;
    drain_pending(e, &d);
    Ok(Value::Undefined)
}

/// `ctrl.error(e)` — échec irréversible, purge la queue.
fn ctrl_error(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Some(d) = rs_data(e, this) else {
        return Err(e.type_error("error on incompatible receiver"));
    };
    {
        let mut dd = d.borrow_mut();
        dd.errored = Some(arg(args, 0));
        dd.queue.clear();
    }
    drain_pending(e, &d);
    Ok(Value::Undefined)
}

/// `rs.getReader()` — un seul reader (lock spec).
fn rs_get_reader(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let Some(d) = rs_data(e, this) else {
        return Err(e.type_error("getReader on incompatible receiver"));
    };
    if d.borrow().locked {
        return Err(e.type_error("ReadableStream is locked"));
    }
    d.borrow_mut().locked = true;
    let rp = reader_proto(e);
    let v = rs_obj(e, &d, rp);
    // `reader.closed` — promesse own-prop, résolue à la fin du
    // stream (approximation : résolue à `releaseLock`/`done`).
    Ok(v)
}

/// `rs.cancel(reason?)` — purge + `source.cancel(reason)`.
fn rs_cancel(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Some(d) = rs_data(e, this) else {
        return Err(e.type_error("cancel on incompatible receiver"));
    };
    cancel_stream(e, &d, arg(args, 0))?;
    Ok(Value::Undefined)
}

/// Annulation partagée `rs.cancel`/`reader.cancel` — purge la
/// queue, `closed`, appelle `source.cancel`.
fn cancel_stream(e: &mut Engine, d: &Rc<RefCell<RsData>>, reason: Value) -> JsResult<()> {
    let (cancel, source) = {
        let mut dd = d.borrow_mut();
        dd.queue.clear();
        dd.closed = true;
        (dd.cancel, dd.source)
    };
    if let Some(f) = cancel {
        e.call_value(f, source, &[reason])?;
    }
    drain_pending(e, d);
    Ok(())
}

/// `reader.releaseLock()` — déverrouille (lectures pendantes
/// restent pendantes — le prochain reader les hérite par `Rc`).
fn reader_release(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let Some(d) = rs_data(e, this) else {
        return Err(e.type_error("releaseLock on incompatible receiver"));
    };
    d.borrow_mut().locked = false;
    Ok(Value::Undefined)
}

/// `reader.cancel(reason?)` — identique à `rs.cancel` mais en
/// promesse (spec) — résolue après `source.cancel`.
fn reader_cancel(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Some(d) = rs_data(e, this) else {
        return Err(e.type_error("cancel on incompatible receiver"));
    };
    let pid = e.new_promise();
    match cancel_stream(e, &d, arg(args, 0)) {
        Ok(()) => e.resolve_promise(pid, Value::Undefined),
        Err(crate::heap::Thrown(v)) => e.reject_promise(pid, v),
    }
    Ok(Value::Obj(pid))
}

/// `reader.read()` — chunk → `{value,done:false}` ; queue vide
/// → `pull()` puis re-check ; toujours vide → promesse pendante
/// (résolue par `enqueue`/`close`/`error` ou au checkpoint).
fn reader_read(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let Some(d) = rs_data(e, this) else {
        return Err(e.type_error("read on incompatible receiver"));
    };
    let pid = e.new_promise();
    // État immédiat — erreur / chunk / fin.
    {
        let mut dd = d.borrow_mut();
        if let Some(err) = dd.errored {
            e.reject_promise(pid, err);
            return Ok(Value::Obj(pid));
        }
        if let Some(v) = dd.queue.pop_front() {
            drop(dd);
            let r = iter_result(e, v, false);
            e.resolve_promise(pid, r);
            return Ok(Value::Obj(pid));
        }
        if dd.closed {
            drop(dd);
            let r = iter_result(e, Value::Undefined, true);
            e.resolve_promise(pid, r);
            return Ok(Value::Obj(pid));
        }
        dd.pending_reads.push(pid);
    }
    // Queue vide → tenter `pull` une fois (le checkpoint
    // re-tente tant que la lecture reste pendante).
    let (pull, source) = {
        let dd = d.borrow();
        (dd.pull, dd.source)
    };
    if let Some(f) = pull {
        let cp = rs_ctrl_proto(e);
        let ctrl = rs_obj(e, &d, cp);
        if let Err(err) = e.call_value(f, source, &[ctrl]) {
            let crate::heap::Thrown(v) = err;
            d.borrow_mut().errored = Some(v);
        }
    }
    drain_pending(e, &d);
    // Toujours pendante → re-`pull` au checkpoint microtask.
    if d.borrow().pending_reads.contains(&pid) {
        if let Value::Obj(wid) = this {
            register_pending_stream(e, wid);
        }
    }
    Ok(Value::Obj(pid))
}

/// Enregistre le stream porteur du reader pour re-`pull` au
/// checkpoint — le wrapper porteur est le *stream* (`d` est
/// partagé reader↔stream mais `pending_reads` vit dans `d` :
/// on enregistre le reader lui-même, `rs_data` le résout).
fn register_pending_stream(e: &mut Engine, wrapper: ObjId) {
    if !e.streams.pending_streams.contains(&wrapper) {
        e.streams.pending_streams.push(wrapper);
    }
}

impl Engine {
    /// Checkpoint microtask (JS-C71) — re-`pull` des
    /// `ReadableStream` ayant des `read()` pendantes. Même
    /// couture que `FileReader` : le JS peut enqueuer de
    /// nouveaux chunks à chaque passe.
    ///
    /// # Errors
    /// Propage les exceptions des `pull`/handlers.
    pub(crate) fn pump_stream_reads(&mut self) -> JsResult<usize> {
        pump_pending_reads(self)
    }
}

/// Livraison checkpoint : re-`pull` les streams avec lectures
/// pendantes. Retourne le nombre de streams encore en attente.
fn pump_pending_reads(e: &mut Engine) -> JsResult<usize> {
    let ws = std::mem::take(&mut e.streams.pending_streams);
    let mut still = 0usize;
    for wid in ws {
        let Some(d) = rs_data(e, Value::Obj(wid)) else {
            continue;
        };
        // Rien en attente → retirer.
        if d.borrow().pending_reads.is_empty() {
            continue;
        }
        let (pull, source) = {
            let dd = d.borrow();
            (dd.pull, dd.source)
        };
        if let Some(f) = pull {
            let cp = rs_ctrl_proto(e);
            let ctrl = rs_obj(e, &d, cp);
            if let Err(err) = e.call_value(f, source, &[ctrl]) {
                let crate::heap::Thrown(v) = err;
                d.borrow_mut().errored = Some(v);
            }
        }
        if drain_pending(e, &d) {
            // Toujours des lectures pendantes.
            e.streams.pending_streams.push(wid);
            still += 1;
        }
    }
    Ok(still)
}

/// `rs.pipeTo(dest)` — drain synchrone : chunks → `writer.write`,
/// fin → `writer.close()` ; erreur source → `writer.abort`.
/// Approximation documentée : pas de backpressure asynchrone.
fn rs_pipe_to(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Some(d) = rs_data(e, this) else {
        return Err(e.type_error("pipeTo on incompatible receiver"));
    };
    let pid = e.new_promise();
    match pipe_drain(e, &d, arg(args, 0)) {
        Ok(()) => e.resolve_promise(pid, Value::Undefined),
        Err(crate::heap::Thrown(v)) => e.reject_promise(pid, v),
    }
    Ok(Value::Obj(pid))
}

/// Drain partagé `pipeTo`/`pipeThrough` : writer via
/// `getWriter()` si possible, appels `write`/`close`/`abort`.
fn pipe_drain(e: &mut Engine, d: &Rc<RefCell<RsData>>, dest: Value) -> JsResult<()> {
    let Value::Obj(did) = dest else {
        return Err(e.type_error("pipeTo: destination is not a WritableStream"));
    };
    let writer = match e.get_prop(did, "getWriter") {
        Some(g) if e.is_callable(g) => e.call_value(g, dest, &[])?,
        _ => dest, // sink nu (TransformStream.writable interne)
    };
    loop {
        let next = {
            let mut dd = d.borrow_mut();
            if let Some(err) = dd.errored {
                Some(Err(err))
            } else if let Some(v) = dd.queue.pop_front() {
                Some(Ok(Some(v)))
            } else if dd.closed {
                Some(Ok(None))
            } else {
                None
            }
        };
        match next {
            Some(Ok(Some(v))) => {
                if let Ok(w) = e.get_prop_value(writer, "write") {
                    if e.is_callable(w) {
                        let r = e.call_value(w, writer, &[v])?;
                        let _ = e.await_value(r)?; // promesse sink
                    }
                }
            }
            Some(Ok(None)) => {
                if let Ok(c) = e.get_prop_value(writer, "close") {
                    if e.is_callable(c) {
                        let r = e.call_value(c, writer, &[])?;
                        let _ = e.await_value(r)?;
                    }
                }
                return Ok(());
            }
            Some(Err(err)) => {
                if let Ok(a) = e.get_prop_value(writer, "abort") {
                    if e.is_callable(a) {
                        let r = e.call_value(a, writer, &[err])?;
                        let _ = e.await_value(r)?;
                    }
                }
                return Ok(());
            }
            None => {
                // Queue vide, pas fermé : `pull` jusqu'à
                // terminaison (borné par le budget ops).
                let cp = rs_ctrl_proto(e);
                let (pull, source, ctrl) = {
                    let dd = d.borrow();
                    (dd.pull, dd.source, rs_obj(e, d, cp))
                };
                match pull {
                    Some(f) => {
                        e.call_value(f, source, &[ctrl])?;
                        e.tick()?;
                    }
                    None => {
                        // Ni chunk ni pull : la promesse pipe
                        // resterait pendante — erreur honnête.
                        return Err(e.type_error("pipeTo: source stalled without pull"));
                    }
                }
            }
        }
    }
}

/// `rs.pipeThrough({readable, writable})` = `pipeTo(writable)`
/// puis `readable` retourné (spec).
fn rs_pipe_through(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Some(d) = rs_data(e, this) else {
        return Err(e.type_error("pipeThrough on incompatible receiver"));
    };
    let t = arg(args, 0);
    let Value::Obj(tid) = t else {
        return Err(e.type_error("pipeThrough: not a transform"));
    };
    let w = e.get_prop(tid, "writable").unwrap_or(Value::Undefined);
    pipe_drain(e, &d, w)?;
    Ok(e.get_prop(tid, "readable").unwrap_or(Value::Undefined))
}

/// `rs.tee()` — deux branches recevant chacune une copie de la
/// queue restante (contenu mémoire ; la source n'est plus
/// tirée — approximation documentée). Le stream devient locked.
fn rs_tee(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let Some(d) = rs_data(e, this) else {
        return Err(e.type_error("tee on incompatible receiver"));
    };
    {
        let mut dd = d.borrow_mut();
        if dd.locked {
            return Err(e.type_error("ReadableStream is locked"));
        }
        dd.locked = true;
    }
    let make = |e: &mut Engine| {
        let (queue, closed, errored) = {
            let dd = d.borrow();
            (dd.queue.clone(), dd.closed, dd.errored)
        };
        let nd = Rc::new(RefCell::new(RsData {
            queue,
            closed,
            errored,
            pull: None,
            cancel: None,
            source: Value::Undefined,
            locked: false,
            pending_reads: Vec::new(),
        }));
        let p = rs_proto(e);
        rs_obj(e, &nd, p)
    };
    let a = make(e);
    let b = make(e);
    Ok(e.new_array(vec![a, b]))
}

/// `[Symbol.asyncIterator]` — `{next}` adossé à un reader neuf
/// (un `for await` = un reader ; le stream se verrouille).
fn rs_async_iter(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let reader = rs_get_reader(e, this, &[])?;
    // Itérateur = objet `{next: bound read}` — chaque `next()`
    // retourne la promesse `read()` (awaitée par le moteur).
    let iid = e.new_object();
    let nf = e.bound_native("next", iter_next, vec![reader]);
    e.set_own(iid, "next", nf, true);
    Ok(Value::Obj(iid))
}

/// `it.next()` → `reader.read()` (bound `[reader]`).
fn iter_next(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    reader_read(e, arg(args, 0), &[])
}

impl Host for RsHost {
    fn class_name(&self) -> &'static str {
        "ReadableStream"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn get(&self, _e: &mut Engine, _t: Value, key: &str) -> JsResult<Option<Value>> {
        let d = self.d.borrow();
        Ok(Some(match key {
            "locked" => Value::Bool(d.locked),
            "desiredSize" => Value::Num(Self::desired_size(&d)),
            _ => return Ok(None),
        }))
    }

    fn trace(&self, out: &mut Vec<ObjId>) {
        let d = self.d.borrow();
        for v in &d.queue {
            crate::heap::push_value(out, *v);
        }
        for v in [d.pull, d.cancel, d.errored].into_iter().flatten() {
            crate::heap::push_value(out, v);
        }
        crate::heap::push_value(out, d.source);
        out.extend(d.pending_reads.iter().copied());
    }
}

// ------------------------------------------------------------------
// WritableStream
// ------------------------------------------------------------------

/// État partagé d'un `WritableStream` (stream ↔ writer ↔ ctrl).
struct WsData {
    /// `sink.write(chunk, ctrl)` — appelé par `writer.write`.
    write: Option<Value>,
    /// `sink.close()` — appelé par `writer.close`.
    close: Option<Value>,
    /// `sink.abort(reason)` — appelé par `writer.abort`.
    abort: Option<Value>,
    /// `underlyingSink` (this des callbacks).
    sink: Value,
    /// Writer verrouillé (`locked`).
    locked: bool,
    /// `true` après `close`/`abort` — les `write` sont rejetés.
    done: bool,
    /// Raison d'`abort` / erreur du controller.
    errored: Option<Value>,
    /// Promesse `writer.closed` — résolue au `close()`,
    /// rejetée à l'`error`/`abort`.
    closed_promise: Option<ObjId>,
}

/// Host `WritableStream` + `WritableStreamDefaultWriter` +
/// `WritableStreamDefaultController` — même `Rc<WsData>`.
struct WsHost {
    d: Rc<RefCell<WsData>>,
}

/// `WsData` d'un `Value` stream/writer/controller.
fn ws_data(e: &Engine, v: Value) -> Option<Rc<RefCell<WsData>>> {
    let Value::Obj(id) = v else { return None };
    match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::Host(h)) => h
            .clone()
            .as_any()
            .downcast_ref::<WsHost>()
            .map(|r| r.d.clone()),
        _ => None,
    }
}

fn ws_proto(e: &mut Engine) -> ObjId {
    if let Some(p) = e.streams.ws_proto {
        return p;
    }
    let p = new_tagged_proto(e, "WritableStream");
    e.add_method(p, "getWriter", ws_get_writer);
    e.add_method(p, "abort", ws_abort);
    e.add_method(p, "close", ws_close);
    e.streams.ws_proto = Some(p);
    p
}

fn writer_proto(e: &mut Engine) -> ObjId {
    if let Some(p) = e.streams.writer_proto {
        return p;
    }
    let p = new_tagged_proto(e, "WritableStreamDefaultWriter");
    e.add_method(p, "write", writer_write);
    e.add_method(p, "close", writer_close);
    e.add_method(p, "abort", writer_abort);
    e.add_method(p, "releaseLock", writer_release);
    e.streams.writer_proto = Some(p);
    p
}

fn ws_ctrl_proto(e: &mut Engine) -> ObjId {
    if let Some(p) = e.streams.ws_ctrl_proto {
        return p;
    }
    let p = new_tagged_proto(e, "WritableStreamDefaultController");
    e.add_method(p, "error", ws_ctrl_error);
    e.streams.ws_ctrl_proto = Some(p);
    p
}

/// Alloue un host `WsHost` avec `proto` donné.
fn ws_obj(e: &mut Engine, d: &Rc<RefCell<WsData>>, proto: ObjId) -> Value {
    let v = e.new_host_object(Rc::new(WsHost { d: d.clone() }));
    if let Value::Obj(id) = v {
        if let Some(o) = e.heap.obj_mut(id) {
            o.proto = Value::Obj(proto);
        }
    }
    v
}

/// `new WritableStream(sink?, strategy?)` — `start(controller)`
/// appelé au construct (spec).
fn writable_stream_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let sink = arg(args, 0);
    let (mut write, mut close, mut abort, mut start) = (None, None, None, None);
    if let Value::Obj(sid) = sink {
        write = e.get_prop(sid, "write");
        close = e.get_prop(sid, "close");
        abort = e.get_prop(sid, "abort");
        start = e.get_prop(sid, "start");
    }
    let d = Rc::new(RefCell::new(WsData {
        write: write.filter(|&f| e.is_callable(f)),
        close: close.filter(|&f| e.is_callable(f)),
        abort: abort.filter(|&f| e.is_callable(f)),
        sink,
        locked: false,
        done: false,
        errored: None,
        closed_promise: None,
    }));
    let wp = ws_proto(e);
    let v = ws_obj(e, &d, wp);
    if let Some(s) = start.filter(|&f| e.is_callable(f)) {
        let cp = ws_ctrl_proto(e);
        let ctrl = ws_obj(e, &d, cp);
        if let Err(err) = e.call_value(s, sink, &[ctrl]) {
            let crate::heap::Thrown(v) = err;
            d.borrow_mut().errored = Some(v);
        }
    }
    Ok(v)
}

/// `ws.getWriter()` — un seul writer ; porte `ready`/`closed`.
fn ws_get_writer(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let Some(d) = ws_data(e, this) else {
        return Err(e.type_error("getWriter on incompatible receiver"));
    };
    if d.borrow().locked {
        return Err(e.type_error("WritableStream is locked"));
    }
    d.borrow_mut().locked = true;
    let wp = writer_proto(e);
    let v = ws_obj(e, &d, wp);
    if let Value::Obj(wid) = v {
        // `writer.ready` — résolu immédiate (file mémoire) ;
        // `writer.closed` — résolu au `close()`, rejeté sinon.
        let ready = e.new_promise();
        e.resolve_promise(ready, Value::Undefined);
        e.set_own(wid, "ready", Value::Obj(ready), true);
        let closed = e.new_promise();
        d.borrow_mut().closed_promise = Some(closed);
        e.set_own(wid, "closed", Value::Obj(closed), true);
    }
    Ok(v)
}

/// Chaîne une promesse de sink : résolue → resolve `undefined`,
/// rejetée → reject, pendante → reject honnête (limite
/// `await_value`).
fn settle_from(e: &mut Engine, pid: ObjId, r: &JsResult<Value>) {
    match r {
        Ok(v) => match e.await_value(*v) {
            Ok(_) => e.resolve_promise(pid, Value::Undefined),
            Err(crate::heap::Thrown(err)) => e.reject_promise(pid, err),
        },
        Err(crate::heap::Thrown(err)) => e.reject_promise(pid, *err),
    }
}

/// `writer.write(chunk)` → `sink.write(chunk, ctrl)` en promesse.
fn writer_write(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Some(d) = ws_data(e, this) else {
        return Err(e.type_error("write on incompatible receiver"));
    };
    let pid = e.new_promise();
    let (write, sink, err, done) = {
        let dd = d.borrow();
        (dd.write, dd.sink, dd.errored, dd.done)
    };
    if let Some(err) = err {
        e.reject_promise(pid, err);
        return Ok(Value::Obj(pid));
    }
    if done {
        let m = e.str_val("write on a closed stream");
        e.reject_promise(pid, m);
        return Ok(Value::Obj(pid));
    }
    match write {
        Some(f) => {
            let cp = ws_ctrl_proto(e);
            let ctrl = ws_obj(e, &d, cp);
            let r = e.call_value(f, sink, &[arg(args, 0), ctrl]);
            settle_from(e, pid, &r);
        }
        None => e.resolve_promise(pid, Value::Undefined),
    }
    Ok(Value::Obj(pid))
}

/// `writer.close()` → `sink.close()` ; résout `closed`.
fn writer_close(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let Some(d) = ws_data(e, this) else {
        return Err(e.type_error("close on incompatible receiver"));
    };
    let pid = e.new_promise();
    let (close, sink) = {
        let mut dd = d.borrow_mut();
        dd.done = true;
        (dd.close, dd.sink)
    };
    let r = match close {
        Some(f) => e.call_value(f, sink, &[]),
        None => Ok(Value::Undefined),
    };
    settle_from(e, pid, &r);
    if let Some(cp) = d.borrow().closed_promise {
        e.resolve_promise(cp, Value::Undefined);
    }
    Ok(Value::Obj(pid))
}

/// `writer.abort(reason)` / `ws.abort` → `sink.abort(reason)` ;
/// `closed` rejeté (spec : résolu en fait — abort résout
/// `closed` avec `undefined` ; on choisit la résolution spec).
fn writer_abort(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Some(d) = ws_data(e, this) else {
        return Err(e.type_error("abort on incompatible receiver"));
    };
    let pid = e.new_promise();
    let reason = arg(args, 0);
    let (abort, sink) = {
        let mut dd = d.borrow_mut();
        dd.done = true;
        dd.errored = Some(reason);
        (dd.abort, dd.sink)
    };
    let r = match abort {
        Some(f) => e.call_value(f, sink, &[reason]),
        None => Ok(Value::Undefined),
    };
    settle_from(e, pid, &r);
    if let Some(cp) = d.borrow().closed_promise {
        e.resolve_promise(cp, Value::Undefined);
    }
    Ok(Value::Obj(pid))
}

/// `ws.abort`/`ws.close` — raccourcis stream-level (spec).
fn ws_abort(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    writer_abort(e, this, args)
}

fn ws_close(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    writer_close(e, this, &[])
}

/// `writer.releaseLock()`.
fn writer_release(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let Some(d) = ws_data(e, this) else {
        return Err(e.type_error("releaseLock on incompatible receiver"));
    };
    d.borrow_mut().locked = false;
    Ok(Value::Undefined)
}

/// `wctrl.error(e)` — stream en échec, `write` suivants rejetés.
fn ws_ctrl_error(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Some(d) = ws_data(e, this) else {
        return Err(e.type_error("error on incompatible receiver"));
    };
    let reason = arg(args, 0);
    let mut dd = d.borrow_mut();
    dd.errored = Some(reason);
    dd.done = true;
    if let Some(cp) = dd.closed_promise {
        drop(dd);
        e.reject_promise(cp, reason);
    }
    Ok(Value::Undefined)
}

impl Host for WsHost {
    fn class_name(&self) -> &'static str {
        "WritableStream"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn get(&self, _e: &mut Engine, _t: Value, key: &str) -> JsResult<Option<Value>> {
        let d = self.d.borrow();
        Ok(Some(match key {
            "locked" => Value::Bool(d.locked),
            "desiredSize" => Value::Num(if d.done { 0.0 } else { 1.0 }),
            _ => return Ok(None),
        }))
    }

    fn trace(&self, out: &mut Vec<ObjId>) {
        let d = self.d.borrow();
        for v in [d.write, d.close, d.abort, d.errored].into_iter().flatten() {
            crate::heap::push_value(out, v);
        }
        crate::heap::push_value(out, d.sink);
        if let Some(cp) = d.closed_promise {
            out.push(cp);
        }
    }
}

// ------------------------------------------------------------------
// TransformStream
// ------------------------------------------------------------------

/// État partagé d'un `TransformStream` (writable ↔ readable).
struct TsData {
    /// `transformer.transform(chunk, ctrl)`.
    transform: Option<Value>,
    /// `transformer.flush(ctrl)` — à `writable.close()`.
    flush: Option<Value>,
    /// `transformer` (this des callbacks).
    transformer: Value,
    /// `RsData` du côté readable (enqueue par le controller).
    rs: Rc<RefCell<RsData>>,
}

/// Host `TransformStream` — porte `readable`/`writable` en
/// own-props ; le sink côté writable est synthétique (`TsSink`).
struct TsHost {
    d: Rc<RefCell<TsData>>,
}

/// Host `TransformStreamDefaultController` — `enqueue`/`error`/
/// `terminate` vers le readable.
struct TsCtrlHost {
    d: Rc<RefCell<TsData>>,
}

/// Sink synthétique côté writable — `write` → `transform`,
/// `close` → `flush` + `readable.close`.
struct TsSinkHost {
    d: Rc<RefCell<TsData>>,
}

/// `TsData`/`TsCtrlHost` partagés — `None` sinon.
fn ts_data(e: &Engine, v: Value) -> Option<Rc<RefCell<TsData>>> {
    let Value::Obj(id) = v else { return None };
    match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::Host(h)) => {
            let any = h.clone();
            let any = any.as_any();
            if let Some(t) = any.downcast_ref::<TsHost>() {
                Some(t.d.clone())
            } else if let Some(t) = any.downcast_ref::<TsCtrlHost>() {
                Some(t.d.clone())
            } else {
                any.downcast_ref::<TsSinkHost>().map(|t| t.d.clone())
            }
        }
        _ => None,
    }
}

fn ts_proto(e: &mut Engine) -> ObjId {
    if let Some(p) = e.streams.ts_proto {
        return p;
    }
    let p = new_tagged_proto(e, "TransformStream");
    e.streams.ts_proto = Some(p);
    p
}

fn ts_ctrl_proto(e: &mut Engine) -> ObjId {
    if let Some(p) = e.streams.ts_ctrl_proto {
        return p;
    }
    let p = new_tagged_proto(e, "TransformStreamDefaultController");
    e.add_method(p, "enqueue", ts_ctrl_enqueue);
    e.add_method(p, "error", ts_ctrl_error);
    e.add_method(p, "terminate", ts_ctrl_terminate);
    e.streams.ts_ctrl_proto = Some(p);
    p
}

/// `new TransformStream(transformer?, wsStrategy?, rsStrategy?)`
/// — `transformer.start(controller)` au construct (spec).
fn transform_stream_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let transformer = arg(args, 0);
    let (mut transform, mut flush, mut start) = (None, None, None);
    if let Value::Obj(tid) = transformer {
        transform = e.get_prop(tid, "transform");
        flush = e.get_prop(tid, "flush");
        start = e.get_prop(tid, "start");
    }
    let d = Rc::new(RefCell::new(TsData {
        transform: transform.filter(|&f| e.is_callable(f)),
        flush: flush.filter(|&f| e.is_callable(f)),
        transformer,
        rs: Rc::new(RefCell::new(RsData {
            queue: VecDeque::new(),
            closed: false,
            errored: None,
            pull: None,
            cancel: None,
            source: Value::Undefined,
            locked: false,
            pending_reads: Vec::new(),
        })),
    }));
    // Côtés : readable = ReadableStream sur `d.rs` ; writable =
    // WritableStream adossé au sink synthétique.
    let rs = d.borrow().rs.clone();
    let rp = rs_proto(e);
    let readable = rs_obj(e, &rs, rp);
    let writable = ts_writable(e, &d);
    let tp = ts_proto(e);
    let v = e.new_host_object(Rc::new(TsHost { d: d.clone() }));
    if let Value::Obj(id) = v {
        if let Some(o) = e.heap.obj_mut(id) {
            o.proto = Value::Obj(tp);
        }
        e.set_own(id, "readable", readable, true);
        e.set_own(id, "writable", writable, true);
    }
    if let Some(s) = start.filter(|&f| e.is_callable(f)) {
        let ctrl = ts_ctrl(e, &d);
        if let Err(err) = e.call_value(s, transformer, &[ctrl]) {
            let crate::heap::Thrown(v) = err;
            d.borrow().rs.borrow_mut().errored = Some(v);
        }
    }
    Ok(v)
}

/// Controller `TransformStream` (proto dédié + `TsCtrlHost`).
fn ts_ctrl(e: &mut Engine, d: &Rc<RefCell<TsData>>) -> Value {
    let cp = ts_ctrl_proto(e);
    let v = e.new_host_object(Rc::new(TsCtrlHost { d: d.clone() }));
    if let Value::Obj(id) = v {
        if let Some(o) = e.heap.obj_mut(id) {
            o.proto = Value::Obj(cp);
        }
    }
    v
}

/// Writable synthétique du transform — `WsData` dont
/// `write`/`close` appellent le transformer.
fn ts_writable(e: &mut Engine, d: &Rc<RefCell<TsData>>) -> Value {
    let wd = Rc::new(RefCell::new(WsData {
        // Callbacks marqués par un sink host — `write`/`close`
        // sont des natives liées au `TsSinkHost` (le `sink` est
        // l'objet host, `this` du call = sink).
        write: None,
        close: None,
        abort: None,
        sink: Value::Undefined,
        locked: false,
        done: false,
        errored: None,
        closed_promise: None,
    }));
    let op = e.protos.object;
    let sink = e.new_host_object(Rc::new(TsSinkHost { d: d.clone() }));
    if let Value::Obj(sid) = sink {
        if let Some(o) = e.heap.obj_mut(sid) {
            o.proto = Value::Obj(op);
        }
        e.add_method(sid, "write", ts_sink_write);
        e.add_method(sid, "close", ts_sink_close);
        e.add_method(sid, "abort", ts_sink_abort);
    }
    wd.borrow_mut().write = e.get_prop_value(sink, "write").ok();
    wd.borrow_mut().close = e.get_prop_value(sink, "close").ok();
    wd.borrow_mut().abort = e.get_prop_value(sink, "abort").ok();
    wd.borrow_mut().sink = sink;
    let wp = ws_proto(e);
    ws_obj(e, &wd, wp)
}

/// `sink.write(chunk, wctrl)` → `transformer.transform(chunk,
/// tsCtrl)` ; sans `transform` → identité (enqueue direct).
fn ts_sink_write(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Some(d) = ts_data(e, this) else {
        return Err(e.type_error("transform write on incompatible receiver"));
    };
    let (transform, transformer) = {
        let dd = d.borrow();
        (dd.transform, dd.transformer)
    };
    let chunk = arg(args, 0);
    if let Some(f) = transform {
        let ctrl = ts_ctrl(e, &d);
        e.call_value(f, transformer, &[chunk, ctrl])
    } else {
        // Transform identité — chunk traversant.
        d.borrow().rs.borrow_mut().queue.push_back(chunk);
        let rs = d.borrow().rs.clone();
        drain_pending(e, &rs);
        Ok(Value::Undefined)
    }
}

/// `sink.close()` → `transformer.flush?(ctrl)` puis
/// `readable.close()`.
fn ts_sink_close(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let Some(d) = ts_data(e, this) else {
        return Err(e.type_error("transform close on incompatible receiver"));
    };
    let (flush, transformer) = {
        let dd = d.borrow();
        (dd.flush, dd.transformer)
    };
    if let Some(f) = flush {
        let ctrl = ts_ctrl(e, &d);
        e.call_value(f, transformer, &[ctrl])?;
    }
    let rs = d.borrow().rs.clone();
    rs.borrow_mut().closed = true;
    drain_pending(e, &rs);
    Ok(Value::Undefined)
}

/// `sink.abort(reason)` → `readable.error(reason)` (spec).
fn ts_sink_abort(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Some(d) = ts_data(e, this) else {
        return Err(e.type_error("transform abort on incompatible receiver"));
    };
    let rs = d.borrow().rs.clone();
    {
        let mut dd = rs.borrow_mut();
        dd.errored = Some(arg(args, 0));
        dd.queue.clear();
    }
    drain_pending(e, &rs);
    Ok(Value::Undefined)
}

/// `tsCtrl.enqueue(chunk)` → chunk dans le readable.
fn ts_ctrl_enqueue(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Some(d) = ts_data(e, this) else {
        return Err(e.type_error("enqueue on incompatible receiver"));
    };
    let rs = d.borrow().rs.clone();
    {
        let dd = rs.borrow();
        if dd.closed || dd.errored.is_some() {
            return Err(e.type_error("enqueue on a closed transform"));
        }
    }
    rs.borrow_mut().queue.push_back(arg(args, 0));
    drain_pending(e, &rs);
    Ok(Value::Undefined)
}

/// `tsCtrl.error(e)` → `readable.error(e)`.
fn ts_ctrl_error(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Some(d) = ts_data(e, this) else {
        return Err(e.type_error("error on incompatible receiver"));
    };
    let rs = d.borrow().rs.clone();
    {
        let mut dd = rs.borrow_mut();
        dd.errored = Some(arg(args, 0));
        dd.queue.clear();
    }
    drain_pending(e, &rs);
    Ok(Value::Undefined)
}

/// `tsCtrl.terminate()` → `readable.close()` + writable errored.
fn ts_ctrl_terminate(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let Some(d) = ts_data(e, this) else {
        return Err(e.type_error("terminate on incompatible receiver"));
    };
    let rs = d.borrow().rs.clone();
    rs.borrow_mut().closed = true;
    drain_pending(e, &rs);
    Ok(Value::Undefined)
}

impl Host for TsHost {
    fn class_name(&self) -> &'static str {
        "TransformStream"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn trace(&self, out: &mut Vec<ObjId>) {
        let d = self.d.borrow();
        for v in [d.transform, d.flush].into_iter().flatten() {
            crate::heap::push_value(out, v);
        }
        crate::heap::push_value(out, d.transformer);
        // `d.rs` est couvert par le host du readable exposé.
    }
}

impl Host for TsCtrlHost {
    fn class_name(&self) -> &'static str {
        "TransformStreamDefaultController"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn get(&self, _e: &mut Engine, _t: Value, key: &str) -> JsResult<Option<Value>> {
        let d = self.d.borrow();
        Ok(Some(match key {
            "desiredSize" => Value::Num(RsHost::desired_size(&d.rs.borrow())),
            _ => return Ok(None),
        }))
    }
    fn trace(&self, out: &mut Vec<ObjId>) {
        let d = self.d.borrow();
        for v in [d.transform, d.flush].into_iter().flatten() {
            crate::heap::push_value(out, v);
        }
        crate::heap::push_value(out, d.transformer);
        for v in &d.rs.borrow().queue {
            crate::heap::push_value(out, *v);
        }
    }
}

impl Host for TsSinkHost {
    fn class_name(&self) -> &'static str {
        "TransformStreamSink"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn trace(&self, out: &mut Vec<ObjId>) {
        let d = self.d.borrow();
        for v in [d.transform, d.flush].into_iter().flatten() {
            crate::heap::push_value(out, v);
        }
        crate::heap::push_value(out, d.transformer);
    }
}

// ------------------------------------------------------------------
// Corps fetch → stream + installation
// ------------------------------------------------------------------

/// `ReadableStream` adossé à un corps en mémoire — un chunk
/// `Uint8Array` unique puis `done` (corps réseau du transport
/// `String`, livré d'un bloc — approximation documentée).
pub(crate) fn body_stream(e: &mut Engine, bytes: &[u8]) -> Value {
    let d = Rc::new(RefCell::new(RsData {
        queue: VecDeque::new(),
        closed: true,
        errored: None,
        pull: None,
        cancel: None,
        source: Value::Undefined,
        locked: false,
        pending_reads: Vec::new(),
    }));
    if !bytes.is_empty() {
        let chunk = crate::typed_array::new_u8_array(e, bytes.to_vec());
        d.borrow_mut().queue.push_back(chunk);
    }
    let p = rs_proto(e);
    rs_obj(e, &d, p)
}

/// Installe `ReadableStream`/`WritableStream`/`TransformStream`
/// sur le global — idempotent.
pub(crate) fn install_streams(e: &mut Engine) {
    let g = e.global;
    if e.get_prop(g, "ReadableStream").is_some() {
        return;
    }
    for (name, f, proto) in [
        (
            "ReadableStream",
            readable_stream_ctor as crate::heap::NativeFn,
            rs_proto(e),
        ),
        ("WritableStream", writable_stream_ctor, ws_proto(e)),
        ("TransformStream", transform_stream_ctor, ts_proto(e)),
    ] {
        let fv = e.native_fn(name, f);
        if let Value::Obj(fid) = fv {
            e.set_own(fid, "prototype", Value::Obj(proto), false);
            e.set_own(proto, "constructor", fv, false);
        }
        e.set_own(g, name, fv, false);
    }
}
