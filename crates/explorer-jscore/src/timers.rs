// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Host-side timers — `setTimeout`/`setInterval`/`clearTimeout`/
//! `clearInterval` (JS-3).
//!
//! The engine owns the *queue*; the embedder owns the clock: the
//! browser's event loop (tokio, ADR-0003) calls
//! [`Engine::next_timer_deadline`] to sleep, then
//! [`Engine::run_due_timers`] to fire. Timers never run inside the
//! interpreter — a `while(1){}` in a callback still dies on
//! `op_limit`, and nothing preempts a running script.
//!
//! Deliberate subset: callback must be a function value, or — the
//! legacy `setTimeout('code()')` form — a string, which the pump
//! re-parses via [`Engine::eval_source`] at fire time (global
//! scope, like a real indirect eval).

use crate::heap::JsResult;
use crate::value::Value;
use crate::Engine;
use std::collections::HashSet;
use std::time::{Duration, Instant};

/// One scheduled timer.
#[derive(Debug)]
struct Timer {
    id: u64,
    /// Next fire time.
    at: Instant,
    /// `Some(ms)` for `setInterval`, `None` for one-shot.
    interval_ms: Option<u64>,
    /// Callback function value (`Str` = legacy eval-at-fire).
    cb: Value,
    /// Extra args forwarded to the callback (`setTimeout(f,ms,a,b)`).
    cb_args: Vec<Value>,
}

/// Pending timers for one engine (page).
#[derive(Debug, Default)]
pub(crate) struct TimerQueue {
    next_id: u64,
    timers: Vec<Timer>,
    /// Ids annulés — un timer *déjà extrait* dans la pompe courante
    /// n'est plus dans `timers` quand son callback (ou un callback
    /// antérieur) l'annule : `cancel` le marque ici, la pompe le
    /// saute/ne le réarme pas.
    cancelled: HashSet<u64>,
}

impl TimerQueue {
    /// Toutes les `Value` retenues par les timers en attente —
    /// racines GC (un callback planifié garde ses captures vivantes).
    pub(crate) fn roots(&self, out: &mut Vec<Value>) {
        for t in &self.timers {
            out.push(t.cb);
            out.extend(t.cb_args.iter().copied());
        }
    }

    pub(crate) fn schedule(
        &mut self,
        cb: Value,
        delay_ms: u64,
        interval: bool,
        cb_args: Vec<Value>,
    ) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        self.timers.push(Timer {
            id,
            at: Instant::now() + Duration::from_millis(delay_ms),
            interval_ms: interval.then_some(delay_ms),
            cb,
            cb_args,
        });
        id
    }

    fn cancel(&mut self, id: u64) {
        self.timers.retain(|t| t.id != id);
        self.cancelled.insert(id);
    }
}

impl Engine {
    /// Install `setTimeout`/`setInterval`/`clearTimeout`/
    /// `clearInterval` on the global object.
    pub fn install_timers(&mut self) {
        let g = self.global;
        for (name, f) in [
            ("setTimeout", timer_set_timeout as crate::heap::NativeFn),
            ("setInterval", timer_set_interval),
            ("clearTimeout", timer_clear),
            ("clearInterval", timer_clear),
            ("requestAnimationFrame", timer_raf),
            ("cancelAnimationFrame", timer_raf_cancel),
            ("requestIdleCallback", timer_ric),
            ("cancelIdleCallback", timer_raf_cancel),
        ] {
            let v = self.native_fn(name, f);
            self.set_own(g, name, v, false);
        }
    }

    /// Run every due timer (earliest first); returns how many fired.
    /// Reschedules intervals. Errors from a callback propagate —
    /// remaining due timers still run on the next pump.
    ///
    /// # Errors
    /// Propagates the first uncaught script exception from a callback.
    pub fn run_due_timers(&mut self) -> JsResult<usize> {
        self.run_due_timers_bounded(usize::MAX)
    }

