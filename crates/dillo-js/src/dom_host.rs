// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! DOM façade — `Host` bindings exposing the mini-`Dom` to scripts:
//! `document`, `Element`, `style`, `classList`, `navigator`.
//!
//! Property coverage targets old-school scripts: `getElementById`,
//! `innerHTML`, `className`, `style`, `value`, `addEventListener`
//! (dispatch réelle = JS-3), `querySelector` à sélecteur simple.

use crate::dom::{Dom, NodeId, NodeKind};
use crate::heap::{Host, JsResult, ObjKind};
use crate::value::{ObjId, Value};
use crate::Engine;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

/// Shared DOM handle type.
pub type DomRef = Rc<RefCell<Dom>>;

/// Embedder-provided cookie jar for `document.cookie` (JS-7e).
/// `get` returns the `"k=v; k2=v2"` string for `url`; `set` stores
/// one `"k=v; attrs…"` write. The jar applies its own policy
/// (scope, expiry, httpOnly…).
pub trait CookieJar {
    /// Cookie header-value fragment for `url` (`document.cookie` read).
    fn get(&self, url: &str) -> String;
    /// Store one `document.cookie = "…"` assignment for `url`.
    fn set(&self, url: &str, cookie: &str);
}

/// `location.assign/href=` callback (`&str` = URL brute, à résoudre
/// côté embedder contre la base de la page).
pub type NavHook = Rc<dyn Fn(&str)>;

/// `history.go(n)` callback.
pub type HistHook = Rc<dyn Fn(i32)>;

/// `prompt(msg, default)` → user text (`None` = cancel → `null`).
pub type PromptHook = Rc<dyn Fn(String, String) -> Option<String>>;

/// Soumission de formulaire émise par `form.submit()` /
/// `requestSubmit()` (étape 54) — l'embedder résout `action`
/// contre l'URL de page, choisit le transport (`method`,
/// `enctype`) et encode `fields`.
#[derive(Debug, Clone)]
pub struct FormSubmit {
    /// `method` du `<form>` (`"get"` par défaut).
    pub method: String,
    /// `action` brute (à résoudre côté hôte).
    pub action: String,
    /// `enctype` brut (`""` ≙ urlencoded).
    pub enctype: String,
    /// `(name, value)` des contrôles « successful » en ordre
    /// d'arbre — collectés côté miroir (mêmes règles que le
    /// moteur : nommés, non `disabled`, cases cochées,
    /// submitter seul pour les boutons).
    pub fields: Vec<(String, String)>,
}

/// Browser-surface hooks (JS-7e) — everything `window.location`,
/// `history` and `document.cookie` need, injected by the embedder so
/// the engine never navigates or touches the cookie store itself.
#[derive(Default)]
pub(crate) struct BrowserHooks {
    /// Absolute URL of the loaded page (`location.href` source).
    pub page_url: String,
    /// Cookie jar for `document.cookie`.
    pub cookie_jar: Option<Rc<dyn CookieJar>>,
    /// `location.assign/replace/href=`/`reload` → navigate to arg.
    pub navigate: Option<NavHook>,
    /// `history.go(n)`/`back()`/`forward()` → relative move.
    pub history_go: Option<HistHook>,
    /// `history.length` — session-history count at install time.
    pub history_len: u32,
    /// `console.*` — one formatted line per call (`"log: a b"`).
    /// Absent → the call is a silent no-op.
    pub console: Option<Rc<dyn Fn(String)>>,
    /// `alert(msg)` — modal-less notification (fire-and-forget).
    pub alert: Option<Rc<dyn Fn(String)>>,
    /// `confirm(msg)` → `true`/`false`; absent → `false`.
    pub confirm: Option<Rc<dyn Fn(String) -> bool>>,
    /// `prompt(msg, default)` → user text; absent/cancel → `null`.
    pub prompt: Option<PromptHook>,
    /// `window.open(url)` — new context. Absent → same-tab
    /// `navigate` fallback.
    pub open: Option<Rc<dyn Fn(String)>>,
    /// `form.submit()`/`requestSubmit()` — absent → no-op
    /// silencieux (le moteur ne navigue jamais seul).
    pub form_submit: Option<Rc<dyn Fn(FormSubmit)>>,
    /// Viewport size in CSS px — backs `innerWidth`/`innerHeight`
    /// and `screen.width`/`height`.
    pub viewport: (u32, u32),
    /// `screen` object id — kept so [`Engine::set_viewport`] can
    /// refresh its props when the real window size arrives late
    /// (embedders learn it after `install_dom`).
    pub(crate) screen_obj: Option<ObjId>,
}

impl Engine {
    /// Point `location`/`document.cookie` at `url`.
    pub fn set_page_url(&mut self, url: &str) {
        self.hooks.page_url = url.to_string();
    }

    /// Install the cookie jar backing `document.cookie`.
    pub fn set_cookie_jar(&mut self, jar: Rc<dyn CookieJar>) {
        self.hooks.cookie_jar = Some(jar);
    }

    /// Install navigation hooks: `navigate` for `location.*`,
    /// `history_go` + `history_len` for `history.*`.
    pub fn set_nav_hooks(&mut self, navigate: NavHook, history_go: HistHook, history_len: u32) {
        self.hooks.navigate = Some(navigate);
        self.hooks.history_go = Some(history_go);
        self.hooks.history_len = history_len;
    }

    /// Install the `console.*` line sink (formatted `"a b"` lines).
    pub fn set_console_hook(&mut self, f: Rc<dyn Fn(String)>) {
        self.hooks.console = Some(f);
    }

    /// Install the dialog hooks: `alert`, `confirm`, `prompt`.
    pub fn set_dialog_hooks(
        &mut self,
        alert: Rc<dyn Fn(String)>,
        confirm: Rc<dyn Fn(String) -> bool>,
        prompt: Rc<dyn Fn(String, String) -> Option<String>>,
    ) {
        self.hooks.alert = Some(alert);
        self.hooks.confirm = Some(confirm);
        self.hooks.prompt = Some(prompt);
    }

    /// Install the `window.open(url)` hook (new context).
    pub fn set_open_hook(&mut self, f: Rc<dyn Fn(String)>) {
        self.hooks.open = Some(f);
    }

    /// Install the `form.submit()`/`requestSubmit()` sink —
    /// the embedder turns [`FormSubmit`] into a navigation.
    pub fn set_form_submit_hook(&mut self, f: Rc<dyn Fn(FormSubmit)>) {
        self.hooks.form_submit = Some(f);
    }

    /// Set the viewport size backing `inner*`/`screen.*` (CSS px).
    /// Safe to call *after* [`Engine::install_dom`] — the `screen`
    /// object and the global `inner*`/`outer*` props are refreshed
    /// in place.
    pub fn set_viewport(&mut self, w: u32, h: u32) {
        self.hooks.viewport = (w, h);
        if let Some(sid) = self.hooks.screen_obj {
            for (k, v) in [
                ("width", w),
                ("height", h),
                ("availWidth", w),
                ("availHeight", h),
            ] {
                self.set_own(sid, k, Value::Num(f64::from(v)), true);
            }
            let g = self.global;
            self.set_own(g, "innerWidth", Value::Num(f64::from(w)), false);
            self.set_own(g, "innerHeight", Value::Num(f64::from(h)), false);
            self.set_own(g, "outerWidth", Value::Num(f64::from(w)), false);
            self.set_own(g, "outerHeight", Value::Num(f64::from(h)), false);
        }
    }
}

/// Node → JS wrapper cache: `getElementById('x') === getElementById('x')`
/// must hold, so a node maps to a single JS object for the engine's
/// lifetime.
pub(crate) type NodeCache = Rc<RefCell<HashMap<NodeId, ObjId>>>;

/// `document` object.
struct DocumentHost {
    dom: DomRef,
    cache: NodeCache,
}

/// Element/text node object (`this` carries the node id).
struct NodeHost {
    dom: DomRef,
    cache: NodeCache,
    node: NodeId,
}

impl NodeHost {
    /// `checkValidity` partagé : `invalid` dispatché sur chaque
    /// contrôle fautif (le `<form>` couvre tous ses candidats)
    /// — `false` dès qu'un contrôle échoue.
    fn check_validity_of(&self, e: &mut Engine, node: NodeId) -> JsResult<bool> {
        let controls = {
            let d = self.dom.borrow();
            if d.nodes[node as usize].tag == "form" {
                let mut v = d.by_tag(node, "input");
                v.extend(d.by_tag(node, "textarea"));
                v.extend(d.by_tag(node, "select"));
                v.retain(|&c| will_validate(&d, c));
                v
            } else {
                vec![node]
            }
        };
        let mut ok = true;
        for c in controls {
            let invalid = {
                let d = self.dom.borrow();
                will_validate(&d, c) && !constraint_failures(&d, c).is_empty()
            };
            if invalid {
                e.dispatch_simple(&self.dom, &self.cache, c, "invalid")?;
                ok = false;
            }
        }
        Ok(ok)
    }

    /// `FormSubmit` du `<form>` `node` (méthode `submit` /
    /// `requestSubmit`) — `None` si `node` n'est pas un
    /// `<form>` ou le hook absent (no-op silencieux).
    fn submit_payload(&self, node: NodeId, submitter: Option<NodeId>) -> Option<FormSubmit> {
        let d = self.dom.borrow();
        if d.nodes[node as usize].tag != "form" {
            return None;
        }
        Some(FormSubmit {
            method: d.attr(node, "method").unwrap_or("get").to_string(),
            action: d.attr(node, "action").unwrap_or("").to_string(),
            enctype: d.attr(node, "enctype").unwrap_or("").to_string(),
            fields: collect_fields(&d, node, submitter),
        })
    }

    /// `prepend`/`before`/`after`/`replaceWith` args → node ids —
    /// host nodes pass through, strings become text nodes (spec).
    fn norm_children(&self, e: &mut Engine, args: &[Value]) -> JsResult<Vec<NodeId>> {
        let mut out = Vec::with_capacity(args.len());
        for &a in args {
            if let Some(n) = node_for(e, a) {
                out.push(n);
            } else {
                let s = e.to_js_string(a)?;
                out.push(self.dom.borrow_mut().create_text(&s));
            }
        }
        Ok(out)
    }

