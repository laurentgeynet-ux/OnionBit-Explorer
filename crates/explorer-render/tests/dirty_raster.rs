// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! R-37 — invalidation bornée du raster : `rasterize_dirty`
//! ne repeint que les régions `changed_rects` par-dessus la
//! frame précédente. **Garde-fou méthode** : l'équivalence
//! pixel-à-pixel « régional == plein cadre » est assertée
//! AVANT tout gain de perf — c'est elle qui doit attraper
//! un débordement d'encre (AA, overhang glyphe, clip) hors
//! des bounds déclarés.

use explorer_css::{Origin, StyleEngine, StyleEnv};
use explorer_html::TreeBuilder;
use explorer_layout::font::{FontRequest, FontSource};
use explorer_layout::{build_box_tree, layout_tree, BlockCtx};
use explorer_render::{build_display_list, rasterize, rasterize_dirty, DisplayList, Frame};

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

/// Display list d'un document posé.
fn list_of(html: &str, css: &str) -> DisplayList {
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
    list
}

/// Frame pleine d'une display list.
fn full(list: &DisplayList) -> Frame {
    rasterize(list, ctx().fonts.expect("fonte"), 1.0).expect("frame")
}

/// La garde d'équivalence : rasterise `next` en complet,
/// puis via `rasterize_dirty` par-dessus la frame de `prev`
/// — les deux frames doivent être **identiques au pixel**.
fn assert_regional_eq_full(prev: &DisplayList, next: &DisplayList) {
    let f_prev = full(prev);
    let f_next_full = full(next);
    let damage = next.changed_rects(prev);
    assert!(!damage.is_empty(), "le scénario doit produire du dirty");
    let f_next_dirty =
        rasterize_dirty(next, ctx().fonts.expect("fonte"), 1.0, f_prev, &damage).expect("régional");
    assert_eq!(
        f_next_dirty.pixels, f_next_full.pixels,
        "régional ≠ plein cadre — débordement d'encre hors bounds"
    );
}

#[test]
fn equivalence_couleur_changee() {
    // Même géométrie, fond rouge → bleu : dirty = la boîte.
    let l1 = list_of(
        "<div></div>",
        "body{margin:0} div{height:60px;background-color:#ff0000}",
    );
    let l2 = list_of(
        "<div></div>",
        "body{margin:0} div{height:60px;background-color:#0000ff}",
    );
    assert_regional_eq_full(&l1, &l2);
}

#[test]
fn equivalence_element_deplace() {
    // Le bloc descend de 60 px : ancienne zone à effacer,
    // nouvelle à peindre.
    let l1 = list_of(
        "<div></div>",
        "body{margin:0} div{height:50px;background-color:#f00}",
    );
    let l2 = list_of(
        "<div></div>",
        "body{margin:0;padding-top:60px} div{height:50px;background-color:#f00}",
    );
    assert_regional_eq_full(&l1, &l2);
}

#[test]
fn equivalence_texte_ajoute_et_retire() {
    // Texte ajouté (glyphs + AA) puis, dans l'autre sens,
    // retiré — l'ancienne zone doit redevenir fond plein.
    let l1 = list_of(
        "<div><p>mot</p></div>",
        "body{margin:0} div{height:80px;background-color:#eee} p{margin:0;font-size:20px}",
    );
    let l2 = list_of(
        "<div></div>",
        "body{margin:0} div{height:80px;background-color:#eee}",
    );
    assert_regional_eq_full(&l1, &l2);
    assert_regional_eq_full(&l2, &l1);
}

#[test]
fn equivalence_clip_overflow() {
    // Contenu clippé par `overflow:hidden` : la dirty ne
    // couvre que la zone visible, pas le contenu masqué.
    let l1 = list_of(
        "<div id=o><div id=in></div></div>",
        "body{margin:0} #o{overflow:hidden;height:30px} #in{height:80px;background-color:#f00}",
    );
    let l2 = list_of(
        "<div id=o><div id=in></div></div>",
        "body{margin:0} #o{overflow:hidden;height:30px} #in{height:80px;background-color:#0f0}",
    );
    assert_regional_eq_full(&l1, &l2);
}

