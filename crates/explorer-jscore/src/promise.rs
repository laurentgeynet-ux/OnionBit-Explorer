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
//! `async`/`await` runs on the suspendable `GExec` (JS-C34) —
//! pending `await` suspends via `then` reactions bound to the
//! `GenLive`. Foreign thenables (library `Promise` polyfills,
//! jQuery Deferred, `IDBRequest`…) are adopted on `await` the
//! same way `resolve` adopts them (JS-C278).

use crate::heap::{Fun, JsResult, ObjKind, Thrown};
use crate::value::{ObjId, Value};
use crate::Engine;
use std::rc::Rc;

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

/// Puits d'une réaction `then` — cible du résultat
/// (`NewPromiseCapability(C)` spec, JS-C280).
///
/// `C === %Promise%` intrinsèque → fast-path natif : la promesse
/// chaînée est un `ObjKind::Promise` settlé en place. Tout autre
/// `C` (`@@species` ou `constructor` custom — sous-classes,
/// polyfills, le `FakePromise` de la détection core-js) →
/// `resolve`/`reject` callables capturés par l'exécuteur
/// capability, appelés au settle.
#[derive(Clone)]
pub enum ThenSink {
    /// Capacité `%Promise%` — `resolve_promise`/`settle_promise`.
    Native(ObjId),
    /// Capacité étrangère — callables capturés sur `new C(exec)`.
    Foreign {
        /// `capability.[[Resolve]]`.
        resolve: Value,
        /// `capability.[[Reject]]`.
        reject: Value,
    },
}

/// A `then` subscription.
pub struct Reaction {
    /// `onFulfilled` — callable or `Undefined`.
    pub on_fulfilled: Value,
    /// `onRejected` — callable or `Undefined`.
    pub on_rejected: Value,
    /// Capability returned by `then`, settled with the handler result.
    pub next: ThenSink,
}

/// JS-C383 — sonde « promesses pendantes nommées » (§B-97) :
/// sites capturés par promesse quand `dom_probe` est armée —
/// le site de création (`new Promise`/`capability`) + les
/// sites de souscription `.then`/adoption. Table latérale
/// [`Engine::prom_sites`] — aucune entrée n'est créée hors
/// sonde (zéro coût par promesse désarmée).
pub(crate) struct PromProbe {
    /// « fn(bundle:l:c) » de l'appelant à la création — `<?>` si
    /// la promesse prédate l'armement de la sonde.
    pub created: Rc<str>,
    /// Sites `.then`/adoption souscrits tant qu'elle pendait —
    /// dédup + borne [`PROM_THEN_SITES_PER`].
    pub thens: Vec<Rc<str>>,
    /// JS-C386 — site du settle (`fn(l:c)` à l'appel de
    /// `resolve`/`reject` ou à la résolution chaînée) + état —
    /// la face miroir : `prom.pending` nomme les pendantes,
    /// `prom.settle` nomme *qui règle* (`1484` créée ici mais
    /// jamais réglée → son `resolve` n'est jamais invoqué).
    pub settled: Option<(Rc<str>, PromiseState)>,
    /// JS-C412 — extrait source **copié** de l'executor
    /// (`new Promise(fn)`) — [`PROM_SRC_CHARS`] premiers
    /// caractères de la fonction. `prom.src` imprime le code
    /// de la fabrique pendante au lieu de son seul site : la
    /// condition que `resolve` attend est *lue*, pas devinée.
    /// Copié — jamais le `Rc<str>` de `lit.src_text` : retenir
    /// le handle épinglerait le bundle entier par promesse,
    /// ironique pour une sonde mémoire.
    pub src: Option<Rc<str>>,
}

