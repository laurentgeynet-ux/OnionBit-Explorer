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
//! Limites documentées : `unicode-bidi` non résolu (R-176)
//! — seul le réordonnancement visuel par niveaux UAX #9 §L2
//! est livré (R-171) ; les clusters gardent les offsets
//! octets pour le hit-test futur.

use std::collections::HashMap;

use explorer_css::{value::Color, ComputedStyle};

use crate::block::{
    css_px, font_size_px, layout_block, measure_fn, raw_length_px, side_px, BlockCtx, FloatCtx,
};
use crate::boxtree::{BoxKind, LayoutBox, Rect};
use crate::font::{self, ShapedRun};
use crate::text::{self, Direction};

/// Ligne `text-decoration` sous le texte (R-153).
pub const DECO_UNDERLINE: u8 = 0b001;
/// Ligne `text-decoration` au-dessus du texte (R-153).
pub const DECO_OVERLINE: u8 = 0b010;
/// Ligne `text-decoration` barrant le texte (R-153).
pub const DECO_LINE_THROUGH: u8 = 0b100;
/// Style de trait `text-decoration-style` ([`TextDeco::style`],
/// R-178) — `solid` par défaut.
pub const DECO_SOLID: u8 = 0;
/// `dotted` — points en pas 2·épaisseur.
pub const DECO_DOTTED: u8 = 1;
/// `dashed` — tirets 3·th on / 2·th off.
pub const DECO_DASHED: u8 = 2;
/// `double` — deux traits fins (tiers haut/bas du slot).
pub const DECO_DOUBLE: u8 = 3;
/// `wavy` — pseudo-vague (segments alternés au rendu,
/// approximation honnête sans chemin courbe).
pub const DECO_WAVY: u8 = 4;

/// `text-decoration` **résolue** portée par un [`Fragment`]
/// (R-153) : l'entrée appartient à l'élément **déclarant** —
/// sa couleur vient de son style à lui, pas de celui du
/// fragment traversé (§16.3.1 : la déco enjambe les inlines
/// descendants, en couleur de déclarant même si les
/// descendants ont un `color` différent).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextDeco {
    /// Lignes actives — masque [`DECO_UNDERLINE`] /
    /// [`DECO_OVERLINE`] / [`DECO_LINE_THROUGH`].
    pub lines: u8,
    /// `color` de l'élément déclarant ; `None` → l'encre
    /// du fragment au paint (repli currentcolor).
    pub color: Option<Color>,
    /// Style de trait `text-decoration-style` (R-178) —
    /// [`DECO_SOLID`] par défaut.
    pub style: u8,
    /// `text-decoration-thickness` résolue en px (`0` =
    /// auto → repli ~1/16em au paint).
    pub thick_px: f32,
    /// `text-underline-offset` résolu en px (`None` = auto
    /// → repli ~10% em au paint).
    pub offset_px: Option<f32>,
}

/// `text-decoration` de `st` → entrée résolue, `None` si
/// absente/`none`. Mots inconnus tolérés (`blink`, styles et
/// couleurs du shorthand Level 4 — ignorés, jamais d'échec).
fn text_deco(st: &ComputedStyle, fs: f32, ctx: &BlockCtx) -> Option<TextDeco> {
    let v = st
        .get("text-decoration")
        .or_else(|| st.get("text-decoration-line"))?;
    let mut lines = 0u8;
    let mut style = DECO_SOLID;
    let style_of = |t: &str| match t {
        "dotted" => DECO_DOTTED,
        "dashed" => DECO_DASHED,
        "double" => DECO_DOUBLE,
        "wavy" => DECO_WAVY,
        _ => DECO_SOLID,
    };
    for t in v.split_ascii_whitespace() {
        if t.eq_ignore_ascii_case("underline") {
            lines |= DECO_UNDERLINE;
        } else if t.eq_ignore_ascii_case("overline") {
            lines |= DECO_OVERLINE;
        } else if t.eq_ignore_ascii_case("line-through") {
            lines |= DECO_LINE_THROUGH;
        } else {
            let st2 = style_of(&t.to_ascii_lowercase());
            if st2 != DECO_SOLID {
                style = st2;
            }
        }
    }
    // Longhands l'emportent sur le shorthand (cascade faite).
    if let Some(s2) = st.get("text-decoration-style") {
        style = style_of(&s2.trim().to_ascii_lowercase());
    }
    if lines == 0 {
        return None;
    }
    // `text-decoration-thickness` / `text-underline-offset`
    // (R-178) résolus contre la fonte du DÉCLARANT — `%` =
    // pourcent de sa font-size ; `auto`/`from-font` → repli.
    let own_fs = font_size_px(st, fs, ctx);
    let thick_px = st
        .get("text-decoration-thickness")
        .and_then(|t| raw_length_px(t, own_fs, own_fs, ctx))
        .unwrap_or(0.0);
    let offset_px = st
        .get("text-underline-offset")
        .and_then(|t| raw_length_px(t, own_fs, own_fs, ctx));
    Some(TextDeco {
        lines,
        // `text-decoration-color` (css-text-decor-3 §2.4 —
        // R-265) : déclarée → son effet (currentcolor → le
        // `color` du déclarant) ; absente → couleur du
        // déclarant (= initiale currentcolor — inchangé).
        color: st
            .color("text-decoration-color")
            .or_else(|| st.color("color")),
        style,
        thick_px,
        offset_px,
    })
}

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
    /// `true` = boîte atomique posée en ligne.
    pub atomic: bool,
    /// Série de l'atome (numéro de pré-ordre `size_atomics`)
    /// — permet de retrouver la **boîte** nichée dont ce
    /// fragment est la projection (`object` sous bloc
    /// anonyme, R-13c) ; `None` pour le texte et les
    /// sentinelles de coupure dure.
    pub aid: Option<usize>,
    /// Position absolue.
    pub rect: Rect,
    /// Distance du haut du fragment à sa baseline.
    pub baseline: f32,
    /// Glyphes shapés si la fonte était disponible.
    pub shaped: Option<std::sync::Arc<ShapedRun>>,
    /// Avance du whitespace FINAL du fragment — retirée de
    /// `rect.width` à la fermeture quand le fragment est le
    /// dernier de la ligne (§16.6.1 : les espaces de fin de
    /// ligne ne comptent pas dans la largeur ni `text-align`).
    pub trail_w: f32,
    /// Marges horizontales de l'atome (`0` pour le texte) —
    /// elles font partie de la **margin box** qui mesure la
    /// largeur de ligne pour `text-align` (§16.2 + §8.3 :
    /// les marges d'un inline-block comptent dans la ligne).
    pub marg_l: f32,
    /// Voir [`Fragment::marg_l`].
    pub marg_r: f32,
    /// Marges verticales de l'atome (`0` pour le texte) —
    /// la margin box sert d'ancre à `vertical-align`
    /// top/bottom/middle (§10.8).
    pub marg_t: f32,
    /// Voir [`Fragment::marg_t`].
    pub marg_b: f32,
    /// Style de l'item propriétaire — l'élément inline le
    /// plus proche du texte (héritage fait). Le rendu lit
    /// `color`/`font-*` ICI, pas sur la boîte des lignes :
    /// `a{color}`/`span{color}` doit colorer ses runs (R-18).
    pub style: ComputedStyle,
    /// `text-decoration` résolues des ancêtres déclarants
    /// (R-153, ordre ancêtre → descendant) — vide sans déco,
    /// vide aussi sur les atomes : la propagation saute les
    /// remplacés (§16.3.1).
    pub deco: Vec<TextDeco>,
    /// Niveau bidi UAX #9 du fragment (pair = LTR) — atomes
    /// et marqueurs portent le niveau de base du paragraphe
    /// (approximation neutre, R-171).
    pub level: u8,
}

/// Une ligne de contenu en-ligne posée.
#[derive(Debug, Clone)]
pub struct LineBox {
    /// Rectangle de la ligne.
    pub rect: Rect,
    /// Fragments dans l'ordre **visuel** (réordonnancement
    /// bidi UAX #9 §L2 appliqué à la fermeture — R-171).
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
    /// Pièces entre opportunités — vide si atomique.
    pieces: Vec<Piece>,
    /// `Some(id)` = atome posé en ligne (inline-block ou
    /// remplacé `intrinsic` — `<object>`/`<img>`/…, R-13c) ;
    /// `id` est le numéro de pré-ordre partagé avec
    /// [`size_atomics`] (table `atomics`).
    aid: Option<usize>,
    /// `Some(…)` = borne d'un élément inline non-remplacé
    /// (ouverture ou fermeture) — porte les marges
    /// horizontales de l'élément (§8.3/§9.4.2).
    marker: Option<Marker>,
    /// Décos résolues au moment de l'itemisation (R-153) —
    /// snapshot de la pile des ancêtres déclarants ; vide
    /// pour atomes et marqueurs.
    deco: Vec<TextDeco>,
}

/// Borne d'élément en-ligne : `Open` applique `margin-left`,
/// `Close` `margin-right` (§9.4.2 — les marges verticales
/// d'un non-remplacé sont ignorées).
#[derive(Clone, Copy)]
enum Marker {
    Open,
    Close,
}

/// Pièce coulable entre deux opportunités de coupure.
struct Piece {
    /// Texte (souvent un mot + espace de queue).
    text: String,
    /// Direction bidi.
    dir: Direction,
    /// Niveau bidi UAX #9 (pair = LTR) — sert au
    /// réordonnancement visuel des fragments (R-171).
    level: u8,
    /// Coupure obligatoire après cette pièce.
    hard_after: bool,
    /// `true` = pièce tabulation (texte = `"\t"`) — son
    /// avance n'est pas shapée mais résolue au coulage
    /// d'après les taquets `tab-size` (§8.1).
    tab: bool,
}

