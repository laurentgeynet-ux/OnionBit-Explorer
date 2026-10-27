// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Bissection du crash `alpine_bundle_dom` (J2) — quelle étape
//! mange la pile native : parse HTML, `install_dom`, ou eval.

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;

const HTML: &str = r##"<html><body><div x-data="{open:false}">
    <nav><ul><li><a href="#">a</a></li><li><a href="#">b</a></li></ul></nav>
    <div x-data="{items:[1,2,3]}"><template x-for="i in items"><span x-text="i"></span></template></div>
    <div x-data="{t:{n:0}}"><button @click="t.n++">+</button><span x-text="t.n"></span></div>
</div></body></html>"##;

#[test]
fn b1_parse_only() {
    let _d = Dom::from_html(HTML);
    eprintln!("[b1] parse ok");
}

#[test]
fn b2_install_only() {
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(HTML))));
    eprintln!("[b2] install ok");
}

#[test]
fn b3_eval_no_dom() {
    let Ok(src) = std::fs::read_to_string("../../docs/Rapport vol/logs/scratch/alpine.js")
        .or_else(|_| std::fs::read_to_string("docs/Rapport vol/logs/scratch/alpine.js"))
    else {
        eprintln!("[b3] bundle absent — skip");
        return;
    };
    let mut e = Engine::new();
    match e.eval(&src) {
        Ok(_) => eprintln!("[b3] eval sans DOM -> ok"),
        Err(x) => eprintln!("[b3] eval sans DOM -> err {x}"),
    }
}

#[test]
fn b4_eval_empty_dom() {
    let Ok(src) = std::fs::read_to_string("../../docs/Rapport vol/logs/scratch/alpine.js")
        .or_else(|_| std::fs::read_to_string("docs/Rapport vol/logs/scratch/alpine.js"))
    else {
        eprintln!("[b4] bundle absent — skip");
        return;
    };
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(
        "<html><body></body></html>",
    ))));
    match e.eval(&src) {
        Ok(_) => eprintln!("[b4] eval DOM vide -> ok"),
        Err(x) => eprintln!("[b4] eval DOM vide -> err {x}"),
    }
}

fn run_alpine(html: &str, tag: &str) {
    let Ok(src) = std::fs::read_to_string("../../docs/Rapport vol/logs/scratch/alpine.js")
        .or_else(|_| std::fs::read_to_string("docs/Rapport vol/logs/scratch/alpine.js"))
    else {
        eprintln!("[{tag}] bundle absent — skip");
        return;
    };
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(html))));
    match e.eval(&src) {
        Ok(_) => eprintln!("[{tag}] -> ok"),
        Err(x) => eprintln!(
            "[{tag}] -> err {}",
            format!("{x}").chars().take(160).collect::<String>()
        ),
    }
}

#[test]
fn b5_xdata_only() {
    run_alpine(
        r##"<html><body><div x-data="{open:false}"><nav><ul><li><a href="#">a</a></li></ul></nav></div></body></html>"##,
        "b5-xdata",
    );
}

#[test]
fn b6_xtext_only() {
    run_alpine(
        r#"<html><body><div x-data="{t:{n:0}}"><button @click="t.n++">+</button><span x-text="t.n"></span></div></body></html>"#,
        "b6-xtext",
    );
}

#[test]
fn b7_xfor_only() {
    run_alpine(
        r#"<html><body><div x-data="{items:[1,2,3]}"><template x-for="i in items"><span x-text="i"></span></template></div></body></html>"#,
        "b7-xfor",
    );
}

#[test]
fn b8_self_call() {
    let mut e = Engine::new();
    match e.eval("function f(){return f();} f();") {
        Ok(v) => eprintln!("[b8] -> {}", e.describe(v)),
        Err(x) => eprintln!("[b8] -> err {x}"),
    }
}

#[test]
fn b9_mutual() {
    let mut e = Engine::new();
    match e.eval("function a(){return b();} function b(){return a();} a();") {
        Ok(v) => eprintln!("[b9] -> {}", e.describe(v)),
        Err(x) => eprintln!("[b9] -> err {x}"),
    }
}

#[test]
fn b10_single_xdata() {
    run_alpine(
        r#"<html><body><div x-data="{open:false}"></div></body></html>"#,
        "b10-single",
    );
}

#[test]
fn b11_xdata_sibling() {
    run_alpine(
        r#"<html><body><div x-data="{a:1}"><p></p></div></body></html>"#,
        "b11-child",
    );
}

#[test]
fn b12_bigstack() {
    let h = std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            run_alpine(
                r#"<html><body><div x-data="{open:false}"></div></body></html>"#,
                "b12-64m",
            );
        })
        .expect("spawn");
    let _ = h.join();
}

/// Repro synthétique du motif Dn : `new AsyncFunction` + `with`
/// + écriture sur param — le code qu'alpine génère par expression.
#[test]
fn b15_asyncfn_with() {
    let mut e = Engine::new();
    match e.eval(
        "var AF = Object.getPrototypeOf(async function(){}).constructor;
         var f = new AF('__self','scope',
           'with(scope){__self.result = {open:false}}; __self.finished = true; return __self.result;');
         var out = [];
         f({result:null,finished:false}, {open:false})
           .then(function(v){ out.push('done '+JSON.stringify(v)); })
           .catch(function(err){ out.push('ERR '+err); });
         out.join('|')",
    ) {
        Ok(v) => eprintln!("[b15] -> {}", e.describe(v)),
        Err(x) => eprintln!("[b15] -> err {x}"),
    }
}

