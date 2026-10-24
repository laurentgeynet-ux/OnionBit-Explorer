// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Contextes de navigation imbriqués (JS-C78) — « un heap, N
//! globals » (modèle agent de la spec) : chaque `<iframe>` same-
//! heap reçoit son propre `global`/`global_env`/`protos` et ses
//! états document-local (DOM miroir, timers, modules, surface
//! XHR/IDB/streams…), garés dans [`Engine::parked`] pendant
//! qu'un autre contexte est courant. [`Engine::switch_to`]
//! échange les champs en bloc — le code existant lit toujours
//! `self.x` et voit le contexte courant.
//!
//! Same-origin : les fenêtres se donnent leurs vrais objets
//! (`contentDocument` = le miroir enfant, accès synchrone).
//! Cross-origin : [`WindowProxyHost`] opaque — seuls
//! `postMessage`/`closed`/`length`/`parent`/`top`/`frames` passent,
//! et toute valeur traverse en clone `WireValue` (jamais de
//! handle de heap partagé entre origines).

use std::collections::VecDeque;
use std::rc::Rc;

use crate::dom_host::{DomRef, FormSubmit, HistHook, NavHook, NodeCache, OpenHook, PushStateHook};
use crate::heap::{EnvRef, Host, JsResult};
use crate::interp::ReStatic;
use crate::value::{ObjId, Value};
use crate::worker::WireValue;

/// Identité stable d'un contexte de navigation — index dans
/// `Engine::ctxs`. Le contexte top-level est `0`.
pub type CtxId = u32;

/// Message `postMessage` en attente de livraison — encodé en
/// `WireValue` à l'émission (structured clone), décodé à la
/// frontière de job de l'émetteur (même heap → `wire_decode`
/// reconstruit des `ObjId` frais dans le heap partagé).
pub(crate) struct PendingMsg {
    /// Contexte destinataire.
    pub target: CtxId,
    /// Donnée clonée.
    pub data: WireValue,
    /// `MessageEvent.origin` — origine sérialisée de l'émetteur.
    pub origin: String,
    /// Contexte émetteur (`MessageEvent.source`).
    pub source: CtxId,
}

/// Métadonnées de l'arbre des contextes — NON garées (l'arbre
/// appartient à l'engine, pas au contexte courant).
pub(crate) struct CtxMeta {
    /// Contexte parent (`None` pour le top-level).
    pub parent: Option<CtxId>,
    /// Enfants directs, en ordre document (`frames[n]`).
    pub children: Vec<CtxId>,
    /// URL effective du document de ce contexte — sert les
    /// comparaisons d'origine (`targetOrigin`, same-origin).
    pub url: String,
    /// Nœud `<iframe>` porteur dans le DOM miroir du parent —
    /// sert `iframe.contentWindow`/`contentDocument`/`frameElement`.
    /// `(ptr-du-Dom-parent, NodeId)` : le `Rc::as_ptr` du miroir
    /// désambiguïse les `NodeId` de documents différents.
    pub frame_node: Option<(usize, crate::dom::NodeId)>,
    /// `WindowProxy` stable alloué au premier accès cross-origin
    /// — `f.contentWindow === f.contentWindow` (raciné au GC via
    /// `collect`).
    pub proxy: Option<ObjId>,
    /// `ObjId` du global du contexte — renseigné à la création,
    /// survit à `parked.remove` (`context_destroy`) : `closed`/
    /// `parent`/`top` restent résolubles tant qu'une référence
    /// JS retient le `window` (le global est un objet heap
    /// ordinaire, réclamé par le GC normal).
    pub global: Option<ObjId>,
    /// Contexte détruit (navigation du frame hors bfcache —
    /// `closed === true`, `postMessage` no-op silencieux).
    pub alive: bool,
    /// Document en cours de parse (commit session non terminé) —
    /// les `postMessage` vers ce contexte restent en file tant
    /// que le drapeau tient : la task n'est livrée qu'après
    /// l'évaluation des scripts du document, sinon un listener
    /// `<script>` postérieur la raterait (spec — la livraison
    /// suit les jobs de parse du document cible). Armé par
    /// `set_ready_state(…, "loading")`, désarmé à
    /// `interactive`/`complete`. Défaut `false` : hors session
    /// (harnais), la livraison est immédiate comme avant.
    pub defer_msgs: bool,
}

