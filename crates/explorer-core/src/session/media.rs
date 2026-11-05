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
    /// Attribut `preload` énuméré (P-71 — `auto` absent ou
    /// invalide).
    pub(crate) preload: MediaPreload,
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
    /// Dernier rapport stocké mais pas encore livré au
    /// moteur (étape 182) — armé quand `entry.js` est détenu
    /// (fenêtre pompe) ; relivré à la réinsertion —
    /// synchro d'état : le dernier rapport suffit, jamais
    /// de file. Sans ce drapeau le `ended`/`playing` final
    /// d'une séquence tombée dans la fenêtre était perdu
    /// (promesse `play()` pendante, event jamais tiré).
    pub(crate) report_pending: bool,
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
    /// `timestampOffset` des `SourceBuffer` — même ordre
    /// que `mse_buffers` (P-54 `pts_offset_secs`).
    pub(crate) mse_offsets: Vec<f64>,
    /// Métadonnées de peinture de la boîte (étape 185, M1) —
    /// capturées sur le même arbre que la frame stockée :
    /// `refresh_media` tourne après `put_paint` dans le même
    /// appel `Session`, `meta` × `scroll_y` × `frame` sont
    /// donc atomiques par construction (contrat `x-explorer-
    /// media` — les drapeaux décrivent ce que le raster a
    /// réellement fait de la boîte, rien de plus).
    pub(crate) meta: MediaMeta,
    /// Chemin d'index de la boîte dans l'arbre posé (étape
    /// 188 — clé de `DisplayList::spans` pour la coupe
    /// top-slice ; `None` = élément sans boîte ou `audio`
    /// sans UI — jamais composite natif, aucun slice à
    /// couper pour lui).
    pub(crate) box_path: Option<Box<[u16]>>,
}

/// Contexte de peinture d'une boîte média (étape 185) —
/// l'accumulation imite `paint_box` : ce que le peintre
/// applique réellement à la boîte ou à son sous-arbre.
/// Les deux seaux de clip reflètent le `fixed_defer` du
/// peintre : un `fixed` échappe aux clips d'ancêtres
/// (drainé hors de tout `PushClip` en file racine) — les
/// clips collectés avant la frontière `fixed` sont en px
/// CSS **document**, ceux d'après en px CSS **viewport**.
#[allow(clippy::struct_excessive_bools)] // drapeaux de peinture orthogonaux.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct MediaMeta {
    /// Boîte ancrée au viewport — `position:fixed` sur
    /// l'élément ou un ancêtre : son rect stocké **est**
    /// déjà la position viewport (pas de `− scroll_y`).
    pub(crate) fixed: bool,
    /// Transforme dépendante de la peinture sur la chaîne —
    /// `transform ≠ none` ou `position:sticky` (décalage
    /// sticky = `PushTransform` fonction du scroll). L'hôte
    /// masque l'overlay : positionner un plan non transformé
    /// sur du contenu transformé serait un mensonge.
    pub(crate) transformed: bool,
    /// Intersection des clips d'ancêtres **espace document**
    /// (avant toute frontière `fixed`) — `overflow ≠
    /// visible` → border box du porteur (le rect exact que
    /// `paint_box` pousse en `PushClip`), `clip-path`/`clip`
    /// → même approximation bornée.
    pub(crate) clip_doc: Option<MediaRect>,
    /// Intersection des clips **espace viewport** — les
    /// ancêtres dans le sous-arbre `fixed` (incluse).
    pub(crate) clip_fixed: Option<MediaRect>,
    /// Un `clip-path`/`clip` géométrique est présent sur la
    /// chaîne — modélisé à la border box de son porteur
    /// (plus lâche que le clip réel de l'inset ; jamais de
    /// clip plus serré que le raster n'en applique).
    pub(crate) clip_path: bool,
    /// Peinte dans la frame : `visibility` calculée ≠
    /// `hidden|collapse` (réactivable par un descendant —
    /// le style calculé porte déjà l'héritage, lecture
    /// locale comme `paint_box`) **et** aucun `opacity ≤ 0`
    /// sur la chaîne (verrou irréversible du peintre). Un
    /// élément sans boîte (`display:none`) est `false` —
    /// rien n'est peint, quoi que joue la source.
    pub(crate) visible: bool,
}

