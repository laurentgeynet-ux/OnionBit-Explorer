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
mod lexer;
mod mediaq;
mod misc_apis;
mod module;
mod mutation;
mod observers;
mod parser;
mod promise;
mod regexp;
mod storage;
mod streams;
mod temporal;
mod timers;
mod typed_array;
mod value;
mod websocket;
mod worker;
mod xhr;

pub use context::CtxId;
pub use crypto::{
    CryptoOp, CryptoRequest, CryptoResponse, CryptoTransport, DigestAlgo, ImportFormat, KeyAlgo,
};
pub use dom::{
    CanvasOpRecord, CanvasState, Dom, JournalOp, MediaCmd, MediaElState, MediaReport, MediaState,
    NodeId, SerializeStats,
};
pub use dom_host::{
    CookieJar, DomRef, DynFrameReq, DynImgReq, DynLinkReq, DynScriptReq, EventProp, FormSubmit,
};
pub use error::{Error, Result};
pub use eventsource::{EsEvent, EsRequest, EsTransport};
pub use geom::{ElementGeom, GeoRect, GeometryMap, GeometrySlot};
pub use heap::{HeapStats, Host};
pub use idb::{CacheEntry, CacheRegistry, IdbDb, IdbIndexDef, IdbRegistry, IdbStore};
pub use interp::Engine;
pub use mediaq::MediaEnv;
pub use module::{is_bare_specifier, module_specifiers, rewrite_bare_specifier, ModuleLoader};
pub use storage::Storage;
pub use value::{ObjId, Value};
pub use websocket::{WsEventKind, WsOut, WsRequest, WsTransport};
pub use worker::{WorkerConfig, WorkerFactory, WorkerLoader};
pub use xhr::{FetchCredentials, FetchMode, XhrRequest, XhrResponse, XhrTransport};