/// JS-C383 — borne globale de la table `prom_sites` (sonde
/// armée : un vol peut créer des milliers de promesses — le
/// résidu utile est de ~30-60 `.then`, large au-dessus).
pub(crate) const PROM_SITES_MAX: usize = 8192;
/// JS-C383 — `.then` retenus par promesse (les chaînes réelles
/// souscrivent peu par promesse ; les chaînages massifs se
/// répartissent sur la séquence, pas un seul objet).
pub(crate) const PROM_THEN_SITES_PER: usize = 8;
/// JS-C383 — sites émis par histogramme `prom.pending.*`.
pub(crate) const PROM_SITES_TOP: usize = 32;
/// JS-C412 — caractères retenus de l'executor par promesse
/// (`prom.src`) — assez pour nommer la condition du gate
/// (signature + ouverture de la fabrique) ; l'extrait complet
/// sort par `fnat` (JS-C413) sur le bundle archivé.
pub(crate) const PROM_SRC_CHARS: usize = 200;
/// JS-C412 — sites `prom.src` émis par passe (top créés).
pub(crate) const PROM_SRC_SITES: usize = 8;

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
    /// `[[PromiseIsHandled]]` spec — posé par tout `then`
    /// (même sans `onRejected`) ; une promesse rejetée jamais
    /// `.then`ée déclenche `unhandledrejection` au checkpoint.
    pub handled: bool,
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
        /// Capability returned by `then`.
        next: ThenSink,
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
    /// Compteurs §B-4(c) — `(créées, réglées)` par moteur : un
    /// `await` gelé laisse un gap stable `créées - réglées ≥ 1`
    /// sans erreur — la signature « évals OK, page inerte ».
    /// Exposé au vol (comparaison entre évals silencieux).
    #[must_use]
    pub fn promise_stats(&self) -> (u64, u64) {
        (self.promises_created, self.promises_settled)
    }

    /// Instantané « boot muet » §B-4(c) — `(timers en attente,
    /// microtasks en file, readyState du ctx courant)`.
    /// Discriminateur : jobs > 0 = bootstrap garé sur un job
    /// jamais drainé (pompe) ; tout à 0 + `complete` = retour
    /// avalé synchrone (gate de feature-detect, autre
    /// famille).
    #[must_use]
    pub fn boot_diag(&self) -> (usize, usize, &'static str) {
        (
            self.pending_timers(),
            self.microtasks.len(),
            self.ready_state_ctx(self.cur_ctx),
        )
    }

    /// JS-C383 — sonde « promesses pendantes nommées » : note le
    /// site de création (`dom_probe` armée uniquement). No-op
    /// désarmée ou table pleine ([`PROM_SITES_MAX`]).
    fn probe_prom_created(&mut self, id: ObjId) {
        if !tracing::enabled!(target: "dom_probe", tracing::Level::INFO)
            || self.prom_sites.len() >= PROM_SITES_MAX
            || self.prom_sites.contains_key(&id)
        {
            return;
        }
        let site: Rc<str> = self.cur_fn_label().into();
        self.prom_sites.insert(
            id,
            PromProbe {
                created: site,
                thens: Vec::new(),
                settled: None,
                src: None,
            },
        );
    }

    /// JS-C383 — note un site de souscription `.then`/adoption
    /// sur promesse pendante (`dom_probe` armée uniquement).
    /// L'entrée peut manquer si la promesse prédate l'armement —
    /// elle est alors créée avec `created = "<?>"`.
    fn probe_prom_then(&mut self, id: ObjId) {
        if !tracing::enabled!(target: "dom_probe", tracing::Level::INFO) {
            return;
        }
        let site: Rc<str> = self.cur_fn_label().into();
        if !self.prom_sites.contains_key(&id) {
            if self.prom_sites.len() >= PROM_SITES_MAX {
                return;
            }
            self.prom_sites.insert(
                id,
                PromProbe {
                    created: "<?>".into(),
                    thens: Vec::new(),
                    settled: None,
                    src: None,
                },
            );
        }
        if let Some(pr) = self.prom_sites.get_mut(&id) {
            if pr.thens.len() < PROM_THEN_SITES_PER && !pr.thens.contains(&site) {
                pr.thens.push(site);
            }
        }
    }

    /// JS-C386 — sonde settle-site : note `fn(l:c)` à l'appel
    /// effectif du settle (`resolve`/`reject` natif, réaction
    /// chaînée ou adoption — `cur_fn_label` nomme la fonction
    /// JS au sommet). Appelée depuis [`Self::settle_promise`]
    /// après la transition d'état — l'early-return « déjà
    /// settlée » garantit l'unicité.
    fn probe_prom_settled(&mut self, id: ObjId, state: PromiseState) {
        if !tracing::enabled!(target: "dom_probe", tracing::Level::INFO) {
            return;
        }
        let site: Rc<str> = self.cur_fn_label().into();
        if !self.prom_sites.contains_key(&id) {
            if self.prom_sites.len() >= PROM_SITES_MAX {
                return;
            }
            self.prom_sites.insert(
                id,
                PromProbe {
                    created: "<?>".into(),
                    thens: Vec::new(),
                    settled: None,
                    src: None,
                },
            );
        }
        if let Some(pr) = self.prom_sites.get_mut(&id) {
            pr.settled = Some((site, state));
        }
    }

    /// JS-C412 — extrait source borné d'une valeur fonction
    /// ([`PROM_SRC_CHARS`] premiers caractères) : lit
    /// `ObjKind::Fun::Script` → `lit.src_text` + `lit.span`
    /// (offsets en **caractères**). `None` pour natives,
    /// synthétisées et fonctions sans verbatim.
    fn fn_source_excerpt(&self, v: Value) -> Option<Rc<str>> {
        let Value::Obj(id) = v else {
            return None;
        };
        let o = self.heap.obj(id)?;
        let ObjKind::Fun(crate::heap::Fun::Script { lit, .. }) = &o.kind else {
            return None;
        };
        let src = lit.src_text.as_ref()?;
        let (lo, hi) = lit.span;
        if hi <= lo {
            return None;
        }
        let excerpt: String = src
            .chars()
            .skip(lo)
            .take((hi - lo).min(PROM_SRC_CHARS))
            .collect();
        Some(excerpt.into())
    }

    /// JS-C412 — « lire la fabrique pendante » (§B-97) : à la
    /// création `new Promise(executor)`, retient l'extrait
    /// copié de la source de l'executor — le `prom.src` du
    /// checkpoint imprime le CODE à côté du site : la condition
    /// que `resolve` attend est lue dans le log, pas devinée.
    /// No-op désarmée (un `enabled!`, zéro coût par promesse).
    fn probe_prom_src(&mut self, id: ObjId, executor: Value) {
        if !tracing::enabled!(target: "dom_probe", tracing::Level::INFO) {
            return;
        }
        let Some(excerpt) = self.fn_source_excerpt(executor) else {
            return;
        };
        if let Some(pr) = self.prom_sites.get_mut(&id) {
            pr.src = Some(excerpt);
        }
    }

    pub(crate) fn new_promise(&mut self) -> ObjId {
        self.promises_created += 1;
        let id = self.heap.alloc_obj(crate::heap::Object {
            proto: Value::Obj(self.protos.promise),
            props: Vec::new(),
            idx: std::cell::OnceCell::new(),
            // Placeholder: resolved below once the bound fns exist.
            kind: ObjKind::Promise(JsPromise {
                state: PromiseState::Pending,
                result: Value::Undefined,
                reactions: Vec::new(),
                resolve_fn: Value::Undefined,
                reject_fn: Value::Undefined,
                handled: false,
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
        self.probe_prom_created(id);
        id
    }

    /// `true` for `ObjKind::Promise` objects.
    pub(crate) fn is_promise(&self, v: Value) -> bool {
        matches!(v, Value::Obj(id) if matches!(self.heap.obj(id).map(|o| &o.kind), Some(ObjKind::Promise(_))))
    }

    /// Settle `id` — enqueues its reactions as microtasks. No-op on
    /// a non-promise or an already-settled promise.
    pub(crate) fn settle_promise(&mut self, id: ObjId, state: PromiseState, result: Value) {
        let (reactions, unhandled) = {
            let Some(o) = self.heap.obj_mut(id) else {
                return;
            };
            let ObjKind::Promise(p) = &mut o.kind else {
                return;
            };
            if p.state != PromiseState::Pending {
                if std::env::var("EXPLORER_PROM_TRACE").is_ok() {
                    eprintln!("[settle-skip] already settled");
                }
                return;
            }
            p.state = state;
            self.promises_settled += 1;
            p.result = result.clone();
            let u = state == PromiseState::Rejected && !p.handled;
            (std::mem::take(&mut p.reactions), u)
        };
        self.probe_prom_settled(id, state);
        // `unhandledrejection` (JS-C154) — promesse rejetée sans
        // aucun `then` : gardée pour le tir en fin de checkpoint
        // (un `.then` tardif la revoit — `rejectionhandled`
        // silencieux). Le host session draine
        // `unhandled_reports` pour le ring console.
        if unhandled {
            self.pending_unhandled
                .push((Value::Obj(id), result.clone()));
        }
        if std::env::var("EXPLORER_PROM_TRACE").is_ok() && !reactions.is_empty() {
            eprintln!("[settle] reactions={}", reactions.len());
        }
        for r in reactions {
            let rejected = state == PromiseState::Rejected;
            self.sched_tick("post.reaction");
            self.microtasks.push_back((
                self.cur_ctx,
                Job::Reaction {
                    cb: if rejected {
                        r.on_rejected
                    } else {
                        r.on_fulfilled
                    },
                    arg: result.clone(),
                    rejected,
                    next: r.next,
                },
            ));
        }
    }

    /// `resolve` semantics: promise adoption, thenable `then` call
    /// (deferred to a microtask), plain fulfillment otherwise.
    /// `SpeciesConstructor(O, %Promise%)` spec (JS-C280) :
    /// `Get(O,"constructor")` → `Get(C, @@species)` — `undefined`
    /// à chaque niveau → `%Promise%` intrinsèque du realm
    /// (`protos.promise_ctor`, jamais `global.Promise` — un
    /// `Promise` réassigné ne détourne pas la capacité).
    /// `constructor` non-objet ou `@@species` non-appelable →
    /// `TypeError` (un callable non-constructible — flèche —
    /// tombe au `construct` de `new_promise_capability`).
    pub(crate) fn species_ctor(&mut self, o: Value) -> JsResult<Value> {
        let c = self.get_prop_value(o, "constructor")?;
        let s = match c {
            Value::Undefined => return Ok(Value::Obj(self.protos.promise_ctor)),
            Value::Obj(_) => self.get_prop_value(c, crate::interp::SYM_SPECIES)?,
            _ => return Err(self.type_error("SpeciesConstructor: constructor is not an object")),
        };
        match s {
            Value::Undefined | Value::Null => Ok(Value::Obj(self.protos.promise_ctor)),
            s if self.is_callable(s.clone()) => Ok(s),
            _ => Err(self.type_error("SpeciesConstructor: @@species is not a constructor")),
        }
    }

    /// `NewPromiseCapability(C)` spec (JS-C280) : `new C(executor)`
    /// — les callables `resolve`/`reject` passés à l'exécuteur
    /// sont capturés dans une boîte-array partagée avec le
    /// `BoundNative`. Fast-path `C === %Promise%` → `new_promise`
    /// natif (settle direct, zéro hop).
    ///
    /// Retourne `(promise, sink)` — `promise` est la valeur que
    /// `then` rend à l'appelant, `sink` ce que les réactions
    /// settlent (`Native` = promesse moteur, `Foreign` =
    /// callables capturés du ctor `@@species` — dont le
    /// `FakePromise` de la détection core-js `FORCED_PROMISE_
    /// CONSTRUCTOR`, qui exige `p.then(…) instanceof C`).
    pub(crate) fn new_promise_capability(&mut self, c: Value) -> JsResult<(Value, ThenSink)> {
        if matches!(c, Value::Obj(id) if id == self.protos.promise_ctor) {
            let id = self.new_promise();
            return Ok((Value::Obj(id), ThenSink::Native(id)));
        }
        let slot = self.new_array(vec![Value::Undefined; 2]);
        let exec = self.bound_native(
            "promise capability executor",
            promise_cap_executor,
            vec![slot.clone()],
        );
        let promise = self.construct(c, &[exec])?;
        let (resolve, reject) = match slot {
            Value::Obj(sid) => (
                self.get_prop(sid, "0").unwrap_or(Value::Undefined),
                self.get_prop(sid, "1").unwrap_or(Value::Undefined),
            ),
            _ => (Value::Undefined, Value::Undefined),
        };
        if !self.is_callable(resolve.clone()) || !self.is_callable(reject.clone()) {
            return Err(
                self.type_error("promise capability: executor did not capture resolve/reject")
            );
        }
        Ok((promise, ThenSink::Foreign { resolve, reject }))
    }

    /// Settle la capability `next` d'une réaction — natif en
    /// place, étranger via les callables capturés (JS-C280).
    fn sink_resolve(&mut self, next: ThenSink, v: Value) -> JsResult<()> {
        match next {
            ThenSink::Native(id) => self.resolve_promise(id, v),
            ThenSink::Foreign { resolve, .. } => {
                self.call_value(resolve, Value::Undefined, &[v])?;
            }
        }
        Ok(())
    }

    /// Rejette la capability `next` — voir [`Self::sink_resolve`].
    fn sink_reject(&mut self, next: ThenSink, t: Value) -> JsResult<()> {
        match next {
            ThenSink::Native(id) => self.settle_promise(id, PromiseState::Rejected, t),
            ThenSink::Foreign { reject, .. } => {
                self.call_value(reject, Value::Undefined, &[t])?;
            }
        }
        Ok(())
    }

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
        if self.is_promise(v.clone()) {
            // Adopt `v`: subscribe our own resolve/reject — re-entering
            // resolve_promise handles thenable chains naturally.
            let Value::Obj(vid) = v else { return };
            let (state, result) = {
                let Some(o) = self.heap.obj(vid) else { return };
                let ObjKind::Promise(p) = &o.kind else { return };
                (p.state, p.result.clone())
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
                        (p.resolve_fn.clone(), p.reject_fn.clone())
                    };
                    self.sched_tick("then");
                    if let Some(o) = self.heap.obj_mut(vid) {
                        if let ObjKind::Promise(p) = &mut o.kind {
                            // Adoption = souscription — `vid` est
                            // « handled » (son rejet propage à `id`).
                            p.handled = true;
                            p.reactions.push(Reaction {
                                on_fulfilled: my_resolve,
                                on_rejected: my_reject,
                                // `next` factice — le
                                // `resolve_promise(next, retour_cb)`
                                // du job est un no-op (`id` déjà
                                // settlée par la réaction elle-même).
                                next: ThenSink::Native(id),
                            });
                        }
                    }
                    self.probe_prom_then(vid);
                }
                // Already settled → adopt its outcome directly.
                PromiseState::Fulfilled | PromiseState::Rejected => {
                    self.settle_promise(id, state, result);
                }
            }
            return;
        }
        // Thenable adoption: `obj.then` callable → deferred call.
        if self.adopt_thenable(id, v.clone()) {
            return;
        }
        self.settle_promise(id, PromiseState::Fulfilled, v);
    }

    /// Thenable adoption — spec `[[Resolve]](thenable)` : `then`
    /// est lu **une fois** par le `[[Get]]` canonique
    /// (`get_prop_value` — accesseurs invoqués, proxies délégants,
    /// `get` host). Callable → `Job::Thenable` enfilé (appel
    /// différé en job) ; getter abrupt → rejet immédiat.
    /// Retourne `true` quand `v` était un thenable (adopté ou
    /// rejeté) — `false` = non-thenable, l'appelant fulfill `id`
    /// avec `v` tel quel. Partagé par `resolve` et `await`
    /// (JS-C278) : un getter `then` à effet de bord ne court
    /// jamais deux fois.
    pub(crate) fn adopt_thenable(&mut self, id: ObjId, v: Value) -> bool {
        if !matches!(v, Value::Obj(_)) {
            return false;
        }
        let then_fn = match self.get_prop_value(v.clone(), "then") {
            Ok(f) => f,
            Err(Thrown(t)) => {
                self.reject_promise(id, t);
                return true;
            }
        };
        if std::env::var("EXPLORER_PROM_TRACE").is_ok() {
            eprintln!(
                "[res-thenable] callable={}",
                self.is_callable(then_fn.clone())
            );
        }
        if !self.is_callable(then_fn.clone()) {
            return false;
        }
        let (resolve, reject) = {
            let Some(o) = self.heap.obj(id) else {
                return true;
            };
            let ObjKind::Promise(p) = &o.kind else {
                return true;
            };
            (p.resolve_fn.clone(), p.reject_fn.clone())
        };
        self.sched_tick("post.thenable");
        self.microtasks.push_back((
            self.cur_ctx,
            Job::Thenable {
                then_fn,
                this: v,
                resolve,
                reject,
            },
        ));
        true
    }

    /// Reject path — plain settle.
    pub(crate) fn reject_promise(&mut self, id: ObjId, v: Value) {
        self.settle_promise(id, PromiseState::Rejected, v);
    }

    /// `queueMicrotask(f)` — also used internally for deferred work.
    pub(crate) fn enqueue_microtask(&mut self, f: Value) {
        self.sched_tick("post.plain");
        self.microtasks.push_back((self.cur_ctx, Job::Plain(f)));
    }

    /// [`run_microtasks`](Self::run_microtasks) dans une frontière
    /// **boot** — `run_program`/`eval_module` : le drain partage
    /// la deadline boot (B-22/§B-23) au lieu de ré-armer la borne
    /// standard de `eval_timeout_ms`. Une chaîne de microtasks
    /// légitime produite par un corps de boot (requirejs/less
    /// de cryptpad — `unhandledrejection: eval deadline
    /// exceeded` au splash) a le même wall-clock que ce corps.
    /// Les pompes et livraisons d'événements gardent la borne
    /// standard via [`run_microtasks`](Self::run_microtasks).
    ///
    /// # Errors
    /// Mêmes erreurs que [`run_microtasks`](Self::run_microtasks)
    /// — deadline, budget d'ops ou tas épuisé pendant le drain.
    pub fn run_microtasks_in_boot(&mut self) -> JsResult<usize> {
        self.run_microtasks_impl(true)
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
        self.run_microtasks_impl(false)
    }

    fn run_microtasks_impl(&mut self, boot_boundary: bool) -> JsResult<usize> {
        // JS-C348 — garde de réentrance de la pompe. Un drain
        // actif plus haut dans la pile rend l'appel no-op : les
        // jobs enfilés sont dépoppés par **sa** boucle au même
        // niveau de pile (« microtask checkpoint » spec — un
        // seul drain, itératif). Sans la garde, une native qui
        // draine depuis un callback de job (`dispatch_media`,
        // livraison XHR, `fire_dom_event`, timers…) empilait un
        // drain par cycle — vol 651 : ~600 frames JS / cycle ~9
        // (dumps `[expr_bisect]`) → `RangeError` expr_depth →
        // bootstrap YouTube mort en plein chaînage `.then`.
        thread_local! {
            static MT_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
        }
        struct MtGuard;
        impl Drop for MtGuard {
            fn drop(&mut self) {
                MT_DEPTH.with(|c| c.set(c.get().saturating_sub(1)));
            }
        }
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
        let mt_d = MT_DEPTH.with(|c| {
            let d = c.get() + 1;
            c.set(d);
            d
        });
        let _g = MtGuard;
        if mt_d > 1 {
            return Ok(0);
        }
        let mut ran = 0usize;
        let mut rounds = 0usize;
        let home = self.cur_ctx;
        // Le drain est une frontière de tâche : budget neuf,
        // comme `run_due_timers_ctx` le fait déjà par timer.
        // Sans ré-armement, un eval lourd laissait `ops_left≈0`
        // et chaque pompe poppait un job pour le perdre au
        // `tick()` — famine muette (github : `.then` post-
        // `import()` mangés un par pompe, `define` jamais
        // atteint).
        self.ops_left = self.op_limit;
        // Frontière de batch aussi pour la deadline (J1) —
        // une file auto-régénérante meurt au wall-clock, pas
        // au timeout canal. Frontière boot (`run_program`/
        // `eval_module`) : la borne boot s'applique à tout le
        // drain (§B-23, cryptpad) — sinon le ré-armement à
        // `eval_timeout_ms` tuait la chaîne à 10 s.
        // J11 : la deadline se ré-arme désormais **par job**
        // (spec : chaque microtask = une tâche) — une chaîne
        // d'hydratation de N jobs × quelques s chacun ne meurt
        // plus au cumul ; le drain global rend simplement la
        // main au budget (yield) et les jobs restants attendent
        // la pompe suivante. Un job unique reste borné par la
        // deadline (hostile ou légitime-lourd).
        let drain_budget_ms = if boot_boundary {
            self.eval_timeout_boot_ms
        } else {
            self.batch_budget_ms()
        };
        let drain_start = std::time::Instant::now();
        let mut yielded = false;
        loop {
            loop {
                if ran >= MICROTASK_DRAIN_CAP {
                    break;
                }
                // J11 — yield de drain : budget global écoulé
                // → sortie gracieuse `Ok(ran)`, la file reste
                // pour la pompe suivante (une chaîne
                // d'hydratation progresse à travers les pompes
                // au lieu de mourir à la borne du drain).
                if ran > 0
                    && drain_budget_ms > 0
                    && drain_start.elapsed() >= std::time::Duration::from_millis(drain_budget_ms)
                {
                    yielded = true;
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
                    // `EXPLORER_PROM_TRACE` : un job dont le ctx
                    // n'est plus joignable (mort ou jamais garé)
                    // est sauté — nomme la famine (github lazy-CE :
                    // réactions `.then` post-`import()` taguées
                    // d'un ctx module disparu).
                    self.sched_tick("ctx_drop");
                    if std::env::var("EXPLORER_PROM_TRACE").is_ok() {
                        eprintln!("[prom-job-drop] ctx={ctx} cur={}", self.cur_ctx);
                    }
                    continue;
                }
                // J11 — budget **par job** (spec : chaque
                // microtask = une tâche) : ops et deadline
                // réarmés à chaque job ; le cumul d'une chaîne
                // légitime ne meurt plus à la borne du drain
                // (le yield ci-dessus borne le drain global).
                self.ops_left = if boot_boundary {
                    self.op_limit.saturating_mul(crate::interp::BOOT_OPS_FACTOR)
                } else {
                    self.op_limit
                };
                if boot_boundary {
                    self.arm_eval_deadline_boot();
                } else {
                    self.arm_eval_deadline();
                }
                if let Err(e) = self.tick() {
                    // Budget/heap épuisé : le job n'a **pas**
                    // couru — re-queue en tête plutôt que le
                    // perdre (la pompe suivante, budget neuf,
                    // le reprendra). Sans ça chaque drain
                    // affamé mangeait un job en silence.
                    self.microtasks.push_front((ctx, job));
                    return Err(e);
                }
                if std::env::var("EXPLORER_PROM_TRACE").is_ok() {
                    let kind = match &job {
                        Job::Reaction { rejected, .. } => {
                            if *rejected {
                                "reaction-rej"
                            } else {
                                "reaction-ok"
                            }
                        }
                        Job::Thenable { .. } => "thenable",
                        Job::Plain(_) => "plain",
                    };
                    eprintln!("[mt-run] {kind} ctx={ctx}");
                }
                ran += 1;
                self.sched_tick(match &job {
                    Job::Reaction { .. } => "fire.reaction",
                    Job::Thenable { .. } => "fire.thenable",
                    Job::Plain(_) => "fire.plain",
                });
                // §B-111 — breadcrumb par job : la dernière
                // ligne avant un overflow nomme la callback
                // fautive (vol 689 : la mort vit dans le drain,
                // ni apply ni eval top-level — la trace tient
                // lieu de morgue, le `.dmp` WER n'existe pas).
                if crate::dlv_trace() {
                    let desc = match &job {
                        Job::Reaction { cb, rejected, .. } => format!(
                            "reaction{} {}",
                            if *rejected { "-rej" } else { "-ok" },
                            self.job_cb_desc(cb.clone())
                        ),
                        Job::Thenable { then_fn, .. } => {
                            format!("thenable {}", self.job_cb_desc(then_fn.clone()))
                        }
                        Job::Plain(f) => format!("plain {}", self.job_cb_desc(f.clone())),
                    };
                    tracing::info!("job: mt ctx={ctx} {desc}");
                }
                match job {
                    Job::Reaction {
                        cb,
                        arg,
                        rejected,
                        next,
                    } => {
                        if self.is_callable(cb.clone()) {
                            match self.call_value(cb, Value::Undefined, &[arg]) {
                                Ok(v) => self.sink_resolve(next, v)?,
                                Err(Thrown(t)) => {
                                    // `EXPLORER_PROM_TRACE` : un cb
                                    // qui jette atterrit en rejet
                                    // silencieux — la raison nomme
                                    // l'API manquante (github lazy-CE :
                                    // le `.then` webpack post-import
                                    // raté, `define` jamais atteint).
                                    if std::env::var("EXPLORER_PROM_TRACE").is_ok() {
                                        eprintln!("[prom-cb-throw] {}", self.describe(t.clone()));
                                    }
                                    self.sink_reject(next, t)?;
                                }
                            }
                        } else {
                            // Pass-through: `then()` without a handler for
                            // this outcome forwards state/result as-is.
                            if rejected && std::env::var("EXPLORER_PROM_TRACE").is_ok() {
                                eprintln!("[prom-unhandled] {}", self.describe(arg.clone()));
                            }
                            if rejected {
                                self.sink_reject(next, arg)?;
                            } else {
                                self.sink_resolve(next, arg)?;
                            }
                        }
                    }
                    Job::Thenable {
                        then_fn,
                        this,
                        resolve,
                        reject,
                    } => {
                        if let Err(Thrown(t)) =
                            self.call_value(then_fn, this, &[resolve.clone(), reject.clone()])
                        {
                            let _ = self.call_value(reject, Value::Undefined, &[t]);
                        }
                    }
                    Job::Plain(f) => {
                        self.call_value(f, Value::Undefined, &[])?;
                    }
                }
            }
            // J11 — sortie gracieuse sur yield : jobs restants
            // en file, mêmes sorties qu'un drain apaisé.
            if yielded {
                if !self.microtasks.is_empty() {
                    self.sched_tick("left_drains");
                }
                job_rail(self.cur_ctx, "unhandled");
                self.fire_unhandled_rejections();
                self.switch_to(home);
                job_rail(self.cur_ctx, "gc");
                self.maybe_collect();
                job_rail(self.cur_ctx, "gc done");
                job_rail(self.cur_ctx, "drain done");
                return Ok(ran);
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
                // §B-97 — comptage par rail (`sched.*`) : le
                // `||` séquentiel court-circuitait les pompes
                // aval dès qu'une pompe amont livrait — chaque
                // pompe tourne désormais à chaque round (plus
                // fidèle à « deliver all ready work ») et sa
                // production est nommée dans le tally.
                job_rail(id, "dlv.mutations");
                let dm = self.deliver_mutations()?;
                job_rail(id, "dlv.observers");
                let dob = self.deliver_observers()?;
                job_rail(id, "dlv.file_reads");
                let dfr = self.deliver_file_reads()?;
                job_rail(id, "dlv.stream_reads");
                let dsr = self.pump_stream_reads()?;
                job_rail(id, "dlv.idb");
                let didb = crate::idb::deliver_idb_requests(self)?;
                job_rail(id, "dlv.ws");
                let dws = self.deliver_ws()?;
                job_rail(id, "dlv.es");
                let des = self.drain_es()?;
                job_rail(id, "dlv.bc");
                let dbc = self.deliver_bc()?;
                job_rail(id, "dlv.mse");
                let dmse = crate::mse::deliver_mse_events(self)?;
                self.sched_add("dlv.mutations", u64::from(dm));
                self.sched_add("dlv.observers", dob as u64);
                self.sched_add("dlv.file_reads", dfr as u64);
                self.sched_add("dlv.stream_reads", dsr as u64);
                self.sched_add("dlv.idb", didb as u64);
                self.sched_add("dlv.ws", dws as u64);
                self.sched_add("dlv.es", des as u64);
                self.sched_add("dlv.bc", dbc as u64);
                self.sched_add("dlv.mse", u64::from(dmse));
                if dm
                    || dob > 0
                    || dfr > 0
                    || dsr > 0
                    || didb > 0
                    || dws > 0
                    || des > 0
                    || dbc > 0
                    || dmse
                {
                    delivered = true;
                }
            }
            job_rail(self.cur_ctx, "msgs");
            let msgs = self.drain_pending_msgs()?;
            job_rail(self.cur_ctx, "workers");
            let workers = self.pump_workers()?;
            // JS-C81 : messages `MessageChannel` — même
            // frontière de job que `postMessage`/workers.
            job_rail(self.cur_ctx, "ports");
            let ports = self.drain_port_msgs()?;
            self.sched_add("dlv.msgs", msgs as u64);
            self.sched_add("dlv.workers", workers as u64);
            self.sched_add("dlv.ports", ports as u64);
            rounds += 1;
            // JS-C103 — sortie bornée : plafond de jobs atteint,
            // rounds de livraison épuisés, ou file réellement
            // apaisée (spec drain-until-empty).
            let settled =
                !delivered && msgs == 0 && workers == 0 && ports == 0 && self.microtasks.is_empty();
            if settled || ran >= MICROTASK_DRAIN_CAP || rounds >= DELIVERY_ROUNDS_CAP {
                // §B-97 — résidu : sortie avec file non vide
                // (borne JS-C103/rounds) — des jobs postés
                // attendent la pompe suivante.
                if !self.microtasks.is_empty() {
                    self.sched_tick("left_drains");
                }
                // JS-C154 — `unhandledrejection` en fin de
                // checkpoint : les rejets non gérés du tour sont
                // revus (un `.then` tardif les a retirés).
                job_rail(self.cur_ctx, "unhandled");
                self.fire_unhandled_rejections();
                self.switch_to(home);
                job_rail(self.cur_ctx, "gc");
                self.maybe_collect();
                job_rail(self.cur_ctx, "gc done");
                job_rail(self.cur_ctx, "drain done");
                return Ok(ran);
            }
            job_rail(self.cur_ctx, "gc");
            self.maybe_collect();
            job_rail(self.cur_ctx, "gc done");
        }
    }

    /// Nommage de la callback d'un job pour le breadcrumb
    /// §B-111 — **sans effet de bord** : lecture directe du
    /// heap uniquement (`describe`/`describe_diag` passent par
    /// `get_prop`/`to_js_string`, qui peuvent invoquer des
    /// getters ou `toPrimitive` utilisateur — inacceptables
    /// dans un chemin exécuté avant chaque job). Une fonction
    /// script nomme `nom @ligne:col`, une native son symbole.
    pub(crate) fn job_cb_desc(&self, v: Value) -> String {
        let Value::Obj(id) = v else {
            return format!("{v:?}");
        };
        let Some(o) = self.heap.obj(id) else {
            return format!("dead#{id}");
        };
        match &o.kind {
            ObjKind::Fun(Fun::Script { lit, .. }) => {
                let n = lit.name.as_deref().unwrap_or("<anon>");
                format!("fn {n} @{}:{}", lit.pos.0, lit.pos.1)
            }
            ObjKind::Fun(Fun::Native { name, .. } | Fun::BoundNative { name, .. }) => {
                format!("native {name}")
            }
            ObjKind::Fun(Fun::Bound { .. }) => "bound".to_string(),
            _ => format!("obj#{id}"),
        }
    }
}

