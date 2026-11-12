// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `<svg>` **inline** (R-9) : le sous-arbre DOM étranger est
//! re-sérialisé en source SVG autonome puis rasterisé via
//! `usvg`+`resvg` — même machine que le `<img src=*.svg>`
//! d'étape 45, même bornes. Arbitrage documenté dans la
//! roadmap : les display items dédiés (parser `d`, paints,
//! gradients…) auraient dupliqué un moteur SVG entier ; la
//! sérialisation réutilise le parseur conforme déjà
//! embarqué, pour un coût borné par [`SVG_MAX_SOURCE`] et
//! `max_px`.
//!
//! Limites assumées : rendu **statique** (SMIL et scripts
//! SVG ignorés), `<foreignObject>` refusé par `usvg` → zone
//! vide dégradée ; le bitmap est produit à la taille
//! naturelle (bornée `max_px`) puis mis à l'échelle nearest
//! au blit — le re-raster à la taille exacte×DPI est un
//! chantier ultérieur (dirty-rects R-7 le permet déjà).

use std::fmt::Write as _;
use std::sync::Arc;

use explorer_css::StyleMap;
use explorer_dom::{Dom, Namespace, NodeData, NodeId};

use crate::image::ImageTable;

/// Borne mémoire sur la source SVG re-sérialisée : au-delà le
/// sous-arbre est déclaré invalide (rendu vide) — un DOM
/// hostile peut produire un arbre énorme ; `usvg` borne déjà
/// le *temps* de parse, cette borne couvre la *taille*.
pub const SVG_MAX_SOURCE: usize = 4 * 1024 * 1024;

/// Rasterise les éléments `<svg>` du sous-arbre `root` —
/// `NodeId` → bitmap, à brancher dans [`crate::PaintImages`]
/// (champ `svgs`). La taille de chaque bitmap est sa taille
/// naturelle (`width`/`height`/`viewBox`), bornée à `max_px`
/// pixels — même budget que [`crate::decode_image`].
/// Les sous-arbres invalides sont ignorés (dégradation : la
/// boîte reste remplacée, image absente → fond/bordure
/// peints, comme un `<img>` cassé).
#[must_use]
pub fn collect_svg(dom: &Dom, root: NodeId, max_px: u32) -> ImageTable {
    collect_svg_with_sources(dom, root, max_px).0
}

/// [`collect_svg`] + la source sérialisée de chaque `<svg>`
/// (`NodeId` → XML, R-133) : le bitmap pré-rasterisé est à la
/// taille naturelle ; la display list re-rasterise la source à
/// la taille utilisée quand la boîte est plus grande (§B-32 —
/// icône 24 px peinte 130 px = blocs, sinon).
#[must_use]
pub fn collect_svg_with_sources(
    dom: &Dom,
    root: NodeId,
    max_px: u32,
) -> (ImageTable, std::collections::HashMap<NodeId, Arc<str>>) {
    collect_impl(dom, root, None, max_px)
}

/// [`collect_svg_with_sources`] **style-aware** (R-283) : la
/// sérialisation projette le style calculé de chaque élément
/// SVG en attributs XML — les déclarations CSS auteur
/// (`svg path { fill: red }`), le `color` hérité du contexte
/// HTML (résolution `currentColor` — icônes de design
/// systems), `display`/`visibility`, les propriétés
/// géométriques SVG2 (`d`, `cx`, `r`, `x`…) et les refs
/// `clip-path`/`mask`/`filter` issues du style. Les valeurs
/// calculées priment l'attribut de présentation (rang UA) —
/// conforme cascade. `styles` incomplet/absent sur un nœud =
/// attributs seuls (dégradé = comportement R-9).
#[must_use]
pub fn collect_svg_styled(
    dom: &Dom,
    root: NodeId,
    styles: &StyleMap,
    max_px: u32,
) -> (ImageTable, std::collections::HashMap<NodeId, Arc<str>>) {
    collect_impl(dom, root, Some(styles), max_px)
}

