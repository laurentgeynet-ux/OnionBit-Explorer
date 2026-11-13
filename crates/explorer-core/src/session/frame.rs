// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Accès frames : PNG paresseux, pixels bruts, régions
//! dirty — la couture de pull (ADR-0004).
use super::*;

use explorer_render::Frame;

/// Enveloppe métadonnées de la frame stockée (étape 185,
/// M1 — contrat `x-explorer-scroll-y`/`x-explorer-media`,
/// parité FFI `explorer_frame_pull`) : le `scroll_y`
/// **peint** + la géométrie des surfaces au moment de la
/// peinture, servis **dans le même corps** que les pixels —
/// la course « frame d'un instant, métadonnées d'un autre »
/// disparaît par construction (même verrou `Session` que
/// `frame_png`/`frame_pixels`).
#[derive(serde::Serialize)]
pub struct FrameMeta {
    /// Scroll ancré dans la frame (px CSS) — l'hôte en
    /// déduit son `paintedScrollY` au lieu de le deviner.
    pub scroll_y: f32,
    /// Dimensions de la frame décrite (px physiques) — le
    /// JSON se porte seul (canal FFI : le consommateur
    /// vérifie l'appairage avec les pixels copiés).
    pub w: u32,
    /// Hauteur de la frame décrite (px physiques).
    pub h: u32,
    /// Surfaces de la frame, ordre du document (ordre de
    /// peinture approximé — les contextes d'empilement
    /// réels ne sont pas ordonnancés en M1, documenté).
    pub media: Vec<FrameMedia>,
    /// Génération de la frame (étape 188 — bumpée à chaque
    /// `put_paint` ; l'hôte la renvoie au pull top-slice,
    /// qui rejette toute génération périmée — base et slice
    /// s'appairent par construction).
    #[serde(rename = "e")]
    pub epoch: u64,
    /// Rect **viewport** px CSS de la zone top-slice —
    /// ∪ des `v∩c` des surfaces éligibles (`visible`,
    /// `!transformed`, aire > 0, boîte spannée) ; `None` =
    /// aucun slice à tirer pour cette frame.
    #[serde(rename = "top")]
    pub top: Option<MediaRect>,
}

/// Résultat du pull top-slice (étape 188) — `Stale` =
/// génération demandée ≠ génération stockée (une peinture
/// s'est interposée : l'hôte re-tire la paire entière).
pub enum FrameTop {
    /// Aucun slice pour la frame courante (pas de surface
    /// composite, pas de `list`, raster impossible).
    None,
    /// Epoch périmée — l'appairage base↔slice est cassé.
    Stale,
    /// Slice prêt : rect viewport px CSS + pixmap RGBA+α.
    Slice(MediaRect, Frame),
}

/// Coupe top-slice de la frame stockée (étape 188) —
/// `(indice de coupe dans `list.items`, rect viewport
/// ∪(v∩c))` ou `None` si aucune surface n'exige de slice.
/// Éligible : `visible` **et** `!transformed` (l'hôte
/// masque les autres — aucun plan natif à recouvrir),
/// aire `v∩c > 0`, et boîte **spannée** (sans span dans
/// `list.spans`, la coupe est introuvable → la surface ne
/// participe ni au cut ni à l'union).
///
/// Coupe unique au `max(span.end)` — approximation nommée
/// dans la réponse PIPE : un élément peint **entre** deux
/// surfaces composite reste sous les deux (les médias qui
/// se recouvrent sont rares ; le décor de la boîte média
/// émet *avant* son contenu ici — il reste donc en base,
/// visibilité identique à l'avant-slice).
fn media_top(entry: &Tab) -> Option<(usize, MediaRect)> {
    let list = entry.list.as_ref()?;
    // `painted_scroll_y` — le list/frame décrivent
    // l'ancrage de leur passe ; le scroll demandé a pu
    // avancer pendant une peinture détachée.
    let sy = entry.painted_scroll_y;
    let shift = |r: MediaRect| MediaRect { y: r.y - sy, ..r };
    let mut cut = 0usize;
    let mut union: Option<MediaRect> = None;
    for s in &entry.media {
        let m = &s.meta;
        if !m.visible || m.transformed {
            continue;
        }
        let Some(span) = s.box_path.as_deref().and_then(|p| list.spans.get(p)) else {
            continue;
        };
        let view = if m.fixed { s.rect } else { shift(s.rect) };
        let clip = if m.fixed {
            m.clip_fixed
        } else {
            m.clip_doc.map(shift)
        };
        let area = clip.map_or(view, |c| rect_intersect(view, c));
        if area.w <= 0.0 || area.h <= 0.0 {
            continue;
        }
        cut = cut.max(usize::try_from(span.end).unwrap_or(usize::MAX));
        union = Some(union.map_or(area, |u| rect_union(u, area)));
    }
    union.map(|u| (cut, u))
}

