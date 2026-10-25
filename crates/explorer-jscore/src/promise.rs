// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `Promise` (ES2015) + microtask queue.
//!
//! Model: a promise is an `ObjKind::Promise` cell carrying its state
//! and its `then` reactions; settling enqueues one [`Job`] per
//! reaction. Jobs are pumped by [`Engine::run_microtasks`] at every
//! job boundary — end of `eval`, after each timer callback, after
//! each event dispatch, after each XHR delivery — matching the spec's
//! "clean up after running script"/per-task microtask checkpoint.
//!
//! `async`/`await` is deferred: our tree-walker cannot suspend a
//! function mid-body without a CPS transform (needs generators or
//! bytecode). `Promise` + `then` chains cover the pre-2017 idiom.

use crate::heap::{JsResult, ObjKind, Thrown};
use crate::value::{ObjId, Value};
use crate::Engine;

/// Promise lifecycle.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PromiseState {
    /// Not settled.
    Pending,
    /// Resolved with `result`.
    Fulfilled,
    /// Rejected with `result` as reason.
    Rejected,
}

/// A `then` subscription.
pub struct Reaction {
    /// `onFulfilled` — callable or `Undefined`.
    pub on_fulfilled: Value,
    /// `onRejected` — callable or `Undefined`.
    pub on_rejected: Value,
    /// Promise returned by `then`, settled with the handler result.
    pub next: ObjId,
}

/// The `ObjKind::Promise` payload.
pub struct JsPromise {
    /// Current state.
    pub state: PromiseState,
    /// Fulfillment value / rejection reason.
    pub result: Value,
    /// Subscribed `then` reactions (drained on settle).
    pub reactions: Vec<Reaction>,
    /// Identity-stable `resolve`/`reject` functions bound to this
    /// promise — shared by executor call and thenable adoption.
    pub resolve_fn: Value,
    pub reject_fn: Value,
}

/// A queued microtask.
pub(crate) enum Job {
    /// Invoke a `then` handler and settle the chained promise.
    Reaction {
        /// `onFulfilled`/`onRejected` callback (or `Undefined` for
        /// pass-through), the settled arg, and the chained promise.
        cb: Value,
        /// Handler input (fulfillment value or rejection reason).
        arg: Value,
        /// `true` when the source promise rejected.
        rejected: bool,
        /// Promise returned by `then`.
        next: ObjId,
    },
    /// `thenable.then(resolve, reject)` — foreign thenable adoption.
    Thenable {
        /// The `then` callable.
        then_fn: Value,
        /// The thenable object (`this`).
        this: Value,
        /// Resolve/reject of the adopting promise.
        resolve: Value,
        /// See `resolve`.
        reject: Value,
    },
    /// `queueMicrotask(f)`.
    Plain(Value),
}

impl Engine {
    /// `new Promise()` internals: pending object + bound settle fns.
    pub(crate) fn new_promise(&mut self) -> ObjId {
        let id = self.heap.alloc_obj(crate::heap::Object {
            proto: Value::Obj(self.protos.promise),
            props: Vec::new(),
            // Placeholder: resolved below once the bound fns exist.
            kind: ObjKind::Promise(JsPromise {
                state: PromiseState::Pending,
                result: Value::Undefined,
                reactions: Vec::new(),
                resolve_fn: Value::Undefined,
                reject_fn: Value::Undefined,
            }),
            extensible: true,
            elem_integrity: 0,
            args_map: None,
        });
        let resolve = self.bound_native("resolve", promise_resolve, vec![Value::Obj(id)]);
        let reject = self.bound_native("reject", promise_reject, vec![Value::Obj(id)]);
        if let Some(o) = self.heap.obj_mut(id) {
            if let ObjKind::Promise(p) = &mut o.kind {
                p.resolve_fn = resolve;
                p.reject_fn = reject;
            }
        }
        id
    }

    /// `true` for `ObjKind::Promise` objects.
    pub(crate) fn is_promise(&self, v: Value) -> bool {
        matches!(v, Value::Obj(id) if matches!(self.heap.obj(id).map(|o| &o.kind), Some(ObjKind::Promise(_))))
    }

