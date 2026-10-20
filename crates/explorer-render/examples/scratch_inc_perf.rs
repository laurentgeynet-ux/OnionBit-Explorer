// Scratch perf R-37 — build incrémental vs plein.
use explorer_css::{Origin, StyleEngine, StyleEnv};
use explorer_html::TreeBuilder;
use explorer_layout::font::{FontRequest, FontSource};
use explorer_layout::{build_box_tree, layout_tree, BlockCtx, LayoutBox};

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
use explorer_render::{build_display_list_incremental, DisplayList, PaintImages};
use std::fmt::Write as _;

fn build_doc(n: usize) -> (explorer_dom::Dom, explorer_layout::LayoutBox) {
    let mut html = String::from("<body>");
    for i in 0..n {
        let _ = write!(
            html,
            "<div class=r><p>ligne {i} avec <i>du texte</i> assez long pour <b>shaper</b> plusieurs fragments</p></div>"
        );
    }
    html.push_str("</body>");
    let dom = TreeBuilder::parse(&html);
    let mut eng = StyleEngine::new();
    eng.add_css(
        "body{margin:0} .r{margin:4px;background-color:#eee} p{margin:0;font-size:14px;color:#333}",
        Origin::Author,
    );
    let styles = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let root_el = dom
        .children(dom.document())
        .into_iter()
        .flatten()
        .find(|&c| dom.element(c).is_ok())
        .expect("racine");
    let mut bx = build_box_tree(&dom, root_el, &styles).expect("arbre");
    layout_tree(&mut bx, 800.0, &ctx());
    (dom, bx)
}

fn inc(bx: &LayoutBox, prev: Option<&DisplayList>) -> DisplayList {
    let mut l = build_display_list_incremental(
        bx,
        800.0,
        300.0,
        (0.0, 0.0),
        &PaintImages::default(),
        prev,
        1,
    );
    l.height = l.height.min(300.0);
    l
}
fn inc_scroll(bx: &LayoutBox, y: f32, prev: Option<&DisplayList>) -> DisplayList {
    let mut l = build_display_list_incremental(
        bx,
        800.0,
        300.0,
        (0.0, y),
        &PaintImages::default(),
        prev,
        1,
    );
    l.height = l.height.min(300.0);
    l
}

fn shift(bx: &mut LayoutBox, dy: f32) {
    bx.rect.y += dy;
    for c in &mut bx.children {
        shift(c, dy);
    }
}

fn main() {
    let (_dom, bx) = build_doc(6000);
    let t = std::time::Instant::now();
    let prev = inc(&bx, None);
    eprintln!(
        "build plein : {:?} ({} items, {} spans)",
        t.elapsed(),
        prev.items.len(),
        prev.spans.len()
    );

    // Mutation locale in-place : un div change de couleur.
    let mut bx2 = bx.clone();
    bx2.children[0].children[42]
        .style
        .set("background-color", "#ff0000");
    // tree_epoch différent → chemin signature (style in-place
    // est hors contrat fast-path).
    let t = std::time::Instant::now();
    let l2 = inc(&bx2, Some(&prev));
    eprintln!("incrémental (1 restyle) : {:?}", t.elapsed());
    let t = std::time::Instant::now();
    let l2f = inc(&bx2, None);
    eprintln!("plein équivalent     : {:?}", t.elapsed());
    debug_assert_eq!(l2.dump(), l2f.dump());

    // Translation rigide d'un sous-arbre.
    let mut bx3 = bx.clone();
    shift(&mut bx3.children[0].children[42], 40.0);
    let t = std::time::Instant::now();
    let l3 = inc(&bx3, Some(&prev));
    eprintln!("incrémental (shift)  : {:?}", t.elapsed());
    let t = std::time::Instant::now();
    let l3f = inc(&bx3, None);
    eprintln!("plein équivalent     : {:?}", t.elapsed());
    debug_assert_eq!(l3.dump(), l3f.dump());

    // Scroll pur : arbre IDENTIQUE (même objet → fast-path).
    let t = std::time::Instant::now();
    let l4 = inc_scroll(&bx, 100.0, Some(&prev));
    eprintln!("incrémental (scroll) : {:?}", t.elapsed());
    let t = std::time::Instant::now();
    let l4f = inc_scroll(&bx, 100.0, None);
    eprintln!("plein équivalent     : {:?}", t.elapsed());
    debug_assert_eq!(l4.dump(), l4f.dump());
}