/// Intersection px CSS (w/h ≥ 0 après saturation).
fn rect_intersect(a: MediaRect, b: MediaRect) -> MediaRect {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    MediaRect {
        x,
        y,
        w: ((a.x + a.w).min(b.x + b.w) - x).max(0.0),
        h: ((a.y + a.h).min(b.y + b.h) - y).max(0.0),
    }
}

/// Plus petit rectangle englobant — bornage du slice.
fn rect_union(a: MediaRect, b: MediaRect) -> MediaRect {
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    MediaRect {
        x,
        y,
        w: (a.x + a.w).max(b.x + b.w) - x,
        h: (a.y + a.h).max(b.y + b.h) - y,
    }
}

/// Géométrie peinte d'une surface — le filaire porte des
/// clés courtes (`i`/`r`/`v`/`f`/`t`/`c`/`n`/`s`/`o`) pour
/// borner `x-explorer-media` dans un header. `r` est en
/// px CSS **document**, `v` et `c` en px CSS **viewport** —
/// la conversion `scroll_y` est faite ici, jamais à l'hôte
/// (la règle `fixed`/`sticky` n'aurait pas à être recopiée).
#[allow(clippy::struct_excessive_bools)] // drapeaux de peinture orthogonaux.
#[derive(serde::Serialize)]
pub struct FrameMedia {
    /// Identité de l'offre (`media` des events SSE).
    #[serde(rename = "i")]
    pub id: u64,
    /// Rect px CSS document.
    #[serde(rename = "r")]
    pub rect: MediaRect,
    /// Rect px CSS viewport — `fixed ? r : r − scroll_y`.
    #[serde(rename = "v")]
    pub view: MediaRect,
    /// Ancrée au viewport (`position:fixed` sur la chaîne).
    #[serde(rename = "f")]
    pub fixed: bool,
    /// Transformée (`transform ≠ none`/`sticky`) — l'hôte
    /// masque : un plan non transformé serait un mensonge.
    #[serde(rename = "t")]
    pub transformed: bool,
    /// Clip **espace viewport** (`None` = aucun clip émis
    /// par le peintre — le seau document est traduit
    /// `−scroll_y`, le seau viewport vit déjà à l'échelle).
    #[serde(rename = "c")]
    pub clip: Option<MediaRect>,
    /// `clip-path`/`clip` géométrique présent — modélisé
    /// border-box (honnêteté : plus lâche que l'inset réel).
    #[serde(rename = "n")]
    pub clip_path: bool,
    /// Effectivement peinte (`false` → l'hôte masque :
    /// `visibility:hidden`, `opacity ≤ 0`, ou pas de boîte).
    #[serde(rename = "s")]
    pub visible: bool,
    /// Index de peinture (ordre document — approx M1).
    #[serde(rename = "o")]
    pub order: u32,
}

