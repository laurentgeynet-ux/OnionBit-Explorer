// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tests de l'arbre de boîtes (étape 17).

use explorer_css::{Origin, StyleEngine, StyleEnv};
use explorer_dom::{Dom, NodeId};
use explorer_html::TreeBuilder;
use explorer_layout::{build_box_tree, BoxKind, LayoutBox};

/// Concatène le contenu des éléments `<style>` du DOM
/// (feuilles liées : hors périmètre de l'étape 17).
fn collect_style_css(dom: &Dom) -> String {
    let mut css = String::new();
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if dom.tag_name(n).is_ok_and(|t| t == "style") {
            if let Ok(t) = dom.text_content(n) {
                css.push_str(&t);
                css.push('\n');
            }
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
        }
    }
    css
}

/// Parse `html`, calcule les styles, construit l'arbre de
/// boîtes sous le premier élément du document.
fn tree(html: &str) -> (Dom, LayoutBox) {
    let dom = TreeBuilder::parse(html);
    let mut eng = StyleEngine::new();
    eng.add_css(&collect_style_css(&dom), Origin::Author);
    let styles = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
    // La racine des boîtes = premier élément sous le document.
    let root_el = dom
        .children(dom.document())
        .into_iter()
        .flatten()
        .find(|&c| dom.element(c).is_ok())
        .expect("un élément sous le document");
    let bx = build_box_tree(&dom, root_el, &styles).expect("arbre de boîtes");
    (dom, bx)
}

/// Cherche la première boîte portant un nœud `id` donné.
fn find(b: &LayoutBox, id: NodeId) -> Option<&LayoutBox> {
    if b.node == Some(id) {
        return Some(b);
    }
    b.children.iter().find_map(|c| find(c, id))
}

/// Trouve un nœud par tag dans le DOM (parcours en largeur).
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
    panic!("tag <{tag}> absent")
}

#[test]
fn arbre_simple() {
    let (dom, bx) = tree("<!doctype html><p>bonjour</p>");
    // html → block ; head/body blocks ; p block ; texte.
    assert_eq!(bx.kind, BoxKind::Block);
    let p = find(&bx, by_tag(&dom, "p")).expect("boîte p");
    assert_eq!(p.kind, BoxKind::Block);
    // Le texte du p est enveloppé dans un bloc anonyme.
    assert_eq!(p.children.len(), 1);
    assert_eq!(p.children[0].kind, BoxKind::AnonymousBlock);
    assert_eq!(p.children[0].children[0].kind, BoxKind::Text);
    assert_eq!(p.children[0].children[0].text.as_deref(), Some("bonjour"));
}

#[test]
fn display_none_saute_le_sous_arbre() {
    let (dom, bx) =
        tree("<!doctype html><style>p{display:none}</style><p>invisible</p><div>vu</div>");
    let p_id = by_tag(&dom, "p");
    assert!(find(&bx, p_id).is_none(), "p display:none → pas de boîte");
    assert!(find(&bx, by_tag(&dom, "div")).is_some());
}

#[test]
fn inline_et_anonyme() {
    let (dom, bx) = tree("<!doctype html><div>a <b>x</b> c</div>");
    let div = find(&bx, by_tag(&dom, "div")).expect("boîte div");
    assert_eq!(div.kind, BoxKind::Block);
    // Tout en-ligne → un seul anonyme ; blancs internes gardés.
    assert_eq!(div.children.len(), 1);
    let anon = &div.children[0];
    assert_eq!(anon.kind, BoxKind::AnonymousBlock);
    assert!(anon.node.is_none());
    assert_eq!(anon.children.len(), 3, "texte, b, texte");
    let b = find(anon, by_tag(&dom, "b")).expect("b");
    assert_eq!(b.kind, BoxKind::Inline);
}

