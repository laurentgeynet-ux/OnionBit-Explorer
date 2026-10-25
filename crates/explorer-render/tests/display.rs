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
    build_display_list, build_display_list_full, rasterize, BorderStyle, DisplayItem, MarkerKind,
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
fn texte_replie_sur_la_famille_mesuree() {
    // R-81 — `font_request` (peinture) délègue à
    // `request_of_family` : le repli `font-family` absent et
    // la pondération `bolder`/`lighter` sont ceux de la mesure
    // (avant : `"sans-serif"` en dur + `bolder`→800 là où la
    // mesure lit 700 — issue #9 4ᵉ défaut).
    let (_dom, _bx, items) = dl("<p>bonjour</p><p id=b>x</p>", "#b{font-weight:bolder}");
    let (def, bold): (Vec<_>, Vec<_>) = items
        .iter()
        .filter_map(|i| match i {
            DisplayItem::Text { font, .. } => Some(font),
            _ => None,
        })
        .partition(|f| f.weight == 400);
    assert!(
        def.iter().all(|f| f.families == ["serif"]),
        "repli = famille mesurée (serif), pas sans-serif : {def:?}"
    );
    assert!(
        bold.iter().all(|f| f.weight == 700),
        "bolder pondéré comme la mesure : {bold:?}"
    );
}

#[test]
fn visibility_hidden_supprime_emission() {
    // R-49 : `visibility:hidden` supprime l'émission de tout
    // le sous-arbre (fond + texte) — motif « visually hidden »
    // des dropdowns/sr-only (BBC `LanguageDropdown` se peignait
    // en rivières de 1px de large).
    let (dom, bx, items) = dl(
        "<div id=h><p>secret</p></div>",
        "#h{visibility:hidden;width:10px;height:10px}",
    );
    assert!(
        !items.iter().any(|i| matches!(i, DisplayItem::Text { .. })),
        "aucun item texte sous visibility:hidden"
    );
    assert!(
        !items
            .iter()
            .any(|i| matches!(i, DisplayItem::FillRect { .. } | DisplayItem::Border { .. })),
        "ni fond ni bordure émis"
    );
    // La place en layout est conservée (≠ display:none).
    let h = find(&bx, by_id(&dom, "h")).expect("boîte #h");
    assert!(h.rect.height > 0.0, "la boîte garde sa hauteur");
}

#[test]
fn visibility_visible_reapparait_sous_parent_hidden() {
    // §11.2 : un descendant `visibility:visible` explicite
    // se repeint sous un parent `hidden` — la vérification
    // est par boîte (le style calculé porte déjà l'héritage).
    let (_dom, _bx, items) = dl(
        "<div id=h><p>secret</p><p id=k>shown</p></div>",
        "#h{visibility:hidden} #k{visibility:visible}",
    );
    let texts = items
        .iter()
        .filter(|i| matches!(i, DisplayItem::Text { .. }))
        .count();
    assert_eq!(texts, 1, "seul le descendant visible émet");
}

