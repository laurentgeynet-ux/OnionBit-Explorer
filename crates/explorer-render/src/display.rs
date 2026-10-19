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
use explorer_layout::{BoxKind, LayoutBox, Overflow, Position, Rect};

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
}

/// Construit la display list d'un arbre posé — pur,
/// déterministe, borné par la taille de l'arbre. `height`
/// couvre au moins le viewport. `scroll_y` décale tout le
/// contenu vers le haut, **sauf** les sous-arbres
/// `position: fixed` qui restent ancrés au viewport.
/// `images` = pixels décodés par `NodeId` `<img>` — une
/// boîte remplacée (`intrinsic` posé) peint son bitmap.
/// [`build_display_list`] équivaut à une map vide.
#[must_use]
pub fn build_display_list_imaged(
    root: &LayoutBox,
    viewport_w: f32,
    viewport_h: f32,
    scroll_y: f32,
    images: &crate::ImageTable,
) -> DisplayList {
    let mut items = Vec::new();
    paint_box(root, &mut items, -scroll_y, images);
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
fn shifted(r: Rect, dy: f32) -> Rect {
    Rect { y: r.y + dy, ..r }
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
            .and_then(|l| l.unit.px_factor().map(|f| l.value * f))
            .map_or(fallback, |v| v as f32),
    }
}

/// Fond + bordures + texte propre d'une boîte (avant les
/// enfants — le fond passe TOUJOURS sous son contenu).
/// `off` = décalage vertical de scroll (`0` dans les
/// sous-arbres `position: fixed`).
fn paint_self(bx: &LayoutBox, items: &mut Vec<DisplayItem>, off: f32) {
    let rect = shifted(bx.rect, off);
    // Blocs anonymes et boîtes texte clonent le style du
    // parent, mais `background`/`border` ne sont PAS hérités
    // en CSS : ils ne peignent ni fond ni bordure propre
    // (sinon le clone masquerait les flottants posés avant,
    // ou la bordure du parent serait redessinée sur chaque
    // fragment de texte — §9.2.1.1).
    let anonymous = matches!(bx.kind, BoxKind::AnonymousBlock | BoxKind::Text);
    // Fond : `background-color` non transparente.
    if !anonymous {
        if let Some(c) = bx.style.get("background-color").and_then(parse_color) {
            if c.a > 0 {
                items.push(DisplayItem::FillRect { rect, color: c });
            }
        }
    }
    // Bordures : 4 côtés, épaisseur > 0 et style non `none`.
    let widths = if anonymous {
        [0.0; 4]
    } else {
        [
            prop_px(&bx.style, "border-top-width", 0.0),
            prop_px(&bx.style, "border-right-width", 0.0),
            prop_px(&bx.style, "border-bottom-width", 0.0),
            prop_px(&bx.style, "border-left-width", 0.0),
        ]
    };
    if widths.iter().any(|v| *v > 0.0) {
        let color_of = |side: &str| {
            bx.style
                .get(side)
                .and_then(parse_color)
                .unwrap_or(Color::BLACK)
        };
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
    if !bx.lines.is_empty() {
        let ink = bx
            .style
            .get("color")
            .and_then(parse_color)
            .unwrap_or(Color::BLACK);
        let size = prop_px(&bx.style, "font-size", 16.0);
        for line in &bx.lines {
            for frag in &line.fragments {
                if frag.atomic {
                    continue;
                }
                if let Some(run) = &frag.shaped {
                    items.push(DisplayItem::Text {
                        origin: (frag.rect.x, frag.rect.y + frag.baseline + off),
                        run: run.clone(),
                        font: font_request(&bx.style),
                        color: ink,
                        size_px: size,
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

/// Décalage `position: sticky` (seuil `top` uniquement) :
/// `off` = translation de scroll déjà appliquée (négative),
/// `bound_bottom` = bas du bloc conteneur en coordonnées
/// document. Retourne `> 0` quand le collage est engagé —
/// borné pour que la boîte ne sorte pas de son parent.
/// `bottom` non supporté (seuil haut seul — MVP documenté).
fn sticky_dy(bx: &LayoutBox, off: f32, bound_bottom: f32) -> f32 {
    if bx.position != Position::Sticky {
        return 0.0;
    }
    let top = prop_px(&bx.style, "top", 0.0);
    let painted_top = bx.rect.y + off;
    let room = (bound_bottom + off - painted_top - bx.rect.height).max(0.0);
    (top - painted_top).clamp(0.0, room)
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
fn paint_box(bx: &LayoutBox, items: &mut Vec<DisplayItem>, off: f32, images: &crate::ImageTable) {
    // `position: fixed` est ancrée au viewport : son sous-arbre
    // ignore le scroll.
    let off = if bx.position == Position::Fixed {
        0.0
    } else {
        off
    };
    // `transform` : matrice poussée avant tout item du
    // sous-arbre, retirée après (imbrication additive).
    let tf = box_transform(bx, &shifted(bx.rect, off));
    if let Some(m) = tf {
        items.push(DisplayItem::PushTransform(m));
    }
    paint_self(bx, items, off);
    // Élément remplacé : bitmap par-dessus bordure (contenu —
    // ordre CSS 2.1 App. E : fond → bordure → contenu).
    if let Some(img) = bx.node.and_then(|n| images.get(&n)).cloned() {
        items.push(DisplayItem::Image {
            rect: shifted(bx.rect, off),
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
        // Enfants non positionnés d'abord (ordre de l'arbre) —
        // `sticky` reste en flux : sa contrainte est une
        // translation de peinture, pas un re-flow.
        let bound_bottom = bx.rect.y + bx.rect.height;
        for child in &bx.children {
            if matches!(child.position, Position::Static | Position::Sticky) {
                let stick = sticky_dy(child, off, bound_bottom);
                if stick > 0.0 {
                    items.push(DisplayItem::PushTransform(
                        tiny_skia::Transform::from_translate(0.0, stick),
                    ));
                }
                paint_box(child, items, off, images);
                if stick > 0.0 {
                    items.push(DisplayItem::PopTransform);
                }
            }
        }
        // Enfants positionnés ensuite, triés par z-index (stable).
        let mut positioned: Vec<&LayoutBox> = bx
            .children
            .iter()
            .filter(|c| !matches!(c.position, Position::Static | Position::Sticky))
            .collect();
        positioned.sort_by_key(|c| c.z_index);
        for child in positioned {
            paint_box(child, items, off, images);
        }
        if clipped {
            items.push(DisplayItem::PopClip);
        }
    }
    if tf.is_some() {
        items.push(DisplayItem::PopTransform);
    }
}
