// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Media Source Extensions — surface JS honnête **sans lecture**
//! (JS-C193, amorce de la piste MSE — ADR-0024 : la lecture est
//! un service d'hôte, le moteur ne décode pas).
//!
//! Ce que le palier livre : `MediaSource`, `SourceBuffer`,
//! `SourceBufferList`, `TimeRanges` réels (constructibles,
//! `instanceof`, `EventTarget` via le rail `__et` partagé), la
//! machine `readyState`, `URL.createObjectURL(mediaSource)` et
//! `appendBuffer` accepté-borné. Ce qu'il ne livre pas —
//! délibérément, honnêteté ADR-0023 :
//!
//! - `isTypeSupported` rend `""` pour **tout** type : aucun
//!   conteneur n'est jouable tant que le transport d'octets vers
//!   l'hôte n'existe pas. Un `""` honnête pousse les players vers
//!   leurs fallbacks (`<video src>` progressif/HLS, servis par le
//!   seam `MediaSurface` existant) plutôt que vers un appendBuffer
//!   sans fin derrière un spinner.
//! - `readyState` reste `"closed"` : rien n'attache le
//!   `MediaSource` à un élément (`blob:` → `<video>` = étape
//!   dédiée côté couture média). `addSourceBuffer`/`appendBuffer`/
//!   `endOfStream` sont néanmoins **permis** (déviation spec
//!   assumée : l'`InvalidStateError` strict tuerait les boots qui
//!   construisent leurs buffers avant l'attache — le stall
//!   « closed » est un comportement de flux bloqué, pas un crash).
//! - `SourceBuffer.buffered` reste vide : les octets sont
//!   comptabilisés, pas stockés — « rien de lisible bufferisé »
//!   est vrai tant que l'hôte ne peut pas les consommer.
//! - Les événements `updatestart`/`update`/`updateend` sont tirés
//!   **en séquence synchrone** dans `appendBuffer` (approximation
//!   documentée du « queue a task » spec — même économie que les
//!   livraisons `FileReader` au checkpoint).
//!
//! Budgets (§8 AGENTS — `appendBuffer` est un vecteur OOM
//! canonique) : cap d'octets par `SourceBuffer`, cap de buffers
//! par `MediaSource`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::heap::{Host, JsResult, ObjKind, Thrown};
use crate::value::{ObjId, Value};
use crate::Engine;

/// Octets retenus par `SourceBuffer` — `appendBuffer` au-delà
/// jette `QuotaExceededError` (budget hostile).
const SB_BYTE_CAP: usize = 64 * 1024 * 1024;

/// Span nominal d'un segment `appendBuffer` quand aucun
/// delta `timestampOffset` n'a encore été observé (JS-C278 —
/// 6 s ≈ segment HLS web courant ; corrigé rétroactivement
/// dès qu'un second offset arrive — voir `sb_append_buffer`).
const EST_SEGMENT_SPAN: f64 = 6.0;

/// `SourceBuffer` vivants par `MediaSource` (un site hostile qui
/// alloue des buffers en boucle reste borné).
const MS_BUFFER_CAP: usize = 8;

/// Epsilon de couverture `remove` (JS-C287) — 1 ms contre
/// les bords flottants `tfdt`/timescale.
const REMOVE_EPS: f64 = 1e-3;

/// Cap de l'historique servi (JS-C296, chantier D) — la
/// fenêtre adressable que `?from=` peut re-servir. Au-delà,
/// les chunks les plus vieux sont évincés (un `from` dedans
/// retourne 416, honnêt).
const SB_HISTORY_CAP: usize = 64 * 1024 * 1024;

/// Lot servi par `mse_pull_segments` (JS-C295/296) :
/// `(octets, curseur, eos, sections)` — `Err` = `?from=`
/// hors fenêtre adressable (416 filaire).
pub(crate) type MsePullOut = Result<(Vec<u8>, u64, bool, Vec<u64>), ()>;

/// Chunk déjà livré à l'hôte, conservé pour `?from=` —
/// position absolue propre pour survivre aux trous
/// `remove`/éviction (JS-C296).
struct ServedChunk {
    /// Position absolue de `bytes[0]` dans le flux servi.
    start: u64,
    bytes: Vec<u8>,
    /// Plage temps revendiquée — sert à l'éviction `remove`.
    range: Option<(f64, f64)>,
    /// Offset intra-chunk d'une frontière de section
    /// (`moov` re-servi) — ré-émis en `X-Mse-Section`
    /// quand le replay inclut cette position.
    boundary: Option<u64>,
}

/// `pos` tombe dans un chunk servi conservé (JS-C296) —
/// `false` = trou `remove`/éviction cap/jamais servi.
fn served_covers(served: &std::collections::VecDeque<ServedChunk>, pos: u64) -> bool {
    served
        .iter()
        .any(|c| c.start <= pos && pos < c.start + c.bytes.len() as u64)
}

/// Bascule le prochain chunk de la file → historique
/// `served` en avançant `pulled` (JS-C296 — l'historique
/// devient la source de tout octet servi : le drain ne
/// fait plus que copier entre `cursor` et `pulled`).
/// Frontière `X-Mse-Section` : figée sur le chunk servi
/// si un `moov` est **re**-servi (`pulled_saw_init` — le
/// premier init n'est jamais une frontière) ; le drain
/// l'émet à la position livrée, à la première passe
/// comme au replay `?from=` — les octets sautés par un
/// seek avant gardent la leur pour un replay ultérieur.
/// `false` = file vide. Éviction `SB_HISTORY_CAP` en tête.
fn serve_next_chunk(sd: &mut SbData) -> bool {
    let Some(seg) = sd.segments.pop_front() else {
        return false;
    };
    let span = sd.seg_ranges.pop_front().flatten();
    let bound = sd.seg_boundary.pop_front().flatten();
    let start = sd.pulled;
    sd.bytes = sd.bytes.saturating_sub(seg.len());
    sd.pulled += seg.len() as u64;
    // `Some(off)` uniquement sur un re-init servi — le
    // drapeau est figé à la bascule pour rester vrai au
    // replay même si d'autres inits ont suivi.
    let boundary = bound.filter(|_| sd.pulled_saw_init);
    if bound.is_some() {
        sd.pulled_saw_init = true;
    }
    sd.served_bytes += seg.len();
    sd.served.push_back(ServedChunk {
        start,
        bytes: seg,
        range: span,
        boundary,
    });
    while sd.served_bytes > SB_HISTORY_CAP {
        let Some(old) = sd.served.pop_front() else {
            break;
        };
        sd.served_bytes = sd.served_bytes.saturating_sub(old.bytes.len());
    }
    true
}