/// Champs document-local d'un contexte garé — miroir exact des
/// champs de `Engine` swappés par `switch_to`. Tout état posé
/// ici dort tant que le contexte n'est pas courant (livraisons
/// en vol gelées — reprises à sa prochaine pompe).
pub(crate) struct BrowsingContext {
    pub global: ObjId,
    pub global_env: EnvRef,
    pub protos: crate::interp::Protos,
    pub gen_ctor: Value,
    pub async_gen_ctor: Value,
    pub gen_fn_proto: Value,
    pub gen_proto: Value,
    pub async_gen_fn_proto: Value,
    pub async_gen_proto: Value,
    pub eval_fn: Value,
    pub re_ctor: Value,
    pub re_static: Option<ReStatic>,
    pub finregs: Vec<ObjId>,
    pub dom_ctx: Option<(DomRef, NodeCache)>,
    pub timers: crate::timers::TimerQueue,
    pub xhr: crate::xhr::XhrEngineState,
    pub mutobs: crate::mutation::MutObsState,
    pub obs: crate::observers::ObsState,
    pub files: crate::files::FilesState,
    pub streams: crate::streams::StreamState,
    pub idb: crate::idb::IdbState,
    pub ws: crate::websocket::WsState,
    /// `EventSource` du document (JS-C92) — flux vivants +
    /// refus différés, garés avec le contexte.
    pub es: crate::eventsource::EsState,
    pub misc: crate::misc_apis::MiscState,
    /// Opérations `crypto.subtle` en vol (JS-C79) — le
    /// transport est engine-global (`Engine::crypto_
    /// transport`), seule la file des promesses est
    /// document-local.
    pub crypto: crate::crypto::CryptoState,
    pub ce_upgrade_slot: Option<Value>,
    pub strict: bool,
    pub completion: Value,
    pub pending_labels: Vec<String>,
    pub modules: Vec<crate::module::Module>,
    pub module_specs: std::collections::HashMap<String, usize>,
    pub module_envs: Vec<EnvRef>,
    pub current_module: String,
    pub import_map: Vec<(String, String)>,
    // `BrowserHooks` document-local (le reste des hooks —
    // `console`/`alert`/`cookie_jar`… — est engine-global).
    pub page_url: String,
    pub screen_obj: Option<ObjId>,
    pub navigate: Option<NavHook>,
    pub history_go: Option<HistHook>,
    pub history_len: u32,
    pub form_submit: Option<Rc<dyn Fn(FormSubmit)>>,
    pub open: Option<OpenHook>,
    /// `history.pushState`/`replaceState` — hook embedder
    /// (CAP-151/158) : `(url_brute, replace, state_json)`.
    pub push_state: Option<PushStateHook>,
    /// `history.state` JSON du contexte (CAP-158) —
    /// document-local, garé/restauré au `switch_to`.
    pub history_state: Option<String>,
    /// `window.name` du contexte (CAP-159) —
    /// document-local, garé/restauré au `switch_to`.
    pub win_name: String,
    /// `history.scrollRestoration == 'manual'` du contexte
    /// (CAP-166) — document-local, garé/restauré au
    /// `switch_to`.
    pub scroll_restore_manual: bool,
    /// `document.readyState` du contexte (CAP-170) —
    /// document-local, garé/restauré au `switch_to`.
    pub ready_state: &'static str,
    /// Slots `<dialog>` du contexte (CAP-176) — `showModal()`
    /// pose `open` + le flag modal ; `close()` efface les
    /// deux + `close` event. `returnValue` = slot spec.
    /// Document-local, garé/restauré au `switch_to`.
    pub dialog_modal: std::collections::BTreeSet<crate::dom::NodeId>,
    pub dialog_retval: std::collections::BTreeMap<crate::dom::NodeId, String>,
    /// Fullscreen/popover (CAP-179) — voir `BrowserHooks`.
    pub fullscreen_el: Option<crate::dom::NodeId>,
    pub popover_open: std::collections::BTreeSet<crate::dom::NodeId>,
    pub viewport: (u32, u32),
}