    /// Bounded variant (étape 52, ADR-0019) : at most `max` due
    /// timers fire — the rest stay queued for the next pump (a
    /// page registering thousands of due timers cannot
    /// monopolise a tick).
    ///
    /// # Errors
    /// Same as [`Self::run_due_timers`].
    pub fn run_due_timers_bounded(&mut self, max: usize) -> JsResult<usize> {
        // JS-C78 : les timers vivent par contexte — une pompe
        // visite chaque contexte vivant (les timers d'un iframe
        // tournent même quand son document n'est pas courant).
        let home = self.cur_ctx;
        let ids: Vec<u32> = self
            .ctxs
            .iter()
            .enumerate()
            .filter(|(_, m)| m.alive)
            .map(|(i, _)| i as u32)
            .collect();
        let mut ran = 0usize;
        for id in ids {
            if ran >= max {
                break;
            }
            self.switch_to(id);
            if self.cur_ctx != id {
                continue;
            }
            ran += self.run_due_timers_ctx(max - ran)?;
        }
        self.switch_to(home);
        // Microtask checkpoint at the task boundary — une fois la
        // tournée terminée (`run_microtasks` draine déjà tous les
        // contextes, JS-C78).
        self.run_microtasks()?;
        Ok(ran)
    }

    /// Due timers du contexte **courant** uniquement — corps de
    /// pompe partagé par [`Self::run_due_timers_bounded`] pour
    /// chaque contexte visité (JS-C78).
    fn run_due_timers_ctx(&mut self, max: usize) -> JsResult<usize> {
        // Snapshot the due timers first: an interval rescheduled to
        // `now` would otherwise be picked up again in this same pump
        // and loop forever. One pump = each due timer fires at most
        // once (browsers behave the same way per task turn).
        let now = Instant::now();
        let mut due: Vec<Timer> = Vec::new();
        let mut kept = Vec::with_capacity(self.timers.timers.len());
        for t in self.timers.timers.drain(..) {
            if t.at <= now {
                due.push(t);
            } else {
                kept.push(t);
            }
        }
        self.timers.timers = kept;
        due.sort_by_key(|t| t.at);
        // Borne : les `due` au-delà de `max` sont re-fileés —
        // ordre conservé au prochain drain (re-tri).
        if due.len() > max {
            self.timers.timers.extend(due.split_off(max));
        }
        let mut ran = 0usize;
        for t in due {
            // Annulé par un callback *plus tôt dans cette même pompe*
            // (le timer était déjà extrait quand `clearTimeout` a
            // tourné → `retain` ne l'a pas vu).
            if self.timers.cancelled.remove(&t.id) {
                continue;
            }
            // Each callback gets a fresh op budget.
            self.ops_left = self.op_limit;
            if self.is_callable(t.cb) {
                let args = t.cb_args.clone();
                self.call_value(t.cb, Value::Obj(self.global), &args)?;
            } else if let Value::Str(sid) = t.cb {
                // Legacy string form — parsed & run in global scope
                // at fire time, not at schedule time.
                let src = self.heap.string(sid).unwrap_or_default();
                self.eval_source(&src)?;
            }
            // `clearInterval` dans son propre callback — le pattern
            // canonique — empêche le réarmement (sinon boucle
            // infinie : l'entrée `due` est déjà hors de `timers`).
            if let Some(ms) = t.interval_ms {
                if !self.timers.cancelled.remove(&t.id) {
                    self.timers.timers.push(Timer {
                        at: Instant::now() + Duration::from_millis(ms),
                        ..t
                    });
                }
            }
            ran += 1;
        }
        Ok(ran)
    }

    /// Earliest pending fire time — the embedder sleeps until then
    /// (or on its next UI event, whichever first). JS-C78 : min
    /// sur tous les contextes vivants (courant + garés).
    #[must_use]
    pub fn next_timer_deadline(&self) -> Option<Instant> {
        self.timers
            .timers
            .iter()
            .map(|t| t.at)
            .chain(
                self.parked
                    .values()
                    .flat_map(|c| c.timers.timers.iter().map(|t| t.at)),
            )
            .min()
    }

    /// Pending timer count (tests/diagnostics) — tous contextes.
    #[must_use]
    pub fn pending_timers(&self) -> usize {
        self.timers.timers.len()
            + self
                .parked
                .values()
                .map(|c| c.timers.timers.len())
                .sum::<usize>()
    }
}

/// `setTimeout(fn, ms)` → timer id.
fn timer_set_timeout(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let cb = arg(args, 0);
    let ms = delay_ms(e, args.get(1).copied())?;
    let cb_args = args.get(2..).unwrap_or(&[]).to_vec();
    let id = e.timers.schedule(cb, ms, false, cb_args);
    Ok(Value::Num(id as f64))
}

/// `setInterval(fn, ms, ...args)` → timer id.
fn timer_set_interval(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let cb = arg(args, 0);
    let ms = delay_ms(e, args.get(1).copied())?;
    let cb_args = args.get(2..).unwrap_or(&[]).to_vec();
    let id = e.timers.schedule(cb, ms, true, cb_args);
    Ok(Value::Num(id as f64))
}