/// Moteur commun de [`collect_svg_with_sources`] et
/// [`collect_svg_styled`] — `styles` optionnel (la variante
/// historique n'en a pas).
fn collect_impl(
    dom: &Dom,
    root: NodeId,
    styles: Option<&StyleMap>,
    max_px: u32,
) -> (ImageTable, std::collections::HashMap<NodeId, Arc<str>>) {
    let mut out = ImageTable::new();
    let mut sources = std::collections::HashMap::new();
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        if let Ok(NodeData::Element(_)) = dom.data(n) {
            if is_svg_root(dom, n) {
                let mut src = String::new();
                serialize_into(dom, n, styles, &mut src);
                // `<use href="#x">`/`url(#x)` cross-îlots (issue
                // cosmétique synthèse — logo reddit aplati) :
                // la cible vit souvent dans un autre îlot
                // (`<symbol>` en début de document) — sans elle
                // usvg rend l'usage vide. On l'injecte dans
                // `<defs>` avant `</svg>` pour la résolution.
                inject_external_refs(dom, n, styles, &mut src);
                // P3 — `xlink:href` survivant sans déclaration :
                // HTML range `xlink:href` dans les attrs mais ne
                // porte pas l'`xmlns` — le XML sérialisé est
                // invalide pour usvg (namespace non déclaré) et
                // TOUT l'îlot tombait (pas seulement la ref).
                if src.contains("xlink:") && !src.contains("xmlns:xlink") {
                    if let Some(i) = src.find("<svg") {
                        if let Some(gt) = src[i..].find('>') {
                            src.insert_str(i + gt, " xmlns:xlink=\"http://www.w3.org/1999/xlink\"");
                        }
                    }
                }
                if src.len() <= SVG_MAX_SOURCE {
                    sources.insert(n, Arc::<str>::from(src.as_str()));
                    if let Some(img) = crate::image::decode_svg_raster(src.as_bytes(), max_px) {
                        out.insert(n, Arc::new(img));
                    }
                }
                continue; // sous-arbre SVG : atomique, jamais de HTML dedans
            }
            if let Ok(children) = dom.children(n) {
                stack.extend(children.iter().rev());
            }
        }
    }
    (out, sources)
}

/// `true` si `n` est l'élément `<svg>` dans le namespace SVG
/// (la racine d'un îlot étranger — les `svg` imbriqués sont
/// rasterisés *par* leur racine, pas collectés seuls).
fn is_svg_root(dom: &Dom, n: NodeId) -> bool {
    dom.tag_name(n).is_ok_and(|t| t == "svg")
        && dom.namespace(n).is_ok_and(|ns| ns == Namespace::Svg)
}

/// Tailles intrinsèques des `<svg>` du sous-arbre, pour
/// `build_box_tree_sized` : attributs `width`/`height` (px
/// ou nombre nu ; `%` sans repère → ignoré), sinon aspect du
/// `viewBox`, sinon défaut CSS des remplacés **300×150**.
/// Les `svg` imbriqués ne sont pas listés (atomiques dans
/// leur racine).
#[must_use]
pub fn svg_sizes(dom: &Dom, root: NodeId) -> std::collections::HashMap<NodeId, (f32, f32)> {
    let mut out = std::collections::HashMap::new();
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        if let Ok(NodeData::Element(_)) = dom.data(n) {
            if is_svg_root(dom, n) {
                out.insert(n, svg_intrinsic(dom, n));
                continue;
            }
            if let Ok(children) = dom.children(n) {
                stack.extend(children.iter().rev());
            }
        }
    }
    out
}

/// Taille intrinsèque d'un `<svg>` : attrs > viewBox > 300×150.
fn svg_intrinsic(dom: &Dom, n: NodeId) -> (f32, f32) {
    let attr_px = |name: &str| {
        dom.attribute_str(n, name)
            .ok()
            .flatten()
            .and_then(parse_svg_length)
    };
    let (w, h) = (attr_px("width"), attr_px("height"));
    let vb = dom
        .attribute_str(n, "viewBox")
        .ok()
        .flatten()
        .and_then(parse_viewbox);
    match (w, h, vb) {
        (Some(w), Some(h), _) => (w, h),
        (Some(w), None, Some((_, _, vw, vh))) if vw > 0.0 => (w, w * vh / vw),
        (None, Some(h), Some((_, _, vw, vh))) if vh > 0.0 => (h * vw / vh, h),
        (None, None, Some((_, _, vw, vh))) if vw > 0.0 && vh > 0.0 => (vw, vh),
        (Some(w), None, _) => (w, 150.0),
        (None, Some(h), _) => (300.0, h),
        _ => (300.0, 150.0),
    }
}

/// Longueur SVG `42`/`42px` — `%` refusé (pas de repère à
/// ce stade), borné aux valeurs finies positives.
fn parse_svg_length(v: &str) -> Option<f32> {
    let t = v.trim();
    if t.ends_with('%') {
        return None;
    }
    t.trim_end_matches("px")
        .parse::<f32>()
        .ok()
        .filter(|f| f.is_finite() && *f > 0.0)
}