/// File de messages `postMessage` — drainée en fin de pompe de
/// microtasks (frontière de job de l'émetteur, spec §9.4).
pub(crate) type PendingMsgs = VecDeque<PendingMsg>;

impl BrowsingContext {
    /// Racines GC d'un contexte garé — réplique du bloc « par
    /// contexte » de [`crate::interp::Engine::collect`] : les
    /// mêmes champs sur `self` y sont tracés quand le contexte
    /// est courant. Les deux côtés doivent rester synchrones —
    /// tout état swappé retenant des `Value`/`ObjId` doit être
    /// tracé ici ET là-bas.
    pub(crate) fn push_roots(
        &mut self,
        ids: &mut Vec<ObjId>,
        visited: &mut std::collections::HashSet<usize>,
    ) {
        let mut vs = Vec::new();
        self.timers.roots(&mut vs);
        for v in vs {
            crate::heap::push_value(ids, v);
        }
        for (_, wrapper, data) in &self.xhr.live {
            ids.push(*wrapper);
            let mut cbs = Vec::new();
            data.borrow().listener_values(&mut cbs);
            for cb in cbs {
                crate::heap::push_value(ids, cb);
            }
        }
        for (_, promise, _, _) in &self.xhr.fetches {
            ids.push(*promise);
        }
        for p in [
            self.xhr.headers_proto,
            self.xhr.req_proto,
            self.xhr.res_proto,
            self.xhr.fd_proto,
        ]
        .into_iter()
        .flatten()
        {
            ids.push(p);
        }
        self.mutobs.root_ids(ids);
        self.obs.root_ids(ids);
        self.files.root_ids(ids);
        self.streams.root_ids(ids);
        self.idb.root_ids(ids);
        self.ws.root_ids(ids);
        self.es.root_ids(ids);
        self.misc.root_ids(ids);
        for (_, promise, _) in &self.crypto.pending {
            ids.push(*promise);
        }
        if let Some((dom, cache)) = &self.dom_ctx {
            for &id in cache.borrow().values() {
                ids.push(id);
            }
            for node in &dom.borrow().nodes {
                for (_, cb, _) in &node.listeners {
                    crate::heap::push_value(ids, *cb);
                }
            }
            for v in dom.borrow().canvas_ctx_values() {
                crate::heap::push_value(ids, v);
            }
            for def in dom.borrow().ce.defs.values() {
                crate::heap::push_value(ids, def.ctor);
            }
            for list in dom.borrow().ce.pending.values() {
                ids.extend(list.iter().copied());
            }
        }
        if let Some(v) = self.ce_upgrade_slot {
            crate::heap::push_value(ids, v);
        }
        if let Some(s) = self.screen_obj {
            ids.push(s);
        }
        ids.push(self.global);
        ids.extend(self.protos.all());
        crate::heap::push_value(ids, self.gen_ctor);
        crate::heap::push_value(ids, self.async_gen_ctor);
        crate::heap::push_value(ids, self.gen_fn_proto);
        crate::heap::push_value(ids, self.gen_proto);
        crate::heap::push_value(ids, self.async_gen_fn_proto);
        crate::heap::push_value(ids, self.async_gen_proto);
        crate::heap::push_value(ids, self.eval_fn);
        crate::heap::push_value(ids, self.re_ctor);
        crate::heap::push_value(ids, self.completion);
        crate::heap::trace_env(&self.global_env, ids, visited);
        for m in &self.modules {
            crate::heap::trace_env(&m.env, ids, visited);
        }
    }

