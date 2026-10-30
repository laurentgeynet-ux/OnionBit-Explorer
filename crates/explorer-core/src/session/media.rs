// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Seam `MediaSurface` (étape 71b, ADR-0024) : détection des
//! éléments `<video>`/`<audio>` lisibles sur le boxtree posé,
//! allocation des identifiants `media: u64`, émission du
//! cycle de vie (`MediaOffered`/`MediaMoved`/`MediaGone`) et
//! état machine claim/state côté session.
//!
//! Cycle de vie : `refresh_media` diffe l'état scanné contre
//! les offres vivantes à chaque point de cohérence (commit de
//! navigation, apply JS repeint) — la correspondance est
//! **positionnelle** (`kind` + `url` dans l'ordre du
//! document) car le re-parse de synchro DOM régénère les
//! `NodeId`. Une surface qui survit conserve son id (et son
//! claim) ; les autres meurent (`MediaGone`) et les nouvelles
//! sont offertes avec un id frais. `close_media` tue tout à
//! la mort de la page (gel bfcache, restauration, fermeture
//! d'onglet) — une page gelée ne transporte pas ses offres,
//! la restauration ré-offre avec des ids neufs (contrat).
//!
//! La géométrie est en **px CSS de document** — l'hôte
//! soustrait `scroll_y` pour son overlay. `MediaMoved` n'est
//! émis que depuis `refresh_media`, dont les appels sont au
//! plus un par commit/apply : la borne « une émission par
//! frame produite » du contrat est tenue par construction.
//!
//! Sans claim, l'élément reste un stub ADR-0023 (clause
//! honnêteté) — la délégation JS (`MediaCommand`, props
//! refletées) arrive à l'étape 71c.

use explorer_dom::NodeId;

use super::*;

/// Surface média vivante d'un onglet — offre émise, état
/// claim + dernier rapport hôte. Ordre du vecteur = ordre
/// du document (correspondance positionnelle du refresh).
#[allow(clippy::struct_excessive_bools)] // drapeaux d'offre orthogonaux.
pub(crate) struct MediaSurface {
    /// Identifiant émis dans `MediaOffered`.
    pub(crate) id: u64,
    /// `video` ou `audio`.
    pub(crate) kind: MediaKind,
    /// Rectangle courant (px CSS document).
    pub(crate) rect: MediaRect,
    /// Source résolue (URL absolue).
    pub(crate) url: String,
    /// Type MIME si connu.
    pub(crate) mime: Option<String>,
    /// UI native attendue.
    pub(crate) controls: bool,
    /// Lecture automatique demandée.
    pub(crate) autoplay: bool,
    /// Attribut `loop` présent (`<video loop>` — 139k).
    pub(crate) looping: bool,
    /// Codecs déclarés par l'hôte claimé (`None` = stub).
    pub(crate) claim: Option<Vec<String>>,
    /// Claim enregistré mais pas encore livré au moteur —
    /// `set_media_claim` est sauté quand `entry.js` est
    /// détenu (vol 176-bis : codecs jamais déclarés →
    /// `isTypeSupported` restait `""` après un claim exact).
    /// Livré au prochain contact moteur-présent.
    pub(crate) claim_pending: bool,
    /// Dernier `state` rapporté par l'hôte.
    pub(crate) report: Option<MediaReport>,
    /// Chemin `(tag, index)` DOM de l'élément — résolution
    /// unique du nœud miroir à claim/release/report (étape
    /// 71c). Maintenu à jour au refresh (l'élément peut avoir
    /// bougé dans l'arbre entre deux applies).
    pub(crate) path: Vec<(String, u32)>,
    /// Source `blob:` `MediaSource` (MSE-2, ADR-0029) —
    /// l'hôte pull `GET …/segments` plutôt que `url`.
    pub(crate) mse: bool,
    /// MIME des `SourceBuffer` publiés dans l'offre
    /// (`?buffer=N` indexe ce tableau — P-53 `open_mse_multi`).
    pub(crate) mse_buffers: Vec<String>,
}

/// Résultat d'un pull de segments MSE (§B-63) — `Pending`
/// distingue l'absence transitoire de moteur de la mort de
/// surface : `410` doit rester terminal.
pub enum MsePull {
    /// Segments drainés : octets, curseur absolu, fin de flux.
    Ready(Vec<u8>, u64, bool),
    /// Moteur momentanément absent (fenêtre pompe) — re-sonder.
    Pending,
    /// Surface morte / non-MSE / index de buffer hors rang.
    Dead,
}

