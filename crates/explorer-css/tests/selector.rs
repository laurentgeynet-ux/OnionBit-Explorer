// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tests de parsing et de matching des sélecteurs CSS.

use explorer_css::{
    matches, matches_any, parse_selector_list, AttrOp, Combinator, PseudoClass, SimpleSelector,
};
use explorer_dom::{Dom, NodeId};

/// DOM de test :
/// `div#a.x[data-kind="main"] > p.b "un" ; span "deux" ; p.c.y[lang=fr-FR]`
/// + `ul > li×3` (nth-child), `section` vide, `em` imbriqué dans `p.c`.
struct Fixture {
    dom: Dom,
    div: NodeId,
    p1: NodeId,
    span: NodeId,
    p2: NodeId,
    em: NodeId,
    lis: Vec<NodeId>,
    section: NodeId,
}

fn fixture() -> Fixture {
    let mut dom = Dom::new();
    let doc = dom.document();

    let div = dom.create_element("div").unwrap();
    dom.set_attribute(div, "id", "a").unwrap();
    dom.set_attribute(div, "class", "x").unwrap();
    dom.set_attribute(div, "data-kind", "main").unwrap();
    dom.append_child(doc, div).unwrap();

    let p1 = dom.create_element("p").unwrap();
    dom.set_attribute(p1, "class", "b").unwrap();
    dom.append_child(div, p1).unwrap();
    let t1 = dom.create_text("un").unwrap();
    dom.append_child(p1, t1).unwrap();

    // Texte entre les éléments — les combinateurs frères doivent
    // sauter les nœuds non-élément.
    let gap = dom.create_text(" ").unwrap();
    dom.append_child(div, gap).unwrap();

    let span = dom.create_element("span").unwrap();
    dom.append_child(div, span).unwrap();
    let t2 = dom.create_text("deux").unwrap();
    dom.append_child(span, t2).unwrap();

    let p2 = dom.create_element("p").unwrap();
    dom.set_attribute(p2, "class", "c y").unwrap();
    dom.set_attribute(p2, "lang", "fr-FR").unwrap();
    dom.append_child(div, p2).unwrap();
    let em = dom.create_element("em").unwrap();
    dom.append_child(p2, em).unwrap();
    let t3 = dom.create_text("ital").unwrap();
    dom.append_child(em, t3).unwrap();

    let ul = dom.create_element("ul").unwrap();
    dom.append_child(doc, ul).unwrap();
    let mut lis = Vec::new();
    for _ in 0..3 {
        let li = dom.create_element("li").unwrap();
        dom.append_child(ul, li).unwrap();
        lis.push(li);
    }

    let section = dom.create_element("section").unwrap();
    dom.append_child(doc, section).unwrap();

    Fixture {
        dom,
        div,
        p1,
        span,
        p2,
        em,
        lis,
        section,
    }
}

/// Premier sélecteur d'un prélude (un seul attendu).
fn sel(prelude: &str) -> explorer_css::ComplexSelector {
    parse_selector_list(prelude)
        .and_then(|l| l.into_iter().next())
        .unwrap_or_else(|| panic!("sélecteur non parsé : {prelude}"))
}

fn list(prelude: &str) -> Vec<explorer_css::ComplexSelector> {
    parse_selector_list(prelude).unwrap_or_else(|| panic!("sélecteur non parsé : {prelude}"))
}

// ── parsing ──────────────────────────────────────────────────

#[test]
fn parse_type_classe_id() {
    let s = sel("div.a#b");
    let simples = &s.parts[0].0.simples;
    assert_eq!(simples[0], SimpleSelector::Type("div".into()));
    assert_eq!(simples[1], SimpleSelector::Class("a".into()));
    assert_eq!(simples[2], SimpleSelector::Id("b".into()));
}

#[test]
fn parse_liste_et_combinateurs() {
    let l = list("div > p, span + em, a ~ b, x y");
    assert_eq!(l.len(), 4);
    assert_eq!(l[0].parts[0].1, Some(Combinator::Child));
    assert_eq!(l[1].parts[0].1, Some(Combinator::NextSibling));
    assert_eq!(l[2].parts[0].1, Some(Combinator::SubsequentSibling));
    assert_eq!(l[3].parts[0].1, Some(Combinator::Descendant));
}

