// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `indexedDB` + `caches` (Cache API) — JS-C72.
//!
//! Persistance : les registres vivent dans des `Rc<RefCell<…>>`
//! injectables — l'embedder peut les partager entre pages d'une
//! même origine ou les sérialiser (même couture que
//! `install_storage`). En mémoire par défaut, pas d'I/O.
//!
//! `IndexedDB` (surface async en `IDBRequest`, pas des promesses) :
//!
//! - `indexedDB.open(name, version?)` → `IDBOpenDBRequest` :
//!   au checkpoint microtask, la base est ouverte/créée ;
//!   `upgradeneeded` (`oldVersion`/`newVersion`, `result` = db,
//!   `transaction` = upgrade txn — seul contexte où
//!   `createObjectStore`/`createIndex` sont légaux) puis
//!   `success`. `deleteDatabase`, `databases`, `cmp`.
//! - `IDBDatabase` : `name`/`version`/`objectStoreNames`,
//!   `createObjectStore`/`deleteObjectStore` (upgrade txn
//!   uniquement), `transaction(stores, mode?)`, `close`.
//! - `IDBTransaction` : `objectStore`, `mode`, `db`, `error`,
//!   `oncomplete`/`onabort`/`onerror` — commit livré quand la
//!   file de requests de la txn est vide.
//! - `IDBObjectStore` : `put`/`add`/`get`/`getAll`/`getKey`/
//!   `getAllKeys`/`delete`/`clear`/`count`/`openCursor`,
//!   `createIndex`/`index`/`deleteIndex`, `keyPath`,
//!   `autoIncrement`, `indexNames` — inline keys via `keyPath`
//!   (chemin pointillé) + `autoIncrement` numérique, sinon
//!   clé explicite (out-of-line) ; `put`/`add` exigent une clé
//!   ou un `keyPath` (`DataError` spec).
//! - `IDBIndex` : `get`/`getAll`/`getAllKeys`/`count`/
//!   `openCursor`, `unique`/`multiEntry` honnêtes sur `put`
//!   (duplicate → `ConstraintError`).
//! - `IDBCursor`/`IDBCursorWithValue` : `key`/`primaryKey`/
//!   `value`, `continue(k?)`/`advance(n)`/`update`/`delete` —
//!   chaque pas ré-enfile une request livrée au checkpoint.
//! - `IDBKeyRange` : `only`/`lowerBound`/`upperBound`/`bound`,
//!   `includes(k)` ; clés `number`/`string`/`Array` —
//!   ordre total spec (nombre < chaîne < tableau, arrays par
//!   comparaison élémentaire).
//!
//! Cache API :
//!
//! - `caches` (`CacheStorage`) : `open(name)`/`has`/`delete`/
//!   `keys`/`match` en promesses.
//! - `Cache` : `put(request, response)`/`match`/`matchAll`/
//!   `keys`/`delete` ; `add(url)`/`addAll` passent par le
//!   transport fetch s'il est installé — sans transport la
//!   promesse rejette honnêtement. Corps copiés à l'écriture
//!   (snapshot de `String`, approximation documentée — la spec
//!   fige la `Response` au `put`).

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::rc::Rc;

use crate::heap::{Host, JsResult, ObjKind};
use crate::value::{ObjId, Value};
use crate::Engine;

/// Argument `args[i]` — `Undefined` hors rang.
fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).copied().unwrap_or(Value::Undefined)
}

// ------------------------------------------------------------------
// Registres partagés (injectables embedder)
// ------------------------------------------------------------------

/// Registre `IndexedDB` partagé — `Rc<RefCell>` injecté par
/// l'embedder (`Engine::install_idb`) : persistence entre
/// pages = sa responsabilité (même modèle que `localStorage`).
/// Les valeurs stockées sont des `Value` du moteur (structured
/// clone appliqué à l'écriture — les `ObjId` sont
/// engine-locaux : un registre partagé entre *engines*
/// exigerait une sérialisation, hors scope JS-C72).
pub type IdbRegistry = Rc<RefCell<HashMap<String, IdbDb>>>;

/// Base `indexedDB` — nom, version, stores.
pub struct IdbDb {
    /// `db.version`.
    pub version: u64,
    /// `db.objectStoreNames` → `IdbStore`.
    pub stores: HashMap<String, IdbStore>,
}

/// Object store — records `BTreeMap` ordonnés par clé spec.
pub struct IdbStore {
    /// `keyPath` (chemin pointillé `a.b.c`) ou clé out-of-line.
    pub key_path: Option<String>,
    /// `autoIncrement` — prochaine clé numérique allouée.
    pub auto_inc: bool,
    /// Compteur `autoIncrement` (spec : >= max clé + 1).
    pub next_key: f64,
    /// Enregistrements `clé → valeur` (clés `IdbKey` totales).
    pub records: BTreeMap<IdbKey, Value>,
    /// `indexNames` → définition.
    pub indexes: HashMap<String, IdbIndexDef>,
}

/// Définition d'index — entrées recalculées depuis les
/// records à la lecture (records peu nombreux en miroir :
/// pas de map secondaire — simple et toujours cohérent).
pub struct IdbIndexDef {
    /// `keyPath` de l'index.
    pub key_path: String,
    /// `unique` — rejet `ConstraintError` sur doublon.
    pub unique: bool,
    /// `multiEntry` — un record indexe chaque élément-array.
    pub multi: bool,
}

/// Registre Cache API partagé — nom de cache →
/// `(url → Response Value)`.
pub type CacheRegistry = Rc<RefCell<HashMap<String, BTreeMap<String, CacheEntry>>>>;

/// Entrée `Cache` — corps snapshoté + métadonnées de la
/// `Response` au `put` (status, headers, type, url).
#[derive(Clone)]
pub struct CacheEntry {
    /// `response.url`.
    pub url: String,
    /// Corps snapshot (octets).
    pub body: String,
    /// `status`/`statusText`.
    pub status: u16,
    /// Headers `(nom, valeur)` jointes.
    pub headers: Vec<(String, String)>,
}

// ------------------------------------------------------------------
// Clés IndexedDB
// ------------------------------------------------------------------

/// Clé `IndexedDB` — `number`/`string`/`Array` (ordre total
/// spec : nombres < chaînes < tableaux). `Date` ramené à son
/// timestamp numérique à l'entrée.
#[derive(Clone, PartialEq)]
pub enum IdbKey {
    /// `number` (NaN rejeté à la validation).
    Num(f64),
    /// `string` — ordre par code units UTF-16 approximé par
    /// l'ordre octet UTF-8 (cohérent pour BMP — documenté).
    Str(String),
    /// `Array` — comparaison élément par élément.
    Arr(Vec<IdbKey>),
}

impl PartialOrd for IdbKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(std::cmp::Ord::cmp(self, other))
    }
}
impl Ord for IdbKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.cmp_idb(other)
    }
}
impl Eq for IdbKey {}

impl IdbKey {
    /// Ordre total spec `IDBKey` : type d'abord (Num < Str <
    /// Arr), puis valeur (arrays : lexicographique puis
    /// longueur).
    fn cmp_idb(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        fn rank(k: &IdbKey) -> u8 {
            match k {
                IdbKey::Num(_) => 0,
                IdbKey::Str(_) => 1,
                IdbKey::Arr(_) => 2,
            }
        }
        let (a, b) = (rank(self), rank(other));
        if a != b {
            return a.cmp(&b);
        }
        match (self, other) {
            (IdbKey::Num(x), IdbKey::Num(y)) => x.partial_cmp(y).unwrap_or(Ordering::Equal),
            (IdbKey::Str(x), IdbKey::Str(y)) => x.cmp(y),
            (IdbKey::Arr(x), IdbKey::Arr(y)) => {
                for (u, v) in x.iter().zip(y.iter()) {
                    let c = u.cmp_idb(v);
                    if c != Ordering::Equal {
                        return c;
                    }
                }
                x.len().cmp(&y.len())
            }
            _ => Ordering::Equal,
        }
    }

    /// Rendu JS de la clé (`req.result`, `cursor.key`…).
    fn to_value(&self, e: &mut Engine) -> Value {
        match self {
            IdbKey::Num(n) => Value::Num(*n),
            IdbKey::Str(s) => e.str_val(s.clone()),
            IdbKey::Arr(items) => {
                let vals: Vec<Value> = items.iter().map(|k| k.to_value(e)).collect();
                e.new_array(vals)
            }
        }
    }
}

/// Convertit un `Value` JS en `IdbKey` — `Err` (`DataError`
/// spec) sur NaN, objet non-array, array circulaire non
/// supporté (profondeur bornée).
fn idb_key_of(e: &mut Engine, v: Value, depth: u32) -> Option<IdbKey> {
    match v {
        Value::Num(n) if !n.is_nan() => Some(IdbKey::Num(n)),
        Value::Str(_) => e.heap_string_of(v).map(IdbKey::Str),
        Value::Obj(id) if depth < 8 => {
            // `Date` → timestamp (own-prop `__time`) ; array → Arr.
            if let Some(n) = date_time_of(e, id) {
                return Some(IdbKey::Num(n));
            }
            let len = e.get_prop(id, "length")?;
            let n = e.to_number(len).ok()?;
            if !(0.0..=10_000.0).contains(&n) {
                return None;
            }
            let mut items = Vec::with_capacity(n as usize);
            for i in 0..n as u32 {
                let el = e.get_prop(id, &i.to_string()).unwrap_or(Value::Undefined);
                items.push(idb_key_of(e, el, depth + 1)?);
            }
            Some(IdbKey::Arr(items))
        }
        _ => None,
    }
}

/// Timestamp d'un `Date` — own-prop `__time` (le miroir pose
/// les dates comme objets ordinaires proto `Date.prototype`).
fn date_time_of(e: &mut Engine, id: ObjId) -> Option<f64> {
    match e.get_prop(id, "__time") {
        Some(Value::Num(n)) if !n.is_nan() => Some(n),
        _ => None,
    }
}

/// `keyPath` pointillé → valeur extraite (`a.b.c` sur objets
/// ordinaires — props natives ignorées).
fn key_path_value(e: &mut Engine, v: Value, path: &str) -> Option<Value> {
    let mut cur = v;
    for seg in path.split('.') {
        let Value::Obj(id) = cur else { return None };
        cur = e.get_prop(id, seg)?;
        if matches!(cur, Value::Undefined | Value::Null) {
            return None;
        }
    }
    Some(cur)
}

// ------------------------------------------------------------------
// État moteur + requests
// ------------------------------------------------------------------

/// État moteur IDB — protos paresseux + file des requests à
/// livrer au checkpoint microtask (modèle FileReader/observers).
#[derive(Default)]
pub(crate) struct IdbState {
    /// Registre injecté (`install_idb`) ou créé à la volée.
    pub(crate) reg: Option<IdbRegistry>,
    /// Registre `caches` injecté (`install_caches`).
    pub(crate) cache_reg: Option<CacheRegistry>,
    /// Requests `IDBRequest`/`IDBOpenDBRequest` en attente —
    /// FIFO, livrées au checkpoint.
    pub(crate) pending: VecDeque<ObjId>,
    /// Protos paresseux.
    db_proto: Option<ObjId>,
    txn_proto: Option<ObjId>,
    store_proto: Option<ObjId>,
    index_proto: Option<ObjId>,
    cursor_proto: Option<ObjId>,
    keyrange_proto: Option<ObjId>,
    req_proto: Option<ObjId>,
    cache_proto: Option<ObjId>,
    cachestorage_proto: Option<ObjId>,
}

impl IdbState {
    /// Racines GC : protos + requests pendantes (le trace du
    /// host couvre result/handlers/op).
    pub(crate) fn root_ids(&self, ids: &mut Vec<ObjId>) {
        for p in [
            self.db_proto,
            self.txn_proto,
            self.store_proto,
            self.index_proto,
            self.cursor_proto,
            self.keyrange_proto,
            self.req_proto,
            self.cache_proto,
            self.cachestorage_proto,
        ]
        .into_iter()
        .flatten()
        {
            ids.push(p);
        }
        ids.extend(self.pending.iter().copied());
        if let Some(reg) = &self.reg {
            for db in reg.borrow().values() {
                for s in db.stores.values() {
                    for v in s.records.values() {
                        crate::heap::push_value(ids, *v);
                    }
                }
            }
        }
        if let Some(reg) = &self.cache_reg {
            for c in reg.borrow().values() {
                for _ in c.values() {}
            }
        }
    }
}