/// `requestAnimationFrame(cb)` ≙ `setTimeout(cb, 16)` — no real vsync
/// source in the host loop, so ~60fps by timer is the best we can do
/// (documented deviation; the callback still gets a timestamp arg).
fn timer_raf(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let cb = arg(args, 0);
    let ts = Value::Num(e.t0.elapsed().as_secs_f64() * 1000.0);
    let id = e.timers.schedule(cb, 16, false, vec![ts]);
    Ok(Value::Num(id as f64))
}

/// `cancelAnimationFrame(id)` ≙ `clearTimeout`.
fn timer_raf_cancel(e: &mut Engine, t: Value, args: &[Value]) -> JsResult<Value> {
    timer_clear(e, t, args)
}

// -- `requestIdleCallback` (CAP-165) -------------------------------------

/// `requestIdleCallback(cb, {timeout})` → id. Pas de vraie
/// période « idle » côté hôte : sans `timeout` le callback est
/// planifié à 0 ms — la prochaine pompe de la boucle *est* le
/// moment creux (déviation bornée, même contrat que
/// `requestAnimationFrame` ≙ `setTimeout(16)`). Avec
/// `{timeout: ms}` le tir part à l'échéance et `didTimeout`
/// vaut `true` (le délai est réellement écoulé — le timer *est*
/// l'échéance). `cancelIdleCallback(id)` ≙ `clearTimeout`.
fn timer_ric(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let cb = arg(args, 0);
    if !e.is_callable(cb) {
        return Err(e.type_error("requestIdleCallback: callback is not a function"));
    }
    let opts = arg(args, 1);
    let timeout = if matches!(opts, Value::Obj(_)) {
        let v = e.get_prop_value(opts, "timeout")?;
        // `{timeout: undefined}`/`{}` = pas d'échéance (spec :
        // la prop doit être un nombre pour armer le timeout).
        if matches!(v, Value::Num(_)) {
            Some(delay_ms(e, Some(v))?)
        } else {
            None
        }
    } else {
        None
    };
    let (delay, did_timeout) = timeout.map_or((0, false), |ms| (ms, true));
    // `IdleDeadline` : objet ordinaire — `didTimeout` en data
    // prop, `timeRemaining()` = native liée portant l'instant
    // d'ordonnancement (ms depuis `t0`) ; reliquat d'une
    // tranche « idle » de 50 ms (spec : ≤ 50 — tir rapproché
    // ≈ tranche entière, tir sur échéance ≈ 0).
    let deadline = e.new_object();
    e.set_own(deadline, "didTimeout", Value::Bool(did_timeout), true);
    let sched = Value::Num(e.t0.elapsed().as_secs_f64() * 1000.0);
    let tr = e.bound_native("timeRemaining", idle_time_remaining, vec![sched]);
    e.set_own(deadline, "timeRemaining", tr, true);
    let id = e
        .timers
        .schedule(cb, delay, false, vec![Value::Obj(deadline)]);
    Ok(Value::Num(id as f64))
}

/// `IdleDeadline.timeRemaining()` — `args[0]` = instant
/// d'ordonnancement lié (ms depuis `t0`) ; reliquat de la
/// tranche de 50 ms, plancher 0.
fn idle_time_remaining(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let sched_ms = match arg(args, 0) {
        Value::Num(ms) => ms,
        _ => 0.0,
    };
    let now_ms = e.t0.elapsed().as_secs_f64() * 1000.0;
    Ok(Value::Num((50.0 - (now_ms - sched_ms)).max(0.0)))
}

/// `clearTimeout(id)` / `clearInterval(id)`.
fn timer_clear(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    if let Value::Num(n) = arg(args, 0) {
        e.timers.cancel(n as u64);
    }
    Ok(Value::Undefined)
}

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).copied().unwrap_or(Value::Undefined)
}

/// Delay arg → ms (default 0, negative clamps — browsers clamp at
/// 0/4ms; we clamp at 0, nested-clamping is a fingerprint anyway).
fn delay_ms(e: &mut Engine, v: Option<Value>) -> JsResult<u64> {
    let n = match v {
        Some(Value::Num(n)) => n,
        Some(other) => e.to_number(other)?,
        None => 0.0,
    };
    Ok(if n.is_finite() && n > 0.0 {
        n as u64
    } else {
        0
    })
}