/// Niveau d'imbrication du paragraphe depuis `direction`
/// (spec : `rtl` → 1, `ltr` → 0) — R-171. Insensible à la
/// casse : la valeur est stockée telle que déclarée.
fn para_level(st: &ComputedStyle) -> u8 {
    u8::from(
        st.get("direction")
            .is_some_and(|d| d.trim().eq_ignore_ascii_case("rtl")),
    )
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
    cb_h: Option<f32>,
) -> f32 {
    let st = bx.style.clone();
    let fs = font_size_px(&st, parent_fs, ctx);
    // R-171 — niveau d'imbrication du paragraphe : `direction`
    // du conteneur (hint `dir` → `direction`, cascade) sert
    // de base aux itemisations et de scel aux fragments non
    // textuels ; la retraite visuelle se fait à `close`.
    let para_lv = para_level(&st);

    // Atomes (inline-block, remplacés `intrinsic` — object,
    // img, svg, canvas) : dimensionnés comme blocs AVANT le
    // flux, à n'importe quelle profondeur d'inline (un
    // `<object>` vit souvent sous un bloc anonyme) ; la série
    // `aid` est l'ordre pré-ordre partagé avec
    // `collect_child`.
    let mut atomics: HashMap<usize, Rect> = HashMap::new();
    size_atomics(bx, w, cb_h, ctx, fs, abs_cb, &mut atomics);

    let items = collect_items(bx, para_lv, fs, ctx);
    // Strut du conteneur (§10.8.1) : `line-height` de la
    // boîte — plancher de chaque ligne (`.chin` Acid2 :
    // `line-height:1em` tient la rangée à 12 px même si le
    // texte dedans fait 4 px).
    let strut_lh = line_height_of(&st, fs, ctx);
    // `text-indent` (R-70) : décalage de la première ligne
    // formatée — négatif = sortie d'écran, la technique
    // image-remplacement (`text-indent:-9999px` + sprite en
    // `background-image`, portail Wikipedia `.sprite`).
    // Mots-clés `each-line`/`hanging` (css-text-4) :
    // `each-line` = indent sur TOUTES les lignes (pas la
    // première seule), `hanging` = l'indent s'applique aux
    // lignes SAUF la première — combinés = tout sauf la
    // première est décalée de l'indent. `side_px` échouerait
    // sur `"20px hanging"` — la longueur est extraite des
    // tokens non mots-clés.
    let ti_raw = st.get("text-indent").map_or("", |v| v);
    let each_line = ti_raw.contains("each-line");
    let hanging = ti_raw.contains("hanging");
    let indent = if each_line || hanging {
        let len = ti_raw
            .split_whitespace()
            .filter(|t| !matches!(*t, "each-line" | "hanging"))
            .collect::<Vec<_>>()
            .join(" ");
        raw_length_px(&len, w, fs, ctx).unwrap_or(0.0)
    } else {
        side_px(&st, "text-indent", w, fs, ctx, 0.0)
    };
    let mut lines = break_into_lines(
        &items,
        &atomics,
        x,
        y,
        w,
        ctx,
        fs,
        fl,
        strut_lh,
        // `each-line` → indent toutes lignes ; `hanging` →
        // indent sauf la première (css-text-4) — le delta
        // première/suivantes est porté à la cassure et à
        // l'alignement.
        if each_line { indent } else { 0.0 },
        if hanging || each_line { indent } else { 0.0 },
        para_lv,
    );
    align_lines(&st, &mut lines, x, w, fl, indent, hanging);
    apply_text_overflow(&st, &mut lines, x + w, ctx, fs);
    apply_line_clamp(&st, &mut lines, x + w, ctx, fs);
    apply_line_clamp(&st, &mut lines, x + w, ctx, fs);
    // Atomes d'abord : leur sous-arbre a été posé en (0,0)
    // par `size_atomics` — translation au rect du fragment
    // (R-23 : sans elle, les enfants d'un atome — valeur d'un
    // contrôle — peignent à l'origine). L'union par `owner`
    // ensuite : elle saute les atomes (leur rect vient déjà
    // du fragment via `aid`).
    let mut aid = 0usize;
    assign_atomic_rects(bx, &lines, &mut aid, w, fs, ctx);
    assign_child_rects(bx, &lines);
    // Span réelle `y → bas de la dernière ligne` : la descente
    // sous les flottants a pu sauter de l'espace vertical.
    let h = lines.last().map_or(0.0, |l| l.rect.y + l.rect.height - y);
    bx.lines = lines;
    // R-177 — `::first-line`/`::first-letter` : overlay §7.1
    // (typographique) sur la première ligne / le premier
    // fragment texte — props hors liste ignorées, les
    // effets de mise en page (line-height, transform) restent
    // documentés non rejoués.
    if let Some(fl) = bx.first_line.clone() {
        if let Some(line) = bx.lines.first_mut() {
            for f in &mut line.fragments {
                overlay_pseudo(&mut f.style, &fl);
            }
        }
    }
    if let Some(fl) = bx.first_letter.clone() {
        if let Some(f) = bx.lines.first_mut().and_then(|l| {
            l.fragments
                .iter_mut()
                .find(|f| !f.atomic && !f.text.is_empty())
        }) {
            overlay_pseudo(&mut f.style, &fl);
        }
    }
    bx.rect.height = h;
    h
}

/// Sous-ensemble typographique admis par `::first-line`/
/// `::first-letter` (css-pseudo §7.1 + §4.1 — couleurs,
/// fontes, espacements, décorations ; les props de boîte et
/// de mise en page sont ignorées).
const PSEUDO_OVERLAY_PROPS: &[&str] = &[
    "color",
    "background-color",
    "font-family",
    "font-size",
    "font-style",
    "font-variant",
    "font-weight",
    "letter-spacing",
    "line-height",
    "text-transform",
    "text-decoration-line",
    "text-decoration-color",
    "text-decoration-style",
    "text-shadow",
    "word-spacing",
];

/// `ps` (style pseudo `::first-line`/`::first-letter`) par
///-dessus `style` de fragment — les déclarations du pseudo
/// gagnent, borné à [`PSEUDO_OVERLAY_PROPS`].
fn overlay_pseudo(style: &mut ComputedStyle, ps: &ComputedStyle) {
    for (k, v) in ps.iter() {
        if PSEUDO_OVERLAY_PROPS.contains(&k) {
            style.set(k, v);
        }
    }
}

/// §text-overflow (`clip` par défaut) : quand le contenu
/// en-ligne déborde le bord droit du bloc conteneur ET que
/// celui-ci clippe (`overflow[-x]` ≠ `visible`), la valeur
/// `ellipsis` tronque les fragments au point de coupure et
/// insère un fragment « … » mesuré dans la fonte du texte
/// coupé. MVP LTR : seul le débordement à droite est traité
/// (`ellipsis` RTL / `'<string>'` hors périmètre). Vol 21/22
/// : « Topics » discourse tronqué net, sans suspension.
fn apply_text_overflow(
    st: &ComputedStyle,
    lines: &mut [LineBox],
    right: f32,
    ctx: &BlockCtx,
    fs: f32,
) {
    if st.get("text-overflow") != Some("ellipsis") {
        return;
    }
    let clipped = matches!(
        st.get("overflow-x").or_else(|| st.get("overflow")),
        Some("hidden" | "clip" | "scroll" | "auto")
    ) || matches!(
        st.get("overflow").or_else(|| st.get("overflow-x")),
        Some("hidden" | "clip" | "scroll" | "auto")
    );
    if !clipped {
        return;
    }
    for line in lines.iter_mut() {
        if line.rect.x + line.rect.width <= right + 0.01 {
            continue;
        }
        ellipsize_line(line, right, ctx, fs);
    }
}
/// `line-clamp` / `-webkit-line-clamp` (entier > 0 —
/// l'entrée `max-lines` d'Overflow-4) : ne garde que les `n`
/// premières lignes de la boîte et suspend la dernière par
/// « … ». Lignes excédentaires détruites — équivalent
/// `overflow:hidden` + `continue:discard`. MVP LTR :
/// appliqué à tout bloc conteneur d'inline sans exiger
/// `display:-webkit-box` ni `-webkit-box-orient` —
/// approximation terrain acceptée (les pages cibles la
/// posent sur des `<p>`/cartes ordinaires). `block-ellipsis`
/// (chaîne autre que « … ») hors périmètre.
fn apply_line_clamp(
    st: &ComputedStyle,
    lines: &mut Vec<LineBox>,
    right: f32,
    ctx: &BlockCtx,
    fs: f32,
) {
    let Some(n) = st
        .get("line-clamp")
        .or_else(|| st.get("-webkit-line-clamp"))
        .and_then(|v| v.trim().parse::<usize>().ok())
    else {
        return;
    };
    if n == 0 || lines.len() <= n {
        return;
    }
    lines.truncate(n);
    if let Some(last) = lines.last_mut() {
        ellipsize_line(last, right, ctx, fs);
    }
}

/// Tronque `line` à `right` et insère « … » au point de
/// coupe (partagé `text-overflow` / `line-clamp`) :
/// l'ellipsis prend la fonte du dernier fragment texte
/// (spec : mise en forme de l'élément qui déborde), les
/// fragments entièrement au-delà sont supprimés (le hit-test
/// et les unions de rect restent propres), le fragment
/// chevauchant la coupe est rogné à la mesure réelle de sa
/// fonte — retrait caractère par caractère, `pop` est sûr
/// Unicode.
fn ellipsize_line(line: &mut LineBox, right: f32, ctx: &BlockCtx, fs: f32) {
    let Some((est, enode, eowner, ebase, edeco, elevel)) =
        line.fragments.iter().rev().find(|f| !f.atomic).map(|f| {
            (
                f.style.clone(),
                f.node,
                f.owner,
                f.baseline,
                f.deco.clone(),
                f.level,
            )
        })
    else {
        return;
    };
    let efs = font_size_px(&est, fs, ctx);
    let ell_w = crate::block::measure_fn(&est, efs, ctx)("…");
    let cut = right - ell_w;
    line.fragments.retain(|f| f.rect.x < cut);
    if let Some(f) = line.fragments.last_mut() {
        if f.rect.x + f.rect.width > cut {
            let avail = (cut - f.rect.x).max(0.0);
            let ffs = font_size_px(&f.style, fs, ctx);
            let fm = crate::block::measure_fn(&f.style, ffs, ctx);
            while !f.text.is_empty() && fm(&f.text) > avail {
                f.text.pop();
            }
            f.rect.width = fm(&f.text).min(avail).max(0.0);
            f.trail_w = 0.0;
        }
    }
    line.fragments.push(Fragment {
        node: enode,
        owner: eowner,
        text: "…".to_string(),
        atomic: false,
        aid: None,
        rect: Rect {
            x: cut,
            y: line.rect.y + (line.baseline - ebase),
            width: ell_w,
            height: line.rect.height,
        },
        baseline: ebase,
        shaped: None,
        trail_w: 0.0,
        marg_l: 0.0,
        marg_r: 0.0,
        marg_t: 0.0,
        marg_b: 0.0,
        style: est,
        deco: edeco,
        level: elevel,
    });
    line.rect.width = (cut + ell_w - line.rect.x).max(0.0);
}

/// `true` si la boîte coule en ligne comme un atome :
/// conteneur de niveau inline établissant un contexte de
/// formatage propre (`inline-block`/`inline-flex`/
/// `inline-grid`/`inline-table` — CSS §9.2.4, même liste que
/// `atomic_inline` de `wrap_children`), ou élément
/// **remplacé** (`intrinsic` — `<object>` résolu, `<img>`,
/// `<svg>`, `<canvas>`) sans texte de repli. Un repli `alt`
/// porte `text` → il reste un item texte, jamais atomique.
fn is_atomic(b: &LayoutBox) -> bool {
    b.text.is_none()
        && (matches!(
            b.display(),
            "inline-block" | "inline-flex" | "inline-grid" | "inline-table"
        ) || b.intrinsic.is_some())
}

/// Fragment d'annotation ruby (css-ruby-1) : le style du
/// fragment est celui de l'élément inline porteur — `rt`
/// direct ou `rtc`. Limite MVP : un `rt` dont le texte est
/// enveloppé dans un `display:inline` intermédiaire n'est
/// pas reconnu (le style du fragment est celui du span).
fn is_ruby_text_frag(f: &Fragment) -> bool {
    matches!(
        f.style.get("display").map(str::trim),
        Some("ruby-text" | "ruby-text-container")
    )
}

/// Sous-rect d'une tranche d'octets `start..end` de
/// `frag.text` (R-35 — surlignage find au glyphe près).
///
/// Horizontalement, l'union des cellules des glyphes dont
/// le `cluster` (offset octet dans le run — les glyphes
/// sont en ordre visuel) tombe dans la tranche : exacte
/// pour LTR comme RTL, ligatures incluses (un glyphe dont
/// le cluster entre dans la tranche est couvert entier).
/// Verticalement, la hauteur de ligne du fragment — un
/// surlignage couvre la ligne, pas l'encre seule.
///
/// `None` : tranche vide ou hors bornes, fragment atomique.
/// Sans run shapé (fonte absente) — ou si aucun cluster ne
/// tombe dedans (tranche hors texte rendu, ex. blancs
/// collapsés) : repli **proportionnel** aux octets sur la
/// largeur du fragment (avances inconnues — approximation
/// documentée).
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn fragment_sub_rect(frag: &Fragment, start: usize, end: usize) -> Option<Rect> {
    if frag.atomic || start >= end || end > frag.text.len() {
        return None;
    }
    let band = |x: f32, w: f32| Rect {
        x,
        y: frag.rect.y,
        width: w,
        height: frag.rect.height,
    };
    if let Some(run) = &frag.shaped {
        let mut pen = frag.rect.x;
        let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
        for g in &run.glyphs {
            let next = pen + g.advance;
            if (g.cluster as usize) >= start && (g.cluster as usize) < end {
                lo = lo.min(pen.min(next));
                hi = hi.max(pen.max(next));
            }
            pen = next;
        }
        if hi > lo {
            return Some(band(lo, hi - lo));
        }
    }
    let len = frag.text.len() as f32;
    if len <= 0.0 {
        return None;
    }
    let x = frag.rect.x + frag.rect.width * (start as f32 / len);
    let w = frag.rect.width * ((end - start) as f32 / len);
    Some(band(x, w))
}