/// Opération différée d'une request IDB — exécutée au
/// checkpoint (`deliver_idb_requests`).
enum IdbOp {
    /// `indexedDB.open(name, version)`.
    Open {
        /// Nom de base.
        name: String,
        /// Version demandée (`None` = courante ou 1).
        version: Option<u64>,
    },
    /// `indexedDB.deleteDatabase(name)`.
    DeleteDb {
        /// Nom de base.
        name: String,
    },
    /// `store.put/add(v, key?)` — `add` rejette les doublons.
    Put {
        /// `Rc` du store cible (reg + db + store).
        ctx: StoreCtx,
        /// Valeur (structured-clonée à l'appel).
        value: Value,
        /// Clé explicite ou extraite `keyPath`/`autoIncrement`.
        key: Option<IdbKey>,
        /// `add` → `ConstraintError` sur clé présente.
        add_only: bool,
        /// Wrappers des index à vérifier (`unique`).
        check_unique: bool,
    },
    /// `store.get/getAll/getKey/getAllKeys/count`.
    Query {
        /// Contexte store.
        ctx: StoreCtx,
        /// Requête (clé ou range — `None` = tout).
        range: Option<Range>,
        /// Ce que produit le résultat.
        what: QueryKind,
    },
    /// `index.get/getAll/getKey/getAllKeys/count` — filtre sur
    /// la clé d'index, lit via la clé primaire du record.
    QueryIdx {
        /// Contexte store parent.
        ctx: StoreCtx,
        /// Nom d'index.
        idx: String,
        /// Requête.
        range: Option<Range>,
        /// Forme du résultat.
        what: QueryKind,
    },
    /// `store.delete(key)`/`clear()`.
    Remove {
        /// Contexte store.
        ctx: StoreCtx,
        /// `None` = `clear()`.
        range: Option<Range>,
    },
    /// `store/index.openCursor(range?, dir)`.
    Cursor {
        /// Contexte (store ou index).
        ctx: CursorCtx,
        /// Range filtrant.
        range: Option<Range>,
        /// Direction (`next`/`prev`/`nextunique`/`prevunique`).
        dir: Dir,
    },
    /// `cursor.continue/advance/update/delete`.
    CursorStep {
        /// Wrapper cursor (`IdbCursorHost`).
        cursor: ObjId,
        /// Action.
        act: CursorAct,
    },
}

/// Cible d'un curseur — store ou index.
enum CursorCtx {
    /// `store.openCursor`.
    Store(StoreCtx),
    /// `index.openCursor` — `value` du cursor = record entier,
    /// `key` = clé d'index, `primaryKey` = clé du record ;
    /// `bool` = `openKeyCursor` (pas de `value`).
    Index(StoreCtx, String, bool),
}

/// Action de `cursor.continue`/`advance`/`update`/`delete`.
enum CursorAct {
    /// `continue(key?)` / `advance(n)` — `None` = pas de 1.
    Next(Option<IdbKey>, u32),
    /// `update(value)` — réécrit le record courant.
    Update(Value),
    /// `delete()` — retire le record courant.
    Delete,
}

/// Direction de curseur.
#[derive(Clone, Copy, PartialEq)]
enum Dir {
    /// `next` — croissant, doublons inclus.
    Next,
    /// `prev` — décroissant.
    Prev,
    /// `nextunique` — croissant sans doublons d'index.
    NextUnique,
    /// `prevunique`.
    PrevUnique,
}

/// Ce que produit une `Query`.
enum QueryKind {
    /// `get` → première valeur.
    One,
    /// `getAll` → toutes les valeurs.
    All,
    /// `getKey` → première clé.
    Key,
    /// `getAllKeys` → toutes les clés.
    AllKeys,
    /// `count` → nombre.
    Count,
}

/// Range de clés (`IDBKeyRange` ou clé ponctuelle).
struct Range {
    /// Borne basse + stricte.
    lower: Option<(IdbKey, bool)>,
    /// Borne haute + stricte.
    upper: Option<(IdbKey, bool)>,
}

impl Range {
    /// La clé appartient-elle au range ?
    fn contains(&self, k: &IdbKey) -> bool {
        if let Some((lo, open)) = &self.lower {
            let c = k.cmp_idb(lo);
            if c == std::cmp::Ordering::Less || (*open && c == std::cmp::Ordering::Equal) {
                return false;
            }
        }
        if let Some((hi, open)) = &self.upper {
            let c = k.cmp_idb(hi);
            if c == std::cmp::Ordering::Greater || (*open && c == std::cmp::Ordering::Equal) {
                return false;
            }
        }
        true
    }

    /// Range d'une clé seule (`get(k)`).
    fn only(k: IdbKey) -> Range {
        Range {
            lower: Some((k.clone(), false)),
            upper: Some((k, false)),
        }
    }
}

/// Contexte store — `reg` + nom de db + nom de store (résolu
/// à chaque livraison : le store peut avoir été supprimé).
#[derive(Clone)]
struct StoreCtx {
    /// Registre partagé.
    reg: IdbRegistry,
    /// Nom de la base.
    db: String,
    /// Nom du store.
    store: String,
    /// Transaction (rétro-lien `complete`/`abort`).
    txn: Option<Rc<RefCell<TxnData>>>,
}

/// Données d'une `IDBTransaction`.
struct TxnData {
    /// Mode (`readonly`/`readwrite`/`versionchange`).
    mode: String,
    /// Requests encore en file — à 0 → `complete`.
    pending: usize,
    /// `abort` appelé.
    aborted: bool,
    /// Handlers `oncomplete`/`onabort`/`onerror` (own-props du
    /// wrapper — lus au fire).
    wrapper: Option<ObjId>,
    /// `db` rétro-lien (prop `transaction.db`).
    db_wrapper: Value,
    /// `error` de la txn (premier error rencontré).
    error: Value,
}

/// Données d'une `IDBRequest`.
struct ReqData {
    /// `pending`/`done` (0/1 → prop `readyState`).
    done: bool,
    /// `result` (set à la livraison).
    result: Value,
    /// `error` (DOMException-ish : `{name, message}`).
    error: Value,
    /// Opération différée.
    op: Option<IdbOp>,
    /// `source` (store/index/indexedDB), `transaction`.
    source: Value,
    txn: Value,
}

/// Host `IDBRequest`/`IDBOpenDBRequest`.
struct IdbReqHost {
    d: Rc<RefCell<ReqData>>,
}

/// `ReqData` d'un `Value` request.
fn req_data(e: &Engine, v: Value) -> Option<Rc<RefCell<ReqData>>> {
    let Value::Obj(id) = v else { return None };
    match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::Host(h)) => h
            .clone()
            .as_any()
            .downcast_ref::<IdbReqHost>()
            .map(|r| r.d.clone()),
        _ => None,
    }
}

/// Nouvelle `IDBRequest` enfilée — `source`/`txn` props.
fn new_request(e: &mut Engine, op: IdbOp, source: Value, txn: Value) -> Value {
    let rp = req_proto(e);
    let d = Rc::new(RefCell::new(ReqData {
        done: false,
        result: Value::Undefined,
        error: Value::Null,
        op: Some(op),
        source,
        txn,
    }));
    let v = e.new_host_object(Rc::new(IdbReqHost { d }));
    if let Value::Obj(id) = v {
        if let Some(o) = e.heap.obj_mut(id) {
            o.proto = Value::Obj(rp);
        }
        e.idb.pending.push_back(id);
    }
    v
}

/// Erreur IDB (`DOMException` shape `{name, message}` — le
/// miroir n'a pas de vrai `DOMException` : approximation
/// documentée, `.name` est le champ que les apps testent).
fn idb_err(e: &mut Engine, name: &str, msg: &str) -> Value {
    let o = e.new_object();
    let n = e.str_val(name);
    e.set_own(o, "name", n, true);
    let m = e.str_val(msg);
    e.set_own(o, "message", m, true);
    Value::Obj(o)
}

/// Event IDB minimal `{type, target, oldVersion?, newVersion?}`.
fn idb_event(e: &mut Engine, ty: &str, target: Value, vers: Option<(u64, u64)>) -> Value {
    let o = e.new_object();
    let t = e.str_val(ty);
    e.set_own(o, "type", t, true);
    e.set_own(o, "target", target, true);
    e.set_own(o, "currentTarget", target, true);
    if let Some((old, new)) = vers {
        e.set_own(o, "oldVersion", Value::Num(old as f64), true);
        e.set_own(o, "newVersion", Value::Num(new as f64), true);
    }
    Value::Obj(o)
}

/// `fire_event` — `on<ty>` own-prop sur le wrapper (listeners
/// `addEventListener` non modélisés — `on*` suffit aux apps
/// IDB classiques ; documenté).
fn fire_idb(e: &mut Engine, wrapper: ObjId, ty: &str, vers: Option<(u64, u64)>) -> JsResult<()> {
    let key = format!("on{ty}");
    if let Some(f) = e.get_prop(wrapper, &key) {
        if e.is_callable(f) {
            let ev = idb_event(e, ty, Value::Obj(wrapper), vers);
            e.call_value(f, Value::Obj(wrapper), &[ev])?;
        }
    }
    Ok(())
}

// ------------------------------------------------------------------
// Protos paresseux
// ------------------------------------------------------------------

/// Proto taggé neuf.
fn new_tagged_proto(e: &mut Engine, tag: &str) -> ObjId {
    let p = e.new_object();
    let t = e.str_val(tag);
    e.set_own(p, crate::interp::SYM_TO_STRING_TAG, t, false);
    p
}

fn req_proto(e: &mut Engine) -> ObjId {
    if let Some(p) = e.idb.req_proto {
        return p;
    }
    let p = new_tagged_proto(e, "IDBRequest");
    e.idb.req_proto = Some(p);
    p
}

fn db_proto(e: &mut Engine) -> ObjId {
    if let Some(p) = e.idb.db_proto {
        return p;
    }
    let p = new_tagged_proto(e, "IDBDatabase");
    e.add_method(p, "createObjectStore", db_create_store);
    e.add_method(p, "deleteObjectStore", db_delete_store);
    e.add_method(p, "transaction", db_transaction);
    e.add_method(p, "close", db_close);
    e.idb.db_proto = Some(p);
    p
}

fn txn_proto(e: &mut Engine) -> ObjId {
    if let Some(p) = e.idb.txn_proto {
        return p;
    }
    let p = new_tagged_proto(e, "IDBTransaction");
    e.add_method(p, "objectStore", txn_object_store);
    e.add_method(p, "abort", txn_abort);
    e.add_method(p, "commit", txn_commit);
    e.idb.txn_proto = Some(p);
    p
}

fn store_proto(e: &mut Engine) -> ObjId {
    if let Some(p) = e.idb.store_proto {
        return p;
    }
    let p = new_tagged_proto(e, "IDBObjectStore");
    e.add_method(p, "get", store_get);
    e.add_method(p, "getAll", store_get_all);
    e.add_method(p, "getKey", store_get_key);
    e.add_method(p, "getAllKeys", store_get_all_keys);
    e.add_method(p, "count", store_count);
    e.add_method(p, "put", store_put);
    e.add_method(p, "add", store_add);
    e.add_method(p, "delete", store_delete);
    e.add_method(p, "clear", store_clear);
    e.add_method(p, "openCursor", store_open_cursor);
    e.add_method(p, "createIndex", store_create_index);
    e.add_method(p, "deleteIndex", store_delete_index);
    e.add_method(p, "index", store_index);
    e.idb.store_proto = Some(p);
    p
}

fn index_proto(e: &mut Engine) -> ObjId {
    if let Some(p) = e.idb.index_proto {
        return p;
    }
    let p = new_tagged_proto(e, "IDBIndex");
    // Mêmes natives que le store — le dispatch store/index
    // vit dans `query_native`/`IdbOp::QueryIdx`.
    e.add_method(p, "get", store_get);
    e.add_method(p, "getAll", store_get_all);
    e.add_method(p, "getKey", store_get_key);
    e.add_method(p, "getAllKeys", store_get_all_keys);
    e.add_method(p, "count", store_count);
    e.add_method(p, "openCursor", index_open_cursor);
    e.add_method(p, "openKeyCursor", index_open_key_cursor);
    e.idb.index_proto = Some(p);
    p
}

fn cursor_proto(e: &mut Engine) -> ObjId {
    if let Some(p) = e.idb.cursor_proto {
        return p;
    }
    let p = new_tagged_proto(e, "IDBCursorWithValue");
    e.add_method(p, "continue", cursor_continue);
    e.add_method(p, "advance", cursor_advance);
    e.add_method(p, "update", cursor_update);
    e.add_method(p, "delete", cursor_delete);
    e.idb.cursor_proto = Some(p);
    p
}

