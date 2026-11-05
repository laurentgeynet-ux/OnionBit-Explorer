// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! explorer-jscore — moteur JavaScript écrit from scratch, livrable de
//! l'équipe JS dédiée (ADR-0007).
//!
//! Noyau ES3/ES5 — `var`/`function`, prototypes, closures,
//! exceptions, ASI — étendu par la couverture dont les pages réelles
//! ont besoin : `let`/`const`, arrow functions, template literals,
//! `?.`/`??`, classes, `Promise` + microtasks, `eval`/`Function`
//! (scope global), regex via le crate `regex`, etc.
//!
//! Ce crate est **autonome** : aucune dépendance vers les autres
//! crates Explorer — son mini-DOM (`dom`, `dom_host`) est un harnais
//! de test, pas le vrai DOM. En phase 8, `explorer-js` l'adapte
//! derrière le trait `JsEngine` et le heap expose les primitives
//! rooting/trace/finalization du modèle mémoire ADR-0008.
//!
//! Un `Engine` est par page : le heap meurt à la navigation,
//! `set_op_limit` borne le travail, profondeurs d'appel/expression
//! plafonnées.

#![forbid(unsafe_code)]
// JS numeric coercions (`ToInt32`, `ToUint32`, array indices…) are
// spec-defined as truncating/wrapping casts — the pedantic cast lints
// would flag correct code.
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_lossless)]
// `NativeFn`'s signature mandates `Result` even for infallible natives;
// a tree-walker's `eval_inner`/`exec_stmt` are naturally long matches.
#![allow(clippy::unnecessary_wraps)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::similar_names)]
#![allow(clippy::many_single_char_names)]
#![allow(clippy::format_push_string)]
// `to_*` methods deliberately take `&mut self`: coercion can run user
// code (valueOf/toString). The names mirror the JS spec operations.
#![allow(clippy::wrong_self_convention)]

mod ast;
mod builtins;
mod channel;
mod cmp;
mod collections;
mod context;
mod crypto;
mod dom;
mod dom_host;
mod error;
mod eventsource;
mod files;
mod geom;
mod heap;
mod idb;
mod interp;
mod intl;
mod json;
#[cfg(test)]
mod leak_probe;
mod lexer;
mod mediaq;
mod misc_apis;
mod module;
mod mse;
mod mutation;
mod observers;
mod parser;
mod promise;
mod regexp;
mod storage;
mod streams;
mod svg_path;
mod temporal;
mod timers;
mod typed_array;
mod value;
mod webaudio;
mod websocket;
mod worker;
mod xhr;

pub use cmp::{CmpConsent, CmpPolicy};
pub use context::CtxId;
pub use crypto::{
    CryptoOp, CryptoRequest, CryptoResponse, CryptoTransport, DigestAlgo, ImportFormat, KeyAlgo,
};
pub use dom::{
    CanvasOpRecord, CanvasState, Dom, JournalOp, MediaCmd, MediaElState, MediaErrorCode,
    MediaFrameStats, MediaReport, MediaState, NodeId, SerializeStats,
};
pub use dom_host::{
    CookieJar, DomRef, DynFrameReq, DynImgReq, DynLinkReq, DynScriptReq, EventProp, FormSubmit,
};
pub use error::{Error, Result};
pub use eventsource::{EsEvent, EsRequest, EsTransport};
pub use geom::{ElementGeom, GeoRect, GeomPosition, GeometryMap, GeometrySlot};
pub use heap::{HeapStats, Host};
pub use idb::{CacheEntry, CacheRegistry, IdbDb, IdbIndexDef, IdbRegistry, IdbStore};
pub use interp::Engine;

/// Interneur de noms partagé (JS-C307+ / chantier `Rc<str>`) :
/// hit → `Rc` cloné (zéro alloc), miss → une alloc pour toute
/// la session. `thread_local` : le moteur est mono-thread par
/// design (`Rc` partout) et la dédup est **partagée entre
/// contextes d'une même page** — noms AST, clés de props et
/// noms de scope tombent dans la même table.
pub(crate) fn intern(name: &str) -> std::rc::Rc<str> {
    thread_local! {
        static INTERNED: std::cell::RefCell<
            std::collections::HashSet<std::rc::Rc<str>>,
        > = std::cell::RefCell::new(std::collections::HashSet::new());
    }
    INTERNED.with(|set| {
        let mut set = set.borrow_mut();
        if let Some(s) = set.get(name) {
            return s.clone();
        }
        let s: std::rc::Rc<str> = std::rc::Rc::from(name);
        set.insert(s.clone());
        s
    })
}