    /// Settle `id` — enqueues its reactions as microtasks. No-op on
    /// a non-promise or an already-settled promise.
    pub(crate) fn settle_promise(&mut self, id: ObjId, state: PromiseState, result: Value) {
        let reactions = {
            let Some(o) = self.heap.obj_mut(id) else {
                return;
            };
            let ObjKind::Promise(p) = &mut o.kind else {
                return;
            };
            if p.state != PromiseState::Pending {
                return;
            }
            p.state = state;
            p.result = result;
            std::mem::take(&mut p.reactions)
        };
        for r in reactions {
            let rejected = state == PromiseState::Rejected;
            self.microtasks.push_back((
                self.cur_ctx,
                Job::Reaction {
                    cb: if rejected {
                        r.on_rejected
                    } else {
                        r.on_fulfilled
                    },
                    arg: result,
                    rejected,
                    next: r.next,
                },
            ));
        }
    }

    /// `resolve` semantics: promise adoption, thenable `then` call
    /// (deferred to a microtask), plain fulfillment otherwise.
    pub(crate) fn resolve_promise(&mut self, id: ObjId, v: Value) {
        // Self-resolution cycle → TypeError.
        if matches!(v, Value::Obj(x) if x == id) {
            let e = self.error_value(
                self.protos.type_error,
                "TypeError",
                "promise cannot resolve to itself",
            );
            self.settle_promise(id, PromiseState::Rejected, e);
            return;
        }
        if self.is_promise(v) {
            // Adopt `v`: subscribe our own resolve/reject — re-entering
            // resolve_promise handles thenable chains naturally.
            let Value::Obj(vid) = v else { return };
            let (state, result) = {
                let Some(o) = self.heap.obj(vid) else { return };
                let ObjKind::Promise(p) = &o.kind else { return };
                (p.state, p.result)
            };
            match state {
                PromiseState::Pending => {
                    // Les réactions adoptantes doivent appeler les
                    // résolveurs de `id` (l'adoptante) — pas ceux de
                    // `vid` (no-op : déjà settlée). Bogue corrigé
                    // JS-C72 : `then` retournant une promesse issue
                    // d'un `.then` résolvait `undefined`.
                    let (my_resolve, my_reject) = {
                        let Some(o) = self.heap.obj(id) else { return };
                        let ObjKind::Promise(p) = &o.kind else { return };
                        (p.resolve_fn, p.reject_fn)
                    };
                    if let Some(o) = self.heap.obj_mut(vid) {
                        if let ObjKind::Promise(p) = &mut o.kind {
                            p.reactions.push(Reaction {
                                on_fulfilled: my_resolve,
                                on_rejected: my_reject,
                                // `next` factice — le
                                // `resolve_promise(next, retour_cb)`
                                // du job est un no-op (`id` déjà
                                // settlée par la réaction elle-même).
                                next: id,
                            });
                        }
                    }
                }
                // Already settled → adopt its outcome directly.
                PromiseState::Fulfilled | PromiseState::Rejected => {
                    self.settle_promise(id, state, result);
                }
            }
            return;
        }
        // Thenable adoption: `obj.then` callable → deferred call.
        if let Value::Obj(oid) = v {
            if let Some(then_fn) = self.get_prop(oid, "then") {
                if self.is_callable(then_fn) {
                    let (resolve, reject) = {
                        let Some(o) = self.heap.obj(id) else { return };
                        let ObjKind::Promise(p) = &o.kind else { return };
                        (p.resolve_fn, p.reject_fn)
                    };
                    self.microtasks.push_back((
                        self.cur_ctx,
                        Job::Thenable {
                            then_fn,
                            this: v,
                            resolve,
                            reject,
                        },
                    ));
                    return;
                }
            }
        }
        self.settle_promise(id, PromiseState::Fulfilled, v);
    }

    /// Reject path — plain settle.
    pub(crate) fn reject_promise(&mut self, id: ObjId, v: Value) {
        self.settle_promise(id, PromiseState::Rejected, v);
    }

    /// `queueMicrotask(f)` — also used internally for deferred work.
    pub(crate) fn enqueue_microtask(&mut self, f: Value) {
        self.microtasks.push_back((self.cur_ctx, Job::Plain(f)));
    }