    /// `insertAdjacentHTML(pos, html)` — parses `html` into a detached
    /// container then splices its children at `pos` relative to `node`.
    /// Positions: `beforebegin`/`afterbegin`/`beforeend`/`afterend`.
    fn insert_adjacent(&self, e: &mut Engine, node: NodeId, pos: &str, html: &str) -> JsResult<()> {
        // Detached scratch parent — its parsed children are then
        // re-parented at the right slot (keeps order).
        let scratch = self.dom.borrow_mut().create_element("div");
        self.dom.borrow_mut().parse_fragment_into(scratch, html);
        let kids = self.dom.borrow().child_nodes(scratch);
        match pos {
            "beforebegin" => {
                if let Some(p) = dom_parent(&self.dom, node) {
                    for k in kids {
                        self.dom.borrow_mut().insert_before(p, k, Some(node));
                    }
                }
            }
            "afterbegin" => {
                for k in kids {
                    let first = self.dom.borrow().child_nodes(node).first().copied();
                    self.dom.borrow_mut().insert_before(node, k, first);
                }
            }
            "beforeend" => {
                for k in kids {
                    self.dom.borrow_mut().append(node, k);
                }
            }
            "afterend" => {
                if let Some(p) = dom_parent(&self.dom, node) {
                    let mut anchor = node;
                    for k in kids {
                        let next = self.dom.borrow().sibling(anchor, true);
                        self.dom.borrow_mut().insert_before(p, k, next);
                        anchor = k;
                    }
                }
            }
            _ => return Err(e.syntax_error("insertAdjacentHTML: bad position")),
        }
        Ok(())
    }

    /// `insertAdjacentText(pos, s)` — one text node at `pos`
    /// relative to `node` (same four slots as `insertAdjacentHTML`,
    /// payload never parsed).
    fn insert_adjacent_text(
        &self,
        e: &mut Engine,
        node: NodeId,
        pos: &str,
        s: &str,
    ) -> JsResult<()> {
        let t = self.dom.borrow_mut().create_text(s);
        match pos {
            "beforebegin" => {
                if let Some(p) = dom_parent(&self.dom, node) {
                    self.dom.borrow_mut().insert_before(p, t, Some(node));
                }
            }
            "afterbegin" => {
                let first = self.dom.borrow().child_nodes(node).first().copied();
                self.dom.borrow_mut().insert_before(node, t, first);
            }
            "beforeend" => self.dom.borrow_mut().append(node, t),
            "afterend" => {
                if let Some(p) = dom_parent(&self.dom, node) {
                    let next = self.dom.borrow().sibling(node, true);
                    self.dom.borrow_mut().insert_before(p, t, next);
                }
            }
            _ => return Err(e.syntax_error("insertAdjacentText: bad position")),
        }
        Ok(())
    }
}

/// `el.style` — camelCase ↔ kebab-case map over the `style` attr.
struct StyleHost {
    dom: DomRef,
    node: NodeId,
}

/// `el.classList` — token ops over the `class` attr.
struct ClassListHost {
    dom: DomRef,
    node: NodeId,
}

/// Mutable DOM `Event` state — shared between the `Event` host
/// wrapper and the dispatcher so `stopPropagation`/`preventDefault`
/// take effect mid-walk.
pub(crate) struct EventState {
    ty: RefCell<String>,
    target: Cell<NodeId>,
    current: Cell<Option<NodeId>>,
    bubbles: Cell<bool>,
    stopped: Cell<bool>,
    prevented: Cell<bool>,
}

/// DOM `Event` host object (from `document.createEvent` +
/// `initEvent`, `dispatchEvent`, or a plain `{type:…}` literal).
struct EventHost {
    dom: DomRef,
    cache: NodeCache,
    state: Rc<EventState>,
}

pub(crate) fn node_value(e: &mut Engine, dom: &DomRef, cache: &NodeCache, node: NodeId) -> Value {
    if let Some(&id) = cache.borrow().get(&node) {
        return Value::Obj(id);
    }
    let v = e.new_host_object(Rc::new(NodeHost {
        dom: dom.clone(),
        cache: cache.clone(),
        node,
    }));
    if let Value::Obj(id) = v {
        cache.borrow_mut().insert(node, id);
    }
    v
}

/// `dom.borrow().parent(n)` without a scrutinee-held `Ref`.
fn dom_parent(dom: &DomRef, n: NodeId) -> Option<NodeId> {
    dom.borrow().parent(n)
}

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).copied().unwrap_or(Value::Undefined)
}

/// Wrap a `Dom` in a `document` host object (fresh node cache) —
/// used by `install_dom` and by `xhr.responseXML`.
pub(crate) fn document_value(e: &mut Engine, dom: &DomRef) -> Value {
    e.new_host_object(Rc::new(DocumentHost {
        dom: dom.clone(),
        cache: Rc::new(RefCell::new(HashMap::new())),
    }))
}

impl Engine {
    /// Install `document` (host object over `dom`) and a minimal
    /// fixed `navigator` on the global object.
    pub fn install_dom(&mut self, dom: &DomRef) {
        // Cache partagé avec l'engine — `dom_event` en a besoin
        // pour wrapper la cible avec la même identité d'objet.
        let cache: NodeCache = Rc::new(RefCell::new(HashMap::new()));
        let doc = self.new_host_object(Rc::new(DocumentHost {
            dom: dom.clone(),
            cache: cache.clone(),
        }));
        // `#document` == `document` : listeners/`this`/`on*` au
        // sommet de la propagation résolvent sur l'objet document
        // (sémantique DOM réelle — `currentTarget === document`).
        if let Value::Obj(id) = doc {
            cache.borrow_mut().insert(dom.borrow().root, id);
        }
        self.dom_ctx = Some((dom.clone(), cache));
        let g = self.global;
        self.set_own(g, "document", doc, false);

        // Fixed, low-fingerprint navigator — by design.
        let nav = self.new_object();
        for (k, v) in [
            ("userAgent", "Explorer/0.1 (dillo-js)"),
            ("appName", "Netscape"),
            ("platform", "Win32"),
            ("language", "en-US"),
            ("appVersion", "5.0"),
        ] {
            let s = self.str_val(v);
            self.set_own(nav, k, s, true);
        }
        self.set_own(g, "navigator", Value::Obj(nav), false);

        // `window`/`self`/`globalThis` — the global object itself.
        let gv = Value::Obj(g);
        self.set_own(g, "window", gv, false);
        self.set_own(g, "self", gv, false);
        self.set_own(g, "globalThis", gv, false);
        // `MutationObserver` (JS-C26) — les records vivent dans
        // la file du `Dom`, livrés au microtask checkpoint.
        self.install_mutation_observer();
        // `location` + `history` — hooks-driven (see BrowserHooks).
        let loc = self.new_host_object(Rc::new(LocationHost));
        let hist = self.new_host_object(Rc::new(HistoryHost));
        self.set_own(g, "location", loc, false);
        self.set_own(g, "history", hist, false);

        // JS-7f window surface — dialogs, screen geometry.
        // (`console` is installed engine-wide in `builtins::install`
        // — it exists in workers too, not just on `window`.)
        self.add_method(g, "alert", win_alert);
        self.add_method(g, "confirm", win_confirm);
        self.add_method(g, "prompt", win_prompt);
        self.add_method(g, "open", win_open);
        let screen = self.new_object();
        let (w, h) = self.hooks.viewport;
        for (k, v) in [
            ("width", w),
            ("height", h),
            ("availWidth", w),
            ("availHeight", h),
            ("colorDepth", 24),
            ("pixelDepth", 24),
        ] {
            self.set_own(screen, k, Value::Num(f64::from(v)), true);
        }
        self.hooks.screen_obj = Some(screen);
        self.set_own(g, "screen", Value::Obj(screen), false);
        self.set_own(g, "innerWidth", Value::Num(f64::from(w)), false);
        self.set_own(g, "innerHeight", Value::Num(f64::from(h)), false);
        self.set_own(g, "outerWidth", Value::Num(f64::from(w)), false);
        self.set_own(g, "outerHeight", Value::Num(f64::from(h)), false);
        self.set_own(g, "devicePixelRatio", Value::Num(1.0), false);
    }

    /// Build the `Event` host wrapper for `state`.
    pub(crate) fn event_value(
        &mut self,
        dom: &DomRef,
        cache: &NodeCache,
        state: &Rc<EventState>,
    ) -> Value {
        self.new_host_object(Rc::new(EventHost {
            dom: dom.clone(),
            cache: cache.clone(),
            state: state.clone(),
        }))
    }

    /// Platform-originated event entry point (JS-5): dispatches a
    /// bubbling `ty` event whose target is `<body>` — document-level
    /// handlers (`document.onclick`, delegated listeners) work.
    /// Returns `true` if a handler called `preventDefault`.
    ///
    /// # Errors
    /// Returns the first thrown exception value (listener error).
    pub fn dom_event(&mut self, ty: &str) -> JsResult<bool> {
        let Some((dom, _)) = self.dom_ctx.clone() else {
            return Ok(false);
        };
        let target = dom.borrow().body();
        self.dom_event_on(ty, target)
    }

    /// Element-targeted platform event (JS-5c): dispatches `ty` on
    /// `node` with full capture → target → bubble propagation and
    /// returns `defaultPrevented` — the embedder uses it to map a
    /// widget-level hit (link click…) back to the mirror node.
    ///
    /// # Errors
    /// Returns the first thrown exception value (listener error).
    pub fn dom_event_on(&mut self, ty: &str, node: NodeId) -> JsResult<bool> {
        let Some((dom, cache)) = self.dom_ctx.clone() else {
            return Ok(false);
        };
        let state = Rc::new(EventState {
            ty: RefCell::new(ty.to_string()),
            target: Cell::new(node),
            current: Cell::new(None),
            bubbles: Cell::new(true),
            stopped: Cell::new(false),
            prevented: Cell::new(false),
        });
        self.propagate_event(&dom, &cache, &state)?;
        // Event dispatch is a job boundary — drain queued reactions.
        self.run_microtasks()?;
        Ok(state.prevented.get())
    }

