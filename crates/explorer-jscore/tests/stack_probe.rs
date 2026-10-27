// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Sonde stack-overflow (mur alpine.js) — chaque construct
//! profond dans un test séparé : le dernier nommé avant
//! `abort` est le transporteur non borné.

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::fmt::Write as _;
use std::rc::Rc;

fn eval_deep(src: &str) -> String {
    let mut e = Engine::new();
    match e.eval(src) {
        Ok(_) => "ok".into(),
        Err(err) => format!("err:{err}").chars().take(60).collect(),
    }
}

macro_rules! probe {
    ($name:ident, $src:expr) => {
        #[test]
        fn $name() {
            eprintln!("[probe] {}", stringify!($name));
            let out = eval_deep(&$src);
            eprintln!("[probe] {} -> {}", stringify!($name), out);
        }
    };
}

probe!(deep_parens, {
    let n = 60000;
    format!("var s='{}';eval(s)", "(".repeat(n) + "1" + &")".repeat(n))
});
probe!(deep_arrays, {
    let n = 60000;
    format!("eval('{}')", "[".repeat(n) + &"]".repeat(n))
});
probe!(deep_objects, {
    let n = 20000;
    format!("eval('{}')", "{a:".repeat(n) + "1" + &"}".repeat(n))
});
probe!(deep_ternary, {
    let n = 40000;
    format!("eval('{}')", "0?0:".repeat(n) + "1")
});
probe!(deep_unary, {
    let n = 60000;
    format!("eval('{}')", "!".repeat(n) + "0")
});
probe!(deep_news, {
    let n = 20000;
    format!("eval('{}')", "new ".repeat(n) + "Object()")
});
probe!(deep_fn_nest, {
    let n = 12000;
    let mut s = String::from("function a(){");
    for i in 0..n {
        let _ = write!(s, "function f{i}(){{");
    }
    s.push('1');
    s.push_str(&"}".repeat(n + 1));
    format!("eval({s:?})")
});
probe!(deep_stmts, {
    let n = 20000;
    format!("eval('{}')", "{".repeat(n) + "1" + &"}".repeat(n))
});
probe!(deep_calls, {
    let n = 40000;
    format!("eval('{}')", "f".to_string() + &"(x)".repeat(n))
});
probe!(deep_arrow, {
    let n = 30000;
    format!("eval('{}')", "a=>".repeat(n) + "1")
});
probe!(deep_binop, {
    let n = 60000;
    format!("eval('{}')", "1".to_string() + &"+1".repeat(n))
});
probe!(deep_comma, {
    let n = 60000;
    format!("eval('{}')", "1".to_string() + &",1".repeat(n))
});
probe!(deep_seq_args, {
    let n = 40000;
    format!("eval('{}')", "f(".to_string() + &"1,".repeat(n) + "1)")
});
probe!(deep_regex_groups, {
    let n = 30000;
    format!("eval('/{}/x/')", "(".repeat(n) + &")".repeat(n))
});
probe!(deep_labels, {
    let n = 20000;
    format!("eval('{}')", "a:".repeat(n) + "1")
});
probe!(deep_try, {
    let n = 15000;
    format!("eval('{}')", "try{".repeat(n) + &"}finally{}".repeat(n))
});

/// Repro du crash vol : le bundle sur un DOM installé —
/// `MutationObserver` présent → l'éval va jusqu'à `initTree`
/// (la strate sous laquelle le daemon meurt — vol :9499).
#[test]
fn alpine_bundle_dom() {
    eprintln!("[probe] alpine_bundle_dom");
    let Ok(src) = std::fs::read_to_string("../../docs/Rapport vol/logs/scratch/alpine.js")
        .or_else(|_| std::fs::read_to_string("docs/Rapport vol/logs/scratch/alpine.js"))
        .or_else(|_| std::fs::read_to_string("/tmp/alpine.min.js"))
    else {
        eprintln!("[probe] bundle absent — skip");
        return;
    };
    // Page imbriquée type docs (x-data sur des éléments profonds).
    let html = r##"<html><body><div x-data="{open:false}">
        <nav><ul><li><a href="#">a</a></li><li><a href="#">b</a></li></ul></nav>
        <div x-data="{items:[1,2,3]}"><template x-for="i in items"><span x-text="i"></span></template></div>
        <div x-data="{t:{n:0}}"><button @click="t.n++">+</button><span x-text="t.n"></span></div>
    </div></body></html>"##;
    let mut e = Engine::new();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(html))));
    match e.eval(&src) {
        Ok(_) => eprintln!("[probe] alpine-dom -> ok (init évalué)"),
        Err(err) => eprintln!(
            "[probe] alpine-dom -> err {}",
            format!("{err}").chars().take(120).collect::<String>()
        ),
    }
}

/// Le vrai bundle alpine.js — crash mesuré en vol (EQ2 #05) :
/// `has overflowed its stack` pendant l'éval du bundle.
#[test]
fn alpine_bundle_real() {
    eprintln!("[probe] alpine_bundle_real");
    let Ok(src) = std::fs::read_to_string("C:/Users/Lou/AppData/Local/Temp/alpine.min.js")
        .or_else(|_| std::fs::read_to_string("/tmp/alpine.min.js"))
    else {
        eprintln!("[probe] bundle absent — skip");
        return;
    };
    let mut e = Engine::new();
    match e.eval(&src) {
        Ok(_) => eprintln!("[probe] alpine -> ok"),
        Err(err) => eprintln!(
            "[probe] alpine -> err {}",
            format!("{err}").chars().take(120).collect::<String>()
        ),
    }
}