/// Breadcrumb de rail §B-111 : nomme la file de livraison
/// ou l'étape en cours — si le drain meurt, la dernière
/// ligne identifie le rail fautif (mutations, observers,
/// streams, idb… ou le sweep GC de checkpoint).
pub(crate) fn job_rail(ctx: u32, rail: &'static str) {
    if crate::dlv_trace() {
        tracing::info!("job: {rail} ctx={ctx}");
    }
}

impl Engine {
    /// Tire `unhandledrejection` pour les promesses encore non
    /// gérées : event `{type, promise, reason, preventDefault}`
    /// sur `onunhandledrejection` + listeners `__et` du global,
    /// puis description poussée dans `unhandled_reports` pour
    /// le drain session (`report_unhandled_rejection` côté core,
    /// FIX-3). Les handlers qui jettent sont avalés (un mur
    /// d'event ne doit pas casser le drain).
    /// Rejets distincts rapportés par checkpoint (§B-21) —
    /// borne anti-tempête : une boucle applicative qui
    /// produit des rejets de promesses variés à chaque tick
    /// (genius `reactAds`, retry non borné) ne peut pas
    /// saturer le drain. Au-delà, les rejets sont avalés
    /// avec une ligne de synthèse.
    const UNHANDLED_REPORT_CAP: u32 = 32;