#[test]
fn equivalence_transform_translate() {
    // Sous-arbre translaté : dirty résolue dans l'espace
    // framebuffer — la région peinte suit la matrice.
    let l1 = list_of(
        "<div></div>",
        "body{margin:0} div{transform:translate(10px,10px);width:40px;height:40px;background-color:#f00}",
    );
    let l2 = list_of(
        "<div></div>",
        "body{margin:0} div{transform:translate(30px,50px);width:40px;height:40px;background-color:#f00}",
    );
    assert_regional_eq_full(&l1, &l2);
}

#[test]
fn equivalence_sous_contenu_chevronchant() {
    // Un élément inchangé qui CHEVAUCHE la zone sale doit
    // être repeint dans la fenêtre (ordre de peinture) :
    // le div bleu reste exact, le vert bouge dessous.
    let l1 = list_of(
        "<div id=a></div><div id=b></div>",
        "body{margin:0} #a{position:absolute;top:10px;left:10px;\
           width:100px;height:100px;background-color:#00f;z-index:2}\
         #b{position:absolute;top:0;left:0;width:60px;height:60px;background-color:#0f0;z-index:1}",
    );
    let l2 = list_of(
        "<div id=a></div><div id=b></div>",
        "body{margin:0} #a{position:absolute;top:10px;left:10px;\
           width:100px;height:100px;background-color:#00f;z-index:2}\
         #b{position:absolute;top:20px;left:20px;width:60px;height:60px;background-color:#0f0;z-index:1}",
    );
    assert_regional_eq_full(&l1, &l2);
}

#[test]
fn equivalence_zones_disjointes_en_y() {
    // Régression du tri des fenêtres : deux régions sales
    // superposées en x mais **disjointes en y**, la seconde
    // avec un `y0` inférieur — une fusion naïve sur les
    // seules bornes max avalait la fenêtre haute dans la
    // basse (coche de checkbox jamais repeinte — attrapé
    // par `checkbox_toggles_then_submits` côté session).
    let l1 = list_of(
        "<div id=a></div><div id=b></div>",
        "body{margin:0} #a{position:absolute;top:4px;left:4px;\
           width:8px;height:8px;background-color:#f00}\
         #b{position:absolute;top:20px;left:0;\
           width:30px;height:20px;background-color:#0f0}",
    );
    let l2 = list_of(
        "<div id=a></div><div id=b></div>",
        "body{margin:0} #a{position:absolute;top:4px;left:4px;\
           width:8px;height:8px;background-color:#ff0}\
         #b{position:absolute;top:20px;left:0;\
           width:30px;height:20px;background-color:#00f}",
    );
    assert_regional_eq_full(&l1, &l2);
}

#[test]
fn dirty_vide_frame_inchangee() {
    // Listes identiques → pas de dirty → frame précédente
    // renvoyée telle quelle (coût ~nul).
    let l1 = list_of("<p>x</p>", "body{margin:0}");
    let l2 = list_of("<p>x</p>", "body{margin:0}");
    let f = full(&l1);
    let out = rasterize_dirty(&l2, ctx().fonts.expect("fonte"), 1.0, f.clone(), &[]);
    let out = out.expect("frame précédente réutilisée");
    assert_eq!(out.pixels, f.pixels);
}

#[test]
fn hauteur_changee_repli_plein_cadre() {
    // Doc plus haut → dimensions différentes → `None`,
    // l'appelant retombe sur `rasterize` complet.
    let l1 = list_of(
        "<div></div>",
        "body{margin:0} div{height:50px;background-color:#f00}",
    );
    let mut l2 = list_of(
        "<div></div>",
        "body{margin:0} div{height:50px;background-color:#f00}",
    );
    l2.height = DOC_H + 500.0;
    let f = full(&l1);
    let damage = l2.changed_rects(&l1);
    assert!(
        rasterize_dirty(&l2, ctx().fonts.expect("fonte"), 1.0, f, &damage).is_none(),
        "hauteur différente → repli plein cadre"
    );
}