#[test]
fn mixte_block_inline_enveloppes() {
    let (dom, bx) = tree("<!doctype html><div>tête<p>bloc</p>queue<span>fin</span></div>");
    let div = find(&bx, by_tag(&dom, "div")).expect("boîte div");
    assert_eq!(div.children.len(), 3, "anon, p, anon");
    assert_eq!(div.children[0].kind, BoxKind::AnonymousBlock);
    assert_eq!(div.children[1].kind, BoxKind::Block);
    assert_eq!(div.children[2].kind, BoxKind::AnonymousBlock);
    // Le span finit dans le second anonyme (texte + inline).
    assert_eq!(div.children[2].children.len(), 2);
    assert_eq!(div.children[2].children[1].kind, BoxKind::Inline);
}

#[test]
fn blancs_purs_en_bord_tronques() {
    let (dom, bx) = tree("<!doctype html><div>\n  <p>x</p>\n</div>");
    let div = find(&bx, by_tag(&dom, "div")).expect("div");
    // Un seul enfant : le p — pas d'anonyme pour les blancs.
    assert_eq!(div.children.len(), 1);
    assert_eq!(div.children[0].kind, BoxKind::Block);
}

#[test]
fn espace_entre_inlines_preserve() {
    let (dom, bx) = tree("<!doctype html><div><b>x</b> <i>y</i></div>");
    let div = find(&bx, by_tag(&dom, "div")).expect("div");
    let anon = &div.children[0];
    assert_eq!(anon.kind, BoxKind::AnonymousBlock);
    // b, " ", i → l'espace survit entre les deux en-lignes.
    assert_eq!(anon.children.len(), 3);
    assert_eq!(anon.children[1].kind, BoxKind::Text);
}

#[test]
fn bloc_dans_inline_eclate_le_parent() {
    // §9.2.1.1 : un bloc en flux niché dans une boîte inline
    // la scinde — le bloc remonte au niveau du conteneur
    // (R-22). Conteneur `div` : dans un `<p>`, le parseur
    // HTML aurait déjà fermé le paragraphe.
    let (dom, bx) = tree(
        "<!doctype html><div class=w>avant <span>dans <div class=in>bloc</div> fin</span> après</div>",
    );
    let w = find(&bx, by_tag(&dom, "div")).expect("conteneur");
    let kinds: Vec<BoxKind> = w.children.iter().map(|c| c.kind).collect();
    assert_eq!(
        kinds,
        [
            BoxKind::AnonymousBlock,
            BoxKind::Block,
            BoxKind::AnonymousBlock
        ],
        "le <div> interne doit couper la suite en-ligne (anon, div, anon)"
    );
    // Les deux moitiés du span restent des boîtes `Inline`.
    assert_eq!(w.children[0].children[1].kind, BoxKind::Inline);
    assert_eq!(w.children[2].children[0].kind, BoxKind::Inline);
}

#[test]
fn absolu_ne_eclate_pas_inline() {
    // `position:absolute` est hors flux — il NE scinde PAS la
    // boîte inline : il reste niché dans le span pour que son
    // bloc conteneur de positionnement reste correct
    // (R-22 — `#flag_dropdown` sous `.dropdown_parent`).
    let (dom, bx) = tree(
        "<!doctype html><style>.dp{position:relative}.a{position:absolute}</style>\
         <div class=w>av <span class=dp>x<div class=a>abs</div>y</span> ap</div>",
    );
    let span = find(&bx, by_tag(&dom, "span")).expect("span");
    assert_eq!(span.kind, BoxKind::Inline);
    // Le span garde le div absolu comme enfant (pas de split).
    assert!(
        span.children.iter().any(|c| c.kind == BoxKind::Block),
        "le div absolu doit rester niché dans le span"
    );
    // Et la ligne externe ne contient pas le contenu de
    // l'absolu : le conteneur a un seul enfant anonyme.
    let w = find(&bx, by_tag(&dom, "div")).expect("conteneur");
    assert_eq!(w.children.len(), 1);
    assert_eq!(w.children[0].kind, BoxKind::AnonymousBlock);
}