    fn fire_unhandled_rejections(&mut self) {
        let pending = std::mem::take(&mut self.pending_unhandled);
        // Dédup des descriptions **consécutives** identiques
        // (§B-21 — reactAds : ×255 `r is not defined`
        // strictement identiques en rafale, chaque tick de
        // l'intervalle créant une promesse qui rejette hors
        // du chemin `TIMER_FAIL_CAP`). Le report est
        // regroupé en `(×N)` ; l'event n'est tiré qu'une
        // fois par motif — un handler invité ×255 fois ne
        // change rien au comportement.
        let mut last_desc = String::new();
        let mut same_desc = 0u32;
        let mut reported = 0u32;
        let mut capped = false;
        for (pv, reason) in pending {
            // `.then` tardif → « rejectionhandled » silencieux :
            // la promesse ne remonte plus.
            let still = match self.heap.obj(if let Value::Obj(id) = pv {
                id
            } else {
                continue;
            }) {
                Some(o) => match &o.kind {
                    ObjKind::Promise(p) => !p.handled,
                    _ => false,
                },
                None => false,
            };
            if !still {
                continue;
            }
            // JS-C272 — `[object Object]` remplacé par le
            // descripteur `Object@@tag`/`Object{clés}` (§B-14
            // vol 529 hltb — payload opaque ×4).
            let desc = self.describe_rejection(reason.clone());
            if desc == last_desc {
                same_desc += 1;
                continue;
            }
            if same_desc > 0 {
                self.unhandled_reports.push(format!(
                    "unhandledrejection: {last_desc} (×{})",
                    same_desc + 1
                ));
                same_desc = 0;
            }
            last_desc.clone_from(&desc);
            if reported >= Self::UNHANDLED_REPORT_CAP {
                capped = true;
                continue;
            }
            reported += 1;
            self.unhandled_reports
                .push(format!("unhandledrejection: {desc}"));
            // Sonde §B-97 — nomme la raison d'un rejet
            // silencieux de la boot-chain : un `.then`
            // applicatif fautif tue l'émetteur `yt-*` sans
            // laisser de trace dans le recensement events.
            if let Some((dom, _)) = &self.dom_ctx {
                if tracing::enabled!(target: "dom_probe", tracing::Level::INFO) {
                    if let Ok(mut d) = dom.try_borrow_mut() {
                        if d.ce.probe_seen.insert(format!("pr:{desc}")) {
                            tracing::info!(target: "dom_probe", site = "promise.reject", reason = %desc);
                        }
                    }
                }
            }
            // Event minimal spec (PromiseRejectionEvent).
            let ev = self.new_object();
            let ty = self.str_val("unhandledrejection");
            self.set_own(ev, "type", ty, true);
            self.set_own(ev, "promise", pv.clone(), true);
            self.set_own(ev, "reason", reason.clone(), true);
            self.set_own(ev, "defaultPrevented", Value::Bool(false), true);
            let pd = self.native_fn("preventDefault", rejection_prevent_default);
            self.set_own(ev, "preventDefault", pd, true);
            let ev = Value::Obj(ev);
            let g = Value::Obj(self.global);
            // `onunhandledrejection` (propriété) puis listeners
            // `addEventListener('unhandledrejection', …)` (`__et`).
            let onh = self.get_prop(self.global, "onunhandledrejection");
            if let Some(f) = onh {
                if self.is_callable(f.clone()) {
                    let _ = self.call_value(f, g.clone(), std::slice::from_ref(&ev));
                }
            }
            if let Some(Value::Obj(map)) = self.get_prop(self.global, "__et") {
                if let Some(Value::Obj(l)) = self.get_prop(map, "unhandledrejection") {
                    let len = match self.get_prop(l, "length") {
                        Some(Value::Num(n)) => n as usize,
                        _ => 0,
                    };
                    for i in 0..len {
                        if let Some(cb) = self.get_prop(l, &i.to_string()) {
                            if self.is_callable(cb.clone()) {
                                let _ = self.call_value(cb, g.clone(), std::slice::from_ref(&ev));
                            }
                        }
                    }
                }
            }
            // JS-C357 — depuis JS-C355, `window.addEventListener`
            // range dans `node.listeners` (racine DOM), plus dans
            // `__et` : l'event doit traverser `propagate_event`
            // sur la racine sinon les handlers de la page ne le
            // voient jamais (trou de conformité — les rejets de
            // la boot-chain restaient invisibles des deux côtés).
            if let Some((dom, cache)) = self.dom_ctx.clone() {
                let root = dom.borrow().root;
                let state = crate::dom_host::make_event_state(
                    "unhandledrejection",
                    root,
                    crate::dom_host::EventFlags {
                        bubbles: true,
                        cancelable: true,
                        composed: false,
                        trusted: true,
                    },
                    vec![("promise".to_string(), pv), ("reason".to_string(), reason)],
                );
                // Handlers qui jettent : avalés, jamais de
                // casse du drain (même contrat que le rail
                // `__et` ci-dessus).
                let _ = self.propagate_event(&dom, &cache, &state);
            }
        }
        if same_desc > 0 {
            self.unhandled_reports.push(format!(
                "unhandledrejection: {last_desc} (×{})",
                same_desc + 1
            ));
        }
        if capped {
            self.unhandled_reports
                .push("unhandledrejection: … — rafale de rejets bornée".to_string());
        }
    }
}