/// Surface morte au `refresh_media` (71c) — `path` et
/// `claimed` servent à purger le claim du nœud miroir s'il
/// survit à l'apply (ex. `src` retirée par JS : l'offre
/// meurt, l'élément reste — `play()` retombe sur le rejet
/// stub, jamais de promesse pendue sans réponse).
struct GoneSurface {
    /// Identifiant de la surface morte.
    id: u64,
    /// Chemin `(tag, index)` vers le nœud miroir.
    path: Vec<(String, u32)>,
    /// URL média — identité de repli du unclaim si le
    /// miroir a divergé (même rôle qu'au claim, MSE-7).
    url: String,
    /// La surface était claimée par l'hôte.
    claimed: bool,
}

/// Élément média détecté sur le boxtree posé — image fixe
/// de l'offre avant allocation d'id.
/// Record d'attributs d'élément — les booléens sont des
/// présences HTML orthogonales (`controls`/`autoplay`/
/// `loop`/`mse`), pas une machine à états.
#[allow(clippy::struct_excessive_bools)]
struct ScannedMedia {
    /// `video` ou `audio`.
    kind: MediaKind,
    /// Rectangle de la boîte (px CSS document ; 0×0 pour
    /// un `audio` sans `controls`).
    rect: MediaRect,
    /// Source résolue (URL absolue).
    url: String,
    /// Type MIME si connu.
    mime: Option<String>,
    /// Attribut `controls` présent.
    controls: bool,
    /// Attribut `autoplay` présent.
    autoplay: bool,
    /// Attribut `loop` présent.
    looping: bool,
    /// Chemin `(tag, index)` de l'élément (71c — miroir JS).
    path: Vec<(String, u32)>,
    /// `url` = object URL `MediaSource` enregistré côté
    /// moteur (MSE-2, ADR-0029) — remplie au refresh via
    /// `JsEngine::is_media_source`.
    mse: bool,
    /// MIME des `SourceBuffer` (index `?buffer=N` — offre
    /// `mseBuffers` pour `open_mse_multi`).
    mse_buffers: Vec<String>,
}

/// MIME média déduit du suffixe d'URL (contrat : `type` >
/// suffixe > `None`). Volontairement limité aux types
/// qu'un hôte v1 saurait jouer — pas un sniffer général.
fn media_mime_from_url(url: &Url) -> Option<String> {
    let path = url.path().to_ascii_lowercase();
    let ext = path.rsplit('.').next()?;
    let mime = match ext {
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "ogv" => "video/ogg",
        "mov" => "video/quicktime",
        "m3u8" => "application/vnd.apple.mpegurl",
        "mpd" => "application/dash+xml",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        "ogg" | "oga" | "opus" => "audio/ogg",
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        _ => return None,
    };
    Some(mime.to_string())
}

/// Attribut booléen présent (`controls`, `autoplay`) —
/// présence compte, valeur ignorée (HTML).
fn has_bool_attr(dom: &Dom, n: NodeId, name: &str) -> bool {
    dom.attribute(n, name).ok().flatten().is_some()
}

/// Source lisible d'un élément média : attribut `src`
/// prioritaire, sinon premier `<source src>` enfant direct
/// (spéc HTML — `src` et `<source>` ne se cumulent pas).
/// Renvoie `(src_brut, type_déclaré)`.
fn media_source(dom: &Dom, n: NodeId) -> Option<(String, Option<String>)> {
    let attr = |name: &str| dom.attribute(n, name).ok().flatten();
    if let Some(src) = attr("src").filter(|s| !s.is_empty()) {
        return Some((src, attr("type")));
    }
    for c in dom.children(n).unwrap_or_default() {
        if !dom.tag_name(c).is_ok_and(|t| t == "source") {
            continue;
        }
        if let Ok(Some(src)) = dom.attribute(c, "src") {
            if !src.is_empty() {
                let ty = dom.attribute(c, "type").ok().flatten();
                return Some((src, ty));
            }
        }
    }
    None
}