/// Repro `with` sur proxy fusionné (mergeProxies) + accès param.
#[test]
fn b16_with_proxy_scope() {
    let mut e = Engine::new();
    match e.eval(
        "var merged = new Proxy({objects:[{open:false}]}, {
           has: function(t,p){ return t.objects.some(function(o){ return p in o; }); },
           get: function(t,p){ for (var i=0;i<t.objects.length;i++){ if (p in t.objects[i]) return t.objects[i][p]; } },
           set: function(t,p,v){ for (var i=0;i<t.objects.length;i++){ if (p in t.objects[i]){ t.objects[i][p]=v; return true; } } t.objects[0][p]=v; return true; }
         });
         var AF = Object.getPrototypeOf(async function(){}).constructor;
         var f = new AF('__self','scope','with(scope){__self.result = open}; return __self.result;');
         var out = [];
         f({result:null}, merged).then(function(v){ out.push('done '+v); }).catch(function(err){ out.push('ERR '+err); });
         out.join('|')",
    ) {
        Ok(v) => eprintln!("[b16] -> {}", e.describe(v)),
        Err(x) => eprintln!("[b16] -> err {x}"),
    }
}

/// `'p' in proxyMerged` — le trap `has` seul, sans `with`.
#[test]
fn b17_proxy_has_trap() {
    let mut e = Engine::new();
    for src in [
        "var m = new Proxy({objects:[{open:false}]}, {
           has: function(t,p){ return t.objects.some(function(o){ return p in o; }); }
         }); 'open' in m",
        "var m2 = new Proxy({}, { has: function(t,p){ return p==='x'; } }); 'x' in m2",
        "var m3 = new Proxy({a:1}, {}); 'a' in m3",
    ] {
        match e.eval(src) {
            Ok(v) => eprintln!("[b17] -> {}", e.describe(v)),
            Err(x) => eprintln!("[b17] -> err {x}"),
        }
    }
}

/// `with(proxyMerged){ open }` — résolution d'identifiant dans
/// un scope proxy (le motif du code généré par `Dn`).
#[test]
fn b18_with_proxy_read() {
    let mut e = Engine::new();
    for src in [
        "var m = new Proxy({objects:[{open:false}]}, {
           has: function(t,p){ return t.objects.some(function(o){ return p in o; }); },
           get: function(t,p){ for (var i=0;i<t.objects.length;i++){ if (p in t.objects[i]) return t.objects[i][p]; } }
         });
         var out; with(m){ out = open; } 'w:'+String(out)",
        "var p2 = new Proxy({open:42}, { has: function(t,p){ return p in t; }, get: function(t,p){ return t[p]; } });
         var o2; with(p2){ o2 = open; } 'w2:'+String(o2)",
        "var o3 = {open: 7}; var r3; with(o3){ r3 = open; } 'w3:'+String(r3)",
    ] {
        match e.eval(src) {
            Ok(v) => eprintln!("[b18] -> {}", e.describe(v)),
            Err(x) => eprintln!("[b18] -> err {x}"),
        }
    }
}

/// Bissecte l'intérieur d'alpine : appelle les API publiques
/// une par une après eval — la première qui meurt nomme le bras.
#[test]
fn b19_internals() {
    let Ok(src) = std::fs::read_to_string("../../docs/Rapport vol/logs/scratch/alpine.js")
        .or_else(|_| std::fs::read_to_string("docs/Rapport vol/logs/scratch/alpine.js"))
    else {
        eprintln!("[b19] bundle absent — skip");
        return;
    };
    let steps = [
        ("eval bundle", None),
        ("reactive({})", Some("Alpine.reactive({}) !== null")),
        ("evaluateLater div", Some(
            "var el=document.querySelector('div'); typeof Alpine.evaluateLater(el,'{open:false}')",
        )),
        ("evaluate div", Some(
            "var el2=document.querySelector('div'); var r2; try { r2=Alpine.evaluate(el2,'{open:false}'); 'eval:'+JSON.stringify(r2) } catch(ex){ 'ERR '+ex }",
        )),
        ("closestDataStack", Some(
            "var el3=document.querySelector('div'); var st=Alpine.closestDataStack(el3); 'stack:'+st.length",
        )),
        ("mergeProxies", Some(
            "var mp=Alpine.mergeProxies([{a:1},{b:2}]); 'mp:'+(mp.a+mp.b)",
        )),
        ("initTree div", Some(
            "var el4=document.querySelector('div'); try { Alpine.initTree(el4); 'tree ok' } catch(ex){ 'ERR '+ex }",
        )),
        ("walk div", Some(
            "var el5=document.querySelector('div'); var n=0; Alpine.walk(el5,function(){n++}); 'walked '+n",
        )),
        ("start()", Some("try { Alpine.start(); 'started' } catch(ex){ 'ERR '+ex }")),
    ];
    for (tag, code) in steps {
        let mut e = Engine::new();
        // DOM vide : le bundle s'évalue et `start()` ne trouve
        // aucun composant — l'état « alpine prêt » est obtenu.
        e.install_dom(&Rc::new(RefCell::new(Dom::from_html(
            "<html><body></body></html>",
        ))));
        match e.eval(&src) {
            Ok(_) => {}
            Err(x) => {
                eprintln!("[b19] bundle eval -> err {x}");
                continue;
            }
        }
        if let Err(x) =
            e.eval("document.body.innerHTML='<div x-data=\"{open:false}\"></div>'; 'injected'")
        {
            eprintln!("[b19] inject -> err {x}");
            continue;
        }
        if let Some(c) = code {
            match e.eval(c) {
                Ok(v) => eprintln!("[b19:{tag}] -> {}", e.describe(v)),
                Err(x) => eprintln!("[b19:{tag}] -> err {x}"),
            }
        } else {
            eprintln!("[b19:{tag}] ok");
        }
    }
}

