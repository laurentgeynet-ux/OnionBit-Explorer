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
    /// Codecs déclarés par l'hôte claimé (`None` = stub).
    pub(crate) claim: Option<Vec<String>>,
    /// Dernier `state` rapporté par l'hôte.
    pub(crate) report: Option<MediaReport>,
}

/// Élément média détecté sur le boxtree posé — image fixe
/// de l'offre avant allocation d'id.
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
                if let Ok(u) = Url::resolve(base, &src) {
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
                        mime: ty.or_else(|| media_mime_from_url(&u)),
                        url: u.to_string(),
                        controls,
                        autoplay: has_bool_attr(dom, n, "autoplay"),
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

impl Session {
    /// Recalcule les surfaces média de l'onglet au point de
    /// cohérence courant (post-commit, post-apply repeint) :
    /// retenue positionnelle (`kind`+`url` ordre document),
    /// `MediaGone` pour les disparues, `MediaMoved` pour les
    /// rect changés, `MediaOffered` pour les nouvelles. No-op
    /// si l'arbre/DOM ne sont pas un couple cohérent (`tree`
    /// absent pendant la fenêtre nav → relayout).
    pub(crate) fn refresh_media(&mut self, tab: TabId) {
        let mut gone: Vec<u64> = Vec::new();
        let mut moved: Vec<(u64, MediaRect)> = Vec::new();
        let mut offered: Vec<MediaSurface> = Vec::new();
        {
            let Some(entry) = self.tabs.get_mut(&tab) else {
                return;
            };
            let (Some(dom), Some(tree), Some(base)) =
                (entry.dom.as_ref(), entry.tree.as_ref(), entry.url.as_ref())
            else {
                return;
            };
            let scanned = scan_media(tree, dom, base);
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
                    retained[pos] = Some(o);
                } else {
                    gone.push(o.id);
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
                        claim: None,
                        report: None,
                    };
                    offered.push(MediaSurface {
                        id,
                        kind: surf.kind,
                        rect: surf.rect,
                        url: surf.url.clone(),
                        mime: surf.mime.clone(),
                        controls: surf.controls,
                        autoplay: surf.autoplay,
                        claim: None,
                        report: None,
                    });
                    media.push(surf);
                }
            }
            entry.media = media;
        }
        for id in gone {
            let _ = self.events.send(CoreEvent::MediaGone { tab, media: id });
        }
        for (id, rect) in moved {
            let _ = self.events.send(CoreEvent::MediaMoved {
                tab,
                media: id,
                rect,
            });
        }
        for s in offered {
            let _ = self.events.send(CoreEvent::MediaOffered {
                tab,
                media: s.id,
                kind: s.kind,
                rect: s.rect,
                url: s.url,
                mime: s.mime,
                controls: s.controls,
                autoplay: s.autoplay,
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
    /// parité FFI future).
    #[must_use]
    pub fn media_list(&self, tab: TabId) -> Vec<MediaOfferInfo> {
        self.tabs
            .get(&tab)
            .map(|e| {
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
                        claim: m.claim.clone(),
                        state: m.report.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Claim d'une surface par l'hôte (premier arrivé gagne)
    /// — `codecs` = capacités déclarées (alimenteront
    /// `canPlayType`/`error` à l'étape 71c). `Dead` → `410`,
    /// `Taken` → `409`.
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
        let Some(m) = self
            .tabs
            .get_mut(&tab)
            .and_then(|e| e.media.iter_mut().find(|m| m.id == media))
        else {
            return Err(MediaError::Dead);
        };
        if m.claim.is_some() {
            return Err(MediaError::Taken);
        }
        m.claim = Some(codecs);
        Ok(())
    }

    /// L'hôte se détache — la surface retombe sur les stubs
    /// ADR-0023 (dernier rapport conservé, inerte).
    ///
    /// # Errors
    /// [`MediaError::Dead`] id inconnu ou mort.
    pub fn media_release(&mut self, tab: TabId, media: u64) -> Result<(), MediaError> {
        let Some(m) = self
            .tabs
            .get_mut(&tab)
            .and_then(|e| e.media.iter_mut().find(|m| m.id == media))
        else {
            return Err(MediaError::Dead);
        };
        m.claim = None;
        Ok(())
    }

    /// Rapport `state` hôte → moteur (battement borné ≤ 4 Hz
    /// côté hôte — le moteur stocke sans émettre ; les props
    /// et events DOM JS arrivent à l'étape 71c). Exige un
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
        let Some(m) = self
            .tabs
            .get_mut(&tab)
            .and_then(|e| e.media.iter_mut().find(|m| m.id == media))
        else {
            return Err(MediaError::Dead);
        };
        if m.claim.is_none() {
            return Err(MediaError::NotClaimed);
        }
        m.report = Some(report);
        Ok(())
    }
}