#[test]
fn visibility_collapse_equivaut_hidden() {
    // `collapse` hors table ≡ `hidden` (spec) ; sur lignes de
    // table le retrait de rangée est hors périmètre — hidden
    // est la dégradation correcte.
    let (_dom, _bx, items) = dl(
        "<div id=h><p>secret</p></div>",
        "#h{visibility:collapse;width:10px;height:10px}",
    );
    assert!(
        !items.iter().any(|i| matches!(i, DisplayItem::Text { .. })),
        "collapse n'émet rien"
    );
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
fn overflow_clip_couvre_les_lignes_propres() {
    // R-55 : motif `.sr-only` de lobste.rs — un élément EN LIGNE
    // positionné (`position:absolute`) est racine de son propre
    // contexte inline : ses fragments vivent sur `bx.lines`, émis
    // par `paint_self` AVANT le `PushClip` — le texte « Lobsters »
    // du wordmark se peignait hors de la boîte 1×1.
    let (_dom, _bx, items) = dl(
        "<h1><a href=#><b class=s>Lobsters</b></a></h1><nav>vu</nav>",
        ".s{position:absolute;width:1px;height:1px;overflow:hidden;\
         white-space:nowrap} nav{margin:0}",
    );
    let push = items
        .iter()
        .position(|i| matches!(i, DisplayItem::PushClip(r) if r.width <= 2.0))
        .expect("clip 1px");
    let pop = items[push..]
        .iter()
        .position(|i| matches!(i, DisplayItem::PopClip))
        .map(|p| p + push)
        .expect("PopClip");
    // Le fragment « Lobsters » (32 px hérités du h1) doit être
    // ENTRE le push et le pop — pas avant.
    let sr = items
        .iter()
        .position(|i| matches!(i, DisplayItem::Text { size_px, .. } if *size_px > 30.0))
        .expect("texte sr-only");
    assert!(
        push < sr && sr < pop,
        "texte hors clip : {sr} vs [{push},{pop}]"
    );
    // Le texte du nav reste hors du clip, émis normalement.
    assert!(items
        .iter()
        .any(|i| matches!(i, DisplayItem::Text { size_px, .. } if *size_px < 30.0)));
}

#[test]
fn overflow_clip_couvre_lignes_inline_block() {
    // R-55, variante atome : `inline-block` est aussi racine de
    // contexte inline — `bx.lines` sur la boîte elle-même.
    let (_dom, _bx, items) = dl(
        "<p><span id=s>Lorem ipsum dolor</span></p>",
        "#s{display:inline-block;overflow:hidden;width:10px;height:10px;\
         white-space:nowrap} p{margin:0}",
    );
    let push = items
        .iter()
        .position(|i| matches!(i, DisplayItem::PushClip(r) if r.width <= 11.0))
        .expect("clip 10px");
    let pop = items[push..]
        .iter()
        .position(|i| matches!(i, DisplayItem::PopClip))
        .map(|p| p + push)
        .expect("PopClip");
    let sr = items
        .iter()
        .position(|i| matches!(i, DisplayItem::Text { .. }))
        .expect("texte de l'atome");
    assert!(
        push < sr && sr < pop,
        "texte hors clip : {sr} vs [{push},{pop}]"
    );
}

// ── R-58 : clip géométrique `clip-path` / `clip` ─────────────

#[test]
fn clip_path_inset_clippe_toute_la_boite() {
    // Motif `.visuallyHidden` GitHub : `clip-path:inset(25%)`
    // borne la border box à son carré central 10×10 (retrait
    // de 5px par côté) — le clip enveloppe la boîte ENTIÈRE
    // (fond + contenu), pas seulement les enfants comme
    // `overflow`. (À `inset(50%)` le clip est 0×0.)
    let (_dom, _bx, items) = dl(
        "<div id=s>x</div>",
        "#s{position:absolute;left:0;top:0;width:20px;height:20px;\
         clip-path:inset(25%)}",
    );
    let push = items
        .iter()
        .position(|i| {
            matches!(i, DisplayItem::PushClip(r) if
            (r.x - 5.0).abs() < 0.6 && (r.width - 10.0).abs() < 0.6
            && (r.height - 10.0).abs() < 0.6)
        })
        .expect("PushClip inset(25%) = 10×10 centré en (5,5)");
    // Tout ce que la boîte peint (texte « x ») suit le push.
    let txt = items
        .iter()
        .position(|i| matches!(i, DisplayItem::Text { .. }))
        .expect("texte");
    assert!(push < txt, "contenu enveloppé : {txt} après {push}");
}

#[test]
fn clip_rect_zero_masque_tout() {
    // `clip:rect(0,0,0,0)` (motif sr-only WordPress/lobste.rs) :
    // clip 0×0 — la boîte ne peint rien, clip émis puis fermé.
    let (_dom, _bx, items) = dl(
        "<div id=s>x</div>",
        "#s{position:absolute;width:20px;height:20px;clip:rect(0,0,0,0)}",
    );
    let push = items
        .iter()
        .position(|i| matches!(i, DisplayItem::PushClip(r) if r.width < 0.5 && r.height < 0.5))
        .expect("PushClip 0×0");
    assert!(
        items[push..]
            .iter()
            .any(|i| matches!(i, DisplayItem::PopClip)),
        "clip refermé"
    );
}

#[test]
fn clip_sans_positionnement_n_est_pas_applique() {
    // CSS2 §11.1.2 : `clip` ne s'applique qu'aux éléments
    // absolument positionnés — un bloc statique ne doit pas
    // être clippé (un `clip` décoratif en flux ne doit pas
    // faire disparaître du contenu réel).
    let (_dom, _bx, items) = dl("<div id=s>x</div>", "#s{clip:rect(0,0,0,0)}");
    assert!(
        !items
            .iter()
            .any(|i| matches!(i, DisplayItem::PushClip(r) if r.width < 0.5)),
        "clip ignoré sur boîte en flux"
    );
}

#[test]
fn clip_path_inconnu_tolerant_pas_de_clip() {
    // `circle()`/`polygon()`/`url()` hors v1 — retombent sur
    // « pas de clip » plutôt qu'un clip faux.
    let (_dom, _bx, items) = dl(
        "<div id=s>x</div>",
        "#s{position:absolute;width:20px;height:20px;\
         clip-path:circle(50%)}",
    );
    assert!(
        !items
            .iter()
            .any(|i| matches!(i, DisplayItem::PushClip(r) if r.width <= 10.6)),
        "fonction non supportée → pas de clip"
    );
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
            spans: explorer_render::SpanMap::default(),
            tree_epoch: 0,
        },
        explorer_render::DisplayList {
            items: i2,
            width: 400.0,
            height: 200.0,
            spans: explorer_render::SpanMap::default(),
            tree_epoch: 0,
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
        spans: explorer_render::SpanMap::default(),
        tree_epoch: 0,
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
        spans: explorer_render::SpanMap::default(),
        tree_epoch: 0,
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
        spans: explorer_render::SpanMap::default(),
        tree_epoch: 0,
    };
    let (l1, l2) = (mk(i1), mk(i2));
    let dirty = l2.changed_rects(&l1);
    assert!(
        dirty.iter().any(|r| r.y >= 29.0 && r.height >= 19.0),
        "nouvelle section dirty : {dirty:?}"
    );
}

// ── R-34 : médias remplacés + `object-fit` ──────────────

