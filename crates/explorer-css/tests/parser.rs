// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

#![allow(clippy::match_wildcard_for_single_variants)]
//! Tests du parser CSS — règles, déclarations, at-rules,
//! récupération d'erreurs de la spec.

use explorer_css::{parse_stylesheet, AtKind, Declaration, Rule};

fn decls(rule: &Rule) -> &[Declaration] {
    match rule {
        Rule::Qualified(q) => &q.declarations,
        _ => &[],
    }
}

#[test]
fn regle_simple() {
    let s = parse_stylesheet("p { color: red; margin: 0 }");
    assert_eq!(s.rules.len(), 1);
    match &s.rules[0] {
        Rule::Qualified(q) => {
            assert_eq!(q.prelude, "p");
            assert_eq!(
                q.declarations,
                vec![
                    Declaration {
                        name: "color".into(),
                        value: "red".into(),
                        important: false,
                    },
                    Declaration {
                        name: "margin".into(),
                        value: "0".into(),
                        important: false,
                    },
                ]
            );
        }
        _ => panic!("attendu Qualified"),
    }
}

#[test]
fn plusieurs_regles() {
    let s = parse_stylesheet("a{x:1} b, c{y:2; z:3}");
    assert_eq!(s.rules.len(), 2);
    match &s.rules[1] {
        Rule::Qualified(q) => assert_eq!(q.prelude, "b, c"),
        _ => panic!(),
    }
}

#[test]
fn important_detecte() {
    let s = parse_stylesheet("p{a:1 !important; b:2 ! IMPORTANT; c:3}");
    let d = decls(&s.rules[0]);
    assert!(d[0].important && d[0].value == "1");
    assert!(d[1].important && d[1].value == "2");
    assert!(!d[2].important);
}

#[test]
fn valeur_complexe_preservee() {
    let s = parse_stylesheet(
        "a{background:url(x.png) 0 0/cover no-repeat, linear-gradient(to top,#fff,#000)}",
    );
    let d = decls(&s.rules[0]);
    assert_eq!(
        d[0].value,
        "url(x.png) 0 0/cover no-repeat, linear-gradient(to top,#fff,#000)"
    );
}

#[test]
fn declarations_invalides_sautees() {
    // `b` sans valeur, `=1` sans nom : récupérés, `a`/`c` survivent.
    let s = parse_stylesheet("p{a:1; b; =1; c:3; broken{:x; y:2}}");
    let d = decls(&s.rules[0]);
    let names: Vec<_> = d.iter().map(|x| x.name.as_str()).collect();
    assert!(names.contains(&"a"), "{names:?}");
    assert!(names.contains(&"c"), "{names:?}");
}

#[test]
fn bloc_non_ferme_eof() {
    // EOF ferme implicitement (tolérance spec).
    let s = parse_stylesheet("p{color:red");
    assert_eq!(decls(&s.rules[0])[0].value, "red");
}

#[test]
fn regle_sans_corps_abandonnee() {
    let s = parse_stylesheet("p div; a{x:1}");
    // `p div;` sans `{` → ignorée ; seule `a` survit.
    assert_eq!(s.rules.len(), 1);
}

#[test]
fn media_simple() {
    let s = parse_stylesheet("@media screen and (min-width: 600px){p{a:1}}");
    match &s.rules[0] {
        Rule::At(at) => {
            assert_eq!(at.name, "media");
            match &at.kind {
                AtKind::Media { queries, rules } => {
                    assert_eq!(queries.len(), 1);
                    assert_eq!(queries[0].media_type.as_deref(), Some("screen"));
                    assert_eq!(
                        queries[0].conditions,
                        vec![("min-width".to_string(), "600px".to_string())]
                    );
                    assert_eq!(rules.len(), 1);
                }
                _ => panic!("attendu Media"),
            }
        }
        _ => panic!(),
    }
}

#[test]
fn media_liste_et_negation() {
    let s = parse_stylesheet("@media not print, screen{p{a:1}}");
    match &s.rules[0] {
        Rule::At(AtRule2 { .. }) => {}
        _ => panic!(),
    }
    if let Rule::At(at) = &s.rules[0] {
        if let AtKind::Media { queries, .. } = &at.kind {
            assert_eq!(queries.len(), 2);
            assert!(queries[0].negated);
            assert_eq!(queries[0].media_type.as_deref(), Some("print"));
            assert_eq!(queries[1].media_type.as_deref(), Some("screen"));
            return;
        }
    }
    panic!();
}
use explorer_css::AtRule as AtRule2;

