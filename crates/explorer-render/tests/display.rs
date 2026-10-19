// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tests de la display list (étape 22) : ordre de peinture,
//! fonds/bordures/texte, clipping `overflow`, tri `z-index`.

use explorer_css::{Origin, StyleEngine, StyleEnv};
use explorer_dom::{Dom, NodeId};
use explorer_html::TreeBuilder;
use explorer_layout::font::{FontRequest, FontSource};
use explorer_layout::{build_box_tree, build_box_tree_sized, layout_tree, BlockCtx, LayoutBox};
use explorer_render::{
    build_display_list, build_display_list_full, rasterize, BorderStyle, DisplayItem,
};

const VIEW: f32 = 400.0;

struct Fixture(Vec<u8>);
impl FontSource for Fixture {
    fn font_data(&self, _req: &FontRequest) -> Option<Vec<u8>> {
        Some(self.0.clone())
    }
}

fn ctx() -> BlockCtx<'static> {
    let data = explorer_test_support::fixture_ttf();
    let src: &'static Fixture = Box::leak(Box::new(Fixture(data)));
    BlockCtx {
        fonts: Some(src),
        ..BlockCtx::default()
    }
}

fn dl(html: &str, css: &str) -> (Dom, LayoutBox, Vec<DisplayItem>) {
    let dom = TreeBuilder::parse(html);
    let mut eng = StyleEngine::new();
    eng.add_css(css, Origin::Author);
    let styles = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let root_el = dom
        .children(dom.document())
        .into_iter()
        .flatten()
        .find(|&c| dom.element(c).is_ok())
        .expect("élément racine");
    let mut bx = build_box_tree(&dom, root_el, &styles).expect("arbre");
    layout_tree(&mut bx, VIEW, &ctx());
    let list = build_display_list(&bx, VIEW, 200.0, 0.0);
    (dom, bx, list.items)
}

fn by_id(dom: &Dom, id: &str) -> NodeId {
    let mut q = vec![dom.document()];
    while let Some(n) = q.pop() {
        if dom.attribute(n, "id").ok().flatten().as_deref() == Some(id) {
            return n;
        }
        if let Ok(ch) = dom.children(n) {
            q.extend(ch.iter().rev());
        }
    }
    panic!("#{id} absent")
}

fn find(b: &LayoutBox, id: NodeId) -> Option<&LayoutBox> {
    if b.node == Some(id) {
        return Some(b);
    }
    b.children.iter().find_map(|c| find(c, id))
}

#[test]
fn fond_emis_avant_contenu() {
    let (_dom, _bx, items) = dl(
        "<div id=d><p>texte</p></div>",
        "#d{background-color:#ff0000} p{margin:0}",
    );
    // Le fond rouge est émis avant les items du <p>.
    let fill = items
        .iter()
        .position(|i| matches!(i, DisplayItem::FillRect { .. }))
        .expect("fond émis");
    assert!(fill < items.len());
    assert!(matches!(items[fill], DisplayItem::FillRect { .. }));
}

#[test]
fn bordure_un_item_quatre_cotes() {
    let (_dom, _bx, items) = dl("<div id=d></div>", "#d{border:2px solid #00f;height:10px}");
    let border = items
        .iter()
        .find_map(|i| match i {
            DisplayItem::Border { widths, styles, .. } => Some((*widths, *styles)),
            _ => None,
        })
        .expect("item bordure");
    assert!(border.0.iter().all(|w| (*w - 2.0).abs() < 0.01));
    assert!(border.1.iter().all(|s| *s == BorderStyle::Solid));
}

#[test]
fn texte_emis_depuis_fragments_shapes() {
    let (_dom, _bx, items) = dl("<p>bonjour</p>", "p{margin:0}");
    let texts = items
        .iter()
        .filter(|i| matches!(i, DisplayItem::Text { .. }))
        .count();
    assert!(texts > 0, "au moins un item texte");
}

#[test]
fn overflow_clip_encadre_les_enfants() {
    let (_dom, _bx, items) = dl(
        "<div id=o><div id=in></div></div>",
        "#o{overflow:hidden;height:20px} #in{height:50px}",
    );
    let push = items
        .iter()
        .position(|i| matches!(i, DisplayItem::PushClip(_)))
        .expect("PushClip");
    let pop = items
        .iter()
        .rposition(|i| matches!(i, DisplayItem::PopClip))
        .expect("PopClip");
    assert!(push < pop, "clip ouvert puis refermé");
    // L'enfant (fond ou fill) est entre les deux.
    assert!(items.len() > push + 1);
}

