// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tests de l'inline layout (étape 20) : line boxes,
//! wrapping UAX #14, `text-align`, inline-block atomique —
//! fonte fixture pour les mesures réelles.

mod common;

use explorer_css::{Origin, StyleEngine, StyleEnv};
use explorer_dom::{Dom, NodeId};
use explorer_html::TreeBuilder;
use explorer_layout::font::{FontRequest, FontSource};
use explorer_layout::{build_box_tree, layout_tree, BlockCtx, LayoutBox};

const VIEW: f32 = 400.0;

/// `FontSource` sur la fixture : avance uniforme 0.5em →
/// `font-size:10px` = 5 px/caractère, métriques réelles.
struct Fixture(Vec<u8>);
impl FontSource for Fixture {
    fn font_data(&self, _req: &FontRequest) -> Option<Vec<u8>> {
        Some(self.0.clone())
    }
}

fn ctx() -> BlockCtx<'static> {
    let data = common::fixture_ttf();
    // Fuite volontaire : fixture statique de test.
    let src: &'static Fixture = Box::leak(Box::new(Fixture(data)));
    BlockCtx {
        fonts: Some(src),
        ..BlockCtx::default()
    }
}

fn laid(html: &str, css: &str) -> (Dom, LayoutBox) {
    laid_w(html, css, VIEW)
}

fn laid_w(html: &str, css: &str, w: f32) -> (Dom, LayoutBox) {
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
    layout_tree(&mut bx, w, &ctx());
    (dom, bx)
}

fn by_tag(dom: &Dom, tag: &str) -> NodeId {
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if dom.tag_name(n).is_ok_and(|t| t == tag) {
            return n;
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
        }
    }
    panic!("<{tag}> absent")
}

fn by_id(dom: &Dom, id: &str) -> NodeId {
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if dom.attribute_str(n, "id").is_ok_and(|v| v == Some(id)) {
            return n;
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
        }
    }
    panic!("#{id} absent")
}

fn find(b: &LayoutBox, id: NodeId) -> &LayoutBox {
    if b.node == Some(id) {
        return b;
    }
    b.children
        .iter()
        .find_map(|c| find_opt(c, id))
        .expect("boîte")
}
fn find_opt(b: &LayoutBox, id: NodeId) -> Option<&LayoutBox> {
    if b.node == Some(id) {
        return Some(b);
    }
    b.children.iter().find_map(|c| find_opt(c, id))
}

#[test]
fn espace_entre_elements_inline_adjacents() {
    // Vol 31 CNN : « Live Updates7 min ago » collé — le
    // nœud texte blanc entre deux `<span>` inline doit
    // produire une espace réelle (§16.6.1).
    let (dom, bx) = laid(
        "<!doctype html><p><span>AAAA</span> <span>BBBB</span></p>",
        "p{font-size:10px;width:400px}",
    );
    let p = find(&bx, by_tag(&dom, "p"));
    let anon = &p.children[0];
    let frags = &anon.lines[0].fragments;
    let texts: Vec<&str> = frags.iter().map(|f| f.text.as_str()).collect();
    // AAAA (20px) + espace (5px) + BBBB — le deuxième run
    // doit démarrer à ≥ 25px, pas 20px.
    let b = frags
        .iter()
        .find(|f| f.text == "BBBB")
        .unwrap_or_else(|| panic!("frags={texts:?}"));
    assert!(b.rect.x >= 24.9, "BBBB.x = {} (frags={texts:?})", b.rect.x);
}

#[test]
fn lignes_wrappent_sur_largeur() {
    // p 100px, texte 5px/car → ~20 car/ligne.
    let (dom, bx) = laid(
        "<!doctype html><p>aaaa bbbb cccc dddd eeee ffff</p>",
        "p{font-size:10px;width:100px}",
    );
    let p = find(&bx, by_tag(&dom, "p"));
    let anon = &p.children[0];
    assert!(anon.lines.len() >= 2, "lines={:?}", anon.lines.len());
    // Chaque ligne ≤ 100px de contenu.
    for l in &anon.lines {
        assert!(l.rect.width <= 100.5, "{}", l.rect.width);
    }
    // Hauteur cumulée = rect.height.
    let sum: f32 = anon.lines.iter().map(|l| l.rect.height).sum();
    assert!((sum - anon.rect.height).abs() < 0.01);
}

#[test]
fn fragments_portent_texte_et_position() {
    let (dom, bx) = laid("<!doctype html><p>ab cd</p>", "p{font-size:10px}");
    let p = find(&bx, by_tag(&dom, "p"));
    let line = &p.children[0].lines[0];
    assert!(line.fragments.len() >= 2, "{:?}", line.fragments.len());
    let t: String = line.fragments.iter().map(|f| f.text.as_str()).collect();
    assert_eq!(t, "ab cd");
    // Positions croissantes en x.
    let mut last = f32::MIN;
    for f in &line.fragments {
        assert!(f.rect.x >= last);
        last = f.rect.x;
        assert!(f.shaped.is_some(), "glyphes shapés par la fixture");
    }
}

#[test]
fn text_align_right_et_center() {
    let (dom, bx) = laid(
        "<!doctype html><p>ab</p>",
        "p{font-size:10px;width:200px;text-align:right}",
    );
    let p = find(&bx, by_tag(&dom, "p"));
    let line = &p.children[0].lines[0];
    // « ab » = 10px → slack 190 → fin à 200 dans le contenu.
    let last = line.fragments.last().expect("frag");
    let end = last.rect.x + last.rect.width - p.children[0].rect.x;
    assert!((end - 200.0).abs() < 1.0, "fin={end}");

    let (d2, bx2) = laid(
        "<!doctype html><p>ab</p>",
        "p{font-size:10px;width:200px;text-align:center}",
    );
    let p2 = find(&bx2, by_tag(&d2, "p"));
    let first = &p2.children[0].lines[0].fragments[0];
    let off = first.rect.x - p2.children[0].rect.x;
    assert!((off - 95.0).abs() < 1.0, "centré off={off}");
}

#[test]
fn saut_de_ligne_obligatoire() {
    let (dom, bx) = laid(
        "<!doctype html><p>a\nb</p>",
        "p{font-size:10px;width:400px;white-space:pre-line}",
    );
    let p = find(&bx, by_tag(&dom, "p"));
    assert!(p.children[0].lines.len() >= 2);
}