/// Éléments média **lisibles** du document posé — parcours
/// du **DOM** (ordre du document ; un `audio` sans
/// `controls` est `display:none` au style UA et n'a pas de
/// boîte — il doit quand même être offert à 0×0, contrat)
/// joint à la carte `NodeId → rect` du boxtree posé. Un
/// élément média sans boîte (display:none explicite) reste
/// offert à 0×0 : l'hôte n'attache que le pipeline. Pas de
/// source → pas d'offre (honnêteté ADR-0023 conservée).
fn scan_media(tree: &LayoutBox, dom: &Dom, base: &Url) -> Vec<ScannedMedia> {
    // Rects par nœud source — une passe itérative sur
    // l'arbre posé (un élément = au plus une boîte ici).
    let mut rects: HashMap<NodeId, MediaRect> = HashMap::new();
    let mut stack = vec![tree];
    while let Some(bx) = stack.pop() {
        if let Some(n) = bx.node {
            rects.entry(n).or_insert(MediaRect {
                x: bx.rect.x,
                y: bx.rect.y,
                w: bx.rect.width,
                h: bx.rect.height,
            });
        }
        stack.extend(bx.children.iter().rev());
    }
    let zero = MediaRect {
        x: 0.0,
        y: 0.0,
        w: 0.0,
        h: 0.0,
    };
    let mut out = Vec::new();
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        let kind = match dom.tag_name(n) {
            Ok("video") => Some(MediaKind::Video),
            Ok("audio") => Some(MediaKind::Audio),
            _ => None,
        };
        if let Some(kind) = kind {
            if let Some((src, ty)) = media_source(dom, n) {
                // `blob:` object URL — le resolveur peut ne
                // pas connaître le schéma opaque ; la chaîne
                // brute suffit (registre moteur, MSE-2).
                let parsed = Url::resolve(base, &src).ok();
                let url_s = parsed
                    .as_ref()
                    .map(ToString::to_string)
                    .or_else(|| src.starts_with("blob:").then(|| src.clone()));
                if let Some(url_s) = url_s {
                    let controls = has_bool_attr(dom, n, "controls");
                    // `audio` sans UI : offre 0×0 quelle que
                    // soit la boîte — rien à positionner.
                    let rect = if kind == MediaKind::Audio && !controls {
                        zero
                    } else {
                        rects.get(&n).copied().unwrap_or(zero)
                    };
                    out.push(ScannedMedia {
                        kind,
                        rect,
                        mime: ty.or_else(|| parsed.as_ref().and_then(media_mime_from_url)),
                        url: url_s,
                        controls,
                        autoplay: has_bool_attr(dom, n, "autoplay"),
                        looping: has_bool_attr(dom, n, "loop"),
                        path: element_path(dom, n).unwrap_or_default(),
                        mse: false,
                        mse_buffers: Vec::new(),
                    });
                }
            }
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
        }
    }
    out
}

/// Marque `mse`/`mse_buffers` des surfaces scannées depuis le
/// registre moteur — à n'appeler que moteur présent : `js` absent
/// (fenêtre pompe) ne doit rien déduire (§B-63 adjacent, vol 177).
fn mark_mse_surfaces(scanned: &mut [ScannedMedia], js: &mut Option<Box<dyn JsEngine>>) {
    for s in scanned.iter_mut() {
        s.mse =
            s.url.starts_with("blob:") && js.as_mut().is_some_and(|e| e.is_media_source(&s.url));
        // Les `SourceBuffer` déjà ajoutés sont publiés
        // dans l'offre (`?buffer=N` les indexe — P-53).
        if s.mse {
            s.mse_buffers = js
                .as_mut()
                .map(|e| e.mse_buffers(&s.url))
                .unwrap_or_default();
        }
    }
}

/// Sonde MSE au contact (vol 176-bis) — une surface `blob:`
/// scannée pendant que le moteur était détenu reste
/// `mse:false` inconnu tant que la tempête dure (les applies
/// scannent presque toujours à l'aveugle). List/pull/claim
/// re-sondent quand le moteur est présent et mettent l'offre
/// à jour en place — `blob:` non-MSE reste `false`, moteur
/// absent → rien déduit (Pending/unknown honnête).
fn lazy_mark_mse(m: &mut MediaSurface, js: &mut Option<Box<dyn JsEngine>>) {
    if m.mse || !m.url.starts_with("blob:") {
        return;
    }
    if let Some(e) = js.as_mut() {
        if e.is_media_source(&m.url) {
            m.mse = true;
            m.mse_buffers = e.mse_buffers(&m.url);
        }
    }
}

