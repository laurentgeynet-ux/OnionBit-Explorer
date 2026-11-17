// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Sonde stack-overflow : évalue `pattern` à `depth`, ou une
//! source arbitraire lue sur stdin (`stack_probe -`) — chaque
//! exécution est un processus jetable (un abort dur tue le
//! process, pas la boucle appelante).

fn gen(pattern: &str, depth: usize) -> String {
    match pattern {
        "paren" => format!("{}{}{}", "(".repeat(depth), "1", ")".repeat(depth)),
        "array" => format!("{}{}{}", "[".repeat(depth), "1", "]".repeat(depth)),
        "graph_obj" => {
            format!("var a={{}};for(var i=0;i<{depth};i++)a={{x:a}};a")
        }
        "graph_arr" => format!("var a=[];for(var i=0;i<{depth};i++)a=[a];a"),
        "graph_json" => {
            format!("var a={{}};for(var i=0;i<{depth};i++)a={{x:a}};JSON.stringify(a)")
        }
        "graph_desc" => {
            format!("var a={{}};for(var i=0;i<{depth};i++)a={{x:a}};a.x")
        }
        "add" => format!("1{}", "+1".repeat(depth)),
        "call" => format!("function f(x){{return x}} {}", "f(".repeat(depth))
            + "1" + &")".repeat(depth),
        "recurse" => "function f(){f()} try{f()}catch(e){e.constructor.name}".to_string(),
        "mutual" => "function f(){g()} function g(){f()} try{f()}catch(e){e.constructor.name}"
            .to_string(),
        "proxy_self" => {
            "var p=new Proxy({},{get:function(t,k){return p[k]}});try{p.x}catch(e){e.constructor.name}"
                .to_string()
        }
        "deep_concat" => format!("'x'{}", "+'y'".repeat(depth)),
        "ifelse" => "var x=0;".to_string()
            + &"if(x){".repeat(depth)
            + &"}else{1;}".repeat(depth),
        "try_fin" => "try{".repeat(depth) + &"}finally{1;}".repeat(depth),
        "logical" => format!("1{}", "&&1".repeat(depth)),
        "chain_call" => format!("var o={{m:function(){{return this}}}};o{}", ".m()".repeat(depth)),
        "str_repeat" => format!("'a'.repeat({depth})"),
        _ => "1".to_string(),
    }
}

fn main() {
    let mut a = std::env::args().skip(1);
    let pattern = a.next().unwrap_or_else(|| "paren".to_string());
    let src = if pattern == "-" {
        use std::io::Read;
        let mut s = String::new();
        let _ = std::io::stdin().read_to_string(&mut s);
        s
    } else {
        let depth: usize = a.next().and_then(|s| s.parse().ok()).unwrap_or(1000);
        gen(&pattern, depth)
    };
    let mut e = explorer_jscore::Engine::new();
    match e.eval(&src) {
        Ok(_) => println!("ok"),
        Err(err) => println!("err {err:?}"),
    }
}
