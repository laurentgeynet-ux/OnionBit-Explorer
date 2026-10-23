// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `Blob`/`File`/`FileReader` + `URL.createObjectURL` (JS-C70).
//!
//! - `Blob` = `ObjKind::Blob { data: Rc<Vec<u8>>, ty }` — octets
//!   immuables partagés (`slice`/`clone` copient le `Rc`) ;
//!   `File` = même kind + `name`/`lastModified`/
//!   `webkitRelativePath` en own-props et `file_proto` en proto
//!   (`file_proto.proto = blob_proto` → `file instanceof Blob`).
//! - `FileReader` : lectures en mémoire livrées au checkpoint
//!   microtask (même couture que les observers JS-C69 —
//!   approximation documentée des tasks spec) : `loadstart` →
//!   `progress` → `load` → `loadend`, `readyState`,
//!   `result`/`error`, `abort`.
//! - `URL.createObjectURL` : registre `blob:<origin>/<n>` →
//!   `(octets, type)` — `fetch("blob:…")` résout en `Response`.
//!   `revokeObjectURL` supprime silencieusement.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::heap::{Host, JsResult, ObjKind};
use crate::value::{ObjId, Value};
use crate::Engine;

/// État moteur des fichiers — protos paresseux, registre
/// d'URL objets et lectures `FileReader` en cours.
#[derive(Default)]
pub(crate) struct FilesState {
    /// Prochain suffixe d'URL objet (`blob:<origin>/<n>` —
    /// compteur, pas un UUID : unicité moteur suffisante).
    next_url: u64,
    /// `createObjectURL` — url → `(octets, type)` ; l'entrée
    /// garde le contenu en vie (spec : l'URL référence le
    /// blob). `revokeObjectURL` la supprime.
    blob_urls: HashMap<String, (Rc<Vec<u8>>, String)>,
    /// Prototypes paresseux `Blob`/`File`/`FileReader`.
    blob_proto: Option<ObjId>,
    file_proto: Option<ObjId>,
    reader_proto: Option<ObjId>,
    /// Wrappers `FileReader` avec une lecture en file
    /// (`readyState == LOADING`) — drainés au checkpoint et
    /// racines GC tant que la lecture est en cours.
    pending_reads: Vec<ObjId>,
}

impl FilesState {
    /// Racines GC : protos installés + readers en lecture (le
    /// `Host::trace` du wrapper couvre ensuite listeners et
    /// `result`).
    pub(crate) fn root_ids(&self, ids: &mut Vec<ObjId>) {
        for p in [self.blob_proto, self.file_proto, self.reader_proto]
            .into_iter()
            .flatten()
        {
            ids.push(p);
        }
        ids.extend(self.pending_reads.iter().copied());
    }
}

// ------------------------------------------------------------------
// Blob / File — données
// ------------------------------------------------------------------

/// `v` est-il un `Blob`/`File` ? → `(octets, type)` partagés.
pub(crate) fn blob_parts(e: &Engine, v: Value) -> Option<(Rc<Vec<u8>>, String)> {
    let Value::Obj(id) = v else { return None };
    match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::Blob { data, ty }) => Some((data.clone(), ty.clone())),
        _ => None,
    }
}

/// `v` est-il un `Blob`/`File` ?
pub(crate) fn is_blob(e: &Engine, v: Value) -> bool {
    blob_parts(e, v).is_some()
}

/// Nom de fichier de `v` — `Some` pour un `File` (own-prop
/// `name`), `None` pour un `Blob` nu (spec : `append(fd)`
/// nomme `File`, `Blob` → `"blob"` par défaut).
pub(crate) fn file_name_of(e: &mut Engine, v: Value) -> Option<String> {
    if !is_blob(e, v) {
        return None;
    }
    let Value::Obj(id) = v else { return None };
    e.get_prop(id, "name")
        .and_then(|n| e.heap_string_of(n))
        .filter(|n| !n.is_empty())
}