#[test]
fn parse_attributs_tous_ops() {
    let l = list("[a][b=v][c~=v][d|=v][e^=v][f$=v][g*=v][h=v i]");
    let ops: Vec<AttrOp> = l[0]
        .parts
        .iter()
        .flat_map(|(c, _)| c.simples.iter())
        .map(|s| match s {
            SimpleSelector::Attribute { op, .. } => *op,
            _ => AttrOp::Exists,
        })
        .collect();
    assert_eq!(
        ops,
        vec![
            AttrOp::Exists,
            AttrOp::Equals,
            AttrOp::Includes,
            AttrOp::DashMatch,
            AttrOp::Prefix,
            AttrOp::Suffix,
            AttrOp::Substring,
            AttrOp::Equals,
        ]
    );
    // Drapeau i sur le dernier.
    let last = &l[0].parts[0].0.simples[7];
    assert!(matches!(
        last,
        SimpleSelector::Attribute {
            case_insensitive: true,
            ..
        }
    ));
}

#[test]
fn parse_pseudo_et_nth() {
    assert!(matches!(
        sel("p:first-child").parts[0].0.simples[1],
        SimpleSelector::Pseudo(PseudoClass::FirstChild)
    ));
    assert!(matches!(
        sel("li:nth-child(2n+1)").parts[0].0.simples[1],
        SimpleSelector::Pseudo(PseudoClass::NthChild(2, 1))
    ));
    assert!(matches!(
        sel("li:nth-child(odd)").parts[0].0.simples[1],
        SimpleSelector::Pseudo(PseudoClass::NthChild(2, 1))
    ));
    assert!(matches!(
        sel("li:nth-child(even)").parts[0].0.simples[1],
        SimpleSelector::Pseudo(PseudoClass::NthChild(2, 0))
    ));
    assert!(matches!(
        sel("li:nth-child(3)").parts[0].0.simples[1],
        SimpleSelector::Pseudo(PseudoClass::NthChild(0, 3))
    ));
    assert!(matches!(
        sel("li:nth-child(-n+2)").parts[0].0.simples[1],
        SimpleSelector::Pseudo(PseudoClass::NthChild(-1, 2))
    ));
}

#[test]
fn parse_not_is_where() {
    let s = sel("div:not(.x)");
    assert!(matches!(
        s.parts[0].0.simples[1],
        SimpleSelector::Pseudo(PseudoClass::Not(_))
    ));
    assert!(matches!(
        sel("p:is(.a, .b)").parts[0].0.simples[1],
        SimpleSelector::Pseudo(PseudoClass::Is(_))
    ));
    assert_eq!(sel("p:where(.a)").specificity(), (0, 0, 1));
}

#[test]
fn specificity_calculee() {
    assert_eq!(sel("#a").specificity(), (1, 0, 0));
    assert_eq!(sel(".a.b").specificity(), (0, 2, 0));
    assert_eq!(sel("div p").specificity(), (0, 0, 2));
    assert_eq!(sel("#a .b > [c] p:first-child").specificity(), (1, 3, 1));
    assert_eq!(sel("*.a").specificity(), (0, 1, 0));
    // :not prend la spécificité max de sa liste.
    assert_eq!(sel("div:not(#a)").specificity(), (1, 0, 1));
}

#[test]
fn selecteurs_invalides_rejetes() {
    for bad in [
        "",
        ">",
        ".",
        "#",
        "[",
        "[a",
        "[a=",
        "p:",
        "p:unknown",
        "::unknown",
        "a, b >",
        ".a..",
        "[a~]",
    ] {
        assert!(parse_selector_list(bad).is_none(), "attendu None : {bad}");
    }
}

// ── matching ─────────────────────────────────────────────────

#[test]
fn match_type_universel() {
    let f = fixture();
    assert!(matches(&f.dom, f.div, &sel("div")));
    assert!(matches(&f.dom, f.p1, &sel("*")));
    assert!(!matches(&f.dom, f.span, &sel("div")));
}

#[test]
fn match_id_classe() {
    let f = fixture();
    assert!(matches(&f.dom, f.div, &sel("#a")));
    assert!(matches(&f.dom, f.div, &sel(".x")));
    assert!(matches(&f.dom, f.p2, &sel(".c")));
    assert!(matches(&f.dom, f.p2, &sel(".y")));
    assert!(matches(&f.dom, f.p2, &sel(".c.y")));
    assert!(!matches(&f.dom, f.p1, &sel(".c")));
    assert!(!matches(&f.dom, f.div, &sel("#b")));
}