/// Affiche `10×10` peinte dans une `<video>` de 300×150
/// selon `object-fit` : `fill` (défaut) étire pleine boîte,
/// `contain` centre en gardant le ratio, `cover` déborde et
/// reçoit un clip sur la content box.
#[test]
fn object_fit_positions_media_content() {
    use explorer_render::PaintImages;
    use std::sync::Arc;

    fn media_list(css: &str) -> Vec<DisplayItem> {
        let dom =
            TreeBuilder::parse("<html><body><video id=v poster=\"p.png\"></video></body></html>");
        let mut eng = StyleEngine::new();
        eng.add_css("body{margin:0}", Origin::Author);
        eng.add_css(css, Origin::Author);
        let styles = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
        let root_el = dom
            .children(dom.document())
            .into_iter()
            .flatten()
            .find(|&c| dom.element(c).is_ok())
            .expect("racine");
        let video = by_id(&dom, "v");
        let img = Arc::new(explorer_render::DecodedImage {
            width: 10,
            height: 10,
            rgba: vec![255u8; 400],
        });
        let sizes: std::collections::HashMap<NodeId, (f32, f32)> =
            std::iter::once((video, (10.0, 10.0))).collect();
        let mut bx = build_box_tree_sized(&dom, root_el, &styles, &sizes).expect("arbre");
        layout_tree(&mut bx, VIEW, &ctx());
        let images = PaintImages {
            replaced: std::iter::once((video, img)).collect(),
            ..PaintImages::default()
        };
        build_display_list_full(&bx, VIEW, 200.0, 0.0, &images).items
    }

    // `fill` par défaut : l'image couvre toute la content
    // box 300×150, sans clip.
    let items = media_list("#v{width:300px;height:150px}");
    let img = items
        .iter()
        .find_map(|i| match i {
            DisplayItem::Image { rect, .. } => Some(*rect),
            _ => None,
        })
        .expect("affiche peinte");
    assert_eq!((img.width, img.height), (300.0, 150.0));
    assert!(
        !items.iter().any(|i| matches!(i, DisplayItem::PushClip(_))),
        "fill : pas de clip"
    );

    // `contain` : 150×150 centré horizontalement (x = 75).
    let items = media_list("#v{width:300px;height:150px;object-fit:contain}");
    let img = items
        .iter()
        .find_map(|i| match i {
            DisplayItem::Image { rect, .. } => Some(*rect),
            _ => None,
        })
        .expect("affiche contain");
    assert!(
        (img.width - 150.0).abs() < 0.01 && (img.height - 150.0).abs() < 0.01,
        "contain 150×150 : {img:?}"
    );
    assert!((img.x - 75.0).abs() < 0.01, "contain centré : {img:?}");

    // `cover` : 300×300 débordant → `PushClip`/`PopClip`
    // encadrent l'item.
    let items = media_list("#v{width:300px;height:150px;object-fit:cover}");
    let img = items
        .iter()
        .find_map(|i| match i {
            DisplayItem::Image { rect, .. } => Some(*rect),
            _ => None,
        })
        .expect("affiche cover");
    assert!(
        img.width > 150.0 && img.height > 150.0,
        "cover déborde : {img:?}"
    );
    let push = items
        .iter()
        .position(|i| matches!(i, DisplayItem::PushClip(_)))
        .expect("cover clippe");
    let pop = items
        .iter()
        .rposition(|i| matches!(i, DisplayItem::PopClip))
        .expect("pop-clip");
    let at = items
        .iter()
        .position(|i| matches!(i, DisplayItem::Image { .. }))
        .unwrap();
    assert!(push < at && at < pop, "image entre les clips");
}

/// R-95 — `background-size` met la tuile à l'échelle au
/// raster : le bitmap est réduit/étiré vers la tuile déclarée
/// au lieu d'être dessiné à taille naturelle. Cas mesuré :
/// `.votearrow` de news.ycombinator.com — SVG 32×32 dont le
/// motif est **centré** ; à taille naturelle la boîte 10×10
/// n'échantillonnait que le coin (0,0) — vide → flèche
/// invisible malgré fetch, décodage et matching corrects.
#[test]
fn background_size_met_la_tuile_a_l_echelle() {
    use explorer_render::PaintImages;
    use std::sync::Arc;

    // Bitmap 32×32 : carré rouge central x12..20 / y12..20
    // — à taille naturelle, une boîte 10×10 ne voit que du
    // transparent ; réduite à 10×10, le rouge couvre le centre.
    fn center_red() -> Arc<explorer_render::DecodedImage> {
        let mut rgba = vec![0u8; 32 * 32 * 4];
        for y in 12..20 {
            for x in 12..20 {
                let i = (y * 32 + x) * 4;
                rgba[i] = 255;
                rgba[i + 3] = 255;
            }
        }
        Arc::new(explorer_render::DecodedImage {
            width: 32,
            height: 32,
            rgba,
        })
    }

    fn bg_frame(css: &str) -> explorer_render::Frame {
        let dom = TreeBuilder::parse("<html><body><div id=v></div></body></html>");
        let mut eng = StyleEngine::new();
        eng.add_css("body{margin:0}", Origin::Author);
        eng.add_css(css, Origin::Author);
        let styles = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
        let root_el = dom
            .children(dom.document())
            .into_iter()
            .flatten()
            .find(|&c| dom.element(c).is_ok())
            .expect("racine");
        let v = by_id(&dom, "v");
        let mut bx = build_box_tree(&dom, root_el, &styles).expect("arbre");
        layout_tree(&mut bx, VIEW, &ctx());
        let images = PaintImages {
            backgrounds: std::iter::once((v, center_red())).collect(),
            ..PaintImages::default()
        };
        let list = build_display_list_full(&bx, VIEW, 200.0, 0.0, &images);
        let fonts = ctx().fonts.expect("fixture fonte");
        rasterize(&list, fonts, 1.0).expect("frame")
    }

    fn px(frame: &explorer_render::Frame, x: usize, y: usize) -> [u8; 4] {
        let i = (y * frame.width as usize + x) * 4;
        [
            frame.pixels[i],
            frame.pixels[i + 1],
            frame.pixels[i + 2],
            frame.pixels[i + 3],
        ]
    }

    // `background-size:10px` → centre de la boîte rouge.
    let f = bg_frame(
        "#v{width:10px;height:10px;background:url(/i.png) no-repeat;background-size:10px}",
    );
    assert_eq!(
        px(&f, 5, 5),
        [255, 0, 0, 255],
        "tuile réduite : le motif centré peint au centre de la boîte"
    );
    // Sans `background-size` → taille naturelle : le même pixel
    // n'échantillonne que le coin transparent (référence du bug
    // — le canvas non couvert reste blanc).
    let f = bg_frame("#v{width:10px;height:10px;background:url(/i.png) no-repeat}");
    assert_eq!(
        px(&f, 5, 5),
        [255, 255, 255, 255],
        "taille naturelle : le coin de l'image reste transparent"
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
            height: 200.0,
            spans: explorer_render::SpanMap::default(),
            tree_epoch: 0,
        }
        .dump()
    );
}

