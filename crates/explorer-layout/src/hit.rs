// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Hit-test sur l'arbre de boîtes (R-19).
//!
//! L'élagage géométrique `point ∈ rect` est **faux** dès
//! qu'un enfant déborde son parent (repositionnements,
//! marges négatives, `overflow: visible`, centrage bloc) :
//! le sous-arbre peint peut excéder le border-box du parent
//! — c'est le bug terrain lite.ddg (`<center>` pose son
//! `<form>` hors de son rect → zones cliquables mortes).
//!
//! La bonne borne d'élagage est `paint_bounds` : l'union des
//! rects du sous-arbre (ce que la peinture couvre
//! réellement). Elle reste conservative — un point hors de
//! l'union ne peut toucher aucun descendant — mais inclut
//! les enfants échappés. `hit_test` conserve la sémantique
//! historique : la boîte la plus **imbriquée** dont le rect
//! propre contient le point gagne ; son `NodeId` remonte au
//! session (`None` pour les boîtes anonymes — l'appelant
//! remonte aux ancêtres DOM).

use explorer_dom::NodeId;

use crate::boxtree::{LayoutBox, Rect};
use crate::transform::{box_transform, Affine};

/// Union des rects du sous-arbre **dans le repère de `bx`**
/// (avant sa propre transform, les transforms des
/// descendants déjà appliquées) — la zone que la peinture
/// peut couvrir (bornes de clip `overflow` ignorées :
/// l'union est une borne *conservative*, jamais
/// restrictive). Un enfant transformé peut déborder très
/// loin de son rect layout : son bound suit la matrice.
fn paint_bounds(bx: &LayoutBox) -> Rect {
    // Récursion ∝ profondeur de l'arbre → arêtes `deep` (R-24).
    bx.children.iter().fold(bx.rect, |acc, c| {
        let b = explorer_common::deep(|| paint_bounds(c));
        // Bounds de l'enfant exprimés dans le repère de `bx` :
        // sa propre `transform` s'applique à tout son
        // sous-arbre (même portée que `PushTransform`).
        let b = box_transform(c, &c.rect).map_or(b, |t| map_rect(t, b));
        union(acc, b)
    })
}

/// Bounding-box du rect `r` sous la matrice `t` (les quatre
/// coins transformés — la rotation élargit, comme le
/// raster).
fn map_rect(t: Affine, r: Rect) -> Rect {
    let (x2, y2) = (r.x + r.width, r.y + r.height);
    let pts = [
        t.apply(r.x, r.y),
        t.apply(x2, r.y),
        t.apply(r.x, y2),
        t.apply(x2, y2),
    ];
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for (px, py) in pts {
        x0 = x0.min(px);
        y0 = y0.min(py);
        x1 = x1.max(px);
        y1 = y1.max(py);
    }
    Rect {
        x: x0,
        y: y0,
        width: x1 - x0,
        height: y1 - y0,
    }
}

fn union(a: Rect, b: Rect) -> Rect {
    let (x, y) = (a.x.min(b.x), a.y.min(b.y));
    let (x2, y2) = (
        (a.x + a.width).max(b.x + b.width),
        (a.y + a.height).max(b.y + b.height),
    );
    Rect {
        x,
        y,
        width: x2 - x,
        height: y2 - y,
    }
}

fn contains(r: Rect, x: f32, y: f32) -> bool {
    r.x <= x && x <= r.x + r.width && r.y <= y && y <= r.y + r.height
}

/// Hit-test profond : la boîte la plus imbriquée dont le
/// **rect propre** contient `(x, y)` document gagne — son
/// `NodeId` source (`None` pour les boîtes anonymes, le
/// repli parent étant laissé à l'appelant).
///
/// L'élagage se fait sur `paint_bounds` (union du
/// sous-arbre), pas sur le border-box : un enfant peint hors
/// du parent reste hit-testable (R-19). Les fragments de
/// lignes en-ligne ne sont pas descendus — parité avec la
/// version historique : l'élément inline porteur gagne.
/// `visibility: hidden|collapse` (style **calculé** — l'héritage
/// est déjà résolu : un descendant explicitement `visible` se
/// relit visible et reste ciblable, §11.2 — R-102). Un élément
/// caché n'est jamais la cible ; on descend quand même dans ses
/// enfants.
fn hidden(bx: &LayoutBox) -> bool {
    matches!(bx.style.get("visibility"), Some("hidden" | "collapse"))
}