#[test]
fn zindex_trie_les_positionnes() {
    // Deux positionnés : z=5 doit passer après z=1.
    let (dom, bx, items) = dl(
        "<div id=a></div><div id=b></div>",
        "#a{position:absolute;z-index:5;background-color:#ff0000;\
         width:10px;height:10px}\
         #b{position:absolute;z-index:1;background-color:#00ff00;\
         width:10px;height:10px}",
    );
    let _ = find(&bx, by_id(&dom, "a"));
    // Les deux fonds : #b (z=1) doit être émis avant #a (z=5)
    // malgré l'ordre DOM.
    let fills: Vec<explorer_css::value::Color> = items
        .iter()
        .filter_map(|i| match i {
            DisplayItem::FillRect { color, .. } => Some(*color),
            _ => None,
        })
        .collect();
    let green = fills
        .iter()
        .position(|c| c.g == 255)
        .expect("fond vert (#b)");
    let red = fills
        .iter()
        .position(|c| c.r == 255)
        .expect("fond rouge (#a)");
    assert!(green < red, "z=1 peint avant z=5 : {fills:?}");
}

#[test]
fn display_none_nemet_rien() {
    let (_dom, _bx, items) = dl(
        "<div id=v></div><div id=h></div>",
        "#v{background-color:#ff0000;height:5px}\
         #h{display:none;background-color:#00ff00;height:5px}",
    );
    let greens = items
        .iter()
        .filter(|i| matches!(i, DisplayItem::FillRect { color, .. } if color.g == 255))
        .count();
    assert_eq!(greens, 0, "display:none ne peint pas");
}

// ── transforms / sticky (étape 42) ────────────────────────────

/// `dl` avec `scroll_y` réglable (sticky dépend du scroll).
fn dl_scrolled(html: &str, css: &str, scroll_y: f32) -> (Dom, LayoutBox, Vec<DisplayItem>) {
    let dom = TreeBuilder::parse(html);
    let mut eng = StyleEngine::new();
    eng.add_css(css, Origin::Author);
    let styles = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let root_el = dom
        .children(dom.document())
        .into_iter()
        .flatten()
        .find(|&c| dom.element(c).is_ok())
        .expect("élément racine");
    let mut bx = build_box_tree(&dom, root_el, &styles).expect("arbre");
    layout_tree(&mut bx, VIEW, &ctx());
    let list = build_display_list(&bx, VIEW, 200.0, scroll_y);
    (dom, bx, list.items)
}

#[test]
fn transform_translate_encadre_le_sous_arbre() {
    let (_dom, _bx, items) = dl(
        "<div id=t><span>x</span></div>",
        "#t{transform:translate(10px, 5px);background-color:#ff0000;\
         width:20px;height:20px}",
    );
    let push = items
        .iter()
        .position(|i| matches!(i, DisplayItem::PushTransform(_)))
        .expect("push-transform émis");
    let pop = items
        .iter()
        .rposition(|i| matches!(i, DisplayItem::PopTransform))
        .expect("pop-transform émis");
    let fill = items
        .iter()
        .position(|i| matches!(i, DisplayItem::FillRect { .. }))
        .expect("fond émis");
    assert!(push < fill && fill < pop, "fond encadré par la transform");
    // La matrice porte la translation + origine centre :
    // T(20,15) · translate(10,5) · T(-20,-15) — résultat net
    // = translate(10,5) quel que soit l'origine.
    if let DisplayItem::PushTransform(m) = &items[push] {
        assert!((m.tx - 10.0).abs() < 0.5, "tx={}", m.tx);
        assert!((m.ty - 5.0).abs() < 0.5, "ty={}", m.ty);
    }
}

#[test]
fn transform_none_ou_invalide_nemet_rien() {
    let (_dom, _bx, items) = dl(
        "<div id=t></div>",
        "#t{transform:foo(1);background-color:#ff0000;height:5px}",
    );
    assert!(
        !items
            .iter()
            .any(|i| matches!(i, DisplayItem::PushTransform(_))),
        "transform invalide ignorée"
    );
}

#[test]
fn sticky_colle_au_seuil_sous_le_scroll() {
    // Document 600px, header sticky top:0 — scroll 300 →
    // le header (haut document à y≈0) doit être translaté
    // de ~300 pour rester visible.
    let (_dom, _bx, items) = dl_scrolled(
        "<header id=h></header><main></main>",
        "body{margin:0} main{height:600px} #h{position:sticky;top:0;background-color:#ff0000;height:20px}",
        300.0,
    );
    let push = items
        .iter()
        .position(|i| matches!(i, DisplayItem::PushTransform(_)))
        .expect("sticky → transform émise");
    if let DisplayItem::PushTransform(m) = &items[push] {
        // painted_top = 0 - 300 = -300 → dy = 0 - (-300) = 300.
        assert!((m.ty - 300.0).abs() < 1.0, "ty={}", m.ty);
    }
}