/// `Blob.prototype` paresseux — méthodes `text`/`arrayBuffer`/
/// `slice`/`stream` (JS-C71 — `ReadableStream` des octets).
fn blob_proto(e: &mut Engine) -> ObjId {
    if let Some(p) = e.files.blob_proto {
        return p;
    }
    let p = e.new_object();
    e.add_method(p, "text", blob_text);
    e.add_method(p, "arrayBuffer", blob_array_buffer);
    e.add_method(p, "slice", blob_slice);
    e.add_method(p, "stream", blob_stream);
    let tag = e.str_val("Blob");
    e.set_own(p, crate::interp::SYM_TO_STRING_TAG, tag, false);
    e.files.blob_proto = Some(p);
    p
}

/// `File.prototype` paresseux — proto = `Blob.prototype`
/// (chaîne `File → Blob → Object.prototype` spec).
fn file_proto(e: &mut Engine) -> ObjId {
    if let Some(p) = e.files.file_proto {
        return p;
    }
    let bp = blob_proto(e);
    let p = e
        .heap
        .alloc_obj(crate::heap::Object::ordinary(Value::Obj(bp)));
    let tag = e.str_val("File");
    e.set_own(p, crate::interp::SYM_TO_STRING_TAG, tag, false);
    e.files.file_proto = Some(p);
    p
}

/// Objet `Blob` complet (proto + `size`/`type` own) — partagé
/// par le ctor, `res.blob()` et `structuredClone`.
pub(crate) fn blob_value(e: &mut Engine, data: Vec<u8>, ty: String) -> Value {
    let bp = blob_proto(e);
    blob_value_rc(e, &Rc::new(data), ty, Value::Obj(bp))
}

/// `blob_value` cœur — proto choisi (`File` pour les File),
/// octets par `Rc` (pas de copie au `slice`/clone).
fn blob_value_rc(e: &mut Engine, data: &Rc<Vec<u8>>, ty: String, proto: Value) -> Value {
    let id = e.heap.alloc_obj(crate::heap::Object {
        proto,
        props: Vec::new(),
        kind: ObjKind::Blob {
            data: data.clone(),
            ty: ty.clone(),
        },
        extensible: true,
        elem_integrity: 0,
        args_map: None,
    });
    e.set_own(id, "size", Value::Num(data.len() as f64), true);
    let tv = e.str_val(ty);
    e.set_own(id, "type", tv, true);
    Value::Obj(id)
}

/// Concatène les `parts` d'un `Blob`/`File` (spec
/// `processBlobParts`) — chaînes en UTF-8, vues/buffers en
/// octets vus, `Blob` en contenu.
fn blob_parts_bytes(e: &mut Engine, parts: Value) -> JsResult<Vec<u8>> {
    let mut out = Vec::new();
    if matches!(parts, Value::Undefined) {
        return Ok(out);
    }
    for v in e.iter_values(parts)? {
        if let Some((data, _)) = blob_parts(e, v) {
            out.extend_from_slice(&data);
        } else if let Some(bytes) = crate::typed_array::bytes_of_value(e, v) {
            out.extend_from_slice(&bytes);
        } else {
            let s = e.to_js_string(v)?;
            out.extend_from_slice(s.as_bytes());
        }
    }
    Ok(out)
}

/// `options.type` normalisé : minuscule ; contient un caractère
/// hors `[0x20,0x7E]` → `""` (spec).
fn blob_norm_type(e: &mut Engine, opts: Value) -> JsResult<String> {
    let v = match opts {
        Value::Obj(o) => e.get_prop(o, "type").unwrap_or(Value::Undefined),
        _ => Value::Undefined,
    };
    if matches!(v, Value::Undefined) {
        return Ok(String::new());
    }
    let s = e.to_js_string(v)?;
    Ok(if s.bytes().all(|b| (0x20..=0x7e).contains(&b)) {
        s.to_lowercase()
    } else {
        String::new()
    })
}