#[test]
fn media_bare_condition() {
    let s = parse_stylesheet("@media (orientation: landscape){a{x:1}}");
    if let Rule::At(at) = &s.rules[0] {
        if let AtKind::Media { queries, .. } = &at.kind {
            assert_eq!(queries[0].media_type, None);
            assert_eq!(
                queries[0].conditions,
                vec![("orientation".to_string(), "landscape".to_string())]
            );
            return;
        }
    }
    panic!();
}

#[test]
fn font_face_declarations() {
    let s = parse_stylesheet("@font-face{font-family:\"X\"; src:url(f.woff2)}");
    match &s.rules[0] {
        Rule::At(at) => match &at.kind {
            AtKind::Declarations { declarations, .. } => {
                assert_eq!(declarations.len(), 2);
                assert_eq!(declarations[0].name, "font-family");
            }
            _ => panic!(),
        },
        _ => panic!(),
    }
}

#[test]
fn import_collecte() {
    let s =
        parse_stylesheet("@import url(a.css); @import \"b.css\" screen; @import url(c.css) print;");
    assert_eq!(s.imports.len(), 3);
    assert_eq!(s.imports[0].url, "a.css");
    assert_eq!(s.imports[1].url, "b.css");
    assert_eq!(s.imports[1].media, "screen");
    assert_eq!(s.imports[2].media, "print");
}

#[test]
fn charset_detecte() {
    let s = parse_stylesheet("@charset \"utf-8\"; a{x:1}");
    assert_eq!(s.charset.as_deref(), Some("utf-8"));
}

#[test]
fn keyframes_parsees() {
    let s = parse_stylesheet("@keyframes fade{from{opacity:0}50%{opacity:.5}to{opacity:1}}");
    match &s.rules[0] {
        Rule::At(at) => match &at.kind {
            AtKind::Keyframes { name, frames } => {
                assert_eq!(name, "fade");
                assert_eq!(frames.len(), 3);
                assert_eq!(frames[0].0, "from");
                assert_eq!(frames[1].0, "50%");
                assert_eq!(frames[2].0, "to");
                assert_eq!(frames[2].1[0].value, "1");
            }
            _ => panic!(),
        },
        _ => panic!(),
    }
}

#[test]
fn at_rule_inconnue_avalee() {
    let s = parse_stylesheet("@weird x { a{b:1} } p{c:2}");
    assert_eq!(s.rules.len(), 2);
    match &s.rules[0] {
        Rule::At(at) => assert!(matches!(at.kind, AtKind::UnknownBlock { .. })),
        _ => panic!(),
    }
    match &s.rules[1] {
        Rule::Qualified(q) => assert_eq!(q.prelude, "p"),
        _ => panic!(),
    }
}

#[test]
fn cdo_cdc_toleres() {
    // Vieilles feuilles : <!-- … --> autour des règles.
    let s = parse_stylesheet("<!-- p{a:1} -->");
    assert_eq!(s.rules.len(), 1);
}

#[test]
fn cascade_ordre_preserve() {
    // La cascade (étape 15) dépend de l'ordre source.
    let s = parse_stylesheet("a{x:1} a{x:2}");
    assert_eq!(s.rules.len(), 2);
    assert_eq!(decls(&s.rules[0])[0].value, "1");
    assert_eq!(decls(&s.rules[1])[0].value, "2");
}

#[test]
fn entrees_hostiles_sans_panic() {
    for css in [
        "",
        "{",
        "}",
        "a{",
        "@",
        "@media{",
        "p{;",
        "url(;;;){",
        "a{!import ant:x}",
        "((((((((((((((((",
        "a{b:c}}}}}}}}",
        "@import",
        "@charset",
        "x{;:;:;:;}",
    ] {
        let _ = parse_stylesheet(css); // jamais de panic
    }
}

#[test]
fn deep_media_nesting_is_capped() {
    // 100 `@media` imbriqués dépassent MAX_NESTING (32) : les blocs
    // excédentaires sont avalés comme inconnus — le parse termine
    // sans débordement de pile.
    let css = format!("{}{}", "@media a{".repeat(100), "}".repeat(100));
    let sheet = parse_stylesheet(&css);
    assert_eq!(sheet.rules.len(), 1);
    assert!(media_depth(&sheet.rules[0]) <= 33, "chaîne trop profonde");
}

/// Profondeur de la chaîne `@media` d'une règle (1 = feuille).
fn media_depth(r: &explorer_css::Rule) -> usize {
    match r {
        explorer_css::Rule::At(at) => match &at.kind {
            explorer_css::AtKind::Media { rules, .. } => {
                rules.first().map_or(1, |r| 1 + media_depth(r))
            }
            _ => 1,
        },
        _ => 1,
    }
}

