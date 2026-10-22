// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Display list (étape 22) : l'arbre de layout devient une
//! **liste ordonnée de primitives de dessin** — la seule
//! chose que la rasterisation (étape 23) consomme.
//!
//! Ordre de peinture — simplification bornée de CSS 2.1
//! Appendix E :
//! 1. fond (`background-color`, border-box) puis bordures
//!    de la boîte ;
//! 2. texte des lignes en-ligne (fragments non atomiques) ;
//! 3. enfants **non positionnés** dans l'ordre de l'arbre
//!    (les flottants y coulent à leur place — approximation
//!    documentée) ;
//! 4. enfants **positionnés** (`relative`/`absolute`/`fixed`)
//!    triés stablement par `z-index` croissant — un contexte
//!    d'empilement complet (transforms, opacity) est reporté.
//!
//! `overflow` ≠ `visible` → `PushClip`/`PopClip` autour des
//! descendants (boîte de padding, approximation sans
//! arrondis). `display: none` ne produit déjà pas de boîte.

use explorer_css::value::Color;
use explorer_css::value::{parse_color, parse_length, Unit};
use explorer_layout::font::ShapedRun;
use explorer_layout::{BoxKind, Float, LayoutBox, Overflow, Position, Rect};

/// Une primitive de dessin, dans l'ordre de peinture.
#[derive(Debug, Clone)]
pub enum DisplayItem {
    /// Rectangle plein (fond, côté de bordure, placeholder).
    FillRect {
        /// Zone couverte (border-box pour un fond).
        rect: Rect,
        /// Couleur RGBA.
        color: Color,
    },
    /// Bordure d'une boîte — 4 côtés uniformisés en un item.
    Border {
        /// Border box extérieure.
        rect: Rect,
        /// Épaisseurs `[haut, droite, bas, gauche]`.
        widths: [f32; 4],
        /// Couleurs par côté (même ordre).
        colors: [Color; 4],
        /// Style (`solid`/`dashed`/`dotted`/…) par côté.
        styles: [BorderStyle; 4],
    },
    /// Run de glyphes shapés, posé depuis l'origine
    /// baseline (`x`, `y` = ligne de base).
    Text {
        /// Origine : x du fragment, y de la baseline.
        origin: (f32, f32),
        /// Glyphes + métriques produits par le shaping —
        /// `Arc` : le clone d'item est O(1) (R-37, re-usage
        /// de plages dans le build incrémental).
        run: std::sync::Arc<ShapedRun>,
        /// Requête fonte — le raster résout les octets via
        /// `FontSource` pour les contours de glyphes.
        font: explorer_layout::font::FontRequest,
        /// `color` hérité du conteneur.
        color: Color,
        /// Taille utilisée (px) — pour le raster de glyphes.
        size_px: f32,
    },
    /// Contenu remplacé non encore dessiné (`img`, `iframe`) —
    /// cadre + étiquette pour le wireframe.
    Placeholder {
        /// Zone réservée.
        rect: Rect,
        /// Étiquette (`alt`, tag…).
        label: String,
    },
    /// Bitmap décodée, blittée dans `rect` (mise à l'échelle
    /// nearest — aucun lissage à cette étape).
    Image {
        /// Zone de destination (border box de l'élément).
        rect: Rect,
        /// Pixels décodés.
        img: std::sync::Arc<crate::image::DecodedImage>,
    },
    /// `background-image` décodée, tuilée dans `rect` selon
    /// `background-repeat` — origine padding box, aire border
    /// box (`background-size` hors portée — taille naturelle).
    BackgroundImage {
        /// Aire de peinture (border box de la boîte porteuse) —
        /// les tuiles n'apparaissent que dedans.
        rect: Rect,
        /// Pixels décodés.
        img: std::sync::Arc<crate::image::DecodedImage>,
        /// Répétition `(x, y)` de `background-repeat`.
        repeat: (bool, bool),
        /// Coin haut-gauche de la tuile (0,0) en coords document
        /// — inclut `background-position` résolu ; pour
        /// `background-attachment: fixed`, la zone de
        /// positionnement est le viewport (R-13d).
        origin: (f32, f32),
    },
    /// `background-color` passé par l'alpha d'un `mask-image`
    /// (R-52) : la couleur n'apparaît que là où le mask est
    /// opaque — motif « icône colorée » des design systems
    /// (`mask-image:url(icon.svg)` + `background-color` ; sans
    /// le mask la couleur remplirait la boîte entière → carré
    /// plein). `mask-size`/`mask-position` hors portée : le
    /// mask est échantillonné à la taille de `rect`, ce qui
    /// couvre le motif (la boîte est dimensionnée à l'icône).
    MaskedFill {
        /// Boîte de destination (border box) — le mask y est
        /// mis à l'échelle en nearest.
        rect: Rect,
        /// `background-color` résolue.
        color: Color,
        /// Bitmap du mask — seul l'alpha est lu.
        mask: std::sync::Arc<crate::image::DecodedImage>,
    },
    /// Entrée de zone de clipping (`overflow` ≠ `visible`).
    PushClip(Rect),
    /// Sortie de la zone de clipping correspondante.
    PopClip,
    /// `transform: …` — matrice affine (origine déjà résolue
    /// dedans) appliquée aux items jusqu'au `PopTransform`
    /// correspondant ; les matrices s'empilent (imbrication).
    PushTransform(tiny_skia::Transform),
    /// Fin de portée du dernier `PushTransform`.
    PopTransform,
}

/// Style de trait de bordure (sous-ensemble rasterisable).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderStyle {
    /// Pas de trait.
    None,
    /// Plein.
    Solid,
    /// Pointillé (`dotted`/`dashed` — pattern au raster).
    Dotted,
    /// Double liseré.
    Double,
}

/// La frame prête à rasteriser : items + taille du viewport.
#[derive(Debug, Clone)]
pub struct DisplayList {
    /// Primitives dans l'ordre de peinture.
    pub items: Vec<DisplayItem>,
    /// Largeur du viewport (px).
    pub width: f32,
    /// Hauteur du document posé (px) — scrollable au-delà
    /// du viewport.
    pub height: f32,
    /// Correspondance boîte → plage d'items pour le build
    /// incrémental (R-37) — clé = chemin d'index enfants
    /// depuis la racine (stable aux re-mappages `NodeId`,
    /// conservateur aux insertions : un décalage produit
    /// un *miss*, jamais une copie fautive).
    pub spans: SpanMap,
    /// Epoch de l'arbre de boîtes peint (compteur côté
    /// appelant, bumpé au remplacement — la mutation
    /// in-place du chemin `relayout_subtree` conserve
    /// l'epoch). Égalité ⇒ fast-path pointeur autorisé.
    pub tree_epoch: u64,
}

/// Plage d'items émise par une boîte — indexe le re-usage
/// de `build_display_list_incremental` (R-37).
#[derive(Debug, Clone)]
pub struct SpanRec {
    /// Début de la plage dans `items`.
    pub start: u32,
    /// Fin de la plage (exclue).
    pub end: u32,
    /// `bx.rect` à l'émission (coords document, scroll exclu).
    pub rect: Rect,
    /// `off` utilisé à l'émission (`-scroll`, `(0,0)` sous
    /// `position: fixed`).
    pub off: (f32, f32),
    /// Fingerprint des champs peints — voir `box_sig`.
    pub sig: u64,
    /// `F_*` — restrictions de ré-usage.
    pub flags: u8,
    /// Adresse de la boîte à l'émission — fast-path quand
    /// l'arbre est **le même objet** (`tree_epoch` inchangé,
    /// mutation in-place du chemin `relayout_subtree`) :
    /// contrat — hors `{rect, node, lines, slot}`, aucun
    /// champ peint n'est muté in-place sur une boîte
    /// préservée (asserté en debug par `reuse_delta`).
    pub bx_ptr: usize,
    /// `bx.lines.as_ptr()` — `layout_block` réécrit `lines`
    /// (nouvelle allocation) sur les ancêtres re-posés :
    /// ptr différent ⇒ contenu inline potentiellement refait.
    pub lines_ptr: usize,
    /// `bx.children.as_ptr()` — le discriminant du fast-path :
    /// `*bx = fresh` (boîte sale remplacée par
    /// `relayout_subtree`) conserve l'adresse du *slot* mais
    /// le Vec enfants de `fresh` est alloué pendant que
    /// l'ancien vit encore — adresses disjointes garanties.
    /// `usize::MAX` quand `children` est vide : le ptr
    /// dangling ne discrimine pas — la feuille retombe alors
    /// sur `lines_ptr`, et les deux vides → signature.
    pub kids_ptr: usize,
    /// `bx.children.len()` — insertion d'enfants.
    pub kids_len: u32,
    /// Chemins des porteurs inline reportés au `defer`
    /// **hérité** pendant l'émission de la plage — relatifs
    /// à la boîte (préfixe = son chemin). Une copie plate
    /// les re-pousse dans le `defer` courant pour que le
    /// drain englobant les ré-émette (via leurs propres
    /// spans) — sinon le contenu en ligne disparaîtrait.
    pub escaped: Box<[Box<[u16]>]>,
}

/// Boîte `position: fixed` — sa plage n'est **pas**
/// translatée au scroll.
pub const F_FIXED: u8 = 1;
/// Ré-usage interdit (média remplacé dont le bitmap arrive
/// tard, `background-attachment: fixed` dont l'origine est
/// ancrée viewport) — la boîte se ré-émet toujours.
pub const F_NO_REUSE: u8 = 2;
/// La boîte a un enfant `position: sticky` — ses items
/// portent le `PushTransform` du décalage sticky, non
/// linéaire en scroll : toute translation non nulle
/// impose une ré-émission.
pub const F_STICKY_KID: u8 = 4;

impl DisplayList {
    /// Dump texte indenté — debug/golden sans raster.
    #[must_use]
    pub fn dump(&self) -> String {
        let mut out = String::new();
        for item in &self.items {
            use std::fmt::Write as _;
            match item {
                DisplayItem::FillRect { rect, color } => {
                    let _ = writeln!(out, "fill {rect:?} #{color:?}");
                }
                DisplayItem::Border { rect, .. } => {
                    let _ = writeln!(out, "border {rect:?}");
                }
                DisplayItem::Text { origin, run, .. } => {
                    let _ = writeln!(
                        out,
                        "text @{:.1},{:.1} ({} glyphs)",
                        origin.0,
                        origin.1,
                        run.glyphs.len()
                    );
                }
                DisplayItem::Placeholder { rect, label } => {
                    let _ = writeln!(out, "placeholder {label} {rect:?}");
                }
                DisplayItem::Image { rect, img } => {
                    let _ = writeln!(out, "image {}x{} {rect:?}", img.width, img.height);
                }
                DisplayItem::BackgroundImage { rect, img, .. } => {
                    let _ = writeln!(out, "bg-image {}x{} {rect:?}", img.width, img.height);
                }
                DisplayItem::MaskedFill { rect, mask, .. } => {
                    let _ = writeln!(out, "masked-fill {}x{} {rect:?}", mask.width, mask.height);
                }
                DisplayItem::PushClip(r) => {
                    let _ = writeln!(out, "push-clip {r:?}");
                }
                DisplayItem::PopClip => out.push_str("pop-clip\n"),
                DisplayItem::PushTransform(m) => {
                    let _ = writeln!(out, "push-transform {m:?}");
                }
                DisplayItem::PopTransform => out.push_str("pop-transform\n"),
            }
        }
        out
    }

    /// Régions à repeindre entre `prev` et `self` — dirty-rects
    /// (R-7). Chaque item est résolu dans son contexte de peinture
    /// (pile `PushTransform`/`PushClip`, même ordre
    /// `pre_concat` que le raster) ; quand un item diffère — ou
    /// que sa zone résolue a bougé — les zones ancienne **et**
    /// nouvelle sont rapportées. Les rects retournés sont dans
    /// l'espace framebuffer (scroll déjà cuit), fusionnés par
    /// intersection ; à l'appelant d'intersecter sa cible.
    /// Listes identiques → `vec![]` (frame propre, rien à
    /// soumettre).
    #[must_use]
    pub fn changed_rects(&self, prev: &DisplayList) -> Vec<Rect> {
        let (old, new) = (resolved_bounds(&prev.items), resolved_bounds(&self.items));
        let mut out = Vec::new();
        for i in 0..self.items.len().max(prev.items.len()) {
            let same = match (self.items.get(i), prev.items.get(i)) {
                (Some(a), Some(b)) => item_eq(a, b) && new[i] == old[i],
                (None, None) => true,
                _ => false,
            };
            if !same {
                if let Some(r) = old.get(i).copied().flatten() {
                    out.push(r);
                }
                if let Some(r) = new.get(i).copied().flatten() {
                    out.push(r);
                }
            }
        }
        merge_rects(&mut out);
        out
    }
}

