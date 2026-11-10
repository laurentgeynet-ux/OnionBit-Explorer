// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! R-212 — LOT-selectors : `:lang()` + tolérance des
//! pseudo-éléments préfixés vendor (`::-moz-*`…) — une
//! règle ne meurt plus pour un nom inconnu de l'autre
//! moteur (mesuré Pareto : centaines de règles Wikipedia).

use explorer_css::parse_selector_list;
use explorer_dom::Dom;
use explorer_html::TreeBuilder;
use explorer_css::matches_any;

fn dom_of(html: &str) -> Dom {
    TreeBuilder::parse(html)
}

#[test]
fn lang_matche_attribut_et_prefixe() {
    let dom = dom_of("<html lang=en><body><h1 lang=fr-CA>x</h1><ol><li>y</li></ol>");
    let h1 = {
        let mut q = vec![dom.document()];
        let mut out = None;
        while let Some(n) = q.pop() {
            if dom.tag_name(n).is_ok_and(|t| t == "h1") { out = Some(n); break; }
            if let Ok(c) = dom.children(n) { q.extend(c.iter().rev()); }
        }
        out.unwrap()
    };
    let sels = parse_selector_list("h1:lang(fr)").unwrap();
    assert!(matches_any(&dom, h1, &sels), "h1:lang(fr) doit matcher fr-CA");
    let sels = parse_selector_list("h1:lang(fr-CA)").unwrap();
    assert!(matches_any(&dom, h1, &sels));
    let sels = parse_selector_list("h1:lang(en)").unwrap();
    assert!(!matches_any(&dom, h1, &sels), "en ne doit pas matcher fr-CA");
}

#[test]
fn lang_herite_de_l_ancetre() {
    let dom = dom_of("<html lang=fr><body><ol><li>x</li></ol>");
    let li = {
        let mut q = vec![dom.document()];
        let mut out = None;
        while let Some(n) = q.pop() {
            if dom.tag_name(n).is_ok_and(|t| t == "li") { out = Some(n); break; }
            if let Ok(c) = dom.children(n) { q.extend(c.iter().rev()); }
        }
        out.unwrap()
    };
    // li n'a pas de lang propre — hérite de html lang=fr.
    let sels = parse_selector_list("ol:lang(fr) li").unwrap();
    assert!(matches_any(&dom, li, &sels), "lang héritée de l'ancêtre");
}

#[test]
fn lang_liste_et_joker() {
    let sels = parse_selector_list("p:lang(de, fr, *)");
    assert!(sels.is_some(), "liste + joker doivent parser");
    assert!(parse_selector_list("p:lang()").is_none(), "vide = invalide");
}

#[test]
fn vendor_pseudo_ne_tue_plus_la_liste() {
    // `::-moz-focus-inner` est hors whitelist — toléré dormant.
    let sels = parse_selector_list("button::-moz-focus-inner, a");
    assert!(sels.is_some(), "vendor pseudo ne doit pas invalider la liste");
    // Un nom vraiment inconnu reste invalide (spec).
    assert!(parse_selector_list("a::totalement-inconnu").is_none());
}

// ── R-217 — `:has(<relatif>)` : combinateurs de tête ──────

#[test]
fn has_fils_direct() {
    let dom = dom_of(
        "<html><body><div id=a><span><b>x</b></span></div><div id=b><b>y</b></div>",
    );
    let divs = {
        let mut out = Vec::new();
        let mut q = vec![dom.document()];
        while let Some(n) = q.pop() {
            if dom.tag_name(n).is_ok_and(|t| t == "div") {
                out.push(n);
            }
            if let Ok(c) = dom.children(n) {
                q.extend(c.iter().rev());
            }
        }
        out
    };
    let sels = parse_selector_list("div:has(> b)").unwrap();
    assert!(!matches_any(&dom, divs[0], &sels), "b n'est pas fils direct de a");
    assert!(matches_any(&dom, divs[1], &sels), "b est fils direct de la 2e div");
}

#[test]
fn has_frere_suivant() {
    let dom = dom_of("<html><body><ul><li>a</li><li>b</li></ul>");
    let lis = {
        let mut out = Vec::new();
        let mut q = vec![dom.document()];
        while let Some(n) = q.pop() {
            if dom.tag_name(n).is_ok_and(|t| t == "li") { out.push(n); }
            if let Ok(c) = dom.children(n) { q.extend(c.iter().rev()); }
        }
        out
    };
    // li:has(+ li) — le premier a un frère suivant, le second non.
    let sels = parse_selector_list("li:has(+ li)").unwrap();
    assert!(matches_any(&dom, lis[0], &sels));
    assert!(!matches_any(&dom, lis[1], &sels));
    // li:has(~ li) idem toute distance.
    let sels = parse_selector_list("li:has(~ li)").unwrap();
    assert!(matches_any(&dom, lis[0], &sels));
}

#[test]
fn has_liste_relative_mixte() {
    let sels = parse_selector_list("a:has(> img), a:has(span)");
    assert!(sels.is_some(), "liste relative mixte doit parser");
    // La forme implicite reste fonctionnelle.
    assert!(parse_selector_list("div:has(span.x)").is_some());
}
