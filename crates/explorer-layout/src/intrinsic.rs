// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tailles intrinsèques (R-3) : `min-content`, `max-content`
//! et `fit-content` mesurées sur le contenu **réel** — texte
//! pesé via la fonte injectée (ADR-0009), sous-arbres
//! récursés. Fini l'approximation « taille du texte direct »
//! pour la mesure ; le layout `width` les résout en px.
//!
//! Convention CSS Sizing : le **max-content** est la plus
//! grande largeur sans coupure souple ; le **min-content**
//! le plus grand morceau insécable (mot le plus long,
//! atome remplacé le plus large). `fit-content` =
//! `min(max-content, max(min-content, disponible))`.
//!
//! Approximations documentées (MVP) :
//! - les enfants de niveau bloc contribuent leur **max**
//!   (flux vertical — les largeurs ne s'additionnent pas) ;
//! - une suite d'enfants en-ligne contribue la **somme** des
//!   max-content, et le **max** des min-content (les
//!   coupures souples autorisées entre eux) ;
//! - `table-row` somme ses cellules (colonnes côte à côte) ;
//! - marges/paddings des enfants ignorés, sauf le padding
//!   propre porté par l'appelant (`width` = content box).

use crate::block::{box_size_to_content, css_px, font_size_px, measure_fn, side_px, BlockCtx};
use crate::boxtree::{BoxKind, LayoutBox};
use explorer_css::ComputedStyle;

/// `width`/`min-width`/`max-width`/`flex-basis` mot-clé → px
/// mesurés, `None` si la valeur n'est pas un mot-clé
/// intrinsèque.
pub(crate) fn intrinsic_size_px(
    st: &ComputedStyle,
    prop: &str,
    bx: &LayoutBox,
    basis: f32,
    fs: f32,
    ctx: &BlockCtx,
) -> Option<f32> {
    // Marges de l'axe concerné : `stretch`/`fit-content`
    // remplissent le bloc conteneur **moins** les marges
    // (css-sizing-4 §4.1 — la margin box comble l'espace).
    let (m1, m2) = if matches!(
        prop,
        "height" | "min-height" | "max-height" | "block-size" | "min-block-size" | "max-block-size"
    ) {
        ("margin-top", "margin-bottom")
    } else {
        ("margin-left", "margin-right")
    };
    let avail =
        (basis - side_px(st, m1, basis, fs, ctx, 0.0) - side_px(st, m2, basis, fs, ctx, 0.0))
            .max(0.0);
    match st.get(prop)?.trim() {
        "max-content" => Some(max_content_px(bx, fs, ctx)),
        "min-content" => Some(min_content_px(bx, fs, ctx)),
        // `stretch` et alias vendor (`-webkit-fill-available`
        // ≈ `stretch`, `-moz-available` ≈ `fit-content` au
        // sens « disponible » — approximations documentées,
        // css-sizing-4 §4) : la margin box comble le bloc
        // conteneur — différent d'`auto` dès que la boîte
        // est shrink-to-fit (absolu, flottant, item non
        // étiré, `flex-basis`).
        "stretch" | "fill-available" | "-webkit-fill-available" | "-moz-available" => Some(avail),
        "fit-content" => {
            Some(max_content_px(bx, fs, ctx).min(min_content_px(bx, fs, ctx).max(avail)))
        }
        _ => None,
    }
}

/// `container-type` pose-t-il un containment de taille sur
/// l'axe **inline** de `bx` ? `size` (les deux axes) et
/// `inline-size` (§2.2, token-wise — `inline-size
/// scroll-state` reste un size container). Inerte sur les
/// boîtes de niveau inline (§2.1 — containment uniquement
/// sur principale non-inline).
pub(crate) fn size_contained(st: &ComputedStyle, bx: &LayoutBox) -> bool {
    !bx.kind.is_inline_level()
        && st.get("container-type").is_some_and(|v| {
            v.split_whitespace()
                .any(|t| matches!(t, "size" | "inline-size"))
        })
}

/// Idem sur l'axe **bloc** — seul `size` contient les deux
/// axes (`inline-size` laisse la hauteur suivre le contenu).
pub(crate) fn block_size_contained(st: &ComputedStyle, bx: &LayoutBox) -> bool {
    !bx.kind.is_inline_level()
        && st
            .get("container-type")
            .is_some_and(|v| v.split_whitespace().any(|t| t == "size"))
}

