// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Sondes d'imbrication profonde — chaque construct tourne en
//! test isolé : un stack overflow tue le binaire de test et
//! nomme le transporteur non gardé (classe « stack overflow
//! jscore » mesurée terrain — alpine/x.com, « fixe à un
//! compteur »).

use explorer_jscore::Engine;

fn probe(src: &str) {
    let mut e = Engine::new();
    // Err borné attendu (ou Ok si légitime) — jamais de crash.
    let _ = e.eval(src);
}

macro_rules! deep {
    ($name:ident, $n:expr, $src:expr) => {
        #[test]
        fn $name() {
            probe(&$src($n));
        }
    };
}

fn rep(n: usize, s: &str) -> String {
    s.repeat(n)
}

deep!(deep_member_chain, 50_000, |n| format!(
    "var o={{a:o}};o{}1",
    ".a".repeat(n)
));
deep!(deep_optional_chain, 50_000, |n| format!(
    "var o={{a:o}};o{}",
    "?.a".repeat(n)
));
deep!(deep_call_chain, 30_000, |n| rep(n, "f(")
    + "1"
    + &rep(n, ")"));
deep!(deep_comma_seq, 200_000, |n| rep(n, "0,") + "1");
deep!(deep_logical_or, 200_000, |n| rep(n, "0||") + "1");
deep!(deep_coalesce, 200_000, |n| rep(n, "0??") + "1");
deep!(deep_obj_literal, 50_000, |n| rep(n, "{a:")
    + "1"
    + &rep(n, "}"));
deep!(deep_array_lit, 50_000, |n| rep(n, "[") + &rep(n, "]"));
deep!(deep_group_pat, 50_000, |n| format!(
    "let {}=0",
    rep(n, "{a:") + &rep(n, "}")
));
deep!(deep_unary_void, 50_000, |n| rep(n, "void ") + "0");
deep!(deep_typeof, 50_000, |n| rep(n, "typeof ") + "x");
deep!(deep_not_chain, 100_000, |n| rep(n, "!") + "0");
deep!(deep_bitnot, 100_000, |n| rep(n, "~") + "0");
deep!(deep_await, 20_000, |n| rep(n, "await ") + "x");
deep!(deep_do_block, 50_000, |n| rep(n, "{") + &rep(n, "}"));
deep!(deep_while, 30_000, |n| rep(n, "while(0)"));
deep!(deep_label, 30_000, |n| rep(n, "a:") + ";");
deep!(deep_try, 30_000, |n| rep(n, "try{") + &rep(n, "}finally{}"));
deep!(deep_switch, 20_000, |n| rep(n, "switch(1){case 1:")
    + &rep(n, "}"));
deep!(deep_template_mid, 30_000, |n| rep(n, "${")
    + "1"
    + &rep(n, "}"));
deep!(deep_spread_call, 30_000, |n| rep(n, "f(...[")
    + "1"
    + &rep(n, "])"));
deep!(deep_regex_group, 20_000, |n| format!(
    "var r=/{}/;",
    rep(n, "(") + "a" + &rep(n, ")")
));
deep!(deep_regex_group_alt, 20_000, |n| format!(
    "var r=/{}/;",
    rep(n, "(?:") + "a" + &rep(n, ")")
));
deep!(deep_str_concat_eval, 50_000, |n| format!(
    "eval(\"{}\")",
    rep(n, "0+") + "1"
));

deep!(deep_json_literal, 30_000, |n| format!(
    "JSON.parse('{}')",
    rep(n, "[") + &rep(n, "]")
));
deep!(deep_innerhtml_parse, 40_000, |n| format!(
    "var d=document.createElement('div');d.innerHTML='{}'",
    rep(n, "<div>")
));
deep!(deep_clone, 40_000, |n| format!(
    "var d=document.createElement('div');d.innerHTML='{}';d.cloneNode(true).innerHTML.length",
    rep(n, "<b>")
));
deep!(deep_textcontent, 40_000, |n| format!(
    "var d=document.createElement('div');d.innerHTML='{}';d.textContent.length",
    rep(n, "<b>x</b>")
));