/// `ev.preventDefault()` — pose `defaultPrevented`.
fn rejection_prevent_default(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    if let Value::Obj(id) = this {
        e.set_prop_value(Value::Obj(id), "defaultPrevented", Value::Bool(true))?;
    }
    Ok(Value::Undefined)
}

/// `resolve` bound native: `bound = [promise_id]`.
fn promise_resolve(e: &mut Engine, _this: Value, args: &[Value]) -> JsResult<Value> {
    let Some(Value::Obj(id)) = args.first().cloned() else {
        return Err(e.type_error("resolve: missing promise"));
    };
    let v = args.get(1).cloned().unwrap_or(Value::Undefined);
    e.resolve_promise(id, v);
    Ok(Value::Undefined)
}

/// `reject` bound native: `bound = [promise_id]`.
fn promise_reject(e: &mut Engine, _this: Value, args: &[Value]) -> JsResult<Value> {
    let Some(Value::Obj(id)) = args.first().cloned() else {
        return Err(e.type_error("reject: missing promise"));
    };
    let v = args.get(1).cloned().unwrap_or(Value::Undefined);
    e.reject_promise(id, v);
    Ok(Value::Undefined)
}

// -- builtins ---------------------------------------------------------------

/// `new Promise(executor)`.
fn promise_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let executor = args.first().cloned().unwrap_or(Value::Undefined);
    if !e.is_callable(executor.clone()) {
        return Err(e.type_error("Promise resolver is not a function"));
    }
    let id = e.new_promise();
    e.probe_prom_src(id, executor.clone());
    let (resolve, reject) = {
        let Some(o) = e.heap.obj(id) else {
            return Err(e.type_error("internal promise error"));
        };
        let ObjKind::Promise(p) = &o.kind else {
            return Err(e.type_error("internal promise error"));
        };
        (p.resolve_fn.clone(), p.reject_fn.clone())
    };
    if let Err(Thrown(t)) = e.call_value(executor, Value::Undefined, &[resolve, reject]) {
        e.reject_promise(id, t);
    }
    Ok(Value::Obj(id))
}

