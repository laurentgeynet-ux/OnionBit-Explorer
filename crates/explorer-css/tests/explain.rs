// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! R-301 — verrous de [`StyleEngine::explain`] : la
//! taxonomie `DeclVerdict` enregistrée à l'endroit réel de
//! la décision (tri, portes de collecte, validation,
//! `var()`, résolutions post-cascade, assise).

use explorer_css::{DeclTrace, DeclVerdict, ElimKey, GateKind, Origin, StyleEngine, StyleEnv};
use explorer_dom::{Dom, NodeId};
use explorer_html::TreeBuilder;

/// Premier élément portant `attr = v` (ordre du document).
fn find(dom: &Dom, attr: &str, v: &str) -> Option<NodeId> {
    fn walk(dom: &Dom, n: NodeId, attr: &str, v: &str, out: &mut Option<NodeId>) {
        if out.is_some() {
            return;
        }
        if dom
            .attribute_str(n, attr)
            .ok()
            .flatten()
            .is_some_and(|a| a == v)
        {
            *out = Some(n);
            return;
        }
        if let Ok(ch) = dom.children(n) {
            for c in ch {
                walk(dom, c, attr, v, out);
            }
        }
    }
    let mut out = None;
    walk(dom, dom.document(), attr, v, &mut out);
    out
}

/// `explain` du nœud `attr="v"` — `<!doctype html>` partout
/// (mode standards : la table quirks UA est `gated(mode)`).
/// `want_parent` = style calculé du parent (verdicts
/// `inherited`).
fn explain_on(
    html: &str,
    css: &str,
    attr: &str,
    v: &str,
    want_parent: bool,
) -> (Dom, NodeId, Vec<DeclTrace>) {
    explain_with(html, css, attr, v, want_parent, |e, _, _, _| e)
}

/// Variante permettant de retoucher le moteur avant
/// `explain` (feuilles scopées, origine UA…).
fn explain_with(
    html: &str,
    css: &str,
    attr: &str,
    v: &str,
    want_parent: bool,
    tune: impl FnOnce(StyleEngine, &Dom, NodeId, &StyleEnv) -> StyleEngine,
) -> (Dom, NodeId, Vec<DeclTrace>) {
    let dom = TreeBuilder::parse(html);
    let node = find(&dom, attr, v).expect("nœud cible");
    let env = StyleEnv::default();
    let mut eng = StyleEngine::new();
    if !css.is_empty() {
        eng.add_css(css, Origin::Author);
    }
    let eng = tune(eng, &dom, node, &env);
    let parent_style = if want_parent {
        let sm = eng.compute_tree(&dom, dom.document(), &env);
        dom.parent(node)
            .ok()
            .flatten()
            .and_then(|p| sm.get(p).cloned())
    } else {
        None
    };
    let traces = eng.explain(
        &dom,
        node,
        &env,
        parent_style.as_ref(),
        None,
        &|_: &str| false,
        None,
    );
    (dom, node, traces)
}

fn by_prop<'a>(traces: &'a [DeclTrace], prop: &str) -> Vec<&'a DeclTrace> {
    traces.iter().filter(|t| t.prop == prop).collect()
}

fn verdicts(traces: &[DeclTrace], prop: &str) -> Vec<DeclVerdict> {
    by_prop(traces, prop).iter().map(|t| t.verdict).collect()
}

#[test]
fn winner_et_elimine_specificite() {
    let (_, _, t) = explain_on(
        "<!doctype html><body><p id=b class=a>x</p></body>",
        ".a { color: red } #b { color: blue }",
        "id",
        "b",
        false,
    );
    let c = by_prop(&t, "color");
    let win = c
        .iter()
        .find(|t| t.verdict == DeclVerdict::Winner)
        .expect("gagnant");
    assert_eq!(win.selector, "#b");
    assert_eq!(win.value.trim(), "blue");
    let loser = c
        .iter()
        .find(|t| matches!(t.verdict, DeclVerdict::EliminatedBy(_)))
        .expect("éliminé");
    assert_eq!(loser.selector, ".a");
    assert_eq!(
        loser.verdict,
        DeclVerdict::EliminatedBy(ElimKey::Specificity)
    );
}

