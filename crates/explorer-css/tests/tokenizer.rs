// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tests du tokenizer CSS — nombres, dimensions, chaînes, url,
//! commentaires, échappements, tolérance bad-string/bad-url.

use explorer_css::{tokenize, Token};

fn toks(input: &str) -> Vec<Token> {
    tokenize(input)
        .into_iter()
        .filter(|t| !matches!(t, Token::Whitespace))
        .collect()
}

#[test]
fn ident_et_ponctuation() {
    let t = toks("div.foo { color: red }");
    assert_eq!(
        t,
        vec![
            Token::Ident("div".into()),
            Token::Delim('.'),
            Token::Ident("foo".into()),
            Token::OpenBrace,
            Token::Ident("color".into()),
            Token::Colon,
            Token::Ident("red".into()),
            Token::CloseBrace,
        ]
    );
}

#[test]
fn nombres_et_dimensions() {
    let t = toks("12 +4.5 -3e2 .5 10px 2.5em 50%");
    assert_eq!(
        t,
        vec![
            Token::Number {
                repr: "12".into(),
                value: 12.0
            },
            Token::Number {
                repr: "+4.5".into(),
                value: 4.5
            },
            Token::Number {
                repr: "-3e2".into(),
                value: -300.0
            },
            Token::Number {
                repr: ".5".into(),
                value: 0.5
            },
            Token::Dimension {
                repr: "10".into(),
                value: 10.0,
                unit: "px".into()
            },
            Token::Dimension {
                repr: "2.5".into(),
                value: 2.5,
                unit: "em".into()
            },
            Token::Percentage {
                repr: "50".into(),
                value: 50.0
            },
        ]
    );
}

#[test]
fn dimension_negative() {
    // `-5px` : un seul token Dimension.
    let t = toks("-5px");
    assert_eq!(
        t,
        vec![Token::Dimension {
            repr: "-5".into(),
            value: -5.0,
            unit: "px".into()
        }]
    );
}

#[test]
fn ident_commencant_par_tiret() {
    let t = toks("-webkit-flex --var -moz-x");
    assert_eq!(
        t,
        vec![
            Token::Ident("-webkit-flex".into()),
            Token::Ident("--var".into()),
            Token::Ident("-moz-x".into()),
        ]
    );
}

#[test]
fn strings_et_echappements() {
    let t = toks(r#""hello" 'wor\ld' "a\62 c""#);
    assert_eq!(
        t,
        vec![
            Token::Str("hello".into()),
            Token::Str("world".into()),
            Token::Str("abc".into()),
        ]
    );
}

#[test]
fn bad_string_sur_newline() {
    let t = toks("\"abc\ndef");
    assert_eq!(t[0], Token::BadString);
    assert_eq!(t[1], Token::Ident("def".into()));
}

#[test]
fn commentaires_sautes() {
    let t = toks("a /* c1 */ b /* multi\nligne */ c /* non fini");
    assert_eq!(
        t,
        vec![
            Token::Ident("a".into()),
            Token::Ident("b".into()),
            Token::Ident("c".into()),
        ]
    );
}

#[test]
fn url_non_quote() {
    let t = toks("url(img.png) url( a b ) url(x)");
    // `url( a b )` : espace interne → bad-url (spec).
    assert_eq!(t[0], Token::Url("img.png".into()));
    assert_eq!(t[1], Token::BadUrl);
    assert_eq!(t[2], Token::Url("x".into()));
}

#[test]
fn url_quote_est_function() {
    let t = toks("url(\"a.png\")");
    assert_eq!(t[0], Token::Function("url".into()));
    assert_eq!(t[1], Token::Str("a.png".into()));
    assert_eq!(t[2], Token::CloseParen);
}

#[test]
fn functions_et_hash() {
    let t = toks("rgb(1,2,3) #id #abc calc(");
    assert_eq!(
        t,
        vec![
            Token::Function("rgb".into()),
            Token::Number {
                repr: "1".into(),
                value: 1.0
            },
            Token::Comma,
            Token::Number {
                repr: "2".into(),
                value: 2.0
            },
            Token::Comma,
            Token::Number {
                repr: "3".into(),
                value: 3.0
            },
            Token::CloseParen,
            Token::Hash {
                value: "id".into(),
                is_id: true
            },
            Token::Hash {
                value: "abc".into(),
                is_id: true
            },
            Token::Function("calc".into()),
        ]
    );
}

#[test]
fn at_keywords() {
    let t = toks("@media @import @charset @-moz-x @");
    assert_eq!(
        t,
        vec![
            Token::AtKeyword("media".into()),
            Token::AtKeyword("import".into()),
            Token::AtKeyword("charset".into()),
            Token::AtKeyword("-moz-x".into()),
            Token::Delim('@'),
        ]
    );
}

#[test]
fn cdo_cdc() {
    let t = toks("<!-- a --> --> <!--");
    assert_eq!(
        t,
        vec![
            Token::Cdo,
            Token::Ident("a".into()),
            Token::Cdc,
            Token::Cdc,
            Token::Cdo,
        ]
    );
}

#[test]
fn delim_divers() {
    let t = toks("a > b + c ~ d *=^=$=|=");
    assert_eq!(
        t,
        vec![
            Token::Ident("a".into()),
            Token::Delim('>'),
            Token::Ident("b".into()),
            Token::Delim('+'),
            Token::Ident("c".into()),
            Token::Delim('~'),
            Token::Ident("d".into()),
            Token::Delim('*'),
            Token::Delim('='),
            Token::Delim('^'),
            Token::Delim('='),
            Token::Delim('$'),
            Token::Delim('='),
            Token::Delim('|'),
            Token::Delim('='),
        ]
    );
}

#[test]
fn entree_vide_et_bizarres() {
    for s in ["", "   ", "}", "]", ")", "\\\n", "url(", "\"", "#"] {
        let _ = tokenize(s); // pas de panic
    }
}

#[test]
fn backslash_echappement_en_tete_de_token() {
    // `\` en tête de token : `\}` est un ident `}` (le `}`
    // échappé ne ferme pas un bloc) ; `\.a` est un ident `.a`.
    let t = toks("\\} \\.a");
    assert_eq!(t, vec![Token::Ident("}".into()), Token::Ident(".a".into())]);
}

#[test]
fn backslash_avant_newline_est_delim() {
    // `\` suivi d'un newline n'est PAS un échappement valide.
    let t = tokenize("\\\nx");
    assert!(t.contains(&Token::Delim('\\')));
}