#[test]
fn bang_non_important_invalide_la_declaration() {
    // `!` non suivi de `important` : déclaration droppée,
    // le reste du bloc survit (spec).
    let s = parse_stylesheet("p{border:5em solid red ! error; color:teal}");
    let d = decls(&s.rules[0]);
    assert!(!d.iter().any(|x| x.name == "border"), "{d:?}");
    assert!(d.iter().any(|x| x.name == "color"), "{d:?}");
}

#[test]
fn accolade_echappee_ne_ferme_pas_le_bloc() {
    // `\}` est une valeur (ident `}`), pas un fermant —
    // `background: yellow` survit derrière (cas Acid2).
    let s = parse_stylesheet(r"p{error:\}; background:yellow}");
    let d = decls(&s.rules[0]);
    assert!(d.iter().any(|x| x.name == "error"), "{d:?}");
    assert_eq!(
        d.iter()
            .find(|x| x.name == "background")
            .map(|x| x.value.as_str()),
        Some("yellow")
    );
}

#[test]
fn parenthese_non_fermee_eof_termine() {
    // §B-50 : `style="transform: translateX(0px;…"` — parenthèse
    // jamais refermée (attribut muté par JS). EOF doit clore la
    // déclaration ; avant le fix, la boucle de valeur attendait
    // la `)` et ne consommait plus rien → boucle infinie dans
    // la cascade après re-parse du DOM miroir (bordeaux).
    let d = explorer_css::parse_declarations(
        "transform: translateX(0px; padding-right: 0%; transition: transform .5s",
    );
    assert!(
        d.iter().any(|x| x.name == "transform"),
        "la déclaration à `(` non fermée doit être récupérée : {d:?}"
    );
    // Idem dans une feuille : `(` jamais fermé — tout le reste
    // entre dans la valeur (spec : `}` n'interrompt pas un bloc
    // `(` ouvert), le parse se termine à EOF.
    let s = parse_stylesheet("a{transform:translateX(0px; b{color:red}");
    assert_eq!(s.rules.len(), 1, "{:?}", s.rules);
    assert!(decls(&s.rules[0]).iter().any(|x| x.name == "transform"));
}

// ---------- R-189 — CSS nesting : désucrage au parse ----------

fn prelude_of(rule: &Rule) -> &str {
    match rule {
        Rule::Qualified(q) => &q.prelude,
        _ => "",
    }
}

/// `&` substitué en `:is(<parent>)` — descendant et composé.
#[test]
fn nesting_esperluette_descendant_et_hover() {
    let s = parse_stylesheet("div{& .x{a:1}&:hover{b:2}}");
    assert_eq!(s.rules.len(), 2, "{:?}", s.rules);
    assert_eq!(prelude_of(&s.rules[0]), ":is(div) .x");
    assert_eq!(prelude_of(&s.rules[1]), ":is(div):hover");
    assert!(decls(&s.rules[0]).iter().any(|d| d.name == "a"));
    assert!(decls(&s.rules[1]).iter().any(|d| d.name == "b"));
}

/// Sélecteur nu → `& ` implicite (descendant) ; combinateur
/// initial `>` préservé.
#[test]
fn nesting_nu_et_combinateur() {
    let s = parse_stylesheet(".a{.b{c:1}> .d{e:2}}");
    assert_eq!(prelude_of(&s.rules[0]), ".a .b");
    assert_eq!(prelude_of(&s.rules[1]), ".a > .d");
}

/// `a:hover{}` à l'intérieur d'un bloc = règle imbriquée,
/// pas une déclaration (l'ambiguïté `:` est tranchée par
/// le `{` à profondeur-0 avant `;`/`}`).
#[test]
fn nesting_hover_ident_n_est_pas_declaration() {
    let s = parse_stylesheet("a{a:hover{color:red}}");
    assert_eq!(s.rules.len(), 1);
    assert_eq!(prelude_of(&s.rules[0]), "a a:hover");
}

/// Liste parente × partie imbriquée → produit cartésien.
#[test]
fn nesting_produit_cartesien() {
    let s = parse_stylesheet("a, b{& .x{c:1}}");
    assert_eq!(prelude_of(&s.rules[0]), ":is(a) .x, :is(b) .x");
}