#[test]
fn ordre_source_departage_a_cle_egale() {
    let (_, _, t) = explain_on(
        "<!doctype html><body><p class=\"a c\">x</p></body>",
        ".a { color: red } .c { color: blue }",
        "class",
        "a c",
        false,
    );
    let c = by_prop(&t, "color");
    let loser = c
        .iter()
        .find(|t| matches!(t.verdict, DeclVerdict::EliminatedBy(_)))
        .expect("éliminé");
    // Même rang, même layer, même spécificité → l'ordre.
    assert_eq!(loser.verdict, DeclVerdict::EliminatedBy(ElimKey::Order));
    assert_eq!(loser.selector, ".a");
}

#[test]
fn important_renverse_la_specificite() {
    let (_, _, t) = explain_on(
        "<!doctype html><body><p id=b>x</p></body>",
        "p { color: red !important } #b { color: blue }",
        "id",
        "b",
        false,
    );
    let c = by_prop(&t, "color");
    let win = c
        .iter()
        .find(|t| t.verdict == DeclVerdict::Winner)
        .expect("gagnant");
    // `!important` vit dans `Declaration.important` → rank 2
    // (auteur!important) ; la valeur portée est la déclarée.
    assert_eq!(win.value.trim(), "red");
    assert_eq!(win.rank, 2);
    // Le gagnant `!important` élimine le perdant sur `rank`
    // (origine×important) — la spécificité n'est pas atteinte.
    let loser = c
        .iter()
        .find(|t| matches!(t.verdict, DeclVerdict::EliminatedBy(_)))
        .expect("éliminé");
    assert_eq!(loser.verdict, DeclVerdict::EliminatedBy(ElimKey::Origin));
}

#[test]
fn layer_perd_contre_hors_layer() {
    let (_, _, t) = explain_on(
        "<!doctype html><body><p id=b>x</p></body>",
        "@layer a { p { color: red } } p { color: blue }",
        "id",
        "b",
        false,
    );
    let c = by_prop(&t, "color");
    let win = c
        .iter()
        .find(|t| t.verdict == DeclVerdict::Winner)
        .expect("gagnant");
    assert_eq!(win.value.trim(), "blue");
    assert_eq!(win.layer, vec![u32::MAX]);
    let loser = c
        .iter()
        .find(|t| matches!(t.verdict, DeclVerdict::EliminatedBy(_)))
        .expect("éliminé");
    assert_eq!(loser.verdict, DeclVerdict::EliminatedBy(ElimKey::Layer));
}

#[test]
fn gated_media_container_pseudo() {
    let (_, _, t) = explain_on(
        "<!doctype html><body><p id=b>x</p></body>",
        "@media (max-width: 1px) { p { color: red } } \
         @container (min-width: 9999px) { p { color: green } } \
         #b::before { color: yellow }",
        "id",
        "b",
        false,
    );
    let c = by_prop(&t, "color");
    assert!(
        c.iter()
            .any(|t| t.verdict == DeclVerdict::Gated(GateKind::Media) && t.value.trim() == "red"),
        "gated(media) attendu"
    );
    assert!(
        c.iter()
            .any(|t| t.verdict == DeclVerdict::Gated(GateKind::Container)
                && t.value.trim() == "green"),
        "gated(container) attendu"
    );
    assert!(
        c.iter()
            .any(|t| t.verdict == DeclVerdict::Gated(GateKind::Pseudo)),
        "gated(pseudo) attendu"
    );
}

#[test]
fn gated_scope_feuille_etrangere() {
    // Règle confinée à un AUTRE arbre : pour un nœud du
    // document elle est refusée par `selector_reaches`.
    let (dom, node, t) = explain_with(
        "<!doctype html><body><p id=b>x</p></body>",
        "p { color: blue }",
        "id",
        "b",
        false,
        |mut eng, dom, node, _| {
            // Scope factice : n'importe quel `NodeId` joue le
            // rôle de racine — la porte compare les `Option`.
            let host = dom.parent(node).ok().flatten().unwrap_or(node);
            eng.add_scoped_css("p { color: purple }", Origin::Author, host);
            eng
        },
    );
    let _ = (dom, node);
    let c = by_prop(&t, "color");
    assert!(
        c.iter()
            .any(|t| t.verdict == DeclVerdict::Gated(GateKind::Scope) && t.value.trim() == "purple"),
        "gated(scope) attendu : {c:?}"
    );
}