/// Zone peinte de chaque item, transform/clip résolus — `None`
/// pour les items d'état (`Push*/Pop*`). Rejoue la même
/// sémantique de pile que `rasterize` (transforms en
/// `pre_concat`, clips poussés dans leur transform courant).
/// `pub(crate)` : `rasterize_dirty` (R-37) filtre les items
/// par intersection avec la fenêtre sale.
pub(crate) fn resolved_bounds(items: &[DisplayItem]) -> Vec<Option<Rect>> {
    let mut tfs: Vec<tiny_skia::Transform> = Vec::new();
    let mut clips: Vec<Rect> = Vec::new();
    let mut cur = tiny_skia::Transform::default();
    items
        .iter()
        .map(|item| match item {
            DisplayItem::PushTransform(m) => {
                tfs.push(*m);
                cur = tfs
                    .iter()
                    .fold(tiny_skia::Transform::default(), |acc, t| acc.pre_concat(*t));
                None
            }
            DisplayItem::PopTransform => {
                tfs.pop();
                cur = tfs
                    .iter()
                    .fold(tiny_skia::Transform::default(), |acc, t| acc.pre_concat(*t));
                None
            }
            DisplayItem::PushClip(r) => {
                clips.push(apply_tf(*r, cur));
                None
            }
            DisplayItem::PopClip => {
                clips.pop();
                None
            }
            _ => item_bounds(item).map(|r| {
                let r = apply_tf(r, cur);
                clips
                    .iter()
                    .fold(r, |a, c| rect_intersect(a, *c).unwrap_or(a))
            }),
        })
        .collect()
}

/// Zone couverte par un item de dessin, avant transform/clip.
fn item_bounds(item: &DisplayItem) -> Option<Rect> {
    match item {
        DisplayItem::FillRect { rect, .. }
        | DisplayItem::Border { rect, .. }
        | DisplayItem::Placeholder { rect, .. }
        | DisplayItem::Image { rect, .. }
        | DisplayItem::MaskedFill { rect, .. }
        | DisplayItem::BackgroundImage { rect, .. } => Some(*rect),
        DisplayItem::Text { origin, run, .. } => Some(match run.ink {
            // R-37 : bbox d'encre réelle (glyphes) plutôt que
            // la boîte de métriques — couvre l'overhang
            // (italique, accents, plein-em) sans inflation.
            // Repli boîte métrique si le run n'a pas de
            // contours (espaces) ou fonte sans bbox.
            Some(ink) => Rect {
                x: origin.0 + ink.x,
                y: origin.1 + ink.y,
                width: ink.width,
                height: ink.height,
            },
            None => Rect {
                x: origin.0,
                y: origin.1 - run.metrics.ascent,
                width: run.advance,
                height: run.metrics.ascent - run.metrics.descent,
            },
        }),
        DisplayItem::PushClip(_)
        | DisplayItem::PopClip
        | DisplayItem::PushTransform(_)
        | DisplayItem::PopTransform => None,
    }
}

/// Applique une matrice affine à un `Rect` — AABB des quatre
/// coins transformés (la rotation élargit, comme le raster).
fn apply_tf(r: Rect, t: tiny_skia::Transform) -> Rect {
    let (mut lx, mut ly, mut hx, mut hy) = (
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    );
    for (cx, cy) in [
        (r.x, r.y),
        (r.x + r.width, r.y),
        (r.x, r.y + r.height),
        (r.x + r.width, r.y + r.height),
    ] {
        let (px, py) = (t.sx * cx + t.kx * cy + t.tx, t.ky * cx + t.sy * cy + t.ty);
        lx = lx.min(px);
        ly = ly.min(py);
        hx = hx.max(px);
        hy = hy.max(py);
    }
    Rect {
        x: lx,
        y: ly,
        width: hx - lx,
        height: hy - ly,
    }
}

/// Égalité visuelle de deux items — `Arc` comparés par
/// identité (pixels différents = arc différent ⇒ dirty).
fn item_eq(a: &DisplayItem, b: &DisplayItem) -> bool {
    use DisplayItem as D;
    match (a, b) {
        (
            D::FillRect {
                rect: r1,
                color: c1,
            },
            D::FillRect {
                rect: r2,
                color: c2,
            },
        ) => r1 == r2 && c1 == c2,
        (
            D::Border {
                rect: r1,
                widths: w1,
                colors: c1,
                styles: s1,
            },
            D::Border {
                rect: r2,
                widths: w2,
                colors: c2,
                styles: s2,
            },
        ) => r1 == r2 && w1 == w2 && c1 == c2 && s1 == s2,
        (
            D::Text {
                origin: o1,
                run: ru1,
                font: f1,
                color: c1,
                size_px: p1,
            },
            D::Text {
                origin: o2,
                run: ru2,
                font: f2,
                color: c2,
                size_px: p2,
            },
        ) => o1 == o2 && ru1 == ru2 && f1 == f2 && c1 == c2 && p1 == p2,
        (
            D::Placeholder {
                rect: r1,
                label: l1,
            },
            D::Placeholder {
                rect: r2,
                label: l2,
            },
        ) => r1 == r2 && l1 == l2,
        (D::Image { rect: r1, img: i1 }, D::Image { rect: r2, img: i2 }) => {
            r1 == r2 && std::sync::Arc::ptr_eq(i1, i2)
        }
        (
            D::BackgroundImage {
                rect: r1,
                img: i1,
                repeat: p1,
                origin: o1,
            },
            D::BackgroundImage {
                rect: r2,
                img: i2,
                repeat: p2,
                origin: o2,
            },
        ) => r1 == r2 && p1 == p2 && o1 == o2 && std::sync::Arc::ptr_eq(i1, i2),
        (
            D::MaskedFill {
                rect: r1,
                color: c1,
                mask: m1,
            },
            D::MaskedFill {
                rect: r2,
                color: c2,
                mask: m2,
            },
        ) => r1 == r2 && c1 == c2 && std::sync::Arc::ptr_eq(m1, m2),
        (D::PushClip(r1), D::PushClip(r2)) => r1 == r2,
        (D::PushTransform(m1), D::PushTransform(m2)) => {
            (m1.sx, m1.ky, m1.kx, m1.sy, m1.tx, m1.ty) == (m2.sx, m2.ky, m2.kx, m2.sy, m2.tx, m2.ty)
        }
        _ => std::mem::discriminant(a) == std::mem::discriminant(b),
    }
}

/// Intersection de deux rects — `None` si disjoints.
fn rect_intersect(a: Rect, b: Rect) -> Option<Rect> {
    let (x1, y1) = (a.x.max(b.x), a.y.max(b.y));
    let (x2, y2) = (
        (a.x + a.width).min(b.x + b.width),
        (a.y + a.height).min(b.y + b.height),
    );
    (x2 > x1 && y2 > y1).then_some(Rect {
        x: x1,
        y: y1,
        width: x2 - x1,
        height: y2 - y1,
    })
}

/// Union englobante de deux rects.
fn rect_union(a: Rect, b: Rect) -> Rect {
    let (x1, y1) = (a.x.min(b.x), a.y.min(b.y));
    let (x2, y2) = (
        (a.x + a.width).max(b.x + b.width),
        (a.y + a.height).max(b.y + b.height),
    );
    Rect {
        x: x1,
        y: y1,
        width: x2 - x1,
        height: y2 - y1,
    }
}

/// Fusionne les rects qui se chevauchent — balayage trié
/// par `x` (un rect ne se compare qu'aux composantes encore
/// ouvertes horizontalement) puis consolidation bornée en
/// point fixe. O(k log k + k·ouvertes + C²·passes) avec
/// C ≪ k, au lieu du O(k²) historique (mesuré ~2 s sur
/// ~8 500 rects d'un diff dégénéré, dette journal NAV 55g).
/// Couverture identique ; la partition peut fusionner
/// davantage que la passe unique d'avant (qui n'atteignait
/// pas le point fixe qu'elle annonçait).
fn merge_rects(rects: &mut Vec<Rect>) {
    if rects.len() < 2 {
        return;
    }
    let mut order = std::mem::take(rects);
    order.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));
    let mut comps: Vec<Rect> = Vec::new();
    for r in order {
        let mut hits: Vec<usize> = Vec::new();
        let mut dead = 0usize;
        for (i, c) in comps.iter().enumerate() {
            if c.x + c.width < r.x {
                // Fermée : aucun rect futur (triés par x) ne
                // peut la toucher — marquée pour compaction.
                dead += 1;
                continue;
            }
            if r.y < c.y + c.height && c.y < r.y + r.height {
                hits.push(i);
            }
        }
        match hits.split_first() {
            None => comps.push(r),
            Some((&first, rest)) => {
                let mut bb = rect_union(comps[first], r);
                // `r` relie plusieurs composantes → elles
                // fusionnent dans `first` (retrait en index
                // décroissant : les suivants restent valides).
                for &i in rest.iter().rev() {
                    bb = rect_union(bb, comps[i]);
                    comps[i] = comps[comps.len() - 1];
                    comps.pop();
                }
                comps[first] = bb;
            }
        }
        // Compaction amortie : purge les composantes fermées
        // quand elles dominent — borne le balayage courant.
        if comps.len() > 64 && dead * 2 >= comps.len() {
            comps.retain(|c| c.x + c.width >= r.x);
        }
    }
    // Consolidation : une composante qui a grossi vers la
    // gauche peut encore chevaucher une voisine fermée.
    // Point fixe borné sur les C ≪ k survivantes.
    for _ in 0..4 {
        let mut merged = false;
        let mut i = 0;
        while i < comps.len() {
            let mut j = i + 1;
            while j < comps.len() {
                if rect_intersect(comps[i], comps[j]).is_some() {
                    comps[i] = rect_union(comps[i], comps[j]);
                    comps.swap_remove(j);
                    merged = true;
                } else {
                    j += 1;
                }
            }
            i += 1;
        }
        if !merged {
            break;
        }
    }
    *rects = comps;
}

/// Construit la display list d'un arbre posé — pur,
/// déterministe, borné par la taille de l'arbre. `height`
/// couvre au moins le viewport. `scroll_y` décale tout le
/// contenu vers le haut, **sauf** les sous-arbres
/// `position: fixed` qui restent ancrés au viewport.
/// `images` = pixels décodés par `NodeId` `<img>` — une
/// boîte remplacée (`intrinsic` posé) peint son bitmap.
/// [`build_display_list`] équivaut à une map vide.
///
/// L'arbre posé d'un document **en construction** est une entrée
/// supportée (R-1) : la liste reflète l'instantané `&LayoutBox`
/// reçu — jamais l'arbre DOM vivant — donc une frame peinte
/// reste cohérente quand le parse reprend ensuite.
#[must_use]
pub fn build_display_list_imaged(
    root: &LayoutBox,
    viewport_w: f32,
    viewport_h: f32,
    scroll_y: f32,
    images: &crate::ImageTable,
) -> DisplayList {
    build_display_list_full(
        root,
        viewport_w,
        viewport_h,
        scroll_y,
        &crate::PaintImages::from_replaced(images.clone()),
    )
}

/// Variante complète : `images` porte les `<img>` décodés ET
/// les `background-image` résolus (par `NodeId`). [`build_display_list`]
/// équivaut à des tables vides.
#[must_use]
pub fn build_display_list_full(
    root: &LayoutBox,
    viewport_w: f32,
    viewport_h: f32,
    scroll_y: f32,
    images: &crate::PaintImages,
) -> DisplayList {
    build_display_list_scrolled(root, viewport_w, viewport_h, (0.0, scroll_y), images)
}

/// Variante à scroll horizontal : `scroll = (x, y)` — les
/// seuils sticky `left`/`right` le consomment (R-6).
#[must_use]
pub fn build_display_list_scrolled(
    root: &LayoutBox,
    viewport_w: f32,
    viewport_h: f32,
    scroll: (f32, f32),
    images: &crate::PaintImages,
) -> DisplayList {
    build_list(root, viewport_w, viewport_h, scroll, images, None, 0)
}

/// Variante incrémentale (R-37) : `prev` fournit les plages
/// de la peinture précédente — les sous-arbres inchangés
/// (même `sig`, même taille) sont **copiés** au lieu d'être
/// ré-émis, éventuellement translatés. `prev` ignorée si la
/// largeur diffère (les coords viewport-ancrées bougent).
///
/// Correction du ré-usage : une plage n'est copiée que si
/// **tous** les descendants partagent le même Δ (translation
/// rigide — un `fixed` interne a Δ=0 et fait descendre le
/// parent) ; contenu/style/lignes sont fingerprintés par
/// `box_sig`. L'équivalence « liste incrémentale == liste
/// pleine » est assertée par `tests/dirty_raster.rs`.
#[must_use]
pub fn build_display_list_incremental(
    root: &LayoutBox,
    viewport_w: f32,
    viewport_h: f32,
    scroll: (f32, f32),
    images: &crate::PaintImages,
    prev: Option<&DisplayList>,
    tree_epoch: u64,
) -> DisplayList {
    build_list(
        root, viewport_w, viewport_h, scroll, images, prev, tree_epoch,
    )
}