/// Résultat d'un pull de segments MSE (§B-63) — `Pending`
/// distingue l'absence transitoire de moteur de la mort de
/// surface : `410` doit rester terminal.
pub enum MsePull {
    /// Segments drainés : octets, curseur absolu, fin de
    /// flux, offsets corps des frontières de section
    /// (`X-Mse-Section`, pont C — JS-C295).
    Ready(Vec<u8>, u64, bool, Vec<u64>),
    /// Moteur momentanément absent (fenêtre pompe) — re-sonder.
    Pending,
    /// `?from=` hors fenêtre adressable (JS-C296, chantier
    /// D — trou `remove`/éviction/position jamais servie)
    /// → `416` filaire, terminal.
    Unservable,
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
    /// Attribut `preload` énuméré (`auto`|`metadata`|`none`,
    /// invalide → `auto`).
    preload: MediaPreload,
    /// Chemin `(tag, index)` de l'élément (71c — miroir JS).
    path: Vec<(String, u32)>,
    /// Chemin d'index de la boîte dans l'arbre posé (étape
    /// 188 — clé de `DisplayList::spans` pour la coupe
    /// top-slice ; `None` = élément sans boîte ou `audio`
    /// sans UI, jamais composite natif).
    box_path: Option<Box<[u16]>>,
    /// Contexte peint de la boîte (étape 185) — voir
    /// [`MediaMeta`]. Défaut pour un élément sans boîte.
    meta: MediaMeta,
    /// `url` = object URL `MediaSource` enregistré côté
    /// moteur (MSE-2, ADR-0029) — remplie au refresh via
    /// `JsEngine::is_media_source`.
    mse: bool,
    /// MIME des `SourceBuffer` (index `?buffer=N` — offre
    /// `mseBuffers` pour `open_mse_multi`).
    mse_buffers: Vec<String>,
    /// `timestampOffset` par rail — même ordre que
    /// `mse_buffers` (P-54 `pts_offset_secs`).
    mse_offsets: Vec<f64>,
}

/// MIME média déduit du suffixe d'URL (contrat : `type` >
/// suffixe > `None`). Volontairement limité aux types
/// qu'un hôte v1 saurait jouer — pas un sniffer général.
fn media_mime_from_url(url: &Url) -> Option<String> {
    let path = url.path().to_ascii_lowercase();
    let ext = path.rsplit('.').next()?;
    let mime = match ext {
        "mp4" | "m4v" => "video/mp4",
        "m4a" | "m4b" | "m4p" => "audio/mp4",
        "3gp" | "3gpp" => "video/3gpp",
        "3g2" | "3gp2" | "3gpp2" => "video/3gpp2",
        "webm" => "video/webm",
        "weba" => "audio/webm",
        "mkv" | "mk3d" => "video/x-matroska",
        "mka" => "audio/x-matroska",
        "ogv" | "ogm" | "ogx" => "video/ogg",
        "mov" => "video/quicktime",
        "ts" | "m2ts" | "mts" => "video/mp2t",
        "avi" | "divx" => "video/x-msvideo",
        "asf" => "video/x-ms-asf",
        "wmv" => "video/x-ms-wmv",
        "wma" => "audio/x-ms-wma",
        "mpg" | "mpeg" | "mpe" | "m2v" | "mpv" | "vob" => "video/mpeg",
        "mpa" | "mp1" | "mp2" | "mp3" | "m1a" | "m2a" => "audio/mpeg",
        "m3u8" => "application/vnd.apple.mpegurl",
        "mpd" => "application/dash+xml",
        "aac" | "adts" => "audio/aac",
        "ac3" => "audio/ac3",
        "ec3" | "eac3" => "audio/ec3",
        "dts" => "audio/dts",
        "ogg" | "oga" | "opus" | "spx" => "audio/ogg",
        "wav" => "audio/wav",
        "flac" | "fla" => "audio/flac",
        _ => return None,
    };
    Some(mime.to_string())
}

/// Attribut booléen présent (`controls`, `autoplay`) —
/// présence compte, valeur ignorée (HTML).
fn has_bool_attr(dom: &Dom, n: NodeId, name: &str) -> bool {
    dom.attribute(n, name).ok().flatten().is_some()
}

/// Préfixe de codec RFC 6381 (`avc1.42E01E` → `avc1`) —
/// même réduction que `note_declared_codecs` côté moteur :
/// un hôte qui déclare `avc1` joue toute la famille.
fn codec_prefix(c: &str) -> Option<String> {
    let p = c.trim().split('.').next()?.trim();
    if p.is_empty() {
        None
    } else {
        Some(p.to_ascii_lowercase())
    }
}