#[test]
fn match_attributs() {
    let f = fixture();
    assert!(matches(&f.dom, f.div, &sel("[data-kind]")));
    assert!(matches(&f.dom, f.div, &sel("[data-kind=main]")));
    assert!(!matches(&f.dom, f.div, &sel("[data-kind=MAIN]")));
    assert!(matches(&f.dom, f.div, &sel("[data-kind=MAIN i]")));
    assert!(matches(&f.dom, f.p2, &sel("[class~=y]")));
    assert!(matches(&f.dom, f.p2, &sel("[lang|=fr]")));
    assert!(matches(&f.dom, f.div, &sel("[data-kind^=ma]")));
    assert!(matches(&f.dom, f.div, &sel("[data-kind$=ain]")));
    assert!(matches(&f.dom, f.div, &sel("[data-kind*=ai]")));
    assert!(!matches(&f.dom, f.p1, &sel("[lang]")));
}

/// R-72 — valeurs d'attribut multi-octets : les découpes
/// `^=`/`$=`/`|=` en octets tombaient hors frontière de
/// char → panic (vol terrain, en.wikipedia.org 'õ').
/// `get()` → `None` = `false` : une coupe hors frontière
/// ne peut de toute façon pas égaler `expected`.
#[test]
fn match_attributs_utf8_ne_panique_pas() {
    let mut dom = Dom::new();
    let doc = dom.document();
    let el = dom.create_element("div").unwrap();
    // 'ç'/'õ' = 2 octets UTF-8 — une coupe à 1 octet tombe
    // en plein char, aux deux bouts.
    dom.set_attribute(el, "data-x", "çbõ").unwrap();
    dom.set_attribute(el, "lang", "pt-BR").unwrap();
    dom.append_child(doc, el).unwrap();
    // Panics pré-fix sur chacun de ces sélecteurs — tous
    // doivent répondre sans planter.
    let checks: &[(&str, bool)] = &[
        ("[data-x^=x]", false), // coupe à 1 → milieu de 'ç'
        ("[data-x^=ç]", true),  // frontière valide
        ("[data-x^=çb]", true),
        ("[data-x$=x]", false), // coupe à 1 → milieu de 'õ'
        ("[data-x$=õ]", true),
        ("[data-x$=bõ]", true),
        ("[data-x|=x]", false), // préfixe + '-' — milieu 'ç'
        ("[data-x|=çbõ]", true),
        ("[lang|=pt]", true),
        ("[lang|=fr]", false),
        ("[data-x~=çbõ]", true),
        ("[data-x*=õ]", true),
        ("[data-x*=çb]", true),
        ("[data-x*=x]", false),
        ("[data-x=çbõ]", true),
        ("[data-x^=\"\"]", true), // vide = préfixe universel
    ];
    for (s, want) in checks {
        assert_eq!(matches(&dom, el, &sel(s)), *want, "sélecteur {s}");
    }
}

#[test]
fn match_descendant_et_enfant() {
    let f = fixture();
    // em est descendant de div (via p2) — pas enfant direct.
    assert!(matches(&f.dom, f.em, &sel("div em")));
    assert!(matches(&f.dom, f.em, &sel("div p em")));
    assert!(matches(&f.dom, f.em, &sel("p em")));
    assert!(!matches(&f.dom, f.em, &sel("div > em")));
    assert!(matches(&f.dom, f.p2, &sel("div > p")));
    // p1 et p2 tous les deux enfants de div.
    assert!(matches(&f.dom, f.p1, &sel("div > p")));
    // p2 n'est pas enfant de p1.
    assert!(!matches(&f.dom, f.p2, &sel("p > p")));
}

#[test]
fn match_freres() {
    let f = fixture();
    // span précède directement p2 (le texte entre est sauté).
    assert!(matches(&f.dom, f.p2, &sel("span + p")));
    // p1 n'est pas un tag ; `p ~ p` : un p précède p2 (p1).
    assert!(matches(&f.dom, f.p2, &sel("p ~ p")));
    assert!(matches(&f.dom, f.span, &sel("p + span")));
    assert!(!matches(&f.dom, f.p1, &sel("span + p")));
}

#[test]
fn match_pseudo_structuraux() {
    let f = fixture();
    assert!(matches(&f.dom, f.p1, &sel("p:first-child")));
    assert!(!matches(&f.dom, f.p2, &sel("p:first-child")));
    assert!(matches(&f.dom, f.p2, &sel("p:last-child")));
    assert!(matches(&f.dom, f.em, &sel("em:only-child")));
    // li 1..3 : odd = 1,3 ; even = 2 ; 3 = dernier.
    assert!(matches(&f.dom, f.lis[0], &sel("li:nth-child(odd)")));
    assert!(!matches(&f.dom, f.lis[1], &sel("li:nth-child(odd)")));
    assert!(matches(&f.dom, f.lis[2], &sel("li:nth-child(3)")));
    assert!(matches(&f.dom, f.lis[1], &sel("li:nth-child(2n)")));
    assert!(matches(&f.dom, f.lis[0], &sel("li:nth-child(-n+2)")));
    assert!(matches(&f.dom, f.lis[1], &sel("li:nth-child(-n+2)")));
    assert!(!matches(&f.dom, f.lis[2], &sel("li:nth-child(-n+2)")));
}