    /// Drain the microtask queue until empty. Runs at every job
    /// boundary. Job count is bounded by `ops_left` so a self-feeding
    /// microtask loop dies like an infinite `while`.
    ///
    /// JS-C103 — borne **hostile-content** en plus du budget
    /// d'ops : une file auto-régénérante (scheduler de lib qui
    /// `queueMicrotask` en boucle) OU des livraisons qui ne se
    /// tassent jamais (`pump_stream_reads` d'un `ReadableStream`
    /// avec `read()` perpétuel, observers ré-armés — reddit
    /// mesuré : `delivered` vrai à chaque round sans toucher le
    /// budget ops → drain infini → `rx.recv()` sous verrou
    /// session → daemon gelé ~1 cœur) ne peuvent plus saturer un
    /// drain unique : les jobs restants restent en file et sont
    /// repris par le `run_pending` suivant — le progrès est
    /// préservé à travers les pompes, seule la rafale est bornée.
    ///
    /// # Errors
    /// Propagates the op-limit error or an uncaught `queueMicrotask`
    /// callback exception.
    pub fn run_microtasks(&mut self) -> JsResult<usize> {
        /// Jobs max par drain — large pour les chaînes légitimes
        /// (hydratation/diff de milliers de promesses), fini pour
        /// les files auto-régénérantes. Le reliquat attend le
        /// drain suivant.
        const MICROTASK_DRAIN_CAP: usize = 65_536;
        /// Rounds de livraison max par drain — une file de jobs
        /// vide mais des `deliver_*` perpétuellement productifs
        /// (stream ouvert, observers qui se réarment) sortent
        /// aussi : sans ce plafond la boucle externe ne revient
        /// jamais tester `microtasks.is_empty()`.
        const DELIVERY_ROUNDS_CAP: usize = 128;
        let mut ran = 0usize;
        let mut rounds = 0usize;
        let home = self.cur_ctx;
        loop {
            loop {
                if ran >= MICROTASK_DRAIN_CAP {
                    break;
                }
                let Some((ctx, job)) = self.microtasks.pop_front() else {
                    break;
                };
                // JS-C78 : le job court dans le realm qui l'a
                // enregistré (spec « job callback realm ») —
                // contexte garé → livraison différée : re-queue
                // si le contexte est mort (le switch no-op
                // garderait le courant et exécuterait le job
                // dans le mauvais realm).
                self.switch_to(ctx);
                if self.cur_ctx != ctx {
                    continue;
                }
                self.tick()?;
                ran += 1;
                match job {
                    Job::Reaction {
                        cb,
                        arg,
                        rejected,
                        next,
                    } => {
                        if self.is_callable(cb) {
                            match self.call_value(cb, Value::Undefined, &[arg]) {
                                Ok(v) => self.resolve_promise(next, v),
                                Err(Thrown(t)) => {
                                    self.settle_promise(next, PromiseState::Rejected, t);
                                }
                            }
                        } else {
                            // Pass-through: `then()` without a handler for
                            // this outcome forwards state/result as-is.
                            self.settle_promise(
                                next,
                                if rejected {
                                    PromiseState::Rejected
                                } else {
                                    PromiseState::Fulfilled
                                },
                                arg,
                            );
                        }
                    }
                    Job::Thenable {
                        then_fn,
                        this,
                        resolve,
                        reject,
                    } => {
                        if let Err(Thrown(t)) = self.call_value(then_fn, this, &[resolve, reject]) {
                            let _ = self.call_value(reject, Value::Undefined, &[t]);
                        }
                    }
                    Job::Plain(f) => {
                        self.call_value(f, Value::Undefined, &[])?;
                    }
                }
            }
            // JS-C26 : livraison MutationObserver en fin de
            // checkpoint — leurs callbacks peuvent enqueuer de
            // nouveaux jobs → reboucle (budget ops borne).
            // JS-C29 : même couture pour les messages workers —
            // un handler `onmessage` qui enqueue des microtasks
            // est vidé au tour suivant.
            // JS-C37 : frontière de jobs = point sûr GC — seuil de
            // cellules vivantes dépassé → passe mark-sweep.
            // JS-C69 : même checkpoint pour les notifications
            // `IntersectionObserver`/`ResizeObserver` (entrée
            // initiale à l'`observe`, callbacks = jobs).
            // JS-C70 : idem pour les lectures `FileReader`
            // (loadstart→progress→load→loadend).
            // JS-C71 : re-`pull` des `ReadableStream` ayant des
            // `read()` pendantes (queue vide post-pull).
            // JS-C72 : même checkpoint pour les `IDBRequest`
            // (success/error, cursor steps, txn complete).
            // JS-C73 : refus de handshake `WebSocket` (error +
            // close 1006 — arme les fallbacks long-polling).
            // JS-C74 : messages `BroadcastChannel` en file.
            // JS-C78 : les états livrables sont **par contexte**
            // (garés) — la pompe visite chaque contexte vivant
            // pour drainer ses files propres, puis les messages
            // `postMessage` (file commune cross-contexte), puis
            // les events workers (pool global, livrés à leur
            // contexte émetteur par la livraison elle-même).
            let mut delivered = false;
            let ids: Vec<u32> = self
                .ctxs
                .iter()
                .enumerate()
                .filter(|(_, m)| m.alive)
                .map(|(i, _)| i as u32)
                .collect();
            for id in ids {
                self.switch_to(id);
                if self.cur_ctx != id {
                    continue;
                }
                if self.deliver_mutations()?
                    || self.deliver_observers()? > 0
                    || self.deliver_file_reads()? > 0
                    || self.pump_stream_reads()? > 0
                    || crate::idb::deliver_idb_requests(self)? > 0
                    || self.deliver_ws()? > 0
                    || self.drain_es()? > 0
                    || self.deliver_bc()? > 0
                {
                    delivered = true;
                }
            }
            let msgs = self.drain_pending_msgs()?;
            let workers = self.pump_workers()?;
            // JS-C81 : messages `MessageChannel` — même
            // frontière de job que `postMessage`/workers.
            let ports = self.drain_port_msgs()?;
            rounds += 1;
            // JS-C103 — sortie bornée : plafond de jobs atteint,
            // rounds de livraison épuisés, ou file réellement
            // apaisée (spec drain-until-empty).
            let settled =
                !delivered && msgs == 0 && workers == 0 && ports == 0 && self.microtasks.is_empty();
            if settled || ran >= MICROTASK_DRAIN_CAP || rounds >= DELIVERY_ROUNDS_CAP {
                self.switch_to(home);
                self.maybe_collect();
                return Ok(ran);
            }
            self.maybe_collect();
        }
    }
}