    /// Contexte vierge — **intrinsics frais par realm** (spec :
    /// `frame.Array !== parent.Array`). Le bloc d'allocation
    /// reflète `Engine::new` — le tenir synchronisé avec lui.
    pub(crate) fn blank(heap: &mut crate::heap::Heap) -> Self {
        let object = heap.alloc_obj(crate::heap::Object::ordinary(Value::Null));
        let function = heap.alloc_obj(crate::heap::Object::ordinary(Value::Obj(object)));
        let array = heap.alloc_obj(crate::heap::Object::array(Vec::new(), Value::Obj(object)));
        let ordinary = |h: &mut crate::heap::Heap| {
            h.alloc_obj(crate::heap::Object::ordinary(Value::Obj(object)))
        };
        let string = ordinary(heap);
        let number = ordinary(heap);
        let boolean = ordinary(heap);
        let error = ordinary(heap);
        let err_of = |h: &mut crate::heap::Heap| {
            h.alloc_obj(crate::heap::Object::ordinary(Value::Obj(error)))
        };
        let type_error = err_of(heap);
        let reference_error = err_of(heap);
        let syntax_error = err_of(heap);
        let eval_error = err_of(heap);
        let range_error = err_of(heap);
        let uri_error = err_of(heap);
        let date = ordinary(heap);
        let regexp = ordinary(heap);
        let promise = ordinary(heap);
        let iterator = ordinary(heap);
        let iter_of = |h: &mut crate::heap::Heap| {
            h.alloc_obj(crate::heap::Object::ordinary(Value::Obj(iterator)))
        };
        let array_iter = iter_of(heap);
        let map_iter = iter_of(heap);
        let set_iter = iter_of(heap);
        let string_iter = iter_of(heap);
        let regexp_iter = iter_of(heap);
        let usp_iter = iter_of(heap);
        let iter_helper = iter_of(heap);
        let map = ordinary(heap);
        let set = ordinary(heap);
        let symbol = ordinary(heap);
        let proxy = ordinary(heap);
        let array_buffer = ordinary(heap);
        let data_view = ordinary(heap);
        let typed_array = ordinary(heap);
        let bigint = ordinary(heap);
        let aggregate_error = err_of(heap);
        let url = ordinary(heap);
        let url_params = ordinary(heap);
        let weak_map = ordinary(heap);
        let weak_set = ordinary(heap);
        let global = heap.alloc_obj(crate::heap::Object::ordinary(Value::Obj(object)));
        Self {
            global,
            global_env: crate::heap::Env::global(),
            protos: crate::interp::Protos {
                object,
                function,
                array,
                string,
                number,
                boolean,
                error,
                type_error,
                reference_error,
                syntax_error,
                eval_error,
                range_error,
                uri_error,
                date,
                regexp,
                promise,
                iterator,
                array_iter,
                map_iter,
                set_iter,
                string_iter,
                regexp_iter,
                usp_iter,
                iter_helper,
                map,
                set,
                symbol,
                proxy,
                array_buffer,
                data_view,
                typed_array,
                bigint,
                aggregate_error,
                url,
                url_params,
                weak_map,
                weak_set,
            },
            gen_ctor: Value::Undefined,
            async_gen_ctor: Value::Undefined,
            gen_fn_proto: Value::Undefined,
            gen_proto: Value::Undefined,
            async_gen_fn_proto: Value::Undefined,
            async_gen_proto: Value::Undefined,
            eval_fn: Value::Undefined,
            re_ctor: Value::Undefined,
            re_static: None,
            finregs: Vec::new(),
            dom_ctx: None,
            timers: crate::timers::TimerQueue::default(),
            xhr: crate::xhr::XhrEngineState::default(),
            mutobs: crate::mutation::MutObsState::default(),
            obs: crate::observers::ObsState::default(),
            files: crate::files::FilesState::default(),
            streams: crate::streams::StreamState::default(),
            idb: crate::idb::IdbState::default(),
            ws: crate::websocket::WsState::default(),
            es: crate::eventsource::EsState::default(),
            misc: crate::misc_apis::MiscState::default(),
            crypto: crate::crypto::CryptoState::default(),
            ce_upgrade_slot: None,
            strict: false,
            completion: Value::Undefined,
            pending_labels: Vec::new(),
            modules: Vec::new(),
            module_specs: std::collections::HashMap::new(),
            module_envs: Vec::new(),
            current_module: String::new(),
            import_map: Vec::new(),
            page_url: String::new(),
            screen_obj: None,
            navigate: None,
            history_go: None,
            history_len: 0,
            form_submit: None,
            open: None,
            push_state: None,
            history_state: None,
            win_name: String::new(),
            scroll_restore_manual: false,
            ready_state: "loading",
            dialog_modal: std::collections::BTreeSet::new(),
            dialog_retval: std::collections::BTreeMap::new(),
            fullscreen_el: None,
            popover_open: std::collections::BTreeSet::new(),
            viewport: (0, 0),
        }
    }
}

