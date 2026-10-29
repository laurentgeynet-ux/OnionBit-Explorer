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
//! Géométrie réelle quand la table est publiée : le miroir
//! n'a pas de moteur de mise en page mais la session publie
//! la `GeometryMap` (`element_path → rect`, scroll inclus) —
//! `isIntersecting`/`intersectionRatio`/`boundingClientRect`
//!/`rootBounds` calculent la vraie intersection rect ∩
//! viewport (`fixed` déjà viewport, cf. `element_geom`).
//! Sans table (harnais), repli honnête : `is_attached` pour
//! `isIntersecting`, rect nul. Les cibles sont **ré-évaluées
//! à chaque checkpoint** : un changement d'état (scroll,
//! relayout, insertion) enfile une livraison — la classe
//! « lazy-loader jamais déclenché » (vol github : `scrollY`
//! figé + IO silencieux après l'init) est couverte.
//!
//! `thresholds`/`root`/`rootMargin` validés/exposés ; le
//! seuil gouverne la livraison (`ratio` croise la borne —
//! approximation bornée vs la courbe spec complète).

use std::collections::HashMap;
use std::rc::Rc;

use crate::dom::NodeId;
use crate::dom_host::{element_geom_rect, node_for_in, node_value, DomRef, NodeCache};
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
    /// Protos capturés à l'installation — même défense que
    /// `MutObsState::proto` : un global remplacé par un
    /// wrapper d'agent ne doit pas déchaîner les instances
    /// vers un proto nu (bras « `observe` sur Zm/Object »,
    /// vols 141/149).
    proto_io: Option<Value>,
    proto_ro: Option<Value>,
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
    /// `options.threshold` figé (`IntersectionObserver`) —
    /// borne de ratio gouvernant la livraison.
    thresholds: Vec<f64>,
    /// Dernier état d'intersection **rapporté** par cible —
    /// la ré-évaluation au checkpoint n'enfile qu'un
    /// changement (scroll/relayout) ; les cibles encore en
    /// `pending` n'ont pas d'état rapporté.
    last: HashMap<NodeId, bool>,
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
    /// Racines GC : **wrapper uniquement** des observers
    /// actifs — cibles encore observées ou notifications
    /// en attente (le `new IO(cb).observe(el)` sans référence
    /// survit pendant l'observation, comme Chrome). Le `cb`
    /// n'est plus raciné ici : il vit dans le host et suit le
    /// wrapper — §B-59 : auparavant tout observer créé
    /// restait raciné à vie, `disconnect` inclus.
    pub(crate) fn root_ids(&self, ids: &mut Vec<ObjId>) {
        for reg in self.regs.values() {
            if !reg.targets.is_empty() || !reg.pending.is_empty() {
                ids.push(reg.wrapper);
            }
        }
        for p in [self.proto_io, self.proto_ro].into_iter().flatten() {
            crate::heap::push_value(ids, p);
        }
    }

    /// Élagage post-sweep : entrées dont le wrapper a été
    /// balayé — l'observer est mort, sa registration part
    /// avec (§B-59).
    pub(crate) fn purge_dead(&mut self, heap: &crate::heap::Heap) {
        self.regs.retain(|_, reg| heap.obj(reg.wrapper).is_some());
    }

    /// Cibles encore surveillées — retenues Dom-side pour le
    /// sweep §B-59 (une cible détachée mais observée par un
    /// observer vivant garde son payload).
    pub(crate) fn live_target_ids(&self) -> std::collections::HashSet<crate::dom::NodeId> {
        self.regs
            .values()
            .flat_map(|reg| reg.targets.iter().copied())
            .collect()
    }

    /// §B-59 instrumentation — `regs` est suspecté de croître
    /// sans borne (sonde `leak_probe`).
    #[cfg(test)]
    pub(crate) fn b59_len(&self) -> usize {
        self.regs.len()
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
    /// Callback — vit dans le host pour que sa survie suive
    /// le wrapper via `Host::trace` (§B-59 : `regs` ne racine
    /// plus `cb`/`wrapper` à vie).
    cb: Value,
}