/// `new Blob(parts?, {type?, endings?})` — `endings: "native"`
/// accepté (EOL plateforme = `\n` ici, identité).
fn blob_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let opts = arg(args, 1);
    if let Value::Obj(o) = opts {
        if let Some(en) = e.get_prop(o, "endings") {
            if !matches!(en, Value::Undefined) {
                let s = e.to_js_string(en)?;
                if !matches!(s.as_str(), "transparent" | "native") {
                    return Err(e.type_error("Blob: invalid endings option"));
                }
            }
        }
    }
    let ty = blob_norm_type(e, opts)?;
    let bytes = blob_parts_bytes(e, arg(args, 0))?;
    Ok(blob_value(e, bytes, ty))
}

/// `new File(parts, name, {type?, lastModified?})` — `name`
/// requis (spec) ; `lastModified` défaut = ms epoch courant.
fn file_ctor(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    if args.len() < 2 {
        return Err(e.type_error("File: 2 arguments required"));
    }
    let opts = arg(args, 2);
    let ty = blob_norm_type(e, opts)?;
    let name = e.to_js_string(arg(args, 1))?;
    let lm = match opts {
        Value::Obj(o) => match e.get_prop(o, "lastModified") {
            Some(v) if !matches!(v, Value::Undefined) => e.to_number(v)?,
            _ => now_ms(),
        },
        _ => now_ms(),
    };
    let bytes = blob_parts_bytes(e, arg(args, 0))?;
    let fp = file_proto(e);
    let v = blob_value_rc(e, &Rc::new(bytes), ty, Value::Obj(fp));
    if let Value::Obj(id) = v {
        let n = e.str_val(name);
        e.set_own(id, "name", n, true);
        e.set_own(id, "lastModified", Value::Num(lm), true);
        let empty = e.str_val("");
        e.set_own(id, "webkitRelativePath", empty, true);
    }
    Ok(v)
}

/// Millisecondes depuis epoch — `lastModified` par défaut.
fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_millis() as f64)
}

/// Receveur `Blob` → `(octets, type)` — `TypeError` sinon.
fn blob_this(e: &mut Engine, this: Value) -> JsResult<(Rc<Vec<u8>>, String)> {
    match blob_parts(e, this) {
        Some(p) => Ok(p),
        None => Err(e.type_error("Blob method on incompatible receiver")),
    }
}

/// `blob.text()` → promesse du texte (UTF-8 lossy — même
/// décodage que les corps réseau du transport `String`).
fn blob_text(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let (data, _) = blob_this(e, this)?;
    let pid = e.new_promise();
    let s = e.str_val(String::from_utf8_lossy(&data).into_owned());
    e.resolve_promise(pid, s);
    Ok(Value::Obj(pid))
}

/// `blob.arrayBuffer()` → promesse `ArrayBuffer` (copie —
/// l'AB est mutable, le `Rc` partagé ne l'est pas).
fn blob_array_buffer(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let (data, _) = blob_this(e, this)?;
    let pid = e.new_promise();
    let id = e.heap.alloc_obj(crate::heap::Object {
        proto: Value::Obj(e.protos.array_buffer),
        props: Vec::new(),
        kind: ObjKind::ByteBuf((*data).clone()),
        extensible: true,
        elem_integrity: 0,
        args_map: None,
    });
    e.resolve_promise(pid, Value::Obj(id));
    Ok(Value::Obj(pid))
}

/// `blob.stream()` → `ReadableStream` d'un chunk `Uint8Array`
/// (JS-C71 — contenu mémoire, livré d'un bloc).
fn blob_stream(e: &mut Engine, this: Value, _a: &[Value]) -> JsResult<Value> {
    let (data, _) = blob_this(e, this)?;
    Ok(crate::streams::body_stream(e, &data))
}

