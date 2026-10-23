// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `IntersectionObserver`/`ResizeObserver` (JS-C69) — succède
//! aux stubs passifs JS-C56 (callback retenu mais jamais
//! appelé). Livraison au checkpoint microtask (même couture
//! que `MutationObserver`) : chaque `observe(target)` enfile
//! une **notification initiale** (spec : « queue an
//! observation » — les navigateurs rapportent toujours l'état
//! courant à la première passe).
//!
//! Géométrie honnête : le miroir n'a pas de moteur de mise en
//! page — les rectangles reprennent la règle des stubs de
//! dimensions (`documentElement`/`body` ≙ viewport, 0 sinon,
//! cf. `clientWidth`). `isIntersecting` ≙ élément rattaché au
//! document (un élément détaché ne peut intersecter) ;
//! `intersectionRatio` 1/0 en conséquence. `thresholds`,
//! `root` et `rootMargin` sont validés/exposés mais non
//! appliqués (pas de géométrie réelle à filtrer).

use std::collections::HashMap;
use std::rc::Rc;

use crate::dom::NodeId;
use crate::dom_host::{node_for, node_value, DomRef, NodeCache};
use crate::heap::{Host, JsResult, ObjKind};
use crate::value::{ObjId, Value};
use crate::Engine;

/// État moteur des observers géométriques — registrations et
/// notifications pendantes vivent ici (les `NodeId` du miroir
/// survivent aux wrappers, cf. `MutObsState`).
#[derive(Default)]
pub(crate) struct ObsState {
    /// Prochain id d'observer alloué.
    next_id: u64,
    /// Observers vivants par id — `disconnect` les retire.
    regs: HashMap<u64, ObsReg>,
}

/// Registration d'un observer — callback + wrapper + cibles
/// + notifications initiales encore à livrer.
struct ObsReg {
    /// `IntersectionObserver` ou `ResizeObserver`.
    kind: ObsKind,
    /// Callback JS `(entries, observer)`.
    cb: Value,
    /// Wrapper JS de l'observer (second argument du callback).
    wrapper: ObjId,
    /// `options.root` (`IntersectionObserver`) — `None` =
    /// viewport implicite (spec).
    root: Option<NodeId>,
    /// Cibles observées, ordre d'`observe` (set — le spec
    /// ignore le re-observe d'une même cible).
    targets: Vec<NodeId>,
    /// Cibles en attente de leur notification initiale.
    pending: Vec<NodeId>,
}

/// Famille d'observer — sélectionne le format d'entry.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ObsKind {
    /// `IntersectionObserver` — entry `IntersectionObserverEntry`.
    Intersection,
    /// `ResizeObserver` — entry `ResizeObserverEntry`.
    Resize,
}

impl ObsState {
    /// Racines GC : callbacks et wrappers des observers
    /// enregistrés (un observer déconnecté est retiré de
    /// `regs` — le reste doit survivre comme `MutObsState`).
    pub(crate) fn root_ids(&self, ids: &mut Vec<ObjId>) {
        for reg in self.regs.values() {
            crate::heap::push_value(ids, reg.cb);
            ids.push(reg.wrapper);
        }
    }
}

/// Host object `IntersectionObserver`/`ResizeObserver` —
/// `id` corrèle la registration moteur, `kind` le nom de
/// classe (`[object ResizeObserver]`…).
struct ObsHost {
    dom: DomRef,
    cache: NodeCache,
    id: u64,
    kind: ObsKind,
}

impl Engine {
    /// Installe les deux constructeurs — remplace les natives
    /// passives JS-C56 (le miroir est requis à l'usage, pas à
    /// l'installation ; les ctors lisent `dom_ctx` à l'appel).
    pub(crate) fn install_observers(&mut self) {
        for (name, f) in [
            (
                "IntersectionObserver",
                intersection_observer_ctor as crate::heap::NativeFn,
            ),
            ("ResizeObserver", resize_observer_ctor),
        ] {
            let fv = self.native_fn(name, f);
            if let Value::Obj(fid) = fv {
                let p = self.new_object();
                self.set_own(fid, "prototype", Value::Obj(p), false);
            }
            let g = self.global;
            self.set_own(g, name, fv, false);
        }
    }