#[test]
fn inline_block_est_atomique() {
    let (dom, bx) = laid(
        "<!doctype html><p>avant <span>bouton</span> après</p>",
        "p{font-size:10px;width:400px} span{display:inline-block;width:60px;height:20px}",
    );
    let p = find(&bx, by_tag(&dom, "p"));
    let line = &p.children[0].lines[0];
    let atom = line.fragments.iter().find(|f| f.atomic);
    assert!(atom.is_some(), "fragment atomique attendu");
    assert!((atom.expect("a").rect.width - 60.0).abs() < 0.5);
}

#[test]
fn baseline_aligne_les_fragments() {
    let (dom, bx) = laid(
        "<!doctype html><p>a<big>b</big></p>",
        "p{font-size:10px} big{font-size:20px}",
    );
    let p = find(&bx, by_tag(&dom, "p"));
    let line = &p.children[0].lines[0];
    assert!(line.fragments.len() >= 2);
    // Tous les fragments partagent la même baseline absolue.
    let bs: Vec<f32> = line
        .fragments
        .iter()
        .map(|f| f.rect.y + f.baseline)
        .collect();
    for b in &bs {
        assert!((b - bs[0]).abs() < 0.6, "baselines {bs:?}");
    }
}

#[test]
fn absolu_niche_ne_gonfle_pas_la_ligne() {
    // Un `position:absolute` niché dans un inline ne produit
    // aucun item dans la ligne englobante — sinon son
    // encombrement gonflerait la hauteur de ligne (R-22 :
    // lobste.rs, ligne de byline à ~4 lignes de haut).
    // Conteneur `div` : dans un `<p>`, le parseur HTML
    // fermerait le paragraphe avant le <div>.
    let (dom, bx) = laid(
        "<!doctype html><div class=w>av <span class=dp>x<div class=a>absolu</div></span> ap</div>",
        ".w{font-size:10px} .dp{position:relative} .a{position:absolute;top:40px;left:0}",
    );
    let w = find(&bx, by_tag(&dom, "div"));
    let anon = &w.children[0];
    assert_eq!(anon.lines.len(), 1);
    let h = anon.lines[0].rect.height;
    assert!(h < 15.0, "ligne ~12 px attendue, mesuré {h}");
    // Et l'absolu a bien été posé à top:40px sous le span
    // `position:relative` (son bloc conteneur).
    let span = find(&bx, by_tag(&dom, "span"));
    let a_node = {
        let mut q = vec![dom.document()];
        let mut hit = None;
        while let Some(n) = q.pop() {
            if dom.attribute(n, "class").ok().flatten().as_deref() == Some("a") {
                hit = Some(n);
            }
            if let Ok(ch) = dom.children(n) {
                q.extend(ch.iter().rev());
            }
        }
        hit.expect("div.a")
    };
    let abs = find(&bx, a_node);
    assert!(
        (abs.rect.y - span.rect.y - 40.0).abs() < 1.0,
        "abs y={:?} span y={:?}",
        abs.rect,
        span.rect
    );
}

#[test]
fn inline_block_avec_bloc_ne_gonfle_pas() {
    // `details.caches` (lobste.rs) : un atome `inline-block`
    // dont les enfants blocs + blancs doivent compter UNE
    // fois le contenu, pas une ligne par enfant blanc.
    let (dom, bx) = laid(
        "<!doctype html><div class=w>av <span class=c> <b>uni</b> <i>deux</i> </span> ap</div>",
        ".w{font-size:10px} .c{display:inline-block} b,i{display:block}",
    );
    let w = find(&bx, by_tag(&dom, "div"));
    let c = find(&bx, by_tag(&dom, "span"));
    // Deux lignes de contenu (b puis i), pas +3 lignes de blancs.
    assert!(
        c.rect.height < 30.0,
        "atome ~2×12 px attendu, mesuré {}",
        c.rect.height
    );
    // La ligne du conteneur reste proche de la hauteur atome.
    let h = w.children[0].lines[0].rect.height;
    assert!(h < 30.0, "hauteur de ligne {h}");
}

#[test]
fn text_align_mesure_les_margin_boxes() {
    // R-27 : `text-align` mesure la ligne sur les **margin
    // boxes** — les marges d'un atome comptent (§16.2 +
    // §8.3). Réf. WPT `flex-direction-row-reverse-ref`.
    let (dom, bx) = laid(
        "<!doctype html><div><span>x</span></div>",
        "div{text-align:right} span{display:inline-block;width:100px;height:10px;margin:0 20px}",
    );
    let span = find(&bx, by_tag(&dom, "span"));
    // Margin box droite au bord du conteneur (VIEW=400) :
    // border box à 400 - 20(mr) - 100 = 280 (l'ancienne
    // mesure border-box la plaçait à 300).
    assert!((span.rect.x - 280.0).abs() < 0.5, "x={}", span.rect.x);
}

/// R-35 : `fragment_sub_rect` — sous-rect glyphe exact sur
/// run shapé (fixture 0.5em → 5 px/car à 10 px), repli
/// proportionnel sans run, `None` sur tranche invalide.
#[test]
fn fragment_sub_rect_glyph_metrics() {
    use explorer_layout::{fragment_sub_rect, Fragment, Rect};
    use std::sync::Arc;

    let run =
        explorer_layout::font::shape(&common::fixture_ttf(), "abcde", 10.0, false).expect("run");
    let frag = |shaped| Fragment {
        node: None,
        owner: 0,
        text: "abcde".into(),
        atomic: false,
        aid: None,
        rect: Rect {
            x: 10.0,
            y: 5.0,
            width: 25.0,
            height: 12.0,
        },
        baseline: 8.0,
        shaped,
        trail_w: 0.0,
        marg_l: 0.0,
        marg_r: 0.0,
        marg_t: 0.0,
        marg_b: 0.0,
        style: explorer_css::ComputedStyle::default(),
        deco: Vec::new(),
        level: 0,
    };

    // Tranche "bc" (octets 1..3) : x = 10 + 5 = 15, w = 10.
    let f = frag(Some(Arc::new(run)));
    let r = fragment_sub_rect(&f, 1, 3).expect("sous-rect");
    assert!((r.x - 15.0).abs() < 0.01, "x={} attendu 15", r.x);
    assert!((r.width - 10.0).abs() < 0.01, "w={} attendu 10", r.width);
    assert_eq!((r.y, r.height), (5.0, 12.0), "bande de ligne pleine");
    // Plein texte = plein fragment.
    let r = fragment_sub_rect(&f, 0, 5).expect("plein");
    assert!((r.x - 10.0).abs() < 0.01 && (r.width - 25.0).abs() < 0.01);

    // Sans run shapé : repli proportionnel (mêmes bornes).
    let f2 = frag(None);
    let r2 = fragment_sub_rect(&f2, 1, 3).expect("repli");
    assert!((r2.x - 15.0).abs() < 0.01 && (r2.width - 10.0).abs() < 0.01);

    // Hors bornes / vide / atomique → None.
    assert!(fragment_sub_rect(&f, 3, 3).is_none());
    assert!(fragment_sub_rect(&f, 2, 9).is_none());
    let mut fa = frag(None);
    fa.atomic = true;
    assert!(fragment_sub_rect(&fa, 0, 2).is_none());
}