/// §14.2 : émet le fond du canvas (couleur puis image) et
/// renvoie le `NodeId` de la boîte source — racine si elle a
/// un fond, sinon son premier enfant bloc (`body`) — pour que
/// `paint_self` ne le repeigne pas sur sa boîte propre.
/// `canvas` = rect de peinture (viewport × hauteur document).
#[allow(clippy::cast_precision_loss)] // dims image ≪ 2^23
fn emit_canvas_bg(
    root: &LayoutBox,
    canvas: Rect,
    viewport_w: f32,
    viewport_h: f32,
    images: &crate::PaintImages,
    items: &mut Vec<DisplayItem>,
) -> Option<explorer_dom::NodeId> {
    let (canvas_from_root, canvas_bx) = if has_bg(&root.style) {
        (true, Some(root))
    } else {
        (
            false,
            root.children
                .iter()
                .find(|c| c.kind == BoxKind::Block && has_bg(&c.style)),
        )
    };
    let src = canvas_bx?;
    if let Some(c) = src.style.get("background-color").and_then(parse_color) {
        if c.a > 0 {
            items.push(DisplayItem::FillRect {
                rect: canvas,
                color: c,
            });
        }
    }
    if src.style.get("background-image").is_some_and(bg_has_url) {
        if let Some(img) = src.node.and_then(|n| images.backgrounds.get(&n)).cloned() {
            let repeat = match src.style.get("background-repeat") {
                Some("no-repeat") => (false, false),
                Some("repeat-x") => (true, false),
                Some("repeat-y") => (false, true),
                _ => (true, true),
            };
            // Origine de la tuile : padding box de la racine
            // pour SON fond (le `0 -1em` de `margin-collapse-020`
            // se mesure depuis son propre bord) ; canvas pour
            // le fond propagé de `body`.
            let (area, base) = if canvas_from_root {
                let bl = border_side_w(&src.style, "left");
                let bt = border_side_w(&src.style, "top");
                let br = border_side_w(&src.style, "right");
                let bb = border_side_w(&src.style, "bottom");
                (
                    (
                        (src.rect.width - bl - br).max(0.0),
                        (src.rect.height - bt - bb).max(0.0),
                    ),
                    (src.rect.x + bl, src.rect.y + bt),
                )
            } else {
                ((viewport_w, viewport_h), (0.0, 0.0))
            };
            let (px, py) = bg_position(
                src.style.get("background-position"),
                area,
                (img.width as f32, img.height as f32),
                font_size_of(&src.style),
            );
            items.push(DisplayItem::BackgroundImage {
                rect: canvas,
                img,
                repeat,
                origin: (base.0 + px, base.1 + py),
            });
        }
    }
    src.node
}

/// Corps partagé des builders — `prev = None` : build plein.
fn build_list(
    root: &LayoutBox,
    viewport_w: f32,
    viewport_h: f32,
    scroll: (f32, f32),
    images: &crate::PaintImages,
    prev: Option<&DisplayList>,
    tree_epoch: u64,
) -> DisplayList {
    let mut items = Vec::new();
    let mut spans = SpanMap::default();
    let same_tree = prev.is_some_and(|p| p.tree_epoch == tree_epoch);
    let inc = prev
        .filter(|p| p.width.to_bits() == viewport_w.to_bits())
        .map(|p| IncCx {
            items: &p.items,
            spans: &p.spans,
        });
    let mut cx = PaintCx {
        vp: (viewport_w, viewport_h),
        images,
        spans: &mut spans,
        path: Vec::new(),
        inc,
        same_tree,
        canvas_bg_src: None,
    };
    // §14.2 : le fond du canvas vient de la racine ; si elle
    // n'en a pas (couleur transparente et pas d'image), c'est
    // celui de son premier enfant bloc (`body`) qui propage —
    // ancré à l'origine du canvas alors que le fond de la
    // racine s'ancre à sa propre padding box
    // (`margin-collapse-021` : le fond de `body` positionné
    // `no-repeat` part du coin du canvas, pas de sa boîte
    // décalée par sa marge).
    let canvas = Rect {
        x: 0.0,
        y: 0.0,
        width: viewport_w,
        height: root.rect.height.max(viewport_h),
    };
    cx.canvas_bg_src = emit_canvas_bg(root, canvas, viewport_w, viewport_h, images, &mut items);
    // Portées racine des reports : inlines (App. E étape 5)
    // et positionnés `z ≥ 0` (étapes 8-9 — la racine ferme
    // toujours le dernier contexte d'empilement).
    let mut defer = Vec::new();
    let mut pos_defer = Vec::new();
    paint_box(
        root,
        &mut items,
        (-scroll.0, -scroll.1),
        &mut cx,
        &mut defer,
        &mut pos_defer,
    );
    {
        // Span racine — `escaped` = les porteurs poussés aux
        // files racine (chemins absolus = relatifs à la racine).
        let mut esc = escaped_paths(&defer, 0, &cx.path).into_vec();
        esc.extend(escaped_paths(&pos_defer, 0, &cx.path).into_vec());
        let end = u32::try_from(items.len()).unwrap_or(u32::MAX);
        let sig = box_sig(root);
        cx.record_span(
            root,
            0,
            end,
            (-scroll.0, -scroll.1),
            sig,
            esc.into_boxed_slice(),
        );
    }
    let mut i = 0;
    while i < defer.len() {
        let (p, c) = defer[i].clone();
        i += 1;
        emit_scoped_at(
            p,
            c,
            &mut items,
            (-scroll.0, -scroll.1),
            &mut cx,
            &mut pos_defer,
        );
    }
    // Positionnés reportés — déjà triés stable par z (`push_z`).
    let mut i = 0;
    while i < pos_defer.len() {
        let (p, c) = pos_defer[i].clone();
        i += 1;
        emit_scoped_at(
            p,
            c,
            &mut items,
            (-scroll.0, -scroll.1),
            &mut cx,
            &mut pos_defer,
        );
    }
    DisplayList {
        items,
        width: viewport_w,
        height: root.rect.height.max(viewport_h),
        spans,
        tree_epoch,
    }
}

/// Contexte d'une passe de peinture — viewport, images,
/// collecteur de `spans`, chemin courant (clé des spans)
/// et source de ré-usage incrémental éventuelle (R-37).
struct PaintCx<'a> {
    /// Viewport `(w, h)` — consommé par sticky et les fonds
    /// `background-attachment: fixed`.
    vp: (f32, f32),
    /// Bitmaps par `NodeId` (remplacés, fonds, svg, canvas).
    images: &'a crate::PaintImages,
    /// Spans produits par cette passe.
    spans: &'a mut SpanMap,
    /// Chemin d'index-enfants de la boîte courante — la clé
    /// de span (un `u16` par niveau, saturé — l'overflow est
    /// un miss, jamais une collision fausse).
    path: Vec<u16>,
    /// Items + spans de la peinture précédente.
    inc: Option<IncCx<'a>>,
    /// `prev.tree_epoch == epoch` — l'arbre est le même objet
    /// : le fast-path `bx_ptr` est valide (sinon une adresse
    /// recyclée pourrait mentir).
    same_tree: bool,
    /// Nœud dont le fond a été propagé au canvas (§14.2 —
    /// racine, ou `body` quand la racine n'a pas de fond) :
    /// sa propre boîte ne repeint pas son fond.
    canvas_bg_src: Option<explorer_dom::NodeId>,
}

/// Source de ré-usage incrémental.
#[derive(Clone, Copy)]
struct IncCx<'a> {
    /// Items de la liste précédente (copiés par plage).
    items: &'a [DisplayItem],
    /// Spans de la liste précédente.
    spans: &'a SpanMap,
}

impl PaintCx<'_> {
    /// Enregistre la plage `[start, end)` émise pour `bx`
    /// sous le chemin courant — flags de ré-usage calculés.
    /// `escaped` = porteurs reportés au `defer` hérité
    /// (chemins **relatifs** à `bx` — voir `SpanRec::escaped`).
    fn record_span(
        &mut self,
        bx: &LayoutBox,
        start: u32,
        end: u32,
        off: (f32, f32),
        sig: u64,
        escaped: Box<[Box<[u16]>]>,
    ) {
        let mut flags = 0u8;
        if bx.position == Position::Fixed {
            flags |= F_FIXED;
        }
        if is_media(bx, self.images) {
            flags |= F_NO_REUSE;
        }
        if bx.children.iter().any(|c| c.position == Position::Sticky) {
            flags |= F_STICKY_KID;
        }
        self.spans.insert(
            self.path.clone().into_boxed_slice(),
            SpanRec {
                start,
                end,
                rect: bx.rect,
                off,
                sig,
                flags,
                bx_ptr: std::ptr::from_ref(bx) as usize,
                lines_ptr: bx.lines.as_ptr() as usize,
                kids_ptr: if bx.children.is_empty() {
                    usize::MAX
                } else {
                    bx.children.as_ptr() as usize
                },
                kids_len: u32::try_from(bx.children.len()).unwrap_or(u32::MAX),
                escaped,
            },
        );
    }
}

/// Extrait les chemins des porteurs poussés dans `defer`
/// depuis l'indice `base`, relativisés à `path` (le chemin
/// de la boîte en cours d'enregistrement).
fn escaped_paths(defer: &[(Vec<u16>, &LayoutBox)], base: usize, path: &[u16]) -> Box<[Box<[u16]>]> {
    defer[base..]
        .iter()
        .filter_map(|(p, _)| p.strip_prefix(path).map(Box::from))
        .collect()
}

/// Résout un chemin relatif depuis `bx` vers la boîte cible.
fn resolve_rel<'a>(bx: &'a LayoutBox, rel: &[u16]) -> Option<&'a LayoutBox> {
    let mut cur = bx;
    for &i in rel {
        cur = cur.children.get(usize::from(i))?;
    }
    Some(cur)
}

/// Sans images (tout `<img>` tombe en boîte `alt`) — voir
/// [`build_display_list_imaged`].
#[must_use]
pub fn build_display_list(
    root: &LayoutBox,
    viewport_w: f32,
    viewport_h: f32,
    scroll_y: f32,
) -> DisplayList {
    build_display_list_imaged(
        root,
        viewport_w,
        viewport_h,
        scroll_y,
        &crate::ImageTable::new(),
    )
}

/// `rect` translaté de `dy`.
fn shifted(r: Rect, off: (f32, f32)) -> Rect {
    Rect {
        x: r.x + off.0,
        y: r.y + off.1,
        ..r
    }
}

/// Style usé d'un côté de bordure.
fn border_style(v: Option<&str>) -> BorderStyle {
    match v {
        Some("solid") => BorderStyle::Solid,
        Some("dotted" | "dashed") => BorderStyle::Dotted,
        Some("double" | "groove" | "ridge" | "inset" | "outset") => BorderStyle::Double,
        _ => BorderStyle::None,
    }
}

/// `prop` → px pour le rendu : unités absolues seulement
/// (`px`, `pt`, `cm`…) ; les mots-clés d'épaisseur CSS
/// (`thin`/`medium`/`thick`) valent 1/3/5 px ; `em`/`rem` et
/// pourcents sont hors portée ici (pas de contexte de fonte)
/// → repli `fallback`.
#[allow(clippy::cast_possible_truncation)]
fn prop_px(st: &explorer_css::ComputedStyle, prop: &str, fallback: f32) -> f32 {
    let raw = st.get_or(prop, "");
    match raw {
        "thin" => 1.0,
        "medium" => 3.0,
        "thick" => 5.0,
        _ => parse_length(raw)
            .map(|l| match l.unit.px_factor() {
                Some(f) => l.value * f,
                // Unités font-relatives : la cascade stocke la
                // `font-size` calculée en px absolu (R-17) —
                // `1em` de bordure = 12 px sous `.picture`,
                // pas le repli `medium` (Acid2 — les bordures
                // 1em peinaient 3 px au lieu de 12).
                None => l.value * font_size_of(st) * em_factor(l.unit),
            })
            .map_or(fallback, |v| v as f32),
    }
}

/// La valeur `background-image` contient une couche `url(…)`
/// — n'importe où : `linear-gradient(…),url(…)` (sprites
/// Wikipedia, R-71) est un `background-image` valide dont la
/// couche image est en seconde position. Insensible à la
/// casse, comme l'extracteur `url(…)` de la collecte.
fn bg_has_url(v: &str) -> bool {
    v.as_bytes()
        .windows(4)
        .any(|w| w.eq_ignore_ascii_case(b"url("))
}

/// La boîte déclare un fond : `background-color` non
/// transparente ou `background-image: url()` (§14.2 —
/// sélection de la source du fond de canvas).
fn has_bg(st: &explorer_css::ComputedStyle) -> bool {
    st.get("background-color")
        .and_then(parse_color)
        .is_some_and(|c| c.a > 0)
        || st.get("background-image").is_some_and(bg_has_url)
}

/// `font-size` calculée de la boîte en px (16 par défaut).
fn font_size_of(st: &explorer_css::ComputedStyle) -> f64 {
    st.get("font-size")
        .and_then(parse_length)
        .and_then(|l| l.unit.px_factor().map(|f| l.value * f))
        .unwrap_or(16.0)
}

/// Rapport à l'em pour les unités font-relatives (`ex`/`ch`
/// ≈ 0.5em — approximation usuelle faute de métriques ici) ;
/// `rem`/`vw`/… retombent sur 1em faute de base racine dans
/// ce contexte.
fn em_factor(u: explorer_css::value::Unit) -> f64 {
    match u {
        explorer_css::value::Unit::Ex | explorer_css::value::Unit::Ch => 0.5,
        _ => 1.0,
    }
}

/// Couleur de bordure `side` (`border-top-color`…) — initiale
/// spec = **`currentcolor`** : une déclaration absente (ex.
/// `var()` non résolue retirée à la cascade) retombe sur la
/// `color` de la boîte, pas sur du noir (R-22 — les pastilles
/// `a.tag` de lobste.rs peignaient un cadre noir).
fn border_color_of(st: &explorer_css::ComputedStyle, side: &str) -> Color {
    st.get(side)
        .and_then(parse_color)
        .or_else(|| st.get("color").and_then(parse_color))
        .unwrap_or(Color::BLACK)
}

