// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! DOM façade — `Host` bindings exposing the mini-`Dom` to scripts:
//! `document`, `Element`, `style`, `classList`, `navigator`.
//!
//! Property coverage targets old-school scripts: `getElementById`,
//! `innerHTML`, `className`, `style`, `value`, `addEventListener`
//! (dispatch réelle = JS-3), `querySelector` à sélecteur simple.

use crate::dom::{
    CanvasOpRecord, Dom, MediaCmd, MediaElState, MediaReport, MediaState, NodeId, NodeKind,
};
use crate::heap::{Host, JsResult, ObjKind};
use crate::value::{ObjId, Value};
use crate::xhr::XhrRequest;
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
    /// `document.implementation` — objet unique par document
    /// (spec), paresseusement construit.
    implementation: RefCell<Option<Value>>,
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

/// Propriété d'événement plateforme portée par l'embedder
/// (étape 103 — `KeyboardEvent.key`/`code`/`keyCode`/`which`,
/// réutilisable `MouseEvent.clientX`…) : la session calcule,
/// le moteur expose tel quel sur l'objet event.
#[derive(Debug, Clone)]
pub enum EventProp {
    /// Chaîne (`key`, `code`…).
    Str(String),
    /// Nombre (`keyCode`, `which`…).
    Num(f64),
    /// Booléen (`ctrlKey`…).
    Bool(bool),
}

/// Mutable DOM `Event` state — shared between the `Event` host
/// wrapper and the dispatcher so `stopPropagation`/`preventDefault`
/// take effect mid-walk.
pub(crate) struct EventState {
    ty: RefCell<String>,
    target: Cell<NodeId>,
    current: Cell<Option<NodeId>>,
    bubbles: Cell<bool>,
    /// `cancelable` (spec `EventInit`) — `preventDefault` est
    /// no-op sans lui.
    cancelable: Cell<bool>,
    /// `composed` (spec `EventInit`) — traversée d'ombres ; sans
    /// shadow DOM, flag purement lisible.
    composed: Cell<bool>,
    /// `isTrusted` (spec) — plateforme (`dom_event*`) vrai,
    /// script (`dispatchEvent`, `el.click()`) faux.
    trusted: bool,
    /// `stopPropagation` — la marche vers les *autres* nœuds
    /// cesse ; les listeners frères du nœud courant finissent
    /// leur passe (spec, contrairement à `immediate`).
    stopped: Cell<bool>,
    /// `stopImmediatePropagation` — tue aussi les listeners
    /// frères du nœud courant. Pose aussi `stopped`.
    immediate: Cell<bool>,
    prevented: Cell<bool>,
    /// `eventPhase` : 0 hors dispatch, 1 capture, 2 at-target,
    /// 3 bubble — posé par `propagate_event`.
    phase: Cell<u8>,
    /// Origine du `timeStamp` — ms depuis la construction de
    /// l'état (Instant, pas epoch : la spec compte depuis la
    /// création de l'event relativement à l'origine temps).
    created: std::time::Instant,
    /// Props plateforme (`key`, `code`, `keyCode`…) — résolues
    /// par l'`EventHost` après les champs du cœur.
    props: Vec<(String, Value)>,
}

/// Flags d'initialisation d'un event (spec `EventInit` +
/// `isTrusted` interne — plateforme vs script).
#[allow(clippy::struct_excessive_bools)] // forme WebIDL `EventInit`.
#[derive(Clone, Copy)]
struct EventFlags {
    bubbles: bool,
    cancelable: bool,
    composed: bool,
    /// `isTrusted` (spec) — plateforme (`dom_event*`) vrai,
    /// script (`dispatchEvent`, `el.click()`) faux.
    trusted: bool,
}

/// État event neuf — `created` marque la « construction » pour
/// `timeStamp` (approximation honnête : l'état naît au dispatch,
/// quasi synchrone de la création JS dans les flux observés).
fn make_event_state(
    ty: &str,
    target: NodeId,
    flags: EventFlags,
    props: Vec<(String, Value)>,
) -> Rc<EventState> {
    Rc::new(EventState {
        ty: RefCell::new(ty.to_string()),
        target: Cell::new(target),
        current: Cell::new(None),
        bubbles: Cell::new(flags.bubbles),
        cancelable: Cell::new(flags.cancelable),
        composed: Cell::new(flags.composed),
        trusted: flags.trusted,
        stopped: Cell::new(false),
        immediate: Cell::new(false),
        prevented: Cell::new(false),
        phase: Cell::new(0),
        created: std::time::Instant::now(),
        props,
    })
}