/// État moteur MSE — protos paresseux + registre des URL objets
/// pointant un `MediaSource`. Document-local : swappé avec le
/// `BrowsingContext` comme `files`/`idb`.
#[derive(Default)]
pub(crate) struct MseState {
    media_source_proto: Option<ObjId>,
    source_buffer_proto: Option<ObjId>,
    buffer_list_proto: Option<ObjId>,
    time_ranges_proto: Option<ObjId>,
    /// `blob:<origin>/<n>` → wrapper `MediaSource`. L'entrée
    /// garde la source en vie (racine GC) jusqu'à
    /// `revokeObjectURL` ou la mort du document — consommée par
    /// la future couture d'offre `mse` (transport vers l'hôte).
    object_urls: HashMap<String, ObjId>,
    /// Préfixes de codecs déclarés par l'hôte (MSE-4) —
    /// alimentés par `set_media_claim` (`OnionCodec` déclare
    /// ce qu'il joue réellement) ; vide = `isTypeSupported`
    /// rend `""` partout (comportement C173 préservé).
    declared_codecs: std::collections::HashSet<String>,
    /// Événements `MediaSource` filés en **tâche** (JS-C324) —
    /// `(wrapper, type)`. Spec : `sourceopen`/`sourceended`
    /// sont mis en file, jamais tirés synchrones — le pattern
    /// canonique des players (`v.src = createObjectURL(ms)` puis
    /// `ms.addEventListener('sourceopen', …)` à la ligne
    /// suivante) ne voit l'événement que si le tir attend la fin
    /// du script courant ; un tir synchrone était perdu à
    /// jamais (dash.js inerte — terrain §B-83). Drainée à la
    /// frontière de job par [`deliver_mse_events`].
    pending_events: std::collections::VecDeque<(ObjId, &'static str)>,
}

impl MseState {
    /// Racines GC : protos installés + `MediaSource` référencés
    /// par une URL objet vivante ou cible d'un événement en
    /// file (le tir différé ne doit pas trouver un wrapper
    /// collecté).
    pub(crate) fn root_ids(&self, ids: &mut Vec<ObjId>) {
        for p in [
            self.media_source_proto,
            self.source_buffer_proto,
            self.buffer_list_proto,
            self.time_ranges_proto,
        ]
        .into_iter()
        .flatten()
        {
            ids.push(p);
        }
        ids.extend(self.object_urls.values().copied());
        ids.extend(self.pending_events.iter().map(|(t, _)| *t));
    }
}

// ------------------------------------------------------------------
// Données internes
// ------------------------------------------------------------------

/// `MediaSource` interne.
struct MsData {
    /// `"closed"` | `"open"` | `"ended"` — reste `"closed"` en
    /// C173 : aucune attache `blob:` → élément n'existe encore.
    ready_state: &'static str,
    /// Wrappers `SourceBuffer` (ordre d'ajout — spec).
    buffers: Vec<Value>,
    /// Wrapper `SourceBufferList` caché (`sourceBuffers ===
    /// sourceBuffers` reste vrai entre deux lectures).
    list: Option<Value>,
    /// `duration` — `NaN` tant que jamais fixée (spec).
    duration: f64,
    /// `setLiveSeekableRange` retenu (`None` = `clear`).
    live_range: Option<(f64, f64)>,
}

/// `SourceBuffer` interne.
struct SbData {
    /// Back-ref vers le wrapper `MediaSource` (tracé).
    ms: Value,
    /// MIME passé à `addSourceBuffer`/`changeType` — retenu
    /// pour le futur transport (le seam d'offre en aura
    /// besoin ; aucun getter spec ne l'expose).
    #[allow(dead_code)]
    mime: String,
    updating: bool,
    /// `"segments"` (défaut) | `"sequence"`.
    mode: String,
    timestamp_offset: f64,
    append_start: f64,
    append_end: f64,
    /// File FIFO des segments `appendBuffer` (MSE-3 transport)
    /// — ordre d'append strict (init puis media segments, le
    /// flux qu'un `ByteSource` MSE doit lire). `bytes` =
    /// total en file pour le cap `SB_BYTE_CAP` ; `pulled` =
    /// position absolue consommée (curseur du pull).
    segments: std::collections::VecDeque<Vec<u8>>,
    bytes: usize,
    pulled: u64,
    /// Plage média revendiquée par chaque chunk en file
    /// (JS-C287, parallèle à `segments`) — `None` = chunk
    /// sans média (init fMP4) ou plage inconnue. Sert à
    /// l'éviction par plage de `remove` : un chunk n'est
    /// jeté que si sa plage est **entièrement** couverte.
    seg_ranges: std::collections::VecDeque<Option<(f64, f64)>>,
    /// `Some(off)` par chunk porteur d'un `moov` (JS-C295,
    /// pont C — parallèle à `segments`) : `off` = position
    /// intra-chunk du `moov` racine (JS-C296 — un append
    /// mixte débute sa section au `moov`, pas au début).
    /// Le pull émet `X-Mse-Section` à cet offset **si** un
    /// init a déjà été servi (`pulled_saw_init`) : un init
    /// queued puis évincé par `remove`/`abort` ne marque
    /// rien, un re-init servi = frontière de section.
    seg_boundary: std::collections::VecDeque<Option<u64>>,
    /// Chunks déjà servis à l'hôte, conservés pour le
    /// repositionnement `?from=` (JS-C296, chantier D —
    /// `MseSource::seek` re-sert l'historique des appends).
    /// Chaque chunk porte sa position absolue : les trous
    /// créés par `remove`/éviction cap restent adressables
    /// de part et d'autre, `from` dans un trou → 416.
    served: std::collections::VecDeque<ServedChunk>,
    /// Total des octets en `served` — borné par
    /// `SB_HISTORY_CAP` (les plus vieux sont évincés).
    served_bytes: usize,
    /// Curseur de service absolu : `== pulled` en lecture
    /// FIFO normale, `< pulled` pendant un replay `?from=`
    /// (ne ré-alloue pas — les positions servies sont déjà
    /// consommées). `pulled` reste le filigrane d'allocation.
    cursor: u64,
    /// Un init `moov` a déjà été servi à l'hôte via le pull.
    pulled_saw_init: bool,
    /// `buffered` estimé (JS-C278/b3, ADR-0029) — plages
    /// déduites des `timestampOffset` d'append successifs,
    /// clippées à l'append window ; approximation documentée
    /// (pas de timestamps réels sans démuxage). `est_span` =
    /// dernier delta inter-offset observé, sinon nominal ;
    /// `last_append_t` = offset du dernier append — un offset
    /// croissant ferme la plage précédente à sa vraie durée.
    est_ranges: Vec<(f64, f64)>,
    est_span: f64,
    last_append_t: Option<f64>,
    /// Timescale média lu dans le `mdhd` de l'init segment
    /// fMP4 (JS-C285) — sert à convertir `tfdt`/durées
    /// `trun` des `moof` en secondes. `None` tant qu'aucun
    /// init fMP4 n'a été appendé (repli estimateur).
    timescale: Option<u32>,
}

struct MsHost {
    d: Rc<RefCell<MsData>>,
}

struct SbHost {
    d: Rc<RefCell<SbData>>,
}

/// `SourceBufferList` — vue vivante sur `MsData.buffers`.
struct SbListHost {
    d: Rc<RefCell<MsData>>,
}

/// `TimeRanges` — snapshot figé de plages `(start, end)`.
struct TrHost {
    ranges: Rc<Vec<(f64, f64)>>,
}

fn ms_data(e: &Engine, v: Value) -> Option<Rc<RefCell<MsData>>> {
    let Value::Obj(id) = v else { return None };
    match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::Host(h)) => h
            .clone()
            .as_any()
            .downcast_ref::<MsHost>()
            .map(|r| r.d.clone()),
        _ => None,
    }
}

fn sb_data(e: &Engine, v: Value) -> Option<Rc<RefCell<SbData>>> {
    let Value::Obj(id) = v else { return None };
    match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::Host(h)) => h
            .clone()
            .as_any()
            .downcast_ref::<SbHost>()
            .map(|r| r.d.clone()),
        _ => None,
    }
}

fn tr_ranges(e: &Engine, v: Value) -> Option<Rc<Vec<(f64, f64)>>> {
    let Value::Obj(id) = v else { return None };
    match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::Host(h)) => h
            .clone()
            .as_any()
            .downcast_ref::<TrHost>()
            .map(|r| r.ranges.clone()),
        _ => None,
    }
}

/// `v` est un wrapper `MediaSource` (pour `createObjectURL`).
pub(crate) fn is_media_source(e: &Engine, v: Value) -> bool {
    ms_data(e, v).is_some()
}

/// Erreur DOM nommée jetée — `{name, message}` (approximation
/// documentée du `DOMException`, même convention qu'IDB/crypto).
fn mse_err(e: &mut Engine, name: &str, msg: &str) -> Thrown {
    let o = e.new_object();
    let n = e.str_val(name);
    e.set_own(o, "name", n, true);
    let m = e.str_val(msg);
    e.set_own(o, "message", m, true);
    Thrown(Value::Obj(o))
}

/// Event minimal `{type, target, currentTarget}`.
fn mse_event(e: &mut Engine, ty: &str, target: Value) -> Value {
    let o = e.new_object();
    let t = e.str_val(ty);
    e.set_own(o, "type", t, true);
    e.set_own(o, "target", target, true);
    e.set_own(o, "currentTarget", target, true);
    Value::Obj(o)
}

/// `on<ty>` own-prop + listeners `addEventListener` (rail `__et`
/// partagé — même helper que `fire_idb`).
fn fire_mse(e: &mut Engine, target: ObjId, ty: &str) -> JsResult<()> {
    let ev = mse_event(e, ty, Value::Obj(target));
    let key = format!("on{ty}");
    if let Some(f) = e.get_prop(target, &key) {
        if e.is_callable(f) {
            e.call_value(f, Value::Obj(target), &[ev])?;
        }
    }
    if let Some(Value::Obj(map)) = e.get_prop(target, "__et") {
        if let Some(Value::Obj(l)) = e.get_prop(map, ty) {
            let len = match e.get_prop(l, "length") {
                Some(Value::Num(n)) => n as usize,
                _ => 0,
            };
            for i in 0..len {
                if let Some(cb) = e.get_prop(l, &i.to_string()) {
                    if e.is_callable(cb) {
                        e.call_value(cb, Value::Obj(target), &[ev])?;
                    }
                }
            }
        }
    }
    Ok(())
}

/// Tire `fire_mse` en capturant la première erreur (JS-C283) :
/// un listener qui jette ne doit **jamais** corrompre la machine
/// à états — `updating` figé `true` gèle `blockBuffers` des
/// players (hls.js n'appelle plus `endOfStream`, file MSE morte
/// — vol hls b-e2e). `updateend` est toujours tiré ensuite ; la
/// première erreur est propagée à l'appelant après nettoyage.
fn fire_mse_guarded(
    e: &mut Engine,
    target: ObjId,
    ty: &str,
    first: &mut Option<crate::heap::Thrown>,
) {
    if let Err(err) = fire_mse(e, target, ty) {
        if first.is_none() {
            *first = Some(err);
        }
    }
}

// ------------------------------------------------------------------
// Protos paresseux
// ------------------------------------------------------------------

fn tagged_proto(e: &mut Engine, tag: &str) -> ObjId {
    let p = e.new_object();
    let t = e.str_val(tag);
    e.set_own(p, crate::interp::SYM_TO_STRING_TAG, t, false);
    p
}