/// Livre au moteur le claim en attente (claim/release posé
/// pendant une fenêtre moteur-absent) — idempotent : le
/// drapeau tombe à la première livraison réussie.
fn sync_claim_to_engine(m: &mut MediaSurface, js: &mut Option<Box<dyn JsEngine>>) {
    if m.claim_pending {
        if let Some(e) = js.as_mut() {
            e.set_media_claim(&m.path, &m.url, m.id, m.claim.clone());
            m.claim_pending = false;
        }
    }
}

impl Session {
    /// Recalcule les surfaces média de l'onglet au point de
    /// cohérence courant (post-commit, post-apply repeint) :
    /// retenue positionnelle (`kind`+`url` ordre document),
    /// `MediaGone` pour les disparues, `MediaMoved` pour les
    /// rect changés, `MediaOffered` pour les nouvelles. No-op
    /// si l'arbre/DOM ne sont pas un couple cohérent (`tree`
    /// absent pendant la fenêtre nav → relayout).
    pub(crate) fn refresh_media(&mut self, tab: TabId) {
        let mut gone: Vec<GoneSurface> = Vec::new();
        let mut moved: Vec<(u64, MediaRect)> = Vec::new();
        let mut offered: Vec<MediaSurface> = Vec::new();
        {
            let Some(entry) = self.tabs.get_mut(&tab) else {
                return;
            };
            let (Some(dom), Some(tree), Some(base)) =
                (entry.dom.as_ref(), entry.tree.as_ref(), entry.nav_base())
            else {
                return;
            };
            let mut scanned = scan_media(tree, dom, base);
            // Marquage MSE (MSE-2, ADR-0029) — le registre
            // vit dans le moteur : `blob:` + `is_media_source`
            // → l'offre route vers `GET …/segments`.
            // §B-63 adjacent : `entry.js` est `None` tant que la
            // pompe détient le moteur — le scan marquait alors
            // `mse:false`/`mseBuffers:[]` à tort (youtube : la
            // tempête §B-41 rendait le drapeau mort en
            // permanence, l'offre MSE invisible pour l'hôte).
            // Moteur absent → on ne déduit rien, les surfaces
            // retenues conservent leur état précédent.
            let js_present = entry.js.is_some();
            if js_present {
                mark_mse_surfaces(&mut scanned, &mut entry.js);
            }
            let old = std::mem::take(&mut entry.media);
            let mut matched = vec![false; scanned.len()];
            // Retenue : chaque ancienne offre cherche la
            // première position non appariée de même
            // (`kind`,`url`) — l'insertion/retrait JS en tête
            // décale les index sans tuer les survivantes.
            let mut retained: Vec<Option<MediaSurface>> = Vec::new();
            retained.resize_with(scanned.len(), || None);
            for mut o in old {
                if let Some(pos) = scanned
                    .iter()
                    .enumerate()
                    .position(|(i, s)| !matched[i] && s.kind == o.kind && s.url == o.url)
                {
                    matched[pos] = true;
                    let s = &scanned[pos];
                    if s.rect != o.rect {
                        o.rect = s.rect;
                        moved.push((o.id, s.rect));
                    }
                    o.mime.clone_from(&s.mime);
                    o.controls = s.controls;
                    o.autoplay = s.autoplay;
                    o.looping = s.looping;
                    o.path.clone_from(&s.path);
                    // Moteur absent → drapeau/buffers retenus
                    // (scan à l'aveugle : ne rien dégrader) ;
                    // moteur présent → rafraîchir, y compris
                    // `mse_buffers` — les `SourceBuffer` ajoutés
                    // APRÈS l'offre doivent la compléter (vol
                    // 177 : `mseBuffers` restait `[]` à vie).
                    if js_present {
                        o.mse = s.mse;
                        o.mse_buffers.clone_from(&s.mse_buffers);
                    }
                    retained[pos] = Some(o);
                } else {
                    gone.push(GoneSurface {
                        id: o.id,
                        path: o.path.clone(),
                        url: o.url.clone(),
                        claimed: o.claim.is_some(),
                    });
                }
            }
            let mut media = Vec::with_capacity(scanned.len());
            for (i, s) in scanned.into_iter().enumerate() {
                if let Some(o) = retained[i].take() {
                    media.push(o);
                } else {
                    let id = self.next_media_id;
                    self.next_media_id += 1;
                    let surf = MediaSurface {
                        id,
                        kind: s.kind,
                        rect: s.rect,
                        url: s.url,
                        mime: s.mime,
                        controls: s.controls,
                        autoplay: s.autoplay,
                        looping: s.looping,
                        claim: None,
                        claim_pending: false,
                        report: None,
                        path: s.path,
                        mse: s.mse,
                        mse_buffers: s.mse_buffers,
                    };
                    offered.push(MediaSurface {
                        id,
                        kind: surf.kind,
                        rect: surf.rect,
                        mse_buffers: surf.mse_buffers.clone(),
                        url: surf.url.clone(),
                        mime: surf.mime.clone(),
                        controls: surf.controls,
                        autoplay: surf.autoplay,
                        looping: surf.looping,
                        claim: None,
                        claim_pending: false,
                        report: None,
                        path: surf.path.clone(),
                        mse: surf.mse,
                    });
                    media.push(surf);
                }
            }
            entry.media = media;
        }
        self.emit_media_events(tab, gone, moved, offered);
    }