/// Flags `cancelable`/`composed` par défaut des events
/// plateforme (UI Events) — les interactifs (click, keydown…)
/// sont annulables et traversants ; les états de focus,
/// chargement et défilement ne le sont pas.
fn platform_event_flags(ty: &str) -> (bool, bool) {
    const NON_CANCELABLE: &[&str] = &[
        "focus",
        "blur",
        "focusin",
        "focusout",
        "load",
        "error",
        "resize",
        "scroll",
        "DOMContentLoaded",
        "readystatechange",
        "abort",
    ];
    const NON_COMPOSED: &[&str] = &[
        "focus",
        "blur",
        "focusin",
        "focusout",
        "load",
        "error",
        "resize",
        "scroll",
        "DOMContentLoaded",
        "readystatechange",
        "abort",
    ];
    (!NON_CANCELABLE.contains(&ty), !NON_COMPOSED.contains(&ty))
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
        implementation: RefCell::new(None),
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
            implementation: RefCell::new(None),
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
            ("userAgent", "Explorer/0.1 (explorer-jscore)"),
            ("appName", "Netscape"),
            ("platform", "Win32"),
            ("language", "en-US"),
            ("appVersion", "5.0"),
        ] {
            let s = self.str_val(v);
            self.set_own(nav, k, s, true);
        }
        // `navigator.languages` — tableau gelé dérivé de
        // `language` (spec, low-fingerprint). Sans lui,
        // `navigator.languages.some(...)` des scripts réels
        // jette `TypeError` (triage example.com, journal
        // PIPE→NAV 2026-10-21) — le script meurt avant ses
        // insertions suivantes.
        let en = self.str_val("en-US");
        let en2 = self.str_val("en");
        let langs = self.new_array(vec![en, en2]);
        let _ = crate::builtins::object_freeze(self, Value::Undefined, &[langs]);
        self.set_own(nav, "languages", langs, true);
        // `navigator.sendBeacon(url, data?)` — POST fire-and-forget
        // via le transport XHR (staple de télémétrie des pages
        // réelles ; `false` sans connecteur, comme la spec).
        self.add_method(nav, "sendBeacon", nav_send_beacon);
        self.set_own(g, "navigator", Value::Obj(nav), false);

        // `Worker` borné (JS-C29) — le constructeur est présent
        // pour la détection de feature ; sans `set_worker_factory`
        // il jette un TypeError honnête.
        self.install_worker_ctor();
        // `window`/`self`/`globalThis` — the global object itself.
        let gv = Value::Obj(g);
        self.set_own(g, "window", gv, false);
        self.set_own(g, "self", gv, false);
        self.set_own(g, "globalThis", gv, false);
        // Pas de frames → `top`/`parent`/`frames` ≙ la fenêtre elle-même.
        self.set_own(g, "top", gv, false);
        self.set_own(g, "parent", gv, false);
        self.set_own(g, "frames", gv, false);
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
        // `window.addEventListener`/`removeEventListener`/
        // `dispatchEvent` — `window` ≙ sommet de propagation : les
        // listeners vivent sur le nœud racine, comme `document`.
        self.add_method(g, "addEventListener", win_add_event_listener);
        self.add_method(g, "removeEventListener", win_remove_event_listener);
        self.add_method(g, "dispatchEvent", win_dispatch_event);
        // `getComputedStyle(el)` — pas de cascade CSS réelle : rend
        // le `StyleHost` de l'élément (valeurs inline uniquement).
        self.add_method(g, "getComputedStyle", win_get_computed_style);
        // `matchMedia(q)` — pas de moteur de media queries :
        // `MediaQueryList` honnête `matches:false`, listeners no-op.
        self.add_method(g, "matchMedia", win_match_media);
        // `CSS` namespace — pas de moteur CSS : `supports` → false,
        // `registerProperty` no-op, `escape` = algorithme spec.
        let css = self.new_object();
        self.add_method(css, "supports", css_supports);
        self.add_method(css, "registerProperty", mql_noop);
        self.add_method(css, "escape", css_escape);
        self.set_own(g, "CSS", Value::Obj(css), false);
        // `Node` — constructeur (inconstructible) portant les
        // constantes `*_NODE` / `DOCUMENT_POSITION_*` ; `prototype`
        // existe pour que `instanceof` reste sûr.
        let node_f = self.native_fn("Node", node_ctor);
        if let Value::Obj(nid) = node_f {
            let p = self.new_object();
            self.set_own(nid, "prototype", Value::Obj(p), false);
            for (k, v) in [
                ("ELEMENT_NODE", 1),
                ("ATTRIBUTE_NODE", 2),
                ("TEXT_NODE", 3),
                ("CDATA_SECTION_NODE", 4),
                ("ENTITY_REFERENCE_NODE", 5),
                ("ENTITY_NODE", 6),
                ("PROCESSING_INSTRUCTION_NODE", 7),
                ("COMMENT_NODE", 8),
                ("DOCUMENT_NODE", 9),
                ("DOCUMENT_TYPE_NODE", 10),
                ("DOCUMENT_FRAGMENT_NODE", 11),
                ("NOTATION_NODE", 12),
                ("DOCUMENT_POSITION_DISCONNECTED", 1),
                ("DOCUMENT_POSITION_PRECEDING", 2),
                ("DOCUMENT_POSITION_FOLLOWING", 4),
                ("DOCUMENT_POSITION_CONTAINS", 8),
                ("DOCUMENT_POSITION_CONTAINED_BY", 16),
                ("DOCUMENT_POSITION_IMPLEMENTATION_SPECIFIC", 32),
            ] {
                self.set_own(nid, k, Value::Num(f64::from(v)), false);
            }
        }
        self.set_own(g, "Node", node_f, false);
        // Interfaces DOM standard — constructeurs-stubs pour la
        // détection (`typeof Element`, `"x" in Element.prototype`,
        // `instanceof`), inconstructibles comme `Node`. Mesuré sur
        // github.com : `HTMLElement`/`Event` manquants →
        // ReferenceError (JS-C56).
        for name in [
            "EventTarget",
            "Element",
            "HTMLElement",
            "HTMLImageElement",
            // Famille `HTML*Element` au complet (mesuré github.com
            // vol 22/10 : `ReferenceError: HTMLButtonElement` dans
            // `t2-*.js` — les stubs évitent le whack-a-mole au
            // prochain membre demandé par le site suivant).
            "HTMLAnchorElement",
            "HTMLAreaElement",
            "HTMLAudioElement",
            "HTMLBaseElement",
            "HTMLBodyElement",
            "HTMLBRElement",
            "HTMLButtonElement",
            "HTMLCanvasElement",
            "HTMLDataElement",
            "HTMLDataListElement",
            "HTMLDetailsElement",
            "HTMLDialogElement",
            "HTMLDirectoryElement",
            "HTMLDivElement",
            "HTMLDListElement",
            "HTMLEmbedElement",
            "HTMLFieldSetElement",
            "HTMLFontElement",
            "HTMLFormElement",
            "HTMLFrameElement",
            "HTMLFrameSetElement",
            "HTMLHeadElement",
            "HTMLHeadingElement",
            "HTMLHRElement",
            "HTMLHtmlElement",
            "HTMLIFrameElement",
            "HTMLInputElement",
            "HTMLLabelElement",
            "HTMLLegendElement",
            "HTMLLIElement",
            "HTMLLinkElement",
            "HTMLMapElement",
            "HTMLMarqueeElement",
            "HTMLMediaElement",
            "HTMLMenuElement",
            "HTMLMetaElement",
            "HTMLMeterElement",
            "HTMLModElement",
            "HTMLObjectElement",
            "HTMLOListElement",
            "HTMLOptGroupElement",
            "HTMLOptionElement",
            "HTMLOutputElement",
            "HTMLParagraphElement",
            "HTMLParamElement",
            "HTMLPictureElement",
            "HTMLPreElement",
            "HTMLProgressElement",
            "HTMLQuoteElement",
            "HTMLScriptElement",
            "HTMLSelectElement",
            "HTMLSlotElement",
            "HTMLSourceElement",
            "HTMLSpanElement",
            "HTMLStyleElement",
            "HTMLTableCaptionElement",
            "HTMLTableCellElement",
            "HTMLTableColElement",
            "HTMLTableElement",
            "HTMLTableRowElement",
            "HTMLTableSectionElement",
            "HTMLTemplateElement",
            "HTMLTextAreaElement",
            "HTMLTimeElement",
            "HTMLTitleElement",
            "HTMLTrackElement",
            "HTMLUListElement",
            "HTMLVideoElement",
            "SVGElement",
            "Document",
            "DocumentFragment",
            "ShadowRoot",
            "CharacterData",
            "Text",
            "Comment",
            "Attr",
            "CDATASection",
            "ProcessingInstruction",
            "DocumentType",
            "DOMTokenList",
            "NodeList",
            "HTMLCollection",
            "NamedNodeMap",
            "DOMRect",
            "DOMRectReadOnly",
            "CSSStyleDeclaration",
            "CSSRule",
            "StyleSheet",
            "CSSStyleSheet",
            "Range",
            "DOMParser",
            "XMLSerializer",
            "AbortSignal",
            "DOMException",
            // dev.to : `instanceOf(Window)` (prop-types react-dates)
            // — détection seule, jamais construit.
            "Window",
        ] {
            let f = self.native_fn(name, illegal_ctor);
            if let Value::Obj(fid) = f {
                let p = self.new_object();
                self.set_own(fid, "prototype", Value::Obj(p), false);
            }
            self.set_own(g, name, f, false);
        }
        // `Element.prototype` porte `matches`/`closest` en
        // *valeurs* : les shims lisent la référence de méthode
        // (`o = Element.prototype.matches || i.msMatchesSelector`
        // — selector-observer github.com) puis l'appellent via
        // `.call(el,…)` ; les méthodes host invoke-only sont
        // invisibles à la lecture. Les alias pré-standards
        // `*MatchesSelector` complètent la détection.
        if let Some(Value::Obj(cid)) = self.get_prop(g, "Element") {
            if let Some(Value::Obj(pid)) = self.get_prop(cid, "prototype") {
                for (k, f2) in [
                    ("matches", element_matches as crate::heap::NativeFn),
                    ("matchesSelector", element_matches),
                    ("webkitMatchesSelector", element_matches),
                    ("mozMatchesSelector", element_matches),
                    ("oMatchesSelector", element_matches),
                    ("msMatchesSelector", element_matches),
                    ("closest", element_closest),
                ] {
                    let m = self.native_fn(k, f2);
                    self.set_own(pid, k, m, true);
                }
            }
        }
        // `document` hérite de `Document.prototype` — les sites
        // monkey-patchent le proto DOM (mesuré dev.to :
        // `Document.prototype.ready = new Promise(…)` puis
        // `document.ready.then`); l'héritage rend aussi
        // `document instanceof Document` vrai, comme la spec.
        if let (Value::Obj(did), Some(Value::Obj(cid))) = (doc, self.get_prop(g, "Document")) {
            if let Some(proto @ Value::Obj(_)) = self.get_prop(cid, "prototype") {
                if let Some(o) = self.heap.obj_mut(did) {
                    o.proto = proto;
                }
            }
        }
        // `Event` + famille — constructibles : l'objet produit est
        // dispatchable (`type`/`bubbles` lus par `dispatchEvent`).
        // Les proto des sous-classes héritent de `Event.prototype`.
        let event_f = self.native_fn("Event", event_ctor);
        let mut event_proto = Value::Undefined;
        if let Value::Obj(fid) = event_f {
            let p = Value::Obj(self.new_object());
            for (k, f2) in [
                (
                    "preventDefault",
                    event_prevent_default as crate::heap::NativeFn,
                ),
                ("stopPropagation", event_stop_propagation),
                ("stopImmediatePropagation", event_stop_propagation),
                ("composedPath", event_composed_path),
                ("initEvent", event_init),
            ] {
                let m = self.native_fn(k, f2);
                if let Value::Obj(pid) = p {
                    self.set_own(pid, k, m, true);
                }
            }
            for (k, v) in [
                ("NONE", 0),
                ("CAPTURING_PHASE", 1),
                ("AT_TARGET", 2),
                ("BUBBLING_PHASE", 3),
            ] {
                self.set_own(fid, k, Value::Num(f64::from(v)), false);
                if let Value::Obj(pid) = p {
                    self.set_own(pid, k, Value::Num(f64::from(v)), false);
                }
            }
            self.set_own(fid, "prototype", p, false);
            event_proto = p;
        }
        self.set_own(g, "Event", event_f, false);
        for name in [
            "CustomEvent",
            "UIEvent",
            "MouseEvent",
            "KeyboardEvent",
            "PointerEvent",
            "FocusEvent",
            "InputEvent",
            "WheelEvent",
            "TouchEvent",
            "DragEvent",
            "SubmitEvent",
            "ErrorEvent",
            "CompositionEvent",
            "AnimationEvent",
            "TransitionEvent",
            "ClipboardEvent",
            "HashChangeEvent",
            "PopStateEvent",
            "PromiseRejectionEvent",
        ] {
            let f = self.native_fn(name, event_ctor);
            if let Value::Obj(fid) = f {
                let p = Value::Obj(self.new_object());
                if let Value::Obj(pid) = p {
                    if let Some(o) = self.heap.obj_mut(pid) {
                        o.proto = event_proto;
                    }
                }
                self.set_own(fid, "prototype", p, false);
            }
            self.set_own(g, name, f, false);
        }
        // `Image` — constructible (alias `createElement('img')` ;
        // `HTMLImageElement` reste le stub de détection au-dessus).
        // Mesuré wikipedia.org vol 22/10 (JS-C57).
        let img_f = self.native_fn("Image", image_ctor);
        if let Value::Obj(fid) = img_f {
            let p = self.new_object();
            self.set_own(fid, "prototype", Value::Obj(p), false);
        }
        self.set_own(g, "Image", img_f, false);
        // `AbortController` minimal : `signal.aborted` + `abort()`.
        let ac_f = self.native_fn("AbortController", abort_controller_ctor);
        if let Value::Obj(fid) = ac_f {
            let p = self.new_object();
            let m = self.native_fn("abort", abort_signal_abort);
            self.set_own(p, "abort", m, true);
            self.set_own(fid, "prototype", Value::Obj(p), false);
        }
        self.set_own(g, "AbortController", ac_f, false);
        // `IntersectionObserver`/`ResizeObserver` — constructibles,
        // callback requis ; `observe`/`unobserve`/`disconnect`
        // no-op honnêtes (le miroir ne génère ni intersection ni
        // resize — le callback n'est jamais appelé). Mesuré
        // github.com : `new IntersectionObserver(cb)` (JS-C56).
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
            self.set_own(g, name, fv, false);
        }
        // `customElements` — registre honnête : `define`/`get`/
        // `whenDefined`/`upgrade`. Mesuré github.com (JS-C56).
        let ce = self.new_host_object(Rc::new(CustomElementsHost {
            registry: RefCell::new(HashMap::new()),
        }));
        self.set_own(g, "customElements", ce, false);
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
        // Pas de scroll réel — la page tient dans le viewport.
        for k in ["scrollX", "scrollY", "pageXOffset", "pageYOffset"] {
            self.set_own(g, k, Value::Num(0.0), false);
        }
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
        // JS-C35 : ces événements marquent la fin du chargement —
        // le document se ferme avant que les listeners ne courent,
        // un `document.write` dedans rouvrira donc en effaçant la
        // page (spec `document open steps`).
        if matches!(ty, "DOMContentLoaded" | "load") {
            dom.borrow_mut().doc_open = false;
        }
        let target = dom.borrow().body();
        self.dom_event_on_props(ty, target, &[])
    }

    /// [`Self::dom_event`] à props (étape 103) — `keydown`/`keyup`
    /// portent `key`/`code`/`keyCode`/`which` calculés par
    /// l'embedder.
    ///
    /// # Errors
    /// Returns the first thrown exception value (listener error).
    pub fn dom_event_props(&mut self, ty: &str, props: &[(String, EventProp)]) -> JsResult<bool> {
        let Some((dom, _)) = self.dom_ctx.clone() else {
            return Ok(false);
        };
        let target = dom.borrow().body();
        self.dom_event_on_props(ty, target, props)
    }

    /// Element-targeted platform event (JS-5c): dispatches `ty` on
    /// `node` with full capture → target → bubble propagation and
    /// returns `defaultPrevented` — the embedder uses it to map a
    /// widget-level hit (link click…) back to the mirror node.
    ///
    /// # Errors
    /// Returns the first thrown exception value (listener error).
    pub fn dom_event_on(&mut self, ty: &str, node: NodeId) -> JsResult<bool> {
        self.dom_event_on_props(ty, node, &[])
    }

    /// [`Self::dom_event_on`] à props (étape 103) — les entrées
    /// `props` deviennent des propriétés de l'objet event
    /// (`KeyboardEvent.key`/`code`/`keyCode`/`which`…), résolues
    /// par l'`EventHost` après les champs du cœur.
    ///
    /// # Errors
    /// Returns the first thrown exception value (listener error).
    pub fn dom_event_on_props(
        &mut self,
        ty: &str,
        node: NodeId,
        props: &[(String, EventProp)],
    ) -> JsResult<bool> {
        let Some((dom, cache)) = self.dom_ctx.clone() else {
            return Ok(false);
        };
        let props: Vec<(String, Value)> = props
            .iter()
            .map(|(k, v)| {
                let v = match v {
                    EventProp::Str(s) => self.str_val(s.clone()),
                    EventProp::Num(n) => Value::Num(*n),
                    EventProp::Bool(b) => Value::Bool(*b),
                };
                (k.clone(), v)
            })
            .collect();
        let (cancelable, composed) = platform_event_flags(ty);
        let flags = EventFlags {
            bubbles: true,
            cancelable,
            composed,
            trusted: true,
        };
        let state = make_event_state(ty, node, flags, props);
        self.propagate_event(&dom, &cache, &state)?;
        // Event dispatch is a job boundary — drain queued reactions.
        self.run_microtasks()?;
        Ok(state.prevented.get())
    }

    /// `set_media_claim` (étape 71c, ADR-0024) : l'embedder
    /// marque l'élément `node` claimé par l'hôte
    /// (`Some(codecs)`, id de surface `media`) ou releasé
    /// (`None` — retour aux stubs honnêtes ADR-0023, promesses
    /// `play()` pendantes rejetées `NotSupportedError`). Un
    /// claim sur élément déjà claimé met à jour id + codecs
    /// (ré-offre = id neuf, promesses conservées : l'élément
    /// persiste).
    pub fn set_media_claim(&mut self, node: NodeId, media: u64, codecs: Option<Vec<String>>) {
        let Some((dom, _)) = self.dom_ctx.clone() else {
            return;
        };
        let pending = {
            let mut d = dom.borrow_mut();
            match codecs {
                Some(c) => {
                    match d.media.get_mut(&node) {
                        Some(st) => {
                            st.media = media;
                            st.codecs = c;
                        }
                        None => {
                            d.media.insert(node, MediaElState::new(media, c));
                        }
                    }
                    return;
                }
                None => d.media.remove(&node).map(|st| st.pending_play),
            }
        };
        if let Some(pids) = pending {
            let msg = self.str_val("NotSupportedError: no supported media");
            for p in pids {
                self.reject_promise(p, msg);
            }
            let _ = self.run_microtasks();
        }
    }

    /// `deliver_media_state` (étape 71c) : l'embedder relaie le
    /// rapport `state` de l'hôte claimé. Les props reflètent le
    /// rapport, les promesses `play()` tranchent (`playing` →
    /// résolution, `error` → rejet) et les events DOM
    /// **non-bubblants** suivent les transitions :
    /// `loadedmetadata` (premier rapport), `durationchange`,
    /// `seeked` (seek en vol + tête déplacée), `play`+`playing`,
    /// `pause`, `ended`, `error`, `timeupdate` (tête déplacée).
    /// Rapport sur élément non claimé → ignoré (l'offre morte
    /// ne réveille pas de surface).
    ///
    /// # Errors
    /// Propage la première exception de listener DOM.
    pub fn deliver_media_state(&mut self, node: NodeId, rep: &MediaReport) -> JsResult<()> {
        let Some((dom, cache)) = self.dom_ctx.clone() else {
            return Ok(());
        };
        let mut events: Vec<&'static str> = Vec::new();
        let mut pids = Vec::new();
        let mut err_msg = String::new();
        {
            let mut d = dom.borrow_mut();
            let Some(st) = d.media.get_mut(&node) else {
                return Ok(());
            };
            let prev = st.report.replace(rep.clone());
            let prev_state = prev.as_ref().map(|r| r.state);
            let prev_pos = prev.map(|r| r.position_ms);
            if !st.metadata_fired {
                st.metadata_fired = true;
                events.push("loadedmetadata");
            }
            if st.last_duration != rep.duration_ms {
                st.last_duration = rep.duration_ms;
                events.push("durationchange");
            }
            if st.seek_pending && prev_pos != Some(rep.position_ms) {
                st.seek_pending = false;
                events.push("seeked");
            }
            match rep.state {
                MediaState::Playing if prev_state != Some(MediaState::Playing) => {
                    events.push("play");
                    events.push("playing");
                    pids = std::mem::take(&mut st.pending_play);
                }
                MediaState::Paused if prev_state != Some(MediaState::Paused) => {
                    events.push("pause");
                }
                MediaState::Ended if prev_state != Some(MediaState::Ended) => {
                    events.push("ended");
                }
                MediaState::Error if prev_state != Some(MediaState::Error) => {
                    events.push("error");
                    err_msg = rep
                        .error
                        .clone()
                        .unwrap_or_else(|| "MediaError".to_string());
                    pids = std::mem::take(&mut st.pending_play);
                }
                _ => {}
            }
            if prev_pos != Some(rep.position_ms) {
                events.push("timeupdate");
            }
        }
        if rep.state == MediaState::Error {
            let msg = self.str_val(&err_msg);
            for p in pids {
                self.reject_promise(p, msg);
            }
        } else {
            for p in pids {
                self.resolve_promise(p, Value::Undefined);
            }
        }
        for ty in events {
            self.dispatch_media(&dom, &cache, node, ty)?;
        }
        self.run_microtasks()?;
        Ok(())
    }

    /// Event média **non-bubblant** (spec `HTMLMediaElement` :
    /// `play`/`pause`/`ended`/`timeupdate`… ne remontent pas).
    /// La phase de capture reste spec (ancêtres en capture).
    pub(crate) fn dispatch_media(
        &mut self,
        dom: &DomRef,
        cache: &NodeCache,
        node: NodeId,
        ty: &str,
    ) -> JsResult<()> {
        // Media : non-bubblant, non-annulable, non-traversant ;
        // déclenché par le pipeline de lecture → `trusted`.
        let state = make_event_state(
            ty,
            node,
            EventFlags {
                bubbles: false,
                cancelable: false,
                composed: false,
                trusted: true,
            },
            Vec::new(),
        );
        self.propagate_event(dom, cache, &state)
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
        // `el.click()` & co sont script-initiés → `trusted:false`
        // (spec `isTrusted`), flags par type via
        // `platform_event_flags`.
        let (cancelable, composed) = platform_event_flags(ty);
        let state = make_event_state(
            ty,
            node,
            EventFlags {
                bubbles: true,
                cancelable,
                composed,
                trusted: false,
            },
            Vec::new(),
        );
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
        ev.phase.set(1);
        for &n in path.iter().rev() {
            if ev.stopped.get() {
                break;
            }
            ev.current.set(Some(n));
            self.fire_listeners(dom, cache, n, ev, &ev_val, true)?;
        }
        // At target: capture listeners, then bubble listeners/on*.
        // `stopped` posé en capture court-circuite la cible (spec) ;
        // à la cible seul `immediate` tue la seconde passe — les
        // listeners de la cible forment un seul groupe (spec).
        if !ev.stopped.get() {
            ev.phase.set(2);
            ev.current.set(Some(target));
            self.fire_listeners(dom, cache, target, ev, &ev_val, true)?;
            if !ev.immediate.get() {
                self.fire_listeners(dom, cache, target, ev, &ev_val, false)?;
            }
        }
        // Bubble: parent → root.
        if ev.bubbles.get() && !ev.stopped.get() {
            ev.phase.set(3);
            for &n in &path {
                if ev.stopped.get() {
                    break;
                }
                ev.current.set(Some(n));
                self.fire_listeners(dom, cache, n, ev, &ev_val, false)?;
            }
        }
        // Spec : hors dispatch, `currentTarget`/`eventPhase` → 0 —
        // et `composedPath` redevient `[]`.
        ev.phase.set(0);
        ev.current.set(None);
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
            // Spec : `stopPropagation` laisse finir les listeners
            // frères du nœud ; seul `immediate` les tue.
            if ev.immediate.get() {
                break;
            }
            if self.is_callable(f) {
                self.call_value(f, this, &[*ev_val])?;
            }
        }
        if !capture && !ev.immediate.get() {
            let key = format!("on{ty}");
            // Prop scriptée (`el.onclick = f`) prioritaire —
            // dernier écrivain gagne, l'attribut est "écrit"
            // au parse ; sinon handler d'attribut compilé.
            let prop = if let Value::Obj(id) = this {
                self.get_prop(id, &key)
            } else {
                None
            };
            let f = match prop {
                Some(f) if self.is_callable(f) => Some(f),
                _ => self.inline_handler(dom, node, &key)?,
            };
            if let Some(f) = f {
                // Spec : un handler `on*` qui retourne `false`
                // annule l'événement (≙ `preventDefault` — pattern
                // `onsubmit="return check()"`).
                if matches!(self.call_value(f, this, &[*ev_val])?, Value::Bool(false)) {
                    ev.prevented.set(true);
                }
            }
        }
        Ok(())
    }

    /// Handler d'attribut `on<ty>` (JS-C45) — la spec compile
    /// `onclick="src"` en `function(event){with(document){
    /// with(this.form||{}){with(this){ src }}}}` (accès nu aux
    /// props de l'élément, de son formulaire, du document).
    /// Compilé **paresseusement** au premier dispatch et mis
    /// en cache sur le nœud `(attr, src, fn)` — un
    /// `setAttribute`/`removeAttribute` qui change la source
    /// invalide par comparaison au prochain tir. Une erreur de
    /// compilation remonte comme une erreur de listener
    /// (premier throw gagne — même sémantique).
    fn inline_handler(&mut self, dom: &DomRef, node: NodeId, key: &str) -> JsResult<Option<Value>> {
        let src = {
            let d = dom.borrow();
            let Some(n) = d.nodes.get(node as usize) else {
                return Ok(None);
            };
            if n.kind != NodeKind::Element {
                return Ok(None);
            }
            match d.attr(node, key) {
                Some(s) => s.to_string(),
                None => return Ok(None),
            }
        };
        {
            let d = dom.borrow();
            if let Some(n) = d.nodes.get(node as usize) {
                if let Some((_, s, f)) = n.inline_handlers.iter().find(|(k, ..)| k == key) {
                    if s == &src {
                        return Ok(Some(*f));
                    }
                }
            }
        }
        let wrapped = format!(
            "(function(event){{with(document||{{}}){{with(this.form||{{}}){{with(this){{{src}\n}}}}}}}})",
        );
        let f = self.eval_source(&wrapped)?;
        if self.is_callable(f) {
            let mut d = dom.borrow_mut();
            if let Some(n) = d.nodes.get_mut(node as usize) {
                n.inline_handlers.retain(|(k, ..)| k != key);
                n.inline_handlers.push((key.to_string(), src, f));
            }
            Ok(Some(f))
        } else {
            Ok(None)
        }
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
            // `document.defaultView` — la fenêtre du document ≙ le
            // global (single-window model).
            "defaultView" => Ok(Some(Value::Obj(e.global))),
            // `document.location` ≙ `window.location` — même
            // objet hôte (trigger réel : scripts lisant
            // `document.location.search`, census arstechnica).
            "location" => Ok(e.get_prop(e.global, "location")),
            // `document.implementation` — unique par document
            // (spec) ; `createHTMLDocument` sert le DOM de
            // parsing détaché de jQuery.
            "implementation" => {
                let v = *self
                    .implementation
                    .borrow_mut()
                    .get_or_insert_with(|| e.new_host_object(Rc::new(DomImplementationHost)));
                Ok(Some(v))
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
            // `document.fonts` — FontFaceSet stub : pas de pipeline
            // de fontes web, `load()`/`ready` résolvent toujours.
            "fonts" => Ok(Some(e.new_host_object(Rc::new(FontFaceSetHost)))),
            // Page affichée = visible/active — pas d'onglet caché
            // dans ce modèle.
            "visibilityState" => Ok(Some(e.str_val("visible"))),
            "hidden" => Ok(Some(Value::Bool(false))),
            // `document.nodeType` — 9 (DOCUMENT_NODE), constant.
            "nodeType" => Ok(Some(Value::Num(9.0))),
            // `document.nodeName`/`tagName` — `#document` / absent.
            "nodeName" => Ok(Some(e.str_val("#document"))),
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
            // Fenêtre unique affichée → focus constant.
            "hasFocus" => Ok(Some(Value::Bool(true))),
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
            // `createElementNS(ns, qualified)` — pas de
            // namespaces dans le miroir : l'URI est acceptée et
            // ignorée, le nœud est créé par son nom local
            // (mesuré github.com `z8-*.js` — svg inline).
            "createElementNS" => {
                let tag = e.to_js_string(arg(args, 1))?;
                let local = tag.rsplit(':').next().unwrap_or(&tag).to_string();
                let n = self.dom.borrow_mut().create_element(&local);
                Ok(Some(node_value(e, &self.dom, &self.cache, n)))
            }
            "createTextNode" => {
                let t = e.to_js_string(arg(args, 0))?;
                let n = self.dom.borrow_mut().create_text(&t);
                Ok(Some(node_value(e, &self.dom, &self.cache, n)))
            }
            // `document.createDocumentFragment()` — nœud
            // `Fragment` détaché ; son insertion dans un parent
            // éclate ses enfants (spec — jQuery l'exige).
            "createDocumentFragment" => {
                let n = self.dom.borrow_mut().create_fragment();
                Ok(Some(node_value(e, &self.dom, &self.cache, n)))
            }
            // `document.createEvent('HTMLEvents')` + `initEvent` —
            // the old-school event constructor path.
            "createEvent" => {
                let st = make_event_state(
                    "",
                    0,
                    EventFlags {
                        bubbles: false,
                        cancelable: false,
                        composed: false,
                        trusted: false,
                    },
                    Vec::new(),
                );
                Ok(Some(e.new_host_object(Rc::new(EventHost {
                    dom: self.dom.clone(),
                    cache: self.cache.clone(),
                    state: st,
                }))))
            }
            // `document.open()` — spec `document open steps` :
            // le contenu est effacé et le document redevient
            // « ouvert » aux `write` suivants. Valeur de retour
            // spec : `undefined` (V8 rend le document — le nôtre
            // reste simple).
            "open" => {
                let mut d = self.dom.borrow_mut();
                let root = d.root;
                d.replace_children(root, &[]);
                d.doc_open = true;
                Ok(Some(Value::Undefined))
            }
            // `document.close()` — ferme le flux d'écriture ; un
            // `write` ultérieur rouvrira implicitement (effacement
            // du contenu — spec).
            "close" => {
                self.dom.borrow_mut().doc_open = false;
                Ok(Some(Value::Undefined))
            }
            // `document.write(…)`/`writeln(…)` — variadique (tous
            // les arguments concaténés, spec), `writeln` ajoute le
            // `\n`. Pendant le chargement (`doc_open`) le fragment
            // s'ajoute à `<body>` — l'insertion au point du parseur
            // relève de l'amont ; sur document fermé l'`open()`
            // implicite efface d'abord le contenu (spec) — le
            // `document.write` post-`load` qui remplace la page.
            // Les `<script>` écrits restent inertes dans le miroir :
            // l'ordonnancement est celui de l'étape 50 (la
            // re-sérialisation les fait découvrir côté hôte).
            "write" | "writeln" => {
                let mut s = String::new();
                for a in args {
                    s.push_str(&e.to_js_string(*a)?);
                }
                if name == "writeln" {
                    s.push('\n');
                }
                let mut d = self.dom.borrow_mut();
                if !d.doc_open {
                    let root = d.root;
                    d.replace_children(root, &[]);
                    d.doc_open = true;
                }
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
    // `HTMLMediaElement` (étape 58) — attributs réfléchis
    // de même nom sur `<video>`/`<audio>`.
    "poster",
    "preload",
];

/// Boolean attributes mirrored as `bool` props (`checked`…).
const BOOL_ATTRS: &[&str] = &[
    "checked", "disabled", "selected", "readonly", "multiple",
    // `HTMLMediaElement` (étape 58).
    "controls", "muted", "loop", "autoplay",
];

/// `className` ↔ `class`, `htmlFor` ↔ `for`, `tagName` (ro)…
fn prop_to_attr(key: &str) -> Option<&'static str> {
    Some(match key {
        "className" => "class",
        "htmlFor" => "for",
        "value" => "value",
        "crossOrigin" => "crossorigin",
        "playsInline" => "playsinline",
        k => return MAPPED_ATTRS.iter().copied().find(|&a| a == k),
    })
}

/// `true` si `node` est un élément média
/// (`<video>`/`<audio>`) — gate des stubs
/// `HTMLMediaElement` (étape 58, ADR-0023 : aucune
/// donnée n'est jamais décodée, les réponses restent
/// honnêtes — `canPlayType` avoue `""`, `play()` rejette).
fn is_media(dom: &DomRef, node: NodeId) -> bool {
    matches!(
        dom.borrow().nodes[node as usize].tag.as_str(),
        "video" | "audio"
    )
}

/// `TimeRanges` vide : `length` 0, `start`/`end` lèvent
/// (hors-bornes — spec `INDEX_SIZE_ERR`) plutôt que mentir.
fn tr_oob(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    Err(e.type_error("TimeRanges: index out of bounds"))
}

fn time_ranges_empty(e: &mut Engine) -> Value {
    let o = e.new_object();
    e.set_own(o, "length", Value::Num(0.0), true);
    e.add_method(o, "start", tr_oob);
    e.add_method(o, "end", tr_oob);
    Value::Obj(o)
}

/// `TimeRanges` rempli (étape 71c) — `length` + `start(i)`/
/// `end(i)` en secondes, `INDEX_SIZE_ERR` hors-bornes (spec).
struct TimeRangesHost {
    spans: Vec<(f64, f64)>,
}

impl Host for TimeRangesHost {
    fn class_name(&self) -> &'static str {
        "TimeRanges"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn get(&self, _e: &mut Engine, _this: Value, key: &str) -> JsResult<Option<Value>> {
        Ok(match key {
            "length" => Some(Value::Num(self.spans.len() as f64)),
            _ => None,
        })
    }
    fn invoke(
        &self,
        e: &mut Engine,
        _this: Value,
        name: &str,
        args: &[Value],
    ) -> JsResult<Option<Value>> {
        let i = match args.first() {
            Some(Value::Num(n)) if *n >= 0.0 => *n as usize,
            _ => usize::MAX,
        };
        Ok(match name {
            "start" | "end" => match self.spans.get(i) {
                Some(&(a, b)) => Some(Value::Num(if name == "start" { a } else { b })),
                None => return Err(e.type_error("TimeRanges: index out of bounds")),
            },
            _ => None,
        })
    }
}

/// `TimeRanges` de `spans` (secondes) — vide → objet léger,
/// non vide → host object `TimeRanges`.
fn time_ranges(e: &mut Engine, spans: Vec<(f64, f64)>) -> Value {
    if spans.is_empty() {
        return time_ranges_empty(e);
    }
    e.new_host_object(Rc::new(TimeRangesHost { spans }))
}

/// Prop `HTMLMediaElement` (étape 58/71c) — **non claimé** :
/// stubs honnêtes ADR-0023 ; **claimé** : reflet du dernier
/// rapport `state` hôte et des réglages locaux (ADR-0024).
fn media_prop(e: &mut Engine, dom: &DomRef, node: NodeId, key: &str) -> Value {
    let d = dom.borrow();
    let Some(st) = d.media.get(&node) else {
        return match key {
            "paused" | "preservesPitch" => Value::Bool(true),
            "ended" | "seeking" | "disableRemotePlayback" => Value::Bool(false),
            "volume" | "playbackRate" | "defaultPlaybackRate" => Value::Num(1.0),
            "currentTime" | "videoWidth" | "videoHeight" | "readyState" => Value::Num(0.0),
            "duration" => Value::Num(f64::NAN),
            // `src` présent mais indécodable → NO_SOURCE ;
            // sinon EMPTY.
            "networkState" => Value::Num(if d.has_attr(node, "src") { 3.0 } else { 0.0 }),
            "currentSrc" => e.str_val(""),
            "buffered" | "played" | "seekable" => time_ranges_empty(e),
            // `error`, `mediaGroup`, `controller`.
            _ => Value::Null,
        };
    };
    let rep = st.report.as_ref();
    let state = rep.map(|r| r.state);
    match key {
        "paused" => Value::Bool(state != Some(MediaState::Playing)),
        "ended" => Value::Bool(state == Some(MediaState::Ended)),
        "seeking" => Value::Bool(st.seek_pending),
        "preservesPitch" => Value::Bool(true),
        "disableRemotePlayback" => Value::Bool(false),
        "volume" => Value::Num(st.volume),
        "playbackRate" | "defaultPlaybackRate" => Value::Num(st.rate),
        "currentTime" => Value::Num(rep.map_or(0.0, |r| r.position_ms as f64 / 1000.0)),
        "duration" => Value::Num(
            rep.and_then(|r| r.duration_ms)
                .map_or(f64::NAN, |d| d as f64 / 1000.0),
        ),
        // Rapport présent = métadonnées + données reçues →
        // HAVE_ENOUGH_DATA (4) ; claimé sans rapport → rien (0).
        "readyState" => Value::Num(if rep.is_some() { 4.0 } else { 0.0 }),
        // Claimé en attente du premier `state` → LOADING (2) ;
        // rapport présent → IDLE (1). Claimé ≠ NO_SOURCE.
        "networkState" => Value::Num(if rep.is_some() { 1.0 } else { 2.0 }),
        "currentSrc" => e.str_val(d.attr(node, "src").unwrap_or("")),
        "buffered" => time_ranges(
            e,
            rep.map_or_else(Vec::new, |r| vec![(0.0, r.buffered_ms as f64 / 1000.0)]),
        ),
        "played" => time_ranges(
            e,
            rep.map_or_else(Vec::new, |r| vec![(0.0, r.position_ms as f64 / 1000.0)]),
        ),
        "seekable" => time_ranges(
            e,
            rep.and_then(|r| r.duration_ms)
                .map_or_else(Vec::new, |d| vec![(0.0, d as f64 / 1000.0)]),
        ),
        "error" => match rep.filter(|r| r.state == MediaState::Error) {
            Some(r) => {
                let msg = e.str_val(r.error.as_deref().unwrap_or("MediaError"));
                let o = e.new_object();
                e.set_own(o, "code", Value::Num(4.0), true);
                e.set_own(o, "message", msg, true);
                Value::Obj(o)
            }
            None => Value::Null,
        },
        // `videoWidth`/`videoHeight` non rapportés par l'hôte v1
        // → 0 honnête ; `mediaGroup`/`controller` → null.
        _ => {
            if matches!(key, "videoWidth" | "videoHeight") {
                Value::Num(0.0)
            } else {
                Value::Null
            }
        }
    }
}

/// Écriture `HTMLMediaElement` claimée (étape 71c) : les
/// props de lecture mutent l'état local **et** enfilent
/// l'ordre vers l'hôte (`media_cmds` → `MediaCommand` SSE) ;
/// les events spec sont tirés (`seeking`, `volumechange`,
/// `ratechange`). Non traitée → `false` (fallback générique).
fn media_set_claimed(
    e: &mut Engine,
    dom: &DomRef,
    cache: &NodeCache,
    node: NodeId,
    key: &str,
    v: Value,
) -> JsResult<bool> {
    let num = |e: &mut Engine, v: Value| -> JsResult<f64> {
        Ok(match v {
            Value::Num(n) => n,
            other => e
                .to_js_string(other)?
                .trim()
                .parse::<f64>()
                .unwrap_or(f64::NAN),
        })
    };
    let cmd = match key {
        "currentTime" => {
            let ms = (num(e, v)? * 1000.0).max(0.0) as u64;
            let mut d = dom.borrow_mut();
            if let Some(st) = d.media.get_mut(&node) {
                st.seek_pending = true;
                let m = st.media;
                d.media_cmds.push((m, MediaCmd::Seek { ms }));
            }
            drop(d);
            e.dispatch_media(dom, cache, node, "seeking")?;
            e.run_microtasks()?;
            return Ok(true);
        }
        "volume" => MediaCmd::Volume {
            v: num(e, v)?.clamp(0.0, 1.0),
        },
        "playbackRate" | "defaultPlaybackRate" => MediaCmd::Rate { r: num(e, v)? },
        "muted" => MediaCmd::Muted {
            on: e.to_boolean(v),
        },
        _ => return Ok(false),
    };
    let ty = match cmd {
        MediaCmd::Volume { .. } | MediaCmd::Muted { .. } => "volumechange",
        MediaCmd::Rate { .. } => "ratechange",
        _ => "",
    };
    {
        let mut d = dom.borrow_mut();
        let mut muted = None;
        if let Some(st) = d.media.get_mut(&node) {
            match cmd {
                MediaCmd::Volume { v } => st.volume = v,
                MediaCmd::Rate { r } => {
                    if r.is_finite() && r > 0.0 {
                        st.rate = r;
                    } else {
                        return Ok(true);
                    }
                }
                MediaCmd::Muted { on } => muted = Some(on),
                _ => {}
            }
            let m = st.media;
            d.media_cmds.push((m, cmd));
        }
        if let Some(on) = muted {
            if on {
                d.set_attr(node, "muted", "");
            } else {
                d.remove_attr(node, "muted");
            }
        }
    }
    e.dispatch_media(dom, cache, node, ty)?;
    e.run_microtasks()?;
    Ok(true)
}

/// Méthode `HTMLMediaElement` **claimée** (étape 71c) :
/// `play()` enfile l'ordre `Play` et rend une promesse tenue
/// au premier rapport `playing` (rejetée `error`/release) —
/// déjà `playing` → résolue d'office. `canPlayType` reflète
/// les codecs déclarés au claim (`"probably"` si un token
/// codec matche, `"maybe"` si seul le MIME matche, `""`
/// sinon). EME/sink/tracks restent non supportés même
/// claimés — jamais de faux succès.
fn media_call_claimed(
    e: &mut Engine,
    dom: &DomRef,
    cache: &NodeCache,
    node: NodeId,
    name: &str,
    args: &[Value],
) -> JsResult<Option<Value>> {
    match name {
        "play" => {
            let playing = {
                let mut d = dom.borrow_mut();
                match d.media.get_mut(&node) {
                    Some(st)
                        if matches!(
                            st.report.as_ref().map(|r| r.state),
                            Some(MediaState::Playing)
                        ) =>
                    {
                        None
                    }
                    Some(st) => {
                        let pid = e.new_promise();
                        st.pending_play.push(pid);
                        let m = st.media;
                        d.media_cmds.push((m, MediaCmd::Play));
                        Some(pid)
                    }
                    None => None,
                }
            };
            Ok(Some(if let Some(pid) = playing {
                Value::Obj(pid)
            } else {
                let pid = e.new_promise();
                e.resolve_promise(pid, Value::Undefined);
                Value::Obj(pid)
            }))
        }
        "pause" => {
            let mut d = dom.borrow_mut();
            if let Some(st) = d.media.get_mut(&node) {
                let m = st.media;
                d.media_cmds.push((m, MediaCmd::Pause));
            }
            Ok(Some(Value::Undefined))
        }
        "fastSeek" => {
            let sec = match args.first() {
                Some(Value::Num(n)) => *n,
                Some(other) => e.to_js_string(*other)?.trim().parse::<f64>().unwrap_or(0.0),
                None => 0.0,
            };
            let ms = (sec * 1000.0).max(0.0) as u64;
            {
                let mut d = dom.borrow_mut();
                if let Some(st) = d.media.get_mut(&node) {
                    st.seek_pending = true;
                    let m = st.media;
                    d.media_cmds.push((m, MediaCmd::Seek { ms }));
                }
            }
            e.dispatch_media(dom, cache, node, "seeking")?;
            e.run_microtasks()?;
            Ok(Some(Value::Undefined))
        }
        "canPlayType" => {
            let t = match args.first() {
                Some(v) => e.to_js_string(*v)?.to_lowercase(),
                None => String::new(),
            };
            let codecs = dom
                .borrow()
                .media
                .get(&node)
                .map(|st| st.codecs.clone())
                .unwrap_or_default();
            Ok(Some(e.str_val(can_play(&t, &codecs))))
        }
        // Claimé ≠ EME ni sortie audio alternative — rejets
        // honnêtes maintenus (ADR-0023).
        "setSinkId" | "setMediaKeys" => {
            let pid = e.new_promise();
            let msg = e.str_val("NotSupportedError: no supported media");
            e.reject_promise(pid, msg);
            Ok(Some(Value::Obj(pid)))
        }
        "addTextTrack" | "captureStream" => Ok(Some(Value::Null)),
        _ => Ok(Some(Value::Undefined)),
    }
}

/// `canPlayType` contre les codecs déclarés au claim
/// (étape 71c) — token codec présent dans l'argument →
/// `"probably"` ; seule la base MIME matche → `"maybe"` ;
/// rien → `""`.
fn can_play(arg: &str, declared: &[String]) -> &'static str {
    let mime = arg.split(';').next().unwrap_or("").trim();
    if mime.is_empty() {
        return "";
    }
    let mut mime_hit = false;
    for c in declared {
        let c = c.to_lowercase();
        let c = c.trim();
        if c.is_empty() {
            continue;
        }
        if arg.contains(c) {
            return "probably";
        }
        if c.contains(mime) || c.starts_with(mime) {
            mime_hit = true;
        }
    }
    if mime_hit {
        "maybe"
    } else {
        ""
    }
}