#[test]
fn match_nth_of_type_et_of_type() {
    let f = fixture();
    // p1 = 1er p, p2 = 2e p.
    assert!(matches(&f.dom, f.p1, &sel("p:nth-of-type(1)")));
    assert!(matches(&f.dom, f.p2, &sel("p:nth-of-type(2)")));
    assert!(matches(&f.dom, f.p2, &sel("p:last-of-type")));
    assert!(matches(&f.dom, f.p1, &sel("p:first-of-type")));
    assert!(matches(&f.dom, f.em, &sel("em:only-of-type")));
}

#[test]
fn match_root_empty_not() {
    let f = fixture();
    // div et ul sont enfants directs du document → :root ? Non —
    // :root = élément dont le parent n'est PAS un élément.
    assert!(matches(&f.dom, f.div, &sel(":root")));
    assert!(!matches(&f.dom, f.p1, &sel(":root")));
    // section est vide ; p1 contient du texte.
    assert!(matches(&f.dom, f.section, &sel(":empty")));
    assert!(!matches(&f.dom, f.p1, &sel(":empty")));
    assert!(!matches(&f.dom, f.em, &sel(":empty")));
    // :not / :is.
    assert!(matches(&f.dom, f.p2, &sel("p:not(.b)")));
    assert!(!matches(&f.dom, f.p1, &sel("p:not(.b)")));
    assert!(matches(&f.dom, f.p2, &sel("p:is(.b, .c)")));
    assert!(!matches_any(&f.dom, f.span, &list("p, div")));
}

#[test]
fn match_sur_handles_obsoletes_ne_panique_pas() {
    let mut f = fixture();
    // Détache puis collecte le sous-arbre de p2 : les handles
    // deviennent obsolètes — matching doit renvoyer false.
    f.dom.remove(f.p2).unwrap();
    let _ = f.dom.collect();
    assert!(!matches(&f.dom, f.p2, &sel("p.c")));
    assert!(!matches(&f.dom, f.em, &sel("em")));
}

#[test]
fn match_liste_complexe_reelle() {
    let f = fixture();
    assert!(matches_any(
        &f.dom,
        f.p2,
        &list("div#a > span, p.c[lang|=fr], em.x")
    ));
    assert!(!matches_any(
        &f.dom,
        f.p1,
        &list("div#a > span, p.c[lang|=fr], em.x")
    ));
}

// ── R-100 : pseudos d'état structurels + :valid/:invalid ────

/// DOM formulaire de test : `form` > `input[required]` (vide),
/// `input` type=email value=malformé, `input` type=checkbox
/// checked, `select` required (option vide), `fieldset[disabled]`
/// > `legend` > `input` + `input`.
struct FormFixture {
    dom: Dom,
    input_req: NodeId,
    input_email: NodeId,
    input_chk: NodeId,
    select: NodeId,
    fs_input: NodeId,
    legend_input: NodeId,
    form: NodeId,
}

fn form_fixture() -> FormFixture {
    let mut dom = Dom::new();
    let doc = dom.document();
    let form = dom.create_element("form").unwrap();
    dom.append_child(doc, form).unwrap();

    let input_req = dom.create_element("input").unwrap();
    dom.set_attribute(input_req, "type", "text").unwrap();
    dom.set_attribute(input_req, "required", "").unwrap();
    dom.append_child(form, input_req).unwrap();

    let input_email = dom.create_element("input").unwrap();
    dom.set_attribute(input_email, "type", "email").unwrap();
    dom.set_attribute(input_email, "value", "pas-un-email")
        .unwrap();
    dom.append_child(form, input_email).unwrap();

    let input_chk = dom.create_element("input").unwrap();
    dom.set_attribute(input_chk, "type", "checkbox").unwrap();
    dom.set_attribute(input_chk, "checked", "").unwrap();
    dom.append_child(form, input_chk).unwrap();

    let select = dom.create_element("select").unwrap();
    dom.set_attribute(select, "required", "").unwrap();
    dom.append_child(form, select).unwrap();
    let opt = dom.create_element("option").unwrap();
    dom.set_attribute(opt, "value", "").unwrap();
    dom.append_child(select, opt).unwrap();

    let fs = dom.create_element("fieldset").unwrap();
    dom.set_attribute(fs, "disabled", "").unwrap();
    dom.append_child(form, fs).unwrap();
    let legend = dom.create_element("legend").unwrap();
    dom.append_child(fs, legend).unwrap();
    let legend_input = dom.create_element("input").unwrap();
    dom.append_child(legend, legend_input).unwrap();
    let fs_input = dom.create_element("input").unwrap();
    dom.append_child(fs, fs_input).unwrap();

    FormFixture {
        dom,
        input_req,
        input_email,
        input_chk,
        select,
        fs_input,
        legend_input,
        form,
    }
}

