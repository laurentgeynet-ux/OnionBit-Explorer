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
// `Value` is the engine's 16-octet tagged handle (JsStr = `Rc` bump,
// JS-C420) : `NativeFn`'s fn-pointer signature `fn(&mut Engine, Value,
// &[Value])` mandates `this: Value` in ~150 handlers, and reader
// helpers keep the uniform by-value idiom — an `Rc` bump is cheaper
// than the churn of a dual `&Value`/`Value` convention.
#![allow(clippy::needless_pass_by_value)]

mod ast;
mod builtins;
mod channel;
mod cjs;
mod cmp;
mod collections;
mod context;
mod crypto;
mod cssom;
mod dom;
mod dom_host;
mod error;
mod eventsource;
mod files;
mod fnat;
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
mod sha2;
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
#[doc(hidden)]
pub use fnat::{fn_chain_at, FnHit};
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
        // JS-C566 — cache par pointeur canonique : les clés
        // déjà internées (noms de membres `Rc<str>` de l'AST —
        // le parseur interne — passés `&*rc`) arrivent avec le
        // pointeur de l'alloc internée. Cette alloc est
        // **immortelle** (la table ne retire jamais) → son
        // adresse ne peut pas être réemployée : `ptr + len`
        // identiques = mêmes octets (la garde `len` rejette
        // les tranches `&interned[..n]` qui partagent le ptr).
        // Hit = hash de pointeur ~10 ns au lieu du hash de
        // contenu. Les `&str` transitoires n'entrent jamais
        // (enregistré seulement quand `rc.as_ptr() ==
        // name.as_ptr()` — adresse canonique) → aucun faux hit
        // sur buffer recyclé.
        static PTR_HIT: std::cell::RefCell<
            std::collections::HashMap<*const u8, std::rc::Rc<str>>,
        > = std::cell::RefCell::new(std::collections::HashMap::new());
    }
    // `EXPLORER_NO_PTRHIT=1` désarme le cache (A/B de filet).
    static PTRHIT_ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let on = *PTRHIT_ON.get_or_init(|| {
        std::env::var("EXPLORER_NO_PTRHIT").ok().as_deref() != Some("1")
    });
    if on {
        if let Some(rc) = PTR_HIT.with(|m| m.borrow().get(&name.as_ptr()).cloned()) {
            if rc.len() == name.len() {
                return rc;
            }
        }
    }
    let rc = INTERNED.with(|set| {
        let mut set = set.borrow_mut();
        if let Some(s) = set.get(name) {
            return s.clone();
        }
        let s: std::rc::Rc<str> = std::rc::Rc::from(name);
        set.insert(s.clone());
        s
    });
    if on && rc.as_ptr() == name.as_ptr() {
        PTR_HIT.with(|m| m.borrow_mut().insert(name.as_ptr(), rc.clone()));
    }
    rc
}