    /// Livraison des notifications en file (JS-C69) — un appel
    /// par observer notifié, ordre de création ; les callbacks
    /// peuvent `observe`/`disconnect` (la file reboucle au
    /// checkpoint suivant, bornée par le budget ops).
    ///
    /// Retourne le nombre d'observers notifiés.
    ///
    /// # Errors
    /// Propage la première exception d'un callback (même
    /// modèle que les listeners et `deliver_mutations`).
    pub(crate) fn deliver_observers(&mut self) -> JsResult<usize> {
        let Some((dom, cache)) = self.dom_ctx.clone() else {
            return Ok(0);
        };
        // Ordre de création des observers (ids monotone).
        let mut order: Vec<u64> = self.obs.regs.keys().copied().collect();
        order.sort_unstable();
        let mut delivered = 0usize;
        for id in order {
            let (kind, cb, wrapper, root, pending) = {
                let Some(reg) = self.obs.regs.get_mut(&id) else {
                    continue;
                };
                if reg.pending.is_empty() {
                    continue;
                }
                (
                    reg.kind,
                    reg.cb,
                    reg.wrapper,
                    reg.root,
                    std::mem::take(&mut reg.pending),
                )
            };
            let entries: Vec<Value> = pending
                .iter()
                .map(|&n| self.observer_entry(kind, &dom, &cache, root, n))
                .collect();
            let arr = self.new_array(entries);
            self.call_value(cb, Value::Obj(wrapper), &[arr, Value::Obj(wrapper)])?;
            delivered += 1;
        }
        Ok(delivered)
    }

    /// `Entry` JS d'une notification — format selon `kind`
    /// (géométrie miroir honnête : viewport pour
    /// `documentElement`/`body`, 0 sinon — même règle que
    /// `clientWidth` ; `isIntersecting` ≙ `is_attached`).
    fn observer_entry(
        &mut self,
        kind: ObsKind,
        dom: &DomRef,
        cache: &NodeCache,
        root: Option<NodeId>,
        node: NodeId,
    ) -> Value {
        let o = self.new_object();
        let target = node_value(self, dom, cache, node);
        self.set_own(o, "target", target, true);
        match kind {
            ObsKind::Intersection => {
                let attached = dom.borrow().is_attached(node);
                self.set_own(o, "isIntersecting", Value::Bool(attached), true);
                let ratio = if attached { 1.0 } else { 0.0 };
                self.set_own(o, "intersectionRatio", Value::Num(ratio), true);
                let time = self.t0.elapsed().as_secs_f64() * 1000.0;
                self.set_own(o, "time", Value::Num(time), true);
                let rect = self.rect_value(dom, node);
                self.set_own(o, "boundingClientRect", rect, true);
                // Pas de root réel mesurable : élément intersectant
                // ≙ rect complet, sinon rect nul.
                let zero = self.rect_zero();
                self.set_own(
                    o,
                    "intersectionRect",
                    if attached { rect } else { zero },
                    true,
                );
                let rb = match root {
                    Some(rn) => self.rect_value(dom, rn),
                    None => self.viewport_rect(),
                };
                self.set_own(o, "rootBounds", rb, true);
            }
            ObsKind::Resize => {
                let rect = self.rect_value(dom, node);
                self.set_own(o, "contentRect", rect, true);
                let (w, h) = {
                    let d = dom.borrow();
                    let is_root = node == d.body() || Some(node) == d.document_element();
                    if is_root {
                        (
                            f64::from(self.hooks.viewport.0),
                            f64::from(self.hooks.viewport.1),
                        )
                    } else {
                        (0.0, 0.0)
                    }
                };
                let box_size = {
                    let b = self.new_object();
                    self.set_own(b, "inlineSize", Value::Num(w), true);
                    self.set_own(b, "blockSize", Value::Num(h), true);
                    Value::Obj(b)
                };
                for k in [
                    "borderBoxSize",
                    "contentBoxSize",
                    "devicePixelContentBoxSize",
                ] {
                    let arr = self.new_array(vec![box_size]);
                    self.set_own(o, k, arr, true);
                }
            }
        }
        Value::Obj(o)
    }

    /// Rect `{x,y,top,left,bottom,right,width,height}` du nœud
    /// — règle `clientWidth` : `documentElement`/`body` ≙
    /// viewport, tout autre élément vaut 0 (pas de layout
    /// dans le miroir — cf. `getBoundingClientRect` qui rend
    /// des zéros honnêtes).
    fn rect_value(&mut self, dom: &DomRef, node: NodeId) -> Value {
        let is_root = {
            let d = dom.borrow();
            node == d.body() || Some(node) == d.document_element()
        };
        if is_root {
            self.viewport_rect()
        } else {
            self.rect_zero()
        }
    }

    /// Rect nul (format `DOMRectReadOnly` minimal).
    fn rect_zero(&mut self) -> Value {
        let r = self.new_object();
        for k in [
            "x", "y", "top", "left", "bottom", "right", "width", "height",
        ] {
            self.set_own(r, k, Value::Num(0.0), true);
        }
        Value::Obj(r)
    }