/// `viewBox="minX minY w h"` — séparateurs espace/virgule.
fn parse_viewbox(v: &str) -> Option<(f32, f32, f32, f32)> {
    let nums: Vec<f32> = v
        .split([',', ' ', '\t'])
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse().ok())
        .collect();
    (nums.len() == 4).then(|| (nums[0], nums[1], nums[2], nums[3]))
}

/// Propriétés projetées dans la sérialisation style-aware
/// (R-283) : peinture SVG + mécanismes d'affichage. La
/// valeur calculée est émise comme attribut de même nom
/// (SVG2 : ces propriétés sont toutes des attributs de
/// présentation) — sauf exceptions listées en commentaire.
/// `display`/`visibility` n'émettent que leurs valeurs
/// censurantes ; `transform` est re-syntaxé (unités CSS →
/// nombres SVG) ; `clip-path`/`mask`/`filter` à `none`
/// suppriment l'attribut DOM.
const SVG_STYLE_PROPS: &[&str] = &[
    "color",
    "fill",
    "fill-opacity",
    "fill-rule",
    "stroke",
    "stroke-width",
    "stroke-opacity",
    "stroke-dasharray",
    "stroke-dashoffset",
    "stroke-linecap",
    "stroke-linejoin",
    "stroke-miterlimit",
    "paint-order",
    "vector-effect",
    "opacity",
    "display",
    "visibility",
    "stop-color",
    "stop-opacity",
    "flood-color",
    "flood-opacity",
    "lighting-color",
    "clip-path",
    "clip-rule",
    "mask",
    "mask-type",
    "filter",
    "marker",
    "marker-start",
    "marker-mid",
    "marker-end",
    "color-interpolation",
    "color-interpolation-filters",
    "shape-rendering",
    "text-rendering",
    "image-rendering",
    "overflow",
    "font-family",
    "font-size",
    "font-weight",
    "font-style",
    "letter-spacing",
    "text-anchor",
    "dominant-baseline",
    "alignment-baseline",
];

/// Propriétés géométriques SVG2 projetées sur les **enfants**
/// (la racine est dimensionnée par le layout HTML — son
/// `width`/`height` CSS n'a pas vocation à re-entrer dans la
/// source, où `100%` deviendrait relatif au viewBox).
const SVG_GEOM_PROPS: &[&str] = &[
    "x", "y", "x1", "y1", "x2", "y2", "cx", "cy", "r", "rx", "ry", "width", "height",
];