/// Host object `WindowProxy` — surface cross-origin opaque.
/// `ctx` = le contexte cible : les accès résolvent contre lui,
/// jamais contre le contexte courant (`frame.contentWindow.x`
/// lit le contexte de `frame`, pas celui du script appelant).
pub(crate) struct WindowProxyHost {
    /// Contexte ciblé.
    pub ctx: CtxId,
}

impl Host for WindowProxyHost {
    fn get(&self, e: &mut crate::Engine, _this: Value, key: &str) -> JsResult<Option<Value>> {
        match key {
            "window" | "self" | "frames" => Ok(Some(e.ctx_window_value(self.ctx))),
            "top" => Ok(Some(e.ctx_window_value(e.ctx_top_of(self.ctx)))),
            "parent" => {
                let p = e.ctxs[self.ctx as usize].parent.unwrap_or(self.ctx);
                Ok(Some(e.ctx_window_value(p)))
            }
            "closed" => Ok(Some(Value::Bool(!e.ctxs[self.ctx as usize].alive))),
            "length" => Ok(Some(Value::Num(
                e.ctxs[self.ctx as usize].children.len() as f64
            ))),
            "opener" => Ok(Some(Value::Null)),
            "postMessage" => Ok(Some(e.native_fn("postMessage", window_post_message))),
            "focus" | "blur" | "close" => Ok(Some(e.native_fn("noop", window_noop))),
            // `location` : write-only cross-origin — get rend un
            // objet opaque (la lecture d'URL est refusée par spec).
            "location" => Ok(Some(Value::Obj(e.new_object()))),
            _ => {
                // `frames[n]` — index numérique d'enfant.
                if let Ok(i) = key.parse::<usize>() {
                    if let Some(&c) = e.ctxs[self.ctx as usize].children.get(i) {
                        return Ok(Some(e.ctx_window_value(c)));
                    }
                }
                Ok(None)
            }
        }
    }

    fn set(&self, _e: &mut crate::Engine, _this: Value, _key: &str, _v: Value) -> JsResult<bool> {
        // Toute écriture cross-origin est ignorée (spec :
        // `location` seule navigue — v1 : pas de navigation
        // proxy, documenté dans la roadmap C78).
        Ok(true)
    }

    fn invoke(
        &self,
        e: &mut crate::Engine,
        this: Value,
        name: &str,
        args: &[Value],
    ) -> JsResult<Option<Value>> {
        if name == "postMessage" {
            return Ok(Some(e.post_message(self.ctx, args)?));
        }
        let _ = this;
        Ok(Some(Value::Undefined))
    }

    fn keys(&self, _e: &mut crate::Engine, _this: Value) -> Vec<String> {
        vec![
            "window".into(),
            "self".into(),
            "frames".into(),
            "top".into(),
            "parent".into(),
            "closed".into(),
            "length".into(),
            "opener".into(),
            "postMessage".into(),
            "location".into(),
        ]
    }

