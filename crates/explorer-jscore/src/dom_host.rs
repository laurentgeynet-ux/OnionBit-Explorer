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

/// `history.pushState`/`replaceState` hook (CAP-151/158) :
/// `(url_brute, replace, state_json)`.
pub type PushStateHook = Rc<dyn Fn(String, bool, Option<String>)>;

/// `window.open(url, target)` hook (CAP-153/159) :
/// `(url, target)` — `""` = contexte neuf sans nom ; un nom
/// désigne un contexte existant à réutiliser (spec — les
/// popups OAuth `window.open(u, "auth")` en dépendent).
pub type OpenHook = Rc<dyn Fn(String, String)>;

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
    /// `formtarget` du submitter ou `target` du `<form>`
    /// (CAP-159) — `""` = contexte courant ; `_blank`/nommé
    /// = contexte neuf ou nommé à réutiliser.
    pub target: String,
}

/// `<script>` inséré dynamiquement devenu attaché (CAP-169) —
/// `createElement('script')`/`innerHTML`-style + `appendChild`
/// & co : la session fetch/évalue puis livre via
/// [`Engine::deliver_dyn_script`] (`load`/`error` sur
/// l'élément). Force-async spec : évalué dès la réponse.
#[derive(Debug, Clone)]
pub struct DynScriptReq {
    /// Identifiant d'émission — répercuté à la livraison.
    pub seq: u64,
    /// `src` brut si présent/non vide — la session résout
    /// contre la base du document émetteur et applique
    /// CSP/mixed-content ; `None` = script inline (`text`).
    pub src: Option<String>,
    /// Corps inline (`textContent`) quand `src` est absent.
    pub text: Option<String>,
    /// Attribut `crossorigin` (même mapping que CAP-167) —
    /// `None` = `no-cors`, `Some` = `mode:cors` + credentials.
    pub crossorigin: Option<crate::xhr::FetchCredentials>,
    /// Attribut `integrity` (SRI, CAP-163) — vérifié côté
    /// session avant acceptation du corps.
    pub integrity: Option<String>,
    /// Attribut `referrerpolicy` brut — prime la politique
    /// du document (étape 132).
    pub referrerpolicy: Option<String>,
    /// Attribut `charset` brut (étape 53b).
    pub charset: Option<String>,
    /// Attribut `nonce` — ouvre `script-src` CSP inline pour
    /// les scripts dynamiques sans `src` (CAP-145).
    pub nonce: Option<String>,
    /// `type="module"` (CAP-186) — la livraison passe par
    /// `eval_module_graph` + `deliver_dyn_module`, jamais
    /// l'éval classique (un `import`/`export` top-level y
    /// serait un `SyntaxError` avalé — mur réel x-web).
    pub module: bool,
    /// Contexte émetteur (`JsCtxId`) — la livraison et
    /// `currentScript` visent son miroir, jamais le parent.
    pub ctx: crate::context::CtxId,
}

/// CAP-171 — intention de chargement d'un `<iframe>` inséré
/// dynamiquement (ou re-navigué par `src=`), drainée par
/// [`crate::interp::Engine::drain_dyn_frames`] et servie par
/// l'embedder : fetch `src` (CSP `frame-src`, XFO/
/// `frame-ancestors`, cookies/partition) ou `srcdoc` direct,
/// puis `context_new` + `install_dom_ctx` dans le realm
/// émetteur. Le moteur ne touche jamais le réseau.
pub struct DynFrameReq {
    /// Identifiant d'émission — répercuté à la livraison
    /// (`deliver_dyn_frame` retrouve le nœud pour
    /// `load`/`error`).
    pub seq: u64,
    /// Index du nœud parmi les `<iframe>` du miroir émetteur
    /// (ordre document) — alimente `context_new(frame_ord)`
    /// pour `contentWindow`/`frameElement`.
    pub ord: u32,
    /// Attribut `src` brut — la session résout contre la
    /// base du document émetteur. `None` quand `srcdoc`.
    pub src: Option<String>,
    /// Attribut `srcdoc` — prime sur `src` (spec, CAP-142).
    pub srcdoc: Option<String>,
    /// Attribut `sandbox` brut — `SandboxFlags` côté
    /// session (CAP-149).
    pub sandbox: Option<String>,
    /// Attribut `referrerpolicy` brut — prime la politique
    /// du document (étape 132).
    pub referrerpolicy: Option<String>,
    /// Contexte émetteur (`JsCtxId`) — le sous-contexte est
    /// créé enfant de lui ; la livraison vise son miroir.
    pub ctx: crate::context::CtxId,
}

/// CAP-172 — intention de chargement d'un `<img>` créé par
/// script (ou `src`/`srcset`/`sizes` modifié), drainée par
/// [`crate::interp::Engine::drain_dyn_imgs`] et servie par
/// l'embedder : fetch `src`/candidat `srcset` (CSP `img-src`,
/// mixed-content, `crossorigin` CORS, `referrerpolicy`) ou
/// décodage `data:` direct, puis `load`/`error` sur
/// l'élément — attaché **ou détaché** (`new Image().src=`
/// = prefetch, spec « update the image data »). Le moteur
/// ne touche jamais le réseau.
pub struct DynImgReq {
    /// Identifiant d'émission — répercuté à la livraison
    /// (`deliver_dyn_img` retrouve le nœud pour
    /// `load`/`error`).
    pub seq: u64,
    /// Attribut `src` brut — la session résout contre la
    /// base du document émetteur (`data:` servi sans fetch).
    pub src: Option<String>,
    /// Attribut `srcset` brut — prime sur `src` quand
    /// présent (candidat choisi par `sizes`/dpr côté
    /// session, même pile que `collect_images`).
    pub srcset: Option<String>,
    /// Attribut `sizes` brut — pilote le choix `srcset`.
    pub sizes: Option<String>,
    /// Attribut `crossorigin` (même mapping que CAP-167) —
    /// `anonymous`/`use-credentials` → `Include`, vide ou
    /// valeur nue → `SameOrigin` (mode CORS).
    pub crossorigin: Option<crate::xhr::FetchCredentials>,
    /// Attribut `referrerpolicy` brut — prime la politique
    /// du document (étape 132).
    pub referrerpolicy: Option<String>,
    /// Contexte émetteur (`JsCtxId`) — la résolution de
    /// base et la livraison visent son miroir.
    pub ctx: crate::context::CtxId,
}

/// CAP-173 — intention de chargement d'un
/// `<link rel=stylesheet>` inséré par script (ou `href`/
/// `rel` modifié sur lien attaché), drainée par
/// [`crate::interp::Engine::drain_dyn_links`] et servie par
/// l'embedder : fetch du `href` (CSP `style-src`,
/// mixed-content, `crossorigin` CORS, `referrerpolicy`,
/// SRI `integrity`) puis application à la cascade et
/// `load`/`error` sur l'élément — attaché requis (spec
/// « link processing » : un lien détaché ne charge pas).
/// Le moteur ne touche jamais le réseau.
pub struct DynLinkReq {
    /// Identifiant d'émission — répercuté à la livraison
    /// (`deliver_dyn_link` retrouve le nœud pour
    /// `load`/`error`).
    pub seq: u64,
    /// Attribut `href` brut — la session résout contre la
    /// base du document émetteur.
    pub href: String,
    /// Attribut `integrity` brut — SRI exigée à la
    /// livraison (CAP-163), même pile que les liens
    /// statiques.
    pub integrity: Option<String>,
    /// Attribut `media` brut — la feuille est appliquée
    /// même si le media ne matche pas (spec : le fetch a
    /// lieu, le style s'applique sous condition — la
    /// cascade réévalue à chaque env, borne historique de
    /// la pile statique).
    pub media: Option<String>,
    /// Attribut `crossorigin` (même mapping que CAP-167).
    pub crossorigin: Option<crate::xhr::FetchCredentials>,
    /// Attribut `referrerpolicy` brut — prime la politique
    /// du document (étape 132).
    pub referrerpolicy: Option<String>,
    /// Contexte émetteur (`JsCtxId`).
    pub ctx: crate::context::CtxId,
}

/// Browser-surface hooks (JS-7e) — everything `window.location`,
/// `history` and `document.cookie` need, injected by the embedder so
/// the engine never navigates or touches the cookie store itself.
#[derive(Default)]
pub(crate) struct BrowserHooks {
    /// Absolute URL of the loaded page (`location.href` source).
    pub page_url: String,
    /// `document.referrer` (CAP-156) — le `Referer`
    /// effectivement envoyé par la navigation du document
    /// (`""` hors navigation suivie d'un referrer). Posé par
    /// l'embedder via [`Engine::set_page_referrer`] — le
    /// moteur ne le déduit jamais lui-même.
    pub page_referrer: String,
    /// `document.currentScript` (CAP-168) — nœud `<script>`
    /// en cours d'évaluation, posé autour de
    /// [`Engine::eval_script_el`] (null hors éval, spec ;
    /// jamais posé pour les modules — spec `currentScript`
    /// = scripts classiques uniquement).
    pub current_script: Option<NodeId>,
    /// `seq` → nœud miroir des `<script>` dynamiques en vol
    /// (CAP-169) — pour `load`/`error` et `currentScript` à
    /// la livraison. Borné : au-delà de 256 en vol, l'entrée
    /// la plus ancienne est abandonnée (jamais de croissance
    /// hostiles).
    pub dyn_script_nodes: std::collections::BTreeMap<u64, NodeId>,
    /// `seq` → nœud miroir des `<iframe>` dynamiques en vol
    /// (CAP-171) — pour `load`/`error` à la livraison. Même
    /// borne que `dyn_script_nodes`.
    pub dyn_frame_nodes: std::collections::BTreeMap<u64, NodeId>,
    /// `seq` → nœud miroir des `<img>` dynamiques en vol
    /// (CAP-172) — pour `load`/`error` à la livraison. Même
    /// borne que `dyn_script_nodes` ; les images détachées
    /// sont livrées aussi (`new Image().src=` = prefetch).
    pub dyn_img_nodes: std::collections::BTreeMap<u64, NodeId>,
    /// `seq` → nœud miroir des `<link>` dynamiques en vol
    /// (CAP-173) — pour `load`/`error` à la livraison. Même
    /// borne que `dyn_script_nodes`.
    pub dyn_link_nodes: std::collections::BTreeMap<u64, NodeId>,
    /// Slots `<dialog>` (CAP-176) : `showModal()` pose `open`
    /// et le flag modal ; `close()` efface les deux et tire
    /// `close`. `returnValue` = slot spec (pas un attribut —
    /// il n'en est pas un). Document-local — garé/restauré
    /// au `switch_to`.
    pub dialog_modal: std::collections::BTreeSet<NodeId>,
    pub dialog_retval: std::collections::BTreeMap<NodeId, String>,
    /// Fullscreen (CAP-179) — un seul élément fullscreen par
    /// document (spec) ; `None` hors fullscreen.
    /// Document-local — garé/restauré au `switch_to`.
    pub fullscreen_el: Option<NodeId>,
    /// Popovers ouverts (CAP-179) — état interne spec
    /// (`:popover-open` n'est **pas** un attribut reflété).
    /// Document-local.
    pub popover_open: std::collections::BTreeSet<NodeId>,
    /// Séquence d'émission des scripts dynamiques.
    pub dyn_script_seq: u64,
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
    /// `navigator.clipboard.writeText` (CAP-181) — l'embedder
    /// décide du presse-papiers (OS, sandbox, permission).
    /// Absent → la promesse rejette `NotAllowedError` (spec :
    /// permission refusée — honnête, jamais de faux succès).
    /// Engine-global (comme `console`) — le presse-papiers
    /// est une ressource plateforme, pas un slot document.
    pub clipboard_write: Option<ClipboardWriteHook>,
    /// `navigator.clipboard.readText` (CAP-181) — `Some` →
    /// texte, `None` → rejet `NotAllowedError`. Absent →
    /// même rejet que l'écriture.
    pub clipboard_read: Option<ClipboardReadHook>,
    /// Canal diagnostic (sondes) — `set_diag_hook` : émis
    /// quand un clone `{...}`/`Object.assign` perd des getters
    /// proto (mur `TanStack` `e.id` → `undefined` — signature
    /// spec mais source de bugs terrain : le walk reçoit des
    /// clones). Engine-global, off par défaut.
    pub diag: Option<Rc<dyn Fn(String)>>,
    /// `alert(msg)` — modal-less notification (fire-and-forget).
    pub alert: Option<Rc<dyn Fn(String)>>,
    /// `confirm(msg)` → `true`/`false`; absent → `false`.
    pub confirm: Option<Rc<dyn Fn(String) -> bool>>,
    /// `prompt(msg, default)` → user text; absent/cancel → `null`.
    pub prompt: Option<PromptHook>,
    /// `window.open(url, target)` — new context. Absent →
    /// same-tab `navigate` fallback.
    pub open: Option<OpenHook>,
    /// `form.submit()`/`requestSubmit()` — absent → no-op
    /// silencieux (le moteur ne navigue jamais seul).
    pub form_submit: Option<Rc<dyn Fn(FormSubmit)>>,
    /// `history.pushState`/`replaceState` — `(url_brute,
    /// replace, state_json)` ; l'embedder résout contre la
    /// base du document et commit URL + historique de
    /// session (CAP-151 ; CAP-158 : `state` est l'objet
    /// `args[0]` sérialisé JSON — `None` = `undefined`/
    /// non sérialisable). Absent → `page_url` remplacée
    /// à la brute (harnais sans session).
    pub push_state: Option<PushStateHook>,
    /// `history.state` (CAP-158) — JSON sérialisé de l'état
    /// de l'entrée courante : posé localement au
    /// `pushState` et restauré par l'embedder à la
    /// traversée (`sync_ctx_state`). `None` → `null`.
    /// Document-local comme `page_url` (swappé au
    /// `switch_to`).
    pub history_state: Option<String>,
    /// `window.name` du contexte (CAP-159) — nom du contexte
    /// top-level posé par l'embedder à l'installation
    /// (`set_ctx_name` — corrélation `target="x"`/
    /// `window.open(u,"x")` document-local, swappé au
    /// `switch_to`).
    pub win_name: String,
    /// `history.scrollRestoration` (CAP-166) — `true` =
    /// `"manual"` (le document gère sa restauration de
    /// scroll — les routeurs SPA le posent avant de
    /// restaurer eux-mêmes). Document-local, swappé au
    /// `switch_to`. La persistance de scroll par entrée
    /// n'existant pas encore, le drapeau est honnêtement
    /// stocké/relu — l'effet `auto` est le comportement
    /// actuel dans tous les cas (borne documentée).
    pub scroll_restore_manual: bool,
    /// Environnement `matchMedia` (CAP-161) — snapshot de la
    /// session posé par `set_media_env` au `spawn_engine` :
    /// viewport, `prefers-color-scheme`, `hover`/`pointer`.
    /// Engine-global (même env pour tous les contextes de
    /// l'onglet), borné — les `MediaQueryList` ne redispatchent
    /// jamais `change` (l'env ne bouge pas à chaud).
    pub media_env: crate::mediaq::MediaEnv,
    // NB — géométrie publiée (issue #19, JS-8) : **pas** de
    // champ ici, elle est **document-local** par contrat
    // (journal interface 2026-10-24) — le slot vit dans
    // `Dom::geometry`, posé par `Engine::set_geometry_ctx`
    // (meurt avec le miroir, swap gratuit, bfcache-sain).
    /// `location.replace(url)` — navigation `historyHandling:
    /// "replace"` : l'entrée d'historique courante est
    /// réécrite, pas poussée (CAP-152 — flows auth).
    /// Absent → `navigate` (comportement historique).
    pub navigate_replace: Option<Rc<dyn Fn(String)>>,
    /// `location.reload()` — rejeu de la requête de l'entrée
    /// courante sans entrée nouvelle (CAP-152). Absent →
    /// `navigate(page_url)` (comportement historique).
    pub navigate_reload: Option<Rc<dyn Fn()>>,
    /// `document.readyState` (CAP-170) — `"loading"`/
    /// `"interactive"`/`"complete"` du document, posé par
    /// l'embedder via [`Engine::set_ready_state`] au rythme
    /// de son cycle de vie (spec : `interactive` avant
    /// `DOMContentLoaded`, `complete` après `load`).
    /// Document-local (swappé au `switch_to`).
    pub ready_state: &'static str,
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
        // JS-C78 : l'URL du contexte **courant** sert l'origine
        // (`ctx_origin`/`same-origin`/`postMessage.targetOrigin`)
        // — la méta suit la page effectivement installée.
        if let Some(m) = self.ctxs.get_mut(self.cur_ctx as usize) {
            m.url = url.to_string();
        }
    }

    /// Install the cookie jar backing `document.cookie`.
    pub fn set_cookie_jar(&mut self, jar: Rc<dyn CookieJar>) {
        self.hooks.cookie_jar = Some(jar);
    }

    /// `document.referrer` (CAP-156) — le `Referer` envoyé
    /// par la navigation du document (`""` = aucun).
    pub fn set_page_referrer(&mut self, referrer: &str) {
        self.hooks.page_referrer = referrer.to_string();
    }

    /// Install navigation hooks: `navigate` for `location.*`,
    /// `history_go` + `history_len` for `history.*`.
    pub fn set_nav_hooks(&mut self, navigate: NavHook, history_go: HistHook, history_len: u32) {
        self.hooks.navigate = Some(navigate);
        self.hooks.history_go = Some(history_go);
        self.hooks.history_len = history_len;
    }

    /// Canal diagnostic pour les sondes (`dbg_*` du terrain) —
    /// `Some` → messages `diag:…` remontés à l'appelant ;
    /// absent → no-op. Pas une surface JS : interne moteur.
    pub fn set_diag_hook(&mut self, f: Rc<dyn Fn(String)>) {
        self.hooks.diag = Some(f);
    }

    /// Emit un message diag si le canal est posé — borrow
    /// sûr : le hook est cloné avant l'appel (re-entrance
    /// possible depuis un getter).
    pub(crate) fn diag(&self, msg: &str) {
        if let Some(h) = self.hooks.diag.clone() {
            h(format!("diag: {msg}"));
        }
    }

    /// Install the `console.*` line sink (formatted `"a b"` lines).
    /// `navigator.clipboard` (CAP-181) — l'embedder pose ses
    /// transporteurs presse-papiers ; sans eux les promesses
    /// rejettent `NotAllowedError` (permission refusée spec).
    pub fn set_clipboard_hooks(
        &mut self,
        write: Option<ClipboardWriteHook>,
        read: Option<ClipboardReadHook>,
    ) {
        self.hooks.clipboard_write = write;
        self.hooks.clipboard_read = read;
    }

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

    /// Install the `history.pushState`/`replaceState` hook —
    /// `(url, replace, state_json)` ; l'embedder résout et
    /// commit (URL + historique + état, CAP-151/158).
    /// Absent → URL locale brute, état non persisté.
    pub fn set_push_state_hook(&mut self, f: PushStateHook) {
        self.hooks.push_state = Some(f);
    }

    /// Install the `location.replace(url)`/`reload()` hooks
    /// (CAP-152) — replace réécrit l'entrée au lieu de
    /// pousser, reload rejoue la requête de l'entrée.
    /// Absents → repli `navigate` (comportement historique).
    pub fn set_nav_replace_reload_hooks(
        &mut self,
        replace: Rc<dyn Fn(String)>,
        reload: Rc<dyn Fn()>,
    ) {
        self.hooks.navigate_replace = Some(replace);
        self.hooks.navigate_reload = Some(reload);
    }

    /// Install the `window.open(url)` hook (new context).
    pub fn set_open_hook(&mut self, f: OpenHook) {
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

    /// Branche la table de géométrie publiée par la session
    /// (issue #19, JS-8) sur le **document du contexte `ctx`**
    /// — `getBoundingClientRect`/`offset*`/`client*`/`scroll*`
    /// la lisent en direct : aucun appel synchrone vers la
    /// session (le JS tourne sur son worker), la map est un
    /// **état pré-rempli** au commit paint. Document-local
    /// (`Dom::geometry` — swap ctx gratuit, bfcache-sain) ;
    /// à poser après `install_dom`/`install_dom_ctx` du
    /// contexte visé. `ctx` inconnu/sans DOM → no-op.
    pub fn set_geometry_ctx(
        &mut self,
        ctx: crate::context::CtxId,
        slot: crate::geom::GeometrySlot,
    ) {
        let dom = if ctx == self.cur_ctx {
            self.dom_ctx.as_ref().map(|(d, _)| d.clone())
        } else {
            self.parked
                .get(&ctx)
                .and_then(|p| p.dom_ctx.as_ref().map(|(d, _)| d.clone()))
        };
        if let Some(d) = dom {
            d.borrow_mut().geometry = Some(slot);
        }
    }
}

/// Node → JS wrapper cache: `getElementById('x') === getElementById('x')`
/// must hold, so a node maps to a single JS object for the engine's
/// lifetime.
pub(crate) type NodeCache = Rc<RefCell<HashMap<NodeId, ObjId>>>;

/// Trace GC (ADR-0008) du `NodeCache` — les wrappers y sont
/// retenus par `ObjId` : sans trace, le mark-sweep réclame un
/// wrapper toujours référencé par le cache et `node_value` rend
/// un `ObjId` mort (« `x` is not a function (on ?) »,
/// `cannot read … of Undefined` sur les pages réelles — trigger
/// jQuery `implementation.createHTMLDocument`, assemblee vol
/// 137).
pub(crate) fn trace_node_cache(cache: &NodeCache, out: &mut Vec<ObjId>) {
    out.extend(cache.borrow().values().copied());
}

/// `document` object.
struct DocumentHost {
    dom: DomRef,
    cache: NodeCache,
    /// `document.implementation` — objet unique par document
    /// (spec), paresseusement construit.
    implementation: RefCell<Option<Value>>,
    /// `document.fonts` — `FontFaceSet` unique par document
    /// (spec) : l'état setlike (`add`/`has`) exige une
    /// instance stable entre accès.
    fonts: RefCell<Option<Value>>,
    /// `document.domain` (issue #24) — suffixe enregistrable
    /// posé par le setter ; `None` = hostname courant.
    domain_override: RefCell<Option<String>>,
    /// `document.adoptedStyleSheets` (CAP-195) — tableau
    /// settable : `[]` paresseux posé à la première lecture
    /// (identité `===` stable spec), l'assignation remplace.
    /// Les feuilles adoptées ne sont PAS appliquées à la
    /// cascade (scoping = tranche 2) — la surface suffit aux
    /// polyfills qui étalent la liste (`[C, ...e.adopted…]`).
    adopted_sheets: RefCell<Option<Value>>,
}

/// Element/text node object (`this` carries the node id).
struct NodeHost {
    dom: DomRef,
    cache: NodeCache,
    node: NodeId,
}

impl NodeHost {
    /// (dom, nœud) pour les accesseurs de prototype IDL
    /// (JS-C82 — `dom_acc_get`/`dom_acc_set` lisent la valeur
    /// **brute** sans repasser par `Host::get`, faute de quoi
    /// un `defineProperty` accesseur propre sur l'élément —
    /// le tracker React `inputValueTracking` — rebouclerait).
    pub(crate) fn dom_node(&self) -> (DomRef, NodeId) {
        (self.dom.clone(), self.node)
    }
}

/// Props IDL porteuses d'un accesseur sur le prototype de la
/// classe — table bornée (JS-C82) : `getOwnPropertyDescriptor(
/// ctor.prototype, 'value')` doit rendre `{get,set}` pour que
/// le `inputValueTracking` de React s'arme.
fn dom_acc_props(class: &str) -> &'static [&'static str] {
    match class {
        "HTMLInputElement" => &[
            "value",
            "checked",
            "type",
            "defaultValue",
            "defaultChecked",
            "readOnly",
            "disabled",
            "name",
            "placeholder",
        ],
        "HTMLTextAreaElement" => &[
            "value",
            "defaultValue",
            "wrap",
            "readOnly",
            "disabled",
            "name",
            "placeholder",
        ],
        "HTMLSelectElement" => &["value", "disabled", "multiple", "name"],
        "HTMLOptionElement" => &["value", "selected", "disabled"],
        "HTMLButtonElement" => &["value", "type", "disabled", "name"],
        "HTMLFormElement" => &["method", "action", "enctype", "target", "name"],
        "HTMLImageElement" => &["src", "alt", "width", "height", "decoding", "loading"],
        "HTMLAnchorElement" => &["href", "target", "rel"],
        "HTMLIFrameElement" => &["src", "srcdoc", "sandbox", "loading"],
        "HTMLScriptElement" | "HTMLLinkElement" => &["src", "href", "type", "rel"],
        _ => &["id", "className", "title"],
    }
}

/// Attribut booléen reflété (`checked`, `disabled`…) — lecture
/// `has_attr`, écriture pose/retire.
fn dom_bool_attr(prop: &str) -> Option<&'static str> {
    Some(match prop {
        "checked" | "defaultChecked" => "checked",
        "selected" => "selected",
        "disabled" => "disabled",
        "readOnly" => "readonly",
        "multiple" => "multiple",
        "required" => "required",
        "hidden" => "hidden",
        "autofocus" => "autofocus",
        _ => return None,
    })
}

/// Lecture brute d'une prop IDL réfléchie — **sans** la voie
/// `Host::get` : les accesseurs de prototype doivent atteindre
/// l'attribut même quand l'élément porte un accesseur propre
/// (`defineProperty`, tracker React).
fn dom_raw_get(e: &mut Engine, dom: &DomRef, node: NodeId, prop: &str) -> JsResult<Value> {
    let d = dom.borrow();
    if let Some(attr) = dom_bool_attr(prop) {
        return Ok(Value::Bool(d.has_attr(node, attr)));
    }
    if prop == "value" {
        let v = d.attr(node, "value").map(str::to_string).or_else(|| {
            matches!(d.nodes[node as usize].tag.as_str(), "textarea" | "select")
                .then(|| control_value(&d, node))
        });
        return Ok(match v {
            Some(v) => e.str_val(v),
            None => e.str_val(""),
        });
    }
    if let Some(v) = enum_attr_default(&d, node, prop) {
        return Ok(e.str_val(v));
    }
    let attr = prop_to_attr(prop).unwrap_or(prop);
    // IDL URL-typé (`src`/`href` sur `<img>`/`<script>`/`<a>`…) :
    // la lecture résout contre `document.baseURI` (spec
    // `[[Get]]` URL — `getAttribute` reste brut). `new URL(
    // el.src)` échouait `Invalid URL` sur les src relatives
    // (lemonde `inline:5`, franceinfo `Invalid scheme`).
    if matches!(attr, "src" | "href") {
        if let Some(raw) = d.attr(node, attr) {
            let resolved = crate::xhr::resolve_url(e, raw).unwrap_or_else(|| raw.to_string());
            return Ok(e.str_val(resolved));
        }
    }
    let v = d.attr(node, attr).unwrap_or("").to_string();
    Ok(e.str_val(v))
}

/// Getter d'accesseur prototype — `bound[0]` = nom de prop,
/// `this` = l'élément (`desc.get.call(el)`).
fn dom_acc_get(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Some(Value::Str(m)) = args.first() else {
        return Ok(Value::Undefined);
    };
    let Some(prop) = e.heap.string(*m) else {
        return Ok(Value::Undefined);
    };
    let Value::Obj(id) = this else {
        return Ok(Value::Undefined);
    };
    let Some((dom, node)) = e.host_of(id).and_then(|h| {
        h.as_any()
            .downcast_ref::<NodeHost>()
            .map(NodeHost::dom_node)
    }) else {
        return Ok(Value::Undefined);
    };
    if !dom.borrow().node_valid(node) {
        return Ok(Value::Undefined);
    }
    dom_raw_get(e, &dom, node, &prop)
}

/// Setter d'accesseur prototype — `bound[0]` = nom de prop,
/// `args[1]` = valeur (`desc.set.call(el, v)`).
fn dom_acc_set(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Some(Value::Str(m)) = args.first() else {
        return Ok(Value::Undefined);
    };
    let Some(prop) = e.heap.string(*m) else {
        return Ok(Value::Undefined);
    };
    let v = arg(args, 1);
    let Value::Obj(id) = this else {
        return Ok(Value::Undefined);
    };
    let Some((dom, node)) = e.host_of(id).and_then(|h| {
        h.as_any()
            .downcast_ref::<NodeHost>()
            .map(NodeHost::dom_node)
    }) else {
        return Ok(Value::Undefined);
    };
    let mut d = dom.borrow_mut();
    if !d.node_valid(node) {
        return Ok(Value::Undefined);
    }
    let tag = d.nodes[node as usize].tag.clone();
    if let Some(attr) = dom_bool_attr(&prop) {
        if e.to_boolean(v) {
            d.set_attr(node, attr, "");
        } else {
            d.remove_attr(node, attr);
        }
        return Ok(Value::Undefined);
    }
    let attr = enum_attr_write_name(&tag, &prop)
        .or_else(|| prop_to_attr(&prop))
        .unwrap_or_else(|| Box::leak(prop.into_boxed_str()));
    let s = e.to_js_string(v)?;
    d.set_attr(node, attr, &s);
    Ok(Value::Undefined)
}

impl Engine {
    /// `el.constructor` typé (JS-C82) : objet fonction non
    /// constructible nommé `class` + `prototype` porteur des
    /// accesseurs IDL de `dom_acc_props` — la surface
    /// `getOwnPropertyDescriptor`/`defineProperty` complète que
    /// `inputValueTracking` (React) exploite. Cache rooté par
    /// nom de classe.
    pub(crate) fn dom_ctor(&mut self, class: &'static str) -> Value {
        if let Some(v) = self.dom_ctors.get(class) {
            return *v;
        }
        let pid = self.new_object();
        let proto = Value::Obj(pid);
        self.install_dom_idl_accessors(pid, class);
        let ctor = self.native_fn(class, illegal_ctor);
        if let Value::Obj(cid) = ctor {
            self.set_own(cid, "prototype", proto, false);
            self.set_own(pid, "constructor", ctor, false);
            self.add_root(cid);
            self.add_root(pid);
        }
        self.dom_ctors.insert(class.to_string(), ctor);
        ctor
    }

    /// Accesseurs IDL de `dom_acc_props` posés sur un proto
    /// (`set_accessor`) — partagé entre `dom_ctor` et les
    /// prototypes des ctors globaux : `el.constructor` rend
    /// le ctor global (identité spec `=== HTMLInputElement`),
    /// sa surface de descripteurs doit donc être la même
    /// (JS-C192 — sinon `inputValueTracking` React perçoit un
    /// proto nu).
    fn install_dom_idl_accessors(&mut self, pid: ObjId, class: &str) {
        for &prop in dom_acc_props(class) {
            let nm = self.str_val(prop);
            let g = self.bound_native("dom.get", dom_acc_get, vec![nm]);
            let s = self.bound_native("dom.set", dom_acc_set, vec![nm]);
            if let Some(o) = self.heap.obj_mut(pid) {
                o.set_accessor(prop, g, s, false);
            }
        }
    }
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

    /// Soumission pilotée partagée (`requestSubmit`,
    /// activation d'un bouton submit — CAP-153) :
    /// validation (`invalid` non annulable sur les fautifs,
    /// `novalidate`/`formnovalidate` exclus) → événement
    /// `submit` (bubbles + cancelable, non-composed) →
    /// `form_submit` si non annulé.
    fn submit_via(&self, e: &mut Engine, form: NodeId, submitter: Option<NodeId>) -> JsResult<()> {
        let ok = {
            let d = self.dom.borrow();
            let skip = d.has_attr(form, "novalidate")
                || submitter.is_some_and(|s| d.has_attr(s, "formnovalidate"));
            skip || {
                drop(d);
                self.check_validity_of(e, form)?
            }
        };
        if !ok {
            return Ok(());
        }
        // `submit` spec : bubbles + cancelable, non-composed.
        let state = make_event_state(
            "submit",
            form,
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
            if let Some(p) = self.submit_payload(form, submitter) {
                if let Some(h) = &e.hooks.form_submit {
                    h(p);
                }
            }
        }
        Ok(())
    }

    /// **Activation behavior** d'un clic scripté non annulé
    /// (CAP-153 — spec « run synthetic click event » :
    /// l'action par défaut suit le dispatch) :
    /// `<a href>` navigue (`target` ≠ `_self` → hook
    /// `open`), `input`/`button` de type submit/image
    /// soumettent le `<form>` ancêtre (même voie que
    /// `requestSubmit`, submitter = le bouton),
    /// `checkbox`/`radio` basculent `checked` (+`input`/
    /// `change`, radio décoche son groupe `name`),
    /// `<label>` délègue à son contrôle (`for=` ou premier
    /// descendant activable), `<summary>` bascule `open`
    /// du `<details>` parent. `disabled` = aucune
    /// activation (spec). Dégradation : pas de `details`
    /// toggle event, `<input type=reset>`/fichier/couleur
    /// inertes à cette tranche.
    /// `el.focus()` (CAP-162 — spec « focus update steps »,
    /// borné) : non focalisable ou déjà focalisé → no-op ;
    /// sinon `blur` puis `focusout` sur l'ancien, focus posé,
    /// `focus` puis `focusin` sur le nouveau (ordre Chrome/
    //  spec). Pas de scroll-into-view — borné.
    fn focus_move(&self, e: &mut Engine, node: NodeId) -> JsResult<()> {
        if !self.dom.borrow().is_focusable(node) {
            return Ok(());
        }
        let old = self.dom.borrow().focused.get();
        if old == Some(node) {
            return Ok(());
        }
        if let Some(o) = old {
            self.dom.borrow_mut().focused.set(None);
            e.dispatch_simple(&self.dom, &self.cache, o, "blur")?;
            e.dispatch_simple(&self.dom, &self.cache, o, "focusout")?;
        }
        self.dom.borrow_mut().focused.set(Some(node));
        e.dispatch_simple(&self.dom, &self.cache, node, "focus")?;
        e.dispatch_simple(&self.dom, &self.cache, node, "focusin")?;
        Ok(())
    }

    /// `el.blur()` (CAP-162) — retire le focus si l'élément
    /// le détient (`blur` + `focusout`), no-op sinon.
    fn blur_el(&self, e: &mut Engine, node: NodeId) -> JsResult<()> {
        if self.dom.borrow().focused.get() != Some(node) {
            return Ok(());
        }
        self.dom.borrow_mut().focused.set(None);
        e.dispatch_simple(&self.dom, &self.cache, node, "blur")?;
        e.dispatch_simple(&self.dom, &self.cache, node, "focusout")?;
        Ok(())
    }

    fn run_activation(&self, e: &mut Engine, node: NodeId) -> JsResult<()> {
        let d = self.dom.borrow();
        let tag = d.nodes[node as usize].tag.clone();
        match tag.as_str() {
            "a" | "area" => {
                let Some(href) = d.attr(node, "href").map(str::to_string) else {
                    return Ok(());
                };
                if href.is_empty() {
                    return Ok(());
                }
                let tgt = d.attr(node, "target").unwrap_or("").to_ascii_lowercase();
                let tgt_raw = d.attr(node, "target").unwrap_or("").to_string();
                drop(d);
                if tgt.is_empty() || tgt == "_self" {
                    if let Some(nav) = e.hooks.navigate.clone() {
                        nav(&href);
                    }
                } else if let Some(f) = e.hooks.open.clone() {
                    // `target` nommé/`_blank` → nouveau
                    // contexte si le hook existe (sinon
                    // intention morte — dégradation) ; le
                    // nom passe **brut** (les noms de
                    // contexte sont sensibles à la casse —
                    // seuls les réservés `_x` ne le sont pas).
                    f(href, tgt_raw);
                }
            }
            "input" => {
                let ty = d.attr(node, "type").unwrap_or("text").to_ascii_lowercase();
                if d.attr(node, "disabled").is_some() {
                    return Ok(());
                }
                match ty.as_str() {
                    "checkbox" => {
                        let on = d.attr(node, "checked").is_some();
                        drop(d);
                        if on {
                            self.dom.borrow_mut().remove_attr(node, "checked");
                        } else {
                            self.dom.borrow_mut().set_attr(node, "checked", "");
                        }
                        // Spec : bascule → `input` puis `change`.
                        e.dispatch_simple(&self.dom, &self.cache, node, "input")?;
                        e.dispatch_simple(&self.dom, &self.cache, node, "change")?;
                    }
                    "radio" => {
                        // Groupe = même `name` sous le même
                        // `<form>` (spec « radio button
                        // group ») — les pairs perdent
                        // `checked`, self le gagne.
                        let name = d.attr(node, "name").map(str::to_string);
                        let form = d
                            .ancestors(node)
                            .iter()
                            .copied()
                            .find(|&a| d.nodes[a as usize].tag == "form");
                        let mut peers = Vec::new();
                        if let Some(n) = name.as_deref().filter(|n| !n.is_empty()) {
                            for (i, nd) in d.nodes.iter().enumerate() {
                                let id = i as NodeId;
                                if id != node
                                    && nd.tag == "input"
                                    && d.attr(id, "type")
                                        .is_some_and(|t| t.eq_ignore_ascii_case("radio"))
                                    && d.attr(id, "name") == Some(n)
                                    && d.ancestors(id)
                                        .iter()
                                        .copied()
                                        .find(|&a| d.nodes[a as usize].tag == "form")
                                        == form
                                {
                                    peers.push(id);
                                }
                            }
                        }
                        drop(d);
                        for p in peers {
                            self.dom.borrow_mut().remove_attr(p, "checked");
                        }
                        self.dom.borrow_mut().set_attr(node, "checked", "");
                        e.dispatch_simple(&self.dom, &self.cache, node, "input")?;
                        e.dispatch_simple(&self.dom, &self.cache, node, "change")?;
                    }
                    "submit" | "image" => {
                        let form = d
                            .ancestors(node)
                            .iter()
                            .copied()
                            .find(|&a| d.nodes[a as usize].tag == "form");
                        drop(d);
                        if let Some(f) = form {
                            self.submit_via(e, f, Some(node))?;
                        }
                    }
                    _ => {}
                }
            }
            "button" => {
                let ty = d
                    .attr(node, "type")
                    .unwrap_or("submit")
                    .to_ascii_lowercase();
                if d.attr(node, "disabled").is_some() {
                    return Ok(());
                }
                if ty == "submit" {
                    let form = d
                        .ancestors(node)
                        .iter()
                        .copied()
                        .find(|&a| d.nodes[a as usize].tag == "form");
                    drop(d);
                    if let Some(f) = form {
                        self.submit_via(e, f, Some(node))?;
                    }
                }
            }
            "label" => {
                // `for=` → contrôle par id ; sinon premier
                // descendant activable (spec « labeled
                // control » — sous-ensemble : les contrôles
                // form). Le contrôle reçoit le clic complet
                // (listeners + sa propre activation).
                let ctrl = d.attr(node, "for").and_then(|id| d.by_id(id));
                let ctrl = ctrl.or_else(|| first_control(&d, node));
                drop(d);
                if let Some(c) = ctrl {
                    let (bubbles, cancelable, composed) = platform_event_flags("click");
                    let st = make_event_state(
                        "click",
                        c,
                        EventFlags {
                            bubbles,
                            cancelable,
                            composed,
                            trusted: false,
                        },
                        Vec::new(),
                    );
                    e.propagate_event(&self.dom, &self.cache, &st)?;
                    if !st.prevented.get() {
                        self.run_activation(e, c)?;
                    }
                }
            }
            "summary" => {
                let det = d
                    .ancestors(node)
                    .iter()
                    .copied()
                    .find(|&a| d.nodes[a as usize].tag == "details");
                let on = det.is_some_and(|dt| d.attr(dt, "open").is_some());
                drop(d);
                if let Some(det) = det {
                    if on {
                        self.dom.borrow_mut().remove_attr(det, "open");
                    } else {
                        self.dom.borrow_mut().set_attr(det, "open", "");
                    }
                }
            }
            _ => {}
        }
        Ok(())
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
            // `formtarget` du submitter prime `target` du
            // `<form>` (spec — CAP-150 en version moteur).
            target: submitter
                .and_then(|s| d.attr(s, "formtarget"))
                .or_else(|| d.attr(node, "target"))
                .unwrap_or("")
                .to_string(),
        })
    }

    /// `prepend`/`before`/`after`/`replaceWith` args → node ids —
    /// host nodes pass through, strings become text nodes (spec).
    fn norm_children(&self, e: &mut Engine, args: &[Value]) -> JsResult<Vec<NodeId>> {
        let mut out = Vec::with_capacity(args.len());
        for &a in args {
            if let Some(n) =
                node_for_in(e, a, &self.dom).or_else(|| adopt_into(e, a, &self.dom, &self.cache))
            {
                out.push(n);
            } else if node_for(e, a).is_some() {
                // Nœud d'une autre arène non adoptable (id invalide)
                // — TypeError plutôt que texte fantaisiste inséré.
                return Err(e.type_error("node is from a different document"));
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
        // Spec : position ASCII case-insensitive — jQuery écrit
        // `"beforeEnd"` (jvc vol 114 : `bad position` tuait la
        // page d'accueil).
        let pos_lower = pos.to_ascii_lowercase();
        let pos = pos_lower.as_str();
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
        self.insert_at_position(e, node, pos, t)
    }

    /// Écriture `innerHTML`/`setHTMLUnsafe` (CAP-178 — rail
    /// factorisé) : `<template>` → `content` ; parse en
    /// fragment temporaire ; scripts « already started »
    /// (jamais exécutés — CAP-169, différence avec
    /// `insertAdjacentHTML` dont les scripts tournent).
    fn set_inner_html(&self, node: NodeId, html: &str) {
        let mut d = self.dom.borrow_mut();
        let target = d.content_target(node);
        let tmp = d.create_element("#fragment");
        d.parse_fragment_into(tmp, html);
        d.mark_scripts_started(tmp);
        let kids = std::mem::take(&mut d.nodes[tmp as usize].children);
        for &k in &kids {
            d.nodes[k as usize].parent = None;
        }
        d.replace_children(target, &kids);
    }

    /// Position switch partagé `insertAdjacent*` (CAP-177 —
    /// `insertAdjacentElement` joint le rail) : `beforebegin`/
    /// `afterbegin`/`beforeend`/`afterend`, `SyntaxError` hors
    /// jeu (spec).
    fn insert_at_position(
        &self,
        e: &mut Engine,
        node: NodeId,
        pos: &str,
        child: NodeId,
    ) -> JsResult<()> {
        // Position ASCII case-insensitive (spec — voir
        // `insert_adjacent`).
        let pos = pos.to_ascii_lowercase();
        match pos.as_str() {
            "beforebegin" => {
                if let Some(p) = dom_parent(&self.dom, node) {
                    self.dom.borrow_mut().insert_before(p, child, Some(node));
                }
            }
            "afterbegin" => {
                let first = self.dom.borrow().child_nodes(node).first().copied();
                self.dom.borrow_mut().insert_before(node, child, first);
            }
            "beforeend" => self.dom.borrow_mut().append(node, child),
            "afterend" => {
                if let Some(p) = dom_parent(&self.dom, node) {
                    let next = self.dom.borrow().sibling(node, true);
                    self.dom.borrow_mut().insert_before(p, child, next);
                }
            }
            _ => return Err(e.syntax_error("insertAdjacent: bad position")),
        }
        Ok(())
    }
}

/// `el.style` / `getComputedStyle(el)` — camelCase ↔ kebab-case
/// map over the `style` attr ; `computed` distingue les deux
/// surfaces : `el.style` ne rend que l'inline (spec), tandis que
/// `getComputedStyle` dispose du repli géométrie (CAP-196).
struct StyleHost {
    dom: DomRef,
    node: NodeId,
    /// `true` seulement pour l'objet rendu par
    /// `getComputedStyle` — autorise le repli cascade borné
    /// (`position` ← `ElementGeom.fixed` publié, i#19).
    computed: bool,
}

/// `el.classList`/`el.relList` — token ops over un attribut
/// à jetons (`class`, `rel` — spec `DOMTokenList` : mêmes
/// rails, le nom d'attribut seul change).
struct ClassListHost {
    dom: DomRef,
    node: NodeId,
    /// Attribut reflété — `"class"` pour `classList`, `"rel"`
    /// pour `relList` (link/a/area/form, spec `DOMTokenList`).
    attr: &'static str,
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
    /// `target` ré-étiqueté pour le listener courant (CAP-198
    /// — spec « retarget » : le nœud le plus profond encore
    /// visible depuis la racine du `currentTarget` ; hors
    /// dispatch, `target` s'affiche nu).
    adjusted: Cell<NodeId>,
    /// `relatedTarget` résolu en `NodeId` + version retargetée
    /// du listener courant (CAP-198 — même retarget que
    /// `target` ; `Host::get` prime les own-props sur les
    /// objets host, d'où la présentation par cellule).
    related_node: Cell<Option<NodeId>>,
    adjusted_related: Cell<Option<NodeId>>,
    /// Chemin de dispatch complet `[target, …ancêtres, …hôtes]`
    /// pour `composedPath()` (spec : la chaîne inclut les
    /// racines shadow ; vidé hors dispatch → `[]`).
    path: RefCell<Vec<NodeId>>,
    /// Origine du `timeStamp` — ms depuis la construction de
    /// l'état (Instant, pas epoch : la spec compte depuis la
    /// création de l'event relativement à l'origine temps).
    created: std::time::Instant,
    /// Props plateforme (`key`, `code`, `keyCode`…) — résolues
    /// par l'`EventHost` après les champs du cœur. `RefCell` :
    /// `initCustomEvent` pose `detail` après construction
    /// (DOM L2 — le payload arrive post-`createEvent`).
    props: RefCell<Vec<(String, Value)>>,
}

/// Flags d'initialisation d'un event (spec `EventInit` +
/// `isTrusted` interne — plateforme vs script).
#[allow(clippy::struct_excessive_bools)] // forme WebIDL `EventInit`.
#[derive(Clone, Copy)]
pub(crate) struct EventFlags {
    pub(crate) bubbles: bool,
    pub(crate) cancelable: bool,
    pub(crate) composed: bool,
    /// `isTrusted` (spec) — plateforme (`dom_event*`) vrai,
    /// script (`dispatchEvent`, `el.click()`) faux.
    pub(crate) trusted: bool,
}

/// État event neuf — `created` marque la « construction » pour
/// `timeStamp` (approximation honnête : l'état naît au dispatch,
/// quasi synchrone de la création JS dans les flux observés).
pub(crate) fn make_event_state(
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
        adjusted: Cell::new(target),
        related_node: Cell::new(None),
        adjusted_related: Cell::new(None),
        path: RefCell::new(Vec::new()),
        created: std::time::Instant::now(),
        props: RefCell::new(props),
    })
}

/// Premier descendant activable d'un `<label>` — contrôles
/// form seulement (`input`/`button`/`select`/`textarea`,
/// spec « labeled control ») : parcours préordre borné par
/// l'arbre miroir (CAP-153).
fn first_control(d: &crate::dom::Dom, node: NodeId) -> Option<NodeId> {
    for k in d.child_nodes(node) {
        if matches!(
            d.nodes[k as usize].tag.as_str(),
            "input" | "button" | "select" | "textarea"
        ) {
            return Some(k);
        }
        if let Some(n) = first_control(d, k) {
            return Some(n);
        }
    }
    None
}

/// Flags `(bubbles, cancelable, composed)` par défaut des
/// events plateforme (UI Events) — les interactifs (click,
/// keydown…) sont annulables et traversants ; les états de
/// focus, chargement et défilement ne le sont pas.
/// `focus`/`blur` ne bullent pas (spec — contrairement à
/// `focusin`/`focusout` et `focus` clavier) : la propagation
/// de `propagate_event` lit `ev.bubbles`.
pub(crate) fn platform_event_flags(ty: &str) -> (bool, bool, bool) {
    // `focus`/`blur` seuls passent non-bullants (CAP-162,
    // spec) — les autres types gardent leur historique
    // (`load`/`error`/`DOMContentLoaded` restent bullants
    // ici, borne documentée).
    const NON_BUBBLING: &[&str] = &["focus", "blur"];
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
        // `AnimationEvent`/`TransitionEvent` — spec : bullants,
        // non-annulables, non-composés.
        "animationstart",
        "animationiteration",
        "animationend",
        "animationcancel",
        "transitionrun",
        "transitionstart",
        "transitionend",
        "transitioncancel",
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
        "animationstart",
        "animationiteration",
        "animationend",
        "animationcancel",
        "transitionrun",
        "transitionstart",
        "transitionend",
        "transitioncancel",
    ];
    (
        !NON_BUBBLING.contains(&ty),
        !NON_CANCELABLE.contains(&ty),
        !NON_COMPOSED.contains(&ty),
    )
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
        // Garde GC : un `ObjId` balayé ne doit jamais être
        // rendu (zombie → « of Undefined » / « (on ?) ») — on
        // recrée le wrapper et on nettoie la entrée morte.
        if e.heap.obj(id).is_some() {
            return Value::Obj(id);
        }
        cache.borrow_mut().remove(&node);
    }
    let v = e.new_host_object(Rc::new(NodeHost {
        dom: dom.clone(),
        cache: cache.clone(),
        node,
    }));
    if let Value::Obj(id) = v {
        // `sr instanceof ShadowRoot` (JS-C159) — la racine
        // hérite `ShadowRoot.prototype` (proto lu sur le
        // ctor global, posé à `install_dom`).
        let kind = dom.borrow().nodes.get(node as usize).map(|n| n.kind);
        let is_sr = dom.borrow().is_shadow_root(node);
        let ctor = if is_sr {
            "ShadowRoot"
        } else {
            // `instanceof Element`/`Text`/`Comment`/`Node` —
            // chaîne plate par kind (J5 : impots/ameli — sans
            // proto, tout `x instanceof Element` était faux).
            // Éléments : interface tag-spécifique quand elle
            // existe (`<script>` → `HTMLScriptElement` — sephora
            // vol 121 : invariant Next.js
            // `currentScript instanceof HTMLScriptElement`) ;
            // tags hors table → `HTMLElement` (spec
            // `HTMLUnknownElement` non modélisé).
            match kind {
                Some(crate::dom::NodeKind::Element) => {
                    let tag = dom.borrow().nodes[node as usize].tag.clone();
                    tag_interface(&tag).unwrap_or("HTMLElement")
                }
                Some(crate::dom::NodeKind::Text) => "Text",
                Some(crate::dom::NodeKind::Comment) => "Comment",
                Some(crate::dom::NodeKind::Fragment) => "DocumentFragment",
                _ => "Node",
            }
        };
        let proto = e.get_prop(e.global, ctor).and_then(|c| match c {
            Value::Obj(cid) => e.get_prop(cid, "prototype"),
            _ => None,
        });
        if let Some(p @ Value::Obj(_)) = proto {
            if let Some(o) = e.heap.obj_mut(id) {
                o.proto = p;
            }
        }
        cache.borrow_mut().insert(node, id);
    }
    v
}

/// Tag HTML → interface DOM (spec `HTML*Element`) — les
/// `x instanceof HTML<Name>Element` réels (invariants Next.js,
/// feature-detects) exigent le proto tag-spécifique ; tags hors
/// table → `HTMLElement` (spec rend `HTMLUnknownElement`, non
/// modélisé — déviation documentée).
fn tag_interface(tag: &str) -> Option<&'static str> {
    Some(match tag {
        "a" => "HTMLAnchorElement",
        "area" => "HTMLAreaElement",
        "audio" => "HTMLAudioElement",
        "base" => "HTMLBaseElement",
        "body" => "HTMLBodyElement",
        "br" => "HTMLBRElement",
        "button" => "HTMLButtonElement",
        "canvas" => "HTMLCanvasElement",
        "data" => "HTMLDataElement",
        "datalist" => "HTMLDataListElement",
        "details" => "HTMLDetailsElement",
        "dialog" => "HTMLDialogElement",
        "div" => "HTMLDivElement",
        "dl" => "HTMLDListElement",
        "embed" => "HTMLEmbedElement",
        "fieldset" => "HTMLFieldSetElement",
        "form" => "HTMLFormElement",
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => "HTMLHeadingElement",
        "head" => "HTMLHeadElement",
        "hr" => "HTMLHRElement",
        "html" => "HTMLHtmlElement",
        "iframe" => "HTMLIFrameElement",
        "img" => "HTMLImageElement",
        "input" => "HTMLInputElement",
        "label" => "HTMLLabelElement",
        "legend" => "HTMLLegendElement",
        "li" => "HTMLLIElement",
        "link" => "HTMLLinkElement",
        "map" => "HTMLMapElement",
        "meta" => "HTMLMetaElement",
        "meter" => "HTMLMeterElement",
        "object" => "HTMLObjectElement",
        "ol" => "HTMLOListElement",
        "optgroup" => "HTMLOptGroupElement",
        "option" => "HTMLOptionElement",
        "output" => "HTMLOutputElement",
        "p" => "HTMLParagraphElement",
        "picture" => "HTMLPictureElement",
        "pre" => "HTMLPreElement",
        "progress" => "HTMLProgressElement",
        "q" | "blockquote" | "cite" => "HTMLQuoteElement",
        "script" => "HTMLScriptElement",
        "select" => "HTMLSelectElement",
        "slot" => "HTMLSlotElement",
        "source" => "HTMLSourceElement",
        "span" => "HTMLSpanElement",
        "style" => "HTMLStyleElement",
        "caption" => "HTMLTableCaptionElement",
        "td" | "th" => "HTMLTableCellElement",
        "col" | "colgroup" => "HTMLTableColElement",
        "table" => "HTMLTableElement",
        "tr" => "HTMLTableRowElement",
        "thead" | "tbody" | "tfoot" => "HTMLTableSectionElement",
        "template" => "HTMLTemplateElement",
        "textarea" => "HTMLTextAreaElement",
        "time" => "HTMLTimeElement",
        "title" => "HTMLTitleElement",
        "track" => "HTMLTrackElement",
        "ul" => "HTMLUListElement",
        "video" => "HTMLVideoElement",
        _ => return None,
    })
}

/// `dom.borrow().parent(n)` without a scrutinee-held `Ref`.
fn dom_parent(dom: &DomRef, n: NodeId) -> Option<NodeId> {
    dom.borrow().parent(n)
}

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).copied().unwrap_or(Value::Undefined)
}

/// `*AttributeNS` — l'arène ne trace pas les espaces de noms :
/// un attr `p:l` a pour partie locale `l`. `ns` nul → seul un
/// attr non préfixé matche (spec no-namespace) ; `ns` donné →
/// partie locale après le dernier `:`.
/// `*AttributeNS` — `ns` null/undefined/`""` compte comme
/// no-namespace (spec DOM : chaîne vide ≡ null).
fn attr_ns_is_null(e: &mut Engine, v: Value) -> JsResult<bool> {
    Ok(match v {
        Value::Null | Value::Undefined => true,
        _ => e.to_js_string(v)?.is_empty(),
    })
}

fn attr_ns_match(key: &str, ns_null: bool, local: &str) -> bool {
    if ns_null {
        return key.eq_ignore_ascii_case(local);
    }
    match key.rsplit_once(':') {
        Some((_, l)) => l.eq_ignore_ascii_case(local),
        None => false,
    }
}

/// `collection.item(i)` — `null` hors-bornes (spec `NodeList`/
/// `HTMLCollection` ; l'accès indexé `collection[i]` rend
/// `undefined`, différence assumée).
fn collection_item(e: &mut Engine, t: Value, args: &[Value]) -> JsResult<Value> {
    let i = e.to_number(arg(args, 0))? as usize;
    if let Value::Obj(id) = t {
        if let Some(o) = e.heap.obj(id) {
            if let ObjKind::Array(elems) = &o.kind {
                return Ok(elems.get(i).copied().unwrap_or(Value::Null));
            }
        }
    }
    Ok(Value::Null)
}

/// `collection.namedItem(name)` (JS-C76) — premier élément dont
/// `id` ou `name` matche, `null` sinon. Déviation documentée :
/// multi-match (radio group) → premier seul, pas de
/// `RadioNodeList`.
fn collection_named_item(e: &mut Engine, t: Value, args: &[Value]) -> JsResult<Value> {
    let name = e.to_js_string(arg(args, 0))?;
    if let Value::Obj(id) = t {
        let elems = match e.heap.obj(id) {
            Some(o) => match &o.kind {
                ObjKind::Array(elems) => elems.clone(),
                _ => Vec::new(),
            },
            None => Vec::new(),
        };
        for v in elems {
            for key in ["id", "name"] {
                if let Ok(Value::Str(s)) = e.get_prop_value(v, key) {
                    if e.heap.string(s).is_some_and(|v| v == name) {
                        return Ok(v);
                    }
                }
            }
        }
    }
    Ok(Value::Null)
}

/// Collection DOM « HTMLCollection-like » (JS-C76 — déclencheur
/// `Error: b` google.com : les helpers `makeIterator` des bundles
/// closure exigent `a[Symbol.iterator]` **ou** `a.length`
/// numérique) : tableau JS réel (`@@iterator`/`length`/indexés
/// natifs) + `item()` ; `named` ajoute `namedItem()` et l'accès
/// nommé (`collection.foo` ≙ élément de `id`/`name` `foo`,
/// propre aux collections de formulaires). Snapshot à l'accès —
/// la vivacité vient de la relecture à chaque get.
fn dom_collection(
    e: &mut Engine,
    dom: &DomRef,
    cache: &NodeCache,
    ids: &[NodeId],
    named: bool,
    tag: &'static str,
) -> Value {
    let vals: Vec<Value> = ids.iter().map(|&n| node_value(e, dom, cache, n)).collect();
    let arr = e.new_array(vals);
    if let Value::Obj(id) = arr {
        // `Symbol.toStringTag` : `Object.prototype.toString.call`
        // rend `[object NodeList]`/`[object HTMLCollection]` —
        // les détecteurs de collection type jQuery/allocine
        // (`/^\[object (HTMLCollection|NodeList|Object)]$/`)
        // rejetaient nos Array nus → `arr.addEventListener` tuait
        // le boot (vol 118).
        let tv = e.str_val(tag);
        e.set_own(id, crate::interp::SYM_TO_STRING_TAG, tv, false);
        e.add_method(id, "item", collection_item);
        if named {
            e.add_method(id, "namedItem", collection_named_item);
            for &n in ids {
                for k in ["id", "name"] {
                    let v = dom.borrow().attr(n, k).map(str::to_string);
                    if let Some(v) = v {
                        if !v.is_empty()
                            && crate::interp::array_index(&v).is_none()
                            && e.get_prop(id, &v).is_none()
                        {
                            let nv = node_value(e, dom, cache, n);
                            e.set_own(id, &v, nv, false);
                        }
                    }
                }
            }
        }
    }
    arr
}

/// Éléments « listés » d'un `<form>`/`<fieldset>` (HTML §4.10.3 —
/// `button fieldset input object output select textarea`), ordre
/// document approximé par ordre d'allocation du miroir (exact pour
/// les documents parsés). L'association `form=` cross-arbre n'est
/// pas suivie — déviation documentée.
fn listed_controls(d: &Dom, root: NodeId) -> Vec<NodeId> {
    let mut out = Vec::new();
    for tag in [
        "input", "select", "textarea", "button", "fieldset", "object", "output",
    ] {
        out.extend(d.by_tag(root, tag));
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// Objet `Attr` honnête (JS-C76 — snapshot nom/valeur + lien
/// `ownerElement`) : pas un vrai nœud du miroir ; les mutations
/// transitent par `attributes.setNamedItem`, qui relit `name`/
/// `value` de l'objet passé.
fn attr_value(e: &mut Engine, owner: Value, name: &str, value: &str) -> Value {
    let o = e.new_object();
    let (n, v) = (e.str_val(name.to_string()), e.str_val(value.to_string()));
    for (k, p) in [
        ("name", n),
        ("nodeName", n),
        ("localName", n),
        ("value", v),
        ("nodeValue", v),
        ("textContent", v),
    ] {
        e.set_own(o, k, p, true);
    }
    e.set_own(o, "nodeType", Value::Num(2.0), true);
    e.set_own(o, "specified", Value::Bool(true), true);
    e.set_own(o, "ownerElement", owner, true);
    Value::Obj(o)
}

/// Wrap a `Dom` in a `document` host object (fresh node cache) —
/// used by `install_dom` and by `xhr.responseXML`.
pub(crate) fn document_value(e: &mut Engine, dom: &DomRef) -> Value {
    e.new_host_object(Rc::new(DocumentHost {
        dom: dom.clone(),
        cache: Rc::new(RefCell::new(HashMap::new())),
        implementation: RefCell::new(None),
        fonts: RefCell::new(None),
        domain_override: RefCell::new(None),
        adopted_sheets: RefCell::new(None),
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
            fonts: RefCell::new(None),
            domain_override: RefCell::new(None),
            adopted_sheets: RefCell::new(None),
        }));
        // `#document` == `document` : listeners/`this`/`on*` au
        // sommet de la propagation résolvent sur l'objet document
        // (sémantique DOM réelle — `currentTarget === document`).
        if let Value::Obj(id) = doc {
            cache.borrow_mut().insert(dom.borrow().root, id);
        }
        self.dom_ctx = Some((dom.clone(), cache.clone()));
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
            // `vendor`/`product*` — props spec lues par les
            // fingerprint/feature-detects réels. `vendor` vide
            // (style Firefox — honnête, pas "Google Inc.") ;
            // ryanair vol 127 : `navigator.vendor.toString()`
            // tuait le boot → zone incomplète →
            // `MutationObserver: callback requis` en cascade.
            ("vendor", ""),
            ("appCodeName", "Mozilla"),
            ("product", "Gecko"),
            ("productSub", "20030107"),
            ("vendorSub", ""),
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
        // `navigator.javaEnabled()` — méthode spec (dépréciée)
        // rendant `false` : pas de plugin Java ; les
        // feature-detects réels l'appellent (maif vol 122).
        self.add_method(nav, "javaEnabled", nav_java_enabled);
        // `navigator.credentials` (JS-C93) — surface spec
        // `CredentialsContainer` : `create`/`get` rejettent
        // `NotSupportedError` (pas d'authentificateur
        // plateforme — la délégation passkey réelle est un
        // chantier embedder à part) ; `preventSilentAccess`
        // résout `undefined` (spec, API dépréciée mais lue).
        let creds = self.new_object();
        self.add_method(creds, "create", creds_unsupported);
        self.add_method(creds, "get", creds_unsupported);
        self.add_method(creds, "preventSilentAccess", creds_psa);
        let tag = self.str_val("CredentialsContainer");
        self.set_own(creds, crate::interp::SYM_TO_STRING_TAG, tag, false);
        self.set_own(nav, "credentials", Value::Obj(creds), true);
        // `navigator.clipboard` (CAP-181) — surface spec
        // `writeText`/`readText` par hook embedder (le presse-
        // papiers est une ressource OS/permission, pas du
        // moteur) ; sans hook → rejet `NotAllowedError`
        // honnête. `typeof navigator.clipboard` = 'object'
        // pour le feature-detect — la promesse tranche.
        let clip = self.new_object();
        self.add_method(clip, "writeText", clip_write_text);
        self.add_method(clip, "readText", clip_read_text);
        let tag = self.str_val("Clipboard");
        self.set_own(clip, crate::interp::SYM_TO_STRING_TAG, tag, false);
        self.set_own(nav, "clipboard", Value::Obj(clip), true);
        // `navigator.serviceWorker` (§B-14 vol 323, bras
        // `register`) — surface spec `ServiceWorkerContainer` :
        // présence honnête (navigateur moderne), mais pas de
        // runtime SW — `register` rejette `NotSupportedError`
        // (le `.catch` du site tourne, jamais de faux succès) ;
        // `controller`=null, `ready` reste pending (spec : sans
        // registration active, elle ne résout jamais),
        // `getRegistration`→`undefined`, `getRegistrations`→`[]`.
        let sw = self.new_object();
        self.add_method(sw, "register", sw_register);
        self.add_method(sw, "getRegistration", sw_get_registration);
        self.add_method(sw, "getRegistrations", sw_get_registrations);
        self.set_own(sw, "controller", Value::Null, true);
        let ready = self.new_promise(); // jamais résolue — spec.
        self.set_own(sw, "ready", Value::Obj(ready), true);
        let tag = self.str_val("ServiceWorkerContainer");
        self.set_own(sw, crate::interp::SYM_TO_STRING_TAG, tag, false);
        self.set_own(nav, "serviceWorker", Value::Obj(sw), true);
        // Props triviales honnêtes (JS-C157) — lues par les
        // CMP/feature-detects : `undefined` cassait la lecture
        // (`navigator.onLine`, `hardwareConcurrency`…).
        // Valeurs low-fingerprint, pas des stubs promettant.
        self.set_own(nav, "onLine", Value::Bool(true), true);
        self.set_own(nav, "cookieEnabled", Value::Bool(true), true);
        // `navigator.userAgentData` (B-24 qwant) — l'absence
        // laissait les détects modernes en fallback « inconnu
        // → mobile » : réponse desktop spec-honnête
        // (`mobile:false`) au lieu de `undefined`, low-
        // fingerprint (`brands` vide, `platform` figé).
        let uad = self.new_object();
        let empty = self.new_array(vec![]);
        self.set_own(uad, "brands", empty, true);
        self.set_own(uad, "mobile", Value::Bool(false), true);
        let win = self.str_val("Windows");
        self.set_own(uad, "platform", win, true);
        let tag = self.str_val("NavigatorUAData");
        self.set_own(uad, crate::interp::SYM_TO_STRING_TAG, tag, false);
        // `getHighEntropyValues` (speedtest vol78) — spec :
        // promesse résolue aux hints demandés, mêmes valeurs
        // low-fingerprint que l'objet — jamais `undefined`.
        let ghev = self.native_fn(
            "getHighEntropyValues",
            |e: &mut Engine, _t: Value, args: &[Value]| {
                let hints: Vec<String> = match args.first() {
                    Some(v @ Value::Obj(_)) => {
                        let len = e
                            .get_prop_value(*v, "length")
                            .ok()
                            .and_then(|l| match l {
                                Value::Num(n) => Some(n as u32),
                                _ => None,
                            })
                            .unwrap_or(0);
                        (0..len)
                            .filter_map(|i| {
                                e.get_prop_value(*v, &i.to_string())
                                    .ok()
                                    .and_then(|x| e.to_js_string(x).ok())
                            })
                            .collect()
                    }
                    _ => Vec::new(),
                };
                let out = e.new_object();
                let vals: Vec<(&str, Value)> = vec![
                    ("brands", e.new_array(vec![])),
                    ("mobile", Value::Bool(false)),
                    ("platform", e.str_val("Windows")),
                    ("platformVersion", e.str_val("15.0.0")),
                    ("architecture", e.str_val("x86")),
                    ("bitness", e.str_val("64")),
                    ("model", e.str_val("")),
                    ("uaFullVersion", e.str_val("0.1.0")),
                    ("fullVersionList", e.new_array(vec![])),
                    ("wow64", Value::Bool(false)),
                ];
                for (k, v) in vals {
                    if hints.iter().any(|h| h == k) {
                        e.set_own(out, k, v, true);
                    }
                }
                let pid = e.new_promise();
                e.resolve_promise(pid, Value::Obj(out));
                Ok(Value::Obj(pid))
            },
        );
        self.set_own(uad, "getHighEntropyValues", ghev, true);
        self.set_own(nav, "userAgentData", Value::Obj(uad), true);
        self.set_own(nav, "webdriver", Value::Bool(false), true);
        self.set_own(nav, "globalPrivacyControl", Value::Bool(false), true);
        self.set_own(nav, "doNotTrack", Value::Null, true);
        self.set_own(nav, "maxTouchPoints", Value::Num(0.0), true);
        self.set_own(nav, "hardwareConcurrency", Value::Num(4.0), true);
        // `navigator.geolocation` (B-14 marmiton vol63) — les
        // feature-detects lisent l'objet (« le navigateur n'a
        // pas de GPS ») ; les méthodes rejettent honnêtement
        // `POSITION_UNAVAILABLE` (code 2) au lieu d'être
        // `undefined` et de casser la détection.
        let geo = self.new_object();
        let geop = geo;
        let tag = self.str_val("Geolocation");
        self.set_own(geop, crate::interp::SYM_TO_STRING_TAG, tag, false);
        for m in ["getCurrentPosition", "watchPosition"] {
            let f = self.native_fn(m, |e: &mut Engine, _t: Value, args: &[Value]| {
                // `success` jamais appelée — pas de GPS ;
                // `error(err)` reçoit `GeolocationPositionError`
                // {code:2,message} (spec), appelée async-borne.
                if let Some(cb) = args.get(1).or_else(|| args.first()) {
                    if e.is_callable(*cb) {
                        let err = e.new_object();
                        e.set_own(err, "code", Value::Num(2.0), true);
                        let msg = e.str_val("position unavailable");
                        e.set_own(err, "message", msg, true);
                        let _ = e.call_value(*cb, Value::Undefined, &[Value::Obj(err)]);
                    }
                }
                Ok(Value::Num(0.0))
            });
            self.set_own(geop, m, f, true);
        }
        let cw = self.native_fn("clearWatch", |_e, _t, _a| Ok(Value::Undefined));
        self.set_own(geop, "clearWatch", cw, true);
        self.set_own(nav, "geolocation", Value::Obj(geop), true);
        self.set_own(g, "navigator", Value::Obj(nav), false);

        // `Worker` borné (JS-C29) — le constructeur est présent
        // pour la détection de feature ; sans `set_worker_factory`
        // il jette un TypeError honnête.
        self.install_worker_ctor();
        // `MessageChannel` (JS-C81) — voie d'ordonnancement
        // des schedulers de frameworks (React la préfère à
        // `setTimeout`).
        self.install_channel_ctor();
        // `window`/`self`/`globalThis` — the global object itself.
        let gv = Value::Obj(g);
        self.set_own(g, "window", gv, false);
        self.set_own(g, "self", gv, false);
        self.set_own(g, "globalThis", gv, false);
        // `top`/`parent`/`frames`/`length`/`closed`/`postMessage` —
        // accessors résolvant leur contexte depuis `this` (JS-C78) ;
        // sans parent, `top`/`parent`/`frames` ≙ la fenêtre
        // elle-même comme avant.
        crate::context::install_ctx_window_props(self);
        // `MutationObserver` (JS-C26) — les records vivent dans
        // la file du `Dom`, livrés au microtask checkpoint.
        self.install_mutation_observer();
        // Interfaces IDB (feature-detect/`instanceof`) —
        // indépendantes du registre `indexedDB` (install_idb).
        crate::idb::install_idb_interfaces(self);
        // `location` + `history` — hooks-driven (see BrowserHooks).
        // `Location` global (JS-C122/news.google) : l'interface
        // existe comme nom global — `location instanceof
        // Location` requiert le proto chaîné (pattern `Document`).
        let loc = self.new_host_object(Rc::new(LocationHost));
        let lf = self.native_fn("Location", illegal_ctor);
        if let Value::Obj(lfid) = lf {
            let lp = self.new_object();
            let tag = self.str_val("Location");
            self.set_own(lp, crate::interp::SYM_TO_STRING_TAG, tag, false);
            self.set_own(lfid, "prototype", Value::Obj(lp), false);
            if let Value::Obj(locid) = loc {
                if let Some(o) = self.heap.obj_mut(locid) {
                    o.proto = Value::Obj(lp);
                }
            }
        }
        self.set_own(g, "Location", lf, false);
        let hist = self.new_host_object(Rc::new(HistoryHost));
        self.set_own(g, "location", loc, false);
        self.set_own(g, "history", hist, false);
        // `window.navigation` — Navigation API (WHATWG).
        // `Navigation` est un `EventTarget` : cloudflare-insights
        // et les routeurs SPA (Next.js app-router) font
        // `navigation.addEventListener('navigate', …)` au boot.
        // Sans la propriété, un `function navigation()` sloppy la
        // remplaçait et le listener mourait « addEventListener is
        // not a function (on function) » (cowcotland §B-14) —
        // posée **non inscriptible** : la déclaration/assignation
        // sloppy est ignorée comme sous Chrome.
        let navapi = self.new_host_object(Rc::new(NavigationHost));
        let nf = self.native_fn("Navigation", illegal_ctor);
        if let Value::Obj(nfid) = nf {
            let np = self.new_object();
            let tag = self.str_val("Navigation");
            self.set_own(np, crate::interp::SYM_TO_STRING_TAG, tag, false);
            // `Navigation.prototype` → `EventTarget.prototype` :
            // chaînage effectué dans la boucle protos DOM plus bas
            // (`EventTarget` est installé après ce bloc).
            self.set_own(nfid, "prototype", Value::Obj(np), false);
            if let Value::Obj(nav_id) = navapi {
                if let Some(o) = self.heap.obj_mut(nav_id) {
                    o.proto = Value::Obj(np);
                }
            }
        }
        self.set_own(g, "Navigation", nf, false);
        self.set_own_attrs(
            g,
            "navigation",
            navapi,
            crate::heap::PropAttrs {
                writable: false,
                enumerable: true,
                configurable: true,
            },
        );

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
        // `HTMLElement` est le seul constructible de la famille —
        // son `super()` rend l'élément en cours d'upgrade CE
        // (JS-C68) ; `new HTMLElement()` direct reste « Illegal ».
        let he = self.native_fn("HTMLElement", html_element_ctor);
        if let Value::Obj(fid) = he {
            let p = self.new_object();
            // `attachShadow`/`attachInternals` (issue #25) —
            // les polyfills lisent `HTMLElement.prototype.*`
            // (github-elements : `e = globalThis.HTMLElement`,
            // `n = e.prototype.attachShadow` puis `n.call`) —
            // la chaîne proto vers `Element.prototype` n'est
            // pas garantie côté stub.
            for (k, f2) in [
                (
                    "attachShadow",
                    element_attach_shadow as crate::heap::NativeFn,
                ),
                ("attachInternals", element_attach_internals),
            ] {
                let m = self.native_fn(k, f2);
                self.set_own(p, k, m, true);
            }
            self.set_own(fid, "prototype", Value::Obj(p), false);
        }
        self.set_own(g, "HTMLElement", he, false);
        for name in [
            "Element",
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
            // `HTMLDocument` — interface legacy de `document`
            // (cookie-monster/woopic : `x instanceof HTMLDocument`,
            // orange §B-14). Proto chaîné `Document.prototype`.
            "HTMLDocument",
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
            "TreeWalker",
            "NodeIterator",
            "MutationRecord",
            "IdleDeadline",
            "XMLSerializer",
            // dev.to : `instanceOf(Window)` (prop-types react-dates)
            // — détection seule, jamais construit.
            "Window",
        ] {
            let f = self.native_fn(name, illegal_ctor);
            if let Value::Obj(fid) = f {
                let p = self.new_object();
                // Chaîne d'héritage des protos DOM (J5) —
                // `instanceof` transitif : `el instanceof
                // Element` et `instanceof Node` à la fois
                // (impots/ameli). Les parents sont posés
                // plus tôt (`Node`/`Element`/`EventTarget`).
                let parent = match name {
                    "Text" | "Comment" | "ProcessingInstruction" => Some("CharacterData"),
                    "CDATASection" => Some("Text"),
                    "ShadowRoot" => Some("DocumentFragment"),
                    "Document" | "DocumentFragment" | "CharacterData" | "Attr" | "DocumentType" => {
                        Some("Node")
                    }
                    "HTMLDocument" => Some("Document"),
                    "SVGElement" => Some("Element"),
                    // Spec : `HTML*Element.prototype` hérite
                    // `HTMLElement.prototype` (pas `Element`
                    // direct) — sinon `el instanceof HTMLElement`
                    // deviendrait faux dès qu'un proto tag-
                    // spécifique est posé (JS-C192, sephora
                    // `currentScript instanceof HTMLScriptElement`).
                    _ if name.starts_with("HTML") && name.ends_with("Element") => {
                        Some("HTMLElement")
                    }
                    _ => None,
                };
                if let Some(pn) = parent {
                    if let Some(Value::Obj(cid)) = self.get_prop(g, pn) {
                        if let Some(pp @ Value::Obj(_)) = self.get_prop(cid, "prototype") {
                            if let Some(o) = self.heap.obj_mut(p) {
                                o.proto = pp;
                            }
                        }
                    }
                }
                self.set_own(fid, "prototype", Value::Obj(p), false);
                // Accesseurs IDL (`value`/`checked`/…) sur le
                // proto global — `el.constructor` rend le ctor
                // global tag-spécifique ; sa surface de
                // descripteurs doit égaler celle des anciens
                // `dom_ctor` (React `inputValueTracking`,
                // JS-C192).
                self.install_dom_idl_accessors(p, name);
                // Spec WebIDL : `X.prototype.constructor === X`
                // (`el.constructor`, `document.constructor`).
                self.set_own(p, "constructor", f, false);
            }
            self.set_own(g, name, f, false);
        }
        // `EventTarget` — constructible spec (corpus css-tricks :
        // wp-polyfill/core-js meurt au feature-detect sinon).
        // Listeners = map cachée `__et` type → [cb] (rail `__ls`).
        let et_f = self.native_fn("EventTarget", event_target_ctor);
        {
            let p = self.new_object();
            let m = self.native_fn("addEventListener", et_add_listener);
            self.set_own(p, "addEventListener", m, true);
            let m = self.native_fn("removeEventListener", et_remove_listener);
            self.set_own(p, "removeEventListener", m, true);
            let m = self.native_fn("dispatchEvent", et_dispatch_event);
            self.set_own(p, "dispatchEvent", m, true);
            let tag = self.str_val("EventTarget");
            self.set_own(p, crate::interp::SYM_TO_STRING_TAG, tag, false);
            if let Value::Obj(fid) = et_f {
                self.set_own(fid, "prototype", Value::Obj(p), false);
            }
            // WebIDL : `window` hérite `EventTarget.prototype` —
            // le polyfill webcomponents (YouTube) lit
            // `window.__shady_native_addEventListener` à travers
            // cette chaîne après y avoir défini la native
            // sauvegardée (`L(proto,…)` → `defineProperty`).
            if let Some(o) = self.heap.obj_mut(g) {
                if matches!(o.proto, Value::Obj(p0) if p0 == self.protos.object) {
                    o.proto = Value::Obj(p);
                }
            }
        }
        self.set_own(g, "EventTarget", et_f, false);
        // Liens hors-liste de la chaîne protos DOM (J5) :
        // `Node` < `EventTarget` (spec WebIDL), `Element` <
        // `Node`, `HTMLElement` < `Element` — `instanceof`
        // transitif complet (`el instanceof EventTarget`).
        for (child, parent) in [
            ("Node", "EventTarget"),
            ("Element", "Node"),
            ("HTMLElement", "Element"),
            ("Worker", "EventTarget"),
            // `MessagePort` (JS-C204) — ctor posé avant
            // `EventTarget` par `install_channel_ctor` ; la
            // boucle le chaîne ici.
            ("MessagePort", "EventTarget"),
            // `Navigation` (cowcotland §B-14) — installé plus haut
            // avec `location`/`history`, avant `EventTarget`.
            ("Navigation", "EventTarget"),
            // IDB (JS-C215) — WebIDL : `IDBRequest`,
            // `IDBOpenDBRequest`, `IDBTransaction` et
            // `IDBDatabase` sont des `EventTarget` ;
            // `IDBOpenDBRequest` < `IDBRequest`,
            // `IDBCursorWithValue` < `IDBCursor` —
            // `instanceof` + greffes proto (ruxit/workbox
            // patchent `X.prototype`) réels.
            ("IDBRequest", "EventTarget"),
            ("IDBOpenDBRequest", "IDBRequest"),
            ("IDBTransaction", "EventTarget"),
            ("IDBDatabase", "EventTarget"),
            ("IDBCursorWithValue", "IDBCursor"),
        ] {
            if let Some(Value::Obj(cid)) = self.get_prop(g, child) {
                if let Some(Value::Obj(cp)) = self.get_prop(cid, "prototype") {
                    if let Some(Value::Obj(pcid)) = self.get_prop(g, parent) {
                        if let Some(pp @ Value::Obj(_)) = self.get_prop(pcid, "prototype") {
                            if let Some(o) = self.heap.obj_mut(cp) {
                                o.proto = pp;
                            }
                        }
                    }
                }
            }
        }
        // `DOMException` — constructible spec : `new
        // DOMException(message?, name?)` → `{name, message}`
        // (même mur css-tricks — `x instanceof DOMException` et
        // `new DOMException("…","NotSupportedError")` passent).
        let de_f = self.native_fn("DOMException", dom_exception_ctor);
        if let Value::Obj(fid) = de_f {
            let p = self.new_object();
            let tag = self.str_val("DOMException");
            self.set_own(p, crate::interp::SYM_TO_STRING_TAG, tag, false);
            self.set_own(fid, "prototype", Value::Obj(p), false);
        }
        self.set_own(g, "DOMException", de_f, false);
        // `DOMParser` — constructible (JS-C91) : htmx parse
        // les réponses `hx-swap` via `new DOMParser().
        // parseFromString(html,"text/html")` — même voie que
        // `xhr.responseXML` (`Dom::from_html` → document miroir).
        let f = self.native_fn("DOMParser", dom_parser_ctor);
        if let Value::Obj(fid) = f {
            let p = self.new_object();
            self.set_own(fid, "prototype", Value::Obj(p), false);
        }
        self.set_own(g, "DOMParser", f, false);
        // `Range` — constructible spec (CAP-182, arc édition) :
        // `new Range()` → frontières `(document,0)` mutables,
        // `Selection.addRange` le consomme directement.
        let f = self.native_fn("Range", crate::misc_apis::range_ctor);
        if let Value::Obj(fid) = f {
            let p = self.new_object();
            self.set_own(fid, "prototype", Value::Obj(p), false);
        }
        self.set_own(g, "Range", f, false);
        // `CSSStyleSheet` — constructible spec (CAP-183) :
        // feuille stockée côté miroir (`insertRule`/`deleteRule`/
        // `replace`/`replaceSync` — cascade = chantier session,
        // documenté).
        let f = self.native_fn("CSSStyleSheet", crate::misc_apis::css_sheet_ctor);
        if let Value::Obj(fid) = f {
            let p = self.new_object();
            // Méthodes sur le PROTOTYPE (IDL, spec) — le
            // feature-detect github catalyst exige
            // `"replaceSync" in CSSStyleSheet.prototype` pour
            // prendre la voie native `adoptedStyleSheets`
            // (sinon fallback `r.Se` = define direct sans
            // découverte lazy). Les instances chaînent leur
            // `proto` dans `css_sheet_ctor`.
            for (k, f2) in [
                (
                    "insertRule",
                    crate::misc_apis::sheet_insert_rule as crate::heap::NativeFn,
                ),
                ("deleteRule", crate::misc_apis::sheet_delete_rule),
                ("replace", crate::misc_apis::sheet_replace),
                ("replaceSync", crate::misc_apis::sheet_replace_sync),
            ] {
                let m = self.native_fn(k, f2);
                self.set_own(p, k, m, true);
            }
            let tag = self.str_val("CSSStyleSheet");
            self.set_own(p, crate::interp::SYM_TO_STRING_TAG, tag, false);
            self.set_own(fid, "prototype", Value::Obj(p), false);
        }
        self.set_own(g, "CSSStyleSheet", f, false);
        // `SharedWorker` (CAP-183) — ctor défini pour le
        // feature-detect ; sans loader → TypeError honnête
        // (comme `Worker`) ; avec → `{port}` déléguant au
        // worker (borné : chaque contexte a le sien, pas de
        // partage — documenté).
        let f = self.native_fn("SharedWorker", |e, t, a| {
            crate::misc_apis::shared_worker_ctor(e, t, a)
        });
        if let Value::Obj(cid) = f {
            // `SharedWorker.prototype` — spec WebIDL : les
            // instruments patchent `X.prototype.addEventListener`
            // (même bras ruxit que `Worker`/`EventSource`).
            // `EventTarget` est déjà installé ici → chaînage
            // direct.
            let p = self.new_object();
            for name in ["addEventListener", "removeEventListener", "dispatchEvent"] {
                let namev = self.str_val(name);
                let bm = self.bound_native("hostMethod", Self::host_method, vec![namev]);
                self.set_own(p, name, bm, true);
            }
            if let Some(Value::Obj(etc)) = self.get_prop(g, "EventTarget") {
                if let Some(pp @ Value::Obj(_)) = self.get_prop(etc, "prototype") {
                    if let Some(o) = self.heap.obj_mut(p) {
                        o.proto = pp;
                    }
                }
            }
            self.set_own(cid, "prototype", Value::Obj(p), false);
            self.set_own(p, "constructor", f, false);
        }
        self.set_own(g, "SharedWorker", f, false);
        // `NodeFilter` — constantes `SHOW_*`/`FILTER_*` lues
        // par `createTreeWalker(root, NodeFilter.SHOW_…)`.
        let nf = self.new_object();
        for (k, v) in [
            ("SHOW_ELEMENT", 1.0),
            ("SHOW_TEXT", 4.0),
            ("SHOW_COMMENT", 128.0),
            ("SHOW_DOCUMENT", 256.0),
            ("SHOW_ALL", f64::from(u32::MAX)),
            ("FILTER_ACCEPT", 1.0),
            ("FILTER_REJECT", 2.0),
            ("FILTER_SKIP", 3.0),
        ] {
            self.set_own(nf, k, Value::Num(v), true);
        }
        self.set_own(g, "NodeFilter", Value::Obj(nf), false);
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
                    // `scrollIntoView` (JS-C74) — no-op honnête,
                    // même motif de lisibilité que `matches`.
                    ("scrollIntoView", element_scroll_into_view),
                    ("scrollIntoViewIfNeeded", element_scroll_into_view),
                    // `scroll`/`scrollTo`/`scrollBy` (§B-14
                    // EXPLOR68 iltalehti) — même stub no-op
                    // honnête que `scrollIntoView` : pas de
                    // scroll d'élément dans le miroir.
                    ("scroll", element_scroll_noop),
                    ("scrollTo", element_scroll_noop),
                    ("scrollBy", element_scroll_noop),
                    // `attachShadow` stub (issue #25) — la
                    // référence doit être **lisible** : le
                    // polyfill Invoker Commands fait
                    // `n = Element.prototype.attachShadow`
                    // puis `n.call(this, init)` (mesuré
                    // github-elements).
                    ("attachShadow", element_attach_shadow),
                    // `getBoundingClientRect` (issue #19) —
                    // référence lisible pour les mesures via
                    // `.call(el)` (utils/polyfills de
                    // géométrie — même motif).
                    ("getBoundingClientRect", element_gbrect),
                    // Sélecteurs en natives `Element.prototype`
                    // (JS-C162) — Bootstrap SelectorEngine fait
                    // `Element.prototype.querySelectorAll.call(
                    // el, sel)` ; la référence absente rendait
                    // `cannot read 'call' of Undefined` à
                    // chaque handler `load` (mesuré OSM +
                    // ameli, vols 27/29).
                    ("querySelector", element_query_selector),
                    ("querySelectorAll", element_query_selector_all),
                    ("getElementsByTagName", element_get_by_tag),
                    ("getElementsByClassName", element_get_by_class),
                ] {
                    let m = self.native_fn(k, f2);
                    self.set_own(pid, k, m, true);
                }
            }
        }
        // `document` hérite de `HTMLDocument.prototype` (spec —
        // lui-même chaîné `Document.prototype` : `instanceof
        // Document`/`HTMLDocument`/`Node` tous vrais) — les sites
        // monkey-patchent le proto DOM (mesuré dev.to :
        // `Document.prototype.ready = new Promise(…)` puis
        // `document.ready.then`).
        if let (Value::Obj(did), Some(Value::Obj(cid))) = (doc, self.get_prop(g, "HTMLDocument")) {
            if let Some(proto @ Value::Obj(_)) = self.get_prop(cid, "prototype") {
                if let Some(o) = self.heap.obj_mut(did) {
                    o.proto = proto;
                }
            }
        }
        // `adoptedStyleSheets` sur `Document.prototype` (IDL) —
        // la feature-detect github l'exige
        // (`"adoptedStyleSheets" in Document.prototype`)
        // avant d'injecter ses règles lazy-detect en feuille
        // construite. Propriété propre `undefined` : la lecture
        // passe par le getter hôte du `document` (proprio `in`,
        // jamais résolution).
        if let Some(Value::Obj(cid)) = self.get_prop(g, "Document") {
            if let Some(Value::Obj(pid)) = self.get_prop(cid, "prototype") {
                self.set_own(pid, "adoptedStyleSheets", Value::Undefined, true);
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
            // CAP-178 — interfaces événement mesurées absentes
            // (sonde `_census_calls`) : init copié par le rail
            // `event_ctor` générique (`message.data`,
            // `storage.key`/`oldValue`/`newValue`/`url`,
            // `close.wasClean`/`code`/`reason`).
            "MessageEvent",
            "StorageEvent",
            "CloseEvent",
        ] {
            // `MouseEvent` & sous-classes : défauts
            // `MouseEventInit` spec (`button`/`buttons`/coords
            // = 0) — sans `button` React Router laisse tomber
            // les clics `<Link>` (son garde `event.button ===
            // 0` rate sur `undefined`).
            let ctor = if matches!(
                name,
                "MouseEvent" | "PointerEvent" | "WheelEvent" | "DragEvent"
            ) {
                mouse_event_ctor as crate::heap::NativeFn
            } else {
                event_ctor as crate::heap::NativeFn
            };
            let f = self.native_fn(name, ctor);
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
        // `Audio`/`Video` (CAP-178) — alias `createElement` des
        // constructeurs historiques (`new Audio(url)` charge
        // la source — les jeux/sites l'utilisent encore ;
        // mesuré `_census_calls` : ReferenceError).
        for (name, tag) in [("Audio", "audio"), ("Video", "video")] {
            let tagv = self.str_val(tag);
            let f = self.bound_native(name, media_el_ctor, vec![tagv]);
            if let Value::Obj(fid) = f {
                let p = self.new_object();
                self.set_own(fid, "prototype", Value::Obj(p), false);
            }
            self.set_own(g, name, f, false);
        }
        // `XMLSerializer` (CAP-178) — `serializeToString(node)`
        // → rail `outer_html` (mesuré `_census_calls` :
        // TypeError ; DOMPurify/sanitisers l'exigent — accepte
        // éléments ET `document`, résolu `#document` par
        // `node_for`).
        let xs_f = self.native_fn("XMLSerializer", xmlserializer_ctor);
        if let Value::Obj(fid) = xs_f {
            let p = self.new_object();
            self.set_own(fid, "prototype", Value::Obj(p), false);
        }
        self.set_own(g, "XMLSerializer", xs_f, false);
        // `AbortSignal` (CAP-194) — surface spec bornée : le
        // ctor reste `Illegal` (spec — `new AbortSignal()`
        // jette), mais les statics `abort`/`timeout`/`any`
        // et le proto (`aborted`, `reason`, `throwIfAborted`,
        // `onabort`, `addEventListener('abort')`) existent —
        // le routeur TanStack de x-web arme `AbortSignal`
        // avant toute navigation (mesuré : ReferenceError).
        let sig_proto = self.new_object();
        {
            let m = self.native_fn("throwIfAborted", signal_throw_if_aborted);
            self.set_own(sig_proto, "throwIfAborted", m, true);
            let m = self.native_fn("addEventListener", signal_add_listener);
            self.set_own(sig_proto, "addEventListener", m, true);
            let m = self.native_fn("removeEventListener", signal_remove_listener);
            self.set_own(sig_proto, "removeEventListener", m, true);
            let tag = self.str_val("AbortSignal");
            self.set_own(sig_proto, crate::interp::SYM_TO_STRING_TAG, tag, false);
        }
        let as_f = self.native_fn("AbortSignal", illegal_ctor);
        if let Value::Obj(fid) = as_f {
            let m = self.native_fn("abort", signal_static_abort);
            self.set_own(fid, "abort", m, true);
            let m = self.native_fn("timeout", signal_static_timeout);
            self.set_own(fid, "timeout", m, true);
            let m = self.native_fn("any", signal_static_any);
            self.set_own(fid, "any", m, true);
            // Statique spec `AbortSignal.throwIfAborted(sig)`
            // (youtube kevlar vol22 : lu sur le ctor →
            // `cannot read 'throwIfAborted' of Undefined`).
            let m = self.native_fn("throwIfAborted", signal_static_throw_if_aborted);
            self.set_own(fid, "throwIfAborted", m, true);
            self.set_own(fid, "prototype", Value::Obj(sig_proto), false);
        }
        self.set_own(g, "AbortSignal", as_f, false);
        // `AbortController` minimal : `signal` réel +
        // `abort()` bascule `aborted`/`reason` + listeners.
        let ac_f = self.native_fn("AbortController", abort_controller_ctor);
        if let Value::Obj(fid) = ac_f {
            let p = self.new_object();
            let m = self.native_fn("abort", abort_signal_abort);
            self.set_own(p, "abort", m, true);
            self.set_own(fid, "prototype", Value::Obj(p), false);
        }
        self.set_own(g, "AbortController", ac_f, false);
        // `IntersectionObserver`/`ResizeObserver` (JS-C69) —
        // notification initiale à l'`observe` livrée au
        // checkpoint microtask (`observers.rs`). Mesuré
        // github.com : `new IntersectionObserver(cb)` (JS-C56).
        self.install_observers();
        // `customElements` — registre adossé au `Dom` :
        // `define`/`get`/`getName`/`whenDefined`/`upgrade` avec
        // upgrade des éléments et callbacks de cycle de vie
        // (JS-C56 → JS-C68).
        let ce = self.new_host_object(Rc::new(CustomElementsHost {
            dom: dom.clone(),
            cache: cache.clone(),
        }));
        self.set_own(g, "customElements", ce, false);
        // `window.name` (CAP-159) — nom du contexte posé
        // par l'embedder (`set_ctx_name`) ; les écritures
        // JS suivantes vivent sur cette propre-prop.
        let wn = self.hooks.win_name.clone();
        let wnv = self.str_val(&wn);
        self.set_own(g, "name", wnv, true);
        // `FormData` (JS-C67) — ctor + proto paresseux partagé
        // avec `res.formData()`/`xhr.send` (détail : FormData §).
        install_formdata_global(self);
        // `Blob`/`File`/`FileReader` + `URL.createObjectURL`
        // (JS-C70) — uploads et lectures client-side.
        crate::files::install_files(self);
        // `MediaSource`/`SourceBuffer`/`SourceBufferList`/
        // `TimeRanges` (JS-C193) — surface honnête sans
        // lecture : classes réelles, `isTypeSupported` → `""`,
        // `createObjectURL` → rail registre MSE (transport
        // vers l'hôte = étape dédiée).
        crate::mse::install_mse(self);
        // `ReadableStream`/`WritableStream`/`TransformStream`
        // (JS-C71) — `for await` sur les corps, bundlers.
        crate::streams::install_streams(self);
        // `WebSocket` (JS-C73) — stub honnête (handshake
        // refusé en async) pour les feature-detects.
        crate::websocket::install_websocket(self);
        // `EventSource` (JS-C92) — même motif : sans transport
        // le ctor arme un refus différé au checkpoint, jamais
        // un global absent (feature-detects des loaders).
        crate::eventsource::install_eventsource(self);
        // `Notification`/`BroadcastChannel`/`reportError`/
        // `getSelection` (JS-C74) — dom_ctx posé → Selection
        // disponible ici.
        crate::misc_apis::install_misc(self);
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
        // `screen.orientation` (§B-14 vol 343) — `ScreenOrientation`
        // hérite `EventTarget` : `orientation.addEventListener(
        // 'change', cb)` est un pattern news/mobile standard.
        // Métriques dérivées du viewport à l'install ;
        // `lock()` rejette `NotSupportedError` (pas de verrou
        // d'orientation hôte — honnête), `unlock()` = `undefined`.
        let so = self.new_object();
        let oty = if w >= h {
            "landscape-primary"
        } else {
            "portrait-primary"
        };
        let oty = self.str_val(oty);
        self.set_own(so, "type", oty, true);
        self.set_own(so, "angle", Value::Num(0.0), true);
        self.set_own(so, "onchange", Value::Null, true);
        self.add_method(so, "lock", so_lock);
        self.add_method(so, "unlock", |_e, _t, _a| Ok(Value::Undefined));
        self.add_method(so, "addEventListener", et_add_listener);
        self.add_method(so, "removeEventListener", et_remove_listener);
        self.add_method(so, "dispatchEvent", et_dispatch_event);
        let tag = self.str_val("ScreenOrientation");
        self.set_own(so, crate::interp::SYM_TO_STRING_TAG, tag, false);
        self.set_own(screen, "orientation", Value::Obj(so), true);
        self.set_own(g, "screen", Value::Obj(screen), false);
        self.set_own(g, "innerWidth", Value::Num(f64::from(w)), false);
        self.set_own(g, "innerHeight", Value::Num(f64::from(h)), false);
        self.set_own(g, "outerWidth", Value::Num(f64::from(w)), false);
        self.set_own(g, "outerHeight", Value::Num(f64::from(h)), false);
        self.set_own(g, "devicePixelRatio", Value::Num(1.0), false);
        // Scroll réel — `scroll*`/`page*Offset` lisent la
        // `GeometryMap` publiée (mise à jour par la session au
        // scroll input, sans rebuild). Vol github : figés à 0,
        // les loaders lazy « élément proche du viewport » ne
        // pouvaient plus se déclencher.
        {
            let sx = self.native_fn("scrollX", global_scroll_x);
            let sy = self.native_fn("scrollY", global_scroll_y);
            let px = self.native_fn("pageXOffset", global_scroll_x);
            let py = self.native_fn("pageYOffset", global_scroll_y);
            if let Some(o) = self.heap.obj_mut(g) {
                o.set_accessor("scrollX", sx, Value::Undefined, true);
                o.set_accessor("scrollY", sy, Value::Undefined, true);
                o.set_accessor("pageXOffset", px, Value::Undefined, true);
                o.set_accessor("pageYOffset", py, Value::Undefined, true);
            }
        }
        // `window.scroll*` — no-op honnêtes : le scroll page
        // vit côté session (input), aucune intention JS→
        // session n'est câblée — la méthode existe pour les
        // lazy-loaders qui la référencent (§B-14 iltalehti).
        for (n, f) in [
            ("scroll", win_scroll_noop as crate::heap::NativeFn),
            ("scrollTo", win_scroll_noop),
            ("scrollBy", win_scroll_noop),
        ] {
            let f = self.native_fn(n, f);
            self.set_own(g, n, f, true);
        }

        // -- JS-C98 — complétude de surface (census) ----------
        // Les globaux web restants : stubs « illegal
        // constructor » (même convention que la famille
        // HTML*Element — feature-detect, `instanceof`, props
        // de proto) pour les APIs dont l'absence tuait un
        // ReferenceError au premier `typeof`/accès direct.
        for name in [
            "HTMLSummaryElement",
            "SVGSVGElement",
            "Selection",
            "FileList",
            "MediaQueryList",
            "CustomElementRegistry",
            // `MessagePort` — vrai ctor + proto WebIDL posé
            // par `install_channel_ctor` (JS-C204) ; l'entrée
            // générique l'écrasait avec un proto nu.
            "IntersectionObserverEntry",
            "ResizeObserverEntry",
            "DOMStringList",
            "CSSRuleList",
            "StyleSheetList",
            "FontFaceSet",
            "Animation",
            "KeyframeEffect",
            "DocumentTimeline",
            "PerformanceEntry",
            "PerformanceMark",
            "PerformanceMeasure",
            "PerformanceNavigationTiming",
            "PerformanceResourceTiming",
            "PerformancePaintTiming",
            "PerformanceLongTaskTiming",
            "VideoFrame",
            "IdleDetector",
            "WakeLock",
            "ImageBitmap",
            "AudioContext",
            "OfflineAudioContext",
            "MediaStream",
            "MediaRecorder",
            "RTCPeerConnection",
            "Geolocation",
            // marmiton (V4) : `x instanceof GeolocationPosition`
            // — le nom absent tuait le feature-detect en
            // ReferenceError ; le callback du position porte
            // `coords` → `GeolocationCoordinates` avec.
            "GeolocationPosition",
            "GeolocationCoordinates",
            "PushManager",
            "ServiceWorker",
            "ServiceWorkerContainer",
            "ServiceWorkerRegistration",
            "BarcodeDetector",
            "EyeDropper",
            "LaunchQueue",
            "CookieStore",
            "FileSystemHandle",
            "Highlight",
            "EditContext",
            "Sanitizer",
            "XPathExpression",
            "VisualViewport",
            "Performance",
            "CSSFontFaceRule",
            "CredentialsContainer",
            "Credential",
            "PublicKeyCredential",
            "CanvasRenderingContext2D",
            "WebGLRenderingContext",
        ] {
            let f = self.native_fn(name, illegal_ctor);
            if let Value::Obj(fid) = f {
                let p = self.new_object();
                self.set_own(fid, "prototype", Value::Obj(p), false);
            }
            self.set_own(g, name, f, false);
        }
        // `XPathResult` — stub + constantes `*_TYPE` lues en
        // argument de `document.evaluate(…, type, …)` (CAP-188).
        let xr_f = self.native_fn("XPathResult", illegal_ctor);
        if let Value::Obj(fid) = xr_f {
            for (k, v) in [
                ("ANY_TYPE", 0),
                ("NUMBER_TYPE", 1),
                ("STRING_TYPE", 2),
                ("BOOLEAN_TYPE", 3),
                ("UNORDERED_NODE_ITERATOR_TYPE", 4),
                ("ORDERED_NODE_ITERATOR_TYPE", 5),
                ("UNORDERED_NODE_SNAPSHOT_TYPE", 6),
                ("ORDERED_NODE_SNAPSHOT_TYPE", 7),
                ("ANY_UNORDERED_NODE_TYPE", 8),
                ("FIRST_ORDERED_NODE_TYPE", 9),
            ] {
                self.set_own(fid, k, Value::Num(f64::from(v)), false);
            }
            let p = self.new_object();
            self.set_own(fid, "prototype", Value::Obj(p), false);
        }
        self.set_own(g, "XPathResult", xr_f, false);
        // Interfaces événement complémentaires — rail
        // `event_ctor` générique (init copié, proto hérité de
        // `Event.prototype` — CAP-178).
        for name in [
            "ProgressEvent",
            "PageTransitionEvent",
            "BeforeUnloadEvent",
            "FormDataEvent",
            "MediaQueryListEvent",
            "NotificationEvent",
            "MediaEncryptedEvent",
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
        // `new Option(text?, value?, defaultSelected?, selected?)`
        // — constructeur historique des `<option>` (form libs :
        // `new Option('x', v)` alias `createElement`).
        let opt_f = self.native_fn("Option", option_ctor);
        if let Value::Obj(fid) = opt_f {
            let p = self.new_object();
            self.set_own(fid, "prototype", Value::Obj(p), false);
        }
        self.set_own(g, "Option", opt_f, false);
        // `DOMPoint`/`DOMMatrix` — math 2D réelle bornée
        // (transforms lues par les scripts canvas/layout :
        // `multiply`/`invert`/`transformPoint`/`matrix(...)`).
        for (name, f2) in [
            ("DOMPoint", dompoint_ctor as crate::heap::NativeFn),
            ("DOMMatrix", dommatrix_ctor),
        ] {
            let f = self.native_fn(name, f2);
            if let Value::Obj(fid) = f {
                let p = self.new_object();
                self.set_own(fid, "prototype", Value::Obj(p), false);
            }
            self.set_own(g, name, f, false);
        }
        // `ImageData` — `{width, height, data:Uint8ClampedArray}`
        // spec (ctor 2 signatures : `(w,h)` / `(u8c,w,h?)`).
        let id_f = self.native_fn("ImageData", imagedata_ctor);
        if let Value::Obj(fid) = id_f {
            let p = self.new_object();
            self.set_own(fid, "prototype", Value::Obj(p), false);
        }
        self.set_own(g, "ImageData", id_f, false);
        // `Path2D` — enregistreur d'ops borné (les ctx `fill`/
        // `stroke` le consommeront quand le raster vivra —
        // aujourd'hui : stockage + détection).
        let p2d_f = self.native_fn("Path2D", path2d_ctor);
        if let Value::Obj(fid) = p2d_f {
            let p = self.new_object();
            self.set_own(fid, "prototype", Value::Obj(p), false);
        }
        self.set_own(g, "Path2D", p2d_f, false);
        // `OffscreenCanvas` — `<canvas>` détaché du miroir :
        // `getContext('2d')` enregistre via `Ctx2dHost` (même
        // mécanique que l'élément, JS-C28).
        let oc_f = self.native_fn("OffscreenCanvas", offscreen_canvas_ctor);
        if let Value::Obj(fid) = oc_f {
            let p = self.new_object();
            self.set_own(fid, "prototype", Value::Obj(p), false);
        }
        self.set_own(g, "OffscreenCanvas", oc_f, false);
        // `PerformanceObserver`/`ReportingObserver` — ctors
        // **légalement constructibles** (régression mesurée :
        // github `environment.js` meurt sur le throw du stub
        // où la garde `typeof` sautait proprement avant C98) —
        // impl bornée headless : callback enregistré,
        // `observe`/`disconnect`/`takeRecords` honnêtes,
        // jamais tirés (aucune entrée produite).
        for (name, f2) in [
            (
                "PerformanceObserver",
                perf_observer_ctor as crate::heap::NativeFn,
            ),
            ("ReportingObserver", reporting_observer_ctor),
        ] {
            let f = self.native_fn(name, f2);
            if let Value::Obj(fid) = f {
                let p = self.new_object();
                self.set_own(fid, "prototype", Value::Obj(p), false);
                // `supportedEntryTypes` — `[]` honnête (aucune
                // entrée produite → aucun type supporté ; les
                // gardes `.includes('x')` sautent proprement).
                let arr = self.new_array(Vec::new());
                self.set_own(fid, "supportedEntryTypes", arr, false);
            }
            self.set_own(g, name, f, false);
        }
        // `XPathEvaluator` — `new` + `evaluate` (rail
        // `xpath_evaluate` existant — CAP-188).
        let xe = self.native_fn("XPathEvaluator", xpath_evaluator_ctor);
        if let Value::Obj(fid) = xe {
            let p = self.new_object();
            self.set_own(fid, "prototype", Value::Obj(p), false);
        }
        self.set_own(g, "XPathEvaluator", xe, false);
        // `FontFace` — `{family, status:'unloaded', load() →
        // Promise résolue self}` (le chargement de fontes réel
        // est un chantier session — le feature-detect des
        // loaders webfont passe).
        let ff_f = self.native_fn("FontFace", fontface_ctor);
        if let Value::Obj(fid) = ff_f {
            let p = self.new_object();
            self.set_own(fid, "prototype", Value::Obj(p), false);
        }
        self.set_own(g, "FontFace", ff_f, false);
        // `createImageBitmap` → promesse résolue
        // `{width,height,close()}` (ImageBitmap-like).
        self.add_method(g, "createImageBitmap", create_image_bitmap);
        // `stop`/`print` no-ops honnêtes ; `find` → `false`
        // spec (jamais de sélection — CAP-178).
        self.add_method(g, "stop", win_noop);
        self.add_method(g, "print", win_noop);
        self.add_method(g, "find", win_find);
        // `show*FilePicker`/`showDirectoryPicker` — geste
        // utilisateur jamais présent ici → rejet `SecurityError`
        // (même réponse que Chrome hors geste).
        self.add_method(g, "showOpenFilePicker", picker_reject);
        self.add_method(g, "showSaveFilePicker", picker_reject);
        self.add_method(g, "showDirectoryPicker", picker_reject);
        // Props window — legacy (`status`/`opener`/`external`)
        // et état de contexte lu par les guards modernes.
        let empty = self.str_val("");
        self.set_own(g, "status", empty, true);
        self.set_own(g, "opener", Value::Null, true);
        self.set_own(g, "frameElement", Value::Null, true);
        let ext = self.new_object();
        self.set_own(g, "external", Value::Obj(ext), true);
        self.set_own(g, "isSecureContext", Value::Bool(true), false);
        self.set_own(g, "crossOriginIsolated", Value::Bool(false), false);
        // `origin` ≙ `location.origin` — lu à l'install (la nav
        // met `location` à jour côté hooks ; l'embedder peut
        // écraser la prop ensuite).
        let origin = self
            .get_prop(g, "location")
            .and_then(|l| match l {
                Value::Obj(id) => self.get_prop(id, "origin"),
                _ => None,
            })
            .unwrap_or(Value::Null);
        self.set_own(g, "origin", origin, false);
        // `visualViewport` — métrique figée à l'install
        // (mobile-focused libs la lisent).
        let (vw, vh) = self.hooks.viewport;
        let vv = self.new_object();
        for (k, v) in [
            ("width", f64::from(vw)),
            ("height", f64::from(vh)),
            ("scale", 1.0),
            ("offsetLeft", 0.0),
            ("offsetTop", 0.0),
            ("pageLeft", 0.0),
            ("pageTop", 0.0),
        ] {
            self.set_own(vv, k, Value::Num(v), true);
        }
        self.set_own(vv, "onresize", Value::Null, true);
        self.set_own(vv, "onscroll", Value::Null, true);
        // `vv.addEventListener('resize'|'scroll')` (§B-14 ×4 —
        // edf/lcl/kleinezeitung/hvg vols 329+) : rail
        // EventTarget générique — listeners stockés sur `__et`
        // (un hook viewport pourra les tirer ; aujourd'hui les
        // enregistrer suffit aux guards qui n'attendent que le
        // non-crash).
        self.add_method(vv, "addEventListener", et_add_listener);
        self.add_method(vv, "removeEventListener", et_remove_listener);
        self.add_method(vv, "dispatchEvent", et_dispatch_event);
        self.set_own(g, "visualViewport", Value::Obj(vv), false);
        // `scheduler` — `scheduler.yield()` → promesse résolue
        // (file microtâche existante) ; `postTask(cb,{delay,
        // priority,signal})` (dna vol 126) — promesse résolue
        // au retour du callback, rejetée si le callback jette
        // ou si `signal` est aborté (re-lu au tir — couvre les
        // deux mondes de signaux dom/xhr) ; `priority` accepté
        // mais non ordonnancé (file FIFO — déviation bornée).
        let sch = self.new_object();
        let y = self.native_fn("yield", scheduler_yield);
        self.set_own(sch, "yield", y, true);
        let pt = self.native_fn("postTask", scheduler_post_task);
        self.set_own(sch, "postTask", pt, true);
        self.set_own(g, "scheduler", Value::Obj(sch), false);
        // `cookieStore` — jar vide honnête (get→null,
        // getAll→[], set/delete→resolve) : les guards
        // `cookieStore && …` des loaders passent.
        // `CookieStore` hérite `EventTarget` (§B-14 vol 344,
        // bras nommé par JS-C228 : `addEventListener on
        // Object{get,getAll,set,delete}` — Nuxt entry.js) :
        // `change` est le seul événement de l'interface.
        let cs = self.new_object();
        for (k, f2) in [
            ("get", cookie_get as crate::heap::NativeFn),
            ("getAll", cookie_get_all),
            ("set", cookie_void),
            ("delete", cookie_void),
        ] {
            let m = self.native_fn(k, f2);
            self.set_own(cs, k, m, true);
        }
        self.set_own(cs, "onchange", Value::Null, true);
        self.add_method(cs, "addEventListener", et_add_listener);
        self.add_method(cs, "removeEventListener", et_remove_listener);
        self.add_method(cs, "dispatchEvent", et_dispatch_event);
        let tag = self.str_val("CookieStore");
        self.set_own(cs, crate::interp::SYM_TO_STRING_TAG, tag, false);
        self.set_own(g, "cookieStore", Value::Obj(cs), false);
        // `WebAssembly` — namespace honnête : les binaires wasm
        // sont hors périmètre moteur ; `validate` → `false` (un
        // octet inconnu est « invalide ») et les compiles
        // rejettent — les bundles gardent leur repli JS.
        let wasm = self.new_object();
        self.add_method(wasm, "validate", wasm_false);
        self.add_method(wasm, "compile", wasm_reject);
        self.add_method(wasm, "instantiate", wasm_reject);
        self.add_method(wasm, "compileStreaming", wasm_reject);
        self.add_method(wasm, "instantiateStreaming", wasm_reject);
        for n in [
            "Module",
            "Instance",
            "Memory",
            "Table",
            "Global",
            "CompileError",
            "LinkError",
            "RuntimeError",
            "Tag",
            "Exception",
        ] {
            let f = self.native_fn(n, illegal_ctor);
            if let Value::Obj(fid) = f {
                let p = self.new_object();
                self.set_own(fid, "prototype", Value::Obj(p), false);
            }
            self.set_own(wasm, n, f, false);
        }
        self.set_own(g, "WebAssembly", Value::Obj(wasm), false);
        // `trustedTypes` — politique « passthrough » honnête
        // (pas d'application TT moteur ; les bundles qui
        // enregistrent une `default` policy obtiennent l'API).
        let tt = self.new_object();
        self.add_method(tt, "createPolicy", tt_create_policy);
        self.set_own(g, "trustedTypes", Value::Obj(tt), false);
        // `speechSynthesis` — global `window` spec (jar vide :
        // `getVoices()` → `[]`, speak/cancel no-op — aucune
        // voix embarquée).
        let sp = self.new_object();
        self.add_method(sp, "getVoices", speech_voices);
        self.add_method(sp, "speak", win_noop);
        self.add_method(sp, "cancel", win_noop);
        self.add_method(sp, "pause", win_noop);
        self.add_method(sp, "resume", win_noop);
        for (k, v) in [("paused", false), ("pending", false), ("speaking", false)] {
            self.set_own(sp, k, Value::Bool(v), true);
        }
        self.set_own(g, "speechSynthesis", Value::Obj(sp), false);
        // navigator : `mediaDevices` (rejets `NotFoundError` —
        // aucun périphérique capturable), `userActivation`,
        // `speechSynthesis` (jar vide), `permissions` (query →
        // `{state:'denied'}` — rien n'est accordé).
        if let Some(Value::Obj(nid)) = self.get_prop(g, "navigator") {
            let md = self.new_object();
            self.add_method(md, "getUserMedia", md_reject);
            self.add_method(md, "getDisplayMedia", md_reject);
            self.add_method(md, "enumerateDevices", md_empty_list);
            self.set_own(nid, "mediaDevices", Value::Obj(md), true);
            let ua = self.new_object();
            self.set_own(ua, "hasBeenActive", Value::Bool(false), true);
            self.set_own(ua, "isActive", Value::Bool(false), true);
            self.set_own(nid, "userActivation", Value::Obj(ua), true);
            let perm = self.new_object();
            self.add_method(perm, "query", perm_query);
            self.set_own(nid, "permissions", Value::Obj(perm), true);
        }

        // `autofocus` (CAP-162 — spec « autofocus processing »,
        // borné) : le premier élément `[autofocus]` focalisable
        // reçoit le focus à l'install — `document.activeElement`
        // et les prochains `focus()` le voient. Pas de dispatch
        // (les listeners ne peuvent exister qu'après install —
        // les scripts voient l'état déjà posé).
        let af = {
            let d = dom.borrow();
            d.nodes
                .iter()
                .enumerate()
                .find(|(i, n)| {
                    n.kind == crate::dom::NodeKind::Element
                        && n.attached
                        && d.is_focusable(*i as u32)
                        && d.attr(*i as u32, "autofocus").is_some()
                })
                .map(|(i, _)| i as u32)
        };
        if let Some(n) = af {
            dom.borrow_mut().focused.set(Some(n));
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
        let (bubbles, cancelable, composed) = platform_event_flags(ty);
        let flags = EventFlags {
            bubbles,
            cancelable,
            composed,
            trusted: true,
        };
        let state = make_event_state(ty, node, flags, props);
        // Synchro miroir : un `focus`/`blur` dispatché par
        // l'hôte (input réel → `set_focus` → chemin — trusted)
        // met à jour `dom.focused` — `document.activeElement`
        // suit le focus réel, pas seulement `el.focus()` ni
        // les `dispatchEvent` synthétiques (écart form github
        // mesuré : `button.flash-close` stale).
        match ty {
            "focus" => dom.borrow_mut().focused.set(Some(node)),
            "blur" => dom.borrow_mut().focused.set(None),
            _ => {}
        }
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
                    // MSE-4 : les codecs que l'hôte déclare
                    // au claim alimentent la table consultée
                    // par `MediaSource.isTypeSupported`.
                    crate::mse::note_declared_codecs(self, &c);
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

    /// Déclare les codecs hôte sans nœud résolu — la table
    /// `isTypeSupported` est document-locale : un claim dont
    /// le path ne résout plus (miroir divergent) doit quand
    /// même alimenter les capacités déclarées (vol 176-bis).
    pub fn note_media_codecs(&mut self, codecs: &[String]) {
        crate::mse::note_declared_codecs(self, codecs);
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
        let (bubbles, cancelable, composed) = platform_event_flags(ty);
        let state = make_event_state(
            ty,
            node,
            EventFlags {
                bubbles,
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
        // §B-37 : un handle JS peut survivre à une arène
        // reconstruite (re-parse `document.open`, mirror resync)
        // — dispatcher sur un id hors arène = no-op, jamais panic.
        if !dom.borrow().node_valid(target) {
            return Ok(());
        }
        let path = {
            let d = dom.borrow();
            let mut p = d.ancestors(target); // parent … root
                                             // CAP-198 — `composed` traverse la frontière shadow :
                                             // chaque racine shadow atteinte est suivie de son
                                             // hôte puis de ses ancêtres (imbriqués compris).
                                             // `composed:false` s'arrête à la racine — la racine
                                             // shadow n'a pas de parent, `ancestors` tronque déjà.
            if ev.composed.get() {
                for _ in 0..64 {
                    // `target` lui-même peut être la racine
                    // shadow (`root.dispatchEvent(ev)`) — le
                    // sommet effectif est alors la cible.
                    let top = p.last().copied().unwrap_or(target);
                    let Some(host) = d.shadow_host(top) else {
                        break;
                    };
                    p.push(host);
                    p.extend(d.ancestors(host));
                }
            }
            p
        };
        // `composedPath` réel pendant le dispatch (spec : le
        // chemin inclut les racines shadow quel que soit le
        // mode ; vidé à la sortie → `[]` hors dispatch).
        *ev.path.borrow_mut() = std::iter::once(target)
            .chain(path.iter().copied())
            .collect();
        let ev_val = self.event_value(dom, cache, ev);
        // CAP-198 — `relatedTarget` retargeté comme `target` :
        // résolu ici, présenté par `fire_listeners` via
        // `adjusted_related` (lu par `EventHost::get`).
        let rt = self.get_prop_value(ev_val, "relatedTarget")?;
        ev.related_node.set(node_for_in(self, rt, dom));
        ev.adjusted.set(target);
        ev.adjusted_related.set(ev.related_node.get());
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
        ev.path.borrow_mut().clear();
        ev.adjusted_related.set(None);
        Ok(())
    }

    /// `retarget` spec (CAP-198) — la cible vue depuis le
    /// listener `n` : tant que la racine de `t` diffère de celle
    /// de `n`, `t` remonte par son hôte shadow (le listener ne
    /// voit pas sous sa frontière). Borné anti-cycle.
    fn retarget_node(d: &Dom, mut t: NodeId, n: NodeId) -> NodeId {
        let root_n = d.root_of(n);
        for _ in 0..64 {
            let root_t = d.root_of(t);
            if root_t == root_n {
                break;
            }
            let Some(host) = d.shadow_host(root_t) else {
                break;
            };
            t = host;
        }
        t
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
        // CAP-198 — `target`/`relatedTarget` ré-étiquetés pour
        // CE listener avant invocation (spec « retarget »).
        let (adjusted, related) = {
            let d = dom.borrow();
            (
                Self::retarget_node(&d, ev.target.get(), node),
                ev.related_node
                    .get()
                    .map(|rn| Self::retarget_node(&d, rn, node)),
            )
        };
        ev.adjusted.set(adjusted);
        ev.adjusted_related.set(related);
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

/// Auto-vive la pièce du squelette document (`html`/`head`/
/// `body`) si absente du miroir — invariant HTML « un document
/// a toujours ces trois éléments » (le parseur les génère même
/// vides ; les miroirs de `about:blank` ou de fragments n'en
/// ont pas). Même insertion que le getter `document.head`
/// (JS-C89) : `html` enfant de la racine, `head` premier
/// enfant de `html`, `body` dernier.
fn ensure_doc_part(dom: &DomRef, tag: &str) {
    let mut d = dom.borrow_mut();
    let root = d.root;
    if !d.by_tag(root, tag).is_empty() {
        return;
    }
    let mut html_el = d.by_tag(root, "html").first().copied();
    if html_el.is_none() {
        let h = d.create_element("html");
        d.append(root, h);
        html_el = Some(h);
    }
    let Some(html_el) = html_el else {
        return;
    };
    match tag {
        "head" => {
            let h = d.create_element("head");
            let first = d.child_nodes(html_el).first().copied();
            d.insert_before(html_el, h, first);
        }
        "body" => {
            let b = d.create_element("body");
            d.append(html_el, b);
            // Un miroir fragmentaire (`DOMParser.parseFrom
            // String`, `from_html` sur fragment) laisse le
            // contenu à plat sous la racine — spec `text/html` :
            // tout ce qui n'est pas `head`/doctype vit dans
            // `body`. Le body auto-vivifié adopte ces
            // orphelins (les commentaires restent au niveau
            // document) sinon `d.body` rendait un élément vide
            // pendant que `<p>` traînait sous la racine.
            let strays: Vec<NodeId> = d
                .child_nodes(root)
                .into_iter()
                .filter(|&c| c != html_el && d.nodes[c as usize].kind != NodeKind::Comment)
                .collect();
            for c in strays {
                if d.nodes[c as usize].tag == "head" {
                    d.insert_before(html_el, c, Some(b));
                } else {
                    d.append(b, c);
                }
            }
        }
        _ => {}
    }
}

impl Host for DocumentHost {
    fn has_method(&self, name: &str) -> bool {
        DOC_METHODS.contains(&name)
    }
    fn class_name(&self) -> &'static str {
        "HTMLDocument"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn trace(&self, out: &mut Vec<ObjId>) {
        trace_node_cache(&self.cache, out);
        if let Some(v) = *self.implementation.borrow() {
            crate::heap::push_value(out, v);
        }
        if let Some(v) = *self.fonts.borrow() {
            crate::heap::push_value(out, v);
        }
        if let Some(v) = *self.adopted_sheets.borrow() {
            crate::heap::push_value(out, v);
        }
    }

    fn get(&self, e: &mut Engine, this: Value, key: &str) -> JsResult<Option<Value>> {
        match key {
            "body" => {
                // Un document de cadre (`iframe.contentDocument`,
                // `about:blank` inclus) porte toujours le
                // squelette `<html><head><body>` — auto-vit `body`
                // comme `head` (JS-C89) et `getElementsByTagName`.
                // Sans ça, `d.body()` repliant sur la racine
                // rendait le nœud Document → `f.contentDocument.
                // body.appendChild` jettait `(on HTMLDocument)`
                // (moin/degiro, web-agri — JS-C208).
                ensure_doc_part(&self.dom, "body");
                let n = self.dom.borrow().body();
                Ok(Some(node_value(e, &self.dom, &self.cache, n)))
            }
            // `document.activeElement` (CAP-162) — l'élément
            // focalisé du miroir (`Dom::focused` tenu par
            // `el.focus()`/`blur()`/`autofocus`), repli
            // `body`, `null` si ni l'un ni l'autre.
            "activeElement" => {
                let n = self.dom.borrow().active_element();
                Ok(Some(match n {
                    Some(n) => node_value(e, &self.dom, &self.cache, n),
                    None => Value::Null,
                }))
            }
            // Fullscreen (CAP-179) — `fullscreenElement` =
            // l'élément posé par `requestFullscreen` (un
            // seul par document), `fullscreen`/`Enabled` :
            // le miroir le supporte → `true` honnête.
            "fullscreenElement" => Ok(Some(match e.hooks.fullscreen_el {
                Some(n) => node_value(e, &self.dom, &self.cache, n),
                None => Value::Null,
            })),
            "fullscreen" => Ok(Some(Value::Bool(e.hooks.fullscreen_el.is_some()))),
            "fullscreenEnabled" => Ok(Some(Value::Bool(true))),
            // `document.scrollingElement` (spec : root en
            // standards mode — pas de quirks détecté dans le
            // miroir) ; les loaders lazy lisent
            // `scrollingElement.scrollTop` (vol github :
            // `undefined` cassait le trigger de visibilité).
            "documentElement" | "scrollingElement" => {
                let n = self.dom.borrow().document_element();
                Ok(Some(match n {
                    Some(n) => node_value(e, &self.dom, &self.cache, n),
                    None => Value::Null,
                }))
            }
            // Accesseurs Node/ParentNode du Document (spec —
            // JS-C208 : `document.firstChild`/`childNodes`/
            // `children` existent ; receveur = racine).
            "firstChild" | "lastChild" => {
                let n = {
                    let d = self.dom.borrow();
                    let kids = d.child_nodes(d.root);
                    if key == "firstChild" {
                        kids.first().copied()
                    } else {
                        kids.last().copied()
                    }
                };
                Ok(Some(match n {
                    Some(n) => node_value(e, &self.dom, &self.cache, n),
                    None => Value::Null,
                }))
            }
            "childNodes" => {
                let ids = {
                    let d = self.dom.borrow();
                    d.child_nodes(d.root)
                };
                Ok(Some(dom_collection(
                    e,
                    &self.dom,
                    &self.cache,
                    &ids,
                    false,
                    "NodeList",
                )))
            }
            "children" => {
                let ids = {
                    let d = self.dom.borrow();
                    d.element_children(d.root)
                };
                Ok(Some(dom_collection(
                    e,
                    &self.dom,
                    &self.cache,
                    &ids,
                    false,
                    "HTMLCollection",
                )))
            }
            "firstElementChild" | "lastElementChild" => {
                let n = {
                    let d = self.dom.borrow();
                    let kids = d.element_children(d.root);
                    if key == "firstElementChild" {
                        kids.first().copied()
                    } else {
                        kids.last().copied()
                    }
                };
                Ok(Some(match n {
                    Some(n) => node_value(e, &self.dom, &self.cache, n),
                    None => Value::Null,
                }))
            }
            "childElementCount" => {
                let n = {
                    let d = self.dom.borrow();
                    d.element_children(d.root).len()
                };
                Ok(Some(Value::Num(n as f64)))
            }
            // `document.adoptedStyleSheets` (CAP-195 — mesuré :
            // le polyfill Popover de github `[C, ...e.adopted
            // StyleSheets]` mourait `value is not iterable`) —
            // tableau `[]` paresseux par document (identité
            // `===` stable spec), assignable via `set` ;
            // feuilles non appliquées (scoping = tranche 2).
            "adoptedStyleSheets" => {
                let v = *self
                    .adopted_sheets
                    .borrow_mut()
                    .get_or_insert_with(|| e.new_array(Vec::new()));
                Ok(Some(v))
            }
            // `document.styleSheets` — mesuré terrain vaadin :
            // le fixup lumo itère `new Set(e.styleSheets)` à
            // l'upgrade CE → absent = `object is not iterable`
            // (même classe que `adoptedStyleSheets` CAP-195).
            // StyleSheetList bornée : une `CSSStyleSheet` shape
            // par `<style>`/`<link rel=stylesheet>` en ordre
            // document (`ownerNode`, `href`), tableau frais par
            // get — la « live list » reste bornée.
            "styleSheets" => {
                let elems: Vec<NodeId> = {
                    let d = self.dom.borrow();
                    d.by_tag(d.root, "*")
                        .into_iter()
                        .filter(|&n| {
                            let t = &d.nodes[n as usize].tag;
                            t == "style"
                                || (t == "link"
                                    && d.attr(n, "rel").is_some_and(|r| {
                                        r.split_whitespace()
                                            .any(|x| x.eq_ignore_ascii_case("stylesheet"))
                                    }))
                        })
                        .collect()
                };
                let mut sheets = Vec::with_capacity(elems.len());
                for n in elems {
                    let v = crate::misc_apis::css_sheet_ctor(e, Value::Undefined, &[])?;
                    if let Value::Obj(o) = v {
                        let own = node_value(e, &self.dom, &self.cache, n);
                        e.set_own(o, "ownerNode", own, true);
                        let href = {
                            let d = self.dom.borrow();
                            d.attr(n, "href").map(str::to_owned)
                        };
                        if let Some(h) = href {
                            let hv = e.str_val(h);
                            e.set_own(o, "href", hv, true);
                        }
                    }
                    sheets.push(v);
                }
                Ok(Some(e.new_array(sheets)))
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
                let mut slot = self.implementation.borrow_mut();
                // Garde GC — un `Value::Obj` balayé est recréé
                // plutôt que rendu zombie (même logique que
                // `node_value`).
                if slot.is_some_and(|v| matches!(v, Value::Obj(id) if e.heap.obj(id).is_none())) {
                    *slot = None;
                }
                let v =
                    *slot.get_or_insert_with(|| e.new_host_object(Rc::new(DomImplementationHost)));
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
            // `document.baseURI` — la base de résolution des
            // URLs relatives (spec : `<base href>` résolu s'il
            // existe, sinon l'URL du document). Mur corpus
            // svelte/fastapi/mkdocs : `new URL(".", document.
            // baseURI)` → « Invalid base URL » sans la prop.
            "baseURI" => {
                let href = {
                    let d = self.dom.borrow();
                    d.by_tag(d.root, "base")
                        .iter()
                        .find_map(|&n| d.attr(n, "href").map(str::to_owned))
                };
                let base = href
                    .and_then(|h| crate::xhr::resolve_url(e, &h))
                    .unwrap_or_else(|| e.hooks.page_url.clone());
                Ok(Some(e.str_val(base)))
            }
            // `document.referrer` (CAP-156) — le `Referer` de
            // la navigation du document, `""` hors referrer
            // (spec : reflète le referrer initial, jamais la
            // politique des requêtes ultérieures).
            "referrer" => Ok(Some(e.str_val(e.hooks.page_referrer.clone()))),
            // `document.currentScript` (CAP-168) — l'élément
            // `<script>` en cours d'évaluation, `null` sinon
            // (spec : posé uniquement pendant l'exécution).
            "currentScript" => Ok(Some(match e.hooks.current_script {
                Some(n) => node_value(e, &self.dom, &self.cache, n),
                None => Value::Null,
            })),
            // `document.links`/`images`/`forms`/`scripts` — live-ish
            // (recomputed at each access) tag collections ≙ amont
            // `links[]`. Accès nommé (`document.forms.login`) +
            // `namedItem` (named getter `HTMLCollection`, JS-C76).
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
                Ok(Some(dom_collection(
                    e,
                    &self.dom,
                    &self.cache,
                    &ids,
                    true,
                    "HTMLCollection",
                )))
            }
            "head" => {
                // Spec : `head` existe TOUJOURS (le parseur
                // l'auto-génère même sans `<head>` dans la
                // source). Miroir sans head → on le crée et
                // l'insère avant `body` (JS-C89 — le hoisting
                // `<title>`/metadata de React 19 fait
                // `document.head.insertBefore` au mount).
                let n = {
                    let mut d = self.dom.borrow_mut();
                    d.by_tag(d.root, "head").first().copied().or_else(|| {
                        let h = d.create_element("head");
                        let html_el = d.by_tag(d.root, "html").first().copied().unwrap_or(d.root);
                        let first = d.child_nodes(html_el).first().copied();
                        d.insert_before(html_el, h, first);
                        Some(h)
                    })
                };
                Ok(Some(match n {
                    Some(n) => node_value(e, &self.dom, &self.cache, n),
                    None => Value::Null,
                }))
            }
            // `document.readyState` (CAP-170) — cycle de vie
            // piloté par l'embedder : `""` (jamais posé) =
            // `"loading"` (le miroir est installé en cours de
            // parse) ; transitions `interactive`/`complete`
            // via `Engine::set_ready_state`. CAP-171 : lu sur
            // le ctx **propriétaire** du miroir — un
            // `contentDocument` d'iframe rapporte SON état.
            "readyState" => Ok(Some(
                e.str_val(
                    e.dom_owner_ctx(&self.dom)
                        .map_or("loading", |ctx| e.ready_state_ctx(ctx)),
                ),
            )),
            // `document.fonts` — FontFaceSet stub **unique par
            // document** (spec) : pas de pipeline de fontes web,
            // `load()`/`ready` résolvent toujours ; l'unicité
            // rend l'état setlike (`add`/`has`) observable
            // entre accès — excalidraw vol 222.
            "fonts" => {
                let mut slot = self.fonts.borrow_mut();
                if slot.is_some_and(|v| matches!(v, Value::Obj(id) if e.heap.obj(id).is_none())) {
                    *slot = None;
                }
                let v = *slot.get_or_insert_with(|| e.new_host_object(Rc::new(FontFaceSetHost)));
                Ok(Some(v))
            }
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
            // `document.domain` (issue #24 — le module
            // `behaviors` de github throw `Unable to get
            // document domain` sans lui) : l'override posé
            // par le setter, sinon le hostname de la page.
            "domain" => Ok(Some(
                e.str_val(
                    self.domain_override
                        .borrow()
                        .clone()
                        .unwrap_or_else(|| url_parts(&e.hooks.page_url).2),
                ),
            )),
            // Méthodes lisibles (JS-C80) — `document.createElement`
            // & co. rendent une fonction bornée → `invoke` : le
            // sniff `typeof d.createElement` (React `canUseDOM`)
            // l'exige — invoke-only le rendait `undefined`.
            k if DOC_METHODS.contains(&k) => {
                let nv = e.str_val(k);
                Ok(Some(e.bound_native(
                    "document.method",
                    host_method_trampoline,
                    vec![nv, this],
                )))
            }
            // Handlers `on*` sur `document` (JS-C80) —
            // `"oninput" in document` est le sniff de support
            // de React (`Lj`) qui arme `ChangeEventPlugin` ;
            // sans lui les `onChange` des champs contrôlés
            // tombent sur le chemin IE8 `propertychange`.
            k if is_event_handler_prop(k) => on_handler_get(e, &self.dom, None, this, k),
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
        if key == "domain" {
            // Spec : seul un suffixe enregistrable du host
            // est accepté (raccourcit le domaine effectif) —
            // faute de PSL, on borne à « suffixe à frontière
            // de label » du hostname courant ; non-suffixe
            // ignoré (tolérant — spec : SecurityError).
            let s = e.to_js_string(v)?;
            let host = url_parts(&e.hooks.page_url).2;
            let ok = host == s
                || (host.len() > s.len() + 1
                    && host.ends_with(&s)
                    && host.as_bytes()[host.len() - s.len() - 1] == b'.');
            if ok && !s.is_empty() {
                *self.domain_override.borrow_mut() = Some(s);
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
        if key == "adoptedStyleSheets" {
            // Assignation acceptée (CAP-195) — la liste remplace
            // le tableau courant ; les `ObjId` des feuilles sont
            // enregistrés sur le miroir — la session les lit via
            // `adopted_sheets_css` pour la cascade (tranche 2).
            if let Value::Obj(aid) = v {
                let sheets: Vec<ObjId> = {
                    match e.heap.obj(aid).map(|o| &o.kind) {
                        Some(ObjKind::Array(elems)) => elems
                            .iter()
                            .filter_map(|el| match el {
                                Value::Obj(id) => Some(*id),
                                _ => None,
                            })
                            .collect(),
                        _ => Vec::new(),
                    }
                };
                self.dom.borrow_mut().adopted_sheets = sheets;
            } else {
                self.dom.borrow_mut().adopted_sheets.clear();
            }
            *self.adopted_sheets.borrow_mut() = Some(v);
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
            // `document.getSelection()` — même `Selection`
            // partagée que `window.getSelection` (JS-C74).
            "getSelection" => {
                let v = crate::misc_apis::get_selection(e, Value::Undefined, &[])?;
                Ok(Some(v))
            }
            // `document.contains(node)` — `Node.contains` au
            // niveau document (ratp vol 45 : la nav off-canvas
            // teste `document.contains(el)` — absent → `of
            // Undefined` dans son handler).
            "contains" => {
                let r = node_for_in(e, arg(args, 0), &self.dom).is_some_and(|o| {
                    let d = self.dom.borrow();
                    d.contains(d.root, o)
                });
                Ok(Some(Value::Bool(r)))
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
                // `html`/`head`/`body` existent TOUJOURS dans un
                // document (le parseur HTML les auto-génère) — les
                // documents de cadre vides (`about:blank`, miroir
                // sans squelette) les auto-vivent comme le getter
                // `head` (JS-C89). Sans ça, `getElementsByTagName(
                // 'head')[0]` rend `undefined` → `appendChild`
                // jette (bootstrap Cloudflare `c()` de jsfiddle/
                // codepen, ×12 en boucle à chaque input).
                if matches!(tag.as_str(), "html" | "head" | "body") {
                    ensure_doc_part(&self.dom, &tag);
                }
                let ids = {
                    let d = self.dom.borrow();
                    d.by_tag(d.root, &tag)
                };
                Ok(Some(dom_collection(
                    e,
                    &self.dom,
                    &self.cache,
                    &ids,
                    false,
                    "HTMLCollection",
                )))
            }
            "getElementsByClassName" => {
                let cls = e.to_js_string(arg(args, 0))?;
                let ids = {
                    let d = self.dom.borrow();
                    d.by_class(d.root, &cls)
                };
                Ok(Some(dom_collection(
                    e,
                    &self.dom,
                    &self.cache,
                    &ids,
                    false,
                    "HTMLCollection",
                )))
            }
            // `getElementsByName` — attribut `name` (JS-C76).
            "getElementsByName" => {
                let name = e.to_js_string(arg(args, 0))?;
                let ids = {
                    let d = self.dom.borrow();
                    d.nodes
                        .iter()
                        .enumerate()
                        .filter(|(_, n)| {
                            n.kind == NodeKind::Element
                                && n.attrs.iter().any(|(k, v)| k == "name" && v == &name)
                        })
                        .map(|(i, _)| i as NodeId)
                        .collect::<Vec<_>>()
                };
                Ok(Some(dom_collection(
                    e,
                    &self.dom,
                    &self.cache,
                    &ids,
                    false,
                    "NodeList",
                )))
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
                Ok(Some(dom_collection(
                    e,
                    &self.dom,
                    &self.cache,
                    &ids,
                    false,
                    "NodeList",
                )))
            }
            "createElement" => {
                let tag = e.to_js_string(arg(args, 0))?;
                let n = self.dom.borrow_mut().create_element(&tag);
                // CE défini → upgrade immédiat (ctor + attrs ;
                // pas de `connectedCallback` — détaché, JS-C68).
                ce_upgrade_node(e, &self.dom, &self.cache, n)?;
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
                ce_upgrade_node(e, &self.dom, &self.cache, n)?;
                Ok(Some(node_value(e, &self.dom, &self.cache, n)))
            }
            "createTextNode" => {
                let t = e.to_js_string(arg(args, 0))?;
                let n = self.dom.borrow_mut().create_text(&t);
                Ok(Some(node_value(e, &self.dom, &self.cache, n)))
            }
            // `document.createComment(data)` — `nodeType` 8 ;
            // Lit y dépose ses marqueurs de parties
            // (`?lit$N$`/`?node=`) — vaadin terrain.
            "createComment" => {
                let t = e.to_js_string(arg(args, 0))?;
                let n = self.dom.borrow_mut().create_comment(&t);
                Ok(Some(node_value(e, &self.dom, &self.cache, n)))
            }
            // `document.createAttribute(name)` — spec Attr :
            // l'arène stocke les attributs en paires
            // `(nom, valeur)` — l'`Attr` est un objet porteur
            // `{nodeType:2, name, value:"", ownerElement:null…}`
            // que `setAttributeNode`/`getAttributeNode`/
            // `removeAttributeNode` lisent (§B-14 EXPLOR74
            // npr ×4 sites — `createAttribute is not a
            // function`).
            "createAttribute" => {
                let name = e.to_js_string(arg(args, 0))?;
                Ok(Some(attr_obj(e, &name, "", None)))
            }
            // `document.createAttributeNS(ns, qualifiedName)` —
            // même rail : l'URI est conservée en
            // `namespaceURI`, le nom qualifié garde son
            // préfixe (`p:l` → `prefix`/`localName`).
            "createAttributeNS" => {
                let ns = e.to_js_string(arg(args, 0))?;
                let name = e.to_js_string(arg(args, 1))?;
                Ok(Some(attr_obj(e, &name, "", Some(ns))))
            }
            // `document.importNode(node, deep)` (CAP-177) —
            // spec : clone dans le document cible ; le miroir
            // n'a qu'un document → même rail que `cloneNode`.
            // `document.importNode(node, deep?)` — spec : clone
            // du nœud (propre ou **étranger**) sans détacher ni
            // rebind (≠ `adoptNode`). §B-14 vol 304 usnews ×5 :
            // nœud d'un document auxiliaire (template parse
            // jQuery) rejeté « nœud requis » — clone cross-arène
            // posé (`adopt_from` profond / `clone_shallow_from`).
            "importNode" => {
                let v = arg(args, 0);
                let deep = e.to_boolean(arg(args, 1));
                let c = if let Some(src) = node_for_in(e, v, &self.dom) {
                    self.dom.borrow_mut().clone_node(src, deep)
                } else if let Some((src, src_dom)) = foreign_node_in(e, v, &self.dom) {
                    if deep {
                        let s = src_dom.borrow();
                        self.dom.borrow_mut().adopt_from(&s, src)
                    } else {
                        let s = src_dom.borrow();
                        self.dom.borrow_mut().clone_shallow_from(&s, src)
                    }
                } else if let Some((name, value, ns)) = attr_shape_in(e, v)? {
                    // §B-14 usnews REVALID5 — spec :
                    // `importNode(Attr)` rend un Attr cloné
                    // dans le document cible (nos porteurs
                    // `{nodeType:2}` ne sont pas des `Host`).
                    return Ok(Some(attr_obj(e, &name, &value, ns)));
                } else {
                    // Message diagnostique : le type reçu nomme
                    // le bras au prochain vol (spec = TypeError
                    // aussi pour undefined/null — parité Chrome).
                    let t = match v {
                        Value::Undefined => "undefined",
                        Value::Null => "null",
                        Value::Bool(_) => "boolean",
                        Value::Num(_) => "number",
                        Value::Str(_) => "string",
                        Value::Big(_) => "bigint",
                        _ => "object non-nœud",
                    };
                    return Err(e.type_error(format!("importNode: nœud requis (reçu={t})")));
                };
                ce_upgrade_subtree(e, &self.dom, &self.cache, c)?;
                Ok(Some(node_value(e, &self.dom, &self.cache, c)))
            }
            // `document.adoptNode(node)` (CAP-177) — spec :
            // détache le nœud et l'adopte dans le document
            // cible ; miroir mono-document → detach + retour
            // (le re-propriétage est un no-op — il n'y a qu'un
            // document).
            // `exitFullscreen` (CAP-179) — promesse résolue,
            // `fullscreenElement` effacé, `fullscreenchange`
            // sur l'ancien élément. Sans fullscreen →
            // promesse résolue no-op (spec : si
            // `fullscreenElement` est `null`, résoudre).
            "exitFullscreen" => {
                let pid = e.new_promise();
                if let Some(n) = e.hooks.fullscreen_el.take() {
                    e.dispatch_simple(&self.dom, &self.cache, n, "fullscreenchange")?;
                }
                e.resolve_promise(pid, Value::Undefined);
                Ok(Some(Value::Obj(pid)))
            }
            // Hit-test inverse (CAP-180) — même `GeometryMap`
            // publiée que `getBoundingClientRect` (i#19) :
            // `elementFromPoint` rend le candidat
            // topmost-first, `elementsFromPoint` tous les
            // contenus dans cet ordre, `caretPositionFromPoint`
            // `{offsetNode, offset:0}` (borné : pas de mesure
            // de glyphe — l'offset dans le texte n'est pas
            // calculable sans shaping). Sans table → `null`/
            // `[]`/`null` (document sans paint publiée).
            "elementFromPoint" => {
                let x = e.to_number(arg(args, 0))?;
                let y = e.to_number(arg(args, 1))?;
                let hits = point_candidate_paths(&self.dom, x, y);
                let n = hits.first().and_then(|p| self.dom.borrow().node_at_path(p));
                Ok(Some(match n {
                    Some(n) => node_value(e, &self.dom, &self.cache, n),
                    None => Value::Null,
                }))
            }
            "elementsFromPoint" => {
                let x = e.to_number(arg(args, 0))?;
                let y = e.to_number(arg(args, 1))?;
                let hits = point_candidate_paths(&self.dom, x, y);
                let nodes: Vec<Value> = hits
                    .iter()
                    .filter_map(|p| self.dom.borrow().node_at_path(p))
                    .map(|n| node_value(e, &self.dom, &self.cache, n))
                    .collect();
                Ok(Some(e.new_array(nodes)))
            }
            "caretPositionFromPoint" => {
                let x = e.to_number(arg(args, 0))?;
                let y = e.to_number(arg(args, 1))?;
                let hits = point_candidate_paths(&self.dom, x, y);
                let Some(n) = hits.first().and_then(|p| self.dom.borrow().node_at_path(p)) else {
                    return Ok(Some(Value::Null));
                };
                let o = e.new_object();
                let nv = node_value(e, &self.dom, &self.cache, n);
                e.set_own(o, "offsetNode", nv, true);
                e.set_own(o, "offset", Value::Num(0.0), true);
                Ok(Some(Value::Obj(o)))
            }
            // `createRange`/`createTreeWalker`/`execCommand`
            // (CAP-182, arc édition) — Range vivant (bornes
            // mutables), TreeWalker DFS filtré (borné :
            // navigation inverse non portée), execCommand
            // `copy`/`cut`→hook clipboard/`selectAll`→range
            // body (le reste → `false` honnête).
            "createRange" => Ok(Some(crate::misc_apis::range_ctor(
                e,
                Value::Undefined,
                args,
            )?)),
            "createTreeWalker" => Ok(Some(crate::misc_apis::create_treewalker(
                e,
                Value::Undefined,
                args,
            )?)),
            "createNodeIterator" => Ok(Some(crate::misc_apis::create_nodeiterator(
                e,
                Value::Undefined,
                args,
            )?)),
            "execCommand" => Ok(Some(crate::misc_apis::exec_command(
                e,
                Value::Undefined,
                args,
            )?)),
            "getAnimations" => Ok(Some(crate::misc_apis::get_animations(
                e,
                Value::Undefined,
                args,
            )?)),
            "startViewTransition" => Ok(Some(crate::misc_apis::start_view_transition(
                e,
                Value::Undefined,
                args,
            )?)),
            "evaluate" => Ok(Some(crate::misc_apis::xpath_evaluate(
                e,
                Value::Undefined,
                args,
            )?)),
            // DOM Level 3 XPath (mur htmx/hyperscript —
            // `createExpression is not a function`).
            "createExpression" => Ok(Some(crate::misc_apis::xpath_create_expression(
                e,
                Value::Undefined,
                args,
            )?)),
            "createNSResolver" => Ok(Some(crate::misc_apis::create_ns_resolver(
                e,
                Value::Undefined,
                args,
            )?)),
            // Storage Access API (JS-C98) — premier parti :
            // `hasStorageAccess` → `true`, `requestStorageAccess`
            // → résolu (aucune restriction tiers ici).
            "hasStorageAccess" => {
                let id = e.new_promise();
                e.resolve_promise(id, Value::Bool(true));
                Ok(Some(Value::Obj(id)))
            }
            "requestStorageAccess" => {
                let id = e.new_promise();
                e.resolve_promise(id, Value::Undefined);
                Ok(Some(Value::Obj(id)))
            }
            "adoptNode" => {
                let Some(src) = node_for_in(e, arg(args, 0), &self.dom) else {
                    return Err(e.type_error("adoptNode: nœud requis"));
                };
                self.dom.borrow_mut().detach(src);
                Ok(Some(node_value(e, &self.dom, &self.cache, src)))
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
                // §B-19 — point d'insertion spec : pendant le
                // chargement le fragment s'insère **après le
                // script courant** (marqueur du parseur), pas
                // en queue de `<body>` — tumblr écrit
                // `main-*.js` via `document.write` et
                // l'ordre de découverte des scripts suivants
                // est préservé. `currentScript` absent ou
                // détaché → repli `<body>` (comportement
                // historique).
                let anchor = e
                    .hooks
                    .current_script
                    .filter(|n| self.dom.borrow().is_attached(*n));
                let mut d = self.dom.borrow_mut();
                if !d.doc_open {
                    let root = d.root;
                    d.replace_children(root, &[]);
                    d.doc_open = true;
                }
                if let Some(cs) = anchor {
                    if let Some(parent) = d.parent(cs) {
                        // Fragment en scratch puis re-parenté
                        // avant le frère suivant — ordre
                        // conservé (`insertAdjacentHTML`
                        // fait de même).
                        let scratch = d.create_element("div");
                        d.parse_fragment_into(scratch, &s);
                        let kids = d.child_nodes(scratch);
                        let next = d.sibling(cs, true);
                        for k in kids {
                            d.insert_before(parent, k, next);
                        }
                    } else {
                        let body = d.body();
                        d.parse_fragment_into(body, &s);
                    }
                } else {
                    let body = d.body();
                    d.parse_fragment_into(body, &s);
                }
                Ok(Some(Value::Undefined))
            }
            // Méthodes Node/ParentNode du Document (spec :
            // `document` EST un nœud parent — `appendChild`,
            // `insertBefore`, `replaceChild`, `prepend`… y
            // existent ; turbolinks fait `document.replaceChild`,
            // les loaders morph `documentElement`). Receveur =
            // racine ; enfant Text illégal → spec
            // `HierarchyRequestError` (JS-C208).
            "append" | "prepend" | "replaceChildren" => {
                let mut kids = Vec::with_capacity(args.len());
                for &a in args {
                    let Some(n) = node_for_in(e, a, &self.dom)
                        .or_else(|| adopt_into(e, a, &self.dom, &self.cache))
                    else {
                        // Chaîne → Text ; Text interdit sous
                        // Document (spec `HierarchyRequestError`).
                        return Err(e.type_error("HierarchyRequestError: text into document"));
                    };
                    if self.dom.borrow().nodes[n as usize].kind == NodeKind::Text {
                        return Err(e.type_error("HierarchyRequestError: text into document"));
                    }
                    kids.push(n);
                }
                let root = self.dom.borrow().root;
                match name {
                    "prepend" => {
                        let first = self.dom.borrow().child_nodes(root).first().copied();
                        for &k in &kids {
                            self.dom.borrow_mut().insert_before(root, k, first);
                        }
                    }
                    "replaceChildren" => {
                        self.dom.borrow_mut().replace_children(root, &kids);
                        return Ok(Some(Value::Undefined));
                    }
                    _ => {
                        for &k in &kids {
                            self.dom.borrow_mut().append(root, k);
                        }
                    }
                }
                Ok(Some(arg(args, 0)))
            }
            "appendChild" | "insertBefore" => {
                let child = node_for_in(e, arg(args, 0), &self.dom)
                    .or_else(|| adopt_into(e, arg(args, 0), &self.dom, &self.cache));
                let Some(child) = child else {
                    return Err(e.type_error("appendChild: not a node"));
                };
                if self.dom.borrow().nodes[child as usize].kind == NodeKind::Text {
                    return Err(e.type_error("HierarchyRequestError: text into document"));
                }
                let root = self.dom.borrow().root;
                // Ancêtre/soi → cycle (spec `HierarchyRequestError`
                // — le document ne peut se contenir).
                if self.dom.borrow().is_ancestor_or_self(root, child) {
                    return Err(e.type_error("HierarchyRequestError: ancestor"));
                }
                if name == "insertBefore" {
                    let ref_node = match arg(args, 1) {
                        Value::Null | Value::Undefined => None,
                        v => node_for_in(e, v, &self.dom),
                    };
                    self.dom.borrow_mut().insert_before(root, child, ref_node);
                } else {
                    self.dom.borrow_mut().append(root, child);
                }
                Ok(Some(arg(args, 0)))
            }
            "replaceChild" => {
                let child = node_for_in(e, arg(args, 0), &self.dom)
                    .or_else(|| adopt_into(e, arg(args, 0), &self.dom, &self.cache));
                let Some(child) = child else {
                    return Err(e.type_error("replaceChild: not a node"));
                };
                if self.dom.borrow().nodes[child as usize].kind == NodeKind::Text {
                    return Err(e.type_error("HierarchyRequestError: text into document"));
                }
                let Some(old) = node_for_in(e, arg(args, 1), &self.dom) else {
                    return Err(e.type_error("replaceChild: old not a node"));
                };
                let root = self.dom.borrow().root;
                if self.dom.borrow().is_ancestor_or_self(root, child) {
                    return Err(e.type_error("HierarchyRequestError: ancestor"));
                }
                self.dom.borrow_mut().insert_before(root, child, Some(old));
                self.dom.borrow_mut().detach(old);
                Ok(Some(arg(args, 1)))
            }
            "removeChild" => {
                let target = node_for_in(e, arg(args, 0), &self.dom);
                match target {
                    Some(child) => {
                        self.dom.borrow_mut().detach(child);
                        Ok(Some(arg(args, 0)))
                    }
                    None => Err(e.type_error("removeChild: not a node")),
                }
            }
            "hasChildNodes" => {
                let has = {
                    let d = self.dom.borrow();
                    !d.child_nodes(d.root).is_empty()
                };
                Ok(Some(Value::Bool(has)))
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
    // `HTMLElement` globaux (JS-C201) — `documentElement.lang`
    // lu par les pluralizers des bundles i18n (orpi vol 136 :
    // `.lang.replace` tuait le chunk → `Translator` jamais défini
    // → traductions mortes en cascade). Attr absent → `""` (spec).
    "lang",
    "dir",
    // `HTMLMediaElement` (étape 58) — attributs réfléchis
    // de même nom sur `<video>`/`<audio>`.
    "poster",
    "preload",
    // `HTMLIFrameElement` (CAP-171) — `f.srcdoc=`/`sandbox=`
    // écrivent l'attribut (la mutation déclenche la
    // navigation du sous-contexte).
    "srcdoc",
    "sandbox",
    // `HTMLImageElement` (CAP-172) — `m.srcset=`/`sizes=`
    // écrivent l'attribut (le choix de source `srcset`
    // relit `sizes`, spec).
    "srcset",
    "sizes",
    // `HTMLLinkElement`/`HTMLStyleElement`/`HTMLSourceElement`
    // (CAP-173) — `l.media=` écrit l'attribut (gate
    // d'application de la feuille, spec IDL `media`).
    "media",
    // `slot` (T3) — `el.slot=` écrit l'attribut
    // (re-distribution flat recalculée à la prochaine
    // lecture `assigned*`).
    "slot",
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
        "encoding" => "enctype",
        "crossOrigin" => "crossorigin",
        "playsInline" => "playsinline",
        "referrerPolicy" => "referrerpolicy",
        k => return MAPPED_ATTRS.iter().copied().find(|&a| a == k),
    })
}

/// Noms `on*` reconnus comme handlers IDL (`GlobalEventHandlers`,
/// pointer/touch/animation/transition). La table borne le sniff :
/// `on` + nom inconnu reste `undefined` (spec), un nom connu
/// devient une vraie prop (null/fonction) — `"oninput" in
/// document` passe à vrai, ce que React (`Lj`) exige pour
/// armer `ChangeEventPlugin`.
const EVENT_HANDLER_PROPS: &[&str] = &[
    // GlobalEventHandlers (HTML §6.1.6.1).
    "onabort",
    "onauxclick",
    "onbeforeinput",
    "onbeforematch",
    "onbeforetoggle",
    "onblur",
    "oncancel",
    "oncanplay",
    "oncanplaythrough",
    "onchange",
    "onclick",
    "onclose",
    "oncontextlost",
    "oncontextmenu",
    "oncontextrestored",
    "oncopy",
    "oncuechange",
    "oncut",
    "ondblclick",
    "ondrag",
    "ondragend",
    "ondragenter",
    "ondragleave",
    "ondragover",
    "ondragstart",
    "ondrop",
    "ondurationchange",
    "onemptied",
    "onended",
    "onerror",
    "onfocus",
    "onformdata",
    "oninput",
    "oninvalid",
    "onkeydown",
    "onkeypress",
    "onkeyup",
    "onload",
    "onloadeddata",
    "onloadedmetadata",
    "onloadstart",
    "onmousedown",
    "onmouseenter",
    "onmouseleave",
    "onmousemove",
    "onmouseout",
    "onmouseover",
    "onmouseup",
    "onpause",
    "onplay",
    "onplaying",
    "onprogress",
    "onratechange",
    "onreset",
    "onresize",
    "onscroll",
    "onscrollend",
    "onsecuritypolicyviolation",
    "onseeked",
    "onseeking",
    "onselect",
    "onslotchange",
    "onstalled",
    "onsubmit",
    "onsuspend",
    "ontimeupdate",
    "ontoggle",
    "onvolumechange",
    "onwaiting",
    "onwheel",
    // Pointer events (§Element).
    "ongotpointercapture",
    "onlostpointercapture",
    "onpointercancel",
    "onpointerdown",
    "onpointerenter",
    "onpointerleave",
    "onpointermove",
    "onpointerout",
    "onpointerover",
    "onpointerup",
    "onpointerrawupdate",
    // Touch events.
    "ontouchstart",
    "ontouchmove",
    "ontouchend",
    "ontouchcancel",
    // Animation/transition events.
    "onanimationstart",
    "onanimationiteration",
    "onanimationend",
    "onanimationcancel",
    "ontransitionrun",
    "ontransitionstart",
    "ontransitionend",
    "ontransitioncancel",
    // WindowEventHandlers — posables aussi sur document/body.
    "onafterprint",
    "onbeforeprint",
    "onbeforeunload",
    "ongamepadconnected",
    "ongamepaddisconnected",
    "onhashchange",
    "onlanguagechange",
    "onmessage",
    "onmessageerror",
    "onoffline",
    "ononline",
    "onpagehide",
    "onpageshow",
    "onpopstate",
    "onrejectionhandled",
    "onstorage",
    "onunhandledrejection",
    "onunload",
];

/// `on` + nom connu ?
fn is_event_handler_prop(key: &str) -> bool {
    EVENT_HANDLER_PROPS.contains(&key)
}

/// Méthodes de `document` — historiquement *invoke-only* :
/// invisibles en lecture de propriété (`typeof d.createElement`
/// → `undefined`), ce qui faisait échouer les sniffs de support
/// des frameworks — mesuré : `canUseDOM` de React faux à
/// l'éval → `isInputEventSupported` faux → `onChange` des
/// champs contrôlés envoyé sur le polyfill IE8
/// (`propertychange`), jamais tiré. Chaque nom rend une
/// fonction bornée qui redispatche [`Host::invoke`].
const DOC_METHODS: &[&str] = &[
    "addEventListener",
    "removeEventListener",
    "dispatchEvent",
    "adoptNode",
    "importNode",
    "exitFullscreen",
    "createRange",
    "createTreeWalker",
    "createNodeIterator",
    "execCommand",
    "getAnimations",
    "startViewTransition",
    "evaluate",
    "createExpression",
    "createNSResolver",
    "hasStorageAccess",
    "requestStorageAccess",
    "elementFromPoint",
    "elementsFromPoint",
    "caretPositionFromPoint",
    "getSelection",
    "hasFocus",
    "contains",
    "getElementById",
    "getElementsByTagName",
    "getElementsByClassName",
    "getElementsByName",
    "querySelector",
    "querySelectorAll",
    "createElement",
    "createElementNS",
    "createAttribute",
    "createAttributeNS",
    "createTextNode",
    "createComment",
    "createDocumentFragment",
    "createEvent",
    "open",
    "close",
    "write",
    "writeln",
    // Node/ParentNode sur le Document (spec — JS-C208 :
    // `document.appendChild`/`replaceChild` existent).
    "appendChild",
    "append",
    "prepend",
    "insertBefore",
    "removeChild",
    "replaceChild",
    "replaceChildren",
    "hasChildNodes",
];

/// Méthodes d'élément universelles (Node + Element — hors
/// variantes à gate de tag : `submit`/`getContext`/`play`… qui
/// restent invoke-only, fidèles aux éléments qui ne les ont
/// pas).
const NODE_METHODS: &[&str] = &[
    "addEventListener",
    "removeEventListener",
    "dispatchEvent",
    "appendChild",
    "insertBefore",
    "removeChild",
    "replaceChild",
    "cloneNode",
    "contains",
    "getRootNode",
    "hasChildNodes",
    "isSameNode",
    "isEqualNode",
    "compareDocumentPosition",
    "normalize",
    "append",
    "prepend",
    "before",
    "after",
    "remove",
    "replaceWith",
    "getElementsByTagName",
    "getElementsByClassName",
    "querySelector",
    "querySelectorAll",
    "setAttribute",
    "getAttribute",
    "hasAttribute",
    "hasAttributes",
    "getAttributeNames",
    "removeAttribute",
    "setAttributeNS",
    "getAttributeNS",
    "hasAttributeNS",
    "removeAttributeNS",
    "getAttributeNode",
    "getAttributeNodeNS",
    "setAttributeNode",
    "setAttributeNodeNS",
    "removeAttributeNode",
    "toggleAttribute",
    "insertAdjacentElement",
    "insertAdjacentHTML",
    "insertAdjacentText",
    "getHTML",
    "setHTMLUnsafe",
    "checkVisibility",
    "replaceChildren",
    "closest",
    "matches",
    "getBoundingClientRect",
    "getClientRects",
    "animate",
    "getAnimations",
    "scrollIntoView",
    "scrollIntoViewIfNeeded",
    "scroll",
    "scrollTo",
    "scrollBy",
    "attachShadow",
    "attachInternals",
    "focus",
    "blur",
    "click",
];

/// Trampoline des méthodes lisibles : `bound ++ args` =
/// `(nom, receveur, args…)` → [`Host::invoke`]. Le receveur est
/// lié à la création — `f.call(autreEl)` ne redispatche pas
/// sur `autreEl` (déviation documentée ; les sites n'y
/// recourent pas).
fn host_method_trampoline(e: &mut Engine, _this: Value, args: &[Value]) -> JsResult<Value> {
    let Some((Value::Str(m), rest)) = args.split_first() else {
        return Ok(Value::Undefined);
    };
    let name = e.heap.string(*m);
    let Some(name) = name else {
        return Ok(Value::Undefined);
    };
    let Some(Value::Obj(id)) = rest.first().copied() else {
        return Ok(Value::Undefined);
    };
    let Some(h) = e.host_of(id) else {
        return Ok(Value::Undefined);
    };
    Ok(h.invoke(e, Value::Obj(id), &name, &rest[1..])?
        .unwrap_or(Value::Undefined))
}

/// Lecture d'un handler `on*` : propre scriptée (`el.onclick =
/// f`, stockée en expando) prioritaire, puis attribut `on*`
/// compilé paresseusement (JS-C45) quand `node` est donné,
/// sinon `null`. Déviation : corps illisible → `null` (le
/// handler spec est « not set »).
fn on_handler_get(
    e: &mut Engine,
    dom: &DomRef,
    node: Option<NodeId>,
    this: Value,
    key: &str,
) -> JsResult<Option<Value>> {
    if !is_event_handler_prop(key) {
        return Ok(None);
    }
    if let Value::Obj(id) = this {
        if let Some(v) = e.find_prop(id, key).and_then(|p| p.data_value()) {
            return Ok(Some(v));
        }
    }
    if let Some(n) = node {
        if let Ok(Some(f)) = e.inline_handler(dom, n, key) {
            return Ok(Some(f));
        }
    }
    Ok(Some(Value::Null))
}

/// Attributs énumérés « limited to only known values » (HTML
/// §2.5.5) : la lecture IDL rend le *mot-clé canonique* —
/// attribut absent → missing-value default, valeur hors
/// vocabulaire → invalid-value default (normalisés en
/// minuscules). Sans cette table `input.type` rend `""` et les
/// sniffers de frameworks tombent — mesuré : React
/// `isTextInputElement` exige `type` dans sa liste, faute de
/// quoi le `ChangeEventPlugin` jette l'événement et `onChange`
/// ne se déclenche jamais sur les champs contrôlés.
/// `key` = nom de propriété JS ; `None` hors table.
fn enum_attr_default(d: &Dom, node: NodeId, key: &str) -> Option<String> {
    let tag = d.nodes[node as usize].tag.as_str();
    let lower = |a: &str| d.attr(node, a).map(str::to_ascii_lowercase);
    let known = |raw: Option<String>, list: &[&str], def: &str| -> String {
        match raw {
            Some(v) if list.contains(&v.as_str()) => v,
            _ => def.to_string(),
        }
    };
    Some(match (tag, key) {
        ("input", "type") => known(
            lower("type"),
            &[
                "hidden",
                "text",
                "search",
                "tel",
                "url",
                "email",
                "password",
                "date",
                "month",
                "week",
                "time",
                "datetime-local",
                "number",
                "range",
                "color",
                "checkbox",
                "radio",
                "file",
                "submit",
                "image",
                "reset",
                "button",
            ],
            "text",
        ),
        ("button", "type") => known(lower("type"), &["submit", "reset", "button"], "submit"),
        // `select.type` n'est pas un reflet : dérivé de
        // `multiple` (HTML §4.10.7).
        ("select", "type") => {
            if d.has_attr(node, "multiple") {
                "select-multiple".to_string()
            } else {
                "select-one".to_string()
            }
        }
        ("form", "method") => known(lower("method"), &["get", "post", "dialog"], "get"),
        ("form", "enctype" | "encoding") => known(
            lower("enctype"),
            &[
                "application/x-www-form-urlencoded",
                "multipart/form-data",
                "text/plain",
            ],
            "application/x-www-form-urlencoded",
        ),
        ("textarea", "wrap") => known(lower("wrap"), &["soft", "hard"], "soft"),
        // `track.kind` : absent → `subtitles`, invalide →
        // `metadata` (deux défauts distincts, HTML §4.8.11.4).
        ("track", "kind") => match lower("kind") {
            Some(v)
                if [
                    "subtitles",
                    "captions",
                    "descriptions",
                    "chapters",
                    "metadata",
                ]
                .contains(&v.as_str()) =>
            {
                v
            }
            Some(_) => "metadata".to_string(),
            None => "subtitles".to_string(),
        },
        ("img", "decoding") => known(lower("decoding"), &["auto", "sync", "async"], "auto"),
        ("img" | "iframe", "loading") => known(lower("loading"), &["eager", "lazy"], "eager"),
        ("th", "scope") => known(
            lower("scope"),
            &["row", "col", "rowgroup", "colgroup"],
            "auto",
        ),
        _ => return None,
    })
}

/// Nom d'attribut reflété en **écriture** pour les paires
/// (tag, prop) énumérées — hors `MAPPED_ATTRS` volontairement
/// : y figurer ferait du reflet un global qui masque le named
/// getter des `<form>` (`form.kind` ≙ contrôle nommé « kind »,
/// régression `c76_surfaces_detail`). `select.type` n'est pas
/// un attribut → `None`.
fn enum_attr_write_name(tag: &str, key: &str) -> Option<&'static str> {
    Some(match (tag, key) {
        ("input" | "button", "type") => "type",
        ("form", "method") => "method",
        ("form", "enctype" | "encoding") => "enctype",
        ("textarea", "wrap") => "wrap",
        ("track", "kind") => "kind",
        ("img", "decoding") => "decoding",
        ("img" | "iframe", "loading") => "loading",
        ("th", "scope") => "scope",
        _ => return None,
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

/// `true` si `node` est `<dialog>` — gate des méthodes/props
/// `HTMLDialogElement` (CAP-176).
fn is_dialog(dom: &DomRef, node: NodeId) -> bool {
    dom.borrow().nodes[node as usize].tag.as_str() == "dialog"
}

/// `true` si `node` est une forme `SVGGeometryElement`
/// (§B-14 vol 331 — `getTotalLength`/`getPointAtLength`).
fn is_svg_geom(dom: &DomRef, node: NodeId) -> bool {
    matches!(
        dom.borrow().nodes[node as usize].tag.as_str(),
        "path" | "line" | "polyline" | "polygon" | "rect" | "circle" | "ellipse"
    )
}

/// DOMException-like `{name, message}` — les erreurs DOM
/// standard hors idb (`InvalidStateError` de `showModal`…).
pub(crate) fn dom_ex_pub(e: &mut Engine, name: &str, msg: &str) -> Value {
    let o = e.new_object();
    let n = e.str_val(name);
    e.set_own(o, "name", n, true);
    let m = e.str_val(msg);
    e.set_own(o, "message", m, true);
    Value::Obj(o)
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
        let d = self.dom.borrow();
        // Id obsolète (arène resynchronisée sous handle JS vivant)
        // → repli générique, jamais panic.
        if !d.node_valid(self.node) {
            return "HTMLElement";
        }
        match d.nodes[self.node as usize].tag.as_str() {
            "video" => "HTMLVideoElement",
            "audio" => "HTMLAudioElement",
            "canvas" => "HTMLCanvasElement",
            // JS-C82 — noms d'interface exacts pour les tags à
            // accesseurs IDL (`constructor.name` fidèle).
            "input" => "HTMLInputElement",
            "textarea" => "HTMLTextAreaElement",
            "select" => "HTMLSelectElement",
            "option" => "HTMLOptionElement",
            "button" => "HTMLButtonElement",
            "form" => "HTMLFormElement",
            "img" => "HTMLImageElement",
            "a" => "HTMLAnchorElement",
            "iframe" => "HTMLIFrameElement",
            "script" => "HTMLScriptElement",
            "link" => "HTMLLinkElement",
            _ => "HTMLElement",
        }
    }

    fn has_method(&self, name: &str) -> bool {
        if !self.dom.borrow().node_valid(self.node) {
            return NODE_METHODS.contains(&name);
        }
        NODE_METHODS.contains(&name)
            || (is_dialog(&self.dom, self.node) && matches!(name, "showModal" | "show" | "close"))
            || (is_field(&self.dom, self.node)
                && matches!(name, "select" | "setSelectionRange" | "setRangeText"))
            || (self.dom.borrow().nodes[self.node as usize].tag.as_str() == "form"
                && name == "reset")
            || (self.dom.borrow().nodes[self.node as usize].tag.as_str() == "img"
                && name == "decode")
            // CAP-179 — fullscreen sur tout élément attaché
            // (spec) ; popover gaté par l'attribut `popover`
            // (`typeof el.showPopover` = 'function' seulement
            // si l'élément est un popover — feature-detect
            // honnête).
            || name == "requestFullscreen"
            || (self.dom.borrow().has_attr(self.node, "popover")
                && matches!(name, "showPopover" | "hidePopover" | "togglePopover"))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn trace(&self, out: &mut Vec<ObjId>) {
        trace_node_cache(&self.cache, out);
    }

    fn get(&self, e: &mut Engine, this: Value, key: &str) -> JsResult<Option<Value>> {
        let node = self.node;
        // Id obsolète (arène resynchronisée — spectrum
        // `groupChildren`, `nodes[614]` sur 507) → prop absente,
        // dégradation `undefined` plutôt que panic.
        if !self.dom.borrow().node_valid(node) {
            return Ok(None);
        }
        // Accesseur propre `defineProperty` (JS-C82) : il shadowe
        // la voie host — `read_prop` l'invoquera (spec : own
        // prop > IDL). Sans ça le tracker React posé par
        // `inputValueTracking` n'intercepterait rien.
        if let Value::Obj(id) = this {
            if matches!(e.find_own(id, key), Some(crate::heap::Prop::Acc { .. })) {
                return Ok(None);
            }
        }
        // `<a>`/`<area>` : décomposition d'URL sur l'attribut
        // `href` (JS-C87) — le patron « parse via ancre » de
        // toutes les libs (`c.href=u; c.pathname` — gtag en meurt
        // si `pathname` rend `undefined` au lieu de `""`).
        // Valeurs brutes du href (pas de résolution base — la
        // déviation est documentée ; absolu passe intact).
        if matches!(
            key,
            "protocol" | "host" | "hostname" | "port" | "pathname" | "search" | "hash" | "origin"
        ) {
            let d = self.dom.borrow();
            if matches!(d.nodes[node as usize].tag.as_str(), "a" | "area") {
                let raw = d.attr(node, "href").unwrap_or("").to_string();
                let (proto, host, hostname, port, path, search, hash) = if raw.is_empty() {
                    // `<a>` sans `href` : champs vides (spec —
                    // `pathname` ≙ `""`, jamais `undefined`).
                    (
                        String::new(),
                        String::new(),
                        String::new(),
                        String::new(),
                        String::new(),
                        String::new(),
                        String::new(),
                    )
                } else {
                    url_parts(&raw)
                };
                let s = match key {
                    "protocol" => proto,
                    "host" => host,
                    "hostname" => hostname,
                    "port" => port,
                    "pathname" => path,
                    "search" => search,
                    "hash" => hash,
                    // `origin` : scheme+host — "null" sans scheme
                    // (spec : tuple d'origine sérialisé).
                    _ => {
                        if proto.is_empty() {
                            "null".to_string()
                        } else {
                            format!("{proto}//{host}")
                        }
                    }
                };
                return Ok(Some(e.str_val(s)));
            }
        }
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
        // `<iframe>.contentWindow`/`contentDocument` (JS-C78) :
        // same-origin uniquement (arrêt v1) — cross-origin rend
        // `null` (comportement spec, pas un stub). `contentWindow`
        // rend le `window` de l'enfant (global same-origin ou
        // `WindowProxy` opaque) ; `contentDocument` rend le
        // `document` de son miroir, `null` si cross ou sans
        // contexte attaché.
        // `.document` legacy = alias de `contentDocument`
        // (dw : `frames[i].document`/`f.document` — les
        // scripts pré-standards posent `.document` sur
        // l'élément, pas `contentDocument`).
        if key == "document" && self.dom.borrow().nodes[node as usize].tag == "iframe" {
            let dom_ptr = std::rc::Rc::as_ptr(&self.dom) as usize;
            let Some(child) = e.ctx_of_frame(dom_ptr, node) else {
                return Ok(Some(Value::Null));
            };
            if !e.ctx_same_origin(e.cur_ctx, child) {
                return Ok(Some(Value::Null));
            }
            let doc = if child == e.cur_ctx {
                e.dom_ctx
                    .as_ref()
                    .and_then(|(d, cache)| cache.borrow().get(&d.borrow().root).copied())
            } else {
                e.parked.get(&child).and_then(|p| {
                    p.dom_ctx
                        .as_ref()
                        .and_then(|(d, cache)| cache.borrow().get(&d.borrow().root).copied())
                })
            };
            return Ok(Some(doc.map_or(Value::Null, Value::Obj)));
        }
        if matches!(key, "contentWindow" | "contentDocument")
            && self.dom.borrow().nodes[node as usize].tag == "iframe"
        {
            let viewer = e.cur_ctx;
            let dom_ptr = std::rc::Rc::as_ptr(&self.dom) as usize;
            let Some(child) = e.ctx_of_frame(dom_ptr, node) else {
                return Ok(Some(Value::Null));
            };
            if key == "contentWindow" {
                if e.ctx_same_origin(viewer, child) {
                    return Ok(Some(e.ctx_window_value_as(viewer, child)));
                }
                // Cross-origin : la fenêtre existe mais est opaque
                // — le `WindowProxy` seul passe, jamais le global.
                return Ok(Some(e.ctx_window_value_as(viewer, child)));
            }
            if !e.ctx_same_origin(viewer, child) {
                return Ok(Some(Value::Null));
            }
            let doc = if child == e.cur_ctx {
                e.dom_ctx
                    .as_ref()
                    .and_then(|(d, cache)| cache.borrow().get(&d.borrow().root).copied())
            } else {
                e.parked.get(&child).and_then(|p| {
                    p.dom_ctx
                        .as_ref()
                        .and_then(|(d, cache)| cache.borrow().get(&d.borrow().root).copied())
                })
            };
            return Ok(Some(doc.map_or(Value::Null, Value::Obj)));
        }
        // `constructor` (JS-C82) : objet fonction typé par
        // classe portant `.prototype` + accesseurs IDL —
        // `ctor.prototype` sert `getOwnPropertyDescriptor`
        // (tracker React `inputValueTracking`).
        // Élément **upgradé CE** → `Ctor.prototype.constructor`
        // = le ctor enregistré (spec : `el.constructor` suit le
        // proto installé par l'upgrade — sinon `this.constructor`
        // dans le corps LitElement rend l'interface `HTMLElement`
        // et `elementProperties` manque → `ce.failed` — verrou
        // Shoelace mesuré, JS-C102).
        if key == "constructor" {
            let upgraded = self
                .dom
                .borrow()
                .ce
                .upgraded
                .get(&node)
                .and_then(|n| self.dom.borrow().ce.defs.get(n).map(|d| d.ctor));
            if let Some(ctor) = upgraded {
                return Ok(Some(ctor));
            }
            // `div.constructor === HTMLDivElement` — spec :
            // le ctor tag-spécifique global (JS-C192, vol 121
            // sephora). Élément → interface par tag ; autres
            // kinds → `class_name()` (Text/Comment/Node).
            let tag = self.dom.borrow().nodes[node as usize].tag.clone();
            let name = tag_interface(&tag).unwrap_or(self.class_name());
            if let Some(v @ Value::Obj(_)) = e.get_prop(e.global, name) {
                return Ok(Some(v));
            }
            return Ok(Some(e.dom_ctor(self.class_name())));
        }
        // Fast paths that don't need the dom borrow held across calls.
        match key {
            "style" => {
                return Ok(Some(e.new_host_object(Rc::new(StyleHost {
                    dom: self.dom.clone(),
                    node,
                    computed: false,
                }))));
            }
            "classList" => {
                return Ok(Some(e.new_iterable_host(Rc::new(ClassListHost {
                    dom: self.dom.clone(),
                    node,
                    attr: "class",
                }))));
            }
            // `relList` (§B-14 vol 141/216 philharmonie —
            // addtoany `link.relList.supports('modulepreload')`)
            // — DOMTokenList reflétant `rel`, spec restreinte
            // à `link`/`a`/`area`/`form`.
            "relList" => {
                let n = &self.dom.borrow().nodes[node as usize];
                let tag = match n.kind {
                    NodeKind::Element => n.tag.as_str(),
                    _ => "",
                };
                return Ok(Some(match tag {
                    "link" | "a" | "area" | "form" => e.new_iterable_host(Rc::new(ClassListHost {
                        dom: self.dom.clone(),
                        node,
                        attr: "rel",
                    })),
                    _ => Value::Undefined,
                }));
            }
            "parentNode" => {
                let p = self.dom.borrow().nodes[node as usize].parent;
                return Ok(Some(match p {
                    Some(p) => node_value(e, &self.dom, &self.cache, p),
                    None => Value::Null,
                }));
            }
            // `isConnected` (CAP-164) — le nœud remonte à
            // `#document` ; les libs réelles le lisent avant
            // mesure/délégation (React, selector-observer).
            "isConnected" => {
                return Ok(Some(Value::Bool(self.dom.borrow().is_attached(node))));
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
                return Ok(Some(dom_collection(
                    e,
                    &self.dom,
                    &self.cache,
                    &kids,
                    false,
                    "HTMLCollection",
                )));
            }
            // `el.attributes` — `NamedNodeMap` vivante (JS-C76) :
            // itérable + `length` + indexés — les helpers
            // `makeIterator` des bundles closure l'exigent
            // (`Error: b` google.com sinon).
            "attributes" => {
                return Ok(Some(e.new_iterable_host(Rc::new(AttributesHost {
                    dom: self.dom.clone(),
                    cache: self.cache.clone(),
                    node,
                }))));
            }
            // `form.elements`/`fieldset.elements` —
            // `HTMLFormControlsCollection` : contrôles listés du
            // sous-arbre + accès nommé + `namedItem` (JS-C76).
            "elements" => {
                let is_listing = {
                    let d = self.dom.borrow();
                    matches!(d.nodes[node as usize].tag.as_str(), "form" | "fieldset")
                };
                if is_listing {
                    let ids = {
                        let d = self.dom.borrow();
                        listed_controls(&d, node)
                    };
                    return Ok(Some(dom_collection(
                        e,
                        &self.dom,
                        &self.cache,
                        &ids,
                        true,
                        "HTMLFormControlsCollection",
                    )));
                }
            }
            // `select.options`/`selectedOptions` —
            // `HTMLOptionsCollection` ≙ tableau des `<option>`
            // (JS-C76) ; `selectedOptions` = filtre `selected`.
            "options" | "selectedOptions" => {
                let is_select = {
                    let d = self.dom.borrow();
                    d.nodes[node as usize].tag == "select"
                };
                if is_select {
                    let opts = {
                        let d = self.dom.borrow();
                        let mut opts = d.by_tag(node, "option");
                        if key == "selectedOptions" {
                            opts.retain(|&o| d.has_attr(o, "selected"));
                        }
                        opts
                    };
                    return Ok(Some(dom_collection(
                        e,
                        &self.dom,
                        &self.cache,
                        &opts,
                        false,
                        "HTMLOptionsCollection",
                    )));
                }
            }
            "selectedIndex" => {
                let d = self.dom.borrow();
                if d.nodes[node as usize].tag == "select" {
                    let i = d
                        .by_tag(node, "option")
                        .iter()
                        .position(|&o| d.has_attr(o, "selected"));
                    return Ok(Some(Value::Num(i.map_or(-1.0, |i| i as f64))));
                }
            }
            // `input.files` — `FileList` honnête vide (`type=file`)
            // ou `null` (autres types) — l'upload réel est JS-C70.
            "files" => {
                let is_file = {
                    let d = self.dom.borrow();
                    d.nodes[node as usize].tag == "input" && d.attr(node, "type") == Some("file")
                };
                return Ok(Some(if is_file {
                    let a = e.new_array(Vec::new());
                    if let Value::Obj(id) = a {
                        e.add_method(id, "item", collection_item);
                    }
                    a
                } else {
                    Value::Null
                }));
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
                return Ok(Some(dom_collection(
                    e,
                    &self.dom,
                    &self.cache,
                    &kids,
                    false,
                    "NodeList",
                )));
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
                let v = if matches!(n.kind, NodeKind::Text | NodeKind::Comment) {
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
            // `shadowRoot.host` (CAP-191 tranche 1) — l'hôte
            // de la racine ; absent (`undefined` fallthrough)
            // sur tout autre nœud — la propriété n'existe
            // que sur `ShadowRoot` (spec).
            "host" if self.dom.borrow().is_shadow_root(node) => {
                let h = self.dom.borrow().shadow_host(node);
                return Ok(Some(match h {
                    Some(h) => node_value(e, &self.dom, &self.cache, h),
                    None => Value::Null,
                }));
            }
            // `el.assignedSlot` (T3) — le `<slot>` de la
            // racine shadow du parent dont `name` égale
            // `el.slot` ; `null` hors distribution.
            "assignedSlot" => {
                let s = self.dom.borrow().assigned_slot(node);
                return Ok(Some(match s {
                    Some(s) => node_value(e, &self.dom, &self.cache, s),
                    None => Value::Null,
                }));
            }
            // `el.shadowRoot` (stub issue #25) — la racine
            // attachée par `attachShadow`, `null` en mode
            // `closed` ou sans racine.
            "shadowRoot" => {
                let r = self.dom.borrow().shadow_root(node, false);
                return Ok(Some(match r {
                    Some(r) => node_value(e, &self.dom, &self.cache, r),
                    None => Value::Null,
                }));
            }
            "length"
                if matches!(
                    self.dom.borrow().nodes[node as usize].kind,
                    NodeKind::Text | NodeKind::Comment
                ) =>
            {
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
                let (k, t) = {
                    let n = &self.dom.borrow().nodes[node as usize];
                    (n.kind, n.tag.clone())
                };
                // Majuscule = éléments HTML (spec) ; les sentinels
                // non-élément gardent leur forme (`#text`,
                // `#comment`, `#document-fragment`, `#document`).
                return Ok(Some(e.str_val(if k == NodeKind::Element {
                    t.to_uppercase()
                } else {
                    t
                })));
            }
            // `localName` — nom qualifié minuscule (spec DOM :
            // n'existe que sur les éléments, `null` ailleurs).
            // Son absence cassait les marcheurs d'ancêtres qui se
            // gardent sur `"body"===r.localName` : le check ratait,
            // la remontée débordait jusqu'au `document` puis
            // `document.hasAttribute` tuait le script (SG vol 128,
            // `localName`/`textContent` nicematin vol 124).
            "localName" => {
                let n = &self.dom.borrow().nodes[node as usize];
                let v = if n.kind == NodeKind::Element {
                    e.str_val(n.tag.clone())
                } else {
                    Value::Null
                };
                return Ok(Some(v));
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
                    NodeKind::Comment => 8.0,
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
            // Dimensions — issue #19 : la table de géométrie
            // publiée par la session sert les éléments non
            // racine (`None` → repli 0, comportement
            // historique). Racine (`documentElement`/`body`) :
            // `client*`/`offset*` = viewport (les pages lisent
            // `documentElement.clientHeight` pour le
            // « scroll bottom » — mesuré sur google.com) ;
            // `scroll*` = dimensions du document publiées
            // (repli viewport sans table — harnais).
            // `offsetParent` (JS-C97) — le plus proche ancêtre
            // positionné (`ElementGeom.positioned` publié par la
            // session) ou `td`/`th`/`table`/`body`, `null` pour
            // `position:fixed`, racine et élément sans boîte.
            "offsetParent" => {
                let (fixed, parent) = {
                    let fixed = element_geom(&self.dom, node).is_some_and(|(_, f, ..)| f);
                    (fixed, offset_parent_node(&self.dom, node))
                };
                return Ok(Some(if fixed {
                    Value::Null
                } else {
                    match parent {
                        Some(p) => node_value(e, &self.dom, &self.cache, p),
                        None => Value::Null,
                    }
                }));
            }
            // `offsetTop`/`offsetLeft` — distance border-edge de
            // l'élément au padding-edge de l'`offsetParent`
            // (union des boîtes ≙ border box, borne `crate::geom`)
            // : `rect − parent.rect` en coords **document**
            // (insensible au scroll) ; `position:fixed` → rect.y
            // (déjà viewport, offsetParent `null` spec) ;
            // pas de géométrie → 0 (repli historique).
            "offsetTop" | "offsetLeft" => {
                let vert = key == "offsetTop";
                let px = match element_geom(&self.dom, node) {
                    Some((r, fixed, ..)) => {
                        if fixed {
                            f64::from(if vert { r.y } else { r.x })
                        } else {
                            let base = offset_parent_node(&self.dom, node)
                                .and_then(|p| element_geom(&self.dom, p))
                                .map_or(0.0, |(pr, ..)| f64::from(if vert { pr.y } else { pr.x }));
                            f64::from(if vert { r.y } else { r.x }) - base
                        }
                    }
                    None => 0.0,
                };
                return Ok(Some(Value::Num(px)));
            }
            "clientWidth" | "clientHeight" | "scrollWidth" | "scrollHeight" | "offsetWidth"
            | "offsetHeight" => {
                let (vw, vh) = e.hooks.viewport;
                let is_root = {
                    let d = self.dom.borrow();
                    node == d.body() || Some(node) == d.document_element()
                };
                let vert = key.ends_with("Height");
                let px = if is_root {
                    if key.starts_with("scroll") {
                        geom_doc_dim(&self.dom, !vert)
                            .unwrap_or_else(|| f64::from(if vert { vh } else { vw }))
                    } else {
                        f64::from(if vert { vh } else { vw })
                    }
                } else {
                    element_geom_size(&self.dom, node)
                        .map_or(0.0, |(w, h)| if vert { h } else { w })
                };
                return Ok(Some(Value::Num(px)));
            }
            // `scrollTop`/`scrollLeft` — scroll réel du
            // document sur les nœuds racines (`body`/
            // `documentElement` — les loaders lazy lisent
            // `document.documentElement.scrollTop` ; vol
            // github : `undefined` figeait le trigger). Les
            // autres éléments : 0 honnête (pas de scroll
            // d'élément dans le moteur).
            "scrollTop" | "scrollLeft" => {
                let is_root = {
                    let d = self.dom.borrow();
                    node == d.body() || Some(node) == d.document_element()
                };
                let px = if is_root {
                    geom_scroll_offset(&self.dom, key == "scrollLeft")
                } else {
                    0.0
                };
                return Ok(Some(Value::Num(px)));
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
            // `scrollIntoView`/`scrollIntoViewIfNeeded`
            // (JS-C74) — pas de scroll réel dans le miroir :
            // méthode no-op honnête (les scripts l'appellent
            // librement ; jamais d'exception).
            "scrollIntoView" | "scrollIntoViewIfNeeded" => {
                return Ok(Some(
                    e.native_fn("scrollIntoView", element_scroll_into_view),
                ));
            }
            // `el.scroll*()` — no-op honnêtes (EXPLOR68 :
            // `scrollTo` absent tuait le lazy-load iltalehti).
            "scroll" | "scrollTo" | "scrollBy" => {
                return Ok(Some(e.native_fn("scroll", element_scroll_noop)));
            }
            // `<dialog>`/`<details>` (CAP-176) — `open`
            // reflété bool ; `returnValue` = slot spec sur
            // `<dialog>` (pas un attribut — il n'en est
            // pas un).
            "open"
                if matches!(
                    self.dom.borrow().nodes[node as usize].tag.as_str(),
                    "dialog" | "details"
                ) =>
            {
                return Ok(Some(Value::Bool(self.dom.borrow().has_attr(node, "open"))));
            }
            "returnValue" if is_dialog(&self.dom, node) => {
                let v = e
                    .hooks
                    .dialog_retval
                    .get(&node)
                    .cloned()
                    .unwrap_or_default();
                return Ok(Some(e.str_val(v)));
            }
            // Sélection de champ `input`/`textarea` (CAP-177) —
            // slots `data-explorer-sel-*` (motif
            // `CUSTOM_VALIDITY_ATTR`) : `selectionStart`/`End`
            // = indices (défaut 0/longueur), `selectionDirection`
            // = `forward|backward|none` (défaut `none`).
            // CAP-177 — primitives mesurées absentes par la
            // sonde `_census_calls` (TypeError terrain) :
            // `insertAdjacentElement` = même rail que Text
            // mais la charge est un nœud ; `replaceChildren`
            // vide puis insère (React/lit-html l'utilisent).
            "selectionStart" | "selectionEnd" if is_field(&self.dom, node) => {
                let (attr, def) = if key == "selectionStart" {
                    (SEL_START_ATTR, 0.0)
                } else {
                    let d = self.dom.borrow();
                    (SEL_END_ATTR, control_value(&d, node).chars().count() as f64)
                };
                let v = self
                    .dom
                    .borrow()
                    .attr(node, attr)
                    .and_then(|s| s.parse::<f64>().ok())
                    .unwrap_or(def);
                return Ok(Some(Value::Num(v)));
            }
            "selectionDirection" if is_field(&self.dom, node) => {
                let v = self
                    .dom
                    .borrow()
                    .attr(node, SEL_DIR_ATTR)
                    .unwrap_or("none")
                    .to_string();
                return Ok(Some(e.str_val(v)));
            }
            // Référence lisible (même motif que `matches`) —
            // les scripts mesurent aussi via
            // `Element.prototype.getBoundingClientRect.call(el)`
            // (issue #19 : invoke seul la rendrait invisible).
            "getBoundingClientRect" => {
                return Ok(Some(e.native_fn("getBoundingClientRect", element_gbrect)));
            }
            _ => {}
        }
        // Méthodes lisibles (JS-C80) — mêmes raisons que
        // `document` : `el.appendChild` & co. étaient
        // invoke-only ; les frameworks sniffent
        // `typeof el.m`/`"m" in el`.
        if NODE_METHODS.contains(&key) {
            let nv = e.str_val(key);
            return Ok(Some(e.bound_native(
                "node.method",
                host_method_trampoline,
                vec![nv, this],
            )));
        }
        // Handlers `on*` (JS-C80) — lecture IDL : propre
        // scriptée (`el.onclick = f`), attribut compilé
        // paresseusement (JS-C45), sinon `null`. `"on*"` reste
        // une vraie prop — sniff React `Lj`.
        if let Some(v) = on_handler_get(e, &self.dom, Some(node), this, key)? {
            return Ok(Some(v));
        }
        // Attributs énumérés « limited to only known values »
        // (JS-C80) — avant le reflet brut : `input.type` rend
        // `"text"`, `button.type` `"submit"`, etc.
        if let Some(v) = enum_attr_default(&self.dom.borrow(), node, key) {
            return Ok(Some(e.str_val(v)));
        }
        // Attribute-mapped props.
        if let Some(attr) = prop_to_attr(key) {
            // IDL URL-typé : `src`/`href` se résolvent contre
            // `document.baseURI` à la lecture (spec) —
            // `getAttribute` reste brut. `new URL(el.src)`
            // échouait `Invalid URL` sur les chemins relatifs
            // (lemonde `inline:5`, franceinfo `Invalid scheme`).
            let raw = self.dom.borrow().attr(node, attr).map(str::to_string);
            let v = match (&raw, attr) {
                (Some(r), "src" | "href") => {
                    crate::xhr::resolve_url(e, r).unwrap_or_else(|| r.clone())
                }
                (Some(r), _) => r.clone(),
                (None, _) => String::new(),
            };
            return Ok(Some(e.str_val(v)));
        }
        if BOOL_ATTRS.contains(&key) {
            let has = self.dom.borrow().has_attr(node, key);
            return Ok(Some(Value::Bool(has)));
        }
        // Named getter du `<form>` (HTML §4.10.3) : `form.username`
        // ≙ contrôle listé de `id`/`name` — multi-match (radio)
        // ≙ collection (RadioNodeList spec → tableau snapshot,
        // déviation documentée). Dernier recours : les props
        // réelles et les attributs priment.
        if self.dom.borrow().nodes[node as usize].tag == "form" {
            let ids = {
                let d = self.dom.borrow();
                listed_controls(&d, node)
                    .into_iter()
                    .filter(|&c| d.attr(c, "id") == Some(key) || d.attr(c, "name") == Some(key))
                    .collect::<Vec<_>>()
            };
            if !ids.is_empty() {
                return Ok(Some(if ids.len() == 1 {
                    node_value(e, &self.dom, &self.cache, ids[0])
                } else {
                    dom_collection(e, &self.dom, &self.cache, &ids, false, "RadioNodeList")
                }));
            }
        }
        let _ = this;
        Ok(None)
    }

    fn set(&self, e: &mut Engine, this: Value, key: &str, v: Value) -> JsResult<bool> {
        let node = self.node;
        // Id obsolète → écriture refusée (set expando ailleurs),
        // jamais panic sur l'arène.
        if !self.dom.borrow().node_valid(node) {
            return Ok(false);
        }
        // Accesseur propre `defineProperty` (JS-C82) : refuser
        // la voie host → `write_prop` invoque le setter.
        if let Value::Obj(id) = this {
            if matches!(e.find_own(id, key), Some(crate::heap::Prop::Acc { .. })) {
                return Ok(false);
            }
        }
        match key {
            "innerHTML" => {
                let html = e.to_js_string(v)?;
                self.set_inner_html(node, &html);
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
                let is_text = matches!(
                    self.dom.borrow().nodes[node as usize].kind,
                    NodeKind::Text | NodeKind::Comment
                );
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
            // `HTMLMediaElement.src = blob:` (étape attache
            // MSE, ADR-0029) — object URL `MediaSource` →
            // `readyState "open"` + `sourceopen`. Attribut
            // reflété normalement d'abord.
            "src" if is_media(&self.dom, node) => {
                let s = e.to_js_string(v)?;
                self.dom.borrow_mut().set_attr(node, "src", &s);
                crate::mse::maybe_attach(e, &s);
                Ok(true)
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
            // `<dialog>`/`<details>` (CAP-176) — `open` reflété
            // bool ; `returnValue` = slot spec sur `<dialog>`.
            "open"
                if matches!(
                    self.dom.borrow().nodes[node as usize].tag.as_str(),
                    "dialog" | "details"
                ) =>
            {
                let on = e.to_boolean(v);
                let mut d = self.dom.borrow_mut();
                if on {
                    d.set_attr(node, "open", "");
                } else {
                    d.remove_attr(node, "open");
                }
                Ok(true)
            }
            "returnValue" if is_dialog(&self.dom, node) => {
                let s = e.to_js_string(v)?;
                e.hooks.dialog_retval.insert(node, s);
                Ok(true)
            }
            // Sélection de champ (CAP-177) — écriture des
            // slots `data-explorer-sel-*`.
            "selectionStart" | "selectionEnd" | "selectionDirection"
                if is_field(&self.dom, node) =>
            {
                let mut d = self.dom.borrow_mut();
                match key {
                    "selectionStart" | "selectionEnd" => {
                        let n = e.to_number(v)?.max(0.0);
                        let attr = if key == "selectionStart" {
                            SEL_START_ATTR
                        } else {
                            SEL_END_ATTR
                        };
                        d.set_attr(node, attr, &format!("{}", n as u64));
                    }
                    _ => {
                        let s = e.to_js_string(v)?;
                        d.set_attr(node, SEL_DIR_ATTR, &s);
                    }
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
                // Attributs énumérés tag-scopés (JS-C80) —
                // écriture = reflet brut ; le clamp spec est
                // côté lecture (`enum_attr_default`).
                {
                    let tag = self.dom.borrow().nodes[node as usize].tag.clone();
                    if let Some(attr) = enum_attr_write_name(&tag, key) {
                        let s = e.to_js_string(v)?;
                        self.dom.borrow_mut().set_attr(node, attr, &s);
                        return Ok(true);
                    }
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
        this: Value,
        name: &str,
        args: &[Value],
    ) -> JsResult<Option<Value>> {
        let node = self.node;
        match name {
            "getElementsByTagName" => {
                let tag = e.to_js_string(arg(args, 0))?;
                let ids = self.dom.borrow().by_tag(node, &tag);
                Ok(Some(dom_collection(
                    e,
                    &self.dom,
                    &self.cache,
                    &ids,
                    false,
                    "HTMLCollection",
                )))
            }
            "getElementsByClassName" => {
                let cls = e.to_js_string(arg(args, 0))?;
                let ids = self.dom.borrow().by_class(node, &cls);
                Ok(Some(dom_collection(
                    e,
                    &self.dom,
                    &self.cache,
                    &ids,
                    false,
                    "HTMLCollection",
                )))
            }
            // `DocumentFragment.getElementById` (spec — exposé
            // sur ShadowRoot ; les hydratations CE le lisent,
            // reddit `faceplate-*`) — scopé au sous-arbre via
            // le chemin sélecteur `[id]`, jamais le document.
            // Éléments ordinaires : non exposé (spec).
            "getElementById" => {
                let is_frag = matches!(
                    self.dom.borrow().nodes[node as usize].kind,
                    NodeKind::Fragment
                );
                if !is_frag {
                    return Ok(None);
                }
                let id = e.to_js_string(arg(args, 0))?;
                let sel = format!("[id=\"{}\"]", id.replace('"', "\\\""));
                let ids = self.dom.borrow().query(node, &sel);
                Ok(Some(match ids.first() {
                    Some(&n) => node_value(e, &self.dom, &self.cache, n),
                    None => Value::Null,
                }))
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
                Ok(Some(dom_collection(
                    e,
                    &self.dom,
                    &self.cache,
                    &ids,
                    false,
                    "NodeList",
                )))
            }
            "appendChild" | "append" => {
                let target = node_for_in(e, arg(args, 0), &self.dom)
                    .or_else(|| adopt_into(e, arg(args, 0), &self.dom, &self.cache));
                match target {
                    Some(child) => {
                        // Spec `HierarchyRequestError` : append un
                        // ancêtre/soi créerait un cycle → toutes les
                        // walks DFS postérieures boucleraient.
                        if self.dom.borrow().is_ancestor_or_self(node, child) {
                            return Err(e.type_error("HierarchyRequestError: ancestor"));
                        }
                        self.dom.borrow_mut().append(node, child);
                        Ok(Some(arg(args, 0)))
                    }
                    None => Err(e.type_error("appendChild: not a node")),
                }
            }
            "removeChild" => {
                let target = node_for_in(e, arg(args, 0), &self.dom);
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
                // Même attache que la prop `src` — les players
                // empruntent les deux voies (étape MSE).
                let attach = k == "src" && is_media(&self.dom, node);
                self.dom.borrow_mut().set_attr(node, &k, &v);
                if attach {
                    crate::mse::maybe_attach(e, &v);
                }
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
            // `el.hasAttributes()` — spec Element : attrs
            // non vides. Le walker de templates Lit l'appelle
            // pour distinguer élément/marqueur (vaadin terrain).
            "hasAttributes" => {
                let n = &self.dom.borrow().nodes[node as usize];
                let has = n.kind == NodeKind::Element && !n.attrs.is_empty();
                Ok(Some(Value::Bool(has)))
            }
            // `el.getAttributeNames()` — tableau des noms
            // d'attributs (spec Element). Lit l'emploie pour
            // énumérer les marqueurs de parties par attribut
            // (`?attr$` — vaadin terrain).
            "getAttributeNames" => {
                let names: Vec<Value> = {
                    let d = self.dom.borrow();
                    let n = &d.nodes[node as usize];
                    if n.kind == NodeKind::Element {
                        n.attrs.iter().map(|(k, _)| e.str_val(k)).collect()
                    } else {
                        Vec::new()
                    }
                };
                Ok(Some(e.new_array(names)))
            }
            "removeAttribute" => {
                let k = e.to_js_string(arg(args, 0))?;
                self.dom.borrow_mut().remove_attr(node, &k);
                Ok(Some(Value::Undefined))
            }
            // `*AttributeNS` (spec Element) — l'arène ne trace pas
            // les espaces de noms : un attr `p:l` a pour partie
            // locale `l`. `ns` nul/absent → seul un attr non
            // préfixé matche (no-namespace). Requis par les
            // helpers SVG (`xlink:href`) — §B-14 boursorama.
            "getAttributeNS" => {
                let ns_null = attr_ns_is_null(e, arg(args, 0))?;
                let local = e.to_js_string(arg(args, 1))?;
                let v = {
                    let d = self.dom.borrow();
                    d.nodes[node as usize]
                        .attrs
                        .iter()
                        .find(|(k, _)| attr_ns_match(k, ns_null, &local))
                        .map(|(_, v)| v.clone())
                };
                Ok(Some(match v {
                    Some(v) => e.str_val(v),
                    None => Value::Null,
                }))
            }
            "hasAttributeNS" => {
                let ns_null = attr_ns_is_null(e, arg(args, 0))?;
                let local = e.to_js_string(arg(args, 1))?;
                let has = self.dom.borrow().nodes[node as usize]
                    .attrs
                    .iter()
                    .any(|(k, _)| attr_ns_match(k, ns_null, &local));
                Ok(Some(Value::Bool(has)))
            }
            "removeAttributeNS" => {
                let ns_null = attr_ns_is_null(e, arg(args, 0))?;
                let local = e.to_js_string(arg(args, 1))?;
                let key = self.dom.borrow().nodes[node as usize]
                    .attrs
                    .iter()
                    .find(|(k, _)| attr_ns_match(k, ns_null, &local))
                    .map(|(k, _)| k.clone());
                if let Some(k) = key {
                    self.dom.borrow_mut().remove_attr(node, &k);
                }
                Ok(Some(Value::Undefined))
            }
            "setAttributeNS" => {
                // `qname` porte le préfixe (`xlink:href`) — posé tel
                // quel, le namespace n'est pas tracé.
                let q = e.to_js_string(arg(args, 1))?;
                let v = e.to_js_string(arg(args, 2))?;
                self.dom.borrow_mut().set_attr(node, &q, &v);
                Ok(Some(Value::Undefined))
            }
            // Rails `*AttributeNode` (spec Attr) — objets
            // porteurs de `createAttribute` : `getAttributeNode`
            // enveloppe l'attr existant, `setAttributeNode`
            // écrit `name`/`value` et rend l'attr remplacé (ou
            // `null`), `removeAttributeNode` rend l'attr retiré
            // (§B-14 npr vol EXPLOR74 — `createAttribute` seul
            // ne suffit pas, les polyfills chaînent).
            "getAttributeNode" => {
                let k = e.to_js_string(arg(args, 0))?;
                let v = self.dom.borrow().attr(node, &k).map(str::to_string);
                Ok(Some(match v {
                    Some(v) => attr_obj(e, &k, &v, None),
                    None => Value::Null,
                }))
            }
            "getAttributeNodeNS" => {
                let ns_null = attr_ns_is_null(e, arg(args, 0))?;
                let local = e.to_js_string(arg(args, 1))?;
                let found = {
                    let d = self.dom.borrow();
                    d.nodes[node as usize]
                        .attrs
                        .iter()
                        .find(|(k, _)| attr_ns_match(k, ns_null, &local))
                        .map(|(k, v)| (k.clone(), v.clone()))
                };
                Ok(Some(match found {
                    Some((k, v)) => attr_obj(e, &k, &v, None),
                    None => Value::Null,
                }))
            }
            "setAttributeNode" | "setAttributeNodeNS" => {
                let a = arg(args, 0);
                let name = match a {
                    Value::Obj(id) => e
                        .get_prop(id, "name")
                        .filter(|v| !matches!(v, Value::Undefined)),
                    _ => None,
                };
                let Some(name_v) = name else {
                    return Err(e.type_error("setAttributeNode: Attr requis"));
                };
                let name = e.to_js_string(name_v)?;
                let value_v = match a {
                    Value::Obj(id) => e.get_prop(id, "value").unwrap_or(Value::Undefined),
                    _ => Value::Undefined,
                };
                let value = e.to_js_string(value_v)?;
                let old = self.dom.borrow().attr(node, &name).map(str::to_string);
                self.dom.borrow_mut().set_attr(node, &name, &value);
                Ok(Some(match old {
                    Some(v) => attr_obj(e, &name, &v, None),
                    None => Value::Null,
                }))
            }
            "removeAttributeNode" => {
                let a = arg(args, 0);
                let name = match a {
                    Value::Obj(id) => e
                        .get_prop(id, "name")
                        .filter(|v| !matches!(v, Value::Undefined)),
                    _ => None,
                };
                let Some(name_v) = name else {
                    return Err(e.type_error("removeAttributeNode: Attr requis"));
                };
                let name = e.to_js_string(name_v)?;
                // Spec : `NotFoundError` si l'attr n'est pas
                // sur l'élément.
                if !self.dom.borrow().has_attr(node, &name) {
                    let o = e.new_object();
                    let n = e.str_val("NotFoundError");
                    e.set_own(o, "name", n, true);
                    return Err(crate::heap::Thrown(Value::Obj(o)));
                }
                self.dom.borrow_mut().remove_attr(node, &name);
                Ok(Some(a))
            }
            // Issue #16 — spec Element.toggleAttribute : `force`
            // absent (ou `undefined` explicite) → bascule ; présent
            // → impose ; retourne l'état final.
            "toggleAttribute" => {
                let k = e.to_js_string(arg(args, 0))?;
                let has = self.dom.borrow().has_attr(node, &k);
                let want = match args.get(1) {
                    None | Some(Value::Undefined) => !has,
                    Some(f) => e.to_boolean(*f),
                };
                if want != has {
                    let mut d = self.dom.borrow_mut();
                    if want {
                        d.set_attr(node, &k, "");
                    } else {
                        d.remove_attr(node, &k);
                    }
                }
                Ok(Some(Value::Bool(want)))
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
            // `el.click()` : dispatch complet, puis
            // **activation behavior** si non annulé
            // (CAP-153 — `<a href>` navigue, submit
            // soumet, checkbox/radio basculent, label
            // délègue, summary bascule details).
            "click" => {
                let (bubbles, cancelable, composed) = platform_event_flags(name);
                let state = make_event_state(
                    name,
                    node,
                    EventFlags {
                        bubbles,
                        cancelable,
                        composed,
                        trusted: false,
                    },
                    Vec::new(),
                );
                e.propagate_event(&self.dom, &self.cache, &state)?;
                if !state.prevented.get() {
                    self.run_activation(e, node)?;
                }
                Ok(Some(Value::Undefined))
            }
            // `el.focus(options?)`/`el.blur()` (CAP-162) —
            // état de focus réel du miroir : `focus()` sur un
            // élément focalisable déplace le focus (quad
            // `blur`/`focusout` → `focus`/`focusin`, spec
            // « focus update steps »), `blur()` le retire.
            // Non focalisable / déjà focalisé = no-op.
            // `document.activeElement` reflète l'état.
            "focus" => {
                self.focus_move(e, node)?;
                Ok(Some(Value::Undefined))
            }
            "blur" => {
                self.blur_el(e, node)?;
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
            // → soumission si non annulée. Voie partagée
            // `submit_via` (avec l'activation des boutons
            // submit, CAP-153).
            "requestSubmit" => {
                let submitter = node_for_in(e, arg(args, 0), &self.dom);
                self.submit_via(e, node, submitter)?;
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
                let new_n = node_for_in(e, arg(args, 0), &self.dom)
                    .or_else(|| adopt_into(e, arg(args, 0), &self.dom, &self.cache));
                let ref_n = node_for_in(e, arg(args, 1), &self.dom);
                match new_n {
                    Some(c) => {
                        if self.dom.borrow().is_ancestor_or_self(node, c) {
                            return Err(e.type_error("HierarchyRequestError: ancestor"));
                        }
                        self.dom.borrow_mut().insert_before(node, c, ref_n);
                        Ok(Some(arg(args, 0)))
                    }
                    None => Err(e.type_error("insertBefore: not a node")),
                }
            }
            "replaceChild" => {
                let new_n = node_for_in(e, arg(args, 0), &self.dom)
                    .or_else(|| adopt_into(e, arg(args, 0), &self.dom, &self.cache));
                let old_n = node_for_in(e, arg(args, 1), &self.dom);
                match (new_n, old_n) {
                    (Some(n), Some(o)) => {
                        if self.dom.borrow().is_ancestor_or_self(node, n) {
                            return Err(e.type_error("HierarchyRequestError: ancestor"));
                        }
                        self.dom.borrow_mut().replace_child(node, n, o);
                        Ok(Some(arg(args, 1)))
                    }
                    _ => Err(e.type_error("replaceChild: not a node")),
                }
            }
            "cloneNode" => {
                let deep = e.to_boolean(arg(args, 0));
                let c = self.dom.borrow_mut().clone_node(node, deep);
                // Clone CE : le clone (et ses descendants deep)
                // matchant une définition s'upgrade — ctor
                // rejoué sur le nœud frais, détaché (JS-C68).
                ce_upgrade_subtree(e, &self.dom, &self.cache, c)?;
                Ok(Some(node_value(e, &self.dom, &self.cache, c)))
            }
            "contains" => {
                let r = node_for_in(e, arg(args, 0), &self.dom)
                    .is_some_and(|o| self.dom.borrow().contains(node, o));
                Ok(Some(Value::Bool(r)))
            }
            // CAP-164 — surface Node de complétude : les libs
            // réelles (React, closure, éditeurs) les lisent
            // pour la délégation et la comparaison d'arbres.
            "getRootNode" => {
                // `{composed:true}` (CAP-191) : depuis
                // l'intérieur d'une racine shadow, remonte à
                // l'arbre de l'hôte (spec — sinon la racine
                // shadow elle-même). Racine `#document` →
                // le global `document` (identité `===`
                // exigée, comme `ownerDocument`).
                let mut root = self.dom.borrow().root_of(node);
                let composed = args.first().is_some_and(|v| {
                    if let Value::Obj(id) = v {
                        e.get_prop(*id, "composed").is_some_and(|c| e.to_boolean(c))
                    } else {
                        false
                    }
                });
                if composed && self.dom.borrow().is_shadow_root(root) {
                    if let Some(h) = self.dom.borrow().shadow_host(root) {
                        root = self.dom.borrow().root_of(h);
                    }
                }
                let doc_root = self.dom.borrow().root;
                let v = if root == doc_root {
                    e.get_prop(e.global, "document").unwrap_or(Value::Null)
                } else {
                    node_value(e, &self.dom, &self.cache, root)
                };
                Ok(Some(v))
            }
            "hasChildNodes" => {
                let r = self
                    .dom
                    .borrow()
                    .nodes
                    .get(node as usize)
                    .is_some_and(|n| !n.children.is_empty());
                Ok(Some(Value::Bool(r)))
            }
            "isSameNode" => {
                let r = node_for_in(e, arg(args, 0), &self.dom).is_some_and(|o| o == node);
                Ok(Some(Value::Bool(r)))
            }
            "isEqualNode" => {
                let r = node_for_in(e, arg(args, 0), &self.dom)
                    .is_some_and(|o| self.dom.borrow().is_equal_node(node, o));
                Ok(Some(Value::Bool(r)))
            }
            "compareDocumentPosition" => {
                let Some(o) = node_for_in(e, arg(args, 0), &self.dom) else {
                    return Err(e.type_error("compareDocumentPosition: not a node"));
                };
                let bits = self.dom.borrow().compare_position(node, o);
                Ok(Some(Value::Num(f64::from(bits))))
            }
            "normalize" => {
                self.dom.borrow_mut().normalize(node);
                Ok(Some(Value::Undefined))
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
            // `<canvas>.toDataURL()`/`toBlob()` — stub honnête
            // (pas de raster côté miroir, JS-C28) : le préfixe
            // `data:` d'un PNG vide suffit aux feature-detects
            // anti-bot (aliexpress sufei-punish lit le format).
            // `toBlob` passe la data-URL vide à son callback.
            "toDataURL" => {
                let is_canvas = self.dom.borrow().nodes[node as usize].tag == "canvas";
                if !is_canvas {
                    return Ok(None);
                }
                Ok(Some(e.str_val("data:image/png;base64,")))
            }
            "toBlob" => {
                let is_canvas = self.dom.borrow().nodes[node as usize].tag == "canvas";
                if !is_canvas {
                    return Ok(None);
                }
                let cb = arg(args, 0);
                if e.is_callable(cb) {
                    let b = e.str_val("data:image/png;base64,");
                    e.call_value(cb, Value::Undefined, &[b])?;
                }
                Ok(Some(Value::Undefined))
            }
            // `attachShadow(init)` — surface tranche 1
            // (CAP-191, hors scoping/slots/répartition) :
            // attache un `DocumentFragment` rôle shadowRoot
            // et le retourne — suffit aux polyfills
            // d'instrumentation (Invoker Commands : `n.call`
            // sur la méthode absente tuait github-elements).
            // `mode:"closed"` → `el.shadowRoot` null, racine
            // réelle quand même. Re-appel → `NotSupportedError`
            // (spec, CAP-191 : renvoyer l'existante trompait
            // les feature-detects qui vérifient le jeté).
            "attachShadow" => {
                let open = match args.first() {
                    Some(Value::Obj(id)) => {
                        e.get_prop(*id, "mode")
                            .and_then(|m| e.to_js_string(m).ok())
                            .as_deref()
                            != Some("closed")
                    }
                    _ => true,
                };
                if self.dom.borrow().shadow_root(node, true).is_some() {
                    let v = e.error_value(
                        e.protos.error,
                        "NotSupportedError",
                        "attachShadow: shadow root already attached",
                    );
                    return Err(crate::heap::Thrown(v));
                }
                let r = self.dom.borrow_mut().create_fragment();
                self.dom.borrow_mut().attach_shadow_root(node, r, open);
                Ok(Some(node_value(e, &self.dom, &self.cache, r)))
            }
            // `attachInternals()` — stub (issue #25) : objet
            // minimal exposant `shadowRoot` (même en mode
            // `closed`, spec `ElementInternals`).
            "attachInternals" => Ok(Some(e.new_host_object(Rc::new(ElementInternalsHost {
                dom: self.dom.clone(),
                cache: self.cache.clone(),
                node,
            })))),
            // `slot.assignedElements()`/`assignedNodes()`
            // (T3) — répartition flat du miroir : enfants
            // light-DOM de l'hôte dont `slot` égale
            // `slot.name` ("" → défaut ; textes → défaut).
            // `{flatten:true}` accepté — borne flat (pas de
            // re-distribution imbriquée).
            "assignedElements" | "assignedNodes" => {
                let els = name == "assignedElements";
                let nodes = self.dom.borrow().slot_assigned(node, els);
                let arr = nodes
                    .into_iter()
                    .map(|n| node_value(e, &self.dom, &self.cache, n))
                    .collect::<Vec<_>>();
                Ok(Some(e.new_array(arr)))
            }
            // `prepend(...kids)` / `before`/`after`/`replaceWith` —
            // nodes or strings (strings → text nodes).
            "prepend" => {
                let kids = self.norm_children(e, args)?;
                // Ancre glissante (comme `after`) : chaque inséré
                // se place après le précédent — l'ordre des args
                // est préservé, et un `first` déplacé par
                // l'opération ne casse pas l'insertion (issue #17).
                let mut anchor = None;
                for k in kids {
                    let ref_n = match anchor {
                        None => self.dom.borrow().child_nodes(node).first().copied(),
                        Some(a) => self.dom.borrow().sibling(a, true),
                    };
                    self.dom.borrow_mut().insert_before(node, k, ref_n);
                    anchor = Some(k);
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
            // CAP-177 — primitives mesurées absentes par la
            // sonde `_census_calls` (TypeError terrain) :
            // `insertAdjacentElement` = même rail que Text
            // mais la charge est un nœud ; `replaceChildren`
            // vide puis insère (React/lit-html l'utilisent).
            "insertAdjacentElement" => {
                let pos = e.to_js_string(arg(args, 0))?;
                let Some(child) = node_for_in(e, arg(args, 1), &self.dom)
                    .or_else(|| adopt_into(e, arg(args, 1), &self.dom, &self.cache))
                else {
                    return Err(e.type_error("insertAdjacentElement: not a node"));
                };
                self.insert_at_position(e, node, &pos, child)?;
                Ok(Some(arg(args, 1)))
            }
            // CAP-178 — sérialisation/visibilité mesurées
            // absentes (sonde `_census_calls`) : `getHTML()` =
            // `outerHTML` (borné : options `serializable`/
            // `shadowRoots` ignorées), `setHTMLUnsafe(html)` =
            // écriture `innerHTML` (même rail, scripts inertes
            // — spec), `checkVisibility(opts?)` = attaché +
            // pas `hidden` + `style.display` ≠ `none` (borné :
            // inline seul — le miroir ne porte pas la cascade
            // ; `opacityProperty`/`visibilityProperty`/`contentVisibilityAuto`
            // non évalués).
            "getHTML" => {
                let s = self.dom.borrow().outer_html(node);
                Ok(Some(e.str_val(s)))
            }
            "setHTMLUnsafe" => {
                let html = e.to_js_string(arg(args, 0))?;
                self.set_inner_html(node, &html);
                Ok(Some(Value::Undefined))
            }
            "checkVisibility" => {
                let d = self.dom.borrow();
                let attached = d.root != node && d.contains(d.root, node);
                let hidden = d.has_attr(node, "hidden");
                let display_none = d.attr(node, "style").is_some_and(|s| {
                    s.split(';').any(|p| {
                        let mut kv = p.splitn(2, ':');
                        matches!(kv.next(), Some(k) if k.trim().eq_ignore_ascii_case("display"))
                            && matches!(kv.next(), Some(v) if v.trim() == "none")
                    })
                });
                Ok(Some(Value::Bool(attached && !hidden && !display_none)))
            }
            "replaceChildren" => {
                let kids = self.norm_children(e, args)?;
                self.dom.borrow_mut().replace_children(node, &kids);
                Ok(Some(Value::Undefined))
            }
            // Sélection de champ `input`/`textarea` (CAP-177) —
            // slots `data-explorer-sel-*` sérialisés dans le
            // miroir (même motif que `CUSTOM_VALIDITY_ATTR`,
            // ADR-0014) : `select()` = 0..len, `setSelectionRange`
            // = bornes explicit, `setRangeText` édite `value`
            // puis repositionne le caret (spec « preserve »
            // par défaut).
            "select" | "setSelectionRange" | "setRangeText" if is_field(&self.dom, node) => {
                match name {
                    "select" => {
                        let len = {
                            let d = self.dom.borrow();
                            control_value(&d, node).chars().count().to_string()
                        };
                        let mut d = self.dom.borrow_mut();
                        d.set_attr(node, SEL_START_ATTR, "0");
                        d.set_attr(node, SEL_END_ATTR, &len);
                        d.set_attr(node, SEL_DIR_ATTR, "none");
                    }
                    "setSelectionRange" => {
                        let s = e.to_number(arg(args, 0))?.max(0.0);
                        let en = e.to_number(arg(args, 1))?.max(0.0);
                        let dir = match arg(args, 2) {
                            Value::Undefined => "none".to_string(),
                            v => e.to_js_string(v)?,
                        };
                        let mut d = self.dom.borrow_mut();
                        d.set_attr(node, SEL_START_ATTR, &format!("{}", s as u64));
                        d.set_attr(node, SEL_END_ATTR, &format!("{}", en as u64));
                        d.set_attr(node, SEL_DIR_ATTR, &dir);
                    }
                    // `setRangeText(repl, s?, e?, mode?)` —
                    // spec : remplace dans `value` puis
                    // repositionne le caret (défaut
                    // « preserve » — borné : repositionnement
                    // toujours à la fin du remplacement,
                    // documenté).
                    _ => {
                        let repl = e.to_js_string(arg(args, 0))?;
                        let s = e.to_number(arg(args, 1))?.max(0.0) as usize;
                        let en = e.to_number(arg(args, 2))?.max(0.0) as usize;
                        let mut d = self.dom.borrow_mut();
                        let cur = control_value(&d, node);
                        let head: String = cur.chars().take(s).collect();
                        let tail: String = cur.chars().skip(en).collect();
                        let caret = head.chars().count() + repl.chars().count();
                        let new_v = format!("{head}{repl}{tail}");
                        if d.nodes[node as usize].tag.eq_ignore_ascii_case("textarea") {
                            let t = d.create_text(&new_v);
                            d.replace_children(node, &[t]);
                        } else {
                            d.set_attr(node, "value", &new_v);
                        }
                        d.set_attr(node, SEL_START_ATTR, &caret.to_string());
                        d.set_attr(node, SEL_END_ATTR, &caret.to_string());
                        d.set_attr(node, SEL_DIR_ATTR, "none");
                    }
                }
                Ok(Some(Value::Undefined))
            }
            // `form.reset()` (CAP-177) — événement `reset`
            // annulable tiré (spec : `preventDefault` conserve
            // les valeurs) ; la restauration des valeurs
            // par défaut reste une couture plus profonde
            // (valeurs initiales non snapshotées) — borné.
            "reset" if self.dom.borrow().nodes[node as usize].tag.as_str() == "form" => {
                e.dispatch_simple(&self.dom, &self.cache, node, "reset")?;
                Ok(Some(Value::Undefined))
            }
            // `<img>.decode()` (CAP-177) — spec : promesse
            // résolue quand l'image est décodée ; le miroir
            // ne décode pas → résolution immédiate honnête
            // (le fetch/décodage réel suit le rail image
            // côté session).
            "decode" if self.dom.borrow().nodes[node as usize].tag.as_str() == "img" => {
                let pid = e.new_promise();
                e.resolve_promise(pid, Value::Undefined);
                Ok(Some(Value::Obj(pid)))
            }
            // Rect viewport depuis la table de géométrie
            // publiée (issue #19, JS-8 — borné : union des
            // boîtes du nœud ≈ border-box ; `None` → zero-rect,
            // comportement historique du stub).
            // `animate`/`getAnimations` (CAP-187) — machine à
            // états honnête (pas de compositeur : state
            // play/pause/finish/cancel, promesses — borné).
            "animate" => Ok(Some(crate::misc_apis::animate(e, this, args)?)),
            "getAnimations" => Ok(Some(crate::misc_apis::get_animations(e, this, args)?)),
            "getBoundingClientRect" => Ok(Some(gbrect_shared(e, &self.dom, node))),
            // `getClientRects` (CAP-180) — même table que
            // `gBCR` ; la map publie l'**union** des boîtes du
            // nœud → la liste spec multi-fragments s'aplatit
            // à `[union]` (borné, documenté). Absent → `[]`.
            "getClientRects" => {
                let rects = if element_geom_rect(&self.dom, node).is_some() {
                    vec![gbrect_shared(e, &self.dom, node)]
                } else {
                    Vec::new()
                };
                Ok(Some(e.new_array(rects)))
            }
            // `HTMLMediaElement` (étape 58/71c, ADR-0023/24) —
            // non claimé : stubs honnêtes, jamais de faux
            // succès. Claimé : délégation hôte (`play()` tenu
            // par le premier `playing` rapporté).
            // `SVGGeometryElement` (§B-14 vol 331, GRC) —
            // `getTotalLength`/`getPointAtLength` sur les
            // formes réelles — géométrie calculée, jamais
            // de valeur bidon.
            "getTotalLength" | "getPointAtLength" if is_svg_geom(&self.dom, node) => {
                let (tag, d_attr, attrs) = {
                    let d = self.dom.borrow();
                    let n = &d.nodes[node as usize];
                    (
                        if n.kind == NodeKind::Element {
                            n.tag.clone()
                        } else {
                            String::new()
                        },
                        d.attr(node, "d").unwrap_or("").to_string(),
                        n.attrs.clone(),
                    )
                };
                if name == "getTotalLength" {
                    let len = if tag == "path" {
                        crate::svg_path::path_total_length(&d_attr)
                    } else {
                        crate::svg_path::shape_length(
                            &tag,
                            &mut attrs.iter().map(|(k, v)| (k.as_str(), v.as_str())),
                        )
                        .unwrap_or(0.0)
                    };
                    return Ok(Some(Value::Num(len)));
                }
                let dist = e.to_number(arg(args, 0))?;
                let pt = if tag == "path" {
                    crate::svg_path::path_point_at(&d_attr, dist)
                } else {
                    crate::svg_path::shape_point_at(
                        &tag,
                        &mut attrs.iter().map(|(k, v)| (k.as_str(), v.as_str())),
                        dist,
                    )
                };
                Ok(Some(match pt {
                    Some((x, y)) => {
                        let o = e.new_object();
                        e.set_own(o, "x", Value::Num(x), true);
                        e.set_own(o, "y", Value::Num(y), true);
                        e.set_own(o, "z", Value::Num(0.0), true);
                        e.set_own(o, "w", Value::Num(1.0), true);
                        Value::Obj(o)
                    }
                    None => Value::Null,
                }))
            }
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
            // `<dialog>` (CAP-176) — `showModal()`/`show()`
            // posent `open` (modal = slot interne) ;
            // `close()` retire `open`, efface le flag modal,
            // pose `returnValue` si arg fourni et tire
            // l'événement `close` — `cancel` (Esc) hors
            // borne miroir (pas d'entrée clavier) : documenté.
            "showModal" | "show" | "close" if is_dialog(&self.dom, node) => {
                match name {
                    "showModal" | "show" => {
                        if self.dom.borrow().has_attr(node, "open") {
                            return Err(crate::heap::Thrown(dom_ex_pub(
                                e,
                                "InvalidStateError",
                                "dialog is already open",
                            )));
                        }
                        self.dom.borrow_mut().set_attr(node, "open", "");
                        if name == "showModal" {
                            e.hooks.dialog_modal.insert(node);
                        }
                        Ok(Some(Value::Undefined))
                    }
                    // `close()` sur dialogue fermé → no-op
                    // silencieux (spec : « if open is not
                    // set, return »).
                    _ => {
                        if !self.dom.borrow().has_attr(node, "open") {
                            return Ok(Some(Value::Undefined));
                        }
                        self.dom.borrow_mut().remove_attr(node, "open");
                        e.hooks.dialog_modal.remove(&node);
                        // `close(result)` — arg présent et non
                        // `undefined` → `returnValue` (spec).
                        if let Some(a) = args.first() {
                            if !matches!(a, Value::Undefined) {
                                let rv = e.to_js_string(*a)?;
                                e.hooks.dialog_retval.insert(node, rv);
                            }
                        }
                        e.dispatch_simple(&self.dom, &self.cache, node, "close")?;
                        Ok(Some(Value::Undefined))
                    }
                }
            }
            // Fullscreen (CAP-179) — spec : promesse résolue,
            // `fullscreenElement` posé, `fullscreenchange` tiré.
            // Élément détaché → promesse rejetée `TypeError`
            // (spec). Borné : état logique honnête — la peinture
            // n'a pas d'overlay plein-écran à changer.
            "requestFullscreen" => {
                let pid = e.new_promise();
                let attached = {
                    let d = self.dom.borrow();
                    d.root_of(node) == d.root
                };
                if attached {
                    e.hooks.fullscreen_el = Some(node);
                    e.dispatch_simple(&self.dom, &self.cache, node, "fullscreenchange")?;
                    e.resolve_promise(pid, Value::Undefined);
                } else {
                    let msg = e.str_val("TypeError: detached element");
                    e.reject_promise(pid, msg);
                }
                Ok(Some(Value::Obj(pid)))
            }
            // Popover (CAP-179) — attribut `popover`
            // requis (spec : `InvalidStateError` sinon,
            // déjà-ouvert pour `show`, déjà-fermé pour
            // `hide`). `togglePopover` rend le nouvel
            // état. `beforetoggle`/`toggle` tirés —
            // `beforetoggle` non-annulable (borné :
            // `dispatch_simple` ne consulte pas
            // `defaultPrevented`).
            "showPopover" | "hidePopover" | "togglePopover" => {
                if !self.dom.borrow().has_attr(node, "popover") {
                    return Err(crate::heap::Thrown(dom_ex_pub(
                        e,
                        "InvalidStateError",
                        "popover attribute required",
                    )));
                }
                let open = e.hooks.popover_open.contains(&node);
                if matches!(name, "showPopover") == open && name != "togglePopover" {
                    return Err(crate::heap::Thrown(dom_ex_pub(
                        e,
                        "InvalidStateError",
                        "popover state invalid",
                    )));
                }
                let now_open = match name {
                    "showPopover" => true,
                    "hidePopover" => false,
                    _ => !open,
                };
                e.dispatch_simple(&self.dom, &self.cache, node, "beforetoggle")?;
                if now_open {
                    e.hooks.popover_open.insert(node);
                } else {
                    e.hooks.popover_open.remove(&node);
                }
                e.dispatch_simple(&self.dom, &self.cache, node, "toggle")?;
                Ok(Some(if name == "togglePopover" {
                    Value::Bool(now_open)
                } else {
                    Value::Undefined
                }))
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
        // Id obsolète (arène resynchronisée pendant que le JS
        // tenait la référence — spectrum `groupChildren` :
        // `nodes[614]` sur arène 507) → pas un nœud : TypeError
        // amont plutôt que panic dans `append`/`detach`.
        if !n.dom.borrow().node_valid(n.node) {
            return None;
        }
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

/// `node_for` restreint à **l'arène `dom` du receveur** — `None`
/// si le handle appartient à un autre miroir (cross-document :
/// `iframe.contentDocument`, `DOMParser`, `xhr.responseXML`,
/// `createHTMLDocument`). Sans ce garde, l'id étranger était
/// validé contre SA propre arène puis indexé dans celle du
/// receveur — hors-limites = panic (`is_ancestor_or_self`),
/// in-bounds = mutation du **mauvais nœud** (spectrum :
/// `<main>` détaché par un `remove()` d'id transplanté).
/// La spec adopterait le nœud cross-document — les arènes ne
/// fusionnant pas, on dégrade en « pas un nœud » (`TypeError`
/// amont), jamais en corruption.
pub(crate) fn node_for_in(e: &mut Engine, v: Value, dom: &DomRef) -> Option<NodeId> {
    let Value::Obj(id) = v else {
        return None;
    };
    let h = match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::Host(h)) => h.clone(),
        _ => return None,
    };
    if let Some(n) = h.as_any().downcast_ref::<NodeHost>() {
        if !Rc::ptr_eq(&n.dom, dom) || !dom.borrow().node_valid(n.node) {
            return None;
        }
        return Some(n.node);
    }
    if let Some(dh) = h.as_any().downcast_ref::<DocumentHost>() {
        if !Rc::ptr_eq(&dh.dom, dom) {
            return None;
        }
        return Some(dh.dom.borrow().root);
    }
    None
}

/// `(node, dom)` d'un `NodeHost` d'un **autre** miroir que
/// `dom` — résolveur d'`importNode` (spec : le nœud source
/// peut vivre dans n'importe quel document). `None` si même
/// miroir (cas couvert par [`node_for_in`]), non-nœud, ou
/// nœud obsolète dans sa propre arène.
fn foreign_node_in(e: &mut Engine, v: Value, dom: &DomRef) -> Option<(NodeId, DomRef)> {
    let Value::Obj(id) = v else {
        return None;
    };
    let h = match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::Host(h)) => h.clone(),
        _ => return None,
    };
    if let Some(n) = h.as_any().downcast_ref::<NodeHost>() {
        if Rc::ptr_eq(&n.dom, dom) || !n.dom.borrow().node_valid(n.node) {
            return None;
        }
        return Some((n.node, n.dom.clone()));
    }
    // Document étranger (usnews §B-14 — `importNode` d'un
    // document auxiliaire jQuery `createHTMLDocument`) : la
    // racine de son arène est la cible du clone, comme
    // `node_for_in` pour le document local.
    if let Some(dh) = h.as_any().downcast_ref::<DocumentHost>() {
        if Rc::ptr_eq(&dh.dom, dom) {
            return None;
        }
        let root = dh.dom.borrow().root;
        return Some((root, dh.dom.clone()));
    }
    None
}

/// Reconnaît un objet **Attr-shaped** (`{nodeType:2, name,
/// value, namespaceURI?}`) — nos porteurs `attr_obj` ou tout
/// objet étranger de même forme. `importNode(attr)` (spec)
/// clone l'attribut dans le document cible.
// nodeType Attr — comparaison exacte d'un discriminant
// entier stocké en f64 (pas une mesure flottante).
#[allow(clippy::float_cmp)]
fn attr_shape_in(e: &mut Engine, v: Value) -> JsResult<Option<(String, String, Option<String>)>> {
    let Value::Obj(_) = v else {
        return Ok(None);
    };
    let nt = e.get_prop_value(v, "nodeType")?;
    if !matches!(nt, Value::Num(n) if n == 2.0) {
        return Ok(None);
    }
    let name_v = e.get_prop_value(v, "name")?;
    let name = e.to_js_string(name_v)?;
    if name.is_empty() {
        return Ok(None);
    }
    let value_v = e.get_prop_value(v, "value")?;
    let value = e.to_js_string(value_v)?;
    let ns = e.get_prop_value(v, "namespaceURI")?;
    let ns = match ns {
        Value::Str(_) => Some(e.to_js_string(ns)?),
        _ => None,
    };
    Ok(Some((name, value, ns)))
}

/// Objet `Attr` porteur (spec) — l'arène stocke les
/// attributs en paires `(nom, valeur)` sans nœud `Attr` :
/// `createAttribute`/`getAttributeNode` rendent cet objet ;
/// `setAttributeNode`/`removeAttributeNode` relisent
/// `name`/`value` dessus (§B-14 npr vol EXPLOR74 ×4 sites).
fn attr_obj(e: &mut Engine, qualified: &str, value: &str, ns: Option<String>) -> Value {
    let local = qualified
        .rsplit(':')
        .next()
        .unwrap_or(qualified)
        .to_string();
    let prefix = qualified.rsplit_once(':').map(|(p, _)| p.to_string());
    let node_name = e.str_val(qualified);
    let name = e.str_val(qualified);
    let local_v = e.str_val(&local);
    let value_v = e.str_val(value);
    let node_value_v = e.str_val(value);
    let text_content_v = e.str_val(value);
    let ns_v = ns.map_or(Value::Null, |s| e.str_val(s));
    let prefix_v = prefix.map_or(Value::Null, |s| e.str_val(s));
    let o = e.new_object();
    e.set_own(o, "nodeType", Value::Num(2.0), true);
    e.set_own(o, "nodeName", node_name, true);
    e.set_own(o, "name", name, true);
    e.set_own(o, "localName", local_v, true);
    e.set_own(o, "value", value_v, true);
    e.set_own(o, "nodeValue", node_value_v, true);
    e.set_own(o, "textContent", text_content_v, true);
    e.set_own(o, "specified", Value::Bool(true), true);
    e.set_own(o, "ownerElement", Value::Null, true);
    e.set_own(o, "namespaceURI", ns_v, true);
    e.set_own(o, "prefix", prefix_v, true);
    Value::Obj(o)
}

/// Spec `[[adopt]]` — clone le sous-arbre étranger dans `dst`,
/// détache la source **et rebinde l'objet JS** vers le nœud
/// adopté (move sémantique : `el` désigne le nœud dans le nouveau
/// document après `insertBefore`). Styled-components/portal :
/// conteneur d'un autre document (appconsent §B-14). `None` si
/// `v` n'est pas un nœud étranger valide.
fn adopt_into(e: &mut Engine, v: Value, dst: &DomRef, dst_cache: &NodeCache) -> Option<NodeId> {
    let Value::Obj(id) = v else {
        return None;
    };
    let h = match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::Host(h)) => h.clone(),
        _ => return None,
    };
    let n = h.as_any().downcast_ref::<NodeHost>()?;
    if Rc::ptr_eq(&n.dom, dst) || !n.dom.borrow().node_valid(n.node) {
        return None;
    }
    let new_id = {
        let s = n.dom.borrow();
        dst.borrow_mut().adopt_from(&s, n.node)
    };
    n.dom.borrow_mut().detach(n.node);
    if let Some(o) = e.heap.obj_mut(id) {
        o.kind = ObjKind::Host(Rc::new(NodeHost {
            dom: dst.clone(),
            cache: dst_cache.clone(),
            node: new_id,
        }));
    }
    dst_cache.borrow_mut().insert(new_id, id);
    Some(new_id)
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
    if !n.dom.borrow().node_valid(n.node) {
        return None;
    }
    Some((n.node, n.dom.clone(), n.cache.clone()))
}

/// Géométrie publiée de `node` (issue #19, JS-8) — lookup
/// `element_path` du miroir dans la table session.
/// `Some((rect, fixed, scroll_x, scroll_y))` — `None` sans
/// table injectée (harnais), nœud détaché, ou chemin absent
/// (divergence miroir/session bornée → repli sur les 0
/// historiques à l'appelant).
pub(crate) fn element_geom(
    dom: &DomRef,
    node: NodeId,
) -> Option<(crate::geom::GeoRect, bool, f32, f32)> {
    let d = dom.borrow();
    let map = d.geometry.as_ref()?.clone();
    let path = d.element_path(node)?;
    drop(d);
    let g = map.lock().ok()?;
    let eg = g.rects.get(&path).copied();
    if eg.is_none() {
        geom_trace_miss(&g, &path);
    }
    Some((eg?.rect, eg?.fixed, g.scroll_x, g.scroll_y))
}

/// Sonde `EXPLORER_GEOM_TRACE` — classe les miss `gBCR` :
/// un chemin présent partageant le préfixe parent → dérive
/// d'index (mutation miroir qui décale les frères — la boîte
/// existe mais sous l'ancien chemin) ; aucun voisin → nœud
/// jamais posé (post-layout/anonyme — rien à re-collecter).
fn geom_trace_miss(g: &crate::geom::GeometryMap, path: &[(String, u32)]) {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    if std::env::var("EXPLORER_GEOM_TRACE").is_err() {
        return;
    }
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if n >= 40 {
        return;
    }
    let parent = &path[..path.len().saturating_sub(1)];
    let near = g.rects.keys().filter(|k| k.starts_with(parent)).count();
    eprintln!(
        "[geom-miss] path={path:?} parent-hits={near} rects={}",
        g.rects.len()
    );
}

/// Rect **viewport** de `node` pour `getBoundingClientRect` —
/// `(x, y, w, h)` : coords document moins scroll, ou telles
/// quelles sous `position:fixed` (le layout pose `fixed`
/// contre le viewport — i#18).
pub(crate) fn element_geom_rect(dom: &DomRef, node: NodeId) -> Option<(f64, f64, f64, f64)> {
    let (r, fixed, sx, sy) = element_geom(dom, node)?;
    let (x, y) = if fixed {
        (r.x, r.y)
    } else {
        (r.x - sx, r.y - sy)
    };
    Some((f64::from(x), f64::from(y), f64::from(r.w), f64::from(r.h)))
}

/// Border-box `(w, h)` de `node` pour `offset*`/`client*`/
/// `scroll*` — indépendante du scroll (bornée : les trois
/// familles ≈ même boîte, voir `crate::geom`).
fn element_geom_size(dom: &DomRef, node: NodeId) -> Option<(f64, f64)> {
    let (r, ..) = element_geom(dom, node)?;
    Some((f64::from(r.w), f64::from(r.h)))
}

/// `positioned` publié pour `node` (JS-C97) — `true` si une
/// boîte contributrice a `position` ≠ `static`.
fn geom_positioned(dom: &DomRef, node: NodeId) -> bool {
    let Some((map, path)) = ({
        let d = dom.borrow();
        d.geometry.clone().zip(d.element_path(node))
    }) else {
        return false;
    };
    map.lock()
        .ok()
        .and_then(|g| g.rects.get(&path).copied())
        .is_some_and(|eg| eg.positioned)
}

/// `offsetParent` spec (JS-C97) — le plus proche ancêtre
/// **élément** positionné (`ElementGeom.positioned`), ou
/// `td`/`th`/`table`/`body` ; `None` quand la marche atteint
/// `#document` (élément racine, sans boîte, `fixed` — le
/// `fixed` est tranché par l'appelant via `ElementGeom.fixed`).
fn offset_parent_node(dom: &DomRef, node: NodeId) -> Option<NodeId> {
    let mut cur = node;
    loop {
        let p = dom_parent(dom, cur)?;
        let (tag, is_el, is_root) = {
            let d = dom.borrow();
            (
                d.nodes[p as usize].tag.clone(),
                d.nodes[p as usize].kind == NodeKind::Element,
                p == d.root,
            )
        };
        if is_root || !is_el {
            return None;
        }
        if matches!(tag.as_str(), "body" | "table" | "td" | "th") || geom_positioned(dom, p) {
            return Some(p);
        }
        cur = p;
    }
}

/// `window.scrollX`/`pageXOffset` — scroll horizontal de la
/// `GeometryMap` publiée (accessors sur le global).
fn global_scroll_x(e: &mut Engine, _this: Value, _a: &[Value]) -> JsResult<Value> {
    let y = e
        .dom_ctx
        .as_ref()
        .and_then(|(d, _)| {
            d.borrow()
                .geometry
                .as_ref()?
                .lock()
                .ok()
                .map(|g| g.scroll_x)
        })
        .unwrap_or(0.0);
    Ok(Value::Num(f64::from(y)))
}

/// `window.scrollY`/`pageYOffset` — scroll vertical publié.
fn global_scroll_y(e: &mut Engine, _this: Value, _a: &[Value]) -> JsResult<Value> {
    let y = e
        .dom_ctx
        .as_ref()
        .and_then(|(d, _)| {
            d.borrow()
                .geometry
                .as_ref()?
                .lock()
                .ok()
                .map(|g| g.scroll_y)
        })
        .unwrap_or(0.0);
    Ok(Value::Num(f64::from(y)))
}

/// `scrollTop`/`scrollLeft` de `documentElement`/`body` — le
/// scroll du document (les éléments ne scrollent pas → 0
/// honnête ailleurs). `x` = horizontal.
fn geom_scroll_offset(dom: &DomRef, horizontal: bool) -> f64 {
    let Some(slot) = dom.borrow().geometry.clone() else {
        return 0.0;
    };
    let Some(g) = slot.lock().ok() else {
        return 0.0;
    };
    f64::from(if horizontal { g.scroll_x } else { g.scroll_y })
}

/// Dimension du document publiée pour `scroll*` de l'élément
/// racine — `horizontal` → `doc_w`, sinon `doc_h`. `None` sans
/// table (repli viewport à l'appelant).
fn geom_doc_dim(dom: &DomRef, horizontal: bool) -> Option<f64> {
    let map = dom.borrow().geometry.as_ref()?.clone();
    let g = map.lock().ok()?;
    let d = if horizontal { g.doc_w } else { g.doc_h };
    (d > 0.0).then(|| f64::from(d))
}

/// Chemins des éléments contenant le point viewport `(x, y)`
/// (CAP-180 — hit-test inverse sur la `GeometryMap` publiée,
/// i#19) : rects non-`fixed` en coords document → `-scroll`,
/// `fixed` déjà viewport (i#18). Tri **topmost-first** borné :
/// chemin le plus long en premier (le plus profond peint
/// par-dessus ses ancêtres), puis ordre DFS décroissant à
/// profondeur égale (frère tardif au-dessus du frère tôt) —
/// approximation honnête sans z-index/stacking context
/// (borné : `display:none`/absents de la map ne matchent
/// jamais — déjà exclu par la session).
fn point_candidate_paths(dom: &DomRef, x: f64, y: f64) -> Vec<Vec<(String, u32)>> {
    let Some(slot) = dom.borrow().geometry.clone() else {
        return Vec::new();
    };
    let Ok(g) = slot.lock() else {
        return Vec::new();
    };
    let mut hits: Vec<Vec<(String, u32)>> = g
        .rects
        .iter()
        .filter(|(_, eg)| {
            let (rx, ry) = if eg.fixed {
                (f64::from(eg.rect.x), f64::from(eg.rect.y))
            } else {
                (
                    f64::from(eg.rect.x - g.scroll_x),
                    f64::from(eg.rect.y - g.scroll_y),
                )
            };
            x >= rx && x <= rx + f64::from(eg.rect.w) && y >= ry && y <= ry + f64::from(eg.rect.h)
        })
        .map(|(p, _)| p.clone())
        .collect();
    drop(g);
    // Le plus profond d'abord, DFS décroissant ensuite.
    hits.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| b.cmp(a)));
    hits
}

/// L'objet `DOMRect`-like de `getBoundingClientRect` — partagé
/// par l'`invoke` et la native `Element.prototype` (issue
/// #19) : `x`/`left`/`top`/`y`, `right`/`bottom` dérivés,
/// `width`/`height`. Absent de la table → zero-rect
/// (comportement historique du stub — un nœud détaché ou une
/// divergence miroir/session dégrade, ne plante pas).
pub(crate) fn gbrect_shared(e: &mut Engine, dom: &DomRef, node: NodeId) -> Value {
    let (x, y, w, h) = element_geom_rect(dom, node).unwrap_or_default();
    let r = e.new_object();
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
        e.set_own(r, k, Value::Num(v), true);
    }
    Value::Obj(r)
}

/// Resolve a `dispatchEvent` argument to event state: an `Event`
/// host object (its target is reset to the dispatch node) or a
/// plain `{ type: 'x', bubbles: bool }` literal.
impl EventState {
    /// `defaultPrevented` après dispatch — lu par
    /// `reportError` (JS-C74 : annulé = erreur « gérée »).
    pub(crate) fn default_prevented(&self) -> bool {
        self.prevented.get()
    }
}

/// `event_state_of` exposé à `misc_apis` (JS-C74 —
/// `reportError` construit son `ErrorEvent` hors dispatch
/// script-classique).
pub(crate) fn misc_event_state(
    e: &mut Engine,
    node: NodeId,
    dom: &DomRef,
    cache: &NodeCache,
    v: Value,
) -> JsResult<Option<Rc<EventState>>> {
    event_state_of(e, node, dom, cache, v)
}

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
    // `message`/`error`/`filename`/`lineno`/`colno` : ErrorEvent
    // (`reportError` JS-C74) ; `data`/`origin`/`source`/
    // `ports` : MessageEvent.
    let props: Vec<(String, Value)> = [
        "key",
        "code",
        "keyCode",
        "which",
        "charCode",
        "ctrlKey",
        "shiftKey",
        "altKey",
        "metaKey",
        "detail",
        "message",
        "error",
        "filename",
        "lineno",
        "colno",
        "data",
        "origin",
        "source",
        "ports",
        "lastEventId",
        "reason",
        "wasClean",
        // `MouseEventInit` (JS-C83) — `button` est le garde-fou
        // des routers JS (`event.button === 0` dans React
        // Router `<Link>`) ; coords et modifieurs rejoints.
        "button",
        "buttons",
        "clientX",
        "clientY",
        "screenX",
        "screenY",
        "pageX",
        "pageY",
        "offsetX",
        "offsetY",
        "movementX",
        "movementY",
        "x",
        "y",
        "layerX",
        "layerY",
        "view",
        "relatedTarget",
        "pointerType",
        "pointerId",
        "isPrimary",
        // `UIEvent`/`InputEvent`/`CompositionEvent`/`WheelEvent`.
        "inputType",
        "isComposing",
        "deltaX",
        "deltaY",
        "deltaZ",
        "deltaMode",
        "width",
        "height",
        "pressure",
        "tiltX",
        "tiltY",
        "twist",
        "tangentialPressure",
        "dataTransfer",
        "touches",
        "targetTouches",
        "changedTouches",
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

    fn trace(&self, out: &mut Vec<ObjId>) {
        trace_node_cache(&self.cache, out);
        for (_, v) in self.state.props.borrow().iter() {
            crate::heap::push_value(out, *v);
        }
    }

    fn get(&self, e: &mut Engine, _t: Value, key: &str) -> JsResult<Option<Value>> {
        Ok(Some(match key {
            "type" => e.str_val(self.state.ty.borrow().clone()),
            "target" | "srcElement" => {
                // CAP-198 — pendant le dispatch, la cible vue
                // par le listener courant (retarget à la
                // frontière shadow) ; hors dispatch, la cible
                // réelle (spec).
                let n = if self.state.current.get().is_some() {
                    self.state.adjusted.get()
                } else {
                    self.state.target.get()
                };
                node_value(e, &self.dom, &self.cache, n)
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
            // CAP-198 — `relatedTarget` retargeté pour le
            // listener courant pendant le dispatch ; hors
            // dispatch, la valeur `state.props` (init/platforme)
            // ou `null` (défaut spec).
            "relatedTarget" => {
                if let Some(n) = self.state.adjusted_related.get() {
                    node_value(e, &self.dom, &self.cache, n)
                } else {
                    match self
                        .state
                        .props
                        .borrow()
                        .iter()
                        .find(|(k, _)| k == "relatedTarget")
                    {
                        Some((_, v)) => *v,
                        None => Value::Null,
                    }
                }
            }
            _ => match self.state.props.borrow().iter().find(|(k, _)| k == key) {
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
                // Spec : `[]` hors dispatch (`currentTarget` null) ;
                // CAP-198 — le chemin de dispatch stocké inclut les
                // racines shadow traversées (`composed`) et s'arrête
                // à la frontière pour les événements non-composés.
                if self.state.current.get().is_none() {
                    return Ok(Some(e.new_array(Vec::new())));
                }
                let vals: Vec<Value> = self
                    .state
                    .path
                    .borrow()
                    .iter()
                    .map(|&n| node_value(e, &self.dom, &self.cache, n))
                    .collect();
                Ok(Some(e.new_array(vals)))
            }
            "initEvent" | "initCustomEvent" => {
                let ty = e.to_js_string(arg(args, 0))?;
                *self.state.ty.borrow_mut() = ty;
                self.state.bubbles.set(e.to_boolean(arg(args, 1)));
                self.state.cancelable.set(e.to_boolean(arg(args, 2)));
                // Spec `initEvent` : les flags de propagation et
                // `defaultPrevented` repartent à zéro.
                self.state.stopped.set(false);
                self.state.immediate.set(false);
                self.state.prevented.set(false);
                // DOM L2 `initCustomEvent` (polymer/shady) :
                // 4e argument = `detail` — posé dans les props
                // comme le ferait l'init du ctor `CustomEvent`.
                if name == "initCustomEvent" {
                    let mut p = self.state.props.borrow_mut();
                    p.retain(|(k, _)| k != "detail");
                    p.push(("detail".to_string(), arg(args, 3)));
                }
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

/// Nom de propriété normalisé — spec css-variables-1 §2.1 :
/// les propriétés custom `--*` sont **sensibles à la casse**,
/// seules les standard passent en ASCII minuscules (miroir du
/// fix cascade R-92 — issue #20 : `setProperty('--Spacer-x')`
/// perdu sinon).
fn prop_key(raw: &str) -> String {
    if raw.starts_with("--") {
        raw.to_string()
    } else {
        raw.to_lowercase()
    }
}

fn style_map(dom: &Dom, node: NodeId) -> Vec<(String, String)> {
    dom.attr(node, "style").map_or_else(Vec::new, parse_decls)
}

/// Déclarations `k: v` d'une liste `style` — tolérante :
/// segments sans `:` et noms vides ignorés, noms standard
/// minuscules, custom `--*` préservées (`prop_key`).
fn parse_decls(style: &str) -> Vec<(String, String)> {
    style
        .split(';')
        .filter_map(|decl| {
            let (k, v) = decl.split_once(':')?;
            let k = prop_key(k.trim());
            (!k.is_empty()).then(|| (k, v.trim().to_string()))
        })
        .collect()
}

/// `ElementInternals` (stub issue #25) — objet minimal
/// retourné par `attachInternals()` : `shadowRoot` lisible
/// même en mode `closed` (spec `ElementInternals`), le
/// reste de la surface reste à l'arc Shadow DOM.
struct ElementInternalsHost {
    dom: DomRef,
    cache: NodeCache,
    node: NodeId,
}

impl Host for ElementInternalsHost {
    fn class_name(&self) -> &'static str {
        "ElementInternals"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn trace(&self, out: &mut Vec<ObjId>) {
        trace_node_cache(&self.cache, out);
    }

    fn get(&self, e: &mut Engine, _t: Value, key: &str) -> JsResult<Option<Value>> {
        match key {
            "shadowRoot" => {
                let r = self.dom.borrow().shadow_root(self.node, true);
                Ok(Some(match r {
                    Some(r) => node_value(e, &self.dom, &self.cache, r),
                    None => Value::Null,
                }))
            }
            _ => Ok(None),
        }
    }
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
        // `length`/`style[i]` : `CSSStyleDeclaration` est array-
        // like spec (nombre + noms de déclarations indexés) — les
        // helpers `makeIterator` des bundles closure lisent
        // `length` numérique (JS-C76 ; avant, `style.length`
        // tombait sur la lookup de propriété CSS → `""`).
        if key == "length" {
            let n = {
                let d = self.dom.borrow();
                style_map(&d, self.node).len()
            };
            return Ok(Some(Value::Num(n as f64)));
        }
        if let Some(i) = crate::interp::array_index(key) {
            let name = {
                let d = self.dom.borrow();
                style_map(&d, self.node).get(i).map(|(k, _)| k.clone())
            };
            return Ok(Some(match name {
                Some(k) => e.str_val(k),
                None => Value::Undefined,
            }));
        }
        // `cssText` (JS-C174 / §B-7) — sérialisation inline de
        // la déclaration entière, pas une propriété CSS.
        if key == "cssText" {
            let css = {
                let d = self.dom.borrow();
                style_map(&d, self.node)
                    .iter()
                    .map(|(k, v)| format!("{k}: {v}"))
                    .collect::<Vec<_>>()
                    .join("; ")
            };
            return Ok(Some(e.str_val(css)));
        }
        // Named-getter spec CSSStyleDeclaration : seules les
        // propriétés CSS servent `""`. `hasOwnProperty`/`toString`/
        // les noms de méthodes host/symboles ne sont PAS des
        // propriétés CSS — `None` laisse la lookup continuer
        // (proto `Object` → `has_method` → `undefined`), sinon
        // `style.hasOwnProperty` rendait `""` et le boot jvc
        // (`s.hasOwnProperty(k)`) mourait en TypeError.
        if key.starts_with('\u{f130}')
            || matches!(
                key,
                "hasOwnProperty"
                    | "isPrototypeOf"
                    | "propertyIsEnumerable"
                    | "toLocaleString"
                    | "toString"
                    | "valueOf"
                    | "constructor"
                    | "__proto__"
                    | "__defineGetter__"
                    | "__defineSetter__"
                    | "__lookupGetter__"
                    | "__lookupSetter__"
                    | "setProperty"
                    | "getPropertyValue"
                    | "removeProperty"
                    | "item"
            )
        {
            return Ok(None);
        }
        let kebab = camel_to_kebab(key);
        let v = {
            let d = self.dom.borrow();
            style_map(&d, self.node)
                .into_iter()
                .find(|(k, _)| k == &kebab)
                .map(|(_, v)| v)
        };
        // CAP-196 — `getComputedStyle(el).position` : la feuille
        // de style n'a pas de relais vers JS, mais la session a
        // vu la cascade au layout — quand la géométrie publiée
        // marque la boîte `fixed` (i#19), servir `"fixed"`.
        // Mur mesuré : garde `"fixed"===position` du script
        // `--marketing-header-fixed-top` de github. Inline
        // prime toujours ; `el.style` (computed=false) reste
        // pur inline.
        let v = match (v, self.computed && kebab == "position") {
            (Some(v), _) => v,
            (None, true) => match element_geom(&self.dom, self.node) {
                Some((_, true, _, _)) => "fixed".to_string(),
                _ => String::new(),
            },
            (None, false) => String::new(),
        };
        Ok(Some(e.str_val(v)))
    }

    fn set(&self, e: &mut Engine, _t: Value, key: &str, v: Value) -> JsResult<bool> {
        let val = e.to_js_string(v)?;
        // `cssText` (JS-C174 / §B-7 — geeksforgeeks hydrate)
        // : remplace **toute** la liste inline — écrire
        // `css-text: …` laisserait `visibility:hidden` en
        // place et la page anti-FOUC resterait invisible.
        if key == "cssText" {
            let map = parse_decls(&val);
            let mut d = self.dom.borrow_mut();
            if map.is_empty() {
                d.remove_attr(self.node, "style");
            } else {
                let css = map
                    .iter()
                    .map(|(k, v)| format!("{k}: {v}"))
                    .collect::<Vec<_>>()
                    .join("; ");
                d.set_attr(self.node, "style", &css);
            }
            return Ok(true);
        }
        let kebab = camel_to_kebab(key);
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
                let k = prop_key(&e.to_js_string(arg(args, 0))?);
                let v = e.to_js_string(arg(args, 1))?;
                style_set(&mut self.dom.borrow_mut(), self.node, &k, &v);
                Ok(Some(Value::Undefined))
            }
            "getPropertyValue" => {
                let k = prop_key(&e.to_js_string(arg(args, 0))?);
                let v = {
                    let d = self.dom.borrow();
                    style_map(&d, self.node)
                        .into_iter()
                        .find(|(kk, _)| kk == &k)
                        .map(|(_, v)| v)
                };
                // CAP-196 — même repli `position`←géométrie que
                // `get` : les deux accesseurs restent cohérents.
                let v = match (v, self.computed && k == "position") {
                    (Some(v), _) => v,
                    (None, true) => match element_geom(&self.dom, self.node) {
                        Some((_, true, _, _)) => "fixed".to_string(),
                        _ => String::new(),
                    },
                    (None, false) => String::new(),
                };
                Ok(Some(e.str_val(v)))
            }
            "removeProperty" => {
                let k = prop_key(&e.to_js_string(arg(args, 0))?);
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
            // `style.item(i)` — nom de la i-ème déclaration, `""`
            // hors-bornes (spec CSSStyleDeclaration).
            "item" => {
                let i = e.to_number(arg(args, 0))? as usize;
                let name = {
                    let d = self.dom.borrow();
                    style_map(&d, self.node)
                        .get(i)
                        .map(|(k, _)| k.clone())
                        .unwrap_or_default()
                };
                Ok(Some(e.str_val(name)))
            }
            _ => Ok(None),
        }
    }

    fn has_method(&self, name: &str) -> bool {
        matches!(
            name,
            "setProperty" | "getPropertyValue" | "removeProperty" | "item"
        )
    }
}

// ------------------------------------------------------------------
// classList
// ------------------------------------------------------------------

/// Types de liens supportés pour `relList.supports` (spec
/// linkTypes courants — la liste exacte évolue avec le moteur ;
/// les types chargés par le pipeline figurent tous).
const LINK_REL_SUPPORTED: &[&str] = &[
    "stylesheet",
    "preload",
    "modulepreload",
    "prefetch",
    "preconnect",
    "dns-prefetch",
    "icon",
    "shortcut icon",
    "apple-touch-icon",
    "manifest",
    "canonical",
    "alternate",
    "next",
    "prev",
    "author",
    "license",
    "help",
    "search",
    "tag",
    "bookmark",
    "nofollow",
    "noopener",
    "noreferrer",
    "opener",
];

impl ClassListHost {
    /// Jetons `class` courants — relus à chaque appel (vivants).
    fn tokens(&self) -> Vec<String> {
        self.dom
            .borrow()
            .attr(self.node, self.attr)
            .map(|c| c.split_whitespace().map(str::to_string).collect())
            .unwrap_or_default()
    }
}

impl Host for ClassListHost {
    fn class_name(&self) -> &'static str {
        "DOMTokenList"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn get(&self, e: &mut Engine, _t: Value, key: &str) -> JsResult<Option<Value>> {
        if key == "length" {
            return Ok(Some(Value::Num(self.tokens().len() as f64)));
        }
        // Accès indexé `cl[i]` + `cl.value` (JS-C76 — DOMTokenList
        // est array-like : le chemin `makeIterator` des bundles
        // closure itère `a[b++]`).
        if let Some(i) = crate::interp::array_index(key) {
            return Ok(Some(match self.tokens().get(i) {
                Some(t) => e.str_val(t.clone()),
                None => Value::Undefined,
            }));
        }
        if key == "value" {
            return Ok(Some(e.str_val(self.tokens().join(" "))));
        }
        Ok(None)
    }

    /// Méthodes lisibles (JS-C91) — `classList["add"]` doit se
    /// lire comme fonction : htmx fait
    /// `e.classList["add"].call(e.classList, "htmx-indicator")`
    /// — `.call` sur `undefined` tuait tout son pipeline
    /// `hx-indicator`.
    fn has_method(&self, name: &str) -> bool {
        matches!(
            name,
            "item"
                | "forEach"
                | "entries"
                | "keys"
                | "values"
                | "supports"
                | "toString"
                | "toLocaleString"
                | "replace"
                | "add"
                | "remove"
                | "toggle"
                | "contains"
        )
    }

    /// `Symbol.iterator` — `DOMTokenList` est itérable spec
    /// (JS-C76).
    fn iter_items(&self, e: &mut Engine) -> JsResult<Option<Vec<Value>>> {
        Ok(Some(
            self.tokens().into_iter().map(|t| e.str_val(t)).collect(),
        ))
    }

    fn keys(&self, _e: &mut Engine, _t: Value) -> Vec<String> {
        (0..self.tokens().len()).map(|i| i.to_string()).collect()
    }

    fn invoke(
        &self,
        e: &mut Engine,
        this: Value,
        name: &str,
        args: &[Value],
    ) -> JsResult<Option<Value>> {
        // Méthodes non-token d'abord — arg0 ≠ jeton (JS-C76 : la
        // coercion préalable jetait `forEach(cb)` en string).
        match name {
            "item" => {
                let i = e.to_number(arg(args, 0))? as usize;
                return Ok(Some(match self.tokens().get(i) {
                    Some(t) => e.str_val(t.clone()),
                    None => Value::Null,
                }));
            }
            "forEach" => {
                let cb = arg(args, 0);
                let this_arg = arg(args, 1);
                for (i, t) in self.tokens().into_iter().enumerate() {
                    let tv = e.str_val(t);
                    e.call_value(cb, this_arg, &[tv, Value::Num(i as f64), this])?;
                }
                return Ok(Some(Value::Undefined));
            }
            "entries" | "keys" | "values" => {
                let tokens = self.tokens();
                let items: Vec<Value> = match name {
                    "keys" => (0..tokens.len()).map(|i| Value::Num(i as f64)).collect(),
                    "values" => tokens.into_iter().map(|t| e.str_val(t)).collect(),
                    _ => tokens
                        .into_iter()
                        .enumerate()
                        .map(|(i, t)| {
                            let tv = e.str_val(t);
                            e.new_array(vec![Value::Num(i as f64), tv])
                        })
                        .collect(),
                };
                let proto = e.protos.array_iter;
                return Ok(Some(crate::collections::iter_obj(e, items, proto)));
            }
            // Spec DOMTokenList.supports : jeton dans la
            // liste des jetons **supportés** de l'attribut —
            // indépendante du contenu courant. `rel` sur
            // link/a/area/form → types de liens ; `class` →
            // pas de jetons supportés → TypeError (Chrome).
            "supports" => {
                let t = e.to_js_string(arg(args, 0))?;
                if self.attr == "rel" {
                    return Ok(Some(Value::Bool(LINK_REL_SUPPORTED.contains(&t.as_str()))));
                }
                return Err(e.type_error("DOMTokenList has no supported tokens"));
            }
            "toString" | "toLocaleString" => {
                return Ok(Some(e.str_val(self.tokens().join(" "))));
            }
            "replace" => {
                let a = e.to_js_string(arg(args, 0))?;
                let b = e.to_js_string(arg(args, 1))?;
                let mut tokens = self.tokens();
                if !tokens.iter().any(|t| t == &a) {
                    return Ok(Some(Value::Bool(false)));
                }
                for t in &mut tokens {
                    if *t == a {
                        t.clone_from(&b);
                    }
                }
                self.dom
                    .borrow_mut()
                    .set_attr(self.node, self.attr, &tokens.join(" "));
                return Ok(Some(Value::Bool(true)));
            }
            _ => {}
        }
        let cls = e.to_js_string(arg(args, 0))?;
        let mut d = self.dom.borrow_mut();
        let mut tokens: Vec<String> = d
            .attr(self.node, self.attr)
            .map(|c| c.split_whitespace().map(str::to_string).collect())
            .unwrap_or_default();
        let contains = tokens.iter().any(|t| t == &cls);
        match name {
            "add" => {
                if !contains {
                    tokens.push(cls.clone());
                }
            }
            "remove" => tokens.retain(|t| t != &cls),
            "toggle" => {
                // `toggle(t, force)` — le second arg force l'état
                // cible (spec DOMTokenList, JS-C76).
                let force = match arg(args, 1) {
                    Value::Undefined => None,
                    f => Some(e.to_boolean(f)),
                };
                let want = force.unwrap_or(!contains);
                if want {
                    tokens.push(cls.clone());
                } else {
                    tokens.retain(|t| t != &cls);
                }
            }
            "contains" => return Ok(Some(Value::Bool(contains))),
            _ => return Ok(None),
        }
        d.set_attr(self.node, self.attr, &tokens.join(" "));
        Ok(Some(match name {
            "toggle" => Value::Bool(tokens.iter().any(|t| t == &cls)),
            _ => Value::Undefined,
        }))
    }
}

// ------------------------------------------------------------------
// attributes — NamedNodeMap (JS-C76)
// ------------------------------------------------------------------

/// `el.attributes` — `NamedNodeMap` vivante sur les `attrs` du
/// miroir : `length`, accès indexé (`attributes[0]`), accès nommé
/// (`attributes.href`/`attributes["href"]`), `item`/`getNamedItem`/
/// `setNamedItem`/`removeNamedItem` (variantes `*NS` = alias sur
/// le nom local — pas de namespaces dans le miroir), itérable
/// (for-of de facto — déclencheur `Error: b` google.com).
struct AttributesHost {
    dom: DomRef,
    cache: NodeCache,
    node: NodeId,
}

impl AttributesHost {
    /// `(name, value)` des attributs courants — relus (vivants).
    fn attrs(&self) -> Vec<(String, String)> {
        self.dom.borrow().nodes[self.node as usize].attrs.clone()
    }

    /// `Attr` snapshot pour `(name, value)` + lien `ownerElement`.
    fn attr_at(&self, e: &mut Engine, name: &str, value: &str) -> Value {
        let owner = node_value(e, &self.dom, &self.cache, self.node);
        attr_value(e, owner, name, value)
    }
}

impl Host for AttributesHost {
    fn class_name(&self) -> &'static str {
        "NamedNodeMap"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn trace(&self, out: &mut Vec<ObjId>) {
        trace_node_cache(&self.cache, out);
    }

    fn get(&self, e: &mut Engine, _t: Value, key: &str) -> JsResult<Option<Value>> {
        if key == "length" {
            return Ok(Some(Value::Num(self.attrs().len() as f64)));
        }
        if let Some(i) = crate::interp::array_index(key) {
            let attrs = self.attrs();
            return Ok(Some(match attrs.get(i) {
                Some((n, v)) => {
                    let (n, v) = (n.clone(), v.clone());
                    self.attr_at(e, &n, &v)
                }
                None => Value::Undefined,
            }));
        }
        // Accès nommé : `attributes.href` — les props réelles de
        // la map priment (longueur/indexés lus plus haut).
        let found = {
            let d = self.dom.borrow();
            d.attr(self.node, key).map(str::to_string)
        };
        if let Some(v) = found {
            return Ok(Some(self.attr_at(e, key, &v)));
        }
        Ok(None)
    }

    fn iter_items(&self, e: &mut Engine) -> JsResult<Option<Vec<Value>>> {
        let mut out = Vec::new();
        for (n, v) in self.attrs() {
            out.push(self.attr_at(e, &n, &v));
        }
        Ok(Some(out))
    }

    fn keys(&self, _e: &mut Engine, _t: Value) -> Vec<String> {
        (0..self.attrs().len()).map(|i| i.to_string()).collect()
    }

    fn invoke(
        &self,
        e: &mut Engine,
        _t: Value,
        name: &str,
        args: &[Value],
    ) -> JsResult<Option<Value>> {
        match name {
            "item" => {
                let i = e.to_number(arg(args, 0))? as usize;
                let attrs = self.attrs();
                return Ok(Some(match attrs.get(i) {
                    Some((n, v)) => {
                        let (n, v) = (n.clone(), v.clone());
                        self.attr_at(e, &n, &v)
                    }
                    None => Value::Null,
                }));
            }
            "getNamedItem" | "getNamedItemNS" => {
                // `*NS` : nom local = dernier arg (pas de
                // namespaces dans le miroir).
                let k = e.to_js_string(arg(args, usize::from(name.ends_with("NS"))))?;
                let v = {
                    let d = self.dom.borrow();
                    d.attr(self.node, &k).map(str::to_string)
                };
                return Ok(Some(match v {
                    Some(v) => self.attr_at(e, &k, &v),
                    None => Value::Null,
                }));
            }
            "setNamedItem" | "setNamedItemNS" => {
                let attr = arg(args, usize::from(name.ends_with("NS")));
                let nv = e.get_prop_value(attr, "name")?;
                let vv = e.get_prop_value(attr, "value")?;
                let k = e.to_js_string(nv)?;
                let v = e.to_js_string(vv)?;
                let old = {
                    let d = self.dom.borrow();
                    d.attr(self.node, &k).map(str::to_string)
                };
                self.dom.borrow_mut().set_attr(self.node, &k, &v);
                // Spec : rend l'attribut remplacé, `null` sinon.
                return Ok(Some(match old {
                    Some(old) => self.attr_at(e, &k, &old),
                    None => Value::Null,
                }));
            }
            "removeNamedItem" | "removeNamedItemNS" => {
                let k = e.to_js_string(arg(args, usize::from(name.ends_with("NS"))))?;
                let old = {
                    let d = self.dom.borrow();
                    d.attr(self.node, &k).map(str::to_string)
                };
                let Some(old) = old else {
                    // `NotFoundError` (DOMException) ≙ TypeError —
                    // la famille DOMException n'est pas encore
                    // constructible (stub `illegal_ctor`).
                    return Err(e.type_error(format!("no attribute named `{k}`")));
                };
                self.dom.borrow_mut().remove_attr(self.node, &k);
                return Ok(Some(self.attr_at(e, &k, &old)));
            }
            _ => {}
        }
        Ok(None)
    }
}

// ------------------------------------------------------------------
// window.location / window.history (JS-7e)
// ------------------------------------------------------------------

/// `location` — getters read `hooks.page_url` (parsed with no URL
/// dep: scheme `//` authority `/` path `?` query `#` fragment);
/// setters/`assign`/`replace`/`reload` go through `hooks.navigate`.
/// `customElements` — `CustomElementRegistry` (JS-C68) adossé
/// au registre du `Dom` : `define` valide nom/ctor/proto, fige
/// `observedAttributes`, résout les `whenDefined` pendantes et
/// **upgrade les éléments attachés** déjà présents ; `upgrade`
/// applique au sous-arbre ; `get`/`getName` interrogent le
/// registre. Mesuré github.com : `customElements.define(name,
/// class)` (JS-C56) — le résidu `turbo-frame.delegate` exigeait
/// `whenDefined` tenue + callbacks.
struct CustomElementsHost {
    dom: DomRef,
    cache: NodeCache,
}

/// Nom de custom element valide (spec `PotentialCustomElementName`,
/// réduit ASCII) : `[a-z][a-z0-9._-]*-[a-z0-9._-]*`, hors la
/// liste réservée (`font-face*`…).
fn ce_valid_name(n: &str) -> bool {
    const RESERVED: &[&str] = &[
        "annotation-xml",
        "color-profile",
        "font-face",
        "font-face-src",
        "font-face-uri",
        "font-face-format",
        "font-face-name",
        "missing-glyph",
    ];
    let mut chars = n.chars();
    if !chars.next().is_some_and(|c| c.is_ascii_lowercase()) {
        return false;
    }
    if !n.contains('-') || RESERVED.contains(&n) {
        return false;
    }
    n.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
}

impl Host for CustomElementsHost {
    fn class_name(&self) -> &'static str {
        "CustomElementRegistry"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn trace(&self, out: &mut Vec<ObjId>) {
        trace_node_cache(&self.cache, out);
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
                tracing::debug!(name = %n, "customElements.define");
                // Spec `define` : DOMException typés — les
                // frameworks (catalyst github) tolèrent
                // `NotSupportedError` par `e.name` et rejettent
                // le reste : un TypeError cassait le chemin.
                if !ce_valid_name(&n) {
                    return Err(dom_exception(
                        e,
                        "SyntaxError",
                        &format!("customElements.define: invalid name {n:?}"),
                    ));
                }
                if !e.is_callable(ctor) {
                    return Err(e.type_error("customElements.define: ctor is not a constructor"));
                }
                let proto = e.get_prop_value(ctor, "prototype")?;
                if !matches!(proto, Value::Obj(_)) {
                    return Err(
                        e.type_error("customElements.define: ctor.prototype is not an object")
                    );
                }
                {
                    let d = self.dom.borrow();
                    if d.ce.defs.contains_key(&n) {
                        return Err(dom_exception(
                            e,
                            "NotSupportedError",
                            "custom element already defined",
                        ));
                    }
                    if d.ce.defs.values().any(|def| e.strict_eq(def.ctor, ctor)) {
                        return Err(dom_exception(
                            e,
                            "NotSupportedError",
                            "custom element ctor already registered",
                        ));
                    }
                }
                // `observedAttributes` figé au define (spec).
                let mut observed = Vec::new();
                if let Value::Obj(oid) = e.get_prop_value(ctor, "observedAttributes")? {
                    if let Some(ObjKind::Array(elems)) = e.heap.obj(oid).map(|o| &o.kind) {
                        for el in elems.clone() {
                            observed.push(e.to_js_string(el)?.to_lowercase());
                        }
                    }
                }
                {
                    let mut d = self.dom.borrow_mut();
                    d.ce.defs
                        .insert(n.clone(), crate::dom::CeDef { ctor, observed });
                    // `whenDefined` pendantes → résolues `undefined`.
                    if let Some(ps) = d.ce.pending.remove(&n) {
                        for p in ps {
                            e.resolve_promise(p, Value::Undefined);
                        }
                    }
                }
                // Upgrade des éléments attachés déjà présents
                // (ordre document via `by_tag`).
                let targets = {
                    let d = self.dom.borrow();
                    d.by_tag(d.root, &n)
                };
                for node in targets {
                    ce_upgrade_node(e, &self.dom, &self.cache, node)?;
                }
                Ok(Some(Value::Undefined))
            }
            "get" => {
                let n = e.to_js_string(arg(args, 0))?;
                Ok(Some(
                    self.dom
                        .borrow()
                        .ce
                        .defs
                        .get(&n)
                        .map_or(Value::Undefined, |d| d.ctor),
                ))
            }
            "getName" => {
                let ctor = arg(args, 0);
                let d = self.dom.borrow();
                Ok(Some(
                    d.ce.defs
                        .iter()
                        .find(|(_, def)| e.strict_eq(def.ctor, ctor))
                        .map_or(Value::Null, |(n, _)| e.str_val(n.clone())),
                ))
            }
            "whenDefined" => {
                let n = e.to_js_string(arg(args, 0))?;
                let p = e.new_promise();
                if !ce_valid_name(&n) {
                    let err = e.str_val(format!("SyntaxError: invalid custom element name {n:?}"));
                    e.reject_promise(p, err);
                    return Ok(Some(Value::Obj(p)));
                }
                if !self.dom.borrow().ce.defs.contains_key(&n) {
                    self.dom
                        .borrow_mut()
                        .ce
                        .pending
                        .entry(n)
                        .or_default()
                        .push(p);
                    return Ok(Some(Value::Obj(p)));
                }
                e.resolve_promise(p, Value::Undefined);
                Ok(Some(Value::Obj(p)))
            }
            "upgrade" => {
                let root = arg(args, 0);
                let Some(node) = node_for_in(e, root, &self.dom) else {
                    return Err(e.type_error("customElements.upgrade: not a node"));
                };
                ce_upgrade_subtree(e, &self.dom, &self.cache, node)?;
                Ok(Some(Value::Undefined))
            }
            _ => Ok(None),
        }
    }

    /// Méthodes lisibles — les patches de registre lisent
    /// `customElements.define` (motif reddit : `'_patched' in
    /// t.define` — invoke-only le rendait `undefined` →
    /// `TypeError` et le patch mourait ; census shadow DOM).
    /// Le repli moteur expose une fonction re-dispatchée sur
    /// `invoke` ; la redéfinition `t.define = f` passe par le
    /// `set` défaut → own-prop shadow — le patch s'installe
    /// et `n.call(t, …)` retombe sur l'invoke original.
    fn has_method(&self, name: &str) -> bool {
        matches!(
            name,
            "define" | "get" | "getName" | "whenDefined" | "upgrade"
        )
    }
}

/// Définition de `node` (si élément customisable) → `(nom,
/// ctor, observed)` — `None` si non-élément, sans définition,
/// déjà upgradé ou marqué failed.
fn ce_def_of(dom: &DomRef, node: NodeId) -> Option<(String, Value, Vec<String>)> {
    let d = dom.borrow();
    let n = &d.nodes[node as usize];
    if n.kind != crate::dom::NodeKind::Element
        || d.ce.upgraded.contains_key(&node)
        || d.ce.failed.contains(&node)
    {
        return None;
    }
    let def = d.ce.defs.get(&n.tag)?;
    Some((n.tag.clone(), def.ctor, def.observed.clone()))
}

/// Upgrade d'un élément (JS-C68) : proto ← `Ctor.prototype`,
/// constructeur exécuté avec `this` = l'élément (slot
/// `ce_upgrade_slot` — le `super()` d'une classe
/// `extends HTMLElement` le rend), puis réactions
/// `attributeChangedCallback` (observés présents) et
/// `connectedCallback` si attaché — ordre spec. Un ctor qui
/// jette ou rend un autre objet → nœud « failed » (callbacks
/// désarmés, erreur rapportée et non propagée — spec).
fn ce_upgrade_node(e: &mut Engine, dom: &DomRef, cache: &NodeCache, node: NodeId) -> JsResult<()> {
    let Some((name, ctor, observed)) = ce_def_of(dom, node) else {
        return Ok(());
    };
    dom.borrow_mut().ce.upgraded.insert(node, name);
    let el = node_value(e, dom, cache, node);
    // `el.__proto__ = Ctor.prototype` (validé objet à define).
    if let Value::Obj(eid) = el {
        if let Value::Obj(p) = e.get_prop_value(ctor, "prototype")? {
            if let Some(o) = e.heap.obj_mut(eid) {
                o.proto = Value::Obj(p);
            }
        }
    }
    // Constructeur : `call_value_ctor` avec `this` = l'élément —
    // le `super()` d'une classe `extends …HTMLElement` atteint la
    // native `HTMLElement` qui rend l'élément via `ce_upgrade_slot`
    // (retour ignoré par `SuperCall`, `this` reste l'élément) ;
    // `FLD_DONE` posé sur `this` après `super()` prouve la
    // chaîne dérivée (une classe de base ou une `function` ne la
    // pose jamais → « failed », comme la spec qui exige le
    // résultat = l'élément). `new.target` = `ctor`.
    let prev = e.ce_upgrade_slot.replace(el);
    let r = e.call_value_ctor(ctor, el, &[], ctor);
    e.ce_upgrade_slot = prev;
    let ok = match r {
        Ok(v) => {
            let returned_other = v.is_object() && !e.strict_eq(v, el);
            let derived =
                matches!(el, Value::Obj(eid) if e.find_own(eid, crate::interp::FLD_DONE).is_some());
            !returned_other && derived
        }
        Err(_) => false,
    };
    if !ok {
        // Ctor throw, chaîne non-HTMLElement, ou retour étranger
        // → état « failed » honnête (proto conservée, spec).
        dom.borrow_mut().ce.failed.insert(node);
        return Ok(());
    }
    // `attributeChangedCallback` initial : attributs observés
    // présents, dans l'ordre de l'attribut list (spec).
    let attr_hits: Vec<(String, String)> = {
        let d = dom.borrow();
        d.nodes[node as usize]
            .attrs
            .iter()
            .filter(|(k, _)| observed.iter().any(|a| a == k))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    };
    for (k, v) in attr_hits {
        ce_fire(e, el, "attributeChangedCallback", k, None, Some(v))?;
    }
    if dom.borrow().is_attached(node) {
        ce_fire0(e, el, "connectedCallback")?;
    }
    Ok(())
}

/// `cb` du proto de `el` → appelée si callable (props héritées
/// de `Ctor.prototype` — `connectedCallback` & co n'ont pas de
/// repli host).
fn ce_call(e: &mut Engine, el: Value, cb_name: &str, args: &[Value]) -> JsResult<()> {
    let cb = e.get_prop_value(el, cb_name)?;
    if e.is_callable(cb) {
        e.call_value(cb, el, args)?;
    }
    Ok(())
}

/// `connectedCallback`/`disconnectedCallback` (sans arguments).
fn ce_fire0(e: &mut Engine, el: Value, cb: &str) -> JsResult<()> {
    ce_call(e, el, cb, &[])
}

/// `attributeChangedCallback(name, old, new)` — `null` absent.
fn ce_fire(
    e: &mut Engine,
    el: Value,
    cb: &str,
    name: String,
    old: Option<String>,
    new: Option<String>,
) -> JsResult<()> {
    let name = e.str_val(name);
    let old = old.map_or(Value::Null, |v| e.str_val(v));
    let new = new.map_or(Value::Null, |v| e.str_val(v));
    ce_call(e, el, cb, &[name, old, new])
}

/// Upgrade récursif du sous-arbre de `root` (éléments matchant
/// une définition — `customElements.upgrade`/`cloneNode`).
fn ce_upgrade_subtree(
    e: &mut Engine,
    dom: &DomRef,
    cache: &NodeCache,
    root: NodeId,
) -> JsResult<()> {
    ce_upgrade_node(e, dom, cache, root)?;
    let kids = dom.borrow().by_tag(root, "*");
    for n in kids {
        ce_upgrade_node(e, dom, cache, n)?;
    }
    Ok(())
}

impl Engine {
    /// Drain synchrone des réactions custom elements (JS-C68) —
    /// appelé après chaque `Host::invoke`/`Host::set` traité :
    /// les callbacks courent *dans* l'opération (spec) et leurs
    /// propres mutations ré-alimentent la file (boucle).
    ///
    /// # Errors
    /// Propage l'exception d'un callback (même modèle que les
    /// listeners d'événements du moteur).
    pub(crate) fn drain_ce_reactions(&mut self) -> JsResult<()> {
        let Some((dom, cache)) = self.dom_ctx.clone() else {
            return Ok(());
        };
        loop {
            let recs = std::mem::take(&mut dom.borrow_mut().ce_reactions);
            if recs.is_empty() {
                return Ok(());
            }
            for rec in recs {
                match rec {
                    crate::dom::CeReaction::Connected(n) => {
                        let (upgraded, failed) = {
                            let d = dom.borrow();
                            (d.ce.upgraded.contains_key(&n), d.ce.failed.contains(&n))
                        };
                        if failed {
                            continue;
                        }
                        if upgraded {
                            let el = node_value(self, &dom, &cache, n);
                            ce_fire0(self, el, "connectedCallback")?;
                        } else {
                            // Upgrade à l'insertion — `ce_upgrade_node`
                            // tire ctor + attrChanged + connected.
                            ce_upgrade_node(self, &dom, &cache, n)?;
                        }
                    }
                    crate::dom::CeReaction::Disconnected(n) => {
                        let live = {
                            let d = dom.borrow();
                            d.ce.upgraded.contains_key(&n) && !d.ce.failed.contains(&n)
                        };
                        if !live {
                            continue;
                        }
                        let el = node_value(self, &dom, &cache, n);
                        ce_fire0(self, el, "disconnectedCallback")?;
                    }
                    crate::dom::CeReaction::AttrChanged {
                        node,
                        name,
                        old,
                        new,
                    } => {
                        let live = {
                            let d = dom.borrow();
                            d.ce.upgraded.contains_key(&node) && !d.ce.failed.contains(&node)
                        };
                        if !live {
                            continue;
                        }
                        let el = node_value(self, &dom, &cache, node);
                        ce_fire(self, el, "attributeChangedCallback", name, old, new)?;
                    }
                }
            }
        }
    }
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
                let doc_v = e.new_host_object(Rc::new(DocumentHost {
                    dom: Rc::new(RefCell::new(d)),
                    cache: Rc::new(RefCell::new(HashMap::new())),
                    implementation: RefCell::new(None),
                    fonts: RefCell::new(None),
                    domain_override: RefCell::new(None),
                    adopted_sheets: RefCell::new(None),
                }));
                // Spec : le document produit hérite
                // `HTMLDocument.prototype` (comme `document`).
                if let Value::Obj(did) = doc_v {
                    if let Some(Value::Obj(cid)) = e.get_prop(e.global, "HTMLDocument") {
                        if let Some(proto @ Value::Obj(_)) = e.get_prop(cid, "prototype") {
                            if let Some(o) = e.heap.obj_mut(did) {
                                o.proto = proto;
                            }
                        }
                    }
                }
                Ok(Some(doc_v))
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
            // `toString` doit être une fonction réelle (spec
            // `Location.prototype.toString` → `href`) : sinon
            // `ToPrimitive` tombe sur `Object.prototype.toString`
            // → "[object Location]" — et `new URL(".", location)`
            // échouait « Invalid base URL » (motif assets
            // svelte/fastapi/mkdocs). `invoke` gère déjà l'appel
            // méthode ; ici c'est la lecture-propriété.
            "toString" => {
                return Ok(Some(e.native_fn("toString", |e, _t, _a| {
                    Ok(e.str_val(e.hooks.page_url.clone()))
                })));
            }
            _ => return Ok(None),
        };
        Ok(Some(e.str_val(s)))
    }

    fn has_method(&self, name: &str) -> bool {
        // `typeof location.assign === 'function'` — surface IDL.
        // `toString` est servi par `get` (fonction réelle), il
        // n'a pas besoin du repli invoke.
        matches!(name, "assign" | "replace" | "reload")
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
        // Setters de composants (CAP-160 — spec « Location
        // object » : chaque setter navigue vers l'URL avec
        // le composant remplacé ; `hash`/`search`/
        // `pathname` sont le gros des usages — routeurs
        // hash, `location.search=` des flow auth). La
        // résolution/validation finale reste côté session.
        if matches!(
            key,
            "hash" | "search" | "pathname" | "host" | "hostname" | "port" | "protocol"
        ) {
            let val = e.to_js_string(v)?;
            let (proto, host, hostname, port, path, search, hash) = url_parts(&e.hooks.page_url);
            if proto.is_empty() {
                // Pas d'URL absolue → setter inerte
                // (dégradation tolérante).
                return Ok(true);
            }
            let (proto, host, _hn, _po, path, search, hash) = match key {
                "protocol" => {
                    let p = val.trim_end_matches(':');
                    (format!("{p}:"), host, hostname, port, path, search, hash)
                }
                "host" => (proto, val.clone(), hostname, port, path, search, hash),
                "hostname" => {
                    let h = if port.is_empty() {
                        val.clone()
                    } else {
                        format!("{val}:{port}")
                    };
                    (proto, h, hostname, port, path, search, hash)
                }
                "port" => {
                    let h = if val.is_empty() {
                        hostname.clone()
                    } else {
                        format!("{hostname}:{val}")
                    };
                    (proto, h, hostname, port, path, search, hash)
                }
                "pathname" => {
                    let p = if val.starts_with('/') {
                        val.clone()
                    } else {
                        format!("/{val}")
                    };
                    (proto, host, hostname, port, p, search, hash)
                }
                "search" => {
                    let s = if val.is_empty() {
                        String::new()
                    } else if val.starts_with('?') {
                        val.clone()
                    } else {
                        format!("?{val}")
                    };
                    (proto, host, hostname, port, path, s, hash)
                }
                _ => {
                    // `hash` — `""` retire le fragment, sinon
                    // `#` préfixé si absent (spec).
                    let h = if val.is_empty() {
                        String::new()
                    } else if val.starts_with('#') {
                        val.clone()
                    } else {
                        format!("#{val}")
                    };
                    (proto, host, hostname, port, path, search, h)
                }
            };
            let nav_url = format!("{proto}//{host}{path}{search}{hash}");
            if let Some(nav) = &e.hooks.navigate {
                nav(&nav_url);
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
            "assign" => {
                let u = e.to_js_string(arg(args, 0))?;
                if let Some(nav) = &e.hooks.navigate {
                    nav(&u);
                }
                Ok(Some(Value::Undefined))
            }
            "replace" => {
                // `historyHandling: "replace"` (CAP-152) :
                // l'entrée courante est réécrite — `back`
                // saute la page remplacée (flows auth).
                let u = e.to_js_string(arg(args, 0))?;
                if let Some(rep) = &e.hooks.navigate_replace {
                    let rep = rep.clone();
                    rep(u);
                } else if let Some(nav) = &e.hooks.navigate {
                    nav(&u);
                }
                Ok(Some(Value::Undefined))
            }
            "reload" => {
                // Rejeu de la requête de l'entrée (CAP-152) —
                // pas d'entrée nouvelle ; repli historique :
                // `navigate(page_url)` (pousse — déviation
                // documentée du harnais).
                if let Some(rel) = &e.hooks.navigate_reload {
                    let rel = rel.clone();
                    rel();
                } else {
                    let u = e.hooks.page_url.clone();
                    if let Some(nav) = &e.hooks.navigate {
                        nav(&u);
                    }
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
            // `history.state` (CAP-158) — état JSON de
            // l'entrée courante désérialisé (`null` absent
            // ou indésérialisable — borne structured clone).
            "state" => {
                let v = match e.hooks.history_state.clone() {
                    Some(s) => {
                        let sv = e.str_val(&s);
                        crate::json::parse(e, sv).unwrap_or(Value::Null)
                    }
                    None => Value::Null,
                };
                Ok(Some(v))
            }
            // Méthodes en *valeur* : les wraps de navigation
            // (`e.call(history,…)` — turbo github.com) lisent la
            // référence puis l'appellent ; invoke-only les rend
            // invisibles. Le `this` est sans effet — tout passe
            // par les hooks.
            // `scrollRestoration` (CAP-166) — `"auto"`/`"manual"`
            // reflété document-local (les routeurs SPA posent
            // `'manual'` avant de restaurer le scroll eux-mêmes).
            "scrollRestoration" => {
                let s = if e.hooks.scroll_restore_manual {
                    "manual"
                } else {
                    "auto"
                };
                Ok(Some(e.str_val(s)))
            }
            "back" => Ok(Some(e.native_fn("back", history_back))),
            "forward" => Ok(Some(e.native_fn("forward", history_forward))),
            "go" => Ok(Some(e.native_fn("go", history_go))),
            "pushState" => Ok(Some(e.native_fn("pushState", history_push_state))),
            "replaceState" => Ok(Some(e.native_fn("replaceState", history_replace_state))),
            _ => Ok(None),
        }
    }

    fn set(&self, e: &mut Engine, _t: Value, key: &str, v: Value) -> JsResult<bool> {
        if key == "scrollRestoration" {
            // Enum WebIDL : toute autre valeur → TypeError.
            let s = e.to_js_string(v)?;
            match s.as_str() {
                "auto" => e.hooks.scroll_restore_manual = false,
                "manual" => e.hooks.scroll_restore_manual = true,
                _ => {
                    return Err(e.type_error("scrollRestoration: invalid enum value"));
                }
            }
            return Ok(true);
        }
        Ok(false)
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

fn history_push_replace(e: &mut Engine, args: &[Value], replace: bool) -> JsResult<Value> {
    // Navigation same-document — l'URL courante change comme
    // chez un navigateur, sans rechargement. Avec un embedder
    // (`push_state`, CAP-151) : l'intention `(url, replace)`
    // remonte — la session résout, commit l'historique et
    // resynchronise `page_url` via `sync_ctx_url` (l'URL
    // brute n'est jamais stockée : elle pourrait être
    // relative). Sans embedder : comportement harnais —
    // `page_url` remplacée à la brute.
    let u = arg(args, 2);
    // `args[0]` = l'état (CAP-158) — sérialisé JSON pour
    // traverser la frontière session (borne documentée :
    // structured clone hors portée — fonctions, cycles,
    // `Map`/`Set` dégradent en `None` → `history.state`
    // = `null`). Enregistré **même sans URL** : spec,
    // `pushState(state)` crée une entrée à URL inchangée.
    let state_json = match arg(args, 0) {
        Value::Undefined => None,
        v => match crate::json::stringify(e, v) {
            Ok(s) if !matches!(s, Value::Undefined) => e.to_js_string(s).ok(),
            _ => None,
        },
    };
    let url = match u {
        Value::Undefined | Value::Null => None,
        _ => {
            let s = e.to_js_string(u)?;
            if s.is_empty() {
                None
            } else {
                Some(s)
            }
        }
    };
    let Some(url) = url else {
        // Sans URL : nouvelle entrée à même adresse —
        // `""` résout sur la base document côté session.
        // Aucun garde same-origin n'entre en jeu.
        e.hooks.history_state.clone_from(&state_json);
        if let Some(h) = &e.hooks.push_state {
            let h = h.clone();
            h(String::new(), replace, state_json);
        }
        if !replace {
            e.hooks.history_len = e.hooks.history_len.saturating_add(1);
        }
        return Ok(Value::Undefined);
    };
    // Résolution locale contre `page_url` (document
    // URL — même grammaire que XHR) : `location.href`
    // relu dans le même script est déjà absolu. La
    // session canonisera via `sync_ctx_url` au drain.
    let resolved = crate::xhr::resolve_url(e, &url).unwrap_or_else(|| url.clone());
    // Same-origin exigée (spec : `SecurityError` — ici
    // refus silencieux ; la session refuserait de
    // même au drain) : une origine distante ne touche
    // ni le miroir ni l'historique — état inclus
    // (posé après ce garde, jamais sur refus).
    if crate::xhr::cross_origin(e, &resolved) == Some(true) {
        return Ok(Value::Undefined);
    }
    e.hooks.history_state.clone_from(&state_json);
    if let Some(h) = &e.hooks.push_state {
        let h = h.clone();
        // L'URL **brute** remonte : la session résout
        // contre la base effective (`nav_base` —
        // `<base href>` s'applique), que `page_url`
        // ne reflète pas.
        h(url.clone(), replace, state_json);
    }
    // L'URL résolue (contre `page_url`) s'applique
    // localement : `location.href`/`document.URL`
    // relus dans le même script et les fetchs suivants
    // sont déjà absolus. Transitoire sous `<base>`
    // — la session recanonise via `sync_ctx_url` au
    // drain.
    e.set_page_url(&resolved);
    if !replace {
        // `history.length` suit l'entrée poussée —
        // `sync_ctx_url` recalera sur la longueur
        // session au drain.
        e.hooks.history_len = e.hooks.history_len.saturating_add(1);
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
    history_push_replace(e, args, false)
}

fn history_replace_state(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    history_push_replace(e, args, true)
}

/// `window.navigation` — Navigation API (WHATWG). `Navigation`
/// est un `EventTarget` (rail `__et` partagé) dont les
/// déplacements délèguent aux hooks nav/history, et dont les
/// entrées sont des `NavigationHistoryEntry` minimales
/// (url/key/id/index/sameDocument/getState = `history.state`
/// désérialisé). `navigate`/`back`/`forward`/`reload`/
/// `traverseTo` rendent `{committed, finished}` — promesses
/// résolues (le hook est fire-and-forget comme `location.assign`) ;
/// `traverseTo` d'une clé inconnue les rejette. Les événements
/// `navigate`/`navigatesuccess` ne sont pas tirés (borne
/// documentée : pas de notifieuse same-document) — les listeners
/// s'enregistrent et la surface ne jette pas.
struct NavigationHost;

/// `NavigationHistoryEntry` minimal — `key`/`id` opaques stables
/// par entrée (une seule entrée modélisée : `"0"`).
fn nav_history_entry(e: &mut Engine) -> Value {
    let entry = e.new_object();
    let u = e.hooks.page_url.clone();
    let v = e.str_val(u);
    e.set_own(entry, "url", v, true);
    let v = e.str_val("0");
    e.set_own(entry, "key", v, true);
    e.set_own(entry, "id", v, true);
    e.set_own(entry, "index", Value::Num(0.0), true);
    e.set_own(entry, "sameDocument", Value::Bool(true), true);
    let gs = e.native_fn("getState", nav_entry_get_state);
    e.set_own(entry, "getState", gs, true);
    Value::Obj(entry)
}

/// `entry.getState()` — état JSON de l'entrée (`history.state`
/// désérialisé, `undefined` absent — spec : pas d'état posé).
fn nav_entry_get_state(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    let v = match e.hooks.history_state.clone() {
        Some(s) => {
            let sv = e.str_val(&s);
            crate::json::parse(e, sv).unwrap_or(Value::Null)
        }
        None => Value::Undefined,
    };
    Ok(v)
}

/// `NavigationResult` `{committed, finished}` — les deux
/// promesses résolues par l'entrée (`ok=true`) ou rejetées
/// (`ok=false`, raison `InvalidStateError` — clé `traverseTo`
/// inconnue).
fn nav_result(e: &mut Engine, ok: bool) -> JsResult<Value> {
    let entry = nav_history_entry(e);
    let res = e.new_object();
    for k in ["committed", "finished"] {
        let pid = e.new_promise();
        if ok {
            e.resolve_promise(pid, entry);
        } else {
            let m = e.str_val("InvalidStateError: unknown navigation key");
            e.reject_promise(pid, m);
        }
        e.set_own(res, k, Value::Obj(pid), true);
    }
    Ok(Value::Obj(res))
}

impl Host for NavigationHost {
    fn class_name(&self) -> &'static str {
        "Navigation"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn get(&self, e: &mut Engine, _t: Value, key: &str) -> JsResult<Option<Value>> {
        match key {
            "currentEntry" => Ok(Some(nav_history_entry(e))),
            // `canGoBack` = historique conjoint > 1 entrée ;
            // `canGoForward` honnêtement `false` (l'index n'est
            // pas suivi — un `true` spéculatif serait un mensonge).
            "canGoBack" => Ok(Some(Value::Bool(e.hooks.history_len > 1))),
            "canGoForward" => Ok(Some(Value::Bool(false))),
            // Pas de transition en cours — `null` spec.
            "transition" => Ok(Some(Value::Null)),
            // `activation` : `entry` = courante, `from` = `null`
            // (pas de document précédent dans ce modèle),
            // `navigationType` `"push"` (entrée poussée).
            "activation" => {
                let a = e.new_object();
                let entry = nav_history_entry(e);
                e.set_own(a, "entry", entry, true);
                e.set_own(a, "from", Value::Null, true);
                let ty = e.str_val("push");
                e.set_own(a, "navigationType", ty, true);
                Ok(Some(Value::Obj(a)))
            }
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
        match name {
            "entries" => {
                let entry = nav_history_entry(e);
                Ok(Some(e.new_array(vec![entry])))
            }
            // `navigate(url, options?)` — navigation réelle via le
            // hook (la session résout la base comme `location.assign`).
            "navigate" => {
                let u = e.to_js_string(arg(args, 0))?;
                if !u.is_empty() {
                    if let Some(nav) = &e.hooks.navigate {
                        let nav = nav.clone();
                        nav(&u);
                    }
                }
                nav_result(e, true).map(Some)
            }
            "back" => {
                history_nav(e, -1)?;
                nav_result(e, true).map(Some)
            }
            "forward" => {
                history_nav(e, 1)?;
                nav_result(e, true).map(Some)
            }
            "reload" => {
                if let Some(rel) = &e.hooks.navigate_reload {
                    let rel = rel.clone();
                    rel();
                } else {
                    let u = e.hooks.page_url.clone();
                    if let Some(nav) = &e.hooks.navigate {
                        nav(&u);
                    }
                }
                nav_result(e, true).map(Some)
            }
            // Clé inconnue → `InvalidStateError` (seule l'entrée
            // `"0"` existe dans ce modèle).
            "traverseTo" => {
                let ok = e.to_js_string(arg(args, 0))? == "0";
                nav_result(e, ok).map(Some)
            }
            // `updateCurrentEntry({state})` — état sérialisé
            // partagé avec `history.state` (même store).
            "updateCurrentEntry" => {
                let st = e
                    .get_prop_value(arg(args, 0), "state")
                    .unwrap_or(Value::Undefined);
                e.hooks.history_state = match st {
                    Value::Undefined => None,
                    v => match crate::json::stringify(e, v) {
                        Ok(s) if !matches!(s, Value::Undefined) => e.to_js_string(s).ok(),
                        _ => None,
                    },
                };
                Ok(Some(Value::Undefined))
            }
            // Rail `EventTarget` partagé — `navigation.addEventListener`
            // (cloudflare-insights / app-router §B-14).
            "addEventListener" => Ok(Some(et_add_listener(e, t, args)?)),
            "removeEventListener" => Ok(Some(et_remove_listener(e, t, args)?)),
            "dispatchEvent" => Ok(Some(et_dispatch_event(e, t, args)?)),
            _ => Ok(None),
        }
    }

    fn has_method(&self, name: &str) -> bool {
        matches!(
            name,
            "entries"
                | "navigate"
                | "back"
                | "forward"
                | "reload"
                | "traverseTo"
                | "updateCurrentEntry"
                | "addEventListener"
                | "removeEventListener"
                | "dispatchEvent"
        )
    }
}

/// `document.fonts` — `FontFaceSet` minimal. Pas de pipeline de
/// fontes web dans ce moteur : `load()`/`ready` résolvent
/// toujours, `check()` dit vrai — surface honnête pour les pages
/// qui feature-detectent puis chargent une fonte de marque
/// (mesuré : bootstrap `Google Sans` de google.com).
struct FontFaceSetHost;

/// Faces posées par `document.fonts.add()` — prop cachée
/// `__faces` : le host est recréé à chaque accès
/// `document.fonts`, l'état du set vit donc sur l'objet.
/// Identité `strict_eq` (spec setlike).
fn font_faces(e: &Engine, this: Value) -> Vec<Value> {
    let Value::Obj(id) = this else {
        return Vec::new();
    };
    let Some(Value::Obj(arr)) = e.heap.obj(id).and_then(|o| o.get_own("__faces")) else {
        return Vec::new();
    };
    match e.heap.obj(arr).map(|o| &o.kind) {
        Some(ObjKind::Array(elems)) => elems.clone(),
        _ => Vec::new(),
    }
}

impl Host for FontFaceSetHost {
    fn class_name(&self) -> &'static str {
        "FontFaceSet"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn get(&self, e: &mut Engine, this: Value, key: &str) -> JsResult<Option<Value>> {
        match key {
            // Setlike : la taille reflète les `add()` posés —
            // `has`/`delete`/`forEach` suivent la même prop.
            "size" => Ok(Some(Value::Num(font_faces(e, this).len() as f64))),
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
        t: Value,
        name: &str,
        args: &[Value],
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
            // `add(face)` — spec : retourne le set (chaînage),
            // dédup identité comme un `Set`. La face reste
            // connue de `has`/`forEach` (excalidraw vol 222 :
            // `fonts.has(f)` après `add(f)` pilotait le boot —
            // `has` absent cassait la chaîne, `has` toujours
            // faux re-chargeait la fonte à l'infini).
            "add" => {
                let mut faces = font_faces(e, t);
                let face = arg(args, 0);
                if !faces.iter().any(|&f| e.strict_eq(f, face)) {
                    faces.push(face);
                    let arr = e.new_array(faces);
                    if let Value::Obj(id) = t {
                        e.set_own(id, "__faces", arr, false);
                    }
                }
                Ok(Some(t))
            }
            // `has(face)`/`delete(face)` — identité stricte
            // (spec setlike) ; `clear()` vide.
            "has" => {
                let face = arg(args, 0);
                Ok(Some(Value::Bool(
                    font_faces(e, t).iter().any(|&f| e.strict_eq(f, face)),
                )))
            }
            "delete" => {
                let face = arg(args, 0);
                let mut faces = font_faces(e, t);
                let n = faces.len();
                faces.retain(|&f| !e.strict_eq(f, face));
                let removed = faces.len() != n;
                let arr = e.new_array(faces);
                if let Value::Obj(id) = t {
                    e.set_own(id, "__faces", arr, false);
                }
                Ok(Some(Value::Bool(removed)))
            }
            "clear" => {
                let arr = e.new_array(Vec::new());
                if let Value::Obj(id) = t {
                    e.set_own(id, "__faces", arr, false);
                }
                Ok(Some(Value::Undefined))
            }
            // `forEach(cb, thisArg?)` — setlike : `cb(v, v, set)`.
            "forEach" => {
                let cb = arg(args, 0);
                let this_arg = args.get(1).copied().unwrap_or(Value::Undefined);
                for f in font_faces(e, t) {
                    e.call_value(cb, this_arg, &[f, f, t])?;
                }
                Ok(Some(Value::Undefined))
            }
            "check" => Ok(Some(Value::Bool(true))),
            // Spec : `FontFaceSet` est un `EventTarget`
            // (loading/loadingdone/loadingerror) — impots DSFR
            // vol 115 enregistre `fonts.addEventListener`. Rail
            // `__et` partagé (map paresseuse).
            "addEventListener" => Ok(Some(et_add_listener(e, t, args)?)),
            "removeEventListener" => Ok(Some(et_remove_listener(e, t, args)?)),
            "dispatchEvent" => Ok(Some(et_dispatch_event(e, t, args)?)),
            _ => Ok(None),
        }
    }

    fn has_method(&self, name: &str) -> bool {
        matches!(
            name,
            "load"
                | "check"
                | "add"
                | "has"
                | "delete"
                | "clear"
                | "forEach"
                | "addEventListener"
                | "removeEventListener"
                | "dispatchEvent"
        )
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
    let Some((dom, _)) = e.dom_ctx.clone() else {
        return Err(e.type_error("getComputedStyle: no document"));
    };
    let Some(node) = node_for_in(e, arg(args, 0), &dom) else {
        return Err(e.type_error("getComputedStyle: parameter 1 is not of type 'Element'"));
    };
    Ok(e.new_host_object(Rc::new(StyleHost {
        dom,
        node,
        computed: true,
    })))
}

/// `window.matchMedia(q)` — pas de moteur de media queries :
/// `MediaQueryList` honnête `matches:false`, `media` = la requête,
/// listeners no-op (viewport fixe, jamais de `change`).
fn win_match_media(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let q = e.to_js_string(arg(args, 0))?;
    let m = e.new_object();
    // CAP-161 — évaluation réelle (bornée) contre l'env de
    // session : `prefers-color-scheme`, `min/max-width`,
    // `orientation`, `hover`, `pointer`… voir `mediaq`.
    e.set_own(
        m,
        "matches",
        Value::Bool(crate::mediaq::eval_media_query(
            &q,
            e.hooks.media_env,
            e.hooks.viewport,
        )),
        true,
    );
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
/// Host `DOMParser` (JS-C91) — `parseFromString` via le
/// parseur HTML du miroir (`Dom::from_html` — tolérant tag
/// soup). Seule la famille `text/html` est parseée ; les
/// autres MIME rendent un document vide (déviation — pas de
/// parseur XML).
struct DomParserHost;

impl Host for DomParserHost {
    fn class_name(&self) -> &'static str {
        "DOMParser"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn has_method(&self, name: &str) -> bool {
        matches!(name, "parseFromString")
    }

    fn invoke(
        &self,
        e: &mut Engine,
        _t: Value,
        name: &str,
        args: &[Value],
    ) -> JsResult<Option<Value>> {
        if name == "parseFromString" {
            let src = e.to_js_string(arg(args, 0))?;
            let mime = e.to_js_string(arg(args, 1))?;
            // Spec : `text/html`/`application/xhtml+xml` →
            // document HTML ; les autres MIME retournent un
            // document d'erreur `<parsererror>` — ici un
            // document vide (déviation honnête).
            let body = if mime.contains("html") {
                src
            } else {
                String::new()
            };
            let domref: DomRef = Rc::new(RefCell::new(crate::dom::Dom::from_html(&body)));
            return Ok(Some(document_value(e, &domref)));
        }
        Ok(None)
    }
}

fn dom_parser_ctor(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    Ok(e.new_host_object(Rc::new(DomParserHost)))
}

fn illegal_ctor(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    Err(e.type_error("illegal constructor"))
}

/// `HTMLElement` — inconstructible directement (« illegal
/// constructor » spec) **sauf** quand `super()` d'une classe
/// custom element l'atteint pendant un upgrade : le slot
/// `ce_upgrade_slot` porte alors l'élément en cours de
/// construction, rendu comme résultat (rôle du mécanisme spec
/// où `super()` produit l'élément — JS-C68).
fn html_element_ctor(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    if let Some(el) = e.ce_upgrade_slot {
        return Ok(el);
    }
    Err(e.type_error("Illegal constructor"))
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

/// `new XMLSerializer()` (CAP-178) — objet à une méthode
/// `serializeToString(node)` → rail `outer_html` du miroir
/// (éléments et `document` acceptés via `node_for` ; borné :
/// sérialisation HTML, pas de mode XML — le miroir n'a pas
/// de namespaces).
fn xmlserializer_ctor(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    let o = e.new_object();
    let f = e.native_fn("serializeToString", xmlserialize_to_string);
    e.set_own(o, "serializeToString", f, true);
    Ok(Value::Obj(o))
}

/// `serializer.serializeToString(node)` — `outer_html` du
/// nœud (ou du document entier, résolu `#document`).
fn xmlserialize_to_string(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let Some((dom, _cache)) = e.dom_ctx.clone() else {
        return Err(e.type_error("serializeToString: no document"));
    };
    let Some(n) = node_for_in(e, arg(args, 0), &dom) else {
        return Err(e.type_error("serializeToString: nœud requis"));
    };
    let s = dom.borrow().outer_html(n);
    Ok(e.str_val(s))
}

/// `new Audio(src?)`/`new Video()` (CAP-178) — alias
/// `createElement` des constructeurs historiques ; `src` pose
/// l'attribut `src` (`new Audio(url)` charge la source).
fn media_el_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let Some((dom, cache)) = e.dom_ctx.clone() else {
        return Err(e.type_error("Audio/Video: no document"));
    };
    let tag = match args.first() {
        Some(Value::Str(s)) => e.heap.string(*s).unwrap_or("audio".to_string()),
        _ => "audio".to_string(),
    };
    let n = dom.borrow_mut().create_element(&tag);
    if let Some(v) = args.get(1).filter(|v| !matches!(v, Value::Undefined)) {
        let s = e.to_js_string(*v)?;
        dom.borrow_mut().set_attr(n, "src", &s);
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
            "detail",
            "message",
            "error",
            "filename",
            "lineno",
            "colno",
            "promise",
            "reason",
            "key",
            "code",
            "keyCode",
            "which",
            "charCode",
            "ctrlKey",
            "shiftKey",
            "altKey",
            "metaKey",
            // JS-C84 — init d'événements spécialisés copiés tels
            // quels : `WheelEvent.delta*`, `PointerEvent.*`,
            // `InputEvent`, `TouchEvent`, `UIEvent.view`,
            // `FocusEvent.relatedTarget`.
            "deltaX",
            "deltaY",
            "deltaZ",
            "deltaMode",
            "pointerType",
            "pointerId",
            "isPrimary",
            "width",
            "height",
            "pressure",
            "tiltX",
            "tiltY",
            "twist",
            "tangentialPressure",
            "inputType",
            "isComposing",
            "data",
            "dataTransfer",
            "view",
            "relatedTarget",
            "touches",
            "targetTouches",
            "changedTouches",
            // CAP-178 — `MessageEvent`/`StorageEvent`/
            // `CloseEvent` : payloads d'init copiés tels
            // quels (mesuré `_census_calls` — les libs
            // les construisent pour le dispatch manuel).
            "origin",
            "source",
            "ports",
            "lastEventId",
            "key",
            "oldValue",
            "newValue",
            "url",
            "storageArea",
            "wasClean",
            "code",
            "reason",
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
        ("isTrusted", Value::Bool(false)),
        ("defaultPrevented", Value::Bool(false)),
        ("cancelBubble", Value::Bool(false)),
        ("returnValue", Value::Bool(true)),
        ("eventPhase", Value::Num(0.0)),
        ("timeStamp", Value::Num(0.0)),
    ] {
        e.set_prop_value(this, k, v)?;
    }
    // CAP-198 — `relatedTarget` est une clé d'init légitime
    // (`FocusEventInit`/`MouseEventInit`, copiée plus haut) :
    // le défaut `null` ne s'applique que si l'init ne l'a pas
    // fourni (l'écriture inconditionnelle l'écrasait avant).
    let rt_set = matches!(this, Value::Obj(id) if e.find_own(id, "relatedTarget").is_some());
    if !rt_set {
        e.set_prop_value(this, "relatedTarget", Value::Null)?;
    }
    Ok(Value::Undefined)
}

/// `MouseEvent`/`PointerEvent`/`WheelEvent`/`DragEvent` :
/// `event_ctor` + défauts `MouseEventInit` — `button`/`buttons`
/// = 0 et toutes les coordonnées = 0, modifieurs = `false`.
/// Sans eux `ev.button === 0` rate (`undefined`), ce qui tue le
/// garde-fou des routers JS (React Router ignore le clic).
fn mouse_event_ctor(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    event_ctor(e, this, args)?;
    let init = arg(args, 1);
    for k in [
        "button",
        "buttons",
        "clientX",
        "clientY",
        "screenX",
        "screenY",
        "pageX",
        "pageY",
        "offsetX",
        "offsetY",
        "movementX",
        "movementY",
        "x",
        "y",
        "detail",
        "layerX",
        "layerY",
    ] {
        let v = if matches!(init, Value::Obj(_)) {
            e.get_prop_value(init, k)?
        } else {
            Value::Undefined
        };
        let n = if matches!(v, Value::Undefined) {
            0.0
        } else {
            match v {
                Value::Num(n) => n,
                other => {
                    let s = e.to_js_string(other)?;
                    s.trim().parse().unwrap_or(0.0)
                }
            }
        };
        e.set_prop_value(this, k, Value::Num(n))?;
    }
    for k in ["ctrlKey", "shiftKey", "altKey", "metaKey"] {
        if !matches!(e.get_prop_value(this, k), Ok(v) if !matches!(v, Value::Undefined)) {
            e.set_prop_value(this, k, Value::Bool(false))?;
        }
    }
    if !matches!(e.get_prop_value(this, "view"), Ok(v) if !matches!(v, Value::Undefined)) {
        e.set_prop_value(this, "view", Value::Null)?;
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

/// `ev.composedPath()` — `[]` hors dispatch, sinon le chemin
/// complet stocké à l'entrée de `propagate_event` (CAP-198 :
/// racines shadow traversées incluses pour `composed`, frontière
/// respectée sinon) — miroir de l'arm `invoke` d'`EventHost`.
fn event_composed_path(e: &mut Engine, t: Value, _a: &[Value]) -> JsResult<Value> {
    let st = match t {
        Value::Obj(id) => e.host_of(id).and_then(|h| {
            h.as_any()
                .downcast_ref::<EventHost>()
                .map(|ev| (ev.state.clone(), ev.dom.clone(), ev.cache.clone()))
        }),
        _ => None,
    };
    let Some((state, dom, cache)) = st else {
        return Ok(e.new_array(Vec::new()));
    };
    if state.current.get().is_none() {
        return Ok(e.new_array(Vec::new()));
    }
    let vals: Vec<Value> = state
        .path
        .borrow()
        .iter()
        .map(|&n| node_value(e, &dom, &cache, n))
        .collect();
    Ok(e.new_array(vals))
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

/// `el.querySelector(sel)`/`querySelectorAll` en natives
/// `Element.prototype` (JS-C162) — Bootstrap fait
/// `Element.prototype.querySelectorAll.call(el, s)`
/// (`SelectorEngine` `find`, mesuré OSM/ameli). Receveur
/// non-Element → `TypeError` (spec).
fn element_query_selector(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let sel = e.to_js_string(arg(args, 0))?;
    let Some((node, dom, cache)) = element_receiver(e, this) else {
        return Err(e.type_error("querySelector: receiver is not an Element"));
    };
    let ids = dom.borrow().query(node, &sel);
    Ok(match ids.first() {
        Some(&n) => node_value(e, &dom, &cache, n),
        None => Value::Null,
    })
}

/// `el.querySelectorAll(sel)` — même motif que
/// [`element_query_selector`] ; collection snapshot.
fn element_query_selector_all(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let sel = e.to_js_string(arg(args, 0))?;
    let Some((node, dom, cache)) = element_receiver(e, this) else {
        return Err(e.type_error("querySelectorAll: receiver is not an Element"));
    };
    let ids = dom.borrow().query(node, &sel);
    Ok(dom_collection(e, &dom, &cache, &ids, false, "NodeList"))
}

/// `el.getElementsByTagName(tag)` en native
/// `Element.prototype` (JS-C162) — collection snapshot
/// (vivacité non implémentée, dégradation honnête).
fn element_get_by_tag(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let tag = e.to_js_string(arg(args, 0))?;
    let Some((node, dom, cache)) = element_receiver(e, this) else {
        return Err(e.type_error("getElementsByTagName: receiver is not an Element"));
    };
    let ids = dom.borrow().by_tag(node, &tag);
    Ok(dom_collection(
        e,
        &dom,
        &cache,
        &ids,
        false,
        "HTMLCollection",
    ))
}

/// `el.getElementsByClassName(cls)` en native
/// `Element.prototype` (JS-C162) — même motif.
fn element_get_by_class(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let cls = e.to_js_string(arg(args, 0))?;
    let Some((node, dom, cache)) = element_receiver(e, this) else {
        return Err(e.type_error("getElementsByClassName: receiver is not an Element"));
    };
    let ids = dom.borrow().by_class(node, &cls);
    Ok(dom_collection(
        e,
        &dom,
        &cache,
        &ids,
        false,
        "HTMLCollection",
    ))
}

/// `el.scrollIntoView(arg?)` — no-op honnête (JS-C74) : le
/// miroir n'a pas de scrolling réel, mais la méthode existe
/// (feature-detect + scripts l'appelant librement — jamais
/// d'exception ; `TypeError` sur receveur non-Element, spec).
fn element_scroll_into_view(e: &mut Engine, this: Value, _args: &[Value]) -> JsResult<Value> {
    if element_receiver(e, this).is_none() {
        return Err(e.type_error("scrollIntoView: receiver is not an Element"));
    }
    Ok(Value::Undefined)
}

/// `el.scroll(x, y)`/`scrollTo`/`scrollBy` — no-op honnête
/// (les éléments ne scrollent pas dans le miroir — même
/// rail que `scrollIntoView`). Receveur non-Element →
/// `TypeError` spec ; coords/options acceptés et ignorés.
fn element_scroll_noop(e: &mut Engine, this: Value, _args: &[Value]) -> JsResult<Value> {
    if element_receiver(e, this).is_none() {
        return Err(e.type_error("scroll: receiver is not an Element"));
    }
    Ok(Value::Undefined)
}

/// `el.getBoundingClientRect()` en native `Element.prototype`
/// (issue #19) — référence lisible pour les mesures via
/// `.call(el)` (polyfills, utils de géométrie). Receveur
/// non-Element → `TypeError` (spec).
fn element_gbrect(e: &mut Engine, this: Value, _args: &[Value]) -> JsResult<Value> {
    let Some((node, dom, _)) = element_receiver(e, this) else {
        return Err(e.type_error("getBoundingClientRect: receiver is not an Element"));
    };
    Ok(gbrect_shared(e, &dom, node))
}

/// `el.attachShadow(init)` — stub spec-minimal (issue #25,
/// PAS le vrai Shadow DOM) : attache un `DocumentFragment`
/// rôle shadowRoot et le retourne — suffit aux polyfills
/// d'instrumentation (Invoker Commands : `n.call` sur la
/// méthode absente tuait le module github-elements).
/// `mode:"closed"` → `el.shadowRoot` null, racine réelle
/// quand même ; re-appel : racine existante retournée
/// (tolérant — spec : `NotSupportedError`).
fn element_attach_shadow(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Some((node, dom, cache)) = element_receiver(e, this) else {
        return Err(e.type_error("attachShadow: receiver is not an Element"));
    };
    let open = match args.first() {
        Some(Value::Obj(id)) => {
            e.get_prop(*id, "mode")
                .and_then(|m| e.to_js_string(m).ok())
                .as_deref()
                != Some("closed")
        }
        _ => true,
    };
    let existing = dom.borrow().shadow_root(node, true);
    let root = if let Some(r) = existing {
        r
    } else {
        let r = dom.borrow_mut().create_fragment();
        dom.borrow_mut().attach_shadow_root(node, r, open);
        r
    };
    Ok(node_value(e, &dom, &cache, root))
}

/// `el.attachInternals()` — stub (issue #25) : objet
/// minimal exposant `shadowRoot` (même `closed`, spec
/// `ElementInternals`).
fn element_attach_internals(e: &mut Engine, this: Value, _args: &[Value]) -> JsResult<Value> {
    let Some((node, dom, cache)) = element_receiver(e, this) else {
        return Err(e.type_error("attachInternals: receiver is not an Element"));
    };
    Ok(e.new_host_object(Rc::new(ElementInternalsHost { dom, cache, node })))
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

/// `new AbortController()` — `{signal}` = vrai
/// `AbortSignal` (CAP-194) ; `abort()` bascule
/// `aborted`/`reason` et tire `onabort` + listeners.
fn abort_controller_ctor(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let sig = new_signal(e);
    e.set_prop_value(this, "signal", sig)?;
    Ok(Value::Undefined)
}

/// `controller.abort(reason?)` — `signal.aborted = true`,
/// `reason` posée (défaut `AbortError`), `onabort` +
/// listeners `abort` tirés dans l'ordre.
fn abort_signal_abort(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let sig = e.get_prop_value(this, "signal")?;
    signal_fire(e, sig, arg(args, 0))
}

/// `AbortSignal.prototype` (résolution dynamique —
/// `ctor` a posé `prototype` sur la fonction globale).
fn signal_proto_of(e: &mut Engine) -> Value {
    e.get_prop(e.global, "AbortSignal")
        .and_then(|c| match c {
            Value::Obj(cid) => e.get_prop(cid, "prototype"),
            _ => None,
        })
        .unwrap_or(Value::Obj(e.protos.object))
}

/// Signal neuf : `aborted:false`, `reason:undefined`,
/// `onabort:null`, `__ls` = listeners `abort` en file.
fn new_signal(e: &mut Engine) -> Value {
    let id = e.new_object();
    e.set_own(id, "aborted", Value::Bool(false), true);
    e.set_own(id, "reason", Value::Undefined, true);
    e.set_own(id, "onabort", Value::Null, true);
    // `__ls` = objet plain `{length,0,1…}` — les éléments
    // des `Array` vivent dans un stockage séparé des props.
    let ls = e.new_object();
    e.set_own(ls, "length", Value::Num(0.0), false);
    e.set_own(id, "__ls", Value::Obj(ls), false);
    if let Value::Obj(proto) = signal_proto_of(e) {
        if let Some(o) = e.heap.obj_mut(id) {
            o.proto = Value::Obj(proto);
        }
    }
    Value::Obj(id)
}

/// `AbortError` borné (pas de `DOMException` matérialisée —
/// `error_value` ordinaire nommé comme spec).
fn abort_reason(e: &mut Engine, name: &str) -> Value {
    e.error_value(e.protos.error, name, "signal is aborted without reason")
}

/// `signal.aborted = true` + `reason` + `onabort` +
/// listeners `abort` (spec « signal abort » : chaque
/// listener appelé avec `this` = le signal).
fn signal_fire(e: &mut Engine, sig: Value, reason: Value) -> JsResult<Value> {
    if let Value::Obj(sid) = sig {
        if matches!(e.get_prop(sid, "aborted"), Some(Value::Bool(true))) {
            return Ok(Value::Undefined);
        }
        let r = if matches!(reason, Value::Undefined) {
            abort_reason(e, "AbortError")
        } else {
            reason
        };
        e.set_prop_value(sig, "aborted", Value::Bool(true))?;
        e.set_prop_value(sig, "reason", r)?;
        let cb = e.get_prop_value(sig, "onabort")?;
        if e.is_callable(cb) {
            e.call_value(cb, sig, &[])?;
        }
        let ls = match e.get_prop(sid, "__ls") {
            Some(Value::Obj(l)) => {
                let len = match e.get_prop(l, "length") {
                    Some(Value::Num(n)) => n as usize,
                    _ => 0,
                };
                (0..len)
                    .filter_map(|i| e.get_prop(l, &i.to_string()))
                    .collect::<Vec<_>>()
            }
            _ => Vec::new(),
        };
        for cb in ls {
            if e.is_callable(cb) {
                e.call_value(cb, sig, &[])?;
            }
        }
    }
    Ok(Value::Undefined)
}

/// `signal.throwIfAborted()` — jette `reason` si aborted.
fn signal_throw_if_aborted(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    if let Value::Obj(sid) = this {
        if matches!(e.get_prop(sid, "aborted"), Some(Value::Bool(true))) {
            let r = e.get_prop(sid, "reason").unwrap_or(Value::Undefined);
            return Err(crate::heap::Thrown(r));
        }
    }
    Ok(Value::Undefined)
}

/// `AbortSignal.throwIfAborted(sig)` — statique spec :
/// même sémantique que la méthode, le signal en argument.
fn signal_static_throw_if_aborted(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(sid) = arg(args, 0) else {
        return Err(e.type_error("AbortSignal.throwIfAborted: not an AbortSignal"));
    };
    if matches!(e.get_prop(sid, "aborted"), Some(Value::Bool(true))) {
        let r = e.get_prop(sid, "reason").unwrap_or(Value::Undefined);
        return Err(crate::heap::Thrown(r));
    }
    Ok(Value::Undefined)
}

/// `signal.addEventListener('abort', cb)` — file `__ls`.
fn signal_add_listener(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(sid) = this else {
        return Ok(Value::Undefined);
    };
    if e.to_js_string(arg(args, 0))? != "abort" {
        return Ok(Value::Undefined);
    }
    if let Some(Value::Obj(l)) = e.get_prop(sid, "__ls") {
        let cb = arg(args, 1);
        let len = match e.get_prop(l, "length") {
            Some(Value::Num(n)) => n as usize,
            _ => 0,
        };
        e.set_own(l, &len.to_string(), cb, true);
        e.set_own(l, "length", Value::Num((len + 1) as f64), true);
    }
    Ok(Value::Undefined)
}

/// `new EventTarget()` — objet ordinaire + map cachée `__et`
/// (`type` → `[cb]`, rail `__ls` d'`AbortSignal`). Spec : le ctor
/// est légal (mesuré css-tricks : core-js meurt au detect sinon).
fn event_target_ctor(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    // `this` porte déjà `EventTarget.prototype` (construct_nt) —
    // écrire `__et` dessus rend `et instanceof EventTarget` vrai.
    if let Value::Obj(oid) = this {
        let mid = e.new_object();
        e.set_own(oid, "__et", Value::Obj(mid), true);
    }
    Ok(Value::Undefined)
}

/// `et.addEventListener(type, cb)` — pousse sur `__et[type]`.
/// La map `__et` est créée paresseusement : les wrappers non-
/// `EventTarget` qui exposent la méthode (`IDBRequest` — JS-C187)
/// et les greffes `EventTarget.prototype.addEventListener.call(o)`
/// des polyfills fonctionnent sans ctor préalable.
pub(crate) fn et_add_listener(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(sid) = this else {
        return Ok(Value::Undefined);
    };
    let ty = e.to_js_string(arg(args, 0))?;
    let cb = arg(args, 1);
    if !e.is_callable(cb) {
        return Ok(Value::Undefined); // spec : non-appelable ignoré
    }
    if e.get_prop(sid, "__et").is_none() {
        let mid = e.new_object();
        e.set_own(sid, "__et", Value::Obj(mid), true);
    }
    if let Some(Value::Obj(map)) = e.get_prop(sid, "__et") {
        let list = e.get_prop(map, &ty).unwrap_or(Value::Undefined);
        if let Value::Obj(l) = list {
            let len = match e.get_prop(l, "length") {
                Some(Value::Num(n)) => n as usize,
                _ => 0,
            };
            e.set_own(l, &len.to_string(), cb, true);
            e.set_own(l, "length", Value::Num((len + 1) as f64), true);
        } else {
            let l = e.new_array(vec![cb]);
            e.set_own(map, &ty, l, true);
        }
    }
    Ok(Value::Undefined)
}

/// `et.removeEventListener(type, cb)` — retire de `__et[type]`.
pub(crate) fn et_remove_listener(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(sid) = this else {
        return Ok(Value::Undefined);
    };
    let ty = e.to_js_string(arg(args, 0))?;
    let cb = arg(args, 1);
    if let Some(Value::Obj(map)) = e.get_prop(sid, "__et") {
        if let Some(Value::Obj(l)) = e.get_prop(map, &ty) {
            let len = match e.get_prop(l, "length") {
                Some(Value::Num(n)) => n as usize,
                _ => 0,
            };
            // Reconstruit sans la cb — écrire `undefined` aux
            // indices resterait sur les props, `elems` du
            // tableau garderait la cb (chemins disjoints).
            let mut keep = Vec::new();
            for i in 0..len {
                if let Some(v) = e.get_prop(l, &i.to_string()) {
                    if v != cb {
                        keep.push(v);
                    }
                }
            }
            let l2 = e.new_array(keep);
            e.set_own(map, &ty, l2, true);
        }
    }
    Ok(Value::Undefined)
}

/// `et.dispatchEvent(ev)` — appelle chaque cb de `__et[ev.type]`
/// (`target`/`currentTarget` = l'objet), `false` si
/// `preventDefault` fut tiré.
pub(crate) fn et_dispatch_event(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(sid) = this else {
        return Err(e.type_error("dispatchEvent: cible invalide"));
    };
    let ev = arg(args, 0);
    let Value::Obj(evid) = ev else {
        return Err(e.type_error("dispatchEvent: événement requis"));
    };
    let ty = e.get_prop(evid, "type").unwrap_or(Value::Undefined);
    let ty = e.to_js_string(ty)?;
    e.set_own(evid, "target", this, true);
    e.set_own(evid, "currentTarget", this, true);
    if let Some(Value::Obj(map)) = e.get_prop(sid, "__et") {
        if let Some(Value::Obj(l)) = e.get_prop(map, &ty) {
            let len = match e.get_prop(l, "length") {
                Some(Value::Num(n)) => n as usize,
                _ => 0,
            };
            for i in 0..len {
                if let Some(cb) = e.get_prop(l, &i.to_string()) {
                    if cb != Value::Undefined && e.is_callable(cb) {
                        e.call_value(cb, this, &[ev])?;
                    }
                }
            }
        }
    }
    let dp = e
        .get_prop(evid, "defaultPrevented")
        .unwrap_or(Value::Bool(false));
    Ok(Value::Bool(!e.to_boolean(dp)))
}

/// `new DOMException(message?, name?)` — `{name, message}`
/// (spec — core-js/css-tricks feature-detect).
fn dom_exception_ctor(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let message = e.to_js_string(arg(args, 0))?;
    let name = if args.len() > 1 && arg(args, 1) != Value::Undefined {
        e.to_js_string(arg(args, 1))?
    } else {
        "Error".to_string()
    };
    // `this` porte `DOMException.prototype` (construct_nt).
    if let Value::Obj(oid) = this {
        let m = e.str_val(message);
        e.set_own(oid, "message", m, true);
        let n = e.str_val(name);
        e.set_own(oid, "name", n, true);
    }
    Ok(Value::Undefined)
}

/// Mint une `DOMException` nommée (`name`/`message`, proto
/// `DOMException.prototype`) — les erreurs spec de `define` et
/// consorts. Repli `TypeError` si le global manque (harnais
/// sans DOM installé).
fn dom_exception(e: &mut Engine, name: &str, msg: &str) -> crate::heap::Thrown {
    let de = e.get_prop_value(Value::Obj(e.global), "DOMException").ok();
    let proto = de
        .and_then(|f| e.get_prop_value(f, "prototype").ok())
        .and_then(|v| match v {
            Value::Obj(p) => Some(p),
            _ => None,
        })
        .unwrap_or(e.protos.object);
    crate::heap::Thrown(e.error_value(proto, name, msg))
}

/// `signal.removeEventListener('abort', cb)` — retire de `__ls`.
fn signal_remove_listener(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(sid) = this else {
        return Ok(Value::Undefined);
    };
    if e.to_js_string(arg(args, 0))? != "abort" {
        return Ok(Value::Undefined);
    }
    let cb = arg(args, 1);
    if let Some(Value::Obj(l)) = e.get_prop(sid, "__ls") {
        let len = match e.get_prop(l, "length") {
            Some(Value::Num(n)) => n as usize,
            _ => 0,
        };
        for i in 0..len {
            if let Some(v) = e.get_prop(l, &i.to_string()) {
                if v == cb {
                    e.set_own(l, &i.to_string(), Value::Undefined, true);
                }
            }
        }
    }
    Ok(Value::Undefined)
}

/// `AbortSignal.abort(reason?)` — signal pré-aborté.
fn signal_static_abort(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let sig = new_signal(e);
    signal_fire(e, sig, arg(args, 0))?;
    Ok(sig)
}

/// `AbortSignal.timeout(ms)` — abort après `ms` via la
/// file de timers (raison `TimeoutError`).
fn signal_static_timeout(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let sig = new_signal(e);
    let ms = match arg(args, 0) {
        Value::Num(n) => n.max(0.0) as u64,
        _ => 0,
    };
    let cb = e.bound_native("abort", signal_timeout_fire, vec![sig]);
    e.timers.schedule(cb, ms, false, Vec::new());
    Ok(sig)
}

/// Callback différé de `AbortSignal.timeout`.
fn signal_timeout_fire(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let r = abort_reason(e, "TimeoutError");
    signal_fire(e, arg(args, 0), r)
}

/// `AbortSignal.any([s1,s2…])` — aborté dès qu'une source
/// l'est (reason reprise) ; écoute posée sur chaque source.
fn signal_static_any(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let out = new_signal(e);
    for src in e.iter_values(arg(args, 0))? {
        if let Value::Obj(sid) = src {
            if matches!(e.get_prop(sid, "aborted"), Some(Value::Bool(true))) {
                let r = e.get_prop(sid, "reason").unwrap_or(Value::Undefined);
                signal_fire(e, out, r)?;
                return Ok(out);
            }
            if let Some(Value::Obj(l)) = e.get_prop(sid, "__ls") {
                let cb = e.bound_native("abort", signal_any_fire, vec![out]);
                let len = match e.get_prop(l, "length") {
                    Some(Value::Num(n)) => n as usize,
                    _ => 0,
                };
                e.set_own(l, &len.to_string(), cb, true);
                e.set_own(l, "length", Value::Num((len + 1) as f64), true);
            }
        }
    }
    Ok(out)
}

/// Listener de `AbortSignal.any` — la source abortée
/// propage sa `reason` au signal combiné.
fn signal_any_fire(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let r = match this {
        Value::Obj(sid) => e.get_prop(sid, "reason").unwrap_or(Value::Undefined),
        _ => Value::Undefined,
    };
    signal_fire(e, arg(args, 0), r)
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
        // Spec `sendBeacon` : `mode: no-cors` + `credentials:
        // include` (JS-C77 — le beacon part avec les cookies,
        // réponse ignorée).
        mode: crate::xhr::FetchMode::NoCors,
        credentials: crate::xhr::FetchCredentials::Include,
        ctx: e.cur_ctx,
    });
    Ok(Value::Bool(true))
}

/// `navigator.javaEnabled()` — spec : `false` (aucun plugin
/// Java n'existe — la méthode est un vestige lu par les
/// feature-detects de fingerprinting, maif vol 122).
fn nav_java_enabled(_e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    Ok(Value::Bool(false))
}

/// `credentials.create`/`get` (JS-C93) — stub honnête : pas
/// d'authentificateur plateforme → `Promise` rejetée
/// `NotSupportedError` (les feature-detects passent, le flux
/// passkey échoue proprement au lieu de casser sur
/// `undefined`).
fn creds_unsupported(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    let pid = e.new_promise();
    let o = e.new_object();
    let n = e.str_val("NotSupportedError");
    e.set_own(o, "name", n, true);
    let m = e.str_val("credentials: no platform authenticator");
    e.set_own(o, "message", m, true);
    e.reject_promise(pid, Value::Obj(o));
    Ok(Value::Obj(pid))
}

/// `serviceWorker.register` (§B-14 vol 323) — pas de runtime
/// SW : `NotSupportedError` rejetée — le `.catch` du site
/// tourne, jamais de faux succès.
fn sw_register(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    let pid = e.new_promise();
    let o = e.new_object();
    let n = e.str_val("NotSupportedError");
    e.set_own(o, "name", n, true);
    let m = e.str_val("serviceWorker: no SW runtime");
    e.set_own(o, "message", m, true);
    e.reject_promise(pid, Value::Obj(o));
    Ok(Value::Obj(pid))
}

/// `screen.orientation.lock` (§B-14 vol 343) — pas de verrou
/// d'orientation hôte → `NotSupportedError` rejetée.
fn so_lock(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    let pid = e.new_promise();
    let o = e.new_object();
    let n = e.str_val("NotSupportedError");
    e.set_own(o, "name", n, true);
    let m = e.str_val("orientation lock: no host support");
    e.set_own(o, "message", m, true);
    e.reject_promise(pid, Value::Obj(o));
    Ok(Value::Obj(pid))
}

/// `serviceWorker.getRegistration` — spec : `undefined` quand
/// aucune registration — promesse résolue `undefined`.
fn sw_get_registration(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    let pid = e.new_promise();
    e.resolve_promise(pid, Value::Undefined);
    Ok(Value::Obj(pid))
}

/// `serviceWorker.getRegistrations` — `[]` honnête.
fn sw_get_registrations(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    let pid = e.new_promise();
    let arr = e.new_array(Vec::new());
    e.resolve_promise(pid, arr);
    Ok(Value::Obj(pid))
}

/// `credentials.preventSilentAccess` — spec dépréciée :
/// résout `undefined`.
/// Transporteurs `navigator.clipboard` posés par l'embedder
/// (CAP-181) — voir `BrowserHooks::clipboard_*`.
type ClipboardWriteHook = Rc<dyn Fn(String) -> Result<(), String>>;
type ClipboardReadHook = Rc<dyn Fn() -> Option<String>>;

/// `navigator.clipboard.writeText(t)` (CAP-181) — le hook
/// embedder écrit (ou refuse) ; absent → `NotAllowedError`
/// (spec : permission refusée — promesse rejetée, jamais de
/// faux succès — Docs « copier » prend le chemin
/// `document.execCommand` hérité ou refuse proprement).
fn clip_write_text(e: &mut Engine, _t: Value, a: &[Value]) -> JsResult<Value> {
    let pid = e.new_promise();
    let text = e.to_js_string(arg(a, 0))?;
    let Some(hook) = e.hooks.clipboard_write.clone() else {
        let msg = e.str_val("NotAllowedError: clipboard write denied");
        e.reject_promise(pid, msg);
        return Ok(Value::Obj(pid));
    };
    match hook(text) {
        Ok(()) => e.resolve_promise(pid, Value::Undefined),
        Err(m) => {
            let msg = e.str_val(format!("NotAllowedError: {m}"));
            e.reject_promise(pid, msg);
        }
    }
    Ok(Value::Obj(pid))
}

/// `navigator.clipboard.readText()` (CAP-181) — même motif :
/// `Some(texte)` du hook → résolution, `None`/absent →
/// `NotAllowedError`.
fn clip_read_text(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    let pid = e.new_promise();
    let text = e.hooks.clipboard_read.clone().and_then(|hook| hook());
    if let Some(t) = text {
        let v = e.str_val(t);
        e.resolve_promise(pid, v);
    } else {
        let msg = e.str_val("NotAllowedError: clipboard read denied");
        e.reject_promise(pid, msg);
    }
    Ok(Value::Obj(pid))
}

fn creds_psa(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    let pid = e.new_promise();
    e.resolve_promise(pid, Value::Undefined);
    Ok(Value::Obj(pid))
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
    let name = match arg(args, 1) {
        Value::Undefined | Value::Null => String::new(),
        v => e.to_js_string(v)?,
    };
    if let Some(f) = e.hooks.open.clone() {
        f(url, name);
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
        // `None` sur attr absent — `el.dataset.x` lit
        // `undefined` par le repli, et `'x' in dataset` reste
        // honnête (`has_prop` lit `is_some` — `Some(Undefined)`
        // rapportait « présent » même après `delete`).
        Ok(self
            .dom
            .borrow()
            .attr(self.node, &camel_to_data_key(key))
            .map(|v| e.str_val(v)))
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

    /// `delete el.dataset.x` → `removeAttribute('data-x')`
    /// (spec `[[Delete]]` de `DOMStringMap` — sans lui la
    /// suppression tombait sur la prop JS propre et
    /// l'attribut restait : spectrum gardait
    /// `data-status="decorated"` → `div[data-status]` restait
    /// `display:none`).
    fn del(&self, _e: &mut Engine, _t: Value, key: &str) -> JsResult<Option<bool>> {
        self.dom
            .borrow_mut()
            .remove_attr(self.node, &camel_to_data_key(key));
        Ok(Some(true))
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

/// Slots de sélection de champ (CAP-177) — attributs
/// internes sérialisés avec le miroir (même motif que
/// `CUSTOM_VALIDITY_ATTR`, ADR-0014) : `selectionStart`/
/// `selectionEnd`/`selectionDirection` sur `input`/`textarea`.
const SEL_START_ATTR: &str = "data-explorer-sel-s";
const SEL_END_ATTR: &str = "data-explorer-sel-e";
const SEL_DIR_ATTR: &str = "data-explorer-sel-d";

/// `true` si `node` porte la sélection de champ
/// (`input`/`textarea` — spec `HTMLInputElement`/
/// `HTMLTextAreaElement` ; les autres types → `null`).
fn is_field(dom: &DomRef, node: NodeId) -> bool {
    matches!(
        dom.borrow().nodes[node as usize].tag.as_str(),
        "input" | "textarea"
    )
}

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

    fn trace(&self, out: &mut Vec<ObjId>) {
        trace_node_cache(&self.cache, out);
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

// -- `FormData` (JS-C67) -------------------------------------------
//
// Objet à prototype réel (`FormData.prototype`) portant ses paires
// `[[nom, valeur], …]` dans la prop sentinelle [`FD_PAIRS`] — les
// valeurs sont des `Value` quelconques (`File`/`Blob` à JS-C70).
// `new FormData(form, submitter?)` collecte les champs
// « successful » du `<form>` miroir (mêmes règles que
// `form.submit()`) ; `new URLSearchParams(fd)` marche via
// `@@iterator` ; `fetch`/`XHR` encodent un corps `FormData` en
// `application/x-www-form-urlencoded` (déviation documentée — la
// spec exige `multipart/form-data`, structurellement hors scope).

/// Prop cachée des paires `FormData` — tableau `[[k, v], …]`.
pub(crate) const FD_PAIRS: &str = "\u{f131}fdpairs";

/// `FormData.prototype` paresseux — les objets peuvent naître de
/// `res.formData()`/`xhr.send` sans qu'`install_dom` ait tourné.
fn fd_proto(e: &mut Engine) -> ObjId {
    if let Some(p) = e.xhr.fd_proto {
        return p;
    }
    let p = e
        .heap
        .alloc_obj(crate::heap::Object::ordinary(Value::Obj(e.protos.object)));
    e.add_method(p, "append", fd_append);
    e.add_method(p, "set", fd_set);
    e.add_method(p, "delete", fd_delete);
    e.add_method(p, "get", fd_get);
    e.add_method(p, "getAll", fd_get_all);
    e.add_method(p, "has", fd_has);
    e.add_method(p, "forEach", fd_for_each);
    e.add_method(p, "entries", fd_entries);
    e.add_method(p, "keys", fd_keys);
    e.add_method(p, "values", fd_values);
    let it = e.native_fn("[Symbol.iterator]", fd_entries);
    e.set_own(p, crate::interp::SYM_ITERATOR, it, false);
    let tag = e.str_val("FormData");
    e.set_own(p, crate::interp::SYM_TO_STRING_TAG, tag, false);
    e.xhr.fd_proto = Some(p);
    p
}

/// Objet `FormData` portant `pairs` (proto paresseux).
pub(crate) fn formdata_new(e: &mut Engine, pairs: Vec<(String, Value)>) -> Value {
    let p = fd_proto(e);
    let id = e
        .heap
        .alloc_obj(crate::heap::Object::ordinary(Value::Obj(p)));
    fd_write(e, id, pairs);
    Value::Obj(id)
}

/// `v` est-il un `FormData` ? → ses paires `(nom, valeur-texte)`.
pub(crate) fn formdata_pairs_of(e: &mut Engine, v: Value) -> Option<Vec<(String, String)>> {
    let Value::Obj(id) = v else { return None };
    if !matches!(e.get_prop(id, FD_PAIRS), Some(Value::Obj(_))) {
        return None;
    }
    Some(
        fd_pairs(e, id)
            .into_iter()
            .map(|(k, val)| {
                // `File`/`Blob` (JS-C70) — la sérialisation
                // urlencodée porte le nom de fichier (spec
                // multipart `filename=` ; `Blob` nu → "blob").
                let s = match crate::files::file_name_of(e, val) {
                    Some(n) => n,
                    None if crate::files::is_blob(e, val) => "blob".to_string(),
                    None => e.to_js_string(val).unwrap_or_default(),
                };
                (k, s)
            })
            .collect(),
    )
}

/// Paires `(nom, valeur)` du prop `FD_PAIRS` de `id`.
fn fd_pairs(e: &mut Engine, id: ObjId) -> Vec<(String, Value)> {
    let Some(Value::Obj(arr)) = e.get_prop(id, FD_PAIRS) else {
        return Vec::new();
    };
    let n = match e.get_prop(arr, "length") {
        Some(Value::Num(n)) => n.max(0.0) as usize,
        _ => 0,
    };
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        if let Some(Value::Obj(pair)) = e.get_prop(arr, &i.to_string()) {
            let k = e
                .get_prop(pair, "0")
                .and_then(|v| e.to_js_string(v).ok())
                .unwrap_or_default();
            let v = e.get_prop(pair, "1").unwrap_or(Value::Undefined);
            out.push((k, v));
        }
    }
    out
}

/// Réécrit le prop `FD_PAIRS` de `id`.
fn fd_write(e: &mut Engine, id: ObjId, pairs: Vec<(String, Value)>) {
    let vals: Vec<Value> = pairs
        .into_iter()
        .map(|(k, v)| {
            let k = e.str_val(k);
            e.new_array(vec![k, v])
        })
        .collect();
    let arr = e.new_array(vals);
    e.set_own(id, FD_PAIRS, arr, false);
}

/// Receveur `FormData` → `(ObjId, paires)` — `TypeError` sinon.
fn fd_this(e: &mut Engine, this: Value) -> JsResult<(ObjId, Vec<(String, Value)>)> {
    let Value::Obj(id) = this else {
        return Err(e.type_error("FormData method on non-object"));
    };
    if !matches!(e.get_prop(id, FD_PAIRS), Some(Value::Obj(_))) {
        return Err(e.type_error("FormData method on incompatible receiver"));
    }
    Ok((id, fd_pairs(e, id)))
}

/// Installe `FormData` sur le global si absent (depuis
/// `install_dom` — et `install_fetch` pour les contextes sans DOM :
/// `new FormData()` y reste valide, seul l'argument `form` exige le
/// miroir).
pub(crate) fn install_formdata_global(e: &mut Engine) {
    let g = e.global;
    if e.get_prop(g, "FormData").is_some() {
        return;
    }
    let p = fd_proto(e);
    let f = e.native_fn("FormData", formdata_ctor);
    if let Value::Obj(fid) = f {
        e.set_own(fid, "prototype", Value::Obj(p), false);
    }
    e.set_own(p, "constructor", f, false);
    e.set_own(g, "FormData", f, false);
}

/// `new FormData(form?, submitter?)` — `form` = élément `<form>`
/// du miroir : champs « successful » collectés (mêmes règles que
/// `form.submit()`) ; `submitter` = bouton nommé à inclure.
fn formdata_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let mut pairs: Vec<(String, Value)> = Vec::new();
    match arg(args, 0) {
        Value::Undefined | Value::Null => {}
        v => {
            let Some(dom) = e.dom_ctx.as_ref().map(|(d, _)| d.clone()) else {
                return Err(e.type_error("FormData: pas de DOM — élément form indisponible"));
            };
            let Some(n) = node_for_in(e, v, &dom) else {
                return Err(e.type_error("FormData: argument is not a form element"));
            };
            let submitter = node_for_in(e, arg(args, 1), &dom);
            let fields = {
                let d = dom.borrow();
                if d.nodes[n as usize].tag != "form" {
                    drop(d);
                    return Err(e.type_error("FormData: argument is not a form element"));
                }
                collect_fields(&d, n, submitter)
            };
            for (k, val) in fields {
                pairs.push((k, e.str_val(val)));
            }
        }
    }
    Ok(formdata_new(e, pairs))
}

/// `fd.append(name, value, filename?)` — `filename` emballe
/// un `Blob` en `File` nommé (spec ; JS-C70).
fn fd_append(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (id, mut pairs) = fd_this(e, this)?;
    if args.len() < 2 {
        return Err(e.type_error("FormData.append: 2 arguments required"));
    }
    let name = e.to_js_string(arg(args, 0))?;
    let mut val = arg(args, 1);
    let fname = arg(args, 2);
    if !matches!(fname, Value::Undefined) {
        let fn_str = e.to_js_string(fname)?;
        if let Some(f) = crate::files::file_wrap_named(e, val, fn_str) {
            val = f;
        }
    }
    pairs.push((name, val));
    fd_write(e, id, pairs);
    Ok(Value::Undefined)
}

/// `fd.set(name, value)` — remplace toutes les entrées du nom (à la
/// position de la première) ou ajoute en fin.
fn fd_set(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (id, pairs) = fd_this(e, this)?;
    if args.len() < 2 {
        return Err(e.type_error("FormData.set: 2 arguments required"));
    }
    let name = e.to_js_string(arg(args, 0))?;
    let mut val = arg(args, 1);
    let fname = arg(args, 2);
    if !matches!(fname, Value::Undefined) {
        let fn_str = e.to_js_string(fname)?;
        if let Some(f) = crate::files::file_wrap_named(e, val, fn_str) {
            val = f;
        }
    }
    let mut out = Vec::with_capacity(pairs.len());
    let mut inserted = false;
    for (k, v) in pairs {
        if k == name {
            if !inserted {
                out.push((k, val));
                inserted = true;
            }
        } else {
            out.push((k, v));
        }
    }
    if !inserted {
        out.push((name, val));
    }
    fd_write(e, id, out);
    Ok(Value::Undefined)
}

/// `fd.delete(name)` — retire toutes les entrées du nom.
fn fd_delete(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (id, pairs) = fd_this(e, this)?;
    let name = e.to_js_string(arg(args, 0))?;
    fd_write(
        e,
        id,
        pairs.into_iter().filter(|(k, _)| *k != name).collect(),
    );
    Ok(Value::Undefined)
}

/// `fd.get(name)` → première valeur | `null`.
fn fd_get(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (_id, pairs) = fd_this(e, this)?;
    let name = e.to_js_string(arg(args, 0))?;
    Ok(match pairs.into_iter().find(|(k, _)| *k == name) {
        Some((_, v)) => v,
        None => Value::Null,
    })
}

/// `fd.getAll(name)` → toutes les valeurs du nom.
fn fd_get_all(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (_id, pairs) = fd_this(e, this)?;
    let name = e.to_js_string(arg(args, 0))?;
    let vals: Vec<Value> = pairs
        .into_iter()
        .filter(|(k, _)| *k == name)
        .map(|(_, v)| v)
        .collect();
    Ok(e.new_array(vals))
}

/// `fd.has(name)` → bool.
fn fd_has(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (_id, pairs) = fd_this(e, this)?;
    let name = e.to_js_string(arg(args, 0))?;
    Ok(Value::Bool(pairs.iter().any(|(k, _)| *k == name)))
}

/// `fd.forEach(cb, thisArg?)` — `(valeur, nom, fd)`.
fn fd_for_each(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (_id, pairs) = fd_this(e, this)?;
    let cb = arg(args, 0);
    if !e.is_callable(cb) {
        return Err(e.type_error("FormData.forEach needs a function"));
    }
    let this_arg = arg(args, 1);
    for (k, v) in pairs {
        let kk = e.str_val(k);
        e.call_value(cb, this_arg, &[v, kk, this])?;
    }
    Ok(Value::Undefined)
}

/// `fd.entries()`/`@@iterator` — itérateur `[nom, valeur]`.
fn fd_entries(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let (_id, pairs) = fd_this(e, this)?;
    let mut items = Vec::with_capacity(pairs.len());
    for (k, v) in pairs {
        let kk = e.str_val(k);
        items.push(e.new_array(vec![kk, v]));
    }
    let proto = e.protos.usp_iter;
    Ok(crate::collections::iter_obj(e, items, proto))
}

/// `fd.keys()` — itérateur des noms.
fn fd_keys(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let (_id, pairs) = fd_this(e, this)?;
    let items: Vec<Value> = pairs.into_iter().map(|(k, _)| e.str_val(k)).collect();
    let proto = e.protos.usp_iter;
    Ok(crate::collections::iter_obj(e, items, proto))
}

/// `fd.values()` — itérateur des valeurs.
fn fd_values(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let (_id, pairs) = fd_this(e, this)?;
    let items: Vec<Value> = pairs.into_iter().map(|(_, v)| v).collect();
    let proto = e.protos.usp_iter;
    Ok(crate::collections::iter_obj(e, items, proto))
}

// ------------------------------------------------------------------
// JS-C98 — complétude de surface (census) — helpers
// ------------------------------------------------------------------

/// `new Option(text?, value?, defaultSelected?, selected?)` —
/// `<option>` historique : `createElement` + `text`/`value`/
/// `selected` (spec `Option()` — form libs, jQuery).
fn option_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let Some((dom, cache)) = e.dom_ctx.clone() else {
        return Err(e.type_error("Option: no document"));
    };
    let n = dom.borrow_mut().create_element("option");
    if let Some(v) = args.first().filter(|v| !matches!(v, Value::Undefined)) {
        let s = e.to_js_string(*v)?;
        dom.borrow_mut().set_attr(n, "label", &s);
        dom.borrow_mut().set_text(n, &s);
    }
    if let Some(v) = args.get(1).filter(|v| !matches!(v, Value::Undefined)) {
        let s = e.to_js_string(*v)?;
        dom.borrow_mut().set_attr(n, "value", &s);
    }
    // `defaultSelected`/`selected` posent tous deux l'attribut
    // `selected` (la distinction IDL/attr est hors miroir).
    for v in args.iter().skip(2).take(2) {
        if e.to_boolean(*v) {
            dom.borrow_mut().set_attr(n, "selected", "");
            break;
        }
    }
    Ok(node_value(e, &dom, &cache, n))
}

/// `new DOMPoint(x?, y?, z?, w?)` — `{x,y,z,w}` +
/// `matrixTransform(m)` (spec — borné affine 2D, w préservé).
fn dompoint_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let mut nums = [0.0f64; 4];
    for (i, slot) in nums.iter_mut().enumerate() {
        if let Some(v) = args.get(i).filter(|v| !matches!(v, Value::Undefined)) {
            *slot = e.to_number(*v)?;
        }
    }
    let o = e.new_object();
    for (k, v) in [
        ("x", nums[0]),
        ("y", nums[1]),
        ("z", nums[2]),
        (
            "w",
            if nums[3] == 0.0 && args.len() < 4 {
                1.0
            } else {
                nums[3]
            },
        ),
    ] {
        e.set_own(o, k, Value::Num(v), true);
    }
    let mt = e.native_fn("matrixTransform", dompoint_transform);
    e.set_own(o, "matrixTransform", mt, true);
    Ok(Value::Obj(o))
}

/// `point.matrixTransform(m)` — `(a*x + c*y + e, b*x + d*y +
/// f)` ; champs `a`..`f` lus sur l'objet (`DOMMatrix` ou
/// littéral `{a,b,c,d,e,f}` — identité par défaut).
fn dompoint_transform(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(pid) = this else {
        return Err(e.type_error("matrixTransform: this requis"));
    };
    let m = matrix_of(e, arg(args, 0));
    let xv = e.get_prop(pid, "x").unwrap_or(Value::Num(0.0));
    let yv = e.get_prop(pid, "y").unwrap_or(Value::Num(0.0));
    let zv = e.get_prop(pid, "z").unwrap_or(Value::Num(0.0));
    let wv = e.get_prop(pid, "w").unwrap_or(Value::Num(1.0));
    let (x, y, z, w) = (
        e.to_number(xv)?,
        e.to_number(yv)?,
        e.to_number(zv)?,
        e.to_number(wv)?,
    );
    let o = e.new_object();
    for (k, v) in [
        ("x", m[0] * x + m[2] * y + m[4]),
        ("y", m[1] * x + m[3] * y + m[5]),
        ("z", z),
        ("w", w),
    ] {
        e.set_own(o, k, Value::Num(v), true);
    }
    let mt = e.native_fn("matrixTransform", dompoint_transform);
    e.set_own(o, "matrixTransform", mt, true);
    Ok(Value::Obj(o))
}

/// Lit les 6 composantes affines `a`..`f` d'un objet —
/// identité si absent/non-objet.
fn matrix_of(e: &mut Engine, v: Value) -> [f64; 6] {
    let Value::Obj(id) = v else {
        return [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    };
    let mut m = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    for (i, k) in ["a", "b", "c", "d", "e", "f"].iter().enumerate() {
        if let Some(Value::Num(n)) = e.get_prop(id, k) {
            m[i] = n;
        }
    }
    m
}

/// `matrix(a,b,c,d,e,f)` / `matrix3d(m1,…,m16)` / `none` →
/// `[a,b,c,d,e,f]` (identité si non reconnu).
fn matrix_parse_str(s: &str) -> [f64; 6] {
    let t = s.trim();
    if t.eq_ignore_ascii_case("none") {
        return [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    }
    let Some(inner) = t
        .strip_prefix("matrix3d(")
        .or_else(|| t.strip_prefix("matrix("))
        .and_then(|r| r.strip_suffix(')'))
    else {
        return [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    };
    let v: Vec<f64> = inner
        .split(',')
        .filter_map(|p| p.trim().parse::<f64>().ok())
        .collect();
    if v.len() >= 16 {
        [v[0], v[1], v[4], v[5], v[12], v[13]]
    } else if v.len() >= 6 {
        [v[0], v[1], v[2], v[3], v[4], v[5]]
    } else {
        [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]
    }
}

/// `m1 * m2` (composition affine — `multiply` spec).
fn matrix_mul(m1: [f64; 6], m2: [f64; 6]) -> [f64; 6] {
    [
        m1[0] * m2[0] + m1[2] * m2[1],
        m1[1] * m2[0] + m1[3] * m2[1],
        m1[0] * m2[2] + m1[2] * m2[3],
        m1[1] * m2[2] + m1[3] * m2[3],
        m1[0] * m2[4] + m1[2] * m2[5] + m1[4],
        m1[1] * m2[4] + m1[3] * m2[5] + m1[5],
    ]
}

/// `true` si `m` est l'identité (comparaison exacte — la
/// matrice n'est construite identité que par `new DOMMatrix()`
/// ou une composition d'identités).
fn matrix_is_ident(m: [f64; 6]) -> bool {
    m.iter()
        .zip([1.0f64, 0.0, 0.0, 1.0, 0.0, 0.0].iter())
        .all(|(a, b)| a.to_bits() == b.to_bits())
}

/// Inverse affine — `None` si non inversible.
fn matrix_inv(m: [f64; 6]) -> Option<[f64; 6]> {
    let det = m[0] * m[3] - m[1] * m[2];
    if det.abs() < 1e-20 {
        return None;
    }
    Some([
        m[3] / det,
        -m[1] / det,
        -m[2] / det,
        m[0] / det,
        (m[2] * m[5] - m[3] * m[4]) / det,
        (m[1] * m[4] - m[0] * m[5]) / det,
    ])
}

/// Construit l'objet `DOMMatrix` JS à partir de `[a..f]` —
/// champs 2D (`a`..`f`, `is2D`, `isIdentity`) + alias 3D
/// (`m11`/`m12`/`m21`/`m22`/`m41`/`m42`, diagonale unité) +
/// méthodes `translate`/`scale`/`rotate`/`multiply` (copie),
/// `*Self` (mutation), `inverse`, `transformPoint`,
/// `toString`.
fn matrix_obj(e: &mut Engine, m: [f64; 6]) -> Value {
    let o = e.new_object();
    for (k, v) in [
        ("a", m[0]),
        ("b", m[1]),
        ("c", m[2]),
        ("d", m[3]),
        ("e", m[4]),
        ("f", m[5]),
        ("m11", m[0]),
        ("m12", m[1]),
        ("m21", m[2]),
        ("m22", m[3]),
        ("m41", m[4]),
        ("m42", m[5]),
        ("m13", 0.0),
        ("m14", 0.0),
        ("m23", 0.0),
        ("m24", 0.0),
        ("m31", 0.0),
        ("m32", 0.0),
        ("m33", 1.0),
        ("m34", 0.0),
        ("m43", 0.0),
        ("m44", 1.0),
    ] {
        e.set_own(o, k, Value::Num(v), true);
    }
    let ident = matrix_is_ident(m);
    e.set_own(o, "is2D", Value::Bool(true), true);
    e.set_own(o, "isIdentity", Value::Bool(ident), true);
    for (k, f) in [
        ("translate", dommatrix_translate as crate::heap::NativeFn),
        ("scale", dommatrix_scale),
        ("rotate", dommatrix_rotate),
        ("multiply", dommatrix_multiply),
        ("multiplySelf", dommatrix_multiply_self),
        ("translateSelf", dommatrix_translate_self),
        ("scaleSelf", dommatrix_scale_self),
        ("rotateSelf", dommatrix_rotate_self),
        ("invertSelf", dommatrix_invert_self),
        ("inverse", dommatrix_inverse),
        ("transformPoint", dommatrix_transform_point),
        ("toString", dommatrix_to_string),
        ("toJSON", dommatrix_to_json),
    ] {
        let mf = e.native_fn(k, f);
        e.set_own(o, k, mf, true);
    }
    Value::Obj(o)
}

/// `new DOMMatrix(init?)` — `matrix(...)`/`matrix3d(...)`/`none`
/// string, copie d'un objet `{a..f}`, ou identité.
fn dommatrix_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let m = match args.first() {
        Some(Value::Str(s)) => matrix_parse_str(&e.heap.string(*s).unwrap_or_default()),
        Some(v @ Value::Obj(_)) => matrix_of(e, *v),
        _ => [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
    };
    Ok(matrix_obj(e, m))
}

fn dommatrix_translate(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let m = matrix_of(e, this);
    let (x, y) = (e.to_number(arg(args, 0))?, e.to_number(arg(args, 1))?);
    Ok(matrix_obj(e, matrix_mul(m, [1.0, 0.0, 0.0, 1.0, x, y])))
}

fn dommatrix_scale(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let m = matrix_of(e, this);
    let sx = e.to_number(arg(args, 0))?;
    let sy = match args.get(1) {
        Some(v) if !matches!(v, Value::Undefined) => e.to_number(*v)?,
        _ => sx,
    };
    Ok(matrix_obj(e, matrix_mul(m, [sx, 0.0, 0.0, sy, 0.0, 0.0])))
}

fn dommatrix_rotate(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let m = matrix_of(e, this);
    let rad = e.to_number(arg(args, 0))?.to_radians();
    let (c, s) = (rad.cos(), rad.sin());
    Ok(matrix_obj(e, matrix_mul(m, [c, s, -s, c, 0.0, 0.0])))
}

fn dommatrix_multiply(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let m = matrix_mul(matrix_of(e, this), matrix_of(e, arg(args, 0)));
    Ok(matrix_obj(e, m))
}

fn dommatrix_inverse(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let m = matrix_of(e, this);
    Ok(matrix_obj(e, matrix_inv(m).unwrap_or(m)))
}

/// Mutation `this` — les `*Self` réécrivent `a`..`f` + alias
/// `m11`/`m12`/`m21`/`m22`/`m41`/`m42` et `isIdentity`.
fn matrix_write_self(e: &mut Engine, this: Value, m: [f64; 6]) {
    let Value::Obj(id) = this else {
        return;
    };
    for (i, k) in ["a", "b", "c", "d", "e", "f"].iter().enumerate() {
        e.set_own(id, k, Value::Num(m[i]), true);
    }
    for (k, v) in [
        ("m11", m[0]),
        ("m12", m[1]),
        ("m21", m[2]),
        ("m22", m[3]),
        ("m41", m[4]),
        ("m42", m[5]),
    ] {
        e.set_own(id, k, Value::Num(v), true);
    }
    let ident = matrix_is_ident(m);
    e.set_own(id, "isIdentity", Value::Bool(ident), true);
}

fn dommatrix_multiply_self(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let m = matrix_mul(matrix_of(e, this), matrix_of(e, arg(args, 0)));
    matrix_write_self(e, this, m);
    Ok(this)
}

fn dommatrix_translate_self(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (x, y) = (e.to_number(arg(args, 0))?, e.to_number(arg(args, 1))?);
    let m = matrix_mul(matrix_of(e, this), [1.0, 0.0, 0.0, 1.0, x, y]);
    matrix_write_self(e, this, m);
    Ok(this)
}

fn dommatrix_scale_self(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let sx = e.to_number(arg(args, 0))?;
    let sy = match args.get(1) {
        Some(v) if !matches!(v, Value::Undefined) => e.to_number(*v)?,
        _ => sx,
    };
    let m = matrix_mul(matrix_of(e, this), [sx, 0.0, 0.0, sy, 0.0, 0.0]);
    matrix_write_self(e, this, m);
    Ok(this)
}

fn dommatrix_rotate_self(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let rad = e.to_number(arg(args, 0))?.to_radians();
    let (c, s) = (rad.cos(), rad.sin());
    let m = matrix_mul(matrix_of(e, this), [c, s, -s, c, 0.0, 0.0]);
    matrix_write_self(e, this, m);
    Ok(this)
}

fn dommatrix_invert_self(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let m = matrix_of(e, this);
    matrix_write_self(e, this, matrix_inv(m).unwrap_or(m));
    Ok(this)
}

/// Point `{x,y,z,w,matrixTransform}` — partagé `DOMPoint`/
/// `transformPoint`.
fn dompoint_obj(e: &mut Engine, x: f64, y: f64, z: f64, w: f64) -> Value {
    let o = e.new_object();
    for (k, v) in [("x", x), ("y", y), ("z", z), ("w", w)] {
        e.set_own(o, k, Value::Num(v), true);
    }
    let mt = e.native_fn("matrixTransform", dompoint_transform);
    e.set_own(o, "matrixTransform", mt, true);
    Value::Obj(o)
}

/// `m.transformPoint(p)` — point `{x,y,z,w}` transformé par la
/// matrice `this` (affine 2D : `z`/`w` préservés).
fn dommatrix_transform_point(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let m = matrix_of(e, this);
    let (x, y, z, w) = match arg(args, 0) {
        Value::Obj(pid) => {
            let xv = e.get_prop(pid, "x").unwrap_or(Value::Num(0.0));
            let yv = e.get_prop(pid, "y").unwrap_or(Value::Num(0.0));
            let zv = e.get_prop(pid, "z").unwrap_or(Value::Num(0.0));
            let wv = e.get_prop(pid, "w").unwrap_or(Value::Num(1.0));
            (
                e.to_number(xv)?,
                e.to_number(yv)?,
                e.to_number(zv)?,
                e.to_number(wv)?,
            )
        }
        _ => (0.0, 0.0, 0.0, 1.0),
    };
    Ok(dompoint_obj(
        e,
        m[0] * x + m[2] * y + m[4],
        m[1] * x + m[3] * y + m[5],
        z,
        w,
    ))
}

fn dommatrix_to_string(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let m = matrix_of(e, this);
    Ok(e.str_val(format!(
        "matrix({}, {}, {}, {}, {}, {})",
        m[0], m[1], m[2], m[3], m[4], m[5]
    )))
}

fn dommatrix_to_json(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = this else {
        return Ok(Value::Null);
    };
    let o = e.new_object();
    for k in [
        "a",
        "b",
        "c",
        "d",
        "e",
        "f",
        "m11",
        "m12",
        "m13",
        "m14",
        "m21",
        "m22",
        "m23",
        "m24",
        "m31",
        "m32",
        "m33",
        "m34",
        "m41",
        "m42",
        "m43",
        "m44",
        "is2D",
        "isIdentity",
    ] {
        if let Some(v) = e.get_prop(id, k) {
            e.set_own(o, k, v, true);
        }
    }
    Ok(Value::Obj(o))
}

/// `new ImageData(w,h)` / `new ImageData(u8c, w, h?)` —
/// `{width,height,data:Uint8ClampedArray}` (spec — `data` de
/// longueur `w*h*4` ou erreur).
fn imagedata_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let (w, h, data) = match args.first() {
        Some(&v @ Value::Obj(_)) if crate::typed_array::bytes_of_value(e, v).is_some() => {
            let w = e.to_number(arg(args, 1))? as usize;
            let h = match args.get(2) {
                Some(v) if !matches!(v, Value::Undefined) => e.to_number(*v)? as usize,
                _ => {
                    let len = crate::typed_array::bytes_of_value(e, v).map_or(0, |b| b.len());
                    if w == 0 || len % (w * 4) != 0 {
                        return Err(e.type_error("ImageData: data size mismatch"));
                    }
                    len / (w * 4)
                }
            };
            (w, h, v)
        }
        _ => {
            let (w, h) = (
                e.to_number(arg(args, 0))? as usize,
                e.to_number(arg(args, 1))? as usize,
            );
            let buf = crate::typed_array::new_array_buffer(
                e,
                vec![0; w.saturating_mul(h).saturating_mul(4)],
            );
            let Value::Obj(bid) = buf else {
                return Err(e.type_error("ImageData: buffer"));
            };
            (
                w,
                h,
                crate::typed_array::new_ta(e, crate::heap::TaKind::U8C, bid, 0, w * h * 4),
            )
        }
    };
    if w == 0 && h == 0 {
        return Err(e.type_error("ImageData: width/height requis"));
    }
    let o = e.new_object();
    e.set_own(o, "width", Value::Num(w as f64), true);
    e.set_own(o, "height", Value::Num(h as f64), true);
    e.set_own(o, "data", data, true);
    Ok(Value::Obj(o))
}

/// `Path2D` — enregistreur d'ops borné : méthodes spec
/// (`moveTo`/`lineTo`/`arc`/`rect`/`closePath`…) accumulées
/// dans une file interne (stockage seul — le raster la
/// consommera quand les ctx canvas peindront).
struct Path2dHost {
    ops: std::cell::RefCell<Vec<String>>,
}

impl Path2dHost {
    const METHODS: &[&str] = &[
        "addPath",
        "moveTo",
        "lineTo",
        "arc",
        "arcTo",
        "ellipse",
        "rect",
        "roundRect",
        "quadraticCurveTo",
        "bezierCurveTo",
        "closePath",
    ];
}

impl crate::heap::Host for Path2dHost {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn invoke(
        &self,
        e: &mut Engine,
        _this: Value,
        name: &str,
        args: &[Value],
    ) -> JsResult<Option<Value>> {
        if !Self::METHODS.contains(&name) {
            return Ok(None);
        }
        let mut s = String::from(name);
        s.push('(');
        for (i, a) in args.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            if let Ok(n) = e.to_number(*a) {
                s.push_str(&n.to_string());
            }
        }
        s.push(')');
        self.ops.borrow_mut().push(s);
        Ok(Some(Value::Undefined))
    }

    fn has_method(&self, name: &str) -> bool {
        Self::METHODS.contains(&name)
    }

    fn class_name(&self) -> &'static str {
        "Path2D"
    }
}

/// `new Path2D(path?)` — enregistreur (l'argument `path`/
/// string SVG `d` est acceptée et stockée telle quelle).
fn path2d_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let ops = std::cell::RefCell::new(Vec::new());
    if let Some(Value::Str(s)) = args.first() {
        let d = e.heap.string(*s).unwrap_or_default();
        ops.borrow_mut().push(format!("d({d})"));
    }
    Ok(e.new_host_object(Rc::new(Path2dHost { ops })))
}

/// `new OffscreenCanvas(w, h)` — `<canvas>` détaché du miroir
/// dont `getContext('2d')` enregistre (`Ctx2dHost`, JS-C28) ;
/// `transferToImageBitmap` rend `{width,height,close()}`.
fn offscreen_canvas_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let Some((dom, cache)) = e.dom_ctx.clone() else {
        return Err(e.type_error("OffscreenCanvas: no document"));
    };
    let (w, h) = (e.to_number(arg(args, 0))?, e.to_number(arg(args, 1))?);
    let n = dom.borrow_mut().create_element("canvas");
    {
        let mut d = dom.borrow_mut();
        d.set_attr(n, "width", &w.to_string());
        d.set_attr(n, "height", &h.to_string());
    }
    let ctx = e.new_host_object(Rc::new(Ctx2dHost {
        dom: dom.clone(),
        cache: cache.clone(),
        node: n,
    }));
    dom.borrow_mut().set_canvas_ctx(n, ctx);
    let o = e.new_object();
    e.set_own(o, "width", Value::Num(w), true);
    e.set_own(o, "height", Value::Num(h), true);
    let g = e.bound_native("getContext", offcanvas_getctx, vec![ctx]);
    e.set_own(o, "getContext", g, true);
    let tb = e.native_fn("transferToImageBitmap", offcanvas_bitmap);
    e.set_own(o, "transferToImageBitmap", tb, true);
    Ok(Value::Obj(o))
}

/// `offscreen.getContext('2d')` — le `Ctx2dHost` lié (bound
/// `[ctx]`), `null` pour les autres modes.
fn offcanvas_getctx(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let ctx = arg(args, 0);
    let kind = e.to_js_string(arg(args, 1))?;
    Ok(if kind == "2d" { ctx } else { Value::Null })
}

/// `offscreen.transferToImageBitmap()` — `{width,height,close}`
/// (image enregistrée — le raster est hors périmètre).
fn offcanvas_bitmap(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = this else {
        return Err(e.type_error("transferToImageBitmap: this requis"));
    };
    let (w, h) = (
        e.get_prop(id, "width").unwrap_or(Value::Num(0.0)),
        e.get_prop(id, "height").unwrap_or(Value::Num(0.0)),
    );
    let o = e.new_object();
    e.set_own(o, "width", w, true);
    e.set_own(o, "height", h, true);
    let c = e.native_fn("close", win_noop);
    e.set_own(o, "close", c, true);
    Ok(Value::Obj(o))
}

/// `new FontFace(family, source?, descriptors?)` —
/// `{family, status:'unloaded', load() → Promise(self)}`
/// (le fetch de fontes réel est un chantier session).
fn fontface_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let o = e.new_object();
    let family = e.to_js_string(arg(args, 0))?;
    let famv = e.str_val(family);
    e.set_own(o, "family", famv, true);
    let st = e.str_val("unloaded");
    e.set_own(o, "status", st, true);
    let l = e.native_fn("load", fontface_load);
    e.set_own(o, "load", l, true);
    Ok(Value::Obj(o))
}

/// `fontface.load()` — promesse résolue `this` (aucune fonte
/// distante n'est chargée — la promesse existe pour les
/// loaders qui enchaînent `document.fonts.add(f)`).
fn fontface_load(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let id = e.new_promise();
    e.resolve_promise(id, this);
    Ok(Value::Obj(id))
}

/// `createImageBitmap(image, sx?, sy?, sw?, sh?)` — promesse
/// résolue `{width,height,close()}` (l'image réelle n'est
/// pas décodée ici — forme spec honnête).
fn create_image_bitmap(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let w = e.to_number(arg(args, 3)).unwrap_or(0.0);
    let h = e.to_number(arg(args, 4)).unwrap_or(0.0);
    let o = e.new_object();
    e.set_own(o, "width", Value::Num(w), true);
    e.set_own(o, "height", Value::Num(h), true);
    let c = e.native_fn("close", win_noop);
    e.set_own(o, "close", c, true);
    let id = e.new_promise();
    e.resolve_promise(id, Value::Obj(o));
    Ok(Value::Obj(id))
}
/// `window.find(str)` → `false` honnête (jamais de
/// sélection de texte ici — CAP-178).
fn win_find(_e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    Ok(Value::Bool(false))
}

/// `window.scroll*` — no-op honnête : le scroll de la page
/// vit côté session (input utilisateur), aucune intention
/// JS→session n'existe — la méthode est servie pour les
/// lazy-loaders qui la référencent/avancent sans erreur
/// (§B-14 EXPLOR68 iltalehti `scrollTo` absent).
fn win_scroll_noop(_e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    Ok(Value::Undefined)
}

/// No-op window (`stop`/`print`/`close` d'`ImageBitmap`…).
fn win_noop(_e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    Ok(Value::Undefined)
}

/// `show*FilePicker` — `SecurityError` rejeté : le geste
/// utilisateur n'existe jamais ici (même réponse que Chrome
/// hors geste — feature-detect ok, appel honnête).
fn picker_reject(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    let id = e.new_promise();
    let msg = e.str_val("SecurityError: user gesture required");
    e.reject_promise(id, msg);
    Ok(Value::Obj(id))
}

/// `mediaDevices.getUserMedia`/`getDisplayMedia` — rejet
/// `NotFoundError` (aucun périphérique — spec quand rien
/// n'est capturable).
fn md_reject(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    let id = e.new_promise();
    let msg = e.str_val("NotFoundError: no capture device");
    e.reject_promise(id, msg);
    Ok(Value::Obj(id))
}

/// `mediaDevices.enumerateDevices()` → `[]` résolu.
fn md_empty_list(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    let id = e.new_promise();
    let empty = e.new_array(Vec::new());
    e.resolve_promise(id, empty);
    Ok(Value::Obj(id))
}

/// `speechSynthesis.getVoices()` → `[]` (aucune voix embarquée).
fn speech_voices(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    Ok(e.new_array(Vec::new()))
}

/// `permissions.query({name})` → `{state:'denied'}` résolu
/// (rien n'est accordé — les sites basculent sur le repli).
fn perm_query(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    let o = e.new_object();
    let st = e.str_val("denied");
    e.set_own(o, "state", st, true);
    e.set_own(o, "onchange", Value::Null, true);
    let id = e.new_promise();
    e.resolve_promise(id, Value::Obj(o));
    Ok(Value::Obj(id))
}

/// `scheduler.yield()` — promesse résolue (file microtâche).
fn scheduler_yield(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    let id = e.new_promise();
    e.resolve_promise(id, Value::Undefined);
    Ok(Value::Obj(id))
}

/// `scheduler.postTask(cb, {delay?, priority?, signal?})` —
/// spec Scheduler API : retourne une promesse résolue au
/// résultat de `cb`, schedulée sur la file de timers après
/// `delay` ms. `signal.aborted` → rejet `AbortError` (à
/// l'appel si déjà aborté, sinon re-lu au moment du tir).
/// `priority` accepté, ignoré (file FIFO unique).
fn scheduler_post_task(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let cb = arg(args, 0);
    if !e.is_callable(cb) {
        return Err(e.type_error("scheduler.postTask: callback is not a function"));
    }
    let mut delay = 0u64;
    let mut signal = Value::Undefined;
    if let Value::Obj(o) = arg(args, 1) {
        if let Some(Value::Num(d)) = e.get_prop(o, "delay") {
            delay = if d.is_finite() { d.max(0.0) as u64 } else { 0 };
        }
        if let Some(s) = e.get_prop(o, "signal") {
            signal = s;
        }
    }
    let pid = e.new_promise();
    // `signal` déjà aborté → rejet `AbortError` immédiat (spec).
    if let Value::Obj(s) = signal {
        if matches!(e.get_prop(s, "aborted"), Some(Value::Bool(true))) {
            let r = e.error_value(e.protos.error, "AbortError", "signal is aborted");
            e.reject_promise(pid, r);
            return Ok(Value::Obj(pid));
        }
    }
    let fire = e.bound_native(
        "postTask",
        post_task_fire,
        vec![cb, Value::Obj(pid), signal],
    );
    e.timers.schedule(fire, delay, false, Vec::new());
    Ok(Value::Obj(pid))
}

/// Tir différé de `postTask` : re-check `signal.aborted`,
/// appelle `cb`, résout/rejette la promesse liée.
fn post_task_fire(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let cb = arg(args, 0);
    let Value::Obj(pid) = arg(args, 1) else {
        return Ok(Value::Undefined);
    };
    if let Value::Obj(s) = arg(args, 2) {
        if matches!(e.get_prop(s, "aborted"), Some(Value::Bool(true))) {
            let r = e.error_value(e.protos.error, "AbortError", "signal is aborted");
            e.reject_promise(pid, r);
            return Ok(Value::Undefined);
        }
    }
    match e.call_value(cb, Value::Undefined, &[]) {
        Ok(v) => e.resolve_promise(pid, v),
        // Jet du callback → rejet de la promesse (spec).
        Err(crate::heap::Thrown(t)) => e.reject_promise(pid, t),
    }
    Ok(Value::Undefined)
}

/// `cookieStore.get(name)` → `null` résolu (jar vide).
fn cookie_get(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    let id = e.new_promise();
    e.resolve_promise(id, Value::Null);
    Ok(Value::Obj(id))
}

/// `cookieStore.getAll()` → `[]` résolu.
fn cookie_get_all(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    let id = e.new_promise();
    let empty = e.new_array(Vec::new());
    e.resolve_promise(id, empty);
    Ok(Value::Obj(id))
}

/// `cookieStore.set/delete` → `undefined` résolu (no-op jar).
fn cookie_void(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    let id = e.new_promise();
    e.resolve_promise(id, Value::Undefined);
    Ok(Value::Obj(id))
}

/// `WebAssembly.validate(bytes)` → `false` honnête (un octet
/// inconnu est invalide pour ce moteur).
fn wasm_false(_e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    Ok(Value::Bool(false))
}

/// `WebAssembly.compile*`/`instantiate*` — rejet
/// `CompileError` (wasm hors périmètre moteur — le repli JS
/// des bundles s'applique).
fn wasm_reject(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    let id = e.new_promise();
    let msg = e.str_val("CompileError: WebAssembly unsupported");
    e.reject_promise(id, msg);
    Ok(Value::Obj(id))
}

/// `trustedTypes.createPolicy(name, rules)` — politique
/// passthrough : `createHTML`/`createScript`/`createScriptURL`
/// rendent la valeur telle quelle (pas d'application TT
/// moteur — les bundles enregistrent leur `default` policy).
fn tt_create_policy(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let o = e.new_object();
    let name = e.to_js_string(arg(args, 0))?;
    let namev = e.str_val(name);
    e.set_own(o, "name", namev, true);
    for k in [
        "createHTML",
        "createScript",
        "createScriptURL",
        "createHTMLBinding",
    ] {
        let f = e.native_fn(k, tt_passthrough);
        e.set_own(o, k, f, true);
    }
    Ok(Value::Obj(o))
}

/// `policy.createX(v)` — passthrough string.
fn tt_passthrough(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let s = e.to_js_string(arg(args, 0))?;
    Ok(e.str_val(s))
}

/// `new PerformanceObserver(cb)` / `new ReportingObserver(cb)`
/// — impl bornée headless : le callback est enregistré mais
/// jamais tiré (aucune entrée produite ici) ; `observe`/
/// `disconnect`/`takeRecords` honnêtes. Régression mesurée :
/// le stub « illegal ctor » tuait `environment.js` de github
/// (le `new` est légal dans un navigateur).
fn perf_observer_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let cb = arg(args, 0);
    if !matches!(cb, Value::Obj(_)) {
        return Err(e.type_error("PerformanceObserver: callback requis"));
    }
    let o = e.new_object();
    e.set_own(o, "cb", cb, false);
    for (k, f) in [
        ("observe", observer_noop as crate::heap::NativeFn),
        ("disconnect", observer_noop),
        ("takeRecords", observer_empty),
    ] {
        let m = e.native_fn(k, f);
        e.set_own(o, k, m, true);
    }
    Ok(Value::Obj(o))
}

/// `new ReportingObserver(cb)` — même forme bornée.
fn reporting_observer_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let cb = arg(args, 0);
    if !matches!(cb, Value::Obj(_)) {
        return Err(e.type_error("ReportingObserver: callback requis"));
    }
    let o = e.new_object();
    e.set_own(o, "cb", cb, false);
    for (k, f) in [
        ("observe", observer_noop as crate::heap::NativeFn),
        ("disconnect", observer_noop),
        ("takeRecords", observer_empty),
    ] {
        let m = e.native_fn(k, f);
        e.set_own(o, k, m, true);
    }
    Ok(Value::Obj(o))
}

/// `observer.observe(opts)`/`disconnect()` — no-op honnête.
fn observer_noop(_e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    Ok(Value::Undefined)
}

/// `observer.takeRecords()` → `[]` (aucune entrée produite).
fn observer_empty(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    Ok(e.new_array(Vec::new()))
}

/// `new XPathEvaluator()` — `{evaluate, createExpression,
/// createNSResolver}` (rail `xpath_*` existant — CAP-188 ;
/// `createExpression` exigé par htmx sur codeberg/gitea :
/// `new XPathEvaluator().createExpression('…hx-on…')`).
fn xpath_evaluator_ctor(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    let o = e.new_object();
    let ev = e.native_fn("evaluate", crate::misc_apis::xpath_evaluate);
    e.set_own(o, "evaluate", ev, true);
    let ce = e.native_fn(
        "createExpression",
        crate::misc_apis::xpath_create_expression,
    );
    e.set_own(o, "createExpression", ce, true);
    let cr = e.native_fn("createNSResolver", crate::misc_apis::create_ns_resolver);
    e.set_own(o, "createNSResolver", cr, true);
    Ok(Value::Obj(o))
}