/// Sérialise le sous-arbre `node` en source SVG — un seul
/// document autonome : la racine reçoit `xmlns` si absente.
/// `styles` (R-283, optionnel) projette le style calculé en
/// attributs : la valeur calculée **prime** l'attribut de
/// présentation de même nom (rang UA — la déclaration auteur
/// gagne), `transform` est re-syntaxée, `d:path(…)` est
/// désenrobée. Échappement XML des textes et attributs ;
/// commentaires ignorés (non significatifs pour le rendu).
/// Projection style → attributs XML pour un élément
/// (R-283) : `nom → valeur` (`None` = supprimer l'attribut
/// DOM — CSS `none` sur `clip-path`/`mask`/`filter`/
/// `transform`). Les filtres `display`/`visibility`/
/// `overflow` n'émettent que leurs valeurs censurantes.
fn project_style(
    st: Option<&explorer_css::ComputedStyle>,
    is_root_svg: bool,
) -> Vec<(&'static str, Option<String>)> {
    let mut projected: Vec<(&'static str, Option<String>)> = Vec::new();
    let Some(st) = st else {
        return projected;
    };
    for &p in SVG_STYLE_PROPS {
        if let Some(v) = st.get(p) {
            let v = v.trim();
            match p {
                "display" => {
                    if v == "none" {
                        projected.push((p, Some(v.to_string())));
                    }
                }
                "visibility" => {
                    if matches!(v, "hidden" | "collapse") {
                        projected.push((p, Some(v.to_string())));
                    }
                }
                "clip-path" | "mask" | "filter" => {
                    // `none` = retirer l'effet → supprimer
                    // l'attribut DOM s'il existe.
                    projected.push((p, (v != "none").then(|| v.to_string())));
                }
                "marker" => {
                    // Shorthand — pas d'attribut `marker` :
                    // les trois longhands reçoivent la valeur.
                    for m in ["marker-start", "marker-mid", "marker-end"] {
                        projected.push((m, Some(v.to_string())));
                    }
                }
                "overflow" => {
                    // SVG2 §14.3.3 : sur svg/symbol/image/
                    // marker/pattern `scroll`/`auto` ≡ `hidden`.
                    if !matches!(v, "visible" | "initial" | "unset" | "inherit") {
                        projected.push(("overflow", Some("hidden".to_string())));
                    }
                }
                _ => projected.push((p, Some(v.to_string()))),
            }
        }
    }
    if let Some(v) = st.get("transform") {
        let v = v.trim();
        projected.push((
            "transform",
            (v != "none" && v != "initial").then(|| svg_transform(v)),
        ));
    }
    if !is_root_svg {
        for &p in SVG_GEOM_PROPS {
            if let Some(v) = st.get(p) {
                projected.push((p, Some(v.trim().to_string())));
            }
        }
        if let Some(v) = st.get("d") {
            if let Some(d) = css_path(v.trim()) {
                projected.push(("d", Some(d)));
            }
        }
    }
    projected
}

fn serialize_into(dom: &Dom, node: NodeId, styles: Option<&StyleMap>, out: &mut String) {
    match dom.data(node) {
        Ok(NodeData::Element(e)) => {
            let st = styles.and_then(|m| m.get(node));
            let is_root_svg = e.namespace == Namespace::Svg && e.tag_name == "svg";
            out.push('<');
            out.push_str(&e.tag_name);
            let mut has_xmlns = false;
            // Attributs projetés : nom → valeur de remplacement
            // (`None` = supprimer l'attribut DOM — CSS `none`).
            let mut projected = project_style(st, is_root_svg);
            for a in &e.attributes {
                // La valeur calculée remplace l'attribut de
                // présentation de même nom (rang UA) — ou le
                // supprime (`transform:none`, `clip-path:none`…).
                let name = a.name.as_str();
                // Cas `style` : dans la cascade usvg, l'attribut
                // `style` bat les attributs de présentation — un
                // `style="fill:blue"` DOM écraserait le `fill`
                // calculé projeté (les exports SVG en sont
                // pleins). On retire donc de la déclaration
                // sérialisée toutes les propriétés projetées —
                // leur valeur calculée (qui englobe déjà la
                // déclaration inline) vit dans l'attribut émis.
                let val = if name == "style" {
                    let kept = strip_style_decls(&a.value, &projected);
                    (!kept.is_empty()).then_some(kept)
                } else {
                    match projected.iter().position(|(n, _)| *n == name) {
                        Some(i) => projected.remove(i).1,
                        None => Some(a.value.clone()),
                    }
                };
                if let Some(v) = val {
                    let _ = write!(out, " {name}=\"");
                    escape_xml(&v, out, true);
                    out.push('"');
                    has_xmlns |= name == "xmlns";
                }
            }
            for (name, val) in projected {
                if let Some(v) = val {
                    let _ = write!(out, " {name}=\"");
                    escape_xml(&v, out, true);
                    out.push('"');
                }
            }
            if !has_xmlns && is_root_svg {
                out.push_str(" xmlns=\"http://www.w3.org/2000/svg\"");
            }
            let mut children = dom.children_iter(node).into_iter().flatten().peekable();
            if children.peek().is_none() {
                out.push_str("/>");
            } else {
                out.push('>');
                for c in children {
                    // Arête ∝ profondeur DOM (R-24).
                    explorer_common::deep(|| serialize_into(dom, c, styles, out));
                }
                let _ = write!(out, "</{}>", e.tag_name);
            }
        }
        Ok(NodeData::Text(t)) => escape_xml(dom.text_at(*t), out, false),
        _ => {}
    }
}

/// Retire d'une valeur d'attribut `style` les déclarations
/// dont la propriété est projetée en attribut — le reste est
/// conservé tel quel (propriétés non couvertes par la
/// projection restent visibles pour `usvg`).
fn strip_style_decls(v: &str, projected: &[(&str, Option<String>)]) -> String {
    v.split(';')
        .filter(|decl| {
            let name = decl
                .split(':')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            name.is_empty() || !projected.iter().any(|(p, _)| *p == name)
        })
        .collect::<Vec<_>>()
        .join(";")
}

/// CSS `transform` → syntaxe d'attribut SVG : `px`/`deg` nus
/// (le parseur d'attribut `usvg` attend des nombres ; `deg`
/// est l'unité implicite), `none` déjà filtré en amont.
fn svg_transform(v: &str) -> String {
    v.replace("px", "").replace("deg", "")
}

/// `d: path("M …")` (valeur CSS) → donnée `d` brute —
/// l'attribut attend la séquence nue, sans `path(…)`.
fn css_path(v: &str) -> Option<String> {
    let inner = v.strip_prefix("path(")?.strip_suffix(')')?;
    let inner = inner.trim().trim_matches('"').trim_matches('\'').trim();
    (!inner.is_empty()).then(|| inner.to_string())
}

/// Échappement XML — `<`/`&` obligatoires, `>` et `"` en
/// attribut, `'` optionnel (guillemets doubles utilisés).
fn escape_xml(s: &str, out: &mut String, attr: bool) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if attr => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
}

