// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Sonde §B-27 : mosaïque géante ouest-france (vol 48) —
//! `su-main-picture{width:fit-content}` + `su-media
//! {width:100%;height:100%}` dans une chaîne `su-row`/
//! `su-col`/`su-focus-big`. Reproduit la structure/classes
//! réelles pour isoler la source de la boîte géante.

use explorer_css::{ColorScheme, Origin, StyleEngine, StyleEnv};
use explorer_dom::Dom;
use explorer_html::TreeBuilder;
use explorer_layout::{build_box_tree_sized, layout_tree, BlockCtx, LayoutBox};

const VIEW: f32 = 1260.0;

fn dump(b: &LayoutBox, d: usize) {
    eprintln!(
        "{:indent$}{:?} [{:.2} {:.2} {:.2}x{:.2}]",
        "",
        b.kind,
        b.rect.x,
        b.rect.y,
        b.rect.width,
        b.rect.height,
        indent = d * 2
    );
    for c in &b.children {
        dump(c, d + 1);
    }
}

fn pipeline(html: &str, css: &str) -> (Dom, LayoutBox) {
    let dom = TreeBuilder::parse(html);
    let mut eng = StyleEngine::new();
    eng.add_css("body { margin: 0 }", Origin::Author);
    eng.add_css(css, Origin::Author);
    let styles = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let root_el = dom
        .children(dom.document())
        .into_iter()
        .flatten()
        .find(|&c| dom.element(c).is_ok())
        .expect("root");
    let mut sizes = std::collections::HashMap::new();
    let mut q = vec![dom.document()];
    while let Some(n) = q.pop() {
        if dom.tag_name(n).is_ok_and(|t| t == "img") {
            sizes.insert(n, (940.0_f32, 528.0_f32));
        }
        if let Ok(ch) = dom.children(n) {
            q.extend(ch.iter().rev());
        }
    }
    let mut bx = build_box_tree_sized(&dom, root_el, &styles, &sizes).expect("arbre");
    layout_tree(&mut bx, VIEW, &BlockCtx::default());
    (dom, bx)
}

fn widest(b: &LayoutBox, acc: &mut Vec<LayoutBox>) {
    acc.push(LayoutBox {
        kind: b.kind,
        node: b.node,
        style: b.style.clone(),
        text: b.text.clone(),
        children: Vec::new(),
        lines: Vec::new(),
        rect: b.rect,
        position: b.position,
        float: b.float,
        overflow: b.overflow,
        overflow_x: b.overflow_x,
        z_index: b.z_index,
        intrinsic: b.intrinsic,
        control_checked: b.control_checked,
        slot: b.slot,
        height_raised: b.height_raised,
        fc_root: b.fc_root,
        marker_ordinal: b.marker_ordinal,
    });
    for c in &b.children {
        widest(c, acc);
    }
}

fn css_base() -> &'static str {
    ".su-row{display:flex;flex-wrap:wrap;column-gap:20px}
     .su-col-lg-24{flex:0 0 100%;max-width:100%}
     .su-articles-list-item{display:flex;column-gap:16px;position:relative}
     .su-focus-big{display:flex;flex-flow:column nowrap;padding:16px}
     .su-focus-big .su-media-container{order:3;z-index:-1}
     .su-focus-big .su-title{order:1;font-size:36px}
     .su-focus-big .su-metadata{order:2;display:flex;column-gap:16px}
     .su-media-container{position:relative;margin-bottom:16px}
     .su-ratio-4-3{aspect-ratio:4/3}
     .su-ratio-16-7-md{aspect-ratio:16/7}
     .su-main-picture{position:relative;overflow:hidden;width:fit-content;box-sizing:border-box}
     .su-media{display:block;width:100%;height:100%;object-fit:cover;border-radius:8px}"
}