impl Host for NodeHost {
    fn class_name(&self) -> &'static str {
        match self.dom.borrow().nodes[self.node as usize].tag.as_str() {
            "video" => "HTMLVideoElement",
            "audio" => "HTMLAudioElement",
            "canvas" => "HTMLCanvasElement",
            _ => "HTMLElement",
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn get(&self, e: &mut Engine, this: Value, key: &str) -> JsResult<Option<Value>> {
        let node = self.node;
        // `<canvas>.width`/`.height` (JS-C28) : valeur
        // **numérique** lue sur l'attribut — défaut spec
        // 300×150 (les autres éléments suivent le chemin
        // générique prop→attr).
        if matches!(key, "width" | "height")
            && self.dom.borrow().nodes[node as usize].tag == "canvas"
        {
            let d = self.dom.borrow();
            let v = d
                .attr(node, key)
                .and_then(|s| s.trim().parse::<f64>().ok())
                .unwrap_or(if key == "width" { 300.0 } else { 150.0 });
            return Ok(Some(Value::Num(v.max(0.0))));
        }
        // `constructor.name` (étape 74) : objet allégé
        // portant le nom de classe — `v.constructor.name`
        // vaut « HTMLVideoElement », pas « Object ». Non
        // constructible (v1 cosmétique documentée — de vrais
        // bindings `new`-ables sont un chantier plus large).
        if key == "constructor" {
            let o = e.new_object();
            let name = e.str_val(self.class_name());
            e.set_own(o, "name", name, true);
            return Ok(Some(Value::Obj(o)));
        }
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
            "parentNode" => {
                let p = self.dom.borrow().nodes[node as usize].parent;
                return Ok(Some(match p {
                    Some(p) => node_value(e, &self.dom, &self.cache, p),
                    None => Value::Null,
                }));
            }
            // `parentElement` : spec — `null` quand le parent n'est
            // pas un élément (ex. `html.parentElement`, dont le
            // parentNode est `#document`). Le confondre avec
            // `parentNode` casse les remontées d'arbre qui
            // terminent sur `null` (mesuré sur google.com).
            "parentElement" => {
                let p = {
                    let d = self.dom.borrow();
                    d.nodes[node as usize].parent.filter(|&p| {
                        // La racine `#document` est stockée `Element`
                        // dans le miroir mais n'est PAS un Element
                        // DOM : `html.parentElement` doit être null.
                        p != d.root && d.nodes[p as usize].kind == NodeKind::Element
                    })
                };
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
            // `<template>.content` — `DocumentFragment` de
            // contenu (spec : ses enfants ne sont pas des
            // enfants de l'élément). `undefined` ailleurs —
            // la prop n'existe que sur `HTMLTemplateElement`.
            // Mesuré github `z8-*.js` :
            // `tpl.content.appendChild(row)`.
            "content" => {
                let frag = self.dom.borrow_mut().template_content(node);
                if let Some(f) = frag {
                    return Ok(Some(node_value(e, &self.dom, &self.cache, f)));
                }
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
                    NodeKind::Fragment => 11.0,
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
            // -- `HTMLMediaElement` (étape 58/71c) ------------------
            // Non claimé : stubs honnêtes ADR-0023. Claimé :
            // reflet du rapport `state` hôte (ADR-0024).
            "paused"
            | "ended"
            | "seeking"
            | "preservesPitch"
            | "disableRemotePlayback"
            | "volume"
            | "currentTime"
            | "duration"
            | "playbackRate"
            | "defaultPlaybackRate"
            | "readyState"
            | "networkState"
            | "videoWidth"
            | "videoHeight"
            | "currentSrc"
            | "buffered"
            | "played"
            | "seekable"
            | "error"
            | "mediaGroup"
            | "controller"
                if is_media(&self.dom, node) =>
            {
                return Ok(Some(media_prop(e, &self.dom, node, key)));
            }
            // Dimensions sans moteur de mise en page derrière :
            // `documentElement`/`body` empruntent le viewport,
            // tout autre élément vaut 0 — stub honnête (les pages
            // lisent `documentElement.clientHeight` pour le
            // « scroll bottom » — mesuré sur google.com).
            "clientWidth" | "clientHeight" | "scrollWidth" | "scrollHeight" | "offsetWidth"
            | "offsetHeight" => {
                let (vw, vh) = e.hooks.viewport;
                let is_root = {
                    let d = self.dom.borrow();
                    node == d.body() || Some(node) == d.document_element()
                };
                let px = if is_root && key.ends_with("Height") {
                    vh
                } else if is_root {
                    vw
                } else {
                    0
                };
                return Ok(Some(Value::Num(f64::from(px))));
            }
            // Références de méthode — les shims lisent
            // `o = el.matches || el.msMatchesSelector`
            // (selector-observer github.com : `o.call(e,t)`)
            // puis invoquent via `.call`/apply ; les méthodes
            // host invoke-only sont invisibles à la lecture.
            // La native prend son `this` du receveur d'appel.
            "matches"
            | "matchesSelector"
            | "webkitMatchesSelector"
            | "mozMatchesSelector"
            | "oMatchesSelector"
            | "msMatchesSelector" => {
                return Ok(Some(e.native_fn("matches", element_matches)));
            }
            "closest" => {
                return Ok(Some(e.native_fn("closest", element_closest)));
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
                // `<template>` : l'écriture cible son fragment de
                // contenu (spec — `tpl.innerHTML` peuple
                // `content`, pas les enfants de l'élément).
                let target = d.content_target(node);
                // Detach existing children, parse into a temp parent,
                // move results over.
                let tmp = d.create_element("#fragment");
                d.parse_fragment_into(tmp, &html);
                let kids = std::mem::take(&mut d.nodes[tmp as usize].children);
                for &k in &kids {
                    d.nodes[k as usize].parent = None;
                }
                d.replace_children(target, &kids);
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
            // `<canvas>.width=`/`height=` (JS-C28) :
            // attribut + effacement du bitmap + remise à
            // zéro de l'état 2D (spec). Coercion avant le
            // borrow — un `valueOf` ré-entrant peut toucher
            // le miroir.
            "width" | "height" => {
                let s = e.to_js_string(v)?;
                let mut d = self.dom.borrow_mut();
                if d.nodes[node as usize].tag == "canvas" {
                    d.set_attr(node, key, &s);
                    d.clear_canvas(node);
                    return Ok(true);
                }
                Ok(false)
            }
            // `HTMLMediaElement` (étape 58/71c) — non claimé :
            // l'écriture est acceptée-ignorée (le stub n'a pas
            // d'état de lecture à muter) plutôt qu'échouée.
            // Claimé : `media_set_claimed` mute l'état local,
            // enfile l'ordre hôte et tire l'event spec.
            "currentTime"
            | "volume"
            | "playbackRate"
            | "defaultPlaybackRate"
            | "muted"
            | "preservesPitch"
            | "disableRemotePlayback"
            | "disablePictureInPicture"
            | "mediaGroup"
                if is_media(&self.dom, node) =>
            {
                if self.dom.borrow().media.contains_key(&node) {
                    media_set_claimed(e, &self.dom, &self.cache, node, key, v)
                } else {
                    Ok(true)
                }
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
                // `submit` spec : bubbles + cancelable, non-composed.
                let state = make_event_state(
                    "submit",
                    node,
                    EventFlags {
                        bubbles: true,
                        cancelable: true,
                        composed: false,
                        trusted: false,
                    },
                    Vec::new(),
                );
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
            // `*MatchesSelector` : alias pré-standards encore
            // utilisés par les shims de libs réelles
            // (selector-observer github.com).
            "matches"
            | "matchesSelector"
            | "webkitMatchesSelector"
            | "mozMatchesSelector"
            | "oMatchesSelector"
            | "msMatchesSelector" => {
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
            // `<canvas>.getContext("2d")` (JS-C28, couture
            // R-11) : le contexte **enregistre** ses appels
            // dans la side-table du miroir — jamais de raster
            // ici. Un contexte par mode et par nœud : l'objet
            // est mis en cache sur le `Dom`.
            "getContext" => {
                let kind = e.to_js_string(arg(args, 0))?;
                let is_canvas = self.dom.borrow().nodes[node as usize].tag == "canvas";
                if !is_canvas || kind != "2d" {
                    return Ok(Some(Value::Null));
                }
                if let Some(v) = self.dom.borrow().canvas_ctx(node) {
                    return Ok(Some(v));
                }
                let v = e.new_host_object(Rc::new(Ctx2dHost {
                    dom: self.dom.clone(),
                    cache: self.cache.clone(),
                    node,
                }));
                self.dom.borrow_mut().set_canvas_ctx(node, v);
                Ok(Some(v))
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
            // `HTMLMediaElement` (étape 58/71c, ADR-0023/24) —
            // non claimé : stubs honnêtes, jamais de faux
            // succès. Claimé : délégation hôte (`play()` tenu
            // par le premier `playing` rapporté).
            "play" | "pause" | "load" | "fastSeek" | "canPlayType" | "setSinkId"
            | "addTextTrack" | "captureStream" | "setMediaKeys"
                if is_media(&self.dom, node) =>
            {
                if self.dom.borrow().media.contains_key(&node) {
                    return media_call_claimed(e, &self.dom, &self.cache, node, name, args);
                }
                match name {
                    // Spec : sans ressource décodable la
                    // promesse rejette `NotSupportedError` —
                    // c'est le même chemin `.catch()` que le
                    // blocage autoplay, déjà géré par les sites.
                    "play" | "setSinkId" | "setMediaKeys" => {
                        let pid = e.new_promise();
                        let msg = e.str_val("NotSupportedError: no supported media");
                        e.reject_promise(pid, msg);
                        Ok(Some(Value::Obj(pid)))
                    }
                    // Aucun codec → `""` (« ne peut pas lire »).
                    "canPlayType" => Ok(Some(e.str_val(""))),
                    "addTextTrack" | "captureStream" => Ok(Some(Value::Null)),
                    _ => Ok(Some(Value::Undefined)),
                }
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
    if let Some(n) = h.as_any().downcast_ref::<NodeHost>() {
        return Some(n.node);
    }
    // `document` est un nœud cible valide (`observe(document)`,
    // `contains`…) — il se résout sur la racine `#document`.
    // Mesuré sur github.com : element-registry observe le
    // document (JS-C56).
    if let Some(dh) = h.as_any().downcast_ref::<DocumentHost>() {
        return Some(dh.dom.borrow().root);
    }
    None
}

/// `(node, dom, cache)` d'un receveur **élément** — pour les
/// natives posées sur `Element.prototype` (`m.call(el, …)` ; le
/// `DocumentHost` ne compte pas, spec : receveur non-Element →
/// `TypeError`).
fn element_receiver(e: &mut Engine, v: Value) -> Option<(NodeId, DomRef, NodeCache)> {
    let Value::Obj(id) = v else {
        return None;
    };
    let h = match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::Host(h)) => h.clone(),
        _ => return None,
    };
    let n = h.as_any().downcast_ref::<NodeHost>()?;
    Some((n.node, n.dom.clone(), n.cache.clone()))
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
    // `EventInit` relu (posé par `new Event(type, init)` — le
    // dict est déjà appliqué sur l'objet au constructeur).
    let cancelable = e
        .get_prop(id, "cancelable")
        .is_some_and(|b| e.to_boolean(b));
    let composed = e.get_prop(id, "composed").is_some_and(|b| e.to_boolean(b));
    // Props portées par l'objet literal (`key`, `code`…) —
    // recopiées sur le state pour que `e.key` resolve au
    // dispatch (`el.dispatchEvent({type:'keydown',key:'a'})`).
    let props: Vec<(String, Value)> = [
        "key", "code", "keyCode", "which", "charCode", "ctrlKey", "shiftKey", "altKey", "metaKey",
        "detail",
    ]
    .into_iter()
    .filter_map(|k| e.get_prop(id, k).map(|v| (k.to_string(), v)))
    .collect();
    Ok(Some(make_event_state(
        &ty,
        node,
        EventFlags {
            bubbles,
            cancelable,
            composed,
            trusted: false,
        },
        props,
    )))
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
            "cancelable" => Value::Bool(self.state.cancelable.get()),
            "composed" => Value::Bool(self.state.composed.get()),
            "isTrusted" => Value::Bool(self.state.trusted),
            "eventPhase" => Value::Num(f64::from(self.state.phase.get())),
            "timeStamp" => Value::Num(self.state.created.elapsed().as_secs_f64() * 1000.0),
            "defaultPrevented" => Value::Bool(self.state.prevented.get()),
            "cancelBubble" => Value::Bool(self.state.stopped.get()),
            // Legacy DOM0 : `returnValue === false` reflète
            // `preventDefault` (spec `Event` historique).
            "returnValue" => Value::Bool(!self.state.prevented.get()),
            _ => match self.state.props.iter().find(|(k, _)| k == key) {
                Some((_, v)) => *v,
                None => return Ok(None),
            },
        }))
    }

    fn set(&self, e: &mut Engine, _t: Value, key: &str, v: Value) -> JsResult<bool> {
        match key {
            // Spec : `cancelBubble = true` ≡ `stopPropagation`
            // (l'écriture de `false` est sans effet).
            "cancelBubble" => {
                if e.to_boolean(v) {
                    self.state.stopped.set(true);
                }
                Ok(true)
            }
            // Legacy DOM0 : `returnValue = false` ≡
            // `preventDefault` (no-op si non-annulable).
            "returnValue" => {
                if !e.to_boolean(v) && self.state.cancelable.get() {
                    self.state.prevented.set(true);
                }
                Ok(true)
            }
            _ => Ok(false),
        }
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
                // Spec : no-op si l'event n'est pas `cancelable`.
                if self.state.cancelable.get() {
                    self.state.prevented.set(true);
                }
                Ok(Some(Value::Undefined))
            }
            "stopPropagation" | "cancel" => {
                self.state.stopped.set(true);
                Ok(Some(Value::Undefined))
            }
            "stopImmediatePropagation" => {
                // Tue aussi les listeners frères du nœud courant ;
                // `stopped` est posé pour barrer la propagation.
                self.state.stopped.set(true);
                self.state.immediate.set(true);
                Ok(Some(Value::Undefined))
            }
            "composedPath" => {
                // Spec : `[]` hors dispatch (`currentTarget` null),
                // sinon le chemin réel target → ancêtres → document.
                if self.state.current.get().is_none() {
                    return Ok(Some(e.new_array(Vec::new())));
                }
                let target = self.state.target.get();
                let mut nodes = vec![target];
                {
                    let d = self.dom.borrow();
                    nodes.extend(d.ancestors(target));
                }
                let vals: Vec<Value> = nodes
                    .iter()
                    .map(|&n| node_value(e, &self.dom, &self.cache, n))
                    .collect();
                Ok(Some(e.new_array(vals)))
            }
            "initEvent" => {
                let ty = e.to_js_string(arg(args, 0))?;
                *self.state.ty.borrow_mut() = ty;
                self.state.bubbles.set(e.to_boolean(arg(args, 1)));
                self.state.cancelable.set(e.to_boolean(arg(args, 2)));
                // Spec `initEvent` : les flags de propagation et
                // `defaultPrevented` repartent à zéro.
                self.state.stopped.set(false);
                self.state.immediate.set(false);
                self.state.prevented.set(false);
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
/// `customElements` — `CustomElementRegistry` minimal : nom → ctor.
/// `define` exige un ctor appelable et un nom libre (`TypeError` —
/// approche `NotSupportedError`/`SyntaxError` de la spec) ;
/// `whenDefined` rend une promesse tenue pour un nom enregistré,
/// pendante sinon (spec) ; `upgrade` est no-op honnête — le
/// miroir ne relie pas le constructeur aux nœuds existants.
/// Mesuré github.com : `customElements.define(name, class)`
/// (JS-C56).
struct CustomElementsHost {
    registry: RefCell<HashMap<String, Value>>,
}

impl Host for CustomElementsHost {
    fn class_name(&self) -> &'static str {
        "CustomElementRegistry"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn invoke(
        &self,
        e: &mut Engine,
        _t: Value,
        name: &str,
        args: &[Value],
    ) -> JsResult<Option<Value>> {
        match name {
            "define" => {
                let n = e.to_js_string(arg(args, 0))?;
                let ctor = arg(args, 1);
                if !e.is_callable(ctor) {
                    return Err(e.type_error("customElements.define: ctor is not a constructor"));
                }
                if self.registry.borrow().contains_key(&n) {
                    return Err(e.type_error("custom element already defined"));
                }
                self.registry.borrow_mut().insert(n, ctor);
                Ok(Some(Value::Undefined))
            }
            "get" => {
                let n = e.to_js_string(arg(args, 0))?;
                Ok(Some(
                    self.registry
                        .borrow()
                        .get(&n)
                        .copied()
                        .unwrap_or(Value::Undefined),
                ))
            }
            "whenDefined" => {
                let n = e.to_js_string(arg(args, 0))?;
                let p = e.new_promise();
                if let Some(&ctor) = self.registry.borrow().get(&n) {
                    e.resolve_promise(p, ctor);
                }
                Ok(Some(Value::Obj(p)))
            }
            "upgrade" => Ok(Some(Value::Undefined)),
            _ => Ok(None),
        }
    }

    fn trace(&self, out: &mut Vec<ObjId>) {
        for v in self.registry.borrow().values() {
            if let Value::Obj(id) = v {
                out.push(*id);
            }
        }
    }
}

/// `IntersectionObserver`/`ResizeObserver` — passifs : le
/// callback est validé/retenu mais jamais appelé (le miroir ne
/// produit ni intersection ni resize — honnête, pas un feu
/// artificiel). `observe`/`unobserve`/`disconnect` no-op,
/// `takeRecords` → `[]`.
struct PassiveObserverHost {
    kind: &'static str,
    cb: Value,
}

impl Host for PassiveObserverHost {
    fn class_name(&self) -> &'static str {
        self.kind
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn invoke(
        &self,
        e: &mut Engine,
        _t: Value,
        name: &str,
        _args: &[Value],
    ) -> JsResult<Option<Value>> {
        match name {
            "observe" | "unobserve" | "disconnect" => Ok(Some(Value::Undefined)),
            "takeRecords" => Ok(Some(e.new_array(Vec::new()))),
            _ => Ok(None),
        }
    }

    fn trace(&self, out: &mut Vec<ObjId>) {
        if let Value::Obj(id) = self.cb {
            out.push(id);
        }
    }
}

/// `new IntersectionObserver(cb[, options])` — callback requis.
fn intersection_observer_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    passive_observer(e, args, "IntersectionObserver")
}

/// `new ResizeObserver(cb)` — callback requis.
fn resize_observer_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    passive_observer(e, args, "ResizeObserver")
}

fn passive_observer(e: &mut Engine, args: &[Value], kind: &'static str) -> JsResult<Value> {
    let cb = arg(args, 0);
    if !e.is_callable(cb) {
        return Err(e.type_error(format!("{kind}: callback is not a function")));
    }
    Ok(e.new_host_object(Rc::new(PassiveObserverHost { kind, cb })))
}

/// `document.implementation` — `createHTMLDocument` produit un
/// document détaché complet (`html`/`head`/`title`/`body`) que
/// les scripts utilisent comme zone de parsing sûre — jQuery
/// l'exige (`implementation.createHTMLDocument`, trigger réel
/// arstechnica/wordpress).
struct DomImplementationHost;

impl Host for DomImplementationHost {
    fn class_name(&self) -> &'static str {
        "DOMImplementation"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn invoke(
        &self,
        e: &mut Engine,
        _t: Value,
        name: &str,
        args: &[Value],
    ) -> JsResult<Option<Value>> {
        match name {
            "createHTMLDocument" => {
                let mut d = Dom::new();
                d.parse_fragment_into(
                    d.root,
                    "<html><head><title></title></head><body></body></html>",
                );
                let title = e.to_js_string(arg(args, 0)).unwrap_or_default();
                if !title.is_empty() {
                    if let Some(&tn) = d.by_tag(d.root, "title").first() {
                        let t = d.create_text(&title);
                        d.append(tn, t);
                    }
                }
                Ok(Some(e.new_host_object(Rc::new(DocumentHost {
                    dom: Rc::new(RefCell::new(d)),
                    cache: Rc::new(RefCell::new(HashMap::new())),
                    implementation: RefCell::new(None),
                }))))
            }
            // `hasFeature()` — spec : retourne toujours `true`
            // (API historique sans substance).
            "hasFeature" => Ok(Some(Value::Bool(true))),
            _ => Ok(None),
        }
    }
}

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
        match key {
            "length" => Ok(Some(Value::Num(f64::from(e.hooks.history_len)))),
            // Méthodes en *valeur* : les wraps de navigation
            // (`e.call(history,…)` — turbo github.com) lisent la
            // référence puis l'appellent ; invoke-only les rend
            // invisibles. Le `this` est sans effet — tout passe
            // par les hooks.
            "back" => Ok(Some(e.native_fn("back", history_back))),
            "forward" => Ok(Some(e.native_fn("forward", history_forward))),
            "go" => Ok(Some(e.native_fn("go", history_go))),
            "pushState" => Ok(Some(e.native_fn("pushState", history_push_state))),
            "replaceState" => Ok(Some(e.native_fn("replaceState", history_replace_state))),
            _ => Ok(None),
        }
    }

    fn invoke(
        &self,
        e: &mut Engine,
        t: Value,
        name: &str,
        args: &[Value],
    ) -> JsResult<Option<Value>> {
        let r = match name {
            "back" => history_back(e, t, args),
            "forward" => history_forward(e, t, args),
            "go" => history_go(e, t, args),
            "pushState" => history_push_state(e, t, args),
            "replaceState" => history_replace_state(e, t, args),
            _ => return Ok(None),
        };
        r.map(Some)
    }
}

fn history_nav(e: &mut Engine, n: i32) -> JsResult<Value> {
    if n != 0 {
        if let Some(go) = &e.hooks.history_go {
            go(n);
        }
    }
    Ok(Value::Undefined)
}

fn history_push_replace(e: &mut Engine, args: &[Value]) -> JsResult<Value> {
    // Pas de pile de session — l'URL courante change comme chez
    // un navigateur, sans rechargement.
    let u = arg(args, 2);
    if !matches!(u, Value::Undefined | Value::Null) {
        let url = e.to_js_string(u)?;
        if !url.is_empty() {
            e.hooks.page_url = url;
        }
    }
    Ok(Value::Undefined)
}

fn history_back(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    history_nav(e, -1)
}

fn history_forward(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    history_nav(e, 1)
}

fn history_go(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let n = e.to_number(arg(args, 0))? as i32;
    history_nav(e, n)
}

fn history_push_state(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    history_push_replace(e, args)
}

fn history_replace_state(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    history_push_replace(e, args)
}

/// `document.fonts` — `FontFaceSet` minimal. Pas de pipeline de
/// fontes web dans ce moteur : `load()`/`ready` résolvent
/// toujours, `check()` dit vrai — surface honnête pour les pages
/// qui feature-detectent puis chargent une fonte de marque
/// (mesuré : bootstrap `Google Sans` de google.com).
struct FontFaceSetHost;

impl Host for FontFaceSetHost {
    fn class_name(&self) -> &'static str {
        "FontFaceSet"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn get(&self, e: &mut Engine, this: Value, key: &str) -> JsResult<Option<Value>> {
        match key {
            "size" => Ok(Some(Value::Num(0.0))),
            "status" => Ok(Some(e.str_val("loaded"))),
            // `fonts.ready` — résolue avec le set lui-même (spec).
            "ready" => {
                let pid = e.new_promise();
                e.resolve_promise(pid, this);
                Ok(Some(Value::Obj(pid)))
            }
            _ => Ok(None),
        }
    }