/// JS-C556 — flags `DBG_*`/`EXPLORER_*` sondés **une fois** :
/// `std::env::var` rescannait le bloc environnement à chaque
/// appel — ~40 M lookups/vol rien que dans `eval_inner`
/// (`DBG_EXPR`), ~7 M dans `exec_body`+stmts (`DBG_DRIVE`),
/// ~2 M par sonde native. Pattern `OnceLock` (précédent
/// `CTXP` de `set_own`). Chaque flag = un `AtomicBool`-like
/// read après init.
macro_rules! env_flag {
    ($name:ident, $var:literal) => {
        /// Voir [`env_flag`] — flag d'environnement caché.
        pub(crate) fn $name() -> bool {
            static F: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
            *F.get_or_init(|| std::env::var_os($var).is_some())
        }
    };
}
env_flag!(dbg_expr, "DBG_EXPR");
env_flag!(dbg_drive, "DBG_DRIVE");
// Flag d'environnement caché — son unique callsite est sous
// `debug_assertions` (`lower_suspend_body`) : l'item est
// compilé en debug seulement.
#[cfg(debug_assertions)]
env_flag!(dbg_lower, "DBG_LOWER");
env_flag!(dbg_nat, "DBG_NAT");
env_flag!(ctx_probe, "EXPLORER_CTX_PROBE");
env_flag!(missing_api, "EXPLORER_MISSING_API");
env_flag!(js_profile, "EXPLORER_JS_PROFILE");
env_flag!(nat_trace, "EXPLORER_NAT_TRACE");
env_flag!(j17_host, "EXPLORER_J17_HOST");
env_flag!(no_immemo, "EXPLORER_NO_IMMEMO");

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
/// Boîtes `JsStr` créées (`JsStr::new`, JS-C420) — 1 alloc `Rc`.
pub static J17_STR_CELL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Cellules objet créées (`Heap::alloc_obj`) — +1 alloc `props`
/// Vec par objet non vide, +1 `String` par clé (`J17_PROP_NEW`).
pub static J17_OBJ_CELL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Nouvelle propre prop (`push_prop`) — 1 `String` de clé.
pub static J17_PROP_NEW: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Liaison insérée en scope (`BindingMap::insert`) — la croissance
/// du `Vec`/`HashMap` de portée suit ce compteur.
pub static J17_BIND: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// JS-C533 — `Bindings::get` servi par le canon slot-indexé
/// (couverture des lookups par le stockage partagé).
pub static J17_BND_CANON: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// JS-C533 — `Bindings::get` servi par `extra` (liaisons
/// dynamiques hors canon — eval-injection, non modélisé).
pub static J17_BND_EXTRA: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// JS-C534 — `extra` servi sur env **canon câblé** : le nom est
/// génuinément dynamique (eval-injection, scope non modélisé) —
/// structurel, non raccordable.
pub static J17_XT_DYN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// JS-C534 — `extra` servi sur env **non câblé** (`canon` vide) :
/// un site `predeclare_slots` manquant — couverture raccordable.
pub static J17_XT_UNW: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// JS-C534 — `predeclare_slots` a trouvé son entrée `names_table`
/// (env câblé).
pub static J17_WIRE_HIT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// JS-C534 — `predeclare_slots` sans entrée `names_table`
/// (env non câblé — site manquant).
pub static J17_WIRE_MISS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
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
// -- V-2 (chantier VITESSE-JS) : attribution des chemins de --
// -- résolution — événements, pas octets. Monotones, deltas --
// -- lus par banc/test/sonde. --------------------------------
/// `find_own` entré — un appel par niveau proto parcouru
/// (`J17_PROTO_LVL` = cumul des niveaux).
pub static J17_PROP_LOOKUP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Niveaux de chaîne prototype parcourus (`find_prop` loop).
pub static J17_PROTO_LVL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Comparaisons de clés payées en scan linéaire `prop_pos`
/// (< `PROPS_IDX_MIN`) — le coût exact de la branche V-3a.
pub static J17_PROP_SCAN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Hit de l'index paresseux `props` (≥ `PROPS_IDX_MIN`).
pub static J17_PROP_IDX_HIT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Miss de l'index paresseux `props`.
pub static J17_PROP_IDX_MISS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Constructions de l'index paresseux `props` (`get_or_init`).
pub static J17_PROP_IDX_BUILD: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// JS-C544 — rendement `rabs_memo` (mémo own-absent par
/// maillon `(shape, kid)`) : hits (maillon sauté sans
/// `find_own_ai`), apprentissages, évictions FIFO au cap
/// `RABS_MEMO_CAP` (JS-C545 — `rabs_full` compte les
/// évictions ; N5.52 = refus d'insert, gel mesuré).
pub static J17_RABS_HIT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Apprentissages `rabs_memo` (insert d'une paire absente).
pub static J17_RABS_INS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Évictions FIFO `rabs_memo` — cap `RABS_MEMO_CAP` atteint,
/// la plus ancienne paire est retirée (était « refus
/// d'insert » avant JS-C545 — le compteur nomme désormais
/// la pression d'éviction, plus la saturation).
pub static J17_RABS_FULL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Niveaux de scope parcourus par `env_get_seen` — profondeur
/// de fermeture moyenne × `env_get` = coût résolution variable.
pub static J17_ENV_LVL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Appels `env_get_seen` — `env_lvl / env_gets` = profondeur de
/// chaîne moyenne par résolution (cran 73b).
pub static J17_ENV_GETS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Lecture d'identifiant servie par l'annotation du resolveur
/// (JS-C490) / annotation présente mais cible manquante →
/// repli sur la marche complète.
pub static J17_RES_HIT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static J17_RES_MISS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// JS-C500 — inline cache own-prop `site → (shape,pos)` :
/// hits vérifiés (`ic_hit`), échecs de vérification (`ic_miss`),
/// apprentissages (`ic_learn`), sites désarmés au seuil
/// (`ic_disarm`), consultations sur site désarmé (`ic_bail`).
pub static J17_IC_HIT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Voir [`J17_IC_HIT`].
pub static J17_IC_MISS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Voir [`J17_IC_HIT`].
pub static J17_IC_LEARN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Voir [`J17_IC_HIT`].
pub static J17_IC_DISARM: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Voir [`J17_IC_HIT`].
pub static J17_IC_BAIL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// JS-C555 — part des bails `ic_bail` par cause : `bail_dead` =
/// site désarmé (`miss == MAX`), `bail_kind` = tête inéligible
/// par construction (la part `ic_bail` restante).
pub static J17_IC_BAIL_KIND: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Voir [`J17_IC_BAIL_KIND`].
pub static J17_IC_BAIL_DEAD: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// JS-C508 — `ic_nlearn` = tentatives de learn échouées (lien
/// mort, maillon inéligible, `Acc`, chaîne > cap) — elles posent
/// une pierre tombale `Dead` ou incrémentent `miss` ;
/// `ic_purge` = purges de `ic_map` au cap (partielles : sains et
/// désarmés conservés).
pub static J17_IC_NLEARN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Voir [`J17_IC_NLEARN`].
pub static J17_IC_PURGE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// JS-C528 — hits servis par un membre `Poly` (site désarmé
/// repassé en polymorphe borné — verdicts par shape de tête).
pub static J17_IC_POLY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// JS-C529 — lectures/écritures globales servies par l'own-scan
/// `o.find` (index `prop_pos`) sans le walk complet
/// `find_prop` — fallback `env_get`/`env_set` sur l'objet global.
pub static J17_ENV_GHIT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// JS-C530 — écritures servies par l'own-first `set_prop_walk`
/// (tête éligible + own `Data` writable → `props[i]` direct,
/// gardes TA / `args_map` / Array / alloc sautées).
pub static J17_W_OWN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// JS-C531 — hits servis par une entrée `Acc` (accesseur hérité
/// ou own, position+chaîne de shapes vérifiées — les sites à
/// setters/getters étaient inapprenables avant C531).
pub static J17_IC_ACC: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// JS-C509 — attribution de `prop_scan` par classe d'entrée
/// (delta `PROP_SCAN` autour du walk — les deltas imbriqués
/// peuvent sur-attribuer, sonde indicative) : `ps_get` lectures
/// (AST+internes), `ps_has` `in`/`has_prop`, `ps_own` `find_own`
/// (attr ops, `hasOwnProperty`…), `ps_set` écritures `[[Set]]`.
pub static J17_PS_GET: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Voir [`J17_PS_GET`].
pub static J17_PS_HAS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Voir [`J17_PS_GET`].
pub static J17_PS_OWN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Voir [`J17_PS_GET`].
pub static J17_PS_SET: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// JS-C511 — attribution de `prop_lookup` (invocations
/// `find_own_ai`) par chemin d'entrée : `pl_get` lectures,
/// `pl_has` `in`/`has_prop`, `pl_own` `find_own` attr-ops,
/// `pl_set` écritures — résiduel = appels internes directs.
pub static J17_PL_GET: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Voir [`J17_PL_GET`].
pub static J17_PL_HAS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Voir [`J17_PL_GET`].
pub static J17_PL_OWN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Voir [`J17_PL_GET`].
pub static J17_PL_SET: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// JS-C502 — fast-path slot d'env `(d,slot)` : `slot_hit` =
/// lecture directe `Small[slot]` vérifiée par nom ; `slot_miss`
/// = échec de vérification (oob, clé différente, `Map`, cellule
/// non-`Bind`) → repli `env_get`. Un `slot_miss` élevé trahit
/// une divergence d'ordre canonique resolver/runtime.
pub static J17_SLOT_HIT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Voir [`J17_SLOT_HIT`].
pub static J17_SLOT_MISS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// JS-C505 — `slot_miss` par cause : `sm_oob` = slot hors bornes
/// (env non pré-déclaré / coverage), `sm_key` = nom au slot ≠
/// clé (divergence d'ordre canonique — le signal de bug),
/// `sm_cell` = cellule non-`Bind` (`Uninit`/alias/`LazyArgs`/
/// immuable en écriture), `sm_map` = réservé (JS-C533 : `extra` non câblé compte `oob`),
/// `sm_kind` = kind d'env non câblé (`With`).
pub static J17_SM_OOB: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Voir [`J17_SM_OOB`].
pub static J17_SM_KEY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Voir [`J17_SM_OOB`].
pub static J17_SM_CELL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Voir [`J17_SM_OOB`].
pub static J17_SM_MAP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Voir [`J17_SM_OOB`].
pub static J17_SM_KIND: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// JS-C505 — histogramme des opérateurs binaires passant par
/// `binary_op`, indexé par `BinOp as usize` (ordre de déclaration
/// `ast.rs`) — calibre la cible du premier quickening (cran 3).
pub static J17_BINOP: [std::sync::atomic::AtomicU64; 25] =
    [const { std::sync::atomic::AtomicU64::new(0) }; 25];
/// JS-C505 — sous-classification de `Add` post-`to_primitive` :
/// `badd_num` = les deux opérandes numériques (cible `AddNum`),
/// `badd_str` = un `Str` (cible `AddStr`), `badd_oth` = autre
/// (`Big`, coercitions résiduelles).
pub static J17_BIN_ADD_NUM: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Voir [`J17_BIN_ADD_NUM`].
pub static J17_BIN_ADD_STR: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Voir [`J17_BIN_ADD_NUM`].
pub static J17_BIN_ADD_OTH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// JS-C535 — couverture du peek homo-typé de `binary_op` :
/// `bin_nn` = paire `(Num,Num)` servie sans `to_primitive`/
/// `to_number`, `bin_ss` = paire `(Str,Str)` (concat `Add`,
/// comparaison lexicographique). Le reste du traffic tombe
/// sur le chemin générique — la somme mesure la part du
/// fast-path dans le total `binop:*`.
pub static J17_BIN_NN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Voir [`J17_BIN_NN`].
pub static J17_BIN_SS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// JS-C538 — `===`/`!==` servis directement dans `binary_op`
/// (hors peek `(Num,Num)` déjà couvert) : `strict_eq` ne
/// coerces pas et ne lance aucun code utilisateur — le
/// `binary_op_slow` n'apportait qu'un appel + un match
/// d'op. `bin_se` vs `binop:seq`+`binop:sne` mesure la part
/// du direct-serve (`typeof x==='f'`, `x===undefined`,
/// `obj===obj`, paires mixtes).
pub static J17_BIN_SE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// JS-C536/JS-C537 — évictions de lits résolus au cap
/// `RESOLVED_LITS_CAP` (avant C537 : clears **globaux** —
/// `resolved_map`/`names_table`/`lowered_suspend_cache`/`ic_map`
/// vidés, 20/session yt au vol N5.45 ; désormais : retrait
/// ciblé des clés du plus ancien lit, `ic_map` auto-bornée).
/// `res_clear` élevé = pression sur le cap (envisager cap
/// adaptatif ou LRU).
pub static J17_RES_CLEAR: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// JS-C505 — noms de [`J17_BINOP`], ordre = déclaration `BinOp`
/// (`ast.rs`) — maintenus ensemble par le commentaire réciproque.
const J17_BINOP_NAMES: [&str; 25] = [
    "or", "and", "bitor", "bitxor", "bitand", "eq", "ne", "seq", "sne", "lt", "le", "gt", "ge",
    "instof", "in", "shl", "shr", "shru", "add", "sub", "mul", "div", "mod", "pow", "nullish",
];
/// `array_index(key)` tenté — multiplicité par `find_own`
/// (× niveau, ×2 par niveau Array) — cible V-3b (hoist).
pub static J17_ARRIDX_TRY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// JS-C532 — histogramme d'évaluation par variante `Expr` :
/// compté à l'entrée d'`eval_inner` **et** au dépile de l'épine
/// gauche (les maillons `Binary`/`Member`/`Index`/`Call`-chaîne
/// sont consommés par `eval_spine` sans passer `eval_inner`) —
/// chaque nœud AST évalué compte exactement une fois. Ordre =
/// déclaration `Expr` (`ast.rs`) — noms ci-dessous au même ordre.
pub static J17_EVAL: [std::sync::atomic::AtomicU64; 33] =
    [const { std::sync::atomic::AtomicU64::new(0) }; 33];
/// JS-C532 — noms de [`J17_EVAL`], ordre = déclaration `Expr`.
const J17_EVAL_NAMES: [&str; 33] = [
    "num",
    "bigint",
    "str",
    "bool",
    "null",
    "ident",
    "this",
    "array",
    "object",
    "function",
    "class",
    "unary",
    "binary",
    "assign",
    "cond",
    "member",
    "index",
    "call",
    "new",
    "seq",
    "template",
    "tagged",
    "regex",
    "chain",
    "spread",
    "supercall",
    "supermember",
    "superindex",
    "await",
    "yield",
    "importcall",
    "importmeta",
    "newtarget",
];
/// JS-C532 — idem par variante `Stmt` (entrée `exec_stmt_inner`,
/// ordre = déclaration `Stmt`).
pub static J17_STMT: [std::sync::atomic::AtomicU64; 24] =
    [const { std::sync::atomic::AtomicU64::new(0) }; 24];
/// JS-C532 — noms de [`J17_STMT`], ordre = déclaration `Stmt`.
const J17_STMT_NAMES: [&str; 24] = [
    "empty",
    "expr",
    "var",
    "lexical",
    "fndecl",
    "classdecl",
    "block",
    "if",
    "while",
    "dowhile",
    "for",
    "forin",
    "forof",
    "using",
    "with",
    "labeled",
    "break",
    "continue",
    "return",
    "throw",
    "try",
    "switch",
    "import",
    "export",
];
/// `call_value_impl` entré — version gatée de `CALL_N`
/// (celui-ci, non gaté, sert les sondes de vol §B-59).
pub static J17_CALL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Dispatch native (`Fun::Native`/`BoundNative`) —
/// `J17_CALL` − `J17_CALL_NAT` ≈ appels script (frame/env).
pub static J17_CALL_NAT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// `Heap::string` clone-read servi — chaque lecture de chaîne
/// clone (`s.clone()`) — suspect V-2h (`stats_alloc` donne
/// les octets, ce compteur les événements).
pub static J17_STR_READ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// `probe_lookup` chemin **armé** atteint (`dom_ctx` présent +
/// clé surveillée/taguée → `format!` + dedup `HashSet`) —
/// V-2i : les vols tournent `dom_probe=info`, ce compteur ×
/// ~coût format+insert = le biais des métriques de vol elles-
/// mêmes (dispatch, phases §B-97).
pub static J17_PROBE_HIT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// `parse_in`/`parse_module_in` entré — dénominateur du ratio
/// de re-parse. Décision V-3c : un cache `Program` ne se
/// justifie que si `parse_dup/parse_call` est significatif
/// sur les vols réels (chaque `<script>` ne parse qu'une fois ;
/// le cache ne rapporterait que sur `eval()`/`Function()` de
/// source répétée).
pub static J17_PARSE_CALL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Parse d'une source déjà vue (hash identique — mode
/// script/module inclus dans le hash, précondition V-3c
/// respectée). Le hash + le set TLS ne tournent que sous
/// `j17_on()` : coût nul désarmé. Collision hash = faux
/// positif toléré (diagnostic borné, pas décision d'exécution).
pub static J17_PARSE_DUP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// V-6 — temps `lex()` cumulé en ns (inclut la matérialisation
/// `chars().collect()` — suspect n°1 : 4 o/char × source).
/// Désarmé : un `load`+branche par parse, ~1 ns.
pub static J17_LEX_NS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// V-6 — temps `close_table` cumulé en ns (O(n) pile — vérifié
/// par inspection, mesuré pour conclure).
pub static J17_CLOSE_NS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// V-6 — temps de la boucle `stmt` (descente récursive +
/// `scan_directives`) cumulé en ns — le cœur du parse.
pub static J17_STMT_NS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// V-6 — tokens produits par `lex` (cumul `toks.len()`) —
/// calibre le volume vs octets source.
pub static J17_LEX_TOK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// V-6 — statements top-level parsés (cumul `body.len()`).
pub static J17_AST_STMT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// V-6 — `Tok` deep-clonés (`peek_clone`/`next_clone`) : les
/// variants `String`/`Vec` paient malloc+memcpy par clone —
/// la cascade de précédence clone ~N× par token littéral.
pub static J17_TOK_CLONE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// V-6 — `next()` appelé : ~`lex_tok` = streaming une-passe
/// confirmé ; >> `lex_tok` = re-consommation cachée
/// (backtracking non couvert par les gardes `close_of`).
pub static J17_PAR_NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// V-5a — corps de fonction différés au parse (slice retenue,
/// saut `close_of` O(1)).
pub static J17_FN_DEFER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// JS-C524 — corps différés **matérialisés** (premier appel
/// réel) : `def_mat`/`fn_defer` = taux d'usage — décide si le
/// lexer streaming (corps jamais lexés) vaut le refactor.
pub static J17_DEFER_MAT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Erreurs de matérialisation de corps différés (JS-C525c diag) —
/// toute valeur >0 = un corps relu a échoué au parse tardif.
pub static J17_DEF_ERR: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// V-5a — tokens couverts par les corps différés (part du
/// stream soustraite à la descente éager).
pub static J17_FN_DEF_TOK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// V-5a — corps différés matérialisés au premier appel.
/// `fn_defer`/`fn_mat` ≫ 1 = la majorité des corps retenus ne
/// sont jamais appelés → le deferral a évité leur parse.
pub static J17_FN_MAT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// V-7 — ns cumulés dans le prologue+épilogue de
/// `call_script_body` (coercition `this`, `Env::local`, liaison
/// des paramètres, `arguments` paresseux, `hoist`, pops) —
/// **hors** `exec_body`. Overhead frame pur par appel JS.
pub static J17_FRAME_NS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// V-7 — ns cumulés dans `env_get_seen` (marche de chaîne de
/// portées, inclusif : les sous-appels éventuels comptent dedans).
pub static J17_ENV_NS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// V-7 — ns cumulés dans `get_prop_value` (résolution de
/// propriété, inclusif — un getter JS re-entrant cumule son
/// propre temps dans le span englobant).
pub static J17_PROP_NS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// V-7 — ns cumulés dans `resolve_lit_body` — matérialisation
/// des corps différés V-5a (sous-parse + construction AST) au
/// premier appel.
pub static J17_MAT_NS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// V-7 — ns cumulés dans `collect` (marquage+balayage).
pub static J17_GC_NS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// V-7 — frames `Env::local` créées (une par appel JS
/// ordinaire — le volume d'allocation de scope).
pub static J17_ENV_NEW: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// V-7 — ns cumulés dans `exec_body` (dispatch du corps —
/// inclusif : les appels imbriqués comptent dedans).
pub static J17_BODY_NS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// JS-C558 — ns cumulés dans l'invocation d'un **getter
/// scripté** (`call_value` des deux bras `Prop::Acc` de
/// `read_prop_at`, inclusifs dans `prop_ns` — le corps du
/// getter compte dedans). Sépare « la lecture déclenche un
/// appel » de « la lecture marche ».
pub static J17_ACC_NS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// JS-C558 — invocations de getter scripté (paire de
/// `J17_ACC_NS` : coût moyen par invocation).
pub static J17_ACC_GET: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// JS-C558 — ns cumulés dans `Host::get` (getters DOM —
/// inclusifs dans `prop_ns` idem).
pub static J17_HOST_NS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// JS-C558 — `Host::get` servis (paire de `J17_HOST_NS`).
pub static J17_HOST_GET: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// JS-C559 — hits du mémo identité immuable `NodeHost`
/// (lectures servies sans `dom.borrow`/`alloc` ni dispatch).
pub static J17_IMMEMO_HIT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// JS-C559 — insertions `immemo` (première résolution par
/// couple wrapper×clé — le volume de wrappers concernés).
pub static J17_IMMEMO_INS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Gate J17 : les compteurs ci-dessus ne s'incrémentent que si
/// `EXPLORER_J17_PROF` est posé (non vide/non "0") au premier
/// appel, ou si [`j17_enable`] a été appelé (tests/benchs). Un
/// `fetch_add` coûte ~10-20 ns chacun — des millions par éval
/// lourde taxaient toute la suite ; un `load`+branche ~1 ns.
static J17_ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static J17_ENV_ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// `true` si les compteurs J17 sont armés (env ou enable).
/// `pub` depuis l'étape 255 : la surface agent (`engine.j17`)
/// doit distinguer « désarmé » d'un snapshot à zéros.
#[inline]
pub fn j17_on() -> bool {
    J17_ENABLED.load(std::sync::atomic::Ordering::Relaxed)
        || *J17_ENV_ON.get_or_init(|| {
            std::env::var("EXPLORER_J17_PROF").is_ok_and(|v| !v.is_empty() && v != "0")
        })
}

/// Arme les compteurs J17 sans variable d'environnement
/// (bancs de test J17 — sinon les relevés restent à zéro).
/// Réglage diagnostic du seuil de deferral (tests) —
/// `usize::MAX` désarme le defer → parse entièrement éager.
/// Prod : jamais appelée (défaut 24).
#[doc(hidden)]
pub fn set_defer_min(n: usize) {
    crate::parser::DEFER_MIN_TOKS.store(n, std::sync::atomic::Ordering::Relaxed);
}

pub fn j17_enable() {
    J17_ENABLED.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Snapshot borné des compteurs V-2 (chantier VITESSE-JS) —
/// une ligne `k=v` pour la piste vol/banc, vide si tout est
/// à zéro. Lecture `Relaxed`, coût nul hors appel.
#[must_use]
pub fn j17_snapshot() -> String {
    let r = std::sync::atomic::Ordering::Relaxed;
    let pairs: [(&str, &std::sync::atomic::AtomicU64); 86] = [
        ("prop_lookup", &J17_PROP_LOOKUP),
        ("proto_lvl", &J17_PROTO_LVL),
        ("prop_scan", &J17_PROP_SCAN),
        ("prop_idx_hit", &J17_PROP_IDX_HIT),
        ("prop_idx_miss", &J17_PROP_IDX_MISS),
        ("prop_idx_build", &J17_PROP_IDX_BUILD),
        ("rabs_hit", &J17_RABS_HIT),
        ("rabs_ins", &J17_RABS_INS),
        ("rabs_full", &J17_RABS_FULL),
        ("env_lvl", &J17_ENV_LVL),
        ("bnd_canon", &J17_BND_CANON),
        ("bnd_extra", &J17_BND_EXTRA),
        ("xt_dyn", &J17_XT_DYN),
        ("xt_unw", &J17_XT_UNW),
        ("wire_hit", &J17_WIRE_HIT),
        ("wire_miss", &J17_WIRE_MISS),
        ("env_gets", &J17_ENV_GETS),
        ("res_hit", &J17_RES_HIT),
        ("res_miss", &J17_RES_MISS),
        ("ic_hit", &J17_IC_HIT),
        ("ic_miss", &J17_IC_MISS),
        ("ic_learn", &J17_IC_LEARN),
        ("ic_disarm", &J17_IC_DISARM),
        ("ic_bail", &J17_IC_BAIL),
        ("bail_kind", &J17_IC_BAIL_KIND),
        ("bail_dead", &J17_IC_BAIL_DEAD),
        ("ic_nlearn", &J17_IC_NLEARN),
        ("ic_purge", &J17_IC_PURGE),
        ("ic_poly", &J17_IC_POLY),
        ("env_ghit", &J17_ENV_GHIT),
        ("w_own", &J17_W_OWN),
        ("ic_acc", &J17_IC_ACC),
        ("ps_get", &J17_PS_GET),
        ("ps_has", &J17_PS_HAS),
        ("ps_own", &J17_PS_OWN),
        ("ps_set", &J17_PS_SET),
        ("pl_get", &J17_PL_GET),
        ("pl_has", &J17_PL_HAS),
        ("pl_own", &J17_PL_OWN),
        ("pl_set", &J17_PL_SET),
        ("slot_hit", &J17_SLOT_HIT),
        ("slot_miss", &J17_SLOT_MISS),
        ("sm_oob", &J17_SM_OOB),
        ("sm_key", &J17_SM_KEY),
        ("sm_cell", &J17_SM_CELL),
        ("sm_map", &J17_SM_MAP),
        ("sm_kind", &J17_SM_KIND),
        ("badd_num", &J17_BIN_ADD_NUM),
        ("badd_str", &J17_BIN_ADD_STR),
        ("badd_oth", &J17_BIN_ADD_OTH),
        ("bin_nn", &J17_BIN_NN),
        ("bin_ss", &J17_BIN_SS),
        ("bin_se", &J17_BIN_SE),
        ("res_clear", &J17_RES_CLEAR),
        ("arridx_try", &J17_ARRIDX_TRY),
        ("call", &J17_CALL),
        ("call_nat", &J17_CALL_NAT),
        ("str_read", &J17_STR_READ),
        ("probe_hit", &J17_PROBE_HIT),
        ("parse_call", &J17_PARSE_CALL),
        ("parse_dup", &J17_PARSE_DUP),
        ("lex_ns", &J17_LEX_NS),
        ("close_ns", &J17_CLOSE_NS),
        ("stmt_ns", &J17_STMT_NS),
        ("lex_tok", &J17_LEX_TOK),
        ("ast_stmt", &J17_AST_STMT),
        ("tok_clone", &J17_TOK_CLONE),
        ("par_next", &J17_PAR_NEXT),
        ("fn_defer", &J17_FN_DEFER),
        ("def_mat", &J17_DEFER_MAT),
        ("def_err", &J17_DEF_ERR),
        ("fn_def_tok", &J17_FN_DEF_TOK),
        ("fn_mat", &J17_FN_MAT),
        ("frame_ns", &J17_FRAME_NS),
        ("env_ns", &J17_ENV_NS),
        ("prop_ns", &J17_PROP_NS),
        ("mat_ns", &J17_MAT_NS),
        ("gc_ns", &J17_GC_NS),
        ("env_new", &J17_ENV_NEW),
        ("body_ns", &J17_BODY_NS),
        ("acc_ns", &J17_ACC_NS),
        ("acc_get", &J17_ACC_GET),
        ("host_ns", &J17_HOST_NS),
        ("host_get", &J17_HOST_GET),
        ("immemo_hit", &J17_IMMEMO_HIT),
        ("immemo_ins", &J17_IMMEMO_INS),
    ];
    let mut out = String::new();
    for (k, c) in pairs {
        let v = c.load(r);
        if v > 0 {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(&format!("{k}={v}"));
        }
    }
    // JS-C505 — histogramme `binary_op` en un seul champ
    // `binop:<nom>=<n>,…` (noms = ordre de déclaration `BinOp`).
    let mut bins = String::new();
    for (i, c) in J17_BINOP.iter().enumerate() {
        let v = c.load(r);
        if v > 0 {
            if !bins.is_empty() {
                bins.push(',');
            }
            bins.push_str(&format!("{}={v}", J17_BINOP_NAMES[i]));
        }
    }
    if !bins.is_empty() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&format!("binop:{bins}"));
    }
    // JS-C532 — histogrammes d'évaluation `eval:<variante>=<n>,…`
    // et `stmt:<variante>=<n>,…` (noms = ordre de déclaration
    // `Expr`/`Stmt` — le tissu non-scopé de `body_ns` devient
    // attribuable par variante).
    let mut evs = String::new();
    for (i, c) in J17_EVAL.iter().enumerate() {
        let v = c.load(r);
        if v > 0 {
            if !evs.is_empty() {
                evs.push(',');
            }
            evs.push_str(&format!("{}={v}", J17_EVAL_NAMES[i]));
        }
    }
    if !evs.is_empty() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&format!("eval:{evs}"));
    }
    let mut sts = String::new();
    for (i, c) in J17_STMT.iter().enumerate() {
        let v = c.load(r);
        if v > 0 {
            if !sts.is_empty() {
                sts.push(',');
            }
            sts.push_str(&format!("{}={v}", J17_STMT_NAMES[i]));
        }
    }
    if !sts.is_empty() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&format!("stmt:{sts}"));
    }
    // JS-C513 — top-12 callsites `find_own_ai` (`foai:f:l=n,…`).
    let mut foai: Vec<(u64, String)> =
        J17_FOAI_SITES.with(|m| m.borrow().values().map(|(l, n)| (*n, l.clone())).collect());
    foai.sort_unstable_by_key(|a| std::cmp::Reverse(a.0));
    if !foai.is_empty() {
        let joined = foai
            .iter()
            .take(12)
            .map(|(n, l)| format!("{l}={n}"))
            .collect::<Vec<_>>()
            .join(",");
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&format!("foai:{joined}"));
    }
    // JS-C515 — top-12 callsites `find_own` (`fown:f:l=n,…`).
    let mut fown: Vec<(u64, String)> =
        J17_FOWN_SITES.with(|m| m.borrow().values().map(|(l, n)| (*n, l.clone())).collect());
    fown.sort_unstable_by_key(|a| std::cmp::Reverse(a.0));
    if !fown.is_empty() {
        let joined = fown
            .iter()
            .take(12)
            .map(|(n, l)| format!("{l}={n}"))
            .collect::<Vec<_>>()
            .join(",");
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&format!("fown:{joined}"));
    }
    // JS-C534c — top-12 callsites de construction d'envs
    // servant en `extra` (`envu:f:l=n,…`).
    let mut envu: Vec<(u64, String)> =
        J17_ENVU_SITES.with(|m| m.borrow().values().map(|(l, n)| (*n, l.clone())).collect());
    envu.sort_unstable_by_key(|a| std::cmp::Reverse(a.0));
    if !envu.is_empty() {
        let joined = envu
            .iter()
            .take(12)
            .map(|(n, l)| format!("{l}={n}"))
            .collect::<Vec<_>>()
            .join(",");
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&format!("envu:{joined}"));
    }
    // JS-C539 — top-12 callsites des essais `array_index`
    // (`arridx:f:l=n,…`).
    let mut arridx: Vec<(u64, String)> =
        J17_ARRIDX_SITES.with(|m| m.borrow().values().map(|(l, n)| (*n, l.clone())).collect());
    arridx.sort_unstable_by_key(|a| std::cmp::Reverse(a.0));
    if !arridx.is_empty() {
        let joined = arridx
            .iter()
            .take(12)
            .map(|(n, l)| format!("{l}={n}"))
            .collect::<Vec<_>>()
            .join(",");
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&format!("arridx:{joined}"));
    }
    // JS-C542 — top-12 callsites des comparaisons `prop_scan`
    // (`pscan:f:l=n,…`) — tranche la part écriture du compteur.
    let mut pscan: Vec<(u64, String)> =
        J17_PSCAN_SITES.with(|m| m.borrow().values().map(|(l, n)| (*n, l.clone())).collect());
    pscan.sort_unstable_by_key(|a| std::cmp::Reverse(a.0));
    if !pscan.is_empty() {
        let joined = pscan
            .iter()
            .take(12)
            .map(|(n, l)| format!("{l}={n}"))
            .collect::<Vec<_>>()
            .join(",");
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&format!("pscan:{joined}"));
    }
    // JS-C543 — top-12 callsites des miss `prop_idx_miss`
    // (`pidxm:f:l=n,…`).
    let mut pidxm: Vec<(u64, String)> =
        J17_PIDXM_SITES.with(|m| m.borrow().values().map(|(l, n)| (*n, l.clone())).collect());
    pidxm.sort_unstable_by_key(|a| std::cmp::Reverse(a.0));
    if !pidxm.is_empty() {
        let joined = pidxm
            .iter()
            .take(12)
            .map(|(n, l)| format!("{l}={n}"))
            .collect::<Vec<_>>()
            .join(",");
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&format!("pidxm:{joined}"));
    }
    // JS-C549 — top-12 noms des lectures d'identifiant non
    // annotées (`idnt:nom=n,…`) — nomme la population qui
    // paie la marche `env_get`.
    let mut idnt: Vec<(u64, String)> = J17_IDNT_NAMES.with(|m| {
        m.borrow()
            .iter()
            .map(|(k, n)| (*n, k.to_string()))
            .collect()
    });
    idnt.sort_unstable_by_key(|a| std::cmp::Reverse(a.0));
    if !idnt.is_empty() {
        let joined = idnt
            .iter()
            .take(12)
            .map(|(n, l)| format!("{l}={n}"))
            .collect::<Vec<_>>()
            .join(",");
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&format!("idnt:{joined}"));
    }
    // JS-C550 — top-12 callsites `env_get` (`envg:f:l=n,…`).
    let mut envg: Vec<(u64, String)> =
        J17_ENVG_SITES.with(|m| m.borrow().values().map(|(l, n)| (*n, l.clone())).collect());
    envg.sort_unstable_by_key(|a| std::cmp::Reverse(a.0));
    if !envg.is_empty() {
        let joined = envg
            .iter()
            .take(12)
            .map(|(n, l)| format!("{l}={n}"))
            .collect::<Vec<_>>()
            .join(",");
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&format!("envg:{joined}"));
    }
    // JS-C527 — top-20 clés marchées (`keys:k=n,…`) ; noms
    // assainis (hors `[A-Za-z0-9_$@.:-]` → `?`, tronqués à 24)
    // pour rester sur une seule ligne de paires.
    let mut keys: Vec<(u64, std::rc::Rc<str>)> =
        J17_KEY_WALK.with(|m| m.borrow().iter().map(|(k, n)| (*n, k.clone())).collect());
    keys.sort_unstable_by_key(|a| std::cmp::Reverse(a.0));
    if !keys.is_empty() {
        let joined = keys
            .iter()
            .take(20)
            .map(|(n, k)| {
                let s: String = k
                    .chars()
                    .take(24)
                    .map(|c| {
                        if c.is_ascii_alphanumeric()
                            || matches!(c, '_' | '$' | '@' | '.' | ':' | '-')
                        {
                            c
                        } else {
                            '?'
                        }
                    })
                    .collect();
                format!("{s}={n}")
            })
            .collect::<Vec<_>>()
            .join(",");
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&format!("keys:{joined}"));
    }
    // JS-C558 — top-12 dispatches `Host` (`host:class:key=n,…`)
    // sous `EXPLORER_J17_HOST=1` — nomme les getters/invokes
    // DOM dominants de `host_ns`.
    let mut host: Vec<(u64, String)> =
        J17_HOST_ATTR.with(|m| m.borrow().iter().map(|(k, n)| (*n, k.clone())).collect());
    host.sort_unstable_by_key(|a| std::cmp::Reverse(a.0));
    if !host.is_empty() {
        let joined = host
            .iter()
            .take(40)
            .map(|(n, l)| format!("{l}={n}"))
            .collect::<Vec<_>>()
            .join(",");
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&format!("host:{joined}"));
    }
    // JS-C560 — `hns:class:key=ns@n` — paire temporelle de
    // `host:` (top-40 par ns cumulés, jointure directe avec
    // le compte `host:` de la même table TLS → ns/get lisible
    // par entrée, sans dépendre des deux cutoffs top-40).
    let mut hns: Vec<(u64, String)> =
        J17_HOST_NS_ATTR.with(|m| m.borrow().iter().map(|(k, n)| (*n, k.clone())).collect());
    hns.sort_unstable_by_key(|a| std::cmp::Reverse(a.0));
    if !hns.is_empty() {
        let joined = J17_HOST_ATTR.with(|m| {
            let counts = m.borrow();
            hns.iter()
                .take(40)
                .map(|(n, l)| {
                    let c = counts.get(l).copied().unwrap_or(0);
                    format!("{l}={n}@{c}")
                })
                .collect::<Vec<_>>()
                .join(",")
        });
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&format!("hns:{joined}"));
    }
    out
}