/// Largeur max-content du contenu de `bx` (content box).
pub(crate) fn max_content_px(bx: &LayoutBox, fs: f32, ctx: &BlockCtx) -> f32 {
    content_px(bx, fs, ctx, true)
}

/// Largeur min-content du contenu de `bx` (content box).
pub(crate) fn min_content_px(bx: &LayoutBox, fs: f32, ctx: &BlockCtx) -> f32 {
    content_px(bx, fs, ctx, false)
}

/// `min|max-width` converties en contenu — les bornes de la
/// contribution intrinsèque (Sizing §5.1 : toute taille
/// intrinsèque utilisée est bridée par elles). Un `%` est
/// indéfini en mesure intrinsèque → ignoré, **sauf** si une
/// base `cb` définitive est fournie (Flexbox §9.9.3 : les
/// descendants d'un item flex résolvent leurs `%` contre la
/// taille main du conteneur — `img{max-width:100%}` sous un
/// ancêtre `auto` borne à la ligne, §B-38).
fn width_bounds(
    st: &ComputedStyle,
    fs: f32,
    ctx: &BlockCtx,
    cb: Option<f32>,
) -> (Option<f32>, Option<f32>) {
    let chassis: f32 = [
        "padding-left",
        "padding-right",
        "border-left-width",
        "border-right-width",
    ]
    .iter()
    .map(|p| side_px(st, p, 0.0, fs, ctx, 0.0))
    .sum();
    let bound = |p: &str| -> Option<f32> {
        let v = st.get(p)?;
        if v.contains('%') {
            let c = cb?;
            return css_px(st, p, c, fs, ctx).map(|v| box_size_to_content(st, v, chassis));
        }
        css_px(st, p, 0.0, fs, ctx).map(|v| box_size_to_content(st, v, chassis))
    };
    (bound("min-width"), bound("max-width"))
}

/// Mesure mutualisée : `max` = insécable complet, `min` =
/// plus grand morceau insécable. `fs` = font-size **de bx**
/// (résolu par l'appelant). Le résultat — taille directe
/// comme mesure des enfants — est borné par les
/// `min|max-width` de `bx` : une boîte ne contribue jamais
/// au-delà de son propre plafond (`thumbinner{
/// max-width:170px}` de Wikipedia ne laisse pas sa légende
/// gonfler le flottant père — issue #12, colonne étranglée).
/// Les boîtes `Text` portent le style du parent : leur
/// borne s'applique au niveau de l'agrégat parent.
fn content_px(bx: &LayoutBox, fs: f32, ctx: &BlockCtx, max: bool) -> f32 {
    content_px_opt(bx, fs, ctx, max, false, None)
}

/// Variantes avec base `%` définitive `cb` — Flexbox §9.9.3 :
/// la mesure du contenu d'un item résout les `%` des
/// descendants contre la taille main du conteneur (définie).
pub(crate) fn max_content_px_cb(bx: &LayoutBox, fs: f32, ctx: &BlockCtx, cb: f32) -> f32 {
    content_px_opt(bx, fs, ctx, true, false, Some(cb))
}

/// [`max_content_px_cb`] — variante min-content.
pub(crate) fn min_content_px_cb(bx: &LayoutBox, fs: f32, ctx: &BlockCtx, cb: f32) -> f32 {
    content_px_opt(bx, fs, ctx, false, false, Some(cb))
}

/// `min-content` de `bx` pour le plancher §4.5 du shrink
/// flex : le `width` propre de l'item est IGNORÉ (la taille
/// spécifiée n'est pas un minimum de contenu — un item
/// `width:80px` peut rétrécir sous 80) ; `max-width` borne.
/// Base `%` `cb` = taille main du conteneur flex.
pub(crate) fn min_content_size_px_cb(bx: &LayoutBox, fs: f32, ctx: &BlockCtx, cb: f32) -> f32 {
    content_px_opt(bx, fs, ctx, false, true, Some(cb))
}