fn html_skeleton(img: &str) -> String {
    format!(
        "<section class=\"su-articles-area\"><div class=\"su-row su-articles-list\">
         <div class=\"su-col-lg-24\"><article class=\"su-articles-list-item su-focus-big\">
         <div class=\"su-media-container\"><figure><div class=\"su-ratio-4-3 su-ratio-16-7-md su-main-picture\">{img}</div></figure></div>
         <p class=\"su-title\"><a>REPORTAGE test titre long</a></p>
         <div class=\"su-metadata\"><time class=\"su-date\">07h00</time></div>
         </article></div></div></section>"
    )
}

#[test]
fn b27_probe_structure_reelle() {
    let (_dom, bx) = pipeline(
        &html_skeleton("<img class=\"su-media\" src=\"/a.jpg\">"),
        css_base(),
    );
    dump(&bx, 0);
    let mut all = Vec::new();
    widest(&bx, &mut all);
    let over: Vec<f32> = all
        .iter()
        .map(|b| b.rect.width)
        .filter(|w| *w > VIEW + 1.0)
        .collect();
    eprintln!("over-width boxes: {over:?}");
    assert!(over.is_empty(), "boîte dépassant le viewport : {over:?}");
}

/// Markup fidèle au dump vol48 (attrs verbatim) + **vraies**
/// feuilles ouest-france (fondation + base) — le but est de
/// reproduire la boîte `[53 671 13387.949x7520.0396]` pour
/// nommer la déclaration coupable.
#[test]
fn b27_probe_real_markup_real_css() {
    let html = r#"<div class="sipaui"><section class="main"><div class="su-container"><div class="_b" id="focus_1_4_6"><section class="su-articles-area   ">
      <div class="su-row su-articles-list "><div class="su-col-lg-24">
      <article class="su-articles-list-item  su-premium  su-focus-big   su-margin-bottom-l  nth_0  ">
      <div class="su-media-container   "><figure><div class="su-ratio-4-3 su-ratio-16-7-md su-main-picture   ">
      <img class="su-media" sizes="(max-width: 979px) and (min-resolution: 2dppx) and (max-resolution: 2.9dppx) 50vw, (max-width: 979px) and (min-resolution: 3dppx) and (max-resolution: 3.9dppx) 33.3vw, (max-width: 979px) and (min-resolution: 4dppx) 25vw, (max-width: 979px) 100vw, (min-width: 980px) and (max-width: 1279px) 66.6vw, 75vw" srcset="/a.jpg 375w,/b.jpg 630w,/c.jpg 940w">
      </div></figure></div>
      <p class="su-title"><a class="" href="/x">REPORTAGE. Titre de test</a></p>
      <div class="su-metadata "><time class="su-date">07h00</time></div>
      </article></div></div></section></div></div></section></div>"#;
    let base = include_str!("data/of_base.css");
    let fond = include_str!("data/of_fond.css");
    let small = include_str!("data/of_small.css");
    let med = include_str!("data/of_med.css");
    let css = format!("{base}\n{fond}\n{small}\n{med}");

    let dom = TreeBuilder::parse(html);
    let mut eng = StyleEngine::new();
    eng.add_css("body { margin: 0 }", Origin::Author);
    eng.add_css(&css, Origin::Author);
    let env = StyleEnv {
        width_px: 1366,
        height_px: 768,
        color_scheme: ColorScheme::default(),
        scripting: true,
        ..StyleEnv::default()
    };
    let styles = eng.compute_tree(&dom, dom.document(), &env);
    let root_el = dom
        .children(dom.document())
        .into_iter()
        .flatten()
        .find(|&c| dom.element(c).is_ok())
        .expect("root");
    let mut sizes = std::collections::HashMap::new();
    let mut q = vec![dom.document()];
    while let Some(n) = q.pop() {
        if dom.tag_name(n).is_ok_and(|t| t == "img") {
            sizes.insert(n, (940.0_f32, 528.0_f32));
        }
        if let Ok(ch) = dom.children(n) {
            q.extend(ch.iter().rev());
        }
    }
    let mut bx = build_box_tree_sized(&dom, root_el, &styles, &sizes).expect("arbre");
    layout_tree(&mut bx, 1366.0, &BlockCtx::default());
    dump(&bx, 0);
    let mut all = Vec::new();
    widest(&bx, &mut all);
    let over: Vec<f32> = all
        .iter()
        .map(|b| b.rect.width)
        .filter(|w| *w > 1366.0)
        .collect();
    eprintln!("over-width: {over:?}");
    assert!(over.is_empty(), "boîte géante reproduite : {over:?}");
}