/// MO neutralisé + DOM vide au départ : isole chaque étape
/// interne (inject div → evaluate → initTree → walk).
#[test]
fn b22_manual_init() {
    let Ok(src) = std::fs::read_to_string("../../docs/Rapport vol/logs/scratch/alpine.js")
        .or_else(|_| std::fs::read_to_string("docs/Rapport vol/logs/scratch/alpine.js"))
    else {
        eprintln!("[b22] bundle absent — skip");
        return;
    };
    let mo_stub = "MutationObserver=function(){return {observe(){},disconnect(){},takeRecords(){return[]}}};'mo-stub'";
    for (tag, code) in [
        ("eval-only", ""),
        ("inject", "document.body.innerHTML='<div x-data=\"{open:false}\"></div>'; 'inj'"),
        (
            "inject+eval-expr",
            "document.body.innerHTML='<div x-data=\"{open:false}\"></div>';\
             var el=document.querySelector('div');\
             try{var r=Alpine.evaluate(el,'{open:false}');'ev '+JSON.stringify(r)}catch(x){'ERR '+x}",
        ),
        (
            "inject+walk",
            "document.body.innerHTML='<div x-data=\"{open:false}\"></div>';\
             var el=document.querySelector('div');var n=0;\
             try{Alpine.walk(el,function(){n++});'w '+n}catch(x){'ERR '+x}",
        ),
        (
            "inject+initTree",
            "document.body.innerHTML='<div x-data=\"{open:false}\"></div>';\
             var el=document.querySelector('div');\
             try{Alpine.initTree(el);'tree ok'}catch(x){'ERR '+x}",
        ),
    ] {
        let mut e = Engine::new();
        e.install_dom(&Rc::new(RefCell::new(Dom::from_html(
            "<html><body></body></html>",
        ))));
        if let Err(x) = e.eval(mo_stub) {
            eprintln!("[b22:{tag}] mo-stub -> err {x}");
            continue;
        }
        if let Err(x) = e.eval(&src) {
            eprintln!("[b22:{tag}] bundle -> err {x}");
            continue;
        }
        if code.is_empty() {
            eprintln!("[b22:{tag}] eval ok");
            continue;
        }
        match e.eval(code) {
            Ok(v) => eprintln!("[b22:{tag}] -> {}", e.describe(v)),
            Err(x) => eprintln!("[b22:{tag}] -> err {x}"),
        }
    }
}

/// Repro verbatim de la chaîne `evaluate` d'alpine (Dn+Pn+B) :
/// `AsyncFunction` + with(mergedProxy) + self-call n(n,a).
#[test]
fn b23_evaluate_chain() {
    // Tn = les traps exactes de mergeProxies d'alpine.
    let tn = r#"
var Tn={
  ownKeys:function(t){return Array.from(new Set(t.objects.flatMap(function(x){return Object.keys(x)})))},
  has:function(t,k){return k==Symbol.unscopables?false:t.objects.some(function(r){return Object.prototype.hasOwnProperty.call(r,k)||Reflect.has(r,k)})},
  get:function(t,k,r){return k=="toJSON"?function(){}:Reflect.get(t.objects.find(function(n){return Reflect.has(n,k)})||{},k,r)},
  set:function(t,k,v,r){var i=t.objects.find(function(s){return Object.prototype.hasOwnProperty.call(s,k)})||t.objects[t.objects.length-1];return Reflect.set(i,k,v,r)}
};
function B(e){return new Proxy({objects:e},Tn)}
"#;
    for (tag, code) in [
        (
            "af-ctor",
            "var AF=Object.getPrototypeOf(async function(){}).constructor;\
             typeof AF",
        ),
        (
            "af-plain",
            "var AF=Object.getPrototypeOf(async function(){}).constructor;\
             var s=new AF(['__self','scope'],\
               'with(scope){__self.result={open:false}};__self.finished=true;return __self.result');\
             'made'",
        ),
        (
            "af-call-plainscope",
            "var AF=Object.getPrototypeOf(async function(){}).constructor;\
             var s=new AF(['__self','scope'],\
               'with(scope){__self.result={open:false}};__self.finished=true;return __self.result');\
             var p=s(s,{x:1}); 'called '+typeof p",
        ),
        (
            "af-call-proxyscope",
            "var AF=Object.getPrototypeOf(async function(){}).constructor;\
             var s=new AF(['__self','scope'],\
               'with(scope){__self.result={open:false}};__self.finished=true;return __self.result');\
             var a=B([{},{mag:1}]);\
             var p=s(s,a); 'called '+typeof p",
        ),
        (
            "pn-verbatim",
            "var AF=Object.getPrototypeOf(async function(){}).constructor;\
             var n=new AF(['__self','scope'],\
               'with(scope){__self.result={open:false}};__self.finished=true;return __self.result');\
             n.result=void 0;n.finished=!1;\
             var a=B([{},{mag:1}]);\
             var c=n(n,a).catch(function(l){return 'caught'});\
             'fin='+n.finished",
        ),
    ] {
        let mut e = Engine::new();
        let full = format!("{tn}\n{code}");
        match e.eval(&full) {
            Ok(v) => eprintln!("[b23:{tag}] -> {}", e.describe(v)),
            Err(x) => eprintln!("[b23:{tag}] -> err {x}"),
        }
    }
}