/// `blob.slice(start?, end?, contentType?)` — index relatifs à
/// `size` (spec), partage le `Rc` (contenu immuable).
fn blob_slice(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let (data, _ty) = blob_this(e, this)?;
    let size = data.len() as i64;
    // Spec : indice relatif = clamp(size + x, 0, size).
    let rel = |x: f64| -> usize {
        if x < 0.0 {
            (size + x as i64).max(0) as usize
        } else {
            (x as i64).min(size) as usize
        }
    };
    let start = match arg(args, 0) {
        Value::Undefined => 0,
        v => rel(e.to_number(v)?),
    };
    let end = match arg(args, 1) {
        Value::Undefined => data.len(),
        v => rel(e.to_number(v)?),
    };
    let span = end.saturating_sub(start);
    let ty = match arg(args, 2) {
        Value::Undefined => String::new(),
        v => {
            let s = e.to_js_string(v)?;
            if s.bytes().all(|b| (0x20..=0x7e).contains(&b)) {
                s.to_lowercase()
            } else {
                String::new()
            }
        }
    };
    let bytes = data.get(start..start + span).unwrap_or(&[]).to_vec();
    Ok(blob_value(e, bytes, ty))
}

// ------------------------------------------------------------------
// FileReader
// ------------------------------------------------------------------

/// Format de lecture demandé.
#[derive(Clone, Copy)]
enum ReadAs {
    /// `readAsText` — UTF-8 lossy.
    Text,
    /// `readAsDataURL` — `data:<type>;base64,…`.
    DataUrl,
    /// `readAsArrayBuffer` — `ByteBuf`.
    ArrayBuffer,
    /// `readAsBinaryString` — byte → code unit (`ByteString`).
    BinaryString,
}

/// Données partagées d'un `FileReader` (spec slots).
struct ReaderData {
    /// `0` EMPTY, `1` LOADING, `2` DONE.
    ready: u8,
    /// Résultat de la dernière lecture (`null` initial).
    result: Value,
    /// Erreur (`null` — une lecture mémoire ne peut guère
    /// échouer ; le slot existe pour la surface spec).
    error: Value,
    /// `addEventListener` handlers `(type, fn, capture)`.
    listeners: Vec<(String, Value, bool)>,
    /// Lecture en file → livrée au checkpoint.
    pending: Option<(Rc<Vec<u8>>, String, ReadAs)>,
}

/// Host `FileReader`.
struct FileReaderHost {
    data: Rc<RefCell<ReaderData>>,
}

/// `ReaderData` d'un `Value` `FileReader` — `None` sinon.
fn reader_data(e: &Engine, v: Value) -> Option<Rc<RefCell<ReaderData>>> {
    let Value::Obj(id) = v else { return None };
    match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::Host(h)) => h
            .clone()
            .as_any()
            .downcast_ref::<FileReaderHost>()
            .map(|r| r.data.clone()),
        _ => None,
    }
}

/// `new FileReader()` — proto pour `instanceof` + toStringTag.
fn file_reader_ctor(e: &mut Engine, _t: Value, _a: &[Value]) -> JsResult<Value> {
    let p = reader_proto(e);
    let v = e.new_host_object(Rc::new(FileReaderHost {
        data: Rc::new(RefCell::new(ReaderData {
            ready: 0,
            result: Value::Null,
            error: Value::Null,
            listeners: Vec::new(),
            pending: None,
        })),
    }));
    if let Value::Obj(id) = v {
        if let Some(o) = e.heap.obj_mut(id) {
            o.proto = Value::Obj(p);
        }
    }
    Ok(v)
}

/// `FileReader.prototype` paresseux — constantes spec +
/// toStringTag (les méthodes passent par `invoke` du host).
fn reader_proto(e: &mut Engine) -> ObjId {
    if let Some(p) = e.files.reader_proto {
        return p;
    }
    let p = e.new_object();
    for (k, n) in [("EMPTY", 0.0), ("LOADING", 1.0), ("DONE", 2.0)] {
        e.set_own(p, k, Value::Num(n), true);
    }
    let tag = e.str_val("FileReader");
    e.set_own(p, crate::interp::SYM_TO_STRING_TAG, tag, false);
    e.files.reader_proto = Some(p);
    p
}