/// `p.then(onF, onR)` → chained promise.
/// `get Promise [@@species]` — spec `get { return this }` :
/// le ctor du résultat de `then`/`catch`/`finally` se dérive
/// par `SpeciesConstructor` — sous-classes héritées (`Q`
/// reçoit `this === Q` → `q.then() instanceof Q`), la
/// détection core-js lit `ctor[@@species]` sur le natif
/// (JS-C280).
fn promise_species_get(_e: &mut Engine, this: Value, _args: &[Value]) -> JsResult<Value> {
    Ok(this)
}

/// Exécuteur `NewPromiseCapability` (spec `GetCapabilitiesExecutor`)
/// — `bound[0]` = boîte `[resolve, reject]` remplie au premier
/// appel ; un second appel est un `TypeError` (spec).
fn promise_cap_executor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let (slot, resolve, reject) = (arg(args, 0), arg(args, 1), arg(args, 2));
    let Value::Obj(sid) = slot else {
        return Err(e.type_error("capability executor missing slot"));
    };
    if e.get_prop(sid, "0")
        .is_some_and(|v| !matches!(v, Value::Undefined))
    {
        return Err(e.type_error("promise capability executor called twice"));
    }
    e.set_prop(sid, "0", resolve)?;
    e.set_prop(sid, "1", reject)?;
    Ok(Value::Undefined)
}

