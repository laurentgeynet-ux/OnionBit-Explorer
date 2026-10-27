// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! R-37 — construction incrémentale de la display list :
//! `build_display_list_incremental` réutilise les plages
//! (`SpanRec`) des sous-arbres inchangés au lieu de tout
//! ré-émettre. **Garde-fou méthode** : l'équivalence
//! « incrémental == plein build » est assertée item par
//! item (`dump`) ET au pixel — avant que le re-usage ne
//! serve en session. Les mutations simulent la sortie de
//! `relayout_subtree` (boîtes préservées = mêmes `Arc`,
//! sous-arbre sale reconstruit).

use explorer_css::{Origin, StyleEngine, StyleEnv};
use explorer_html::TreeBuilder;
use explorer_layout::font::{FontRequest, FontSource};
use explorer_layout::{build_box_tree, layout_tree, BlockCtx, LayoutBox};
use explorer_render::{
    build_display_list, build_display_list_incremental, rasterize, DisplayList, PaintImages,
};

const VIEW: f32 = 400.0;
const DOC_H: f32 = 300.0;

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

/// `(arbre posé, liste pleine)` d'un document — l'arbre est
/// rendu pour être cloné et muté dans les scénarios.
fn tree_of(html: &str, css: &str) -> (LayoutBox, DisplayList) {
    let dom = TreeBuilder::parse(html);
    let mut eng = StyleEngine::new();
    // R-32 : marge UA `body { margin: 8px }` neutralisée —
    // fixtures écrites pour une géométrie à marge nulle.
    eng.add_css("body { margin: 0 }", Origin::Author);
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
    let mut list = build_display_list(&bx, VIEW, DOC_H, 0.0);
    list.width = VIEW;
    (bx, list)
}

/// Liste pleine de référence sur `bx`.
fn full_of(bx: &LayoutBox, scroll_y: f32) -> DisplayList {
    let mut l = build_display_list_incremental(
        bx,
        VIEW,
        DOC_H,
        (0.0, scroll_y),
        &PaintImages::default(),
        None,
        0,
    );
    l.width = VIEW;
    l
}

/// Liste incrémentale sur `bx` depuis `prev` — `fast` =
/// même arbre (`tree_epoch` conservé → fast-path pointeur
/// exercé ; mutations in-place limitées à rect/lines) ;
/// `false` = nouvel arbre → signature complète.
fn inc_of(bx: &LayoutBox, scroll_y: f32, prev: &DisplayList, fast: bool) -> DisplayList {
    let epoch = if fast {
        prev.tree_epoch
    } else {
        prev.tree_epoch.wrapping_add(1)
    };
    let mut l = build_display_list_incremental(
        bx,
        VIEW,
        DOC_H,
        (0.0, scroll_y),
        &PaintImages::default(),
        Some(prev),
        epoch,
    );
    l.width = VIEW;
    l
}

/// La garde : la liste incrémentale doit être **identique**
/// à un plein build sur le même arbre — dump item par item
/// (géométrie + contenu) puis frame raster au pixel.
/// `fast` : mutations autorisées in-place = rect seulement.
fn assert_inc_eq_full(bx: &LayoutBox, scroll_y: f32, prev: &DisplayList, fast: bool) {
    let inc = inc_of(bx, scroll_y, prev, fast);
    let full = full_of(bx, scroll_y);
    assert_eq!(
        inc.dump(),
        full.dump(),
        "items incrémentaux ≠ items plein build"
    );
    let f_inc = rasterize(&inc, ctx().fonts.expect("fonte"), 1.0).expect("frame inc");
    let f_full = rasterize(&full, ctx().fonts.expect("fonte"), 1.0).expect("frame full");
    assert_eq!(f_inc.pixels, f_full.pixels, "pixels incrémental ≠ plein");
}

/// Translate récursivement `rect.y` d'un sous-arbre —
/// simule le décalage produit par `relayout_subtree`
/// (toutes les boîtes du sous-arbre bougent du même Δ).
fn shift_tree(bx: &mut LayoutBox, dy: f32) {
    bx.rect.y += dy;
    for line in &mut bx.lines {
        for f in &mut line.fragments {
            f.rect.y += dy;
        }
    }
    for c in &mut bx.children {
        shift_tree(c, dy);
    }
}