/// Isolation fine du triplet async × with × proxy.
#[test]
fn b24_with_proxy_matrix() {
    let tn = r#"
var Tn={
  has:function(t,k){return k==Symbol.unscopables?false:t.objects.some(function(r){return Object.prototype.hasOwnProperty.call(r,k)||Reflect.has(r,k)})},
  get:function(t,k,r){return k=="toJSON"?function(){}:Reflect.get(t.objects.find(function(n){return Reflect.has(n,k)})||{},k,r)},
  set:function(t,k,v,r){var i=t.objects.find(function(s){return Object.prototype.hasOwnProperty.call(s,k)})||t.objects[t.objects.length-1];return Reflect.set(i,k,v,r)}
};
function B(e){return new Proxy({objects:e},Tn)}
var P1=new Proxy({},{});
var P2=new Proxy({a:7},{has:function(t,k){return true},get:function(t,k){return k in t?t[k]:undefined}});
"#;
    for (tag, code) in [
        (
            "sync-with-proxy",
            "function f(scope){with(scope){var r=42}return r};f(P1)",
        ),
        (
            "sync-with-alpineproxy",
            "function f(scope){with(scope){var r=42}return r};f(B([{},{m:1}]))",
        ),
        (
            "async-nowith-proxy",
            "var AF=Object.getPrototypeOf(async function(){}).constructor;\
             var s=new AF(['scope'],'return 42'); s(P1); 'ok'",
        ),
        (
            "async-with-plain",
            "var AF=Object.getPrototypeOf(async function(){}).constructor;\
             var s=new AF(['scope'],'with(scope){var r=42}return r'); s({}); 'ok'",
        ),
        (
            "async-with-proxy-notraps",
            "var AF=Object.getPrototypeOf(async function(){}).constructor;\
             var s=new AF(['scope'],'with(scope){var r=42}return r'); s(P1); 'ok'",
        ),
        (
            "async-with-proxy-trivial",
            "var AF=Object.getPrototypeOf(async function(){}).constructor;\
             var s=new AF(['scope'],'with(scope){var r=42}return r'); s(P2); 'ok'",
        ),
        (
            "async-with-alpineproxy",
            "var AF=Object.getPrototypeOf(async function(){}).constructor;\
             var s=new AF(['scope'],'with(scope){var r=42}return r'); s(B([{},{m:1}])); 'ok'",
        ),
    ] {
        let mut e = Engine::new();
        let full = format!("{tn}\n{code}");
        match e.eval(&full) {
            Ok(v) => eprintln!("[b24:{tag}] -> {}", e.describe(v)),
            Err(x) => eprintln!("[b24:{tag}] -> err {x}"),
        }
    }
}

/// Une trap à la fois : has seul / get seul / set seul / has trivial.
#[test]
fn b25_trap_matrix() {
    for (tag, setup) in [
        (
            "has-trivial-true",
            "var P=new Proxy({a:1},{has:function(t,k){return true}});with(P){a}",
        ),
        (
            "has-trivial-false",
            "var P=new Proxy({a:1},{has:function(t,k){return false}});'n'",
        ),
        (
            "has-alpine",
            "var P=new Proxy({objects:[{},{m:1}]},{has:function(t,k){return k==Symbol.unscopables?false:t.objects.some(function(r){return Object.prototype.hasOwnProperty.call(r,k)||Reflect.has(r,k)})}});\
             function f(s){with(s){return 42}};f(P)",
        ),
        (
            "get-trivial",
            "var P=new Proxy({a:1},{get:function(t,k){return t[k]}});with(P){a}",
        ),
        (
            "get-alpine",
            "var P=new Proxy({objects:[{m:1}]},{get:function(t,k,r){return k==\"toJSON\"?function(){}:Reflect.get(t.objects.find(function(n){return Reflect.has(n,k)})||{},k,r)}});\
             function f(s){with(s){return m}};f(P)",
        ),
        (
            "set-alpine",
            "var P=new Proxy({objects:[{m:1}]},{set:function(t,k,v,r){var i=t.objects.find(function(s){return Object.prototype.hasOwnProperty.call(s,k)})||t.objects[t.objects.length-1];return Reflect.set(i,k,v,r)}});\
             function f(s){with(s){m=9;return m}};f(P)",
        ),
    ] {
        let mut e = Engine::new();
        match e.eval(setup) {
            Ok(v) => eprintln!("[b25:{tag}] -> {}", e.describe(v)),
            Err(x) => eprintln!("[b25:{tag}] -> err {x}"),
        }
    }
}