    /// Fire a simple bubbling event (`click()`, `submit()`… and the
    /// future platform-event entry point — JS-3/JS-5).
    pub(crate) fn dispatch_simple(
        &mut self,
        dom: &DomRef,
        cache: &NodeCache,
        node: NodeId,
        ty: &str,
    ) -> JsResult<()> {
        let state = Rc::new(EventState {
            ty: RefCell::new(ty.to_string()),
            target: Cell::new(node),
            current: Cell::new(None),
            bubbles: Cell::new(true),
            stopped: Cell::new(false),
            prevented: Cell::new(false),
        });
        self.propagate_event(dom, cache, &state)
    }

    /// Full capture → target → bubble walk over the mini-DOM.
    pub(crate) fn propagate_event(
        &mut self,
        dom: &DomRef,
        cache: &NodeCache,
        ev: &Rc<EventState>,
    ) -> JsResult<()> {
        let target = ev.target.get();
        let path = dom.borrow().ancestors(target); // parent … root
        let ev_val = self.event_value(dom, cache, ev);
        // Capture: root → parent (capture listeners only).
        for &n in path.iter().rev() {
            if ev.stopped.get() {
                return Ok(());
            }
            ev.current.set(Some(n));
            self.fire_listeners(dom, cache, n, ev, &ev_val, true)?;
        }
        // At target: capture listeners, then bubble listeners/on*.
        ev.current.set(Some(target));
        self.fire_listeners(dom, cache, target, ev, &ev_val, true)?;
        if !ev.stopped.get() {
            self.fire_listeners(dom, cache, target, ev, &ev_val, false)?;
        }
        // Bubble: parent → root.
        if ev.bubbles.get() {
            for &n in &path {
                if ev.stopped.get() {
                    break;
                }
                ev.current.set(Some(n));
                self.fire_listeners(dom, cache, n, ev, &ev_val, false)?;
            }
        }
        Ok(())
    }

    /// Run the matching listeners on one node (`capture` selects the
    /// listener subset); on the non-capture pass the `on<type>` own
    /// prop also fires (script-assigned `el.onclick = f`).
    fn fire_listeners(
        &mut self,
        dom: &DomRef,
        cache: &NodeCache,
        node: NodeId,
        ev: &Rc<EventState>,
        ev_val: &Value,
        capture: bool,
    ) -> JsResult<()> {
        let ty = ev.ty.borrow().clone();
        let fns: Vec<Value> = {
            let d = dom.borrow();
            d.nodes
                .get(node as usize)
                .map(|n| {
                    n.listeners
                        .iter()
                        .filter(|(t, _, c)| t == &ty && *c == capture)
                        .map(|(_, f, _)| *f)
                        .collect()
                })
                .unwrap_or_default()
        };
        let this = node_value(self, dom, cache, node);
        for f in fns {
            if ev.stopped.get() {
                break;
            }
            if self.is_callable(f) {
                self.call_value(f, this, &[*ev_val])?;
            }
        }
        if !capture && !ev.stopped.get() {
            if let Value::Obj(id) = this {
                let key = format!("on{ty}");
                if let Some(f) = self.get_prop(id, &key) {
                    if self.is_callable(f) {
                        self.call_value(f, this, &[*ev_val])?;
                    }
                }
            }
        }
        Ok(())
    }
}

// ------------------------------------------------------------------
// document
// ------------------------------------------------------------------

impl Host for DocumentHost {
    fn class_name(&self) -> &'static str {
        "HTMLDocument"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn get(&self, e: &mut Engine, _this: Value, key: &str) -> JsResult<Option<Value>> {
        match key {
            "body" => {
                let n = self.dom.borrow().body();
                Ok(Some(node_value(e, &self.dom, &self.cache, n)))
            }
            "documentElement" => {
                let n = self.dom.borrow().document_element();
                Ok(Some(match n {
                    Some(n) => node_value(e, &self.dom, &self.cache, n),
                    None => Value::Null,
                }))
            }
            "title" => {
                let t = {
                    let d = self.dom.borrow();
                    d.by_tag(d.root, "title")
                        .first()
                        .map(|&n| d.text_content(n))
                        .unwrap_or_default()
                };
                Ok(Some(e.str_val(t)))
            }
            "URL" | "documentURI" => Ok(Some(e.str_val(e.hooks.page_url.clone()))),
            // `document.links`/`images`/`forms`/`scripts` — live-ish
            // (recomputed at each access) tag collections ≙ amont
            // `links[]`.
            "links" | "images" | "forms" | "scripts" => {
                let tag = match key {
                    "links" => "a",
                    "images" => "img",
                    "forms" => "form",
                    _ => "script",
                };
                let ids = {
                    let d = self.dom.borrow();
                    d.by_tag(d.root, tag)
                };
                let vals: Vec<Value> = ids
                    .iter()
                    .map(|&n| node_value(e, &self.dom, &self.cache, n))
                    .collect();
                Ok(Some(e.new_array(vals)))
            }
            "head" => {
                let n = {
                    let d = self.dom.borrow();
                    d.by_tag(d.root, "head").first().copied()
                };
                Ok(Some(match n {
                    Some(n) => node_value(e, &self.dom, &self.cache, n),
                    None => Value::Null,
                }))
            }
            // Parsing is done when scripts run — `complete` is the
            // only state we can observe (no `loading`/`interactive`).
            "readyState" => Ok(Some(e.str_val("complete"))),
            // `document.cookie` — raw `"k=v; …"` via the injected jar.
            "cookie" => Ok(Some(e.str_val(match &e.hooks.cookie_jar {
                Some(jar) => jar.get(&e.hooks.page_url),
                None => String::new(),
            }))),
            _ => Ok(None),
        }
    }

    fn set(&self, e: &mut Engine, _this: Value, key: &str, v: Value) -> JsResult<bool> {
        if key == "cookie" {
            let s = e.to_js_string(v)?;
            if let Some(jar) = &e.hooks.cookie_jar {
                jar.set(&e.hooks.page_url, &s);
            }
            return Ok(true);
        }
        if key == "title" {
            // Spec : le texte du `<title>` existant est
            // remplacé ; à défaut un `<title>` est créé et
            // rattaché à `<head>` (pas de `<head>` → no-op).
            let s = e.to_js_string(v)?;
            let mut d = self.dom.borrow_mut();
            let title = d.by_tag(d.root, "title").first().copied();
            match title {
                Some(t) => {
                    let txt = d.create_text(&s);
                    d.replace_children(t, &[txt]);
                }
                None => {
                    if let Some(head) = d.by_tag(d.root, "head").first().copied() {
                        let t = d.create_element("title");
                        let txt = d.create_text(&s);
                        d.append(t, txt);
                        d.append(head, t);
                    }
                }
            }
            return Ok(true);
        }
        Ok(false)
    }

    fn invoke(
        &self,
        e: &mut Engine,
        _this: Value,
        name: &str,
        args: &[Value],
    ) -> JsResult<Option<Value>> {
        match name {
            // Listeners au niveau document → nœud racine : la
            // propagation bulle/capture l'atteint en fin de parcours
            // (≙ `document.addEventListener` réel).
            "addEventListener" => {
                let ty = e.to_js_string(arg(args, 0))?;
                let f = arg(args, 1);
                let capture = e.to_boolean(arg(args, 2));
                if e.is_callable(f) {
                    let root = self.dom.borrow().root;
                    self.dom.borrow_mut().nodes[root as usize]
                        .listeners
                        .push((ty, f, capture));
                }
                Ok(Some(Value::Undefined))
            }
            "removeEventListener" => {
                let ty = e.to_js_string(arg(args, 0))?;
                let f = arg(args, 1);
                let capture = e.to_boolean(arg(args, 2));
                let root = self.dom.borrow().root;
                self.dom.borrow_mut().nodes[root as usize]
                    .listeners
                    .retain(|(t, lf, c)| !(t == &ty && *c == capture && e.strict_eq(*lf, f)));
                Ok(Some(Value::Undefined))
            }
            "dispatchEvent" => {
                let root = self.dom.borrow().root;
                let Some(state) = event_state_of(e, root, &self.dom, &self.cache, arg(args, 0))?
                else {
                    return Ok(Some(Value::Bool(false)));
                };
                e.propagate_event(&self.dom, &self.cache, &state)?;
                Ok(Some(Value::Bool(!state.prevented.get())))
            }
            "getElementById" => {
                let id = e.to_js_string(arg(args, 0))?;
                let found = self.dom.borrow().by_id(&id);
                Ok(Some(match found {
                    Some(n) => node_value(e, &self.dom, &self.cache, n),
                    None => Value::Null,
                }))
            }
            "getElementsByTagName" => {
                let tag = e.to_js_string(arg(args, 0))?;
                let ids = {
                    let d = self.dom.borrow();
                    d.by_tag(d.root, &tag)
                };
                let vals: Vec<Value> = ids
                    .iter()
                    .map(|&n| node_value(e, &self.dom, &self.cache, n))
                    .collect();
                Ok(Some(e.new_array(vals)))
            }
            "getElementsByClassName" => {
                let cls = e.to_js_string(arg(args, 0))?;
                let ids = {
                    let d = self.dom.borrow();
                    d.by_class(d.root, &cls)
                };
                let vals: Vec<Value> = ids
                    .iter()
                    .map(|&n| node_value(e, &self.dom, &self.cache, n))
                    .collect();
                Ok(Some(e.new_array(vals)))
            }
            "querySelector" => {
                let sel = e.to_js_string(arg(args, 0))?;
                let root = self.dom.borrow().root;
                let ids = self.dom.borrow().query(root, &sel);
                Ok(Some(match ids.first() {
                    Some(&n) => node_value(e, &self.dom, &self.cache, n),
                    None => Value::Null,
                }))
            }
            "querySelectorAll" => {
                let sel = e.to_js_string(arg(args, 0))?;
                let root = self.dom.borrow().root;
                let ids = self.dom.borrow().query(root, &sel);
                let vals: Vec<Value> = ids
                    .iter()
                    .map(|&n| node_value(e, &self.dom, &self.cache, n))
                    .collect();
                Ok(Some(e.new_array(vals)))
            }
            "createElement" => {
                let tag = e.to_js_string(arg(args, 0))?;
                let n = self.dom.borrow_mut().create_element(&tag);
                Ok(Some(node_value(e, &self.dom, &self.cache, n)))
            }
            "createTextNode" => {
                let t = e.to_js_string(arg(args, 0))?;
                let n = self.dom.borrow_mut().create_text(&t);
                Ok(Some(node_value(e, &self.dom, &self.cache, n)))
            }
            // `document.createEvent('HTMLEvents')` + `initEvent` —
            // the old-school event constructor path.
            "createEvent" => {
                let st = Rc::new(EventState {
                    ty: RefCell::new(String::new()),
                    target: Cell::new(0),
                    current: Cell::new(None),
                    bubbles: Cell::new(false),
                    stopped: Cell::new(false),
                    prevented: Cell::new(false),
                });
                Ok(Some(e.new_host_object(Rc::new(EventHost {
                    dom: self.dom.clone(),
                    cache: self.cache.clone(),
                    state: st,
                }))))
            }
            // document.write during parse is upstream's semantic;
            // here it appends a fragment to <body>.
            "write" | "writeln" => {
                let s = e.to_js_string(arg(args, 0))?;
                let mut d = self.dom.borrow_mut();
                let body = d.body();
                d.parse_fragment_into(body, &s);
                Ok(Some(Value::Undefined))
            }
            _ => Ok(None),
        }
    }
}

