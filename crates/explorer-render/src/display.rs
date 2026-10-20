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
use explorer_css::value::{parse_color, parse_length};
use explorer_layout::font::ShapedRun;
use explorer_layout::{BoxKind, Float, Fragment, LayoutBox, Overflow, Position, Rect};

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
        /// Glyphes + métriques produits par le shaping.
        run: ShapedRun,
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
}

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
fn resolved_bounds(items: &[DisplayItem]) -> Vec<Option<Rect>> {
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
        | DisplayItem::BackgroundImage { rect, .. } => Some(*rect),
        DisplayItem::Text { origin, run, .. } => Some(Rect {
            x: origin.0,
            y: origin.1 - run.metrics.ascent,
            width: run.advance,
            height: run.metrics.ascent - run.metrics.descent,
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

/// Fusionne les rects qui se chevauchent (point fixe) — borne
/// le nombre de zones rapportées sans perdre de couverture.
fn merge_rects(rects: &mut Vec<Rect>) {
    let mut i = 0;
    while i < rects.len() {
        let mut j = i + 1;
        while j < rects.len() {
            if rect_intersect(rects[i], rects[j]).is_some() {
                rects[i] = rect_union(rects[i], rects[j]);
                rects.swap_remove(j);
            } else {
                j += 1;
            }
        }
        i += 1;
    }
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
    let mut items = Vec::new();
    // Portée racine du report des inlines (App. E étape 5).
    let mut defer = Vec::new();
    paint_box(
        root,
        &mut items,
        (-scroll.0, -scroll.1),
        (viewport_w, viewport_h),
        images,
        &mut defer,
    );
    let mut i = 0;
    while i < defer.len() {
        let c = defer[i];
        i += 1;
        paint_scoped(
            c,
            &mut items,
            (-scroll.0, -scroll.1),
            (viewport_w, viewport_h),
            images,
        );
    }
    DisplayList {
        items,
        width: viewport_w,
        height: root.rect.height.max(viewport_h),
    }
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
/// une valeur seule → x, l'autre `center` ; `em` hors
/// portée → 0). Acid2 utilise `1px 0` sur les yeux.
#[allow(clippy::cast_possible_truncation)]
fn bg_position(raw: Option<&str>, area: (f32, f32), img: (f32, f32)) -> (f32, f32) {
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
                            .and_then(|l| l.unit.px_factor().map(|f| l.value * f))
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
    let inline_flow = bx.kind == BoxKind::Inline
        && bx.intrinsic.is_none()
        && bx.display() != "inline-block"
        && bx.display() != "inline-flex"
        && bx.display() != "inline-table";
    // Fond : `background-color` non transparente, puis
    // `background-image` décodée (au-dessus de la couleur,
    // sous la bordure — App. E ; origine padding box ≈ bord
    // interne de la bordure gauche/haute).
    if !anonymous && !inline_flow {
        if let Some(c) = bx.style.get("background-color").and_then(parse_color) {
            if c.a > 0 {
                items.push(DisplayItem::FillRect { rect, color: c });
            }
        }
        if bx
            .style
            .get("background-image")
            .is_some_and(|v| v.starts_with("url("))
        {
            if let Some(img) = bx.node.and_then(|n| images.backgrounds.get(&n)).cloned() {
                let repeat = match bx.style.get("background-repeat") {
                    Some("no-repeat") => (false, false),
                    Some("repeat-x") => (true, false),
                    Some("repeat-y") => (false, true),
                    _ => (true, true), // `repeat` = défaut.
                };
                // Zone de positionnement : border box ≈ padding
                // box pour `scroll`, **viewport** pour `fixed`
                // (CSS2 §14.2.1 — le fond reste calé à l'écran).
                // L'origine est en coords FRAME : le viewport
                // y démarre en (0,0) — ancrer au scroll
                // déphaserait la tuile de `scroll mod taille`.
                let fixed = bx.style.get("background-attachment") == Some("fixed");
                let (area, base) = if fixed {
                    ((vp.0, vp.1), (0.0, 0.0))
                } else {
                    ((rect.width, rect.height), (rect.x, rect.y))
                };
                let (px, py) = bg_position(
                    bx.style.get("background-position"),
                    area,
                    (img.width as f32, img.height as f32),
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
    // Texte : fragments non atomiques des lignes produites
    // par le layout en-ligne (les atomes = enfants
    // inline-block, peints à leur place en récursion).
    // `frag.style` = style de l'élément inline propriétaire
    // (le plus proche du texte) — `a{color}`/`span{color}`
    // colorent leurs runs, pas seulement la boîte (R-18).
    if !bx.lines.is_empty() {
        for line in &bx.lines {
            for frag in &line.fragments {
                if frag.atomic {
                    continue;
                }
                // R-22 : fond + bordure de l'élément INLINE
                // propriétaire, tranchés par fragment. Le texte
                // direct du conteneur porte `bx.style` (déjà
                // peint sur la boîte entière) → sauté ; un
                // fragment de style différent appartient à un
                // élément inline avec ses propres déclarations.
                if frag.style != bx.style {
                    paint_inline_deco(frag, items, off);
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

/// Décorations d'un fragment d'élément inline (R-22) : fond et
/// bordure sur le rect du fragment étendu de son padding et des
/// épaisseurs de bordure.
///
/// Approximation « `box-decoration-break: clone` » documentée :
/// chaque fragment porte les 4 côtés — la spec `slice` ouvre
/// les bords verticaux aux coupures de ligne ; et `em`/`rem`
/// ≈ font-size du fragment (pas de contexte racine ici).
fn paint_inline_deco(frag: &Fragment, items: &mut Vec<DisplayItem>, off: (f32, f32)) {
    let st = &frag.style;
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
            x: frag.rect.x - l,
            y: frag.rect.y - t,
            width: frag.rect.width + l + r,
            height: frag.rect.height + t + b,
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
    bx.position != Position::Static
        || bx.float != Float::None
        || bx.intrinsic.is_some()
        || (bx.kind == BoxKind::Inline && bx.children.iter().any(|c| !c.kind.is_inline_level()))
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
fn paint_scoped(
    bx: &LayoutBox,
    items: &mut Vec<DisplayItem>,
    off: (f32, f32),
    vp: (f32, f32),
    images: &crate::PaintImages,
) {
    let mut defer = Vec::new();
    paint_box(bx, items, off, vp, images, &mut defer);
    let mut i = 0;
    while i < defer.len() {
        let c = defer[i];
        i += 1;
        paint_scoped(c, items, off, vp, images);
    }
}

fn paint_box<'a>(
    bx: &'a LayoutBox,
    items: &mut Vec<DisplayItem>,
    off: (f32, f32),
    vp: (f32, f32),
    images: &crate::PaintImages,
    defer: &mut Vec<&'a LayoutBox>,
) {
    // `position: fixed` est ancrée au viewport : son sous-arbre
    // ignore le scroll.
    let off = if bx.position == Position::Fixed {
        (0.0, 0.0)
    } else {
        off
    };
    // `transform` : matrice poussée avant tout item du
    // sous-arbre, retirée après (imbrication additive).
    let tf = box_transform(bx, &shifted(bx.rect, off));
    if let Some(m) = tf {
        items.push(DisplayItem::PushTransform(m));
    }
    paint_self(bx, items, off, vp, images);
    // Élément remplacé : bitmap par-dessus bordure (contenu —
    // ordre CSS 2.1 App. E : fond → bordure → contenu).
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
        items.push(DisplayItem::Image {
            // Le contenu remplacé peint dans la **content
            // box** — padding et bordures restent autour
            // (§10.3.2 — Acid2 : l'œil 96×24 étiré à 131
            // sinon, couvrant padding 11/12 + bordure 12).
            rect: shifted(content_rect(bx), off),
            img,
        });
    }
    if !bx.children.is_empty() {
        // Clip sur le padding box (approx : border box moins les
        // bordures — on garde le rect entier, documenté).
        let clipped = bx.overflow != Overflow::Visible;
        if clipped {
            items.push(DisplayItem::PushClip(shifted(bx.rect, off)));
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
        let mut scoped_defer: Vec<&LayoutBox> = Vec::new();
        let d: &mut Vec<&LayoutBox> = if inline_scope(bx) {
            &mut scoped_defer
        } else {
            defer
        };
        let bound = bx.rect;
        let in_flow = |c: &&LayoutBox| matches!(c.position, Position::Static | Position::Sticky);
        let mut neg_z: Vec<&LayoutBox> = bx
            .children
            .iter()
            .filter(|c| !in_flow(c) && c.z_index < 0)
            .collect();
        neg_z.sort_by_key(|c| c.z_index);
        for child in neg_z {
            explorer_common::deep(|| paint_scoped(child, items, off, vp, images));
        }
        // (a) blocs en flux — `sticky` garde sa translation de
        // peinture ; récursion `deep` ∝ profondeur (R-24).
        for child in &bx.children {
            if !in_flow(&child) || child.float != Float::None || inline_carrier(child) {
                continue;
            }
            let stick = sticky_offset(child, off, bound, vp);
            if stick != (0.0, 0.0) {
                items.push(DisplayItem::PushTransform(
                    tiny_skia::Transform::from_translate(stick.0, stick.1),
                ));
            }
            explorer_common::deep(|| paint_box(child, items, off, vp, images, d));
            if stick != (0.0, 0.0) {
                items.push(DisplayItem::PopTransform);
            }
        }
        // (b) flottants non positionnés — portée propre.
        for child in &bx.children {
            if in_flow(&child) && child.float != Float::None {
                explorer_common::deep(|| paint_scoped(child, items, off, vp, images));
            }
        }
        // (c) porteurs inline — reportés au contexte englobant.
        for child in &bx.children {
            if in_flow(&child) && inline_carrier(child) {
                d.push(child);
            }
        }
        // Si cette boîte ferme la portée : drainer ses inlines
        // dans l'ordre de collecte (étape 5).
        let mut i = 0;
        while i < scoped_defer.len() {
            let c = scoped_defer[i];
            i += 1;
            paint_scoped(c, items, off, vp, images);
        }
        // Enfants positionnés `z-index` ≥ 0 ensuite, triés
        // (stable) — les négatifs ont déjà été peints sous
        // les blocs en flux.
        let mut positioned: Vec<&LayoutBox> = bx
            .children
            .iter()
            .filter(|c| !in_flow(c) && c.z_index >= 0)
            .collect();
        positioned.sort_by_key(|c| c.z_index);
        for child in positioned {
            explorer_common::deep(|| paint_scoped(child, items, off, vp, images));
        }
        if clipped {
            items.push(DisplayItem::PopClip);
        }
    }
    if tf.is_some() {
        items.push(DisplayItem::PopTransform);
    }
}
