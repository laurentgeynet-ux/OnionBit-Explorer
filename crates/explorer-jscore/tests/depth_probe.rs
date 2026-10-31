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

// --- Transporteurs runtime (J2) — récursions d'appel hors
// `call_depth` : walk stringify, proto-chaîne proxy→proxy,
// flat récursif, RegExp backtrack, coercion chaînée.

deep!(deep_stringify_nested, 30_000, |n| format!(
    "var o={{}};for(var i=0;i<{n};i++)o={{a:o}};JSON.stringify(o).length"
));
deep!(deep_stringify_cyc, 20_000, |n| format!(
    "var o={{}};for(var i=0;i<{n};i++)o={{a:o}};o.a=o;try{{JSON.stringify(o)}}catch(e){{e.name}}"
));
deep!(deep_proxy_proto_chain, 20_000, |n| format!(
    "var o={{x:1}};for(var i=0;i<{n};i++)o=new Proxy(o,{{}});o.x"
));
deep!(deep_proxy_target_chain, 20_000, |n| {
    format!(
    "var o={{x:1}};for(var i=0;i<{n};i++)o=new Proxy(o,{{ownKeys:function(t){{return Reflect.ownKeys(t)}}}});Object.keys(o).join()"
)
});
deep!(deep_flat_nested, 30_000, |n| rep(n, "[")
    + "1"
    + &rep(n, "]")
    + ".flat(Infinity).length");
deep!(deep_regex_backtrack, 15_000, |n| format!(
    "'{}' + 'x' .match(/(a+)+$/).index",
    "a".repeat(28)
) + &format!(";{n}"));
deep!(deep_tostring_chain, 40_000, |n| format!(
    "var o={{toString:function(){{return 'x'}}}};var s=o;for(var i=0;i<{n};i++)s=[s];String(s)"
));
deep!(deep_concat_chain, 30_000, |n| format!(
    "var a=[1];for(var i=0;i<{n};i++)a=[a];a.flat(Infinity).length"
));
deep!(deep_getters_native, 10_000, |n| {
    format!(
    "var o={{}};var cur=o;for(var i=0;i<{n};i++){{var nxt={{}};Object.defineProperty(cur,'n',{{get:function(){{return nxt}}}});cur=nxt}}o.n.x"
)
});

// --- Chaînes de délégation (J2) — chaque méthode interne qui
// re-sautait de proxy en proxy (ou de bound en bound) hors
// `call_depth` : get/set/has/delete/ownKeys/getOwnPropertyDescriptor
// + Reflect/Object statics + instanceof sur bind chaîné.

deep!(deep_proxy_has_chain, 20_000, |n| format!(
    "var o={{x:1}};for(var i=0;i<{n};i++)o=new Proxy(o,{{}});'x' in o"
));
deep!(deep_proxy_delete_chain, 20_000, |n| format!(
    "var o={{x:1}};for(var i=0;i<{n};i++)o=new Proxy(o,{{}});delete o.x"
));
deep!(deep_proxy_set_chain, 20_000, |n| format!(
    "var o={{x:1}};for(var i=0;i<{n};i++)o=new Proxy(o,{{}});o.x=2;o.x"
));
deep!(deep_proxy_ownkeys_chain, 20_000, |n| format!(
    "var o={{x:1}};for(var i=0;i<{n};i++)o=new Proxy(o,{{}});Object.keys(o).length"
));
deep!(deep_proxy_getowndesc_chain, 20_000, |n| {
    format!(
    "var o={{x:1}};for(var i=0;i<{n};i++)o=new Proxy(o,{{}});Object.getOwnPropertyDescriptor(o,'x').value"
)
});
deep!(deep_proxy_defineprop_chain, 20_000, |n| {
    format!(
    "var o={{}};for(var i=0;i<{n};i++)o=new Proxy(o,{{}});Object.defineProperty(o,'y',{{value:7,configurable:true}});o.y"
)
});
deep!(deep_proxy_getproto_chain, 20_000, |n| format!(
    "var o={{}};for(var i=0;i<{n};i++)o=new Proxy(o,{{}});typeof Object.getPrototypeOf(o)"
));
deep!(deep_proxy_setproto_chain, 20_000, |n| {
    format!(
    "var o={{}};for(var i=0;i<{n};i++)o=new Proxy(o,{{}});try{{Object.setPrototypeOf(o,{{}});'ok'}}catch(e){{e.name}}"
)
});
deep!(deep_proxy_preventext_chain, 20_000, |n| {
    format!(
    "var o={{}};for(var i=0;i<{n};i++)o=new Proxy(o,{{}});Object.preventExtensions(o);Object.isExtensible(o)"
)
});
deep!(deep_bind_instanceof_chain, 20_000, |n| format!(
    "function F(){{}};var b=F;for(var i=0;i<{n};i++)b=b.bind(null);(new F()) instanceof b"
));

/// §B-59 second tueur / vol 368 (snapchat éval 9 Mo → kill →
/// PID daemon perdu ; même signature clubmed vol 158) : un
/// corps profond survit dans le tas via `Rc<FnLit>` → la
/// chute de l'engine détruit l'AST **récursivement**, hors
/// toute garde `maybe_grow` → débordement pile native en
/// `Drop`. `Engine::drop` vide `modules`/`call_stack`/`heap`/
/// parked sous `stacker` — le test meurt en crash si la
/// garde saute.
#[test]
fn engine_drop_deep_ast_under_stacker() {
    let mut e = Engine::new();
    let body = "(".repeat(40_000) + "1" + &")".repeat(40_000);
    let _ = e.eval(&format!("globalThis.f = function(){{ return {body} }}"));
    drop(e); // avant le correctif : stack overflow en Drop
}