fn promise_then(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = this else {
        return Err(e.type_error("then called on non-promise"));
    };
    let on_f = args.first().cloned().unwrap_or(Value::Undefined);
    let on_r = args.get(1).cloned().unwrap_or(Value::Undefined);
    // Spec `PerformPromiseThen` : `C = SpeciesConstructor(this,
    // %Promise%)`, `next = NewPromiseCapability(C)` — le ctor de
    // résultat se lit via `constructor`/`@@species` (JS-C280 —
    // la détection `FORCED_PROMISE_CONSTRUCTOR` de core-js exige
    // `p.then(…) instanceof FakePromise` pour garder le natif ;
    // un `new_promise` inconditionnel la faisait échouer →
    // `globalThis.Promise` remplacé par un polyfill sans
    // `finally` — vol 577 dailymotion).
    let c = e.species_ctor(this)?;
    let (cap_promise, next) = e.new_promise_capability(c)?;
    let (state, result) = {
        let Some(o) = e.heap.obj_mut(id) else {
            return Err(e.type_error("then called on non-promise"));
        };
        let ObjKind::Promise(p) = &mut o.kind else {
            return Err(e.type_error("then called on non-promise"));
        };
        // Spec `[[PromiseIsHandled]]` : tout `then` marque la
        // promesse (même sans handler — le rejet propage à la
        // chaîne) ; coupe `unhandledrejection` sur `.then` tardif.
        p.handled = true;
        (p.state, p.result.clone())
    };
    match state {
        PromiseState::Pending => {
            e.sched_tick("then");
            if let Some(o) = e.heap.obj_mut(id) {
                if let ObjKind::Promise(p) = &mut o.kind {
                    p.reactions.push(Reaction {
                        on_fulfilled: on_f,
                        on_rejected: on_r,
                        next,
                    });
                }
            }
            e.probe_prom_then(id);
        }
        PromiseState::Fulfilled | PromiseState::Rejected => {
            let rejected = state == PromiseState::Rejected;
            e.sched_tick("post.reaction");
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
    Ok(cap_promise)
}

/// `p.catch(onR)` ≡ `then(undefined, onR)`.
fn promise_catch(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    promise_then(e, this, &[Value::Undefined, arg(args, 0)])
}

/// Bound `finally` handler — `bound = [cb]`; fulfill path returns the
/// original value after running `cb`.
fn promise_finally_ok(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let cb = args.first().cloned().unwrap_or(Value::Undefined);
    let v = args.get(1).cloned().unwrap_or(Value::Undefined);
    if e.is_callable(cb.clone()) {
        e.call_value(cb, this, &[])?;
    }
    Ok(v)
}

/// Bound `finally` handler — reject path rethrows the reason.
fn promise_finally_err(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let cb = args.first().cloned().unwrap_or(Value::Undefined);
    let v = args.get(1).cloned().unwrap_or(Value::Undefined);
    if e.is_callable(cb.clone()) {
        e.call_value(cb, this, &[])?;
    }
    Err(Thrown(v))
}

/// `p.finally(cb)`.
fn promise_finally(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let cb = arg(args, 0);
    let ok = e.bound_native("finally", promise_finally_ok, vec![cb.clone()]);
    let err = e.bound_native("finally", promise_finally_err, vec![cb]);
    promise_then(e, this, &[ok, err])
}

/// `Promise.resolve(v)` — spec `PromiseResolve(C, v)` : `v` déjà
/// promesse du ctor `this` → retournée telle quelle ; sinon
/// `NewPromiseCapability(this)` — `Q.resolve(…)` sur une sous-
/// classe rend une instance de Q (JS-C280).
fn promise_static_resolve(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let v = arg(args, 0);
    if e.is_promise(v.clone()) {
        // `v.constructor === C` → `v` renvoyée (spec) — le `Get`
        // peut jeter (accesseur hostile) : propagé.
        if let Ok(c) = e.get_prop_value(v.clone(), "constructor") {
            if matches!((c, this.clone()), (Value::Obj(a), Value::Obj(b)) if a == b) {
                return Ok(v);
            }
        }
    }
    let (promise, sink) = e.new_promise_capability(this)?;
    match sink {
        ThenSink::Native(id) => e.resolve_promise(id, v),
        ThenSink::Foreign { resolve, .. } => {
            e.call_value(resolve, Value::Undefined, &[v])?;
        }
    }
    Ok(promise)
}

/// `Promise.reject(r)` — spec `NewPromiseCapability(this)`.
fn promise_static_reject(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (promise, sink) = e.new_promise_capability(this)?;
    let r = arg(args, 0);
    match sink {
        ThenSink::Native(id) => e.reject_promise(id, r),
        ThenSink::Foreign { reject, .. } => {
            e.call_value(reject, Value::Undefined, &[r])?;
        }
    }
    Ok(promise)
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
        let pv = if e.is_promise(v.clone()) {
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
            p.reject_fn.clone()
        };
        promise_then(e, pv, &[on_f, on_r])?;
    }
    Ok(Value::Obj(id))
}

/// `all` per-element fulfill — `bound = [state, index]`.
fn promise_all_fulfill(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let Some(Value::Obj(state)) = args.first().cloned() else {
        return Err(e.type_error("all: missing state"));
    };
    let i = e.to_number(args.get(1).cloned().unwrap_or(Value::Num(0.0)))? as usize;
    let v = args.get(2).cloned().unwrap_or(Value::Undefined);
    let (results, target) = {
        let r = e.get_prop(state, "results").unwrap_or(Value::Undefined);
        let t = e.get_prop(state, "target").unwrap_or(Value::Undefined);
        (r, t)
    };
    if let (Value::Obj(rid), Value::Obj(tid)) = (&results, &target) {
        if let Some(o) = e.heap.obj_mut(*rid) {
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
            e.resolve_promise(*tid, results);
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
        (p.resolve_fn.clone(), p.reject_fn.clone())
    };
    for v in vals {
        if e.is_promise(v.clone()) {
            promise_then(e, v, &[resolve.clone(), reject.clone()])?;
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
        let pv = if e.is_promise(v.clone()) {
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
    let Some(Value::Obj(state)) = args.first().cloned() else {
        return Err(e.type_error("allSettled: missing state"));
    };
    let i = e.to_number(args.get(1).cloned().unwrap_or(Value::Num(0.0)))? as usize;
    let v = args.get(2).cloned().unwrap_or(Value::Undefined);
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
    if let (Value::Obj(rid), Value::Obj(tid)) = (&results, &target) {
        if let Some(o) = e.heap.obj_mut(*rid) {
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
            e.resolve_promise(*tid, results);
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
        let pv = if e.is_promise(v.clone()) {
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
            p.resolve_fn.clone()
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
    let Some(Value::Obj(state)) = args.first().cloned() else {
        return Err(e.type_error("any: missing state"));
    };
    let i = e.to_number(args.get(1).cloned().unwrap_or(Value::Num(0.0)))? as usize;
    let reason = args.get(2).cloned().unwrap_or(Value::Undefined);
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
        (p.resolve_fn.clone(), p.reject_fn.clone())
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
    let f = args.first().cloned().unwrap_or(Value::Undefined);
    let end = args.len().saturating_sub(1).max(1);
    let extras: Vec<Value> = args.get(1..end).unwrap_or(&[]).to_vec();
    e.call_value(f, Value::Undefined, &extras)
}

/// `queueMicrotask(f)`.
fn queue_microtask(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let f = arg(args, 0);
    if !e.is_callable(f.clone()) {
        return Err(e.type_error("queueMicrotask: not a function"));
    }
    e.enqueue_microtask(f);
    Ok(Value::Undefined)
}

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).cloned().unwrap_or(Value::Undefined)
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
        crate::promise::c_id(ctor.clone()),
        "prototype",
        Value::Obj(rp),
        false,
    );
    e.set_own(rp, "constructor", ctor.clone(), false);
    if let Value::Obj(cid) = ctor {
        // `%Promise%` intrinsèque du realm — défaut de
        // `SpeciesConstructor`, résistant à la réécriture de
        // `Promise.prototype.constructor` ou `global.Promise`
        // (JS-C280).
        e.protos.promise_ctor = cid;
        // `get Promise[@@species] { return this }` — le ctor du
        // résultat de `then` se dérive par SpeciesConstructor.
        let sg = e.native_fn("get [Symbol.species]", promise_species_get);
        if let Some(o) = e.heap.obj_mut(cid) {
            o.set_accessor(crate::interp::SYM_SPECIES, sg, Value::Undefined, false);
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Subscriber minimal — arme la cible `dom_probe` seule,
    /// ignore tout le reste (la sonde JS-C383 gate sur
    /// `enabled!` ; aucune dépendance `tracing-subscriber`
    /// n'est requise pour un test in-crate).
    struct ArmProbe;
    impl tracing::Subscriber for ArmProbe {
        fn enabled(&self, m: &tracing::Metadata<'_>) -> bool {
            m.target() == "dom_probe"
        }
        fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
        fn event(&self, _: &tracing::Event<'_>) {}
        fn enter(&self, _: &tracing::span::Id) {}
        fn exit(&self, _: &tracing::span::Id) {}
    }

    /// JS-C383 — la sonde nomme la promesse pendante : créée
    /// en `<top>`, `.then` souscrit depuis `f` ; `collect`
    /// conserve l'entrée (encore `Pending` + réaction).
    #[test]
    fn probe_noms_les_promesses_pendantes() {
        let _g = tracing::subscriber::set_default(ArmProbe);
        // Les callsites `enabled!` sont cachés — les tests en
        // parallèle les ont réchauffés désarmés avant
        // `set_default` ; il faut reconstruire le cache
        // d'intérêt sinon la sonde reste muette ici (en vol,
        // le subscriber est posé au boot avant tout callsite).
        tracing::callsite::rebuild_interest_cache();
        let mut e = Engine::new();
        let v = e.eval(
            "let p = new Promise(function(){}); function f(){ p.then(function(){}); } f();\
             var q; new Promise(function(res){ q = res; }); function g(){ q(1); } g();",
        );
        assert!(v.is_ok(), "eval: {v:?}");
        e.collect();
        let named_then = e
            .prom_sites
            .values()
            .any(|pr| pr.thens.iter().any(|t| t.starts_with("f(")));
        assert!(named_then, "prom_sites doit nommer f: {:?}", {
            let v: Vec<_> = e
                .prom_sites
                .values()
                .map(|pr| (pr.created.to_string(), pr.thens.len()))
                .collect();
            v
        });
        // Le label `created` peut tomber `<?>` quand le callsite
        // `enabled!` de `probe_prom_created` a été gelé `never`
        // par les tests parallèles (cache d'intérêt global) —
        // le signal du vol est le nommage des `.then`, prouvé
        // ci-dessus ; `created` est non vide quoi qu'il arrive.
        assert!(
            e.prom_sites.values().all(|pr| !pr.created.is_empty()),
            "created doit etre non vide"
        );
        // JS-C386 — `prom.settle` : la promesse résolue depuis
        // `g` porte le site du settle nommé.
        let settle_named = e.prom_sites.values().any(|pr| {
            matches!(
                &pr.settled,
                Some((site, PromiseState::Fulfilled)) if site.starts_with("g(")
            )
        });
        assert!(settle_named, "settle doit nommer g: {:?}", {
            let v: Vec<_> = e
                .prom_sites
                .values()
                .map(|pr| pr.settled.as_ref().map(|(s, _)| s.to_string()))
                .collect();
            v
        });
    }

    /// JS-C412 — `prom.src` : l'extrait source de l'executor
    /// est retenu **copié et borné** (`PROM_SRC_CHARS`) — le
    /// code de la fabrique pendante devient lisible dans le
    /// log, sans retenir le `Rc` du bundle.
    #[test]
    fn c412_prom_src_dumps_executor_excerpt() {
        let _g = tracing::subscriber::set_default(ArmProbe);
        // Course de scheduling : le cache d'intérêt **global**
        // peut être re-gelé `never` par un test parallèle entre
        // le `rebuild_interest_cache` et l'éval → la sonde reste
        // muette (`src=[]` observé en suite complète). On
        // retente le couple rebuild+éval tant que l'extrait
        // n'a pas atterri — le verrou porte sur le mécanisme
        // (extrait copié+borné), pas sur la course.
        let mut found = false;
        let mut last = Vec::new();
        for _ in 0..8 {
            if found {
                break;
            }
            tracing::callsite::rebuild_interest_cache();
            let mut e = Engine::new();
            let v = e.eval(
                "new Promise(function gatekeeper(){\
                     if (window.__ready) { resolve(); } poll(); });",
            );
            assert!(v.is_ok(), "eval: {v:?}");
            found = e.prom_sites.values().any(|pr| {
                pr.src
                    .as_deref()
                    .is_some_and(|s| s.contains("gatekeeper") && s.contains("__ready"))
            });
            last = e.prom_sites.values().map(|pr| pr.src.clone()).collect();
            // Borné : jamais plus de PROM_SRC_CHARS caractères.
            assert!(e.prom_sites.values().all(|pr| {
                pr.src
                    .as_deref()
                    .is_none_or(|s| s.chars().count() <= PROM_SRC_CHARS)
            }));
        }
        assert!(found, "extrait de la fabrique attendu: {last:?}");
    }

    /// JS-C412 — désarmée, la sonde est muette : aucune entrée
    /// `prom_sites`, aucun extrait retenu.
    #[test]
    fn c412_prom_src_disarmed_is_silent() {
        // Pas de subscriber → `enabled!` faux → zéro coût.
        let mut e = Engine::new();
        e.eval("new Promise(function(){ poll(); });").ok();
        e.eval("var p = new Promise(function(){}); p.then(function(){});")
            .ok();
        assert!(
            e.prom_sites.is_empty(),
            "désarmée, prom_sites doit rester vide"
        );
    }
}