// ── R-38 : quirks — reset d'héritage sur table ───────────────

#[test]
fn quirks_table_cellule_alignee_gauche() {
    // Doctype absent → quirks : `<center>` ne centre pas le
    // texte des cellules (reset `text-align` sur `table`,
    // HTML Rendering « In quirks mode » — terrain : les
    // titres de news.ycombinator.com restent à gauche).
    let (dom, bx) = laid(
        "<center><table><tr><td style=\"font-size:10px\">ab</td></tr></table></center>",
        "table{width:200px}",
    );
    assert_eq!(dom.document_mode(), explorer_dom::DocumentMode::Quirks);
    let td = find(&bx, by_tag(&dom, "td"));
    let line = &td.children[0].lines[0];
    let frag = line
        .fragments
        .iter()
        .find(|f| !f.atomic)
        .expect("frag texte");
    let off = frag.rect.x - td.children[0].rect.x;
    assert!(off < 5.0, "quirks : texte à gauche, off={off}");
}

#[test]
fn no_quirks_table_cellule_herite_centre() {
    // Contrôle : même markup avec DOCTYPE — `-webkit-center`
    // traverse l'héritage et centre le texte de cellule.
    let (dom, bx) = laid(
        "<!doctype html><center><table><tr><td style=\"font-size:10px\">ab</td></tr></table></center>",
        "table{width:200px}",
    );
    let td = find(&bx, by_tag(&dom, "td"));
    let line = &td.children[0].lines[0];
    let frag = line
        .fragments
        .iter()
        .find(|f| !f.atomic)
        .expect("frag texte");
    let off = frag.rect.x - td.children[0].rect.x;
    assert!(off > 40.0, "no-quirks : texte centré, off={off}");
}

// ── R-90 : rects des inlines imbriqués + atomes inline-* ──

/// Toutes les boîtes portant un nœud de tag donné (ordre doc).
fn find_all(b: &LayoutBox, dom: &Dom, tag: &str, out: &mut Vec<NodeId>) {
    if let Some(n) = b.node {
        if dom.tag_name(n).is_ok_and(|t| t == tag) {
            out.push(n);
        }
    }
    for c in &b.children {
        find_all(c, dom, tag, out);
    }
}

#[test]
fn controle_inline_flex_niche_est_atomique() {
    // Mur de consentement Google (journal interface, terrain
    // mission Étape 120) : deux `<input type=submit>` en
    // `display:inline-flex` nichés dans des `<span>` inline.
    // `is_atomic` n'exigeait que `inline-block` : l'atome
    // `inline-flex` était traité en conteneur inline, et
    // l'union `owner == i` en récursion comparait l'indice
    // LOCAL à l'indice RACINE — le 2ᵉ bouton recevait le
    // rect du 1ᵉʳ → hit-test → `form`, clic `Ignored`.
    let (dom, bx) = laid(
        "<!doctype html><div><span><input class=b type=submit value=refuser></span>\
         <span><input class=b type=submit value=accepter></span></div>",
        "div{font-size:10px} .b{display:inline-flex}",
    );
    let mut ids = Vec::new();
    find_all(&bx, &dom, "input", &mut ids);
    // La valeur synthétique du contrôle porte le même NodeId
    // que l'input (Text enfant) → un id par boîte, dédupliqué.
    ids.dedup();
    assert_eq!(ids.len(), 2, "deux inputs attendus, {ids:?}");
    let i1 = find(&bx, ids[0]);
    let i2 = find(&bx, ids[1]);
    // Chacun a son propre rect, côte à côte (pas la copie du
    // sibling).
    assert!(i1.rect.width > 0.0 && i2.rect.width > 0.0);
    assert!(
        i2.rect.x > i1.rect.x,
        "i2 devrait être à droite de i1 : {i1:?} vs {i2:?}",
        i1 = i1.rect,
        i2 = i2.rect
    );
    // Le clic au centre du 2ᵉ bouton remonte SON input.
    let cx = i2.rect.x + i2.rect.width / 2.0;
    let cy = i2.rect.y + i2.rect.height / 2.0;
    assert_eq!(
        explorer_layout::hit_test(&bx, cx, cy),
        Some(ids[1]),
        "hit au 2ᵉ bouton"
    );
}

#[test]
fn inline_niche_recupere_ses_propres_fragments() {
    // Même défaut sur inline NON atomique : `<i>` seul enfant
    // de `<span>` (index local 0) recevait l'union des
    // fragments de l'enfant racine n°0 (`<b>`) — le rect du
    // sibling, pas le sien.
    let (dom, bx) = laid(
        "<!doctype html><p><b>AAAA</b><span><i>BBBB</i></span></p>",
        "p{font-size:10px;width:200px}",
    );
    let b = find(&bx, by_tag(&dom, "b"));
    let i = find(&bx, by_tag(&dom, "i"));
    // Fixture 0.5em → « AAAA » = 20 px ; « BBBB » suit à x≈20.
    assert!(
        (i.rect.x - 20.0).abs() < 1.0,
        "i.x={} attendu ~20 (après AAAA), pas {}",
        i.rect.x,
        b.rect.x
    );
    assert!(
        (i.rect.width - 20.0).abs() < 1.0,
        "i.width={} attendu ~20",
        i.rect.width
    );
}