impl Session {
    /// Métadonnées atomiques de la frame stockée (`None`
    /// si jamais peint — comme `frame_pixels`). Servie
    /// header REST (`x-explorer-media`) et JSON FFI
    /// (`explorer_frame_pull`) — sérialisée à la demande,
    /// jamais stockée (l'état vit déjà dans `entry.media`).
    #[must_use]
    pub fn frame_meta(&self, tab: TabId) -> Option<FrameMeta> {
        let entry = self.tabs.get(&tab)?;
        let frame = entry.frame.as_ref()?;
        let sy = entry.painted_scroll_y;
        let shift = |r: MediaRect| MediaRect { y: r.y - sy, ..r };
        let media = entry
            .media
            .iter()
            .enumerate()
            .map(|(o, m)| {
                let meta = &m.meta;
                FrameMedia {
                    id: m.id,
                    rect: m.rect,
                    view: if meta.fixed { m.rect } else { shift(m.rect) },
                    fixed: meta.fixed,
                    transformed: meta.transformed,
                    // `fixed` échappe les clips ancêtres
                    // (`fixed_defer` hors clips) : seule la
                    // branche viewport compte pour lui.
                    clip: if meta.fixed {
                        meta.clip_fixed
                    } else {
                        meta.clip_doc.map(shift)
                    },
                    clip_path: meta.clip_path,
                    visible: meta.visible,
                    #[allow(clippy::cast_possible_truncation)]
                    // surfaces < 2^32 par document.
                    order: o as u32,
                }
            })
            .collect();
        Some(FrameMeta {
            scroll_y: sy,
            w: frame.width,
            h: frame.height,
            media,
            epoch: entry.frame_epoch,
            top: media_top(entry).map(|(_, r)| r),
        })
    }

    /// Pixels RGBA + métadonnées de la frame stockée en
    /// **un seul emprunt** (étape 185 — canal FFI du pull
    /// atomique : `frame_size` + `frame_copy` + `frame_meta`
    /// laisseraient deux fenêtres de course entre trois
    /// lectures ; ici l'octet et le JSON décrivent la même
    /// frame par construction).
    #[must_use]
    pub fn frame_pull(&self, tab: TabId) -> Option<(u32, u32, &[u8], FrameMeta)> {
        let (w, h, px) = self.frame_pixels(tab)?;
        let meta = self.frame_meta(tab)?;
        Some((w, h, px, meta))
    }