#[test]
fn controle_input_valeur_posee_sous_le_controle() {
    // R-23 : la valeur doit peindre à la POSITION du
    // contrôle — avant : le fragment recevait le rect final
    // mais le sous-arbre de l'atome restait posé en (0,0).
    let (_dom, _bx, items) = dl(
        "<p>avant</p><input value=\"abc\">",
        "body{margin:0} p{height:40px;line-height:40px}",
    );
    let origins: Vec<(f32, f32)> = items
        .iter()
        .filter_map(|i| match i {
            DisplayItem::Text { origin, .. } => Some(*origin),
            _ => None,
        })
        .collect();
    assert_eq!(origins.len(), 2, "texte p + valeur input : {origins:?}");
    assert!(
        origins[1].1 > origins[0].1,
        "valeur attendue sous le paragraphe, origines {origins:?}"
    );
    // Et pas collée à l'origine du document.
    assert!(origins[1].1 > 20.0, "valeur en (0,0) = régression R-23");
}

// ── R-13d : background-position / attachment ─────────────────

/// div 40×20 avec `css_extra` + un fond image factice 4×2
/// injecté par `NodeId` — retourne la display list.
fn bg_div(css_extra: &str) -> explorer_render::DisplayList {
    let dom = TreeBuilder::parse("<body><div class=x></div></body>");
    let mut eng = StyleEngine::new();
    eng.add_css(
        &format!("body{{margin:0}} .x{{width:40px;height:20px;{css_extra}}}"),
        Origin::Author,
    );
    let styles = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let mut bx = build_box_tree_sized(
        &dom,
        dom.document_element().unwrap(),
        &styles,
        &std::collections::HashMap::new(),
    )
    .expect("arbre");
    layout_tree(&mut bx, VIEW, &ctx());
    let img = std::sync::Arc::new(explorer_render::DecodedImage {
        width: 4,
        height: 2,
        rgba: vec![255; 16],
    });
    let div_node = {
        let mut q = vec![dom.document()];
        let mut f = None;
        while let Some(n) = q.pop() {
            if dom.tag_name(n).is_ok_and(|t| t == "div") {
                f = Some(n);
            }
            if let Ok(c) = dom.children(n) {
                q.extend(c);
            }
        }
        f.unwrap()
    };
    let mut images = explorer_render::PaintImages::default();
    images.backgrounds.insert(div_node, img);
    build_display_list_full(&bx, VIEW, 200.0, 0.0, &images)
}

#[test]
fn background_position_decale_lorigine() {
    let list = bg_div("background:url(x) no-repeat;background-position:1px 0");
    let item = list
        .items
        .iter()
        .find_map(|i| match i {
            DisplayItem::BackgroundImage { origin, rect, .. } => Some((*origin, *rect)),
            _ => None,
        })
        .expect("bg item");
    assert_eq!(item.0, (1.0, 0.0), "1px 0 → origine décalée");
}

#[test]
fn background_image_multicouche_peint_la_couche_url() {
    // R-71 : `linear-gradient(…),url(…)` (sprites Wikipedia
    // `.sprite`) — la couche `url()` doit peindre même quand
    // elle n'est pas en première position, avec tranche
    // d'atlas par `background-position` négatif.
    let list = bg_div(
        "background-image:linear-gradient(transparent,transparent),url(a.svg);\
         background-repeat:no-repeat;background-position:0 -5px",
    );
    let (origin, rect) = list
        .items
        .iter()
        .find_map(|i| match i {
            DisplayItem::BackgroundImage { origin, rect, .. } => Some((*origin, *rect)),
            _ => None,
        })
        .expect("l'item fond multi-couche doit être émis");
    // `-5px` = tranche d'atlas : l'origine de la tuile
    // remonte de 5 px pour dévoiler la rangée visée.
    assert_eq!(origin.1, -5.0, "position négative = tranche d'atlas");
    assert_eq!((rect.width, rect.height), (40.0, 20.0));
}

#[test]
fn linear_gradient_peint_un_degrade_reel() {
    // Issue #10 : la couche `linear-gradient` est rasterisée
    // à la taille de la zone de positionnement puis peinte —
    // `to right` → bord gauche rouge, bord droit bleu.
    let list = bg_div("background-image:linear-gradient(to right, red, blue)");
    let img = list
        .items
        .iter()
        .find_map(|i| match i {
            DisplayItem::BackgroundImage { img, .. } => Some(img.clone()),
            _ => None,
        })
        .expect("l'item dégradé doit être émis");
    assert_eq!((img.width, img.height), (40, 20), "bitmap = zone padding");
    let px = |x: usize| &img.rgba[x * 4..x * 4 + 4];
    assert!(
        px(0)[0] > 200 && px(0)[2] < 60,
        "gauche rouge : {:?}",
        px(0)
    );
    assert!(
        px(39)[2] > 200 && px(39)[0] < 60,
        "droite bleue : {:?}",
        px(39)
    );
}