/// Content box de la boîte = border box − bordures − padding
/// (les `em` sont résolus via `prop_px`/`font_size_of`).
fn content_rect(bx: &explorer_layout::LayoutBox) -> Rect {
    let st = &bx.style;
    let l = border_side_w(st, "left") + prop_px(st, "padding-left", 0.0);
    let t = border_side_w(st, "top") + prop_px(st, "padding-top", 0.0);
    let r = border_side_w(st, "right") + prop_px(st, "padding-right", 0.0);
    let b = border_side_w(st, "bottom") + prop_px(st, "padding-bottom", 0.0);
    Rect {
        x: bx.rect.x + l,
        y: bx.rect.y + t,
        width: (bx.rect.width - l - r).max(0.0),
        height: (bx.rect.height - t - b).max(0.0),
    }
}

/// Épaisseur de bordure `side` (`top`/`right`/…) — 0 si le
/// style du côté est absent ou `none` (`border: solid` sans
/// épaisseur peint `medium` = 3 px).
fn border_side_w(st: &explorer_css::ComputedStyle, side: &str) -> f32 {
    if st
        .get(&format!("border-{side}-style"))
        .is_none_or(|s| s.trim() == "none")
    {
        return 0.0;
    }
    prop_px(st, &format!("border-{side}-width"), 3.0)
}

/// `background-position` → décalage (px) de la tuile (0,0)
/// dans la zone de positionnement `area` (CSS2 §14.2.1 :
/// `%` mesure `(zone - image)` ; mots-clés = 0/50/100 % ;
/// une valeur seule → x, l'autre `center` ; les unités
/// font-relatives se résolvent sur `fs` — `0 -1em` de
/// `margin-collapse-020` décale le fond de sa propre
/// font-size). Acid2 utilise `1px 0` sur les yeux.
#[allow(clippy::cast_possible_truncation)]
fn bg_position(raw: Option<&str>, area: (f32, f32), img: (f32, f32), fs: f64) -> (f32, f32) {
    let toks: Vec<&str> = raw.unwrap_or("").split_whitespace().collect();
    if toks.is_empty() {
        return (0.0, 0.0);
    }
    // Une dimension : longueur/`%` = offset depuis le coin,
    // mot-clé → fraction de (zone − image).
    let token = |t: &str, span: f32, dim: f32| -> f32 {
        match t {
            "left" | "top" => 0.0,
            "center" => (span - dim) / 2.0,
            "right" | "bottom" => span - dim,
            _ => t
                .strip_suffix('%')
                .and_then(|p| p.parse::<f32>().ok())
                .map_or_else(
                    || {
                        parse_length(t)
                            .map(|l| match l.unit.px_factor() {
                                Some(f) => l.value * f,
                                None => l.value * fs * em_factor(l.unit),
                            })
                            .map_or(0.0, |v| v as f32)
                    },
                    |p| (span - dim) * p / 100.0,
                ),
        }
    };
    let (x_tok, y_tok) = match toks.as_slice() {
        [a] => {
            // Seul un mot-clé vertical indique y.
            if matches!(*a, "top" | "bottom") {
                ("center", *a)
            } else {
                (*a, "center")
            }
        }
        [a, b, ..] => (*a, *b),
        _ => ("left", "top"),
    };
    (token(x_tok, area.0, img.0), token(y_tok, area.1, img.1))
}

/// Fond + bordures + texte propre d'une boîte (avant les
/// enfants — le fond passe TOUJOURS sous son contenu).
/// `off` = décalage vertical de scroll (`0` dans les
/// sous-arbres `position: fixed`).
#[allow(clippy::too_many_lines, clippy::cast_precision_loss)]
fn paint_self(
    bx: &LayoutBox,
    items: &mut Vec<DisplayItem>,
    off: (f32, f32),
    vp: (f32, f32),
    images: &crate::PaintImages,
    canvas_bg_src: Option<explorer_dom::NodeId>,
) {
    let rect = shifted(bx.rect, off);
    // Blocs anonymes et boîtes texte clonent le style du
    // parent, mais `background`/`border` ne sont PAS hérités
    // en CSS : ils ne peignent ni fond ni bordure propre
    // (sinon le clone masquerait les flottants posés avant,
    // ou la bordure du parent serait redessinée sur chaque
    // fragment de texte — §9.2.1.1).
    let anonymous = matches!(bx.kind, BoxKind::AnonymousBlock | BoxKind::Text);
    // Boîte `display:inline` pure (ni atome ni remplacé) :
    // `bx.rect` n'est que l'UNION de ses fragments — un fond
    // ou une bordure peint dessus produirait un blob couvrant
    // le texte voisin (R-22). Sa décoration est émise par
    // fragment dans la boucle des lignes (voir plus bas).
    // `display()` retombe sur "block" pour un élément sans
    // déclaration — le discriminant fiable est `kind` : une
    // boîte `Inline` non atomique (pas `inline-block`, pas
    // remplacée) est un pur conteneur de flux en-ligne.
    let inline_flow = is_inline_flow(bx);
    // Fond : `background-color` non transparente, puis
    // `background-image` décodée (au-dessus de la couleur,
    // sous la bordure — App. E ; origine padding box ≈ bord
    // interne de la bordure gauche/haute). Sautée quand le
    // fond de cette boîte a été propagé au canvas (§14.2) —
    // `canvas_bg_src` l'a déjà peint sur tout le viewport.
    if !anonymous && !inline_flow && canvas_bg_src != bx.node {
        if let Some(c) = bx.style.get("background-color").and_then(parse_color) {
            if c.a > 0 {
                // R-52 — `mask-image` présent : la couleur ne
                // passe que par l'alpha du mask (icônes des
                // design systems). Mask absent des tables
                // (non décodé/réseau) → repli fond plein,
                // comportement antérieur.
                if let Some(mask) = bx.node.and_then(|n| images.masks.get(&n)) {
                    items.push(DisplayItem::MaskedFill {
                        rect,
                        color: c,
                        mask: mask.clone(),
                    });
                } else {
                    items.push(DisplayItem::FillRect { rect, color: c });
                }
            }
        }
        if bx.style.get("background-image").is_some_and(bg_has_url) {
            if let Some(img) = bx.node.and_then(|n| images.backgrounds.get(&n)).cloned() {
                let repeat = match bx.style.get("background-repeat") {
                    Some("no-repeat") => (false, false),
                    Some("repeat-x") => (true, false),
                    Some("repeat-y") => (false, true),
                    _ => (true, true), // `repeat` = défaut.
                };
                // Zone de positionnement : **padding box** (bord
                // interne des bordures — `background-position`
                // s'y réfère, §14.2.1) pour `scroll`, viewport
                // pour `fixed` (le fond reste calé à l'écran).
                // L'origine est en coords FRAME : le viewport
                // y démarre en (0,0) — ancrer au scroll
                // déphaserait la tuile de `scroll mod taille`.
                // La zone PEINTE reste la border box (clip par
                // défaut) — seule l'ancre de la tuile bouge.
                let fixed = bx.style.get("background-attachment") == Some("fixed");
                let (area, base) = if fixed {
                    ((vp.0, vp.1), (0.0, 0.0))
                } else {
                    let bl = border_side_w(&bx.style, "left");
                    let bt = border_side_w(&bx.style, "top");
                    let br = border_side_w(&bx.style, "right");
                    let bb = border_side_w(&bx.style, "bottom");
                    (
                        (
                            (rect.width - bl - br).max(0.0),
                            (rect.height - bt - bb).max(0.0),
                        ),
                        (rect.x + bl, rect.y + bt),
                    )
                };
                let (px, py) = bg_position(
                    bx.style.get("background-position"),
                    area,
                    (img.width as f32, img.height as f32),
                    font_size_of(&bx.style),
                );
                items.push(DisplayItem::BackgroundImage {
                    rect,
                    img,
                    repeat,
                    origin: (base.0 + px, base.1 + py),
                });
            }
        }
    }
    // Bordures : 4 côtés, épaisseur > 0 et style non `none`.
    // Sémantique de `border_side_w` : la largeur ne compte que
    // si le style du côté est déclaré non-`none` — l'initiale
    // `medium` vaut alors 3 px (`border: solid` sans épaisseur
    // peint 3 px).
    let widths = if anonymous || inline_flow {
        [0.0; 4]
    } else {
        [
            border_side_w(&bx.style, "top"),
            border_side_w(&bx.style, "right"),
            border_side_w(&bx.style, "bottom"),
            border_side_w(&bx.style, "left"),
        ]
    };
    if widths.iter().any(|v| *v > 0.0) {
        let color_of = |side: &str| border_color_of(&bx.style, side);
        items.push(DisplayItem::Border {
            rect,
            widths,
            colors: [
                color_of("border-top-color"),
                color_of("border-right-color"),
                color_of("border-bottom-color"),
                color_of("border-left-color"),
            ],
            styles: [
                border_style(bx.style.get("border-top-style")),
                border_style(bx.style.get("border-right-style")),
                border_style(bx.style.get("border-bottom-style")),
                border_style(bx.style.get("border-left-style")),
            ],
        });
    }
    // (Les fragments `bx.lines` sont émis par `paint_lines`,
    // appelée dans la zone clippée — voir `paint_box` / R-55.)
    // Contrôle coché (`checkbox`/`radio`, étape 38) : marque
    // pleine centrée — carré à ~55 % du plus petit côté,
    // couleur = encre de la boîte (approx MVP documentée).
    if bx.control_checked {
        let side = (rect.width.min(rect.height) * 0.55).max(0.0);
        if side > 0.0 {
            let ink = bx
                .style
                .get("color")
                .and_then(parse_color)
                .unwrap_or(Color::BLACK);
            items.push(DisplayItem::FillRect {
                rect: Rect {
                    x: rect.x + (rect.width - side) / 2.0,
                    y: rect.y + (rect.height - side) / 2.0,
                    width: side,
                    height: side,
                },
                color: ink,
            });
        }
    }
}

/// Fragments de texte des lignes propres de `bx` — le contenu
/// inline de la boîte quand elle est racine d'un contexte
/// inline (élément inline positionné/flottant, bloc anonyme).
/// Séparé de `paint_self` : ces fragments sont du CONTENU —
/// `paint_box` les émet dans la zone du clip `overflow`
/// (R-55 : le `.sr-only` de lobste.rs — `position:absolute`,
/// `overflow:hidden`, 1×1 — fuyait son clip : ses
/// fragments sortaient avant le `PushClip`).
///
/// `frag.style` = style de l'élément inline propriétaire
/// (le plus proche du texte) — `a{color}`/`span{color}`
/// colorent leurs runs, pas seulement la boîte (R-18).
/// `true` = boîte `display:inline` pure (ni atomique ni
/// remplacée) : son `rect` n'est que l'union de fragments —
/// sa décoration s'émet par pièce de ligne, pas sur la boîte.
fn is_inline_flow(bx: &LayoutBox) -> bool {
    bx.kind == BoxKind::Inline
        && bx.intrinsic.is_none()
        && bx.display() != "inline-block"
        && bx.display() != "inline-flex"
        && bx.display() != "inline-table"
}

fn paint_lines(bx: &LayoutBox, items: &mut Vec<DisplayItem>, off: (f32, f32)) {
    // Deux passes (R-70) : TOUTES les décorations inline
    // d'abord, TOUS les textes ensuite — un fond émis entre
    // deux textes couvrirait l'encre du premier (le fond du
    // bouton-langue Wikipedia mangeait la queue de
    // « language »).
    for line in &bx.lines {
        // Décos — une pièce par élément et par ligne (approx.
        // `box-decoration-break: slice`, R-70) : l'union des
        // fragments de l'élément SUR LA LIGNE, padding aux
        // bords de la pièce seulement. `frag.owner` = indice
        // de l'enfant direct de `bx` (même convention que
        // `assign_child_rects`) → tous les fragments d'un
        // élément, descendants compris.
        for (o, child) in bx.children.iter().enumerate() {
            if !is_inline_flow(child) {
                continue;
            }
            let mut union: Option<Rect> = None;
            for f in &line.fragments {
                if f.owner == o {
                    union = Some(match union {
                        None => f.rect,
                        Some(u) => rect_union(u, f.rect),
                    });
                }
            }
            if let Some(u) = union {
                paint_inline_deco(&child.style, u, items, off);
            }
        }
        // Éléments imbriqués (leur fragment porte le style le
        // plus profond — `frag.style` ≠ style de l'enfant
        // direct `owner` et ≠ `bx.style`) : pièce par run
        // contigu de même style, union y comprise.
        let mut run: Option<(Rect, usize)> = None;
        for (i, f) in line.fragments.iter().enumerate() {
            let nested = !f.atomic
                && f.style != bx.style
                && bx.children.get(f.owner).is_some_and(|c| f.style != c.style);
            if !run.is_some_and(|(_, first)| line.fragments[first].style == f.style) {
                if let Some((u, first)) = run.take() {
                    paint_inline_deco(&line.fragments[first].style, u, items, off);
                }
            }
            if nested {
                match &mut run {
                    Some((u, _)) => *u = rect_union(*u, f.rect),
                    r => *r = Some((f.rect, i)),
                }
            }
        }
        if let Some((u, first)) = run {
            paint_inline_deco(&line.fragments[first].style, u, items, off);
        }
        for frag in &line.fragments {
            if frag.atomic {
                continue;
            }
            if let Some(run) = &frag.shaped {
                let ink = frag
                    .style
                    .get("color")
                    .and_then(parse_color)
                    .unwrap_or(Color::BLACK);
                items.push(DisplayItem::Text {
                    origin: (frag.rect.x + off.0, frag.rect.y + frag.baseline + off.1),
                    run: run.clone(),
                    font: font_request(&frag.style),
                    color: ink,
                    size_px: prop_px(&frag.style, "font-size", 16.0),
                });
            }
        }
    }
}