/// La boîte ne peut pas être **cible** de hit-test : outre
/// `visibility`, `pointer-events: none` (vol 2026-10-26 —
/// les overlays auteur `pointer-events:none` volaient tous
/// les hits au-dessus des champs : `editable:false` et clic
/// `ignored` sur des `<input>` pourtant peints). Comme pour
/// `visibility`, on descend quand même : un enfant portant
/// `pointer-events: auto` explicite reste ciblable (sa
/// valeur calculée diffère de celle du parent).
fn unhittable(bx: &LayoutBox) -> bool {
    hidden(bx) || bx.style.get("pointer-events") == Some("none")
}

/// Rang de peinture d'une boîte au sein de son parent
/// (CSS 2.1 §9.9.1 / css-position §3.5 — ordre d'empilement
/// simplifié) : les éléments positionnés (`relative`,
/// `absolute`, `fixed`, `sticky`) passent **après** le flux
/// normal, les `z-index` négatifs avant, les positifs après
/// encore. Le hit-test doit frapper la couche la plus haute
/// en premier — un bandeau/overlays `fixed` transparent qui
/// précède le contenu dans le DOM est peint AU-DESSUS de
/// lui : tester les enfants en ordre de document lui
/// volait les clics des liens situés dessous (vol 19 :
/// `{"result":"ignored"}` sur liens valides de
/// gamefaqs/speedtest, « Français » de canada.ca).
fn paint_rank(c: &LayoutBox) -> i64 {
    let positioned = !matches!(
        c.kind,
        crate::BoxKind::Text | crate::BoxKind::AnonymousBlock
    ) && matches!(
        c.style.get("position"),
        Some("relative" | "absolute" | "fixed" | "sticky")
    );
    if !positioned {
        return 0;
    }
    #[allow(clippy::cast_possible_truncation)]
    let z = c
        .style
        .number("z-index")
        .map_or(i64::from(c.z_index), |v| v as i64);
    // Positionnés : z < 0 sous le flux, z = 0/auto au rang
    // central, z > 0 au-dessus de tout.
    1_000_000_000 + z.signum() * 1_000_000_000 + z.clamp(-999_999_999, 999_999_999)
}

/// Rang de peinture **au sommet** d'un sous-arbre : le rang
/// maximal atteint par un descendant positionné (§B-42).
/// Une couche `position:fixed`/`absolute` enfouie sous des
/// ancêtres en flux (bandeau de consentement DSFR sous un
/// wrapper statique en fin de document) est **peinte
/// au-dessus** des sous-arbres positionnés voisins — l'ordre
/// d'empilement se décide au `z-index`, pas à la profondeur
/// du porteur. Le rang local du porteur statique (0) laissait
/// ces overlays tester APRÈS les sous-arbres positionnés de
/// leur parent : clic « Tout accepter » culture.gouv résolu
/// sur l'ancre du mégamenu **sous** le bandeau. Approximation
/// assumée : les contextes d'empilement intermédiaires
/// (`transform`, `opacity < 1`, `filter`…) qui confineraient
/// le descendant ne sont pas modélisés.
fn subtree_rank(bx: &LayoutBox) -> i64 {
    bx.children.iter().fold(paint_rank(bx), |acc, c| {
        acc.max(explorer_common::deep(|| subtree_rank(c)))
    })
}

/// Indices des enfants dans l'ordre de **peinture inversé**
/// (la dernière couche peinte, la plus haute, testée en
/// premier) — à rang égal, le dernier du document gagne.
fn hit_order(bx: &LayoutBox) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..bx.children.len()).collect();
    idx.sort_by_key(|&i| subtree_rank(&bx.children[i]));
    idx.reverse();
    idx
}

#[must_use]
pub fn hit_test(bx: &LayoutBox, x: f32, y: f32) -> Option<NodeId> {
    hit_test_m(bx, x, y, Affine::IDENTITY)
}