/// Descend `bx.children` suivant `path` (root = `<html>`,
/// `[0]` = `<body>`, `[0, k]` = k-ième enfant du body…).
fn at<'a>(bx: &'a mut LayoutBox, path: &[usize]) -> &'a mut LayoutBox {
    let mut cur = bx;
    for &i in path {
        cur = &mut cur.children[i];
    }
    cur
}

#[test]
fn incr_identique_sans_changement() {
    // Arbre intact : tout le sous-arbre partage Δ=0 —
    // copie plate totale, items identiques.
    let (bx, prev) = tree_of(
        "<div><p>alpha</p><p>beta</p><div><b>gamma</b></div></div>",
        "body{margin:0} div,p,b{font-size:16px}",
    );
    assert_inc_eq_full(&bx, 0.0, &prev, true);
}

#[test]
fn incr_scroll_copie_translatee() {
    // Scroll : Δ = -Δscroll uniforme — le sous-arbre entier
    // est copié translaté, rien ré-émis.
    let (bx, prev) = tree_of(
        "<div><p>ligne un</p><p>ligne deux</p></div>",
        "body{margin:0} p{font-size:20px;margin:8px 0}",
    );
    assert_inc_eq_full(&bx, 25.0, &prev, true);
}

#[test]
fn incr_translation_rigide_sous_arbre() {
    // Le second div descend de 40 px (sortie
    // `relayout_subtree` simulée) — ses items copiés
    // translatés, ses frères copiés à Δ=0.
    let (mut bx, prev) = tree_of(
        "<div id=a></div><div id=b><p>texte</p></div><div id=c></div>",
        "body{margin:0} div{height:30px} #a{background-color:#f00}\
         #b{background-color:#0f0} #c{background-color:#00f} p{font-size:14px;margin:0}",
    );
    shift_tree(at(&mut bx, &[0, 1]), 40.0);
    assert_inc_eq_full(&bx, 0.0, &prev, true);
}

#[test]
fn incr_mouvement_non_rigide_descend() {
    // Le parent bouge mais PAS son enfant (Δ divergent) —
    // l'invariant « Δ uniforme » doit faire descendre le
    // parent : items identiques quand même.
    let (mut bx, prev) = tree_of(
        "<div><p>fixe</p></div>",
        "body{margin:0} div{height:40px;background-color:#eee} p{font-size:16px;margin:0}",
    );
    at(&mut bx, &[0, 0, 0]).rect.y += 15.0;
    assert_inc_eq_full(&bx, 0.0, &prev, true);
}

#[test]
fn incr_restyle_sans_geometrie() {
    // Couleur changée, géométrie identique — `box_sig` doit
    // voir le style et interdire la copie (re-emit) : sans
    // la signature, la copie servirait un item stale.
    let (mut bx, prev) = tree_of(
        "<div></div><section></section>",
        "body{margin:0} div,section{height:40px} div{background-color:#f00}\
         section{background-color:#0f0}",
    );
    at(&mut bx, &[0, 0])
        .style
        .set("background-color", "#0000ff");
    // `style` muté in-place → epoch différent (la signature
    // attrape le changement ; le fast-path l'interdirait).
    assert_inc_eq_full(&bx, 0.0, &prev, false);
}

#[test]
fn incr_texte_change() {
    // Fragment texte muté (même arbre, run préservé) — la
    // signature lignes/texte doit rejeter la copie.
    let (mut bx, prev) = tree_of(
        "<p>avant</p><p>stable</p>",
        "body{margin:0} p{font-size:18px;margin:4px 0}",
    );
    let p0 = at(&mut bx, &[0, 0]);
    if let Some(f) = p0.lines.first_mut().and_then(|l| l.fragments.first_mut()) {
        f.text = "après".to_string();
    }
    assert_inc_eq_full(&bx, 0.0, &prev, false);
}