/// Décorations d'une pièce d'élément inline (R-22) : fond et
/// bordure sur `rect` — union des fragments de l'élément sur
/// la ligne — étendue de son padding et des épaisseurs de
/// bordure.
///
/// Approximation « `box-decoration-break: slice` » documentée :
/// une pièce par élément et par ligne, padding aux bords de
/// la pièce ; et `em`/`rem` ≈ font-size de l'élément (pas de
/// contexte racine ici).
fn paint_inline_deco(
    st: &explorer_css::ComputedStyle,
    rect: Rect,
    items: &mut Vec<DisplayItem>,
    off: (f32, f32),
) {
    let fs = prop_px(st, "font-size", 16.0);
    #[allow(clippy::cast_possible_truncation)]
    let pad = |name: &str| -> f32 {
        let Some(l) = parse_length(st.get_or(name, "")) else {
            return 0.0;
        };
        match l.unit.px_factor() {
            Some(f) => (l.value * f) as f32,
            None => l.value as f32 * fs,
        }
    };
    let l = pad("padding-left") + border_side_w(st, "left");
    let r = pad("padding-right") + border_side_w(st, "right");
    let t = pad("padding-top") + border_side_w(st, "top");
    let b = pad("padding-bottom") + border_side_w(st, "bottom");
    let outer = shifted(
        Rect {
            x: rect.x - l,
            y: rect.y - t,
            width: rect.width + l + r,
            height: rect.height + t + b,
        },
        off,
    );
    if let Some(c) = st.get("background-color").and_then(parse_color) {
        if c.a > 0 {
            items.push(DisplayItem::FillRect {
                rect: outer,
                color: c,
            });
        }
    }
    // `background-image` sur élément inline : hors portée
    // (rare — la plupart des fonds inline sont des couleurs).
    let widths = [
        border_side_w(st, "top"),
        border_side_w(st, "right"),
        border_side_w(st, "bottom"),
        border_side_w(st, "left"),
    ];
    if widths.iter().any(|v| *v > 0.0) {
        items.push(DisplayItem::Border {
            rect: outer,
            widths,
            colors: [
                border_color_of(st, "border-top-color"),
                border_color_of(st, "border-right-color"),
                border_color_of(st, "border-bottom-color"),
                border_color_of(st, "border-left-color"),
            ],
            styles: [
                border_style(st.get("border-top-style")),
                border_style(st.get("border-right-style")),
                border_style(st.get("border-bottom-style")),
                border_style(st.get("border-left-style")),
            ],
        });
    }
}

/// Décalage `position: sticky` — les **quatre côtés**
/// (R-6) : `off` = translation de scroll déjà appliquée
/// `(x, y)` (négative), `bound` = border box du bloc
/// conteneur en coordonnées document, `vp` = `(largeur,
/// hauteur)` du scrollport (le viewport — référence du
/// sticky racine ; les conteneurs `overflow` internes ne
/// scrollent pas encore). La boîte collée est bornée pour
/// ne jamais sortir de son parent. Sur-contrainte spec :
/// `top` gagne sur `bottom`, `left` sur `right` (LTR).
fn sticky_offset(bx: &LayoutBox, off: (f32, f32), bound: Rect, vp: (f32, f32)) -> (f32, f32) {
    if bx.position != Position::Sticky {
        return (0.0, 0.0);
    }
    let painted = shifted(bx.rect, off);
    let bound_p = shifted(bound, off);
    let dy = if bx.style.get("top").is_some_and(|v| v != "auto") {
        let thr = prop_px(&bx.style, "top", 0.0);
        let room = (bound_p.y + bound_p.height - painted.y - painted.height).max(0.0);
        (thr - painted.y).clamp(0.0, room)
    } else if bx.style.get("bottom").is_some_and(|v| v != "auto") {
        let thr = prop_px(&bx.style, "bottom", 0.0);
        // Colle quand le bas peint dépasserait `vp_h - bottom`.
        let room = (painted.y - bound_p.y).max(0.0);
        -(painted.y + painted.height - (vp.1 - thr)).clamp(0.0, room)
    } else {
        0.0
    };
    let dx = if bx.style.get("left").is_some_and(|v| v != "auto") {
        let thr = prop_px(&bx.style, "left", 0.0);
        let room = (bound_p.x + bound_p.width - painted.x - painted.width).max(0.0);
        (thr - painted.x).clamp(0.0, room)
    } else if bx.style.get("right").is_some_and(|v| v != "auto") {
        let thr = prop_px(&bx.style, "right", 0.0);
        let room = (painted.x - bound_p.x).max(0.0);
        -(painted.x + painted.width - (vp.0 - thr)).clamp(0.0, room)
    } else {
        0.0
    };
    (dx, dy)
}

/// `FontRequest` dérivé du style : familles `font-family`
/// (virgules), `font-weight` (100-900), `font-style`.
fn font_request(st: &explorer_css::ComputedStyle) -> explorer_layout::font::FontRequest {
    let families = st.get("font-family").map_or_else(
        || vec!["sans-serif".to_string()],
        |v| {
            v.split(',')
                .map(|f| f.trim().trim_matches('"').trim_matches('\'').to_string())
                .filter(|f| !f.is_empty())
                .collect()
        },
    );
    let weight = st.get("font-weight").map_or(400, |v| match v.trim() {
        "normal" => 400,
        "bold" => 700,
        "bolder" => 800,
        "lighter" => 300,
        n => n.parse::<u16>().unwrap_or(400).clamp(1, 1000),
    });
    let italic = matches!(st.get("font-style"), Some("italic" | "oblique"));
    explorer_layout::font::FontRequest {
        families,
        weight,
        italic,
    }
}

/// Longueur d'une fonction `transform` → px : absolues via
/// `px_factor`, `%` relatif à `basis` (taille propre de la
/// boîte — spec : reference box = border box).
fn tf_len_px(s: &str, basis: f32) -> Option<f64> {
    let t = s.trim();
    if let Some(p) = t.strip_suffix('%') {
        return p
            .trim()
            .parse::<f64>()
            .ok()
            .map(|v| v * f64::from(basis) / 100.0);
    }
    parse_length(t).and_then(|l| l.unit.px_factor().map(|f| l.value * f))
}

/// Angle `deg`/`rad`/`turn`/`grad` → radians.
fn tf_angle(s: &str) -> Option<f64> {
    let t = s.trim().to_ascii_lowercase();
    let (num, k) = ["deg", "grad", "rad", "turn"]
        .iter()
        .find_map(|u| t.strip_suffix(u).map(|n| (n, *u)))?;
    let v: f64 = num.trim().parse().ok()?;
    Some(match k {
        "deg" => v.to_radians(),
        "rad" => v,
        "turn" => v * std::f64::consts::TAU,
        _ => v * std::f64::consts::PI / 200.0,
    })
}

/// `transform: f1(...) f2(...)…` → matrice brute (sans
/// origine). Fonctions : `matrix`, `translate[X|Y]`,
/// `scale[X|Y]`, `rotate`, `skew[X|Y]` ; `%` des `translate`
/// relatif à la border box. `none`/inconnu → `None`.
#[allow(clippy::cast_possible_truncation)]
fn transform_matrix(raw: &str, rect: &Rect) -> Option<tiny_skia::Transform> {
    use tiny_skia::Transform;
    let mut m = Transform::identity();
    let mut any = false;
    let mut left = raw.trim();
    if left.eq_ignore_ascii_case("none") || left.is_empty() {
        return None;
    }
    while let Some(open) = left.find('(') {
        let name = left[..open].trim().to_ascii_lowercase();
        let close = left.find(')')?;
        let args: Vec<&str> = left[open + 1..close].split(',').collect();
        let num = |i: usize| -> Option<f64> { args.get(i)?.trim().parse().ok() };
        let f = match name.as_str() {
            "matrix" => Transform::from_row(
                num(0)? as f32,
                num(1)? as f32,
                num(2)? as f32,
                num(3)? as f32,
                num(4)? as f32,
                num(5)? as f32,
            ),
            "translate" => Transform::from_translate(
                tf_len_px(args.first()?, rect.width)? as f32,
                args.get(1)
                    .map_or(0.0, |v| tf_len_px(v, rect.height).unwrap_or(0.0) as f32),
            ),
            "translatex" => {
                Transform::from_translate(tf_len_px(args.first()?, rect.width)? as f32, 0.0)
            }
            "translatey" => {
                Transform::from_translate(0.0, tf_len_px(args.first()?, rect.height)? as f32)
            }
            "scale" => {
                let sx = num(0)? as f32;
                Transform::from_scale(
                    sx,
                    args.get(1)
                        .map_or(sx, |v| v.trim().parse::<f32>().unwrap_or(sx)),
                )
            }
            "scalex" => Transform::from_scale(num(0)? as f32, 1.0),
            "scaley" => Transform::from_scale(1.0, num(0)? as f32),
            "rotate" => {
                // tiny-skia prend des degrés (`tf_angle` rend
                // des radians pour `tan()` des skews).
                Transform::from_rotate(tf_angle(args.first()?)?.to_degrees() as f32)
            }
            "skewx" => Transform::from_skew(tf_angle(args.first()?)?.tan() as f32, 0.0),
            "skewy" => Transform::from_skew(0.0, tf_angle(args.first()?)?.tan() as f32),
            "skew" => Transform::from_skew(
                tf_angle(args.first()?)?.tan() as f32,
                args.get(1).and_then(|v| tf_angle(v)).unwrap_or(0.0).tan() as f32,
            ),
            _ => return None,
        };
        // Composition gauche→droite : chaque fonction s'applique
        // APRÈS les précédentes (post-concaténation).
        m = m.post_concat(f);
        any = true;
        left = left[close + 1..].trim();
    }
    if !any {
        return None;
    }
    Some(m)
}

/// Résout `transform-origin` en point absolu de `rect` —
/// deux valeurs px/`%` (défaut 50 % 50 %).
#[allow(clippy::cast_possible_truncation)]
fn origin_px(st: &explorer_css::ComputedStyle, rect: &Rect) -> (f32, f32) {
    let raw = st.get_or("transform-origin", "50% 50%");
    let mut it = raw.split_whitespace();
    let x = it
        .next()
        .and_then(|v| tf_len_px(v, rect.width))
        .map_or(rect.width / 2.0, |v| v as f32);
    let y = it
        .next()
        .and_then(|v| tf_len_px(v, rect.height))
        .map_or(rect.height / 2.0, |v| v as f32);
    (rect.x + x, rect.y + y)
}

/// `transform` d'une boîte → matrice complète (origine
/// incluse) — `None` si identité/invalidée.
fn box_transform(bx: &LayoutBox, rect: &Rect) -> Option<tiny_skia::Transform> {
    use tiny_skia::Transform;
    let raw = bx.style.get("transform")?;
    let m = transform_matrix(raw, rect)?;
    let (ox, oy) = origin_px(&bx.style, rect);
    // F(p) = o + M(p − o) : on post-concatène dans l'ordre
    // d'application — T(−o) d'abord, M, puis T(+o).
    Some(
        Transform::from_translate(-ox, -oy)
            .post_concat(m)
            .post_concat(Transform::from_translate(ox, oy)),
    )
}

/// Peint une boîte et ses descendants dans l'ordre borné.
/// La boîte ferme la portée de report des inlines : racine
/// (gérée à l'entrée), positionnée, flottante ou atome en
/// ligne (remplacée, `inline-block` à enfants blocs) — leurs
/// descendants en ligne restent peints à l'intérieur
/// (App. E : un flottant peint tout son sous-arbre à
/// l'étape 4, les inlines d'un contexte à l'étape 5).
fn inline_scope(bx: &LayoutBox) -> bool {
    // Tout conteneur de niveau bloc ferme la portée : ses
    // descendants en ligne se peignent dans SA phase inline
    // (App. E — récursion par élément, ordre document) — pas
    // à la fin du contexte de pilement, sinon leur débord
    // couvrirait la bordure des boîtes suivantes (wpt 155).
    !bx.kind.is_inline_level()
        || bx.position != Position::Static
        || bx.float != Float::None
        || bx.intrinsic.is_some()
        || bx.children.iter().any(|c| !c.kind.is_inline_level())
}

/// Enfant porteur de contenu en ligne — peint à l'étape 5 de
/// son contexte de pilement, pas à sa position d'arbre :
/// blocs anonymes (leurs lignes), boîtes `Inline`, `Text`.
fn inline_carrier(c: &LayoutBox) -> bool {
    c.float == Float::None
        && matches!(
            c.kind,
            BoxKind::AnonymousBlock | BoxKind::Inline | BoxKind::Text
        )
}