/// Chaîne complète du dump — `su-container`, puis `su-row`,
/// `su-col-lg-16.su-col-xl-18` (base `calc(75% - 5px)`),
/// `_b#focus`, `su-articles-area`, `su-row`,
/// `su-col-lg-24` (base `100%`), `article`. Le vol 48 a
/// posé le fils `_b` à 13387.949px : la base `calc(75%…)`
/// cassée au split whitespace retombait sur la mesure
/// max-content du sous-arbre, verrouillée par `flex-shrink:0`.
#[test]
fn b27_probe_full_chain() {
    let html = r#"<div class="sipaui"><section class="main su-container"><div class="su-row"><div class="su-col-lg-16 su-col-xl-18">
      <div class="_b" id="prev_focus_2"></div>
      <div class="_b" id="focus_1_4_6"><section class="su-articles-area   ">
      <div class="su-row su-articles-list "><div class="su-col-lg-24">
      <article class="su-articles-list-item  su-premium  su-focus-big   su-margin-bottom-l  nth_0  ">
      <div class="su-media-container   "><figure><div class="su-ratio-4-3 su-ratio-16-7-md su-main-picture   ">
      <img class="su-media" sizes="(min-width: 980px) 75vw, 100vw" srcset="/a.jpg 375w,/b.jpg 630w,/c.jpg 940w">
      </div></figure></div>
      <p class="su-title"><a class="" href="/x">REPORTAGE. Titre de test</a></p>
      <div class="su-metadata "><time class="su-date">07h00</time></div>
      </article></div><div class="su-col-lg-24">
      <article class="su-articles-list-item"><div class="su-media-container"><figure>
      <div class="su-ratio-1-1 su-main-picture"><img class="su-media" src="/d.jpg"></div>
      </figure></div><p class="su-title"><a href="/y">DIRECT. Manifestations</a></p>
      </article></div></div></section></div>
      </div><div class="su-col-lg-8 su-col-xl-6"><div class="_b">colonne droite</div></div>
      </div></section></div>"#;
    let base = include_str!("data/of_base.css");
    let fond = include_str!("data/of_fond.css");
    let small = include_str!("data/of_small.css");
    let med = include_str!("data/of_med.css");
    let css = format!("{base}\n{fond}\n{small}\n{med}");

    let dom = TreeBuilder::parse(html);
    let mut eng = StyleEngine::new();
    eng.add_css("body { margin: 0 }", Origin::Author);
    eng.add_css(&css, Origin::Author);
    let env = StyleEnv {
        width_px: 1366,
        height_px: 768,
        scripting: true,
        ..StyleEnv::default()
    };
    let styles = eng.compute_tree(&dom, dom.document(), &env);
    let root_el = dom
        .children(dom.document())
        .into_iter()
        .flatten()
        .find(|&c| dom.element(c).is_ok())
        .expect("root");
    let mut sizes = std::collections::HashMap::new();
    let mut q = vec![dom.document()];
    while let Some(n) = q.pop() {
        if dom.tag_name(n).is_ok_and(|t| t == "img") {
            sizes.insert(n, (940.0_f32, 528.0_f32));
        }
        if let Ok(ch) = dom.children(n) {
            q.extend(ch.iter().rev());
        }
    }
    let mut bx = build_box_tree_sized(&dom, root_el, &styles, &sizes).expect("arbre");
    layout_tree(&mut bx, 1366.0, &BlockCtx::default());
    dump(&bx, 0);
    let mut all = Vec::new();
    widest(&bx, &mut all);
    let over: Vec<f32> = all
        .iter()
        .map(|b| b.rect.width)
        .filter(|w| *w > 1366.0)
        .collect();
    eprintln!("over-width: {over:?}");
    assert!(
        over.is_empty(),
        "boîte dépassant le viewport (§B-27) : {over:?}"
    );
    // Verrou : `flex:0 0 calc(75% - 5px)` → base = 940 sur
    // main_cb 1260 (le split whitespace cassait la base en
    // « calc(75% » → repli max-content géant) ; `max-width:
    // calc(75% - 5px)` se résout contre le conteneur, pas
    // contre l'item lui-même (940 → 700 avant le fix).
    let widths: Vec<f32> = all.iter().map(|b| b.rect.width).collect();
    assert!(
        widths.iter().any(|w| (*w - 940.0).abs() < 0.5),
        "col-lg-16/xl-18 attendue à 940px : {widths:?}"
    );
    assert!(
        widths.iter().any(|w| (*w - 300.0).abs() < 0.5),
        "col-lg-8/xl-6 attendue à 300px : {widths:?}"
    );
}