    fn invoke(
        &self,
        e: &mut Engine,
        _t: Value,
        name: &str,
        _args: &[Value],
    ) -> JsResult<Option<Value>> {
        match name {
            // `load(font, text?)` → Promise<FontFace[]> — résolu
            // avec un tableau vide : aucune fonte web déclarée.
            "load" => {
                let arr = e.new_array(Vec::new());
                let pid = e.new_promise();
                e.resolve_promise(pid, arr);
                Ok(Some(Value::Obj(pid)))
            }
            "check" => Ok(Some(Value::Bool(true))),
            _ => Ok(None),
        }
    }
}

/// `window.addEventListener(ty, f, capture?)` — enregistré sur le
/// nœud racine (`window` ≙ sommet de l'arbre de propagation, même
/// cible que les listeners `document.*`).
fn win_add_event_listener(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let ty = e.to_js_string(arg(args, 0))?;
    let f = arg(args, 1);
    let capture = e.to_boolean(arg(args, 2));
    if e.is_callable(f) {
        if let Some((dom, _)) = &e.dom_ctx {
            let root = dom.borrow().root;
            dom.borrow_mut().nodes[root as usize]
                .listeners
                .push((ty, f, capture));
        }
    }
    Ok(Value::Undefined)
}

/// `window.removeEventListener(ty, f, capture?)` — miroir de
/// [`win_add_event_listener`].
fn win_remove_event_listener(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let ty = e.to_js_string(arg(args, 0))?;
    let f = arg(args, 1);
    let capture = e.to_boolean(arg(args, 2));
    if let Some((dom, _)) = &e.dom_ctx {
        let root = dom.borrow().root;
        dom.borrow_mut().nodes[root as usize]
            .listeners
            .retain(|(t, lf, c)| !(t == &ty && *c == capture && e.strict_eq(*lf, f)));
    }
    Ok(Value::Undefined)
}

/// `window.dispatchEvent(ev)` — propagation complète depuis le
/// nœud racine (≙ `document.dispatchEvent` sur `#document`).
fn win_dispatch_event(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let Some((dom, cache)) = e.dom_ctx.clone() else {
        return Ok(Value::Bool(false));
    };
    let root = dom.borrow().root;
    let Some(state) = event_state_of(e, root, &dom, &cache, arg(args, 0))? else {
        return Ok(Value::Bool(false));
    };
    e.propagate_event(&dom, &cache, &state)?;
    Ok(Value::Bool(!state.prevented.get()))
}

/// `getComputedStyle(el, pseudo?)` — pas de cascade CSS : l'objet
/// rendu lit les valeurs **inline** de l'élément (honnête — les
/// sondes `display`/`visibility` des scripts obtiennent `""`,
/// jamais une exception). `TypeError` si l'argument n'est pas un
/// élément, comme la spec.
fn win_get_computed_style(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let Some(node) = node_for(e, arg(args, 0)) else {
        return Err(e.type_error("getComputedStyle: parameter 1 is not of type 'Element'"));
    };
    let Some((dom, _)) = e.dom_ctx.clone() else {
        return Err(e.type_error("getComputedStyle: no document"));
    };
    Ok(e.new_host_object(Rc::new(StyleHost { dom, node })))
}

/// `window.matchMedia(q)` — pas de moteur de media queries :
/// `MediaQueryList` honnête `matches:false`, `media` = la requête,
/// listeners no-op (viewport fixe, jamais de `change`).
fn win_match_media(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let q = e.to_js_string(arg(args, 0))?;
    let m = e.new_object();
    e.set_own(m, "matches", Value::Bool(false), true);
    let media = e.str_val(q);
    e.set_own(m, "media", media, true);
    e.set_own(m, "onchange", Value::Null, true);
    e.add_method(m, "addListener", mql_noop);
    e.add_method(m, "removeListener", mql_noop);
    e.add_method(m, "addEventListener", mql_noop);
    e.add_method(m, "removeEventListener", mql_noop);
    e.add_method(m, "dispatchEvent", mql_noop);
    Ok(Value::Obj(m))
}

/// `MediaQueryList` listener no-op — les queries ne changent pas.
fn mql_noop(_e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    Ok(Value::Undefined)
}

/// `CSS.supports(…)` — pas de moteur CSS : `false` honnête.
fn css_supports(_e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    Ok(Value::Bool(false))
}

/// `new Node()` — inconstructible ("illegal constructor" per spec).
fn node_ctor(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    Err(e.type_error("Node: illegal constructor"))
}

/// Constructeur-stub des interfaces DOM sans implémentation
/// (`HTMLElement`, `IntersectionObserver`…) — « illegal
/// constructor » comme `Node`, la détection `typeof`/`in` suffit.
fn illegal_ctor(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    Err(e.type_error("illegal constructor"))
}

/// `new Image(width?, height?)` — alias constructeur de
/// `document.createElement('img')` (nœud détaché du miroir) +
/// attributs `width`/`height` optionnels, args numériques ou
/// chaînes. Mesuré wikipedia.org (vol 22/10) : `doWhenReady`
/// précharge via `new Image()` → `ReferenceError` (JS-C57).
/// Sans `document` installé : `TypeError` honnête, jamais panic.
fn image_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let Some((dom, cache)) = e.dom_ctx.clone() else {
        return Err(e.type_error("Image: no document"));
    };
    let n = dom.borrow_mut().create_element("img");
    for (i, attr) in ["width", "height"].into_iter().enumerate() {
        if let Some(v) = args.get(i).filter(|v| !matches!(v, Value::Undefined)) {
            let s = e.to_js_string(*v)?;
            dom.borrow_mut().set_attr(n, attr, &s);
        }
    }
    Ok(node_value(e, &dom, &cache, n))
}