/// Combinaisons des traps : has+get, has+set, get+set, has+get+set.
#[test]
fn b26_trap_pairs() {
    let has = "has:function(t,k){return k==Symbol.unscopables?false:t.objects.some(function(r){return Object.prototype.hasOwnProperty.call(r,k)||Reflect.has(r,k)})}";
    let get = "get:function(t,k,r){return k==\"toJSON\"?function(){}:Reflect.get(t.objects.find(function(n){return Reflect.has(n,k)})||{},k,r)}";
    let set = "set:function(t,k,v,r){var i=t.objects.find(function(s){return Object.prototype.hasOwnProperty.call(s,k)})||t.objects[t.objects.length-1];return Reflect.set(i,k,v,r)}";
    for (tag, traps) in [
        ("has+get", format!("{has},{get}")),
        ("has+set", format!("{has},{set}")),
        ("get+set", format!("{get},{set}")),
        ("has+get+set", format!("{has},{get},{set}")),
    ] {
        let mut e = Engine::new();
        let code = format!(
            "var P=new Proxy({{objects:[{{}},{{m:1}}]}},{{{traps}}});\
             function f(s){{with(s){{var r=42}}return r}};f(P)"
        );
        match e.eval(&code) {
            Ok(v) => eprintln!("[b26:{tag}] -> {}", e.describe(v)),
            Err(x) => eprintln!("[b26:{tag}] -> err {x}"),
        }
    }
}

/// Bissecte le corps du trap `has` : chaque sous-expression seule.
#[test]
fn b27_has_body() {
    let get = "get:function(t,k,r){return k==\"toJSON\"?function(){}:Reflect.get(t.objects.find(function(n){return Reflect.has(n,k)})||{},k,r)}";
    for (tag, has) in [
        ("hop-call", "has:function(t,k){return Object.prototype.hasOwnProperty.call(t.objects[0],k)}"),
        ("reflect-has", "has:function(t,k){return Reflect.has(t.objects[0],k)}"),
        ("some-hop", "has:function(t,k){return t.objects.some(function(r){return Object.prototype.hasOwnProperty.call(r,k)})}"),
        ("some-reflect", "has:function(t,k){return t.objects.some(function(r){return Reflect.has(r,k)})}"),
        ("some-full", "has:function(t,k){return t.objects.some(function(r){return Object.prototype.hasOwnProperty.call(r,k)||Reflect.has(r,k)})}"),
        ("unscopables", "has:function(t,k){return k==Symbol.unscopables}"),
    ] {
        let mut e = Engine::new();
        let code = format!(
            "var P=new Proxy({{objects:[{{}},{{m:1}}]}},{{{has},{get}}});\
             function f(s){{with(s){{var r=42}}return r}};f(P)"
        );
        match e.eval(&code) {
            Ok(v) => eprintln!("[b27:{tag}] -> {}", e.describe(v)),
            Err(x) => eprintln!("[b27:{tag}] -> err {x}"),
        }
    }
}

/// Régression JS-C167 — B-3 résolu : `loose_eq` rebouclait sur
/// `to_primitive(Symbol)` (Obj resté `Obj`) → overflow natif.
#[test]
fn b28_regression() {
    for (tag, code, want) in [
        ("sym-eq-str", "'x'==Symbol.unscopables", "false"),
        ("sym-eq-sym", "Symbol.unscopables==Symbol.unscopables", "true"),
        ("sym-neq", "Symbol.unscopables!=Symbol.iterator", "true"),
        (
            "with-alpine-merged",
            "var Tn={has:function(t,k){return k==Symbol.unscopables?false:t.objects.some(function(r){return Object.prototype.hasOwnProperty.call(r,k)||Reflect.has(r,k)})},\
             get:function(t,k,r){return k=='toJSON'?function(){}:Reflect.get(t.objects.find(function(n){return Reflect.has(n,k)})||{},k,r)},\
             set:function(t,k,v,r){var i=t.objects.find(function(s){return Object.prototype.hasOwnProperty.call(s,k)})||t.objects[t.objects.length-1];return Reflect.set(i,k,v,r)}};\
             var P=new Proxy({objects:[{open:false},{m:1}]},Tn);\
             function f(s){with(s){return open}};f(P)",
            "false",
        ),
        (
            "callable-proxy-chain",
            "var p=new Proxy(function(){return 7},{});\
             for(var i=0;i<200;i++)p=new Proxy(p,{});\
             typeof p=='function'&&p()==7",
            "true",
        ),
        (
            "callable-proxy-chain-deep",
            "var p=new Proxy(function(){return 7},{});\
             for(var i=0;i<5000;i++)p=new Proxy(p,{});\
             try{p()}catch(x){x&&x.name=='RangeError'?'range':'?'+x.name}",
            "range",
        ),
    ] {
        let mut e = Engine::new();
        let got = match e.eval(code) {
            Ok(v) => e.describe(v),
            Err(x) => format!("err {x}"),
        };
        // Le point J2/B-3 : jamais de mort process — le résultat
        // doit être une valeur ou une erreur JS bornée.
        assert_eq!(got, want, "[b28:{tag}] {code}");
    }
}