/// Ordre source : une déclaration APRÈS une règle imbriquée
/// est soulevée en règle plate ordonnée (spec : la séquence
/// compte — la dernière l'emporte à spécificité égale).
#[test]
fn nesting_ordre_source_runs() {
    let s = parse_stylesheet("div{color:1;&:hover{x:2}color:3}");
    assert_eq!(s.rules.len(), 3, "{:?}", s.rules);
    assert_eq!(prelude_of(&s.rules[0]), "div");
    assert_eq!(prelude_of(&s.rules[1]), ":is(div):hover");
    assert_eq!(prelude_of(&s.rules[2]), "div");
    assert_eq!(decls(&s.rules[2])[0].value, "3");
}

/// `&` profond : dans `& .b { & .c }`, le `&` interne
/// résout contre le sélecteur SUBSTITUÉ (`:is(div) .b`),
/// pas contre `div`.
#[test]
fn nesting_esperluette_profonde() {
    let s = parse_stylesheet("div{& .b{& .c{a:1}}}");
    assert_eq!(prelude_of(&s.rules[0]), ":is(:is(div) .b) .c");
}

/// `@media` imbriqué dans un bloc : ses items sont des
/// block-items liés au parent — règle plate DANS l'at-rule.
#[test]
fn nesting_media_imbrique() {
    let s = parse_stylesheet("div{@media screen{color:red}}");
    assert_eq!(s.rules.len(), 1);
    match &s.rules[0] {
        Rule::At(at) => {
            assert_eq!(at.name, "media");
            if let AtKind::Media { rules, .. } = &at.kind {
                assert_eq!(rules.len(), 1);
                assert_eq!(prelude_of(&rules[0]), "div");
                assert!(decls(&rules[0]).iter().any(|d| d.name == "color"));
            } else {
                panic!("attendu AtKind::Media");
            }
        }
        _ => panic!("attendu At"),
    }
}

/// Hostile : `&` top-level sans parent n'est pas substitué
/// (prélude littéral — le sélecteur amont tranche) ; bloc
/// imbriqué déséquilibré → récupération, feuille terminée.
#[test]
fn nesting_hostile_tolere() {
    let s = parse_stylesheet("&{color:red}p{x:1}");
    assert_eq!(s.rules.len(), 2);
    assert_eq!(prelude_of(&s.rules[0]), "&");
    // Feuille sans nesting : output identique à avant.
    let s2 = parse_stylesheet("p{a:1}b{c:2}");
    assert_eq!(s2.rules.len(), 2);
    assert_eq!(prelude_of(&s2.rules[0]), "p");
}

// ---------- R-194 — @scope : désucrage au parse ----------

/// `@scope (R) { S }` → prélude `R S` ; la cascade itère le
/// bloc comme un groupe ordinaire.
#[test]
fn scope_desucre_descendant() {
    let s = parse_stylesheet("@scope (.card){.x{a:1}}");
    match &s.rules[0] {
        Rule::At(at) => {
            assert_eq!(at.name, "scope");
            if let AtKind::Scope { rules } = &at.kind {
                assert_eq!(rules.len(), 1);
                assert_eq!(prelude_of(&rules[0]), ".card .x");
            } else {
                panic!("attendu AtKind::Scope");
            }
        }
        _ => panic!("attendu At"),
    }
}

/// Borne `to (B)` → `:not(B *)` sur l'élément cible.
#[test]
fn scope_borne_donut() {
    let s = parse_stylesheet("@scope (.card) to (.inner){.x{a:1}}");
    if let Rule::At(at) = &s.rules[0] {
        if let AtKind::Scope { rules } = &at.kind {
            assert_eq!(prelude_of(&rules[0]), ".card .x:not(.inner *)");
        } else {
            panic!();
        }
    }
}

/// `&` et `:scope` sont déjà ancrés à la racine — substitués
/// `:is(R)` sans préfixe descendant.
#[test]
fn scope_ancres_esperluette() {
    let s = parse_stylesheet("@scope (.card){& .x{a:1}}");
    if let Rule::At(at) = &s.rules[0] {
        if let AtKind::Scope { rules } = &at.kind {
            assert_eq!(prelude_of(&rules[0]), ":is(.card) .x");
        } else {
            panic!();
        }
    }
}

/// Hostile : scope vide, borne absente, prélude non parenthésé
/// → toléré, jamais panic.
#[test]
fn scope_hostile_tolere() {
    let s = parse_stylesheet("@scope{}@scope (.a){}p{x:1}");
    assert!(s
        .rules
        .iter()
        .all(|r| matches!(r, Rule::At(_)) || prelude_of(r) == "p"));
}