/// `new Event(type, init)` — objet ordinaire dispatchable
/// (`dispatchEvent` lit `type`/`bubbles` sur l'objet) : props
/// standard posées sur `this`, proto `Event.prototype` assuré par
/// le chemin `[[Construct]]` générique (JS-C56).
fn event_ctor(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let ty = e.to_js_string(arg(args, 0))?;
    let init = arg(args, 1);
    // `init` est un dictionnaire IDL : `undefined`/`null` → vide,
    // objet → lu, autre primitif → TypeError (spec WebIDL).
    match init {
        Value::Undefined | Value::Null | Value::Obj(_) => {}
        _ => return Err(e.type_error("event init is not an object")),
    }
    let tv = e.str_val(ty);
    e.set_prop_value(this, "type", tv)?;
    if matches!(init, Value::Obj(_)) {
        for k in ["bubbles", "cancelable", "composed"] {
            let v = e.get_prop_value(init, k)?;
            e.set_prop_value(this, k, Value::Bool(e.to_boolean(v)))?;
        }
    } else {
        for (k, v) in [
            ("bubbles", Value::Bool(false)),
            ("cancelable", Value::Bool(false)),
            ("composed", Value::Bool(false)),
        ] {
            e.set_prop_value(this, k, v)?;
        }
    }
    // `CustomEvent`/`ErrorEvent`/`PromiseRejectionEvent`/
    // `KeyboardEvent` portent des payloads d'init — copiés
    // tels quels.
    if matches!(init, Value::Obj(_)) {
        for k in [
            "detail", "message", "error", "filename", "lineno", "colno", "promise", "reason",
            "key", "code", "keyCode", "which", "charCode", "ctrlKey", "shiftKey", "altKey",
            "metaKey",
        ] {
            let v = e.get_prop_value(init, k)?;
            if !matches!(v, Value::Undefined) {
                e.set_prop_value(this, k, v)?;
            }
        }
    }
    for (k, v) in [
        ("target", Value::Null),
        ("srcElement", Value::Null),
        ("currentTarget", Value::Null),
        ("relatedTarget", Value::Null),
        ("isTrusted", Value::Bool(false)),
        ("defaultPrevented", Value::Bool(false)),
        ("cancelBubble", Value::Bool(false)),
        ("returnValue", Value::Bool(true)),
        ("eventPhase", Value::Num(0.0)),
        ("timeStamp", Value::Num(0.0)),
    ] {
        e.set_prop_value(this, k, v)?;
    }
    Ok(Value::Undefined)
}