impl Engine {
    /// Installe les deux constructeurs — remplace les natives
    /// passives JS-C56 (le miroir est requis à l'usage, pas à
    /// l'installation ; les ctors lisent `dom_ctx` à l'appel).
    pub(crate) fn install_observers(&mut self) {
        for (name, kind, f) in [
            (
                "IntersectionObserver",
                ObsKind::Intersection,
                intersection_observer_ctor as crate::heap::NativeFn,
            ),
            ("ResizeObserver", ObsKind::Resize, resize_observer_ctor),
        ] {
            let fv = self.native_fn(name, f);
            if let Value::Obj(fid) = fv {
                // `X.prototype` avec ses méthodes — spec WebIDL
                // (même rail que `MutationObserver`, JS-C204) :
                // les agents terrain (ruxit, zone.js) lisent et
                // re-graffent `X.prototype.observe` ; un proto nu
                // rendait la lecture `undefined` et leurs
                // wrappers naissaient sans `observe` — bras
                // « `observe` is not a function (on Object) /
                // (on Zm) » (gog vol 149, leclerc vol 141).
                // `takeRecords` est exposé aussi sur RO
                // (sur-ensemble tolérant, l'impl le fournit).
                let p = self.new_object();
                for m in ["observe", "unobserve", "disconnect", "takeRecords"] {
                    let mv = self.str_val(m);
                    let f = self.bound_native("hostMethod", Self::host_method, vec![mv]);
                    self.set_own(p, m, f, true);
                }
                self.set_own(p, "constructor", fv, false);
                self.set_own(fid, "prototype", Value::Obj(p), false);
                match kind {
                    ObsKind::Intersection => self.obs.proto_io = Some(Value::Obj(p)),
                    ObsKind::Resize => self.obs.proto_ro = Some(Value::Obj(p)),
                }
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
            let (kind, cb, wrapper, root, thresholds, targets, mut pending) = {
                let Some(reg) = self.obs.regs.get_mut(&id) else {
                    continue;
                };
                (
                    reg.kind,
                    reg.cb,
                    reg.wrapper,
                    reg.root,
                    reg.thresholds.clone(),
                    reg.targets.clone(),
                    std::mem::take(&mut reg.pending),
                )
            };
            // Ré-évaluation (Intersection) : l'état courant de
            // chaque cible déjà rapportée est recalculé à ce
            // checkpoint — scroll/relayout entre deux pompes.
            // Un changement d'état rejoint `pending` (spec :
            // « queue an IntersectionObserverEntry »).
            if kind == ObsKind::Intersection {
                for &n in &targets {
                    if pending.contains(&n) {
                        continue;
                    }
                    let now = self.is_intersecting(&dom, root, thresholds.first(), n);
                    let last = self.obs.regs.get(&id).and_then(|r| r.last.get(&n)).copied();
                    if last != Some(now) {
                        pending.push(n);
                    }
                }
            }
            if pending.is_empty() {
                continue;
            }
            let entries: Vec<Value> = pending
                .iter()
                .map(|&n| {
                    if kind == ObsKind::Intersection {
                        let cur = self.is_intersecting(&dom, root, thresholds.first(), n);
                        if let Some(reg) = self.obs.regs.get_mut(&id) {
                            reg.last.insert(n, cur);
                        }
                    }
                    self.observer_entry(kind, &dom, &cache, root, n)
                })
                .collect();
            let arr = self.new_array(entries);
            self.call_value(cb, Value::Obj(wrapper), &[arr, Value::Obj(wrapper)])?;
            delivered += 1;
        }
        Ok(delivered)
    }

    /// État `isIntersecting` courant de `node` — vraie
    /// intersection rect ∩ root quand la `GeometryMap` est
    /// publiée ; seuil le plus bas demandé (toucher le
    /// viewport suffit à `threshold: 0`, la borne spécifique
    /// s'applique au ratio). Repli `is_attached` sans table.
    fn is_intersecting(
        &mut self,
        dom: &DomRef,
        root: Option<NodeId>,
        threshold: Option<&f64>,
        node: NodeId,
    ) -> bool {
        let Some((x, y, w, h)) = element_geom_rect(dom, node) else {
            return dom.borrow().is_attached(node);
        };
        // Borne du root : viewport par défaut, rect réel sinon.
        let (rx, ry, rw, rh) = root.map_or_else(
            || {
                let (vw, vh) = self.hooks.viewport;
                (0.0, 0.0, f64::from(vw), f64::from(vh))
            },
            |rn| element_geom_rect(dom, rn).unwrap_or((0.0, 0.0, 0.0, 0.0)),
        );
        let (ix, iy) = (x.max(rx), y.max(ry));
        let (ir, ib) = ((x + w).min(rx + rw), (y + h).min(ry + rh));
        let (iw, ih) = (ir - ix, ib - iy);
        if iw < 0.0 || ih < 0.0 {
            return false;
        }
        // Touche = intersectant (spec : `intersectionRect` non
        // vide aux bords — `iw/ih == 0` compte true à
        // `threshold: 0`) ; au-delà, le ratio gouverne.
        let area_el = w * h;
        let ratio = if area_el > 0.0 {
            (iw * ih) / area_el
        } else {
            0.0
        };
        match threshold {
            Some(&t) if t > 0.0 => ratio >= t,
            _ => true,
        }
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
                // Rect réel (geometry publiée) — hors table,
                // règle `clientWidth` : racines ≙ viewport,
                // 0 sinon. `isIntersecting` repli `is_attached`.
                let real = element_geom_rect(dom, node);
                let (x, y, w, h) = real.unwrap_or_else(|| {
                    let is_root = {
                        let d = dom.borrow();
                        node == d.body() || Some(node) == d.document_element()
                    };
                    if is_root {
                        let (vw, vh) = self.hooks.viewport;
                        (0.0, 0.0, f64::from(vw), f64::from(vh))
                    } else {
                        (0.0, 0.0, 0.0, 0.0)
                    }
                });
                let (rx, ry, rw, rh) = root.map_or_else(
                    || {
                        let (vw, vh) = self.hooks.viewport;
                        (0.0, 0.0, f64::from(vw), f64::from(vh))
                    },
                    |rn| element_geom_rect(dom, rn).unwrap_or((0.0, 0.0, 0.0, 0.0)),
                );
                let (ix, iy) = (x.max(rx), y.max(ry));
                let (ir, ib) = ((x + w).min(rx + rw), (y + h).min(ry + rh));
                let (iw, ih) = ((ir - ix).max(0.0), (ib - iy).max(0.0));
                let (intersecting, ratio) = if real.is_some() {
                    let touches = ir - ix >= 0.0 && ib - iy >= 0.0;
                    let ratio = if w * h > 0.0 {
                        (iw * ih) / (w * h)
                    } else {
                        0.0
                    };
                    (touches, ratio)
                } else {
                    let attached = dom.borrow().is_attached(node);
                    (attached, if attached { 1.0 } else { 0.0 })
                };
                self.set_own(o, "isIntersecting", Value::Bool(intersecting), true);
                self.set_own(o, "intersectionRatio", Value::Num(ratio), true);
                let time = self.t0.elapsed().as_secs_f64() * 1000.0;
                self.set_own(o, "time", Value::Num(time), true);
                let rect = self.rect_of(x, y, w, h);
                let irect = self.rect_of(ix, iy, iw, ih);
                let rbounds = self.rect_of(rx, ry, rw, rh);
                self.set_own(o, "boundingClientRect", rect, true);
                self.set_own(o, "intersectionRect", irect, true);
                self.set_own(o, "rootBounds", rbounds, true);
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

    /// Rect arbitraire `(x, y, w, h)` au format
    /// `DOMRectReadOnly` minimal — `x`/`left`/`top` etc.
    /// tous dérivés (spec : `bottom`/`right` = bords).
    fn rect_of(&mut self, x: f64, y: f64, w: f64, h: f64) -> Value {
        let r = self.new_object();
        for (k, v) in [
            ("x", x),
            ("y", y),
            ("top", y),
            ("left", x),
            ("bottom", y + h),
            ("right", x + w),
            ("width", w),
            ("height", h),
        ] {
            self.set_own(r, k, Value::Num(v), true);
        }
        Value::Obj(r)
    }

    /// Rect nul (format `DOMRectReadOnly` minimal).
    fn rect_zero(&mut self) -> Value {
        self.rect_of(0.0, 0.0, 0.0, 0.0)
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
        v => {
            let Some((dom, _)) = e.dom_ctx.clone() else {
                return Err(e.type_error("IntersectionObserver: document non installé"));
            };
            match node_for_in(e, v, &dom) {
                Some(n) => Some(n),
                None => return Err(e.type_error("IntersectionObserver: root is not an Element")),
            }
        }
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
        cb,
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
            thresholds: thresholds.to_vec(),
            last: HashMap::new(),
        },
    );
    // Méthodes sur le prototype du ctor (spec WebIDL) — pas
    // en own-props : les greffes `X.prototype.m = wrapper`
    // des agents doivent être vues par les instances (une
    // own-prop les shadowait → instrumentation silencieusement
    // contournée). `typeof io.observe` reste « function »
    // (lecture de prop suit la chaîne de prototypes). Proto
    // capturé à l'installation, pas relu via le global : un
    // `window.X = wrapper` en amont ne déchaîne pas les
    // instances vers le proto nu du wrapper.
    let pp = match kind {
        ObsKind::Intersection => e.obs.proto_io,
        ObsKind::Resize => e.obs.proto_ro,
    };
    if let Some(pp) = pp {
        if let Some(o) = e.heap.obj_mut(oid) {
            o.proto = pp;
        }
    }
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

    fn trace(&self, out: &mut Vec<ObjId>) {
        crate::dom_host::trace_node_cache(&self.cache, out);
        crate::heap::push_value(out, self.cb);
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
                let Some(node) = node_for_in(e, target, &self.dom) else {
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
                let Some(node) = node_for_in(e, target, &self.dom) else {
                    return Err(e.type_error("unobserve: Element requis"));
                };
                if let Some(reg) = e.obs.regs.get_mut(&self.id) {
                    reg.targets.retain(|&n| n != node);
                    reg.pending.retain(|&n| n != node);
                    // L'état rapporté part avec la cible — un
                    // re-`observe` postérieur repart d'une
                    // notification initiale (spec), pas d'un
                    // diff contre l'ancien `last`.
                    reg.last.remove(&node);
                }
                Ok(Some(Value::Undefined))
            }
            "disconnect" => {
                // Spec : cibles et notifications pendantes purgées.
                if let Some(reg) = e.obs.regs.get_mut(&self.id) {
                    reg.targets.clear();
                    reg.pending.clear();
                    reg.last.clear();
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
                    .map(|&n| {
                        // Les entries prises sont rapportées :
                        // la ré-évaluation au checkpoint diff
                        // contre ce `last` — sans lui, l'état
                        // `None → intersectant` re-pendait et
                        // le callback revoyait la cible déjà
                        // vidée (régression du rail invoke).
                        if kind == ObsKind::Intersection {
                            let cur = e.is_intersecting(&self.dom, root, None, n);
                            if let Some(reg) = e.obs.regs.get_mut(&self.id) {
                                reg.last.insert(n, cur);
                            }
                        }
                        e.observer_entry(kind, &self.dom, &self.cache, root, n)
                    })
                    .collect();
                Ok(Some(e.new_array(entries)))
            }
            _ => Ok(None),
        }
    }
}