#[test]
fn linear_gradient_direction_et_shorthand() {
    // Direction par défaut `to bottom` via le shorthand
    // `background:` (tokenisation paren-consciente — le
    // dégradé à espaces reste un atome, issue #10).
    let list = bg_div("background:linear-gradient(red, blue)");
    let img = list
        .items
        .iter()
        .find_map(|i| match i {
            DisplayItem::BackgroundImage { img, .. } => Some(img.clone()),
            _ => None,
        })
        .expect("item dégradé via shorthand");
    let top = &img.rgba[0..4];
    let bottom = &img.rgba[(img.height as usize - 1) * img.width as usize * 4..];
    assert!(top[0] > 200 && top[2] < 60, "haut rouge : {top:?}");
    assert!(bottom[2] > 200 && bottom[0] < 60, "bas bleu : {bottom:?}");
}

#[test]
fn shorthand_size_preserve_la_declaration() {
    // Issue #10 (bonus) : `50% / contain` ne doit plus faire
    // tomber la déclaration entière — la couleur survit et
    // peint ; la couche image est `none` → aucun item image.
    let list = bg_div("background:red 50% / contain no-repeat");
    assert!(
        list.items.iter().any(|i| matches!(
            i,
            DisplayItem::FillRect { color, .. } if color.r > 200 && color.g < 60
        )),
        "le fond rouge doit peindre"
    );
    assert!(
        !list
            .items
            .iter()
            .any(|i| matches!(i, DisplayItem::BackgroundImage { .. })),
        "background-image:none → pas d'item image"
    );
}

#[test]
fn couches_multiples_ordre_spec() {
    // `gradient,url` : la DERNIÈRE couche se peint en premier
    // (fond) puis la première (dessus) — l'item url doit
    // précéder l'item dégradé dans la display list.
    let list = bg_div("background-image:linear-gradient(red,blue),url(x)");
    let mut kinds = Vec::new();
    for i in &list.items {
        if let DisplayItem::BackgroundImage { img, .. } = i {
            kinds.push(img.width); // url injectée = 4 px, dégradé = 40 px.
        }
    }
    assert_eq!(kinds, vec![4, 40], "url (dessous) puis dégradé (dessus)");
}

#[test]
fn background_fixed_ancre_au_viewport() {
    // Attachment fixed : la zone de positionnement est le
    // viewport — à scroll 0, origine = coin écran + position.
    let list = bg_div("background:url(x) no-repeat;background-attachment:fixed");
    let origin = list
        .items
        .iter()
        .find_map(|i| match i {
            DisplayItem::BackgroundImage { origin, .. } => Some(*origin),
            _ => None,
        })
        .expect("bg fixed");
    // Viewport à scroll 0 : origine = (0,0) + position 0.
    assert_eq!(origin, (0.0, 0.0));
}

#[test]
fn deco_inline_peinte_par_fragment_pas_par_union() {
    // R-22 : le fond/la bordure d'un lien qui se coupe sur
    // deux lignes doit être repeint PAR FRAGMENT (rect de
    // chaque fragment), pas une fois sur l'union — sinon le
    // fond couvre toute la zone entre les deux morceaux.
    let (_d, _b, items) = dl(
        "<p>pré <a>aaaa bbbb cccc dddd eeee ffff gggg</a> post</p>",
        "p{font-size:10px;width:80px} a{background-color:#00ff00;border:1px solid #000000;padding:2px}",
    );
    let fills: Vec<(f32, f32)> = items
        .iter()
        .filter_map(|i| match i {
            DisplayItem::FillRect { rect, color } if color.g > 200 => {
                Some((rect.width, rect.height))
            }
            _ => None,
        })
        .collect();
    // Le lien se coupe sur ≥2 lignes → ≥2 fills verts, chacun
    // large de son fragment + padding (≤ 80px conteneur), et
    // bien plus étroit qu'une union traversière.
    assert!(fills.len() >= 2, "un fill par fragment : {fills:?}");
    for (w, h) in &fills {
        assert!(*w <= 80.0, "fill {w}×{h} > conteneur");
        assert!(*h <= 20.0, "fill hauteur fragment ~16, mesuré {h}");
    }
}

#[test]
fn border_color_sans_couleur_suit_currentcolor() {
    // R-22 : `border:1px solid` sans couleur → currentColor
    // = `color` de l'élément (≠ noir). Avant : BLACK codé en
    // dur → pastilles lobste.rs sombres.
    let (_d, _b, items) = dl(
        "<div id=x>texte</div>",
        "#x{color:#123456;border:1px solid}",
    );
    let colors = items
        .iter()
        .find_map(|i| match i {
            DisplayItem::Border { colors, .. } => Some(*colors),
            _ => None,
        })
        .expect("item bordure");
    // #123456 = rgb(18,52,86).
    assert_eq!(colors[0].r, 18);
    assert_eq!(colors[0].g, 52);
    assert_eq!(colors[0].b, 86, "currentColor attendu : {colors:?}");
}

#[test]
fn ordre_appendice_e_bloc_flottant_inline() {
    // App. E : fonds de blocs en flux → flottants → contenu en
    // ligne. Un bloc statique passe sous le flottant, un span en
    // ligne passe au-dessus — indépendamment de l'ordre DOM.
    let (_d, _b, items) = dl(
        "<div id=c>\
         <span id=inl>txt</span>\
         <div id=flt></div>\
         <div id=blk></div></div>",
        "#c{position:relative;width:100px}\
         #inl{position:static;color:#010203}\
         #flt{float:left;width:20px;height:20px;background:#040506}\
         #blk{width:20px;height:20px;background:#070809}",
    );
    let pos = |needle: [u8; 3]| -> usize {
        items
            .iter()
            .position(|i| match i {
                DisplayItem::FillRect { color, .. } => [color.r, color.g, color.b] == needle,
                _ => false,
            })
            .unwrap_or(usize::MAX)
    };
    let blk = pos([7, 8, 9]);
    let flt = pos([4, 5, 6]);
    assert!(blk < flt, "bloc {blk} avant flottant {flt}");
}

