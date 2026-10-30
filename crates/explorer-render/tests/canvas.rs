// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tests R-11 : playback d'op-list `<canvas>` 2D → bitmap,
//! taille intrinsèque au layout, émission `DisplayItem::Image`.

use explorer_css::value::Color;
use explorer_css::{Origin, StyleEngine, StyleEnv};
use explorer_dom::Dom;
use explorer_html::TreeBuilder;
use explorer_layout::{build_box_tree_sized, layout_tree, BlockCtx};
use explorer_render::{raster_ops, CanvasOp, CanvasTable, DisplayItem, PaintImages};

const RED: Color = Color {
    r: 255,
    g: 0,
    b: 0,
    a: 255,
};
const BLUE: Color = Color {
    r: 0,
    g: 0,
    b: 255,
    a: 255,
};

fn px(img: &explorer_render::DecodedImage, x: u32, y: u32) -> [u8; 4] {
    let i = (y * img.width + x) as usize * 4;
    [
        img.rgba[i],
        img.rgba[i + 1],
        img.rgba[i + 2],
        img.rgba[i + 3],
    ]
}

/// §B-59 — attributs `width`/`height` hostiles : le raster
/// dégrade en `None` au lieu de demander `Pixmap::new(w,h)`
/// = alloc unique jusqu'à 2³⁵ octets (abort mesuré terrain).
#[test]
fn canvas_geant_est_degrade_pas_alloue() {
    let ops = vec![
        CanvasOp::FillColor(RED),
        CanvasOp::FillRect(0.0, 0.0, 1.0, 1.0),
    ];
    assert!(raster_ops(&ops, 131_072, 65_536).is_none(), "2^33 px → refus");
    assert!(raster_ops(&ops, 8192, 8192).is_none(), "64 Mpx → refus");
    assert!(raster_ops(&ops, 4000, 4000).is_some(), "16 Mpx pile → OK");
}

#[test]
fn fill_rect_produit_un_bitmap_rouge() {
    let ops = vec![
        CanvasOp::FillColor(RED),
        CanvasOp::FillRect(10.0, 10.0, 20.0, 20.0),
    ];
    let img = raster_ops(&ops, 100, 100).expect("bitmap");
    assert_eq!((img.width, img.height), (100, 100));
    assert_eq!(px(&img, 15, 15), [255, 0, 0, 255]);
    assert_eq!(px(&img, 0, 0)[3], 0, "hors rect = transparent");
}

#[test]
fn chemin_stroke_et_fill() {
    let ops = vec![
        CanvasOp::BeginPath,
        CanvasOp::MoveTo(5.0, 5.0),
        CanvasOp::LineTo(45.0, 5.0),
        CanvasOp::LineTo(45.0, 45.0),
        CanvasOp::ClosePath,
        CanvasOp::FillColor(BLUE),
        CanvasOp::Fill,
        // Fill n'efface pas le chemin : même path re-stroké.
        CanvasOp::StrokeColor(RED),
        CanvasOp::LineWidth(4.0),
        CanvasOp::Stroke,
    ];
    let img = raster_ops(&ops, 50, 50).expect("bitmap");
    assert_eq!(px(&img, 40, 10)[2], 255, "intérieur bleu");
    assert_eq!(px(&img, 5, 25)[3], 0, "hors du triangle");
    assert_eq!(px(&img, 25, 5)[0], 255, "bord stroke rouge");
}

#[test]
fn transform_translate_deplace_le_dessin() {
    let ops = vec![
        CanvasOp::Translate(50.0, 0.0),
        CanvasOp::FillColor(RED),
        CanvasOp::FillRect(0.0, 0.0, 10.0, 10.0),
    ];
    let img = raster_ops(&ops, 100, 20).expect("bitmap");
    assert_eq!(px(&img, 55, 5)[0], 255);
    assert_eq!(px(&img, 5, 5)[3], 0, "zone non translatée vide");
}

#[test]
fn save_restore_restaure_le_style() {
    let ops = vec![
        CanvasOp::FillColor(RED),
        CanvasOp::Save,
        CanvasOp::FillColor(BLUE),
        CanvasOp::FillRect(0.0, 0.0, 10.0, 10.0),
        CanvasOp::Restore,
        CanvasOp::FillRect(20.0, 0.0, 10.0, 10.0),
    ];
    let img = raster_ops(&ops, 40, 10).expect("bitmap");
    assert_eq!(px(&img, 5, 5)[2], 255, "sous save = bleu");
    assert_eq!(px(&img, 25, 5)[0], 255, "après restore = rouge");
}

#[test]
fn clear_rect_efface_vers_transparent() {
    let ops = vec![
        CanvasOp::FillColor(RED),
        CanvasOp::FillRect(0.0, 0.0, 40.0, 40.0),
        CanvasOp::ClearRect(10.0, 10.0, 20.0, 20.0),
    ];
    let img = raster_ops(&ops, 40, 40).expect("bitmap");
    assert_eq!(px(&img, 20, 20)[3], 0, "clear → transparent");
    assert_eq!(px(&img, 5, 5)[0], 255);
}