#[test]
fn inline_grid_et_inline_table_sont_atomiques() {
    // CSS §9.2.4 : tous les `inline-*` à contexte de formatage
    // propre coulent en atome — un seul fragment `atomic` par
    // boîte, contenu jamais itemisé dans la ligne parente.
    let (dom, bx) = laid(
        "<!doctype html><div>AA<span class=g><i>BB</i><i>CC</i></span>DD</div>",
        "div{font-size:10px} .g{display:inline-grid}",
    );
    let g = find(&bx, by_tag(&dom, "span"));
    assert!(g.rect.width > 0.0 && g.rect.height > 0.0, "{:?}", g.rect);
    let (dom2, bx2) = laid(
        "<!doctype html><div>AA<table class=t><tr><td>cc</td></tr></table>DD</div>",
        "div{font-size:10px} .t{display:inline-table}",
    );
    let t = find(&bx2, by_tag(&dom2, "table"));
    // Vraie table posée (sa cellule existe et tient dedans).
    let td = find(&bx2, by_tag(&dom2, "td"));
    assert!(t.rect.width > 0.0 && td.rect.width > 0.0);
    assert!(
        td.rect.x >= t.rect.x - 0.01 && td.rect.y >= t.rect.y - 0.01,
        "cellule dans la table : {td:?} vs {t:?}",
        td = td.rect,
        t = t.rect
    );
}

#[test]
fn text_indent_premiere_ligne_seule() {
    // R-70 : `text-indent` ne décale que la première ligne
    // formatée (les suivantes repartent au bord) ; négatif,
    // il sort le texte de l'écran — technique
    // image-remplacement des sprites (`text-indent:-9999px`,
    // portail Wikipedia).
    let (dom, bx) = laid(
        "<!doctype html><p>aaaa bbbb cccc dddd eeee ffff</p>",
        "p{font-size:10px;width:100px;text-indent:20px}",
    );
    let p = find(&bx, by_tag(&dom, "p"));
    let anon = &p.children[0];
    assert!(anon.lines.len() >= 2, "{:?}", anon.lines.len());
    let first_x = anon.lines[0].fragments[0].rect.x;
    let second_x = anon.lines[1].fragments[0].rect.x;
    assert!((first_x - 20.0).abs() < 1.0, "1re ligne x={first_x}");
    assert!(second_x.abs() < 1.0, "lignes suivantes x={second_x}");

    // Négatif : le texte démarre hors écran.
    let (d2, bx2) = laid(
        "<!doctype html><p>hidden</p>",
        "p{font-size:10px;text-indent:-9999px}",
    );
    let p2 = find(&bx2, by_tag(&d2, "p"));
    let fx = p2.children[0].lines[0].fragments[0].rect.x;
    assert!(fx <= -9990.0, "indent négatif x={fx}");
}

/// Vol 21/22 : `text-overflow:ellipsis` — un fragment « … »
/// est inséré au point de coupure quand le texte déborde un
/// conteneur à `overflow:hidden` (discourse « Topics »
/// tronqué net sans suspension).
#[test]
fn text_overflow_ellipsis_insere_suspension() {
    let (dom, bx) = laid_w(
        "<!doctype html><p>un titre beaucoup trop long pour tenir</p>",
        "body { margin: 0 } p { width: 80px; white-space: nowrap; \
         overflow: hidden; text-overflow: ellipsis; font-size: 10px }",
        800.0,
    );
    let p = find(&bx, by_tag(&dom, "p"));
    let anon = &p.children[0];
    assert_eq!(anon.lines.len(), 1, "nowrap : une ligne");
    let last = anon.lines[0].fragments.last().expect("fragment ellipsis");
    assert_eq!(last.text, "…", "dernier fragment = suspension");
    // Le « … » finit au bord droit du content box (x=0, w=80).
    assert!(
        (last.rect.x + last.rect.width - 80.0).abs() < 1.0,
        "ellipsis au bord : {:?}",
        last.rect
    );
    // Le fragment tronqué précédent se termine avant l'« … ».
    let prev = &anon.lines[0].fragments[anon.lines[0].fragments.len() - 2];
    assert!(
        prev.rect.x + prev.rect.width <= last.rect.x + 0.6,
        "troncature : {:?} vs {:?}",
        prev.rect,
        last.rect
    );
}

/// `text-overflow:clip` (défaut) : aucun fragment « … ».
#[test]
fn text_overflow_clip_par_defaut() {
    let (dom, bx) = laid_w(
        "<!doctype html><p>un titre beaucoup trop long pour tenir</p>",
        "body { margin: 0 } p { width: 80px; white-space: nowrap; overflow: hidden; font-size: 10px }",
        800.0,
    );
    let p = find(&bx, by_tag(&dom, "p"));
    let last = p.children[0].lines[0].fragments.last().expect("frag");
    assert_ne!(last.text, "…", "clip : pas de suspension");
}

/// `ellipsis` sans clip (`overflow:visible`) : pas d'effet.
#[test]
fn text_overflow_ellipsis_sans_clip_inerte() {
    let (dom, bx) = laid_w(
        "<!doctype html><p>un titre beaucoup trop long pour tenir</p>",
        "body { margin: 0 } p { width: 80px; white-space: nowrap; text-overflow: ellipsis; font-size: 10px }",
        800.0,
    );
    let p = find(&bx, by_tag(&dom, "p"));
    let last = p.children[0].lines[0].fragments.last().expect("frag");
    assert_ne!(last.text, "…", "visible : pas de suspension");
}

/// R-161 — `sup`/`sub`/`vertical-align:px`/`%` : la
/// baseline des runs concernés se décale (les autres
/// restent sur la baseline commune).
#[test]
fn vertical_align_sub_super_px_pct_decale_la_baseline() {
    let (dom, bx) = laid_w(
        "<!doctype html><p>x<sup>2</sup>y<sub>i</sub>\
         <span style=\"vertical-align:4px\">z</span>\
         <span style=\"vertical-align:-20%\">w</span></p>",
        "body { margin: 0 } p { font-size: 20px; margin: 0 }",
        800.0,
    );
    let p = find(&bx, by_tag(&dom, "p"));
    let frags = &p.children[0].lines[0].fragments;
    let fy = |needle: &str| -> f32 {
        frags
            .iter()
            .find(|f| f.text.trim() == needle)
            .map_or(f32::NAN, |f| f.rect.y)
    };
    let base_y = fy("x");
    // Le texte courant peut fusionner « x » au voisin — la
    // baseline de référence = un fragment sans décalage.
    let ref_y = frags
        .iter()
        .filter(|f| f.text.contains('y') || f.text.contains('x'))
        .map(|f| f.rect.y)
        .fold(f32::INFINITY, f32::min);
    assert!(ref_y.is_finite(), "ligne sans texte courant ?");
    let up = fy("2");
    let lo = fy("i");
    let px = fy("z");
    let pct = fy("w");
    assert!(up < ref_y - 2.0, "sup doit monter : up={up} ref={ref_y}");
    assert!(lo > ref_y + 2.0, "sub doit descendre : lo={lo} ref={ref_y}");
    assert!(px < ref_y - 3.0, "4px doit monter : px={px} ref={ref_y}");
    assert!(
        pct > ref_y + 2.0,
        "-20% doit descendre : pct={pct} ref={ref_y}"
    );
    let _ = base_y;
}