// ── triage terrain : <img> remplacée dans une flex row ──────

#[test]
fn img_flex_row_garde_son_ratio() {
    // rust-lang.org : icônes blanches étirées en colonne —
    // un <img> remplacé dans une flex row ne doit pas hériter
    // la hauteur de la rangée (align-items:stretch → start
    // pour un élément à ratio intrinsèque, CSS Align §5.3).
    let dom = TreeBuilder::parse("<div id=r><div><img id=i></div><div id=t>t</div></div>");
    let mut eng = StyleEngine::new();
    eng.add_css("body{margin:0}", Origin::Author);
    eng.add_css(
        "#r{display:flex} #t{width:200px;height:2000px}",
        Origin::Author,
    );
    let styles = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let root_el = dom
        .children(dom.document())
        .into_iter()
        .flatten()
        .find(|&c| dom.element(c).is_ok())
        .expect("racine");
    let i = by_id(&dom, "i");
    let sizes: std::collections::HashMap<NodeId, (f32, f32)> =
        std::iter::once((i, (512.0, 512.0))).collect();
    let mut bx = build_box_tree_sized(&dom, root_el, &styles, &sizes).expect("arbre");
    layout_tree(&mut bx, VIEW, &ctx());
    let img = find(&bx, i).expect("box img");
    eprintln!("img box = {:?}", img.rect);
    assert!(
        (img.rect.height - 512.0).abs() < 1.0,
        "img étirée à la rangée : {:?}",
        img.rect
    );
}
#[test]
fn repro_social() {
    let dom = TreeBuilder::parse("<div id=c><a><img id=i></a><a><img id=j></a></div>");
    let mut eng = StyleEngine::new();
    eng.add_css("body{margin:0}", Origin::Author);
    eng.add_css(
        "#c{display:flex;flex-direction:row;flex-wrap:wrap;align-items:center;width:215px}",
        Origin::Author,
    );
    let styles = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let root_el = dom
        .children(dom.document())
        .into_iter()
        .flatten()
        .find(|&c| dom.element(c).is_ok())
        .expect("racine");
    let mut sizes = std::collections::HashMap::<NodeId, (f32, f32)>::new();
    sizes.insert(by_id(&dom, "i"), (480.0, 511.0));
    sizes.insert(by_id(&dom, "j"), (235.0, 166.0));
    let mut bx = build_box_tree_sized(&dom, root_el, &styles, &sizes).expect("arbre");
    layout_tree(&mut bx, VIEW, &ctx());
    eprintln!("img i = {:?}", find(&bx, by_id(&dom, "i")).unwrap().rect);
    eprintln!("img j = {:?}", find(&bx, by_id(&dom, "j")).unwrap().rect);
}

// ── R-70 : décorations inline SOUS le texte de la ligne ────

#[test]
fn deco_inline_peint_sous_le_texte_de_la_ligne() {
    // Wikipedia portail : le fond+bordure `.lang-list-button`
    // tranchés par fragment (`box-decoration-break: clone`,
    // approx. R-22) se peignaient entrelacés avec les textes —
    // le rect du fragment espace, gonflé de `padding`,
    // couvrait la queue de « language » (encre du frère).
    // Les décos d'une ligne doivent précéder tous ses textes.
    let (_dom, _bx, items) = dl(
        "<p><a id=a>x <span id=t>language</span> y</a></p>",
        "p{margin:0}\
         #a{background-color:#f8f9fa;border:3px solid #767676;\
            padding:.6rem 1.2rem}",
    );
    let deco =
        |i: &DisplayItem| matches!(i, DisplayItem::FillRect { .. } | DisplayItem::Border { .. });
    let texte = |i: &DisplayItem| matches!(i, DisplayItem::Text { .. });
    let n_deco = items.iter().filter(|i| deco(i)).count();
    assert!(
        n_deco >= 2,
        "décos des fragments espace attendues : {items:?}"
    );
    assert!(items.iter().any(&texte), "textes attendus : {items:?}");
    let dernier_deco = items.iter().rposition(deco).unwrap();
    let premier_texte = items.iter().position(texte).unwrap();
    assert!(
        dernier_deco < premier_texte,
        "déco émise après un texte de la ligne : {items:?}"
    );
}

// ── Issue #9 : marqueurs de list-item (puces UA) ──────────

fn marker(items: &[DisplayItem]) -> Option<(explorer_layout::Rect, MarkerKind)> {
    items.iter().find_map(|i| match i {
        DisplayItem::Marker { rect, kind, .. } => Some((*rect, *kind)),
        _ => None,
    })
}

#[test]
fn list_item_ul_emet_une_puce_disc() {
    // `ul` → `list-style-type: disc` (défaut UA + initiale) :
    // la puce se peint dans la gouttière avant le contenu,
    // centrée sur la première ligne.
    let (dom, bx, items) = dl("<ul><li id=i>item</li></ul>", "");
    let (rect, kind) = marker(&items).expect("puce disc attendue");
    assert_eq!(kind, MarkerKind::Disc);
    let li = find(&bx, by_id(&dom, "i")).expect("li");
    let content_x = li.rect.x
        + li.style
            .get("border-left-width")
            .and_then(|v| v.trim().strip_suffix("px"))
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(0.0)
        + li.style
            .get("padding-left")
            .and_then(|v| v.trim().strip_suffix("px"))
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(0.0);
    assert!(
        rect.x + rect.width <= content_x + 0.5,
        "puce à gauche du contenu : {rect:?} vs content_x={content_x}"
    );
    assert!(rect.width > 2.0 && rect.width < 16.0, "d ≈ em/3 : {rect:?}");
}