#[test]
fn sticky_sans_scroll_ne_decolle_pas() {
    let (_dom, _bx, items) = dl_scrolled(
        "<header id=h></header><main></main>",
        "body{margin:0} main{height:600px} #h{position:sticky;top:0;background-color:#ff0000;height:20px}",
        0.0,
    );
    assert!(
        !items
            .iter()
            .any(|i| matches!(i, DisplayItem::PushTransform(_))),
        "pas de scroll → pas de translation"
    );
}

// ── R-18 : color des éléments inline jusqu'aux runs ───────

#[test]
fn color_inline_atteint_le_run() {
    // Avant R-18 : l'encre du texte venait de la boîte des
    // lignes (div) — `span{color}` / `a{color}` peignaient
    // noir. Le fragment doit porter le style de l'élément
    // inline le plus proche.
    let (_dom, _bx, items) = dl(
        "<div>x<span>b</span><a href=\"#\">l</a></div>",
        "div{color:#ff0000} span{color:#0000ff}",
    );
    // 3 fragments sur une ligne : « x » rouge (div), « b »
    // bleu (span), « l » bleu lien UA — triés par x.
    let mut texts: Vec<(f32, explorer_css::value::Color)> = items
        .iter()
        .filter_map(|i| match i {
            DisplayItem::Text { origin, color, .. } => Some((origin.0, *color)),
            _ => None,
        })
        .collect();
    texts.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let colors: Vec<_> = texts.iter().map(|t| t.1).collect();
    assert_eq!(colors.len(), 3, "3 runs attendus : {colors:?}");
    assert_eq!(colors[0].r, 255, "x hérite le rouge div");
    assert_eq!((colors[1].r, colors[1].b), (0, 255), "span → bleu");
    assert_eq!(
        (colors[2].r, colors[2].g, colors[2].b),
        (0, 0, 0xee),
        "a[href] → #00e UA"
    );
}

// ── R-6 : sticky tous côtés ─────────────────────────────────

/// Variante à scroll `(x, y)` — seuils `left`/`right`.
fn dl_scrolled_xy(html: &str, css: &str, scroll: (f32, f32)) -> (Dom, LayoutBox, Vec<DisplayItem>) {
    let dom = TreeBuilder::parse(html);
    let mut eng = StyleEngine::new();
    eng.add_css(css, Origin::Author);
    let styles = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let root_el = dom
        .children(dom.document())
        .into_iter()
        .flatten()
        .find(|&c| dom.element(c).is_ok())
        .expect("élément racine");
    let mut bx = build_box_tree(&dom, root_el, &styles).expect("arbre");
    layout_tree(&mut bx, VIEW, &ctx());
    let list = explorer_render::build_display_list_scrolled(
        &bx,
        VIEW,
        200.0,
        scroll,
        &explorer_render::PaintImages::default(),
    );
    (dom, bx, list.items)
}

#[test]
fn sticky_bottom_epingle_au_bas() {
    // Footer APRÈS un espaceur de 1100px (bas naturel = 1120),
    // `bottom: 0`, viewport 200 — au repos son bas dépasse le
    // viewport → translation -920 (colle au bas visible).
    let (_dom, _bx, items) = dl_scrolled(
        "<main><div style=\"height:1100px\"></div><footer id=f></footer></main>",
        "body{margin:0}\
         #f{position:sticky;bottom:0;height:20px;background-color:#ff0000}",
        0.0,
    );
    let push = items
        .iter()
        .position(|i| matches!(i, DisplayItem::PushTransform(_)))
        .expect("sticky bottom → transform");
    if let DisplayItem::PushTransform(m) = &items[push] {
        assert!((m.ty + 920.0).abs() < 1.0, "ty={}", m.ty);
    }
}

#[test]
fn sticky_left_epingle_au_bord() {
    // Item à x=50, `left: 0` — scroll_x 100 → peint à -50,
    // collé au bord gauche (dx=+50).
    let (_dom, _bx, items) = dl_scrolled_xy(
        "<main><div id=l></div></main>",
        "body{margin:0} main{width:1200px}\
         #l{position:sticky;left:0;margin-left:50px;width:50px;height:20px}",
        (100.0, 0.0),
    );
    let push = items
        .iter()
        .position(|i| matches!(i, DisplayItem::PushTransform(_)))
        .expect("sticky left → transform");
    if let DisplayItem::PushTransform(m) = &items[push] {
        assert!((m.tx - 50.0).abs() < 1.0, "tx={}", m.tx);
        assert!(m.ty.abs() < 0.01, "ty={}", m.ty);
    }
}