    /// Émission du diff de `refresh_media` (hors emprunt de
    /// l'entrée) : purge des claims miroir des surfaces
    /// mortes puis `MediaGone`, `MediaMoved`, `MediaOffered`.
    fn emit_media_events(
        &mut self,
        tab: TabId,
        gone: Vec<GoneSurface>,
        moved: Vec<(u64, MediaRect)>,
        offered: Vec<MediaSurface>,
    ) {
        for g in gone {
            if g.claimed {
                if let Some(eng) = self.tabs.get_mut(&tab).and_then(|e| e.js.as_mut()) {
                    eng.set_media_claim(&g.path, &g.url, g.id, None);
                }
            }
            let _ = self.events.send(CoreEvent::MediaGone { tab, media: g.id });
        }
        for (id, rect) in moved {
            let _ = self.events.send(CoreEvent::MediaMoved {
                tab,
                media: id,
                rect,
            });
        }
        for s in offered {
            // 139b — tout flux de lecture http(s) passe par le
            // proxy local (même câblage `local_proxy` de
            // référence) ; `file:` et autres schémas restent
            // directs (le proxy ne sait servir que du http) ;
            // proxy pas encore prêt → URL directe (repli).
            let url = if s.url.starts_with("http") {
                self.media_proxy
                    .get()
                    .map_or_else(|| s.url.clone(), |p| p.url(&s.url))
            } else {
                s.url.clone()
            };
            let _ = self.events.send(CoreEvent::MediaOffered {
                tab,
                media: s.id,
                kind: s.kind,
                rect: s.rect,
                url,
                mime: s.mime,
                controls: s.controls,
                autoplay: s.autoplay,
                mse: s.mse,
                looping: s.looping,
                mse_buffers: s.mse_buffers.clone(),
            });
        }
    }

    /// Tue toutes les surfaces de l'onglet (`MediaGone`
    /// chacune) — mort de page (gel bfcache, navigation,
    /// restauration) ou fermeture d'onglet. Une page gelée
    /// ne transporte pas ses offres : la restauration
    /// ré-offre avec des ids neufs (contrat `MediaSurface`).
    pub(crate) fn close_media(&mut self, tab: TabId) {
        let ids: Vec<u64> = self
            .tabs
            .get_mut(&tab)
            .map(|e| {
                std::mem::take(&mut e.media)
                    .into_iter()
                    .map(|m| m.id)
                    .collect()
            })
            .unwrap_or_default();
        for id in ids {
            let _ = self.events.send(CoreEvent::MediaGone { tab, media: id });
        }
    }