/// Descente sous une matrice cumulative `m` (produit des
/// `transform` des ancêtres, origines résolues) : le point
/// sondé est ramené dans le repère **non transformé** de la
/// boîte par `m⁻¹` — les rects du layout restent exprimés
/// avant transform, comme la display list qui les émet sous
/// `PushTransform`. Une matrice non inversible (`scale(0)`)
/// aplatit le sous-arbre : rien n'y est cliquable.
fn hit_test_m(bx: &LayoutBox, x: f32, y: f32, m: Affine) -> Option<NodeId> {
    // La transform propre de la boîte s'applique à elle-même
    // ET à ses descendants (même portée que `PushTransform`).
    let m = box_transform(bx, &bx.rect).map_or(m, |t| t.post_concat(m));
    let (px, py) = m.invert()?.apply(x, y);
    if !contains(paint_bounds(bx), px, py) {
        return None;
    }
    for i in hit_order(bx) {
        let child = &bx.children[i];
        if let Some(n) = explorer_common::deep(|| hit_test_m(child, x, y, m)) {
            return Some(n);
        }
    }
    if unhittable(bx) {
        return None;
    }
    contains(bx.rect, px, py).then_some(bx.node).flatten()
}

/// Hit-test conscient du scroll (issue #18) : `y` est en
/// coordonnées **viewport** — `scroll_y` s'ajoute en
/// descendant, sauf sous `position:fixed` dont les boîtes
/// sont posées **contre le viewport** (leurs rects sont
/// déjà en coords viewport : le delta retombe à 0 pour tout
/// le sous-arbre — parité avec `build_display_list_scrolled`
/// qui « ré-initialise l'offset » sous `fixed`).
///
/// Sans ce filage, un élément fixed visuellement épinglé
/// n'encaissait plus rien dès `scroll_y > 0` (sonde `y+scroll`
/// sur un rect resté à `y≈0`) — nav github morte au hover.
/// `hit_test` reste la variante coords document (delta nul).
#[must_use]
pub fn hit_test_scrolled(bx: &LayoutBox, x: f32, y: f32, scroll_y: f32) -> Option<NodeId> {
    // Sans scrollport connu, les seuils `bottom`/`right` ne
    // peuvent évaluer `vp` — dégradé = offset nul sur cet
    // axe (comportement antérieur, pas de régression) ;
    // `top`/`left` restent exacts. Préférer
    // [`hit_test_scrolled_vp`] dès que le viewport est connu.
    hit_test_delta_m_vp(bx, x, y, scroll_y, (0.0, 0.0), Affine::IDENTITY)
}

/// [`hit_test_scrolled`] avec le scrollport `(w, h)` —
/// requis pour les sticky à seuil `bottom`/`right` (B-40) ;
/// identique sinon.
#[must_use]
pub fn hit_test_scrolled_vp(
    bx: &LayoutBox,
    x: f32,
    y: f32,
    scroll_y: f32,
    vp: (f32, f32),
) -> Option<NodeId> {
    hit_test_delta_m_vp(bx, x, y, scroll_y, vp, Affine::IDENTITY)
}

/// Variante scrollée de [`hit_test_m`] : `m` cumule les
/// `transform` d'ancêtres (inverse appliquée au point, même
/// parité que la display list — vol : hit décalé sur
/// élément transformé). `vp` = scrollport `(w, h)` pour les
/// seuils sticky `bottom`/`right`.
#[allow(clippy::too_many_arguments)]
fn hit_test_delta_m_vp(
    bx: &LayoutBox,
    x: f32,
    y_vp: f32,
    delta: f32,
    vp: (f32, f32),
    m: Affine,
) -> Option<NodeId> {
    let m = box_transform(bx, &bx.rect).map_or(m, |t| t.post_concat(m));
    let (px, py) = m.invert()?.apply(x, y_vp + delta);
    if !contains(paint_bounds(bx), px, py) {
        return None;
    }
    for i in hit_order(bx) {
        let child = &bx.children[i];
        // `position:fixed` = coords viewport : delta nul pour
        // tout le sous-arbre. Détection par le **style** (comme
        // `resolve_absolutes`) — le champ `position` n'est
        // renseigné que sur les boîtes de bloc.
        let d = if child.style.get("position") == Some("fixed") {
            0.0
        } else {
            delta
        };
        // `position:sticky` (B-40) : la display list translate
        // la boîte vers son seuil (`sticky_offset`) mais le
        // rect layout reste au doc-y naturel — un clic sur
        // l'élément épinglé le manquait (`ignored`) ou frappait
        // la couche dessous (`navigated` vers l'ancre
        // pleine-largeur — culture.gouv). Le point est
        // ramené dans le repère pré-décalage, comme `fixed`.
        let (sx, sy) = if child.position == crate::Position::Sticky {
            sticky_offset(child, (0.0, -delta), bx.rect, vp)
        } else {
            (0.0, 0.0)
        };
        if let Some(n) =
            explorer_common::deep(|| hit_test_delta_m_vp(child, x - sx, y_vp - sy, d, vp, m))
        {
            return Some(n);
        }
    }
    if unhittable(bx) {
        return None;
    }
    contains(bx.rect, px, py).then_some(bx.node).flatten()
}