#[test]
fn sticky_right_epingle_au_bord() {
    // Item débordant à droite (x=900, w=50), `right: 0` —
    // peint à droite du viewport 800 → tiré à gauche.
    let (_dom, _bx, items) = dl_scrolled_xy(
        "<main><div id=r></div></main>",
        "body{margin:0} main{width:1200px}\
         #r{position:sticky;right:0;margin-left:900px;width:50px;height:20px}",
        (0.0, 0.0),
    );
    let push = items
        .iter()
        .position(|i| matches!(i, DisplayItem::PushTransform(_)))
        .expect("sticky right → transform");
    if let DisplayItem::PushTransform(m) = &items[push] {
        // painted_right = 950 > vp 400 → tx = -(950-400) = -550.
        assert!((m.tx + 550.0).abs() < 1.0, "tx={}", m.tx);
    }
}

#[test]
fn sticky_ne_sort_pas_du_parent() {
    // Borne parente : scroll 500, parent haut 100px — le
    // sticky s'arrête au bas du parent (dy=80, pas 500).
    let (_dom, _bx, items) = dl_scrolled(
        "<main><header id=h></header></main>",
        "body{margin:0} main{height:100px}\
         #h{position:sticky;top:0;height:20px;background-color:#ff0000}",
        500.0,
    );
    let push = items
        .iter()
        .position(|i| matches!(i, DisplayItem::PushTransform(_)))
        .expect("sticky → transform");
    if let DisplayItem::PushTransform(m) = &items[push] {
        assert!((m.ty - 80.0).abs() < 1.0, "borné ty={}", m.ty);
    }
}

#[test]
fn dirty_listes_identiques_rien_a_peindre() {
    let (_d1, _b1, i1) = dl("<p>x</p>", "body{margin:0}");
    let (_d2, _b2, i2) = dl("<p>x</p>", "body{margin:0}");
    let (l1, l2) = (
        explorer_render::DisplayList {
            items: i1,
            width: 400.0,
            height: 200.0,
        },
        explorer_render::DisplayList {
            items: i2,
            width: 400.0,
            height: 200.0,
        },
    );
    assert!(
        l2.changed_rects(&l1).is_empty(),
        "frame identique → aucun dirty"
    );
}

#[test]
fn dirty_couleur_changee_zone_de_litem() {
    // Même géométrie, fond rouge→bleu : le dirty rect = la
    // boîte du div (ancienne + nouvelle zone, ici identiques).
    let (_d, _b, i1) = dl(
        "<div></div>",
        "body{margin:0} div{height:50px;background-color:#ff0000}",
    );
    let (_d2, _b2, i2) = dl(
        "<div></div>",
        "body{margin:0} div{height:50px;background-color:#0000ff}",
    );
    let mk = |items| explorer_render::DisplayList {
        items,
        width: 400.0,
        height: 200.0,
    };
    let (l1, l2) = (mk(i1), mk(i2));
    let dirty = l2.changed_rects(&l1);
    assert_eq!(dirty.len(), 1, "une seule zone fusionnée : {dirty:?}");
    let r = dirty[0];
    assert!(
        r.height >= 50.0 && r.width >= 400.0,
        "couvre le div : {r:?}"
    );
}

#[test]
fn dirty_element_deplace_vieille_et_nouvelle_zones() {
    // Le bloc descend de 40px → deux zones (ou leur union
    // fusionnée) couvrant l'ancien et le nouvel emplacement.
    let (_d, _b, i1) = dl(
        "<div></div>",
        "body{margin:0} div{height:40px;background-color:#f00}",
    );
    let (_d2, _b2, i2) = dl(
        "<div></div>",
        "body{margin:0;padding-top:40px} div{height:40px;background-color:#f00}",
    );
    let mk = |items| explorer_render::DisplayList {
        items,
        width: 400.0,
        height: 200.0,
    };
    let (l1, l2) = (mk(i1), mk(i2));
    let dirty = l2.changed_rects(&l1);
    assert!(!dirty.is_empty(), "déplacement → dirty");
    let cover = dirty.iter().fold(f32::INFINITY, |a, r| a.min(r.y));
    let bottom = dirty
        .iter()
        .fold(f32::NEG_INFINITY, |a, r| a.max(r.y + r.height));
    assert!(cover <= 0.5 && bottom >= 79.5, "couvre y=0..80 : {dirty:?}");
}