    /// Surfaces média vivantes de l'onglet (debug REST,
    /// parité FFI future). `&mut` : la lecture re-sonde les
    /// `blob:` scannées à l'aveugle quand le moteur est
    /// présent (`lazy_mark_mse`, vol 176-bis).
    pub fn media_list(&mut self, tab: TabId) -> Vec<MediaOfferInfo> {
        self.tabs
            .get_mut(&tab)
            .map(|e| {
                for m in &mut e.media {
                    lazy_mark_mse(m, &mut e.js);
                    sync_claim_to_engine(m, &mut e.js);
                }
                e.media
                    .iter()
                    .map(|m| MediaOfferInfo {
                        id: m.id,
                        tab,
                        kind: m.kind,
                        rect: m.rect,
                        url: m.url.clone(),
                        mime: m.mime.clone(),
                        controls: m.controls,
                        autoplay: m.autoplay,
                        looping: m.looping,
                        claim: m.claim.clone(),
                        state: m.report.clone(),
                        mse: m.mse,
                        mse_buffers: m.mse_buffers.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Pull de segments MSE (MSE-3, ADR-0029) — drain FIFO
    /// du `SourceBuffer` `buffer` de la `MediaSource` attachée
    /// à la surface `mid`. Tri-état : `Ready` = octets +
    /// curseur + eos ; `Pending` = surface MSE vivante mais
    /// moteur momentanément absent (fenêtre pompe — §B-63 :
    /// un `410` terminal tuait le `MseSource` sur un état
    /// transitoire, l'hôte doit re-sonder) ; `Dead` = surface
    /// morte, non-MSE ou buffer hors rang → 410.
    pub fn media_segments_pull(
        &mut self,
        tab: TabId,
        mid: u64,
        buffer: usize,
        max: usize,
    ) -> MsePull {
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return MsePull::Dead;
        };
        let Some(m) = entry.media.iter_mut().find(|m| m.id == mid) else {
            return MsePull::Dead;
        };
        if !m.mse {
            // Surface scannée à l'aveugle (fenêtre pompe) :
            // le contact re-sonde — `blob:` vraiment non-MSE
            // → Dead ; `blob:` + moteur encore absent →
            // Pending, inconnu honnête (§B-63, jamais de 410
            // sur un état re-sondable).
            lazy_mark_mse(m, &mut entry.js);
            if !m.mse {
                if m.url.starts_with("blob:") && entry.js.is_none() {
                    return MsePull::Pending;
                }
                return MsePull::Dead;
            }
        }
        let Some(eng) = entry.js.as_mut() else {
            return MsePull::Pending;
        };
        match eng.mse_pull(&m.url.clone(), buffer, max) {
            Some((data, cursor, eos)) => MsePull::Ready(data, cursor, eos),
            None => MsePull::Dead,
        }
    }

    /// Claim d'une surface par l'hôte (premier arrivé gagne)
    /// — `codecs` = capacités déclarées (alimentent
    /// `canPlayType`). Le miroir JS est marqué claimé : les
    /// props média reflètent alors les rapports hôte et les
    /// commandes page partent vers l'hôte (étape 71c).
    /// `Dead` → `410`, `Taken` → `409`.
    ///
    /// # Errors
    /// [`MediaError::Dead`] id inconnu ou mort ;
    /// [`MediaError::Taken`] déjà claimée.
    pub fn media_claim(
        &mut self,
        tab: TabId,
        media: u64,
        codecs: Vec<String>,
    ) -> Result<(), MediaError> {
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return Err(MediaError::Dead);
        };
        let Some(m) = entry.media.iter_mut().find(|m| m.id == media) else {
            return Err(MediaError::Dead);
        };
        if m.claim.is_some() {
            return Err(MediaError::Taken);
        }
        // Claim sur une `blob:` scannée aveugle : moteur
        // présent → l'offre se corrige au contact (les
        // pulls/list suivants verront `mse:true`).
        lazy_mark_mse(m, &mut entry.js);
        m.claim = Some(codecs);
        // Moteur détenu → la livraison est différée au
        // prochain contact (`claim_pending`), jamais perdue.
        m.claim_pending = true;
        sync_claim_to_engine(m, &mut entry.js);
        Ok(())
    }

    /// L'hôte se détache — la surface retombe sur les stubs
    /// ADR-0023 (dernier rapport conservé, inerte ; promesses
    /// `play()` pendantes rejetées côté miroir).
    ///
    /// # Errors
    /// [`MediaError::Dead`] id inconnu ou mort.
    pub fn media_release(&mut self, tab: TabId, media: u64) -> Result<(), MediaError> {
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return Err(MediaError::Dead);
        };
        let Some(m) = entry.media.iter_mut().find(|m| m.id == media) else {
            return Err(MediaError::Dead);
        };
        m.claim = None;
        m.claim_pending = true;
        sync_claim_to_engine(m, &mut entry.js);
        Ok(())
    }

    /// Rapport `state` hôte → moteur (battement borné ≤ 4 Hz
    /// côté hôte). Stocké sur la surface **et** relu au miroir
    /// JS : props média refletées, promesses `play()`
    /// tranchées, events DOM tirés (étape 71c). Exige un
    /// claim actif : un rapport sans claim est sans effet.
    ///
    /// # Errors
    /// [`MediaError::Dead`] id inconnu ou mort ;
    /// [`MediaError::NotClaimed`] surface non claimée.
    pub fn media_report(
        &mut self,
        tab: TabId,
        media: u64,
        report: MediaReport,
    ) -> Result<(), MediaError> {
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return Err(MediaError::Dead);
        };
        let Some(m) = entry.media.iter_mut().find(|m| m.id == media) else {
            return Err(MediaError::Dead);
        };
        if m.claim.is_none() {
            return Err(MediaError::NotClaimed);
        }
        if let Some(eng) = entry.js.as_mut() {
            eng.deliver_media_state(
                &m.path,
                m.id,
                explorer_js::JsMediaReport {
                    position_ms: report.position_ms,
                    duration_ms: report.duration_ms,
                    buffered_ms: report.buffered_ms,
                    state: match report.state {
                        MediaHostState::Paused => explorer_js::JsMediaHostState::Paused,
                        MediaHostState::Ended => explorer_js::JsMediaHostState::Ended,
                        MediaHostState::Error => explorer_js::JsMediaHostState::Error,
                        // `waiting` reste « playing » côté JS —
                        // le DOM n'a pas d'état buffering dans
                        // la couture `MediaState`.
                        MediaHostState::Playing | MediaHostState::Buffering => {
                            explorer_js::JsMediaHostState::Playing
                        }
                    },
                    error: report.error.clone(),
                },
            );
        }
        m.report = Some(report);
        Ok(())
    }

    /// Draine les ordres média émis par la page sur éléments
    /// claimés (étape 71c) et émet `CoreEvent::MediaCommand`
    /// pour chacun dont la surface vit encore — les commandes
    /// d'une surface morte tombent (l'hôte a déjà `MediaGone`).
    /// Appelée au point de synchro post-script (`after_script`)
    /// au même titre que `drain_script_net`.
    pub(crate) fn drain_media_commands(&mut self, tab: TabId) {
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return;
        };
        let Some(eng) = entry.js.as_mut() else {
            return;
        };
        let cmds = eng.drain_media_commands();
        for c in cmds {
            if entry.media.iter().any(|m| m.id == c.media) {
                let cmd = match c.cmd {
                    explorer_js::JsMediaCmd::Play => MediaCmd::Play,
                    explorer_js::JsMediaCmd::Pause => MediaCmd::Pause,
                    explorer_js::JsMediaCmd::Seek { ms } => MediaCmd::Seek { ms },
                    explorer_js::JsMediaCmd::Volume { v } => MediaCmd::Volume { v },
                    explorer_js::JsMediaCmd::Muted { on } => MediaCmd::Muted { on },
                    explorer_js::JsMediaCmd::Rate { r } => MediaCmd::Rate { r },
                };
                let _ = self.events.send(CoreEvent::MediaCommand {
                    tab,
                    media: c.media,
                    cmd,
                });
            }
        }
    }
}