/// Méthodes `EventTarget` communes (`__et` partagé).
fn install_et_methods(e: &mut Engine, p: ObjId) {
    e.add_method(p, "addEventListener", crate::dom_host::et_add_listener);
    e.add_method(
        p,
        "removeEventListener",
        crate::dom_host::et_remove_listener,
    );
    e.add_method(p, "dispatchEvent", crate::dom_host::et_dispatch_event);
}

fn ms_proto(e: &mut Engine) -> ObjId {
    if let Some(p) = e.mse.media_source_proto {
        return p;
    }
    let p = tagged_proto(e, "MediaSource");
    install_et_methods(e, p);
    e.add_method(p, "addSourceBuffer", ms_add_source_buffer);
    e.add_method(p, "removeSourceBuffer", ms_remove_source_buffer);
    e.add_method(p, "endOfStream", ms_end_of_stream);
    e.add_method(p, "setLiveSeekableRange", ms_set_live_range);
    e.add_method(p, "clearLiveSeekableRange", ms_clear_live_range);
    e.mse.media_source_proto = Some(p);
    p
}

fn sb_proto(e: &mut Engine) -> ObjId {
    if let Some(p) = e.mse.source_buffer_proto {
        return p;
    }
    let p = tagged_proto(e, "SourceBuffer");
    install_et_methods(e, p);
    e.add_method(p, "appendBuffer", sb_append_buffer);
    e.add_method(p, "abort", sb_abort);
    e.add_method(p, "remove", sb_remove);
    e.add_method(p, "changeType", sb_change_type);
    e.mse.source_buffer_proto = Some(p);
    p
}

fn sbl_proto(e: &mut Engine) -> ObjId {
    if let Some(p) = e.mse.buffer_list_proto {
        return p;
    }
    let p = tagged_proto(e, "SourceBufferList");
    install_et_methods(e, p);
    e.mse.buffer_list_proto = Some(p);
    p
}

fn tr_proto(e: &mut Engine) -> ObjId {
    if let Some(p) = e.mse.time_ranges_proto {
        return p;
    }
    let p = tagged_proto(e, "TimeRanges");
    e.add_method(p, "start", tr_start);
    e.add_method(p, "end", tr_end);
    e.mse.time_ranges_proto = Some(p);
    p
}

/// Wrapper `TimeRanges` figé sur un snapshot de plages.
fn time_ranges(e: &mut Engine, ranges: Vec<(f64, f64)>) -> Value {
    let tp = tr_proto(e);
    let v = e.new_host_object(Rc::new(TrHost {
        ranges: Rc::new(ranges),
    }));
    if let Value::Obj(id) = v {
        if let Some(o) = e.heap.obj_mut(id) {
            o.proto = Value::Obj(tp);
        }
    }
    v
}

/// Wrapper `SourceBufferList` caché sur le `MediaSource`.
fn buffer_list(e: &mut Engine, d: &Rc<RefCell<MsData>>) -> Value {
    if let Some(l) = d.borrow().list {
        return l;
    }
    let lp = sbl_proto(e);
    let v = e.new_host_object(Rc::new(SbListHost { d: d.clone() }));
    if let Value::Obj(id) = v {
        if let Some(o) = e.heap.obj_mut(id) {
            o.proto = Value::Obj(lp);
        }
        d.borrow_mut().list = Some(v);
    }
    v
}

// ------------------------------------------------------------------
// Constructeurs
// ------------------------------------------------------------------

/// `new MediaSource()` — source `closed` vide.
fn ms_ctor(e: &mut Engine, _t: Value, _args: &[Value]) -> JsResult<Value> {
    let mp = ms_proto(e);
    let d = Rc::new(RefCell::new(MsData {
        ready_state: "closed",
        buffers: Vec::new(),
        list: None,
        duration: f64::NAN,
        live_range: None,
    }));
    let v = e.new_host_object(Rc::new(MsHost { d }));
    if let Value::Obj(id) = v {
        if let Some(o) = e.heap.obj_mut(id) {
            o.proto = Value::Obj(mp);
        }
    }
    Ok(v)
}

/// `SourceBuffer`/`SourceBufferList`/`TimeRanges` — interfaces
/// non constructibles (spec : `Illegal constructor`) mais à
/// prototype réel pour `instanceof`/feature-detect.
fn sb_illegal_ctor(e: &mut Engine, _t: Value, _args: &[Value]) -> JsResult<Value> {
    Err(e.type_error("SourceBuffer: Illegal constructor"))
}

fn sbl_illegal_ctor(e: &mut Engine, _t: Value, _args: &[Value]) -> JsResult<Value> {
    Err(e.type_error("SourceBufferList: Illegal constructor"))
}

fn tr_illegal_ctor(e: &mut Engine, _t: Value, _args: &[Value]) -> JsResult<Value> {
    Err(e.type_error("TimeRanges: Illegal constructor"))
}

/// Codecs déclarés par l'hôte (claim média) — normalisés en
/// préfixe (`avc1.42E01E` → `avc1`) et fusionnés dans la
/// table document-locale que `isTypeSupported` consulte
/// (MSE-4, ADR-0029 : la table déclare ce que la stack
/// `OnionCodec` joue réellement — honnête par construction).
pub(crate) fn note_declared_codecs(e: &mut Engine, codecs: &[String]) {
    for c in codecs {
        let c = c.trim();
        // L'hôte déclare soit un codec nu (`avc1.42E01E`), soit
        // un MIME complet (`video/mp4;codecs="avc1.4d,mp4a.40"`)
        // — la liste `codecs=` prime ; sans elle, la valeur est
        // prise pour un codec nu (vol 177 : le claim en MIME
        // complet n'alimentait jamais la table →
        // `isTypeSupported(mime;codecs)` restait `""`).
        let raw = match c.find("codecs=") {
            Some(pos) => &c[pos + 7..],
            None => c,
        };
        for part in raw.trim_matches(|ch| ch == '"' || ch == '\'').split(',') {
            let part = part.trim().trim_matches(|ch| ch == '"' || ch == '\'');
            // `video/mp4` nu (sans `codecs=`) = conteneur, pas un
            // codec — rien à déclarer.
            if part.contains('/') {
                continue;
            }
            if let Some(p) = codec_prefix(part) {
                e.mse.declared_codecs.insert(p);
            }
        }
    }
}

/// Préfixe codec normalisé : `avc1.42E01E`→`avc1`,
/// `vp09.00.10.08`→`vp09`, `mp4a.40.2`→`mp4a` — la famille
/// suffit au matching déclaré (profil/niveau = affinement).
fn codec_prefix(s: &str) -> Option<String> {
    let base = s.trim();
    let p = base.split('.').next()?.trim();
    if p.is_empty() {
        return None;
    }
    Some(p.to_ascii_lowercase())
}

/// `MediaSource.isTypeSupported(mime)` — MSE-4 : pilote par
/// la table de codecs **déclarés par l'hôte** (`set_media_claim`
/// — `OnionCodec` dit ce qu'il joue). Conteneur connu + tous les
/// codecs du param `codecs` déclarés → `"maybe"` ; conteneur
/// connu sans `codecs` → `"maybe"` ; sinon `""`. Table vide →
/// `""` partout (état C173 préservé — jamais de promesse sans
/// transport déclaré, ADR-0023).
fn ms_is_type_supported(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let mime = e
        .to_js_string(args.first().copied().unwrap_or(Value::Undefined))?
        .to_lowercase();
    // Conteneur `type/subtype` — la borne `;` isole `codecs`.
    let container = mime.split(';').next().unwrap_or("").trim();
    let known = matches!(
        container,
        "video/mp4"
            | "audio/mp4"
            | "video/webm"
            | "audio/webm"
            | "video/x-matroska"
            | "audio/mpeg"
            | "video/mp2t"
            | "audio/ogg"
            | "audio/wav"
            | "audio/x-wav"
    );
    let ok = known
        && match mime.split("codecs=").nth(1) {
            Some(cstr) => {
                let cstr = cstr.trim_matches(|c| c == '"' || c == '\'');
                cstr.split(',')
                    .filter_map(codec_prefix)
                    .all(|p| e.mse.declared_codecs.contains(&p))
            }
            // Pas de liste : le conteneur connu suffit —
            // `"maybe"` honnête (le codec réel peut manquer).
            None => !e.mse.declared_codecs.is_empty(),
        };
    Ok(e.str_val(if ok { "maybe" } else { "" }))
}

