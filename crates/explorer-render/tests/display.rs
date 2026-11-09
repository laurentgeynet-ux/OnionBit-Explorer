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
fn fixed_echappe_au_clip_overflow_ancetre() {
    // Vol 26 (indeed) : un `position:fixed` ancré viewport ne
    // doit pas être rogné par le `overflow` d'un ancêtre —
    // son bloc conteneur est le viewport. La boîte fixe
    // `bottom:0` (y≈180 dans un viewport de 200) sort du
    // clip de l'ancêtre (50 px de haut) : son item doit être
    // émis avec une profondeur de clip nulle.
    let (_dom, _bx, items) = dl(
        "<div id=o><div id=f></div></div>",
        "#o{overflow:hidden;height:50px;position:relative} \
         #f{position:fixed;bottom:0;left:0;width:100px;height:20px;background-color:#ff0000}",
    );
    let mut depth = 0i32;
    let mut fixed_clip_depth = None;
    for it in &items {
        match it {
            DisplayItem::PushClip(_) => depth += 1,
            DisplayItem::PopClip => depth -= 1,
            DisplayItem::FillRect { rect, .. } if rect.y > 150.0 => {
                fixed_clip_depth = Some(depth);
            }
            _ => {}
        }
    }
    assert_eq!(
        fixed_clip_depth,
        Some(0),
        "le fond du `fixed` doit être émis hors de tout clip : {items:?}"
    );
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
    // R-187 : `circle()`/`polygon()`/`inset()`/`ellipse()`
    // produisent un clip ; `url()` et les formes inconnues
    // retombent sur « pas de clip » plutôt qu'un clip faux.
    let (_dom, _bx, items) = dl(
        "<div id=s>x</div>",
        "#s{position:absolute;width:20px;height:20px;\
         clip-path:circle(50%)}",
    );
    assert!(
        items
            .iter()
            .any(|i| matches!(i, DisplayItem::PushClip(r) if r.width <= 20.6)),
        "circle(50%) supporté → clip émis"
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
            dropped_emits: 0,
            first_cycle_path: None,
        },
        explorer_render::DisplayList {
            items: i2,
            width: 400.0,
            height: 200.0,
            spans: explorer_render::SpanMap::default(),
            tree_epoch: 0,
            dropped_emits: 0,
            first_cycle_path: None,
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
        dropped_emits: 0,
        first_cycle_path: None,
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
        dropped_emits: 0,
        first_cycle_path: None,
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
        dropped_emits: 0,
        first_cycle_path: None,
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
fn svg_inline_reraster_a_la_taille_utilisee() {
    // R-133 (§B-32/§B-36) : `<svg viewBox 24>` peint dans une
    // boîte 200×200 — le bitmap pré-rasterisé à la taille
    // naturelle s'étirait ×8 en blocs ; la source conservée
    // doit re-rasteriser à la destination (≈200 px).
    let dom = TreeBuilder::parse(
        "<svg viewBox=\"0 0 24 24\"><rect width=\"24\" height=\"24\" fill=\"red\"/></svg>",
    );
    let mut eng = StyleEngine::new();
    eng.add_css(
        "body{margin:0} svg{width:200px;height:200px;display:block}",
        Origin::Author,
    );
    let styles = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let root_el = dom
        .children(dom.document())
        .into_iter()
        .flatten()
        .find(|&c| dom.element(c).is_ok())
        .expect("racine");
    let sizes = explorer_render::svg_sizes(&dom, root_el);
    let (svgs, svg_sources) = explorer_render::collect_svg_with_sources(&dom, root_el, 1_000_000);
    assert_eq!(svgs.len(), 1, "un svg collecté");
    assert_eq!(svg_sources.len(), 1, "source conservée");
    let mut bx = build_box_tree_sized(&dom, root_el, &styles, &sizes).expect("arbre");
    layout_tree(&mut bx, VIEW, &ctx());
    let images = explorer_render::PaintImages {
        svgs,
        svg_sources,
        ..explorer_render::PaintImages::default()
    };
    let list = build_display_list_full(&bx, VIEW, 200.0, 0.0, &images);
    let img = list
        .items
        .iter()
        .find_map(|i| match i {
            DisplayItem::Image { img, .. } => Some(img.clone()),
            _ => None,
        })
        .expect("svg → item Image");
    assert!(
        img.width >= 150 && img.height >= 150,
        "re-raster à la taille utilisée (~200px), obtenu {}×{}",
        img.width,
        img.height
    );
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
fn sticky_peint_comme_positionne_pas_en_flux() {
    // R-134 (§B-36 wttj) : `position:sticky` est un positionné
    // (css-position — peint comme `relative`, App. E) — un
    // en-tête sticky à `z-index` doit peindre APRÈS une couche
    // absolue de section voisine, pas avant comme du flux.
    let (_dom, _bx, items) = dl(
        "<div id=\"st\">S</div><div id=\"ab\"></div>",
        "body{margin:0}#st{position:sticky;top:0;z-index:998;background:#0000ff;height:20px}\
         #ab{position:absolute;top:0;left:0;width:50px;height:50px;background:#ff0000}",
    );
    let idx_of = |r: u8, g: u8, b: u8| {
        items.iter().position(|i| {
            matches!(i, DisplayItem::FillRect { color, .. }
                if color.r == r && color.g == g && color.b == b)
        })
    };
    let st = idx_of(0, 0, 255).expect("fond sticky");
    let ab = idx_of(255, 0, 0).expect("fond absolu");
    assert!(
        st > ab,
        "sticky (z=998) doit peindre après l'absolu : st={st} ab={ab}"
    );
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
            dropped_emits: 0,
            first_cycle_path: None,
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
fn multicouche_url_seule_la_premiere_peint_limage_stockee() {
    // Mesuré : w3.org/TR/REC-html32 — `background:` à deux
    // couches `url()` (filigrane `no-repeat` + texture
    // `repeat`). La table `backgrounds` est clé `NodeId →
    // image` et la collecte ne fetch que le premier url :
    // la couche texture repeignait le filigrane en tuilage,
    // recouvrant tout le viewport. Seule la **première**
    // couche `url()` peut légitimement peindre l'image
    // stockée ; les suivantes sont sautées (dégradation
    // gracieuse — leurs urls seront fetchées par la session,
    // étape multi-couches complète).
    let list = bg_div("background:url(strip) no-repeat,url(tex)");
    let n = list
        .items
        .iter()
        .filter(|i| matches!(i, DisplayItem::BackgroundImage { .. }))
        .count();
    assert_eq!(
        n, 1,
        "seule la 1re couche url peint l'image stockée — la couche repeat doit être sautée"
    );
    let repeat = list
        .items
        .iter()
        .find_map(|i| match i {
            DisplayItem::BackgroundImage { repeat, .. } => Some(*repeat),
            _ => None,
        })
        .expect("item fond");
    assert_eq!(repeat, (false, false), "couche no-repeat conservée");
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
            DisplayItem::FillRect { rect, color, .. } if color.g > 200 => {
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

// ── R-107 : shorthand `background` multi-couches ───────────

#[test]
fn background_multicouche_paint_les_deux_couches() {
    // `background: url(spr) no-repeat, linear-gradient(…)` —
    // les longhands sont jointes par couche et `emit_bg_layers`
    // les consomme par couche : 2 items, le `no-repeat` de la
    // couche sprite préservé.
    let list = bg_div("background: url(a.png) no-repeat, linear-gradient(red, blue)");
    let items: Vec<&DisplayItem> = list
        .items
        .iter()
        .filter(|i| matches!(i, DisplayItem::BackgroundImage { .. }))
        .collect();
    assert_eq!(
        items.len(),
        2,
        "deux couches émises : {:?}",
        list.items.len()
    );
    // La couche url (première = au-dessus) garde son no-repeat.
    let url_item = items
        .iter()
        .find(|i| match i {
            DisplayItem::BackgroundImage { repeat, .. } => *repeat == (false, false),
            _ => false,
        })
        .expect("couche url no-repeat");
    let _ = url_item;
}

// ── R-108 : marqueurs textuels (decimal, alpha, roman) ─────

#[test]
fn markers_textuels_decimal_roman_emis() {
    // `ol` → `decimal` par défaut : 3 items = 3 MarkerText
    // "1." "2." "3." ; `upper-roman` → "I." "II." "III.".
    let (_d, _b, items) = dl("<ol><li>a</li><li>b</li><li>c</li></ol>", "body{margin:0}");
    let texts: Vec<&str> = items
        .iter()
        .filter_map(|i| match i {
            DisplayItem::MarkerText { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(texts, ["1.", "2.", "3."], "decimal par défaut");
    let (_d, _b, items) = dl(
        "<ol start=9 reversed><li>a</li><li value=4>b</li><li>c</li></ol>",
        "body{margin:0} ol{list-style-type:upper-roman}",
    );
    let texts: Vec<&str> = items
        .iter()
        .filter_map(|i| match i {
            DisplayItem::MarkerText { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    // reversed+start=9 : 9, puis value=4 ré-ancre, puis 3.
    assert_eq!(texts, ["IX.", "IV.", "III."], "romain + reversed/value");
}

#[test]
fn marker_list_style_string_litteral() {
    // R-112 — `list-style-type:"*"` : la chaîne EST le
    // marqueur (css-lists §3.3) — datatracker IETF peignait
    // `decimal` faute de `<string>` sur 41 listes.
    let (_d, _b, items) = dl(
        "<ul><li>a</li><li>b</li></ul>",
        "body{margin:0} ul{list-style-type:\"*\"}",
    );
    let texts: Vec<&str> = items
        .iter()
        .filter_map(|i| match i {
            DisplayItem::MarkerText { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(texts, ["*", "*"], "chaîne littérale comme marqueur");
}

/// Vol 2026-10-26 — `opacity: 0` : tout le sous-arbre est
/// invisible (modales `opacity-0` bookshop/ecma peintes en
/// plein). Un élément `opacity:0` n'émet RIEN, enfants
/// compris (l'opacité se compose multiplicativement — 0 ne
/// se relève jamais, contrairement à `visibility`).
#[test]
fn opacity_zero_nemet_rien() {
    let (_d, _b, items) = dl(
        "<div id=v>visible</div><div id=o>cache</div>",
        "body{margin:0} #o{opacity:0;background-color:#ff0000}",
    );
    let n_text = items
        .iter()
        .filter(|i| matches!(i, DisplayItem::Text { .. } | DisplayItem::MarkerText { .. }))
        .count();
    let n_fill = items
        .iter()
        .filter(|i| matches!(i, DisplayItem::FillRect { .. }))
        .count();
    assert_eq!(n_text, 1, "seul 'visible' peint : {items:?}");
    assert_eq!(n_fill, 0, "fond de la modale invisible : {items:?}");
}

/// `opacity` positive (0<o<1, MVP non appliquée) : la boîte
/// reste peinte — seul `0` court-circuite.
#[test]
fn opacity_partielle_ne_court_circuite_pas() {
    let (_d, _b, items) = dl("<div id=o>semi</div>", "body{margin:0} #o{opacity:0.4}");
    assert!(
        items
            .iter()
            .any(|i| matches!(i, DisplayItem::Text { .. } | DisplayItem::MarkerText { .. })),
        "opacité partielle : contenu peint : {items:?}"
    );
}

/// `<use href="#id">` cross-îlot : le `<symbol>` vit dans un
/// `<svg>` séparé (pattern icônes/defs de début de document —
/// logo reddit aplati). L'usage doit rasteriser la cible.
#[test]
fn svg_use_cross_ilot_resolu() {
    let dom = TreeBuilder::parse(
        "<svg width=\"40\" height=\"40\"><use href=\"#sym\"/></svg>\
         <svg style=\"display:none\"><symbol id=\"sym\">\
         <rect width=\"40\" height=\"40\" fill=\"#00ff00\"/></symbol></svg>",
    );
    let mut eng = StyleEngine::new();
    eng.add_css("body{margin:0}", Origin::Author);
    let root_el = dom
        .children(dom.document())
        .into_iter()
        .flatten()
        .find(|&c| dom.element(c).is_ok())
        .expect("racine");
    let svgs = explorer_render::collect_svg(&dom, root_el, 1_000_000);
    // L'îlot `<use>` produit une image (le symbol définit
    // viewBox par défaut — usvg résout le contenu injecté).
    assert!(!svgs.is_empty(), "aucun svg rasterisé");
}

/// P3 — `<use>` cross-îlots transitif : la cible injectée
/// (`<symbol>` dans un autre îlot) portait ses propres refs
/// non résolues (îlot vide quand même). Le fixpoint borné
/// rescanne les cibles injectées. `xlink:href` sans
/// `xmlns:xlink` rendait tout l'îlot XML-invalide.
#[test]
fn svg_use_cross_ilot_transitif() {
    let dom = TreeBuilder::parse(
        "<svg id=defs style=\"display:none\">\
         <symbol id=sym><path id=p1 d=\"M0 0h10v10z\"/><use href=\"#inner\"/></symbol>\
         <path id=inner d=\"M1 1h2v2z\"/>\
         </svg>\
         <svg id=u width=32 height=32><use href=\"#sym\"/></svg>",
    );
    let mut u = None;
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if let Ok(id) = dom.attribute(n, "id") {
            if id.as_deref() == Some("u") {
                u = Some(n);
            }
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
        }
    }
    let u = u.expect("svg u");
    let root_el = dom
        .children(dom.document())
        .into_iter()
        .flatten()
        .find(|&c| dom.element(c).is_ok())
        .expect("racine");
    let (_imgs, sources) = explorer_render::collect_svg_with_sources(&dom, root_el, 256);
    let src = sources.get(&u).expect("source svg u");
    // sym ET inner transitif + p1 (contenu de sym).
    assert!(src.contains("id=\"sym\""), "symbol injecté : {src}");
    assert!(src.contains("id=\"inner\""), "ref transitive : {src}");
    assert!(src.contains("id=\"p1\""), "contenu symbol : {src}");
}

// ── R-153 : `text-decoration` peint ─────────────────────────

/// Runs texte `(origine_x, baseline_y, encre)` de la liste.
fn texts(items: &[DisplayItem]) -> Vec<(f32, f32, explorer_css::value::Color)> {
    items
        .iter()
        .filter_map(|i| match i {
            DisplayItem::Text { origin, color, .. } => Some((origin.0, origin.1, *color)),
            _ => None,
        })
        .collect()
}

/// `FillRect` de soulignement : rect plein dont le haut tombe
/// entre la baseline `b` et `b + 8` px (sous l'encre).
fn underlines(items: &[DisplayItem], baseline: f32) -> Vec<(f32, explorer_css::value::Color)> {
    items
        .iter()
        .filter_map(|i| match i {
            DisplayItem::FillRect { rect, color, .. }
            | DisplayItem::DecoRect { rect, color, .. }
                if rect.y >= baseline - 0.5 && rect.y <= baseline + 8.0 =>
            {
                Some((rect.x, *color))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn text_decoration_u_souligne_sous_la_baseline() {
    // R-153 : `u` (UA underline) produit un FillRect fin sous
    // la baseline de son run — avant : rien n'était peint.
    let (_dom, _bx, items) = dl("<p><u>hello</u></p>", "p{margin:0}");
    let t = texts(&items);
    assert_eq!(t.len(), 1, "un run attendu : {t:?}");
    let u = underlines(&items, t[0].1);
    assert_eq!(u.len(), 1, "un souligné attendu : {u:?}");
}

#[test]
fn text_decoration_couleur_du_declarant_pas_du_fragment() {
    // §16.3.1 (piège du lead) : la déco enjambe les inlines
    // descendants avec la couleur de l'élément DÉCLARANT —
    // `<a>` rouge souligne le `<b>` bleu EN ROUGE. Un « le
    // fragment peint sa propre déco » naïf donnerait bleu.
    let (_dom, _bx, items) = dl(
        "<p><a href=\"#\"><b>x</b></a></p>",
        "p{margin:0} a{color:#ff0000} b{color:#0000ff}",
    );
    let t = texts(&items);
    assert_eq!(t.len(), 1, "un run attendu : {t:?}");
    assert_eq!((t[0].2.r, t[0].2.b), (0, 255), "encre = bleu b");
    let u = underlines(&items, t[0].1);
    assert_eq!(u.len(), 1, "un souligné attendu : {u:?}");
    assert_eq!(
        (u[0].1.r, u[0].1.g, u[0].1.b),
        (255, 0, 0),
        "souligné = rouge de `a` (déclarant), pas bleu de `b`"
    );
}

#[test]
fn text_decoration_saute_les_remplaces() {
    // §16.3.1 : la propagation saute les atomiques —
    // l'inline-block descendant d'un `<a>` n'est pas souligné
    // par la déco du lien (un seul FillRect : celui de « a »).
    let (_dom, _bx, items) = dl(
        "<p><a href=\"#\">a<span class=\"i\">b</span></a></p>",
        "p{margin:0} .i{display:inline-block}",
    );
    let t = texts(&items);
    let rects: Vec<_> = items
        .iter()
        .filter(|i| {
            matches!(
                i,
                DisplayItem::FillRect { .. } | DisplayItem::DecoRect { .. }
            )
        })
        .collect();
    assert_eq!(t.len(), 2, "runs « a » + « b » : {t:?}");
    assert_eq!(rects.len(), 1, "un seul souligné : {rects:?}");
}

#[test]
fn text_decoration_del_barre_apres_l_encre() {
    // `del` → line-through : le FillRect est AU-DESSUS de la
    // baseline et émis APRÈS le Text (il barre les glyphes).
    let (_dom, _bx, items) = dl("<p><del>x</del></p>", "p{margin:0}");
    let t = texts(&items);
    assert_eq!(t.len(), 1, "un run attendu : {t:?}");
    let pos_text = items
        .iter()
        .position(|i| matches!(i, DisplayItem::Text { .. }))
        .expect("texte");
    let pos_strike = items
        .iter()
        .position(|i| match i {
            DisplayItem::FillRect { rect, .. } | DisplayItem::DecoRect { rect, .. } => {
                rect.y < t[0].1
            }
            _ => false,
        })
        .expect("barré au-dessus de la baseline");
    assert!(pos_strike > pos_text, "le barré peint après l'encre");
}

#[test]
fn text_decoration_enjambe_depuis_le_conteneur_bloc() {
    // §16.3.1 : `text-decoration` posée sur le bloc conteneur
    // enjambe son propre contenu en-ligne (seed de la pile
    // à `collect_items`) — `<div style=…>x</div>` souligné.
    let (_dom, _bx, items) = dl(
        "<div style=\"text-decoration:underline\">x</div>",
        "div{margin:0}",
    );
    let t = texts(&items);
    assert_eq!(t.len(), 1, "un run attendu : {t:?}");
    assert_eq!(underlines(&items, t[0].1).len(), 1, "un souligné");
}

#[test]
fn empty_cells_hide_ne_peint_ni_fond_ni_bordure() {
    // R-164 — `empty-cells:hide` : sous le modèle `separate`,
    // une cellule vide ne peint ni fond ni bordure (§17.6.1.1)
    // ; `&nbsp;` compte comme contenu. `collapse` → inerte.
    let (dom, _bx, items) = dl(
        "<table><tr><td id=pleine>x</td><td id=vide></td>\
         <td id=nbsp>&nbsp;</td></tr></table>",
        "table { empty-cells: hide; border-spacing: 4px } \
         td { border: 2px solid black; background: red; width: 40px }",
    );
    // 2 cellules peinent (pleine + nbsp) ; la vide saute.
    let borders = items
        .iter()
        .filter(|i| matches!(i, DisplayItem::Border { .. }))
        .count();
    assert_eq!(borders, 2, "cellules bordées peintes : {borders}");
    // Contrôle `collapse` : la cellule vide re-peint (modèle
    // hors périmètre de empty-cells).
    let (_d2, _b2, items2) = dl(
        "<table><tr><td id=pleine>x</td><td id=vide></td></tr></table>",
        "table { empty-cells: hide; border-collapse: collapse } \
         td { border: 2px solid black }",
    );
    let borders2 = items2
        .iter()
        .filter(|i| matches!(i, DisplayItem::Border { .. }))
        .count();
    assert_eq!(borders2, 2, "collapse : empty-cells inerte : {borders2}");
    let _ = dom;
}

#[test]
fn styles_3d_mappees_aux_variantes_reelles() {
    // R-166 — `inset`/`outset`/`groove`/`ridge`/`double`
    // ne sont plus rabattus sur `Double` : la display list
    // porte la variante réelle pour le bevel par côté.
    let cases = [
        ("outset", BorderStyle::Outset),
        ("inset", BorderStyle::Inset),
        ("groove", BorderStyle::Groove),
        ("ridge", BorderStyle::Ridge),
        ("double", BorderStyle::Double),
        ("solid", BorderStyle::Solid),
    ];
    for (kw, want) in cases {
        let (_d, _b, items) = dl(
            "<div>x</div>",
            &format!("div {{ border: 6px {kw} #888; width: 40px }}"),
        );
        let got = items.iter().find_map(|i| match i {
            DisplayItem::Border { styles, .. } => Some(styles[0]),
            _ => None,
        });
        assert_eq!(got, Some(want), "{kw}");
    }
}

#[test]
fn border_radius_shorthand_et_radii_sur_items() {
    // R-167 — `border-radius` éclate en 4 coins, arrive en
    // `radii` sur `Border` ET `FillRect` (fond arrondi) ;
    // `%` du plus petit côté, borné à la moitié.
    let (_d, _b, items) = dl(
        "<div>x</div>",
        "div { border: 4px solid black; background: #ccc; \
         border-radius: 10px 20px; width: 80px; height: 60px }",
    );
    let border = items.iter().find_map(|i| match i {
        DisplayItem::Border { radii, .. } => Some(*radii),
        _ => None,
    });
    assert_eq!(
        border,
        Some([10.0, 20.0, 10.0, 20.0]),
        "radii border : {border:?}"
    );
    let fill = items.iter().find_map(|i| match i {
        DisplayItem::FillRect { radii, .. } => Some(*radii),
        _ => None,
    });
    assert_eq!(
        fill,
        Some([10.0, 20.0, 10.0, 20.0]),
        "radii fond : {fill:?}"
    );
    // `50%` → moitié du plus petit côté (60→30px).
    let (_d2, _b2, items2) = dl(
        "<div>x</div>",
        "div { border: 2px solid black; border-radius: 50%; \
         width: 80px; height: 60px }",
    );
    let r2 = items2.iter().find_map(|i| match i {
        DisplayItem::Border { radii, .. } => Some(*radii),
        _ => None,
    });
    assert_eq!(r2, Some([32.0; 4]), "50% : {r2:?}");
}

#[test]
fn box_shadow_parsed_sur_item() {
    // R-M2 — `box-shadow` → `DisplayItem::BoxShadow` par
    // couche : offset/blur/spread/color ; `inset` sauté
    // (approx. documentée) ; `none` → rien.
    let (_d, _b, items) = dl(
        "<div>x</div>",
        "div { width: 40px; height: 20px; \
         box-shadow: 4px 4px 8px 2px rgba(255,0,0,.5), -2px -2px blue }",
    );
    let mut sh = items.iter().filter_map(|i| match i {
        DisplayItem::BoxShadow {
            offset,
            blur,
            spread,
            color,
            ..
        } => Some((*offset, *blur, *spread, *color)),
        _ => None,
    });
    let a = sh.next().expect("ombre 1");
    assert_eq!(a.0, (4.0, 4.0));
    assert!((a.1 - 8.0).abs() < 0.01 && (a.2 - 2.0).abs() < 0.01);
    assert_eq!((a.3.r, a.3.g, a.3.b), (255, 0, 0));
    let b = sh.next().expect("ombre 2");
    assert_eq!(b.0, (-2.0, -2.0));
    let (_d2, _b2, items2) = dl("<div>x</div>", "div { box-shadow: none }");
    assert!(
        !items2
            .iter()
            .any(|i| matches!(i, DisplayItem::BoxShadow { .. })),
        "none → aucun item"
    );
}

#[test]
fn text_shadow_fantomes_sous_texte() {
    // R-M4 — `text-shadow: 3px 2px red` : une run fantôme
    // décalée (origin+offset, couleur de l'ombre) émise
    // AVANT le texte réel ; `none` → aucun fantôme.
    let (_d, _b, items) = dl(
        "<div>ombre</div>",
        "div { font-size: 20px; text-shadow: 3px 2px red }",
    );
    let texts: Vec<_> = items
        .iter()
        .filter_map(|i| match i {
            DisplayItem::Text { origin, color, .. } => Some((*origin, *color)),
            _ => None,
        })
        .collect();
    assert!(texts.len() >= 2, "fantôme + texte attendus : {texts:?}");
    let ghost = &texts[0];
    let real = &texts[1];
    assert_eq!(ghost.1.r, 255);
    assert!(
        (ghost.0 .0 - real.0 .0 - 3.0).abs() < 0.5 && (ghost.0 .1 - real.0 .1 - 2.0).abs() < 0.5,
        "offset fantôme : {ghost:?} vs {real:?}"
    );
    let (_d2, _b2, items2) = dl("<div>x</div>", "div { text-shadow: none }");
    assert_eq!(
        items2
            .iter()
            .filter(|i| matches!(i, DisplayItem::Text { .. }))
            .count(),
        1,
        "none → texte seul"
    );
}

#[test]
fn filter_bracket_et_ops_parsed() {
    // R-M5 — `filter:blur(4px) brightness(2)` →
    // `PushFilter([Blur(4),Brightness(2)])`…`PopFilter`
    // encadrant le sous-arbre ; `none` → rien.
    let (_d, _b, items) = dl(
        "<div>x</div>",
        "div { filter: blur(4px) brightness(2) grayscale(50%) }",
    );
    let ops = items.iter().find_map(|i| match i {
        DisplayItem::PushFilter(ops) => Some(ops),
        _ => None,
    });
    match ops {
        Some(ops) => {
            assert_eq!(ops.len(), 3, "ops : {ops:?}");
            assert_eq!(ops[0], explorer_render::display::FilterOp::Blur(4.0));
            assert_eq!(ops[1], explorer_render::display::FilterOp::Brightness(2.0));
            assert_eq!(ops[2], explorer_render::display::FilterOp::Grayscale(0.5));
        }
        None => panic!("pas de PushFilter : {items:?}"),
    }
    assert!(
        items.iter().any(|i| matches!(i, DisplayItem::PopFilter)),
        "PopFilter manquant"
    );
    let (_d2, _b2, items2) = dl("<div>x</div>", "div { filter: none }");
    assert!(
        !items2
            .iter()
            .any(|i| matches!(i, DisplayItem::PushFilter(_))),
        "none → aucun bracket"
    );
}

#[test]
fn overflow_arrondi_emet_pushcliprounded() {
    // R-M9 — `overflow:hidden` + `border-radius` : le clip
    // porte les rayons (`PushClipRounded`) au lieu du
    // `PushClip` rectangulaire (le contenu ne déborde plus
    // des coins).
    let (_d, _b, items) = dl(
        "<div>x</div>",
        "div { overflow: hidden; border-radius: 8px; \
         width: 30px; height: 10px }",
    );
    let radii = items.iter().find_map(|i| match i {
        DisplayItem::PushClipRounded { radii, .. } => Some(*radii),
        _ => None,
    });
    assert_eq!(radii, Some([5.0; 4]), "radii clip : {radii:?}");
    assert!(
        !items.iter().any(|i| matches!(i, DisplayItem::PushClip(_))),
        "PushClip rect en trop"
    );
    // Sans rayon : `PushClip` rect inchangé.
    let (_d2, _b2, items2) = dl(
        "<div>x</div>",
        "div { overflow: hidden; width: 30px; height: 10px }",
    );
    assert!(
        items2.iter().any(|i| matches!(i, DisplayItem::PushClip(_))),
        "PushClip attendu sans radius"
    );
}

#[test]
fn backdrop_filter_item_et_ops() {
    // R-M9b — `backdrop-filter:blur(6px) grayscale(1)` →
    // `BackdropFilter {rect, ops}` émis avant le fond de la
    // boîte (les pixels peints sous `rect` reçoivent les
    // ops au raster, le contenu se peint par-dessus) ;
    // `none` → rien.
    let (_d, _b, items) = dl(
        "<div>x</div>",
        "div { width: 40px; height: 20px; \
         backdrop-filter: blur(6px) grayscale(1) }",
    );
    let pos = items
        .iter()
        .position(|i| matches!(i, DisplayItem::BackdropFilter { .. }));
    assert!(pos.is_some(), "BackdropFilter absent : {items:?}");
    let p = pos.unwrap();
    match &items[p] {
        DisplayItem::BackdropFilter { ops, .. } => {
            assert_eq!(ops.len(), 2, "ops : {ops:?}");
            assert_eq!(ops[0], explorer_render::display::FilterOp::Blur(6.0));
            assert_eq!(ops[1], explorer_render::display::FilterOp::Grayscale(1.0));
        }
        _ => unreachable!(),
    }
    // Émis avant le fond (fond lui suit).
    let bg = items.iter().position(|i| {
        matches!(
            i,
            DisplayItem::FillRect { .. } | DisplayItem::BoxShadow { .. }
        )
    });
    assert!(bg.is_none_or(|b| b > p), "ordre : filtre avant fond");
    let (_d2, _b2, items2) = dl("<div>x</div>", "div { backdrop-filter: none }");
    assert!(
        !items2
            .iter()
            .any(|i| matches!(i, DisplayItem::BackdropFilter { .. })),
        "none → aucun item"
    );
}

/// R-M11 — `drop-shadow()` parse `dx dy [blur] [color]` :
/// op portée par `PushFilter` comme les autres filtres ;
/// défauts blur=0, couleur = encre (`Color::BLACK` en
/// l'absence d'encre propre à l'ombre — approximation
/// `currentcolor` documentée).
#[test]
fn drop_shadow_parsee_et_bracket() {
    let (_d, _b, items) = dl(
        "<div>x</div>",
        "div { filter: drop-shadow(3px 4px 5px rgb(255, 0, 0)) }",
    );
    let ops = items.iter().find_map(|i| match i {
        DisplayItem::PushFilter(ops) => Some(ops),
        _ => None,
    });
    match ops {
        Some(ops) => {
            assert_eq!(ops.len(), 1, "ops : {ops:?}");
            assert_eq!(
                ops[0],
                explorer_render::display::FilterOp::DropShadow {
                    dx: 3.0,
                    dy: 4.0,
                    blur: 5.0,
                    color: explorer_css::value::Color {
                        r: 255,
                        g: 0,
                        b: 0,
                        a: 255,
                    },
                }
            );
        }
        None => panic!("pas de PushFilter : {items:?}"),
    }
    // Défauts : blur 0, couleur noire.
    let (_d2, _b2, items2) = dl("<div>x</div>", "div { filter: drop-shadow(1px 2px) }");
    let ops2 = items2.iter().find_map(|i| match i {
        DisplayItem::PushFilter(ops) => Some(ops),
        _ => None,
    });
    match ops2 {
        Some(ops) => {
            assert_eq!(
                ops[0],
                explorer_render::display::FilterOp::DropShadow {
                    dx: 1.0,
                    dy: 2.0,
                    blur: 0.0,
                    color: explorer_css::value::Color::BLACK,
                }
            );
        }
        None => panic!("pas de PushFilter"),
    }
}

/// R-M13 — `outline` : anneau `Border` émis hors border
/// box, offset + width, sans emprise au layout ; `none`
/// inerte ; shorthand `outline:` expansé.
#[test]
fn outline_emis_hors_border_box() {
    let (_d, _b, items) = dl(
        "<div>x</div>",
        "div { width: 100px; height: 50px; outline: 2px solid rgb(255, 0, 0) }",
    );
    let b = items
        .iter()
        .find(|i| matches!(i, DisplayItem::Border { .. }));
    match b {
        Some(DisplayItem::Border {
            rect,
            widths,
            colors,
            styles,
            ..
        }) => {
            assert_eq!(widths, &[2.0; 4], "outline-width uniforme : {widths:?}");
            assert_eq!(rect.x, -2.0, "rect étendu de l'offset+width : {rect:?}");
            assert_eq!(rect.width, 104.0, "largeur étendue : {rect:?}");
            assert_eq!(colors[0].r, 255, "outline-color");
            assert!(
                styles
                    .iter()
                    .all(|s| *s != explorer_render::display::BorderStyle::None),
                "styles peints : {styles:?}"
            );
        }
        _ => panic!("pas de Border outline : {items:?}"),
    }
    // `none` → rien.
    let (_d2, _b2, items2) = dl(
        "<div>x</div>",
        "div { width: 100px; height: 50px; outline-style: none }",
    );
    assert!(
        !items2
            .iter()
            .any(|i| matches!(i, DisplayItem::Border { .. })),
        "none → aucun item"
    );
}

/// R-178 — `text-decoration` styles : `dotted`/`dashed`/
/// `double`/`wavy` sortent en `DecoRect` (patterns raster,
/// plus de plein `solid`), `text-decoration-thickness` et
/// `text-underline-offset` résolus contre la fonte du
/// déclarant. `skip-ink` hors scope (géométrie des glyphes).
#[test]
fn text_deco_styles_thickness_offset() {
    let (_, _, items) = dl(
        "<p><u id=a>soul</u><s id=b>barre</s><i id=c>vague</i></p>",
        "u{text-decoration:underline wavy;text-decoration-thickness:2px;\
           text-underline-offset:4px}\
         s{text-decoration:line-through dashed}\
         i{text-decoration:underline dotted;font-style:normal}",
    );
    let deco = |style: u8| -> &DisplayItem {
        items
            .iter()
            .find(|i| matches!(i, DisplayItem::DecoRect { style: s, .. } if *s == style))
            .expect("deco item")
    };
    // `wavy` : DECO_WAVY = 4, épaisseur 2 px, offset > repli.
    let w = deco(explorer_layout::inline::DECO_WAVY);
    if let DisplayItem::DecoRect { rect, .. } = w {
        assert!(
            (rect.height - 2.0).abs() < 0.5,
            "thickness 2px : {}",
            rect.height
        );
    }
    // `dashed`/`dotted` : variantes présentes.
    let _ = deco(explorer_layout::inline::DECO_DASHED);
    let _ = deco(explorer_layout::inline::DECO_DOTTED);
    // Plus de FillRect plein pour les décors (dégradé = re-test).
    let fills = items
        .iter()
        .filter(|i| matches!(i, DisplayItem::DecoRect { .. }))
        .count();
    assert_eq!(fills, 3, "3 traits décorés : {fills}");
}

/// R-181 — `column-rule` : un `DecoRect` vertical centré dans
/// le gap, au-dessus du fond (§5). Shorthand + longhands.
#[test]
fn column_rule_peint_un_trait_dans_le_gap() {
    let (_, _, items) = dl(
        "<div id=c><p>a</p><p>b</p></div>",
        "#c{column-count:2;column-gap:10px;width:210px;\
           column-rule:2px dashed #ff0000;margin:0}",
    );
    let rules: Vec<_> = items
        .iter()
        .filter(|i| {
            matches!(
                i,
                DisplayItem::DecoRect {
                    style: explorer_layout::inline::DECO_DASHED,
                    ..
                }
            )
        })
        .collect();
    assert_eq!(rules.len(), 1, "un trait de colonne : {rules:?}");
    if let DisplayItem::DecoRect { rect, color, .. } = rules[0] {
        // Centré dans le gap : x ≈ 104 (col 100 + gap 10/2 - 1).
        assert!((rect.x - 104.0).abs() < 6.0, "x du trait : {}", rect.x);
        assert!((rect.width - 2.0).abs() < 0.5, "épaisseur : {}", rect.width);
        assert_eq!((color.r, color.g, color.b), (255, 0, 0));
    }
    // Sans `column-rule-style` → rien.
    let (_, _, items2) = dl(
        "<div id=c><p>a</p><p>b</p></div>",
        "#c{column-count:2;width:210px;margin:0}",
    );
    assert!(
        !items2
            .iter()
            .any(|i| matches!(i, DisplayItem::DecoRect { .. })),
        "pas de trait par défaut"
    );
}

/// R-183 — `writing-mode: vertical-rl` : le sous-arbre
/// transposé est remis dans le repère parent par un
/// `PushTransform` de rotation — pas de double pivot.
#[test]
fn writing_mode_emet_push_transform() {
    let (_, _, items) = dl(
        "<div style=\"writing-mode:vertical-rl;width:200px;height:100px\">\
         <div style=\"background:red\">A</div></div>",
        "",
    );
    // Au moins un transform de repère de flux (rotation
    // ±90° = coefficients croisés non nuls, diagonale nulle).
    assert!(
        items.iter().any(|it| matches!(
            it,
            DisplayItem::PushTransform(t)
                if t.sx.abs() < 0.01 && t.sy.abs() < 0.01
                    && t.kx.abs() > 0.9 && t.ky.abs() > 0.9
        )),
        "transform de flux attendu dans {items:?}"
    );
}

/// R-184 — `background-blend-mode` : tag porté par
/// `BackgroundImage::blend` (0 = normal, index+1 =
/// `BLEND_MODES`) ; `background-clip:text` émet la couche
/// une fois par fragment de ligne (`PushClip`).
#[test]
fn background_blend_et_clip_text() {
    use explorer_render::DisplayItem;
    let (_, _, items) = dl(
        "<div style=\"width:100px;height:40px;\
         background:linear-gradient(to right,red,blue) luminosity,\
                   url(data:image/png;base64,) multiply;\
         background-clip:text;border-radius:0\">\
         <span>AB</span></div>",
        "",
    );
    // Une couche au moins porte blend ≠ 0, et clip:text →
    // plusieurs PushClip (un par fragment).
    assert!(
        items.iter().any(|it| matches!(
            it,
            DisplayItem::BackgroundImage { blend, .. } if *blend > 0
        )),
        "blend attendu dans {items:?}"
    );
    let clips = items
        .iter()
        .filter(|it| matches!(it, DisplayItem::PushClip(_)))
        .count();
    assert!(clips >= 1, "PushClip de clip:text attendu");
}

/// R-185 — `mix-blend-mode`/`isolation`/`will-change`/
/// `contain` : `PushBlend`/`PopBlend` encadrent le
/// sous-arbre fondu ; `contain:paint` ajoute un clip.
#[test]
fn mix_blend_isolation_contain() {
    use explorer_render::DisplayItem;
    let (_, _, items) = dl(
        "<div style=\"mix-blend-mode:multiply\">\
         <div style=\"isolation:isolate;contain:paint\">\
         <span>A</span></div></div>",
        "",
    );
    let pushes = items
        .iter()
        .filter(|it| matches!(it, DisplayItem::PushBlend(_)))
        .count();
    let pops = items
        .iter()
        .filter(|it| matches!(it, DisplayItem::PopBlend))
        .count();
    assert_eq!(pushes, pops, "brackets PushBlend/PopBlend appairés");
    assert!(
        pushes >= 2,
        "mix-blend + isolation : 2 groupes attendus, {pushes}"
    );
    assert!(
        items
            .iter()
            .any(|it| matches!(it, DisplayItem::PushClip(_))),
        "contain:paint → PushClip attendu"
    );
}

/// R-186 — typo longue traîne : `text-emphasis` émet un
/// item `Emphasis` par run (marque par glyphe au raster) ;
/// `font-variant`/`font-kerning`/`font-stretch`/`font-size-
/// adjust`/`font-synthesis`/`text-spacing-trim`/
/// `hanging-punctuation` parsés et hérités.
#[test]
fn text_emphasis_emet_item() {
    use explorer_render::DisplayItem;
    let (_, _, items) = dl("<p style=\"text-emphasis:filled sesame red\">AB</p>", "");
    assert!(
        items
            .iter()
            .any(|it| matches!(it, DisplayItem::Emphasis { kind: 4, .. })),
        "Emphasis sesame attendu dans {items:?}"
    );
}

/// R-187 — `clip-path` : `polygon` → `PushClipPath` réel
/// (mask `fill_path`), `inset`/`circle`/`ellipse` →
/// `PushClip` rectangulaire ; `filter:url()` parsé,
/// ressource non résolue → no-op documenté.
#[test]
fn clip_path_polygon_emet_clip() {
    use explorer_render::DisplayItem;
    let (_, _, items) = dl(
        "<div style=\"clip-path:polygon(0% 0%,100% 0%,50% 100%);\
         background:red\">A</div>",
        "",
    );
    assert!(
        items.iter().any(|it| matches!(
            it,
            DisplayItem::PushClipPath { points } if points.len() == 3
        )),
        "PushClipPath attendu dans {items:?}"
    );
}

/// R-188 — transforms 3D : `rotateX`/`rotateY`/`rotateZ`/
/// `perspective` → projection affine ; `backface-visibility:
/// hidden` + dos tourné (rotateY 180°) → non peint.
#[test]
fn rotate3d_projete_affine_et_backface() {
    use explorer_render::DisplayItem;
    // rotateX(30deg) → compression Y (d=cos30≈0.87).
    let (_, _, items) = dl("<div style=\"transform:rotateX(30deg)\">A</div>", "");
    assert!(
        items.iter().any(|i| matches!(
            i, DisplayItem::PushTransform(m) if (m.sy - 0.866).abs() < 0.02)),
        "rotateX projeté attendu"
    );
    // backface-visibility:hidden + rotateY(180deg) → non peint.
    let (_, _, items) = dl(
        "<div style=\"transform:rotateY(180deg);\
         backface-visibility:hidden\">B</div>",
        "",
    );
    assert!(
        !items.iter().any(|i| matches!(i, DisplayItem::Text { .. })),
        "dos tourné → pas de texte peint"
    );
}

/// R-187 fix — `inset` : les longueurs `px` sont absolues,
/// pas des % (régression post-revue : `10px` lu `10%`).
#[test]
fn clip_path_inset_px_absolu() {
    use explorer_render::DisplayItem;
    let (_, _, items) = dl(
        "<div id=s>x</div>",
        "#s{position:absolute;width:200px;height:200px;\
         clip-path:inset(10px)}",
    );
    assert!(
        items.iter().any(|i| matches!(
            i,
            DisplayItem::PushClip(r) if (r.width - 180.0).abs() < 0.5
        )),
        "inset(10px) sur 200px → clip 180px dans {items:?}"
    );
}

/// R-187 fix — `ellipse(rx ry)` lit les deux rayons ;
/// `circle(<px>)` accepte les longueurs absolues.
#[test]
fn clip_path_ellipse_deux_rayons() {
    use explorer_render::DisplayItem;
    let (_, _, items) = dl(
        "<div id=s>x</div>",
        "#s{position:absolute;width:100px;height:100px;\
         clip-path:ellipse(30% 60%)}",
    );
    assert!(
        items.iter().any(|i| matches!(
            i,
            DisplayItem::PushClip(r) if (r.width - 60.0).abs() < 0.5
                && (r.height - 120.0).abs() < 0.5
        )),
        "ellipse(30% 60%) → clip 60x120 dans {items:?}"
    );
    let (_, _, items) = dl(
        "<div id=s>x</div>",
        "#s{position:absolute;width:200px;height:200px;\
         clip-path:circle(10px)}",
    );
    assert!(
        items.iter().any(|i| matches!(
            i,
            DisplayItem::PushClip(r) if (r.width - 20.0).abs() < 0.5
        )),
        "circle(10px) → clip 20px dans {items:?}"
    );
}

/// R-187 fix — `inset(... round <r>)` → `PushClipRounded`.
#[test]
fn clip_path_inset_round_arrondi() {
    use explorer_render::DisplayItem;
    let (_, _, items) = dl(
        "<div id=s>x</div>",
        "#s{position:absolute;width:100px;height:100px;\
         clip-path:inset(10% round 5px)}",
    );
    assert!(
        items.iter().any(|i| matches!(
            i,
            DisplayItem::PushClipRounded { radii, .. }
                if (radii[0] - 5.0).abs() < 0.01
        )),
        "PushClipRounded attendu dans {items:?}"
    );
}

/// R-188 fix — `backface_hidden` s'évalue AVANT tout push :
/// `contain:paint` + dos tourné ne doit laisser aucun
/// `PushClip` orphelin (pile raster déséquilibrée).
#[test]
fn backface_hidden_laisse_la_pile_equilibree() {
    use explorer_render::DisplayItem;
    let (_, _, items) = dl(
        "<div style=\"contain:paint;transform:rotateY(180deg);\
         backface-visibility:hidden\">A</div>",
        "",
    );
    let pushes = items
        .iter()
        .filter(|i| {
            matches!(
                i,
                DisplayItem::PushClip(_)
                    | DisplayItem::PushClipRounded { .. }
                    | DisplayItem::PushClipPath { .. }
            )
        })
        .count();
    let pops = items
        .iter()
        .filter(|i| matches!(i, DisplayItem::PopClip))
        .count();
    assert_eq!(pushes, pops, "clips déséquilibrés dans {items:?}");
}

/// R-188 — propriétés individuelles `translate`/`rotate`/
/// `scale` (CSS Transforms 2) consommées par `box_transform`.
#[test]
fn props_transform_individuelles() {
    use explorer_render::DisplayItem;
    let (_, _, items) = dl("<div style=\"scale:2\">A</div>", "");
    assert!(
        items.iter().any(|i| matches!(
            i, DisplayItem::PushTransform(m) if (m.sx - 2.0).abs() < 0.01)),
        "scale:2 → sx=2 dans {items:?}"
    );
    let (_, _, items) = dl("<div style=\"rotate:x 90deg\">A</div>", "");
    assert!(
        items.iter().any(|i| matches!(
            i, DisplayItem::PushTransform(m) if m.sy.abs() < 0.01)),
        "rotate:x 90deg → sy≈0 dans {items:?}"
    );
    let (_, _, items) = dl("<div style=\"translate:10px 5px\">A</div>", "");
    assert!(
        items.iter().any(|i| matches!(
            i, DisplayItem::PushTransform(m) if (m.tx - 10.0).abs() < 0.5)),
        "translate:10px 5px → tx≈10 dans {items:?}"
    );
}

/// R-188 suite — `transform-box` : les `%` de la fonction
/// et `transform-origin` se résolvent contre la boîte de
/// référence choisie (`content-box` = moins bordure+padding,
/// `border-box` = défaut).
#[test]
fn transform_box_content_box_change_la_reference() {
    use explorer_render::DisplayItem;
    let tx_of = |items: &[DisplayItem]| -> f32 {
        items
            .iter()
            .find_map(|i| match i {
                DisplayItem::PushTransform(m) => Some(m.tx),
                _ => None,
            })
            .unwrap_or(f32::NAN)
    };
    // `translate(50%,0)` : % contre la boîte de référence —
    // `content-box` = border-box moins les 20px de bordures
    // → tx décalé de 10px, indépendamment du box-sizing.
    let (_, _, bb) = dl(
        "<div id=s>x</div>",
        "#s{position:absolute;width:200px;height:200px;\
         border:10px solid black;transform:translate(50%, 0)}",
    );
    let (_, _, cb) = dl(
        "<div id=s>x</div>",
        "#s{position:absolute;width:200px;height:200px;\
         border:10px solid black;transform-box:content-box;\
         transform:translate(50%, 0)}",
    );
    let (tx_bb, tx_cb) = (tx_of(&bb), tx_of(&cb));
    assert!(
        (tx_bb - tx_cb - 10.0).abs() < 1.5,
        "border-box={tx_bb} content-box={tx_cb} — delta attendu 10"
    );
}

/// Audit — diagnostic portée ancêtre : un descendant
/// `position:absolute` d'un ancêtre `transform`é doit se
/// peindre DANS la matrice (spec : transform s'applique à
/// tout le rendu de la boîte, descendants positionnés
/// inclus).
#[test]
fn transform_ancetre_englobe_enfant_positionne() {
    use explorer_render::DisplayItem;
    let (_, _, items) = dl(
        "<div id=t><div id=a>x</div></div>",
        "#t{transform:translate(10px, 5px);width:20px;height:20px}\
         #a{position:absolute;top:0;left:0}",
    );
    let push = items
        .iter()
        .position(|i| matches!(i, DisplayItem::PushTransform(_)));
    let pop = items
        .iter()
        .rposition(|i| matches!(i, DisplayItem::PopTransform));
    let text = items
        .iter()
        .position(|i| matches!(i, DisplayItem::Text { .. }));
    let (Some(p), Some(q), Some(t)) = (push, pop, text) else {
        panic!("brackets/texte manquants : {items:?}");
    };
    assert!(
        p < t && t < q,
        "texte positionné hors de la transform ancêtre : push={p} text={t} pop={q}"
    );
}

/// Audit — diagnostic portée ancêtre (2) : même cas mais le
/// descendant positionné porte `z-index` → file `pos_defer`.
#[test]
fn transform_ancetre_englobe_positionne_z() {
    use explorer_render::DisplayItem;
    let (_, _, items) = dl(
        "<div id=t><div id=a>x</div></div>",
        "#t{transform:translate(10px, 5px);width:20px;height:20px}\
         #a{position:absolute;top:0;left:0;z-index:1}",
    );
    let push = items
        .iter()
        .position(|i| matches!(i, DisplayItem::PushTransform(_)));
    let pop = items
        .iter()
        .rposition(|i| matches!(i, DisplayItem::PopTransform));
    let text = items
        .iter()
        .position(|i| matches!(i, DisplayItem::Text { .. }));
    let (Some(p), Some(q), Some(t)) = (push, pop, text) else {
        panic!("brackets/texte manquants : {items:?}");
    };
    assert!(
        p < t && t < q,
        "texte z-positionné hors de la transform ancêtre : push={p} text={t} pop={q}"
    );
}

// ── R-198 : portée ancêtre dans les files différées ──────────
//
// Verrous écrits AVANT le fix — diagnostic figé en preuve.
// Les positionnés z ≥ 0 et les `fixed` sont drainés par
// `pos_defer`/`fixed_defer` au contexte d'empilement
// englobant : ils ressortent hors des brackets des ancêtres
// qui ne créent pas de contexte d'empilement dans
// `stacking_ctx`. Spec : `clip-path` et `filter` créent un
// stacking context ET un containing block pour les
// descendants abs/fixed (css-masking-1, filter-effects) —
// leurs descendants positionnés doivent peindre DANS le
// bracket, pas à la racine.

/// Index du `Pop*` fermant le bracket ouvert à `push` —
/// profondeur comptée sur `is_push`/`is_pop` (brackets
/// imbriqués du même type ignorés).
fn matching_pop(
    items: &[DisplayItem],
    push: usize,
    is_push: fn(&DisplayItem) -> bool,
    is_pop: fn(&DisplayItem) -> bool,
) -> Option<usize> {
    let mut depth = 0i32;
    for (i, it) in items.iter().enumerate().skip(push) {
        if is_push(it) {
            depth += 1;
        } else if is_pop(it) {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

fn clip_push(i: &DisplayItem) -> bool {
    matches!(
        i,
        DisplayItem::PushClip(_)
            | DisplayItem::PushClipRounded { .. }
            | DisplayItem::PushClipPath { .. }
    )
}
fn clip_pop(i: &DisplayItem) -> bool {
    matches!(i, DisplayItem::PopClip)
}
fn filter_push(i: &DisplayItem) -> bool {
    matches!(i, DisplayItem::PushFilter(_))
}
fn filter_pop(i: &DisplayItem) -> bool {
    matches!(i, DisplayItem::PopFilter)
}
fn blend_push(i: &DisplayItem) -> bool {
    matches!(i, DisplayItem::PushBlend(_))
}
fn blend_pop(i: &DisplayItem) -> bool {
    matches!(i, DisplayItem::PopBlend)
}

/// Le `FillRect` 30×30 du descendant positionné des
/// fixtures ci-dessous (ancêtres sans fond).
fn fill_enfant(items: &[DisplayItem]) -> Option<usize> {
    items.iter().position(|i| {
        matches!(i, DisplayItem::FillRect { rect, .. }
            if (rect.width - 30.0).abs() < 0.5 && (rect.height - 30.0).abs() < 0.5)
    })
}

/// R-198 (verrou rouge) — `clip-path` ancêtre + descendant
/// `position:absolute` : le clip géométrique borne TOUT le
/// rendu de la boîte, descendants positionnés inclus
/// (css-masking-1 : clip-path = containing block + stacking
/// context). Aujourd'hui l'enfant part en `pos_defer` et se
/// peint après le `PopClip`.
#[test]
fn clip_path_ancetre_clippe_descendant_absolu() {
    let (_, _, items) = dl(
        "<div id=c><div id=a></div></div>",
        "#c{clip-path:polygon(0% 0%,100% 0%,50% 100%);width:100px;height:100px}\
         #a{position:absolute;left:0;top:0;width:30px;height:30px;\
            background-color:#ff0000}",
    );
    let push = items
        .iter()
        .position(|i| matches!(i, DisplayItem::PushClipPath { .. }))
        .expect("PushClipPath ancêtre");
    let pop = matching_pop(&items, push, clip_push, clip_pop).expect("PopClip");
    let fill = fill_enfant(&items).expect("fond de l'enfant absolu");
    assert!(
        push < fill && fill < pop,
        "enfant absolu hors du clip-path ancêtre : push={push} fill={fill} pop={pop}"
    );
}

/// R-198 (verrou rouge) — même cas avec `z-index` explicite
/// : l'enfant transite par `push_z` dans la file triée —
/// autre chemin d'enfilement, même trou de portée.
#[test]
fn clip_path_ancetre_clippe_descendant_absolu_z() {
    let (_, _, items) = dl(
        "<div id=c><div id=a></div></div>",
        "#c{clip-path:polygon(0% 0%,100% 0%,50% 100%);width:100px;height:100px}\
         #a{position:absolute;left:0;top:0;width:30px;height:30px;\
            z-index:1;background-color:#ff0000}",
    );
    let push = items
        .iter()
        .position(|i| matches!(i, DisplayItem::PushClipPath { .. }))
        .expect("PushClipPath ancêtre");
    let pop = matching_pop(&items, push, clip_push, clip_pop).expect("PopClip");
    let fill = fill_enfant(&items).expect("fond de l'enfant absolu");
    assert!(
        push < fill && fill < pop,
        "enfant z-positionné hors du clip-path ancêtre : \
         push={push} fill={fill} pop={pop}"
    );
}

/// R-198 (verrou rouge) — `filter` ancêtre + descendant
/// absolu : filter-effects — `filter` non-none crée un
/// stacking context et un containing block ; l'enfant doit
/// subir le flou dans le groupe `PushFilter`…`PopFilter`,
/// pas être peint net à la racine.
#[test]
fn filter_ancetre_encadre_descendant_absolu() {
    let (_, _, items) = dl(
        "<div id=c><div id=a></div></div>",
        "#c{filter:blur(4px);width:100px;height:100px}\
         #a{position:absolute;left:0;top:0;width:30px;height:30px;\
            background-color:#ff0000}",
    );
    let push = items
        .iter()
        .position(|i| matches!(i, DisplayItem::PushFilter(_)))
        .expect("PushFilter ancêtre");
    let pop = matching_pop(&items, push, filter_push, filter_pop).expect("PopFilter");
    let fill = fill_enfant(&items).expect("fond de l'enfant absolu");
    assert!(
        push < fill && fill < pop,
        "enfant absolu hors du filter ancêtre : push={push} fill={fill} pop={pop}"
    );
}

/// R-198 (cliquet vert) — `z-index:-1` : les positionnés à
/// z négatif sont émis en phase négative par `emit_scoped`
/// DANS l'émission du parent — déjà à l'intérieur du
/// bracket. Verrouille le cas correct à ne pas casser en
/// généralisant la route `stacking_ctx`.
#[test]
fn clip_path_ancetre_clippe_descendant_neg_z() {
    let (_, _, items) = dl(
        "<div id=c><div id=a></div></div>",
        "#c{clip-path:polygon(0% 0%,100% 0%,50% 100%);width:100px;height:100px}\
         #a{position:absolute;left:0;top:0;width:30px;height:30px;\
            z-index:-1;background-color:#ff0000}",
    );
    let push = items
        .iter()
        .position(|i| matches!(i, DisplayItem::PushClipPath { .. }))
        .expect("PushClipPath ancêtre");
    let pop = matching_pop(&items, push, clip_push, clip_pop).expect("PopClip");
    let fill = fill_enfant(&items).expect("fond de l'enfant z:-1");
    assert!(
        push < fill && fill < pop,
        "enfant z:-1 hors du clip-path ancêtre : push={push} fill={fill} pop={pop}"
    );
}

/// R-198 (cliquet vert) — `mix-blend-mode` figure déjà dans
/// `stacking_ctx` : ses positionnés drainent dans `my_pos`,
/// à l'intérieur du `PushBlend`. Prouve que l'infra de
/// portée existe — le fix étend la même route aux ancêtres
/// clip-path/filter.
#[test]
fn mix_blend_ancetre_encadre_descendant_absolu() {
    let (_, _, items) = dl(
        "<div id=c><div id=a></div></div>",
        "#c{mix-blend-mode:multiply;width:100px;height:100px}\
         #a{position:absolute;left:0;top:0;width:30px;height:30px;\
            background-color:#ff0000}",
    );
    let push = items
        .iter()
        .position(|i| matches!(i, DisplayItem::PushBlend(_)))
        .expect("PushBlend ancêtre");
    let pop = matching_pop(&items, push, blend_push, blend_pop).expect("PopBlend");
    let fill = fill_enfant(&items).expect("fond de l'enfant absolu");
    assert!(
        push < fill && fill < pop,
        "enfant absolu hors du groupe blend : push={push} fill={fill} pop={pop}"
    );
}

/// R-198 (cliquet vert) — `contain:paint` : déjà dans
/// `stacking_ctx` ET émet son propre `PushClip` — le
/// descendant absolu est clippé comme la spec le veut.
/// Verrouille le comportement correct.
#[test]
fn contain_paint_clippe_descendant_absolu() {
    let (_, _, items) = dl(
        "<div id=c><div id=a></div></div>",
        "#c{contain:paint;width:100px;height:100px}\
         #a{position:absolute;left:0;top:0;width:30px;height:30px;\
            background-color:#ff0000}",
    );
    let push = items
        .iter()
        .position(|i| matches!(i, DisplayItem::PushClip(_)))
        .expect("PushClip contain");
    let pop = matching_pop(&items, push, clip_push, clip_pop).expect("PopClip");
    let fill = fill_enfant(&items).expect("fond de l'enfant absolu");
    assert!(
        push < fill && fill < pop,
        "enfant absolu hors du clip contain:paint : \
         push={push} fill={fill} pop={pop}"
    );
}

/// R-198 (cliquet vert, garde-fou inverse) — un ancêtre
/// `overflow:hidden` NON positionné n'est pas le bloc
/// conteneur de l'absolu : la spec veut l'évasion (clip
/// `overflow` ≠ clip de tout le rendu — il ne s'applique
/// qu'aux descendants de sa chaîne de containing blocks).
/// Le fix clip-path/filter ne doit PAS tout clipper.
#[test]
fn abs_echappe_clip_overflow_ancetre_statique() {
    let (_, _, items) = dl(
        "<div id=c><div id=a></div></div>",
        "#c{overflow:hidden;width:50px;height:50px}\
         #a{position:absolute;left:0;top:0;width:30px;height:30px;\
            background-color:#ff0000}",
    );
    let push = items
        .iter()
        .position(|i| matches!(i, DisplayItem::PushClip(_)))
        .expect("PushClip overflow");
    let pop = matching_pop(&items, push, clip_push, clip_pop).expect("PopClip");
    let fill = fill_enfant(&items).expect("fond de l'enfant absolu");
    assert!(
        fill > pop,
        "enfant absolu rogné par le clip d'un ancêtre statique \
         (spec : évasion correcte) : fill={fill} pop={pop}"
    );
}

/// R-198 (cliquet d'écart — comportement actuel figé) —
/// `position:fixed` sous ancêtre `clip-path` : la spec fait
/// de clip-path le containing block du fixed → il devrait
/// être clippé. Mais le rect `fixed` est résolu **vs
/// viewport au layout** : la correction est côté
/// `explorer-layout` (CB modélisé), pas display — hors du
/// périmètre de ce fix. Flip attendu à l'étape CB-fixed.
#[test]
fn fixed_sous_clip_path_ancetre_echappe_aujourdhui() {
    let (_, _, items) = dl(
        "<div id=c><div id=a></div></div>",
        "#c{clip-path:polygon(0% 0%,100% 0%,50% 100%);width:100px;height:100px}\
         #a{position:fixed;left:0;top:0;width:30px;height:30px;\
            background-color:#ff0000}",
    );
    let push = items
        .iter()
        .position(|i| matches!(i, DisplayItem::PushClipPath { .. }))
        .expect("PushClipPath ancêtre");
    let pop = matching_pop(&items, push, clip_push, clip_pop).expect("PopClip");
    let fill = fill_enfant(&items).expect("fond du fixed");
    assert!(
        fill > pop,
        "comportement courant changé (fixed sous clip-path) — \
         mettre à jour le verrou : fill={fill} pop={pop}"
    );
}

/// R-198 (cliquet d'écart) — `position:fixed` sous ancêtre
/// `transform`é : spec — transform crée le CB du fixed →
/// peint dans la matrice, ancré à l'ancêtre. Trou connu de
/// l'audit (« fixed sous ancêtre transformé ») : le CB est
/// résolu au layout — verrouillé à l'état actuel en
/// attendant l'étape layout.
#[test]
fn fixed_sous_transform_ancetre_echappe_aujourdhui() {
    let (_, _, items) = dl(
        "<div id=c><div id=a></div></div>",
        "#c{transform:translate(10px,5px);width:100px;height:100px}\
         #a{position:fixed;left:0;top:0;width:30px;height:30px;\
            background-color:#ff0000}",
    );
    let push = items
        .iter()
        .position(|i| matches!(i, DisplayItem::PushTransform(_)))
        .expect("PushTransform ancêtre");
    let pop = matching_pop(
        &items,
        push,
        |i| matches!(i, DisplayItem::PushTransform(_)),
        |i| matches!(i, DisplayItem::PopTransform),
    )
    .expect("PopTransform");
    let fill = fill_enfant(&items).expect("fond du fixed");
    assert!(
        fill > pop,
        "comportement courant changé (fixed sous transform) — \
         mettre à jour le verrou : fill={fill} pop={pop}"
    );
}

/// R-198 (cliquet d'écart) — `overflow:hidden` sur ancêtre
/// POSITIONNÉ : le clip s'applique aux descendants de sa
/// chaîne de containing blocks, et l'ancêtre positionné EST
/// le CB de l'absolu → spec : clippé. L'évasion actuelle
/// relève d'une route par containing block (au-delà de la
/// seule extension `stacking_ctx`) — figé pour l'étape de
/// généralisation.
#[test]
fn abs_sous_overflow_ancetre_positionne_echappe_aujourdhui() {
    let (_, _, items) = dl(
        "<div id=c><div id=a></div></div>",
        "#c{position:relative;overflow:hidden;width:50px;height:50px}\
         #a{position:absolute;left:0;top:0;width:30px;height:30px;\
            background-color:#ff0000}",
    );
    let push = items
        .iter()
        .position(|i| matches!(i, DisplayItem::PushClip(_)))
        .expect("PushClip overflow");
    let pop = matching_pop(&items, push, clip_push, clip_pop).expect("PopClip");
    let fill = fill_enfant(&items).expect("fond de l'enfant absolu");
    assert!(
        fill > pop,
        "comportement courant changé (abs sous overflow positionné) — \
         mettre à jour le verrou : fill={fill} pop={pop}"
    );
}

/// R-198 (cliquet d'écart) — `clip:rect()` legacy : borne
/// le rendu de TOUT l'élément (descendants inclus, CSS2
/// §11.1.2) sans créer de stacking context — la spec veut
/// l'enfant clippé, la route par contexte n'est qu'une
/// approximation possible (z-order non modifié par spec).
/// Figé à l'état actuel.
#[test]
fn clip_rect_ancetre_ne_clippe_pas_descendant_absolu() {
    let (_, _, items) = dl(
        "<div id=c><div id=a></div></div>",
        "#c{position:absolute;width:100px;height:100px;\
            clip:rect(0,50px,50px,0)}\
         #a{position:absolute;left:0;top:0;width:30px;height:30px;\
            background-color:#ff0000}",
    );
    let push = items
        .iter()
        .position(|i| matches!(i, DisplayItem::PushClip(_)))
        .expect("PushClip clip:rect");
    let pop = matching_pop(&items, push, clip_push, clip_pop).expect("PopClip");
    let fill = fill_enfant(&items).expect("fond de l'enfant absolu");
    assert!(
        fill > pop,
        "comportement courant changé (abs sous clip:rect) — \
         mettre à jour le verrou : fill={fill} pop={pop}"
    );
}

// ── R-199 / §B-118 — répartition boîte→item (EmitStats) ──────

fn dl_stats(html: &str, css: &str) -> explorer_render::EmitStats {
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
        .expect("élément racine");
    let mut bx = build_box_tree(&dom, root_el, &styles).expect("arbre");
    layout_tree(&mut bx, VIEW, &ctx());
    let (_list, st) = explorer_render::build_display_list_scrolled_stats(
        &bx,
        VIEW,
        200.0,
        (0.0, 0.0),
        &explorer_render::PaintImages::default(),
    );
    st
}

/// Signature pluto.tv reconstituée : un ancêtre
/// `visibility:hidden` (hérité → chaque descendant skippe
/// son émission propre) — les boîtes vivent, les items
/// s'effondrent. Le compteur doit nommer la cause ET
/// l'ancêtre en une lecture.
#[test]
fn emit_stats_visibility_ancetre_collapse() {
    let st = dl_stats(
        "<div id=v><p>texte <a href=#>lien</a></p><div><span>suite</span></div></div>\
         <p id=out>hors</p>",
        "#v{visibility:hidden} p{margin:0}",
    );
    assert!(st.boxes >= 5, "boxes={}", st.boxes);
    assert!(
        st.skip_visibility >= 3,
        "skip_visibility={}",
        st.skip_visibility
    );
    assert_eq!(st.drop_opacity0, 0);
    assert_eq!(
        st.by_kind.get("text").copied().unwrap_or(0),
        1,
        "seul le <p> externe émet du texte : {:?}",
        st.by_kind
    );
    assert!(st.visibility_path.is_some(), "l'ancêtre coupable est nommé");
}

/// `opacity:0` sur l'ancêtre : retour précoce — le sous-arbre
/// n'émet rien du tout (ni propre ni descendants).
#[test]
fn emit_stats_opacity0_tue_sous_arbre() {
    let st = dl_stats(
        "<div id=o><p>texte</p><div>suite</div></div><p id=out>hors</p>",
        "#o{opacity:0} p{margin:0}",
    );
    assert_eq!(st.drop_opacity0, 1, "drop_opacity0={}", st.drop_opacity0);
    assert_eq!(st.skip_visibility, 0);
    assert_eq!(
        st.by_kind.get("text").copied().unwrap_or(0),
        1,
        "seul le <p> externe émet : {:?}",
        st.by_kind
    );
    assert!(st.opacity0_path.is_some());
}

/// Page saine : les compteurs racontent une passe normale —
/// texte émis, pas de drop, pas d'off-canvas.
#[test]
fn emit_stats_page_saine() {
    let st = dl_stats(
        "<div id=d><p>texte <a href=#>lien</a></p></div>",
        "#d{background-color:#ff0000} p{margin:0}",
    );
    assert!(st.boxes >= 3);
    assert!(st.items >= 2, "items={}", st.items);
    assert!(
        st.by_kind.get("text").copied().unwrap_or(0) >= 1,
        "texte émis : {:?}",
        st.by_kind
    );
    assert!(
        st.by_kind.get("fill").copied().unwrap_or(0) >= 1,
        "fond émis : {:?}",
        st.by_kind
    );
    assert_eq!(st.drop_opacity0 + st.drop_backface + st.skip_visibility, 0);
    assert_eq!(st.off_canvas, 0, "off_canvas={}", st.off_canvas);
}

/// Un élément translaté hors de la zone canvas : émis dans la
/// liste mais borne résolue dehors — `off_canvas` le compte
/// (classe « transform géant » des suspects).
#[test]
fn emit_stats_transform_hors_canvas_compte() {
    let st = dl_stats(
        "<div id=t>x</div><p>y</p>",
        "#t{transform:translate(5000px,0);background-color:#ff0000;\
           width:50px;height:50px} p{margin:0}",
    );
    assert!(
        st.off_canvas >= 1,
        "le fond translaté hors zone compté : {:?} / off={}",
        st.by_kind,
        st.off_canvas
    );
}

// ── R-203 — background multi-couches : url() au-delà de la 1re ──

const PX1: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR42mP4%2F58BAAT%2FAf9jgNErAAAAAElFTkSuQmCC";

/// Couche url 2+ : chaque couche est indexée dans
/// `background_layers` et peinte — l'ordre spec (couche 0
/// au-dessus) est respecté : la couche 1 émise en premier.
#[test]
fn r203_deux_couches_url_peintes_dans_l_ordre() {
    let dom = TreeBuilder::parse("<div id=d></div>");
    let mut eng = StyleEngine::new();
    eng.add_css("body { margin: 0 }", Origin::Author);
    eng.add_css(
        "#d{width:100px;height:100px;\
            background-image:url(PX),url(PX);\
            background-repeat:no-repeat,no-repeat;\
            background-position:0 0,50px 50px}"
            .replace("PX", PX1)
            .as_str(),
        Origin::Author,
    );
    let styles = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let images = explorer_render::collect_data_images(&dom, &styles, 10_000);
    let root_el = dom
        .children(dom.document())
        .into_iter()
        .flatten()
        .find(|&c| dom.element(c).is_ok())
        .expect("élément racine");
    let mut bx = build_box_tree(&dom, root_el, &styles).expect("arbre");
    layout_tree(&mut bx, VIEW, &ctx());
    let n = images
        .background_layers
        .values()
        .map(Vec::len)
        .sum::<usize>();
    assert_eq!(n, 2, "les 2 couches url indexées");
    let items = explorer_render::build_display_list_full(&bx, VIEW, 200.0, 0.0, &images).items;
    let bgs: Vec<_> = items
        .iter()
        .filter_map(|i| match i {
            DisplayItem::BackgroundImage { origin, .. } => Some(*origin),
            _ => None,
        })
        .collect();
    assert_eq!(bgs.len(), 2, "les 2 couches peintes : {bgs:?}");
    // Couche 1 (basse, position 50/50) émise avant la couche
    // 0 (haute, 0/0) — ordre spec : la première couche
    // déclarée est au-dessus.
    assert!(
        (bgs[0].0 - 50.0).abs() < 1.0 && (bgs[1].0).abs() < 1.0,
        "ordre des couches : {bgs:?}"
    );
}

/// `background-clip`/`background-origin` **par couche** : la
/// couche 1 (border-box) peint sur la border box, la couche
/// 0 (content-box) sur la boîte de contenu — aires
/// distinctes par couche.
#[test]
fn r203_clip_origin_par_couche() {
    let dom = TreeBuilder::parse("<div id=d></div>");
    let mut eng = StyleEngine::new();
    eng.add_css("body { margin: 0 }", Origin::Author);
    eng.add_css(
        "#d{width:80px;height:80px;padding:10px;border:5px solid #000;\
            background-image:url(PX),url(PX);\
            background-clip:content-box,border-box;\
            background-repeat:no-repeat,no-repeat}"
            .replace("PX", PX1)
            .as_str(),
        Origin::Author,
    );
    let styles = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    let images = explorer_render::collect_data_images(&dom, &styles, 10_000);
    let root_el = dom
        .children(dom.document())
        .into_iter()
        .flatten()
        .find(|&c| dom.element(c).is_ok())
        .expect("élément racine");
    let mut bx = build_box_tree(&dom, root_el, &styles).expect("arbre");
    layout_tree(&mut bx, VIEW, &ctx());
    let items = explorer_render::build_display_list_full(&bx, VIEW, 200.0, 0.0, &images).items;
    let mut rects = items.iter().filter_map(|i| match i {
        DisplayItem::BackgroundImage { rect, .. } => Some(*rect),
        _ => None,
    });
    let bottom = rects.next().expect("couche 1 (basse)");
    let top = rects.next().expect("couche 0 (haute)");
    // Border box : 80 + 2*10 + 2*5 = 110 ; content : 80.
    assert!(
        (bottom.width - 110.0).abs() < 1.0,
        "couche 1 = border box : {bottom:?}"
    );
    assert!(
        (top.width - 80.0).abs() < 1.0,
        "couche 0 = content box : {top:?}"
    );
}