/// Dimensionne récursivement chaque boîte atomique du
/// sous-arbre inline (bloc bloc, puis coulée comme pièce
/// incassable). La clé `aid` = ordre pré-ordre : le même
/// parcours que [`collect_child`], qui s'arrête aussi à la
/// frontière des atomes.
#[allow(clippy::too_many_arguments)]
fn size_atomics(
    b: &mut LayoutBox,
    w: f32,
    cb_h: Option<f32>,
    ctx: &BlockCtx,
    fs: f32,
    abs_cb: Rect,
    atomics: &mut HashMap<usize, Rect>,
) {
    let mut aid = 0usize;
    size_atomics_walk(b, w, cb_h, ctx, fs, abs_cb, atomics, &mut aid);
}

#[allow(clippy::too_many_arguments)]
fn size_atomics_walk(
    b: &mut LayoutBox,
    w: f32,
    cb_h: Option<f32>,
    ctx: &BlockCtx,
    fs: f32,
    abs_cb: Rect,
    atomics: &mut HashMap<usize, Rect>,
    aid: &mut usize,
) {
    for child in &mut b.children {
        // Hors-flux : `absolute`/`fixed` est posé plus tard par
        // `resolve_absolutes` — jamais atomisé ici (la série
        // `aid` doit rester synchrone avec `collect_child`,
        // qui les saute aussi — R-22). Le champ `position` n'est
        // renseigné qu'au layout : on lit le style calculé.
        if child.out_of_flow() {
            continue;
        }
        if is_atomic(child) {
            // §10.3.9 shrink-to-fit : un atome `inline-block`
            // à `width:auto` occupe `clamp(dispo, min-content,
            // max-content)` — jamais toute la ligne si son
            // contenu est étroit (R-22 : `ul.tags` de lobste.rs
            // s'étalait sur la largeur conteneur → une ligne
            // entière par story, interlignes ~180 px). Les
            // remplacés (`intrinsic`) se dimensionnent déjà.
            let own = crate::intrinsic::own_sides(child, fs, ctx);
            let cb_w = if child.intrinsic.is_some() {
                // Remplacé : son cb est sa **taille utilisée**,
                // pas la ligne — §10.3.9. `width`/`height`
                // résolus d'abord (`height` → ratio, §10.6.2),
                // bornés par `min|max-width` contre la ligne
                // (B-38 : `img{height:28px}` intrinsèque
                // 5839×745 recevait la ligne entière → item
                // flex irrétrécissable → nav à x=5879).
                let h_css = if child.style.get("height").is_some_and(|v| v.contains('%')) {
                    cb_h.and_then(|h| css_px(&child.style, "height", h, fs, ctx))
                } else {
                    css_px(&child.style, "height", 0.0, fs, ctx)
                };
                let own_w = css_px(&child.style, "width", w, fs, ctx).or_else(|| {
                    child.intrinsic.and_then(|(iw, ih)| {
                        h_css.map_or(Some(iw), |h| (ih > 0.0).then(|| h * iw / ih))
                    })
                });
                own_w
                    .map_or(w, |mut ow| {
                        if let Some(mx) = css_px(&child.style, "max-width", w, fs, ctx) {
                            ow = ow.min(mx);
                        }
                        if let Some(mn) = css_px(&child.style, "min-width", w, fs, ctx) {
                            ow = ow.max(mn);
                        }
                        ow
                    })
                    .max(0.0)
            } else if css_px(&child.style, "width", w, fs, ctx).is_none() {
                (w - own)
                    .max(0.0)
                    .min(crate::intrinsic::max_content_px(child, fs, ctx))
                    .max(crate::intrinsic::min_content_px(child, fs, ctx))
                    + own
            } else {
                w
            };
            layout_block(
                child,
                0.0,
                0.0,
                cb_w,
                cb_h,
                ctx,
                fs,
                abs_cb,
                &mut FloatCtx::default(),
            );
            atomics.insert(*aid, child.rect);
            *aid += 1;
        } else {
            // Arête ∝ profondeur de l'arbre (R-24).
            explorer_common::deep(|| {
                size_atomics_walk(child, w, cb_h, ctx, fs, abs_cb, atomics, aid);
            });
        }
    }
}

/// Items de flux : chaque enfant → pièces (texte) ou atomique.
/// `para_lv` = niveau d'imbrication du paragraphe (R-171).
fn collect_items(bx: &LayoutBox, para_lv: u8, fs: f32, ctx: &BlockCtx) -> Vec<Item> {
    let mut items = Vec::new();
    let mut aid = 0usize;
    // §16.3.1 / R-153 : une déco posée sur le CONTENEUR
    // enjambe son propre contenu en-ligne — la pile des
    // déclarants démarre avec la sienne (`<div style=
    // "text-decoration:underline">`). Les descendants la
    // complètent à l'entrée de chaque élément inline.
    let mut decos: Vec<TextDeco> = text_deco(&bx.style, fs, ctx).into_iter().collect();
    // css-ruby-1 : séquence des conteneurs `rtc` — chaque
    // `<rtc>` ouvre un niveau d'annotation distinct (bandes
    // empilées, §3.2). `0` = `rt` nu hors `rtc`.
    let mut rtc_seq = 0u32;
    for (owner, child) in bx.children.iter().enumerate() {
        collect_child(
            child,
            owner,
            &mut items,
            &mut aid,
            &mut decos,
            para_lv,
            fs,
            ctx,
            &mut rtc_seq,
            0,
        );
    }
    items
}

#[allow(clippy::too_many_arguments)]
fn collect_child(
    b: &LayoutBox,
    owner: usize,
    items: &mut Vec<Item>,
    aid: &mut usize,
    decos: &mut Vec<TextDeco>,
    para_lv: u8,
    fs: f32,
    ctx: &BlockCtx,
    rtc_seq: &mut u32,
    ruby_tier: u32,
) {
    // Hors-flux : `absolute`/`fixed` est posé par
    // `resolve_absolutes` — son contenu n'entre jamais dans
    // les items de la ligne, sinon son texte serait posé une
    // fois ici et une fois par l'absolu, et son encombrement
    // gonflerait la hauteur de ligne (R-22 : `#flag_dropdown`
    // de lobste.rs, absolu sous un `span` inline → ligne
    // de ~93 px).
    if b.out_of_flow() {
        return;
    }
    // Texte d'abord : une boîte `Text` (dont le repli `alt`
    // d'un `<img>` cassé) s'itemise même si son style porte
    // `display: inline-block` — l'atomicité ne vaut que pour
    // les conteneurs.
    if let Some(t) = &b.text {
        items.push(text_item(b, owner, t, decos, para_lv, ruby_tier));
        return;
    }
    if is_atomic(b) {
        let id = *aid;
        *aid += 1;
        items.push(Item {
            owner,
            node: b.node,
            style: b.style.clone(),
            pieces: Vec::new(),
            aid: Some(id),
            marker: None,
            // §16.3.1 : la déco ne propage pas dans un
            // remplacé/atomique — `deco` reste vide même si
            // des ancêtres déclarent (`<a><img></a>`).
            deco: Vec::new(),
        });
        return;
    }
    // Boîte inline conteneur : descendre — chaque texte
    // descendant devient un item rattaché au même owner.
    // Les marges horizontales d'un **élément** inline non-
    // remplacé s'appliquent à ses extrémités (§9.4.2) : on
    // les matérialise par des items marqueurs. Les boîtes
    // `AnonymousBlock` clonent le style du conteneur — leur
    // « marge » copiée n'est pas la leur : transparentes.
    let margins = b.kind == BoxKind::Inline
        && [
            "margin-left",
            "margin-right",
            "padding-left",
            "padding-right",
            "border-left-width",
            "border-right-width",
        ]
        .iter()
        .any(|p| b.style.get(p).is_some());
    // Élément en-ligne déclarant (§16.3.1 / R-153) : sa déco
    // enjambe le contenu descendant — empilée pour les items
    // produits par la descente, dépilée à la fermeture.
    // `AnonymousBlock` n'est pas un déclarant : son style
    // cloné porte la déco du conteneur, déjà seedée par
    // `collect_items` — la repousser la doublerait.
    let pushed = (b.kind == BoxKind::Inline)
        .then(|| text_deco(&b.style, fs, ctx))
        .flatten();
    if let Some(d) = pushed {
        decos.push(d);
    }
    if margins {
        items.push(Item {
            owner,
            node: b.node,
            style: b.style.clone(),
            pieces: Vec::new(),
            aid: None,
            marker: Some(Marker::Open),
            deco: Vec::new(),
        });
    }
    // Niveau d'annotation ruby (css-ruby-1 §3.2) : un
    // conteneur `<ruby>` relance la numérotation des `rtc` ;
    // chaque `ruby-text-container` descendu tague ses
    // annotations du niveau suivant (bandes empilées au
    // coulage — `lift_ruby`).
    let mut tier = ruby_tier;
    match b.style.get("display").map_or("", |d| d.trim()) {
        "ruby" => {
            *rtc_seq = 0;
            tier = 0;
        }
        "ruby-text-container" => {
            *rtc_seq += 1;
            tier = *rtc_seq;
        }
        _ => {}
    }
    for c in &b.children {
        explorer_common::deep(|| {
            collect_child(c, owner, items, aid, decos, para_lv, fs, ctx, rtc_seq, tier);
        });
    }
    if pushed.is_some() {
        decos.pop();
    }
    if margins {
        items.push(Item {
            owner,
            node: b.node,
            style: b.style.clone(),
            pieces: Vec::new(),
            aid: None,
            marker: Some(Marker::Close),
            deco: Vec::new(),
        });
    }
}

