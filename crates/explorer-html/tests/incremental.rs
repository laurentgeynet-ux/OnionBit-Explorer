// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Conduite incrémentale `push`/`finish` (R-1) : le document en
//! cours de construction est observable entre les chunks —
//! arbre toujours cohérent à frontière de token, `NodeId`
//! jamais collectés tant que le builder vit, et convergence
//! exacte vers le parse complet une fois le flux clos.

use explorer_dom::{Dom, NodeId};
use explorer_html::debug::dump;
use explorer_html::{Tokenizer, TreeBuilder};

/// Document volontairement tordu : rawtext coupé, entité
/// découpée, table jamais fermée, formatage ouvert.
const TRICKY: &str = "<!DOCTYPE html><p>un<b>deux</b>&amp;<table><tr><td>x\
    </table><style>a{b:c}<script>if(a<b){}</script><!--fin";

// -- Convergence --------------------------------------------------------------

#[test]
fn push_finish_converge_vers_parse_a_toute_coupure() {
    let attendu = dump(&TreeBuilder::parse(TRICKY));
    for cut in 0..=TRICKY.len() {
        let mut tb = TreeBuilder::new();
        tb.push(&TRICKY[..cut]);
        tb.push(&TRICKY[cut..]);
        tb.finish();
        assert_eq!(dump(tb.dom()), attendu, "coupure à l'octet {cut}");
    }
}

#[test]
fn push_par_petits_morceaux() {
    let attendu = dump(&TreeBuilder::parse(TRICKY));
    // Chunks de 7 octets ASCII — frontières arbitraires au
    // milieu de balises, entités, rawtext.
    let mut tb = TreeBuilder::new();
    for chunk in TRICKY.as_bytes().chunks(7) {
        tb.push(std::str::from_utf8(chunk).expect("ascii"));
    }
    tb.finish();
    assert_eq!(dump(tb.dom()), attendu);
}

// -- Cohérence entre pushes ----------------------------------------------------

#[test]
fn dom_observable_et_coherent_entre_pushes() {
    let mut tb = TreeBuilder::new();
    tb.push("  <!-- préambule -->");
    // Que des non-éléments : pas encore de racine à peindre.
    assert!(tb.dom().document_element().is_none());
    tb.push("<p>premier");
    // Le texte non fermé est déjà inséré — la frame intermédiaire
    // le voit.
    let html = tb.dom().document_element().expect("élément racine");
    let texte = tb.dom().text_content(html).expect("text_content");
    assert!(texte.contains("premier"), "texte attendu : {texte:?}");
    tb.finish();
}

#[test]
fn rawtext_partiel_visible_entre_pushes() {
    let mut tb = TreeBuilder::new();
    tb.push("<style>a{b");
    // Le contenu rawtext accumulé atteint le DOM sans attendre
    // `</style>` — la frame intermédiaire peut déjà styler ce
    // qui est arrivé (ou ignorer un bout de déclaration — les
    // deux sont des états cohérents).
    let html = tb.dom().document_element().expect("élément racine");
    let css = tb.dom().text_content(html).expect("text_content");
    assert!(css.contains("a{b"), "rawtext partiel attendu : {css:?}");
    tb.finish();
}

#[test]
fn node_ids_ne_meurent_pas_pendant_la_construction() {
    // Une frame peinte entre deux pushes référence des NodeId
    // (hit-test, tables d'images) : tant que le builder vit,
    // aucun nœud inséré n'est collecté — invariant R-1.
    let mut tb = TreeBuilder::new();
    let mut ids = Vec::new();
    for chunk in [
        "<div>a</div><p>b",
        "<table><tr><td>c</table>",
        "<b>d<i>e</b></i>",
    ] {
        tb.push(chunk);
        collect_ids(tb.dom(), tb.dom().document(), &mut ids);
    }
    tb.finish();
    assert_ne!(ids.len(), 0);
    for id in ids {
        assert!(tb.dom().is_alive(id), "NodeId {id:?} collecté en route");
    }
}

fn collect_ids(dom: &Dom, id: NodeId, out: &mut Vec<NodeId>) {
    out.push(id);
    for c in dom.children(id).expect("children") {
        collect_ids(dom, c, out);
    }
}

// -- Fin de flux -----------------------------------------------------------------

#[test]
fn push_apres_finish_est_ignore() {
    let mut tb = TreeBuilder::new();
    tb.push("<p>fin");
    tb.finish();
    let fige = dump(tb.dom());
    tb.push("<p>tardif");
    tb.finish(); // idempotent
    assert_eq!(dump(tb.dom()), fige);
}

#[test]
fn finish_sur_flux_vide() {
    // La spec synthétise html/head/body même sans contenu.
    let mut tb = TreeBuilder::new();
    tb.finish();
    assert_eq!(dump(tb.dom()), dump(&TreeBuilder::parse("")));
}

#[test]
fn tokenizer_is_finished() {
    let mut t = Tokenizer::new();
    assert!(!t.is_finished());
    t.feed("x");
    assert!(!t.is_finished());
    t.finish();
    assert!(t.is_finished());
}