/// `HTMLMediaElement.canPlayType(mime)` (§B-82) — même table
/// honnête `declared_codecs` que `isTypeSupported` (l'hôte
/// déclare ce que la stack `OnionCodec` joue — ADR-0029) :
/// conteneur connu + tous les codecs du param `codecs`
/// déclarés → `"probably"` ; conteneur connu sans `codecs`
/// → `"maybe"` ; sinon `""`. Table vide → `""` partout
/// (jamais de promesse sans transport déclaré, ADR-0023).
///
/// Appelable sur tout élément média — claimé ou non : les
/// codecs d'un claim sont déjà repliés dans la table à
/// `set_media_claim`, donc la lecture est unifiée (la gate
/// d'attach des players — dash.js/shaka — interroge
/// l'élément *avant* tout claim).
pub(crate) fn media_can_play_type(e: &Engine, arg: &str) -> &'static str {
    let mime = arg.trim().to_ascii_lowercase();
    let container = mime.split(';').next().unwrap_or("").trim();
    let known = matches!(
        container,
        "video/mp4"
            | "audio/mp4"
            | "video/webm"
            | "audio/webm"
            | "video/x-matroska"
            | "audio/mpeg"
            | "video/mp2t"
            | "audio/ogg"
            | "audio/wav"
            | "audio/x-wav"
    );
    if !known {
        return "";
    }
    match mime.split("codecs=").nth(1) {
        Some(cstr) => {
            let cstr = cstr.trim_matches(|c| c == '"' || c == '\'');
            let all = cstr
                .split(',')
                .filter_map(codec_prefix)
                .all(|p| e.mse.declared_codecs.contains(&p));
            if all {
                "probably"
            } else {
                ""
            }
        }
        None => {
            if e.mse.declared_codecs.is_empty() {
                ""
            } else {
                "maybe"
            }
        }
    }
}

// ------------------------------------------------------------------
// Méthodes MediaSource
// ------------------------------------------------------------------

/// `ms.addSourceBuffer(mime)` — buffer neuf, poussé dans
/// `sourceBuffers` (ordre conservé), `addsourcebuffer` tiré sur
/// la liste. Permis quel que soit `readyState` (déviation
/// documentée — voir en-tête de module).
fn ms_add_source_buffer(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Some(d) = ms_data(e, this) else {
        return Err(e.type_error("MediaSource.addSourceBuffer: Illegal invocation"));
    };
    let Some(&v) = args.first() else {
        return Err(e.type_error("MediaSource.addSourceBuffer: 1 argument required"));
    };
    let mime = e.to_js_string(v)?;
    if d.borrow().buffers.len() >= MS_BUFFER_CAP {
        return Err(mse_err(
            e,
            "QuotaExceededError",
            "MediaSource: source buffer quota",
        ));
    }
    let sp = sb_proto(e);
    let sb = Rc::new(RefCell::new(SbData {
        ms: this,
        mime,
        updating: false,
        est_ranges: Vec::new(),
        est_span: EST_SEGMENT_SPAN,
        last_append_t: None,
        timescale: None,
        mode: "segments".to_string(),
        timestamp_offset: 0.0,
        append_start: 0.0,
        append_end: f64::INFINITY,
        segments: std::collections::VecDeque::new(),
        bytes: 0,
        pulled: 0,
        seg_ranges: std::collections::VecDeque::new(),
        seg_boundary: std::collections::VecDeque::new(),
        served: std::collections::VecDeque::new(),
        served_bytes: 0,
        cursor: 0,
        pulled_saw_init: false,
    }));
    let v = e.new_host_object(Rc::new(SbHost { d: sb }));
    if let Value::Obj(id) = v {
        if let Some(o) = e.heap.obj_mut(id) {
            o.proto = Value::Obj(sp);
        }
        d.borrow_mut().buffers.push(v);
    }
    // `addsourcebuffer` sur la liste (spec) — la liste est
    // matérialisée pour porter l'événement même si le site ne
    // l'a jamais lue.
    if let Value::Obj(lid) = buffer_list(e, &d) {
        fire_mse(e, lid, "addsourcebuffer")?;
    }
    Ok(v)
}

/// `ms.removeSourceBuffer(sb)` — splice + `removesourcebuffer`.
fn ms_remove_source_buffer(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Some(d) = ms_data(e, this) else {
        return Err(e.type_error("MediaSource.removeSourceBuffer: Illegal invocation"));
    };
    let v = args.first().copied().unwrap_or(Value::Undefined);
    if sb_data(e, v).is_none() {
        return Err(e.type_error("MediaSource.removeSourceBuffer: not a SourceBuffer"));
    }
    let pos = {
        let mut md = d.borrow_mut();
        let pos = md.buffers.iter().position(|b| *b == v);
        if let Some(i) = pos {
            md.buffers.remove(i);
        }
        pos
    };
    if pos.is_none() {
        return Err(mse_err(
            e,
            "NotFoundError",
            "MediaSource.removeSourceBuffer: buffer not in sourceBuffers",
        ));
    }
    if let Value::Obj(lid) = buffer_list(e, &d) {
        fire_mse(e, lid, "removesourcebuffer")?;
    }
    Ok(Value::Undefined)
}

/// `ms.endOfStream(err?)` — `readyState` → `"ended"` +
/// `sourceended` **filé en tâche** (JS-C324 — spec : mise en
/// file, pas synchrone). Permis hors `"open"` (déviation
/// documentée).
fn ms_end_of_stream(e: &mut Engine, this: Value, _args: &[Value]) -> JsResult<Value> {
    let Some(d) = ms_data(e, this) else {
        return Err(e.type_error("MediaSource.endOfStream: Illegal invocation"));
    };
    d.borrow_mut().ready_state = "ended";
    let Value::Obj(mid) = this else {
        return Ok(Value::Undefined);
    };
    e.mse.pending_events.push_back((mid, "sourceended"));
    Ok(Value::Undefined)
}

/// `ms.setLiveSeekableRange(start, end)` — plage live retenue,
/// relue par `liveSeekableRange`.
fn ms_set_live_range(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Some(d) = ms_data(e, this) else {
        return Err(e.type_error("MediaSource.setLiveSeekableRange: Illegal invocation"));
    };
    let s = e.to_number(args.first().copied().unwrap_or(Value::Undefined))?;
    let t = e.to_number(args.get(1).copied().unwrap_or(Value::Undefined))?;
    if !(0.0..=t).contains(&s) {
        return Err(e.type_error("MediaSource.setLiveSeekableRange: bad range"));
    }
    d.borrow_mut().live_range = Some((s, t));
    Ok(Value::Undefined)
}

/// `ms.clearLiveSeekableRange()` — plage live vidée.
fn ms_clear_live_range(e: &mut Engine, this: Value, _args: &[Value]) -> JsResult<Value> {
    let Some(d) = ms_data(e, this) else {
        return Err(e.type_error("MediaSource.clearLiveSeekableRange: Illegal invocation"));
    };
    d.borrow_mut().live_range = None;
    Ok(Value::Undefined)
}

// ------------------------------------------------------------------
// Méthodes SourceBuffer
// ------------------------------------------------------------------

/// En-tête de box ISO-BMFF → `(taille_totale, fourcc,
/// taille_header)`. `size32==0` = jusqu'à la fin du chunk
/// (toléré) ; `1` = largesize 64 bits (bornée à `usize`).
fn box_header(b: &[u8]) -> Option<(usize, [u8; 4], usize)> {
    if b.len() < 8 {
        return None;
    }
    let sz32 = u32::from_be_bytes(b[..4].try_into().ok()?);
    let fourcc: [u8; 4] = b[4..8].try_into().ok()?;
    match sz32 {
        0 => Some((b.len(), fourcc, 8)),
        1 => {
            if b.len() < 16 {
                return None;
            }
            let sz = u64::from_be_bytes(b[8..16].try_into().ok()?);
            Some((usize::try_from(sz).ok()?, fourcc, 16))
        }
        _ => Some((sz32 as usize, fourcc, 8)),
    }
}

/// Itère les boxes enfants de `b` (région payload d'un
/// conteneur) en passant `(fourcc, payload)` à `f` — stop
/// net sur box tronqué (contenu hostile : borne, jamais panic).
/// Offset du premier `moov` **de niveau racine** de `b` —
/// position exacte de la frontière de section `X-Mse-Section`
/// (JS-C296/D : un append mixte `[moof][mdat][moov]` voit sa
/// section débuter au `moov`, pas au début du chunk).
fn moov_offset(b: &[u8]) -> Option<u64> {
    let mut pos = 0usize;
    while let Some((size, fourcc, _hdr)) = box_header(&b[pos..]) {
        let end = pos.checked_add(size).filter(|&e| e <= b.len())?;
        if &fourcc == b"moov" {
            return Some(pos as u64);
        }
        pos = end;
    }
    None
}

fn walk_boxes(b: &[u8], mut f: impl FnMut(&[u8; 4], &[u8])) {
    let mut pos = 0usize;
    while let Some((size, fourcc, hdr)) = box_header(&b[pos..]) {
        let Some(end) = pos
            .checked_add(size)
            .filter(|&e| e <= b.len() && e > pos + hdr)
        else {
            break;
        };
        f(&fourcc, &b[pos + hdr..end]);
        pos = end;
    }
}

/// `mdhd` → timescale média (`u32`) — v0 : champ à offset
/// 12 du payload, v1 : offset 20.
fn mdhd_timescale(b: &[u8]) -> Option<u32> {
    let off = match *b.first()? {
        0 => 12,
        1 => 20,
        _ => return None,
    };
    b.get(off..off + 4)
        .map(|s| u32::from_be_bytes(s.try_into().unwrap_or([0; 4])))
}

/// `tfdt` → `baseMediaDecodeTime` — v0 `u32`, v1 `u64`.
fn tfdt_base(b: &[u8]) -> Option<u64> {
    match *b.first()? {
        0 => Some(u64::from(u32::from_be_bytes(b.get(4..8)?.try_into().ok()?))),
        1 => Some(u64::from_be_bytes(b.get(4..12)?.try_into().ok()?)),
        _ => None,
    }
}