// ------------------------------------------------------------------
// Element / text node
// ------------------------------------------------------------------

/// Attributes mirrored as properties (get + set).
const MAPPED_ATTRS: &[&str] = &[
    "id",
    "class",
    "name",
    "value",
    "href",
    "src",
    "type",
    "title",
    "alt",
    "placeholder",
    "action",
    "method",
    "target",
    "rel",
];

/// Boolean attributes mirrored as `bool` props (`checked`…).
const BOOL_ATTRS: &[&str] = &["checked", "disabled", "selected", "readonly", "multiple"];

/// `className` ↔ `class`, `htmlFor` ↔ `for`, `tagName` (ro)…
fn prop_to_attr(key: &str) -> Option<&'static str> {
    Some(match key {
        "className" => "class",
        "htmlFor" => "for",
        "value" => "value",
        k => return MAPPED_ATTRS.iter().copied().find(|&a| a == k),
    })
}

impl Host for NodeHost {
    fn class_name(&self) -> &'static str {
        "HTMLElement"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn get(&self, e: &mut Engine, this: Value, key: &str) -> JsResult<Option<Value>> {
        let node = self.node;
        // Fast paths that don't need the dom borrow held across calls.
        match key {
            "style" => {
                return Ok(Some(e.new_host_object(Rc::new(StyleHost {
                    dom: self.dom.clone(),
                    node,
                }))));
            }
            "classList" => {
                return Ok(Some(e.new_host_object(Rc::new(ClassListHost {
                    dom: self.dom.clone(),
                    node,
                }))));
            }
            "parentNode" | "parentElement" => {
                let p = self.dom.borrow().nodes[node as usize].parent;
                return Ok(Some(match p {
                    Some(p) => node_value(e, &self.dom, &self.cache, p),
                    None => Value::Null,
                }));
            }
            "children" => {
                let kids = self.dom.borrow().element_children(node);
                let vals: Vec<Value> = kids
                    .iter()
                    .map(|&n| node_value(e, &self.dom, &self.cache, n))
                    .collect();
                return Ok(Some(e.new_array(vals)));
            }
            "firstChild" => {
                let c = self.dom.borrow().nodes[node as usize]
                    .children
                    .first()
                    .copied();
                return Ok(Some(match c {
                    Some(c) => node_value(e, &self.dom, &self.cache, c),
                    None => Value::Null,
                }));
            }
            "lastChild" => {
                let c = self.dom.borrow().nodes[node as usize]
                    .children
                    .last()
                    .copied();
                return Ok(Some(match c {
                    Some(c) => node_value(e, &self.dom, &self.cache, c),
                    None => Value::Null,
                }));
            }
            // `childNodes` — all kinds incl. text (`children` is
            // elements-only above).
            "childNodes" => {
                let kids = self.dom.borrow().child_nodes(node);
                let vals: Vec<Value> = kids
                    .iter()
                    .map(|&n| node_value(e, &self.dom, &self.cache, n))
                    .collect();
                return Ok(Some(e.new_array(vals)));
            }
            "childElementCount" => {
                let n = self.dom.borrow().element_children(node).len();
                return Ok(Some(Value::Num(n as f64)));
            }
            "firstElementChild" | "lastElementChild" => {
                let kids = self.dom.borrow().element_children(node);
                let c = if key == "firstElementChild" {
                    kids.first()
                } else {
                    kids.last()
                };
                return Ok(Some(match c {
                    Some(&c) => node_value(e, &self.dom, &self.cache, c),
                    None => Value::Null,
                }));
            }
            "previousSibling" | "previousElementSibling" => {
                let c = {
                    let d = self.dom.borrow();
                    let mut sib = d.sibling(node, false);
                    while let Some(s) = sib {
                        if key == "previousSibling" || d.nodes[s as usize].kind == NodeKind::Element
                        {
                            break;
                        }
                        sib = d.sibling(s, false);
                    }
                    sib
                };
                return Ok(Some(match c {
                    Some(c) => node_value(e, &self.dom, &self.cache, c),
                    None => Value::Null,
                }));
            }
            "outerHTML" => {
                let s = self.dom.borrow().outer_html(node);
                return Ok(Some(e.str_val(s)));
            }
            // `dataset` — live host view over `data-*` attrs
            // (`el.dataset.fooBar` ≙ `data-foo-bar`).
            "dataset" => {
                return Ok(Some(e.new_host_object(Rc::new(DatasetHost {
                    dom: self.dom.clone(),
                    node,
                }))));
            }
            "nodeValue" | "data" => {
                let n = &self.dom.borrow().nodes[node as usize];
                let v = if n.kind == NodeKind::Text {
                    n.text.clone()
                } else {
                    String::new()
                };
                let v = if v.is_empty() && n.kind == NodeKind::Element {
                    Value::Null
                } else {
                    e.str_val(v)
                };
                return Ok(Some(v));
            }
            "ownerDocument" => {
                return Ok(Some(
                    e.get_prop(e.global, "document").unwrap_or(Value::Null),
                ));
            }
            "length" if self.dom.borrow().nodes[node as usize].kind == NodeKind::Text => {
                let n = self.dom.borrow().nodes[node as usize].text.len();
                return Ok(Some(Value::Num(n as f64)));
            }
            "nextSibling" | "nextElementSibling" => {
                let c = {
                    let d = self.dom.borrow();
                    d.nodes[node as usize].parent.and_then(|p| {
                        let kids = &d.nodes[p as usize].children;
                        let pos = kids.iter().position(|&k| k == node)?;
                        kids[pos + 1..].iter().copied().find(|&k| {
                            key == "nextSibling" || d.nodes[k as usize].kind == NodeKind::Element
                        })
                    })
                };
                return Ok(Some(match c {
                    Some(c) => node_value(e, &self.dom, &self.cache, c),
                    None => Value::Null,
                }));
            }
            "tagName" | "nodeName" => {
                let t = self.dom.borrow().nodes[node as usize].tag.clone();
                return Ok(Some(e.str_val(t.to_uppercase())));
            }
            "innerHTML" => {
                let s = self.dom.borrow().inner_html(node);
                return Ok(Some(e.str_val(s)));
            }
            "textContent" | "innerText" => {
                let s = self.dom.borrow().text_content(node);
                return Ok(Some(e.str_val(s)));
            }
            "nodeType" => {
                let k = self.dom.borrow().nodes[node as usize].kind;
                return Ok(Some(Value::Num(match k {
                    NodeKind::Element => 1.0,
                    NodeKind::Text => 3.0,
                })));
            }
            // `textarea.value` ≙ le contenu texte, `select.value`
            // ≙ la valeur de l'option `selected` (pas d'attribut
            // `value` dans le markup pour ces deux tags).
            "value" => {
                let d = self.dom.borrow();
                let tag = d.nodes[node as usize].tag.as_str();
                let v = d.attr(node, "value").map(str::to_string).or_else(|| {
                    matches!(tag, "textarea" | "select").then(|| control_value(&d, node))
                });
                return Ok(Some(match v {
                    Some(v) => e.str_val(v),
                    None => e.str_val(""),
                }));
            }
            // -- Validation de contraintes (étape 54) ------------------
            "willValidate" => {
                let d = self.dom.borrow();
                return Ok(Some(Value::Bool(will_validate(&d, node))));
            }
            "validationMessage" => {
                let d = self.dom.borrow();
                let msg = constraint_failures(&d, node)
                    .first()
                    .map(|f| failure_message(&d, node, f))
                    .unwrap_or_default();
                return Ok(Some(e.str_val(msg)));
            }
            "validity" => {
                let d = self.dom.borrow();
                let flags = constraint_failures(&d, node);
                let has = |f: &'static str| flags.contains(&f);
                let obj = e.new_object();
                for (k, v) in [
                    ("valueMissing", has("valueMissing")),
                    ("typeMismatch", false),
                    ("patternMismatch", has("patternMismatch")),
                    ("tooLong", has("tooLong")),
                    ("tooShort", has("tooShort")),
                    ("rangeUnderflow", has("rangeUnderflow")),
                    ("rangeOverflow", has("rangeOverflow")),
                    ("stepMismatch", has("stepMismatch")),
                    ("badInput", false),
                    ("customError", has("customError")),
                    ("valid", flags.is_empty()),
                ] {
                    e.set_own(obj, k, Value::Bool(v), true);
                }
                return Ok(Some(Value::Obj(obj)));
            }
            _ => {}
        }
        // Attribute-mapped props.
        if let Some(attr) = prop_to_attr(key) {
            let v = self.dom.borrow().attr(node, attr).map(str::to_string);
            return Ok(Some(match v {
                Some(v) => e.str_val(v),
                None => e.str_val(""),
            }));
        }
        if BOOL_ATTRS.contains(&key) {
            let has = self.dom.borrow().has_attr(node, key);
            return Ok(Some(Value::Bool(has)));
        }
        let _ = this;
        Ok(None)
    }