#[test]
fn etat_disabled_enabled_fieldset() {
    let f = form_fixture();
    // Contrôle direct dans un fieldset[disabled] → désactivé.
    assert!(matches(&f.dom, f.fs_input, &sel(":disabled")));
    assert!(!matches(&f.dom, f.fs_input, &sel(":enabled")));
    // Exception première <legend> : reste activable.
    assert!(matches(&f.dom, f.legend_input, &sel(":enabled")));
    assert!(!matches(&f.dom, f.legend_input, &sel(":disabled")));
    // Contrôle libre → enabled.
    assert!(matches(&f.dom, f.input_req, &sel(":enabled")));
}

#[test]
fn etat_checked() {
    let f = form_fixture();
    assert!(matches(&f.dom, f.input_chk, &sel(":checked")));
    assert!(!matches(&f.dom, f.input_req, &sel(":checked")));
}

#[test]
fn etat_required_optional() {
    let f = form_fixture();
    assert!(matches(&f.dom, f.input_req, &sel(":required")));
    assert!(matches(&f.dom, f.select, &sel(":required")));
    assert!(!matches(&f.dom, f.input_req, &sel(":optional")));
    assert!(matches(&f.dom, f.input_chk, &sel(":optional")));
}

#[test]
fn etat_valid_invalid() {
    let f = form_fixture();
    // required vide → :invalid ; checkbox cochée non requise → :valid.
    assert!(matches(&f.dom, f.input_req, &sel(":invalid")));
    assert!(!matches(&f.dom, f.input_req, &sel(":valid")));
    assert!(matches(&f.dom, f.input_chk, &sel(":valid")));
    // email malformé → :invalid.
    assert!(matches(&f.dom, f.input_email, &sel(":invalid")));
    // select required à option vide → :invalid.
    assert!(matches(&f.dom, f.select, &sel(":invalid")));
    // form agrège : contient des invalides → :invalid.
    assert!(matches(&f.dom, f.form, &sel(":invalid")));
    assert!(!matches(&f.dom, f.form, &sel(":valid")));
}

#[test]
fn etat_valid_apres_remplissage() {
    let mut f = form_fixture();
    // L'état se lit dans le DOM — poser la valeur suffit.
    f.dom.set_attribute(f.input_req, "value", "ok").unwrap();
    f.dom
        .set_attribute(f.input_email, "value", "a@b.co")
        .unwrap();
    assert!(matches(&f.dom, f.input_req, &sel(":valid")));
    assert!(matches(&f.dom, f.input_email, &sel(":valid")));
    // Le select reste invalide → le form aussi.
    assert!(matches(&f.dom, f.select, &sel(":invalid")));
    assert!(matches(&f.dom, f.form, &sel(":invalid")));
}

#[test]
fn etat_readonly_placeholder() {
    let mut dom = Dom::new();
    let doc = dom.document();
    let inp = dom.create_element("input").unwrap();
    dom.set_attribute(inp, "placeholder", "Nom…").unwrap();
    dom.append_child(doc, inp).unwrap();
    let ro = dom.create_element("input").unwrap();
    dom.set_attribute(ro, "readonly", "").unwrap();
    dom.append_child(doc, ro).unwrap();
    let div = dom.create_element("div").unwrap();
    dom.set_attribute(div, "contenteditable", "").unwrap();
    dom.append_child(doc, div).unwrap();

    assert!(matches(&dom, inp, &sel(":placeholder-shown")));
    assert!(matches(&dom, inp, &sel(":read-write")));
    assert!(matches(&dom, ro, &sel(":read-only")));
    assert!(matches(&dom, div, &sel(":read-write")));
}

#[test]
fn etat_user_invalid_gatee_par_state() {
    let f = form_fixture();
    // Invalide réel mais jamais touché → :user-invalid faux
    // sans flag d'interaction (conservateur spec).
    assert!(!matches(&f.dom, f.input_req, &sel(":user-invalid")));
    assert!(explorer_css::matches_with_state(
        &f.dom,
        f.input_req,
        &sel(":user-invalid"),
        &|n| n == "user-invalid"
    ));
}