    fn class_name(&self) -> &'static str {
        "Window"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// No-op cross-origin (`focus`/`blur`/`close` sur un proxy).
pub(crate) fn window_noop(
    _e: &mut crate::Engine,
    _this: Value,
    _args: &[Value],
) -> JsResult<Value> {
    Ok(Value::Undefined)
}

/// `this` → le contexte propriétaire du global — les accessors
/// `parent`/`top`/`length` posés sur chaque global résolvent
/// TOUJOURS leur contexte depuis `this` (`f.contentWindow.parent`
/// interroge le contexte de `f`, pas `cur_ctx`).
fn ctx_of_this(e: &mut crate::Engine, this: Value) -> Option<CtxId> {
    match this {
        Value::Obj(id) => e.ctx_of_objid(id),
        _ => None,
    }
}

/// `window.parent` getter — parent du contexte de `this`, ou
/// `this` lui-même au top-level (spec).
pub(crate) fn window_parent_get(
    e: &mut crate::Engine,
    this: Value,
    _args: &[Value],
) -> JsResult<Value> {
    let Some(ctx) = ctx_of_this(e, this) else {
        return Err(e.type_error("Illegal invocation"));
    };
    let p = e.ctxs[ctx as usize].parent.unwrap_or(ctx);
    Ok(e.ctx_window_value_as(ctx, p))
}

/// `window.top` getter — racine de la chaîne.
pub(crate) fn window_top_get(
    e: &mut crate::Engine,
    this: Value,
    _args: &[Value],
) -> JsResult<Value> {
    let Some(ctx) = ctx_of_this(e, this) else {
        return Err(e.type_error("Illegal invocation"));
    };
    let t = e.ctx_top_of(ctx);
    Ok(e.ctx_window_value_as(ctx, t))
}

/// `window.frames` getter — la fenêtre elle-même (spec :
/// `frames === window`), les enfants étant les props
/// numériques `0..length` posées à l'attache.
pub(crate) fn window_frames_get(
    e: &mut crate::Engine,
    this: Value,
    _args: &[Value],
) -> JsResult<Value> {
    let Some(ctx) = ctx_of_this(e, this) else {
        return Err(e.type_error("Illegal invocation"));
    };
    Ok(e.ctx_window_value_as(ctx, ctx))
}

/// `window.length` getter — nombre d'enfants directs (dynamique).
pub(crate) fn window_length_get(
    e: &mut crate::Engine,
    this: Value,
    _args: &[Value],
) -> JsResult<Value> {
    let Some(ctx) = ctx_of_this(e, this) else {
        return Err(e.type_error("Illegal invocation"));
    };
    Ok(Value::Num(e.ctxs[ctx as usize].children.len() as f64))
}

/// `window[n]` getter — enfant `n` du contexte de `this`,
/// dynamique (`destroy`/réordonnancement pris en compte —
/// jamais de prop figée). Bound `[index]`.
pub(crate) fn window_index_get(
    e: &mut crate::Engine,
    this: Value,
    args: &[Value],
) -> JsResult<Value> {
    let Some(ctx) = ctx_of_this(e, this) else {
        return Err(e.type_error("Illegal invocation"));
    };
    let i = match args.first() {
        Some(Value::Num(n)) => *n as usize,
        _ => 0,
    };
    match e.ctxs[ctx as usize].children.get(i) {
        Some(&c) => Ok(e.ctx_window_value_as(ctx, c)),
        None => Ok(Value::Undefined),
    }
}

/// `window.closed` getter — contexte détruit.
pub(crate) fn window_closed_get(
    e: &mut crate::Engine,
    this: Value,
    _args: &[Value],
) -> JsResult<Value> {
    let Some(ctx) = ctx_of_this(e, this) else {
        return Err(e.type_error("Illegal invocation"));
    };
    Ok(Value::Bool(!e.ctxs[ctx as usize].alive))
}

/// Pose les props contexte sur le global du contexte **courant**
/// — appelé à la fin de `install_dom` (le global est déjà garni
/// `window`/`self`/`globalThis` ; `top`/`parent`/`frames`/`length`/
/// `closed`/`postMessage` remplacent les stubs self-refs).
pub(crate) fn install_ctx_window_props(e: &mut crate::Engine) {
    let g = e.global;
    let set_acc = |e: &mut crate::Engine, key: &'static str, get: crate::heap::NativeFn| {
        let gv = e.native_fn(key, get);
        if let Some(o) = e.heap.obj_mut(g) {
            o.set_accessor(key, gv, Value::Undefined, true);
        }
    };
    set_acc(e, "parent", window_parent_get);
    set_acc(e, "top", window_top_get);
    set_acc(e, "frames", window_frames_get);
    set_acc(e, "length", window_length_get);
    set_acc(e, "closed", window_closed_get);
    let pm = e.native_fn("postMessage", window_post_message);
    e.set_own(g, "postMessage", pm, true);
    // `scrollTo`/`scrollBy`/`scroll`/`focus`/`blur`/`close`
    // sur le global top — no-op bornés honnêtes (même motif
    // documenté que `scrollIntoView`, JS-C74 : les scripts
    // les appellent librement, jamais d'exception ; la
    // géométrie/scroll réel reste l'arc #19/JS-8).
    for name in ["scrollTo", "scrollBy", "scroll", "focus", "blur", "close"] {
        let f = e.native_fn(name, window_noop);
        e.set_own(g, name, f, true);
    }
    // CAP-178 — méthodes fenêtre legacy mesurées absentes
    // (sonde `_census_calls`) : `moveTo`/`resizeTo`/
    // `openDialog`/`showModalDialog` = no-op honnêtes (les
    // navigateurs modernes les no-op aussi sur la fenêtre
    // principale — Chrome : `moveTo`/`resizeTo` silencieux
    // hors popup, `showModalDialog` supprimé → `undefined`
    // de retour) ; `find` rend `false` (texte non trouvé —
    // la recherche document réelle est un arc séparé).
    for name in [
        "moveTo",
        "moveBy",
        "resizeTo",
        "resizeBy",
        "openDialog",
        "showModalDialog",
    ] {
        let f = e.native_fn(name, window_noop);
        e.set_own(g, name, f, true);
    }
    let find_f = e.native_fn("find", window_find);
    e.set_own(g, "find", find_f, true);
}