    fn set(&self, e: &mut Engine, _this: Value, key: &str, v: Value) -> JsResult<bool> {
        let node = self.node;
        match key {
            "innerHTML" => {
                let html = e.to_js_string(v)?;
                let mut d = self.dom.borrow_mut();
                // Detach existing children, parse into a temp parent,
                // move results over.
                let tmp = d.create_element("#fragment");
                d.parse_fragment_into(tmp, &html);
                let kids = std::mem::take(&mut d.nodes[tmp as usize].children);
                for &k in &kids {
                    d.nodes[k as usize].parent = None;
                }
                d.replace_children(node, &kids);
                Ok(true)
            }
            "textContent" | "innerText" => {
                let s = e.to_js_string(v)?;
                let mut d = self.dom.borrow_mut();
                let t = d.create_text(&s);
                d.replace_children(node, &[t]);
                Ok(true)
            }
            // `CharacterData` — écriture directe du payload
            // texte (record `characterData`, JS-C26).
            "nodeValue" | "data" => {
                let is_text = self.dom.borrow().nodes[node as usize].kind == NodeKind::Text;
                if is_text {
                    let s = e.to_js_string(v)?;
                    self.dom.borrow_mut().set_text(node, &s);
                }
                Ok(is_text)
            }
            "className" => {
                let s = e.to_js_string(v)?;
                self.dom.borrow_mut().set_attr(node, "class", &s);
                Ok(true)
            }
            "value" => {
                let s = e.to_js_string(v)?;
                let mut d = self.dom.borrow_mut();
                if d.nodes[node as usize].tag.eq_ignore_ascii_case("textarea") {
                    // `textarea` n'a pas d'attribut `value` — la
                    // valeur est le contenu texte.
                    let t = d.create_text(&s);
                    d.replace_children(node, &[t]);
                } else {
                    d.set_attr(node, "value", &s);
                }
                Ok(true)
            }
            _ => {
                if BOOL_ATTRS.contains(&key) {
                    let on = e.to_boolean(v);
                    let mut d = self.dom.borrow_mut();
                    if on {
                        d.set_attr(node, key, "");
                    } else {
                        d.remove_attr(node, key);
                    }
                    return Ok(true);
                }
                if let Some(attr) = prop_to_attr(key) {
                    let s = e.to_js_string(v)?;
                    self.dom.borrow_mut().set_attr(node, attr, &s);
                    return Ok(true);
                }
                Ok(false)
            }
        }
    }

    fn invoke(
        &self,
        e: &mut Engine,
        _this: Value,
        name: &str,
        args: &[Value],
    ) -> JsResult<Option<Value>> {
        let node = self.node;
        match name {
            "getElementsByTagName" => {
                let tag = e.to_js_string(arg(args, 0))?;
                let ids = self.dom.borrow().by_tag(node, &tag);
                let vals: Vec<Value> = ids
                    .iter()
                    .map(|&n| node_value(e, &self.dom, &self.cache, n))
                    .collect();
                Ok(Some(e.new_array(vals)))
            }
            "getElementsByClassName" => {
                let cls = e.to_js_string(arg(args, 0))?;
                let ids = self.dom.borrow().by_class(node, &cls);
                let vals: Vec<Value> = ids
                    .iter()
                    .map(|&n| node_value(e, &self.dom, &self.cache, n))
                    .collect();
                Ok(Some(e.new_array(vals)))
            }
            "querySelector" => {
                let sel = e.to_js_string(arg(args, 0))?;
                let ids = self.dom.borrow().query(node, &sel);
                Ok(Some(match ids.first() {
                    Some(&n) => node_value(e, &self.dom, &self.cache, n),
                    None => Value::Null,
                }))
            }
            "querySelectorAll" => {
                let sel = e.to_js_string(arg(args, 0))?;
                let ids = self.dom.borrow().query(node, &sel);
                let vals: Vec<Value> = ids
                    .iter()
                    .map(|&n| node_value(e, &self.dom, &self.cache, n))
                    .collect();
                Ok(Some(e.new_array(vals)))
            }
            "appendChild" | "append" => {
                let target = node_for(e, arg(args, 0));
                match target {
                    Some(child) => {
                        self.dom.borrow_mut().append(node, child);
                        Ok(Some(arg(args, 0)))
                    }
                    None => Err(e.type_error("appendChild: not a node")),
                }
            }
            "removeChild" => {
                let target = node_for(e, arg(args, 0));
                match target {
                    Some(child) => {
                        self.dom.borrow_mut().detach(child);
                        Ok(Some(arg(args, 0)))
                    }
                    None => Err(e.type_error("removeChild: not a node")),
                }
            }
            "remove" => {
                self.dom.borrow_mut().detach(node);
                Ok(Some(Value::Undefined))
            }
            "setAttribute" => {
                let k = e.to_js_string(arg(args, 0))?;
                let v = e.to_js_string(arg(args, 1))?;
                self.dom.borrow_mut().set_attr(node, &k, &v);
                Ok(Some(Value::Undefined))
            }
            "getAttribute" => {
                let k = e.to_js_string(arg(args, 0))?;
                let v = self.dom.borrow().attr(node, &k).map(str::to_string);
                Ok(Some(match v {
                    Some(v) => e.str_val(v),
                    None => Value::Null,
                }))
            }
            "hasAttribute" => {
                let k = e.to_js_string(arg(args, 0))?;
                let has = self.dom.borrow().has_attr(node, &k);
                Ok(Some(Value::Bool(has)))
            }
            "removeAttribute" => {
                let k = e.to_js_string(arg(args, 0))?;
                self.dom.borrow_mut().remove_attr(node, &k);
                Ok(Some(Value::Undefined))
            }
            "addEventListener" => {
                let ty = e.to_js_string(arg(args, 0))?;
                let f = arg(args, 1);
                let capture = e.to_boolean(arg(args, 2));
                if e.is_callable(f) {
                    self.dom.borrow_mut().nodes[node as usize]
                        .listeners
                        .push((ty, f, capture));
                }
                Ok(Some(Value::Undefined))
            }
            "removeEventListener" => {
                let ty = e.to_js_string(arg(args, 0))?;
                let f = arg(args, 1);
                let capture = e.to_boolean(arg(args, 2));
                self.dom.borrow_mut().nodes[node as usize]
                    .listeners
                    .retain(|(t, lf, c)| !(t == &ty && *c == capture && e.strict_eq(*lf, f)));
                Ok(Some(Value::Undefined))
            }
            // `el.dispatchEvent(ev)` — EventHost or plain {type:…}.
            "dispatchEvent" => {
                let Some(state) = event_state_of(e, node, &self.dom, &self.cache, arg(args, 0))?
                else {
                    return Err(e.type_error("dispatchEvent: not an event"));
                };
                e.propagate_event(&self.dom, &self.cache, &state)?;
                Ok(Some(Value::Bool(!state.prevented.get())))
            }
            // Event triggers callable from scripts; the same
            // `propagate_event` is the platform-event entry point.
            "click" | "focus" | "blur" => {
                e.dispatch_simple(&self.dom, &self.cache, node, name)?;
                Ok(Some(Value::Undefined))
            }
            // `form.submit()` : soumission directe — ni
            // événement `submit` ni validation (spec HTML,
            // contrairement au clic sur un bouton).
            "submit" => {
                if let Some(p) = self.submit_payload(node, None) {
                    if let Some(h) = &e.hooks.form_submit {
                        h(p);
                    }
                }
                Ok(Some(Value::Undefined))
            }
            // `form.requestSubmit(submitter?)` : validation
            // (`invalid` sur les fautifs, `novalidate`/
            // `formnovalidate` exclus) → événement `submit`
            // → soumission si non annulée.
            "requestSubmit" => {
                let submitter = node_for(e, arg(args, 0));
                let ok = {
                    let d = self.dom.borrow();
                    let skip = d.has_attr(node, "novalidate")
                        || submitter.is_some_and(|s| d.has_attr(s, "formnovalidate"));
                    skip || {
                        drop(d);
                        self.check_validity_of(e, node)?
                    }
                };
                if !ok {
                    return Ok(Some(Value::Undefined));
                }
                let state = Rc::new(EventState {
                    ty: RefCell::new("submit".to_string()),
                    target: Cell::new(node),
                    current: Cell::new(None),
                    bubbles: Cell::new(true),
                    stopped: Cell::new(false),
                    prevented: Cell::new(false),
                });
                e.propagate_event(&self.dom, &self.cache, &state)?;
                if !state.prevented.get() {
                    if let Some(p) = self.submit_payload(node, submitter) {
                        if let Some(h) = &e.hooks.form_submit {
                            h(p);
                        }
                    }
                }
                Ok(Some(Value::Undefined))
            }
            // -- Validation de contraintes (étape 54) ------------------
            // `setCustomValidity(m)` : l'état vit dans l'attribut
            // `data-explorer-cv` — il se sérialise avec le miroir
            // et le moteur bloque la soumission dessus (canal
            // miroir→DOM réel documenté ADR-0014).
            "setCustomValidity" => {
                let msg = e.to_js_string(arg(args, 0))?;
                let mut d = self.dom.borrow_mut();
                if msg.is_empty() {
                    d.remove_attr(node, CUSTOM_VALIDITY_ATTR);
                } else {
                    d.set_attr(node, CUSTOM_VALIDITY_ATTR, &msg);
                }
                Ok(Some(Value::Undefined))
            }
            // `checkValidity`/`reportValidity` : `invalid` (non
            // annulable) part sur chaque contrôle fautif — sur un
            // `<form>`, sur tous ses candidats. Pas de bulle UI
            // native : les deux méthodes sont identiques (MVP).
            "checkValidity" | "reportValidity" => {
                Ok(Some(Value::Bool(self.check_validity_of(e, node)?)))
            }
            // -- JS-7g structural ops ------------------------------------
            "insertBefore" => {
                let new_n = node_for(e, arg(args, 0));
                let ref_n = node_for(e, arg(args, 1));
                match new_n {
                    Some(c) => {
                        self.dom.borrow_mut().insert_before(node, c, ref_n);
                        Ok(Some(arg(args, 0)))
                    }
                    None => Err(e.type_error("insertBefore: not a node")),
                }
            }
            "replaceChild" => {
                let new_n = node_for(e, arg(args, 0));
                let old_n = node_for(e, arg(args, 1));
                match (new_n, old_n) {
                    (Some(n), Some(o)) => {
                        self.dom.borrow_mut().replace_child(node, n, o);
                        Ok(Some(arg(args, 1)))
                    }
                    _ => Err(e.type_error("replaceChild: not a node")),
                }
            }
            "cloneNode" => {
                let deep = e.to_boolean(arg(args, 0));
                let c = self.dom.borrow_mut().clone_node(node, deep);
                Ok(Some(node_value(e, &self.dom, &self.cache, c)))
            }
            "contains" => {
                let r =
                    node_for(e, arg(args, 0)).is_some_and(|o| self.dom.borrow().contains(node, o));
                Ok(Some(Value::Bool(r)))
            }
            "matches" | "webkitMatchesSelector" => {
                let sel = e.to_js_string(arg(args, 0))?;
                let r = self.dom.borrow().matches(node, &sel);
                Ok(Some(Value::Bool(r)))
            }
            "closest" => {
                let sel = e.to_js_string(arg(args, 0))?;
                let hit = self.dom.borrow().closest(node, &sel);
                Ok(Some(match hit {
                    Some(n) => node_value(e, &self.dom, &self.cache, n),
                    None => Value::Null,
                }))
            }
            // `prepend(...kids)` / `before`/`after`/`replaceWith` —
            // nodes or strings (strings → text nodes).
            "prepend" => {
                let kids = self.norm_children(e, args)?;
                for k in kids {
                    let first = self.dom.borrow().child_nodes(node).first().copied();
                    self.dom.borrow_mut().insert_before(node, k, first);
                }
                Ok(Some(Value::Undefined))
            }
            "before" | "after" => {
                let kids = self.norm_children(e, args)?;
                // `before`: insert each kid just before `node`.
                // `after`: anchor slides so kids keep arg order.
                if let Some(p) = dom_parent(&self.dom, node) {
                    let mut anchor = node;
                    for k in kids {
                        let ref_n = if name == "after" {
                            self.dom.borrow().sibling(anchor, true)
                        } else {
                            Some(node)
                        };
                        self.dom.borrow_mut().insert_before(p, k, ref_n);
                        anchor = k;
                    }
                }
                Ok(Some(Value::Undefined))
            }
            "replaceWith" => {
                let kids = self.norm_children(e, args)?;
                if let Some(p) = dom_parent(&self.dom, node) {
                    for k in kids {
                        self.dom.borrow_mut().insert_before(p, k, Some(node));
                    }
                    self.dom.borrow_mut().detach(node);
                }
                Ok(Some(Value::Undefined))
            }
            "insertAdjacentHTML" => {
                let pos = e.to_js_string(arg(args, 0))?;
                let html = e.to_js_string(arg(args, 1))?;
                self.insert_adjacent(e, node, &pos, &html)?;
                Ok(Some(Value::Undefined))
            }
            // Same slots as `insertAdjacentHTML` but the payload is a
            // text node — never parsed (spec).
            "insertAdjacentText" => {
                let pos = e.to_js_string(arg(args, 0))?;
                let s = e.to_js_string(arg(args, 1))?;
                self.insert_adjacent_text(e, node, &pos, &s)?;
                Ok(Some(Value::Undefined))
            }
            // Zero-rect placeholder — layout metrics live in dw, not
            // in the mirror. Real geometry needs a dw bridge (JS-8).
            "getBoundingClientRect" => {
                let r = e.new_object();
                for k in [
                    "x", "y", "top", "left", "bottom", "right", "width", "height",
                ] {
                    e.set_own(r, k, Value::Num(0.0), true);
                }
                Ok(Some(Value::Obj(r)))
            }
            _ => Ok(None),
        }
    }
}