/// Références `#id`/`url(#id)` d'un îlot résolues depuis le
/// document entier (cross-îlot — les `<symbol>` partagés
/// vivent souvent dans un `<svg>` caché ailleurs). Chaque
/// cible trouvée hors de l'îlot est sérialisée dans un
/// `<defs>` injecté avant `</svg>` ; les refs introuvables
/// ou auto-référencées sont ignorées (rendu dégradé usvg).
fn inject_external_refs(dom: &Dom, svg: NodeId, styles: Option<&StyleMap>, src: &mut String) {
    let mut needed = Vec::<String>::new();
    let mut defined = Vec::<String>::new();
    let mut stack = vec![svg];
    while let Some(n) = stack.pop() {
        let Ok(e) = dom.element(n) else { continue };
        for a in &e.attributes {
            if a.name == "id" {
                defined.push(a.value.clone());
            } else if let Some(id) = a.value.strip_prefix('#') {
                if matches!(a.name.as_str(), "href" | "xlink:href") {
                    needed.push(id.to_string());
                }
            } else {
                // `url(#x)` quelque part dans la valeur —
                // fill/stroke/clip/filter peuvent en porter
                // plusieurs (P3 — seul le premier était lu).
                let mut rest = a.value.as_str();
                while let Some(i) = rest.find("url(#") {
                    rest = &rest[i + 5..];
                    if let Some(end) = rest.find(')') {
                        needed.push(rest[..end].to_string());
                        rest = &rest[end..];
                    } else {
                        break;
                    }
                }
            }
        }
        if let Ok(ch) = dom.children(n) {
            stack.extend(ch.iter().rev());
        }
    }
    // R-283 — les attributs **émis** depuis le style calculé
    // (`clip-path:url(#x)` projeté en CSS, `fill:url(#g)`…)
    // ne sont pas dans le scan DOM : re-scanner la source.
    {
        let mut rest = src.as_str();
        while let Some(i) = rest.find("url(#") {
            rest = &rest[i + 5..];
            if let Some(end) = rest.find([')', '"', '\'']) {
                needed.push(rest[..end].to_string());
                rest = &rest[end..];
            } else {
                break;
            }
        }
    }
    // P3 — fixpoint borné : la cible injectée (`<symbol>`)
    // peut référencer d'AUTRES ids externes (`<use>` dans un
    // `<symbol>`, `fill="url(#g)"`…) — une seule passe les
    // laissait non résolus (îlot rendu vide quand même).
    // 8 passes suffisent aux chaînes réelles ; chaque cible
    // injectée joint `defined` (pas de réinjection).
    let mut injected = Vec::<NodeId>::new();
    for _ in 0..8 {
        let mut defs = String::new();
        let mut progress = false;
        for id in &needed {
            if defined.contains(id) {
                continue;
            }
            if let Some(target) = dom.by_id(id) {
                serialize_into(dom, target, styles, &mut defs);
                defined.push(id.clone());
                injected.push(target);
                progress = true;
            }
        }
        if !progress {
            break;
        }
        // Rescan des cibles injectées pour leurs refs propres.
        let mut st = std::mem::take(&mut injected);
        while let Some(n) = st.pop() {
            let Ok(e) = dom.element(n) else { continue };
            for a in &e.attributes {
                if let Some(id) = a.value.strip_prefix('#') {
                    if matches!(a.name.as_str(), "href" | "xlink:href") {
                        needed.push(id.to_string());
                    }
                } else if let Some(i) = a.value.find("url(#") {
                    let rest = &a.value[i + 5..];
                    if let Some(end) = rest.find(')') {
                        needed.push(rest[..end].to_string());
                    }
                }
            }
            if let Ok(ch) = dom.children(n) {
                st.extend(ch.iter().rev());
            }
        }
        let close = "</svg>".to_string();
        if let Some(i) = src.rfind(&close) {
            let defs_tag = format!("<defs>{defs}</defs>");
            src.insert_str(i, &defs_tag);
        }
    }
}