/// Dump des styles calculés de la chaîne — nomme la
/// déclaration qui dimensionne chaque anneau.
#[test]
fn b27_dump_computed_styles() {
    const INTERESTING: &[&str] = &[
        "display",
        "width",
        "min-width",
        "max-width",
        "height",
        "min-height",
        "max-height",
        "aspect-ratio",
        "flex",
        "flex-basis",
        "order",
        "font-size",
        "margin",
        "padding",
        "column-gap",
        "container-type",
        "object-fit",
        "box-sizing",
        "overflow",
        "padding-top",
    ];

    let html = r#"<div class="sipaui"><section class="main"><div class="su-container"><div class="_b" id="focus_1_4_6"><section class="su-articles-area   ">
      <div class="su-row su-articles-list "><div class="su-col-lg-24">
      <article class="su-articles-list-item  su-premium  su-focus-big   su-margin-bottom-l  nth_0  ">
      <div class="su-media-container   "><figure><div class="su-ratio-4-3 su-ratio-16-7-md su-main-picture   ">
      <img class="su-media" sizes="(max-width: 979px) and (min-resolution: 2dppx) and (max-resolution: 2.9dppx) 50vw, (max-width: 979px) and (min-resolution: 3dppx) and (max-resolution: 3.9dppx) 33.3vw, (max-width: 979px) and (min-resolution: 4dppx) 25vw, (max-width: 979px) 100vw, (min-width: 980px) and (max-width: 1279px) 66.6vw, 75vw" srcset="/a.jpg 375w,/b.jpg 630w,/c.jpg 940w">
      </div></figure></div>
      <p class="su-title"><a class="" href="/x">REPORTAGE. Titre de test</a></p>
      <div class="su-metadata "><time class="su-date">07h00</time></div>
      </article></div></div></section></div></div></section></div>"#;
    let base = include_str!("data/of_base.css");
    let fond = include_str!("data/of_fond.css");
    let small = include_str!("data/of_small.css");
    let med = include_str!("data/of_med.css");
    let css = format!("{base}\n{fond}\n{small}\n{med}");

    let dom = TreeBuilder::parse(html);
    let mut eng = StyleEngine::new();
    eng.add_css("body { margin: 0 }", Origin::Author);
    eng.add_css(&css, Origin::Author);
    let env = StyleEnv {
        width_px: 1366,
        height_px: 768,
        scripting: true,
        ..StyleEnv::default()
    };
    let styles = eng.compute_tree(&dom, dom.document(), &env);
    let mut q = vec![dom.document()];
    while let Some(n) = q.pop() {
        if let Ok(tag) = dom.tag_name(n) {
            let cls = dom
                .attribute(n, "class")
                .ok()
                .flatten()
                .map(|c| c.trim().replace(' ', "·"))
                .unwrap_or_default();
            if let Some(st) = styles.get(n) {
                let mut props = Vec::new();
                for &p in INTERESTING {
                    if let Some(v) = st.get(p) {
                        props.push(format!("{p}={v}"));
                    }
                }
                eprintln!("<{tag} .{cls}>  {}", props.join("  "));
            }
        }
        if let Ok(ch) = dom.children(n) {
            q.extend(ch.iter().rev());
        }
    }
}