/// `tfhd` → `default_sample_duration` (flag 0x8), en sautant
/// les champs optionnels dans l'ordre spec.
fn tfhd_default_dur(b: &[u8]) -> Option<u32> {
    if b.len() < 8 {
        return None;
    }
    let flags = u32::from(b[1]) << 16 | u32::from(b[2]) << 8 | u32::from(b[3]);
    let mut p = 8usize; // version+flags + track_ID
    if flags & 0x1 != 0 {
        p += 8; // base_data_offset
    }
    if flags & 0x2 != 0 {
        p += 4; // sample_description_index
    }
    if flags & 0x8 != 0 {
        return b
            .get(p..p + 4)
            .map(|s| u32::from_be_bytes(s.try_into().unwrap_or([0; 4])));
    }
    None
}

/// Durée totale d'un `trun` en unités timescale : somme des
/// `sample_duration` par échantillon (flag 0x100, premier
/// champ du record) ; sinon `default_sample_duration` ×
/// `sample_count` quand connu.
fn trun_duration(b: &[u8], default_dur: Option<u32>) -> Option<u64> {
    if b.len() < 8 {
        return None;
    }
    let flags = u32::from(b[1]) << 16 | u32::from(b[2]) << 8 | u32::from(b[3]);
    let count = u64::from(u32::from_be_bytes(b.get(4..8)?.try_into().ok()?));
    let mut p = 8usize;
    if flags & 0x1 != 0 {
        p += 4; // data_offset
    }
    if flags & 0x4 != 0 {
        p += 4; // first_sample_flags
    }
    let rec = 4
        * (usize::from(flags & 0x100 != 0)
            + usize::from(flags & 0x200 != 0)
            + usize::from(flags & 0x400 != 0)
            + usize::from(flags & 0x800 != 0));
    if flags & 0x100 == 0 {
        return default_dur.map(|d| u64::from(d) * count);
    }
    let mut sum = 0u64;
    for _ in 0..count {
        let s = b.get(p..p + 4)?;
        sum = sum.saturating_add(u64::from(u32::from_be_bytes(
            s.try_into().unwrap_or([0; 4]),
        )));
        p += rec;
    }
    Some(sum)
}

/// Un `traf` → `(tfdt, durée_totale)` en unités timescale —
/// deux passes : `tfhd`/`tfdt` d'abord (ordre de box non
/// garanti), puis somme des `trun`.
fn traf_range(b: &[u8]) -> Option<(u64, u64)> {
    let mut tfdt = None;
    let mut ddef = None;
    walk_boxes(b, |fourcc, payload| match fourcc {
        b"tfdt" => tfdt = tfdt_base(payload),
        b"tfhd" => ddef = tfhd_default_dur(payload),
        _ => {}
    });
    let mut dur = 0u64;
    walk_boxes(b, |fourcc, payload| {
        if fourcc == b"trun" {
            dur = dur.saturating_add(trun_duration(payload, ddef).unwrap_or(0));
        }
    });
    tfdt.map(|t| (t, dur))
}

/// Scan fMP4 du chunk appendé (JS-C285) : `timescale` = le
/// `mdhd` du `moov` (init segment) ; `moofs` = les plages
/// `(tfdt, durée)` en unités timescale, une par `traf`.
/// Les timestamps **réels** priment l'estimateur : c'est ce
/// qu'un navigateur lit en démuxant — sans eux
/// `media.buffered` surestime chaque append et un player
/// (hls.js) saute des fragments entiers.
struct Fmp4Scan {
    timescale: Option<u32>,
    moofs: Vec<(u64, u64)>,
    /// Un box fMP4 top-niveau reconnu (`moov`/`moof`/`ftyp`/
    /// `styp`/`sidx`) — chunk bien formé : sans `moof` il ne
    /// porte pas de média (init) → **pas** de repli
    /// estimateur (un init ne couvre aucune plage).
    recognized: bool,
    /// `moov` présent — candidat init. Un init **re**-servi
    /// (switch ABR hls.js : nouvel init appendé sur le même
    /// `SourceBuffer`) marque une frontière de section pour
    /// le pont C (`X-Mse-Section` — l'hôte resynchronise
    /// le démuxeur pile à cet offset, `BoundaryGate`).
    has_moov: bool,
    /// Offset du premier `moov` racine dans le chunk
    /// (JS-C296 — frontière intra-chunk : un append mixte
    /// `[média][moov]` débute sa section au `moov`).
    moov_off: Option<u64>,
}

fn fmp4_scan(bytes: &[u8]) -> Fmp4Scan {
    let mut out = Fmp4Scan {
        timescale: None,
        moofs: Vec::new(),
        recognized: false,
        has_moov: false,
        moov_off: moov_offset(bytes),
    };
    walk_boxes(bytes, |fourcc, payload| match fourcc {
        b"moov" => {
            out.recognized = true;
            out.has_moov = true;
            walk_boxes(payload, |f2, p2| {
                if f2 == b"trak" {
                    walk_boxes(p2, |f3, p3| {
                        if f3 == b"mdia" {
                            walk_boxes(p3, |f4, p4| {
                                if f4 == b"mdhd" && out.timescale.is_none() {
                                    out.timescale = mdhd_timescale(p4);
                                }
                            });
                        }
                    });
                }
            });
        }
        b"moof" => {
            out.recognized = true;
            walk_boxes(payload, |f2, p2| {
                if f2 == b"traf" {
                    if let Some(r) = traf_range(p2) {
                        out.moofs.push(r);
                    }
                }
            });
        }
        b"ftyp" | b"styp" | b"sidx" | b"emsg" => out.recognized = true,
        _ => {}
    });
    out
}

/// Repli estimateur de plage `buffered` (JS-C278/b3 +
/// JS-C284) — utilisé quand le chunk ne porte pas de
/// timestamps exploitables (init seul, MPEG-TS, `WebM`…).
/// Retourne la plage que cet append revendique (JS-C287 —
/// attribution par chunk pour l'éviction `remove`), `None`
/// si rien n'est ajouté (append window saturée).
fn est_append_range(sd: &mut SbData) -> Option<(f64, f64)> {
    // JS-C278/b3 — `buffered` estimé : le segment couvre
    // `[t, t+est_span]` clippé à l'append window ; un
    // offset croissant affine le span du segment
    // précédent (delta réel rétroactif).
    let t = sd.timestamp_offset;
    if let Some(p) = sd.last_append_t {
        if t > p {
            sd.est_span = t - p;
            if let Some(last) = sd.est_ranges.last_mut() {
                if last.1 > p && last.1 < t {
                    last.1 = t;
                }
            }
        }
    }
    // JS-C284 — `timestampOffset` **inchangé** entre deux
    // appends : les timestamps vivent en bande (tfdt
    // fMP4) — les players réels (hls.js) enchaînent les
    // segments, la plage estimée doit donc **s'étendre**
    // d'un segment nominal par append, sinon
    // `media.buffered.end` plafonne et le loader d'un
    // player re-cible un fragment déjà livré (stall
    // observé à ~6 s). Un offset *changeant* repasse en
    // sémantique spec (plage pilotée par `t`).
    let sequential = sd.last_append_t == Some(t);
    sd.last_append_t = Some(t);
    let s = t.max(sd.append_start);
    let base = if sequential {
        sd.est_ranges.last().map_or(s, |&(_, e)| e)
    } else {
        s
    };
    let end = (base.max(s) + sd.est_span).min(sd.append_end);
    if end > s {
        match sd.est_ranges.last_mut() {
            Some(last) if s <= last.1 => last.1 = last.1.max(end),
            _ => sd.est_ranges.push((s, end)),
        }
        // Couverture nouvelle du chunk : `[base, end)` en
        // append séquentiel (le début `s` est déjà couvert
        // par le chunk précédent), `[s, end)` sinon.
        Some((if sequential { base.max(s) } else { s }, end))
    } else {
        None
    }
}