impl Engine {
    /// Livraison des lectures `FileReader` en file (JS-C70) —
    /// séquence `loadstart` → `progress` → `load` → `loadend`
    /// par reader, ordre d'enregistrement ; un `abort` entre-
    /// temps retire la lecture (`pending` vidé → sautée).
    ///
    /// Retourne le nombre de readers notifiés.
    ///
    /// # Errors
    /// Propage la première exception d'un handler (même modèle
    /// que les listeners DOM).
    pub(crate) fn deliver_file_reads(&mut self) -> JsResult<usize> {
        let pending = std::mem::take(&mut self.files.pending_reads);
        let mut n = 0usize;
        for wid in pending {
            let Some(data) = reader_data(self, Value::Obj(wid)) else {
                continue;
            };
            let Some((bytes, ty, how)) = data.borrow_mut().pending.take() else {
                continue;
            };
            n += 1;
            let total = bytes.len() as f64;
            self.fire_reader_event(wid, &data, "loadstart", 0.0, total)?;
            let result = match how {
                ReadAs::Text => self.str_val(String::from_utf8_lossy(&bytes).into_owned()),
                ReadAs::DataUrl => {
                    let med = if ty.is_empty() {
                        "application/octet-stream"
                    } else {
                        ty.as_str()
                    };
                    self.str_val(format!(
                        "data:{med};base64,{}",
                        crate::typed_array::b64_encode(&bytes)
                    ))
                }
                ReadAs::ArrayBuffer => {
                    let id = self.heap.alloc_obj(crate::heap::Object {
                        proto: Value::Obj(self.protos.array_buffer),
                        props: Vec::new(),
                        kind: ObjKind::ByteBuf((*bytes).clone()),
                        extensible: true,
                        elem_integrity: 0,
                        args_map: None,
                    });
                    Value::Obj(id)
                }
                ReadAs::BinaryString => {
                    self.str_val(bytes.iter().map(|&b| b as char).collect::<String>())
                }
            };
            {
                let mut d = data.borrow_mut();
                d.result = result;
                d.ready = 2;
            }
            self.fire_reader_event(wid, &data, "progress", total, total)?;
            self.fire_reader_event(wid, &data, "load", total, total)?;
            self.fire_reader_event(wid, &data, "loadend", total, total)?;
        }
        Ok(n)
    }

    /// Event `ProgressEvent` minimal `{type, target,
    /// lengthComputable, loaded, total}` : `on<ty>` own prop
    /// puis `addEventListener` — même fan-out que XHR.
    fn fire_reader_event(
        &mut self,
        wrapper: ObjId,
        data: &Rc<RefCell<ReaderData>>,
        ty: &str,
        loaded: f64,
        total: f64,
    ) -> JsResult<()> {
        let this = Value::Obj(wrapper);
        let ev = {
            let o = self.new_object();
            let t = self.str_val(ty);
            self.set_own(o, "type", t, true);
            self.set_own(o, "target", this, true);
            self.set_own(o, "lengthComputable", Value::Bool(true), true);
            self.set_own(o, "loaded", Value::Num(loaded), true);
            self.set_own(o, "total", Value::Num(total), true);
            Value::Obj(o)
        };
        let key = format!("on{ty}");
        if let Some(f) = self.get_prop(wrapper, &key) {
            if self.is_callable(f) {
                self.call_value(f, this, &[ev])?;
            }
        }
        let fns: Vec<Value> = data
            .borrow()
            .listeners
            .iter()
            .filter(|(t, _, _)| t == ty)
            .map(|(_, f, _)| *f)
            .collect();
        for f in fns {
            if self.is_callable(f) {
                self.call_value(f, this, &[ev])?;
            }
        }
        Ok(())
    }
}