#[test]
fn gated_mode_table_quirks_en_standards() {
    // `<table>` est ciblé par la feuille quirks UA — en
    // standards elle sort en `gated(mode)` au lieu d'être
    // collectée silencieusement.
    let (_, _, t) = explain_on(
        "<!doctype html><body><table id=b><tr><td>x</td></tr></table></body>",
        "",
        "id",
        "b",
        false,
    );
    assert!(
        t.iter()
            .any(|t| t.verdict == DeclVerdict::Gated(GateKind::Mode)),
        "gated(mode) attendu sur les reset UA quirks"
    );
}

#[test]
fn validation_droppe_la_declaration() {
    let (_, _, t) = explain_on(
        "<!doctype html><body><p id=b>x</p></body>",
        "p { width: banana; color: blue }",
        "id",
        "b",
        false,
    );
    // `width` exige longueur/% — `banana` n'écrit rien
    // (`value_is_valid_for` ne couvre que les familles de
    // longueurs et les raccourcis).
    assert_eq!(
        verdicts(&t, "width")
            .iter()
            .filter(|v| **v == DeclVerdict::DroppedValidation)
            .count(),
        1
    );
    assert!(by_prop(&t, "color")
        .iter()
        .any(|t| t.verdict == DeclVerdict::Winner));
}

#[test]
fn var_irresolu_invalide_au_calcul() {
    let (_, _, t) = explain_on(
        "<!doctype html><body><p id=b>x</p></body>",
        "p { color: var(--absent) }",
        "id",
        "b",
        false,
    );
    assert!(
        by_prop(&t, "color")
            .iter()
            .any(|t| t.verdict == DeclVerdict::InvalidAtComputed),
        "invalid_at_computed attendu"
    );
}

#[test]
fn shorthand_expande_par_longhand() {
    let (_, _, t) = explain_on(
        "<!doctype html><body><p id=b>x</p></body>",
        "p { margin: 1px } #b { margin-top: 5px }",
        "id",
        "b",
        false,
    );
    // `margin` écrit 4 longhands : `margin-top` perd sur la
    // spécificité, les trois autres restent gagnants.
    let top = by_prop(&t, "margin-top");
    assert!(
        top.iter().any(|t| {
            t.value.trim() == "1px" && t.verdict == DeclVerdict::EliminatedBy(ElimKey::Specificity)
        }),
        "longhand du shorthand éliminée : {top:?}"
    );
    assert!(top
        .iter()
        .any(|t| t.value.trim() == "5px" && t.verdict == DeclVerdict::Winner));
    for side in ["margin-right", "margin-bottom", "margin-left"] {
        assert!(
            by_prop(&t, side)
                .iter()
                .any(|t| t.value.trim() == "1px" && t.verdict == DeclVerdict::Winner),
            "{side} devrait rester gagnant"
        );
    }
}

#[test]
fn inline_marque_et_gagne() {
    let (_, _, t) = explain_on(
        "<!doctype html><body><p id=b style=\"color: green\">x</p></body>",
        "p { color: red }",
        "id",
        "b",
        false,
    );
    let c = by_prop(&t, "color");
    let win = c
        .iter()
        .find(|t| t.verdict == DeclVerdict::Winner)
        .expect("gagnant");
    assert!(win.inline);
    assert_eq!(win.selector, "");
    assert!(c
        .iter()
        .any(|t| matches!(t.verdict, DeclVerdict::EliminatedBy(_)) && !t.inline));
}

#[test]
fn presentational_hint_rang_zero() {
    // `<img width>` = hint de présentation — n'écrit que si
    // rien n'a déclaré `width`.
    let (_, _, t) = explain_on(
        "<!doctype html><body><img id=b width=64></body>",
        "",
        "id",
        "b",
        false,
    );
    assert!(
        by_prop(&t, "width")
            .iter()
            .any(|t| t.verdict == DeclVerdict::PresentationalHint),
        "hint attendu"
    );
}

#[test]
fn hint_cede_a_la_regle() {
    let (_, _, t) = explain_on(
        "<!doctype html><body><img id=b width=64></body>",
        "img { width: 10px }",
        "id",
        "b",
        false,
    );
    // La règle a posé `width` : le hint ne passe pas (aucune
    // trace `PresentationalHint` pour `width`).
    assert!(
        !verdicts(&t, "width").contains(&DeclVerdict::PresentationalHint),
        "le hint n'écrit pas par-dessus une déclaration"
    );
}