#[test]
fn list_item_styles_circle_square_none() {
    let (_d, _b, items) = dl(
        "<ul><li>a</li></ul><ul id=c><li>b</li></ul><ul id=s><li>c</li></ul><ul id=n><li>d</li></ul>",
        "#c{list-style-type:circle}#s{list-style-type:square}#n{list-style-type:none}",
    );
    let kinds: Vec<MarkerKind> = items
        .iter()
        .filter_map(|i| match i {
            DisplayItem::Marker { kind, .. } => Some(*kind),
            _ => None,
        })
        .collect();
    assert_eq!(
        kinds,
        vec![MarkerKind::Disc, MarkerKind::Circle, MarkerKind::Square],
        "4 listes → 3 puces (none n'en émet pas) : {kinds:?}"
    );
}

#[test]
fn list_item_ol_decimal_ne_peint_pas_de_disc_trompeur() {
    // `ol` → `list-style-type: decimal` (UA, spec §lists) :
    // la puce textuelle n'est pas encore shapée — aucune
    // puce plutôt qu'un disc faux.
    let (_d, _b, items) = dl("<ol><li>a</li></ol>", "");
    assert!(
        marker(&items).is_none(),
        "decimal hors portée → pas de puce, pas de disc : {items:?}"
    );
}

#[test]
fn list_item_ul_imbrique_circle() {
    // Spec §lists : `ul ul` → `circle` — le fils hérite du
    // type du conteneur.
    let (_d, _b, items) = dl("<ul><li>a<ul><li>b</li></ul></li></ul>", "");
    let kinds: Vec<MarkerKind> = items
        .iter()
        .filter_map(|i| match i {
            DisplayItem::Marker { kind, .. } => Some(*kind),
            _ => None,
        })
        .collect();
    assert_eq!(
        kinds,
        vec![MarkerKind::Disc, MarkerKind::Circle],
        "niveau 1 disc, niveau 2 circle : {kinds:?}"
    );
}

#[test]
fn list_item_vide_peint_quand_meme_sa_puce() {
    // `<li></li>` sans ligne ni enfant : la puce reste du
    // contenu de la boîte (baseline de repli sur font-size).
    let (_d, _b, items) = dl("<ul><li></li></ul>", "");
    let (_rect, kind) = marker(&items).expect("li vide → puce quand même");
    assert_eq!(kind, MarkerKind::Disc);
}

#[test]
fn list_item_marqueur_rasterise_des_pixels() {
    // La puce n'est pas qu'un item — elle peint des pixels
    // de la couleur du texte à sa place.
    let (_d, bx, items) = dl(
        "<ul><li style=\"color:#ff0000\">item</li></ul>",
        "body{color:#000}",
    );
    let (rect, _kind) = marker(&items).expect("puce");
    let fonts = ctx().fonts.expect("fixture fonte");
    let list = build_display_list(&bx, VIEW, 200.0, 0.0);
    let frame = rasterize(&list, fonts, 1.0).expect("frame");
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let px_idx = |v: f32| v.round() as usize;
    let (cx, cy) = (
        px_idx(rect.x + rect.width / 2.0),
        px_idx(rect.y + rect.height / 2.0),
    );
    let idx = (cy * frame.width as usize + cx) * 4;
    let px = &frame.pixels[idx..idx + 4];
    assert!(
        px[0] > 200 && px[1] < 60,
        "pixel de puce rouge (couleur du li) attendu à ({cx},{cy}), got {px:?}"
    );
}

// ── R-74 bis : pseudo `::after` positionné + dégradé (NPR) ──

#[test]
fn pseudo_after_positionne_peint_son_fond() {
    // text.npr.org : `.hr-line:after` = bandeau tricolore —
    // `content:''` + `position:absolute` + `background:
    // linear-gradient`. Deux trous chaînés :
    // (a) `box_kind_of` ne blockifiait pas `display:inline` +
    //     `position:absolute` (§9.7) — le pseudo dégénérait
    //     en boîte inline vide ;
    // (b) `canvas_bg_src != bx.node` sautait la section fond
    //     de TOUTE boîte sans nœud (`None != None` → faux) —
    //     un pseudo ne pouvait jamais peindre un fond.
    let (_dom, _bx, items) = dl(
        "<div class=\"hr-line\"></div>",
        ".hr-line{position:relative;height:4px}\
         .hr-line:after{background:linear-gradient(to right,\
           #e60000 33%,#000 33%,#000 66%,#36c 66%);\
           position:absolute;content:'';height:4px;right:0;left:0;top:0}",
    );
    let img = items
        .iter()
        .find_map(|i| match i {
            DisplayItem::BackgroundImage { img, .. } => Some(img.clone()),
            _ => None,
        })
        .expect("couche dégradé du ::after attendue");
    // Tricolore horizontal : rouge à gauche, noir au centre,
    // bleu à droite.
    let px = |x: usize| &img.rgba[x * 4..x * 4 + 4];
    let (w, mid) = (img.width as usize, img.width as usize / 2);
    assert!(px(0)[0] > 200, "gauche rouge : {:?}", px(0));
    assert!(
        px(mid)[0] < 60 && px(mid)[2] < 60,
        "centre noir : {:?}",
        px(mid)
    );
    assert!(px(w - 1)[2] > 150, "droite bleue : {:?}", px(w - 1));
}