/// Extract the DOM node behind a host value, if it is one of ours.
pub(crate) fn node_for(e: &mut Engine, v: Value) -> Option<NodeId> {
    let Value::Obj(id) = v else {
        return None;
    };
    let h = match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::Host(h)) => h.clone(),
        _ => return None,
    };
    h.as_any().downcast_ref::<NodeHost>().map(|n| n.node)
}

/// Resolve a `dispatchEvent` argument to event state: an `Event`
/// host object (its target is reset to the dispatch node) or a
/// plain `{ type: 'x', bubbles: bool }` literal.
fn event_state_of(
    e: &mut Engine,
    node: NodeId,
    _dom: &DomRef,
    _cache: &NodeCache,
    v: Value,
) -> JsResult<Option<Rc<EventState>>> {
    let Value::Obj(id) = v else {
        return Ok(None);
    };
    if let Some(ObjKind::Host(h)) = e.heap.obj(id).map(|o| &o.kind) {
        if let Some(ev) = h.as_any().downcast_ref::<EventHost>() {
            let state = ev.state.clone();
            state.target.set(node);
            return Ok(Some(state));
        }
    }
    // Plain object literal: read `type` (required) + `bubbles`.
    let Some(ty_v) = e.get_prop(id, "type") else {
        return Ok(None);
    };
    let ty = e.to_js_string(ty_v)?;
    if ty.is_empty() {
        return Ok(None);
    }
    let bubbles = e.get_prop(id, "bubbles").is_some_and(|b| e.to_boolean(b));
    Ok(Some(Rc::new(EventState {
        ty: RefCell::new(ty),
        target: Cell::new(node),
        current: Cell::new(None),
        bubbles: Cell::new(bubbles),
        stopped: Cell::new(false),
        prevented: Cell::new(false),
    })))
}

// ------------------------------------------------------------------
// Event
// ------------------------------------------------------------------