/// Item texte : runs itemisés → pièces entre opportunités.
/// `decos` = pile des décos déclarées par les ancêtres
/// en-ligne (R-153) — copiée en snapshot sur l'item.
/// `para_lv` = niveau d'imbrication du paragraphe : la
/// `direction` du bloc conteneur sert de base UAX #9 à
/// toutes les itemisations de la ligne (R-171).
#[allow(clippy::too_many_lines)]
fn text_item(
    b: &LayoutBox,
    owner: usize,
    t: &str,
    decos: &[TextDeco],
    para_lv: u8,
    ruby_tier: u32,
) -> Item {
    // §16.6.1 white-space processing : `normal`/`nowrap` effondrent
    // toute suite de blancs en une espace ; `pre-line` effondre les
    // espaces/tabs mais garde `\n` (saut forcé) ; `pre`/`pre-wrap`
    // préservent tout. Sans ça, `\n`/`\t` bruts sont shapés →
    // glyphe `.notdef` visible (ex. carré Ahem des reftests WPT).
    // R-264 : le mode effectif vient de `ws_mode` (css-text-4
    // §7.1 — `white-space-collapse`/`text-wrap-mode` sont les
    // longhands réels posés par l'expansion du shorthand ;
    // `preserve-spaces`/`discard`/`break-spaces` s'ajoutent).
    let collapsed;
    let (ws_collapse, _wrap) = ws_mode(&b.style);
    let src = match ws_collapse {
        WsCollapse::Preserve | WsCollapse::BreakSpaces => t,
        WsCollapse::PreserveBreaks => {
            collapsed = collapse_spaces(t);
            &collapsed
        }
        WsCollapse::PreserveSpaces => {
            collapsed = preserve_spaces(t);
            &collapsed
        }
        WsCollapse::Discard => {
            collapsed = discard_ws(t);
            &collapsed
        }
        WsCollapse::Collapse => {
            collapsed = collapse_ws(t);
            &collapsed
        }
    };
    // R-162 — text-transform : applique a la chaine avant
    // itemisation/segmentation UAX #14 (§16.5) — les
    // lettres changent, la ponctuation et les espaces non ;
    // herite via le style calcule.
    let transformed;
    let src = match b.style.get("text-transform").map(str::trim) {
        Some("uppercase") => {
            transformed = src.to_uppercase();
            &transformed
        }
        Some("lowercase") => {
            transformed = src.to_lowercase();
            &transformed
        }
        Some("capitalize") => {
            transformed = capitalize_words(src);
            &transformed
        }
        // `font-variant-caps` (css-fonts-4 §6.6) — capitales
        // **synthétisées** par uppercase : les features OT
        // `smcp`/`c2sc`/`pcap`/`c2pc` sont aussi demandées au
        // shaping (`FontRequest.caps`) ; sur une fonte qui
        // les porte, l'uppercase ci-dessous rend la version
        // pleine taille (approximation documentée — mieux
        // qu'un no-op silencieux). `titling-caps` → `tit`.
        _ if b
            .style
            .get("font-variant-caps")
            .or_else(|| b.style.get("font-variant"))
            .is_some_and(|v| {
                v.split_whitespace().any(|kw| {
                    matches!(
                        kw,
                        "small-caps"
                            | "all-small-caps"
                            | "petite-caps"
                            | "all-petite-caps"
                            | "unicase"
                            | "titling-caps"
                    )
                })
            }) =>
        {
            transformed = src.to_uppercase();
            &transformed
        }
        _ => src,
    };
    let mut pieces = Vec::new();
    let para = if para_lv == 0 {
        unicode_bidi::Level::ltr()
    } else {
        unicode_bidi::Level::rtl()
    };
    // R-176 — `unicode-bidi` : `isolate`/`embed`/
    // `isolate-override`/`bidi-override`/`plaintext` ouvrent
    // un niveau propre (direction de l'élément — `dir` déjà
    // résolu en `direction` par les hints) ; `plaintext`
    // auto-détecte au premier fort (`None`). Les
    // `*-override` aplatissent les niveaux internes — UAX #9
    // §3.3.2 : l'override interdit tout réordonnancement
    // interne (`<bdo dir=rtl>` s'affiche en miroir).
    let ub = b.style.get("unicode-bidi").map(str::trim);
    let overrides = matches!(ub, Some("bidi-override" | "isolate-override"));
    let (para_arg, flat) = match ub {
        Some("plaintext") => (None, 0),
        Some("isolate" | "isolate-override" | "embed" | "bidi-override") => {
            // Embeddings : prochain niveau de la parité
            // directionnelle > `para` (rtl → impair, ltr → pair).
            let n = if para_level(&b.style) == 1 {
                (para.number() + 1) | 1
            } else {
                (para.number() + 2) & !1
            };
            (unicode_bidi::Level::new(n).ok(), n)
        }
        _ => (Some(para), 0),
    };
    for run in text::itemize(src, para_arg) {
        let level = if overrides { flat } else { run.level };
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
                        level,
                        hard_after: br.mandatory && ends_hard_break(p),
                        tab: false,
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
                    level,
                    hard_after: ends_hard_break(p),
                    tab: false,
                });
            }
        }
    }
    // §8.1 — `tab-size` : en `pre`/`pre-wrap` les `\t` ont
    // survécu à l'effondrement des blancs ; UAX #14 ne les
    // isole pas forcément dans leurs propres pièces.
    if pieces.iter().any(|p| p.text.contains('\t')) {
        pieces = split_tabs(pieces);
    }
    // `break-spaces` (css-text-4 §4.1.3) : l'opportunité de
    // coupure existe après CHAQUE espace préservée — chaque
    // ` ` devient sa propre pièce pour que le boucleur puisse
    // casser en plein milieu d'une suite d'espaces. La
    // première espace d'une suite « pend » (la coupure est
    // APRES elle — voir le point de décision plus bas).
    if ws_collapse == WsCollapse::BreakSpaces {
        pieces = pieces.into_iter().flat_map(explode_break_spaces).collect();
    }
    let mut style = b.style.clone();
    if ruby_tier > 0 {
        // Marqueur interne (jamais déclaré en cascade) :
        // niveau `rtc` lu par `lift_ruby` pour empiler les
        // bandes d'annotation (css-ruby-1 §3.2).
        style.set("x-ruby-tier", &ruby_tier.to_string());
    }
    Item {
        owner,
        node: b.node,
        style,
        pieces,
        aid: None,
        marker: None,
        deco: decos.to_vec(),
    }
}

/// §8.1 — chaque tabulation devient une pièce dédiée
/// (`tab`) : sans ça elle partirait au shaping → glyphe
/// `.notdef` visible. `hard_after` reste sur la dernière
/// sous-pièce de la pièce d'origine.
fn split_tabs(pieces: Vec<Piece>) -> Vec<Piece> {
    let mut split = Vec::with_capacity(pieces.len() + 4);
    for p in pieces {
        let mut rest = p.text.as_str();
        while let Some(i) = rest.find('\t') {
            if i > 0 {
                split.push(Piece {
                    text: rest[..i].to_string(),
                    dir: p.dir,
                    level: p.level,
                    hard_after: false,
                    tab: false,
                });
            }
            split.push(Piece {
                text: "\t".to_string(),
                dir: p.dir,
                level: p.level,
                hard_after: false,
                tab: true,
            });
            rest = &rest[i + 1..];
        }
        if !rest.is_empty() {
            split.push(Piece {
                text: rest.to_string(),
                dir: p.dir,
                level: p.level,
                hard_after: p.hard_after,
                tab: false,
            });
        } else if p.hard_after {
            if let Some(last) = split.last_mut() {
                last.hard_after = true;
            }
        }
    }
    split
}

/// `break-spaces` (css-text-4 §4.1.3) — chaque espace
/// préservée devient sa propre pièce : la coupure souple est
/// permise APRÈS chacune, y compris à l'intérieur d'une
/// suite. Les tabulations (pièces `tab` dédiées) et les
/// caractères non-espaces restent agglomérés.
fn explode_break_spaces(p: Piece) -> Vec<Piece> {
    if !p.text.contains(' ') {
        return vec![p];
    }
    let mut out = Vec::new();
    let mut cur = String::with_capacity(p.text.len());
    for c in p.text.chars() {
        if c == ' ' && !cur.is_empty() {
            out.push(Piece {
                text: std::mem::take(&mut cur),
                dir: p.dir,
                level: p.level,
                hard_after: false,
                tab: false,
            });
        }
        cur.push(c);
        if c == ' ' {
            out.push(Piece {
                text: std::mem::take(&mut cur),
                dir: p.dir,
                level: p.level,
                hard_after: false,
                tab: false,
            });
        }
    }
    if !cur.is_empty() {
        out.push(Piece {
            text: cur,
            dir: p.dir,
            level: p.level,
            hard_after: p.hard_after,
            tab: false,
        });
    } else if p.hard_after {
        if let Some(last) = out.last_mut() {
            last.hard_after = true;
        }
    }
    out
}

/// §8.1 — `tab-size` : `<number>` = multiple de l'avance
/// d'une espace, `<length>` absolue ; initiale `8`. Les
/// valeurs négatives sont hors grammaire — défense : bornées
/// à ≥ 0 (un taquet nul rend des avances nulles).
pub(crate) fn tab_size_px(st: &ComputedStyle, space_w: f32, fs: f32, ctx: &BlockCtx) -> f32 {
    match st.get("tab-size").map(str::trim) {
        Some(v) => v
            .parse::<f32>()
            .ok()
            .map(|n| n.max(0.0) * space_w)
            .or_else(|| raw_length_px(v, fs, fs, ctx).map(|l| l.max(0.0)))
            .unwrap_or(8.0 * space_w),
        None => 8.0 * space_w,
    }
}

/// §16.6.1 : espaces CSS (` ` `\t` `\n` `\r` `\x0B` `\x0C`)
/// — une suite quelconque devient une seule ` `.
pub(crate) fn collapse_ws(t: &str) -> String {
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

/// css-text-4 §4.1 — mode d'effondrement des blancs, tiré de
/// `white-space-collapse` (le shorthand `white-space` le pose
/// via son expansion ; les hints `nowrap`/styles inline ne
/// portant que `white-space` passent par le repli ci-dessous).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WsCollapse {
    /// `collapse` — toute suite de blancs → une espace.
    Collapse,
    /// `preserve` — tout verbatim (sauts = coupures dures,
    /// tabulations = taquets `tab-size`).
    Preserve,
    /// `preserve-breaks` — sauts conservés, espaces et
    /// tabulations effondrés (`pre-line`).
    PreserveBreaks,
    /// `preserve-spaces` — espaces verbatim ; tabulations et
    /// sauts convertis en espaces (plus de coupure dure).
    PreserveSpaces,
    /// `discard` — tout blanc supprimé.
    Discard,
    /// `break-spaces` — verbatim + opportunité de coupure
    /// après CHAQUE espace ; les blancs de fin de ligne
    /// comptent dans la mesure (ils ne pendent pas, §4.1.3).
    BreakSpaces,
}

/// Mode `white-space` effectif (css-text-4 §7.1) —
/// `(collapse, wrap autorisé)`. Priorité aux longhands
/// `white-space-collapse`/`text-wrap-mode` (l'expansion du
/// shorthand les pose, un longhand déclaré ensuite gagne par
/// ordre de cascade) ; `white-space` sert de repli pour les
/// chemins qui le posent seul (hint `<td nowrap>`, valeurs
/// inline héritées avant l'expansion).
pub(crate) fn ws_mode(st: &ComputedStyle) -> (WsCollapse, bool) {
    let mut collapse = match st.get("white-space").map(str::trim) {
        Some("pre" | "pre-wrap") => WsCollapse::Preserve,
        Some("pre-line") => WsCollapse::PreserveBreaks,
        Some("break-spaces") => WsCollapse::BreakSpaces,
        _ => WsCollapse::Collapse,
    };
    let mut wrap = !matches!(st.get("white-space").map(str::trim), Some("nowrap" | "pre"));
    if let Some(v) = st.get("white-space-collapse").map(str::trim) {
        collapse = match v {
            "preserve" => WsCollapse::Preserve,
            "preserve-breaks" => WsCollapse::PreserveBreaks,
            "preserve-spaces" => WsCollapse::PreserveSpaces,
            "discard" => WsCollapse::Discard,
            "break-spaces" => WsCollapse::BreakSpaces,
            _ => WsCollapse::Collapse,
        };
    }
    if let Some(m) = st.get("text-wrap-mode").map(str::trim) {
        wrap = m != "nowrap";
    }
    (collapse, wrap)
}

/// `white-space-collapse:preserve-spaces` (css-text-4 §4.1) :
/// espaces conservées verbatim ; tabulations, retours et
/// sauts de ligne → une espace chacun (`\r\n` = un seul saut
/// de segment → une seule espace). Plus aucune coupure dure
/// n'existe dans le résultat — le retour à la ligne n'est
/// plus que souple, aux opportunités UAX #14 ordinaires.
fn preserve_spaces(t: &str) -> String {
    let mut out = String::with_capacity(t.len());
    let mut prev_cr = false;
    for c in t.chars() {
        match c {
            '\n' if prev_cr => {}
            _ if text::is_css_ws(c) => out.push(' '),
            _ => out.push(c),
        }
        prev_cr = c == '\r';
    }
    out
}