/// `MediaHostState` → chaîne du contrat (`playing`/`paused`/
/// `ended`/`error` — le même vocabulaire en SSE, REST et
/// FFI ; étape 71e).
#[must_use]
pub fn media_host_state_str(s: MediaHostState) -> &'static str {
    match s {
        MediaHostState::Playing => "playing",
        MediaHostState::Paused => "paused",
        MediaHostState::Ended => "ended",
        MediaHostState::Error => "error",
        MediaHostState::Buffering => "buffering",
    }
}

/// Sérialise une surface média (fiche `GET …/media` du
/// contrat `MediaSurface`) — la forme JSON appartient au
/// contrat, pas au transport : REST et FFI la partagent.
#[must_use]
pub fn media_offer_json(m: &MediaOfferInfo) -> serde_json::Value {
    serde_json::json!({
        "id": m.id, "tab": m.tab,
        "kind": match m.kind {
            MediaKind::Video => "video",
            MediaKind::Audio => "audio",
        },
        "rect": { "x": m.rect.x, "y": m.rect.y, "w": m.rect.w, "h": m.rect.h },
        "url": m.url, "mime": m.mime,
        "mse": m.mse, "mseBuffers": m.mse_buffers,
        "controls": m.controls, "autoplay": m.autoplay, "loop": m.looping,
        "claimed": m.claim.is_some(),
        "codecs": m.claim,
        "state": m.state.as_ref().map(|r| serde_json::json!({
            "position_ms": r.position_ms,
            "duration_ms": r.duration_ms,
            "buffered_ms": r.buffered_ms,
            "state": media_host_state_str(r.state),
            "error": r.error,
        })),
    })
}