/// Décalage `position: sticky` — **source unique** partagée
/// par la display list (`explorer-render`) et le hit-test
/// (B-40 : un header sticky épinglé au viewport restait
/// cliquable à son doc-y naturel alors qu'il est peint
/// décalé — clic `ignored`/`navigated` vers la cible
/// dessous, trainline/culture.gouv).
///
/// `off` = translation de scroll déjà appliquée `(x, y)`
/// (négative — un rect document + `off` = position
/// viewport), `bound` = border box du bloc conteneur en
/// coords document, `vp` = `(w, h)` du scrollport. Les
/// quatre côtés `top`/`bottom`/`left`/`right` ; sur-
/// contrainte spec : `top` gagne `bottom`, `left` gagne
/// `right` (LTR).
#[must_use]
pub fn sticky_offset(bx: &LayoutBox, off: (f32, f32), bound: Rect, vp: (f32, f32)) -> (f32, f32) {
    if bx.position != crate::Position::Sticky {
        return (0.0, 0.0);
    }
    let painted = shifted(bx.rect, off);
    let bound_p = shifted(bound, off);
    let dy = if let Some(thr) = thr_px(&bx.style, "top") {
        let room = (bound_p.y + bound_p.height - painted.y - painted.height).max(0.0);
        (thr - painted.y).clamp(0.0, room)
    } else if let Some(thr) = thr_px(&bx.style, "bottom") {
        let room = (painted.y - bound_p.y).max(0.0);
        -(painted.y + painted.height - (vp.1 - thr)).clamp(0.0, room)
    } else {
        0.0
    };
    let dx = if let Some(thr) = thr_px(&bx.style, "left") {
        let room = (bound_p.x + bound_p.width - painted.x - painted.width).max(0.0);
        (thr - painted.x).clamp(0.0, room)
    } else if let Some(thr) = thr_px(&bx.style, "right") {
        let room = (painted.x - bound_p.x).max(0.0);
        -(painted.x + painted.width - (vp.0 - thr)).clamp(0.0, room)
    } else {
        0.0
    };
    (dx, dy)
}

/// Seuil sticky `top`/`bottom`/`left`/`right` → px ;
/// `auto`/absent/`%`/`calc` → `None` (pas de seuil sur cet
/// axe). `em`/`rem` résolvent sur la `font-size` calculée
/// (px absolu en cascade — même convention que `prop_px`
/// côté render).
#[allow(clippy::cast_possible_truncation)]
fn thr_px(st: &explorer_css::ComputedStyle, prop: &str) -> Option<f32> {
    let raw = st.get(prop)?.trim();
    if raw.is_empty() || raw == "auto" {
        return None;
    }
    let l = explorer_css::value::parse_length(raw)?;
    Some(
        (match l.unit.px_factor() {
            Some(f) => l.value * f,
            None if raw.ends_with("em") => {
                let fs = st
                    .get("font-size")
                    .and_then(explorer_css::value::parse_length)
                    .and_then(|fl| fl.unit.px_factor().map(|f| fl.value * f))
                    .unwrap_or(16.0);
                l.value * fs
            }
            None => return None,
        }) as f32,
    )
}

fn shifted(r: Rect, off: (f32, f32)) -> Rect {
    Rect {
        x: r.x + off.0,
        y: r.y + off.1,
        ..r
    }
}