#[test]
fn pseudo_after_fond_peint_meme_sans_canvas() {
    // Le cas limite exact du garde : pseudo avec fond alors
    // qu'AUCUN canvas n'est propagé (`canvas_bg_src = None`
    // ET `bx.node = None` — l'ancien `!=` rendait faux).
    let (_d, _b, items) = dl(
        "<div id=d><span id=s>x</span></div>",
        "#s:after{content:'';display:block;height:6px;\
           background-color:#ff0000}",
    );
    assert!(
        items.iter().any(|i| matches!(
            i,
            DisplayItem::FillRect { color, .. } if color.r > 200 && color.g < 60
        )),
        "fond rouge du ::after attendu : {items:?}"
    );
}

// ── R-104 : radial-/conic-gradient peints ───────────────────

#[test]
fn radial_gradient_peint_un_degrade_centre_bord() {
    // `radial-gradient(red, blue)` : centre rouge, bord bleu.
    let list = bg_div("background-image:radial-gradient(red, blue)");
    let img = list
        .items
        .iter()
        .find_map(|i| match i {
            DisplayItem::BackgroundImage { img, .. } => Some(img.clone()),
            _ => None,
        })
        .expect("dégradé radial émis");
    assert_eq!((img.width, img.height), (40, 20));
    let px = |x: usize, y: usize| {
        let o = (y * img.width as usize + x) * 4;
        &img.rgba[o..o + 4]
    };
    assert!(
        px(20, 10)[0] > 200 && px(20, 10)[2] < 60,
        "centre rouge : {:?}",
        px(20, 10)
    );
    // Coin = au-delà de l'ellipse farthest-corner → bleu.
    assert!(
        px(0, 0)[2] > 200 && px(0, 0)[0] < 60,
        "bord bleu : {:?}",
        px(0, 0)
    );
}

#[test]
fn conic_gradient_peint_des_quadrants() {
    // `conic-gradient(red 0 25%, blue 25% 100%)` : haut rouge,
    // le reste bleu (le stop en % est une fraction de tour).
    let list = bg_div("background-image:conic-gradient(red 0 25%, blue 25% 100%)");
    let img = list
        .items
        .iter()
        .find_map(|i| match i {
            DisplayItem::BackgroundImage { img, .. } => Some(img.clone()),
            _ => None,
        })
        .expect("dégradé conique émis");
    let px = |x: usize, y: usize| {
        let o = (y * img.width as usize + x) * 4;
        &img.rgba[o..o + 4]
    };
    // Au-dessus du centre (0deg → haut) = rouge.
    assert!(
        px(20, 1)[0] > 200 && px(20, 1)[2] < 60,
        "haut rouge : {:?}",
        px(20, 1)
    );
    // En dessous (≈180deg > 25%) = bleu.
    assert!(
        px(20, 19)[2] > 200 && px(20, 19)[0] < 60,
        "bas bleu : {:?}",
        px(20, 19)
    );
}

#[test]
fn radial_circle_et_at_position() {
    // `radial-gradient(circle at 25% 50%, white 0%, black 100%)`
    // : centre décalé → pixel (10,10) blanc, (39,10) sombre.
    let list = bg_div("background-image:radial-gradient(circle at 25% 50%, white 0%, black 100%)");
    let img = list
        .items
        .iter()
        .find_map(|i| match i {
            DisplayItem::BackgroundImage { img, .. } => Some(img.clone()),
            _ => None,
        })
        .expect("radial émis");
    let px = |x: usize, y: usize| {
        let o = (y * img.width as usize + x) * 4;
        &img.rgba[o..o + 4]
    };
    assert!(
        px(10, 10)[0] > 200,
        "centre décalé blanc : {:?}",
        px(10, 10)
    );
    assert!(px(39, 10)[0] < 60, "loin noir : {:?}", px(39, 10));
}

// ── R-106 : background-clip / background-origin consommés ───

#[test]
fn background_clip_content_box_clipe_au_contenu() {
    // `background-clip:content-box` : la zone clippée est la
    // boîte de contenu (border − border-width − padding).
    let list = bg_div(
        "border:5px solid #000;padding:4px;\
         background-image:linear-gradient(red,blue);\
         background-clip:content-box",
    );
    let rect = list
        .items
        .iter()
        .find_map(|i| match i {
            DisplayItem::BackgroundImage { rect, .. } => Some(*rect),
            _ => None,
        })
        .expect("item fond émis");
    // bg_div : `width:40px` en `content-box` (défaut) →
    // border box = 40+2·(5+4) = 58 ; le clip `content-box`
    // attendu = (5+4, 5+4, 40, 20).
    assert_eq!(
        (rect.x, rect.y, rect.width, rect.height),
        (9.0, 9.0, 40.0, 20.0),
        "clip sur la boîte de contenu"
    );
}

#[test]
fn background_origin_border_box_positionne_a_la_bordure() {
    // `background-origin:border-box` : l'aire de positionnement
    // = border box → `background-position:0 0` place l'origine
    // de la tuile au bord externe (pas au padding box).
    let list = bg_div(
        "border:5px solid #000;padding:4px;\
         background:url(x) no-repeat;\
         background-origin:border-box;background-position:0 0",
    );
    let origin = list
        .items
        .iter()
        .find_map(|i| match i {
            DisplayItem::BackgroundImage { origin, .. } => Some(*origin),
            _ => None,
        })
        .expect("item fond émis");
    assert_eq!(origin, (0.0, 0.0), "origine au bord externe : {origin:?}");
}