#[test]
fn inline_block_anonymise_ses_enfants() {
    // Un `inline-block` = BFC : ses enfants sont anonymisés
    // comme un conteneur bloc — les blancs entre enfants blocs
    // disparaissent (R-22 — `details.caches` de lobste.rs
    // comptait une ligne pleine par enfant blanc : hauteur
    // 4× la ligne). `details` fermé : la feuille UA masque
    // tout sauf le <summary> → un seul enfant `Block`.
    let (dom, bx) = tree(
        "<!doctype html><style>.c{display:inline-block}</style>\
         <div class=w><details class=c> <summary>sommaire</summary> <div>suite</div> </details></div>",
    );
    let d = find(&bx, by_tag(&dom, "details")).expect("details");
    assert_eq!(d.kind, BoxKind::Inline);
    let kinds: Vec<BoxKind> = d.children.iter().map(|c| c.kind).collect();
    assert_eq!(
        kinds,
        [BoxKind::Block],
        "seul le summary — blancs purgés (était [Text, Block, Text, Text])"
    );
}

#[test]
fn list_item_est_bloc_avec_puce() {
    let (dom, bx) = tree("<!doctype html><ul><li>un</li><li>deux</li></ul>");
    let li = find(&bx, by_tag(&dom, "li")).expect("li");
    assert_eq!(li.kind, BoxKind::ListItem);
}

#[test]
fn dump_wireframe_lisible() {
    let (dom, bx) = tree("<!doctype html><p>salut</p>");
    let dump = bx.dump(&dom);
    assert!(dump.contains("block"), "{dump}");
    assert!(dump.contains("anon"), "{dump}");
    assert!(dump.contains("\"salut\""), "{dump}");
    assert!(dump.contains("<p>") || dump.contains('p'), "{dump}");
}

/// `<img>` sans taille connue → boîte `Text` de l'`alt` ;
/// avec taille → boîte remplacée portant `intrinsic`.
#[test]
fn img_alt_or_intrinsic() {
    use explorer_layout::build_box_tree_sized;
    fn img_box(sizes: &std::collections::HashMap<NodeId, (f32, f32)>) -> (Dom, LayoutBox) {
        let dom =
            TreeBuilder::parse("<html><body><img src=\"x.png\" alt=\"fallback\"></body></html>");
        let mut eng = StyleEngine::new();
        eng.add_css(&collect_style_css(&dom), Origin::Author);
        let styles = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
        let root_el = dom
            .children(dom.document())
            .into_iter()
            .flatten()
            .find(|&c| dom.element(c).is_ok())
            .unwrap();
        let bx = build_box_tree_sized(&dom, root_el, &styles, sizes).unwrap();
        (dom, bx)
    }
    fn find<'a>(b: &'a LayoutBox, dom: &Dom, tag: &str) -> Option<&'a LayoutBox> {
        if b.node
            .is_some_and(|n| dom.tag_name(n).is_ok_and(|t| t == tag))
        {
            return Some(b);
        }
        b.children.iter().find_map(|c| find(c, dom, tag))
    }

    // Sans taille : repli texte `alt`.
    let (dom, bx) = img_box(&std::collections::HashMap::new());
    let img = find(&bx, &dom, "img").expect("boîte img");
    assert_eq!(img.text.as_deref(), Some("fallback"));
    assert_eq!(img.intrinsic, None);
    let img_node = img.node.unwrap();

    // Avec taille : boîte remplacée à `intrinsic`.
    let (dom2, bx2) = img_box(&std::iter::once((img_node, (40.0, 20.0))).collect());
    let img2 = find(&bx2, &dom2, "img").expect("boîte img");
    assert_eq!(img2.intrinsic, Some((40.0, 20.0)));
    assert!(img2.text.is_none());
}