/// `ev.preventDefault()` — sur un event script-créé : flag posé
/// (`defaultPrevented`/`returnValue`), pas d'effet sur un event
/// de dispatch (ceux-ci passent par `EventHost`).
fn event_prevent_default(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    e.set_prop_value(this, "defaultPrevented", Value::Bool(true))?;
    e.set_prop_value(this, "returnValue", Value::Bool(false))?;
    Ok(Value::Undefined)
}

/// `ev.stopPropagation()`/`stopImmediatePropagation()` — flag
/// `cancelBubble` posé (best-effort hors dispatch).
fn event_stop_propagation(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    e.set_prop_value(this, "cancelBubble", Value::Bool(true))?;
    Ok(Value::Undefined)
}

/// `ev.composedPath()` — `[]` honnête hors dispatch.
fn event_composed_path(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    Ok(e.new_array(Vec::new()))
}

/// `el.matches(sel)` — native d'`Element.prototype` : les
/// shims lisent la *référence* (`o = Element.prototype.matches
/// || i.msMatchesSelector` mesuré selector-observer github) —
/// les méthodes host invoke-only sont invisibles à la lecture.
fn element_matches(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let sel = e.to_js_string(arg(args, 0))?;
    let Some((node, dom, _)) = element_receiver(e, this) else {
        return Err(e.type_error("matches: receiver is not an Element"));
    };
    let r = dom.borrow().matches(node, &sel);
    Ok(Value::Bool(r))
}