/// `sb.appendBuffer(buf)` — octets acceptés-**comptabilisés**
/// (jamais stockés : pas de transport en C173). Séquence
/// synchrone `updatestart` → `update` → `updateend` ; le cap
/// d'octets jette `QuotaExceededError` (budget hostile).
fn sb_append_buffer(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Some(d) = sb_data(e, this) else {
        return Err(e.type_error("SourceBuffer.appendBuffer: Illegal invocation"));
    };
    let v = args.first().copied().unwrap_or(Value::Undefined);
    let Some(bytes) = crate::typed_array::bytes_of_value(e, v) else {
        return Err(
            e.type_error("SourceBuffer.appendBuffer: argument is not an ArrayBuffer or view")
        );
    };
    {
        let mut sd = d.borrow_mut();
        let next = sd.bytes.saturating_add(bytes.len());
        if next > SB_BYTE_CAP {
            return Err(mse_err(
                e,
                "QuotaExceededError",
                "SourceBuffer: byte quota exceeded",
            ));
        }
        // JS-C285 — timestamps en bande : scan fMP4 AVANT
        // l'entrée en file (la donnée réelle — `mdhd` de
        // l'init, `tfdt`+durées `trun` des `moof` — est ce
        // qu'un navigateur obtient en démuxant).
        let scan = fmp4_scan(&bytes);
        // MSE-3 transport : le segment est stocké en file
        // FIFO (ordre d'append — le flux `ByteSource` exact
        // qu'OnionCodec consomme côté hôte, ADR-0029).
        sd.segments.push_back(bytes);
        sd.bytes = next;
        sd.updating = true;
        if let Some(ts) = scan.timescale {
            sd.timescale = Some(ts);
        }
        // JS-C287 — plage revendiquée par ce chunk (éviction
        // `remove` par plage) : union des plages fMP4,
        // plage de l'estimateur, `None` pour les chunks sans
        // média (init…).
        let span = match (sd.timescale, scan.moofs.is_empty()) {
            // Media segment fMP4 : les plages réelles
            // `tfdt`→`tfdt+dur` (+`timestampOffset`, fenêtre
            // clippée) — pas d'estimation.
            (Some(ts), false) => {
                sd.last_append_t = Some(sd.timestamp_offset);
                let ts = f64::from(ts);
                let mut span: Option<(f64, f64)> = None;
                for (start, dur) in scan.moofs {
                    let s = (sd.timestamp_offset + start as f64 / ts).max(sd.append_start);
                    let en = (sd.timestamp_offset + (start + dur) as f64 / ts).min(sd.append_end);
                    if en > s {
                        match sd.est_ranges.last_mut() {
                            Some(last) if s <= last.1 => last.1 = last.1.max(en),
                            _ => sd.est_ranges.push((s, en)),
                        }
                        span = Some(match span {
                            Some((a, b)) => (a.min(s), b.max(en)),
                            None => (s, en),
                        });
                    }
                }
                span
            }
            // fMP4 reconnu sans moof (init `moov`+`mdhd`,
            // `ftyp`/`sidx` seuls…) : pas de média → aucune
            // plage — l'init n'est **pas** 6 s de contenu.
            _ if scan.recognized => None,
            _ => est_append_range(&mut sd),
        };
        sd.seg_ranges.push_back(span);
        // JS-C295 — pont C : un `moov` re-servi plus tard au
        // pull = frontière `X-Mse-Section` (switch ABR) ;
        // JS-C296 — offset intra-chunk exact (`moov` peut
        // suivre du média dans le même append).
        sd.seg_boundary.push_back(scan.moov_off);
    }
    let Value::Obj(sid) = this else {
        return Ok(Value::Undefined);
    };
    // Approximation documentée : les trois événements partent en
    // séquence synchrone (spec : « queue a task ») — l'ordre vu
    // par les listeners est correct. JS-C283 : un listener qui
    // jette n'interrompt plus la séquence — `updating` retombe
    // et `updateend` part quand même, sinon `blockBuffers` d'un
    // player réel gèle (hls.js → `endOfStream` jamais appelé).
    let mut first_err = None;
    fire_mse_guarded(e, sid, "updatestart", &mut first_err);
    fire_mse_guarded(e, sid, "update", &mut first_err);
    d.borrow_mut().updating = false;
    fire_mse_guarded(e, sid, "updateend", &mut first_err);
    match first_err {
        Some(err) => Err(err),
        None => Ok(Value::Undefined),
    }
}

/// `sb.abort()` — `updating` retombé + `abort`/`updateend`.
/// Spec : abort vide le buffer — les octets en file non
/// consommés sont abandonnés (MSE-3).
fn sb_abort(e: &mut Engine, this: Value, _args: &[Value]) -> JsResult<Value> {
    let Some(d) = sb_data(e, this) else {
        return Err(e.type_error("SourceBuffer.abort: Illegal invocation"));
    };
    {
        let mut sd = d.borrow_mut();
        sd.updating = false;
        sd.segments.clear();
        sd.seg_ranges.clear();
        sd.seg_boundary.clear();
        sd.bytes = 0;
        // JS-C278 — abort vide le buffer : l'estimation
        // repart à zéro comme la file d'octets.
        sd.est_ranges.clear();
        sd.last_append_t = None;
    }
    let Value::Obj(sid) = this else {
        return Ok(Value::Undefined);
    };
    let mut first_err = None;
    fire_mse_guarded(e, sid, "abort", &mut first_err);
    fire_mse_guarded(e, sid, "updateend", &mut first_err);
    match first_err {
        Some(err) => Err(err),
        None => Ok(Value::Undefined),
    }
}

/// `sb.remove(start, end)` — accepté : `buffered` est vide, il
/// n'y a rien à évincer ; la séquence d'événements est tirée.
fn sb_remove(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Some(d) = sb_data(e, this) else {
        return Err(e.type_error("SourceBuffer.remove: Illegal invocation"));
    };
    let s = e.to_number(args.first().copied().unwrap_or(Value::Undefined))?;
    let t = e.to_number(args.get(1).copied().unwrap_or(Value::Undefined))?;
    if !(s.is_finite() && s >= 0.0) || t <= s {
        return Err(e.type_error("SourceBuffer.remove: bad range"));
    }
    {
        let mut sd = d.borrow_mut();
        sd.updating = true;
        // JS-C287 — éviction par plage : chaque chunk file
        // porte la plage qu'il a contribuée ; seuls les
        // chunks **entièrement** couverts par `[s, t)` sont
        // jetés (epsilon 1 ms contre les bords flottants
        // `tfdt`/timescale). Un chunk partiellement couvert
        // ou sans plage attribuée (init) est conservé —
        // découper l'intérieur d'un fragment exige un
        // démuxage ; approximation conservatrice documentée.
        let segs = std::mem::take(&mut sd.segments);
        let spans = std::mem::take(&mut sd.seg_ranges);
        let bounds = std::mem::take(&mut sd.seg_boundary);
        let mut bytes = 0usize;
        for ((chunk, span), bound) in segs.into_iter().zip(spans).zip(bounds) {
            if span.is_some_and(|(a, b)| a >= s - REMOVE_EPS && b <= t + REMOVE_EPS) {
                continue;
            }
            bytes += chunk.len();
            sd.segments.push_back(chunk);
            sd.seg_ranges.push_back(span);
            sd.seg_boundary.push_back(bound);
        }
        sd.bytes = bytes;
        // JS-C296 — même règle sur l'historique servi : un
        // chunk entièrement couvert laisse un trou adressable
        // (`?from=` dedans → 416) ; le média retiré ne doit
        // plus être re-servissable (mémoire + fidélité).
        let served = std::mem::take(&mut sd.served);
        for c in served {
            if c.range
                .is_some_and(|(a, b)| a >= s - REMOVE_EPS && b <= t + REMOVE_EPS)
            {
                sd.served_bytes = sd.served_bytes.saturating_sub(c.bytes.len());
                continue;
            }
            sd.served.push_back(c);
        }
        // JS-C278 — l'estimation `buffered`, elle, connaît
        // des plages : soustraction honnête de `[s, t)`.
        sd.est_ranges = sd
            .est_ranges
            .iter()
            .flat_map(|&(a, b)| {
                if b <= s || a >= t {
                    vec![(a, b)]
                } else {
                    let mut v = Vec::new();
                    if a < s {
                        v.push((a, s));
                    }
                    if b > t {
                        v.push((t, b));
                    }
                    v
                }
            })
            .collect();
    }
    let Value::Obj(sid) = this else {
        return Ok(Value::Undefined);
    };
    let mut first_err = None;
    fire_mse_guarded(e, sid, "updatestart", &mut first_err);
    fire_mse_guarded(e, sid, "update", &mut first_err);
    d.borrow_mut().updating = false;
    fire_mse_guarded(e, sid, "updateend", &mut first_err);
    match first_err {
        Some(err) => Err(err),
        None => Ok(Value::Undefined),
    }
}

/// `sb.changeType(mime)` — mime retenu (spec : re-parse du
/// type supporté — le support reste `""` en C173, on accepte
/// et on retient pour le futur transport).
fn sb_change_type(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Some(d) = sb_data(e, this) else {
        return Err(e.type_error("SourceBuffer.changeType: Illegal invocation"));
    };
    let mime = e.to_js_string(args.first().copied().unwrap_or(Value::Undefined))?;
    d.borrow_mut().mime = mime;
    Ok(Value::Undefined)
}

// ------------------------------------------------------------------
// Méthodes TimeRanges
// ------------------------------------------------------------------

/// `tr.start(i)` / `tr.end(i)` — borne de la i-ème plage, erreur
/// hors rang.
fn tr_bound(e: &mut Engine, this: Value, args: &[Value], start: bool) -> JsResult<Value> {
    let Some(ranges) = tr_ranges(e, this) else {
        return Err(e.type_error("TimeRanges: Illegal invocation"));
    };
    let i = e.to_number(args.first().copied().unwrap_or(Value::Undefined))?;
    if !(i >= 0.0 && i.fract() == 0.0) {
        return Err(e.type_error("TimeRanges: index out of range"));
    }
    let Some(&(s, t)) = ranges.get(i as usize) else {
        return Err(e.type_error("TimeRanges: index out of range"));
    };
    Ok(Value::Num(if start { s } else { t }))
}