#[test]
fn clip_borne_le_dessin() {
    let ops = vec![
        CanvasOp::BeginPath,
        CanvasOp::Rect(0.0, 0.0, 20.0, 40.0),
        CanvasOp::Clip,
        CanvasOp::FillColor(RED),
        CanvasOp::FillRect(0.0, 0.0, 40.0, 40.0),
    ];
    let img = raster_ops(&ops, 40, 40).expect("bitmap");
    assert_eq!(px(&img, 10, 20)[0], 255, "dans le clip");
    assert_eq!(px(&img, 30, 20)[3], 0, "hors clip = vide");
}

#[test]
fn arc_dessine_un_disque() {
    let ops = vec![
        CanvasOp::BeginPath,
        CanvasOp::Arc(25.0, 25.0, 20.0, 0.0, std::f32::consts::TAU, false),
        CanvasOp::FillColor(RED),
        CanvasOp::Fill,
    ];
    let img = raster_ops(&ops, 50, 50).expect("bitmap");
    assert_eq!(px(&img, 25, 25)[0], 255, "centre du disque");
    assert_eq!(px(&img, 25, 2)[3], 0, "coin hors disque");
}

#[test]
fn ops_vides_ou_surface_nulle_rendent_none() {
    assert!(raster_ops(&[], 10, 10).is_none());
    assert!(raster_ops(&[CanvasOp::Save], 0, 10).is_none());
}

// --- Intégration layout + display list ---

#[test]
fn canvas_taille_intrinseque_300x150_et_attributs() {
    let dom = TreeBuilder::parse("<html><body><canvas id=c></canvas></body></html>");
    let mut eng = StyleEngine::new();
    eng.add_css("", Origin::Author);
    let styles = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let root = dom.document_element().expect("el");
    let bx = build_box_tree_sized(&dom, root, &styles, &std::collections::HashMap::new())
        .expect("arbre");
    let c = find_tag(&dom, &bx, "canvas").expect("boîte canvas");
    assert_eq!(c.intrinsic, Some((300.0, 150.0)), "défaut spec");

    let dom = TreeBuilder::parse("<html><body><canvas width=80 height=40></canvas></body></html>");
    let styles = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let root = dom.document_element().expect("el");
    let bx = build_box_tree_sized(&dom, root, &styles, &std::collections::HashMap::new())
        .expect("arbre");
    let c = find_tag(&dom, &bx, "canvas").expect("boîte canvas");
    assert_eq!(c.intrinsic, Some((80.0, 40.0)));
}

fn find_tag<'a>(
    dom: &Dom,
    bx: &'a explorer_layout::LayoutBox,
    tag: &str,
) -> Option<&'a explorer_layout::LayoutBox> {
    if bx
        .node
        .and_then(|n| dom.tag_name(n).ok())
        .is_some_and(|t| t == tag)
    {
        return Some(bx);
    }
    bx.children.iter().find_map(|c| find_tag(dom, c, tag))
}

#[test]
fn canvas_rejoue_son_oplist_en_image() {
    let dom =
        TreeBuilder::parse("<html><body><canvas id=c width=40 height=40></canvas></body></html>");
    let mut eng = StyleEngine::new();
    eng.add_css("", Origin::Author);
    let styles = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let root = dom.document_element().expect("el");
    let cnode = find_node(&dom, "c").expect("nœud canvas");
    let mut bx = build_box_tree_sized(&dom, root, &styles, &std::collections::HashMap::new())
        .expect("arbre");
    layout_tree(&mut bx, 400.0, &BlockCtx::default());
    let mut canvases = CanvasTable::new();
    canvases.insert(
        cnode,
        vec![
            CanvasOp::FillColor(RED),
            CanvasOp::FillRect(0.0, 0.0, 40.0, 40.0),
        ],
    );
    let images = PaintImages {
        canvases,
        ..PaintImages::default()
    };
    let list = explorer_render::build_display_list_full(&bx, 400.0, 200.0, 0.0, &images);
    let img = list
        .items
        .iter()
        .find_map(|i| match i {
            DisplayItem::Image { img, .. } => Some(img),
            _ => None,
        })
        .expect("DisplayItem::Image du canvas");
    assert_eq!((img.width, img.height), (40, 40));
    assert_eq!(px(img, 20, 20), [255, 0, 0, 255]);
}

fn find_node(dom: &Dom, id: &str) -> Option<explorer_dom::NodeId> {
    fn walk(dom: &Dom, n: explorer_dom::NodeId, id: &str) -> Option<explorer_dom::NodeId> {
        if dom.attribute(n, "id").ok().flatten().as_deref() == Some(id) {
            return Some(n);
        }
        let mut c = dom.first_child(n).ok().flatten();
        while let Some(node) = c {
            if let Some(f) = walk(dom, node, id) {
                return Some(f);
            }
            c = dom.next_sibling(node).ok().flatten();
        }
        None
    }
    walk(dom, dom.document(), id)
}