/// `resolve` bound native: `bound = [promise_id]`.
fn promise_resolve(e: &mut Engine, _this: Value, args: &[Value]) -> JsResult<Value> {
    let Some(Value::Obj(id)) = args.first().copied() else {
        return Err(e.type_error("resolve: missing promise"));
    };
    let v = args.get(1).copied().unwrap_or(Value::Undefined);
    e.resolve_promise(id, v);
    Ok(Value::Undefined)
}

/// `reject` bound native: `bound = [promise_id]`.
fn promise_reject(e: &mut Engine, _this: Value, args: &[Value]) -> JsResult<Value> {
    let Some(Value::Obj(id)) = args.first().copied() else {
        return Err(e.type_error("reject: missing promise"));
    };
    let v = args.get(1).copied().unwrap_or(Value::Undefined);
    e.reject_promise(id, v);
    Ok(Value::Undefined)
}

// -- builtins ---------------------------------------------------------------

/// `new Promise(executor)`.
fn promise_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let executor = args.first().copied().unwrap_or(Value::Undefined);
    if !e.is_callable(executor) {
        return Err(e.type_error("Promise resolver is not a function"));
    }
    let id = e.new_promise();
    let (resolve, reject) = {
        let Some(o) = e.heap.obj(id) else {
            return Err(e.type_error("internal promise error"));
        };
        let ObjKind::Promise(p) = &o.kind else {
            return Err(e.type_error("internal promise error"));
        };
        (p.resolve_fn, p.reject_fn)
    };
    if let Err(Thrown(t)) = e.call_value(executor, Value::Undefined, &[resolve, reject]) {
        e.reject_promise(id, t);
    }
    Ok(Value::Obj(id))
}