fn keyrange_proto(e: &mut Engine) -> ObjId {
    if let Some(p) = e.idb.keyrange_proto {
        return p;
    }
    let p = new_tagged_proto(e, "IDBKeyRange");
    e.add_method(p, "includes", kr_includes);
    e.idb.keyrange_proto = Some(p);
    p
}

// ------------------------------------------------------------------
// Hosts
// ------------------------------------------------------------------

/// Host `IDBDatabase` — identité par nom dans le registre
/// (le contenu vit dans le `Rc` partagé).
struct IdbDbHost {
    /// Registre partagé.
    reg: IdbRegistry,
    /// Nom de base.
    name: String,
    /// Transaction en cours de versionchange — seule elle
    /// autorise `createObjectStore`/`deleteObjectStore`
    /// (valeur du wrapper txn, `None` hors upgrade).
    upgrade_txn: RefCell<Value>,
}

/// Host `IDBTransaction` — porte `TxnData` partagée.
struct IdbTxnHost {
    /// Données txn.
    d: Rc<RefCell<TxnData>>,
    /// `Rc` pour objectStore (reg + db).
    reg: IdbRegistry,
    /// Nom de base.
    db_name: String,
    /// Stores autorisés (`transaction(['a','b'])`).
    stores: Vec<String>,
}

/// Host `IDBObjectStore` + `IDBIndex` — même shape (`index`
/// = nom d'index ou vide).
struct IdbStoreHost {
    /// Contexte (reg + db + store + txn).
    ctx: StoreCtx,
    /// Nom d'index — `None` = le store lui-même.
    index: Option<String>,
}

/// `IdbStoreHost` d'un `Value` store/index.
fn store_data(e: &Engine, v: Value) -> Option<(StoreCtx, Option<String>)> {
    let Value::Obj(id) = v else { return None };
    match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::Host(h)) => h
            .clone()
            .as_any()
            .downcast_ref::<IdbStoreHost>()
            .map(|s| (s.ctx.clone(), s.index.clone())),
        _ => None,
    }
}

/// État d'un `IDBCursor` — itère `(key, pkey, value)` sur une
/// liste matérialisée à l'ouverture (records figés — les apps
/// relisent, pas de mutation sous curseur en pratique).
struct CursorData {
    /// Entrées triées `(index_key, primary_key, record_val)`
    /// — `record_val` = `Undefined` pour `openKeyCursor`.
    items: VecDeque<(IdbKey, IdbKey, Value)>,
    /// Entrée courante `(index_key, pkey, value)`.
    current: Option<(IdbKey, IdbKey, Value)>,
    /// Contexte pour `update`/`delete`.
    ctx: StoreCtx,
    /// Direction (détermine `continue`).
    dir: Dir,
    /// `source` prop (store ou index wrapper).
    source: Value,
    /// Request du cursor — `continue`/`advance` re-tirent son
    /// `onsuccess` (spec : même request, pas une nouvelle).
    req: ObjId,
    /// `openKeyCursor` — `value` n'est pas exposé.
    key_only: bool,
}

/// Host `IDBCursor`/`IDBCursorWithValue`.
struct IdbCursorHost {
    d: RefCell<CursorData>,
}

/// Host `IDBKeyRange` — bornes + flags stricts.
struct IdbKrHost {
    /// Borne basse `(clé, open)`.
    lower: Option<(IdbKey, bool)>,
    /// Borne haute `(clé, open)`.
    upper: Option<(IdbKey, bool)>,
}

/// `IdbKrHost`/`IdbKey` d'un `Value` range — `Value` clé
/// directe acceptée partout où la spec attend une requête.
fn range_of(e: &mut Engine, v: Value) -> JsResult<Option<Range>> {
    if matches!(v, Value::Undefined | Value::Null) {
        return Ok(None);
    }
    if let Value::Obj(id) = v {
        if let Some(ObjKind::Host(h)) = e.heap.obj(id).map(|o| &o.kind) {
            if let Some(kr) = h.clone().as_any().downcast_ref::<IdbKrHost>() {
                return Ok(Some(Range {
                    lower: kr.lower.clone(),
                    upper: kr.upper.clone(),
                }));
            }
        }
    }
    match idb_key_of(e, v, 0) {
        Some(k) => Ok(Some(Range::only(k))),
        None => Err(e.type_error("IDB: invalid key/range argument")),
    }
}

/// Registre ou création paresseuse (sans `install_idb`
/// explicite — `install_dom` appelle `install_idb` avec un
/// registre neuf si l'embedder n'en a pas injecté).
fn reg(e: &mut Engine) -> IdbRegistry {
    e.idb
        .reg
        .get_or_insert_with(|| Rc::new(RefCell::new(HashMap::new())))
        .clone()
}

/// Registre `caches` paresseux.
fn cache_reg(e: &mut Engine) -> CacheRegistry {
    e.idb
        .cache_reg
        .get_or_insert_with(|| Rc::new(RefCell::new(HashMap::new())))
        .clone()
}

// ------------------------------------------------------------------
// indexedDB.open / deleteDatabase / databases / cmp
// ------------------------------------------------------------------

/// `new IDBKeyRange`-less statiques — `IDBKeyRange.only/
/// lowerBound/upperBound/bound`.
fn idb_keyrange_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    // `new IDBKeyRange()` — objet nu (la spec l'interdit mais
    // le ctor existe ; bornes vides = tout).
    let kp = keyrange_proto(e);
    let v = e.new_host_object(Rc::new(IdbKrHost {
        lower: None,
        upper: None,
    }));
    if let Value::Obj(id) = v {
        if let Some(o) = e.heap.obj_mut(id) {
            o.proto = Value::Obj(kp);
        }
    }
    let _ = args;
    Ok(v)
}

/// Fabrique un `IDBKeyRange` host.
fn kr_value(e: &mut Engine, lower: Option<(IdbKey, bool)>, upper: Option<(IdbKey, bool)>) -> Value {
    let kp = keyrange_proto(e);
    let v = e.new_host_object(Rc::new(IdbKrHost { lower, upper }));
    if let Value::Obj(id) = v {
        if let Some(o) = e.heap.obj_mut(id) {
            o.proto = Value::Obj(kp);
        }
    }
    v
}

/// `IDBKeyRange.only(v)`.
fn kr_only(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let Some(k) = idb_key_of(e, arg(args, 0), 0) else {
        return Err(e.type_error("IDBKeyRange.only: invalid key"));
    };
    Ok(kr_value(e, Some((k.clone(), false)), Some((k, false))))
}

/// `IDBKeyRange.lowerBound(v, open?)`.
fn kr_lower(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let Some(k) = idb_key_of(e, arg(args, 0), 0) else {
        return Err(e.type_error("IDBKeyRange.lowerBound: invalid key"));
    };
    let open = e.to_boolean(arg(args, 1));
    Ok(kr_value(e, Some((k, open)), None))
}

/// `IDBKeyRange.upperBound(v, open?)`.
fn kr_upper(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let Some(k) = idb_key_of(e, arg(args, 0), 0) else {
        return Err(e.type_error("IDBKeyRange.upperBound: invalid key"));
    };
    let open = e.to_boolean(arg(args, 1));
    Ok(kr_value(e, None, Some((k, open))))
}

/// `IDBKeyRange.bound(lo, hi, loOpen?, upOpen?)`.
fn kr_bound(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let (Some(lo), Some(hi)) = (
        idb_key_of(e, arg(args, 0), 0),
        idb_key_of(e, arg(args, 1), 0),
    ) else {
        return Err(e.type_error("IDBKeyRange.bound: invalid keys"));
    };
    let lo_open = e.to_boolean(arg(args, 2));
    let up_open = e.to_boolean(arg(args, 3));
    Ok(kr_value(e, Some((lo, lo_open)), Some((hi, up_open))))
}

/// `kr.includes(k)`.
fn kr_includes(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = this else {
        return Err(e.type_error("includes on incompatible receiver"));
    };
    let Some(ObjKind::Host(h)) = e.heap.obj(id).map(|o| &o.kind) else {
        return Err(e.type_error("includes on incompatible receiver"));
    };
    let h = h.clone();
    let Some(kr) = h.as_any().downcast_ref::<IdbKrHost>() else {
        return Err(e.type_error("includes on incompatible receiver"));
    };
    let Some(k) = idb_key_of(e, arg(args, 0), 0) else {
        return Err(e.type_error("includes: invalid key"));
    };
    let r = Range {
        lower: kr.lower.clone(),
        upper: kr.upper.clone(),
    };
    Ok(Value::Bool(r.contains(&k)))
}

/// `indexedDB.open(name, version?)` → request livrée au
/// checkpoint (`upgradeneeded` puis `success`).
fn idb_open(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let name = e.to_js_string(arg(args, 0))?;
    if name.is_empty() {
        return Err(e.type_error("indexedDB.open: name required"));
    }
    let version = match arg(args, 1) {
        Value::Undefined => None,
        v => {
            let n = e.to_number(v)?;
            if !(n >= 1.0 && n.fract() == 0.0) {
                return Err(e.type_error("indexedDB.open: version must be >= 1"));
            }
            Some(n as u64)
        }
    };
    Ok(new_request(
        e,
        IdbOp::Open { name, version },
        Value::Obj(e.global),
        Value::Null,
    ))
}

/// `indexedDB.deleteDatabase(name)` → request.
fn idb_delete_db(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let name = e.to_js_string(arg(args, 0))?;
    Ok(new_request(
        e,
        IdbOp::DeleteDb { name },
        Value::Obj(e.global),
        Value::Null,
    ))
}

/// `indexedDB.databases()` → promesse `[{name, version}]`.
fn idb_databases(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    let reg = reg(e);
    let mut out = Vec::new();
    for (name, db) in reg.borrow().iter() {
        let o = e.new_object();
        let n = e.str_val(name.clone());
        e.set_own(o, "name", n, true);
        e.set_own(o, "version", Value::Num(db.version as f64), true);
        out.push(Value::Obj(o));
    }
    let arr = e.new_array(out);
    let pid = e.new_promise();
    e.resolve_promise(pid, arr);
    Ok(Value::Obj(pid))
}

/// `indexedDB.cmp(a, b)` → -1/0/1 (ordre clés spec).
fn idb_cmp(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let (Some(a), Some(b)) = (
        idb_key_of(e, arg(args, 0), 0),
        idb_key_of(e, arg(args, 1), 0),
    ) else {
        return Err(e.type_error("indexedDB.cmp: invalid keys"));
    };
    Ok(Value::Num(match a.cmp_idb(&b) {
        std::cmp::Ordering::Less => -1.0,
        std::cmp::Ordering::Equal => 0.0,
        std::cmp::Ordering::Greater => 1.0,
    }))
}

// ------------------------------------------------------------------
// IDBDatabase
// ------------------------------------------------------------------

/// `db.createObjectStore(name, {keyPath?, autoIncrement?})` —
/// upgrade txn uniquement (spec : `InvalidStateError` sinon).
fn db_create_store(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Some((reg, name, upgrade)) = db_host(e, this) else {
        return Err(e.type_error("createObjectStore on incompatible receiver"));
    };
    if matches!(upgrade, Value::Undefined) {
        return Err(e.type_error("createObjectStore outside an upgrade transaction"));
    }
    let sname = e.to_js_string(arg(args, 0))?;
    if sname.is_empty() {
        return Err(e.type_error("createObjectStore: name required"));
    }
    let opts = arg(args, 1);
    let (mut kp, mut ai) = (None, false);
    if let Value::Obj(oid) = opts {
        kp = e
            .get_prop(oid, "keyPath")
            .and_then(|v| e.heap_string_of(v))
            .filter(|s| !s.is_empty());
        ai = e
            .get_prop(oid, "autoIncrement")
            .is_some_and(|v| e.to_boolean(v));
    }
    {
        let mut regb = reg.borrow_mut();
        let Some(db) = regb.get_mut(&name) else {
            return Err(e.type_error("createObjectStore: database gone"));
        };
        if db.stores.contains_key(&sname) {
            return Err(e.type_error("createObjectStore: store already exists"));
        }
        db.stores.insert(
            sname.clone(),
            IdbStore {
                key_path: kp,
                auto_inc: ai,
                next_key: 1.0,
                records: BTreeMap::new(),
                indexes: HashMap::new(),
            },
        );
    }
    // Retourne le wrapper store lié à la txn d'upgrade.
    let txn = store_ctx_txn(e, upgrade);
    Ok(store_value(
        e,
        &StoreCtx {
            reg,
            db: name,
            store: sname,
            txn,
        },
        None,
    ))
}