/// R-162 — `text-transform` appliqué avant itemisation :
/// `uppercase`/`lowercase` sur tout le texte, `capitalize`
/// par mot, hérité aux éléments descendants.
#[test]
fn text_transform_applique_avant_itemisation() {
    let (dom, bx) = laid_w(
        "<!doctype html><p id=u>hello WORLD</p><p id=l>HELLO World</p>\
         <p id=c>bonjour le monde</p><div id=d><span id=s>abc def</span></div>",
        "body { margin: 0 } #u { text-transform: uppercase; margin: 0 } \
         #l { text-transform: lowercase; margin: 0 } \
         #c { text-transform: capitalize; margin: 0 } \
         #d { text-transform: uppercase } span { margin: 0 }",
        800.0,
    );
    let text = |id: &str| -> String {
        let b = find(&bx, by_id(&dom, id));
        b.children[0]
            .lines
            .iter()
            .flat_map(|l| l.fragments.iter())
            .map(|f| f.text.as_str())
            .collect()
    };
    assert!(text("u").contains("HELLO WORLD"), "u={}", text("u"));
    assert!(text("l").contains("hello world"), "l={}", text("l"));
    assert!(text("c").contains("Bonjour Le Monde"), "c={}", text("c"));
    // Hérité au descendant `span` (§16.5 — la propriété
    // descend l'arbre comme `color`).
    assert!(text("d").contains("ABC DEF"), "d={}", text("d"));
}

/// R-163 — `input[type=password]` : la valeur rend une
/// puce par caractère, jamais le texte clair ;
/// `input[type=image]` : même boîte qu'un `img` — repli
/// `alt` tant que `src` n'est pas dans `sizes` (le fetch
/// reste la couture NAV).
#[test]
fn input_password_puces_et_type_image_replie_alt() {
    let (dom, bx) = laid_w(
        "<!doctype html><input id=pw type=password value=secret>\
         <input id=im type=image src=btn.png alt=Envoyer>",
        "body { margin: 0 } input { margin: 0 }",
        800.0,
    );
    // La boîte contrôle enveloppe l'enfant Text dans un
    // bloc anonyme — collecte récursive du texte.
    let all_text = |b: &LayoutBox| -> String {
        let mut out = String::new();
        let mut stack = vec![b];
        while let Some(x) = stack.pop() {
            if let Some(t) = &x.text {
                out.push_str(t);
            }
            stack.extend(x.children.iter());
        }
        out
    };
    let pw = find(&bx, by_id(&dom, "pw"));
    let t = all_text(pw);
    // 6 caractères → 6 puces, jamais le texte clair.
    assert_eq!(
        t, "\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}",
        "pw={t}"
    );
    assert!(!t.contains('s'), "le texte clair fuit : {t}");
    // `type=image` sans `sizes` → repli `alt` (chemin img_box).
    let im = find(&bx, by_id(&dom, "im"));
    assert_eq!(all_text(im), "Envoyer");
}

/// R-165 — `<wbr>` : opportunité de coupure souple à
/// l'itemisation — un mot insecable se coupe au point
/// `<wbr>` quand la bande est étroite, sans rien peindre
/// (la sentinelle ZWSP est strippée avant le shaping).
#[test]
fn wbr_opportunite_de_coupure_souple() {
    let (dom, bx) = laid_w(
        "<!doctype html><p>mot_insecable<wbr>suite_du_mot</p>",
        "body { margin: 0 } p { font-size: 20px; margin: 0; width: 110px }",
        400.0,
    );
    let p = find(&bx, by_tag(&dom, "p"));
    let lines = &p.children[0].lines;
    assert!(
        lines.len() >= 2,
        "le mot devrait couper au wbr : {} lignes",
        lines.len()
    );
    // La sentinelle ne peint rien : aucun fragment ne
    // porte U+200B.
    for l in lines {
        for f in &l.fragments {
            assert!(!f.text.contains('\u{200B}'), "zwsp peinte : {:?}", f.text);
        }
    }
}

/// R-168 — `word-spacing` : chaque espace U+0020 gagne
/// le supplément — le fragment texte « a b c » (2 espaces)
/// s'élargit de 2×ws vs `normal`.
#[test]
fn word_spacing_ajoute_par_espace() {
    let w = |css: &str| -> f32 {
        let (dom, bx) = laid_w(
            "<!doctype html><p>a b c</p>",
            &format!("body {{ margin: 0 }} p {{ margin: 0; font-size: 20px; {css} }}"),
            800.0,
        );
        let p = find(&bx, by_tag(&dom, "p"));
        p.children[0].lines[0].rect.width
    };
    let base = w("word-spacing: normal");
    let plus = w("word-spacing: 10px");
    assert!(
        plus - base > 19.0 && plus - base < 21.0,
        "2 espaces ×10px attendus : base={base} plus={plus}"
    );
}

/// R-168 — `content: attr(name)` : la valeur de
/// l'attribut de l'élément entre dans le texte généré
/// du pseudo-élément (concaténée entre les chaînes).
#[test]
fn attr_dans_content_pseudo() {
    let (dom, bx) = laid_w(
        "<!doctype html><a href=https://ex.fr>lien</a>",
        "a::after { content: \" [\" attr(href) \"]\" }",
        800.0,
    );
    let a = find(&bx, by_tag(&dom, "a"));
    // Le pseudo `::after` est le dernier enfant ; son
    // enfant Text porte la chaîne générée.
    let mut found = String::new();
    let mut stack = vec![a];
    while let Some(b) = stack.pop() {
        if let Some(t) = &b.text {
            found.push_str(t);
        }
        stack.extend(b.children.iter());
    }
    assert!(
        found.contains("https://ex.fr"),
        "attr(href) absent du texte généré : {found:?}"
    );
}