/// R-34 : `<video>`/`<audio>` = remplacés **natifs** —
/// feuille atomique dont la taille intrinsèque vient de
/// l'affiche décodée (`sizes`), sinon du gabarit standard
/// (vidéo 300×150, audio 300×54) ; le contenu de repli
/// n'est jamais boxifié. Un `audio` sans `controls` est
/// `display:none` (UA) → aucune boîte.
#[test]
fn media_native_replaced_boxes() {
    use explorer_layout::build_box_tree_sized;
    fn tag_node(dom: &Dom, tag: &str) -> NodeId {
        let mut q = vec![dom.document()];
        while let Some(n) = q.pop() {
            if dom.tag_name(n).is_ok_and(|t| t == tag) {
                return n;
            }
            if let Ok(c) = dom.children(n) {
                q.extend(c);
            }
        }
        panic!("<{tag}> absent")
    }
    fn media_tree(
        html: &str,
        sizes_of: impl Fn(&Dom) -> std::collections::HashMap<NodeId, (f32, f32)>,
    ) -> (Dom, LayoutBox) {
        let dom = TreeBuilder::parse(html);
        let mut eng = StyleEngine::new();
        eng.add_css(&collect_style_css(&dom), Origin::Author);
        let styles = eng.compute_tree(&dom, dom.document(), &StyleEnv::default());
        let root_el = dom
            .children(dom.document())
            .into_iter()
            .flatten()
            .find(|&c| dom.element(c).is_ok())
            .expect("racine");
        let sizes = sizes_of(&dom);
        let bx = build_box_tree_sized(&dom, root_el, &styles, &sizes).unwrap();
        (dom, bx)
    }
    fn by_tag<'a>(b: &'a LayoutBox, dom: &Dom, tag: &str) -> Option<&'a LayoutBox> {
        if b.node
            .is_some_and(|n| dom.tag_name(n).is_ok_and(|t| t == tag))
        {
            return Some(b);
        }
        b.children.iter().find_map(|c| by_tag(c, dom, tag))
    }

    // Vidéo sans affiche : gabarit 300×150, pas d'enfants
    // (le contenu de repli n'est jamais boxifié).
    let (dom, bx) = media_tree(
        "<html><body><video><p>repli</p></video></body></html>",
        |_| std::collections::HashMap::new(),
    );
    let video = by_tag(&bx, &dom, "video").expect("boîte video");
    assert_eq!(video.intrinsic, Some((300.0, 150.0)));
    assert!(video.children.is_empty(), "feuille : {video:?}");

    // Vidéo + affiche décodée 640×360 : la taille
    // intrinsèque vient de `sizes` (pas de 300×150 forcé).
    let (dom2, bx2) = media_tree(
        "<html><body><video poster=\"p.png\"></video></body></html>",
        |d| std::iter::once((tag_node(d, "video"), (640.0, 360.0))).collect(),
    );
    let video2 = by_tag(&bx2, &dom2, "video").expect("boîte video");
    assert_eq!(video2.intrinsic, Some((640.0, 360.0)));

    // Audio avec `controls` : gabarit 300×54 de la barre.
    let (dom3, bx3) = media_tree(
        "<html><body><audio controls src=\"a.mp3\"></audio></body></html>",
        |_| std::collections::HashMap::new(),
    );
    let audio = by_tag(&bx3, &dom3, "audio").expect("boîte audio");
    assert_eq!(audio.intrinsic, Some((300.0, 54.0)));

    // Audio sans `controls` : `display:none` UA → aucune boîte.
    let (dom4, bx4) = media_tree(
        "<html><body><audio src=\"a.mp3\"></audio></body></html>",
        |_| std::collections::HashMap::new(),
    );
    assert!(
        by_tag(&bx4, &dom4, "audio").is_none(),
        "audio sans controls masqué par l'UA"
    );
}

// ── R-96 — flat tree shadow ───────────────────────────────────