#[test]
fn herite_avec_provenance() {
    let (dom, node, t) = explain_on(
        "<!doctype html><body><div id=p><p id=b>x</p></div></body>",
        "div { color: teal }",
        "id",
        "b",
        true,
    );
    let parent = dom.parent(node).ok().flatten();
    let c = by_prop(&t, "color");
    let inh = c
        .iter()
        .find(|t| t.verdict == DeclVerdict::Inherited)
        .expect("hérité");
    assert_eq!(inh.value.trim(), "teal");
    assert_eq!(inh.inherited_from, parent);
}

#[test]
fn initial_pour_non_declaree() {
    let (_, _, t) = explain_on(
        "<!doctype html><body><p id=b>x</p></body>",
        "p { color: blue }",
        "id",
        "b",
        false,
    );
    assert!(
        verdicts(&t, "z-index").contains(&DeclVerdict::Initial),
        "initial attendu pour une longhand jamais déclarée"
    );
}

#[test]
fn superseded_par_resolution_logique() {
    // UA `figure { margin-left: 40px }` (rang 0) vs auteur
    // `margin-inline-start` (rang 1) : la résolution
    // logique→physique écrase le physique moindre (R-215).
    let (_, _, t) = explain_on(
        "<!doctype html><body><figure id=b>x</figure></body>",
        "figure { margin-inline-start: 5px }",
        "id",
        "b",
        false,
    );
    assert!(
        by_prop(&t, "margin-left")
            .iter()
            .any(|t| t.verdict == DeclVerdict::Superseded && t.value.trim() == "40px"),
        "UA margin-left superseded attendu"
    );
    assert!(by_prop(&t, "margin-inline-start")
        .iter()
        .any(|t| t.verdict == DeclVerdict::Winner));
}

#[test]
fn superseded_par_text_fill() {
    let (_, _, t) = explain_on(
        "<!doctype html><body><p id=b>x</p></body>",
        "p { color: red; -webkit-text-fill-color: blue }",
        "id",
        "b",
        false,
    );
    assert!(
        by_prop(&t, "color")
            .iter()
            .any(|t| t.verdict == DeclVerdict::Superseded && t.value.trim() == "red"),
        "color superseded attendu (R-235)"
    );
}

#[test]
fn selector_text_retenu_a_la_collecte() {
    // Le prélude multi-sélecteurs est découpé par virgule
    // de tête — chaque trace nomme SON sélecteur.
    let (_, _, t) = explain_on(
        "<!doctype html><body><p id=b>x</p></body>",
        "span, #b { color: pink }",
        "id",
        "b",
        false,
    );
    let c = by_prop(&t, "color");
    assert!(
        c.iter()
            .any(|t| t.selector == "#b" && t.verdict == DeclVerdict::Winner),
        "sélecteur source attendu : {c:?}"
    );
}

#[test]
fn explain_reproduit_le_computed() {
    // La trace `Winner` par propriété == le `ComputedStyle`
    // réel — le puits ne ment pas sur la cascade.
    let html = "<!doctype html><body><p id=b>x</p></body>";
    let css = "p { color: blue; padding-left: 3px } .z { color: red }";
    let dom = TreeBuilder::parse(html);
    let node = find(&dom, "id", "b").expect("nœud");
    let env = StyleEnv::default();
    let mut eng = StyleEngine::new();
    eng.add_css(css, Origin::Author);
    let sm = eng.compute_tree(&dom, dom.document(), &env);
    let computed = sm.get(node).cloned().expect("style");
    let parent = dom
        .parent(node)
        .ok()
        .flatten()
        .and_then(|p| sm.get(p).cloned());
    let t = eng.explain(
        &dom,
        node,
        &env,
        parent.as_ref(),
        None,
        &|_: &str| false,
        None,
    );
    let colors = by_prop(&t, "color");
    let win = colors
        .iter()
        .find(|t| t.verdict == DeclVerdict::Winner)
        .expect("gagnant");
    assert_eq!(computed.get("color"), Some(win.value.trim()));
}
