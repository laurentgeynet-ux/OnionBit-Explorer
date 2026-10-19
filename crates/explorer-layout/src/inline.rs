// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Inline layout (étape 20) : des boîtes en-ligne d'un
//! conteneur aux **line boxes** réelles.
//!
//! Chaîne : texte → runs itemisés (UAX #9 + opportunités
//! UAX #14 de l'étape 18) → fragments mesurés par la fonte
//! (`FontSource` injectée, sinon repli `0.55em` headless) →
//! wrapping greedy sur les opportunités → `text-align`
//! (left/right/center/justify) → pose sur la baseline de la
//! ligne (`vertical-align` = baseline uniquement).
//!
//! `display: inline-block` : boîte atomique — dimensionnée
//! par le layout bloc (`layout_block`) dans la largeur du
//! conteneur, puis coulée en ligne comme un « gros caractère »
//! incassable.
//!
//! Limites documentées : runs RTL posés en ordre logique
//! (réordonnancement visuel UAX #9 §L2 reporté à la display
//! list ; les clusters gardent les offsets octets pour le
//! hit-test futur).

use std::collections::HashMap;

use explorer_css::ComputedStyle;

use crate::block::{css_px, font_size_px, layout_block, measure_fn, BlockCtx};
use crate::boxtree::{LayoutBox, Rect};
use crate::font::{self, FontRequest, ShapedRun};
use crate::text::{self, Direction};

/// Fragment posé dans une ligne : tranche de texte shapé, ou
/// boîte atomique `inline-block` (`atomic`, `text` vide).
#[derive(Debug, Clone)]
pub struct Fragment {
    /// Nœud DOM le plus profond (hit-test futur).
    pub node: Option<explorer_dom::NodeId>,
    /// Indice de l'enfant direct du conteneur possédant ce
    /// fragment (union des rects).
    pub owner: usize,
    /// Tranche de texte (vide si `atomic`).
    pub text: String,
    /// `true` = boîte `inline-block` posée en ligne.
    pub atomic: bool,
    /// Position absolue.
    pub rect: Rect,
    /// Distance du haut du fragment à sa baseline.
    pub baseline: f32,
    /// Glyphes shapés si la fonte était disponible.
    pub shaped: Option<ShapedRun>,
}

/// Une ligne de contenu en-ligne posée.
#[derive(Debug, Clone)]
pub struct LineBox {
    /// Rectangle de la ligne.
    pub rect: Rect,
    /// Fragments dans l'ordre logique.
    pub fragments: Vec<Fragment>,
    /// Baseline de la ligne (depuis `rect.y`).
    pub baseline: f32,
}

/// Item de flux : contenu en-ligne homogène, pré-segmenté.
struct Item {
    /// Indice de l'enfant direct du conteneur (`bx.children`).
    owner: usize,
    /// Nœud le plus profond (boîte texte).
    node: Option<explorer_dom::NodeId>,
    /// Style de l'item.
    style: ComputedStyle,
    /// Pièces entre opportunités — vide si `atomic`.
    pieces: Vec<Piece>,
    /// `true` = `inline-block` (posé hors bande puis coulé).
    atomic: bool,
}

/// Pièce coulable entre deux opportunités de coupure.
struct Piece {
    /// Texte (souvent un mot + espace de queue).
    text: String,
    /// Direction bidi.
    dir: Direction,
    /// Coupure obligatoire après cette pièce.
    hard_after: bool,
}