/// R-169 — `counter-reset`/`counter-increment`/`counter()` :
/// la valeur de l'env des compteurs entre dans `content`
/// (incrément par élément, reset à zéro).
#[test]
fn counter_increment_reset_dans_content() {
    let (dom, bx) = laid_w(
        "<!doctype html><div><h2>a</h2><h2>b</h2></div>",
        "body { margin: 0 } div { counter-reset: ch 0 } \
         h2 { counter-increment: ch } h2::before { content: counter(ch) \". \" } h2 { margin: 0 }",
        800.0,
    );
    let texts = || -> Vec<String> {
        let mut out = Vec::new();
        let mut stack = vec![bx.clone()];
        while let Some(b) = stack.pop() {
            if let Some(t) = &b.text {
                out.push(t.clone());
            }
            stack.extend(b.children.iter().cloned());
        }
        out
    };
    let all = texts().join("");
    assert!(
        all.contains("1. ") && all.contains("2. "),
        "compteurs : {all:?}"
    );
    let _ = dom;
}

/// `counter-set` (css-lists-3) : valeur absolue, pas
/// l'incrément — « ch » saute de 1 à 10 sur le 2e h2.
#[test]
fn counter_set_absolu() {
    let (dom, bx) = laid_w(
        "<!doctype html><div><h2>a</h2><h2 class=skip>b</h2></div>",
        "body { margin: 0 } div { counter-reset: ch 0 } \
         h2 { counter-increment: ch } h2.skip { counter-set: ch 10 } \
         h2::before { content: counter(ch) \". \" } h2 { margin: 0 }",
        800.0,
    );
    let mut out = Vec::new();
    let mut stack = vec![bx.clone()];
    while let Some(b) = stack.pop() {
        if let Some(t) = &b.text {
            out.push(t.clone());
        }
        stack.extend(b.children.iter().cloned());
    }
    let all = out.join("");
    assert!(all.contains("1. "), "1er h2 → 1 : {all:?}");
    assert!(all.contains("11. "), "skip → set 10 + increment : {all:?}");
    let _ = dom;
}
/// R-169 fix — `counter-reset: a b 2` : le token suivant
/// n'est un pas que s'il PARSE en entier — sinon c'est le
/// nom suivant. Régression post-revue : `b` était avalé
/// comme pas manqué de `a` et un compteur « 2 » naissait.
#[test]
fn counter_reset_liste_sans_pas_n_avale_pas_le_nom() {
    let (dom, bx) = laid_w(
        "<!doctype html><div>x</div>",
        "div { counter-reset: a b 2; margin: 0 } \
         div::before { content: counter(a) \"/\" counter(b) }",
        800.0,
    );
    let mut out = Vec::new();
    let mut stack = vec![bx];
    while let Some(b) = stack.pop() {
        if let Some(t) = &b.text {
            out.push(t.clone());
        }
        stack.extend(b.children.iter().cloned());
    }
    let all = out.join("");
    assert!(all.contains("0/2"), "counter(a)=0 counter(b)=2 : {all:?}");
    let _ = dom;
}

/// `line-clamp` / `-webkit-line-clamp` : ne garde que les
/// n premières lignes + « … » sur la dernière — la hauteur
/// de la boîte suit (mêmes lignes, mêmes fragments).
#[test]
fn line_clamp_tronque_les_lignes_et_suspend() {
    let long = "mot ".repeat(60);
    // Contrôle : sans clamp, le texte fait > 3 lignes.
    let (dom0, bx0) = laid_w(
        &format!("<!doctype html><p>{long}</p>"),
        "body { margin: 0 } p { width: 100px; font-size: 10px }",
        800.0,
    );
    let (dom1, bx1) = laid_w(
        &format!("<!doctype html><p>{long}</p>"),
        "body { margin: 0 } p { width: 100px; font-size: 10px; -webkit-line-clamp: 2 }",
        800.0,
    );
    let lines0 = find(&bx0, by_tag(&dom0, "p")).children[0].lines.len();
    let lines1 = find(&bx1, by_tag(&dom1, "p")).children[0].lines.len();
    assert!(lines0 > 3, "contrôle : >3 lignes ({lines0})");
    assert_eq!(lines1, 2, "clamped : 2 lignes ({lines1})");
    let last = find(&bx1, by_tag(&dom1, "p")).children[0].lines[1]
        .fragments
        .last()
        .expect("fragment ellipsis");
    assert_eq!(last.text, "…", "dernier fragment = suspension");
    // La hauteur de la boîte suit les lignes restantes.
    let h_full = find(&bx0, by_tag(&dom0, "p")).rect.height;
    let h_clamped = find(&bx1, by_tag(&dom1, "p")).rect.height;
    assert!(
        h_clamped < h_full,
        "hauteur réduite : {h_clamped} vs {h_full}"
    );
    // `line-clamp` (nom moderne) a le même effet.
    let (dom2, bx2) = laid_w(
        &format!("<!doctype html><p>{long}</p>"),
        "body { margin: 0 } p { width: 100px; font-size: 10px; line-clamp: 1 }",
        800.0,
    );
    let lines2 = find(&bx2, by_tag(&dom2, "p")).children[0].lines.len();
    assert_eq!(lines2, 1, "line-clamp:1 → une ligne ({lines2})");
}

/// R-200 — repro lichess : colonne flex étroite, items
/// hétérogènes (glyphe PUA type icon-font + texte). Verrou :
/// deux fragments adjacents de la même ligne ne se
/// superposent jamais (x + width strictement croissant ou
/// lignes distinctes).
#[test]
fn r200_colonne_flex_etroite_pas_de_chevauchement() {
    // Collecte tous les fragments avec leur rect — vérifie
    // l'absence de superposition dans chaque ligne.
    fn frags(b: &LayoutBox, out: &mut Vec<(f32, f32, f32, String)>) {
        for line in &b.lines {
            for f in &line.fragments {
                out.push((f.rect.x, f.rect.width, f.rect.y, f.text.clone()));
            }
        }
        for c in &b.children {
            frags(c, out);
        }
    }
    let (dom, bx) = laid(
        "<!doctype html><aside><ul>\
         <li><a href=#><i>\u{e000}</i> Games</a></li>\
         <li><a href=#><i>\u{e001}</i> Puzzles</a></li>\
         <li><a href=#><i>\u{e002}</i> Learn</a></li></ul></aside>",
        "aside{display:flex;flex-direction:column;width:120px}\
         ul{display:flex;flex-direction:column;margin:0;padding:0}\
         li{display:block} a{display:block} i{font-style:normal}",
    );
    let aside = find(&bx, by_tag(&dom, "aside"));
    let mut fs = Vec::new();
    frags(aside, &mut fs);
    // Regrouper par ligne (même y) — dans une ligne, les x
    // doivent être croissants sans recouvrement.
    for w in fs.windows(2) {
        let (x0, w0, y0, t0) = &w[0];
        let (x1, _w1, y1, t1) = &w[1];
        assert!(
            !((y0 - y1).abs() < 0.5 && x1 < &(x0 + w0 - 0.5)),
            "fragments superposés même ligne : {t0:?} x={x0} w={w0} \
             vs {t1:?} x={x1} (y={y0})"
        );
    }
}