impl Host for FileReaderHost {
    fn class_name(&self) -> &'static str {
        "FileReader"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn get(&self, _e: &mut Engine, _t: Value, key: &str) -> JsResult<Option<Value>> {
        let d = self.data.borrow();
        Ok(Some(match key {
            "readyState" => Value::Num(f64::from(d.ready)),
            "result" => d.result,
            "error" => d.error,
            "EMPTY" => Value::Num(0.0),
            "LOADING" => Value::Num(1.0),
            "DONE" => Value::Num(2.0),
            _ => return Ok(None),
        }))
    }

    fn invoke(
        &self,
        e: &mut Engine,
        this: Value,
        name: &str,
        args: &[Value],
    ) -> JsResult<Option<Value>> {
        match name {
            "readAsText" | "readAsDataURL" | "readAsArrayBuffer" | "readAsBinaryString" => {
                let blob = arg(args, 0);
                let Some((data, ty)) = blob_parts(e, blob) else {
                    return Err(e.type_error("FileReader: argument is not a Blob"));
                };
                let how = match name {
                    "readAsText" => ReadAs::Text,
                    "readAsDataURL" => ReadAs::DataUrl,
                    "readAsArrayBuffer" => ReadAs::ArrayBuffer,
                    _ => ReadAs::BinaryString,
                };
                {
                    let mut d = self.data.borrow_mut();
                    // Spec : un readAs* pendant LOADING remplace la
                    // lecture en cours (atomique mémoire ici).
                    d.pending = Some((data, ty, how));
                    d.ready = 1;
                }
                if let Value::Obj(wid) = this {
                    if !e.files.pending_reads.contains(&wid) {
                        e.files.pending_reads.push(wid);
                    }
                }
                Ok(Some(Value::Undefined))
            }
            "abort" => {
                let had_read = {
                    let mut d = self.data.borrow_mut();
                    let was = d.pending.take().is_some();
                    if was {
                        d.ready = 2;
                        d.result = Value::Null;
                    }
                    was
                };
                if had_read {
                    if let Value::Obj(wid) = this {
                        // Spec : `abort` = event `abort` puis
                        // `loadend` (terminaison de la lecture).
                        e.fire_reader_event(wid, &self.data, "abort", 0.0, 0.0)?;
                        e.fire_reader_event(wid, &self.data, "loadend", 0.0, 0.0)?;
                    }
                }
                Ok(Some(Value::Undefined))
            }
            "addEventListener" => {
                let ty = e.to_js_string(arg(args, 0))?;
                let f = arg(args, 1);
                let capture = e.to_boolean(arg(args, 2));
                if e.is_callable(f) {
                    self.data.borrow_mut().listeners.push((ty, f, capture));
                }
                Ok(Some(Value::Undefined))
            }
            "removeEventListener" => {
                let ty = e.to_js_string(arg(args, 0))?;
                let f = arg(args, 1);
                self.data
                    .borrow_mut()
                    .listeners
                    .retain(|(t, g, _)| !(*t == ty && e.strict_eq(*g, f)));
                Ok(Some(Value::Undefined))
            }
            _ => Ok(None),
        }
    }

    fn trace(&self, out: &mut Vec<ObjId>) {
        let d = self.data.borrow();
        for (_, cb, _) in &d.listeners {
            if let Value::Obj(id) = cb {
                out.push(*id);
            }
        }
        crate::heap::push_value(out, d.result);
        crate::heap::push_value(out, d.error);
    }
}

// ------------------------------------------------------------------
// URL.createObjectURL / revokeObjectURL
// ------------------------------------------------------------------

/// Origine des URL objets — `hooks.page_url` ou `"null"`.
fn object_url_origin(e: &Engine) -> String {
    let base = e.hooks.page_url.clone();
    crate::builtins::url::url_parse_absolute(&base).map_or_else(
        || "null".to_string(),
        |p| crate::builtins::url::url_origin(&p),
    )
}