    /// Rect du viewport `(0,0,vw,vh)` — `hooks.viewport`.
    fn viewport_rect(&mut self) -> Value {
        let r = self.new_object();
        let (w, h) = self.hooks.viewport;
        for (k, v) in [
            ("x", 0.0),
            ("y", 0.0),
            ("top", 0.0),
            ("left", 0.0),
            ("bottom", f64::from(h)),
            ("right", f64::from(w)),
            ("width", f64::from(w)),
            ("height", f64::from(h)),
        ] {
            self.set_own(r, k, Value::Num(v), true);
        }
        Value::Obj(r)
    }
}

/// `new IntersectionObserver(cb[, options])` — callback
/// requis ; `options.threshold` validé `0..=1` (spec
/// `RangeError`), `root` = `Element`/`Document`/`null`,
/// `rootMargin` au format `<len(px|%)>{1,4}` (spec
/// `SyntaxError` — marge non appliquée, pas de géométrie).
fn intersection_observer_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let cb = args.first().copied().unwrap_or(Value::Undefined);
    if !e.is_callable(cb) {
        return Err(e.type_error("IntersectionObserver: callback is not a function"));
    }
    let opts = args.get(1).copied().unwrap_or(Value::Undefined);
    let thresholds = obs_opt_thresholds(e, opts)?;
    let root = match obs_opt(e, opts, "root") {
        Value::Undefined | Value::Null => None,
        v => match node_for(e, v) {
            Some(n) => Some(n),
            None => return Err(e.type_error("IntersectionObserver: root is not an Element")),
        },
    };
    let margin = obs_opt(e, opts, "rootMargin");
    if !matches!(margin, Value::Undefined) {
        let m = e.to_js_string(margin)?;
        if !obs_valid_margin(&m) {
            return Err(e.type_error("IntersectionObserver: invalid rootMargin"));
        }
    }
    new_obs(e, ObsKind::Intersection, cb, root, &thresholds, margin)
}

/// `new ResizeObserver(cb[, options])` — callback requis ;
/// `options.box` ∈ `{content,border,device-pixel-content}-box`
/// (spec `TypeError`) — non appliqué.
fn resize_observer_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let cb = args.first().copied().unwrap_or(Value::Undefined);
    if !e.is_callable(cb) {
        return Err(e.type_error("ResizeObserver: callback is not a function"));
    }
    let opts = args.get(1).copied().unwrap_or(Value::Undefined);
    match obs_opt(e, opts, "box") {
        Value::Undefined => {}
        v => {
            let b = e.to_js_string(v)?;
            if !matches!(
                b.as_str(),
                "content-box" | "border-box" | "device-pixel-content-box"
            ) {
                return Err(e.type_error("ResizeObserver: invalid box option"));
            }
        }
    }
    new_obs(e, ObsKind::Resize, cb, None, &[], Value::Undefined)
}

/// Propriété d'options — `undefined` hors objet.
fn obs_opt(e: &mut Engine, opts: Value, key: &str) -> Value {
    match opts {
        Value::Obj(o) => e.get_prop(o, key).unwrap_or(Value::Undefined),
        _ => Value::Undefined,
    }
}

/// `options.threshold` → `Vec<f64>` figé — nombre ou array de
/// nombres `0..=1` (spec : `RangeError` hors borne, ordre
/// conservé pour le getter `thresholds`).
fn obs_opt_thresholds(e: &mut Engine, opts: Value) -> JsResult<Vec<f64>> {
    let v = obs_opt(e, opts, "threshold");
    let raw: Vec<f64> = match v {
        Value::Undefined => vec![0.0],
        Value::Num(n) => vec![n],
        Value::Obj(o) => match e.heap.obj(o).map(|ob| &ob.kind) {
            Some(ObjKind::Array(elems)) => {
                let elems = elems.clone();
                elems
                    .iter()
                    .map(|x| e.to_number(*x))
                    .collect::<JsResult<Vec<f64>>>()?
            }
            _ => return Err(e.type_error("IntersectionObserver: threshold is not a number")),
        },
        _ => return Err(e.type_error("IntersectionObserver: threshold is not a number")),
    };
    if raw.iter().any(|t| !(0.0..=1.0).contains(t)) {
        return Err(e.type_error("IntersectionObserver: threshold out of range [0, 1]"));
    }
    Ok(if raw.is_empty() { vec![0.0] } else { raw })
}

