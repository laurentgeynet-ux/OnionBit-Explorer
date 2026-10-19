// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tests du tokenizer — états, entités, doctype, rawtext/RCDATA,
//! et surtout les **coupures de chunk** (ADR-0012).

use explorer_html::{ContentKind, Token, Tokenizer};

fn toks(input: &str) -> Vec<Token> {
    Tokenizer::on(input).run()
}

fn chars(s: &str) -> Token {
    Token::Character(s.to_owned())
}

#[test]
fn texte_simple() {
    assert_eq!(
        toks("bonjour monde"),
        vec![chars("bonjour monde"), Token::Eof]
    );
}

#[test]
fn balise_simple_attributs() {
    assert_eq!(
        toks(r#"<p class="a" id='x' hidden>hi</p>"#),
        vec![
            Token::StartTag {
                name: "p".into(),
                attributes: vec![
                    explorer_html::TokenAttribute {
                        name: "class".into(),
                        value: "a".into(),
                    },
                    explorer_html::TokenAttribute {
                        name: "id".into(),
                        value: "x".into(),
                    },
                    explorer_html::TokenAttribute {
                        name: "hidden".into(),
                        value: String::new(),
                    },
                ],
                self_closing: false,
            },
            chars("hi"),
            Token::EndTag { name: "p".into() },
            Token::Eof,
        ]
    );
}

#[test]
fn casse_balise_normalisee() {
    match &toks("<DIV>X</DIV>")[0] {
        Token::StartTag { name, .. } => assert_eq!(name, "div"),
        t => panic!("attendu StartTag, eu {t:?}"),
    }
}

#[test]
fn self_closing() {
    match &toks("<br/>")[0] {
        Token::StartTag {
            name, self_closing, ..
        } => {
            assert_eq!(name, "br");
            assert!(self_closing);
        }
        t => panic!("attendu StartTag, eu {t:?}"),
    }
}

#[test]
fn attribut_doublon_premier_gagne() {
    match &toks(r#"<a href="1" href="2">"#)[0] {
        Token::StartTag { attributes, .. } => {
            assert_eq!(attributes.len(), 1);
            assert_eq!(attributes[0].value, "1");
        }
        t => panic!("attendu StartTag, eu {t:?}"),
    }
}

#[test]
fn attribut_sans_quotes_ni_valeur() {
    match &toks("<input disabled type=text>")[0] {
        Token::StartTag { attributes, .. } => {
            assert_eq!(attributes[0].name, "disabled");
            assert_eq!(attributes[0].value, "");
            assert_eq!(attributes[1].name, "type");
            assert_eq!(attributes[1].value, "text");
        }
        t => panic!("attendu StartTag, eu {t:?}"),
    }
}

#[test]
fn lt_litteral_pas_balise() {
    assert_eq!(toks("a < b et <3"), vec![chars("a < b et <3"), Token::Eof]);
}

#[test]
fn commentaires() {
    assert_eq!(
        toks("<!-- hi -->x<!--->y<!---->"),
        vec![
            Token::Comment(" hi ".into()),
            chars("x"),
            Token::Comment(String::new()),
            chars("y"),
            Token::Comment(String::new()),
            Token::Eof,
        ]
    );
}

#[test]
fn commentaire_bogus_pi() {
    assert_eq!(
        toks("<?php echo 1; ?>"),
        vec![Token::Comment("?php echo 1; ?".into()), Token::Eof]
    );
}

#[test]
fn doctype_complet() {
    assert_eq!(
        toks(r"<!DOCTYPE html>>"),
        vec![
            Token::Doctype {
                name: Some("html".into()),
                public_id: None,
                system_id: None,
                force_quirks: false,
            },
            chars(">"),
            Token::Eof,
        ]
    );
    assert_eq!(
        toks(r#"<!DOCTYPE html PUBLIC "-//W3C//DTD HTML 4.01//EN" "http://x">"#),
        vec![
            Token::Doctype {
                name: Some("html".into()),
                public_id: Some("-//W3C//DTD HTML 4.01//EN".into()),
                system_id: Some("http://x".into()),
                force_quirks: false,
            },
            Token::Eof,
        ]
    );
}

#[test]
fn doctype_quirks() {
    match &toks("<!DOCTYPE>")[0] {
        Token::Doctype { force_quirks, .. } => assert!(force_quirks),
        t => panic!("attendu Doctype, eu {t:?}"),
    }
}

#[test]
fn entites_texte() {
    assert_eq!(
        toks("a &amp; b &lt;&gt; &#65; &#x42; &eacute;&hellip;"),
        vec![chars("a & b <> A B é…"), Token::Eof]
    );
}

#[test]
fn entite_legacy_sans_pointvirgule() {
    // `&amp` sans `;` dans le contenu : toléré (legacy).
    assert_eq!(toks("x &amp y"), vec![chars("x & y"), Token::Eof]);
    // Dans un attribut suivi de `=` ou alphanum : bloqué.
    match &toks(r#"<a href="?a=1&amp=x">"#)[0] {
        Token::StartTag { attributes, .. } => {
            assert_eq!(attributes[0].value, "?a=1&amp=x");
        }
        t => panic!("attendu StartTag, eu {t:?}"),
    }
    match &toks(r#"<a href="?a=1&amp">"#)[0] {
        Token::StartTag { attributes, .. } => {
            assert_eq!(attributes[0].value, "?a=1&");
        }
        t => panic!("attendu StartTag, eu {t:?}"),
    }
}

#[test]
fn entites_plus_long_prefixe() {
    // `not` est préfixe de `notin` : le plus long gagne avec `;`,
    // le legacy s'applique sans `;` (spec : `&notin` = `¬in`).
    assert_eq!(toks("&notin;&not;x"), vec![chars("∉¬x"), Token::Eof]);
    // Entité inconnue : `&` littéral.
    assert_eq!(toks("&bidule; &"), vec![chars("&bidule; &"), Token::Eof]);
}

#[test]
fn numerique_windows1252_remap() {
    // &#150; = tiret windows-1252, pas U+0096.
    assert_eq!(toks("&#150;"), vec![chars("–"), Token::Eof]);
    assert_eq!(toks("&#0;"), vec![chars("\u{FFFD}"), Token::Eof]);
}

#[test]
fn rcdata_title() {
    // Simule le tree-builder : <title> bascule en RCDATA.
    let mut t = Tokenizer::new();
    t.feed("<title>");
    let _ = t.run();
    t.set_content_state(ContentKind::Rcdata);
    t.feed("a &amp; b </title> apres");
    t.finish();
    let out = t.run();
    assert_eq!(
        out,
        vec![
            chars("a & b "),
            Token::EndTag {
                name: "title".into()
            },
            chars(" apres"),
            Token::Eof,
        ]
    );
}

#[test]
fn rawtext_pas_d_entites() {
    let mut t = Tokenizer::new();
    t.feed("<style>");
    let _ = t.run();
    t.set_content_state(ContentKind::RawText);
    t.feed("a &amp; </style>");
    t.finish();
    assert_eq!(
        t.run(),
        vec![
            chars("a &amp; "),
            Token::EndTag {
                name: "style".into()
            },
            Token::Eof,
        ]
    );
}

#[test]
fn rawtext_mauvaise_fermeture_est_texte() {
    let mut t = Tokenizer::new();
    t.feed("<style>");
    let _ = t.run();
    t.set_content_state(ContentKind::RawText);
    t.feed("x </div> y</style>");
    t.finish();
    let out = t.run();
    assert_eq!(
        out,
        vec![
            chars("x </div> y"),
            Token::EndTag {
                name: "style".into()
            },
            Token::Eof,
        ]
    );
}

#[test]
fn script_data() {
    let mut t = Tokenizer::new();
    t.feed("<script>");
    let _ = t.run();
    t.set_content_state(ContentKind::ScriptData);
    t.feed("if (a < b) {}</script>");
    t.finish();
    let out = t.run();
    assert_eq!(
        out,
        vec![
            chars("if (a < b) {}"),
            Token::EndTag {
                name: "script".into()
            },
            Token::Eof,
        ]
    );
}

#[test]
fn coupure_chunk_milieu_balise() {
    // `</` coupé, attribut coupé en plein milieu, entité coupée.
    for cut in 1..20 {
        let input = r#"<div class="a&amp;b">x</div>"#;
        let mut t = Tokenizer::new();
        t.feed(&input[..cut]);
        let mut out = t.run();
        t.feed(&input[cut..]);
        t.finish();
        out.extend(t.run());
        let attendu = toks(input);
        assert_eq!(out, attendu, "coupure à {cut}");
    }
}

#[test]
fn coupure_chunk_milieu_entite() {
    // Chaque coupure doit produire le même texte qu'un flux continu.
    let cas = [
        ("&am", "p; y", "& y"),
        ("&amp", "p; y", "&p; y"),
        ("&#x4", "2;", "B"),
    ];
    for (a, b, attendu) in cas {
        let mut t = Tokenizer::new();
        t.feed(a);
        let _ = t.run();
        t.feed(b);
        t.finish();
        let out = t.run();
        let texte: String = out
            .iter()
            .filter_map(|tk| match tk {
                Token::Character(s) => Some(s.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(texte, attendu, "coupure {a}|{b}");
    }
}

#[test]
fn coupure_doctype_motcle() {
    let input = r#"<!DOCTYPE html PUBLIC "-//W3C//DTD HTML 4.01//EN" "http://x">"#;
    for cut in 8..input.len().min(20) {
        let mut t = Tokenizer::new();
        t.feed(&input[..cut]);
        let mut out = t.run();
        t.feed(&input[cut..]);
        t.finish();
        out.extend(t.run());
        assert_eq!(out, toks(input), "coupure à {cut}");
    }
}

#[test]
fn cdata_refuse_par_defaut() {
    // Sans allow_cdata, <![CDATA[ est un bogus comment (casse
    // d'origine conservée).
    assert_eq!(
        toks("<![CDATA[x]]>"),
        vec![Token::Comment("[CDATA[x]]".into()), Token::Eof]
    );
}

#[test]
fn cdata_autorise() {
    let mut t = Tokenizer::new();
    t.feed("<svg>");
    let _ = t.run();
    t.set_cdata_allowed(true);
    t.feed("<![CDATA[a < b]]> fin");
    t.finish();
    let out = t.run();
    assert_eq!(out, vec![chars("a < b"), chars(" fin"), Token::Eof]);
}

#[test]
fn plaintext_tout_texte() {
    let mut t = Tokenizer::new();
    t.set_content_state(ContentKind::PlainText);
    t.feed("a<b>c</b>&amp;");
    t.finish();
    assert_eq!(t.run(), vec![chars("a<b>c</b>&amp;"), Token::Eof]);
}