#[test]
fn incr_insertion_frere_decale_chemins() {
    // Boîte insérée en tête : les chemins des frères
    // décalent → misses de spans (conservateur) — la liste
    // reste strictement égale au plein build.
    let (bx, prev) = tree_of(
        "<div><p>un</p><p>deux</p></div>",
        "body{margin:0} p{font-size:16px;margin:4px 0}",
    );
    let (mut bx2, _) = tree_of(
        "<div><p>zero</p><p>un</p><p>deux</p></div>",
        "body{margin:0} p{font-size:16px;margin:4px 0}",
    );
    let _ = bx;
    let _ = &mut bx2;
    // `prev` vient d'un arbre à 2 enfants, `bx2` en a 3 :
    // les chemins [0,0],[0,1] matchent des contenus
    // différents → signature divergente → ré-émission.
    assert_inc_eq_full(&bx2, 0.0, &prev, true);
}

#[test]
fn incr_fixed_sous_scroll() {
    // `position:fixed` dans le document scrollé : son Δ = 0
    // ≠ Δ du document — la copie plate du parent est
    // refusée, le fixe est copié à Δ=0 en descendant.
    let (bx, prev) = tree_of(
        "<div id=t></div><div id=f></div>",
        "body{margin:0} #t{height:200px;background-color:#eee}\
         #f{position:fixed;top:10px;left:10px;width:50px;height:30px;background-color:#f00}",
    );
    assert_inc_eq_full(&bx, 60.0, &prev, true);
}

#[test]
fn incr_sticky_sous_scroll() {
    // `position:sticky` : le wrap de décalage est non
    // linéaire en scroll — F_STICKY_KID interdit la copie
    // plate du parent quand Δ ≠ 0.
    let (bx, prev) = tree_of(
        "<div id=h></div><div id=c></div>",
        "body{margin:0} #h{position:sticky;top:0;height:30px;background-color:#f00}\
         #c{height:200px;background-color:#eee}",
    );
    assert_inc_eq_full(&bx, 80.0, &prev, true);
}

#[test]
fn incr_porteur_inline_et_drain() {
    // Inline enfant de bloc : les porteurs sont reportés et
    // drainés par la portée — les spans doivent rester
    // corrects à travers la collecte/drain (chemin complet
    // capturé à l'enfilement, pas l'index local).
    let (bx, prev) = tree_of(
        "<div><i>italique</i> et <b>gras</b><p>para</p></div>",
        "body{margin:0} div,i,b,p{font-size:16px}",
    );
    assert_inc_eq_full(&bx, 0.0, &prev, true);
    assert_inc_eq_full(&bx, 12.0, &prev, true);
}

#[test]
fn incr_overflow_clip_translate() {
    // Clip + translation : les PushClip translatés doivent
    // suivre la plage.
    let (mut bx, prev) = tree_of(
        "<div id=o><div id=in></div></div><div id=s></div>",
        "body{margin:0} #o{overflow:hidden;height:30px}\
         #in{height:80px;background-color:#f00} #s{height:20px;background-color:#0f0}",
    );
    shift_tree(at(&mut bx, &[0, 0]), 10.0);
    assert_inc_eq_full(&bx, 0.0, &prev, true);
}

#[test]
fn incr_float_et_positionnes() {
    // Flottant + positionné z-index : émis hors-flux par
    // `emit_scoped` — ré-usage correct sous translation.
    let (mut bx, prev) = tree_of(
        "<div id=f></div><div id=p></div><div id=n></div>",
        "body{margin:0} #f{float:left;width:50px;height:30px;background-color:#f00}\
         #p{position:absolute;top:5px;left:60px;width:40px;height:40px;background-color:#0f0;z-index:1}\
         #n{height:50px;background-color:#eee}",
    );
    shift_tree(at(&mut bx, &[0, 2]), 5.0);
    assert_inc_eq_full(&bx, 0.0, &prev, true);
}