/// `TxnData` d'un wrapper `IDBTransaction` — `None` sinon.
fn txn_data(e: &Engine, v: Value) -> Option<Rc<RefCell<TxnData>>> {
    let Value::Obj(id) = v else { return None };
    match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::Host(h)) => h
            .clone()
            .as_any()
            .downcast_ref::<IdbTxnHost>()
            .map(|t| t.d.clone()),
        _ => None,
    }
}

/// `txn: Option<Rc<TxnData>>` d'un wrapper — helper store.
fn store_ctx_txn(e: &mut Engine, v: Value) -> Option<Rc<RefCell<TxnData>>> {
    txn_data(e, v)
}

/// `db.deleteObjectStore(name)` — upgrade txn uniquement.
fn db_delete_store(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Some((reg, name, upgrade)) = db_host(e, this) else {
        return Err(e.type_error("deleteObjectStore on incompatible receiver"));
    };
    if matches!(upgrade, Value::Undefined) {
        return Err(e.type_error("deleteObjectStore outside an upgrade transaction"));
    }
    let sname = e.to_js_string(arg(args, 0))?;
    let mut regb = reg.borrow_mut();
    let Some(db) = regb.get_mut(&name) else {
        return Err(e.type_error("deleteObjectStore: database gone"));
    };
    if db.stores.remove(&sname).is_none() {
        return Err(e.type_error("deleteObjectStore: no such store"));
    }
    Ok(Value::Undefined)
}

/// `db.transaction(stores, mode?)` → `IDBTransaction`.
fn db_transaction(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Some((reg, name, _)) = db_host(e, this) else {
        return Err(e.type_error("transaction on incompatible receiver"));
    };
    let mut names = Vec::new();
    match arg(args, 0) {
        Value::Str(_) => names.push(e.to_js_string(arg(args, 0))?),
        v @ Value::Obj(_) => {
            for el in e.iter_values(v)? {
                names.push(e.to_js_string(el)?);
            }
        }
        _ => {}
    }
    if names.is_empty() {
        return Err(e.type_error("transaction: no stores given"));
    }
    {
        let regb = reg.borrow();
        if let Some(db) = regb.get(&name) {
            for n in &names {
                if !db.stores.contains_key(n) {
                    return Err(e.type_error(format!("transaction: no such store {n:?}")));
                }
            }
        }
    }
    let mode = match arg(args, 1) {
        Value::Undefined => "readonly".to_string(),
        v => e.to_js_string(v)?,
    };
    Ok(txn_value(e, reg, name, names, mode, this))
}

/// `db.close()` — no-op honnête (connexions non traquées).
fn db_close(_e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    Ok(Value::Undefined)
}

/// `(reg, db_name, upgrade_txn)` d'un `Value` `IDBDatabase`.
fn db_host(e: &Engine, v: Value) -> Option<(IdbRegistry, String, Value)> {
    let Value::Obj(id) = v else { return None };
    match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::Host(h)) => h
            .clone()
            .as_any()
            .downcast_ref::<IdbDbHost>()
            .map(|d| (d.reg.clone(), d.name.clone(), *d.upgrade_txn.borrow())),
        _ => None,
    }
}

/// Wrapper `IDBDatabase`.
fn db_value(e: &mut Engine, reg: &IdbRegistry, name: &str, version: u64) -> Value {
    let dp = db_proto(e);
    let v = e.new_host_object(Rc::new(IdbDbHost {
        reg: reg.clone(),
        name: name.to_string(),
        upgrade_txn: RefCell::new(Value::Undefined),
    }));
    if let Value::Obj(id) = v {
        if let Some(o) = e.heap.obj_mut(id) {
            o.proto = Value::Obj(dp);
        }
        let n = e.str_val(name);
        e.set_own(id, "name", n, true);
        e.set_own(id, "version", Value::Num(version as f64), true);
        let names: Vec<String> = {
            let regb = reg.borrow();
            regb.get(name)
                .map(|d| {
                    let mut v: Vec<String> = d.stores.keys().cloned().collect();
                    v.sort();
                    v
                })
                .unwrap_or_default()
        };
        let vals: Vec<Value> = names.into_iter().map(|s| e.str_val(s)).collect();
        let arr = e.new_array(vals);
        e.set_own(id, "objectStoreNames", arr, true);
    }
    v
}

/// Wrapper `IDBTransaction`.
fn txn_value(
    e: &mut Engine,
    reg: IdbRegistry,
    db_name: String,
    stores: Vec<String>,
    mode: String,
    db_wrapper: Value,
) -> Value {
    let tp = txn_proto(e);
    let d = Rc::new(RefCell::new(TxnData {
        mode: mode.clone(),
        pending: 0,
        aborted: false,
        wrapper: None,
        db_wrapper,
        error: Value::Null,
    }));
    let v = e.new_host_object(Rc::new(IdbTxnHost {
        d: d.clone(),
        reg,
        db_name,
        stores: stores.clone(),
    }));
    if let Value::Obj(id) = v {
        if let Some(o) = e.heap.obj_mut(id) {
            o.proto = Value::Obj(tp);
        }
        d.borrow_mut().wrapper = Some(id);
        let m = e.str_val(mode);
        e.set_own(id, "mode", m, true);
        e.set_own(id, "db", db_wrapper, true);
        e.set_own(id, "error", Value::Null, true);
        let names: Vec<Value> = stores.into_iter().map(|s| e.str_val(s)).collect();
        let arr = e.new_array(names);
        e.set_own(id, "objectStoreNames", arr, true);
    }
    v
}

/// Wrapper `IDBObjectStore`/`IDBIndex` (`index` = `Some(nom)`
/// pour un index).
fn store_value(e: &mut Engine, ctx: &StoreCtx, index: Option<&str>) -> Value {
    let sp = if index.is_some() {
        index_proto(e)
    } else {
        store_proto(e)
    };
    let v = e.new_host_object(Rc::new(IdbStoreHost {
        ctx: ctx.clone(),
        index: index.map(str::to_string),
    }));
    if let Value::Obj(id) = v {
        if let Some(o) = e.heap.obj_mut(id) {
            o.proto = Value::Obj(sp);
        }
        // `transaction` rétro-lien — wrapper de la txn.
        let txn_w = ctx
            .txn
            .as_ref()
            .and_then(|t| t.borrow().wrapper)
            .map_or(Value::Null, Value::Obj);
        e.set_own(id, "transaction", txn_w, true);
        let regb = ctx.reg.borrow();
        if let Some(st) = regb.get(&ctx.db).and_then(|d| d.stores.get(&ctx.store)) {
            if let Some(iname) = index {
                if let Some(idef) = st.indexes.get(iname) {
                    let n = e.str_val(iname.to_string());
                    e.set_own(id, "name", n, true);
                    let kp = e.str_val(idef.key_path.clone());
                    e.set_own(id, "keyPath", kp, true);
                    e.set_own(id, "unique", Value::Bool(idef.unique), true);
                    e.set_own(id, "multiEntry", Value::Bool(idef.multi), true);
                }
            } else {
                let n = e.str_val(ctx.store.clone());
                e.set_own(id, "name", n, true);
                match &st.key_path {
                    Some(kp) => {
                        let kpv = e.str_val(kp.clone());
                        e.set_own(id, "keyPath", kpv, true);
                    }
                    None => e.set_own(id, "keyPath", Value::Null, true),
                }
                e.set_own(id, "autoIncrement", Value::Bool(st.auto_inc), true);
                let mut v: Vec<String> = st.indexes.keys().cloned().collect();
                v.sort();
                let vals: Vec<Value> = v.into_iter().map(|s| e.str_val(s)).collect();
                let arr = e.new_array(vals);
                e.set_own(id, "indexNames", arr, true);
                // `objectStore` rétro-lien sur un index =
                // le store lui-même (renseigné à la lecture
                // via prop `source` de la txn — posé dans
                // `store_index`).
            }
        }
    }
    v
}

/// `txn.objectStore(name)`.
fn txn_object_store(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = this else {
        return Err(e.type_error("objectStore on incompatible receiver"));
    };
    let Some(ObjKind::Host(h)) = e.heap.obj(id).map(|o| &o.kind) else {
        return Err(e.type_error("objectStore on incompatible receiver"));
    };
    let h = h.clone();
    let Some(t) = h.as_any().downcast_ref::<IdbTxnHost>() else {
        return Err(e.type_error("objectStore on incompatible receiver"));
    };
    let name = e.to_js_string(arg(args, 0))?;
    if !t.stores.iter().any(|s| s == &name) {
        return Err(e.type_error(format!("objectStore: {name:?} not in transaction scope")));
    }
    Ok(store_value(
        e,
        &StoreCtx {
            reg: t.reg.clone(),
            db: t.db_name.clone(),
            store: name,
            txn: Some(t.d.clone()),
        },
        None,
    ))
}

/// `txn.abort()` — vide la file + `abort` event.
fn txn_abort(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let Some(d) = txn_data(e, this) else {
        return Err(e.type_error("abort on incompatible receiver"));
    };
    d.borrow_mut().aborted = true;
    Ok(Value::Undefined)
}

/// `txn.commit()` — no-op honnête : le commit est automatique
/// au drain de la file (spec explicite acceptée).
fn txn_commit(_e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    Ok(Value::Undefined)
}

/// Contexte store extrait de `this` — `None` = pas un store.
fn store_ctx(e: &Engine, this: Value) -> Option<(StoreCtx, Option<String>)> {
    store_data(e, this)
}

/// Request store enfilée — incrémente `txn.pending` et lie
/// `source`/`transaction` sur la request.
fn store_request(e: &mut Engine, ctx: &StoreCtx, op: IdbOp, source: Value) -> Value {
    if let Some(t) = &ctx.txn {
        t.borrow_mut().pending += 1;
    }
    let txn_w = ctx
        .txn
        .as_ref()
        .and_then(|t| t.borrow().wrapper)
        .map_or(Value::Null, Value::Obj);
    new_request(e, op, source, txn_w)
}

/// `this` n'est pas un store host → `TypeError`.
fn bad_store(e: &mut Engine) -> crate::heap::Thrown {
    e.type_error("method on incompatible IDBObjectStore/IDBIndex")
}

/// `store.get(key|range)` / `index.get(k)`.
fn store_get(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    query_native(e, this, args, QueryKind::One)
}

/// `store.getAll(range?, count?)`.
fn store_get_all(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    query_native(e, this, args, QueryKind::All)
}

/// `store.getKey(range?)`.
fn store_get_key(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    query_native(e, this, args, QueryKind::Key)
}

/// `store.getAllKeys(range?, count?)`.
fn store_get_all_keys(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    query_native(e, this, args, QueryKind::AllKeys)
}

/// `store.count(range?)`.
fn store_count(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    query_native(e, this, args, QueryKind::Count)
}

/// Corps commun des requêtes de lecture.
fn query_native(e: &mut Engine, this: Value, args: &[Value], what: QueryKind) -> JsResult<Value> {
    let Some((ctx, idx)) = store_ctx(e, this) else {
        return Err(bad_store(e));
    };
    let range = range_of(e, arg(args, 0))?;
    let op = match idx {
        Some(iname) => IdbOp::QueryIdx {
            ctx: ctx.clone(),
            idx: iname,
            range,
            what,
        },
        None => IdbOp::Query {
            ctx: ctx.clone(),
            range,
            what,
        },
    };
    Ok(store_request(e, &ctx, op, this))
}

/// `store.put(v, key?)` / `store.add(v, key?)` — valeur
/// structured-clonée à l'appel (spec : l'écriture fige la
/// valeur), clé résolue ici pour `keyPath`/explicite ;
/// `autoIncrement` allouée à la livraison.
fn put_or_add(e: &mut Engine, this: Value, args: &[Value], add_only: bool) -> JsResult<Value> {
    let Some((ctx, idx)) = store_ctx(e, this) else {
        return Err(bad_store(e));
    };
    if idx.is_some() {
        return Err(e.type_error("put/add on an IDBIndex"));
    }
    if let Some(t) = &ctx.txn {
        if t.borrow().mode == "readonly" {
            return Err(e.type_error("put/add on a readonly transaction"));
        }
    }
    let value = arg(args, 0);
    // Clone à l'appel — graphes profonds/circulaires →
    // DataCloneError typé.
    let cloned = crate::builtins::clone::clone_deep(e, value, &mut Vec::new(), 0)?;
    let explicit = arg(args, 1);
    let key = if matches!(explicit, Value::Undefined) {
        // `keyPath` → extraction depuis le clone ; `autoInc`
        // sans keyPath → clé allouée à la livraison.
        let (kp, ai) = {
            let regb = ctx.reg.borrow();
            regb.get(&ctx.db)
                .and_then(|d| d.stores.get(&ctx.store))
                .map_or((None, false), |s| (s.key_path.clone(), s.auto_inc))
        };
        if let Some(path) = kp {
            let kv = key_path_value(e, cloned, &path)
                .ok_or_else(|| e.type_error("put/add: keyPath yields no key"))?;
            let k =
                idb_key_of(e, kv, 0).ok_or_else(|| e.type_error("put/add: keyPath key invalid"))?;
            Some(k)
        } else if ai {
            None // allouée à la livraison (`next_key`)
        } else {
            return Err(e.type_error("put/add: no key given and no keyPath"));
        }
    } else {
        Some(idb_key_of(e, explicit, 0).ok_or_else(|| e.type_error("put/add: invalid key"))?)
    };
    let op = IdbOp::Put {
        ctx: ctx.clone(),
        value: cloned,
        key,
        add_only,
        check_unique: true,
    };
    Ok(store_request(e, &ctx, op, this))
}