/// `white-space-collapse:discard` (css-text-4 §4.1) : tout
/// blanc CSS est supprimé — le texte se soude (`a b` → `ab`),
/// il ne reste plus d'opportunité de coupure sur blanc.
fn discard_ws(t: &str) -> String {
    t.chars().filter(|c| !text::is_css_ws(*c)).collect()
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
            '\n' | '\r' | '\u{85}' | '\u{B}' | '\u{C}' | '\u{2028}' | '\u{2029}' | '\u{200B}'
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

/// État courant de la ligne en construction.
struct LineAcc {
    frags: Vec<Fragment>,
    /// `line-height` max rencontrée.
    max_lh: f32,
    /// Baseline max.
    max_base: f32,
    /// Strut du conteneur (§10.8.1) : chaque ligne démarre
    /// avec une boîte invisible de la `line-height` du bloc —
    /// elle plancherde la hauteur même sans fragment élevé.
    strut_lh: f32,
    /// Baseline du strut (même modèle que les fragments).
    strut_base: f32,
}

impl LineAcc {
    fn new(strut_lh: f32, strut_base: f32) -> Self {
        Self {
            frags: Vec::new(),
            max_lh: 0.0,
            max_base: 0.0,
            strut_lh,
            strut_base,
        }
    }

    fn is_empty(&self) -> bool {
        self.frags.is_empty()
    }

    /// Réordonnancement visuel bidi (UAX #9 §L2 — R-171) :
    /// les fragments sont accumulés en ordre **logique** ;
    /// chaque séquence de niveau ≥ seuil est retournée (du
    /// niveau max vers le plus bas niveau impair), puis les
    /// margin boxes sont re-tuilées dans l'ordre visuel —
    /// à gauche pour un paragraphe LTR, ancrées au bord
    /// droit de la bande pour RTL (`text-align` consomme le
    /// slack restant ensuite, comme en LTR).
    fn reorder_bidi(&mut self, base_rtl: bool, band_right: f32) {
        if self.frags.is_empty() {
            return;
        }
        let base = u8::from(base_rtl);
        // Court-circuit : paragraphe LTR sans niveau impair —
        // ordre logique = visuel, positions déjà tuilées. En
        // RTL le retournement et l'ancre droite s'imposent
        // même si tous les fragments sont niveau 1.
        if !base_rtl && self.frags.iter().all(|f| f.level == base) {
            return;
        }
        let levels: Vec<unicode_bidi::Level> = self
            .frags
            .iter()
            .map(|f| unicode_bidi::Level::new(f.level).unwrap_or(unicode_bidi::LTR_LEVEL))
            .collect();
        // `map[i]` = indice logique du fragment posé au
        // slot visuel `i` (UAX #9 §L2).
        let map = unicode_bidi::BidiInfo::reorder_visual(&levels);
        let mut frags = Vec::with_capacity(self.frags.len());
        for &li in &map {
            frags.push(self.frags[li].clone());
        }
        let total: f32 = frags
            .iter()
            .map(|f| f.marg_l + f.rect.width + f.marg_r)
            .sum();
        let mut cx = if base_rtl {
            band_right - total
        } else {
            frags
                .iter()
                .map(|f| f.rect.x - f.marg_l)
                .fold(f32::INFINITY, f32::min)
        };
        for f in &mut frags {
            f.rect.x = cx + f.marg_l;
            cx += f.marg_l + f.rect.width + f.marg_r;
        }
        self.frags = frags;
    }

    /// §16.6.1 : retire les fragments-blancs de fin de ligne
    /// (invisibles, hors mesure `text-align`) et défalque
    /// l'espace finale intra-fragment (`"mot "` fusionné) —
    /// appelé par `close` hors mode `break-spaces` (R-264 :
    /// les blancs préservés y sont du contenu mesuré).
    fn drop_trailing_ws(&mut self) {
        while self
            .frags
            .last()
            .is_some_and(|f| !f.atomic && text::trim_css_ws(&f.text).is_empty())
        {
            self.frags.pop();
        }
        if let Some(f) = self.frags.last_mut() {
            f.rect.width -= f.trail_w;
            f.trail_w = 0.0;
        }
    }

    /// Ferme la ligne à `y` : aligne les fragments sur la
    /// baseline commune, calcule le rect. `base_rtl`/
    /// `band_right` alimentent le réordonnancement bidi —
    /// le bord droit de la bande où la ligne a été coulée.
    fn close(
        &mut self,
        y: &mut f32,
        lines: &mut Vec<LineBox>,
        ctx: &BlockCtx,
        base_rtl: bool,
        band_right: f32,
    ) {
        // §16.6.1 : les espaces en FIN de ligne sont retirées
        // (elles ne rendent rien et ne comptent pas dans la
        // largeur — `text-align` mesure sans elles).
        // R-264 — `break-spaces` (css-text-4 §4.1.3) : les
        // blancs préservés sont du CONTENU — ils gardent leur
        // avance et leur décoration, ils ne pendent pas.
        let keep_trail = self
            .frags
            .last()
            .is_some_and(|f| ws_mode(&f.style).0 == WsCollapse::BreakSpaces && !f.atomic);
        if !keep_trail {
            self.drop_trailing_ws();
        }
        if self.is_empty() {
            return;
        }
        self.reorder_bidi(base_rtl, band_right);
        // R-182 — quirks « images en cellule » (HTML
        // Standard §quirks) : une ligne sans aucun texte
        // (tout en atomes) n'impose pas le strut — le
        // remplacé descend au bas de sa cellule.
        if ctx.quirks.get() && !self.frags.is_empty() && self.frags.iter().all(|f| f.atomic) {
            self.strut_base = 0.0;
            self.strut_lh = 0.0;
        }
        // Le strut pose la baseline ET la hauteur plancher
        // de la ligne (§10.8.1) — un fragment bas s'y aligne
        // comme sur un caractère de la fonte du conteneur.
        let base = self.max_base.max(self.strut_base);
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
        let mut h = self.max_lh.max(self.strut_lh).max(base);
        for f in &mut self.frags {
            // `vertical-align` des atomes (inline-block,
            // remplacés) : `top`/`bottom` ancrent la **margin
            // box** aux bords de la ligne, `middle` la centre
            // (approximation — la spec centre sur x-height/2
            // au-dessus de la baseline). Texte et autres clés
            // : baseline.
            // `alignment-baseline` (css-inline-3 — réexport SVG)
            // quand `vertical-align` est absent : repli sur les
            // clés communes. `alphabetic`→baseline, `ideographic`
            // →text-bottom, `hanging`→top, `central`/`mathematical`
            // →middle. `baseline-shift` = alias de `vertical-align`
            // (`sub`/`super`/longueur) — même traitement.
            let va_owned;
            let va = if let Some(v) = f.style.get("vertical-align") {
                v
            } else if let Some(bs) = f.style.get("baseline-shift") {
                bs
            } else if let Some(ab) = f.style.get("alignment-baseline") {
                va_owned = match ab.trim() {
                    "ideographic" | "text-after-edge" | "text-bottom" => "text-bottom",
                    "hanging" | "text-before-edge" | "text-top" => "top",
                    "central" | "mathematical" | "middle" => "middle",
                    _ => "baseline",
                };
                va_owned
            } else if let Some(db) = f.style.get("dominant-baseline") {
                // `dominant-baseline` (SVG, css-inline-3) —
                // même table de repli qu'`alignment-baseline`
                // quand rien d'autre n'aligne le fragment.
                va_owned = match db.trim() {
                    "ideographic" | "text-after-edge" | "text-bottom" => "text-bottom",
                    "hanging" | "text-before-edge" | "text-top" => "top",
                    "central" | "mathematical" | "middle" => "middle",
                    _ => "baseline",
                };
                va_owned
            } else if let Some(fp) = f.style.get("font-variant-position") {
                // css-fonts-4 §6.8 : `sub`/`super` déplacent
                // la baseline (approximation des décalages
                // `subs`/`sups` — les métriques de la fonte
                // ne sont pas lues ici).
                va_owned = match fp.trim() {
                    "sub" => "sub",
                    "super" => "super",
                    _ => "baseline",
                };
                va_owned
            } else {
                "baseline"
            };
            // R-161 — décalages de baseline sur tout fragment
            // : / en fractions de font-size
            // (métriques fonte hors périmètre), longueur en
            // px négative = monte,  de la hauteur de strut
            // (approxime la line-height, §10.8).
            let fs_px = f
                .style
                .get("font-size")
                .and_then(|v| v.strip_suffix("px"))
                .and_then(|t| t.trim().parse::<f32>().ok())
                .unwrap_or(self.strut_lh);
            let dy = match va {
                "sub" => fs_px * 0.2,
                "super" => -(fs_px * 0.33),
                _ => raw_length_px(va, self.strut_lh, fs_px, ctx).map_or(0.0, |l| -l),
            };
            f.rect.y = match va {
                // Synonymes SVG (css-inline-3 — `alphabetic`/
                // `ideographic`/`hanging`/`central`/
                // `mathematical`/*-edge tolérés aussi sur
                // `vertical-align` — repli gracieux).
                "top" | "text-top" | "hanging" | "text-before-edge" => *y + f.marg_t,
                "bottom" | "text-bottom" | "ideographic" | "text-after-edge" => {
                    *y + h - (f.rect.height + f.marg_b)
                }
                "middle" | "center" | "central" | "mathematical" => {
                    *y + (h - f.rect.height - f.marg_t - f.marg_b) / 2.0 + f.marg_t
                }
                _ => *y + (base - f.baseline) + dy,
            };
        }
        let mut base = base;
        self.lift_ruby(&mut h, &mut base);
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

    /// css-ruby-1 : l'annotation (`display:ruby-text*`) sort
    /// du flux vertical de la base — posée dans une bande
    /// au-dessus (`ruby-position:over`, défaut) ou en dessous
    /// (`under`), la ligne grandit d'autant. Chaque run
    /// d'annotation est apparié au segment de base `ruby*`
    /// qui le précède et **centré** dessus (`ruby-align:
    /// center`, défaut) ; `inter-character` cale le run
    /// après sa base, lignes de base alignées (usage texte
    /// vertical — dégradation documentée en horizontal).
    /// `ruby-align` est lu sur le run (repli : segment) —
    /// `start`/`center`/`space-around`≈centré/`space-between`
    /// = étiré aux bords du segment ; l'overhang est borné
    /// à l'étendue `ruby*` non-rt de la ligne (conteneur —
    /// approximation : la boîte `ruby` elle-même n'est pas
    /// retrouvée). Résidus : `rtc` multi-annotations,
    /// positions mixtes sur une même ligne → repli `over`.
    #[allow(clippy::cast_precision_loss)]
    fn lift_ruby(&mut self, h: &mut f32, base: &mut f32) {
        if !self.frags.iter().any(is_ruby_text_frag) {
            return;
        }
        let pos = self
            .frags
            .iter()
            .find(|f| is_ruby_text_frag(f))
            .and_then(|f| f.style.get("ruby-position"))
            .unwrap_or("over")
            .trim()
            .to_string();
        // Hauteur de bande par niveau d'annotation (css-ruby-1
        // §3.2) : tier 0 = `rt` nu hors `rtc`, chaque `rtc`
        // = un niveau empilé — le plus proche de la base en
        // premier (over : le plus haut niveau est le plus
        // éloigné de la base ; under : miroir).
        let mut tier_h: std::collections::BTreeMap<u32, f32> = std::collections::BTreeMap::new();
        for f in self.frags.iter().filter(|f| is_ruby_text_frag(f)) {
            let t = ruby_tier(f);
            let h = tier_h.entry(t).or_default();
            *h = h.max(f.rect.height);
        }
        let total_h: f32 = tier_h.values().sum();
        let rt_h = total_h;
        let base_top = self
            .frags
            .iter()
            .filter(|f| !is_ruby_text_frag(f))
            .map(|f| f.rect.y)
            .fold(f32::INFINITY, f32::min);
        let base_bot = self
            .frags
            .iter()
            .filter(|f| !is_ruby_text_frag(f))
            .map(|f| f.rect.y + f.rect.height)
            .fold(0.0_f32, f32::max);
        if rt_h <= 0.0 || !base_top.is_finite() {
            return;
        }
        // Étendue du conteneur ruby de la ligne — borne de
        // l'overhang (l'annotation ne doit pas dépasser le
        // conteneur qui la porte, approximation : la boîte
        // `ruby` n'est pas retrouvée, on borne à l'étendue
        // `ruby*` non-rt de la ligne).
        let (cont_l, cont_r) = self
            .frags
            .iter()
            .filter(|f| {
                !is_ruby_text_frag(f)
                    && f.style
                        .get("display")
                        .is_some_and(|d| d.trim().starts_with("ruby"))
            })
            .fold((f32::INFINITY, 0.0_f32), |(l, r), f| {
                (l.min(f.rect.x), r.max(f.rect.x + f.rect.width))
            });
        // Runs d'annotation consécutifs (ordre des fragments)
        // → indice de début/fin + segment de base apparié
        // (contigu `ruby*` non-rt en arrière ; repli = toute
        // la base de la ligne).
        // Bande verticale de chaque niveau — `over` : les
        // niveaux s'empilent au-dessus de `base_top` (le plus
        // grand tier le plus haut) ; `under` : miroir sous
        // `base_bot` (le plus petit tier le plus proche).
        let band_top = |t: u32, over: bool| -> f32 {
            if over {
                base_top
                    + tier_h
                        .iter()
                        .filter(|(j, _)| **j > t)
                        .map(|(_, h)| *h)
                        .sum::<f32>()
            } else {
                base_bot
                    + tier_h
                        .iter()
                        .filter(|(j, _)| **j < t)
                        .map(|(_, h)| *h)
                        .sum::<f32>()
            }
        };
        let runs = rt_runs(&self.frags);
        for (lo, hi) in &runs {
            let t = ruby_tier(&self.frags[*lo]);
            self.place_rt_run(
                *lo,
                *hi,
                &pos,
                tier_h[&t],
                band_top(t, true),
                band_top(t, false),
                (cont_l, cont_r),
            );
        }
        if pos.as_str() == "inter-character" {
            return;
        }
        // Bande d'annotation : la base descend (`over`) ou la
        // ligne grandit vers le bas (`under`).
        let over = pos.as_str() != "under";
        for f in &mut self.frags {
            if !is_ruby_text_frag(f) && over {
                f.rect.y += rt_h;
            }
        }
        *h += rt_h;
        if over {
            *base += rt_h;
        }
    }

    /// Pose un run d'annotation `frags[lo..=hi]` : `dx`
    /// horizontal selon `ruby-align` (`start`/centré/
    /// `space-between` étiré aux bords du segment) et bande
    /// verticale selon `pos` — `band_h`/`top_over`/`top_under`
    /// sont déjà relatifs au niveau `rtc` du run ; overhang
    /// borné à l'étendue `cont` du conteneur (hors
    /// `inter-character`, sorti par construction ; run plus
    /// large que le conteneur → calé sur son bord de départ).
    #[allow(clippy::too_many_arguments, clippy::cast_precision_loss)]
    fn place_rt_run(
        &mut self,
        lo: usize,
        hi: usize,
        pos: &str,
        band_h: f32,
        top_over: f32,
        top_under: f32,
        cont: (f32, f32),
    ) {
        let (seg_l, seg_r) = ruby_base_span(&self.frags, lo);
        let run_l = self.frags[lo].rect.x;
        let run_r = self.frags[hi].rect.x + self.frags[hi].rect.width;
        // `ruby-align` (lu sur le run, repli sur le segment) :
        // `start`/`space-between` = collé au début (`space-
        // between` étire en plus), `center`/`space-around`
        // (defaut spec) = centré ; `inter-character` = après
        // la base, sur sa ligne de base.
        let align = self.frags[lo]
            .style
            .get("ruby-align")
            .or_else(|| {
                (lo > 0)
                    .then(|| self.frags[lo - 1].style.get("ruby-align"))
                    .flatten()
            })
            .map_or("center", str::trim);
        let mut dx = if pos == "inter-character" {
            seg_r - run_l
        } else {
            match align {
                "start" | "space-between" => seg_l - run_l,
                _ => seg_l.midpoint(seg_r) - run_l.midpoint(run_r),
            }
        };
        let base_ref = if lo > 0 {
            let b = &self.frags[lo - 1];
            (b.rect.y, b.baseline)
        } else {
            (0.0, 0.0)
        };
        // `space-between` : l'espace entre fragments absorbe
        // le reliquat jusqu'aux bords du segment (§4.4 — un
        // seul fragment = collé au départ).
        let span_gap = if align == "space-between" && hi > lo {
            ((seg_r - seg_l) - (run_r - run_l)) / (hi - lo) as f32
        } else {
            0.0
        };
        if pos != "inter-character" {
            let (cl, cr) = if cont.0.is_finite() {
                cont
            } else {
                (seg_l, seg_r)
            };
            dx = if run_r - run_l > cr - cl {
                cl - run_l
            } else {
                dx.clamp(cl - run_l, cr - run_r)
            };
        }
        for (k, f) in self.frags[lo..=hi].iter_mut().enumerate() {
            f.rect.x += dx + span_gap * k as f32;
            f.rect.y = match pos {
                "under" => top_under,
                "inter-character" => {
                    if lo > 0 {
                        base_ref.0 + (base_ref.1 - f.baseline)
                    } else {
                        f.rect.y
                    }
                }
                _ => top_over + band_h - f.rect.height,
            };
        }
    }
}

/// Niveau `rtc` d'un fragment (`0` = `rt` nu hors `rtc`).
fn ruby_tier(f: &Fragment) -> u32 {
    f.style
        .get("x-ruby-tier")
        .and_then(|v| v.trim().parse::<u32>().ok())
        .unwrap_or(0)
}

/// Runs consécutifs de fragments `ruby-text*` — indices
/// `(début, fin)` inclusifs dans `frags`. Un changement de
/// niveau `rtc` coupe le run : chaque `rtc` a sa propre
/// bande (css-ruby-1 §3.2).
fn rt_runs(frags: &[Fragment]) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();
    for (i, f) in frags.iter().enumerate() {
        if is_ruby_text_frag(f) {
            match out.last_mut() {
                Some((_, hi)) if i == *hi + 1 && ruby_tier(f) == ruby_tier(&frags[*hi]) => {
                    *hi = i;
                }
                _ => out.push((i, i)),
            }
        }
    }
    out
}

/// Étendue horizontale `(gauche, droite)` du segment de
/// base `ruby*` contigu précédant l'indice `rt_lo` — repli
/// sur toute la base non-rt de la ligne.
fn ruby_base_span(frags: &[Fragment], rt_lo: usize) -> (f32, f32) {
    let (mut l, mut r) = (f32::INFINITY, 0.0_f32);
    let mut i = rt_lo;
    while i > 0 {
        i -= 1;
        let f = &frags[i];
        if !f
            .style
            .get("display")
            .map_or("", |d| d.trim())
            .starts_with("ruby")
            || is_ruby_text_frag(f)
        {
            break;
        }
        l = l.min(f.rect.x);
        r = r.max(f.rect.x + f.rect.width);
    }
    if !l.is_finite() {
        for f in frags {
            if !is_ruby_text_frag(f) {
                l = l.min(f.rect.x);
                r = r.max(f.rect.x + f.rect.width);
            }
        }
    }
    (if l.is_finite() { l } else { 0.0 }, r)
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
    strut_lh: f32,
    indent_first: f32,
    indent_rest: f32,
    para_lv: u8,
) -> Vec<LineBox> {
    /// Requêtes de bande bornées (contenu hostile : les
    /// flottants sont finis, chaque descente en franchit un).
    const MAX_BAND_STEPS: u32 = 128;

    let mut lines = Vec::new();
    // R-171 — ancre RTL de la ligne : le contenu se re-tuile
    // depuis le bord droit de la bande (`reorder_bidi`).
    let base_rtl = para_lv == 1;
    // Baseline du strut = demi-leading + ascent (comme un
    // fragment texte de la fonte du conteneur) — avec
    // `line-height: 0` elle tombe sous l'em box et ne
    // relève ni la baseline ni la hauteur de la ligne.
    let strut_base = (strut_lh - parent_fs).mul_add(0.5, parent_fs * 0.8);
    let mut acc = LineAcc::new(strut_lh, strut_base);
    let (mut line_x, mut line_w) = fl.band(y, x, w);
    // `text-indent` décale le départ de la première ligne
    // seulement (`new_line!` repart de `line_x` brut) ; les
    // tests de fit soustraient déjà `cursor_x - line_x`, la
    // largeur de bande reste la référence.
    let mut cursor_x = line_x + indent_first;
    let mut cursor_y = y;

    // Descend la ligne tant que la bande est vide ou trop
    // étroite pour un fragment **seul** de `pw` — borne dure.
    // `+ 0.01` sur le fit : la mesure par pièces du line-break
    // peut dépasser le max-content intrinsèque qui a taillé
    // le flottant parent d'un ulp agrégé (~1e-5) — sans
    // tolérance, un flottant shrink-to-fit voit son propre
    // texte « déborder » et coupe (issue #13, nav weather.gov).
    macro_rules! descend_band {
        ($pw:expr) => {{
            let pw = $pw;
            let mut steps = 0u32;
            while (line_w <= 0.01 || pw > line_w + 0.01) && steps < MAX_BAND_STEPS {
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
            acc.close(&mut cursor_y, &mut lines, ctx, base_rtl, line_x + line_w);
            (line_x, line_w) = fl.band(cursor_y, x, w);
            cursor_x = line_x + indent_rest;
        }};
    }

    for item in items {
        let fs = font_size_px(&item.style, parent_fs, ctx);
        // R-207 — la valeur calculée est matérialisée en px
        // dans le style porté par le fragment : les
        // consommateurs aval (paint `prop_px`/`font_size_of`,
        // `em` des bordures et décorations) ne savent pas
        // résoudre `calc()`/`clamp()`/`min()`/`max()` — sans
        // ça ils retombaient sur 16 px et rastraient les
        // glyphes à une taille différente des avances shapées
        // (plyr.io : titre éclaté lettre par lettre).
        let frag_style = {
            let mut s = item.style.clone();
            s.set("font-size", &format!("{fs}px"));
            s
        };
        let lh = line_height_of(&item.style, fs, ctx);
        let font_req = font::request_of(&item.style, ctx);
        let measure = measure_fn(&item.style, fs, ctx);
        // R-168 — `word-spacing` : supplément par caractère
        // espace (% de l'avance de l'espace, §16.4.7 —
        // `normal` → 0, donc inerte).
        let ws = item
            .style
            .get("word-spacing")
            .and_then(|v| raw_length_px(v, measure(" "), fs, ctx))
            .unwrap_or(0.0);
        let measure = move |s: &str| -> f32 {
            let w = measure(s);
            if ws.abs() < f32::EPSILON {
                w
            } else {
                let spaces: u16 = s.matches(' ').count().try_into().unwrap_or(u16::MAX);
                w + ws * f32::from(spaces)
            }
        };

        // Boîte atomique : coule comme une pièce incassable.
        // Ses marges horizontales font partie de sa boîte
        // externe (§8.3) — y compris en tête de ligne après
        // un wrap : `margin-left` s'y applique aussi.
        if let Some(aid) = item.aid {
            let Some(r) = atomics.get(&aid) else {
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
            // `+ 0.01` comme le test texte et `descend_band!`
            // : les atomes sommant exactement `line_w`
            // (shrink-to-fit mesuré à l'identique)
            // s'enroulaient sur le bruit f32 — liens nav
            // google.com empilés (§B-64).
            let fits = cursor_x - line_x + outer <= line_w + 0.01
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
                aid: item.aid,
                style: frag_style.clone(),
                rect: Rect {
                    x: px + ml,
                    y: cursor_y + mt,
                    width: r.width,
                    height: r.height,
                },
                baseline: r.height,
                shaped: None,
                trail_w: 0.0,
                marg_l: ml,
                marg_r: mr,
                marg_t: mt,
                marg_b: mb,
                deco: Vec::new(),
                level: para_lv,
            });
            // (`rect.y = y + base - h`), `mb` allonge la ligne.
            acc.max_lh = acc.max_lh.max(mt + r.height + mb);
            acc.max_base = acc.max_base.max(mt + r.height);
            cursor_x = px + outer;
            continue;
        }

        // Borne d'élément inline non-remplacé : `margin-left`
        // à l'ouverture, `margin-right` à la fermeture
        // (§8.3/§9.4.2 — les verticales sont ignorées).
        // Fragment zéro-largeur portant la marge pour que
        // `align_lines` mesure la margin box ; le marqueur
        // n'ouvre jamais de ligne ni n'influe sa hauteur.
        if let Some(mark) = item.marker {
            let open = matches!(mark, Marker::Open);
            // Bord consommé sur la ligne : marge + bordure +
            // remplissage du côté concerné (§8.3/§9.4.2 — R-211 :
            // `padding:0 8px` sur `<a>` inline retombait à 0 →
            // items nav contigus, vol 772 dr.dk/vimeo/bitmovin).
            let side = if open { "left" } else { "right" };
            let m = side_px(&item.style, &format!("margin-{side}"), w, fs, ctx, 0.0)
                + side_px(
                    &item.style,
                    &format!("border-{side}-width"),
                    w,
                    fs,
                    ctx,
                    0.0,
                )
                + side_px(&item.style, &format!("padding-{side}"), w, fs, ctx, 0.0);
            if m != 0.0 {
                let (fx, marg_l, marg_r) = if open {
                    (cursor_x + m, m, 0.0)
                } else {
                    (cursor_x, 0.0, m)
                };
                acc.frags.push(Fragment {
                    node: item.node,
                    owner: item.owner,
                    text: String::new(),
                    atomic: false,
                    aid: None,
                    style: frag_style.clone(),
                    rect: Rect {
                        x: fx,
                        y: cursor_y,
                        width: 0.0,
                        height: 0.0,
                    },
                    baseline: 0.0,
                    shaped: None,
                    trail_w: 0.0,
                    marg_l,
                    marg_r,
                    marg_t: 0.0,
                    marg_b: 0.0,
                    deco: Vec::new(),
                    level: para_lv,
                });
                cursor_x += m;
            }
            continue;
        }

        // `pre`/`pre-wrap`/`preserve-spaces`/`break-spaces` :
        // §16.6.1 ne s'applique pas — les espaces de tête de
        // ligne sont préservées aussi (R-264 : mode effectif
        // via `ws_mode` — `white-space-collapse`/`text-wrap-mode`
        // réels, pas seulement le shorthand).
        let (ws_collapse, ws_wrap) = ws_mode(&item.style);
        let keep_ws = matches!(
            ws_collapse,
            WsCollapse::Preserve | WsCollapse::PreserveSpaces | WsCollapse::BreakSpaces
        );
        for piece in &item.pieces {
            // §8.1 — taquet de tabulation (`pre`/`pre-wrap`) :
            // l'avance mène au prochain multiple de `tab-size`
            // compté depuis le bord de départ du conteneur de
            // bloc (gauche en LTR, droite en RTL). Fragment
            // non-atomique sans glyphe : rien à peindre, et
            // `close` le retire s'il pend en fin de ligne
            // (blanc pendant, §16.6.1).
            if piece.tab {
                let tab_w = tab_size_px(&item.style, measure(" "), fs, ctx);
                let pos = if base_rtl {
                    x + w - cursor_x
                } else {
                    cursor_x - x
                };
                // `rem_euclid` ∈ [0, tab_w) ; déjà sur un
                // taquet (r ≈ 0) → taquet SUIVANT (avance
                // pleine — une tabulation avance toujours).
                let adv = if tab_w <= f32::EPSILON {
                    0.0
                } else {
                    let r = pos.rem_euclid(tab_w);
                    if r < 0.01 {
                        tab_w
                    } else {
                        tab_w - r
                    }
                };
                // `text-wrap-mode:nowrap`/`pre` : les coupures
                // souples sont interdites — une tabulation qui
                // déborde continue à droite (R-264).
                if ws_wrap
                    && cursor_x > line_x
                    && cursor_x - line_x + adv > line_w + 0.01
                    && !acc.is_empty()
                {
                    new_line!();
                }
                if acc.is_empty() {
                    descend_band!(adv);
                }
                acc.frags.push(Fragment {
                    node: item.node,
                    owner: item.owner,
                    text: piece.text.clone(),
                    atomic: false,
                    aid: None,
                    style: frag_style.clone(),
                    rect: Rect {
                        x: cursor_x,
                        y: cursor_y,
                        width: adv,
                        height: lh,
                    },
                    baseline: fs * 0.8,
                    shaped: None,
                    trail_w: 0.0,
                    marg_l: 0.0,
                    marg_r: 0.0,
                    marg_t: 0.0,
                    marg_b: 0.0,
                    deco: item.deco.clone(),
                    level: piece.level,
                });
                acc.max_lh = acc.max_lh.max(lh);
                acc.max_base = acc.max_base.max(fs * 0.8);
                cursor_x += adv;
                if piece.hard_after {
                    new_line!();
                }
                continue;
            }
            // §16.6.1 : les espaces en TÊTE de ligne sont
            // supprimées — y compris en tête d'une pièce
            // non-blanche (le collapse fusionne `"  mot"`).
            let line_start = cursor_x <= line_x;
            let text = if line_start && !keep_ws {
                piece.text.trim_start_matches(text::is_css_ws)
            } else {
                piece.text.as_str()
            };
            if text.is_empty() {
                // Pièce réduite à sa coupure dure — le `<br>`
                // porte un U+2028 strippé, `\n` nu en `pre` :
                // rien à rendre mais la ligne force quand
                // même. Sentinelle atomique zéro-largeur
                // pour qu'une ligne vide soit quand même
                // émise (`<br>` seul, lignes vides de `pre`)
                // — `atomic` la met à l'abri du retrait
                // des blancs de `close`.
                if piece.hard_after {
                    acc.frags.push(Fragment {
                        node: item.node,
                        owner: item.owner,
                        text: String::new(),
                        atomic: true,
                        aid: None,
                        style: frag_style.clone(),
                        rect: Rect {
                            x: cursor_x,
                            y: cursor_y,
                            width: 0.0,
                            height: lh,
                        },
                        baseline: fs * 0.8,
                        shaped: None,
                        trail_w: 0.0,
                        marg_l: 0.0,
                        marg_r: 0.0,
                        marg_t: 0.0,
                        marg_b: 0.0,
                        deco: Vec::new(),
                        level: para_lv,
                    });
                    acc.max_lh = acc.max_lh.max(lh);
                    acc.max_base = acc.max_base.max(fs * 0.8);
                    new_line!();
                }
                continue;
            }
            let pw = measure(text);
            // `nowrap`/`pre` (`text-wrap-mode:nowrap` — R-264)
            // : les coupures SOUPLES sont interdites (§16.6) —
            // seul `hard_after` (<br>, \n en pre) force une
            // ligne. Sans ce garde le texte `nowrap` se coupait
            // comme `normal` — bloquait tout débordement
            // `text-overflow`.
            let no_wrap = !ws_wrap;
            // `break-spaces` : l'opportunité de coupure est
            // APRES chaque espace — la première espace d'une
            // suite se pose donc sur la ligne courante même si
            // elle la fait déborder (« hang », §4.1.3). Les
            // suivantes — le fragment posé juste avant est un
            // blanc — cassent normalement en pleine suite.
            let hangs = ws_collapse == WsCollapse::BreakSpaces
                && !text.is_empty()
                && text.chars().all(text::is_css_ws)
                && !acc
                    .frags
                    .last()
                    .is_some_and(|f| !f.atomic && text::trim_css_ws(&f.text).is_empty());
            // Même tolérance `+ 0.01` que `descend_band!` :
            // dépassement sub-ulp agrégé → la pièce tient,
            // sinon `new_line!`.
            if !no_wrap
                && !hangs
                && cursor_x > line_x
                && cursor_x - line_x + pw > line_w + 0.01
                && !acc.is_empty()
            {
                new_line!();
            }
            if acc.is_empty() {
                descend_band!(pw);
            }
            // `overflow-wrap`/`word-break` (R-126 — css-text-3
            // §5.5) : la pièce dépasse la bande même seule sur
            // sa ligne → la couper au plus long préfixe tenant,
            // le reste continue sur les lignes suivantes. Sans
            // cette borne un mot long (URL, `break-words`
            // Tailwind) débordait sa boîte au lieu de couper.
            // R-262 : `word-break:break-word` (alias déprécié
            // d'`overflow-wrap:break-word`, css-text-3 §6.1),
            // `word-wrap` (nom legacy d'`overflow-wrap`) et
            // `line-break:anywhere` (§6.2 — opportunité souple
            // autour de chaque caractère typographique) ouvrent
            // la même coupure intra-mot.
            let break_any = matches!(
                item.style.get("overflow-wrap"),
                Some("break-word" | "anywhere")
            ) || matches!(
                item.style.get("word-wrap"),
                Some("break-word" | "anywhere")
            ) || matches!(
                item.style.get("word-break"),
                Some("break-all" | "break-word")
            ) || matches!(item.style.get("line-break"), Some("anywhere"));
            let segs = if break_any && !no_wrap && pw > line_w + 0.01 {
                split_to_fit(text, line_w - (cursor_x - line_x), line_w, &measure)
            } else {
                vec![text]
            };
            for (si, seg) in segs.iter().enumerate() {
                if si > 0 {
                    new_line!();
                }
                let pw = measure(seg);
                let shaped = ctx.fonts.and_then(|src| {
                    ctx.font_cache.borrow_mut().shape(
                        src,
                        &font_req,
                        seg,
                        fs,
                        piece.dir == Direction::Rtl,
                        crate::block::letter_spacing_px(&item.style, fs, ctx),
                    )
                });
                let baseline = shaped.as_ref().map_or(fs * 0.8, |s| s.metrics.ascent);
                let trail_w = if si + 1 == segs.len() && seg.ends_with(text::is_css_ws) {
                    pw - measure(seg.trim_end_matches(text::is_css_ws))
                } else {
                    0.0
                };
                acc.frags.push(Fragment {
                    node: item.node,
                    owner: item.owner,
                    text: (*seg).to_string(),
                    atomic: false,
                    aid: None,
                    style: frag_style.clone(),
                    rect: Rect {
                        x: cursor_x,
                        y: cursor_y,
                        width: pw,
                        height: lh,
                    },
                    baseline,
                    shaped,
                    trail_w,
                    marg_l: 0.0,
                    marg_r: 0.0,
                    marg_t: 0.0,
                    marg_b: 0.0,
                    deco: item.deco.clone(),
                    level: piece.level,
                });
                acc.max_lh = acc.max_lh.max(lh);
                acc.max_base = acc.max_base.max(baseline);
                cursor_x += pw;
            }
            if piece.hard_after {
                new_line!();
            }
        }
    }
    acc.close(&mut cursor_y, &mut lines, ctx, base_rtl, line_x + line_w);
    lines
}

/// `text-align` sur les lignes posées. `justify` étire
/// l'espace inter-fragments (approximation word-spacing,
/// documentée) sauf sur la dernière ligne. La bande retenue
/// est l'intersection sur toute l'étendue verticale de la
/// ligne (§9.5 : une line box est raccourcie par TOUT
/// flottant qu'elle touche, pas seulement à son haut).
/// `start`/`end` suivent `direction` (rtl → start = right).
fn align_lines(
    st: &ComputedStyle,
    lines: &mut [LineBox],
    x: f32,
    w: f32,
    fl: &FloatCtx,
    indent: f32,
    hanging: bool,
) {
    let rtl = para_level(st) == 1;
    let align = match st.get_or("text-align", "start") {
        "start" => {
            if rtl {
                "right"
            } else {
                "left"
            }
        }
        "end" => {
            if rtl {
                "left"
            } else {
                "right"
            }
        }
        // `-webkit-center` (UA : <center>, comportement
        // historique) = `center` pour les lignes inline ; la
        // centrage des boîtes bloc filles est fait dans
        // `flow_children`.
        "-webkit-center" => "center",
        a => a,
    };
    let count = lines.len();
    for (i, line) in lines.iter_mut().enumerate() {
        // Largeur de ligne = étendue des **margin boxes** :
        // les marges d'un atome comptent dans la ligne pour
        // l'alignement (§16.2 — `text-align:right` sur des
        // inline-blocks à marges pousse jusqu'au bord la
        // margin box, pas la border box).
        let (mut mleft, mut mright) = (f32::INFINITY, 0.0f32);
        for f in &line.fragments {
            mleft = mleft.min(f.rect.x - f.marg_l);
            mright = mright.max(f.rect.x + f.rect.width + f.marg_r);
        }
        if !mleft.is_finite() {
            continue;
        }
        let used = mright - mleft;
        let (ebx, ebw) = fl.band_over(line.rect.y, line.rect.height, x, w);
        let slack = (ebw - used).max(0.0);
        let last = i + 1 == count;
        // Le bord gauche de la bande intersectée peut être à
        // droite de celui où la ligne a été coulée (flottant
        // commençant au milieu de la ligne) : on y recale
        // d'abord, puis l'alignement consomme le slack restant.
        // Jamais de tirage vers la gauche (max 0).
        // R-70 : la première ligne recale sur le bord **indenté**
        // (`ebx + indent`) — `text-indent` négatif n'est pas
        // annulé par le recalage (image-remplacement des
        // sprites, `.sprite{text-indent:-9999px}` Wikipedia).
        // `hanging` : l'indent s'applique aux lignes SAUF la
        // première — edge décalé sur i>0 seulement.
        let edge = if i == 0 {
            if hanging {
                ebx
            } else {
                ebx + indent
            }
        } else if hanging {
            ebx + indent
        } else {
            ebx
        };
        // `text-indent` négatif : le recalage tire vers la
        // GAUCHE (le bord indenté sort de la bande — technique
        // image-remplacement des sprites). Sans indent négatif
        // sur cette ligne, jamais de tirage gauche (flottant
        // commençant au milieu de la ligne).
        let indented = (i == 0 && !hanging) || (i != 0 && hanging);
        let recale = if indented && indent < 0.0 {
            edge - mleft
        } else {
            (edge - mleft).max(0.0)
        };
        #[allow(clippy::cast_precision_loss)]
        let (dx, space) = match align {
            "right" => ((ebx + ebw) - mright, 0.0),
            "center" => (recale + slack / 2.0, 0.0),
            "justify" if !last && line.fragments.len() > 1 => {
                (recale, slack / (line.fragments.len() - 1) as f32)
            }
            _ => (recale, 0.0),
        };
        if dx == 0.0 && space <= 0.0 {
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
        // Hors-flux (`absolute`/`fixed`) : aucun fragment émis
        // — mais `resolve_absolutes` lit `rect` comme position
        // STATIQUE quand `top`/`left` sont `auto` (§10.6.4 ;
        // le motif `.a11y-menu` de MDN : `a` absolu seul dans
        // un `ul` absolu hors-champ ancrait à y=0). Place
        // hypothétique = fin du dernier fragment d'un enfant
        // précédent, sinon début de la première ligne, sinon
        // l'origine du conteneur (aucune ligne en flux).
        if child.out_of_flow() {
            let mut pos = None;
            for line in lines {
                for f in &line.fragments {
                    if f.owner < i {
                        pos = Some((f.rect.x + f.rect.width, line.rect.y));
                    }
                }
            }
            let (sx, sy) = pos
                .or_else(|| lines.first().map(|l| (l.rect.x, l.rect.y)))
                .unwrap_or((bx.rect.x, bx.rect.y));
            child.rect.x = sx;
            child.rect.y = sy;
            continue;
        }
        // Atome : rect et sous-arbre déjà posés par
        // `assign_atomic_rects` — les indices `owner` de ces
        // lignes ne parlent pas de son intérieur.
        if is_atomic(child) {
            continue;
        }
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
        // Arête ∝ profondeur de l'arbre (R-24).
        explorer_common::deep(|| assign_nested_rects(child, lines));
    }
}

/// Rects des descendants d'un enfant direct de la boîte à
/// lignes (récursif). `Fragment::owner` n'indexe que les
/// enfants **directs** de la racine IFC : en profondeur,
/// comparer l'indice local à `owner` copiait le rect d'un
/// sibling (R-90 — le `<input>` « Tout accepter » du
/// consentement Google recevait le rect du 1ᵉʳ bouton →
/// hit-test → `form`, clic `Ignored`). Un descendant
/// retrouve donc les siens via `f.node` ∈ son sous-arbre :
/// le fragment porte le nœud DOM de sa feuille émettrice
/// (texte, atome, élément de marqueur). Imprécision bornée
/// documentée : les fragments de pseudo-boîtes (`node:
/// None`) et les marqueurs de morceaux `split_piece`
/// partageant un même `node` peuvent manquer ou s'agréger à
/// la mauvaise moitié — jamais le rect d'un sibling disjoint.
fn assign_nested_rects(bx: &mut LayoutBox, lines: &[LineBox]) {
    // Nœuds du contenu in-flow des frères déjà traités — la
    // position statique d'un hors-flux s'ancre à leur
    // dernier fragment (même logique que la racine).
    let mut seen = std::collections::HashSet::new();
    for child in &mut bx.children {
        if child.out_of_flow() {
            let mut pos = None;
            for line in lines {
                for f in &line.fragments {
                    if f.node.is_some_and(|n| seen.contains(&n)) {
                        pos = Some((f.rect.x + f.rect.width, line.rect.y));
                    }
                }
            }
            let (sx, sy) = pos.unwrap_or((bx.rect.x, bx.rect.y));
            child.rect.x = sx;
            child.rect.y = sy;
            continue;
        }
        let mut mine = std::collections::HashSet::new();
        subtree_nodes(child, &mut mine);
        // Atome : rect posé par `assign_atomic_rects` via
        // `aid` — mais ses fragments comptent comme contenu
        // précédent pour un frère hors-flux.
        if !is_atomic(child) {
            let mut union: Option<Rect> = None;
            for line in lines {
                for f in &line.fragments {
                    if f.node.is_some_and(|n| mine.contains(&n)) {
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
            // Arête ∝ profondeur de l'arbre (R-24).
            explorer_common::deep(|| assign_nested_rects(child, lines));
        }
        seen.extend(mine);
    }
}

/// `NodeId`s du sous-arbre de `bx` (la boîte incluse) —
/// base d'attribution des fragments à un descendant,
/// `f.node` portant le nœud de la feuille émettrice.
fn subtree_nodes(bx: &LayoutBox, out: &mut std::collections::HashSet<explorer_dom::NodeId>) {
    if let Some(n) = bx.node {
        out.insert(n);
    }
    for c in &bx.children {
        explorer_common::deep(|| subtree_nodes(c, out));
    }
}

/// Rects des boîtes atomiques nichées : `owner` n'atteint
/// que l'enfant direct — chaque atome retrouve son fragment
/// par sa série `aid` (même ordre pré-ordre que
/// [`size_atomics`]) et copie son rect posé.
fn assign_atomic_rects(
    bx: &mut LayoutBox,
    lines: &[LineBox],
    aid: &mut usize,
    w: f32,
    fs: f32,
    ctx: &BlockCtx,
) {
    for child in &mut bx.children {
        // Même saut hors-flux que `size_atomics`/`collect_child`
        // — sinon la série `aid` se désynchronise.
        if child.out_of_flow() {
            continue;
        }
        if is_atomic(child) {
            let id = *aid;
            *aid += 1;
            for line in lines {
                for f in &line.fragments {
                    if f.aid == Some(id) {
                        // `layout_block` a posé le sous-arbre
                        // en (0,0) → translation entière au
                        // rect du fragment (rect, lignes,
                        // enfants — R-23). Un atome
                        // `position:relative` s'est déjà
                        // décalé au layout : le fragment
                        // s'ancre sur sa position **en
                        // flux**, son offset propre reste.
                        let (ox, oy) = crate::block::rel_offsets(child, w, fs, ctx);
                        let dx = f.rect.x - (child.rect.x - ox);
                        let dy = f.rect.y - (child.rect.y - oy);
                        crate::flex::translate_subtree(child, dx, dy);
                        break;
                    }
                }
            }
        } else {
            // Arête ∝ profondeur de l'arbre (R-24).
            explorer_common::deep(|| assign_atomic_rects(child, lines, aid, w, fs, ctx));
        }
    }
}

/// Union de deux rectangles.
pub(crate) fn union_rect(first: Rect, second: Rect) -> Rect {
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

/// `line-height` en px : `normal` (ou absent) → métriques
/// réelles de la fonte (`natural_line_height`, §10.8.1 —
/// 1.2em en repli sans `FontSource`).
fn line_height_of(style: &ComputedStyle, fs: f32, ctx: &BlockCtx) -> f32 {
    let normal = crate::font::natural_line_height(style, fs, ctx);
    let Some(raw) = style.get("line-height") else {
        return normal;
    };
    let raw = raw.trim();
    if raw == "normal" {
        return normal;
    }
    if let Ok(n) = raw.parse::<f32>() {
        return (n * fs).max(1.0);
    }
    css_px(style, "line-height", fs, fs, ctx).unwrap_or(normal)
}

/// Découpe `text` en segments tenant chacun dans la bande
/// (`avail` sur la première ligne, `line_w` ensuite) —
/// `overflow-wrap: break-word`/`anywhere` et
/// `word-break: break-all` (R-126). Coupure aux frontières
/// UTF-8 ; au moins un caractère par segment (progression
/// garantie même si la bande est plus étroite qu'un glyphe).
fn split_to_fit<'a>(
    text: &'a str,
    avail: f32,
    line_w: f32,
    measure: &dyn Fn(&str) -> f32,
) -> Vec<&'a str> {
    let mut segs = Vec::new();
    let mut rest = text;
    let mut budget = avail;
    while measure(rest) > line_w.max(budget) + 0.01 && !rest.is_empty() {
        let mut cut = 0;
        for (i, ch) in rest.char_indices() {
            let end = i + ch.len_utf8();
            if measure(&rest[..end]) <= budget + 0.01 {
                cut = end;
            } else {
                break;
            }
        }
        if cut == 0 {
            cut = rest.char_indices().nth(1).map_or(rest.len(), |(i, _)| i);
        }
        segs.push(&rest[..cut]);
        rest = &rest[cut..];
        budget = line_w;
    }
    if !rest.is_empty() {
        segs.push(rest);
    }
    segs
}

/// capitalize (R-162) : premiere lettre de chaque mot en
/// capitale, le reste inchange (approximation UAX #29 —
/// un mot = suite alphanumerique ; un chiffre en tete
/// garde le mot ouvert : «2abc» → «2Abc»).
fn capitalize_words(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut boundary = true;
    for c in s.chars() {
        if boundary && c.is_alphabetic() {
            out.extend(c.to_uppercase());
            boundary = false;
        } else {
            out.push(c);
            if !c.is_alphanumeric() {
                boundary = true;
            }
        }
    }
    out
}