/// `innerHTML` + observer d'alpine : div neutre vs div x-data.
#[test]
fn b20_innerhtml_observer() {
    let Ok(src) = std::fs::read_to_string("../../docs/Rapport vol/logs/scratch/alpine.js")
        .or_else(|_| std::fs::read_to_string("docs/Rapport vol/logs/scratch/alpine.js"))
    else {
        eprintln!("[b20] bundle absent — skip");
        return;
    };
    for (tag, html) in [
        ("plain-div", "<div></div>"),
        ("xdata-div", "<div x-data=\"{open:false}\"></div>"),
    ] {
        let mut e = Engine::new();
        e.install_dom(&Rc::new(RefCell::new(Dom::from_html(
            "<html><body></body></html>",
        ))));
        if let Err(x) = e.eval(&src) {
            eprintln!("[b20:{tag}] bundle -> err {x}");
            continue;
        }
        match e.eval(&format!("document.body.innerHTML='{html}'; 'ok'")) {
            Ok(v) => eprintln!("[b20:{tag}] inject -> {}", e.describe(v)),
            Err(x) => eprintln!("[b20:{tag}] inject -> err {x}"),
        }
    }
}

/// Après injection d'un div x-data : isole evaluate / initTree /
/// walk / observer dans des évals séparés.
#[test]
fn b21_init_steps() {
    let Ok(src) = std::fs::read_to_string("../../docs/Rapport vol/logs/scratch/alpine.js")
        .or_else(|_| std::fs::read_to_string("docs/Rapport vol/logs/scratch/alpine.js"))
    else {
        eprintln!("[b21] bundle absent — skip");
        return;
    };
    for (tag, code) in [
        ("eval-only", ""),
        (
            "evaluate",
            "var el=document.querySelector('div'); try{var r=Alpine.evaluate(el,'{open:false}');'eval '+JSON.stringify(r)}catch(x){'ERR '+x}",
        ),
        (
            "closestDataStack",
            "var el=document.querySelector('div'); try{'cds '+Alpine.closestDataStack(el).length}catch(x){'ERR '+x}",
        ),
        (
            "mergeProxies",
            "try{'mp '+Alpine.mergeProxies([{a:1}]).a}catch(x){'ERR '+x}",
        ),
        (
            "walk",
            "var el=document.querySelector('div'); var n=0; try{Alpine.walk(el,function(){n++});'walk '+n}catch(x){'ERR '+x}",
        ),
        (
            "initTree",
            "var el=document.querySelector('div'); try{Alpine.initTree(el);'tree ok'}catch(x){'ERR '+x}",
        ),
    ] {
        let mut e = Engine::new();
        e.install_dom(&Rc::new(RefCell::new(Dom::from_html(
            "<html><body><div x-data=\"{open:false}\"></div></body></html>",
        ))));
        // Bundle + div présents : l'observer part au drain post-eval
        // seulement si la boucle init n'a pas déjà tout joué.
        if let Err(x) = e.eval(&src) {
            eprintln!("[b21:{tag}] bundle -> err {x}");
            continue;
        }
        if code.is_empty() {
            eprintln!("[b21:{tag}] eval ok");
            continue;
        }
        match e.eval(code) {
            Ok(v) => eprintln!("[b21:{tag}] -> {}", e.describe(v)),
            Err(x) => eprintln!("[b21:{tag}] -> err {x}"),
        }
    }
}

#[test]
fn b13_parent_chain() {
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(
        "<html><body><div x-data=\"{open:false}\"></div></body></html>",
    ))));
    match e.eval(
        "var n=document.querySelector('div'); var out=[]; var seen={};
         for(var i=0;i<40;i++){ if(!n){out.push('null@'+i);break;}
           var id=n.tagName+'#'+(n.nodeType);
           if(seen[id]&&seen[id]>2){out.push('CYCLE '+id+' @'+i);break;}
           seen[id]=(seen[id]||0)+1;
           n=n.parentNode; }
         out.push(n===null?'END-NULL':typeof n);
         out.join('|')",
    ) {
        Ok(v) => eprintln!("[b13] {}", e.describe(v)),
        Err(x) => eprintln!("[b13] err {x}"),
    }
}

/// b29 — `%AsyncFunction%` caché : la voie alpine
/// `Object.getPrototypeOf(async f).constructor` doit rendre un
/// ctor dont le produit retourne une **promesse** (`.catch`
/// existe — avant : fonction sync → valeur brute → `TypeError:
/// catch is not a function` sur chaque `evaluate`).
#[test]
fn b29_async_function_ctor() {
    let mut e = Engine::new();
    // 1. ctor obtenu par la voie spec (proto d'un async fn).
    // 2. `new AsyncFunction` → fonction dont l'appel rend une
    //    promesse qui résout la valeur de retour.
    // 3. `AsyncFunction` direct (sans new, via le ctor caché)
    //    idem — et `new GeneratorFunction` produit un gén.
    let src = r#"
        var AF = Object.getPrototypeOf(async function(){}).constructor;
        var f = new AF("a", "return a+1");
        var p = f(41);
        var out = [];
        out.push(typeof AF);
        out.push(typeof p.catch === 'function');
        out.push(p instanceof Promise);
        out.push(f.constructor === AF);
        var r = null;
        p.then(function(v){ r = v; });
        out.join('|') + '##' + r
    "#;
    match e.eval(src) {
        Ok(v) => {
            let d = e.describe(v);
            eprintln!("[b29] {d}");
            assert!(
                d.contains("function|true|true|true"),
                "chaine ctor/promesse: {d}"
            );
            assert!(
                d.contains("##42") || d.contains("##null"),
                "valeur resolue: {d}"
            );
        }
        Err(x) => panic!("b29 eval err {x}"),
    }
    // `then` livré via microtask — draine.
    let r2 = e.eval("(function(){var AF=Object.getPrototypeOf(async function(){}).constructor;var f=new AF('a','return a+1');var r=null;f(41).then(function(v){r=v});return r})()");
    let _ = r2;
    // Generator/AsyncGeneratorFunction produisent le bon genre.
    match e.eval(
        "var GF=Object.getPrototypeOf(function*(){}).constructor; var g=new GF('yield 7'); typeof g().next=='function'",
    ) {
        Ok(v) => assert_eq!(e.describe(v), "true"),
        Err(x) => panic!("gen ctor err {x}"),
    }
}

