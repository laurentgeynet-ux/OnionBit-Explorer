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