/// Format `rootMargin` : 1–4 longueurs `<nombre>(px|%)`
/// séparées par espaces (unité optionnelle = `px` — CSS
/// margin shorthand ; vide = `"0px"` spec).
fn obs_valid_margin(m: &str) -> bool {
    let parts: Vec<&str> = m.split_whitespace().collect();
    if m.is_empty() {
        return true;
    }
    if parts.is_empty() || parts.len() > 4 {
        return false;
    }
    parts.iter().all(|p| {
        let num = p
            .strip_suffix("px")
            .or_else(|| p.strip_suffix('%'))
            .unwrap_or(p);
        !num.is_empty() && num.parse::<f64>().is_ok()
    })
}

/// Registration + wrapper — mêmes conventions que
/// `mutobs_ctor` (id corrélé, cbs racinées GC).
fn new_obs(
    e: &mut Engine,
    kind: ObsKind,
    cb: Value,
    root: Option<NodeId>,
    thresholds: &[f64],
    margin: Value,
) -> JsResult<Value> {
    let Some((dom, cache)) = e.dom_ctx.clone() else {
        return Err(e.type_error("observer: document non installé"));
    };
    e.obs.next_id += 1;
    let id = e.obs.next_id;
    let v = e.new_host_object(Rc::new(ObsHost {
        dom,
        cache,
        id,
        kind,
    }));
    let Value::Obj(oid) = v else {
        return Err(e.type_error("observer: allocation impossible"));
    };
    e.obs.regs.insert(
        id,
        ObsReg {
            kind,
            cb,
            wrapper: oid,
            root,
            targets: Vec::new(),
            pending: Vec::new(),
        },
    );
    // Options exposées (`observer.thresholds`/`rootMargin` —
    // figées à l'instanciation, spec).
    if kind == ObsKind::Intersection {
        let th = e.new_array(thresholds.iter().map(|t| Value::Num(*t)).collect());
        e.set_own(oid, "thresholds", th, true);
        let m = match margin {
            Value::Undefined => e.str_val("0px"),
            other => {
                let s = e.to_js_string(other)?;
                e.str_val(if s.is_empty() { "0px" } else { &s })
            }
        };
        e.set_own(oid, "rootMargin", m, true);
    }
    Ok(v)
}

impl Host for ObsHost {
    fn class_name(&self) -> &'static str {
        match self.kind {
            ObsKind::Intersection => "IntersectionObserver",
            ObsKind::Resize => "ResizeObserver",
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn get(&self, e: &mut Engine, _t: Value, key: &str) -> JsResult<Option<Value>> {
        if key != "root" {
            return Ok(None);
        }
        let Some(reg) = e.obs.regs.get(&self.id) else {
            return Ok(None);
        };
        Ok(Some(match reg.root {
            Some(n) => node_value(e, &self.dom, &self.cache, n),
            None => Value::Null,
        }))
    }

    fn invoke(
        &self,
        e: &mut Engine,
        _t: Value,
        name: &str,
        args: &[Value],
    ) -> JsResult<Option<Value>> {
        match name {
            "observe" => {
                let target = args.first().copied().unwrap_or(Value::Undefined);
                let Some(node) = node_for(e, target) else {
                    return Err(e.type_error("observe: Element requis"));
                };
                let Some(reg) = e.obs.regs.get_mut(&self.id) else {
                    return Ok(Some(Value::Undefined));
                };
                // Spec : re-observe d'une même cible = no-op.
                if !reg.targets.contains(&node) {
                    reg.targets.push(node);
                    reg.pending.push(node);
                }
                Ok(Some(Value::Undefined))
            }
            "unobserve" => {
                let target = args.first().copied().unwrap_or(Value::Undefined);
                let Some(node) = node_for(e, target) else {
                    return Err(e.type_error("unobserve: Element requis"));
                };
                if let Some(reg) = e.obs.regs.get_mut(&self.id) {
                    reg.targets.retain(|&n| n != node);
                    reg.pending.retain(|&n| n != node);
                }
                Ok(Some(Value::Undefined))
            }
            "disconnect" => {
                // Spec : cibles et notifications pendantes purgées.
                if let Some(reg) = e.obs.regs.get_mut(&self.id) {
                    reg.targets.clear();
                    reg.pending.clear();
                }
                Ok(Some(Value::Undefined))
            }
            "takeRecords" => {
                let (kind, root, pending) = match e.obs.regs.get_mut(&self.id) {
                    Some(reg) => (reg.kind, reg.root, std::mem::take(&mut reg.pending)),
                    None => (ObsKind::Intersection, None, Vec::new()),
                };
                let entries: Vec<Value> = pending
                    .iter()
                    .map(|&n| e.observer_entry(kind, &self.dom, &self.cache, root, n))
                    .collect();
                Ok(Some(e.new_array(entries)))
            }
            _ => Ok(None),
        }
    }
}