/// b30 — résidu vol : scopes `x-for`/imbriqués. Charge le bundle
/// réel sur un DOM avec `x-data` + `x-for` + prop imbriquée, puis
/// vérifie que les expressions résolvent (pas de `ReferenceError`).
#[test]
fn b30_xfor_scope_render() {
    let Ok(src) = std::fs::read_to_string("../../docs/Rapport vol/logs/scratch/alpine.js")
        .or_else(|_| std::fs::read_to_string("docs/Rapport vol/logs/scratch/alpine.js"))
    else {
        eprintln!("[b30] bundle absent — skip");
        return;
    };
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(
        r#"<html><body>
          <div x-data="{items:[10,20]}"><template x-for="i in items"><span x-text="i"></span></template></div>
          <div x-data="{step:1}"><p x-text="step"></p><div x-data="{own:2}"><b x-text="step"></b></div></div>
        </body></html>"#,
    ))));
    if let Err(x) = e.eval(&src) {
        eprintln!("[b30] bundle err {x}");
    }
    // Draine microtasks + timers (initTree est async-schedulé).
    let _ = e.eval("void 0");
    match e.eval(
        "(function(){var s=[]; var spans=document.querySelectorAll('span');
          for(var i=0;i<spans.length;i++)s.push(spans[i].textContent);
          var b=document.querySelector('b'); s.push(b?b.textContent:'no-b');
          return s.join(',')})()",
    ) {
        Ok(v) => eprintln!("[b30] rendu spans+b = {}", e.describe(v)),
        Err(x) => eprintln!("[b30] query err {x}"),
    }
    match e.eval(
        "(function(){var errs=[]; var els=document.querySelectorAll('[x-data]');
          for(var i=0;i<els.length;i++){ try { Alpine.evaluate(els[i],'1+1'); } catch(x){ errs.push(''+x); } }
          return errs.join(';')||'ok'})()",
    ) {
        Ok(v) => eprintln!("[b30] evaluate direct = {}", e.describe(v)),
        Err(x) => eprintln!("[b30] evaluate err {x}"),
    }
}

/// b31 — dump DOM post-init : où en est `x-for` (clones produits ?
/// `template.content` ? directive ignorée ?).
#[test]
fn b31_xfor_dom_dump() {
    let Ok(src) = std::fs::read_to_string("../../docs/Rapport vol/logs/scratch/alpine.js")
        .or_else(|_| std::fs::read_to_string("docs/Rapport vol/logs/scratch/alpine.js"))
    else {
        eprintln!("[b31] bundle absent — skip");
        return;
    };
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(
        r#"<html><body>
          <div id=a x-data="{items:[10,20]}"><template x-for="i in items"><span x-text="i"></span></template></div>
          <div id=b x-data="{step:1}"><p x-text="step"></p></div>
        </body></html>"#,
    ))));
    let _ = e.eval(&src);
    let _ = e.eval("void 0");
    for probe in [
        "document.getElementById('a').innerHTML",
        "document.getElementById('b').innerHTML",
        "typeof document.querySelector('template').content",
        "document.querySelectorAll('span').length",
    ] {
        match e.eval(probe) {
            Ok(v) => eprintln!("[b31] {probe} => {}", e.describe(v)),
            Err(x) => eprintln!("[b31] {probe} => err {x}"),
        }
    }
}

/// b32 — `i` du clone x-for vaut 1 : itération scope ratée ou
/// `i` lu dans un global fuité ?
#[test]
fn b32_xfor_scope_probe() {
    let Ok(src) = std::fs::read_to_string("../../docs/Rapport vol/logs/scratch/alpine.js")
        .or_else(|_| std::fs::read_to_string("docs/Rapport vol/logs/scratch/alpine.js"))
    else {
        eprintln!("[b32] bundle absent — skip");
        return;
    };
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(
        r#"<html><body>
          <div id=a x-data="{items:[10,20]}"><template x-for="i in items"><span x-text="i"></span></template></div>
        </body></html>"#,
    ))));
    let _ = e.eval(&src);
    let _ = e.eval("void 0");
    for probe in [
        "Alpine.evaluate(document.getElementById('a'),'items')",
        "Alpine.evaluate(document.getElementById('a'),'typeof items')",
        "Alpine.evaluate(document.getElementById('a'),'i')",
        "typeof window.i",
        "Alpine.evaluate(document.querySelector('span'),'i')",
        "Alpine.evaluate(document.querySelector('span'),'typeof i')",
        "document.querySelector('template').content===document.querySelector('template').content",
        "typeof document.querySelector('template').content.querySelectorAll",
    ] {
        match e.eval(probe) {
            Ok(v) => eprintln!("[b32] {probe} => {}", e.describe(v)),
            Err(x) => eprintln!("[b32] {probe} => err {x}"),
        }
    }
}