/// `p.then(onF, onR)` → chained promise.
fn promise_then(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = this else {
        return Err(e.type_error("then called on non-promise"));
    };
    let on_f = args.first().copied().unwrap_or(Value::Undefined);
    let on_r = args.get(1).copied().unwrap_or(Value::Undefined);
    let next = e.new_promise();
    let (state, result) = {
        let Some(o) = e.heap.obj(id) else {
            return Err(e.type_error("then called on non-promise"));
        };
        let ObjKind::Promise(p) = &o.kind else {
            return Err(e.type_error("then called on non-promise"));
        };
        (p.state, p.result)
    };
    match state {
        PromiseState::Pending => {
            if let Some(o) = e.heap.obj_mut(id) {
                if let ObjKind::Promise(p) = &mut o.kind {
                    p.reactions.push(Reaction {
                        on_fulfilled: on_f,
                        on_rejected: on_r,
                        next,
                    });
                }
            }
        }
        PromiseState::Fulfilled | PromiseState::Rejected => {
            let rejected = state == PromiseState::Rejected;
            e.microtasks.push_back((
                e.cur_ctx,
                Job::Reaction {
                    cb: if rejected { on_r } else { on_f },
                    arg: result,
                    rejected,
                    next,
                },
            ));
        }
    }
    Ok(Value::Obj(next))
}

/// `p.catch(onR)` ≡ `then(undefined, onR)`.
fn promise_catch(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    promise_then(e, this, &[Value::Undefined, arg(args, 0)])
}

/// Bound `finally` handler — `bound = [cb]`; fulfill path returns the
/// original value after running `cb`.
fn promise_finally_ok(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let cb = args.first().copied().unwrap_or(Value::Undefined);
    let v = args.get(1).copied().unwrap_or(Value::Undefined);
    if e.is_callable(cb) {
        e.call_value(cb, this, &[])?;
    }
    Ok(v)
}

/// Bound `finally` handler — reject path rethrows the reason.
fn promise_finally_err(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let cb = args.first().copied().unwrap_or(Value::Undefined);
    let v = args.get(1).copied().unwrap_or(Value::Undefined);
    if e.is_callable(cb) {
        e.call_value(cb, this, &[])?;
    }
    Err(Thrown(v))
}

/// `p.finally(cb)`.
fn promise_finally(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let cb = arg(args, 0);
    let ok = e.bound_native("finally", promise_finally_ok, vec![cb]);
    let err = e.bound_native("finally", promise_finally_err, vec![cb]);
    promise_then(e, this, &[ok, err])
}

/// `Promise.resolve(v)` — a promise already fulfilled, or `v` if it
/// is already a promise.
fn promise_static_resolve(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let v = arg(args, 0);
    if e.is_promise(v) {
        return Ok(v);
    }
    let id = e.new_promise();
    e.resolve_promise(id, v);
    Ok(Value::Obj(id))
}

/// `Promise.reject(r)`.
fn promise_static_reject(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let id = e.new_promise();
    e.reject_promise(id, arg(args, 0));
    Ok(Value::Obj(id))
}

/// Elements of an array-like value.
fn iter_values(e: &mut Engine, v: Value) -> JsResult<Vec<Value>> {
    let Value::Obj(id) = v else {
        return Err(e.type_error("argument is not iterable"));
    };
    if let Some(o) = e.heap.obj(id) {
        if let ObjKind::Array(elems) = &o.kind {
            return Ok(elems.clone());
        }
    }
    // Array-like: length + numeric props.
    let len = e
        .get_prop(id, "length")
        .map_or(0, |v| e.to_number(v).unwrap_or(0.0) as usize);
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        out.push(e.get_prop(id, &i.to_string()).unwrap_or(Value::Undefined));
    }
    Ok(out)
}

/// `Promise.all(iter)` — fulfill when all settle, first reject wins.
fn promise_all(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let vals = iter_values(e, arg(args, 0))?;
    let id = e.new_promise();
    if vals.is_empty() {
        let empty = e.new_array(Vec::new());
        e.resolve_promise(id, empty);
        return Ok(Value::Obj(id));
    }
    // Shared bookkeeping: `{ remaining, results, target }`.
    let arr = e.new_array(vec![Value::Undefined; vals.len()]);
    let Value::Obj(results) = arr else {
        return Err(e.type_error("internal error"));
    };
    let state = e.new_object();
    e.set_own(state, "remaining", Value::Num(vals.len() as f64), false);
    e.set_own(state, "results", Value::Obj(results), false);
    e.set_own(state, "target", Value::Obj(id), false);
    for (i, v) in vals.into_iter().enumerate() {
        let pv = if e.is_promise(v) {
            v
        } else {
            let pid = e.new_promise();
            e.resolve_promise(pid, v);
            Value::Obj(pid)
        };
        let on_f = e.bound_native(
            "all",
            promise_all_fulfill,
            vec![Value::Obj(state), Value::Num(i as f64)],
        );
        let on_r = {
            let Some(o) = e.heap.obj(id) else { continue };
            let ObjKind::Promise(p) = &o.kind else {
                continue;
            };
            p.reject_fn
        };
        promise_then(e, pv, &[on_f, on_r])?;
    }
    Ok(Value::Obj(id))
}

