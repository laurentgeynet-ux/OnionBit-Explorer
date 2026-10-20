// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Popup natif `<select>` — modèle, géométrie et peinture dans
//! la display list (ADR-0014).

use explorer_css::value::Color;
use explorer_layout::font::{self, FontRequest, FontSource};
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

/// Retrait horizontal du texte dans une ligne du popup.
const PAD_X: f32 = 5.0;

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
/// fond opaque + bordure, étiquettes d'options en vrai
/// texte shapé (étape 63 — `Placeholder` ne peignait qu'un
/// gris translucide), ligne sélectionnée/survolée teintée.
/// `fonts` résout la fonte UI (`sans-serif`) — un label
/// trop long est tronqué avec `...` plutôt que déborder.
pub(crate) fn append_select_popup(
    list: &mut DisplayList,
    pop: &SelectPopup,
    vw: f32,
    doc_h: f32,
    fonts: &dyn FontSource,
) {
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
    let ink = Color {
        r: 32,
        g: 32,
        b: 32,
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
    // Police chrome du popup : `sans-serif` ~¾ de ligne,
    // bornée — lisible sans dépendre du style de la page.
    let size_px = (pop.row_h * 0.75).clamp(9.0, 16.0);
    let freq = FontRequest {
        families: vec!["sans-serif".to_string()],
        weight: 400,
        italic: false,
    };
    let data = fonts.font_data(&freq);
    let avail = (rect.width - 2.0 * PAD_X).max(0.0);
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
        let Some(run) = data
            .as_deref()
            .and_then(|d| shape_fitted(d, label, size_px, avail))
        else {
            continue;
        };
        // Baseline centrée dans la ligne.
        let lh = run.metrics.line_height();
        let baseline = row.y + (row.height - lh).max(0.0) / 2.0 + run.metrics.ascent;
        list.items.push(DisplayItem::Text {
            origin: (row.x + PAD_X, baseline),
            run: std::sync::Arc::new(run),
            font: freq.clone(),
            color: if *selected { white } else { ink },
            size_px,
        });
    }
}

/// Shape `text` à `size_px` en tronquant à `avail` px de
/// large — le suffixe `...` remplace la queue (ASCII, le
/// glyphe `…` n'est pas garanti dans la fonte). `None` si
/// la fonte est illisible.
fn shape_fitted(font_data: &[u8], text: &str, size_px: f32, avail: f32) -> Option<font::ShapedRun> {
    let run = font::shape(font_data, text, size_px, false)?;
    if run.advance <= avail {
        return Some(run);
    }
    // Tronquage : retire des caractères jusqu'à tenir
    // (étiquettes courtes — le re-shape itératif est borné
    // par la longueur du label).
    let mut t = text.to_string();
    while !t.is_empty() {
        t.pop();
        let cand = format!("{t}...");
        let r = font::shape(font_data, &cand, size_px, false)?;
        if r.advance <= avail {
            return Some(r);
        }
    }
    font::shape(font_data, "...", size_px, false).filter(|r| r.advance <= avail)
}