/// Réduit une liste `mediaCodecs` hôte en préfixes dans
/// `set` — même boucle que `note_declared_codecs` côté
/// moteur : la valeur `codecs=` d'un MIME complet prime ;
/// sans elle, la valeur est prise pour un codec nu ; les
/// conteneurs (`video/mp4` seul) sont exclus. Partagée
/// entre `media_claim` (a1) et `declare_media_codecs`
/// (étape 178).
fn note_codec_prefixes(set: &mut std::collections::HashSet<String>, codecs: &[String]) {
    for c in codecs {
        let c = c.trim();
        let raw = match c.find("codecs=") {
            Some(pos) => &c[pos + 7..],
            None => c,
        };
        for part in raw.trim_matches(|ch| ch == '"' || ch == '\'').split(',') {
            let part = part.trim().trim_matches(|ch| ch == '"' || ch == '\'');
            if part.contains('/') {
                continue;
            }
            if let Some(p) = codec_prefix(part) {
                set.insert(p);
            }
        }
    }
}

/// Un `<source type="…">` est retenu quand son conteneur est
/// connu et que **tous** ses codecs déclarés sont dans
/// `declared` (préfixes d'hôte — `mediaCodecs` du claim).
/// `declared` vide → tout passe (capacités encore inconnues :
/// comportement historique, jamais de promesse retirée).
/// Type absent → candidat (spéc : l'agent teste la ressource).
fn source_type_playable(ty: &str, declared: &std::collections::HashSet<String>) -> bool {
    if declared.is_empty() {
        return true;
    }
    let t = ty.to_ascii_lowercase();
    let container = t.split(';').next().unwrap_or("").trim();
    // Conteneurs dont un démuxeur OnionCodec est enregistré
    // (ou routés par extension : manifests m3u8/mpd). Absents
    // à dessein : avi/asf/mpeg-ps (aucun démuxeur), `video/ogg`
    // (ogv réel = theora indécodable), `audio/ec3`/`eac3`
    // (démux OK mais `bsid>10` rejeté trame par trame).
    let known = matches!(
        container,
        "video/mp4"
            | "audio/mp4"
            | "audio/x-m4a"
            | "video/x-m4v"
            | "video/quicktime"
            | "video/3gpp"
            | "video/3gpp2"
            | "audio/3gpp"
            | "audio/3gpp2"
            | "video/webm"
            | "audio/webm"
            | "video/x-matroska"
            | "audio/x-matroska"
            | "video/mp2t"
            | "video/mpegts"
            | "audio/ogg"
            | "application/ogg"
            | "audio/opus"
            | "audio/mpeg"
            | "audio/mp3"
            | "audio/mpeg3"
            | "audio/x-mpeg"
            | "audio/x-mp3"
            | "audio/aac"
            | "audio/x-aac"
            | "audio/aacp"
            | "audio/adts"
            | "audio/ac3"
            | "audio/ac-3"
            | "audio/wav"
            | "audio/x-wav"
            | "audio/wave"
            | "audio/vnd.wave"
            | "audio/flac"
            | "audio/x-flac"
            | "application/vnd.apple.mpegurl"
            | "application/x-mpegURL"
            | "application/dash+xml"
    );
    if !known {
        return false;
    }
    match t.split("codecs=").nth(1) {
        Some(cstr) => {
            let cstr = cstr.trim_matches(|c| c == '"' || c == '\'');
            cstr.split(',')
                .filter_map(codec_prefix)
                .all(|p| declared.contains(&p))
        }
        None => true,
    }
}

/// Source lisible d'un élément média : attribut `src`
/// prioritaire, sinon premier `<source src>` enfant direct
/// **au `type` jouable** — la spec écarte les `<source>` à
/// type non supporté avant d'élire (a1 : un `webm` en tête
/// ne bloque plus un `mp4` décodable par l'hôte).
/// Renvoie `(src_brut, type_déclaré)`.
fn media_source(
    dom: &Dom,
    n: NodeId,
    declared: &std::collections::HashSet<String>,
) -> Option<(String, Option<String>)> {
    let attr = |name: &str| dom.attribute(n, name).ok().flatten();
    if let Some(src) = attr("src").filter(|s| !s.is_empty()) {
        return Some((src, attr("type")));
    }
    for c in dom.children(n).unwrap_or_default() {
        if !dom.tag_name(c).is_ok_and(|t| t == "source") {
            continue;
        }
        let Ok(Some(src)) = dom.attribute(c, "src") else {
            continue;
        };
        if src.is_empty() {
            continue;
        }
        let ty = dom.attribute(c, "type").ok().flatten();
        if ty
            .as_deref()
            .is_some_and(|t| !source_type_playable(t, declared))
        {
            continue;
        }
        return Some((src, ty));
    }
    None
}

