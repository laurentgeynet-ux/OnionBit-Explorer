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

use crate::context::CtxId;
use crate::dom::{MediaReport, MediaState, NodeId};
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

/// Fenêtre `?peek=` (vol 627 — diagnostic/consommateur
/// externe) : sert `[f, f+max)` **en lecture seule** —
/// `cursor`/`pulled`/`served` intacts, aucun chunk
/// file→served : un peek ne perturbe jamais le FIFO du
/// consommateur réel. Les chunks en file ont des positions
/// virtuelles contigües à partir de `pulled` (ordre d'append).
/// Sections `X-Mse-Section` émises comme au pull normal.
/// `Err` = cible hors fenêtre adressable (trou `served`
/// `remove`/éviction, ou au-delà du total appendé) — même
/// contrat `416` que `?from=`. Corps vide possible : `f`
/// au bout exact du flux.
fn peek_window(sd: &SbData, f: u64, max: usize) -> Result<(Vec<u8>, u64, Vec<u64>), ()> {
    let total_end = sd.pulled + sd.segments.iter().map(|s| s.len() as u64).sum::<u64>();
    if f > total_end {
        return Err(());
    }
    let mut out = Vec::new();
    let mut sections = Vec::new();
    let mut pos = f;
    for c in &sd.served {
        let end = c.start + c.bytes.len() as u64;
        if pos >= end {
            continue;
        }
        if pos < c.start {
            return Err(()); // trou `remove`/éviction
        }
        let off = (pos - c.start) as usize;
        let take = (c.bytes.len() - off).min(max - out.len());
        if let Some(abs) = c.boundary.map(|b| c.start + b) {
            if abs >= pos && abs < pos + take as u64 {
                sections.push(abs - f);
            }
        }
        out.extend_from_slice(&c.bytes[off..off + take]);
        pos += take as u64;
        if out.len() >= max {
            return Ok((out, pos, sections));
        }
    }
    let mut vpos = sd.pulled;
    for (i, seg) in sd.segments.iter().enumerate() {
        let end = vpos + seg.len() as u64;
        if pos >= end {
            vpos = end;
            continue;
        }
        if pos < vpos {
            return Err(()); // trou entre `served` et la file
        }
        let off = (pos - vpos) as usize;
        let take = (seg.len() - off).min(max - out.len());
        if let Some(abs) = sd.seg_boundary.get(i).copied().flatten().map(|b| vpos + b) {
            if abs >= pos && abs < pos + take as u64 {
                sections.push(abs - f);
            }
        }
        out.extend_from_slice(&seg[off..off + take]);
        pos += take as u64;
        if out.len() >= max {
            return Ok((out, pos, sections));
        }
        vpos = end;
    }
    Ok((out, pos, sections))
}

/// Capacité d'un ring PCM (Étape 204b) — ~500 ms à la spec
/// fixe dépôt (48 kHz stéréo f32 = 384 Ko/s).
const PCM_RING_CAP: usize = 192 * 1024;

/// Ring PCM borné par surface média (Étape 204b — tee
/// `AUDIO_MODE_TAP`, ADR-0032) : octets interleaved, **drop-
/// oldest** quand plein (l'analyse voit toujours la fenêtre
/// la plus récente — jamais de file infinie), compteur
/// `dropped` pour diagnostic. Spec portée par le dernier
/// chunk reçu — un changement de spec **purge** le ring
/// (octets d'ancien format = données fausses).
#[derive(Debug)]
pub(crate) struct PcmRing {
    buf: std::collections::VecDeque<u8>,
    /// Octets écartés faute de place (diagnostic §B).
    pub dropped: u64,
    /// Spec du dernier chunk (Hz / canaux / format dépôt).
    pub spec: (u32, u16, i32),
}

impl Default for PcmRing {
    fn default() -> Self {
        Self {
            buf: std::collections::VecDeque::with_capacity(PCM_RING_CAP),
            dropped: 0,
            spec: (0, 0, -1),
        }
    }
}

impl PcmRing {
    /// Pousse un chunk (drop-oldest borné à `PCM_RING_CAP`) ;
    /// spec différente du dernier chunk = resync → purge.
    pub fn push(&mut self, rate: u32, channels: u16, format: i32, chunk: &[u8]) {
        if self.spec != (rate, channels, format) {
            self.dropped += self.buf.len() as u64;
            self.buf.clear();
            self.spec = (rate, channels, format);
        }
        if chunk.len() >= PCM_RING_CAP {
            self.dropped += (chunk.len() - PCM_RING_CAP) as u64;
            self.buf.clear();
            self.buf
                .extend(chunk[chunk.len() - PCM_RING_CAP..].iter().copied());
            return;
        }
        let over = (self.buf.len() + chunk.len()).saturating_sub(PCM_RING_CAP);
        if over > 0 {
            let over = over.min(self.buf.len());
            self.buf.drain(..over);
            self.dropped += over as u64;
        }
        self.buf.extend(chunk.iter().copied());
    }