/// `el.closest(sel)` — native d'`Element.prototype` (même
/// motif que [`element_matches`]).
fn element_closest(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let sel = e.to_js_string(arg(args, 0))?;
    let Some((node, dom, cache)) = element_receiver(e, this) else {
        return Err(e.type_error("closest: receiver is not an Element"));
    };
    let hit = dom.borrow().closest(node, &sel);
    Ok(match hit {
        Some(n) => node_value(e, &dom, &cache, n),
        None => Value::Null,
    })
}

/// `ev.initEvent(type, bubbles, cancelable)` — héritage DOM Level 2.
fn event_init(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let ty = e.to_js_string(arg(args, 0))?;
    let tv = e.str_val(ty);
    e.set_prop_value(this, "type", tv)?;
    e.set_prop_value(this, "bubbles", Value::Bool(e.to_boolean(arg(args, 1))))?;
    e.set_prop_value(this, "cancelable", Value::Bool(e.to_boolean(arg(args, 2))))?;
    Ok(Value::Undefined)
}

/// `new AbortController()` — `{signal: {aborted:false}}` minimal ;
/// `abort()` (sur le prototype) bascule `signal.aborted`.
fn abort_controller_ctor(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let sig = e.new_object();
    e.set_own(sig, "aborted", Value::Bool(false), true);
    e.set_own(sig, "onabort", Value::Null, true);
    e.set_prop_value(this, "signal", Value::Obj(sig))?;
    Ok(Value::Undefined)
}

/// `controller.abort()` — `signal.aborted = true`, `onabort` appelé.
fn abort_signal_abort(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let sig = e.get_prop_value(this, "signal")?;
    e.set_prop_value(sig, "aborted", Value::Bool(true))?;
    let cb = e.get_prop_value(sig, "onabort")?;
    if e.is_callable(cb) {
        e.call_value(cb, sig, &[])?;
    }
    Ok(Value::Undefined)
}

/// `CSS.escape(s)` — algorithme « serialize an identifier »
/// (cssom) : échappement `\NN ` des contrôles, chiffre initial,
/// `-` seul ; identité pour ident-chars et non-ASCII.
fn css_escape(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let s = e.to_js_string(arg(args, 0))?;
    let mut out = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        let cp = c as u32;
        if cp == 0 {
            out.push('\u{fffd}');
        } else if (0x01..=0x1f).contains(&cp)
            || cp == 0x7f
            || (i == 0 && c.is_ascii_digit())
            || (i == 1 && c.is_ascii_digit() && chars[0] == '-')
        {
            out.push_str(&format!("\\{cp:x} "));
        } else if i == 0 && c == '-' && chars.len() == 1 {
            out.push_str("\\-");
        } else if c == '-' || c == '_' || c.is_ascii_alphanumeric() || cp >= 0x80 {
            out.push(c);
        } else {
            out.push('\\');
            out.push(c);
        }
    }
    Ok(e.str_val(out))
}