/// État hérité de la descente du boxtree (étape 185) —
/// accumulateur de [`MediaMeta`], mêmes règles que
/// `paint_box` dans `explorer-render` :
/// `fixed`/`sticky`/`transform`/`opacity`/`overflow`/
/// `clip-path` lus au style **calculé** de chaque boîte.
#[allow(clippy::struct_excessive_bools)] // drapeaux de peinture orthogonaux.
#[derive(Clone, Copy, Default)]
struct MediaCtx {
    /// Frontière `fixed` déjà croisée — les ancêtres d'un
    /// `fixed` ne le clippent pas (`fixed_defer` drainé hors
    /// clips en file racine) : les seaux basculent.
    fixed: bool,
    /// Transforme peinte sur la chaîne (latch).
    transformed: bool,
    /// Clips collectés avant la frontière `fixed` — espace
    /// document (le peintre soustrait `scroll_y`).
    clip_doc: Option<MediaRect>,
    /// Clips collectés dans le sous-arbre `fixed` — espace
    /// viewport (les rects stockés y vivent déjà).
    clip_fixed: Option<MediaRect>,
    /// `clip-path`/`clip` géométrique vu sur la chaîne.
    clip_path: bool,
    /// `opacity ≤ 0` rencontré — verrou irréversible du
    /// peintre (0 ne se relève jamais, contrairement à
    /// `visibility:hidden` qui se réactive).
    opacity_dead: bool,
}

impl MediaCtx {
    /// Applique les contributions propres de `bx` — dans le
    /// même ordre que `paint_box` les émet : `fixed` d'abord
    /// (le statut de la boîte décide le seau de ses propres
    /// clips), puis transform, opacity, clips.
    fn accumulate(&mut self, bx: &LayoutBox) {
        if bx.position == explorer_layout::Position::Fixed {
            self.fixed = true;
        }
        if bx.position == explorer_layout::Position::Sticky
            || bx
                .style
                .get("transform")
                .is_some_and(|v| v.trim() != "none")
        {
            self.transformed = true;
        }
        if bx
            .style
            .get("opacity")
            .is_some_and(|v| v.trim().parse::<f32>().is_ok_and(|o| o <= 0.0))
        {
            self.opacity_dead = true;
        }
        // `overflow` ≠ visible → `PushClip(border box)` en
        // peinture (même rect, documenté R-55/R-120).
        if bx.overflow != explorer_layout::Overflow::Visible
            || bx.overflow_x != explorer_layout::Overflow::Visible
        {
            self.push_clip(MediaRect {
                x: bx.rect.x,
                y: bx.rect.y,
                w: bx.rect.width,
                h: bx.rect.height,
            });
        }
        // `clip-path`/`clip` : le moteur n'émet que les
        // formes rectangulaires (`inset()`, `clip:rect()`) —
        // la border box du porteur est la borne honnête
        // (jamais plus serrée que son clip réel), le reste
        // n'est pas émis du tout → flag, pas de clip.
        if bx
            .style
            .get("clip-path")
            .is_some_and(|v| v.trim() != "none")
            || bx.style.get("clip").is_some_and(|v| v.trim() != "auto")
        {
            self.clip_path = true;
            self.push_clip(MediaRect {
                x: bx.rect.x,
                y: bx.rect.y,
                w: bx.rect.width,
                h: bx.rect.height,
            });
        }
    }

    /// Intersecte `r` dans le seau courant — `fixed` pose la
    /// frontière : avant → espace document, après → espace
    /// viewport (les deux sont convertis à la lecture).
    fn push_clip(&mut self, r: MediaRect) {
        let slot = if self.fixed {
            &mut self.clip_fixed
        } else {
            &mut self.clip_doc
        };
        *slot = Some(match *slot {
            None => r,
            Some(c) => {
                let (x, y) = (c.x.max(r.x), c.y.max(r.y));
                MediaRect {
                    x,
                    y,
                    w: ((c.x + c.w).min(r.x + r.w) - x).max(0.0),
                    h: ((c.y + c.h).min(r.y + r.h) - y).max(0.0),
                }
            }
        });
    }
}