/// Layout en-ligne du conteneur `bx` dans `[x, y, w]` : pose
/// `bx.lines` + `rect` des enfants (union des fragments) et
/// retourne la hauteur utilisée.
pub(crate) fn layout_inline(
    bx: &mut LayoutBox,
    x: f32,
    y: f32,
    w: f32,
    ctx: &BlockCtx,
    parent_fs: f32,
    abs_cb: Rect,
) -> f32 {
    let st = bx.style.clone();
    let fs = font_size_px(&st, parent_fs, ctx);

    // inline-block : dimensionnés comme blocs AVANT le flux
    // (ils gardent leur contenu, puis coulent en ligne). Une
    // boîte portant `text` (alt d'`<img>` cassée) reste un
    // item texte — sa valeur `display` ne s'applique qu'au
    // contenu qu'elle n'a pas.
    let mut atomics: HashMap<usize, Rect> = HashMap::new();
    for (i, child) in bx.children.iter_mut().enumerate() {
        if child.display() == "inline-block" && child.text.is_none() {
            layout_block(child, 0.0, 0.0, w, ctx, fs, abs_cb);
            atomics.insert(i, child.rect);
        }
    }

    let items = collect_items(bx);
    let mut lines = break_into_lines(&items, &atomics, x, y, w, ctx, fs);
    align_lines(&st, &mut lines, x, w);
    assign_child_rects(bx, &lines);
    let h = lines.iter().map(|l| l.rect.height).sum::<f32>();
    bx.lines = lines;
    bx.rect.height = h;
    h
}

/// Items de flux : chaque enfant → pièces (texte) ou atomique.
fn collect_items(bx: &LayoutBox) -> Vec<Item> {
    let mut items = Vec::new();
    for (owner, child) in bx.children.iter().enumerate() {
        collect_child(child, owner, &mut items);
    }
    items
}

fn collect_child(b: &LayoutBox, owner: usize, items: &mut Vec<Item>) {
    // Texte d'abord : une boîte `Text` (dont le repli `alt`
    // d'un `<img>` cassé) s'itemise même si son style porte
    // `display: inline-block` — l'atomicité ne vaut que pour
    // les conteneurs.
    if let Some(t) = &b.text {
        items.push(text_item(b, owner, t));
        return;
    }
    if b.display() == "inline-block" {
        items.push(Item {
            owner,
            node: b.node,
            style: b.style.clone(),
            pieces: Vec::new(),
            atomic: true,
        });
        return;
    }
    // Boîte inline conteneur : descendre — chaque texte
    // descendant devient un item rattaché au même owner.
    for c in &b.children {
        collect_child(c, owner, items);
    }
}

/// Item texte : runs itemisés → pièces entre opportunités.
fn text_item(b: &LayoutBox, owner: usize, t: &str) -> Item {
    let mut pieces = Vec::new();
    for run in text::itemize(t) {
        let mut prev = 0usize;
        for br in &run.breaks {
            if br.offset > prev && br.offset <= run.text.len() {
                if let Some(p) = run.text.get(prev..br.offset) {
                    pieces.push(Piece {
                        text: p.to_string(),
                        dir: run.direction,
                        // UAX #14 marque la FIN du texte comme
                        // mandatory — une vraie coupure dure n'est
                        // que celle après un caractère de saut de
                        // ligne (\n, NEL, LS/PS…).
                        hard_after: br.mandatory && ends_hard_break(p),
                    });
                }
                prev = br.offset;
            }
        }
        if prev < run.text.len() {
            if let Some(p) = run.text.get(prev..) {
                pieces.push(Piece {
                    text: p.to_string(),
                    dir: run.direction,
                    hard_after: ends_hard_break(p),
                });
            }
        }
    }
    Item {
        owner,
        node: b.node,
        style: b.style.clone(),
        pieces,
        atomic: false,
    }
}

/// La pièce finit-elle par un caractère de saut de ligne dur
/// (BK/Mandatory réel : `\n`, `\r`, `\u{0085}`, `\u{2028/9}`,
/// `\u{000B/C}`) — distingue les vraies lignes forcées de la
/// fin de texte (UAX #14 la marque mandatory aussi).
fn ends_hard_break(piece: &str) -> bool {
    piece.chars().next_back().is_some_and(|c| {
        matches!(
            c,
            '\n' | '\r' | '\u{85}' | '\u{B}' | '\u{C}' | '\u{2028}' | '\u{2029}'
        )
    })
}