/// `store.put(v, k?)`.
fn store_put(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    put_or_add(e, this, args, false)
}

/// `store.add(v, k?)`.
fn store_add(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    put_or_add(e, this, args, true)
}

/// `store.delete(key|range)` — spec accepte un range.
fn store_delete(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Some((ctx, idx)) = store_ctx(e, this) else {
        return Err(bad_store(e));
    };
    if idx.is_some() {
        return Err(e.type_error("delete on an IDBIndex"));
    }
    let range =
        range_of(e, arg(args, 0))?.ok_or_else(|| e.type_error("store.delete: key required"))?;
    let op = IdbOp::Remove {
        ctx: ctx.clone(),
        range: Some(range),
    };
    Ok(store_request(e, &ctx, op, this))
}

/// `store.clear()`.
fn store_clear(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let Some((ctx, idx)) = store_ctx(e, this) else {
        return Err(bad_store(e));
    };
    if idx.is_some() {
        return Err(e.type_error("clear on an IDBIndex"));
    }
    let op = IdbOp::Remove {
        ctx: ctx.clone(),
        range: None,
    };
    Ok(store_request(e, &ctx, op, this))
}

/// Direction texte → `Dir`.
fn dir_of(s: &str) -> Dir {
    match s {
        "prev" => Dir::Prev,
        "nextunique" => Dir::NextUnique,
        "prevunique" => Dir::PrevUnique,
        _ => Dir::Next,
    }
}

/// `store.openCursor(range?, direction?)`.
fn store_open_cursor(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Some((ctx, idx)) = store_ctx(e, this) else {
        return Err(bad_store(e));
    };
    if idx.is_some() {
        return Err(e.type_error("openCursor: use index proto"));
    }
    let range = range_of(e, arg(args, 0))?;
    let dir = dir_of(&e.to_js_string(arg(args, 1)).unwrap_or_default());
    let op = IdbOp::Cursor {
        ctx: CursorCtx::Store(ctx.clone()),
        range,
        dir,
    };
    Ok(store_request(e, &ctx, op, this))
}

/// `index.openCursor(range?, dir)` — `value` = record.
fn index_open_cursor(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    index_cursor(e, this, args, false)
}

/// `index.openKeyCursor` — pas de `value` (`IDBCursor`).
fn index_open_key_cursor(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    index_cursor(e, this, args, true)
}

/// Corps `openCursor`/`openKeyCursor` d'index.
fn index_cursor(e: &mut Engine, this: Value, args: &[Value], key_only: bool) -> JsResult<Value> {
    let Some((ctx, idx)) = store_ctx(e, this) else {
        return Err(bad_store(e));
    };
    let Some(iname) = idx else {
        return Err(e.type_error("openKeyCursor on a store"));
    };
    let range = range_of(e, arg(args, 0))?;
    let dir = dir_of(&e.to_js_string(arg(args, 1)).unwrap_or_default());
    let op = IdbOp::Cursor {
        ctx: CursorCtx::Index(ctx.clone(), iname, key_only),
        range,
        dir,
    };
    Ok(store_request(e, &ctx, op, this))
}

/// `store.createIndex(name, keyPath, {unique?, multiEntry?})` —
/// légal en upgrade txn (et versionchange ; le miroir ne
/// distingue pas — approximation documentée).
fn store_create_index(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Some((ctx, idx)) = store_ctx(e, this) else {
        return Err(bad_store(e));
    };
    if idx.is_some() {
        return Err(e.type_error("createIndex on an IDBIndex"));
    }
    let name = e.to_js_string(arg(args, 0))?;
    let kp = e.to_js_string(arg(args, 1))?;
    if kp.is_empty() {
        return Err(e.type_error("createIndex: keyPath required"));
    }
    let opts = arg(args, 2);
    let (mut uniq, mut multi) = (false, false);
    if let Value::Obj(oid) = opts {
        uniq = e.get_prop(oid, "unique").is_some_and(|v| e.to_boolean(v));
        multi = e
            .get_prop(oid, "multiEntry")
            .is_some_and(|v| e.to_boolean(v));
    }
    let mut regb = ctx.reg.borrow_mut();
    let Some(st) = regb
        .get_mut(&ctx.db)
        .and_then(|d| d.stores.get_mut(&ctx.store))
    else {
        return Err(e.type_error("createIndex: store gone"));
    };
    if st.indexes.contains_key(&name) {
        return Err(e.type_error("createIndex: index exists"));
    }
    st.indexes.insert(
        name.clone(),
        IdbIndexDef {
            key_path: kp,
            unique: uniq,
            multi,
        },
    );
    drop(regb);
    Ok(store_value(e, &ctx, Some(&name)))
}

/// `store.deleteIndex(name)`.
fn store_delete_index(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Some((ctx, idx)) = store_ctx(e, this) else {
        return Err(bad_store(e));
    };
    if idx.is_some() {
        return Err(e.type_error("deleteIndex on an IDBIndex"));
    }
    let name = e.to_js_string(arg(args, 0))?;
    let mut regb = ctx.reg.borrow_mut();
    let Some(st) = regb
        .get_mut(&ctx.db)
        .and_then(|d| d.stores.get_mut(&ctx.store))
    else {
        return Err(e.type_error("deleteIndex: store gone"));
    };
    if st.indexes.remove(&name).is_none() {
        return Err(e.type_error("deleteIndex: no such index"));
    }
    Ok(Value::Undefined)
}

/// `store.index(name)` → wrapper `IDBIndex`.
fn store_index(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Some((ctx, idx)) = store_ctx(e, this) else {
        return Err(bad_store(e));
    };
    if idx.is_some() {
        return Err(e.type_error("index() on an IDBIndex"));
    }
    let name = e.to_js_string(arg(args, 0))?;
    {
        let regb = ctx.reg.borrow();
        let ok = regb
            .get(&ctx.db)
            .and_then(|d| d.stores.get(&ctx.store))
            .is_some_and(|s| s.indexes.contains_key(&name));
        if !ok {
            return Err(e.type_error(format!("index {name:?} missing")));
        }
    }
    let iv = store_value(e, &ctx, Some(&name));
    // `objectStore` rétro-lien.
    if let Value::Obj(iid) = iv {
        e.set_own(iid, "objectStore", this, true);
    }
    Ok(iv)
}

// ------------------------------------------------------------------
// IDBCursor
// ------------------------------------------------------------------

/// `cursor.continue(key?)`.
fn cursor_continue(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = this else {
        return Err(e.type_error("continue on incompatible receiver"));
    };
    let key = match arg(args, 0) {
        Value::Undefined => None,
        v => Some(idb_key_of(e, v, 0).ok_or_else(|| e.type_error("continue: invalid key"))?),
    };
    let _ = id;
    let ctx = cursor_ctx(e, this)?;
    Ok(store_request(
        e,
        &ctx,
        IdbOp::CursorStep {
            cursor: id,
            act: CursorAct::Next(key, 1),
        },
        this,
    ))
}

/// `cursor.advance(n)`.
fn cursor_advance(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = this else {
        return Err(e.type_error("advance on incompatible receiver"));
    };
    let n = e.to_number(arg(args, 0))?.max(1.0) as u32;
    let ctx = cursor_ctx(e, this)?;
    Ok(store_request(
        e,
        &ctx,
        IdbOp::CursorStep {
            cursor: id,
            act: CursorAct::Next(None, n),
        },
        this,
    ))
}

/// `cursor.update(value)` → request `result` = clé primaire.
fn cursor_update(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = this else {
        return Err(e.type_error("update on incompatible receiver"));
    };
    let cloned = crate::builtins::clone::clone_deep(e, arg(args, 0), &mut Vec::new(), 0)?;
    let ctx = cursor_ctx(e, this)?;
    Ok(store_request(
        e,
        &ctx,
        IdbOp::CursorStep {
            cursor: id,
            act: CursorAct::Update(cloned),
        },
        this,
    ))
}

/// `cursor.delete()`.
fn cursor_delete(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = this else {
        return Err(e.type_error("delete on incompatible receiver"));
    };
    let _ = args;
    let ctx = cursor_ctx(e, this)?;
    Ok(store_request(
        e,
        &ctx,
        IdbOp::CursorStep {
            cursor: id,
            act: CursorAct::Delete,
        },
        this,
    ))
}

/// Request porteuse d'un cursor (`None` = pas un cursor).
fn cursor_req(e: &mut Engine, cursor: ObjId) -> Option<ObjId> {
    let Some(ObjKind::Host(h)) = e.heap.obj(cursor).map(|o| &o.kind) else {
        return None;
    };
    let h = h.clone();
    h.as_any()
        .downcast_ref::<IdbCursorHost>()
        .map(|c| c.d.borrow().req)
}

/// `StoreCtx` d'un cursor host.
fn cursor_ctx(e: &mut Engine, this: Value) -> JsResult<StoreCtx> {
    let Value::Obj(id) = this else {
        return Err(e.type_error("cursor method on non-cursor"));
    };
    let Some(ObjKind::Host(h)) = e.heap.obj(id).map(|o| &o.kind) else {
        return Err(e.type_error("cursor method on non-cursor"));
    };
    let h = h.clone();
    let Some(c) = h.as_any().downcast_ref::<IdbCursorHost>() else {
        return Err(e.type_error("cursor method on non-cursor"));
    };
    let ctx = c.d.borrow().ctx.clone();
    Ok(ctx)
}

// ------------------------------------------------------------------
// Livraison au checkpoint microtask
// ------------------------------------------------------------------