impl Host for EventHost {
    fn class_name(&self) -> &'static str {
        "Event"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn get(&self, e: &mut Engine, _t: Value, key: &str) -> JsResult<Option<Value>> {
        Ok(Some(match key {
            "type" => e.str_val(self.state.ty.borrow().clone()),
            "target" | "srcElement" => {
                node_value(e, &self.dom, &self.cache, self.state.target.get())
            }
            "currentTarget" => match self.state.current.get() {
                Some(n) => node_value(e, &self.dom, &self.cache, n),
                None => Value::Null,
            },
            "bubbles" => Value::Bool(self.state.bubbles.get()),
            "defaultPrevented" => Value::Bool(self.state.prevented.get()),
            "cancelBubble" => Value::Bool(self.state.stopped.get()),
            _ => return Ok(None),
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
            "preventDefault" => {
                self.state.prevented.set(true);
                Ok(Some(Value::Undefined))
            }
            "stopPropagation" | "cancel" => {
                self.state.stopped.set(true);
                Ok(Some(Value::Undefined))
            }
            "initEvent" => {
                let ty = e.to_js_string(arg(args, 0))?;
                *self.state.ty.borrow_mut() = ty;
                self.state.bubbles.set(e.to_boolean(arg(args, 1)));
                Ok(Some(Value::Undefined))
            }
            _ => Ok(None),
        }
    }
}

// ------------------------------------------------------------------
// style
// ------------------------------------------------------------------

fn camel_to_kebab(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for c in s.chars() {
        if c.is_uppercase() {
            out.push('-');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

fn style_map(dom: &Dom, node: NodeId) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Some(style) = dom.attr(node, "style") {
        for decl in style.split(';') {
            if let Some((k, v)) = decl.split_once(':') {
                let k = k.trim().to_lowercase();
                if !k.is_empty() {
                    out.push((k, v.trim().to_string()));
                }
            }
        }
    }
    out
}

fn style_set(dom: &mut Dom, node: NodeId, key: &str, value: &str) {
    let mut map = style_map(dom, node);
    if let Some((_, v)) = map.iter_mut().find(|(k, _)| k == key) {
        *v = value.to_string();
    } else {
        map.push((key.to_string(), value.to_string()));
    }
    let css = map
        .iter()
        .map(|(k, v)| format!("{k}: {v}"))
        .collect::<Vec<_>>()
        .join("; ");
    dom.set_attr(node, "style", &css);
}

impl Host for StyleHost {
    fn class_name(&self) -> &'static str {
        "CSSStyleDeclaration"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn get(&self, e: &mut Engine, _t: Value, key: &str) -> JsResult<Option<Value>> {
        let kebab = camel_to_kebab(key);
        let v = {
            let d = self.dom.borrow();
            style_map(&d, self.node)
                .into_iter()
                .find(|(k, _)| k == &kebab)
                .map(|(_, v)| v)
        };
        Ok(Some(match v {
            Some(v) => e.str_val(v),
            None => e.str_val(""),
        }))
    }

    fn set(&self, e: &mut Engine, _t: Value, key: &str, v: Value) -> JsResult<bool> {
        let kebab = camel_to_kebab(key);
        let val = e.to_js_string(v)?;
        style_set(&mut self.dom.borrow_mut(), self.node, &kebab, &val);
        Ok(true)
    }

    fn invoke(
        &self,
        e: &mut Engine,
        _t: Value,
        name: &str,
        args: &[Value],
    ) -> JsResult<Option<Value>> {
        match name {
            "setProperty" => {
                let k = e.to_js_string(arg(args, 0))?.to_lowercase();
                let v = e.to_js_string(arg(args, 1))?;
                style_set(&mut self.dom.borrow_mut(), self.node, &k, &v);
                Ok(Some(Value::Undefined))
            }
            "getPropertyValue" => {
                let k = e.to_js_string(arg(args, 0))?.to_lowercase();
                let v = {
                    let d = self.dom.borrow();
                    style_map(&d, self.node)
                        .into_iter()
                        .find(|(kk, _)| kk == &k)
                        .map(|(_, v)| v)
                        .unwrap_or_default()
                };
                Ok(Some(e.str_val(v)))
            }
            "removeProperty" => {
                let k = e.to_js_string(arg(args, 0))?.to_lowercase();
                let mut d = self.dom.borrow_mut();
                let mut map = style_map(&d, self.node);
                map.retain(|(kk, _)| kk != &k);
                let css = map
                    .iter()
                    .map(|(k, v)| format!("{k}: {v}"))
                    .collect::<Vec<_>>()
                    .join("; ");
                if css.is_empty() {
                    d.remove_attr(self.node, "style");
                } else {
                    d.set_attr(self.node, "style", &css);
                }
                Ok(Some(e.str_val("")))
            }
            _ => Ok(None),
        }
    }
}

// ------------------------------------------------------------------
// classList
// ------------------------------------------------------------------

impl Host for ClassListHost {
    fn class_name(&self) -> &'static str {
        "DOMTokenList"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn get(&self, e: &mut Engine, _t: Value, key: &str) -> JsResult<Option<Value>> {
        if key == "length" {
            let n = {
                let d = self.dom.borrow();
                d.attr(self.node, "class")
                    .map_or(0, |c| c.split_whitespace().count())
            };
            return Ok(Some(Value::Num(n as f64)));
        }
        let _ = e;
        Ok(None)
    }

    fn invoke(
        &self,
        e: &mut Engine,
        _t: Value,
        name: &str,
        args: &[Value],
    ) -> JsResult<Option<Value>> {
        let cls = e.to_js_string(arg(args, 0))?;
        let mut d = self.dom.borrow_mut();
        let mut tokens: Vec<String> = d
            .attr(self.node, "class")
            .map(|c| c.split_whitespace().map(str::to_string).collect())
            .unwrap_or_default();
        let contains = tokens.iter().any(|t| t == &cls);
        match name {
            "add" => {
                if !contains {
                    tokens.push(cls);
                }
            }
            "remove" => tokens.retain(|t| t != &cls),
            "toggle" => {
                if contains {
                    tokens.retain(|t| t != &cls);
                } else {
                    tokens.push(cls);
                }
            }
            "contains" => return Ok(Some(Value::Bool(contains))),
            _ => return Ok(None),
        }
        d.set_attr(self.node, "class", &tokens.join(" "));
        Ok(Some(match name {
            "toggle" => Value::Bool(!contains),
            _ => Value::Undefined,
        }))
    }
}

// ------------------------------------------------------------------
// window.location / window.history (JS-7e)
// ------------------------------------------------------------------

/// `location` — getters read `hooks.page_url` (parsed with no URL
/// dep: scheme `//` authority `/` path `?` query `#` fragment);
/// setters/`assign`/`replace`/`reload` go through `hooks.navigate`.
struct LocationHost;

/// `(protocol, host, hostname, port, pathname, search, hash)` of `u`.
fn url_parts(u: &str) -> (String, String, String, String, String, String, String) {
    let (scheme, rest) = u.split_once("://").unwrap_or(("", u));
    let (authority, tail) = match rest.find(['/', '?', '#']) {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (hostname, port) = authority.split_once(':').unwrap_or((authority, ""));
    let hash = tail.rfind('#').map_or("", |i| &tail[i..]);
    let tail = &tail[..tail.len() - hash.len()];
    let search = tail.find('?').map_or("", |i| &tail[i..]);
    let path = &tail[..tail.len() - search.len()];
    (
        if scheme.is_empty() {
            String::new()
        } else {
            format!("{scheme}:")
        },
        authority.to_string(),
        hostname.to_string(),
        port.to_string(),
        if path.is_empty() {
            "/".into()
        } else {
            path.into()
        },
        search.into(),
        hash.into(),
    )
}

impl Host for LocationHost {
    fn class_name(&self) -> &'static str {
        "Location"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn get(&self, e: &mut Engine, _t: Value, key: &str) -> JsResult<Option<Value>> {
        let u = &e.hooks.page_url;
        let (proto, host, hostname, port, path, search, hash) = url_parts(u);
        let s = match key {
            "href" => u.clone(),
            "protocol" => proto,
            "host" => host,
            "hostname" => hostname,
            "port" => port,
            "pathname" => path,
            "search" => search,
            "hash" => hash,
            "origin" => {
                if proto.is_empty() {
                    "null".to_string()
                } else {
                    format!("{proto}//{host}")
                }
            }
            _ => return Ok(None),
        };
        Ok(Some(e.str_val(s)))
    }

    fn set(&self, e: &mut Engine, _t: Value, key: &str, v: Value) -> JsResult<bool> {
        // `location.href = u` (and bare `location = u`) navigate.
        if key == "href" {
            let u = e.to_js_string(v)?;
            if let Some(nav) = &e.hooks.navigate {
                nav(&u);
            }
            return Ok(true);
        }
        Ok(false)
    }

    fn invoke(
        &self,
        e: &mut Engine,
        _t: Value,
        name: &str,
        args: &[Value],
    ) -> JsResult<Option<Value>> {
        match name {
            "assign" | "replace" => {
                let u = e.to_js_string(arg(args, 0))?;
                if let Some(nav) = &e.hooks.navigate {
                    nav(&u);
                }
                Ok(Some(Value::Undefined))
            }
            "reload" => {
                let u = e.hooks.page_url.clone();
                if let Some(nav) = &e.hooks.navigate {
                    nav(&u);
                }
                Ok(Some(Value::Undefined))
            }
            "toString" => Ok(Some(e.str_val(e.hooks.page_url.clone()))),
            _ => Ok(None),
        }
    }
}

/// `history` — `back`/`forward`/`go(n)` through `hooks.history_go`,
/// `length` from `hooks.history_len` (embedder keeps it current).
struct HistoryHost;

impl Host for HistoryHost {
    fn class_name(&self) -> &'static str {
        "History"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn get(&self, e: &mut Engine, _t: Value, key: &str) -> JsResult<Option<Value>> {
        if key == "length" {
            return Ok(Some(Value::Num(f64::from(e.hooks.history_len))));
        }
        Ok(None)
    }

    fn invoke(
        &self,
        e: &mut Engine,
        _t: Value,
        name: &str,
        args: &[Value],
    ) -> JsResult<Option<Value>> {
        let n = match name {
            "back" => -1,
            "forward" => 1,
            "go" => e.to_number(arg(args, 0))? as i32,
            _ => return Ok(None),
        };
        if n != 0 {
            if let Some(go) = &e.hooks.history_go {
                go(n);
            }
        }
        Ok(Some(Value::Undefined))
    }
}

// ------------------------------------------------------------------
// JS-7f window surface — console + dialogs + window.open.
// All funnel into `BrowserHooks`; absent hooks degrade silently
// (alert → no-op, confirm → false, prompt → null) exactly like a
// headless embedder.
// ------------------------------------------------------------------

fn fmt_args(e: &mut Engine, args: &[Value]) -> JsResult<String> {
    let mut parts = Vec::with_capacity(args.len());
    for &a in args {
        parts.push(e.to_js_string(a)?);
    }
    Ok(parts.join(" "))
}

pub(crate) fn console_line(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let line = fmt_args(e, args)?;
    if let Some(f) = e.hooks.console.clone() {
        f(line);
    }
    Ok(Value::Undefined)
}

fn win_alert(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let msg = e.to_js_string(arg(args, 0))?;
    if let Some(f) = e.hooks.alert.clone() {
        f(msg);
    }
    Ok(Value::Undefined)
}

fn win_confirm(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let msg = e.to_js_string(arg(args, 0))?;
    let r = e.hooks.confirm.clone().is_some_and(|f| f(msg));
    Ok(Value::Bool(r))
}

fn win_prompt(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let msg = e.to_js_string(arg(args, 0))?;
    let default = e.to_js_string(arg(args, 1))?;
    let r = e.hooks.prompt.clone().and_then(|f| f(msg, default));
    Ok(r.map_or(Value::Null, |s| e.str_val(s)))
}

/// `window.open(url)` — `open` hook, else same-tab navigate.
fn win_open(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let url = e.to_js_string(arg(args, 0))?;
    if url.is_empty() {
        return Ok(Value::Null);
    }
    if let Some(f) = e.hooks.open.clone() {
        f(url);
    } else if let Some(nav) = e.hooks.navigate.clone() {
        nav(&url);
    }
    Ok(Value::Null)
}

// ------------------------------------------------------------------
// JS-7g — `dataset` : vue live sur les attributs `data-*`.
// `el.dataset.fooBar` ≙ `data-foo-bar` (camelCase ↔ kebab-case).
// ------------------------------------------------------------------

/// `data-*` attr name → dataset key (`data-foo-bar` → `fooBar`).
fn data_key_to_camel(attr: &str) -> Option<String> {
    let rest = attr.strip_prefix("data-")?;
    let mut out = String::with_capacity(rest.len());
    let mut up = false;
    for c in rest.chars() {
        if c == '-' {
            up = true;
        } else if up {
            out.extend(c.to_uppercase());
            up = false;
        } else {
            out.push(c);
        }
    }
    Some(out)
}

/// Dataset key → `data-*` attr (`fooBar` → `data-foo-bar`).
fn camel_to_data_key(key: &str) -> String {
    let mut out = String::with_capacity(key.len() + 5);
    out.push_str("data-");
    for c in key.chars() {
        if c.is_uppercase() {
            out.push('-');
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// `el.dataset` host object — reads/writes the element's `data-*`
/// attributes live.
struct DatasetHost {
    dom: DomRef,
    node: NodeId,
}

impl Host for DatasetHost {
    fn get(&self, e: &mut Engine, _t: Value, key: &str) -> JsResult<Option<Value>> {
        let v = self
            .dom
            .borrow()
            .attr(self.node, &camel_to_data_key(key))
            .map(str::to_string);
        Ok(Some(match v {
            Some(v) => e.str_val(v),
            None => Value::Undefined,
        }))
    }

    fn set(&self, e: &mut Engine, _t: Value, key: &str, v: Value) -> JsResult<bool> {
        let s = e.to_js_string(v)?;
        self.dom
            .borrow_mut()
            .set_attr(self.node, &camel_to_data_key(key), &s);
        Ok(true)
    }

    fn keys(&self, _e: &mut Engine, _t: Value) -> Vec<String> {
        self.dom.borrow().nodes[self.node as usize]
            .attrs
            .iter()
            .filter_map(|(k, _)| data_key_to_camel(k))
            .collect()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

// ------------------------------------------------------------------
// Validation de contraintes des formulaires (étape 54)
// ------------------------------------------------------------------

/// Attribut portant l'état `setCustomValidity` — sérialisé avec le
/// miroir, il atteint le DOM réel qui bloque la soumission
/// (canal miroir→moteur, déviation `outerHTML` documentée).
const CUSTOM_VALIDITY_ATTR: &str = "data-explorer-cv";

/// `willValidate` : `input`/`textarea`/`select` non `disabled`,
/// types « barrés » exclus (`hidden`, boutons, `file` — HTML
/// §4.10.21.2).
fn will_validate(d: &Dom, node: NodeId) -> bool {
    let n = &d.nodes[node as usize];
    if n.kind != NodeKind::Element || d.has_attr(node, "disabled") {
        return false;
    }
    match n.tag.as_str() {
        "input" => !matches!(
            d.attr(node, "type").unwrap_or("").to_lowercase().as_str(),
            "hidden" | "submit" | "image" | "reset" | "button" | "file"
        ),
        "textarea" | "select" => true,
        _ => false,
    }
}

/// Valeur courante du contrôle : attribut `value` (texte du
/// `<textarea>` / option `selected` du `<select>` à défaut).
fn control_value(d: &Dom, node: NodeId) -> String {
    match d.nodes[node as usize].tag.as_str() {
        "textarea" => d
            .attr(node, "value")
            .map_or_else(|| d.text_content(node), str::to_string),
        "select" => {
            let opts = d.by_tag(node, "option");
            let sel = opts
                .iter()
                .find(|&&o| d.has_attr(o, "selected"))
                .or_else(|| opts.first());
            sel.map_or_else(String::new, |&o| {
                d.attr(o, "value")
                    .map_or_else(|| d.text_content(o).trim().to_string(), str::to_string)
            })
        }
        _ => d.attr(node, "value").unwrap_or("").to_string(),
    }
}

/// Drapeaux de `ValidityState` en échec — mêmes règles que le
/// moteur (`session.rs::control_invalid_reason`) : les deux
/// implémentations divergent volontairement (le miroir est
/// autonome), toute différence est une déviation documentée.
fn constraint_failures(d: &Dom, node: NodeId) -> Vec<&'static str> {
    if !will_validate(d, node) {
        return Vec::new();
    }
    let mut bad = Vec::new();
    if d.attr(node, CUSTOM_VALIDITY_ATTR)
        .is_some_and(|m| !m.is_empty())
    {
        bad.push("customError");
    }
    let tag = d.nodes[node as usize].tag.as_str();
    let ty = d.attr(node, "type").unwrap_or("").to_lowercase();
    let value = control_value(d, node);
    if d.has_attr(node, "required") {
        let missing = match (tag, ty.as_str()) {
            ("input", "checkbox") => !d.has_attr(node, "checked"),
            ("input", "radio") => !radio_group_checked(d, node),
            _ => value.is_empty(),
        };
        if missing {
            bad.push("valueMissing");
        }
    }
    let textual = tag == "textarea"
        || matches!(
            ty.as_str(),
            "" | "text" | "search" | "tel" | "url" | "email" | "password"
        );
    if textual && !value.is_empty() {
        let len = value.chars().count();
        if let Some(min) = d
            .attr(node, "minlength")
            .and_then(|m| m.parse::<usize>().ok())
        {
            if len < min {
                bad.push("tooShort");
            }
        }
        if let Some(max) = d
            .attr(node, "maxlength")
            .and_then(|m| m.parse::<usize>().ok())
        {
            if len > max {
                bad.push("tooLong");
            }
        }
        if let Some(pat) = d.attr(node, "pattern") {
            if !pattern_full_match(pat, &value) {
                bad.push("patternMismatch");
            }
        }
    }
    if matches!(ty.as_str(), "number" | "range") {
        if let Ok(v) = value.trim().parse::<f64>() {
            let num = |k: &str| d.attr(node, k).and_then(|s| s.parse::<f64>().ok());
            if num("min").is_some_and(|min| v < min) {
                bad.push("rangeUnderflow");
            }
            if num("max").is_some_and(|max| v > max) {
                bad.push("rangeOverflow");
            }
            if let Some(step) = num("step").filter(|s| *s > 0.0) {
                let base = num("min").unwrap_or(0.0);
                let rem = ((v - base) / step).fract();
                if rem.abs() > 1e-9 && (rem.abs() - 1.0).abs() > 1e-9 {
                    bad.push("stepMismatch");
                }
            }
        }
    }
    bad
}

/// Un radio du même groupe (`name`, même `<form>` ou document)
/// est coché — `required` se satisfait au niveau du groupe.
fn radio_group_checked(d: &Dom, node: NodeId) -> bool {
    let Some(name) = d.attr(node, "name") else {
        return d.has_attr(node, "checked");
    };
    let mut scope = node;
    while let Some(p) = d.parent(scope) {
        if d.nodes[p as usize].tag == "form" {
            scope = p;
            break;
        }
        scope = p;
    }
    d.by_tag(scope, "input").iter().any(|&r| {
        d.attr(r, "type")
            .unwrap_or("")
            .eq_ignore_ascii_case("radio")
            && d.attr(r, "name") == Some(name)
            && d.has_attr(r, "checked")
    })
}

/// `pattern` HTML : correspondance entière via le moteur regexp
/// interne (`^(?:…)$`) — un pattern non compilable ne contraint
/// pas (HTML).
fn pattern_full_match(pat: &str, value: &str) -> bool {
    crate::regexp::JsRegExp::compile(&format!("^(?:{pat})$"), "")
        .is_ok_and(|re| re.exec(value, 0).is_some())
}

/// Texte de `validationMessage` pour le premier drapeau en
/// échec (messages alignés sur les navigateurs, en-US).
fn failure_message(d: &Dom, node: NodeId, flag: &str) -> String {
    let a = |k: &str| d.attr(node, k).unwrap_or("").to_string();
    match flag {
        "customError" => d.attr(node, CUSTOM_VALIDITY_ATTR).unwrap_or("").to_string(),
        "valueMissing" => "Please fill out this field.".to_string(),
        "tooShort" => format!(
            "Please lengthen this text to {} characters or more (you are currently using {} characters).",
            a("minlength"),
            control_value(d, node).chars().count()
        ),
        "tooLong" => format!(
            "Please shorten this text to {} characters or less (you are currently using {} characters).",
            a("maxlength"),
            control_value(d, node).chars().count()
        ),
        "patternMismatch" => "Please match the requested format.".to_string(),
        "rangeUnderflow" => {
            format!("Value must be greater than or equal to {}.", a("min"))
        }
        "rangeOverflow" => format!("Value must be less than or equal to {}.", a("max")),
        "stepMismatch" => "Please enter a valid value.".to_string(),
        _ => String::new(),
    }
}

/// Champs « successful » du `<form>` miroir (mêmes règles que
/// `session.rs::collect_form_data` — nommés, non `disabled`,
/// cases cochées, `submitter` seul pour les boutons). Les
/// `File` portent leur nom de fichier comme valeur (pas de
/// picker — l'embedder émettra un fichier vide, déviation
/// documentée).
fn collect_fields(d: &Dom, form: NodeId, submitter: Option<NodeId>) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut stack = vec![form];
    while let Some(n) = stack.pop() {
        let tag = d.nodes[n as usize].tag.as_str();
        if n != form && d.nodes[n as usize].kind == NodeKind::Element && !d.has_attr(n, "disabled")
        {
            match tag {
                "input" => {
                    let ty = d.attr(n, "type").unwrap_or("").to_lowercase();
                    let ok = match ty.as_str() {
                        "checkbox" | "radio" => d.has_attr(n, "checked"),
                        "submit" | "image" | "button" | "reset" => Some(n) == submitter,
                        _ => true,
                    };
                    if ok {
                        push_named_field(d, n, control_value(d, n), &mut out);
                    }
                }
                "textarea" | "select" => {
                    push_named_field(d, n, control_value(d, n), &mut out);
                }
                "button" => {
                    let ty = d.attr(n, "type").unwrap_or("submit").to_lowercase();
                    if Some(n) == submitter && !matches!(ty.as_str(), "button" | "reset") {
                        push_named_field(
                            d,
                            n,
                            d.attr(n, "value").unwrap_or("").to_string(),
                            &mut out,
                        );
                    }
                }
                _ => {}
            }
        }
        if n == form || tag != "form" {
            let kids = d.child_nodes(n);
            stack.extend(kids.iter().rev());
        }
    }
    out
}

/// Pousse `(name, value)` si le contrôle est nommé.
fn push_named_field(d: &Dom, n: NodeId, value: String, out: &mut Vec<(String, String)>) {
    if let Some(name) = d.attr(n, "name") {
        if !name.is_empty() {
            out.push((name.to_string(), value));
        }
    }
}