/// Fonte (octets) éventuelle pour le shaping des fragments.
fn font_data_of(style: &ComputedStyle, ctx: &BlockCtx) -> Option<Vec<u8>> {
    let src = ctx.fonts?;
    let families: Vec<String> = style.get("font-family").map_or_else(
        || vec![ctx.default_family.to_string()],
        |f| {
            f.split(',')
                .map(|s| s.trim().trim_matches(|c| c == '"' || c == '\'').to_string())
                .collect()
        },
    );
    let weight = style
        .get("font-weight")
        .and_then(|w| match w.trim() {
            "bold" | "bolder" => Some(700u16),
            _ => w.trim().parse().ok(),
        })
        .unwrap_or(400);
    let italic = matches!(style.get("font-style"), Some("italic" | "oblique"));
    src.font_data(&FontRequest {
        families,
        weight,
        italic,
    })
}

/// État courant de la ligne en construction.
struct LineAcc {
    frags: Vec<Fragment>,
    /// `line-height` max rencontrée.
    max_lh: f32,
    /// Baseline max.
    max_base: f32,
}

impl LineAcc {
    fn new() -> Self {
        Self {
            frags: Vec::new(),
            max_lh: 0.0,
            max_base: 0.0,
        }
    }

    fn is_empty(&self) -> bool {
        self.frags.is_empty()
    }

    /// Ferme la ligne à `y` : aligne les fragments sur la
    /// baseline commune, calcule le rect.
    fn close(&mut self, y: &mut f32, lines: &mut Vec<LineBox>) {
        if self.is_empty() {
            return;
        }
        let base = self.max_base;
        let min_x = self
            .frags
            .iter()
            .map(|f| f.rect.x)
            .fold(f32::INFINITY, f32::min);
        let max_x = self
            .frags
            .iter()
            .map(|f| f.rect.x + f.rect.width)
            .fold(0.0_f32, f32::max);
        for f in &mut self.frags {
            f.rect.y = *y + (base - f.baseline);
        }
        let h = self.max_lh.max(base);
        lines.push(LineBox {
            rect: Rect {
                x: min_x,
                y: *y,
                width: max_x - min_x,
                height: h,
            },
            fragments: std::mem::take(&mut self.frags),
            baseline: base,
        });
        self.max_lh = 0.0;
        self.max_base = 0.0;
        *y += h;
    }
}

/// Greedy : pièces → lignes à `w` max, fragments posés en x.
fn break_into_lines(
    items: &[Item],
    atomics: &HashMap<usize, Rect>,
    x: f32,
    y: f32,
    w: f32,
    ctx: &BlockCtx,
    parent_fs: f32,
) -> Vec<LineBox> {
    let mut lines = Vec::new();
    let mut acc = LineAcc::new();
    let mut cursor_x = x;
    let mut cursor_y = y;

    for item in items {
        let fs = font_size_px(&item.style, parent_fs, ctx);
        let lh = line_height_of(&item.style, fs);
        let font_data = font_data_of(&item.style, ctx);
        let measure = measure_fn(&item.style, fs, ctx);

        // Boîte atomique : coule comme une pièce incassable.
        if item.atomic {
            let Some(r) = atomics.get(&item.owner) else {
                continue;
            };
            if cursor_x > x && cursor_x - x + r.width > w && !acc.is_empty() {
                acc.close(&mut cursor_y, &mut lines);
                cursor_x = x;
            }
            acc.frags.push(Fragment {
                node: item.node,
                owner: item.owner,
                text: String::new(),
                atomic: true,
                rect: Rect {
                    x: cursor_x,
                    y: cursor_y,
                    width: r.width,
                    height: r.height,
                },
                baseline: r.height,
                shaped: None,
            });
            acc.max_lh = acc.max_lh.max(r.height);
            acc.max_base = acc.max_base.max(r.height);
            cursor_x += r.width;
            continue;
        }

        for piece in &item.pieces {
            if piece.text.is_empty() {
                continue;
            }
            let pw = measure(&piece.text);
            if cursor_x > x && cursor_x - x + pw > w && !acc.is_empty() {
                acc.close(&mut cursor_y, &mut lines);
                cursor_x = x;
            }
            // Blanc en tête de ligne : supprimé (collapsing).
            if cursor_x <= x && piece.text.trim().is_empty() {
                continue;
            }
            let shaped = font_data
                .as_deref()
                .and_then(|d| font::shape(d, &piece.text, fs, piece.dir == Direction::Rtl));
            let baseline = shaped.as_ref().map_or(fs * 0.8, |s| s.metrics.ascent);
            acc.frags.push(Fragment {
                node: item.node,
                owner: item.owner,
                text: piece.text.clone(),
                atomic: false,
                rect: Rect {
                    x: cursor_x,
                    y: cursor_y,
                    width: pw,
                    height: lh,
                },
                baseline,
                shaped,
            });
            acc.max_lh = acc.max_lh.max(lh);
            acc.max_base = acc.max_base.max(baseline);
            cursor_x += pw;
            if piece.hard_after {
                acc.close(&mut cursor_y, &mut lines);
                cursor_x = x;
            }
        }
    }
    acc.close(&mut cursor_y, &mut lines);
    lines
}