/// R-207 — `font-size:clamp()` (et `calc()`/`min()`/`max()`)
/// est matérialisée en px absolu dans le style porté par le
/// fragment : les consommateurs aval (paint `prop_px` /
/// `font_size_of`) ne résolvent pas les fonctions maths —
/// sans la matérialisation, les glyphes étaient rastrés à
/// 16 px sous des avances shapées à la taille réelle
/// (plyr.io : titre éclaté lettre par lettre).
#[test]
fn r207_clamp_font_size_materialisee_px_dans_style_fragment() {
    fn frag_style(b: &LayoutBox) -> Option<String> {
        if let Some(f) = b.lines.iter().flat_map(|l| &l.fragments).next() {
            return Some(f.style.get_or("font-size", "").to_string());
        }
        b.children.iter().find_map(frag_style)
    }
    // `clamp(10px, 50vw, 40px)` à VIEW=400 → 50vw=200 borné
    // à 40 px. Idem `calc(30px + 4px)` = 34 px.
    let (dom, bx) = laid(
        "<!doctype html><p id=a>un</p><p id=b>deux</p>",
        "#a{font-size:clamp(10px,50vw,40px)}#b{font-size:calc(30px + 4px)}",
    );
    let a = find(&bx, by_id(&dom, "a"));
    assert_eq!(
        frag_style(a).as_deref(),
        Some("40px"),
        "font-size:clamp matérialisée en px dans le style du fragment"
    );
    let b = find(&bx, by_id(&dom, "b"));
    assert_eq!(
        frag_style(b).as_deref(),
        Some("34px"),
        "font-size:calc matérialisée en px dans le style du fragment"
    );
}

/// R-211 — `padding`/`border` horizontaux sur élément inline non-
/// remplacé consomment de l'espace sur la ligne (§9.4.2). Avant le
/// fix, seuls `margin-left/right` déclenchaient les marqueurs
/// d'extrémité → `a{padding:0 8px}` retombait à 0 → items de nav
/// contigus (vol 772 dr.dk, vimeo, bitmovin).
#[test]
fn padding_horizontal_inline_consomme_la_ligne() {
    let (dom, bx) = laid(
        "<!doctype html><p><a>x1</a><a>x2</a></p>",
        "p{font-size:10px;width:400px} a{padding:0 40px}",
    );
    let p = find(&bx, by_tag(&dom, "p"));
    let anon = &p.children[0];
    let frags = &anon.lines[0].fragments;
    let x_of = |t: &str| {
        frags
            .iter()
            .find(|f| f.text == t)
            .map(|f| f.rect.x)
            .expect(t)
    };
    // « x1 » = 10 px à 5 px/char : padding-left 40px consommé
    // par le marqueur d'ouverture → le texte démarre à x=40.
    assert_eq!(x_of("x1"), 40.0, "padding-left de <a> ignoré");
    // x2 : 40 (pad) + 10 (x1) + 40 (pad) + 40 (pad) = 130.
    assert_eq!(x_of("x2"), 130.0, "padding-right de <a> ignoré");
}

fn first_box_with_lines(bx: &LayoutBox) -> &LayoutBox {
    if !bx.lines.is_empty() {
        return bx;
    }
    bx.children
        .iter()
        .find_map(|c| {
            let c = first_box_with_lines(c);
            (!c.lines.is_empty()).then_some(c)
        })
        .expect("boîte p")
}

/// `text-indent` `hanging`/`each-line` (css-text-4) :
/// `hanging` = indent sauf la première ligne ; `each-line`
/// = toutes les lignes.
#[test]
fn text_indent_keywords() {
    let long = "aaaa bbbb cccc dddd eeee ffff gggg hhhh iiii jjjj";
    let html = format!("<p>{long}</p>");
    // `hanging` : la 2e ligne est décalée de 20px.
    let (_dom, bx) = laid_w(
        &html,
        "p{width:100px;text-indent:20px hanging;margin:0}",
        800.0,
    );
    let p = first_box_with_lines(&bx);
    let (first, rest) = (p.lines.first().unwrap().rect.x, p.lines[1].rect.x);
    assert_eq!(first, 0.0, "hanging : L1 sans indent");
    assert_eq!(rest, 20.0, "hanging : L2+ indentées ({rest})");
    // `each-line` : toutes indentées.
    let (_d2, bx2) = laid_w(
        &html,
        "p{width:100px;text-indent:20px each-line;margin:0}",
        800.0,
    );
    let p2 = first_box_with_lines(&bx2);
    assert_eq!(p2.lines[1].rect.x, 20.0, "each-line : L2 indentée");
}