/// `all` per-element fulfill — `bound = [state, index]`.
fn promise_all_fulfill(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let Some(Value::Obj(state)) = args.first().copied() else {
        return Err(e.type_error("all: missing state"));
    };
    let i = e.to_number(args.get(1).copied().unwrap_or(Value::Num(0.0)))? as usize;
    let v = args.get(2).copied().unwrap_or(Value::Undefined);
    let (results, target) = {
        let r = e.get_prop(state, "results").unwrap_or(Value::Undefined);
        let t = e.get_prop(state, "target").unwrap_or(Value::Undefined);
        (r, t)
    };
    if let (Value::Obj(rid), Value::Obj(tid)) = (results, target) {
        if let Some(o) = e.heap.obj_mut(rid) {
            if let ObjKind::Array(elems) = &mut o.kind {
                if let Some(slot) = elems.get_mut(i) {
                    *slot = v;
                }
            }
        }
        let rem = e
            .get_prop(state, "remaining")
            .map_or(1.0, |v| e.to_number(v).unwrap_or(1.0))
            - 1.0;
        e.set_own(state, "remaining", Value::Num(rem), false);
        if rem <= 0.0 {
            e.resolve_promise(tid, results);
        }
    }
    Ok(Value::Undefined)
}

/// `Promise.race(iter)` — first settle wins.
fn promise_race(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let vals = iter_values(e, arg(args, 0))?;
    let id = e.new_promise();
    let (resolve, reject) = {
        let Some(o) = e.heap.obj(id) else {
            return Err(e.type_error("internal promise error"));
        };
        let ObjKind::Promise(p) = &o.kind else {
            return Err(e.type_error("internal promise error"));
        };
        (p.resolve_fn, p.reject_fn)
    };
    for v in vals {
        if e.is_promise(v) {
            promise_then(e, v, &[resolve, reject])?;
        } else {
            e.resolve_promise(id, v);
        }
    }
    Ok(Value::Obj(id))
}

/// `Promise.allSettled(iter)` — never rejects; results are
/// `{status, value|reason}` objects.
fn promise_all_settled(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let vals = iter_values(e, arg(args, 0))?;
    let id = e.new_promise();
    if vals.is_empty() {
        let empty = e.new_array(Vec::new());
        e.resolve_promise(id, empty);
        return Ok(Value::Obj(id));
    }
    let arr = e.new_array(vec![Value::Undefined; vals.len()]);
    let Value::Obj(results) = arr else {
        return Err(e.type_error("internal error"));
    };
    let state = e.new_object();
    e.set_own(state, "remaining", Value::Num(vals.len() as f64), false);
    e.set_own(state, "results", Value::Obj(results), false);
    e.set_own(state, "target", Value::Obj(id), false);
    for (i, v) in vals.into_iter().enumerate() {
        let pv = if e.is_promise(v) {
            v
        } else {
            let pid = e.new_promise();
            e.resolve_promise(pid, v);
            Value::Obj(pid)
        };
        let on_f = e.bound_native(
            "allSettled",
            promise_all_settled_ok,
            vec![Value::Obj(state), Value::Num(i as f64)],
        );
        let on_r = e.bound_native(
            "allSettled",
            promise_all_settled_err,
            vec![Value::Obj(state), Value::Num(i as f64)],
        );
        promise_then(e, pv, &[on_f, on_r])?;
    }
    Ok(Value::Obj(id))
}