/// `text-align` sur les lignes posées. `justify` étire
/// l'espace inter-fragments (approximation word-spacing,
/// documentée) sauf sur la dernière ligne.
fn align_lines(st: &ComputedStyle, lines: &mut [LineBox], _x: f32, w: f32) {
    let align = st.get_or("text-align", "start");
    let count = lines.len();
    for (i, line) in lines.iter_mut().enumerate() {
        let used = line.fragments.iter().map(|f| f.rect.width).sum::<f32>();
        let slack = (w - used).max(0.0);
        let last = i + 1 == count;
        #[allow(clippy::cast_precision_loss)]
        let (dx, space) = match align {
            "right" | "end" => (slack, 0.0),
            "center" => (slack / 2.0, 0.0),
            "justify" if !last && line.fragments.len() > 1 => {
                (0.0, slack / (line.fragments.len() - 1) as f32)
            }
            _ => (0.0, 0.0),
        };
        if dx <= 0.0 && space <= 0.0 {
            continue;
        }
        let mut shift = dx;
        for f in &mut line.fragments {
            f.rect.x += shift;
            shift += space;
        }
        line.rect.x += dx;
    }
}

/// Rect de chaque enfant direct = union de ses fragments
/// (récursif pour les inline imbriqués).
fn assign_child_rects(bx: &mut LayoutBox, lines: &[LineBox]) {
    for (i, child) in bx.children.iter_mut().enumerate() {
        let mut union: Option<Rect> = None;
        for line in lines {
            for f in &line.fragments {
                if f.owner == i {
                    union = Some(match union {
                        None => f.rect,
                        Some(u) => union_rect(u, f.rect),
                    });
                }
            }
        }
        if let Some(u) = union {
            child.rect = u;
        }
        assign_child_rects(child, lines);
    }
}

/// Union de deux rectangles.
fn union_rect(first: Rect, second: Rect) -> Rect {
    let left = first.x.min(second.x);
    let top = first.y.min(second.y);
    let right = (first.x + first.width).max(second.x + second.width);
    let bottom = (first.y + first.height).max(second.y + second.height);
    Rect {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    }
}

/// `line-height` en px.
fn line_height_of(style: &ComputedStyle, fs: f32) -> f32 {
    let Some(raw) = style.get("line-height") else {
        return fs * 1.2;
    };
    let raw = raw.trim();
    if raw == "normal" {
        return fs * 1.2;
    }
    if let Ok(n) = raw.parse::<f32>() {
        return (n * fs).max(1.0);
    }
    css_px(style, "line-height", fs, fs, &BlockCtx::default()).unwrap_or(fs * 1.2)
}
