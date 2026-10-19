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

use crate::block::{css_px, font_size_px, layout_block, measure_fn, side_px, BlockCtx, FloatCtx};
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
    /// Avance du whitespace FINAL du fragment — retirée de
    /// `rect.width` à la fermeture quand le fragment est le
    /// dernier de la ligne (§16.6.1 : les espaces de fin de
    /// ligne ne comptent pas dans la largeur ni `text-align`).
    pub trail_w: f32,
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
/// retourne la hauteur utilisée. `fl` est le registre des
/// flottants du BFC — chaque ligne se pose dans la bande
/// libre à son propre `y` (§9.5).
#[allow(clippy::too_many_arguments)]
pub(crate) fn layout_inline(
    bx: &mut LayoutBox,
    x: f32,
    y: f32,
    w: f32,
    ctx: &BlockCtx,
    parent_fs: f32,
    abs_cb: Rect,
    fl: &FloatCtx,
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
            layout_block(
                child,
                0.0,
                0.0,
                w,
                ctx,
                fs,
                abs_cb,
                &mut FloatCtx::default(),
            );
            atomics.insert(i, child.rect);
        }
    }

    let items = collect_items(bx);
    let mut lines = break_into_lines(&items, &atomics, x, y, w, ctx, fs, fl);
    align_lines(&st, &mut lines, x, w, fl);
    assign_child_rects(bx, &lines);
    // Span réelle `y → bas de la dernière ligne` : la descente
    // sous les flottants a pu sauter de l'espace vertical.
    let h = lines.last().map_or(0.0, |l| l.rect.y + l.rect.height - y);
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
    // §16.6.1 white-space processing : `normal`/`nowrap` effondrent
    // toute suite de blancs en une espace ; `pre-line` effondre les
    // espaces/tabs mais garde `\n` (saut forcé) ; `pre`/`pre-wrap`
    // préservent tout. Sans ça, `\n`/`\t` bruts sont shapés →
    // glyphe `.notdef` visible (ex. carré Ahem des reftests WPT).
    let collapsed;
    let src = match b.style.get("white-space") {
        Some(v) if v.trim() == "pre" || v.trim() == "pre-wrap" => t,
        Some(v) if v.trim() == "pre-line" => {
            collapsed = collapse_spaces(t);
            &collapsed
        }
        _ => {
            collapsed = collapse_ws(t);
            &collapsed
        }
    };
    let mut pieces = Vec::new();
    for run in text::itemize(src) {
        let mut prev = 0usize;
        for br in &run.breaks {
            if br.offset > prev && br.offset <= run.text.len() {
                if let Some(p) = run.text.get(prev..br.offset) {
                    // UAX #14 marque la FIN du texte comme
                    // mandatory — une vraie coupure dure n'est
                    // que celle après un caractère de saut de
                    // ligne (\n, NEL, LS/PS…). Ce caractère est
                    // retiré du texte shapé (sinon `.notdef`).
                    pieces.push(Piece {
                        text: strip_break_chars(p).to_string(),
                        dir: run.direction,
                        hard_after: br.mandatory && ends_hard_break(p),
                    });
                }
                prev = br.offset;
            }
        }
        if prev < run.text.len() {
            if let Some(p) = run.text.get(prev..) {
                pieces.push(Piece {
                    text: strip_break_chars(p).to_string(),
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

/// §16.6.1 : espaces CSS (` ` `\t` `\n` `\r` `\x0B` `\x0C`)
/// — une suite quelconque devient une seule ` `.
fn collapse_ws(t: &str) -> String {
    let mut out = String::with_capacity(t.len());
    let mut in_ws = false;
    for c in t.chars() {
        if matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{B}' | '\u{C}') {
            if !in_ws {
                out.push(' ');
                in_ws = true;
            }
        } else {
            out.push(c);
            in_ws = false;
        }
    }
    out
}

/// Variante `pre-line` : seuls espaces et tabulations sont
/// effondrés ; les sauts de ligne restent des coupures dures.
fn collapse_spaces(t: &str) -> String {
    let mut out = String::with_capacity(t.len());
    let mut in_ws = false;
    for c in t.chars() {
        if matches!(c, ' ' | '\t') {
            if !in_ws {
                out.push(' ');
                in_ws = true;
            }
        } else {
            out.push(c);
            in_ws = false;
        }
    }
    out
}

/// Retire les caractères de saut dur en fin de pièce — ils
/// portent la coupure (`hard_after`) mais ne se rendent pas.
fn strip_break_chars(p: &str) -> &str {
    p.trim_end_matches(|c| {
        matches!(
            c,
            '\n' | '\r' | '\u{85}' | '\u{B}' | '\u{C}' | '\u{2028}' | '\u{2029}'
        )
    })
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
        // §16.6.1 : les espaces en FIN de ligne sont retirées
        // (elles ne rendent rien et ne comptent pas dans la
        // largeur — `text-align` mesure sans elles).
        while self
            .frags
            .last()
            .is_some_and(|f| !f.atomic && f.text.trim().is_empty())
        {
            self.frags.pop();
        }
        // L'espace finale intra-fragment (pièce fusionnée
        // `"mot "`) est exclue aussi : la ligne se mesure sur
        // le contenu visible.
        if let Some(f) = self.frags.last_mut() {
            f.rect.width -= f.trail_w;
            f.trail_w = 0.0;
        }
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

/// Greedy : pièces → lignes posées dans la bande libre de
/// leur `y` (`fl` — registre des flottants du BFC ; §9.5).
/// Une ligne démarre dans `fl.band(y)` ; un fragment seul
/// plus large que la bande fait descendre la ligne sous le
/// prochain flottant — à bande vide (`w ≤ 0`) la ligne
/// descend jusqu'à réapparition d'espace.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn break_into_lines(
    items: &[Item],
    atomics: &HashMap<usize, Rect>,
    x: f32,
    y: f32,
    w: f32,
    ctx: &BlockCtx,
    parent_fs: f32,
    fl: &FloatCtx,
) -> Vec<LineBox> {
    /// Requêtes de bande bornées (contenu hostile : les
    /// flottants sont finis, chaque descente en franchit un).
    const MAX_BAND_STEPS: u32 = 128;

    let mut lines = Vec::new();
    let mut acc = LineAcc::new();
    let (mut line_x, mut line_w) = fl.band(y, x, w);
    let mut cursor_x = line_x;
    let mut cursor_y = y;

    // Descend la ligne tant que la bande est vide ou trop
    // étroite pour un fragment **seul** de `pw` — borne dure.
    macro_rules! descend_band {
        ($pw:expr) => {{
            let pw = $pw;
            let mut steps = 0u32;
            while (line_w <= 0.01 || pw > line_w) && steps < MAX_BAND_STEPS {
                steps += 1;
                match fl.next_band_change(cursor_y) {
                    Some(ny) if ny > cursor_y => {
                        cursor_y = ny;
                        (line_x, line_w) = fl.band(ny, x, w);
                        cursor_x = line_x;
                    }
                    _ => break,
                }
            }
        }};
    }
    // Ferme la ligne courante et repart sur la bande du `y`
    // suivant (les flottants peuvent avoir commencé/fini).
    macro_rules! new_line {
        () => {{
            acc.close(&mut cursor_y, &mut lines);
            (line_x, line_w) = fl.band(cursor_y, x, w);
            cursor_x = line_x;
        }};
    }

    for item in items {
        let fs = font_size_px(&item.style, parent_fs, ctx);
        let lh = line_height_of(&item.style, fs);
        let font_data = font_data_of(&item.style, ctx);
        let measure = measure_fn(&item.style, fs, ctx);

        // Boîte atomique : coule comme une pièce incassable.
        // Ses marges horizontales font partie de sa boîte
        // externe (§8.3) — y compris en tête de ligne après
        // un wrap : `margin-left` s'y applique aussi.
        if item.atomic {
            let Some(r) = atomics.get(&item.owner) else {
                continue;
            };
            let ml = side_px(&item.style, "margin-left", w, fs, ctx, 0.0);
            let mr = side_px(&item.style, "margin-right", w, fs, ctx, 0.0);
            // Marges verticales : appliquées aussi sur les
            // atomes (inline-block, §10.8 — c'est leur margin
            // box qui compte dans la hauteur de ligne).
            let mt = side_px(&item.style, "margin-top", w, fs, ctx, 0.0);
            let mb = side_px(&item.style, "margin-bottom", w, fs, ctx, 0.0);
            let outer = ml + r.width + mr;
            let outer_h = mt + r.height + mb;
            // §9.5 : la boîte externe d'un atome (inline-block =
            // BFC) ne doit chevaucher aucun flottant sur toute
            // sa hauteur — bande effective intersectée sur
            // `[y, y+h]` ; si elle n'y tient pas, elle descend
            // sous le premier flottant finissant dans l'étendue.
            let (mut ebx, mut ebw) = fl.band_over(cursor_y, outer_h, x, w);
            let fits = cursor_x - line_x + outer <= line_w
                && cursor_x >= ebx - 0.01
                && cursor_x + outer <= ebx + ebw + 0.01;
            if !fits && !acc.is_empty() {
                new_line!();
                (ebx, ebw) = fl.band_over(cursor_y, outer_h, x, w);
            }
            if acc.is_empty() {
                let mut steps = 0u32;
                while outer > ebw + 0.01 && steps < MAX_BAND_STEPS {
                    steps += 1;
                    match fl.first_bottom_within(cursor_y, outer_h) {
                        Some(ny) if ny > cursor_y => {
                            cursor_y = ny;
                            (ebx, ebw) = fl.band_over(ny, outer_h, x, w);
                        }
                        _ => break,
                    }
                }
                line_x = ebx;
                line_w = ebw;
                cursor_x = line_x;
            }
            let px = cursor_x.max(ebx);
            acc.frags.push(Fragment {
                node: item.node,
                owner: item.owner,
                text: String::new(),
                atomic: true,
                rect: Rect {
                    x: px + ml,
                    y: cursor_y + mt,
                    width: r.width,
                    height: r.height,
                },
                baseline: r.height,
                shaped: None,
                trail_w: 0.0,
            });
            // Margin box dans la ligne (§10.8) : `mt` pousse
            // la boîte vers le bas via la baseline commune
            // (`rect.y = y + base - h`), `mb` allonge la ligne.
            acc.max_lh = acc.max_lh.max(mt + r.height + mb);
            acc.max_base = acc.max_base.max(mt + r.height);
            cursor_x = px + outer;
            continue;
        }

        for piece in &item.pieces {
            if piece.text.is_empty() {
                continue;
            }
            // §16.6.1 : les espaces en TÊTE de ligne sont
            // supprimées — y compris en tête d'une pièce
            // non-blanche (le collapse fusionne `"  mot"`).
            let line_start = cursor_x <= line_x;
            let text = if line_start {
                piece.text.trim_start()
            } else {
                piece.text.as_str()
            };
            if text.is_empty() {
                continue;
            }
            let pw = measure(text);
            if cursor_x > line_x && cursor_x - line_x + pw > line_w && !acc.is_empty() {
                new_line!();
            }
            if acc.is_empty() {
                descend_band!(pw);
            }
            let shaped = font_data
                .as_deref()
                .and_then(|d| font::shape(d, text, fs, piece.dir == Direction::Rtl));
            let baseline = shaped.as_ref().map_or(fs * 0.8, |s| s.metrics.ascent);
            let trail_w = if text.ends_with(char::is_whitespace) {
                pw - measure(text.trim_end())
            } else {
                0.0
            };
            acc.frags.push(Fragment {
                node: item.node,
                owner: item.owner,
                text: text.to_string(),
                atomic: false,
                rect: Rect {
                    x: cursor_x,
                    y: cursor_y,
                    width: pw,
                    height: lh,
                },
                baseline,
                shaped,
                trail_w,
            });
            acc.max_lh = acc.max_lh.max(lh);
            acc.max_base = acc.max_base.max(baseline);
            cursor_x += pw;
            if piece.hard_after {
                new_line!();
            }
        }
    }
    acc.close(&mut cursor_y, &mut lines);
    lines
}

/// `text-align` sur les lignes posées. `justify` étire
/// l'espace inter-fragments (approximation word-spacing,
/// documentée) sauf sur la dernière ligne. Le slack est
/// mesuré dans la **bande** de la ligne (flottants, §9.5).
fn align_lines(st: &ComputedStyle, lines: &mut [LineBox], x: f32, w: f32, fl: &FloatCtx) {
    let align = st.get_or("text-align", "start");
    let count = lines.len();
    for (i, line) in lines.iter_mut().enumerate() {
        let used = line.fragments.iter().map(|f| f.rect.width).sum::<f32>();
        let (_bx, bw) = fl.band(line.rect.y + 0.01, x, w);
        let slack = (bw - used).max(0.0);
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