    /// Copie les `n` derniers octets du ring (fenêtre la plus
    /// récente pour `AnalyserNode`, 204c) — vide si insuffisant.
    #[allow(dead_code)] // consommé par `MediaElementAudioSourceNode` (204c).
    pub fn read_latest(&self, n: usize) -> Vec<u8> {
        let take = n.min(self.buf.len());
        let start = self.buf.len() - take;
        self.buf.iter().skip(start).copied().collect()
    }
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
    media_error_proto: Option<ObjId>,
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
    /// frontière de job par [`deliver_mse_events`]. Les types
    /// ciblant l'élément attaché (`progress`, étape 182) y
    /// transitent aussi — cible résolue via `attached_to`.
    pending_events: std::collections::VecDeque<(ObjId, &'static str)>,
    /// Rings PCM par surface claimée (Étape 204b — tee
    /// `AUDIO_MODE_TAP`, ADR-0032) : alimentés par
    /// `Engine::deliver_media_audio`, lus par le graphe Web
    /// Audio (`MediaElementAudioSourceNode`, 204c). Clé = id
    /// de surface média session.
    pub(crate) pcm_rings: HashMap<u64, PcmRing>,
    /// Élément `<video>`/`<audio>` attaché à chaque
    /// `MediaSource` (étape 182) — posé au `src = blob:`/
    /// `setAttribute("src", …)`, dernier posé gagne (une
    /// object-URL n'a qu'un attaché effectif, spec). Sert à
    /// router les events **élément** (`progress` tiré à
    /// l'`updateend` d'append) vers le nœud miroir. Entrées
    /// mortes (nœud détaché) : le dispatch est un no-op sûr
    /// (`node_valid` §B-37) — pas de purge nécessaire, la
    /// table meurt avec le document.
    attached_to: HashMap<ObjId, NodeId>,
    /// Échelle `readyState` montée **sur l'élément** attaché
    /// (§B-88 — JS-C332) : les events de cycle de vie
    /// (`loadstart` → `loadedmetadata` → `canplay`…) vivent
    /// sur `HTMLMediaElement`, pas sur `MediaSource` — un
    /// élément MSE non encore claimé par l'hôte devait voir
    /// la rampe montée par les `appendBuffer` (bitmovin arme
    /// des listeners exhaustifs puis charge : zéro event =
    /// machine armée à jamais, `mseBuffers` vide). `NodeId →
    /// niveau atteint` — monotone tant que l'élément n'est
    /// pas claimé (l'underrun `waiting`/`stalled` reste
    /// l'affaire de `deliver_media_state` post-claim ; au
    /// claim le niveau est **seedé** dans `MediaElState` —
    /// pas de double tir). L'entrée meurt avec le document.
    el_ladder: HashMap<NodeId, u8>,
    /// Intention `play()` enregistrée pré-claim (§B-88) :
    /// `paused` reflète le flag interne spec (faux dès le
    /// `play()` demandé) et les promesses pendent jusqu'au
    /// premier rapport `playing` post-claim — le rejet
    /// `NotSupportedError` immédiat n'a de sens que hors
    /// attach MSE. `NodeId → (play demandé, promesses)`.
    el_play: HashMap<NodeId, (bool, Vec<ObjId>)>,
    /// Protos piste JS-C327 — `[audio, video, text]` pour
    /// `AudioTrack`/`VideoTrack`/`TextTrack` et leurs listes
    /// (paresseux, racinés GC comme les autres protos).
    track_protos: [Option<ObjId>; 3],
    track_list_protos: [Option<ObjId>; 3],
    /// `TrackEvent`s filés en **tâche** (JS-C327 — même
    /// couture que `pending_events`/JS-C324) : `(liste_cible,
    /// type, piste)`. `addtrack`/`removetrack` tirés à la
    /// frontière de job — un listener enregistré juste après
    /// l'`appendBuffer` d'init doit les voir (pattern
    /// canonique player : append puis `addEventListener`).
    pending_track_events: std::collections::VecDeque<(ObjId, &'static str, Value)>,
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
            self.media_error_proto,
        ]
        .into_iter()
        .flatten()
        {
            ids.push(p);
        }
        ids.extend(self.object_urls.values().copied());
        ids.extend(self.pending_events.iter().map(|(t, _)| *t));
        // Un `MediaSource` attaché survit tant que l'élément
        // vit (spec : l'attache le maintient référencé —
        // l'attribut `src` n'est pas une racine GC).
        ids.extend(self.attached_to.keys().copied());
        for p in self
            .track_protos
            .iter()
            .chain(self.track_list_protos.iter())
            .flatten()
        {
            ids.push(*p);
        }
        for (t, _, tr) in &self.pending_track_events {
            ids.push(*t);
            crate::heap::push_value(ids, *tr);
        }
        // Promesses `play()` pendantes pré-claim (§B-88) —
        // le tir différé ne doit pas trouver un wrapper
        // collecté (même règle que `pending_events`).
        for (_, pids) in self.el_play.values() {
            ids.extend(pids.iter().copied());
        }
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
    /// Un init `moov` a été appendé sur un des `SourceBuffer`
    /// (§B-88) — les métadonnées de la présentation sont
    /// connues côté élément → `loadedmetadata` (spec
    /// « initialization segment received »).
    init_seen: bool,
    /// Realm propriétaire (§B-89) — `cur_ctx` au
    /// `new MediaSource()`. Les events filés vers ce `ms`
    /// sont routés dans la file **de ce ctx** : les host-traits
    /// (`MsHost::set`, `appendBuffer`…) tournent sous le
    /// `cur_ctx` de l'appel, qui peut être un realm parasite
    /// (livraison session, pompe worker, transition) — sans
    /// ce tag l'event atterrissait dans la file du parasite et
    /// mourait au drain (vol 635 : `sourceopen` perdu).
    ctx: CtxId,
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
    /// `timestampOffset` posé explicitement depuis le dernier
    /// append (JS-C325) — en mode `sequence` le prochain
    /// append part de cette valeur puis le chaînage reprend
    /// (spec) ; consommé par chaque `appendBuffer`.
    offset_dirty: bool,
    /// Timescale média lu dans le `mdhd` de l'init segment
    /// fMP4 (JS-C285) — sert à convertir `tfdt`/durées
    /// `trun` des `moof` en secondes. `None` tant qu'aucun
    /// init fMP4 n'a été appendé (repli estimateur).
    timescale: Option<u32>,
    /// Pistes déclarées par le dernier init `moov` (JS-C327)
    /// — ordre `trak` (spec). État mutable spec par piste
    /// (`enabled`/`selected`/`mode`) + wrapper JS caché
    /// (identité stable entre lectures).
    tracks: Vec<SbTrack>,
    /// Wrappers `AudioTrackList`/`VideoTrackList`/
    /// `TextTrackList` cachés — `[audio, video, text]`
    /// (`sb.audioTracks === sb.audioTracks` spec).
    track_lists: [Option<Value>; 3],
}

/// Piste exposée aux listes `*Tracks` (JS-C327) — clone
/// borné (snapshot pour les events `removetrack` : la piste
/// disparue garde un wrapper valide pour l'événement).
#[derive(Clone)]
struct SbTrack {
    /// `tkhd.track_ID` — rendu en `DOMString` (spec).
    id: u32,
    /// `"audio"|"video"|"text"` — quelle liste la porte.
    list: &'static str,
    /// `kind` `TextTrack` ; `""` audio/vidéo.
    kind: &'static str,
    /// `mdhd.language` dépackée, `""` si `und`.
    lang: String,
    /// `AudioTrack.enabled` (défaut spec `true`).
    enabled: bool,
    /// `VideoTrack.selected` (défaut `false`).
    selected: bool,
    /// `TextTrack.mode` : `"disabled"` initial (spec MSE —
    /// les pistes in-band démarrent masquées).
    mode: &'static str,
    /// Wrapper JS caché (identité stable).
    obj: Option<Value>,
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

/// Index de liste `audio|video|text` → slot `[0,1,2]`.
fn list_idx(list: &str) -> usize {
    match list {
        "audio" => 0,
        "video" => 1,
        _ => 2,
    }
}

/// `AudioTrack` | `VideoTrack` | `TextTrack` (JS-C327) —
/// champs immuables pris en snapshot à la création (un
/// wrapper porté par un `removetrack` reste lisible après
/// la sortie de piste) ; l'état mutable spec vit dans
/// `SbData.tracks[idx]` quand la piste est présente.
struct TrackHost {
    d: Rc<RefCell<SbData>>,
    /// Index dans `SbData.tracks` — `usize::MAX` = piste
    /// retirée (wrapper détaché : lectures sur `snap`,
    /// sets = no-op documentés).
    idx: usize,
    /// Wrapper `SourceBuffer` parent (`track.sourceBuffer`).
    sb: Value,
    snap: SbTrack,
}

/// `AudioTrackList` | `VideoTrackList` | `TextTrackList` —
/// vue vivante filtrée de `SbData.tracks` (ordre `trak`).
struct TrackListHost {
    d: Rc<RefCell<SbData>>,
    list: &'static str,
}

/// Wrapper piste (créé à la demande, caché dans
/// `SbTrack.obj` tant que la piste est présente — identité
/// stable `list[0] === list[0]`).
fn track_obj(e: &mut Engine, d: &Rc<RefCell<SbData>>, idx: usize, sb: Value) -> Option<Value> {
    {
        let sd = d.borrow();
        if let Some(o) = sd.tracks.get(idx).and_then(|t| t.obj) {
            return Some(o);
        }
    }
    let (snap, proto) = {
        let sd = d.borrow();
        let t = sd.tracks.get(idx)?;
        (t.clone(), track_proto(e, t.list))
    };
    let v = e.new_host_object(Rc::new(TrackHost {
        d: d.clone(),
        idx,
        sb,
        snap,
    }));
    if let Value::Obj(id) = v {
        if let Some(o) = e.heap.obj_mut(id) {
            o.proto = Value::Obj(proto);
        }
        d.borrow_mut().tracks[idx].obj = Some(v);
    }
    Some(v)
}

/// Wrapper piste **détachée** pour un `removetrack` — la
/// piste n'est plus dans `SbData.tracks` : snapshot pur.
fn detached_track_obj(e: &mut Engine, d: &Rc<RefCell<SbData>>, sb: Value, snap: SbTrack) -> Value {
    let proto = track_proto(e, snap.list);
    let v = e.new_host_object(Rc::new(TrackHost {
        d: d.clone(),
        idx: usize::MAX,
        sb,
        snap,
    }));
    if let Value::Obj(id) = v {
        if let Some(o) = e.heap.obj_mut(id) {
            o.proto = Value::Obj(proto);
        }
    }
    v
}

/// `sb.<kind>Tracks` — liste cachée, matérialisée à la
/// demande (nécessaire aussi pour porter les `TrackEvents`
/// même si la page ne l'a jamais lue, comme
/// `addsourcebuffer` sur `sourceBuffers`).
fn track_list(e: &mut Engine, d: &Rc<RefCell<SbData>>, list: &'static str) -> Value {
    let i = list_idx(list);
    if let Some(l) = d.borrow().track_lists[i] {
        return l;
    }
    let lp = track_list_proto(e, list);
    let v = e.new_host_object(Rc::new(TrackListHost { d: d.clone(), list }));
    if let Value::Obj(id) = v {
        if let Some(o) = e.heap.obj_mut(id) {
            o.proto = Value::Obj(lp);
        }
        d.borrow_mut().track_lists[i] = Some(v);
    }
    v
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

// ------------------------------------------------------------------
// Routage des états par ctx (§B-89)
// ------------------------------------------------------------------
//
// `MseState` est **document-local** — swapé par
// `Engine::switch_to`. Mais `MsData`/`SbData` vivent en
// `Rc<RefCell>` sur les objets heap : un host-trait
// (`MsHost::set`, `SbHost::set`, `appendBuffer`, getter
// `readyState`/`paused`…) peut tourner sous un `cur_ctx`
// parasite — realm de livraison session, pompe worker, realm
// de transition `context_new`. Sans routage, un push sous ce
// parasite atterrissait dans la file **du parasite** : drainé
// sous lui, `attached_to`/`dom_ctx` du mauvais document →
// drop silencieux (vol 635 : `loadstart`/`sourceopen`/
// `durationchange` perdus, `mseBuffers` jamais peuplé). Le
// drain per-ctx redevient un filtre, pas un vol de file :
// chaque push est routé vers la file du ctx **propriétaire**.

/// `MseState` mutable du ctx `id` — `e.mse` s'il est courant,
/// le contexte garé sinon. `None` = ctx détruit.
fn mse_state_at(e: &mut Engine, id: CtxId) -> Option<&mut MseState> {
    if id == e.cur_ctx {
        Some(&mut e.mse)
    } else {
        e.parked.get_mut(&id).map(|c| &mut c.mse)
    }
}

/// `MseState` lu du ctx `id` — mêmes règles.
fn mse_state_ref(e: &Engine, id: CtxId) -> Option<&MseState> {
    if id == e.cur_ctx {
        Some(&e.mse)
    } else {
        e.parked.get(&id).map(|c| &c.mse)
    }
}

/// `dom_ctx` du ctx `id` — celui du document qui porte
/// l'élément attaché (un dispatch sous un autre ctx verrait
/// `NodeId` étranger → drop).
fn mse_dom_at(
    e: &Engine,
    id: CtxId,
) -> Option<(crate::dom_host::DomRef, crate::dom_host::NodeCache)> {
    if id == e.cur_ctx {
        e.dom_ctx.clone()
    } else {
        e.parked.get(&id).and_then(|c| c.dom_ctx.clone())
    }
}

/// Realm propriétaire du `MediaSource` (`MsData.ctx`).
fn ms_ctx(e: &Engine, mid: ObjId) -> Option<CtxId> {
    ms_data(e, Value::Obj(mid)).map(|d| d.borrow().ctx)
}

/// `(ctx, node)` de l'attache `mid` — scan `e.mse` (courant)
/// puis les contextes garés : l'attache vit dans le document
/// de l'élément, qui peut différer du `cur_ctx` de l'appelant.
fn attach_ctx(e: &Engine, mid: ObjId) -> Option<(CtxId, NodeId)> {
    if let Some(&n) = e.mse.attached_to.get(&mid) {
        return Some((e.cur_ctx, n));
    }
    e.parked
        .iter()
        .find_map(|(id, c)| c.mse.attached_to.get(&mid).map(|&n| (*id, n)))
}

/// `(ctx, mid)` de l'attache portant `node` — mêmes scans.
fn el_attach(e: &Engine, node: NodeId) -> Option<(CtxId, ObjId)> {
    if let Some(mid) = e
        .mse
        .attached_to
        .iter()
        .find_map(|(&m, &n)| (n == node).then_some(m))
    {
        return Some((e.cur_ctx, mid));
    }
    e.parked.iter().find_map(|(id, c)| {
        c.mse
            .attached_to
            .iter()
            .find_map(|(&m, &n)| (n == node).then_some(m))
            .map(|m| (*id, m))
    })
}

/// L'état `st` connaît-il `node` (ladder/play/attache) ?
fn has_el(st: &MseState, node: NodeId) -> bool {
    st.el_ladder.contains_key(&node)
        || st.el_play.contains_key(&node)
        || st.attached_to.values().any(|&n| n == node)
}

/// Ctx dont l'`MseState` connaît `node` — pour les accessors
/// élément (`readyState`/`paused`/`play`/`load`…) appelés
/// sous un realm parasite.
fn el_ctx(e: &Engine, node: NodeId) -> Option<CtxId> {
    if has_el(&e.mse, node) {
        return Some(e.cur_ctx);
    }
    e.parked
        .iter()
        .find_map(|(id, c)| has_el(&c.mse, node).then_some(*id))
}

/// Miroir du match de `deliver_mse_events` : events adressés
/// à l'**élément** attaché (routés vers le ctx de l'attache).
fn is_el_event(ty: &str) -> bool {
    matches!(
        ty,
        "loadstart"
            | "loadedmetadata"
            | "loadeddata"
            | "canplay"
            | "canplaythrough"
            | "durationchange"
            | "progress"
            | "emptied"
            | "play"
            | "playing"
            | "pause"
            | "ended"
            | "seeking"
            | "seeked"
            | "timeupdate"
            | "volumechange"
            | "ratechange"
            | "waiting"
            | "stalled"
            | "suspend"
            | "abort"
            | "error"
    )
}

/// Push routé (§B-89) : la file de destination est celle du
/// ctx **propriétaire**, jamais celle du `cur_ctx` pousseur.
/// Events élément → ctx de l'attache ; events `MediaSource`
/// (`sourceopen`/`sourceended`) → realm du `ms`. Propriétaire
/// inconnu ou mort → repli file courante (une livraison
/// possible vaut mieux qu'un drop silencieux).
fn mse_push(e: &mut Engine, mid: ObjId, ty: &'static str) {
    let owner = if is_el_event(ty) {
        attach_ctx(e, mid)
            .map(|(c, _)| c)
            .or_else(|| ms_ctx(e, mid))
    } else {
        ms_ctx(e, mid)
    };
    match owner.and_then(|o| mse_state_at(e, o)) {
        Some(st) => st.pending_events.push_back((mid, ty)),
        None => e.mse.pending_events.push_back((mid, ty)),
    }
}

/// Push `TrackEvent` routé (§B-89) — les listeners vivent
/// dans le realm du `MediaSource` parent de la liste.
fn track_push(e: &mut Engine, lid: ObjId, ty: &'static str, track: Value) {
    let owner = track_list_data(e, lid).and_then(|d| {
        let ms = d.borrow().ms;
        let Value::Obj(mid) = ms else { return None };
        ms_ctx(e, mid)
    });
    match owner.and_then(|o| mse_state_at(e, o)) {
        Some(st) => st.pending_track_events.push_back((lid, ty, track)),
        None => e.mse.pending_track_events.push_back((lid, ty, track)),
    }
}

/// `TrackListHost` interne — back-ref `SourceBuffer` pour le
/// routage §B-89 (`lid` → `sd.ms` → realm du `ms`).
fn track_list_data(e: &Engine, lid: ObjId) -> Option<Rc<RefCell<SbData>>> {
    match e.heap.obj(lid).map(|o| &o.kind) {
        Some(ObjKind::Host(h)) => h
            .clone()
            .as_any()
            .downcast_ref::<TrackListHost>()
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

/// `readyState` du `MediaSource` parent (`sd.ms` = wrapper) —
/// `None` = parent mort (orphelin, traité comme `closed`).
fn parent_state(e: &Engine, sd: &SbData) -> Option<&'static str> {
    ms_data(e, sd.ms).map(|ms| ms.borrow().ready_state)
}

/// Garde `updating` spec (JS-C325) : un appel re-entrant
/// depuis un listener `updatestart`/`update` **synchrone**
/// doit jeter `InvalidStateError`, pas corrompre la machine
/// à états (nos events sont tirés en séquence — `updating`
/// n'est observable `true` que pendant leur fenêtre).
fn sb_updating_guard(e: &mut Engine, sd: &SbData, op: &'static str) -> JsResult<()> {
    if sd.updating {
        return Err(mse_err(
            e,
            "InvalidStateError",
            &format!("SourceBuffer.{op}: updating"),
        ));
    }
    Ok(())
}

/// Spec (JS-C325) — `appendBuffer`/`abort` sur un parent
/// `readyState=="ended"` : `open` + `sourceopen` **filé en
/// tâche** (file JS-C324). C'est le mécanisme live-edge des
/// players réels — `endOfStream()` à la frontière puis les
/// appends reprennent ; sans le flip `mse_pull_segments`
/// rendait `eos` figé (file vide + `ended`) et l'hôte
/// croyait le flux clos. `closed`/orphelin : déviation
/// documentée conservée (le transport sert hors attache).
fn reopen_if_ended(e: &mut Engine, sd: &SbData) {
    if parent_state(e, sd) != Some("ended") {
        return;
    }
    let Some(ms) = ms_data(e, sd.ms) else {
        return;
    };
    ms.borrow_mut().ready_state = "open";
    if let Value::Obj(mid) = sd.ms {
        mse_push(e, mid, "sourceopen");
    }
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
    dispatch_mse_event(e, target, ty, ev)
}

/// `TrackEvent` `{type, target, track}` sur une `*TrackList`
/// (JS-C327) — même rail `on<ty>` + `__et` que `fire_mse`.
fn fire_track_event(e: &mut Engine, list: ObjId, ty: &'static str, track: Value) -> JsResult<()> {
    let ev = mse_event(e, ty, Value::Obj(list));
    if let Value::Obj(eo) = ev {
        e.set_own(eo, "track", track, true);
    }
    dispatch_mse_event(e, list, ty, ev)
}

/// Corps de dispatch partagé : `on<ty>` own-prop puis
/// listeners `addEventListener` (rail `__et`).
fn dispatch_mse_event(e: &mut Engine, target: ObjId, ty: &str, ev: Value) -> JsResult<()> {
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

/// Proto piste JS-C327 — `AudioTrack`/`VideoTrack`/
/// `TextTrack`, paresseux par kind (`[0,1,2]`).
fn track_proto(e: &mut Engine, list: &'static str) -> ObjId {
    let i = list_idx(list);
    if let Some(p) = e.mse.track_protos[i] {
        return p;
    }
    let tag = match list {
        "audio" => "AudioTrack",
        "video" => "VideoTrack",
        _ => "TextTrack",
    };
    let p = tagged_proto(e, tag);
    if list == "text" {
        // `TextTrack` est un EventTarget (spec) — cuechange
        // vit sur la piste, pas la liste.
        install_et_methods(e, p);
    }
    e.mse.track_protos[i] = Some(p);
    p
}

/// Proto liste JS-C327 — `AudioTrackList`/`VideoTrackList`/
/// `TextTrackList` : `EventTarget` (`addtrack`/`removetrack`/
/// `change`) + `getTrackById`.
fn track_list_proto(e: &mut Engine, list: &'static str) -> ObjId {
    let i = list_idx(list);
    if let Some(p) = e.mse.track_list_protos[i] {
        return p;
    }
    let tag = match list {
        "audio" => "AudioTrackList",
        "video" => "VideoTrackList",
        _ => "TextTrackList",
    };
    let p = tagged_proto(e, tag);
    install_et_methods(e, p);
    e.add_method(p, "getTrackById", tl_get_track_by_id);
    e.mse.track_list_protos[i] = Some(p);
    p
}

/// Proto `MediaError` (étape 182) — paresseux comme les
/// autres interfaces média ; `code`/`message` vivent sur
/// l'instance (props own, spec), les constantes
/// `MEDIA_ERR_*` sur le constructeur ET le proto.
fn me_proto(e: &mut Engine) -> ObjId {
    if let Some(p) = e.mse.media_error_proto {
        return p;
    }
    let p = tagged_proto(e, "MediaError");
    for (name, code) in MEDIA_ERR_CONSTANTS {
        e.set_own(p, name, Value::Num(code), false);
    }
    e.mse.media_error_proto = Some(p);
    p
}

/// `MediaError` — interface non constructible (spec :
/// `Illegal constructor`).
fn me_illegal_ctor(e: &mut Engine, _t: Value, _args: &[Value]) -> JsResult<Value> {
    Err(e.type_error("MediaError: Illegal constructor"))
}

/// Constantes `MediaError` spec — posées sur le
/// constructeur et le prototype (lecture des deux côtés).
const MEDIA_ERR_CONSTANTS: [(&str, f64); 4] = [
    ("MEDIA_ERR_ABORTED", 1.0),
    ("MEDIA_ERR_NETWORK", 2.0),
    ("MEDIA_ERR_DECODE", 3.0),
    ("MEDIA_ERR_SRC_NOT_SUPPORTED", 4.0),
];

/// Objet `MediaError` pour `el.error` (étape 182) —
/// `code` classifié depuis la chaîne hôte
/// (`MediaErrorCode::of`, 1-4) au lieu du `4` en dur
/// historique ; instance du proto `MediaError`
/// (`instanceof` + `MEDIA_ERR_*` hérités).
pub(crate) fn media_error_obj(e: &mut Engine, msg: &str) -> Value {
    let p = me_proto(e);
    let o = e.new_object();
    if let Some(ob) = e.heap.obj_mut(o) {
        ob.proto = Value::Obj(p);
    }
    e.set_own(
        o,
        "code",
        Value::Num(crate::dom::MediaErrorCode::of(msg) as u32 as f64),
        true,
    );
    let m = e.str_val(msg);
    e.set_own(o, "message", m, true);
    Value::Obj(o)
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
        init_seen: false,
        ctx: e.cur_ctx,
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

/// Interfaces piste JS-C327 non constructibles (spec :
/// `Illegal constructor` — les tracks naissent du `moov`,
/// pas de `new`).
fn track_illegal_ctor(e: &mut Engine, _t: Value, _args: &[Value]) -> JsResult<Value> {
    Err(e.type_error("Illegal constructor"))
}

/// Réconciliation « initialization segment received »
/// (JS-C327, spec MSE) : le nouvel init **remplace** la
/// déclaration de pistes. Conservation par `(list, id)` —
/// un re-init ABR hls.js (mêmes `track_ID`) préserve wrapper
/// et état (`enabled`/`selected`/`mode`) ; disparues →
/// `removetrack`, nouvelles → `addtrack` (valeurs par défaut
/// spec : `enabled`, `!selected`, `mode:"disabled"`).
/// Retourne les ops à filer en tâche `(snapshot, type,
/// index)` dans l'ordre spec : retraits d'abord, puis
/// ajouts. `index` = position finale dans `sd.tracks`
/// (`Some` pour les ajouts — la piste y vit ; `None` pour
/// les retraits — wrapper détaché à créer).
fn reconcile_tracks(
    sd: &mut SbData,
    new: Vec<Fmp4Track>,
) -> Vec<(SbTrack, &'static str, Option<usize>)> {
    let mut ops = Vec::new();
    let mut kept: Vec<SbTrack> = Vec::with_capacity(new.len());
    for old in sd.tracks.drain(..) {
        if new.iter().any(|n| n.id == old.id && n.list == old.list) {
            kept.push(old);
        } else {
            ops.push((old, "removetrack", None));
        }
    }
    let mut out = Vec::with_capacity(new.len());
    for n in new {
        if let Some(i) = kept.iter().position(|k| k.id == n.id && k.list == n.list) {
            out.push(kept.remove(i));
        } else {
            let t = SbTrack {
                id: n.id,
                list: n.list,
                kind: n.kind,
                lang: n.lang,
                enabled: true,
                selected: false,
                mode: "disabled",
                obj: None,
            };
            ops.push((t.clone(), "addtrack", Some(out.len())));
            out.push(t);
        }
    }
    sd.tracks = out;
    ops
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
/// Support type MSE partagé (JS-C325 — extrait pour
/// `changeType`) : conteneur connu, et tous les codecs du
/// param `codecs=` déclarés quand la liste est présente
/// (le conteneur seul suffit sinon, table non vide requise).
/// Table vide → `false` partout (état C173 — jamais de
/// promesse sans transport déclaré, ADR-0023).
fn mse_type_supported(e: &Engine, mime: &str) -> bool {
    let mime = mime.to_lowercase();
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
    known
        && match mime.split("codecs=").nth(1) {
            Some(cstr) => {
                let cstr = cstr.trim_matches(|c| c == '"' || c == '\'');
                cstr.split(',')
                    .filter_map(codec_prefix)
                    .all(|p| e.mse.declared_codecs.contains(&p))
            }
            // Pas de liste : le conteneur connu suffit —
            // honnête (le codec réel peut manquer).
            None => !e.mse.declared_codecs.is_empty(),
        }
}

/// Trace de mesure (Étape 202) — chaque interrogation MIME
/// `isTypeSupported`/`canPlayType` est journalisée avec son
/// verdict pour le vol corpus : `RUST_LOG=media_probe=info`
/// collecte la couverture réelle vs `mediaCodecProbes` de
/// l'app. `declared_n` = taille de la table d'hôte au moment
/// de la requête (course claim/feature-detect visible).
fn media_probe_log(site: &str, mime: &str, verdict: &str, declared_n: usize) {
    tracing::info!(target: "media_probe", site, %mime, verdict, declared_n);
}

fn ms_is_type_supported(e: &mut Engine, _t: Value, args: &[Value]) -> JsResult<Value> {
    let mime = e.to_js_string(args.first().copied().unwrap_or(Value::Undefined))?;
    let verdict = if mse_type_supported(e, &mime) {
        "maybe"
    } else {
        ""
    };
    media_probe_log(
        "mse.isTypeSupported",
        &mime,
        verdict,
        e.mse.declared_codecs.len(),
    );
    Ok(e.str_val(verdict))
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
    let verdict = media_can_play_type_verdict(e, arg);
    media_probe_log(
        "media.canPlayType",
        arg,
        verdict,
        e.mse.declared_codecs.len(),
    );
    verdict
}

fn media_can_play_type_verdict(e: &Engine, arg: &str) -> &'static str {
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
    // Spec (JS-C325) : chaîne vide → `TypeError`. La
    // validation du type reste tolérante (déviation
    // documentée — la table `declared_codecs` peut être
    // vide avant tout claim ; cf. en-tête de module).
    if mime.trim().is_empty() {
        return Err(e.type_error("MediaSource.addSourceBuffer: empty type"));
    }
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
        offset_dirty: false,
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
        tracks: Vec::new(),
        track_lists: [None, None, None],
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
/// file, pas synchrone). JS-C325 : garde spec `updating`
/// (`InvalidStateError` si un buffer met à jour — un appel
/// re-entrant pendant `updatestart` ne clôt pas le flux sous
/// le dos du buffer) ; param `"network"`/`"decode"` →
/// `MediaError` livré à l'élément attaché (infra étape 182),
/// autre valeur → `TypeError`. Déviations documentées
/// conservées : permis hors `"open"` (`closed` → `ended`
/// directement — le transport sert hors attache) et un
/// double appel est un no-op propre (`sourceended` non
/// re-filé).
fn ms_end_of_stream(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Some(d) = ms_data(e, this) else {
        return Err(e.type_error("MediaSource.endOfStream: Illegal invocation"));
    };
    // Spec : un `SourceBuffer` `updating` → InvalidStateError.
    for sbv in &d.borrow().buffers {
        let Some(sd) = sb_data(e, *sbv) else {
            continue;
        };
        if sd.borrow().updating {
            return Err(mse_err(
                e,
                "InvalidStateError",
                "MediaSource.endOfStream: buffer updating",
            ));
        }
    }
    let kind = match args.first() {
        None | Some(Value::Undefined) => None,
        Some(v) => {
            let s = e.to_js_string(*v)?;
            match s.as_str() {
                "network" | "decode" => Some(s),
                _ => return Err(e.type_error("MediaSource.endOfStream: bad error name")),
            }
        }
    };
    let mid = if let Value::Obj(id) = this {
        Some(id)
    } else {
        None
    };
    if d.borrow().ready_state != "ended" {
        d.borrow_mut().ready_state = "ended";
        if let Some(m) = mid {
            mse_push(e, m, "sourceended");
        }
    }
    if let (Some(m), Some(k)) = (mid, kind) {
        deliver_eos_error(e, m, &k)?;
    }
    Ok(Value::Undefined)
}

/// `endOfStream("network"|"decode")` → `MediaError` sur
/// l'élément attaché (JS-C325 — spec : la faute EOS est
/// rapportée à l'élément média). `v.error` reflète
/// `state:Error` quand la surface est **claimée** (position/
/// durée préservées du rapport courant) + les promesses
/// `play()` pendantes sont rejetées — même contrat que le
/// chemin `state` hôte (étape 182). L'event `error` est
/// tiré dans tous les cas : non-claimé = signal listener
/// seul (`el.error` reste `null` — l'élément sans surface
/// n'a pas d'état de lecture à corrompre, déviation
/// documentée). Attache morte → no-op sûr.
fn deliver_eos_error(e: &mut Engine, mid: ObjId, kind: &str) -> JsResult<()> {
    // §B-89 — attache et dom résolus dans le ctx propriétaire,
    // pas le `cur_ctx` de l'`endOfStream` (realm parasite →
    // `attached_to`/`dom_ctx` du mauvais document → drop).
    let Some((actx, node)) = attach_ctx(e, mid) else {
        return Ok(());
    };
    let Some((dom, cache)) = mse_dom_at(e, actx) else {
        return Ok(());
    };
    // Chaîne classifiable `MediaErrorCode::of` — "network"
    // → `MEDIA_ERR_NETWORK` (2), "decode" → `MEDIA_ERR_DECODE`
    // (3) : le message porte le code, comme le rapport hôte.
    let msg = format!("MediaSource endOfStream: {kind} error");
    let pids = {
        let mut d = dom.borrow_mut();
        match d.media.get_mut(&node) {
            Some(st) => {
                let prev = st.report.take();
                st.report = Some(MediaReport {
                    position_ms: prev.as_ref().map_or(0, |r| r.position_ms),
                    duration_ms: prev.as_ref().and_then(|r| r.duration_ms),
                    buffered_ms: prev.as_ref().map_or(0, |r| r.buffered_ms),
                    state: MediaState::Error,
                    error: Some(msg.clone()),
                    buffered_ranges: prev
                        .as_ref()
                        .map_or_else(Vec::new, |r| r.buffered_ranges.clone()),
                    frame_stats: prev.as_ref().and_then(|r| r.frame_stats),
                });
                std::mem::take(&mut st.pending_play)
            }
            None => Vec::new(),
        }
    };
    let mv = e.str_val(&msg);
    for p in pids {
        e.reject_promise(p, mv);
    }
    e.dispatch_media(&dom, &cache, node, "error")
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

/// `tkhd` → `track_ID` — v0 : offset 12, v1 : offset 20.
fn tkhd_id(b: &[u8]) -> Option<u32> {
    let off = match *b.first()? {
        0 => 12,
        1 => 20,
        _ => return None,
    };
    b.get(off..off + 4)
        .map(|s| u32::from_be_bytes(s.try_into().unwrap_or([0; 4])))
}

/// `mdhd` → `language` ISO 639-2/T (u16 packé 3×5 bits +
/// `0x60`) — v0 : offset 20, v1 : offset 32. `und`/`0` →
/// `""` (spec : langue indéterminée honnête).
fn mdhd_language(b: &[u8]) -> Option<String> {
    let off = match *b.first()? {
        0 => 20,
        1 => 32,
        _ => return None,
    };
    let w = u16::from_be_bytes(b.get(off..off + 2)?.try_into().ok()?);
    if w == 0 || w == 0x55c4 {
        return Some(String::new());
    }
    let c = |s: u16| char::from_u32(u32::from((w >> s) & 0x1f) + 0x60);
    Some([c(10)?, c(5)?, c(0)?].into_iter().collect())
}

/// `hdlr` → `(liste, kind)` depuis `handler_type` (offset 8
/// du payload : version+flags, `pre_defined`, puis le
/// fourcc). `vide`→video, `soun`→audio, `text`/`subt`/
/// `sbtl`→texte `"subtitles"`, `clcp`→`"captions"`,
/// `meta`→`"metadata"` ; autres (`hint`…) → ignorés — pas
/// de fausse piste exposée.
fn hdlr_track_kind(b: &[u8]) -> (Option<&'static str>, &'static str) {
    match b.get(8..12) {
        Some(b"vide") => (Some("video"), ""),
        Some(b"soun") => (Some("audio"), ""),
        Some(b"text" | b"subt" | b"sbtl") => (Some("text"), "subtitles"),
        Some(b"clcp") => (Some("text"), "captions"),
        Some(b"meta") => (Some("text"), "metadata"),
        _ => (None, ""),
    }
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
/// Une piste déclarée par un `trak` d'init (JS-C327) —
/// alimente `audioTracks`/`videoTracks`/`textTracks` du
/// `SourceBuffer` : c'est la donnée qu'un navigateur lit en
/// parsant le `moov`, sans démuxage des samples.
struct Fmp4Track {
    /// `tkhd.track_ID`.
    id: u32,
    /// Liste de rattachement : `"audio"` (`soun`) |
    /// `"video"` (`vide`) | `"text"` (`text`/`subt`/`sbtl`/
    /// `clcp`/`meta`). `hdlr` inconnu → piste ignorée.
    list: &'static str,
    /// `kind` `TextTrack` (`"subtitles"`/`"captions"`/
    /// `"metadata"`) ; `""` pour audio/vidéo (spec : kind
    /// indéterminable depuis `hdlr` seul).
    kind: &'static str,
    /// `mdhd.language` dépackée (ISO 639-2/T 3 lettres) ;
    /// `""` quand `und`/absente (spec : chaîne vide honnête).
    lang: String,
}

struct Fmp4Scan {
    timescale: Option<u32>,
    moofs: Vec<(u64, u64)>,
    /// Pistes des `trak` du `moov` (JS-C327) — vide hors init.
    tracks: Vec<Fmp4Track>,
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
        tracks: Vec::new(),
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
                    // JS-C327 — chaque `trak` = une piste :
                    // `tkhd.track_ID`, `mdia.hdlr` (kind),
                    // `mdia.mdhd` (langue + timescale — le
                    // premier `mdhd` vu garde le timescale,
                    // invariant JS-C285).
                    let mut tid = None;
                    let mut list = None;
                    let mut kind = "";
                    let mut lang = String::new();
                    walk_boxes(p2, |f3, p3| {
                        if f3 == b"tkhd" {
                            tid = tkhd_id(p3);
                        }
                        if f3 == b"mdia" {
                            walk_boxes(p3, |f4, p4| {
                                if f4 == b"mdhd" {
                                    if out.timescale.is_none() {
                                        out.timescale = mdhd_timescale(p4);
                                    }
                                    if let Some(l) = mdhd_language(p4) {
                                        lang = l;
                                    }
                                }
                                if f4 == b"hdlr" {
                                    let h = hdlr_track_kind(p4);
                                    list = h.0;
                                    kind = h.1;
                                }
                            });
                        }
                    });
                    if let (Some(id), Some(list)) = (tid, list) {
                        out.tracks.push(Fmp4Track {
                            id,
                            list,
                            kind,
                            lang,
                        });
                    }
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
    let mut track_ops: Vec<(SbTrack, &'static str, Option<usize>)> = Vec::new();
    let mut saw_moov = false;
    {
        let mut sd = d.borrow_mut();
        // JS-C325 — gardes spec : appel re-entrant pendant
        // `updatestart`/`update` → `InvalidStateError` ;
        // parent `ended` → `open` + `sourceopen` filé
        // (mécanisme live-edge — voir `reopen_if_ended`).
        sb_updating_guard(e, &sd, "appendBuffer")?;
        reopen_if_ended(e, &sd);
        // JS-C325 — mode `sequence` : la base d'offset chaîne
        // à la fin du dernier range bufferisé (spec — appends
        // contigus sans `timestampOffset`) ; un set explicite
        // (`offset_dirty`) s'applique une fois puis la chaîne
        // reprend. `timestamp_offset` est swapé le temps du
        // calcul pour réutiliser les deux chemins existants.
        let eff_offset = if sd.mode == "sequence" && !sd.offset_dirty {
            sd.est_ranges
                .last()
                .map_or(sd.timestamp_offset, |&(_, b)| b)
        } else {
            sd.timestamp_offset
        };
        if sd.mode == "sequence" && sd.offset_dirty {
            // Set explicite en `sequence` = nouveau point de
            // chaînage (spec), pas un delta observé : la
            // correction rétroactive de span de l'estimateur
            // ne doit pas « réparer » la plage précédente en
            // l'étirant jusqu'au nouvel offset.
            sd.last_append_t = None;
        }
        let next = sd.bytes.saturating_add(bytes.len());
        if next > SB_BYTE_CAP {
            return Err(mse_err(
                e,
                "QuotaExceededError",
                "SourceBuffer: byte quota exceeded",
            ));
        }
        let raw_offset = std::mem::replace(&mut sd.timestamp_offset, eff_offset);
        // JS-C285 — timestamps en bande : scan fMP4 AVANT
        // l'entrée en file (la donnée réelle — `mdhd` de
        // l'init, `tfdt`+durées `trun` des `moof` — est ce
        // qu'un navigateur obtient en démuxant).
        let mut scan = fmp4_scan(&bytes);
        // JS-C327 — init `moov` : réconciliation des pistes
        // déclarées (spec « initialization segment
        // received ») — TrackEvents collectés pour la file
        // de tâches (tirés après la rafale `update*`).
        if scan.has_moov {
            saw_moov = true;
            track_ops = reconcile_tracks(&mut sd, std::mem::take(&mut scan.tracks));
        }
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
        // JS-C325 — post-append : en `sequence` l'offset
        // avance à la nouvelle fin bufferisée (le prochain
        // append chaîne là, spec) ; en `segments` la valeur
        // de la page est restaurée inchangée. L'offset
        // explicite (`offset_dirty`) est consommé dans tous
        // les cas — un set vaut pour un seul append.
        if sd.mode == "sequence" {
            sd.timestamp_offset = sd.est_ranges.last().map_or(eff_offset, |&(_, b)| b);
        } else {
            sd.timestamp_offset = raw_offset;
        }
        sd.offset_dirty = false;
    }
    // JS-C327 — `TrackEvent`s filés en **tâche** (spec
    // « queue a task » — même couture JS-C324 que
    // `sourceopen` : un listener posé juste après
    // l'`appendBuffer` d'init doit voir `addtrack`).
    for (snap, ty, idx) in track_ops {
        let track_v = match idx {
            Some(i) => track_obj(e, &d, i, this).unwrap_or(Value::Null),
            None => detached_track_obj(e, &d, this, snap.clone()),
        };
        if let Value::Obj(lid) = track_list(e, &d, snap.list) {
            track_push(e, lid, ty, track_v);
        }
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
    // §B-88 — l'échelle de cycle de vie de **l'élément** est
    // montée par les appends (spec MSE : les métadonnées
    // viennent de l'init `moov`, les données des plages
    // bufferisées — l'élément n'a pas besoin de l'hôte pour
    // savoir). Position 0 tant que non claimé : init reçu →
    // `loadedmetadata` ; plage couvrant la tête →
    // `loadeddata`+`canplay` ; fin ≥ durée connue ou marge
    // `CANPLAYTHROUGH_AHEAD_MS` → `canplaythrough`. Filé en
    // tâche (JS-C324), cible = `attached_to[mid]` — servi
    // après `updateend`. **Claimé** : le bras hôte
    // (`deliver_media_state`, étape 182) possède l'échelle —
    // la poussée d'appends ne double pas les tirs.
    if let Value::Obj(mid) = d.borrow().ms {
        // §B-89 — attache/dom/échelle résolus dans le ctx
        // **propriétaire** : un `appendBuffer` invoqué sous
        // un realm parasite (pompe, livraison) monte quand
        // même l'échelle du vrai document.
        if let Some((actx, node)) = attach_ctx(e, mid) {
            let claimed =
                mse_dom_at(e, actx).is_some_and(|(dom, _)| dom.borrow().media.contains_key(&node));
            if !claimed {
                let mut end = 0.0f64;
                let mut covers = false;
                let (init, dur) = match ms_data(e, Value::Obj(mid)) {
                    Some(ms) => {
                        let mut m = ms.borrow_mut();
                        if saw_moov {
                            m.init_seen = true;
                        }
                        let (init, dur) = (m.init_seen, m.duration);
                        for sbv in &m.buffers {
                            if let Some(sd2) = sb_data(e, *sbv) {
                                for &(a, b) in &sd2.borrow().est_ranges {
                                    end = end.max(b);
                                    if a <= 0.0 && b > 0.0 {
                                        covers = true;
                                    }
                                }
                            }
                        }
                        (init, dur)
                    }
                    None => (false, f64::NAN),
                };
                let cur = mse_state_ref(e, actx)
                    .and_then(|st| st.el_ladder.get(&node).copied())
                    .unwrap_or(0);
                let mut target = cur;
                if init {
                    target = target.max(1);
                }
                if covers {
                    target = target.max(3);
                }
                if (dur.is_finite() && end >= dur)
                    || end * 1000.0 >= crate::dom_host::CANPLAYTHROUGH_AHEAD_MS as f64
                {
                    target = target.max(4);
                }
                for lvl in (cur + 1)..=target {
                    mse_push(
                        e,
                        mid,
                        match lvl {
                            1 => "loadedmetadata",
                            2 => "loadeddata",
                            3 => "canplay",
                            _ => "canplaythrough",
                        },
                    );
                }
                if let Some(st) = mse_state_at(e, actx) {
                    st.el_ladder.insert(node, target);
                }
            }
        }
        // `progress` élément (étape 182) : la croissance du
        // buffer MSE vit dans `appendBuffer`, pas dans le
        // `buffered_ms` du rapport hôte.
        mse_push(e, mid, "progress");
    }
    fire_mse_guarded(e, sid, "updateend", &mut first_err);
    match first_err {
        Some(err) => Err(err),
        None => Ok(Value::Undefined),
    }
}

/// `sb.abort()` — spec (JS-C325) : annule l'op en vol
/// (`updating` → `abort`+`updateend`) et reset l'état
/// **parseur** — sans toucher le média committé. La sémantique
/// historique « abort vide tout » est retirée : elle vidait
/// `segments`/`est_ranges` → `sb.buffered`/`media.buffered`
/// tombaient à zéro après un abort que les players réels
/// utilisent au switch/détachement (hls.js) — famine
/// fantôme, `readyState` rétrogradé. `segments` (file de
/// transport déjà commitée), `served`, `est_ranges`,
/// `timescale` = média appendé, conservé ; seuls
/// `updating` et le chaînage de l'estimateur sont reset
/// (`last_append_t` — spec « reset parser state »).
/// Parent `ended` → `open` + `sourceopen` filé
/// (`reopen_if_ended`). `abort`+`updateend` tirés
/// inconditionnellement (déviation documentée — spec : la
/// paire n'existe qu'en abortant un append en vol ; notre
/// modèle est atomique, les players bouclent sur
/// `updateend` pour armer la suite).
fn sb_abort(e: &mut Engine, this: Value, _args: &[Value]) -> JsResult<Value> {
    let Some(d) = sb_data(e, this) else {
        return Err(e.type_error("SourceBuffer.abort: Illegal invocation"));
    };
    {
        let mut sd = d.borrow_mut();
        reopen_if_ended(e, &sd);
        sd.updating = false;
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
        sb_updating_guard(e, &sd, "remove")?;
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

/// `sb.changeType(mime)` — JS-C325 : garde `updating` +
/// validation spec `NotSupportedError` **quand la table
/// `declared_codecs` est renseignée** (vide = aucune vérité
/// terrain avant claim → tolérance documentée, même économie
/// qu'`addSourceBuffer`). Le nouveau mime met à jour
/// `sd.mime` — le seam `mse_source_buffers` le publie
/// gratuitement au prochain claim. Reset parseur spec :
/// la chaîne d'offsets de l'estimateur recommence.
fn sb_change_type(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Some(d) = sb_data(e, this) else {
        return Err(e.type_error("SourceBuffer.changeType: Illegal invocation"));
    };
    let mime = e.to_js_string(args.first().copied().unwrap_or(Value::Undefined))?;
    let mut sd = d.borrow_mut();
    sb_updating_guard(e, &sd, "changeType")?;
    if !e.mse.declared_codecs.is_empty() && !mse_type_supported(e, &mime) {
        return Err(mse_err(
            e,
            "NotSupportedError",
            "SourceBuffer.changeType: unsupported type",
        ));
    }
    sd.mime = mime;
    sd.last_append_t = None;
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
/// déjà `open`/`ended` n'est pas re-tirée. `node` enregistre le
/// binding attache → élément (`attached_to`, étape 182 — les
/// events élément comme `progress` le visent).
pub(crate) fn maybe_attach(e: &mut Engine, url: &str, node: NodeId) {
    let Some(mid) = object_url_media(e, url) else {
        return;
    };
    // Dernier posé gagne : rebinding `src=` ou deux éléments
    // sur le même `blob:` — `progress` va à l'attaché effectif.
    e.mse.attached_to.insert(mid, node);
    // §B-88 — cycle de vie élément : l'attache `src=blob:`
    // est l'algo de sélection de ressource — `loadstart` est
    // filé en tâche sur l'élément (avant `sourceopen`, ordre
    // navigateur), l'échelle repart de `HAVE_NOTHING`. Sans
    // ce tir le player arme ses listeners et attend le signal
    // à jamais (vol 634 : zéro event élément, `mseBuffers:[]`).
    e.mse.el_ladder.insert(node, 0);
    mse_push(e, mid, "loadstart");
    let Some(d) = ms_data(e, Value::Obj(mid)) else {
        return;
    };
    if d.borrow().ready_state != "closed" {
        return;
    }
    let dur = d.borrow().duration;
    d.borrow_mut().ready_state = "open";
    mse_push(e, mid, "sourceopen");
    // Durée connue à l'attache (fixée pré-`src=`) : l'élément
    // la reflète immédiatement → `durationchange` (spec).
    if dur.is_finite() {
        mse_push(e, mid, "durationchange");
    }
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
        // Les events de cycle de vie ciblent **l'élément**
        // attaché (`progress` étape 182, `loadstart`/
        // `durationchange`/échelle `loadedmetadata`… §B-88) —
        // dispatch média sur le nœud miroir, pas `fire_mse`
        // sur le wrapper ; attache morte → tir sauté.
        let r = match ty {
            "loadstart" | "loadedmetadata" | "loadeddata" | "canplay" | "canplaythrough"
            | "durationchange" | "progress" | "emptied" | "play" | "playing" | "pause"
            | "ended" | "seeking" | "seeked" | "timeupdate" | "volumechange" | "ratechange"
            | "waiting" | "stalled" | "suspend" | "abort" | "error" => {
                match (e.mse.attached_to.get(&target).copied(), e.dom_ctx.clone()) {
                    (Some(node), Some((dom, cache))) => e.dispatch_media(&dom, &cache, node, ty),
                    _ => Ok(()),
                }
            }
            _ => fire_mse(e, target, ty),
        };
        if let Err(err) = r {
            if first.is_none() {
                first = Some(err);
            }
        }
    }
    while let Some((list, ty, track)) = e.mse.pending_track_events.pop_front() {
        fired = true;
        if let Err(err) = fire_track_event(e, list, ty, track) {
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

    /// `readyState` de l'élément MSE-attaché **non claimé**
    /// (§B-88) — niveau monté par les `appendBuffer`
    /// (`el_ladder`) ; `0` hors attache. Claimé : le bras
    /// `deliver_media_state` possède l'échelle (seedé de ce
    /// niveau au claim — pas de re-tir). §B-89 : l'état est
    /// résolu dans le ctx de l'attache — un getter sous un
    /// realm parasite lit quand même le vrai niveau.
    pub(crate) fn mse_el_ready(&self, node: NodeId) -> u8 {
        el_ctx(self, node)
            .and_then(|id| mse_state_ref(self, id))
            .and_then(|st| st.el_ladder.get(&node).copied())
            .unwrap_or(0)
    }

    /// `paused` sous attache MSE pré-claim (§B-88) — le flag
    /// interne spec : `true` tant que `play()` n'a pas été
    /// demandé, `false` dès que l'intention est posée.
    pub(crate) fn mse_el_paused(&self, node: NodeId) -> bool {
        let armed = el_ctx(self, node)
            .and_then(|id| mse_state_ref(self, id))
            .is_some_and(|st| st.el_play.get(&node).is_some_and(|ent| ent.0));
        !armed
    }

    /// `play()` sur élément MSE-attaché pré-claim (§B-88) :
    /// la promesse **pend** (spec — elle tranche au premier
    /// `playing` post-claim, migre dans `pending_play` au
    /// claim) ; le rejet `NotSupportedError` immédiat n'a de
    /// sens que hors attach.
    pub(crate) fn mse_el_pend_play(&mut self, node: NodeId, pid: ObjId) {
        let Some(id) = el_ctx(self, node) else { return };
        if let Some(st) = mse_state_at(self, id) {
            let ent = st.el_play.entry(node).or_default();
            ent.0 = true;
            ent.1.push(pid);
        }
    }

    /// `pause()` pré-claim (§B-88) — l'intention retombe ;
    /// un `pause` est filé si un play était armé (spec).
    pub(crate) fn mse_el_pause(&mut self, node: NodeId) {
        let Some(id) = el_ctx(self, node) else { return };
        let armed = mse_state_ref(self, id)
            .is_some_and(|st| st.el_play.get(&node).is_some_and(|ent| ent.0));
        if armed {
            if let Some(st) = mse_state_at(self, id) {
                st.el_play.entry(node).or_default().0 = false;
            }
            if let Some((_, mid)) = el_attach(self, node) {
                mse_push(self, mid, "pause");
            }
        }
    }

    /// `load()` sur élément MSE-attaché (§B-88) — la
    /// sélection de ressource re-joue → `loadstart` re-filé
    /// (spec). L'échelle montée est conservée (approximation
    /// documentée : pas de ré-arme `emptied`).
    pub(crate) fn mse_el_refire_loadstart(&mut self, node: NodeId) {
        if let Some((_, mid)) = el_attach(self, node) {
            mse_push(self, mid, "loadstart");
        }
    }

    /// Handoff au claim (§B-88) : niveau `readyState` monté
    /// pré-claim + promesses `play()` pendantes migrées vers
    /// `MediaElState` (résolues au premier `playing` hôte).
    /// §B-89 : l'état est lu dans le ctx de l'attache — un
    /// claim relayé sous un realm parasite migre quand même.
    pub(crate) fn mse_el_handoff(&mut self, node: NodeId) -> (u8, Vec<ObjId>) {
        let Some(id) = el_ctx(self, node) else {
            return (0, Vec::new());
        };
        let Some(st) = mse_state_at(self, id) else {
            return (0, Vec::new());
        };
        (
            st.el_ladder.get(&node).copied().unwrap_or(0),
            st.el_play
                .remove(&node)
                .map_or_else(Vec::new, |(_, pids)| pids),
        )
    }

    /// `true` si un attach MSE vit sur `node` (§B-88) —
    /// l'échelle/pré-claim ne s'applique qu'à un élément
    /// réellement attaché (`el_ladder` posé par
    /// `maybe_attach`). §B-89 : résolu sur tous les ctx.
    pub(crate) fn mse_el_is_attached(&self, node: NodeId) -> bool {
        el_ctx(self, node).is_some()
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
    ///
    /// `peek = true` (vol 627 — diagnostic pull) : `from` est
    /// alors une fenêtre **en lecture seule** — `sd.cursor` et
    /// `sd.pulled` intacts, aucun chunk file→served. Un peek
    /// ne perturbe jamais le FIFO du consommateur réel ; un
    /// `?from=` nu (seek commité) le fait, par design.
    pub fn mse_pull_segments(
        &mut self,
        url: &str,
        buffer: usize,
        max: usize,
        from: Option<u64>,
        peek: bool,
    ) -> Option<MsePullOut> {
        let mid = self.mse.object_urls.get(url).copied()?;
        let ms = ms_data(self, Value::Obj(mid))?;
        let sbv = ms.borrow().buffers.get(buffer).copied()?;
        let sd = sb_data(self, sbv)?;
        let mut sd = sd.borrow_mut();
        if let Some(f) = from {
            if peek {
                let Ok((out, end, sections)) = peek_window(&sd, f, max) else {
                    // Hors fenêtre — même `416` filaire que `?from=`.
                    return Some(Err(()));
                };
                let total_end = sd.pulled + sd.segments.iter().map(|s| s.len() as u64).sum::<u64>();
                let eos = end == total_end && ms.borrow().ready_state == "ended";
                return Some(Ok((out, end, eos, sections)));
            }
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
            // JS-C325 — spec : un `SourceBuffer` `updating` →
            // `InvalidStateError` ; nouvelle durée < ancienne
            // → suppression de plage (`est_ranges` tronquées,
            // `buffered`/`seekable` MSE en héritent via
            // `mse_buffered_union`).
            let (buffers, old) = {
                let md = self.d.borrow();
                (md.buffers.clone(), md.duration)
            };
            for sbv in &buffers {
                let Some(sd) = sb_data(e, *sbv) else {
                    continue;
                };
                if sd.borrow().updating {
                    return Err(mse_err(
                        e,
                        "InvalidStateError",
                        "MediaSource.duration: buffer updating",
                    ));
                }
            }
            if old.is_finite() && n < old {
                for sbv in &buffers {
                    let Some(sd) = sb_data(e, *sbv) else {
                        continue;
                    };
                    let mut s = sd.borrow_mut();
                    s.est_ranges = s
                        .est_ranges
                        .iter()
                        .filter_map(|&(a, b)| (a < n).then_some((a, b.min(n))))
                        .collect();
                }
            }
            self.d.borrow_mut().duration = n;
            // §B-88 — la durée reflétée sur l'élément attaché
            // a changé → `durationchange` filé en tâche (un
            // set de même valeur ne tire rien). Cible
            // retrouvée par identité du `MsData` —
            // `attached_to` reste minuscule (1 attaché/URL).
            // §B-89 : scanné dans **tous** les ctx — le set
            // peut courir sous un realm parasite.
            if old.total_cmp(&n).is_ne() {
                let mid = std::iter::once(&e.mse)
                    .chain(e.parked.values().map(|c| &c.mse))
                    .find_map(|st| {
                        st.attached_to
                            .iter()
                            .find(|(mid, _)| {
                                ms_data(e, Value::Obj(**mid))
                                    .is_some_and(|m| Rc::ptr_eq(&m, &self.d))
                            })
                            .map(|(&mid, _)| mid)
                    });
                if let Some(mid) = mid {
                    mse_push(e, mid, "durationchange");
                }
            }
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
            // JS-C327 — vraies `*TrackList` (vues vivantes
            // sur `SbData.tracks`, peuplées par l'init
            // `moov` — liste vide honnête avant tout init).
            "audioTracks" | "videoTracks" | "textTracks" => {
                let list = match k {
                    "audioTracks" => "audio",
                    "videoTracks" => "video",
                    _ => "text",
                };
                Some(track_list(e, &self.d, list))
            }
            _ => None,
        })
    }

    fn set(&self, e: &mut Engine, _t: Value, k: &str, v: Value) -> JsResult<bool> {
        match k {
            // JS-C325 — spec : set pendant `updating` →
            // `InvalidStateError` (fenêtre re-entrante des
            // events synchrones).
            "mode" => {
                sb_updating_guard(e, &self.d.borrow(), "mode")?;
                let m = e.to_js_string(v)?;
                if m != "segments" && m != "sequence" {
                    return Err(e.type_error("SourceBuffer.mode: bad value"));
                }
                self.d.borrow_mut().mode = m;
                Ok(true)
            }
            "timestampOffset" => {
                sb_updating_guard(e, &self.d.borrow(), "timestampOffset")?;
                let n = e.to_number(v)?;
                if !n.is_finite() {
                    return Err(e.type_error("SourceBuffer.timestampOffset: bad value"));
                }
                let mut d = self.d.borrow_mut();
                d.timestamp_offset = n;
                // JS-C325 — offset explicite : en `sequence`
                // le prochain append part d'ici (spec) puis
                // le chaînage reprend — marqueur consommé par
                // `appendBuffer`. `last_append_t` reste :
                // la correction de span de l'estimateur vit
                // des deltas d'offsets successifs.
                d.offset_dirty = true;
                Ok(true)
            }
            "appendWindowStart" | "appendWindowEnd" => {
                sb_updating_guard(e, &self.d.borrow(), "appendWindow")?;
                let n = e.to_number(v)?;
                if n.is_nan() {
                    return Err(e.type_error("SourceBuffer.appendWindow: bad value"));
                }
                let mut d = self.d.borrow_mut();
                if k == "appendWindowStart" {
                    // Spec : `start` ≥ 0 et < `appendWindowEnd`.
                    if n < 0.0 || n >= d.append_end {
                        return Err(e.type_error("SourceBuffer.appendWindowStart: bad value"));
                    }
                    d.append_start = n;
                } else {
                    // Spec : `end` > `appendWindowStart`.
                    if n <= d.append_start {
                        return Err(e.type_error("SourceBuffer.appendWindowEnd: bad value"));
                    }
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
        // JS-C327 — wrappers piste/liste cachés : racinés
        // tant que le buffer vit (identité stable garantie).
        for t in &d.tracks {
            if let Some(o) = t.obj {
                crate::heap::push_value(out, o);
            }
        }
        for l in d.track_lists.iter().flatten() {
            crate::heap::push_value(out, *l);
        }
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

/// État mutable courant de la piste (live si présente,
/// snapshot sinon).
fn track_state(
    d: &Rc<RefCell<SbData>>,
    idx: usize,
    snap: &SbTrack,
    f: impl Fn(&SbTrack) -> Value,
) -> Value {
    match d.borrow().tracks.get(idx) {
        Some(t) => f(t),
        None => f(snap),
    }
}

/// Tire `change` sur la liste `*TrackList` du parent (spec —
/// après mutation `enabled`/`selected`/`mode`). La liste est
/// matérialisée pour porter l'événement même jamais lue.
fn fire_track_list_change(e: &mut Engine, d: &Rc<RefCell<SbData>>, list: &'static str) {
    if let Value::Obj(lid) = track_list(e, d, list) {
        let _ = fire_mse(e, lid, "change");
    }
}

impl Host for TrackHost {
    fn get(&self, e: &mut Engine, _t: Value, k: &str) -> JsResult<Option<Value>> {
        let d = &self.d;
        let idx = self.idx;
        let snap = &self.snap;
        Ok(match k {
            // Spec : `id` est DOMString — `tkhd.track_ID`
            // numérique sérialisé (players : `track.id`).
            "id" => Some(e.str_val(snap.id.to_string())),
            "kind" => Some(e.str_val(snap.kind)),
            "label" => Some(e.str_val("")),
            "language" => Some(e.str_val(snap.lang.clone())),
            "enabled" if snap.list == "audio" => {
                Some(track_state(d, idx, snap, |t| Value::Bool(t.enabled)))
            }
            "selected" if snap.list == "video" => {
                Some(track_state(d, idx, snap, |t| Value::Bool(t.selected)))
            }
            "mode" if snap.list == "text" => {
                let m = d.borrow().tracks.get(idx).map_or(snap.mode, |t| t.mode);
                Some(e.str_val(m))
            }
            // Honnêteté : pas de cues sans démuxage des
            // samples texte — `null` spec, pas une liste vide
            // mensongère (un parser qui itère `cues.length`
            // sur un stub voit 0 cues = même verdict honnête).
            "cues" | "activeCues" if snap.list == "text" => Some(Value::Null),
            "inBandMetadataTrackDispatchType" if snap.list == "text" => Some(e.str_val("")),
            // MSE spec — `track.sourceBuffer` = le buffer
            // d'origine (`null` hors MSE ; ici toujours posé).
            "sourceBuffer" => Some(self.sb),
            _ => None,
        })
    }

    fn set(&self, e: &mut Engine, _t: Value, k: &str, v: Value) -> JsResult<bool> {
        // Piste détachée : sets no-op (approximation
        // documentée — la piste n'existe plus côté média).
        if self.idx == usize::MAX {
            return Ok(true);
        }
        match (self.snap.list, k) {
            ("audio", "enabled") => {
                let on = e.to_boolean(v);
                self.d.borrow_mut().tracks[self.idx].enabled = on;
                fire_track_list_change(e, &self.d, "audio");
                Ok(true)
            }
            ("video", "selected") => {
                let on = e.to_boolean(v);
                {
                    let mut d = self.d.borrow_mut();
                    // Spec : `selected` exclusif par élément —
                    // approximation bornée documentée :
                    // exclusif **dans la videoTracks du
                    // buffer** (l'union élément est hors
                    // scope JS-C327).
                    for t in d.tracks.iter_mut().filter(|t| t.list == "video") {
                        t.selected = false;
                    }
                    if let Some(t) = d.tracks.get_mut(self.idx) {
                        t.selected = on;
                    }
                }
                fire_track_list_change(e, &self.d, "video");
                Ok(true)
            }
            ("text", "mode") => {
                let m = e.to_js_string(v)?;
                if m != "disabled" && m != "hidden" && m != "showing" {
                    return Err(e.type_error("TextTrack.mode: bad value"));
                }
                self.d.borrow_mut().tracks[self.idx].mode = match m.as_str() {
                    "hidden" => "hidden",
                    "showing" => "showing",
                    _ => "disabled",
                };
                fire_track_list_change(e, &self.d, "text");
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn trace(&self, out: &mut Vec<ObjId>) {
        crate::heap::push_value(out, self.sb);
        if let Some(o) = self.snap.obj {
            crate::heap::push_value(out, o);
        }
    }

    fn class_name(&self) -> &'static str {
        match self.snap.list {
            "audio" => "AudioTrack",
            "video" => "VideoTrack",
            _ => "TextTrack",
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl Host for TrackListHost {
    fn get(&self, e: &mut Engine, _t: Value, k: &str) -> JsResult<Option<Value>> {
        if k == "length" {
            let n = self
                .d
                .borrow()
                .tracks
                .iter()
                .filter(|t| t.list == self.list)
                .count();
            return Ok(Some(Value::Num(n as f64)));
        }
        // `selectedIndex` (spec VideoTrackList) — rang du
        // `selected` parmi les pistes de la liste, `-1`.
        if k == "selectedIndex" && self.list == "video" {
            let d = self.d.borrow();
            let sel = d
                .tracks
                .iter()
                .filter(|t| t.list == "video")
                .position(|t| t.selected)
                .map_or(-1.0, |i| i as f64);
            return Ok(Some(Value::Num(sel)));
        }
        if let Ok(i) = k.parse::<usize>() {
            let idx = {
                let d = self.d.borrow();
                d.tracks
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| t.list == self.list)
                    .nth(i)
                    .map(|(i, _)| i)
            };
            return Ok(idx.and_then(|i| track_obj(e, &self.d, i, sb_value(e, &self.d))));
        }
        Ok(None)
    }

    fn trace(&self, out: &mut Vec<ObjId>) {
        let d = self.d.borrow();
        crate::heap::push_value(out, d.ms);
        for t in &d.tracks {
            if let Some(o) = t.obj {
                crate::heap::push_value(out, o);
            }
        }
    }

    fn class_name(&self) -> &'static str {
        match self.list {
            "audio" => "AudioTrackList",
            "video" => "VideoTrackList",
            _ => "TextTrackList",
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Valeur wrapper du `SourceBuffer` — back-ref pour
/// `track.sourceBuffer` (`TrackHost.sb`). `SbData` ne la
/// stocke pas (le wrapper se trouve dans `MsData.buffers`) :
/// on la résout via le parent.
fn sb_value(e: &Engine, d: &Rc<RefCell<SbData>>) -> Value {
    let ms = d.borrow().ms;
    ms_data(e, ms).map_or(Value::Undefined, |m| {
        m.borrow()
            .buffers
            .iter()
            .copied()
            .find(|&b| sb_data(e, b).is_some_and(|s| std::rc::Rc::ptr_eq(&s, d)))
            .unwrap_or(Value::Undefined)
    })
}

/// `list.getTrackById(id)` — spec : premier match `id`
/// stringifié, `null` sinon.
fn tl_get_track_by_id(e: &mut Engine, this: Value, args: &[Value]) -> JsResult<Value> {
    let Value::Obj(id) = this else {
        return Err(e.type_error("TrackList.getTrackById: Illegal invocation"));
    };
    let Some(d) = (match e.heap.obj(id).map(|o| &o.kind) {
        Some(ObjKind::Host(h)) => h
            .clone()
            .as_any()
            .downcast_ref::<TrackListHost>()
            .map(|r| (r.d.clone(), r.list)),
        _ => None,
    }) else {
        return Err(e.type_error("TrackList.getTrackById: Illegal invocation"));
    };
    let want = match args.first() {
        Some(v) => e.to_js_string(*v)?,
        None => return Ok(Value::Null),
    };
    let idx = {
        let sd = d.0.borrow();
        sd.tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| t.list == d.1)
            .find(|(_, t)| t.id.to_string() == want)
            .map(|(i, _)| i)
    };
    Ok(idx
        .and_then(|i| track_obj(e, &d.0, i, sb_value(e, &d.0)))
        .unwrap_or(Value::Null))
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
        ("MediaError", me_illegal_ctor, me_proto(e)),
        ("AudioTrack", track_illegal_ctor, track_proto(e, "audio")),
        ("VideoTrack", track_illegal_ctor, track_proto(e, "video")),
        ("TextTrack", track_illegal_ctor, track_proto(e, "text")),
        (
            "AudioTrackList",
            track_illegal_ctor,
            track_list_proto(e, "audio"),
        ),
        (
            "VideoTrackList",
            track_illegal_ctor,
            track_list_proto(e, "video"),
        ),
        (
            "TextTrackList",
            track_illegal_ctor,
            track_list_proto(e, "text"),
        ),
    ] {
        let fv = e.native_fn(name, ctor);
        if let Value::Obj(fid) = fv {
            e.set_own(fid, "prototype", Value::Obj(proto), false);
            // `MediaError` porte ses constantes spec sur le
            // constructeur (`MediaError.MEDIA_ERR_DECODE`).
            if name == "MediaError" {
                for (cname, code) in MEDIA_ERR_CONSTANTS {
                    e.set_own(fid, cname, Value::Num(code), false);
                }
            }
        }
        e.set_own(g, name, fv, false);
    }
}

// ------------------------------------------------------------------
// Tests §B-89 — routage des events/états par ctx propriétaire
// ------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::Dom;

    const PAGE: &str = r#"<html><body><video id="v"></video></body></html>"#;

    fn engine_dom() -> (Engine, Rc<RefCell<Dom>>) {
        let mut e = Engine::new();
        let dom = Rc::new(RefCell::new(Dom::from_html(PAGE)));
        e.install_dom(&dom);
        (e, dom)
    }

    /// §B-89 — un host-trait (`MsHost::set`) tournant sous un
    /// realm parasite ne doit pas orpheliner les events filés :
    /// sans routage, `durationchange` atterrissait dans la file
    /// du parasite → `attached_to`/`dom_ctx` du mauvais
    /// document → drop silencieux (vol 635).
    #[test]
    fn duration_set_under_foreign_ctx_still_delivers() {
        let (mut e, _dom) = engine_dom();
        let msv = e
            .eval(
                "var ms=new MediaSource(),L=[];\
                 var v=document.getElementById('v');\
                 v.addEventListener('durationchange',function(){L.push('dc')});\
                 v.src=URL.createObjectURL(ms);ms",
            )
            .expect("arm");
        e.eval("0").expect("drain attach");
        // Realm nu courant — une pompe session qui laisserait
        // `cur_ctx` sur un realm de traitement produit le même
        // poison. `MsHost::set` ne switche pas de ctx.
        let _c1 = e.context_new("about:blank", None);
        e.set_prop_value(msv, "duration", Value::Num(5.0))
            .expect("set sous ctx parasite");
        e.eval("0").expect("drain sous parasite");
        e.context_switch(0);
        let out = e.eval("L.join(',')").expect("read");
        assert_eq!(e.describe(out), "dc");
    }

    /// §B-89 — les accessors élément (`paused`/`readyState`/
    /// `is_attached`/`pause`) résolvent l'état dans le ctx de
    /// l'attache même appelés sous un realm parasite.
    #[test]
    fn el_state_resolves_under_foreign_ctx() {
        let (mut e, dom) = engine_dom();
        e.eval(
            "var ms=new MediaSource();var v=document.getElementById('v');\
             v.src=URL.createObjectURL(ms);var R='pending';\
             v.play().then(function(){R='resolved'},function(){R='rejected'});0",
        )
        .expect("arm+pend");
        let node = dom.borrow().by_id("v").expect("v node");
        let _c1 = e.context_new("about:blank", None);
        // Sous le bug : `el_play`/`el_ladder` du parasite vides
        // → `is_attached`=false, `paused`=true (intention
        // perdue). Après fix : le vrai état de l'attache.
        assert!(e.mse_el_is_attached(node));
        assert!(!e.mse_el_paused(node));
        assert_eq!(e.mse_el_ready(node), 0);
        // `pause()` sous parasite : l'intention retombe dans le
        // ctx de l'attache, `pause` routé vers sa file.
        e.mse_el_pause(node);
        assert!(e.mse_el_paused(node));
        e.context_switch(0);
    }

    /// §B-89 — `load()` (`mse_el_refire_loadstart`) sous realm
    /// parasite : `loadstart` est routé vers la file du ctx de
    /// l'attache et servi à l'élément du vrai document.
    #[test]
    fn loadstart_refire_under_foreign_ctx_routes() {
        let (mut e, dom) = engine_dom();
        e.eval(
            "var ms=new MediaSource(),L=[];var v=document.getElementById('v');\
             v.addEventListener('loadstart',function(){L.push('ls')});\
             v.src=URL.createObjectURL(ms);0",
        )
        .expect("arm");
        e.eval("0").expect("drain");
        let node = dom.borrow().by_id("v").expect("v");
        let _c1 = e.context_new("about:blank", None);
        e.mse_el_refire_loadstart(node);
        e.eval("0").expect("drain sous parasite");
        e.context_switch(0);
        let out = e.eval("L.join(',')").expect("read");
        assert_eq!(e.describe(out), "ls,ls");
    }

    /// §B-89 — `appendBuffer` invoqué sous un realm parasite :
    /// `progress` et l'échelle (`loadedmetadata`) sont routés
    /// vers le ctx de l'attache — le mur vol 635
    /// (`sourceopen`/échelle avalés) se joue sur ce chemin.
    #[test]
    fn append_under_foreign_ctx_routes_element_events() {
        let (mut e, _dom) = engine_dom();
        let sbv = e
            .eval(
                "var ms=new MediaSource(),L=[];\
                 var v=document.getElementById('v');\
                 v.addEventListener('progress',function(){L.push('p')});\
                 v.addEventListener('loadedmetadata',function(){L.push('lm')});\
                 ms.addEventListener('sourceopen',function(){L.push('so')});\
                 v.src=URL.createObjectURL(ms);\
                 var sb=ms.addSourceBuffer('video/mp4');sb",
            )
            .expect("arm");
        e.eval("0").expect("drain attach");
        // `appendBuffer` bound — `call_native_ctx` ne trouve
        // pas de ctx sur un `this` non-global → tourne sous
        // `cur_ctx` : le parasite. L'init `moov` (payload non
        // vide pour `walk_boxes`) monte `loadedmetadata`.
        let append = e.get_prop_value(sbv, "appendBuffer").expect("method");
        let bytes = b"\x00\x00\x00\x10moov\x00\x00\x00\x08free";
        let arr = format!("new Uint8Array({bytes:?}).buffer");
        let ab = e.eval(&arr).expect("buffer");
        let _c1 = e.context_new("about:blank", None);
        e.call_value(append, sbv, &[ab])
            .expect("append sous parasite");
        e.eval("0").expect("drain sous parasite");
        e.context_switch(0);
        let out = e.eval("L.join(',')").expect("read");
        // Sous le bug : 'so' seul (progress/lm avalés dans la
        // file parasite). Après fix : routés.
        assert_eq!(e.describe(out), "so,lm,p");
    }
}