/// `allSettled` slot writer — `bound = [state, index]`, the slot
/// value comes as arg, `ok` picks `value` vs `reason` key.
fn all_settled_slot(e: &mut Engine, args: &[Value], key: &'static str) -> JsResult<()> {
    let Some(Value::Obj(state)) = args.first().copied() else {
        return Err(e.type_error("allSettled: missing state"));
    };
    let i = e.to_number(args.get(1).copied().unwrap_or(Value::Num(0.0)))? as usize;
    let v = args.get(2).copied().unwrap_or(Value::Undefined);
    let slot = e.new_object();
    let status = e.str_val(if key == "value" {
        "fulfilled"
    } else {
        "rejected"
    });
    e.set_own(slot, "status", status, true);
    e.set_own(slot, key, v, true);
    let (results, target) = {
        (
            e.get_prop(state, "results").unwrap_or(Value::Undefined),
            e.get_prop(state, "target").unwrap_or(Value::Undefined),
        )
    };
    if let (Value::Obj(rid), Value::Obj(tid)) = (results, target) {
        if let Some(o) = e.heap.obj_mut(rid) {
            if let ObjKind::Array(elems) = &mut o.kind {
                if let Some(s) = elems.get_mut(i) {
                    *s = Value::Obj(slot);
                }
            }
        }
        let rem = e
            .get_prop(state, "remaining")
            .map_or(1.0, |v| e.to_number(v).unwrap_or(1.0))
            - 1.0;
        e.set_own(state, "remaining", Value::Num(rem), false);
        if rem <= 0.0 {
            e.resolve_promise(tid, results);
        }
    }
    Ok(())
}

fn promise_all_settled_ok(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    all_settled_slot(e, args, "value")?;
    Ok(Value::Undefined)
}

fn promise_all_settled_err(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    all_settled_slot(e, args, "reason")?;
    Ok(Value::Undefined)
}

/// `Promise.any(iter)` — first fulfill wins; all reject → reject.
fn promise_any(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let vals = iter_values(e, arg(args, 0))?;
    let id = e.new_promise();
    if vals.is_empty() {
        let err = aggregate(e, Vec::new());
        e.reject_promise(id, err);
        return Ok(Value::Obj(id));
    }
    let state = e.new_object();
    let arr = e.new_array(vec![Value::Undefined; vals.len()]);
    e.set_own(state, "remaining", Value::Num(vals.len() as f64), false);
    e.set_own(state, "target", Value::Obj(id), false);
    e.set_own(state, "errors", arr, false);
    for (i, v) in vals.into_iter().enumerate() {
        let pv = if e.is_promise(v) {
            v
        } else {
            let pid = e.new_promise();
            e.resolve_promise(pid, v);
            Value::Obj(pid)
        };
        let on_f = {
            let Some(o) = e.heap.obj(id) else { continue };
            let ObjKind::Promise(p) = &o.kind else {
                continue;
            };
            p.resolve_fn
        };
        let on_r = e.bound_native(
            "any",
            promise_any_reject,
            vec![Value::Obj(state), Value::Num(i as f64)],
        );
        promise_then(e, pv, &[on_f, on_r])?;
    }
    Ok(Value::Obj(id))
}

/// `AggregateError` produit par `Promise.any` — proto dédié +
/// prop `errors` (spec `AggregateError(errors, msg)`).
fn aggregate(e: &mut Engine, errors: Vec<Value>) -> Value {
    let list = e.new_array(errors);
    let err = e.error_value(
        e.protos.aggregate_error,
        "AggregateError",
        "All promises were rejected",
    );
    if let Value::Obj(eid) = err {
        e.set_own(eid, "errors", list, false);
    }
    err
}

/// `any` reject bookkeeping — raisons collectées en `errors[i]` ;
/// la dernière produit l'`AggregateError`.
fn promise_any_reject(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let Some(Value::Obj(state)) = args.first().copied() else {
        return Err(e.type_error("any: missing state"));
    };
    let i = e.to_number(args.get(1).copied().unwrap_or(Value::Num(0.0)))? as usize;
    let reason = args.get(2).copied().unwrap_or(Value::Undefined);
    let (target, errors) = {
        let t = e.get_prop(state, "target").unwrap_or(Value::Undefined);
        let r = e.get_prop(state, "errors").unwrap_or(Value::Undefined);
        (t, r)
    };
    if let Value::Obj(rid) = errors {
        if let Some(o) = e.heap.obj_mut(rid) {
            if let ObjKind::Array(elems) = &mut o.kind {
                if let Some(slot) = elems.get_mut(i) {
                    *slot = reason;
                }
            }
        }
    }
    let rem = e
        .get_prop(state, "remaining")
        .map_or(1.0, |v| e.to_number(v).unwrap_or(1.0))
        - 1.0;
    e.set_own(state, "remaining", Value::Num(rem), false);
    if rem <= 0.0 {
        if let (Value::Obj(tid), Value::Obj(rid)) = (target, errors) {
            let errs = match e.heap.obj(rid).map(|o| &o.kind) {
                Some(ObjKind::Array(elems)) => elems.clone(),
                _ => Vec::new(),
            };
            let err = aggregate(e, errs);
            e.reject_promise(tid, err);
        }
    }
    Ok(Value::Undefined)
}