#[test]
fn dirty_item_ajoute_sa_zone() {
    // Un div de plus en fin de liste → sa zone est dirty.
    let (_d, _b, i1) = dl(
        "<div></div>",
        "body{margin:0} div{height:30px;background-color:#f00}",
    );
    let (_d2, _b2, i2) = dl(
        "<div></div><section></section>",
        "body{margin:0} div{height:30px;background-color:#f00} section{height:20px;background-color:#0f0}",
    );
    let mk = |items| explorer_render::DisplayList {
        items,
        width: 400.0,
        height: 200.0,
    };
    let (l1, l2) = (mk(i1), mk(i2));
    let dirty = l2.changed_rects(&l1);
    assert!(
        dirty.iter().any(|r| r.y >= 29.0 && r.height >= 19.0),
        "nouvelle section dirty : {dirty:?}"
    );
}

// ── R-9 : `<svg>` inline ────────────────────────────────────

#[test]
fn svg_inline_boite_remplacee_et_image() {
    // `<svg width=100 height=50>` → boîte remplacée 100×50 ;
    // `collect_svg` rasterise le sous-arbre → `Image` émis.
    let dom = TreeBuilder::parse(
        "<div><svg width=\"100\" height=\"50\"><rect width=\"100\" height=\"50\" fill=\"red\"/></svg></div>",
    );
    let mut eng = StyleEngine::new();
    eng.add_css("body{margin:0}", Origin::Author);
    let styles = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let root_el = dom
        .children(dom.document())
        .into_iter()
        .flatten()
        .find(|&c| dom.element(c).is_ok())
        .expect("racine");
    let sizes = explorer_render::svg_sizes(&dom, root_el);
    let svgs = explorer_render::collect_svg(&dom, root_el, 1_000_000);
    assert_eq!(svgs.len(), 1, "un svg collecté");
    let mut bx = build_box_tree_sized(&dom, root_el, &styles, &sizes).expect("arbre");
    layout_tree(&mut bx, VIEW, &ctx());
    let images = explorer_render::PaintImages {
        svgs,
        ..explorer_render::PaintImages::default()
    };
    let list = build_display_list_full(&bx, VIEW, 200.0, 0.0, &images);
    let img = list
        .items
        .iter()
        .find_map(|i| match i {
            DisplayItem::Image { rect, .. } => Some(*rect),
            _ => None,
        })
        .expect("svg → item Image");
    assert_eq!((img.width, img.height), (100.0, 50.0), "rect svg : {img:?}");
}

#[test]
fn svg_inline_rasterise_des_pixels() {
    // Le rect rouge du SVG doit produire des pixels rouges.
    let dom = TreeBuilder::parse(
        "<svg width=\"40\" height=\"40\"><rect width=\"40\" height=\"40\" fill=\"#ff0000\"/></svg>",
    );
    let mut eng = StyleEngine::new();
    eng.add_css("body{margin:0}", Origin::Author);
    let styles = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let root_el = dom
        .children(dom.document())
        .into_iter()
        .flatten()
        .find(|&c| dom.element(c).is_ok())
        .expect("racine");
    let svgs = explorer_render::collect_svg(&dom, root_el, 1_000_000);
    assert!(!svgs.is_empty(), "svg rasterisé");
    let mut bx = build_box_tree_sized(
        &dom,
        root_el,
        &styles,
        &explorer_render::svg_sizes(&dom, root_el),
    )
    .expect("arbre");
    layout_tree(&mut bx, VIEW, &ctx());
    let images = explorer_render::PaintImages {
        svgs,
        ..explorer_render::PaintImages::default()
    };
    let list = build_display_list_full(&bx, VIEW, 200.0, 0.0, &images);
    let fonts = ctx().fonts.expect("fixture fonte");
    let frame = rasterize(&list, fonts, 1.0).expect("frame");
    // pixel (10,10) — dans le rect svg rouge.
    let idx = (10 * frame.width as usize + 10) * 4;
    let px = &frame.pixels[idx..idx + 4];
    assert!(px[0] > 200 && px[1] < 60, "pixel rouge attendu, got {px:?}");
}

#[test]
fn controle_input_peint_sa_valeur() {
    // R-21 : la valeur synthétique d'un `<input>` doit
    // produire un item Text (avant : enfant Text nu dans une
    // boîte atomique → jamais posé en ligne → invisible).
    let (_dom, _bx, items) = dl("<input value=\"abc\">", "body{margin:0}");
    let has_text = items.iter().any(|i| matches!(i, DisplayItem::Text { .. }));
    assert!(
        has_text,
        "valeur input → item Text : {}",
        explorer_render::DisplayList {
            items: items.clone(),
            width: 400.0,
            height: 200.0
        }
        .dump()
    );
}