fn content_px_opt(
    bx: &LayoutBox,
    fs: f32,
    ctx: &BlockCtx,
    max: bool,
    ignore_w: bool,
    cb: Option<f32>,
) -> f32 {
    let w = content_px_unclamped(bx, fs, ctx, max, ignore_w, cb);
    if bx.kind == BoxKind::Text {
        return w;
    }
    let (mn, mx) = width_bounds(&bx.style, fs, ctx, cb);
    mx.map_or(w, |m| w.min(m)).max(mn.unwrap_or(0.0))
}

/// Mesure brute, avant bridage `min|max-width` (appliqué
/// par [`content_px`]). La branche « taille directe »
/// (`width`/intrinsèque remplacé) se borne déjà elle-même
/// — le re-bridage du wrapper est idempotent.
#[allow(clippy::too_many_lines)] // chaîne de priorités §5 — 101 lignes, bornée
fn content_px_unclamped(
    bx: &LayoutBox,
    fs: f32,
    ctx: &BlockCtx,
    max: bool,
    ignore_w: bool,
    cb: Option<f32>,
) -> f32 {
    // Taille directe : `width` explicite (hors %) fait foi
    // AVANT l'intrinsèque remplacé (§5.1 : la contribution
    // d'un remplacé est sa taille utilisée — un
    // `img{width:40px}` mesure 40, pas son intrinsèque.
    // Terrain : icônes sociales rust-lang.org empilées une
    // par ligne, la base flex des `<a>` valant l'intrinsèque
    // 512 > conteneur). Un `%` indéfini retombe sur le
    // contenu (§5.2). `min|max-width` non-% bornent la
    // source retenue. `box-sizing` convertit les tailles
    // SPÉCIFIÉES en contenu (R-43) — l'intrinsèque est déjà
    // du contenu (les marges restent hors — comptées par
    // `own_sides` côté appelant).
    let st = &bx.style;
    let (min_b, max_b) = width_bounds(st, fs, ctx, cb);
    let chassis: f32 = [
        "padding-left",
        "padding-right",
        "border-left-width",
        "border-right-width",
    ]
    .iter()
    .map(|p| side_px(st, p, 0.0, fs, ctx, 0.0))
    .sum();
    let bound = |p: &str| -> Option<f32> {
        let v = st.get(p)?;
        if v.contains('%') {
            // R-224 — un `%` NU sur `width` (`width:100%`)
            // est indéfini en mesure intrinsèque, même avec
            // base : sinon un bloc `width:100%` dans un
            // flex item shrink-wrap s'étire au conteneur
            // (`vector-collapsible` wikipedia 1088 vs 297 —
            // ses menus abs `right:0` dérivaient dx +1032
            // px). Le §9.9.3 vaut pour `min|max-width` et
            // les maths à % (`min(300px,100%)`) — pas pour
            // un `width:%` nu dont le parent se mesure.
            let bare = v
                .trim()
                .strip_suffix('%')
                .is_some_and(|n| n.trim().parse::<f32>().is_ok());
            if bare {
                return None;
            }
            let c = cb?;
            return css_px(st, p, c, fs, ctx).map(|v| box_size_to_content(st, v, chassis));
        }
        css_px(st, p, 0.0, fs, ctx).map(|v| box_size_to_content(st, v, chassis))
    };
    let bound_v = |p: &str| -> Option<f32> {
        let v = st.get(p)?;
        if v.contains('%') {
            return None;
        }
        css_px(st, p, 0.0, fs, ctx)
    };
    // §4.1 cyclique : un `width:%` en mesure intrinsèque est
    // indéfini (le bloc contenant dépend de cette mesure).
    // R-284 — la réponse dépend de la mesure :
    // * **min-content** → contribution **0** : sinon un
    //   `img{width:100%}` imposerait son bitmap entier aux
    //   ancêtres shrink-to-fit (R-51, terrain : carte BBC
    //   `min-width:fit-content` à 480px — `fit-content` =
    //   `min(max, max(min, avail))` ne pouvait plus clamp
    //   à la piste 236, le plancher `min` valant déjà 480).
    // * **max-content** → `%` traité `auto` → **intrinsèque**
    //   (§5.1 : remplacé `auto` = taille intrinsèque) — sinon
    //   tout remplacé `width:100%` en item flex shrink-to-fit
    //   mesurait 0 (geo_diff N0.3 : `svg{w/h:100%}` des
    //   icônes masthead YouTube → 0×0, `yt-icon-shape`/
    //   `#logo`/`#content` à h=0). `height` résolue garde la
    //   dérivation de ratio (B-38 : `height:28px` + bitmap
    //   5839×745 contribue ~220, pas 5839).
    // Un non-remplacé retombe sur son contenu (`bx.intrinsic`
    // absent → `None` → mesure des enfants). `%` **nu**
    // seulement (`width:100%`) — une fonction math contenant
    // un `%` (`min(300px, 100%)` — le hint UA des `<svg
    // viewBox>` sans attr) garde une borne définie :
    // `bound()` l'évalue contre `cb` quand elle est définie,
    // l'intrinsèque/ratio reprend sinon — la tuer à 0
    // écrasait tout ancêtre shrink-to-fit (logos svg w=0,
    // vols pluto/lichess).
    let pct = st.get("width").is_some_and(|v| {
        v.trim()
            .strip_suffix('%')
            .is_some_and(|n| n.trim().parse::<f32>().is_ok())
    });
    let base = if matches!(bx.kind, BoxKind::Text | BoxKind::AnonymousBlock) {
        // Texte et bloc anonyme clonent le style du parent —
        // leur `width` « hérité » n'est pas une taille de
        // contenu (les pseudo-éléments gardent `Block`/`Inline`
        // et leur `width` propre).
        None
    } else if ignore_w {
        // Même sous `ignore_w`, un remplacé à `height`
        // résolue contribue par son ratio (§10.6.2), pas par
        // sa largeur brute — le `width` ignoré n'invalide
        // pas la hauteur spécifiée.
        bx.intrinsic
            .map(|(iw, ih)| bound_v("height").map_or(iw, |h| h * iw / ih))
    } else if pct && bx.intrinsic.is_some() {
        if max {
            bx.intrinsic
                .map(|(iw, ih)| bound_v("height").map_or(iw, |h| h * iw / ih))
        } else {
            Some(0.0)
        }
    } else {
        bound("width")
            .or_else(|| {
                // §10.6.2 en mesure intrinsèque : remplacé à
                // `height` résolue → la contribution suit le
                // ratio (h × iw/ih), pas la largeur brute
                // (§B-38 : logo svg intrinsèque 5839×745 +
                // `height:28px` mesurait 5839 — l'item flex
                // le voisin devenait irrétrécissable).
                bx.intrinsic
                    .and_then(|(iw, ih)| bound_v("height").map(|h| h * iw / ih))
            })
            .or(bx.intrinsic.map(|i| i.0))
    };
    if let Some(mut w) = base {
        if let Some(mx) = max_b {
            w = w.min(mx);
        }
        if let Some(mn) = min_b {
            w = w.max(mn);
        }
        return w;
    }
    // R-274 — `container-type: size|inline-size` = containment
    // de taille sur l'axe inline (css-contain-3 §2.2) : la
    // boîte se mesure « comme vide » — le contenu ne contribue
    // pas à sa taille intrinsèque. Le `width` direct plus haut
    // fait déjà foi ; inerte sur les boîtes de niveau inline
    // (spec : containment sur principale non-inline seule).
    if size_contained(&bx.style, bx) {
        return 0.0;
    }
    // Boîte texte : mesure de la fonte réelle.
    if bx.kind == BoxKind::Text {
        if let Some(t) = &bx.text {
            return text_px(&bx.style, t, fs, ctx, max);
        }
        return 0.0;
    }
    // Conteneur flex en axe horizontal (§9.9) : les items
    // s'alignent sur le main axis → max-content = somme d'une
    // ligne ; min-content d'un conteneur `wrap` = plus grand
    // item (chacun peut tenir sa propre ligne), `nowrap` =
    // somme également (aucune coupure possible).
    if matches!(bx.display(), "flex" | "inline-flex") {
        let dir = bx.style.get("flex-direction").unwrap_or("row");
        if matches!(dir.trim(), "row" | "row-reverse") {
            let wrap = bx
                .style
                .get("flex-wrap")
                .is_some_and(|v| v.trim() != "nowrap");
            let items: Vec<f32> = bx
                .children
                .iter()
                .filter(|c| {
                    // Hors flux (`absolute`/`fixed`) : aucune
                    // contribution — voir la boucle bloc plus bas
                    // (§4.1 / B-49 ; vimeo : `a.inline-flex >
                    // div.absolute.w-full` gonflait le `li`
                    // item à ~la ligne → nav écrasée, vol 772).
                    !c.out_of_flow()
                })
                .map(|c| {
                    let cfs = font_size_px(&c.style, fs, ctx);
                    explorer_common::deep(|| content_px_opt(c, cfs, ctx, max, false, cb))
                        + own_sides(c, fs, ctx)
                })
                .collect();
            if max || !wrap {
                return items.iter().sum();
            }
            return items.iter().fold(0.0_f32, |a, &b| a.max(b));
        }
    }
    // Ligne de table = somme des cellules (colonnes).
    // Chaque arête ∝ profondeur de l'arbre passe par
    // `explorer_common::deep` (R-24).
    if bx.display() == "table-row" {
        return bx
            .children
            .iter()
            .map(|c| {
                explorer_common::deep(|| {
                    content_px_opt(c, font_size_px(&c.style, fs, ctx), ctx, max, false, cb)
                }) + own_sides(c, fs, ctx)
            })
            .sum();
    }
    // Conteneur : enfants en-ligne s'additionnent (max) ou se
    // maximisent par morceau (min) ; enfants bloc = max.
    let mut best = 0.0f32;
    let mut run = 0.0f32;
    for c in &bx.children {
        // Hors flux (`absolute`/`fixed`) : aucune contribution
        // intrinsèque — la taille du conteneur ne doit pas
        // dépendre d'une boîte qui n'y coule pas (§4.1 /
        // B-49 : `.skip-links` absolu `width:100%` gonflait
        // la mesure du parent flex).
        if c.out_of_flow() {
            continue;
        }
        let cfs = font_size_px(&c.style, fs, ctx);
        if inline_level(c) {
            // En-ligne atomique (`inline-block`, remplacé) :
            // la contribution est la boîte de marge entière
            // (Sizing §4.2) — châssis compris, comme les
            // branches flottant/bloc ci-dessous. Sans lui,
            // le max-content d'un parent `padding` se mesure
            // court → enroulement de ligne dans les
            // shrink-to-fit (liens nav google.com §B-64).
            // Les `AnonymousBlock` clonent le style parent
            // (padding et display inclus) : leur `own_sides`
            // compte déjà le châssis du vrai parent ou serait
            // compté en double — exclus.
            let chassis = if !matches!(c.kind, BoxKind::AnonymousBlock)
                && (c.display() == "inline-block" || c.intrinsic.is_some())
            {
                own_sides(c, fs, ctx)
            } else {
                0.0
            };
            let w = explorer_common::deep(|| content_px_opt(c, cfs, ctx, max, false, cb)) + chassis;
            if max {
                run += w;
            } else {
                run = run.max(w);
            }
        } else if matches!(c.style.get("float"), Some("left" | "right")) {
            // Enfant flottant : à max-content il se tasse en
            // ligne avec ses voisins flottants — la même
            // contribution de « run » que l'en-ligne (un bloc
            // en-flux clôt la suite), **châssis compris**
            // (§4.2 : la contribution inclut marges, padding
            // et bordures). Le style est lu ici : `c.float`
            // n'est posé que lors de la pose de l'enfant.
            let w = explorer_common::deep(|| content_px_opt(c, cfs, ctx, max, false, cb))
                + own_sides(c, fs, ctx);
            if max {
                run += w;
            } else {
                run = run.max(w);
            }
        } else {
            best = best.max(run);
            run = 0.0;
            best = best.max(
                explorer_common::deep(|| content_px_opt(c, cfs, ctx, max, false, cb))
                    + own_sides(c, fs, ctx),
            );
        }
    }
    best.max(run)
}