/// `Promise.withResolvers()` → `{promise, resolve, reject}` (ES2024).
fn promise_with_resolvers(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    let id = e.new_promise();
    let (resolve, reject) = {
        let Some(o) = e.heap.obj(id) else {
            return Err(e.type_error("internal promise error"));
        };
        let ObjKind::Promise(p) = &o.kind else {
            return Err(e.type_error("internal promise error"));
        };
        (p.resolve_fn, p.reject_fn)
    };
    let out = e.new_object();
    e.set_own(out, "promise", Value::Obj(id), true);
    e.set_own(out, "resolve", resolve, true);
    e.set_own(out, "reject", reject, true);
    Ok(Value::Obj(out))
}

/// `Promise.try(fn, ...args)` (ES2025) — `fn(...args)` appelé en
/// microtâche (via `then` sur une promesse résolue), résultat
/// adopté / exception rejetée.
fn promise_try(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let f = arg(args, 0);
    if !e.is_callable(f) {
        return Err(e.type_error("Promise.try: not a function"));
    }
    // Bound = `[f, ...extras]` ; le dernier arg reçu à l'appel est
    // la valeur `then` (ignorée).
    let cb = e.bound_native("try", promise_try_call, args.to_vec());
    let pid = e.new_promise();
    e.resolve_promise(pid, Value::Undefined);
    promise_then(e, Value::Obj(pid), &[cb, Value::Undefined])
}

/// Callback `then` de `Promise.try` — bound `[f, ...extras]`,
/// appelle `f(...extras)` et laisse `then` adopter le retour.
fn promise_try_call(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let f = args.first().copied().unwrap_or(Value::Undefined);
    let end = args.len().saturating_sub(1).max(1);
    let extras: Vec<Value> = args.get(1..end).unwrap_or(&[]).to_vec();
    e.call_value(f, Value::Undefined, &extras)
}

/// `queueMicrotask(f)`.
fn queue_microtask(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let f = arg(args, 0);
    if !e.is_callable(f) {
        return Err(e.type_error("queueMicrotask: not a function"));
    }
    e.enqueue_microtask(f);
    Ok(Value::Undefined)
}

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).copied().unwrap_or(Value::Undefined)
}

/// Install `Promise`/`queueMicrotask` on the global object.
pub(crate) fn install(e: &mut Engine) {
    let g = e.global;
    let rp = e.protos.promise;
    e.add_method(rp, "then", promise_then);
    e.add_method(rp, "catch", promise_catch);
    e.add_method(rp, "finally", promise_finally);
    let ctor = e.native_fn("Promise", promise_ctor);
    e.set_own(
        crate::promise::c_id(ctor),
        "prototype",
        Value::Obj(rp),
        false,
    );
    e.set_own(rp, "constructor", ctor, false);
    if let Value::Obj(cid) = ctor {
        e.add_method(cid, "resolve", promise_static_resolve);
        e.add_method(cid, "reject", promise_static_reject);
        e.add_method(cid, "all", promise_all);
        e.add_method(cid, "race", promise_race);
        e.add_method(cid, "allSettled", promise_all_settled);
        e.add_method(cid, "any", promise_any);
        e.add_method(cid, "withResolvers", promise_with_resolvers);
        e.add_method(cid, "try", promise_try);
    }
    e.set_own(g, "Promise", ctor, false);
    e.add_method(g, "queueMicrotask", queue_microtask);
}

fn c_id(v: Value) -> ObjId {
    match v {
        Value::Obj(id) => id,
        _ => 0,
    }
}