/// `CoreEvent` média → `(nom SSE, charge JSON)` — les deux
/// transposent à l'identique en SSE `data:` et en ligne
/// NDJSON FFI. `None` pour toute variante hors média.
#[must_use]
pub fn media_event_json(ev: &CoreEvent) -> Option<(&'static str, String)> {
    let out = match ev {
        CoreEvent::MediaOffered {
            tab,
            media,
            kind,
            rect,
            url,
            mime,
            controls,
            autoplay,
            mse,
            looping,
            mse_buffers,
        } => (
            "media_offered",
            serde_json::json!({
                "tab": tab, "media": media,
                "kind": match kind {
                    MediaKind::Video => "video",
                    MediaKind::Audio => "audio",
                },
                "rect": { "x": rect.x, "y": rect.y, "w": rect.w, "h": rect.h },
                "url": url, "mime": mime, "mse": mse,
                "controls": controls, "autoplay": autoplay,
                "loop": looping, "mseBuffers": mse_buffers,
            })
            .to_string(),
        ),
        CoreEvent::MediaMoved { tab, media, rect } => (
            "media_moved",
            serde_json::json!({
                "tab": tab, "media": media,
                "rect": { "x": rect.x, "y": rect.y, "w": rect.w, "h": rect.h },
            })
            .to_string(),
        ),
        CoreEvent::MediaGone { tab, media } => (
            "media_gone",
            serde_json::json!({ "tab": tab, "media": media }).to_string(),
        ),
        CoreEvent::MediaCommand { tab, media, cmd } => (
            "media_command",
            serde_json::json!({
                "tab": tab, "media": media,
                "cmd": match cmd {
                    MediaCmd::Play => serde_json::json!({ "type": "play" }),
                    MediaCmd::Pause => serde_json::json!({ "type": "pause" }),
                    MediaCmd::Seek { ms } => {
                        serde_json::json!({ "type": "seek", "ms": ms })
                    }
                    MediaCmd::Volume { v } => {
                        serde_json::json!({ "type": "volume", "v": v })
                    }
                    MediaCmd::Muted { on } => {
                        serde_json::json!({ "type": "muted", "on": on })
                    }
                    MediaCmd::Rate { r } => {
                        serde_json::json!({ "type": "rate", "r": r })
                    }
                },
            })
            .to_string(),
        ),
        _ => return None,
    };
    Some(out)
}

/// Corps `{codecs: […]}` d'un `claim` → capacités déclarées
/// par l'hôte (le JSON non-tableau tombe sur `[]`, comme le
/// REST — tolérant, jamais de rejet pour un champ manquant).
#[must_use]
pub fn media_codecs_from_json(v: &serde_json::Value) -> Vec<String> {
    v.get("codecs")
        .and_then(serde_json::Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|c| c.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// Corps `{position_ms, duration_ms, buffered_ms, state,
/// error?}` d'un `state` → `MediaReport`. `None` si `state`
/// est absent ou hors du vocabulaire (→ `400` côté REST,
/// `-400` côté FFI).
#[must_use]
pub fn media_report_from_json(v: &serde_json::Value) -> Option<MediaReport> {
    let u64f = |k: &str| v.get(k).and_then(serde_json::Value::as_u64);
    let state = match v.get("state").and_then(serde_json::Value::as_str) {
        Some("playing") => MediaHostState::Playing,
        Some("paused") => MediaHostState::Paused,
        Some("ended") => MediaHostState::Ended,
        Some("error") => MediaHostState::Error,
        Some("buffering") => MediaHostState::Buffering,
        _ => return None,
    };
    Some(MediaReport {
        position_ms: u64f("position_ms").unwrap_or(0),
        duration_ms: u64f("duration_ms"),
        buffered_ms: u64f("buffered_ms").unwrap_or(0),
        state,
        error: v
            .get("error")
            .and_then(serde_json::Value::as_str)
            .map(String::from),
    })
}