/// `true` = enfant qui coule en ligne (texte, inline,
/// remplacé) — regroupé en « run » pour la mesure.
fn inline_level(c: &LayoutBox) -> bool {
    c.kind.is_inline_level() || c.display() == "inline-block" || c.intrinsic.is_some()
}

/// Mesure `text` : max = chaîne entière (espaces effondrés,
/// §16.6.1) ; min = le mot le plus long.
fn text_px(st: &ComputedStyle, text: &str, fs: f32, ctx: &BlockCtx, max: bool) -> f32 {
    use crate::inline::{ws_mode, WsCollapse};
    let measure = measure_fn(st, fs, ctx);
    let (ws_collapse, _wrap) = ws_mode(st);
    // `pre`/`pre-wrap`/`break-spaces` : §16.6.1 hors jeu —
    // lignes littérales et tabulations aux taquets `tab-size`
    // (§8.1, avance au prochain multiple depuis le début de
    // la ligne). `preserve-breaks` garde les sauts durs mais
    // effondre espaces/tabs par ligne ; `preserve-spaces`
    // convertit tabs/sauts en espaces verbatim ;
    // `discard` supprime tout blanc (texte soudé).
    if matches!(ws_collapse, WsCollapse::Preserve | WsCollapse::BreakSpaces) {
        let tab_w = crate::inline::tab_size_px(st, measure(" "), fs, ctx);
        let line_w = |line: &str| -> f32 {
            let mut pos = 0.0_f32;
            for (i, seg) in line.split('\t').enumerate() {
                if i > 0 && tab_w > f32::EPSILON {
                    pos += tab_w - pos.rem_euclid(tab_w);
                }
                pos += measure(seg);
            }
            pos
        };
        if max {
            return text.split('\n').map(line_w).fold(0.0, f32::max);
        }
        // `pre-wrap`/`break-spaces` coupent aux espaces et
        // après tabulation (UAX #14 BA — `break-spaces`
        // casse même en pleine suite d'espaces) — min = mot
        // le plus long ou un taquet.
        return text
            .split('\n')
            .flat_map(|l| l.split([' ', '\t']))
            .filter(|w| !w.is_empty())
            .map(&measure)
            .fold(0.0, f32::max)
            .max(if text.contains('\t') { tab_w } else { 0.0 });
    }
    if ws_collapse == WsCollapse::PreserveBreaks {
        if max {
            return text
                .split('\n')
                .map(|l| measure(&crate::inline::collapse_ws(l)))
                .fold(0.0, f32::max);
        }
        return text
            .split(crate::text::is_css_ws)
            .filter(|w| !w.is_empty())
            .map(&measure)
            .fold(0.0, f32::max);
    }
    if ws_collapse == WsCollapse::PreserveSpaces {
        let verbatim = text
            .chars()
            .map(|c| if crate::text::is_css_ws(c) { ' ' } else { c })
            .collect::<String>();
        if max {
            return measure(&verbatim);
        }
        return verbatim
            .split(' ')
            .filter(|w| !w.is_empty())
            .map(&measure)
            .fold(0.0, f32::max);
    }
    if ws_collapse == WsCollapse::Discard {
        let welded: String = text
            .chars()
            .filter(|c| !crate::text::is_css_ws(*c))
            .collect();
        return measure(&welded);
    }
    if max {
        return measure(&crate::inline::collapse_ws(text));
    }
    text.split(crate::text::is_css_ws)
        .filter(|w| !w.is_empty())
        .map(&measure)
        .fold(0.0, f32::max)
}

/// Marges + padding + bordures horizontales propres d'un
/// enfant — comptées dans l'intrinsèque du parent (la boîte
/// de marge déborde la content box du conteneur).
/// Un `AnonymousBlock` clone le style de son parent —
/// padding, marges et display compris : son « propre »
/// châssis n'existe pas ; le compter reviendrait à mesurer
/// deux fois le châssis du conteneur (§B-64 — un
/// `padding-right` parent gonflait la mesure de sa propre
/// boîte via le clone anonyme).
pub(crate) fn own_sides(c: &LayoutBox, fs: f32, ctx: &BlockCtx) -> f32 {
    if matches!(c.kind, BoxKind::AnonymousBlock) {
        return 0.0;
    }
    let st = &c.style;
    let cfs = font_size_px(st, fs, ctx);
    let mut acc = 0.0f32;
    for prop in [
        "margin-left",
        "margin-right",
        "padding-left",
        "padding-right",
        "border-left-width",
        "border-right-width",
    ] {
        acc += side_px(st, prop, 0.0, cfs, ctx, 0.0);
    }
    acc
}