/// `window.find(needle)` — spec : `false` quand introuvable ;
/// la recherche réelle dans le document est un arc séparé
/// (borné : toujours `false` — honnête, jamais de faux
/// positif).
pub(crate) fn window_find(
    _e: &mut crate::Engine,
    _this: Value,
    _args: &[Value],
) -> JsResult<Value> {
    Ok(Value::Bool(false))
}

/// `window.postMessage` — `this` peut être le global d'un
/// contexte same-origin OU un `WindowProxy` cross-origin ;
/// la cible se résout depuis `this`, jamais `cur_ctx`.
pub(crate) fn window_post_message(
    e: &mut crate::Engine,
    this: Value,
    args: &[Value],
) -> JsResult<Value> {
    let Value::Obj(id) = this else {
        return Err(e.type_error("Illegal invocation"));
    };
    // `WindowProxy` d'abord (sa cible est dans le host), sinon
    // le global d'un contexte vivant.
    let target = match e.heap.obj(id).map(|o| &o.kind) {
        Some(crate::heap::ObjKind::Host(h)) => h
            .as_any()
            .downcast_ref::<WindowProxyHost>()
            .map(|w| w.ctx)
            .or_else(|| e.ctx_of_objid(id)),
        _ => e.ctx_of_objid(id),
    };
    let Some(target) = target else {
        return Err(e.type_error("postMessage: cible hors contexte"));
    };
    e.post_message(target, args)
}