    /// Slice « top » de la frame stockée (étape 188 — M2
    /// UX-vidéo, demande NAV→PIPE `interface-equipes.md`) :
    /// les items émis **après** la plus haute surface
    /// composite re-rasterisés RGBA+α, bornés à `∪(v∩c)` —
    /// l'hôte compose `base → surface(s) → slice`. La base
    /// reste la frame complète : le slice n'est qu'un
    /// re-raster de ce que le plan natif masque.
    ///
    /// `epoch = Some(e)` exige la génération `e` (celle de
    /// `FrameMeta`/`x-explorer-frame-epoch`) : `Stale` si
    /// une autre peinture s'est interposée — jamais
    /// d'appairage base↔slice de deux frames. Raster
    /// paresseux depuis `entry.list` (même mécanique que
    /// `frame_png_full`, cache glyphes partagé).
    pub fn frame_top(&mut self, tab: TabId, epoch: Option<u64>) -> FrameTop {
        let fonts = self.apply_ctx().fonts;
        let scale = self.scale.max(f32::EPSILON);
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return FrameTop::None;
        };
        // Pull consommateur (Étape 222) — même armeur que
        // `tick_animations` : le slice est servi sans tick
        // (epoch figée), il arme quand même la fenêtre anim.
        entry.last_frame_pull = Some(std::time::Instant::now());
        if epoch.is_some_and(|e| e != entry.frame_epoch) {
            return FrameTop::Stale;
        }
        // Fenêtre nav/detachée : `list` peut survivre à
        // `frame` — un slice sans base stockée n'est pas
        // appairable → `None`, jamais un plan orphelin.
        if entry.frame.is_none() {
            return FrameTop::None;
        }
        let Some(list) = entry.list.as_ref() else {
            return FrameTop::None;
        };
        let Some((cut, win)) = media_top(entry) else {
            return FrameTop::None;
        };
        let win_css = explorer_layout::Rect {
            x: win.x,
            y: win.y,
            width: win.w,
            height: win.h,
        };
        let Some(frame) = explorer_render::rasterize_slice_cached(
            list,
            &fonts as &dyn explorer_layout::font::FontSource,
            scale,
            win_css,
            cut,
            &mut entry.font_cache,
        ) else {
            return FrameTop::None;
        };
        FrameTop::Slice(win, frame)
    }

    /// PNG courant de l'onglet (`None` si jamais navigué) —
    /// encodé paresseusement depuis la frame raster puis
    /// caché jusqu'au prochain repaint (étape 55g : un
    /// burst de repaints entre deux lectures n'encode
    /// qu'une fois, pas par repaint).
    #[must_use]
    pub fn frame_png(&mut self, tab: TabId) -> Option<Vec<u8>> {
        self.ensure_painted(tab);
        let entry = self.tabs.get_mut(&tab)?;
        // Pull consommateur (Étape 222) — lire des pixels
        // arme la fenêtre `anim_pull_idle` des ticks pompe.
        entry.last_frame_pull = Some(std::time::Instant::now());
        if entry.png.is_none() {
            // Un encodage vide n'est jamais un PNG serviable
            // — rejeté plutôt que caché : la prochaine
            // demande retentera l'encodage (cohérence avec
            // le `204` + `Retry-After` de l'API). L'échec
            // d'encodage est `WARN`-nommé côté
            // `encode_png_to` (Étape 180 — `png_b=0` >55 Mo).
            entry.png = entry.frame.as_ref()?.encode_png().filter(|p| !p.is_empty());
        }
        entry.png.clone()
    }

    /// PNG du **document entier** (`?full=1`, Étape 180 —
    /// P4). En mode plein-doc la frame stockée couvre déjà
    /// tout ; en mode fenêtré le document est re-rasterisé
    /// à la demande depuis la display list — bande
    /// `[-scroll_y, list.height]` (items ancrés `-scroll_y`
    /// au build, clamp négatif relâché R-140), bornée à
    /// `MAX_FRAME_DIM / scale` px CSS : au-delà, `WARN` +
    /// repli sur la tranche stockée (contrat `full=1`
    /// historique). Le PNG plein-doc est **transitoire** —
    /// jamais rangé dans `entry.png` qui reste le miroir
    /// strict de `entry.frame`.
    pub fn frame_png_full(&mut self, tab: TabId) -> Option<Vec<u8>> {
        self.ensure_painted(tab);
        if !self.tabs.get(&tab).is_some_and(|t| t.windowed) {
            return self.frame_png(tab);
        }
        let fonts = self.apply_ctx().fonts;
        let scale = self.scale.max(f32::EPSILON);
        let entry = self.tabs.get_mut(&tab)?;
        // Pull consommateur (Étape 222) — raster plein-doc.
        entry.last_frame_pull = Some(std::time::Instant::now());
        let list = entry.list.as_ref()?;
        if list.height > MAX_FRAME_DIM / scale {
            let (list_h, doc_h) = (list.height, entry.doc_h);
            tracing::warn!(
                tab,
                doc_h,
                list_h,
                "frame_png_full : document au-delà de la borne — tranche viewport servie"
            );
            return self.frame_png(tab);
        }
        let frame = explorer_render::rasterize_window_cached(
            list,
            &fonts as &dyn explorer_layout::font::FontSource,
            scale,
            -entry.painted_scroll_y,
            list.height,
            &mut entry.font_cache,
        )?;
        let png = frame.encode_png();
        if png.as_ref().is_none_or(Vec::is_empty) {
            tracing::warn!(
                tab,
                w = frame.width,
                h = frame.height,
                "frame_png_full : encodage vide ou échoué"
            );
            return None;
        }
        png
    }

    /// Auto-réparation (étape 56b rouvert, terrain mission
    /// 13) : un **DOM commis** sans produit peint — fenêtre
    /// `nav_apply` → `sub_commit`, `unspec` sur relayout
    /// impossible, tout chemin qui jette la frame — re-peint
    /// paresseusement à la première demande plutôt que de
    /// servir `/frame` 204 avec hit-test mort à vie. No-op
    /// si une frame existe ou si rien n'est commis ; si le
    /// DOM commis est réellement non-layable, le relayout
    /// échoue et l'appelant sert un 204 honnête.
    pub(crate) fn ensure_painted(&mut self, tab: TabId) {
        let needs = self
            .tabs
            .get(&tab)
            .is_some_and(|e| e.dom.is_some() && e.frame.is_none());
        if needs {
            let _ = self.relayout(tab);
        }
    }

    /// Frame brute courante `(largeur, hauteur, RGBA)` —
    /// `None` si jamais navigué. Canal FFI/texture (étape
    /// 47, ADR-0018) : les pixels restent empruntés, la
    /// copie est à la charge de l'appelant.
    #[must_use]
    pub fn frame_pixels(&self, tab: TabId) -> Option<(u32, u32, &[u8])> {
        self.tabs.get(&tab).and_then(|t| {
            t.frame
                .as_ref()
                .map(|f| (f.width, f.height, f.pixels.as_slice()))
        })
    }

    /// Extrait la région `(x, y, w, h)` — pixels physiques —
    /// de la frame courante (étape 55 : copie partielle des
    /// dirty-rects). Clippée aux bornes ; `None` si pas de
    /// frame ou région vide hors frame.
    #[must_use]
    pub fn frame_region(
        &self,
        tab: TabId,
        x: u32,
        y: u32,
        w: u32,
        h: u32,
    ) -> Option<(u32, u32, Vec<u8>)> {
        let frame = self.tabs.get(&tab)?.frame.as_ref()?;
        let rw = w.min(frame.width.saturating_sub(x));
        let rh = h.min(frame.height.saturating_sub(y));
        if rw == 0 || rh == 0 {
            return None;
        }
        let stride = frame.width as usize * 4;
        let row_bytes = rw as usize * 4;
        let mut out = Vec::with_capacity(row_bytes * rh as usize);
        for row in y..y + rh {
            let start = row as usize * stride + x as usize * 4;
            out.extend_from_slice(&frame.pixels[start..start + row_bytes]);
        }
        Some((rw, rh, out))
    }

    /// PNG de la région `(x, y, w, h)` — pixels physiques —
    /// (étape 55, pull partiel REST `frame?x=&y=&w=&h=`).
    /// `None` si pas de frame ou région hors bornes.
    #[must_use]
    pub fn frame_png_region(&self, tab: TabId, x: u32, y: u32, w: u32, h: u32) -> Option<Vec<u8>> {
        let (w2, h2, px) = self.frame_region(tab, x, y, w, h)?;
        Frame {
            width: w2,
            height: h2,
            pixels: px,
            scale: 1.0,
        }
        .encode_png()
    }

    /// Vide et renvoie les régions invalides accumulées —
    /// pixels physiques (étape 55). Canal de pull pour un
    /// client sans abonnement aux événements (FFI) : il
    /// copie ensuite chaque région via [`Self::frame_region`].
    /// Vide = rien à soumettre depuis le dernier appel.
    /// Indépendant des émissions (étape 55d — chaque
    /// `FrameInvalidated` n'avance que le watermark
    /// `emitted`). Limite connue : après un `scroll` les
    /// régions couvrent le document entier — le défilement
    /// est cuit dans les coordonnées raster, le pull
    /// partiel d'un scroll n'a pas de sens.
    pub fn take_dirty(&mut self, tab: TabId) -> Vec<FrameRegion> {
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return Vec::new();
        };
        let s = self.scale.max(f32::EPSILON);
        entry.emitted = 0;
        entry
            .dirty
            .drain(..)
            .map(|r| FrameRegion {
                x: r.x * s,
                y: r.y * s,
                w: r.width * s,
                h: r.height * s,
            })
            .collect()
    }

    /// Émet `FrameInvalidated` avec le **delta** des régions
    /// invalides depuis l'émission précédente (étape 55d) —
    /// les rects internes sont en px CSS (`changed_rects`),
    /// le contrat sort en pixels physiques (`× scale`,
    /// comme `frame_pixels`). Watermark `emitted`, pas drain :
    /// le canal pull FFI (`take_dirty`) vide sa propre
    /// accumulation sans être affamé par les événements (le
    /// handle FFI garde un receveur inerte du bus).
    pub(crate) fn emit_invalidated(&mut self, tab: TabId) {
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return;
        };
        let s = self.scale.max(f32::EPSILON);
        let rects = entry
            .dirty
            .get(entry.emitted.min(entry.dirty.len())..)
            .unwrap_or_default()
            .iter()
            .map(|r| FrameRegion {
                x: r.x * s,
                y: r.y * s,
                w: r.width * s,
                h: r.height * s,
            })
            .collect();
        entry.emitted = entry.dirty.len();
        // `scroll_y` de la frame peinte — l'hôte positionne
        // les overlays média à `rect − scroll_y` (contrat 71).
        let scroll_y = entry.scroll_y;
        let _ = self.events.send(CoreEvent::FrameInvalidated {
            tab,
            rects,
            scroll_y,
        });
    }
}
