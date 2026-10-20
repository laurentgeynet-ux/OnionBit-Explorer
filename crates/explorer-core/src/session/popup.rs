// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Popup natif `<select>` — modèle, géométrie et peinture dans
//! la display list (ADR-0014).

use explorer_css::value::Color;
use explorer_layout::Rect;
use explorer_render::{BorderStyle, DisplayItem, DisplayList};

/// Popup `<select>` moteur (étape 54) — l'état vit hors DOM :
/// les options sont figées à l'ouverture, le `<select>` est
/// re-résolu par chemin au moment du choix (le nœud peut
/// mourir au re-parse de synchro).
pub(crate) struct SelectPopup {
    /// Chemin `(tag, index)` du `<select>` dans `dom`.
    pub(crate) path: Vec<(String, u32)>,
    /// Boîte du contrôle (coordonnées document).
    pub(crate) rect: Rect,
    /// `(étiquette, sélectionnée)` par `<option>` descendant.
    pub(crate) options: Vec<(String, bool)>,
    /// Hauteur d'une ligne du popup (px).
    pub(crate) row_h: f32,
    /// Ligne survolée au dernier `mousemove` — surlignage
    /// hover, sans sélection tant que non cliquée.
    pub(crate) hovered: Option<usize>,
}

/// Lignes visibles du popup `<select>` — le reste des
/// options est tronqué (MVP, pas de défilement interne).
pub(crate) const MAX_POPUP_ROWS: usize = 8;

/// Rectangle du popup ouvert (coords document) : sous le
/// contrôle, basculé au-dessus quand il déborderait la page.
pub(crate) fn popup_rect(pop: &SelectPopup, vw: f32, doc_h: f32) -> Rect {
    // ≤ `MAX_POPUP_ROWS` lignes — la conversion est exacte.
    #[allow(clippy::cast_precision_loss)]
    let rows = pop.options.len().min(MAX_POPUP_ROWS) as f32;
    let w = pop.rect.width.max(140.0).min(vw.max(1.0));
    let x = pop.rect.x.clamp(0.0, (vw - w).max(0.0));
    let h = pop.row_h * rows;
    let below = pop.rect.y + pop.rect.height;
    let y = if below + h <= doc_h {
        below
    } else {
        (pop.rect.y - h).max(0.0)
    };
    Rect {
        x,
        y,
        width: w,
        height: h,
    }
}

/// Ligne du popup sous `(x, doc_y)` — `None` hors du
/// rectangle du popup.
pub(crate) fn popup_row(
    pop: &SelectPopup,
    x: f32,
    doc_y: f32,
    vw: f32,
    doc_h: f32,
) -> Option<usize> {
    let r = popup_rect(pop, vw, doc_h);
    if !point_in(&r, x, doc_y) {
        return None;
    }
    // `point_in` garantit `doc_y - r.y ∈ [0, height]` — la
    // troncature est le numéro de ligne attendu.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let idx = ((doc_y - r.y) / pop.row_h) as usize;
    (idx < pop.options.len().min(MAX_POPUP_ROWS)).then_some(idx)
}

/// Point `(x, y)` dans `rect` (bornes incluses).
pub(crate) fn point_in(r: &Rect, x: f32, y: f32) -> bool {
    x >= r.x && x <= r.x + r.width && y >= r.y && y <= r.y + r.height
}

/// Overlay du popup `<select>` en fin de display list —
/// fond opaque + bordure, lignes `Placeholder` (étiquette
/// rendue), ligne sélectionnée ou survolée teintée.
pub(crate) fn append_select_popup(list: &mut DisplayList, pop: &SelectPopup, vw: f32, doc_h: f32) {
    if pop.options.is_empty() {
        return;
    }
    let rect = popup_rect(pop, vw, doc_h);
    let white = Color {
        r: 255,
        g: 255,
        b: 255,
        a: 255,
    };
    let edge = Color {
        r: 118,
        g: 118,
        b: 118,
        a: 255,
    };
    let sel_bg = Color {
        r: 30,
        g: 106,
        b: 219,
        a: 255,
    };
    let hov_bg = Color {
        r: 222,
        g: 235,
        b: 255,
        a: 255,
    };
    list.items
        .push(DisplayItem::FillRect { rect, color: white });
    list.items.push(DisplayItem::Border {
        rect,
        widths: [1.0; 4],
        colors: [edge; 4],
        styles: [BorderStyle::Solid; 4],
    });
    for (i, (label, selected)) in pop.options.iter().take(MAX_POPUP_ROWS).enumerate() {
        // `i < MAX_POPUP_ROWS` — conversion exacte.
        #[allow(clippy::cast_precision_loss)]
        let row = Rect {
            x: rect.x,
            y: rect.y + i as f32 * pop.row_h,
            width: rect.width,
            height: pop.row_h,
        };
        if *selected {
            list.items.push(DisplayItem::FillRect {
                rect: row,
                color: sel_bg,
            });
        } else if pop.hovered == Some(i) {
            list.items.push(DisplayItem::FillRect {
                rect: row,
                color: hov_bg,
            });
        }
        list.items.push(DisplayItem::Placeholder {
            rect: row,
            label: label.clone(),
        });
    }
}