/// b33 — `items` évalué = objet (array [10,20]?) mais une seule
/// itération i=1 : `items.length`/index lecture dans la pile x-data.
#[test]
fn b33_xfor_items_probe() {
    let Ok(src) = std::fs::read_to_string("../../docs/Rapport vol/logs/scratch/alpine.js")
        .or_else(|_| std::fs::read_to_string("docs/Rapport vol/logs/scratch/alpine.js"))
    else {
        eprintln!("[b33] bundle absent — skip");
        return;
    };
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(
        r#"<html><body>
          <div id=a x-data="{items:[10,20,30]}"><template x-for="i in items"><span x-text="i"></span></template></div>
        </body></html>"#,
    ))));
    let _ = e.eval(&src);
    let _ = e.eval("void 0");
    for probe in [
        "Alpine.evaluate(document.getElementById('a'),'items.length')",
        "Alpine.evaluate(document.getElementById('a'),'items[0]')",
        "Alpine.evaluate(document.getElementById('a'),'items[1]')",
        "Alpine.evaluate(document.getElementById('a'),'Array.isArray(items)')",
        "Alpine.evaluate(document.getElementById('a'),'items.constructor.name')",
        "document.querySelectorAll('#a > span').length",
        "(function(){var s='';var sp=document.querySelectorAll('span');for(var j=0;j<sp.length;j++)s+='['+sp[j].textContent+']';return s})()",
        "document.getElementById('a')._x_dataStack?document.getElementById('a')._x_dataStack.length:'nostack'",
    ] {
        match e.eval(probe) {
            Ok(v) => eprintln!("[b33] {probe} => {}", e.describe(v)),
            Err(x) => eprintln!("[b33] {probe} => err {x}"),
        }
    }
}

/// b34 — motif réel alpinejs.dev : `x-for="([number,name],key) in
/// steps"` sur **objet** + clones lisant `currentStep` du scope
/// parent (`:class`, `@click`, `x-init` `$watch`).
#[test]
fn b34_xfor_object_key_parent_scope() {
    let Ok(src) = std::fs::read_to_string("../../docs/Rapport vol/logs/scratch/alpine.js")
        .or_else(|_| std::fs::read_to_string("docs/Rapport vol/logs/scratch/alpine.js"))
    else {
        eprintln!("[b34] bundle absent — skip");
        return;
    };
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(
        r#"<html><body><div id=r x-data="{currentStep:'a', steps:{a:[15,'A'],b:[6,'B']}}">
          <template x-for="([number,name],key) in steps">
            <button :class="currentStep===key && 'on'" @click="currentStep=key"><b x-text="number"></b><i x-text="name"></i></button>
          </template>
        </div></body></html>"#,
    ))));
    if let Err(x) = e.eval(&src) {
        eprintln!("[b34] bundle err {x}");
    }
    let _ = e.eval("void 0");
    for probe in [
        "document.querySelectorAll('button').length",
        "(function(){var b=document.querySelectorAll('button'),s='';for(var i=0;i<b.length;i++)s+='['+b[i].className+'|'+b[i].textContent+']';return s})()",
    ] {
        match e.eval(probe) {
            Ok(v) => eprintln!("[b34] {probe} => {}", e.describe(v)),
            Err(x) => eprintln!("[b34] {probe} => err {x}"),
        }
    }
}

/// b35 — clic sur clone : `currentStep=key` en scope item →
/// délégation parent + `$watch` qui lit `steps`/`direction`.
#[test]
fn b35_xfor_click_watch() {
    let Ok(src) = std::fs::read_to_string("../../docs/Rapport vol/logs/scratch/alpine.js")
        .or_else(|_| std::fs::read_to_string("docs/Rapport vol/logs/scratch/alpine.js"))
    else {
        eprintln!("[b35] bundle absent — skip");
        return;
    };
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(
        r#"<html><body><div id=r x-data="{currentStep:'a', direction:'right',
             steps:{a:[15,'A'],b:[6,'B']}}"
             x-init="$watch('currentStep',(v,o)=>{var k=Object.keys(steps);direction=k.indexOf(v)>k.indexOf(o)?'right':'left'})">
          <template x-for="([number,name],key) in steps">
            <button :class="currentStep===key && 'on'" @click="currentStep=key"><b x-text="number"></b></button>
          </template>
        </div></body></html>"#,
    ))));
    let _ = e.eval(&src);
    let _ = e.eval("void 0");
    for probe in [
        "document.querySelectorAll('button')[1].click(); 'clicked'",
        "Alpine.evaluate(document.getElementById('r'),'currentStep')",
        "Alpine.evaluate(document.getElementById('r'),'direction')",
    ] {
        match e.eval(probe) {
            Ok(v) => eprintln!("[b35] {probe} => {}", e.describe(v)),
            Err(x) => eprintln!("[b35] {probe} => err {x}"),
        }
    }
}