fn tr_start(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    tr_bound(e, this, args, true)
}

fn tr_end(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    tr_bound(e, this, args, false)
}

// ------------------------------------------------------------------
// `URL.createObjectURL(mediaSource)`
// ------------------------------------------------------------------

/// `createObjectURL` sur un `MediaSource` — enregistre
/// `blob:<origin>/<n>` → wrapper dans `object_urls` et rend
/// l'URL. `None` si `v` n'est pas un `MediaSource` (le chemin
/// `Blob` prend le relais dans `files.rs`).
pub(crate) fn object_url_for(e: &mut Engine, v: Value) -> Option<String> {
    let Value::Obj(mid) = v else { return None };
    if !is_media_source(e, v) {
        return None;
    }
    let url = crate::files::alloc_object_url(e);
    e.mse.object_urls.insert(url.clone(), mid);
    Some(url)
}

/// `revokeObjectURL` — purge aussi le registre MSE (appelé
/// depuis `files.rs` ; retourne `true` si une entrée partait).
pub(crate) fn revoke_object_url(e: &mut Engine, url: &str) -> bool {
    e.mse.object_urls.remove(url).is_some()
}

/// `blob:` → wrapper `MediaSource` enregistré — point d'entrée
/// de la future couture d'offre `mse` côté `explorer-core`
/// `url` → `ObjId` `MediaSource` enregistré (`createObjectURL`).
/// Consulté par le scan média core (MSE-2) et par
/// l'attache `src = blob:`.
pub(crate) fn object_url_media(e: &Engine, url: &str) -> Option<ObjId> {
    e.mse.object_urls.get(url).copied()
}

/// Attache tentative (MSE-2, ADR-0029) — appelée quand un
/// `<video>`/`<audio>` reçoit `src = blob:…` (prop directe ou
/// `setAttribute`) : `url` enregistrée `kind=mse` →
/// `readyState` `"closed"` → `"open"` synchrone (approximation
/// tolérante — un polleur voit l'état tôt) + `sourceopen` **filé
/// en tâche** (JS-C324 — spec : le tir est une tâche de l'algo
/// d'attachement, jamais synchrone ; les players enregistrent
/// leur listener *après* le `src=`). Idempotent : une source
/// déjà `open`/`ended` n'est pas re-tirée.
pub(crate) fn maybe_attach(e: &mut Engine, url: &str) {
    let Some(mid) = object_url_media(e, url) else {
        return;
    };
    let Some(d) = ms_data(e, Value::Obj(mid)) else {
        return;
    };
    if d.borrow().ready_state != "closed" {
        return;
    }
    d.borrow_mut().ready_state = "open";
    e.mse.pending_events.push_back((mid, "sourceopen"));
}

/// Drain des événements `MediaSource` filés (JS-C324) — frontière
/// de job, même couture que `deliver_mutations`/`deliver_ws`
/// dans `run_microtasks`. La file est vidée entièrement : un
/// listener qui jette ne doit pas perdre les événements suivants
/// (même règle que `fire_mse_guarded`) — la première erreur est
/// propagée après vidange. `true` = au moins un tir tenté.
pub(crate) fn deliver_mse_events(e: &mut Engine) -> JsResult<bool> {
    let mut fired = false;
    let mut first: Option<Thrown> = None;
    while let Some((target, ty)) = e.mse.pending_events.pop_front() {
        fired = true;
        if let Err(err) = fire_mse(e, target, ty) {
            if first.is_none() {
                first = Some(err);
            }
        }
    }
    match first {
        Some(t) => Err(t),
        None => Ok(fired),
    }
}

impl Engine {
    /// `true` si `url` est un object URL `MediaSource`
    /// enregistré — port `JsEngine` consulté par le scan
    /// média core pour marquer l'offre `mse` (ADR-0029).
    #[must_use]
    pub fn is_media_source_url(&self, url: &str) -> bool {
        self.mse.object_urls.contains_key(url)
    }

    /// Pull de segments du `SourceBuffer` `buffer` (index
    /// `addSourceBuffer`) de la `MediaSource` enregistrée
    /// sous `url` — transport MSE-3 (ADR-0029) : le
    /// `MseSource` `ByteSource` hôte draine la file FIFO par
    /// `max` octets en vrac. Retour `(octets, curseur_suivant,
    /// eos)` : le curseur avance du nombre d'octets rendus ;
    /// `eos` vrai quand `readyState == "ended"` **et** la
    /// file est vide (la lecture bloque autrement — le
    /// site peut encore appender).
    /// `None` : `url` inconnue ou index de buffer hors rang.
    /// `sections` = offsets **dans le corps** où commence
    /// une nouvelle section (JS-C295, pont C — `X-Mse-Section`
    /// du contrat filaire : un `moov` re-servi = l'hôte
    /// resynchronise le démuxeur à cet offset exact).
    /// `from` (JS-C296, chantier D — `MseSource::seek`) :
    /// repositionne le curseur de service à une position
    /// absolue déjà vue. **Arrière** : re-sert l'historique
    /// conservé (`served`) ; **avant** : bascule les chunks
    /// file→served jusqu'à couvrir la cible (les octets
    /// sautés restent re-servissables, un seek ne retire
    /// pas de média). `Err` = cible hors fenêtre
    /// adressable (trou `remove`/éviction cap/jamais servi)
    /// → `416` filaire.
    pub fn mse_pull_segments(
        &mut self,
        url: &str,
        buffer: usize,
        max: usize,
        from: Option<u64>,
    ) -> Option<MsePullOut> {
        let mid = self.mse.object_urls.get(url).copied()?;
        let ms = ms_data(self, Value::Obj(mid))?;
        let sbv = ms.borrow().buffers.get(buffer).copied()?;
        let sd = sb_data(self, sbv)?;
        let mut sd = sd.borrow_mut();
        if let Some(f) = from {
            while sd.pulled < f {
                // Avance : les chunks file basculent servis —
                // les positions restent re-servissables.
                if !serve_next_chunk(&mut sd) {
                    break;
                }
            }
            if (f < sd.pulled && !served_covers(&sd.served, f)) || f > sd.pulled {
                // Trou (remove/éviction) ou jamais servi.
                return Some(Err(()));
            }
            sd.cursor = f;
        }
        let mut out = Vec::new();
        let mut sections = Vec::new();
        let body_start = sd.cursor;
        while out.len() < max {
            if sd.cursor < sd.pulled {
                // Replay : copie depuis le chunk d'historique
                // contenant le curseur ; un trou (média
                // `remove`d) termine le corps ici.
                let Some(pos) = sd.served.iter().position(|c| {
                    c.start <= sd.cursor && sd.cursor < c.start + c.bytes.len() as u64
                }) else {
                    break;
                };
                let c = &sd.served[pos];
                let off = (sd.cursor - c.start) as usize;
                let take = (c.bytes.len() - off).min(max - out.len());
                if let Some(boff) = c.boundary {
                    let abs = c.start + boff;
                    if abs >= sd.cursor && abs < sd.cursor + take as u64 {
                        sections.push(abs - body_start);
                    }
                }
                out.extend_from_slice(&c.bytes[off..off + take]);
                sd.cursor += take as u64;
                continue;
            }
            if !serve_next_chunk(&mut sd) {
                break;
            }
        }
        let cursor = sd.cursor;
        let ended = sd.cursor == sd.pulled && sd.segments.is_empty();
        drop(sd);
        let eos = ended && ms.borrow().ready_state == "ended";
        Some(Ok((out, cursor, eos, sections)))
    }

    /// MIME des `SourceBuffer` de la `MediaSource` sous `url`
    /// (index = ordre `addSourceBuffer`) — l'offre `mse` les
    /// publie pour que l'hôte compose ses `MseSource` (P-53
    /// `open_mse_multi` : un descripteur par buffer — MSE-3
    /// transport déjà indexé `?buffer=N`). `[]` hors registre.
    pub fn mse_source_buffers(&mut self, url: &str) -> Vec<String> {
        let Some(mid) = self.mse.object_urls.get(url).copied() else {
            return Vec::new();
        };
        let Some(ms) = ms_data(self, Value::Obj(mid)) else {
            return Vec::new();
        };
        let buffers = ms.borrow().buffers.clone();
        buffers
            .into_iter()
            .filter_map(|v| sb_data(self, v))
            .map(|d| d.borrow().mime.clone())
            .collect()
    }

    /// `timestampOffset` des `SourceBuffer` sous `url` — même
    /// ordre que [`Self::mse_source_buffers`] (`?buffer=N`).
    /// `0.0` par défaut (spec) ; l'offre `mseOffsets` alimente
    /// `MseDesc::pts_offset_secs` côté `open_mse_multi`.
    pub fn mse_buffer_offsets(&mut self, url: &str) -> Vec<f64> {
        let Some(mid) = self.mse.object_urls.get(url).copied() else {
            return Vec::new();
        };
        let Some(ms) = ms_data(self, Value::Obj(mid)) else {
            return Vec::new();
        };
        let buffers = ms.borrow().buffers.clone();
        buffers
            .into_iter()
            .filter_map(|v| sb_data(self, v))
            .map(|d| d.borrow().timestamp_offset)
            .collect()
    }