/// §8.1 `tab-size` : les `\t` préservés (`pre`/`pre-wrap`)
// avancent au prochain taquet — multiple de `tab-size` ×
// avance d'espace — depuis le bord de départ du bloc.
#[test]
fn tab_size_avance_aux_taquets() {
    // Fixture 5px/car à font-size:10px → espace 5px,
    // tab-size:4 → taquets tous les 20px.
    let (_dom, bx) = laid(
        "<!doctype html><p>a\tb</p>",
        "p{font-size:10px;width:400px;white-space:pre;tab-size:4;margin:0}",
    );
    let p = first_box_with_lines(&bx);
    let frags = &p.lines[0].fragments;
    let b = frags
        .iter()
        .find(|f| f.text == "b")
        .unwrap_or_else(|| panic!("frags={frags:?}"));
    // « a » occupe [0,5] → taquet suivant = 20.
    assert_eq!(
        b.rect.x,
        20.0,
        "b doit partir au taquet 20 ({:?})",
        frags
            .iter()
            .map(|f| (&f.text, f.rect.x, f.rect.width))
            .collect::<Vec<_>>()
    );
    // Le fragment tabulation porte l'avance, aucun glyphe.
    let tab = frags.iter().find(|f| f.text == "\t").expect("frag tab");
    assert_eq!(tab.rect.width, 15.0, "avance tab 5→20");
    assert!(tab.shaped.is_none(), "pas de shaping sur une tab");

    // Taquet déjà atteint → avance au taquet SUIVANT.
    let (_d2, bx2) = laid(
        "<!doctype html><p>aaaa\tb</p>",
        "p{font-size:10px;width:400px;white-space:pre;tab-size:4;margin:0}",
    );
    let p2 = first_box_with_lines(&bx2);
    let b2 = p2.lines[0]
        .fragments
        .iter()
        .find(|f| f.text == "b")
        .expect("b");
    assert_eq!(b2.rect.x, 40.0, "sur un taquet → suivant ({})", b2.rect.x);

    // `\t` en tête de ligne : taquet depuis le bord du bloc.
    let (_d3, bx3) = laid(
        "<!doctype html><p>\ta</p>",
        "p{font-size:10px;width:400px;white-space:pre;tab-size:4;margin:0}",
    );
    let p3 = first_box_with_lines(&bx3);
    let a3 = p3.lines[0]
        .fragments
        .iter()
        .find(|f| f.text == "a")
        .expect("a");
    assert_eq!(a3.rect.x, 20.0, "tab en tête → taquet 20 ({})", a3.rect.x);

    // tab-size en <length> : taquets absolus de 30px.
    let (_d4, bx4) = laid(
        "<!doctype html><p>a\tb</p>",
        "p{font-size:10px;width:400px;white-space:pre;tab-size:30px;margin:0}",
    );
    let p4 = first_box_with_lines(&bx4);
    let b4 = p4.lines[0]
        .fragments
        .iter()
        .find(|f| f.text == "b")
        .expect("b");
    assert_eq!(b4.rect.x, 30.0, "length → taquet 30 ({})", b4.rect.x);

    // Sans `pre`, les tabs s'effondrent en espace ordinaire.
    let (_d5, bx5) = laid(
        "<!doctype html><p>a\tb</p>",
        "p{font-size:10px;width:400px;margin:0}",
    );
    let p5 = first_box_with_lines(&bx5);
    assert!(
        !p5.lines[0].fragments.iter().any(|f| f.text == "\t"),
        "hors pre : pas de frag tab"
    );
}

/// css-ruby-1 : l'annotation `rt` (`display:ruby-text`)
/// sort du flux vertical — posée dans la bande au-dessus
/// de la base (`ruby-position:over`), la ligne grandit ;
/// `under` la pose en dessous.
#[test]
fn ruby_rt_au_dessus_de_la_base() {
    let (_d, bx) = laid(
        "<!doctype html><p>x<ruby>B<rt>a</rt></ruby>y</p>",
        "p{font-size:20px;width:400px;margin:0}",
    );
    let p = first_box_with_lines(&bx);
    let line = &p.lines[0];
    let rt = line
        .fragments
        .iter()
        .find(|f| f.style.get("display") == Some("ruby-text"))
        .expect("frag rt");
    let base = line
        .fragments
        .iter()
        .filter(|f| f.style.get("display") != Some("ruby-text"))
        .map(|f| f.rect.y)
        .fold(f32::INFINITY, f32::min);
    assert!(
        rt.rect.y + rt.rect.height <= base + 0.01,
        "rt au-dessus de la base (rt {:?} vs base {base})",
        rt.rect
    );
    // La ligne a grandi de la hauteur de l'annotation.
    assert!(
        line.rect.height > 20.0 + rt.rect.height - 0.01,
        "ligne agrandie de l'annotation ({})",
        line.rect.height
    );

    // `ruby-position:under` : annotation sous la base, la
    // base ne bouge pas.
    let (_d2, bx2) = laid(
        "<!doctype html><p>x<ruby style=ruby-position:under>B<rt>a</rt></ruby>y</p>",
        "p{font-size:20px;width:400px;margin:0}",
    );
    let line2 = &first_box_with_lines(&bx2).lines[0];
    let rt2 = line2
        .fragments
        .iter()
        .find(|f| f.style.get("display") == Some("ruby-text"))
        .expect("frag rt under");
    let base_bot = line2
        .fragments
        .iter()
        .filter(|f| f.style.get("display") != Some("ruby-text"))
        .map(|f| f.rect.y + f.rect.height)
        .fold(0.0_f32, f32::max);
    assert!(
        rt2.rect.y >= base_bot - 0.01,
        "rt sous la base (rt {:?} vs bas {base_bot})",
        rt2.rect
    );
}

/// css-ruby-1 suite : l'annotation est **centrée** sur le
/// segment de base qui la précède (ruby-align:center
/// défaut) ; `inter-character` la cale après la base sur la
/// même ligne de base.
#[test]
fn ruby_rt_centre_et_inter_character() {
    // Base "BB" (20 px) + annotation "a" (5 px) : centre à
    // x_seg(10) + 10 - 2.5 = 17.5.
    let (_d, bx) = laid(
        "<!doctype html><p><ruby>BB<rt>a</rt></ruby></p>",
        "p{font-size:20px;width:400px;margin:0}",
    );
    let line = &first_box_with_lines(&bx).lines[0];
    let rt = line
        .fragments
        .iter()
        .find(|f| f.style.get("display") == Some("ruby-text"))
        .expect("frag rt");
    let bb = line
        .fragments
        .iter()
        .find(|f| f.style.get("display") == Some("ruby"))
        .expect("frag base");
    let want = bb.rect.x + (bb.rect.width - rt.rect.width) / 2.0;
    assert!(
        (rt.rect.x - want).abs() < 0.01,
        "rt centré sur la base : x={} attendu {want}",
        rt.rect.x
    );

    // `inter-character` : après la base, baseline alignée.
    let (_d2, bx2) = laid(
        "<!doctype html><p><ruby style=ruby-position:inter-character>BB<rt>a</rt></ruby></p>",
        "p{font-size:20px;width:400px;margin:0}",
    );
    let line2 = &first_box_with_lines(&bx2).lines[0];
    let rt2 = line2
        .fragments
        .iter()
        .find(|f| f.style.get("display") == Some("ruby-text"))
        .expect("frag rt ic");
    let bb2 = line2
        .fragments
        .iter()
        .find(|f| f.style.get("display") == Some("ruby"))
        .expect("frag base ic");
    assert!(
        (rt2.rect.x - (bb2.rect.x + bb2.rect.width)).abs() < 0.01,
        "inter-character : rt après la base ({} vs {})",
        rt2.rect.x,
        bb2.rect.x + bb2.rect.width
    );
    assert!(
        (rt2.rect.y + rt2.baseline - (bb2.rect.y + bb2.baseline)).abs() < 0.5,
        "inter-character : baselines alignées"
    );
}