/// §B-59 — dernier natif entré dans `call_native`, écrit
/// inconditionnellement (~20 ns, `&'static str`, zéro alloc).
/// Le `rss_guard` du daemon le lit à l'abort : une mort dans
/// un span natif non-tické nomme son natif même sans retour.
pub static LAST_NAT: std::sync::Mutex<&'static str> = std::sync::Mutex::new("");

// ----------------------------------------------------------------
// JS-C313 — sonde §B-4(c) : journal borné des rejets
// `[[DefineOwnProperty]]`/`Object.assign`. Le mur YouTube =
// installation de méthodes tronquée à mi-composition de mixin
// (protos `own=4/10` vs ~20-30 attendus) — chaque rejet nomme
// cible, clé, descripteur courant et raison pour borner le drop
// exact. Cap 128 entrées (borné, ~20 ko), drainable par test.
// ----------------------------------------------------------------
thread_local! {
    static DEFPROP_LOG: std::cell::RefCell<Vec<String>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Note un rejet `defineProperty`/`defineProperties`/`assign` —
/// borné à 128 entrées (une composition fautive suffit).
pub(crate) fn defprop_note(msg: String) {
    DEFPROP_LOG.with(|l| {
        let mut l = l.borrow_mut();
        if l.len() < 128 {
            l.push(msg);
        }
    });
}

/// Vide et rend le journal des rejets (lecture test — JS-C313).
pub(crate) fn defprop_log_drain() -> Vec<String> {
    DEFPROP_LOG.with(|l| std::mem::take(&mut *l.borrow_mut()))
}

thread_local! {
    static NULLREAD_LOG: std::cell::RefCell<Vec<String>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Note une lecture `undefined`/`null` — chaque entrée nomme la
/// clé lue et la fonction JS appelante (`fn(bundle:l:c)`) : le
/// throw `cannot read 'call' of Undefined` (signature §B-4(c))
/// identifie le `X.call` où `X` était undefined. Cap 512.
pub(crate) fn nullread_note(msg: String) {
    NULLREAD_LOG.with(|l| {
        let mut l = l.borrow_mut();
        if l.len() < 512 {
            l.push(msg);
        }
    });
}

/// Vide et rend le journal des lectures nulles (lecture test).
pub(crate) fn nullread_log_drain() -> Vec<String> {
    NULLREAD_LOG.with(|l| std::mem::take(&mut *l.borrow_mut()))
}
/// Dernier variant `Expr` évalué (armé `EXPLORER_JS_RSS_WATCH`) —
/// discrimine le span fatal : natif (`last_nat`) vs éval-side
/// (le `Expr` courant nomme la famille : `Call`, `Assign`…).
pub static LAST_EXPR: std::sync::Mutex<&'static str> = std::sync::Mutex::new("");
/// Dernier `Stmt` entré dans `exec_stmt` (armé
/// `EXPLORER_JS_RSS_WATCH`) — nomme la forme de statement qui
/// encadre le span fatal (`For`/`Var`/`Expr`/`Try`…).
pub static LAST_STMT: std::sync::Mutex<&'static str> = std::sync::Mutex::new("");
/// Taille (octets) de la dernière requête `alloc_guard` (armé
/// `EXPLORER_JS_RSS_WATCH`) — si le span fatal passe par une
/// garde, sa demande est nommée à l'abort.
pub static LAST_ALLOC_REQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
/// Recensement heap au dernier tick « chaud » (RSS > watch) :
/// compteurs + `ObjKind` du plus gros objet vivant — nomme le
/// réservoir si le burst est une structure comptée, prouve
/// « hors compteurs » sinon (§B-59).
pub static LAST_STATS: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// §B-59 — compteur total des entrées dans `call_value_impl`
/// (atomique, inconditionnel) : figé pendant la montée = span
/// unique non-tické ; qui monte = boucle d'appels chaude.
pub static CALL_N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

// ----------------------------------------------------------------
// J17 — compteurs d'attribution d'alloc par site (§B-59) :
// `stats_alloc` donne le total par phase ; ces atomiques comptent
// les ÉVÉNEMENTS par site chaud pour nommer la famille dominante
// (octets ≈ événement × taille type du site). Monotones — le banc
// lit le delta avant/après. `Relaxed`, ~1-2 ns par bump.
// ----------------------------------------------------------------
/// Cellules `Str` créées (`Heap::alloc_str`) — 1 alloc buffer.
pub static J17_STR_CELL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Cellules objet créées (`Heap::alloc_obj`) — +1 alloc `props`
/// Vec par objet non vide, +1 `String` par clé (`J17_PROP_NEW`).
pub static J17_OBJ_CELL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Nouvelle propre prop (`push_prop`) — 1 `String` de clé.
pub static J17_PROP_NEW: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Liaison insérée en scope (`BindingMap::insert`) — la croissance
/// du `Vec`/`HashMap` de portée suit ce compteur.
pub static J17_BIND: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// `Env` créé (`local`/`block`/`with`/`global`) — 1 alloc `Rc`.
pub static J17_ENV: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// `Vec<Value>` d'args matérialisé (`eval_args`, non vide) —
/// 1 alloc par appel avec args.
pub static J17_ARGS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Clone `String` du nom de membre au site d'appel
/// (`o.m(…)`/`o[k](…)` dans `Expr::Call`).
pub static J17_MEMBER_KEY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Tableau matérialisé (`new_array`) — 1 alloc `Vec<Value>`.
pub static J17_ARRAY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Itérateur drainé en `Vec` (`iter_values`/`_await`).
pub static J17_ITER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// `RegExp.exec`/match natif entré (`Re::exec`/`exec_from`/
/// `exec_all`) — fancy-regex alloue en interne par exécution.
pub static J17_REGEX: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// `Tok::Ident` produit par le lexer — 1 `String` par ident.
pub static J17_TOK_IDENT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// `prop_key(v)` appelé — chaque `o[k]`/`arr[i]` matérialise une
/// `String` de clé (nombre → `num_to_js_string` relu ensuite en
/// indice par la chaîne tableau).
pub static J17_PROP_KEY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// J17 — lectures `arr[i]` servies par le fast-path éléments
/// sans `String` de clé (`arr_index_get`).
pub static J17_ARR_IDX: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Gate J17 : les compteurs ci-dessus ne s'incrémentent que si
/// `EXPLORER_J17_PROF` est posé (non vide/non "0") au premier
/// appel, ou si [`j17_enable`] a été appelé (tests/benchs). Un
/// `fetch_add` coûte ~10-20 ns chacun — des millions par éval
/// lourde taxaient toute la suite ; un `load`+branche ~1 ns.
static J17_ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static J17_ENV_ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// `true` si les compteurs J17 sont armés (env ou enable).
#[inline]
pub(crate) fn j17_on() -> bool {
    J17_ENABLED.load(std::sync::atomic::Ordering::Relaxed)
        || *J17_ENV_ON.get_or_init(|| {
            std::env::var("EXPLORER_J17_PROF").is_ok_and(|v| !v.is_empty() && v != "0")
        })
}

/// Arme les compteurs J17 sans variable d'environnement
/// (bancs de test J17 — sinon les relevés restent à zéro).
pub fn j17_enable() {
    J17_ENABLED.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Incrément borné d'un compteur J17 — no-op hors mode profil.
#[inline]
pub(crate) fn j17_tick(c: &'static std::sync::atomic::AtomicU64) {
    if j17_on() {
        c.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

/// §B-59 — dernière méthode/prop `Host` dispatchée
/// (`class:name`) : les `h.invoke`/`h.get`/`iter_items`…
/// passent **hors** `call_value_impl` — `calls`/`nat` muets,
/// le span natif hôte est nommé ici.
pub static LAST_HOST: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// Écrit `class:name` dans [`LAST_HOST`] (diagnostic §B-59).
pub fn host_mark(class: &str, name: &str) {
    if let Ok(mut g) = LAST_HOST.lock() {
        g.clear();
        g.push_str(class);
        g.push(':');
        g.push_str(name);
    }
}

/// §B-59 — dernier frame JS du tick (écrit sous
/// `EXPLORER_JS_RSS_WATCH` uniquement — `format!` par tick est
/// trop cher hors diagnostic).
pub static LAST_FRAME: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());
/// Phase de l'appelant JS — **inconditionnelle** (`&'static
/// str`, ~20 ns) : écrite à l'entrée de `call_value_impl` et
/// `exec_body`, là où aucun tick n'a encore couru. Vide à
/// l'abort = le span fatal est dans une statement/éval tickée
/// (couvert par `LAST_STMT`) ou hors fil JS ; `"call"`/`"body"`
/// = mort dans le préambule d'appel (liage, hoist, suspend).
pub static LAST_PHASE: std::sync::Mutex<&'static str> = std::sync::Mutex::new("");
/// §B-59 — fonction `Fun::Script` en cours d'entrée (écrit au
/// début de `call_value_impl`, **avant** tout préambule) :
/// `last_phase=call` figé + `last_fun` nommé = la mort vit
/// dans le préambule d'appel de CETTE fonction (liage, hoist,
/// `arguments`, switch de realm) — zones que `last_stmt`/
/// `last_nat` ne couvrent pas.
pub static LAST_FUN: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// §B-59 — journal de fil d'Ariane **externe au fil JS** : un
/// thread vidange les statiques `LAST_*` + commit/RSS vers un
/// fichier toutes les ~400 ms (env `EXPLORER_JS_BREADCRUMB`).
/// Les breadcrumbs écrits par tick sont muets quand le span
/// fatal n'est pas tické (une expression géante n'offre aucun
/// tick) — le journal tourne sur son propre fil et survit à
/// l'abort alloc : sa dernière ligne nomme le span vivant.
pub fn start_breadcrumb_journal() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let Some(path) = std::env::var_os("EXPLORER_JS_BREADCRUMB") else {
            return;
        };
        std::thread::spawn(move || {
            let t0 = std::time::Instant::now();
            loop {
                let (mut rss, mut com) = (0usize, 0usize);
                if let Some(m) = memory_stats::memory_stats() {
                    rss = m.physical_mem / (1024 * 1024);
                    com = m.virtual_mem / (1024 * 1024);
                }
                let get = |m: &std::sync::Mutex<&'static str>| {
                    m.lock().map_or("", |g| *g)
                };
                let gets = |m: &std::sync::Mutex<String>| {
                    m.lock().map(|g| g.clone()).unwrap_or_default()
                };
                let areq = LAST_ALLOC_REQ.load(std::sync::atomic::Ordering::Relaxed);
                let calls = CALL_N.load(std::sync::atomic::Ordering::Relaxed);
                let line = format!(
                    "t={}ms rss={}MiB commit={}MiB | calls={} | frame={} | fun={} | stmt={} | expr={} | nat={} | host={} | phase={} | areq={} | stats={}\n",
                    t0.elapsed().as_millis(),
                    rss,
                    com,
                    calls,
                    gets(&LAST_FRAME),
                    gets(&LAST_FUN),
                    get(&LAST_STMT),
                    get(&LAST_EXPR),
                    get(&LAST_NAT),
                    gets(&LAST_HOST),
                    get(&LAST_PHASE),
                    areq,
                    gets(&LAST_STATS),
                );
                // Append : l'historique discrimine « breadcrumbs
                // figés pendant la montée » (span unique non
                // tické) vs « breadcrumbs vivants » (chemins
                // tickés qui passent sous toutes les gardes).
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
                {
                    use std::io::Write as _;
                    let _ = f.write_all(line.as_bytes());
                }
                std::thread::sleep(std::time::Duration::from_millis(400));
            }
        });
    });
}
pub use mediaq::MediaEnv;
pub use module::{
    is_bare_specifier, module_specifiers, rewrite_bare_specifier, DynImportReq, ModuleLoader,
};
pub use storage::Storage;
pub use value::{ObjId, Value};
pub use websocket::{WsEventKind, WsOut, WsRequest, WsTransport};
pub use worker::{WorkerConfig, WorkerFactory, WorkerLoader, WorkerNetRequest, WorkerNetResponse};
pub use xhr::{FetchCredentials, FetchMode, XhrRequest, XhrResponse, XhrTransport};