    /// Union des plages `buffered` estimées de tous les
    /// `SourceBuffer` de la `MediaSource` sous `url` (JS-C284)
    /// — spec : `HTMLMediaElement.buffered` = union des
    /// buffers **actifs** attachés. Sans ça `media.buffered`
    /// reste vide sous MSE et `bufferInfo.end` d'un player
    /// (hls.js) gèle à 0 → re-ciblage d'un fragment déjà
    /// livré, chargement dead-locked. `None` si `url` n'est
    /// pas un object-URL MSE.
    pub(crate) fn mse_buffered_union(&mut self, url: &str) -> Option<Vec<(f64, f64)>> {
        let mid = self.mse.object_urls.get(url).copied()?;
        let ms = ms_data(self, Value::Obj(mid))?;
        let buffers = ms.borrow().buffers.clone();
        let mut ranges: Vec<(f64, f64)> = Vec::new();
        for v in buffers {
            if let Some(d) = sb_data(self, v) {
                ranges.extend_from_slice(&d.borrow().est_ranges);
            }
        }
        ranges.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        let mut out: Vec<(f64, f64)> = Vec::new();
        for (s, t) in ranges {
            match out.last_mut() {
                Some(last) if s <= last.1 => last.1 = last.1.max(t),
                _ => out.push((s, t)),
            }
        }
        Some(out)
    }

    /// `MediaSource.duration` sous l'object-URL `url`
    /// (JS-C288 — `video.duration` reflète la durée
    /// `MediaSource` fixée par le site, spec « media element
    /// attachement »). `None` hors registre ou durée jamais
    /// fixée (`NaN`).
    pub(crate) fn mse_source_duration(&self, url: &str) -> Option<f64> {
        let mid = self.mse.object_urls.get(url).copied()?;
        let ms = ms_data(self, Value::Obj(mid))?;
        let d = ms.borrow().duration;
        d.is_finite().then_some(d)
    }

    /// `seekable` sous MSE (JS-C288) : `liveSeekableRange`
    /// quand le site l'a fixée, sinon l'union `buffered`
    /// (VOD — les plages seekables = les plages présentes).
    /// `None` hors registre MSE.
    pub(crate) fn mse_seekable(&mut self, url: &str) -> Option<Vec<(f64, f64)>> {
        let mid = self.mse.object_urls.get(url).copied()?;
        let ms = ms_data(self, Value::Obj(mid))?;
        if let Some(r) = ms.borrow().live_range {
            return Some(vec![r]);
        }
        drop(ms);
        self.mse_buffered_union(url)
    }
}

// ------------------------------------------------------------------
// impl Host
// ------------------------------------------------------------------

impl Host for MsHost {
    fn get(&self, e: &mut Engine, _t: Value, k: &str) -> JsResult<Option<Value>> {
        Ok(match k {
            "readyState" => Some(e.str_val(self.d.borrow().ready_state)),
            "sourceBuffers" | "activeSourceBuffers" => {
                // `buffer_list` emprunte le même `RefCell` —
                // relâcher le borrow avant d'y entrer.
                let dd = self.d.clone();
                Some(buffer_list(e, &dd))
            }
            "duration" => Some(Value::Num(self.d.borrow().duration)),
            "liveSeekableRange" => {
                let ranges = self
                    .d
                    .borrow()
                    .live_range
                    .map_or_else(Vec::new, |r| vec![r]);
                Some(time_ranges(e, ranges))
            }
            _ => None,
        })
    }

    fn set(&self, e: &mut Engine, _t: Value, k: &str, v: Value) -> JsResult<bool> {
        if k == "duration" {
            let n = e.to_number(v)?;
            // Spec : `NaN` ou négatif → `TypeError`.
            if n.is_nan() || n < 0.0 {
                return Err(e.type_error("MediaSource.duration: bad value"));
            }
            self.d.borrow_mut().duration = n;
            return Ok(true);
        }
        Ok(false)
    }

    fn trace(&self, out: &mut Vec<ObjId>) {
        let d = self.d.borrow();
        for b in &d.buffers {
            crate::heap::push_value(out, *b);
        }
        if let Some(l) = d.list {
            crate::heap::push_value(out, l);
        }
    }

    fn class_name(&self) -> &'static str {
        "MediaSource"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl Host for SbHost {
    fn get(&self, e: &mut Engine, _t: Value, k: &str) -> JsResult<Option<Value>> {
        Ok(match k {
            "updating" => Some(Value::Bool(self.d.borrow().updating)),
            "mode" => Some(e.str_val(self.d.borrow().mode.clone())),
            "timestampOffset" => Some(Value::Num(self.d.borrow().timestamp_offset)),
            "appendWindowStart" => Some(Value::Num(self.d.borrow().append_start)),
            "appendWindowEnd" => Some(Value::Num(self.d.borrow().append_end)),
            "buffered" => {
                // JS-C278 — estimé (ADR-0029), pas vide : les
                // players bouclent sur `buffered.end()`.
                let ranges = self.d.borrow().est_ranges.clone();
                Some(time_ranges(e, ranges))
            }
            "audioTracks" | "videoTracks" | "textTracks" => Some(e.new_array(Vec::new())),
            _ => None,
        })
    }

    fn set(&self, e: &mut Engine, _t: Value, k: &str, v: Value) -> JsResult<bool> {
        match k {
            "mode" => {
                let m = e.to_js_string(v)?;
                if m != "segments" && m != "sequence" {
                    return Err(e.type_error("SourceBuffer.mode: bad value"));
                }
                self.d.borrow_mut().mode = m;
                Ok(true)
            }
            "timestampOffset" => {
                let n = e.to_number(v)?;
                if !n.is_finite() {
                    return Err(e.type_error("SourceBuffer.timestampOffset: bad value"));
                }
                self.d.borrow_mut().timestamp_offset = n;
                Ok(true)
            }
            "appendWindowStart" | "appendWindowEnd" => {
                let n = e.to_number(v)?;
                if n.is_nan() {
                    return Err(e.type_error("SourceBuffer.appendWindow: bad value"));
                }
                let mut d = self.d.borrow_mut();
                if k == "appendWindowStart" {
                    if n < 0.0 {
                        return Err(e.type_error("SourceBuffer.appendWindowStart: bad value"));
                    }
                    d.append_start = n;
                } else {
                    d.append_end = n;
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn trace(&self, out: &mut Vec<ObjId>) {
        let d = self.d.borrow();
        crate::heap::push_value(out, d.ms);
    }

    fn class_name(&self) -> &'static str {
        "SourceBuffer"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl Host for SbListHost {
    fn get(&self, _e: &mut Engine, _t: Value, k: &str) -> JsResult<Option<Value>> {
        let d = self.d.borrow();
        if k == "length" {
            return Ok(Some(Value::Num(d.buffers.len() as f64)));
        }
        if let Ok(i) = k.parse::<usize>() {
            return Ok(d.buffers.get(i).copied());
        }
        Ok(None)
    }

    fn trace(&self, out: &mut Vec<ObjId>) {
        let d = self.d.borrow();
        for b in &d.buffers {
            crate::heap::push_value(out, *b);
        }
    }

    fn class_name(&self) -> &'static str {
        "SourceBufferList"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl Host for TrHost {
    fn get(&self, _e: &mut Engine, _t: Value, k: &str) -> JsResult<Option<Value>> {
        if k == "length" {
            return Ok(Some(Value::Num(self.ranges.len() as f64)));
        }
        Ok(None)
    }

    fn class_name(&self) -> &'static str {
        "TimeRanges"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

// ------------------------------------------------------------------
// Installation
// ------------------------------------------------------------------

/// Installe `MediaSource`/`SourceBuffer`/`SourceBufferList`/
/// `TimeRanges` sur le global — idempotent (appelé par
/// `install_dom`, donc par contexte iframe aussi).
pub(crate) fn install_mse(e: &mut Engine) {
    let g = e.global;
    if e.get_prop(g, "MediaSource").is_some() {
        return;
    }
    // `MediaSource` — vrai constructeur.
    let mp = ms_proto(e);
    let fv = e.native_fn("MediaSource", ms_ctor);
    if let Value::Obj(fid) = fv {
        e.set_own(fid, "prototype", Value::Obj(mp), false);
        if let Some(o) = e.heap.obj_mut(mp) {
            o.set_own("constructor", fv, false);
        }
        e.add_method(fid, "isTypeSupported", ms_is_type_supported);
    }
    e.set_own(g, "MediaSource", fv, false);
    // Interfaces non constructibles — ctor `Illegal constructor`
    // + vrai prototype (`instanceof`, feature-detect proto).
    for (name, ctor, proto) in [
        (
            "SourceBuffer",
            sb_illegal_ctor as crate::heap::NativeFn,
            sb_proto(e),
        ),
        ("SourceBufferList", sbl_illegal_ctor, sbl_proto(e)),
        ("TimeRanges", tr_illegal_ctor, tr_proto(e)),
    ] {
        let fv = e.native_fn(name, ctor);
        if let Value::Obj(fid) = fv {
            e.set_own(fid, "prototype", Value::Obj(proto), false);
        }
        e.set_own(g, name, fv, false);
    }
}