#[test]
fn incr_image_jamais_reusee() {
    // Contenu remplacé : F_NO_REUSE — le bitmap (Arc
    // nouvelle à chaque build) n'est jamais copié.
    use explorer_render::DecodedImage;
    let dom = TreeBuilder::parse("<div><img src=x></div>");
    let mut eng = StyleEngine::new();
    eng.add_css("body{margin:0} img{width:20px;height:10px}", Origin::Author);
    let styles = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let root_el = dom
        .children(dom.document())
        .into_iter()
        .flatten()
        .find(|&c| dom.element(c).is_ok())
        .expect("racine");
    let mut bx = build_box_tree(&dom, root_el, &styles).expect("arbre");
    layout_tree(&mut bx, VIEW, &ctx());
    let img = std::sync::Arc::new(DecodedImage {
        width: 2,
        height: 2,
        rgba: vec![255, 0, 0, 255, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 0, 255],
    });
    let mut images = PaintImages::default();
    // Le NodeId de l'img — retrouvé dans le DOM.
    let mut img_node = None;
    let mut q = vec![dom.document()];
    while let Some(n) = q.pop() {
        if dom.tag_name(n).is_ok_and(|t| t == "img") {
            img_node = Some(n);
        }
        if let Ok(c) = dom.children(n) {
            q.extend(c);
        }
    }
    images.replaced.insert(img_node.expect("img node"), img);
    let prev = build_display_list_incremental(&bx, VIEW, DOC_H, (0.0, 0.0), &images, None, 7);
    let inc =
        build_display_list_incremental(&bx, VIEW, DOC_H, (0.0, 20.0), &images, Some(&prev), 7);
    let full = build_display_list_incremental(&bx, VIEW, DOC_H, (0.0, 20.0), &images, None, 7);
    assert_eq!(inc.dump(), full.dump(), "image : incrémental ≠ plein");
    // Et l'item Image de `inc` porte la bitmap (pas d'Arc stale).
    assert!(
        inc.items
            .iter()
            .any(|i| matches!(i, explorer_render::DisplayItem::Image { .. })),
        "item Image attendu"
    );
}

#[test]
fn incr_prev_none_build_plein() {
    // `prev = None` → équivalent au builder plein.
    let (bx, _) = tree_of("<p>x</p>", "body{margin:0}");
    let a = full_of(&bx, 0.0);
    let b = build_display_list(&bx, VIEW, DOC_H, 0.0);
    assert_eq!(a.dump(), b.dump());
}

#[test]
fn incr_frame_session_identique() {
    // Scénario bout-en-bout : deux builds (prev→inc) sur un
    // doc mixte + scroll + mutation — la frame incrémentale
    // est pixel-identique à la frame pleine.
    let (mut bx, prev) = tree_of(
        "<div id=hd></div><p>para un</p><div id=fl></div><p>para deux</p>",
        "body{margin:0} #hd{position:sticky;top:0;height:24px;background-color:#ddd}\
         p{font-size:16px;margin:6px 0} #fl{float:right;width:30px;height:30px;background-color:#0f0}",
    );
    // Mutation locale + scroll — le pire cas combiné.
    at(&mut bx, &[0, 1]).style.set("color", "#ff0000");
    assert_inc_eq_full(&bx, 30.0, &prev, false);
}

#[test]
fn incr_scroll_grand_doc_non_blanc() {
    // Repro glassdoor (anomalie #8 — frame blanche à
    // scroll_y=1600 sur doc 5520) : doc haute, `prev` peint
    // à scroll 0, incrémental à 1600 — identique au plein
    // build ET pixels non blancs.
    let html = (0..30).fold(String::new(), |mut s, i| {
        use std::fmt::Write as _;
        let _ = write!(s, "<div>bloc{i}</div>");
        s
    });
    let css = "body{margin:0} div{height:200px;background:#fed;color:#000;font-size:40px}";
    let (bx, prev) = tree_of(&html, css);
    assert!(
        bx.rect.height > 5000.0,
        "doc trop court : {}",
        bx.rect.height
    );
    assert_inc_eq_full(&bx, 1600.0, &prev, true);
    // Garde anti-blanc : la frame à 1600 doit contenir des
    // pixels non blancs (la #fed remplit le viewport).
    let inc = inc_of(&bx, 1600.0, &prev, true);
    let f = rasterize(&inc, ctx().fonts.expect("fonte"), 1.0).expect("frame");
    let non_white = f
        .pixels
        .chunks_exact(4)
        .filter(|p| p[0] != 255 || p[1] != 255 || p[2] != 255)
        .count();
    assert!(non_white > 0, "frame blanche à scroll 1600");
}
