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
use explorer_layout::{LayoutBox, Overflow, Position, Rect};

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
    /// Entrée de zone de clipping (`overflow` ≠ `visible`).
    PushClip(Rect),
    /// Sortie de la zone de clipping correspondante.
    PopClip,
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
                DisplayItem::PushClip(r) => {
                    let _ = writeln!(out, "push-clip {r:?}");
                }
                DisplayItem::PopClip => out.push_str("pop-clip\n"),
            }
        }
        out
    }
}

/// Construit la display list d'un arbre posé — pur,
/// déterministe, borné par la taille de l'arbre.
#[must_use]
pub fn build_display_list(root: &LayoutBox, viewport_w: f32) -> DisplayList {
    let mut items = Vec::new();
    paint_box(root, &mut items);
    DisplayList {
        items,
        width: viewport_w,
        height: root.rect.height,
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
            .and_then(|l| l.unit.px_factor().map(|f| l.value * f))
            .map_or(fallback, |v| v as f32),
    }
}

/// Fond + bordures + texte propre d'une boîte (avant les
/// enfants — le fond passe TOUJOURS sous son contenu).
fn paint_self(bx: &LayoutBox, items: &mut Vec<DisplayItem>) {
    // Fond : `background-color` non transparente.
    if let Some(c) = bx.style.get("background-color").and_then(parse_color) {
        if c.a > 0 {
            items.push(DisplayItem::FillRect {
                rect: bx.rect,
                color: c,
            });
        }
    }
    // Bordures : 4 côtés, épaisseur > 0 et style non `none`.
    let widths = [
        prop_px(&bx.style, "border-top-width", 0.0),
        prop_px(&bx.style, "border-right-width", 0.0),
        prop_px(&bx.style, "border-bottom-width", 0.0),
        prop_px(&bx.style, "border-left-width", 0.0),
    ];
    if widths.iter().any(|v| *v > 0.0) {
        let color_of = |side: &str| {
            bx.style
                .get(side)
                .and_then(parse_color)
                .unwrap_or(Color::BLACK)
        };
        items.push(DisplayItem::Border {
            rect: bx.rect,
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
                        origin: (frag.rect.x, frag.rect.y + frag.baseline),
                        run: run.clone(),
                        color: ink,
                        size_px: size,
                    });
                }
            }
        }
    }
}

/// Peint une boîte et ses descendants dans l'ordre borné.
fn paint_box(bx: &LayoutBox, items: &mut Vec<DisplayItem>) {
    paint_self(bx, items);
    if bx.children.is_empty() {
        return;
    }
    // Clip sur le padding box (approx : border box moins les
    // bordures — on garde le rect entier, documenté).
    let clipped = bx.overflow != Overflow::Visible;
    if clipped {
        items.push(DisplayItem::PushClip(bx.rect));
    }
    // Enfants non positionnés d'abord (ordre de l'arbre).
    for child in &bx.children {
        if child.position == Position::Static {
            paint_box(child, items);
        }
    }
    // Enfants positionnés ensuite, triés par z-index (stable).
    let mut positioned: Vec<&LayoutBox> = bx
        .children
        .iter()
        .filter(|c| c.position != Position::Static)
        .collect();
    positioned.sort_by_key(|c| c.z_index);
    for child in positioned {
        paint_box(child, items);
    }
    if clipped {
        items.push(DisplayItem::PopClip);
    }
}