/// Incrément borné d'un compteur J17 — no-op hors mode profil.
#[inline]
pub(crate) fn j17_tick(c: &'static std::sync::atomic::AtomicU64) {
    if j17_on() {
        c.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

thread_local! {
    /// Hashes des sources déjà parsées — rempli uniquement sous
    /// `j17_on()` (sonde V-3c), jamais lu hors de la sonde.
    static J17_PARSE_SEEN: std::cell::RefCell<std::collections::HashSet<u64>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
    /// JS-C513 — attribution `find_own_ai` par callsite interne :
    /// `Location::caller()` (`#[track_caller]`) → `file:line`
    /// lisible + volume. Rempli sous `j17_on()` uniquement —
    /// les callsites sont des `&'static` bornés (quelques
    /// centaines max).
    static J17_FOAI_SITES: std::cell::RefCell<
        std::collections::BTreeMap<usize, (String, u64)>,
    > = const { std::cell::RefCell::new(std::collections::BTreeMap::new()) };
    /// JS-C515 — même attribution un cran plus haut : `find_own`
    /// (le wrapper `foai:7325` = 46 % du mur) par *son* callsite —
    /// nomme la builtin/interne dominante avant la fusion
    /// `hasOwn`+`get` décidée au vol N5.14.
    static J17_FOWN_SITES: std::cell::RefCell<
        std::collections::BTreeMap<usize, (String, u64)>,
    > = const { std::cell::RefCell::new(std::collections::BTreeMap::new()) };
    /// JS-C527 — histogramme des **clés** marchées par
    /// `find_prop_owner_links` (`keys:` du snapshot) : nomme les
    /// lookups dominants (`hasOwnProperty`, `constructor`, …)
    /// avant toute décision de fast-path. **Opt-in**
    /// (`EXPLORER_J17_KEYS=1` — voir [`j17_keys_on`]) : un insert
    /// par lookup coûterait ~le mur qu'il mesure.
    static J17_KEY_WALK: std::cell::RefCell<
        std::collections::HashMap<std::rc::Rc<str>, u64>,
    > = std::cell::RefCell::new(std::collections::HashMap::new());
    /// JS-C534c — `envu:` — hits `extra` attribués au **callsite
    /// de construction** de l'env (`#[track_caller]` des
    /// constructeurs `Env::*`). Deux hypothèses de localisation
    /// réfutées par N5.39/N5.40 (frames GExec, scopes gen/async)
    /// — cette sonde nomme la population en un vol au lieu de
    /// câbler à l'aveugle.
    static J17_ENVU_SITES: std::cell::RefCell<
        std::collections::BTreeMap<usize, (String, u64)>,
    > = const { std::cell::RefCell::new(std::collections::BTreeMap::new()) };
    /// JS-C539 — `arridx:` — essais `array_index` attribués au
    /// callsite (`#[track_caller]` sur `array_index`). Le
    /// compteur est intrinsèque depuis les hoists V-3b (un
    /// essai par `find_own` + kinds indexés) — cette sonde
    /// nomme le dominant restant (find, string-index, TA,
    /// builtins) en un vol.
    static J17_ARRIDX_SITES: std::cell::RefCell<
        std::collections::BTreeMap<usize, (String, u64)>,
    > = const { std::cell::RefCell::new(std::collections::BTreeMap::new()) };
    /// JS-C542 — `pscan:` — comparaisons `prop_scan` attribuées
    /// au callsite (`#[track_caller]` sur `prop_pos`). Nomme la
    /// part lecture (`o.find` via `find_own_ai`) vs écriture
    /// (`prop_pos` directs : own-first, learn, `set_prop`) du
    /// compteur au prochain vol lourd.
    static J17_PSCAN_SITES: std::cell::RefCell<
        std::collections::BTreeMap<usize, (String, u64)>,
    > = const { std::cell::RefCell::new(std::collections::BTreeMap::new()) };
    /// JS-C543 — `pidxm:` — miss `prop_idx_miss` attribués au
    /// callsite (même `Location` capturée à l'entrée de
    /// `prop_pos`). 927 k miss/session yt N5.50 : le vol lourd
    /// nomme le driver des lookups absents sur objets indexés.
    static J17_PIDXM_SITES: std::cell::RefCell<
        std::collections::BTreeMap<usize, (String, u64)>,
    > = const { std::cell::RefCell::new(std::collections::BTreeMap::new()) };
    /// JS-C549 — `idnt:` — lectures d'identifiant **non
    /// annotées** (miss `resolved_map` → marche `env_get`),
    /// attribuées par **nom** (clé `Rc<str>` internée —
    /// hit = probe de chaîne, intern au premier vu seulement).
    /// ~5 M marches/vol yt : quelques noms dominants
    /// (globals, props hôte) → cran ciblé ; dispersion →
    /// plancher structurel.
    static J17_IDNT_NAMES: std::cell::RefCell<
        std::collections::BTreeMap<std::rc::Rc<str>, u64>,
    > = const { std::cell::RefCell::new(std::collections::BTreeMap::new()) };
    /// JS-C550 — `envg:` — appels `env_get` attribués au
    /// callsite (`#[track_caller]`). `idnt:` vide sur yt a
    /// réfuté la piste des lectures d'identifiant : les ~3 M
    /// marches restantes viennent d'autres appelants — la
    /// sonde nomme le dominant (callee, typeof, assign,
    /// for-in, machinerie de frames) en un vol.
    static J17_ENVG_SITES: std::cell::RefCell<
        std::collections::BTreeMap<usize, (String, u64)>,
    > = const { std::cell::RefCell::new(std::collections::BTreeMap::new()) };
    /// JS-C558 — `host:` — dispatches `Host` attribués par
    /// `class:key` (gets + invokes + méthodes feature-detect
    /// passant par `host_mark`). Opt-in `EXPLORER_J17_HOST=1`,
    /// dump top-12 au teardown — nomme le dominant de
    /// `host_ns`.
    static J17_HOST_ATTR: std::cell::RefCell<
        std::collections::BTreeMap<String, u64>,
    > = const { std::cell::RefCell::new(std::collections::BTreeMap::new()) };
    /// JS-C560 — `hns:` — ns cumulés par `class:key` autour de
    /// `Host::get` (paire temporelle de `J17_HOST_ATTR`) :
    /// tranche « coût fixe par get » vs « corps lourds
    /// nommés » — `hns:key / host:key` = coût moyen par get.
    static J17_HOST_NS_ATTR: std::cell::RefCell<
        std::collections::BTreeMap<String, u64>,
    > = const { std::cell::RefCell::new(std::collections::BTreeMap::new()) };
}

/// `EXPLORER_J17_KEYS=1` — arme l'histogramme `keys:` en plus
/// de `j17_on` : à ~1,2 M de marches par vol, un insert
/// `HashMap` par clé gonflerait `prop_ns` de ~5-10 % — l'audit
/// tourne en vol dédié, jamais fondu aux mesures de mur.
static J17_KEYS_ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static J17_KEYS_ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// `true` si l'histogramme `keys:` est armé.
#[inline]
fn j17_keys_on() -> bool {
    J17_KEYS_ENABLED.load(std::sync::atomic::Ordering::Relaxed)
        || *J17_KEYS_ON.get_or_init(|| {
            std::env::var("EXPLORER_J17_KEYS").is_ok_and(|v| !v.is_empty() && v != "0")
        })
}

/// Arme l'histogramme `keys:` sans variable d'environnement
/// (bancs de test — sinon le relevé reste vide).
pub fn j17_keys_enable() {
    J17_KEYS_ENABLED.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// JS-C515 — compte une invocation `find_own` attribuée à son
/// callsite Rust (`fown:` du snapshot).
pub(crate) fn j17_fown_site(loc: &'static std::panic::Location<'static>) {
    if !j17_on() {
        return;
    }
    let site = std::ptr::from_ref(loc) as usize;
    J17_FOWN_SITES.with(|m| {
        let mut m = m.borrow_mut();
        let e = m
            .entry(site)
            .or_insert_with(|| (format!("{}:{}", loc.file(), loc.line()), 0));
        e.1 += 1;
    });
}

/// JS-C527 — compte une marche de chaîne attribuée à sa clé
/// (`keys:` du snapshot) — la clé internée est réutilisée telle
/// quelle (bump de refcount, zéro alloc). Opt-in
/// `EXPLORER_J17_KEYS` : désarmé, deux `load`+branche ~2 ns.
pub(crate) fn j17_key_walk(k: &std::rc::Rc<str>) {
    if !j17_on() || !j17_keys_on() {
        return;
    }
    J17_KEY_WALK.with(|m| *m.borrow_mut().entry(k.clone()).or_default() += 1);
}

/// JS-C513 — compte une invocation `find_own_ai` attribuée à son
/// callsite Rust (le résiduel `pl_*` 33 % du vol N5.11 est
/// anonyme : envs globaux, invariants proxy, JSON, host-shadow —
/// cette sonde le nomme avant tout cran supplémentaire).
pub(crate) fn j17_foai_site(loc: &'static std::panic::Location<'static>) {
    if !j17_on() {
        return;
    }
    let site = std::ptr::from_ref(loc) as usize;
    J17_FOAI_SITES.with(|m| {
        let mut m = m.borrow_mut();
        let e = m
            .entry(site)
            .or_insert_with(|| (format!("{}:{}", loc.file(), loc.line()), 0));
        e.1 += 1;
    });
}

/// JS-C534c — compte un hit `extra` attribué au callsite de
/// **construction** de l'env (`envu:` du snapshot) — nomme la
/// population d'envs non câblés.
pub(crate) fn j17_envu_site(loc: &'static std::panic::Location<'static>) {
    if !j17_on() {
        return;
    }
    let site = std::ptr::from_ref(loc) as usize;
    J17_ENVU_SITES.with(|m| {
        let mut m = m.borrow_mut();
        let e = m
            .entry(site)
            .or_insert_with(|| (format!("{}:{}", loc.file(), loc.line()), 0));
        e.1 += 1;
    });
}

/// JS-C539 — sonde `arridx:` : tick `arridx_try` + attribution
/// du callsite en un seul `j17_on` (miroir [`j17_envu_site`]).
pub(crate) fn j17_arridx_probe(loc: &'static std::panic::Location<'static>) {
    if !j17_on() {
        return;
    }
    J17_ARRIDX_TRY.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let site = std::ptr::from_ref(loc) as usize;
    J17_ARRIDX_SITES.with(|m| {
        let mut m = m.borrow_mut();
        let e = m
            .entry(site)
            .or_insert_with(|| (format!("{}:{}", loc.file(), loc.line()), 0));
        e.1 += 1;
    });
}

/// JS-C542 — sonde `pscan:` : tick `prop_scan` + attribution
/// du callsite en un seul `j17_on` (miroir [`j17_arridx_probe`]).
/// Appelée par comparaison dans [`crate::heap::Object::prop_pos`].
pub(crate) fn j17_pscan_probe(loc: &'static std::panic::Location<'static>) {
    if !j17_on() {
        return;
    }
    J17_PROP_SCAN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let site = std::ptr::from_ref(loc) as usize;
    J17_PSCAN_SITES.with(|m| {
        let mut m = m.borrow_mut();
        let e = m
            .entry(site)
            .or_insert_with(|| (format!("{}:{}", loc.file(), loc.line()), 0));
        e.1 += 1;
    });
}

/// JS-C549 — sonde `idnt:` : lecture d'identifiant dont le
/// site n'est **pas annoté** (`resolved_map` miss — repli
/// `env_get` dans `env_get_resolved_at`). Clé = le nom
/// (interné au premier vu — pas de `Location`, le `&str`
/// seul ne survit pas à l'AST). Armée seulement : ~10-20 ns
/// par lecture non résolue, compteurs seuls restent exacts.
/// JS-C550 — sonde `envg:` : attribution du callsite de chaque
/// `env_get` (miroir [`j17_envu_site`]) — nomme l'appelant
/// dominant des marches d'environnement.
pub(crate) fn j17_envg_probe(loc: &'static std::panic::Location<'static>) {
    if !j17_on() {
        return;
    }
    let site = std::ptr::from_ref(loc) as usize;
    J17_ENVG_SITES.with(|m| {
        let mut m = m.borrow_mut();
        let e = m
            .entry(site)
            .or_insert_with(|| (format!("{}:{}", loc.file(), loc.line()), 0));
        e.1 += 1;
    });
}

pub(crate) fn j17_idnt_probe(name: &str) {
    if !j17_on() {
        return;
    }
    J17_IDNT_NAMES.with(|m| {
        let mut m = m.borrow_mut();
        if let Some(c) = m.get_mut(name) {
            *c += 1;
        } else {
            m.insert(crate::intern(name), 1);
        }
    });
}

/// JS-C543 — sonde `pidxm:` : tick `prop_idx_miss` +
/// attribution du callsite en un seul `j17_on` (miroir
/// [`j17_pscan_probe`]). Appelée par miss d'index dans
/// [`crate::heap::Object::prop_pos`].
pub(crate) fn j17_pidxm_probe(loc: &'static std::panic::Location<'static>) {
    if !j17_on() {
        return;
    }
    J17_PROP_IDX_MISS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let site = std::ptr::from_ref(loc) as usize;
    J17_PIDXM_SITES.with(|m| {
        let mut m = m.borrow_mut();
        let e = m
            .entry(site)
            .or_insert_with(|| (format!("{}:{}", loc.file(), loc.line()), 0));
        e.1 += 1;
    });
}

/// Sonde V-3c à l'entrée de `parse_in`/`parse_module_in` :
/// `parse_call` à chaque appel, `parse_dup` si le hash
/// (source + mode) est déjà connu. Le hash FNV-1a n'est
/// calculé qu'armé — désarmé, un `load`+branche ~1 ns.
pub(crate) fn j17_parse_probe(src: &str, module: bool) {
    if !j17_on() {
        return;
    }
    J17_PARSE_CALL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    // FNV-1a 64 — zéro dépendance ; le mode entre dans la
    // racine du hash (précondition V-3c : script ≠ module).
    let mut h: u64 = 0xcbf2_9ce4_8422_2325 ^ u64::from(module);
    for b in src.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    let dup = J17_PARSE_SEEN.with(|s| !s.borrow_mut().insert(h));
    if dup {
        J17_PARSE_DUP.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Addition bornée sur un compteur J17 — no-op hors mode profil.
#[inline]
pub(crate) fn j17_add(c: &'static std::sync::atomic::AtomicU64, n: u64) {
    if j17_on() {
        c.fetch_add(n, std::sync::atomic::Ordering::Relaxed);
    }
}

/// V-6 — exécute `f` en cumulant son temps (ns) dans `c`,
/// armé seulement ; désarmé = `load`+branche puis appel direct
/// (~1-2 ns, aucun `Instant` pris). Les phases du parse
/// appellent ceci au lieu de chronométrer à la main.
pub(crate) fn j17_time<T>(c: &'static std::sync::atomic::AtomicU64, f: impl FnOnce() -> T) -> T {
    if j17_on() {
        let t = std::time::Instant::now();
        let r = f();
        c.fetch_add(
            u64::try_from(t.elapsed().as_nanos()).unwrap_or(u64::MAX),
            std::sync::atomic::Ordering::Relaxed,
        );
        r
    } else {
        f()
    }
}

/// V-7 — garde RAII chronométrant un span jusqu'à son `Drop` :
/// permet de mesurer des zones **non contiguës** (prologue +
/// épilogue de `call_script_body` hors `exec_body`) et des
/// fonctions à sorties multiples. Désarmé = `Option<Instant>`
/// `None`, un `load`+branche à la création, rien au drop.
pub(crate) struct J17Scope {
    c: &'static std::sync::atomic::AtomicU64,
    t: Option<std::time::Instant>,
}

/// Arme un [`J17Scope`] sur `c` — à poser en `let _g = …` en
/// tête de zone, ou `drop(g)` explicite pour fermer un span.
pub(crate) fn j17_scope(c: &'static std::sync::atomic::AtomicU64) -> J17Scope {
    J17Scope {
        c,
        t: j17_on().then(std::time::Instant::now),
    }
}

impl Drop for J17Scope {
    fn drop(&mut self) {
        if let Some(t) = self.t {
            self.c.fetch_add(
                u64::try_from(t.elapsed().as_nanos()).unwrap_or(u64::MAX),
                std::sync::atomic::Ordering::Relaxed,
            );
        }
    }
}

/// §B-59 — dernière méthode/prop `Host` dispatchée
/// (`class:name`) : les `h.invoke`/`h.get`/`iter_items`…
/// passent **hors** `call_value_impl` — `calls`/`nat` muets,
/// le span natif hôte est nommé ici.
pub static LAST_HOST: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// JS-C566 — breadcrumb `host=` paresseux : l'écriture
/// `Mutex<String>` ne court que si un lecteur est abonné
/// (flight recorder du daemon, armé via [`host_mark_enable`]).
/// Désarmé : un `load` atomique par appel au lieu du
/// lock+format sur tout `Host::get`/`invoke` — le marqueur
/// sert aux post-mortem, pas au chemin chaud.
static HOST_MARK_ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Arme l'écriture du breadcrumb `host=` (daemon : à appeler
/// au démarrage du flight recorder — process-wide).
pub fn host_mark_enable() {
    HOST_MARK_ENABLED.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Écrit `class:name` dans [`LAST_HOST`] (diagnostic §B-59).
pub fn host_mark(class: &str, name: &str) {
    if HOST_MARK_ENABLED.load(std::sync::atomic::Ordering::Relaxed) {
        if let Ok(mut g) = LAST_HOST.lock() {
            g.clear();
            g.push_str(class);
            g.push(':');
            g.push_str(name);
        }
    }
    // JS-C558 — attribution `host:class:name=n` sous
    // `EXPLORER_J17_HOST=1` (dump `host:` au teardown) :
    // nomme les getters/invocations DOM dominants dans
    // `host_ns` (vol n558 : ~39 % de `prop_ns`).
    if j17_host() {
        J17_HOST_ATTR.with(|m| {
            *m.borrow_mut().entry(format!("{class}:{name}")).or_insert(0) += 1;
        });
    }
}

/// JS-C560 — cumule `ns` dans `hns:` (`class:name`) autour de
/// `Host::get` — la paire temporelle de [`host_mark`]. Opt-in
/// `EXPLORER_J17_HOST=1` ; le chrono lui-même (~20-40 ns/get)
/// n'est payé que flag armé (effet d'observateur : les totaux
/// `hns:` incluent ce coût, l'attribution *relative* reste
/// valide).
pub(crate) fn host_ns_mark(class: &str, name: &str, ns: u64) {
    J17_HOST_NS_ATTR.with(|m| {
        *m.borrow_mut().entry(format!("{class}:{name}")).or_insert(0) += ns;
    });
}

/// §B-59 — dernier frame JS du tick (écrit sous
/// `EXPLORER_JS_RSS_WATCH` uniquement — `format!` par tick est
/// trop cher hors diagnostic).
pub static LAST_FRAME: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// Sonde §B-111 — `EXPLORER_DLV_TRACE=1` logue chaque job
/// **avant** invocation dans les drains (microtasks, timers,
/// livraisons, GC de checkpoint) : la dernière ligne avant
/// un overflow de pile nomme la callback fautive — le
/// `.dmp` WER n'existe pas, la trace tient lieu de morgue
/// (pendant moteur de la sonde par-op session, JS-C401).
/// Désarmée : un booléen lu une fois.
pub(crate) fn dlv_trace() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("EXPLORER_DLV_TRACE").is_some())
}
/// Phase de l'appelant JS — **inconditionnelle** (store
/// `Relaxed` ~2 ns ; le `Mutex` coûtait ~20 ns × ~6 écritures
/// par appel dans le prologue — JS-C498) : écrite à l'entrée
/// de `call_value_impl` et `exec_body`, là où aucun tick n'a
/// encore couru. [`last_phase::NONE`] à l'abort = le span fatal
/// est dans une statement/éval tickée (couvert par
/// `LAST_STMT`) ou hors fil JS ; `call`/`body` = mort dans le
/// préambule d'appel (liage, hoist, suspend).
pub static LAST_PHASE: std::sync::atomic::AtomicU8 =
    std::sync::atomic::AtomicU8::new(last_phase::NONE);

/// Codes de [`LAST_PHASE`] — vocabulaire fixe des phases du
/// fil JS (le `Mutex<&'static str>` pesait ~20 ns par écriture
/// dans le prologue d'appel ; le code tient dans un `u8`).
pub mod last_phase {
    /// Aucune phase enregistrée.
    pub const NONE: u8 = 0;
    /// Préambule `call_value_impl` (délégations Bound/Proxy).
    pub const CALL: u8 = 1;
    /// Résolution du contexte/realm de la fonction (JS-C78).
    pub const CTX: u8 = 2;
    /// Switch transactionnel de contexte effectif (JS-C78).
    pub const SW: u8 = 3;
    /// Entrée `call_script_body`.
    pub const CSB: u8 = 4;
    /// Liage des paramètres (`bind_pat` × N, rest, défauts).
    pub const BIND: u8 = 5;
    /// Pose `arguments`/`new.target` (drapeaux JS-C497).
    pub const ARGV: u8 = 6;
    /// Hoist `var`/`function`/`export`.
    pub const HOIST: u8 = 7;
    /// Corps atteint — le span fatal est couvert par
    /// `LAST_STMT`/`LAST_EXPR`.
    pub const BODY: u8 = 8;
    /// Libellés breadcrumb — index = code (`NAMES[NONE]` = "").
    pub const NAMES: [&str; 9] = [
        "", "call", "ctx", "sw", "csb", "bind", "argv", "hoist", "body",
    ];
}

/// Écrit [`LAST_PHASE`] — ~2 ns, inconditionnelle (§B-59 : le
/// postmortem ne dépend d'aucun flag armé).
pub(crate) fn phase_note(code: u8) {
    LAST_PHASE.store(code, std::sync::atomic::Ordering::Relaxed);
}

/// Libellé de la phase enregistrée — `""` si aucune ou code
/// inconnu (lecteurs : journal breadcrumb, `rss_guard` daemon).
pub fn last_phase_str() -> &'static str {
    let c = LAST_PHASE.load(std::sync::atomic::Ordering::Relaxed) as usize;
    last_phase::NAMES.get(c).copied().unwrap_or("")
}
/// §B-59 — fonction `Fun::Script` en cours d'entrée (écrit au
/// début de `call_value_impl`, **avant** tout préambule) :
/// `last_phase=call` figé + `last_fun` nommé = la mort vit
/// dans le préambule d'appel de CETTE fonction (liage, hoist,
/// `arguments`, switch de realm) — zones que `last_stmt`/
/// `last_nat` ne couvrent pas.
pub static LAST_FUN: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// Étape 279 — balise GC en vol (wedge `XHR:responseText`
/// ~50 s, vol field274 : les balises `host`/`phase` sont
/// **périmées** pendant un mark/sweep — elles nomment le
/// dernier écrivain, pas le site). `calls` figé +
/// `gc_active=1` = passe GC en cours nommée ; `calls` figé +
/// `gc_active=0` = vrai spin dans le site nommé par
/// `host`/`phase`. ~2 ns, inconditionnelle.
pub static GC_ACTIVE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// Compteur de passes GC menées (mark `mark_live` ou sweep
/// `collect_bounded`) — s'il avance pendant un gel de
/// `calls`, l'interprète meurt de faim en GC.
pub static GC_PASSES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Durée de la dernière passe GC en ms — la borne
/// « passée lente » se mesure au postmortem.
pub static GC_LAST_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Étape 281 — lectures de `xhr.responseText`/`response`
/// texte : `reads` ≫ `decodes` = boucle de lecture nommée
/// (le vol field274 montrait un spin ~50 s dont ce compteur
/// aurait mesuré la forme).
pub static XHR_TEXT_READS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Décodages `from_utf8_lossy` réels du corps XHR — depuis
/// 281 le texte est caché : chaque livraison décode au plus
/// une fois, `decodes` borne le coût résiduel.
pub static XHR_TEXT_DECODES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Millisecondes cumulées dans le décodage `responseText`.
pub static XHR_TEXT_DECODE_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

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
                    last_phase_str(),
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