/// Peint `bx` avec sa propre portée de report — utilisé pour
/// les flottants, les positionnés et les porteurs inline
/// drainés : leurs descendants en ligne se peignent dedans.
/// Les positionnés `z ≥ 0` passent par `pos_defer` (file du
/// contexte d'empilement englobant) sauf si `bx` en est un.
fn paint_scoped<'a>(
    bx: &'a LayoutBox,
    items: &mut Vec<DisplayItem>,
    off: (f32, f32),
    cx: &mut PaintCx<'a>,
    pos_defer: &mut Vec<(Vec<u16>, &'a LayoutBox)>,
) {
    let mut defer = Vec::new();
    paint_box(bx, items, off, cx, &mut defer, pos_defer);
    let mut i = 0;
    while i < defer.len() {
        let (p, c) = defer[i].clone();
        i += 1;
        emit_scoped_at(p, c, items, off, cx, pos_defer);
    }
}

/// Émet `bx` en réutilisant sa plage précédente si le
/// sous-arbre est ré-utilisable — sinon descente normale.
/// `idx` = son index dans `parent.children` (clé de span).
#[allow(clippy::too_many_arguments)]
fn emit_child<'a>(
    idx: usize,
    bx: &'a LayoutBox,
    items: &mut Vec<DisplayItem>,
    off: (f32, f32),
    cx: &mut PaintCx<'a>,
    defer: &mut Vec<(Vec<u16>, &'a LayoutBox)>,
    pos_defer: &mut Vec<(Vec<u16>, &'a LayoutBox)>,
) {
    cx.path.push(u16::try_from(idx).unwrap_or(u16::MAX));
    if let Some((d, s, sig)) = reuse_delta(bx, off, cx) {
        copy_span(bx, s, d, sig, items, off, cx);
        // Re-pousse les porteurs échappés de la plage —
        // relatifs → absolus ; le drain englobant les
        // ré-émettra via leurs propres spans. La file cible
        // se re-déduit (inlines → `defer`, positionnés →
        // `pos_defer`) — les deux y échappent ensemble.
        for rel in &s.escaped {
            if let Some(target) = resolve_rel(bx, rel) {
                let mut p = cx.path.clone();
                p.extend(rel.iter().copied());
                if !matches!(target.position, Position::Static | Position::Sticky)
                    && target.z_index >= 0
                {
                    push_z(pos_defer, p, target);
                } else {
                    defer.push((p, target));
                }
            }
        }
    } else {
        let start = u32::try_from(items.len()).unwrap_or(u32::MAX);
        let base = defer.len();
        let pbase = pos_defer.len();
        explorer_common::deep(|| paint_box(bx, items, off, cx, defer, pos_defer));
        let end = u32::try_from(items.len()).unwrap_or(u32::MAX);
        let sig = box_sig(bx);
        let mut esc = escaped_paths(defer, base, &cx.path).into_vec();
        esc.extend(escaped_paths(pos_defer, pbase, &cx.path).into_vec());
        cx.record_span(
            bx,
            start,
            end,
            effective_off(bx, off),
            sig,
            esc.into_boxed_slice(),
        );
    }
    cx.path.pop();
}

/// Variante pour les enfants peints hors-flux (flottants,
/// positionnés, porteurs drainés) — portée propre.
#[allow(clippy::too_many_arguments)]
fn emit_scoped<'a>(
    idx: usize,
    bx: &'a LayoutBox,
    items: &mut Vec<DisplayItem>,
    off: (f32, f32),
    cx: &mut PaintCx<'a>,
    pos_defer: &mut Vec<(Vec<u16>, &'a LayoutBox)>,
) {
    cx.path.push(u16::try_from(idx).unwrap_or(u16::MAX));
    if let Some((d, s, sig)) = reuse_delta(bx, off, cx) {
        copy_span(bx, s, d, sig, items, off, cx);
    } else {
        let start = u32::try_from(items.len()).unwrap_or(u32::MAX);
        explorer_common::deep(|| paint_scoped(bx, items, off, cx, pos_defer));
        let end = u32::try_from(items.len()).unwrap_or(u32::MAX);
        let sig = box_sig(bx);
        cx.record_span(bx, start, end, effective_off(bx, off), sig, Box::default());
    }
    cx.path.pop();
}

/// Variante pour les porteurs inline reportés — leur vrai
/// chemin (captured à l'enfilement, dans le parent direct)
/// remplace le chemin courant le temps de l'émission, sinon
/// leur span s'enregistrerait sous le chemin de l'ancêtre
/// drainant et pourrait entrer en collision avec un vrai
/// enfant de celui-ci.
fn emit_scoped_at<'a>(
    path: Vec<u16>,
    bx: &'a LayoutBox,
    items: &mut Vec<DisplayItem>,
    off: (f32, f32),
    cx: &mut PaintCx<'a>,
    pos_defer: &mut Vec<(Vec<u16>, &'a LayoutBox)>,
) {
    let saved = std::mem::replace(&mut cx.path, path);
    if let Some((d, s, sig)) = reuse_delta(bx, off, cx) {
        copy_span(bx, s, d, sig, items, off, cx);
    } else {
        let start = u32::try_from(items.len()).unwrap_or(u32::MAX);
        explorer_common::deep(|| paint_scoped(bx, items, off, cx, pos_defer));
        let end = u32::try_from(items.len()).unwrap_or(u32::MAX);
        let sig = box_sig(bx);
        cx.record_span(bx, start, end, effective_off(bx, off), sig, Box::default());
    }
    cx.path = saved;
}

/// `off` réellement appliqué aux items de `bx` —
/// `position: fixed` ré-initialise l'offset (ses coords
/// sont viewport). `paint_box` fait de même : `reuse_delta`
/// et `copy_span` doivent comparer/enregistrer sur cette
/// base, sinon un fixe se voit appliquer le scroll du parent.
fn effective_off(bx: &LayoutBox, off: (f32, f32)) -> (f32, f32) {
    if bx.position == Position::Fixed {
        (0.0, 0.0)
    } else {
        off
    }
}

/// Δ de translation si le sous-arbre `bx` est ré-utilisable
/// en entier — `Some((d, span, sig))`, `None` = ré-émission
/// requise. **Invariant** : la copie plate d'une plage n'est
/// valide que si tous les items bougent du même Δ — chaque
/// descendant doit retourner le même Δ (un `fixed` interne
/// donne Δ=0 ≠ Δ du parent → le parent descend ; un sticky
/// est couvert par le flag `F_STICKY_KID` du parent).
fn reuse_delta<'a>(
    bx: &'a LayoutBox,
    off: (f32, f32),
    cx: &mut PaintCx<'a>,
) -> Option<((f32, f32), &'a SpanRec, u64)> {
    let inc = cx.inc?;
    let s = inc.spans.get(cx.path.as_slice())?;
    // Jamais de ré-usage sur média remplacé (bitmap tardif) ou
    // fond `attachment:fixed` (origine ancrée viewport — sa
    // translation divergerait de la géométrie).
    if s.flags & F_NO_REUSE != 0 {
        return None;
    }
    // Même objet LayoutBox (mutation in-place) : `lines`
    // (Vec réassigné par `relay`) et les enfants sont les
    // champs réalloués sur une boîte préservée — comparés ;
    // la signature est sautée (contrat consigné dans
    // `SpanRec::bx_ptr`, **vérifié** en debug ci-dessous).
    // `bx.node` n'est **pas** comparé : `remap_box_nodes`
    // le réécrit sur toute boîte préservée à chaque reparse
    // — et il n'est pas émis dans les items.
    // `kids_ptr` est le discriminant des remplacements
    // `*bx = fresh` : le slot garde son adresse mais le Vec
    // de `fresh` est alloué avant le drop de l'ancien —
    // disjoint. Une **feuille** (children + lines vides) n'a
    // que des ptrs dangling : rien ne la distingue d'une
    // remplacée au même slot → signature obligatoire.
    let leaf = bx.children.is_empty() && bx.lines.is_empty();
    let same_obj = !leaf
        && cx.same_tree
        && s.bx_ptr == std::ptr::from_ref(bx) as usize
        && s.lines_ptr == bx.lines.as_ptr() as usize
        && s.kids_ptr
            == if bx.children.is_empty() {
                usize::MAX
            } else {
                bx.children.as_ptr() as usize
            }
        && s.kids_len == u32::try_from(bx.children.len()).unwrap_or(u32::MAX);
    let sig = if same_obj {
        #[cfg(debug_assertions)]
        debug_assert_eq!(
            box_sig(bx),
            s.sig,
            "contrat R-37 violé : champ peint muté in-place              hors {{rect, node, lines, slot}} sur boîte préservée"
        );
        s.sig
    } else {
        let sig = box_sig(bx);
        if sig != s.sig {
            return None;
        }
        sig
    };
    if s.rect.width.to_bits() != bx.rect.width.to_bits()
        || s.rect.height.to_bits() != bx.rect.height.to_bits()
    {
        return None;
    }
    let eff = effective_off(bx, off);
    let d = (
        bx.rect.x - s.rect.x + eff.0 - s.off.0,
        bx.rect.y - s.rect.y + eff.1 - s.off.1,
    );
    // Enfant sticky → le wrap de décalage vit dans la plage
    // du parent et dépend du scroll non linéairement — toute
    // translation non nulle impose la ré-émission du parent.
    if s.flags & F_STICKY_KID != 0 && d != (0.0, 0.0) {
        return None;
    }
    for (i, c) in bx.children.iter().enumerate() {
        cx.path.push(u16::try_from(i).unwrap_or(u16::MAX));
        let cof = effective_off(c, off);
        let cd = explorer_common::deep(|| reuse_delta(c, cof, cx));
        cx.path.pop();
        // Translation uniforme exigée — un descendant qui
        // diverge (fixed, sale) interdit la copie plate.
        if cd.map(|(cd, _, _)| cd) != Some(d) {
            return None;
        }
    }
    Some((d, s, sig))
}

/// Copie la plage `s` en translatant chaque item de `d` et
/// enregistre le nouveau span de `bx`.
#[allow(clippy::too_many_arguments)]
fn copy_span(
    bx: &LayoutBox,
    s: &SpanRec,
    d: (f32, f32),
    sig: u64,
    items: &mut Vec<DisplayItem>,
    off: (f32, f32),
    cx: &mut PaintCx,
) {
    let Some(inc) = cx.inc else { return };
    let start = u32::try_from(items.len()).unwrap_or(u32::MAX);
    for it in &inc.items[(s.start as usize)..(s.end as usize)] {
        items.push(translate_item(it, d));
    }
    let eff = effective_off(bx, off);
    // `escaped` est préservé : les chemins relatifs restent
    // valides dans le nouvel arbre (même sous-arbre).
    cx.record_span(
        bx,
        start,
        u32::try_from(items.len()).unwrap_or(u32::MAX),
        eff,
        sig,
        s.escaped.clone(),
    );
}

/// `bx` a-t-il du contenu remplacé/retardé — bitmap `<img>`,
/// `background-image`, svg, `<canvas>` (arrivent après le
/// paint, `Arc` nouvelle à chaque build) → ré-usage interdit.
fn is_media(bx: &LayoutBox, images: &crate::PaintImages) -> bool {
    bx.node.is_some_and(|n| {
        images.replaced.contains_key(&n)
            || images.svgs.contains_key(&n)
            || images.backgrounds.contains_key(&n)
            || images.canvases.contains_key(&n)
    }) || bx.style.get("background-attachment") == Some("fixed")
}

/// FNV-1a — fingerprint interne des spans (signature de
/// boîte + clés de `spans`). Pas cryptographique : le rôle
/// est de rejeter une égalité fausse, et le volume (~10⁴
/// spans par paint) exige ~1 ns/octet, pas `SipHash`.
#[derive(Clone)]
pub struct SigHasher(u64);