#[test]
fn hote_shadow_peint_ses_enfants_shadow_pas_le_light() {
    // DSD : `<x-foo>` reçoit une racine shadow contenant
    // `<p class="inner">` ; son light DOM `<em>` n'est pas
    // distribué → absent des boîtes.
    let (dom, bx) = tree(
        "<html><body><x-foo><template shadowrootmode=\"open\">\
         <p class=\"inner\">S</p></template><em>L</em></x-foo></body></html>",
    );
    let host = by_tag(&dom, "x-foo");
    let root = dom.shadow_root(host, true).expect("racine attachée");
    let inner = dom
        .children(root)
        .unwrap()
        .into_iter()
        .find(|&c| dom.tag_name(c).is_ok_and(|t| t == "p"))
        .expect("p dans la racine");
    assert!(find(&bx, inner).is_some(), "enfant shadow en boîte");
    let em = by_tag(&dom, "em");
    assert!(
        find(&bx, em).is_none(),
        "light DOM non distribué non peint (T3)"
    );
    // Le template DSD consommé n'est pas une boîte.
    let tpl = by_tag(&dom, "template");
    assert!(find(&bx, tpl).is_none(), "template DSD consommé");
}

/// R-97 — distribution : les enfants light assignés sont
/// peints **dans** la boîte de leur `<slot>` ; le non-assigné
/// et le repli couvert ne peignent pas.
#[test]
fn slot_distribue_les_enfants_light_assignes() {
    let (dom, bx) = tree(
        "<html><body><x-foo><template shadowrootmode=\"open\">\
         <slot name=\"t\"></slot><slot><b>repli</b></slot></template>\
         <em slot=\"t\">E</em><i>I</i><u slot=\"z\">Z</u></x-foo></body></html>",
    );
    let host = by_tag(&dom, "x-foo");
    let root = dom.shadow_root(host, true).expect("racine");
    let mut slots = dom.slots_in(root).into_iter();
    let slot_t = slots.next().expect("slot t");
    let slot_def = slots.next().expect("slot defaut");

    let em = by_tag(&dom, "em");
    let i = by_tag(&dom, "i");
    let u = by_tag(&dom, "u");
    // Le `<b>` de repli vit dans la racine latérale — hors
    // BFS ordinaire : il est le seul enfant du slot défaut.
    let fb = dom
        .children(slot_def)
        .unwrap()
        .into_iter()
        .next()
        .expect("repli du slot défaut");
    let host_bx = find(&bx, host).expect("boîte hôte");

    // `em` est enfant de la boîte de `slot name=t` ; `i` de
    // celle du slot défaut — et le repli est couvert par `i`.
    let under = |parent: &explorer_layout::LayoutBox, n| {
        parent
            .children
            .iter()
            .any(|c| c.node == Some(n) || find(c, n).is_some())
    };
    assert!(under(host_bx, em), "em peint");
    assert!(under(host_bx, i), "i peint");
    let slot_t_bx = find(&bx, slot_t).expect("boîte slot t");
    assert!(
        slot_t_bx.children.iter().any(|c| c.node == Some(em)),
        "em distribué au slot t"
    );
    let slot_def_bx = find(&bx, slot_def).expect("boîte slot defaut");
    assert!(
        slot_def_bx.children.iter().any(|c| c.node == Some(i)),
        "i distribué au slot defaut"
    );
    assert!(find(&bx, u).is_none(), "slot=z orphelin non peint");
    assert!(find(&bx, fb).is_none(), "repli couvert par i");
}

/// Sans enfant assigné, le `<slot>` peint son contenu de
/// repli (css-scoping §3.4 — fallback).
#[test]
fn slot_sans_assigne_peint_son_repli() {
    let (dom, bx) = tree(
        "<html><body><x-foo><template shadowrootmode=\"open\">\
         <slot><b>repli</b></slot></template></x-foo></body></html>",
    );
    let host = by_tag(&dom, "x-foo");
    let root = dom.shadow_root(host, true).expect("racine");
    let slot = dom.slots_in(root).into_iter().next().expect("slot");
    let fb = dom
        .children(slot)
        .unwrap()
        .into_iter()
        .next()
        .expect("repli");
    assert!(find(&bx, fb).is_some(), "repli peint faute d'assigné");
}