/// Éléments média **lisibles** du document posé — parcours
/// du **DOM** (ordre du document ; un `audio` sans
/// `controls` est `display:none` au style UA et n'a pas de
/// boîte — il doit quand même être offert à 0×0, contrat)
/// joint à la carte `NodeId → rect` du boxtree posé. Un
/// élément média sans boîte (display:none explicite) reste
/// offert à 0×0 : l'hôte n'attache que le pipeline. Pas de
/// source → pas d'offre (honnêteté ADR-0023 conservée).
fn scan_media(
    tree: &LayoutBox,
    dom: &Dom,
    base: &Url,
    declared: &std::collections::HashSet<String>,
) -> Vec<ScannedMedia> {
    // Rects + contexte peint par nœud source — une passe
    // itérative sur l'arbre posé (un élément = au plus une
    // boîte ici). `MediaCtx` descend avec la récursion :
    // chaque boîte applique SES contributions (position,
    // transform, opacity, clips) avant d'être enregistrée —
    // mêmes règles que `paint_box`, même ordre de lecture.
    let mut rects: HashMap<NodeId, (MediaRect, MediaMeta, Box<[u16]>)> = HashMap::new();
    let mut stack: Vec<(&LayoutBox, MediaCtx, Vec<u16>)> =
        vec![(tree, MediaCtx::default(), Vec::new())];
    while let Some((bx, mut ctx, path)) = stack.pop() {
        ctx.accumulate(bx);
        if let Some(n) = bx.node {
            let meta = MediaMeta {
                fixed: ctx.fixed,
                transformed: ctx.transformed,
                clip_doc: ctx.clip_doc,
                clip_fixed: ctx.clip_fixed,
                clip_path: ctx.clip_path,
                visible: !matches!(bx.style.get("visibility"), Some("hidden" | "collapse"))
                    && !ctx.opacity_dead,
            };
            rects.entry(n).or_insert((
                MediaRect {
                    x: bx.rect.x,
                    y: bx.rect.y,
                    w: bx.rect.width,
                    h: bx.rect.height,
                },
                meta,
                // Chemin d'index de la boîte dans l'arbre
                // posé (étape 188 — clé `DisplayList::spans`
                // pour la coupe top-slice : la même passe qui
                // pose le contexte peint porte l'adresse de
                // la plage d'items émise).
                path.clone().into_boxed_slice(),
            ));
        }
        stack.extend(bx.children.iter().enumerate().rev().map(|(i, c)| {
            let mut p = path.clone();
            p.push(u16::try_from(i).unwrap_or(u16::MAX));
            (c, ctx, p)
        }));
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
            if let Some((src, ty)) = media_source(dom, n, declared) {
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
                    let (rect, meta, box_path) = if kind == MediaKind::Audio && !controls {
                        (zero, MediaMeta::default(), None)
                    } else {
                        rects
                            .get(&n)
                            .map(|(r, m, p)| (*r, *m, Some(p.clone())))
                            .unwrap_or((zero, MediaMeta::default(), None))
                    };
                    out.push(ScannedMedia {
                        kind,
                        rect,
                        meta,
                        mime: ty.or_else(|| parsed.as_ref().and_then(media_mime_from_url)),
                        url: url_s,
                        controls,
                        autoplay: has_bool_attr(dom, n, "autoplay"),
                        looping: has_bool_attr(dom, n, "loop"),
                        preload: MediaPreload::parse(
                            dom.attribute(n, "preload").ok().flatten().as_deref(),
                        ),
                        path: element_path(dom, n).unwrap_or_default(),
                        box_path,
                        mse: false,
                        mse_buffers: Vec::new(),
                        mse_offsets: Vec::new(),
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
            s.mse_offsets = js
                .as_mut()
                .map(|e| e.mse_offsets(&s.url))
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
            m.mse_offsets = e.mse_offsets(&m.url);
        }
    }
}

/// Traduction rapport session → payload moteur (étape
/// 71c/182) — `buffering` passe désormais en état distinct
/// (`waiting`/`stalled` HTML5), plus aplati en `playing`.
fn js_report_of(report: &MediaReport) -> explorer_js::JsMediaReport {
    explorer_js::JsMediaReport {
        position_ms: report.position_ms,
        duration_ms: report.duration_ms,
        buffered_ms: report.buffered_ms,
        state: match report.state {
            MediaHostState::Playing => explorer_js::JsMediaHostState::Playing,
            MediaHostState::Paused => explorer_js::JsMediaHostState::Paused,
            MediaHostState::Ended => explorer_js::JsMediaHostState::Ended,
            MediaHostState::Error => explorer_js::JsMediaHostState::Error,
            MediaHostState::Buffering => explorer_js::JsMediaHostState::Buffering,
        },
        error: report.error.clone(),
        buffered_ranges: report.buffered_ranges.clone(),
        frame_stats: report.frame_stats.map(|f| explorer_js::JsMediaFrameStats {
            presented: f.presented,
            dropped: f.dropped,
            decoded: f.decoded,
            corrupted: f.corrupted,
            total_delay_ms: f.total_delay_ms,
        }),
    }
}

/// Format PCM d'un flux `audio` hôte (Étape 204b —
/// `AUDIO_MODE_TAP` dépôt) : échantillons interleaved non
/// coupés trame ; `format` = vocabulaire dépôt (`4` f32 LE,
/// la `AUDIO_SPEC` fixe actuelle).
#[derive(Debug, Clone, Copy)]
pub struct MediaAudioSpec {
    /// Cadence (Hz).
    pub sample_rate: u32,
    /// Canaux.
    pub channels: u16,
    /// Format d'échantillon dépôt (`0` u8 … `5` f64).
    pub format: i32,
}

/// Traduction spec session → payload moteur (Étape 204b).
fn js_audio_spec_of(spec: MediaAudioSpec) -> explorer_js::JsAudioSpec {
    explorer_js::JsAudioSpec {
        sample_rate: spec.sample_rate,
        channels: spec.channels,
        format: spec.format,
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

/// Livre au moteur le dernier rapport `state` retenu pendant
/// une fenêtre moteur-absent (étape 182) — synchro d'état :
/// le dernier rapport suffit, jamais de file d'événements.
fn sync_report_to_engine(m: &mut MediaSurface, js: &mut Option<Box<dyn JsEngine>>) {
    if m.report_pending {
        if let (Some(e), Some(rep)) = (js.as_mut(), m.report.as_ref()) {
            e.deliver_media_state(&m.path, m.id, js_report_of(rep));
            m.report_pending = false;
        }
    }
}

/// Claim puis dernier rapport — ordre obligé : un `state`
/// livré sur un miroir jamais claimé est un no-op moteur
/// (`d.media` vide → gel silencieux, étape 182).
fn sync_pending_to_engine(m: &mut MediaSurface, js: &mut Option<Box<dyn JsEngine>>) {
    sync_claim_to_engine(m, js);
    sync_report_to_engine(m, js);
}

impl Session {
    /// Recalcule les surfaces média de l'onglet au point de
    /// cohérence courant (post-commit, post-apply repeint) :
    /// retenue positionnelle (`kind`+`url` ordre document),
    /// `MediaGone` pour les disparues, `MediaMoved` pour les
    /// rect changés, `MediaOffered` pour les nouvelles. No-op
    /// si l'arbre/DOM ne sont pas un couple cohérent (`tree`
    /// absent pendant la fenêtre nav → relayout).
    #[allow(clippy::too_many_lines)] // offre/rétention par surface — grandir ne factorise rien
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
            let mut scanned = scan_media(tree, dom, base, &self.declared_media_codecs);
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
                    // Métadonnées peintes (étape 185) —
                    // rafraîchies silencieusement : un flag
                    // peut changer à rect identique (un
                    // ancêtre gagne `overflow:hidden`) ; le
                    // canal géométrie est la réponse `/frame`,
                    // jamais un event à part.
                    o.meta = s.meta;
                    o.mime.clone_from(&s.mime);
                    o.controls = s.controls;
                    o.autoplay = s.autoplay;
                    o.looping = s.looping;
                    o.preload = s.preload;
                    o.path.clone_from(&s.path);
                    o.box_path.clone_from(&s.box_path);
                    // Moteur absent → drapeau/buffers retenus
                    // (scan à l'aveugle : ne rien dégrader) ;
                    // moteur présent → rafraîchir, y compris
                    // `mse_buffers` — les `SourceBuffer` ajoutés
                    // APRÈS l'offre doivent la compléter (vol
                    // 177 : `mseBuffers` restait `[]` à vie).
                    if js_present {
                        o.mse = s.mse;
                        o.mse_buffers.clone_from(&s.mse_buffers);
                        o.mse_offsets.clone_from(&s.mse_offsets);
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
                        preload: s.preload,
                        claim: None,
                        claim_pending: false,
                        report: None,
                        report_pending: false,
                        path: s.path,
                        box_path: s.box_path,
                        mse: s.mse,
                        mse_buffers: s.mse_buffers,
                        mse_offsets: s.mse_offsets,
                        meta: s.meta,
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
                        preload: surf.preload,
                        claim: None,
                        claim_pending: false,
                        report: None,
                        report_pending: false,
                        path: surf.path.clone(),
                        box_path: surf.box_path.clone(),
                        mse: surf.mse,
                        mse_offsets: surf.mse_offsets.clone(),
                        meta: surf.meta,
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
                preload: s.preload,
                mse_buffers: s.mse_buffers.clone(),
                mse_offsets: s.mse_offsets.clone(),
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
                    sync_pending_to_engine(m, &mut e.js);
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
                        preload: m.preload,
                        claim: m.claim.clone(),
                        state: m.report.clone(),
                        mse: m.mse,
                        mse_buffers: m.mse_buffers.clone(),
                        mse_offsets: m.mse_offsets.clone(),
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
    /// morte, non-MSE ou buffer hors rang → 410. `from`
    /// (JS-C296, chantier D — `MseSource::seek` → `?from=`) :
    /// repositionnement absolu du curseur de service ;
    /// `Unservable` si la cible est hors fenêtre adressable.
    pub fn media_segments_pull(
        &mut self,
        tab: TabId,
        mid: u64,
        buffer: usize,
        max: usize,
        from: Option<u64>,
        peek: bool,
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
        match eng.mse_pull(&m.url.clone(), buffer, max, from, peek) {
            Some(Ok((data, cursor, eos, sections))) => MsePull::Ready(data, cursor, eos, sections),
            Some(Err(())) => MsePull::Unservable,
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
        // Capacités hôte apprises au claim (a1) — même
        // normalisation que `note_declared_codecs` côté
        // moteur : préfixes codec, conteneurs exclus. Le set
        // sert au choix du `<source>` aux scans suivants.
        note_codec_prefixes(&mut self.declared_media_codecs, &codecs);
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

    /// Déclare les capacités média de l'hôte **avant tout
    /// claim** (étape 178 — b-e2e) : la liste `mediaCodecs`
    /// de l'app (MIME `codecs=` ou codecs nus) alimente le
    /// set `declared_media_codecs` (choix `<source>` dès le
    /// premier scan) et la liste brute `host_media_codecs`
    /// poussée dans chaque contexte JS au spawn — sans elle,
    /// `MediaSource.isTypeSupported` répond `""` sur toute
    /// page fraîche et un player MSE réel n'instancie jamais
    /// la `MediaSource` dont le claim apprendrait les
    /// codecs (cercle vicieux d'amorçage). Déclaration
    /// additive et idempotente ; les contextes déjà vivants
    /// reçoivent la table immédiatement.
    pub fn declare_media_codecs(&mut self, codecs: &[String]) {
        note_codec_prefixes(&mut self.declared_media_codecs, codecs);
        for c in codecs {
            let c = c.trim();
            if !c.is_empty() && !self.host_media_codecs.iter().any(|x| x == c) {
                self.host_media_codecs.push(c.to_string());
            }
        }
        for entry in self.tabs.values_mut() {
            if let Some(e) = entry.js.as_mut() {
                e.note_media_codecs(codecs);
            }
        }
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
        // Un claim posé pendant une fenêtre pompe doit
        // atteindre le moteur **avant** ce rapport — sinon le
        // miroir n'a pas d'état média et la livraison est un
        // no-op silencieux à vie (gel post-claim, étape 182).
        sync_claim_to_engine(m, &mut entry.js);
        if let Some(eng) = entry.js.as_mut() {
            eng.deliver_media_state(&m.path, m.id, js_report_of(&report));
            m.report_pending = false;
        } else {
            // Fenêtre pompe : le dernier rapport est retenu
            // puis relivré à la réinsertion du moteur
            // (`sync_media_pending` au commit — étape 182).
            m.report_pending = true;
        }
        m.report = Some(report);
        Ok(())
    }

    /// Chunk PCM de la surface claimée (Étape 204b — tee
    /// `AUDIO_MODE_TAP` dépôt, ADR-0032) → ring `pcm`
    /// moteur par surface, lu par le graphe Web Audio
    /// (204c). Flux : un chunk pendant la fenêtre
    /// moteur-absent est perdu (pas de rejeu), le ring
    /// reprend au chunk suivant. Exige un claim actif.
    ///
    /// # Errors
    /// [`MediaError::Dead`] id inconnu ou mort ;
    /// [`MediaError::NotClaimed`] surface non claimée.
    pub fn media_audio(
        &mut self,
        tab: TabId,
        media: u64,
        spec: MediaAudioSpec,
        chunk: &[u8],
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
        sync_claim_to_engine(m, &mut entry.js);
        if let Some(eng) = entry.js.as_mut() {
            eng.deliver_media_audio(&m.path, m.id, js_audio_spec_of(spec), chunk);
        }
        Ok(())
    }

    /// Relivre au moteur les claim et derniers rapports retenus
    /// pendant son absence (étape 182 — fenêtre de pompe
    /// détachée). Appelée au `commit_js_pump` (réinsertion) et
    /// aux contacts REST ; l'ordre claim→state est tenu par
    /// surface (`sync_pending_to_engine`). Livraisons
    /// fire-and-forget : sûres même sous disjoncteur — les
    /// drapeaux retombent seulement si le moteur est présent.
    pub(crate) fn sync_media_pending(&mut self, tab: TabId) {
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return;
        };
        for m in &mut entry.media {
            sync_pending_to_engine(m, &mut entry.js);
        }
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
                    explorer_js::JsMediaCmd::FastSeek { ms } => MediaCmd::FastSeek { ms },
                    explorer_js::JsMediaCmd::Volume { v } => MediaCmd::Volume { v },
                    explorer_js::JsMediaCmd::Muted { on } => MediaCmd::Muted { on },
                    explorer_js::JsMediaCmd::Loop { on } => MediaCmd::Loop { on },
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
        "mseOffsets": m.mse_offsets,
        "controls": m.controls, "autoplay": m.autoplay, "loop": m.looping,
        "preload": m.preload.as_str(),
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
            preload,
            mse_buffers,
            mse_offsets,
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
                "loop": looping, "preload": preload.as_str(),
                "mseBuffers": mse_buffers,
                "mseOffsets": mse_offsets,
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
                    MediaCmd::FastSeek { ms } => {
                        serde_json::json!({ "type": "seek", "ms": ms, "fast": true })
                    }
                    MediaCmd::Volume { v } => {
                        serde_json::json!({ "type": "volume", "v": v })
                    }
                    MediaCmd::Muted { on } => {
                        serde_json::json!({ "type": "muted", "on": on })
                    }
                    MediaCmd::Loop { on } => {
                        serde_json::json!({ "type": "loop", "on": on })
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
    // Extensions dépôt codec — `buffered_ranges` = occupation
    // réelle du PacketBuffer ([[start,end],…] ms), `frame_stats`
    // = télémétrie frames. Absentes = surface pré-land → vides.
    let buffered_ranges = v
        .get("buffered_ranges")
        .and_then(serde_json::Value::as_array)
        .map(|rs| {
            rs.iter()
                .filter_map(|r| {
                    let p = r.as_array()?;
                    Some((p.first()?.as_u64()?, p.get(1)?.as_u64()?))
                })
                .collect()
        })
        .unwrap_or_default();
    let frame_stats = v.get("frame_stats").and_then(|fs| {
        Some(MediaFrameStats {
            presented: fs.get("presented")?.as_u64().unwrap_or(0),
            dropped: fs.get("dropped")?.as_u64().unwrap_or(0),
            decoded: fs.get("decoded")?.as_u64().unwrap_or(0),
            corrupted: fs.get("corrupted")?.as_u64().unwrap_or(0),
            total_delay_ms: fs.get("total_delay_ms")?.as_f64().unwrap_or(0.0),
        })
    });
    Some(MediaReport {
        position_ms: u64f("position_ms").unwrap_or(0),
        duration_ms: u64f("duration_ms"),
        buffered_ms: u64f("buffered_ms").unwrap_or(0),
        state,
        error: v
            .get("error")
            .and_then(serde_json::Value::as_str)
            .map(String::from),
        buffered_ranges,
        frame_stats,
    })
}