/// Itère les records d'un index → `(idx_key, pkey, record)`.
fn index_entries(
    st: &IdbStore,
    idef: &IdbIndexDef,
    e: &mut Engine,
) -> Vec<(IdbKey, IdbKey, Value)> {
    let mut out = Vec::new();
    for (pk, v) in &st.records {
        if let Some(iv) = key_path_value(e, *v, &idef.key_path) {
            if idef.multi {
                // `multiEntry` : un élément-array par entrée.
                if let Value::Obj(oid) = iv {
                    if let Some(len) = e.get_prop(oid, "length").and_then(|l| e.to_number(l).ok()) {
                        for i in 0..len as u32 {
                            if let Some(el) = e.get_prop(oid, &i.to_string()) {
                                if let Some(k) = idb_key_of(e, el, 0) {
                                    out.push((k, pk.clone(), *v));
                                }
                            }
                        }
                        continue;
                    }
                }
            }
            if let Some(k) = idb_key_of(e, iv, 0) {
                out.push((k, pk.clone(), *v));
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp_idb(&b.0).then(a.1.cmp_idb(&b.1)));
    out
}

/// Exécute une `IdbOp` → `Ok(result)` / `Err(error value)`.
/// Le `wrapper` de la request sert à `upgradeneeded` (Open).
fn run_idb_op(e: &mut Engine, req: ObjId, op: &IdbOp) -> Result<Value, Value> {
    match op {
        IdbOp::Open { name, version } => open_op(e, req, name, *version),
        IdbOp::DeleteDb { name } => {
            let r = reg(e);
            r.borrow_mut().remove(name);
            Ok(Value::Undefined)
        }
        IdbOp::Put {
            ctx,
            value,
            key,
            add_only,
            check_unique,
        } => put_op(e, ctx, *value, key.clone(), *add_only, *check_unique),
        IdbOp::Query { ctx, range, what } => query_op(e, ctx, None, range.as_ref(), what),
        IdbOp::QueryIdx {
            ctx,
            idx,
            range,
            what,
        } => query_op(e, ctx, Some(idx.as_str()), range.as_ref(), what),
        IdbOp::Remove { ctx, range } => remove_op(e, ctx, range.as_ref()),
        IdbOp::Cursor { ctx, range, dir } => cursor_open_op(e, ctx, range.as_ref(), *dir, req),
        IdbOp::CursorStep { cursor, act } => cursor_step_op(e, *cursor, act),
    }
}

/// `open` : crée/ouvre la base, tire `upgradeneeded` si
/// version > existante (ou création), puis `success`.
fn open_op(e: &mut Engine, req: ObjId, name: &str, version: Option<u64>) -> Result<Value, Value> {
    let r = reg(e);
    let (old_v, new_v) = {
        let mut rb = r.borrow_mut();
        if let Some(db) = rb.get_mut(name) {
            let old = db.version;
            let want = version.unwrap_or(old);
            if want < old {
                drop(rb);
                return Err(idb_err(
                    e,
                    "VersionError",
                    "requested version lower than current",
                ));
            }
            db.version = want;
            (old, want)
        } else {
            let v = version.unwrap_or(1);
            rb.insert(
                name.to_string(),
                IdbDb {
                    version: v,
                    stores: HashMap::new(),
                },
            );
            (0, v)
        }
    };
    let dbv = db_value(e, &r, name, new_v);
    // Upgrade nécessaire si la version a monté ou création.
    if new_v > old_v || old_v == 0 {
        // Transaction `versionchange` — l'upgrade autorise
        // createObjectStore/createIndex sur cette connexion.
        let txv = txn_value(
            e,
            r.clone(),
            name.to_string(),
            Vec::new(),
            "versionchange".to_string(),
            dbv,
        );
        if let Value::Obj(did) = dbv {
            if let Some(ObjKind::Host(h)) = e.heap.obj(did).map(|o| &o.kind) {
                if let Some(d) = h.as_any().downcast_ref::<IdbDbHost>() {
                    *d.upgrade_txn.borrow_mut() = txv;
                }
            }
        }
        // `req.transaction` = la txn d'upgrade pendant
        // l'event ; `result` déjà dispo (spec).
        if let Some(d) = req_data(e, Value::Obj(req)) {
            d.borrow_mut().result = dbv;
            d.borrow_mut().txn = txv;
        }
        // `Thrown(v)` → erreur de request (handler throw =
        // abort txn spec — approx : error sur la request).
        fire_idb(e, req, "upgradeneeded", Some((old_v, new_v))).map_err(|t| t.0)?;
        // Fin de l'upgrade — plus de createObjectStore.
        if let Value::Obj(did) = dbv {
            if let Some(ObjKind::Host(h)) = e.heap.obj(did).map(|o| &o.kind) {
                if let Some(d) = h.as_any().downcast_ref::<IdbDbHost>() {
                    *d.upgrade_txn.borrow_mut() = Value::Undefined;
                }
            }
        }
        if let Some(td) = txn_data(e, txv) {
            td.borrow_mut().pending = usize::MAX; // txn close
        }
    }
    Ok(dbv)
}

/// `put`/`add` : clé `autoIncrement` allouée ici, unique
/// vérifié, `ConstraintError` spec.
fn put_op(
    e: &mut Engine,
    ctx: &StoreCtx,
    value: Value,
    key: Option<IdbKey>,
    add_only: bool,
    check_unique: bool,
) -> Result<Value, Value> {
    let mut rb = ctx.reg.borrow_mut();
    let Some(db) = rb.get_mut(&ctx.db) else {
        return Err(idb_err(e, "InvalidStateError", "database gone"));
    };
    let Some(st) = db.stores.get_mut(&ctx.store) else {
        return Err(idb_err(e, "InvalidStateError", "store gone"));
    };
    let k = if let Some(k) = key {
        k
    } else {
        // `autoIncrement` — alloue `next_key`.
        let k = IdbKey::Num(st.next_key);
        st.next_key += 1.0;
        k
    };
    if let IdbKey::Num(n) = k {
        if n >= st.next_key {
            st.next_key = n + 1.0;
        }
    }
    if st.records.contains_key(&k) && add_only {
        return Err(idb_err(e, "ConstraintError", "key already exists (add)"));
    }
    // Unicité des index `unique` (recalcul depuis les records).
    if check_unique {
        for idef in st.indexes.values().filter(|i| i.unique) {
            if let Some(iv) = key_path_value(e, value, &idef.key_path) {
                if let Some(nk) = idb_key_of(e, iv, 0) {
                    for (pk, rec) in &st.records {
                        if *pk == k {
                            continue;
                        }
                        if let Some(ev2) = key_path_value(e, *rec, &idef.key_path) {
                            if idb_key_of(e, ev2, 0) == Some(nk.clone()) {
                                return Err(idb_err(e, "ConstraintError", "unique index violated"));
                            }
                        }
                    }
                }
            }
        }
    }
    st.records.insert(k.clone(), value);
    Ok(k.to_value(e))
}

/// `get`/`getAll`/`getKey`/`getAllKeys`/`count` — `idx =
/// Some` : range sur la clé d'index, résultat via `pkey`.
fn query_op(
    e: &mut Engine,
    ctx: &StoreCtx,
    idx: Option<&str>,
    range: Option<&Range>,
    what: &QueryKind,
) -> Result<Value, Value> {
    let entries: Vec<(IdbKey, IdbKey, Value)> = {
        let rb = ctx.reg.borrow();
        let Some(st) = rb.get(&ctx.db).and_then(|d| d.stores.get(&ctx.store)) else {
            return Err(idb_err(e, "InvalidStateError", "store gone"));
        };
        match idx {
            Some(iname) => {
                let Some(idef) = st.indexes.get(iname) else {
                    return Err(idb_err(e, "NotFoundError", "index gone"));
                };
                index_entries(st, idef, e)
            }
            None => st
                .records
                .iter()
                .map(|(k, v)| (k.clone(), k.clone(), *v))
                .collect(),
        }
    };
    // Filtre range : sur `key` (index) ou `pkey` (store).
    let on_index = idx.is_some();
    let hit = |(ik, pk, _v): &(IdbKey, IdbKey, Value)| {
        let probe = if on_index { ik } else { pk };
        range.is_none_or(|r| r.contains(probe))
    };
    let filtered: Vec<&(IdbKey, IdbKey, Value)> = entries.iter().filter(|t| hit(t)).collect();
    // Structured clone à la lecture (spec renvoie des copies).
    let cl = |e: &mut Engine, v: Value| {
        crate::builtins::clone::clone_deep(e, v, &mut Vec::new(), 0).unwrap_or(Value::Undefined)
    };
    match what {
        QueryKind::One => Ok(filtered
            .first()
            .map_or(Value::Undefined, |(_, _, v)| cl(e, *v))),
        QueryKind::All => {
            let vals: Vec<Value> = filtered.iter().map(|(_, _, v)| cl(e, *v)).collect();
            Ok(e.new_array(vals))
        }
        QueryKind::Key => Ok(filtered.first().map_or(Value::Undefined, |(ik, pk, _)| {
            if on_index {
                ik.to_value(e)
            } else {
                pk.to_value(e)
            }
        })),
        QueryKind::AllKeys => {
            let vals: Vec<Value> = filtered
                .iter()
                .map(|(ik, pk, _)| {
                    if on_index {
                        ik.to_value(e)
                    } else {
                        pk.to_value(e)
                    }
                })
                .collect();
            Ok(e.new_array(vals))
        }
        QueryKind::Count => Ok(Value::Num(filtered.len() as f64)),
    }
}

/// `delete(range)`/`clear()`.
fn remove_op(e: &mut Engine, ctx: &StoreCtx, range: Option<&Range>) -> Result<Value, Value> {
    let mut rb = ctx.reg.borrow_mut();
    let Some(st) = rb
        .get_mut(&ctx.db)
        .and_then(|d| d.stores.get_mut(&ctx.store))
    else {
        return Err(idb_err(e, "InvalidStateError", "store gone"));
    };
    match range {
        None => st.records.clear(),
        Some(r) => st.records.retain(|k, _| !r.contains(k)),
    }
    Ok(Value::Undefined)
}

/// `openCursor` — matérialise les entrées filtrées et pose le
/// premier élément ; `result` = cursor ou `null` (vide).
fn cursor_open_op(
    e: &mut Engine,
    cctx: &CursorCtx,
    range: Option<&Range>,
    dir: Dir,
    req: ObjId,
) -> Result<Value, Value> {
    let (ctx, idx, key_only) = match cctx {
        CursorCtx::Store(c) => (c, None, false),
        CursorCtx::Index(c, i, ko) => (c, Some(i.as_str()), *ko),
    };
    let mut entries: Vec<(IdbKey, IdbKey, Value)> = {
        let rb = ctx.reg.borrow();
        let Some(st) = rb.get(&ctx.db).and_then(|d| d.stores.get(&ctx.store)) else {
            return Err(idb_err(e, "InvalidStateError", "store gone"));
        };
        match idx {
            Some(iname) => {
                let Some(idef) = st.indexes.get(iname) else {
                    return Err(idb_err(e, "NotFoundError", "index gone"));
                };
                index_entries(st, idef, e)
            }
            None => st
                .records
                .iter()
                .map(|(k, v)| (k.clone(), k.clone(), *v))
                .collect(),
        }
    };
    let on_index = idx.is_some();
    entries.retain(|(ik, pk, _)| {
        let probe = if on_index { ik } else { pk };
        range.is_none_or(|r| r.contains(probe))
    });
    match dir {
        Dir::Prev | Dir::PrevUnique => {
            entries.sort_by(|a, b| b.0.cmp_idb(&a.0).then(b.1.cmp_idb(&a.1)));
        }
        _ => {}
    }
    if matches!(dir, Dir::NextUnique | Dir::PrevUnique) {
        entries.dedup_by(|a, b| a.0 == b.0);
    }
    if entries.is_empty() {
        return Ok(Value::Null);
    }
    let mut dq: VecDeque<(IdbKey, IdbKey, Value)> = entries.into();
    let first = dq.pop_front();
    // Clone de `value` pour le cursor courant.
    let cur = first.map(|(ik, pk, v)| {
        let cv = crate::builtins::clone::clone_deep(e, v, &mut Vec::new(), 0)
            .unwrap_or(Value::Undefined);
        (ik, pk, cv)
    });
    let cp = cursor_proto(e);
    let cv = e.new_host_object(Rc::new(IdbCursorHost {
        d: RefCell::new(CursorData {
            items: dq,
            current: cur,
            ctx: ctx.clone(),
            dir,
            source: Value::Null,
            req,
            key_only,
        }),
    }));
    if let Value::Obj(cid) = cv {
        if let Some(o) = e.heap.obj_mut(cid) {
            o.proto = Value::Obj(cp);
        }
    }
    Ok(cv)
}

/// `cursor.continue/advance/update/delete` — livrés comme
/// requests (spec : `continue` re-déclenche `onsuccess` sur
/// la MÊME request — approximation : nouvelle request, le
/// pattern `cursor` nul de fin fonctionne identiquement).
fn cursor_step_op(e: &mut Engine, cursor: ObjId, act: &CursorAct) -> Result<Value, Value> {
    let Some(ObjKind::Host(h)) = e.heap.obj(cursor).map(|o| &o.kind) else {
        return Err(idb_err(e, "InvalidStateError", "bad cursor"));
    };
    let h = h.clone();
    let Some(c) = h.as_any().downcast_ref::<IdbCursorHost>() else {
        return Err(idb_err(e, "InvalidStateError", "bad cursor"));
    };
    match act {
        CursorAct::Next(key, n) => {
            let mut d = c.d.borrow_mut();
            // `continue(key)` : avance jusqu'à `key` incluse
            // (direction croissante — approx : skip < key).
            if let Some(k) = key {
                while let Some((ik, pk, _)) = d.items.front() {
                    let probe = ik;
                    if probe.cmp_idb(k) == std::cmp::Ordering::Less {
                        let _ = pk;
                        d.items.pop_front();
                    } else {
                        break;
                    }
                }
            }
            for _ in 1..*n {
                d.items.pop_front();
            }
            d.current = d.items.pop_front().map(|(ik, pk, v)| {
                let cv = crate::builtins::clone::clone_deep(e, v, &mut Vec::new(), 0)
                    .unwrap_or(Value::Undefined);
                (ik, pk, cv)
            });
            // `result` = le cursor lui-même ou null en fin.
            Ok(if d.current.is_some() {
                drop(d);
                Value::Obj(cursor)
            } else {
                drop(d);
                Value::Null
            })
        }
        CursorAct::Update(v) => {
            let d = c.d.borrow();
            let Some((_, pk, _)) = &d.current else {
                return Err(idb_err(e, "InvalidStateError", "cursor finished"));
            };
            let pk = pk.clone();
            let ctx = d.ctx.clone();
            drop(d);
            let mut rb = ctx.reg.borrow_mut();
            let Some(st) = rb
                .get_mut(&ctx.db)
                .and_then(|d| d.stores.get_mut(&ctx.store))
            else {
                return Err(idb_err(e, "InvalidStateError", "store gone"));
            };
            st.records.insert(pk.clone(), *v);
            Ok(pk.to_value(e))
        }
        CursorAct::Delete => {
            let d = c.d.borrow();
            let Some((_, pk, _)) = &d.current else {
                return Err(idb_err(e, "InvalidStateError", "cursor finished"));
            };
            let pk = pk.clone();
            let ctx = d.ctx.clone();
            drop(d);
            let mut rb = ctx.reg.borrow_mut();
            let Some(st) = rb
                .get_mut(&ctx.db)
                .and_then(|d| d.stores.get_mut(&ctx.store))
            else {
                return Err(idb_err(e, "InvalidStateError", "store gone"));
            };
            st.records.remove(&pk);
            Ok(Value::Undefined)
        }
    }
}

/// Livre les requests IDB pendantes — appelé au checkpoint
/// microtask (`run_microtasks`), après les observers.
/// Chaque op s'exécute, pose `result`/`error`/`done`, puis
/// `onsuccess`/`onerror` (et `onblocked` pour Open jamais —
/// connexions non traquées). La txn `complete` quand sa file
/// se vide.
pub(crate) fn deliver_idb_requests(e: &mut Engine) -> JsResult<usize> {
    let mut delivered = 0usize;
    let mut guard = 0usize;
    while let Some(req) = e.idb.pending.pop_front() {
        guard += 1;
        if guard > 10_000 {
            break;
        }
        let Some(d) = req_data(e, Value::Obj(req)) else {
            continue;
        };
        let (op, txn_v) = {
            let mut db = d.borrow_mut();
            if db.done {
                continue;
            }
            db.done = true;
            (db.op.take(), db.txn)
        };
        delivered += 1;
        let Some(op) = op else { continue };
        // Txn abortée → op sautée.
        if let Some(td) = txn_data(e, txn_v) {
            if td.borrow().aborted {
                continue;
            }
        }
        // `cursor.continue`/`advance` : la request réelle est
        // celle du cursor (spec) — `result` = cursor ou null,
        // `onsuccess` re-tiré sur elle.
        let fire_req = match &op {
            IdbOp::CursorStep {
                cursor,
                act: CursorAct::Next(..),
            } => cursor_req(e, *cursor).unwrap_or(req),
            _ => req,
        };
        match run_idb_op(e, req, &op) {
            Ok(res) => {
                if fire_req == req {
                    d.borrow_mut().result = res;
                } else if let Some(fd) = req_data(e, Value::Obj(fire_req)) {
                    fd.borrow_mut().result = res;
                }
                fire_idb(e, fire_req, "success", None)?;
            }
            Err(err) => {
                d.borrow_mut().error = err;
                if let Some(td) = txn_data(e, txn_v) {
                    td.borrow_mut().error = err;
                }
                fire_idb(e, req, "error", None)?;
            }
        }
        // `pending--` + `complete` quand la txn se vide.
        if let Some(td) = txn_data(e, txn_v) {
            let mut tb = td.borrow_mut();
            if tb.pending != usize::MAX && tb.pending > 0 {
                tb.pending -= 1;
            }
            if tb.pending == 0 && !tb.aborted {
                let w = tb.wrapper;
                drop(tb);
                if let Some(wid) = w {
                    fire_idb(e, wid, "complete", None)?;
                }
            }
        }
    }
    Ok(delivered)
}

// ------------------------------------------------------------------
// impl Host
// ------------------------------------------------------------------

impl Host for IdbReqHost {
    fn get(&self, e: &mut Engine, _t: Value, k: &str) -> JsResult<Option<Value>> {
        let d = self.d.borrow();
        Ok(match k {
            "result" => Some(d.result),
            "error" => Some(d.error),
            "readyState" => Some(e.str_val(if d.done { "done" } else { "pending" })),
            "source" => Some(d.source),
            "transaction" => Some(d.txn),
            _ => None,
        })
    }

    fn trace(&self, out: &mut Vec<ObjId>) {
        let d = self.d.borrow();
        crate::heap::push_value(out, d.result);
        crate::heap::push_value(out, d.error);
        crate::heap::push_value(out, d.source);
        crate::heap::push_value(out, d.txn);
        if let Some(op) = &d.op {
            match op {
                IdbOp::Put { value, ctx, .. } => {
                    crate::heap::push_value(out, *value);
                    if let Some(t) = &ctx.txn {
                        let tb = t.borrow();
                        if let Some(w) = tb.wrapper {
                            out.push(w);
                        }
                        crate::heap::push_value(out, tb.db_wrapper);
                    }
                }
                IdbOp::Query { ctx, .. }
                | IdbOp::QueryIdx { ctx, .. }
                | IdbOp::Remove { ctx, .. } => {
                    if let Some(t) = &ctx.txn {
                        let tb = t.borrow();
                        if let Some(w) = tb.wrapper {
                            out.push(w);
                        }
                        crate::heap::push_value(out, tb.db_wrapper);
                    }
                }
                IdbOp::Cursor { ctx: cc, .. } => {
                    let ctx = match cc {
                        CursorCtx::Store(c) | CursorCtx::Index(c, _, _) => c,
                    };
                    if let Some(t) = &ctx.txn {
                        let tb = t.borrow();
                        if let Some(w) = tb.wrapper {
                            out.push(w);
                        }
                        crate::heap::push_value(out, tb.db_wrapper);
                    }
                }
                IdbOp::CursorStep { cursor, act } => {
                    out.push(*cursor);
                    if let CursorAct::Update(v) = act {
                        crate::heap::push_value(out, *v);
                    }
                }
                IdbOp::Open { .. } | IdbOp::DeleteDb { .. } => {}
            }
        }
    }

    fn class_name(&self) -> &'static str {
        "IDBRequest"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl Host for IdbDbHost {
    fn get(&self, e: &mut Engine, _t: Value, k: &str) -> JsResult<Option<Value>> {
        match k {
            "name" => return Ok(Some(e.str_val(self.name.clone()))),
            "version" => {
                let v = self.reg.borrow().get(&self.name).map_or(0, |d| d.version);
                return Ok(Some(Value::Num(v as f64)));
            }
            "objectStoreNames" => {
                let mut names: Vec<String> = self
                    .reg
                    .borrow()
                    .get(&self.name)
                    .map(|d| d.stores.keys().cloned().collect())
                    .unwrap_or_default();
                names.sort();
                let vals: Vec<Value> = names.into_iter().map(|s| e.str_val(s)).collect();
                return Ok(Some(e.new_array(vals)));
            }
            _ => {}
        }
        Ok(None)
    }

    fn trace(&self, out: &mut Vec<ObjId>) {
        crate::heap::push_value(out, *self.upgrade_txn.borrow());
    }

    fn class_name(&self) -> &'static str {
        "IDBDatabase"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl Host for IdbTxnHost {
    fn get(&self, _e: &mut Engine, _t: Value, k: &str) -> JsResult<Option<Value>> {
        let d = self.d.borrow();
        Ok(match k {
            "error" => Some(d.error),
            _ => None,
        })
    }

    fn trace(&self, out: &mut Vec<ObjId>) {
        let d = self.d.borrow();
        if let Some(w) = d.wrapper {
            out.push(w);
        }
        crate::heap::push_value(out, d.db_wrapper);
        crate::heap::push_value(out, d.error);
    }

    fn class_name(&self) -> &'static str {
        "IDBTransaction"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl Host for IdbStoreHost {
    fn trace(&self, out: &mut Vec<ObjId>) {
        if let Some(t) = &self.ctx.txn {
            let tb = t.borrow();
            if let Some(w) = tb.wrapper {
                out.push(w);
            }
            crate::heap::push_value(out, tb.db_wrapper);
            crate::heap::push_value(out, tb.error);
        }
    }

    fn class_name(&self) -> &'static str {
        if self.index.is_some() {
            "IDBIndex"
        } else {
            "IDBObjectStore"
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl Host for IdbCursorHost {
    fn get(&self, e: &mut Engine, _t: Value, k: &str) -> JsResult<Option<Value>> {
        let d = self.d.borrow();
        Ok(match k {
            "key" => d.current.as_ref().map(|(ik, _, _)| ik.to_value(e)),
            "primaryKey" => d.current.as_ref().map(|(_, pk, _)| pk.to_value(e)),
            "value" => {
                if d.key_only {
                    Some(Value::Undefined)
                } else {
                    d.current.as_ref().map(|(_, _, v)| *v)
                }
            }
            "direction" => Some(e.str_val(match d.dir {
                Dir::Next => "next",
                Dir::Prev => "prev",
                Dir::NextUnique => "nextunique",
                Dir::PrevUnique => "prevunique",
            })),
            "source" => Some(d.source),
            _ => None,
        })
    }

    fn trace(&self, out: &mut Vec<ObjId>) {
        let d = self.d.borrow();
        for (_, _, v) in &d.items {
            crate::heap::push_value(out, *v);
        }
        if let Some((_, _, v)) = &d.current {
            crate::heap::push_value(out, *v);
        }
        crate::heap::push_value(out, d.source);
        out.push(d.req);
        if let Some(t) = &d.ctx.txn {
            let tb = t.borrow();
            if let Some(w) = tb.wrapper {
                out.push(w);
            }
            crate::heap::push_value(out, tb.db_wrapper);
        }
    }

    fn class_name(&self) -> &'static str {
        "IDBCursorWithValue"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl Host for IdbKrHost {
    fn get(&self, e: &mut Engine, _t: Value, k: &str) -> JsResult<Option<Value>> {
        Ok(match k {
            "lower" => self
                .lower
                .as_ref()
                .map(|(k, _)| k.to_value(e))
                .or(Some(Value::Undefined)),
            "upper" => self
                .upper
                .as_ref()
                .map(|(k, _)| k.to_value(e))
                .or(Some(Value::Undefined)),
            "lowerOpen" => Some(Value::Bool(self.lower.as_ref().is_some_and(|(_, o)| *o))),
            "upperOpen" => Some(Value::Bool(self.upper.as_ref().is_some_and(|(_, o)| *o))),
            _ => None,
        })
    }

    fn class_name(&self) -> &'static str {
        "IDBKeyRange"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

// ------------------------------------------------------------------
// Cache API — `CacheStorage` (global `caches`) + `Cache`
// ------------------------------------------------------------------

/// Host `Cache` — nom + registre partagé.
struct CacheHost {
    /// Registre.
    reg: CacheRegistry,
    /// Nom du cache.
    name: String,
}

/// `CacheHost` d'un `Value`.
fn cache_data(e: &Engine, v: Value) -> Option<(CacheRegistry, String)> {
    let Value::Obj(id) = v else { return None };
    match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::Host(h)) => h
            .clone()
            .as_any()
            .downcast_ref::<CacheHost>()
            .map(|c| (c.reg.clone(), c.name.clone())),
        _ => None,
    }
}

/// Clé d'une requête cache — `Request` (`url` prop) ou chaîne
/// résolue contre l'URL de page (matching exact, spec de base).
fn cache_key(e: &mut Engine, req: Value) -> JsResult<String> {
    match req {
        Value::Obj(id) => {
            if let Some(u) = e.get_prop(id, "url").and_then(|v| e.heap_string_of(v)) {
                Ok(u)
            } else {
                e.to_js_string(req)
            }
        }
        Value::Str(_) => {
            let s = e.to_js_string(req)?;
            // Résolution relative → `page_url` (même helper
            // que fetch — brut si base absente).
            Ok(crate::xhr::resolve_url(e, &s).unwrap_or(s))
        }
        _ => Err(e.type_error("cache: bad request argument")),
    }
}

/// `caches.open(name)` → promesse `Cache` (créé si absent).
fn cs_open(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let name = e.to_js_string(arg(args, 0))?;
    let reg = cache_reg(e);
    reg.borrow_mut().entry(name.clone()).or_default();
    let cp = cache_proto(e);
    let v = e.new_host_object(Rc::new(CacheHost {
        reg,
        name: name.clone(),
    }));
    if let Value::Obj(id) = v {
        if let Some(o) = e.heap.obj_mut(id) {
            o.proto = Value::Obj(cp);
        }
    }
    let pid = e.new_promise();
    e.resolve_promise(pid, v);
    Ok(Value::Obj(pid))
}

/// `caches.has(name)` → promesse bool.
fn cs_has(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let name = e.to_js_string(arg(args, 0))?;
    let has = cache_reg(e).borrow().contains_key(&name);
    let pid = e.new_promise();
    e.resolve_promise(pid, Value::Bool(has));
    Ok(Value::Obj(pid))
}

/// `caches.delete(name)` → promesse bool.
fn cs_delete(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let name = e.to_js_string(arg(args, 0))?;
    let had = cache_reg(e).borrow_mut().remove(&name).is_some();
    let pid = e.new_promise();
    e.resolve_promise(pid, Value::Bool(had));
    Ok(Value::Obj(pid))
}

/// `caches.keys()` → promesse `[noms]`.
fn cs_keys(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    let names: Vec<String> = cache_reg(e).borrow().keys().cloned().collect();
    let vals: Vec<Value> = names.into_iter().map(|s| e.str_val(s)).collect();
    let arr = e.new_array(vals);
    let pid = e.new_promise();
    e.resolve_promise(pid, arr);
    Ok(Value::Obj(pid))
}

/// `Response` reconstruite depuis une entrée cache.
fn cached_response(e: &mut Engine, en: &CacheEntry) -> Value {
    e.make_fetch_response(
        crate::xhr::XhrResponse {
            handle: 0,
            status: en.status,
            status_text: "OK".into(),
            headers: en.headers.clone(),
            body: en.body.clone().into_bytes(),
        },
        en.url.clone(),
        "basic",
    )
}

/// `caches.match(req)` → premier hit dans les caches, ordre
/// d'insertion ; `undefined` sinon.
fn cs_match(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let key = cache_key(e, arg(args, 0))?;
    let reg = cache_reg(e);
    let hit = {
        let rb = reg.borrow();
        rb.values().find_map(|m| m.get(&key).cloned())
    };
    let v = hit.map_or(Value::Undefined, |en| cached_response(e, &en));
    let pid = e.new_promise();
    e.resolve_promise(pid, v);
    Ok(Value::Obj(pid))
}

/// Wrapper `Cache`.
fn cache_value(e: &mut Engine, reg: &CacheRegistry, name: &str) -> Value {
    let cp = cache_proto(e);
    let v = e.new_host_object(Rc::new(CacheHost {
        reg: reg.clone(),
        name: name.to_string(),
    }));
    if let Value::Obj(id) = v {
        if let Some(o) = e.heap.obj_mut(id) {
            o.proto = Value::Obj(cp);
        }
    }
    v
}

fn cache_proto(e: &mut Engine) -> ObjId {
    if let Some(p) = e.idb.cache_proto {
        return p;
    }
    let p = new_tagged_proto(e, "Cache");
    e.add_method(p, "match", cache_match);
    e.add_method(p, "matchAll", cache_match_all);
    e.add_method(p, "put", cache_put);
    e.add_method(p, "delete", cache_delete);
    e.add_method(p, "keys", cache_keys);
    e.add_method(p, "add", cache_add);
    e.add_method(p, "addAll", cache_add_all);
    e.idb.cache_proto = Some(p);
    p
}

fn cachestorage_proto(e: &mut Engine) -> ObjId {
    if let Some(p) = e.idb.cachestorage_proto {
        return p;
    }
    let p = new_tagged_proto(e, "CacheStorage");
    e.idb.cachestorage_proto = Some(p);
    p
}

/// `cache.match(req)` — hit → `Response`, miss → undefined.
fn cache_match(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Some((reg, name)) = cache_data(e, this) else {
        return Err(e.type_error("match on incompatible receiver"));
    };
    let key = cache_key(e, arg(args, 0))?;
    let hit = reg.borrow().get(&name).and_then(|m| m.get(&key)).cloned();
    let v = hit.map_or(Value::Undefined, |en| cached_response(e, &en));
    let pid = e.new_promise();
    e.resolve_promise(pid, v);
    Ok(Value::Obj(pid))
}

/// `cache.matchAll(req?)` → array de `Response`.
fn cache_match_all(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Some((reg, name)) = cache_data(e, this) else {
        return Err(e.type_error("matchAll on incompatible receiver"));
    };
    let filter = match arg(args, 0) {
        Value::Undefined => None,
        v => Some(cache_key(e, v)?),
    };
    let entries: Vec<CacheEntry> = {
        let rb = reg.borrow();
        rb.get(&name)
            .map(|m| {
                m.iter()
                    .filter(|(k, _)| filter.as_ref().is_none_or(|f| *k == f))
                    .map(|(_, en)| en.clone())
                    .collect()
            })
            .unwrap_or_default()
    };
    let vals: Vec<Value> = entries.iter().map(|en| cached_response(e, en)).collect();
    let arr = e.new_array(vals);
    let pid = e.new_promise();
    e.resolve_promise(pid, arr);
    Ok(Value::Obj(pid))
}

/// `cache.put(req, res)` — snapshot corps + métadonnées.
fn cache_put(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Some((reg, name)) = cache_data(e, this) else {
        return Err(e.type_error("put on incompatible receiver"));
    };
    let key = cache_key(e, arg(args, 0))?;
    let res = arg(args, 1);
    let pid = e.new_promise();
    match crate::xhr::cache_entry_snapshot(e, res) {
        Ok(Some((url, status, _status_text, headers, body, _rtype))) => {
            let en = CacheEntry {
                url: if key.is_empty() { url } else { key.clone() },
                body,
                status,
                headers,
            };
            reg.borrow_mut().entry(name).or_default().insert(key, en);
            e.resolve_promise(pid, Value::Undefined);
        }
        Ok(None) => {
            let err = e.error_value(
                e.protos.type_error,
                "TypeError",
                "cache.put: response argument invalid",
            );
            e.reject_promise(pid, err);
        }
        Err(t) => return Err(t),
    }
    Ok(Value::Obj(pid))
}

/// `cache.delete(req)` → bool.
fn cache_delete(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Some((reg, name)) = cache_data(e, this) else {
        return Err(e.type_error("delete on incompatible receiver"));
    };
    let key = cache_key(e, arg(args, 0))?;
    let had = reg
        .borrow_mut()
        .get_mut(&name)
        .is_some_and(|m| m.remove(&key).is_some());
    let pid = e.new_promise();
    e.resolve_promise(pid, Value::Bool(had));
    Ok(Value::Obj(pid))
}

/// `cache.keys()` → `[Request]` (urls cachées).
fn cache_keys(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let Some((reg, name)) = cache_data(e, this) else {
        return Err(e.type_error("keys on incompatible receiver"));
    };
    let urls: Vec<String> = {
        let rb = reg.borrow();
        rb.get(&name)
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default()
    };
    let vals: Vec<Value> = urls
        .into_iter()
        .map(|u| {
            let uv = e.str_val(u);
            // `new Request(url)` via le ctor partagé.
            crate::xhr::request_of(e, uv).unwrap_or(uv)
        })
        .collect();
    let arr = e.new_array(vals);
    let pid = e.new_promise();
    e.resolve_promise(pid, arr);
    Ok(Value::Obj(pid))
}

/// `cache.add(url|Request)` = `fetch(req).then(put)` — le
/// `put` est chaîné via `.then` (livraison transport au
/// checkpoint). Sans transport la promesse rejette (même
/// comportement que `fetch` — honnête).
fn cache_add(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Some((reg, name)) = cache_data(e, this) else {
        return Err(e.type_error("add on incompatible receiver"));
    };
    let key = cache_key(e, arg(args, 0))?;
    // Spec `cache.add` : le fetch interne court en
    // `credentials: include` (le cache embarque les cookies).
    let init = e.new_object();
    let inc = e.str_val("include");
    e.set_own(init, "credentials", inc, true);
    let fp = crate::xhr::fetch_fn(e, Value::Undefined, &[arg(args, 0), Value::Obj(init)])?;
    // `.then(res => put(url, res))` — callback bound au ctx.
    let cache_v = cache_value(e, &reg, &name);
    let kv = e.str_val(key);
    let fin = e.bound_native("cacheAddPut", cache_add_put, vec![cache_v, kv]);
    let Value::Obj(fid) = fp else {
        return Ok(fp);
    };
    let then = e.get_prop(fid, "then").unwrap_or(Value::Undefined);
    e.call_value(then, fp, &[fin])
}

/// `.then` handler de `cache.add` — `put(key, res)` puis
/// rend `res` (la promesse `add` résout `undefined` spec —
/// `put` résout undefined, on le propage).
fn cache_add_put(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let (cache_v, key_v, res) = (arg(args, 0), arg(args, 1), arg(args, 2));
    cache_put(e, cache_v, &[key_v, res])
}

/// `cache.addAll(urls)` — `Promise.all(add(u))` ; rejet global
/// si un fetch échoue (spec : rien n'est écrit — approx : les
/// puts réussis restent, documenté).
fn cache_add_all(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let list = match arg(args, 0) {
        v @ Value::Obj(_) => e.iter_values(v)?,
        v => vec![v],
    };
    let mut parts = Vec::with_capacity(list.len());
    for u in list {
        parts.push(cache_add(e, this, &[u])?);
    }
    let arr = e.new_array(parts);
    // `Promise.all(parts)` via le global.
    let pall = e
        .get_prop(e.global, "Promise")
        .and_then(|p| {
            if let Value::Obj(pid) = p {
                e.get_prop(pid, "all")
            } else {
                None
            }
        })
        .unwrap_or(Value::Undefined);
    if e.is_callable(pall) {
        return e.call_value(pall, Value::Undefined, &[arr]);
    }
    Ok(arr)
}

impl Host for CacheHost {
    fn class_name(&self) -> &'static str {
        "Cache"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

// ------------------------------------------------------------------
// Installation
// ------------------------------------------------------------------

/// `install_idb(reg)` — pose `indexedDB` + `IDBKeyRange` sur le
/// global. `reg = Some(r)` : registre injecté par l'embedder
/// (persistance inter-pages = sa responsabilité, comme
/// `install_storage`) ; `None` : registre mémoire neuf par
/// engine. Rappeler avec `Some` après `install_fetch`
/// **remplace** le registre — c'est la voie d'injection.
pub(crate) fn install_idb(e: &mut Engine, inject: Option<IdbRegistry>) {
    if let Some(r) = inject {
        e.idb.reg = Some(r);
    }
    // `indexedDB` factory — objet ordinaire + méthodes.
    let f = e.new_object();
    e.add_method(f, "open", idb_open);
    e.add_method(f, "deleteDatabase", idb_delete_db);
    e.add_method(f, "databases", idb_databases);
    e.add_method(f, "cmp", idb_cmp);
    let tag = e.str_val("IDBFactory");
    e.set_own(f, crate::interp::SYM_TO_STRING_TAG, tag, false);
    let g = e.global;
    e.set_own(g, "indexedDB", Value::Obj(f), false);

    // `IDBKeyRange` — ctor + statiques (`only`/`bound`…).
    let kc = e.native_fn("IDBKeyRange", idb_keyrange_ctor);
    if let Value::Obj(kid) = kc {
        e.add_method(kid, "only", kr_only);
        e.add_method(kid, "lowerBound", kr_lower);
        e.add_method(kid, "upperBound", kr_upper);
        e.add_method(kid, "bound", kr_bound);
    }
    let kp = keyrange_proto(e);
    e.link_ctor(kc, Some(kp));
    e.set_own(g, "IDBKeyRange", kc, false);
}

/// `install_caches(reg)` — pose `caches` (`CacheStorage`) sur le
/// global. Même modèle d'injection que `install_idb`.
pub(crate) fn install_caches(e: &mut Engine, inject: Option<CacheRegistry>) {
    if let Some(r) = inject {
        e.idb.cache_reg = Some(r);
    }
    let sp = cachestorage_proto(e);
    let cs = e.new_object();
    if let Some(o) = e.heap.obj_mut(cs) {
        o.proto = Value::Obj(sp);
    }
    e.add_method(cs, "open", cs_open);
    e.add_method(cs, "has", cs_has);
    e.add_method(cs, "delete", cs_delete);
    e.add_method(cs, "keys", cs_keys);
    e.add_method(cs, "match", cs_match);
    let g = e.global;
    e.set_own(g, "caches", Value::Obj(cs), false);
}

impl Engine {
    /// Injecte un registre `IndexedDB` partagé (persistance
    /// inter-pages = responsabilité de l'embedder — même
    /// couture que [`Engine::install_storage`]). Appelable
    /// après `install_fetch`/`install_xhr` — **remplace** le
    /// registre mémoire par défaut.
    pub fn install_idb(&mut self, reg: &IdbRegistry) {
        install_idb(self, Some(reg.clone()));
    }

    /// Injecte un registre `caches` partagé (Cache API) —
    /// même couture que [`Engine::install_idb`].
    pub fn install_caches(&mut self, reg: &CacheRegistry) {
        install_caches(self, Some(reg.clone()));
    }
}