/// `navigator.sendBeacon(url, data?)` — POST fire-and-forget sur
/// le transport XHR de l'embedder ; `false` sans connecteur.
fn nav_send_beacon(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let url = e.to_js_string(arg(args, 0))?;
    if url.is_empty() {
        return Ok(Value::Bool(false));
    }
    let body = match arg(args, 1) {
        Value::Undefined => None,
        v => Some(e.to_js_string(v)?),
    };
    let Some(t) = &e.xhr.transport else {
        return Ok(Value::Bool(false));
    };
    t.send(&XhrRequest {
        method: "POST".into(),
        url,
        headers: Vec::new(),
        body,
    });
    Ok(Value::Bool(true))
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

// -- `<canvas>` 2D (JS-C28, couture R-11) ------------------------------
//
// `Ctx2dHost` **enregistre** les appels `CanvasRenderingContext2D`
// dans la side-table `Dom::canvases` — aucun raster côté moteur :
// la session traduit `CanvasOpRecord` → `CanvasOp` et le playback
// `raster_ops` peint un bitmap remplacé. Chaque op pousse
// `mark_dirty_at(canvas)` → le nœud est le chemin sale de la
// prochaine synchro (relayout ciblé du sous-arbre `<canvas>`).

/// Méthodes contexte 2D hors MVP R-11 absorbées en no-op — les
/// scripts réels les chaînent au milieu d'une séquence de
/// dessin ; une `TypeError` tuerait le bloc entier alors que
/// l'op manquante est un trou visuel acceptable.
const NOOP_CTX_METHODS: &[&str] = &[
    "ellipse",
    "roundRect",
    "drawImage",
    "fillText",
    "strokeText",
    "setLineDash",
    "drawFocusIfNeeded",
    "scrollPathIntoView",
    "putImageData",
    "reset",
    "resetTransform",
];

/// `CanvasRenderingContext2D` — enregistreur d'op-list sur le
/// `<canvas>` `node` (JS-C28).
struct Ctx2dHost {
    dom: DomRef,
    cache: NodeCache,
    node: NodeId,
}

/// `CanvasGradient`/`CanvasPattern` opaque (hors MVP R-11) :
/// `addColorStop` existe pour que les scripts n'échouent pas ;
/// assigné à `fillStyle`, le parse couleur échoue et le
/// réglage précédent survit (spec : valeur illisible ignorée).
struct GradientHost;

/// Argument numérique `i` coercé (`to_number` — un `valueOf`
/// ré-entrant peut toucher le miroir : toujours **avant** le
/// `borrow_mut` qui pousse l'op).
fn fnum(e: &mut Engine, args: &[Value], i: usize) -> JsResult<f32> {
    Ok(e.to_number(arg(args, i))? as f32)
}

impl Host for Ctx2dHost {
    fn class_name(&self) -> &'static str {
        "CanvasRenderingContext2D"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn get(&self, e: &mut Engine, _this: Value, key: &str) -> JsResult<Option<Value>> {
        match key {
            "canvas" => Ok(Some(node_value(e, &self.dom, &self.cache, self.node))),
            "fillStyle" | "strokeStyle" => {
                let c = self
                    .dom
                    .borrow()
                    .canvases
                    .get(&self.node)
                    .map_or((0, 0, 0, 255), |s| {
                        if key == "fillStyle" {
                            s.fill
                        } else {
                            s.stroke
                        }
                    });
                Ok(Some(e.str_val(css_color_string(c))))
            }
            "globalAlpha" => {
                let a = self
                    .dom
                    .borrow()
                    .canvases
                    .get(&self.node)
                    .map_or(1.0, |s| s.alpha);
                Ok(Some(Value::Num(f64::from(a))))
            }
            "lineWidth" => {
                let w = self
                    .dom
                    .borrow()
                    .canvases
                    .get(&self.node)
                    .map_or(1.0, |s| s.line_w);
                Ok(Some(Value::Num(f64::from(w))))
            }
            _ => Ok(None),
        }
    }

    fn set(&self, e: &mut Engine, _this: Value, key: &str, v: Value) -> JsResult<bool> {
        match key {
            // Spec : valeur illisible → assignation ignorée
            // (pas d'exception, pas d'op enregistrée).
            "fillStyle" | "strokeStyle" => {
                let s = e.to_js_string(v)?;
                if let Some((r, g, b, a)) = parse_css_color(&s) {
                    let mut d = self.dom.borrow_mut();
                    {
                        let st = d.canvas_state(self.node);
                        if key == "fillStyle" {
                            st.fill = (r, g, b, a);
                        } else {
                            st.stroke = (r, g, b, a);
                        }
                    }
                    d.push_canvas_op(
                        self.node,
                        if key == "fillStyle" {
                            CanvasOpRecord::FillColor { r, g, b, a }
                        } else {
                            CanvasOpRecord::StrokeColor { r, g, b, a }
                        },
                    );
                }
                Ok(true)
            }
            // Spec : hors `[0, 1]` → assignation ignorée.
            "globalAlpha" => {
                let x = e.to_number(v)?;
                if (0.0..=1.0).contains(&x) {
                    #[allow(clippy::cast_possible_truncation)]
                    let x = x as f32;
                    let mut d = self.dom.borrow_mut();
                    d.canvas_state(self.node).alpha = x;
                    d.push_canvas_op(self.node, CanvasOpRecord::GlobalAlpha(x));
                }
                Ok(true)
            }
            // Spec : non fini ou ≤ 0 → assignation ignorée.
            "lineWidth" => {
                let x = e.to_number(v)?;
                if x.is_finite() && x > 0.0 {
                    #[allow(clippy::cast_possible_truncation)]
                    let x = x as f32;
                    let mut d = self.dom.borrow_mut();
                    d.canvas_state(self.node).line_w = x;
                    d.push_canvas_op(self.node, CanvasOpRecord::LineWidth(x));
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    #[allow(clippy::cast_possible_truncation, clippy::too_many_lines)]
    fn invoke(
        &self,
        e: &mut Engine,
        _this: Value,
        name: &str,
        args: &[Value],
    ) -> JsResult<Option<Value>> {
        use CanvasOpRecord as R;
        let op = match name {
            "save" => R::Save,
            "restore" => R::Restore,
            "beginPath" => R::BeginPath,
            "closePath" => R::ClosePath,
            "fill" => R::Fill,
            "stroke" => R::Stroke,
            "clip" => R::Clip,
            "translate" => R::Translate(fnum(e, args, 0)?, fnum(e, args, 1)?),
            "scale" => R::Scale(fnum(e, args, 0)?, fnum(e, args, 1)?),
            "rotate" => R::Rotate(fnum(e, args, 0)?),
            "transform" => R::Transform(
                fnum(e, args, 0)?,
                fnum(e, args, 1)?,
                fnum(e, args, 2)?,
                fnum(e, args, 3)?,
                fnum(e, args, 4)?,
                fnum(e, args, 5)?,
            ),
            "setTransform" => R::SetTransform(
                fnum(e, args, 0)?,
                fnum(e, args, 1)?,
                fnum(e, args, 2)?,
                fnum(e, args, 3)?,
                fnum(e, args, 4)?,
                fnum(e, args, 5)?,
            ),
            "moveTo" => R::MoveTo(fnum(e, args, 0)?, fnum(e, args, 1)?),
            "lineTo" => R::LineTo(fnum(e, args, 0)?, fnum(e, args, 1)?),
            "quadraticCurveTo" => R::QuadraticTo(
                fnum(e, args, 0)?,
                fnum(e, args, 1)?,
                fnum(e, args, 2)?,
                fnum(e, args, 3)?,
            ),
            "bezierCurveTo" => R::BezierTo(
                fnum(e, args, 0)?,
                fnum(e, args, 1)?,
                fnum(e, args, 2)?,
                fnum(e, args, 3)?,
                fnum(e, args, 4)?,
                fnum(e, args, 5)?,
            ),
            "arc" => R::Arc(
                fnum(e, args, 0)?,
                fnum(e, args, 1)?,
                fnum(e, args, 2)?,
                fnum(e, args, 3)?,
                fnum(e, args, 4)?,
                e.to_boolean(arg(args, 5)),
            ),
            "rect" => R::Rect(
                fnum(e, args, 0)?,
                fnum(e, args, 1)?,
                fnum(e, args, 2)?,
                fnum(e, args, 3)?,
            ),
            "fillRect" => R::FillRect(
                fnum(e, args, 0)?,
                fnum(e, args, 1)?,
                fnum(e, args, 2)?,
                fnum(e, args, 3)?,
            ),
            "strokeRect" => R::StrokeRect(
                fnum(e, args, 0)?,
                fnum(e, args, 1)?,
                fnum(e, args, 2)?,
                fnum(e, args, 3)?,
            ),
            "clearRect" => R::ClearRect(
                fnum(e, args, 0)?,
                fnum(e, args, 1)?,
                fnum(e, args, 2)?,
                fnum(e, args, 3)?,
            ),
            // APIs hors MVP R-11 avec valeur de retour
            // plausible — absorbées, jamais de TypeError.
            "measureText" => {
                let o = e.new_object();
                e.set_own(o, "width", Value::Num(0.0), true);
                return Ok(Some(Value::Obj(o)));
            }
            "getLineDash" => return Ok(Some(e.new_array(Vec::new()))),
            "isPointInPath" | "isPointInStroke" => return Ok(Some(Value::Bool(false))),
            "createLinearGradient"
            | "createRadialGradient"
            | "createConicGradient"
            | "createPattern" => {
                return Ok(Some(e.new_host_object(Rc::new(GradientHost))));
            }
            "getImageData" => {
                let w = fnum(e, args, 2)?.max(0.0);
                let h = fnum(e, args, 3)?.max(0.0);
                let o = e.new_object();
                e.set_own(o, "width", Value::Num(f64::from(w)), true);
                e.set_own(o, "height", Value::Num(f64::from(h)), true);
                let data = e.new_array(Vec::new());
                e.set_own(o, "data", data, true);
                return Ok(Some(Value::Obj(o)));
            }
            _ if NOOP_CTX_METHODS.contains(&name) => return Ok(Some(Value::Undefined)),
            _ => return Ok(None),
        };
        self.dom.borrow_mut().push_canvas_op(self.node, op);
        Ok(Some(Value::Undefined))
    }
}

impl Host for GradientHost {
    fn class_name(&self) -> &'static str {
        "CanvasGradient"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn invoke(
        &self,
        _e: &mut Engine,
        _this: Value,
        name: &str,
        _args: &[Value],
    ) -> JsResult<Option<Value>> {
        if name == "addColorStop" {
            return Ok(Some(Value::Undefined));
        }
        Ok(None)
    }
}

/// `rgba8` — la forme couleur des op-lists canvas (JS-C28).
type Rgba8 = (u8, u8, u8, u8);

/// Couleur courante → chaîne normalisée du getter
/// (`#rrggbb` si opaque, `rgba(…)` sinon — forme spec).
fn css_color_string((r, g, b, a): Rgba8) -> String {
    if a == 255 {
        format!("#{r:02x}{g:02x}{b:02x}")
    } else {
        format!("rgba({r}, {g}, {b}, {:.3})", f64::from(a) / 255.0)
    }
}

/// Parse couleur CSS pour `fillStyle`/`strokeStyle` (JS-C28)
/// : `#rgb[a]`, `#rrggbb[aa]`, `rgb[a](…)` (entiers ou `%`,
/// alpha `0–1` ou `%`), `transparent`, noms usuels. Hors MVP :
/// `hsl()`/`hwb()`/`color()`/dégradés — `None` = assignation
/// ignorée (spec).
fn parse_css_color(s: &str) -> Option<Rgba8> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        let h = hex.as_bytes();
        let dig = |i: usize| (h.get(i).copied()? as char).to_digit(16).map(|d| d as u8);
        return match h.len() {
            3 | 4 => Some((
                dig(0)? * 17,
                dig(1)? * 17,
                dig(2)? * 17,
                if h.len() == 4 { dig(3)? * 17 } else { 255 },
            )),
            6 | 8 => Some((
                dig(0)? * 16 + dig(1)?,
                dig(2)? * 16 + dig(3)?,
                dig(4)? * 16 + dig(5)?,
                if h.len() == 8 {
                    dig(6)? * 16 + dig(7)?
                } else {
                    255
                },
            )),
            _ => None,
        };
    }
    let fn_body = s
        .strip_prefix("rgba(")
        .or_else(|| s.strip_prefix("rgb("))
        .and_then(|t| t.strip_suffix(')'));
    if let Some(inner) = fn_body {
        return parse_rgb_fn(inner);
    }
    if s.eq_ignore_ascii_case("transparent") {
        return Some((0, 0, 0, 0));
    }
    NAMED_COLORS
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(s))
        .map(|(_, c)| *c)
}

/// `rgb()`/`rgba()` — composantes `0–255` ou `%`, alpha
/// `0–1` ou `%` ; formes virgule et espace (`/`) acceptées.
fn parse_rgb_fn(inner: &str) -> Option<Rgba8> {
    let (comps, alpha) = match inner.split_once('/') {
        Some((c, a)) => (c, Some(a.trim())),
        None => (inner, None),
    };
    let toks: Vec<&str> = if comps.contains(',') {
        let t: Vec<&str> = comps.split(',').map(str::trim).collect();
        let (rgb, a) = t.split_at(t.len().min(3));
        let alpha = alpha.or_else(|| a.first().copied());
        return toks_color(rgb, alpha);
    } else {
        comps.split_whitespace().collect()
    };
    toks_color(&toks, alpha)
}

fn toks_color(rgb: &[&str], alpha: Option<&str>) -> Option<Rgba8> {
    if rgb.len() != 3 {
        return None;
    }
    Some((
        css_chan(rgb[0])?,
        css_chan(rgb[1])?,
        css_chan(rgb[2])?,
        alpha.map_or(Some(255), css_alpha)?,
    ))
}

/// Composante `0–255` ou `%`.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn css_chan(t: &str) -> Option<u8> {
    if let Some(p) = t.strip_suffix('%') {
        return Some((p.parse::<f32>().ok()? * 2.55).round().clamp(0.0, 255.0) as u8);
    }
    Some(t.parse::<f32>().ok()?.round().clamp(0.0, 255.0) as u8)
}

/// Alpha `0–1` ou `%`.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn css_alpha(t: &str) -> Option<u8> {
    if let Some(p) = t.strip_suffix('%') {
        return Some((p.parse::<f32>().ok()? * 2.55).round().clamp(0.0, 255.0) as u8);
    }
    Some((t.parse::<f32>().ok()? * 255.0).round().clamp(0.0, 255.0) as u8)
}

/// `(nom, rgba8)` d'une couleur nommée.
type NamedColor = (&'static str, Rgba8);

/// Noms usuels (niveau CSS 1 + fréquents du web réel) —
/// la table complète (~150) est un chantier PIPE, pas ici.
const NAMED_COLORS: &[NamedColor] = &[
    ("black", (0, 0, 0, 255)),
    ("white", (255, 255, 255, 255)),
    ("red", (255, 0, 0, 255)),
    ("lime", (0, 255, 0, 255)),
    ("green", (0, 128, 0, 255)),
    ("blue", (0, 0, 255, 255)),
    ("yellow", (255, 255, 0, 255)),
    ("cyan", (0, 255, 255, 255)),
    ("aqua", (0, 255, 255, 255)),
    ("magenta", (255, 0, 255, 255)),
    ("fuchsia", (255, 0, 255, 255)),
    ("gray", (128, 128, 128, 255)),
    ("grey", (128, 128, 128, 255)),
    ("silver", (192, 192, 192, 255)),
    ("maroon", (128, 0, 0, 255)),
    ("olive", (128, 128, 0, 255)),
    ("purple", (128, 0, 128, 255)),
    ("teal", (0, 128, 128, 255)),
    ("navy", (0, 0, 128, 255)),
    ("orange", (255, 165, 0, 255)),
    ("brown", (165, 42, 42, 255)),
    ("pink", (255, 192, 203, 255)),
    ("gold", (255, 215, 0, 255)),
    ("rebeccapurple", (102, 51, 153, 255)),
];