impl Default for SigHasher {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl SigHasher {
    #[inline]
    fn mix(&mut self, v: u64) {
        for b in v.to_ne_bytes() {
            self.0 = (self.0 ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    #[inline]
    fn mix_f32(&mut self, v: f32) {
        self.mix(u64::from(v.to_bits()));
    }
}

impl std::hash::Hasher for SigHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0 ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}

/// Table de correspondance boîte→plage — hasher rapide (les
/// clés sont des chemins courts `u16`, hashés à chaque
/// span produit *et* cherché).
pub type SpanMap =
    std::collections::HashMap<Box<[u16]>, SpanRec, std::hash::BuildHasherDefault<SigHasher>>;

/// Fingerprint des champs lus par l'émission hors géométrie —
/// deux boîtes de `sig` égale produisent des items égaux aux
/// mêmes positions (modulo `rect`/`off`, comparés à part).
/// Couvre kind/positions/overflow/z-index/checked/intrinsic/
/// texte/style/lignes (rects de fragments, contenu texte et
/// **identité** des runs shapés — le `Arc` refait à chaque
/// layout, donc conservateur : même texte re-shapé ≠ réusé).
fn box_sig(bx: &LayoutBox) -> u64 {
    use std::hash::Hasher;
    let mut h = SigHasher::default();
    // Discriminants d'enums → petit rang stable.
    h.mix(match bx.kind {
        BoxKind::Block => 0,
        BoxKind::ListItem => 1,
        BoxKind::Inline => 2,
        BoxKind::Text => 3,
        BoxKind::AnonymousBlock => 4,
    });
    h.mix(match bx.position {
        Position::Static => 0,
        Position::Relative => 1,
        Position::Absolute => 2,
        Position::Fixed => 3,
        Position::Sticky => 4,
    });
    h.mix(match bx.float {
        Float::None => 0,
        Float::Left => 1,
        Float::Right => 2,
    });
    h.mix(match bx.overflow {
        Overflow::Visible => 0,
        Overflow::Hidden => 1,
        Overflow::Scroll => 2,
        Overflow::Auto => 3,
    });
    h.mix(u64::from(u32::from_ne_bytes(bx.z_index.to_ne_bytes())));
    h.mix(u64::from(bx.control_checked));
    let (iw, ih) = bx.intrinsic.unwrap_or((0.0, 0.0));
    h.mix_f32(iw);
    h.mix_f32(ih);
    if let Some(t) = &bx.text {
        h.write(t.as_bytes());
    }
    h.mix(0x9e37);
    // Style : itération d'ordre instable → hash par paire,
    // pli commutatif (XOR) — ordre-insensible.
    let mut sh = 0u64;
    for (k, v) in bx.style.iter() {
        let mut ph = SigHasher::default();
        ph.write(k.as_bytes());
        ph.write(v.as_bytes());
        sh ^= ph.finish();
    }
    h.mix(sh);
    for line in &bx.lines {
        for f in &line.fragments {
            // La géométrie des fragments (rect, baseline,
            // marges, trail_w) est **dérivée** des entrées
            // signées (style + texte + taille du conteneur,
            // comparée à part) — et `translate_y` la décale
            // in-place sous `relayout_subtree` : l'inclure
            // violerait le contrat du fast-path. Elle est
            // couverte par `s.rect` + l'invariant Δ-uniforme.
            h.write(f.text.as_bytes());
            h.mix(u64::from(f.atomic));
            // Contenu du run (pas le ptr `Arc` — instable
            // entre passes) : une fonte chargée entre deux
            // builds change les glyphes → la signature doit
            // l'attraper. Même ordre de coût que le texte.
            if let Some(r) = &f.shaped {
                h.mix(r.glyphs.len() as u64);
                for g in &r.glyphs {
                    h.mix(u64::from(g.id));
                    h.mix_f32(g.advance);
                    h.mix_f32(g.offset_x);
                    h.mix_f32(g.offset_y);
                }
            }
            let mut fh = 0u64;
            for (k, v) in f.style.iter() {
                let mut ph = SigHasher::default();
                ph.write(k.as_bytes());
                ph.write(v.as_bytes());
                fh ^= ph.finish();
            }
            h.mix(fh);
        }
    }
    h.finish()
}

/// Item translaté de `d` — utilisé par les copies de plage
/// incrémentales. `d = (0,0)` → clone simple.
fn translate_item(it: &DisplayItem, d: (f32, f32)) -> DisplayItem {
    use DisplayItem as D;
    if d == (0.0, 0.0) {
        return it.clone();
    }
    match it {
        D::FillRect { rect, color } => D::FillRect {
            rect: shifted(*rect, d),
            color: *color,
        },
        D::Border {
            rect,
            widths,
            colors,
            styles,
        } => D::Border {
            rect: shifted(*rect, d),
            widths: *widths,
            colors: *colors,
            styles: *styles,
        },
        D::Text {
            origin,
            run,
            font,
            color,
            size_px,
        } => D::Text {
            origin: (origin.0 + d.0, origin.1 + d.1),
            run: run.clone(),
            font: font.clone(),
            color: *color,
            size_px: *size_px,
        },
        D::Placeholder { rect, label } => D::Placeholder {
            rect: shifted(*rect, d),
            label: label.clone(),
        },
        D::Image { rect, img } => D::Image {
            rect: shifted(*rect, d),
            img: img.clone(),
        },
        D::BackgroundImage {
            rect,
            img,
            repeat,
            origin,
        } => D::BackgroundImage {
            rect: shifted(*rect, d),
            img: img.clone(),
            repeat: *repeat,
            origin: (origin.0 + d.0, origin.1 + d.1),
        },
        D::MaskedFill { rect, color, mask } => D::MaskedFill {
            rect: shifted(*rect, d),
            color: *color,
            mask: mask.clone(),
        },
        D::PushClip(r) => D::PushClip(shifted(*r, d)),
        D::PushTransform(m) => {
            let mut m = *m;
            m.tx += d.0;
            m.ty += d.1;
            D::PushTransform(m)
        }
        D::PopClip | D::PopTransform => it.clone(),
    }
}

/// Élément remplacé : bitmap par-dessus bordure (contenu —
/// ordre CSS 2.1 App. E : fond → bordure → contenu).
fn paint_replaced(
    bx: &LayoutBox,
    items: &mut Vec<DisplayItem>,
    off: (f32, f32),
    images: &crate::PaintImages,
) {
    if let Some(img) = bx.node.and_then(|n| {
        images
            .replaced
            .get(&n)
            .or_else(|| images.svgs.get(&n))
            .cloned()
            .or_else(|| {
                // `<canvas>` : op-list rejouée vers un bitmap à
                // la taille intrinsèque de l'élément (attributs
                // width/height — défaut spec 300×150).
                images.canvases.get(&n).and_then(|ops| {
                    let (w, h) = bx.intrinsic.unwrap_or((300.0, 150.0));
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    crate::canvas::raster_ops(ops, w.max(0.0) as u32, h.max(0.0) as u32)
                        .map(std::sync::Arc::new)
                })
            })
    }) {
        // Le contenu remplacé se place dans la **content
        // box** — padding et bordures restent autour
        // (§10.3.2 — Acid2 : l'œil 96×24 étiré à 131 sinon,
        // couvrant padding 11/12 + bordure 12). `object-fit`
        // (R-34) positionne le bitmap dedans ; les modes qui
        // débordent (`cover`, `none` trop grand, position
        // décalée) sont clipés à la content box.
        let content = shifted(content_rect(bx), off);
        let (dst, clip) = object_fit_dst(bx, &img, content);
        if clip {
            items.push(DisplayItem::PushClip(content));
        }
        items.push(DisplayItem::Image { rect: dst, img });
        if clip {
            items.push(DisplayItem::PopClip);
        }
    }
}

/// Destination `object-fit` (R-34, minimal) dans la content
/// box : `fill` (défaut) étire le bitmap à la boîte ;
/// `contain`/`scale-down`/`cover`/`none` préservent le ratio
/// ou la taille naturelle ; `object-position` règle
/// l'ancrage dans l'espace libre (50 % par axe = centré).
/// Renvoie `(rect, clip)` — `clip` vrai quand la destination
/// déborde la boîte (le peintre pose un `PushClip`).
fn object_fit_dst(bx: &LayoutBox, img: &crate::DecodedImage, content: Rect) -> (Rect, bool) {
    #[allow(clippy::cast_precision_loss)] // bitmaps < 2^24 px par axe.
    let (iw, ih) = (img.width as f32, img.height as f32);
    if iw <= 0.0 || ih <= 0.0 || content.width <= 0.0 || content.height <= 0.0 {
        return (content, false);
    }
    let (w, h) = match bx.style.get("object-fit").map_or("fill", |v| v.trim()) {
        "contain" => {
            let s = (content.width / iw).min(content.height / ih);
            (iw * s, ih * s)
        }
        "cover" => {
            let s = (content.width / iw).max(content.height / ih);
            (iw * s, ih * s)
        }
        "none" => (iw, ih),
        "scale-down" => {
            let s = (content.width / iw).min(content.height / ih).min(1.0);
            (iw * s, ih * s)
        }
        // `fill` et toute valeur inconnue : étirement pleine
        // boîte (comportement historique — tolérance CSS).
        _ => (content.width, content.height),
    };
    let (ox, oy) = object_position(&bx.style, content.width - w, content.height - h);
    let dst = Rect {
        x: content.x + ox,
        y: content.y + oy,
        width: w,
        height: h,
    };
    let clip = dst.x < content.x
        || dst.y < content.y
        || dst.x + dst.width > content.x + content.width
        || dst.y + dst.height > content.y + content.height;
    (dst, clip)
}

/// `object-position` → décalage `(x, y)` en px dans
/// l'espace libre de chaque axe : mots-clés
/// (`left`/`center`/`right` × `top`/`center`/`bottom`),
/// `<pourcentage>` (part de l'espace libre — 50 % centre,
/// défaut) et `<longueur>` px. Une valeur unique sans axe
/// explicite reçoit l'axe horizontal ; l'axe non couvert
/// reste centré (spec `<position>`).
fn object_position(st: &explorer_css::ComputedStyle, free_x: f32, free_y: f32) -> (f32, f32) {
    let (mut x, mut y) = (None, None);
    if let Some(v) = st.get("object-position") {
        for tok in v.split_whitespace().take(2) {
            match tok.to_ascii_lowercase().as_str() {
                "left" => x = Some(0.0),
                "right" => x = Some(free_x),
                "top" => y = Some(0.0),
                "bottom" => y = Some(free_y),
                "center" => {
                    if x.is_none() {
                        x = Some(free_x / 2.0);
                    } else {
                        y = Some(free_y / 2.0);
                    }
                }
                t => {
                    let off = t
                        .strip_suffix('%')
                        .and_then(|p| p.parse::<f32>().ok())
                        .map(|n| (n / 100.0, true))
                        .or_else(|| {
                            t.strip_suffix("px")
                                .and_then(|p| p.parse::<f32>().ok())
                                .map(|n| (n, false))
                        });
                    if let Some((n, frac)) = off {
                        if x.is_none() {
                            x = Some(if frac { free_x * n } else { n });
                        } else {
                            y = Some(if frac { free_y * n } else { n });
                        }
                    }
                }
            }
        }
    }
    (x.unwrap_or(free_x / 2.0), y.unwrap_or(free_y / 2.0))
}

/// La boîte crée-t-elle un contexte d'empilement au sens de
/// l'App. E ? Positionnée à `z-index` explicite, ou transformée
/// (CSS Transforms). La racine est traitée par l'appelant.
/// `auto` et `0` ne sont pas distingués par `z_index` (champ
/// aplati) — on lit le style calculé.
fn stacking_ctx(bx: &LayoutBox) -> bool {
    (bx.position != Position::Static && bx.style.get("z-index").is_some_and(|v| v.trim() != "auto"))
        || bx
            .style
            .get("transform")
            .is_some_and(|v| v.trim() != "none")
}

/// Enfile un enfant positionné dans la file de son contexte
/// d'empilement — insertion triée par `z-index`, stable (les
/// égalités gardent l'ordre d'arbre — App. E étapes 8-9).
fn push_z<'a>(q: &mut Vec<(Vec<u16>, &'a LayoutBox)>, path: Vec<u16>, bx: &'a LayoutBox) {
    let z = bx.z_index;
    let at = q.partition_point(|(_, c)| c.z_index <= z);
    q.insert(at, (path, bx));
}

/// R-58 — clip géométrique `clip-path: inset(…)` /
/// `clip: rect(…)` sur la border box (boîte de référence par
/// défaut, spec CSS Masking). v1 : `circle()`/`ellipse()`/
/// `polygon()`, la clause `round <radius>`, `url()` et les
/// `<geometry-box>` explicites retombent sur « pas de clip »
/// (tolérance — jamais de clip faux). `clip` ne s'applique
/// qu'aux boîtes absolument positionnées (CSS2 §11.1.2 —
/// le champ `position` peut rester `Static` sur un inline
/// positionné → lecture au style calculé). Les deux
/// propriétés se combinent par intersection.
#[allow(clippy::cast_possible_truncation)]
fn clip_geo_rect(bx: &LayoutBox, r: Rect, vp: (f32, f32)) -> Option<Rect> {
    let fs = bx
        .style
        .length("font-size")
        .map_or(16.0, |l| l.value as f32);
    let px = |tok: &str, base: f32| -> Option<f32> {
        let t = tok.trim();
        if t.eq_ignore_ascii_case("auto") {
            return None;
        }
        if let Some(p) = t.strip_suffix('%') {
            return p
                .trim()
                .parse::<f64>()
                .ok()
                .map(|n| (n as f32) * base / 100.0);
        }
        let l = parse_length(t)?;
        Some(match l.unit {
            Unit::Em | Unit::Rem => l.value as f32 * fs,
            Unit::Ex | Unit::Ch => l.value as f32 * fs * 0.5,
            Unit::Vw => l.value as f32 * vp.0 / 100.0,
            Unit::Vh => l.value as f32 * vp.1 / 100.0,
            Unit::Vmin => l.value as f32 * vp.0.min(vp.1) / 100.0,
            Unit::Vmax => l.value as f32 * vp.0.max(vp.1) / 100.0,
            u => (l.value * u.px_factor()?) as f32,
        })
    };
    let mut out: Option<Rect> = None;
    let mut and = |c: Rect| {
        out = Some(match out {
            None => c,
            // Intersection : le plus restrictif des deux clips.
            Some(a) => Rect {
                x: a.x.max(c.x),
                y: a.y.max(c.y),
                width: (a.x + a.width).min(c.x + c.width) - a.x.max(c.x),
                height: (a.y + a.height).min(c.y + c.height) - a.y.max(c.y),
            },
        });
        if let Some(o) = out {
            if o.width < 0.0 || o.height < 0.0 {
                out = Some(Rect {
                    width: 0.0,
                    height: 0.0,
                    ..o
                });
            }
        }
    };
    if let Some(v) = bx.style.get("clip-path") {
        if let Some(r2) = clip_path_inset(v.trim(), r, &px) {
            and(r2);
        }
    }
    let pos = bx.style.get("position").unwrap_or("static");
    if matches!(pos, "absolute" | "fixed") {
        if let Some(v) = bx.style.get("clip") {
            if let Some(r2) = legacy_clip_rect(v.trim(), r, &px) {
                and(r2);
            }
        }
    }
    out
}

/// `inset(t r b l [round …])` — 1 à 4 retraits depuis les
/// bords (top/right/bottom/left, raccourci façon `margin`) ;
/// la clause `round` est acceptée mais ignorée en v1
/// (coins carrés). `None` si la forme n'est pas un `inset()`
/// ou si un côté est inclassable (tolérance : pas de clip).
fn clip_path_inset(
    raw: &str,
    bounds: Rect,
    to_px: &dyn Fn(&str, f32) -> Option<f32>,
) -> Option<Rect> {
    let inner = raw
        .strip_prefix("inset")?
        .trim_start()
        .strip_prefix('(')?
        .strip_suffix(')')?;
    // Coupe la clause `round` éventuelle — les coins restent
    // carrés en v1.
    let inner = inner.split_once("round").map_or(inner, |(a, _)| a).trim();
    let toks: Vec<&str> = inner.split_whitespace().collect();
    if toks.is_empty() || toks.len() > 4 {
        return None;
    }
    let top = to_px(toks[0], bounds.height)?;
    let right = to_px(toks.get(1).unwrap_or(&toks[0]), bounds.width)?;
    let bottom = to_px(toks.get(2).unwrap_or(&toks[0]), bounds.height)?;
    let left = to_px(
        toks.get(3).unwrap_or(toks.get(1).unwrap_or(&toks[0])),
        bounds.width,
    )?;
    Some(Rect {
        x: bounds.x + left,
        y: bounds.y + top,
        width: (bounds.width - left - right).max(0.0),
        height: (bounds.height - top - bottom).max(0.0),
    })
}

/// `clip: rect(t, r, b, l)` (CSS2 §11.1.2 — virgules
/// facultatives) : bords mesurés depuis le coin haut-gauche
/// — `top`/`bottom` depuis le haut, `left`/`right` depuis la
/// gauche ; `auto` = bord de la boîte. `None` si la forme
/// n'est pas `rect()` ou le compte ≠ 4.
fn legacy_clip_rect(v: &str, r: Rect, px: &dyn Fn(&str, f32) -> Option<f32>) -> Option<Rect> {
    let inner = v
        .strip_prefix("rect")?
        .trim_start()
        .strip_prefix('(')?
        .strip_suffix(')')?;
    let toks: Vec<&str> = inner.split([',', ' ']).filter(|s| !s.is_empty()).collect();
    if toks.len() != 4 {
        return None;
    }
    // `auto` = le bord correspondant de la boîte.
    let side = |tok: &str, base: f32, dft: f32| -> Option<f32> {
        if tok.trim().eq_ignore_ascii_case("auto") {
            Some(dft)
        } else {
            px(tok, base)
        }
    };
    let top = side(toks[0], r.height, 0.0)?;
    let right = side(toks[1], r.width, r.width)?;
    let bottom = side(toks[2], r.height, r.height)?;
    let left = side(toks[3], r.width, 0.0)?;
    let x = r.x + left;
    let y = r.y + top;
    Some(Rect {
        x,
        y,
        width: (r.x + right - x).max(0.0),
        height: (r.y + bottom - y).max(0.0),
    })
}

#[allow(clippy::too_many_lines)]
fn paint_box<'a>(
    bx: &'a LayoutBox,
    items: &mut Vec<DisplayItem>,
    off: (f32, f32),
    cx: &mut PaintCx<'a>,
    defer: &mut Vec<(Vec<u16>, &'a LayoutBox)>,
    pos_defer: &mut Vec<(Vec<u16>, &'a LayoutBox)>,
) {
    // `position: fixed` est ancrée au viewport : son sous-arbre
    // ignore le scroll.
    let off = if bx.position == Position::Fixed {
        (0.0, 0.0)
    } else {
        off
    };

    // `transform` : matrice poussée avant tout item du
    // sous-arbre, retiré après (imbrication additive).
    let tf = box_transform(bx, &shifted(bx.rect, off));
    if let Some(m) = tf {
        items.push(DisplayItem::PushTransform(m));
    }
    // R-58 — `clip-path`/`clip` : clip géométrique sur la
    // border box — il borne TOUT le rendu de la boîte (fond,
    // bordures, lignes, enfants) contrairement au clip
    // `overflow` qui n'englobe que le contenu. Émis dans la
    // portée transform : le raster transforme le rect comme
    // le clip `overflow`. Limite v1 : les descendants
    // positionnés reportés au contexte ancêtre en sortent.
    let geo_clip = clip_geo_rect(bx, shifted(bx.rect, off), cx.vp);
    if let Some(gc) = geo_clip {
        items.push(DisplayItem::PushClip(gc));
    }
    // `visibility` (§11.2) : `hidden`/`collapse` interdit
    // l'émission PROPRE mais la boîte garde sa place en
    // layout et la descente continue — un descendant
    // `visibility:visible` explicite se repeint (son style
    // calculé porte déjà l'héritage → vérification locale).
    // Transform/clip/enfants non court-circuités : le
    // sous-arbre visible s'il existe doit garder sa matrice
    // et son clip.
    let hidden = matches!(bx.style.get("visibility"), Some("hidden" | "collapse"));
    if !hidden {
        paint_self(bx, items, off, cx.vp, cx.images, cx.canvas_bg_src);
        paint_replaced(bx, items, off, cx.images);
    }
    if !bx.children.is_empty() || !bx.lines.is_empty() {
        // Clip sur le padding box (approx : border box moins les
        // bordures — on garde le rect entier, documenté).
        // `bx.lines` compte pour l'ouverture : les lignes
        // propres sont du contenu autant que les enfants (R-55).
        let clipped = bx.overflow != Overflow::Visible;
        if clipped {
            items.push(DisplayItem::PushClip(shifted(bx.rect, off)));
        }
        // Fragments de lignes propres (racine d'un contexte
        // inline — élément inline positionné, anon…) : du
        // contenu → DANS le clip, pas avec les décorations de
        // `paint_self` émises avant (R-55).
        if !hidden {
            paint_lines(bx, items, off);
        }
        // Ordre App. E au sein des non positionnés : (a)
        // blocs en flux, (b) flottants, (c) contenu en ligne
        // reporté — les porteurs inline remontent au contexte
        // de pilement englobant (`defer` hérité) sauf si cette
        // boîte en ferme la portée (`inline_scope`). Les
        // positionnés à `z-index` **négatif** passent avant
        // les blocs en flux (étape 2 de l'ordre de peinture —
        // le fond d'erreur `z-index:-1` des reftests WPT se
        // peint sous la grille, pas dessus).
        let mut scoped_defer: Vec<(Vec<u16>, &LayoutBox)> = Vec::new();
        let d: &mut Vec<(Vec<u16>, &LayoutBox)> = if inline_scope(bx) {
            &mut scoped_defer
        } else {
            defer
        };
        // File des positionnés `z ≥ 0` : le contexte d'empilement
        // englobant les peint après TOUT le flux normal — un
        // parent `position:relative` à `z-index:auto` ne ferme
        // pas leur phase (App. E). `my_pos` seulement si cette
        // boîte crée le contexte ; sinon file héritée.
        let mut my_pos: Vec<(Vec<u16>, &LayoutBox)> = Vec::new();
        let pd: &mut Vec<(Vec<u16>, &LayoutBox)> = if stacking_ctx(bx) {
            &mut my_pos
        } else {
            pos_defer
        };
        let bound = bx.rect;
        let in_flow = |c: &&LayoutBox| matches!(c.position, Position::Static | Position::Sticky);
        let mut neg_z: Vec<(usize, &LayoutBox)> = bx
            .children
            .iter()
            .enumerate()
            .filter(|(_, c)| !in_flow(c) && c.z_index < 0)
            .collect();
        neg_z.sort_by_key(|(_, c)| c.z_index);
        for (ci, child) in neg_z {
            emit_scoped(ci, child, items, off, cx, pd);
        }
        // (a) blocs en flux — `sticky` garde sa translation de
        // peinture ; récursion `deep` ∝ profondeur (R-24).
        for (ci, child) in bx.children.iter().enumerate() {
            if !in_flow(&child) || child.float != Float::None || inline_carrier(child) {
                continue;
            }
            let stick = sticky_offset(child, off, bound, cx.vp);
            if stick != (0.0, 0.0) {
                items.push(DisplayItem::PushTransform(
                    tiny_skia::Transform::from_translate(stick.0, stick.1),
                ));
            }
            emit_child(ci, child, items, off, cx, d, pd);
            if stick != (0.0, 0.0) {
                items.push(DisplayItem::PopTransform);
            }
        }
        // (b) flottants non positionnés — portée propre.
        for (ci, child) in bx.children.iter().enumerate() {
            if in_flow(&child) && child.float != Float::None {
                emit_scoped(ci, child, items, off, cx, pd);
            }
        }
        // (c) porteurs inline — reportés au contexte englobant.
        for (ci, child) in bx.children.iter().enumerate() {
            if in_flow(&child) && inline_carrier(child) {
                // Le chemin complet est capturé ici — le drain
                // peut avoir lieu chez un ancêtre englobant.
                let mut p = cx.path.clone();
                p.push(u16::try_from(ci).unwrap_or(u16::MAX));
                d.push((p, child));
            }
        }
        // Si cette boîte ferme la portée : drainer ses inlines
        // dans l'ordre de collecte (étape 5).
        let mut i = 0;
        while i < scoped_defer.len() {
            let (p, c) = scoped_defer[i].clone();
            i += 1;
            emit_scoped_at(p, c, items, off, cx, pd);
        }
        // Enfants positionnés `z-index` ≥ 0 : enfilés triés dans
        // la file du contexte — drainés à sa fermeture (ici si
        // `my_pos`), sinon au contexte ancêtre ou à la racine.
        for (ci, child) in bx.children.iter().enumerate() {
            if in_flow(&child) || child.z_index < 0 {
                continue;
            }
            let mut p = cx.path.clone();
            p.push(u16::try_from(ci).unwrap_or(u16::MAX));
            push_z(pd, p, child);
        }
        let mut i = 0;
        while i < my_pos.len() {
            let (p, c) = my_pos[i].clone();
            i += 1;
            emit_scoped_at(p, c, items, off, cx, &mut my_pos);
        }
        if clipped {
            items.push(DisplayItem::PopClip);
        }
    }
    if geo_clip.is_some() {
        items.push(DisplayItem::PopClip);
    }
    if tf.is_some() {
        items.push(DisplayItem::PopTransform);
    }
}

#[cfg(test)]
mod tests {
    use super::{merge_rects, rect_union};
    use explorer_layout::Rect;
    use std::time::Instant;

    fn r(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect {
            x,
            y,
            width: w,
            height: h,
        }
    }

    /// Aucun rect ne chevauche `a` directement, mais `b ∪ c`
    /// le couvre — l'ancienne passe unique laissait `a`
    /// séparé (jamais de point fixe).
    #[test]
    fn merge_point_fixe_pont_bbox() {
        let mut v = vec![
            r(0.0, 0.0, 10.0, 10.0),
            r(0.0, 15.0, 30.0, 5.0),
            r(20.0, 0.0, 10.0, 16.0),
        ];
        merge_rects(&mut v);
        assert_eq!(v.len(), 1);
        let u = v[0];
        assert_eq!((u.x, u.y, u.width, u.height), (0.0, 0.0, 30.0, 20.0));
    }

    #[test]
    fn merge_preserve_la_couverture() {
        let src = [
            r(0.0, 0.0, 5.0, 5.0),
            r(4.0, 4.0, 5.0, 5.0),
            r(40.0, 0.0, 8.0, 8.0),
            r(40.0, 8.0, 8.0, 8.0),
            r(0.0, 40.0, 4.0, 4.0),
        ];
        let total = src.iter().copied().reduce(rect_union);
        let mut v = src.to_vec();
        merge_rects(&mut v);
        let merged = v.iter().copied().reduce(rect_union);
        assert_eq!(merged, total);
        // Disjoints non chaînés : jamais de fusion abusive.
        assert!(v.len() <= src.len());
    }

    /// Le diff dégénéré journalisé par NAV : ~5 000 rects en
    /// chaîne diagonale — l'ancien O(k²) coûtait ~2 s sur
    /// ~8 500 ; le balayage doit rester très en deçà.
    #[test]
    fn merge_degenere_reste_rapide() {
        let mut v: Vec<Rect> = (0..5_000_u16)
            .map(|i| r(f32::from(i) * 8.0, (f32::from(i) * 8.0) % 600.0, 20.0, 20.0))
            .collect();
        let t0 = Instant::now();
        merge_rects(&mut v);
        assert!(
            t0.elapsed().as_secs() < 2,
            "merge_rects trop lent : {:?}",
            t0.elapsed()
        );
    }
}