/// `URL.createObjectURL(blob)` — registre `blob:<origin>/<n>`
/// (contenu gardé en vie par le `Rc`, spec).
pub(crate) fn url_create_object_url(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let v = arg(args, 0);
    let Some((data, ty)) = blob_parts(e, v) else {
        return Err(e.type_error("URL.createObjectURL: argument is not a Blob"));
    };
    e.files.next_url += 1;
    let url = format!("blob:{}/{}", object_url_origin(e), e.files.next_url);
    e.files.blob_urls.insert(url.clone(), (data, ty));
    Ok(e.str_val(url))
}

/// `URL.revokeObjectURL(url)` — suppression silencieuse
/// (spec : aucune erreur sur URL inconnue).
pub(crate) fn url_revoke_object_url(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    if let Ok(u) = e.to_js_string(arg(args, 0)) {
        e.files.blob_urls.remove(&u);
    }
    Ok(Value::Undefined)
}

/// `fetch("blob:<origin>/<n>")` → `(octets, type)` de l'URL
/// objet enregistrée — `None` sinon.
pub(crate) fn blob_url_lookup(e: &Engine, url: &str) -> Option<(Rc<Vec<u8>>, String)> {
    e.files.blob_urls.get(url).cloned()
}

/// `FormData.append(name, blob, filename)` — spec : le blob
/// est emballé en `File` portant `filename` (lastModified =
/// maintenant) ; `None` si `v` n'est pas un `Blob`.
pub(crate) fn file_wrap_named(e: &mut Engine, v: Value, filename: String) -> Option<Value> {
    let (data, ty) = blob_parts(e, v)?;
    let fp = file_proto(e);
    let out = blob_value_rc(e, &data, ty, Value::Obj(fp));
    if let Value::Obj(id) = out {
        let n = e.str_val(filename);
        e.set_own(id, "name", n, true);
        e.set_own(id, "lastModified", Value::Num(now_ms()), true);
        let empty = e.str_val("");
        e.set_own(id, "webkitRelativePath", empty, true);
    }
    Some(out)
}

/// Installe `Blob`, `File`, `FileReader` sur le global —
/// idempotent (`install_dom`/`install_fetch` l'appellent tous
/// deux pour couvrir les contextes sans DOM).
pub(crate) fn install_files(e: &mut Engine) {
    let g = e.global;
    if e.get_prop(g, "Blob").is_some() {
        return;
    }
    for (name, f) in [
        ("Blob", blob_ctor as crate::heap::NativeFn),
        ("File", file_ctor),
        ("FileReader", file_reader_ctor),
    ] {
        let fv = e.native_fn(name, f);
        if let Value::Obj(fid) = fv {
            let p = match name {
                "Blob" => Value::Obj(blob_proto(e)),
                "File" => Value::Obj(file_proto(e)),
                _ => Value::Obj(reader_proto(e)),
            };
            e.set_own(fid, "prototype", p, false);
        }
        e.set_own(g, name, fv, false);
    }
    // Constantes statiques `FileReader.EMPTY/LOADING/DONE` (spec
    // — doublon des props du proto et du host).
    if let Some(Value::Obj(fid)) = e.get_prop(g, "FileReader") {
        for (k, n) in [("EMPTY", 0.0), ("LOADING", 1.0), ("DONE", 2.0)] {
            e.set_own(fid, k, Value::Num(n), true);
        }
    }
    // `URL.createObjectURL`/`revokeObjectURL` — statiques sur le
    // ctor `URL` (installé par `builtins::install`, avant DOM).
    if let Some(Value::Obj(uid)) = e.get_prop(g, "URL") {
        e.add_method(uid, "createObjectURL", url_create_object_url);
        e.add_method(uid, "revokeObjectURL", url_revoke_object_url);
    }
}

/// Argument `args[i]` — `Undefined` hors rang (helper local,
/// même convention que `xhr::arg`).
fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).copied().unwrap_or(Value::Undefined)
}
