// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! §B-22/J11 — chaîne redux+thunk réelle en-process : un dispatch
//! de fonction (action thunk) doit être intercepté par le
//! middleware, pas atteindre le `dispatch` de base (sinon
//! `Minified Redux error #7` — mur mesuré mastodon).

use explorer_jscore::Engine;

/// Loader minimal : spec → source (même forme que `lang.rs`).
struct MapLoader(std::collections::BTreeMap<String, String>);

impl explorer_jscore::ModuleLoader for MapLoader {
    fn load(&mut self, specifier: &str, _referrer: &str) -> std::result::Result<String, String> {
        self.0
            .get(specifier)
            .map(std::string::ToString::to_string)
            .ok_or_else(|| format!("not found: {specifier}"))
    }
}

fn eval_or_panic(e: &mut Engine, src: &str) -> explorer_jscore::Value {
    e.eval(src).unwrap_or_else(|x| panic!("{x}"))
}

#[test]
fn redux_thunk_chain_intercepts_function_actions() {
    let redux =
        std::fs::read_to_string("../../_scratch/redux_classic.js").expect("bundle redux absent");
    let thunk =
        std::fs::read_to_string("../../_scratch/thunk_classic.js").expect("bundle thunk absent");
    let probe = std::fs::read_to_string("../../_scratch/redux_probe.js").expect("sonde absente");
    let mut e = Engine::new();
    eval_or_panic(&mut e, &redux);
    eval_or_panic(&mut e, &thunk);
    let v = eval_or_panic(&mut e, &probe);
    let report = e.describe(v);
    eprintln!("{report}");
    assert!(
        report.contains("thunk dispatch → called=yes ret=7"),
        "{report}"
    );
}

#[test]
fn redux_edge_forms_probe() {
    let redux =
        std::fs::read_to_string("../../_scratch/redux_classic.js").expect("bundle redux absent");
    let thunk =
        std::fs::read_to_string("../../_scratch/thunk_classic.js").expect("bundle thunk absent");
    let probe = std::fs::read_to_string("../../_scratch/redux_probe2.js").expect("sonde absente");
    let mut e = Engine::new();
    // `Promise` est requis par la forme (f).
    e.install_dom(&std::rc::Rc::new(std::cell::RefCell::new(
        explorer_jscore::Dom::from_html("<html><body></body></html>"),
    )));
    eval_or_panic(&mut e, &redux);
    eval_or_panic(&mut e, &thunk);
    let v = eval_or_panic(&mut e, &probe);
    eprintln!("{}", e.describe(v));
}

/// §B-22 — la vraie chaîne RTK : `configureStore` avec
/// `getDefaultMiddleware` (`MiddlewareArray`, sous-classe Array
/// → `.concat`/`Symbol.species`/spread dans `applyMiddleware`)
/// puis dispatch d'une fonction-thunk — repro fidèle du mur
/// mastodon `Minified Redux error #7`.
#[test]
fn rtk_configure_store_thunk_dispatch() {
    let read = |n: &str| {
        std::fs::read_to_string(format!("../../_scratch/{n}"))
            .unwrap_or_else(|_| panic!("bundle {n} absent"))
    };
    let deps: Vec<(String, String)> = [
        ("redux", read("redux.mjs")),
        ("immer", read("immer.mjs")),
        ("reselect", read("reselect.mjs")),
        ("redux-thunk", read("redux-thunk.mjs")),
        ("rtk", read("rtk.mjs")),
    ]
    .into_iter()
    .map(|(a, b)| (a.to_string(), b))
    .collect();
    let main = r"
import {configureStore, createAsyncThunk, createSlice} from 'rtk';
var out = [];
try {
  const slice = createSlice({name:'c', initialState:{v:0},
    reducers:{inc(s){s.v+=1}}});
  const store = configureStore({reducer: slice.reducer});
  out.push('store ok');
  store.dispatch(slice.actions.inc());
  out.push('plain ok v=' + store.getState().v);
  var hit = 'n';
  store.dispatch(function(d,g){ hit='y'; return 5; });
  out.push('thunk hit=' + hit);
} catch (e) { out.push('EXC ' + (e && e.message ? e.message : e)); }
globalThis.__r = out.join('\n');
";
    let mut e = Engine::new();
    // Shim `process.env` — webpack le substitue au build chez
    // mastodon ; le bundle ESM brut le référence littéralement.
    eval_or_panic(&mut e, "var process={env:{NODE_ENV:'production'}}");
    let map: std::collections::BTreeMap<String, String> = deps.into_iter().collect();
    let loader = MapLoader(map);
    e.set_module_loader(std::rc::Rc::new(std::cell::RefCell::new(loader)));
    match e.eval_module(main, "main.js") {
        Ok(_) => {}
        Err(x) => panic!("module eval → {x}"),
    }
    let v = e.eval("globalThis.__r").unwrap_or_else(|x| panic!("{x}"));
    let report = e.describe(v);
    eprintln!("{report}");
    assert!(report.contains("thunk hit=y"), "{report}");
}

/// Bissecte la phase qui jette dans la chaîne RTK.
#[test]
fn rtk_phase_bisect() {
    let read = |n: &str| {
        std::fs::read_to_string(format!("../../_scratch/{n}"))
            .unwrap_or_else(|_| panic!("bundle {n} absent"))
    };
    let deps: Vec<(String, String)> = [
        ("redux", read("redux.mjs")),
        ("immer", read("immer.mjs")),
        ("reselect", read("reselect.mjs")),
        ("redux-thunk", read("redux-thunk.mjs")),
        ("rtk", read("rtk.mjs")),
    ]
    .into_iter()
    .map(|(a, b)| (a.to_string(), b))
    .collect();
    let main = std::fs::read_to_string("../../_scratch/rtk_bisect.mjs").expect("bisect absent");
    let mut e = Engine::new();
    eval_or_panic(&mut e, "var process={env:{NODE_ENV:'production'}}");
    let map: std::collections::BTreeMap<String, String> = deps.into_iter().collect();
    e.set_module_loader(std::rc::Rc::new(std::cell::RefCell::new(MapLoader(map))));
    match e.eval_module(&main, "main.js") {
        Ok(_) => {}
        Err(x) => panic!("module eval → {x}"),
    }
    let v = e.eval("globalThis.__r").unwrap_or_else(|x| panic!("{x}"));
    eprintln!("{}", e.describe(v));
}

/// §B-22 — repro minimal hors modules : `class extends Array`
/// doit propager ses arguments de construction (`new MA('x')`
/// → `['x']`), sinon `MiddlewareArray` naît vide et aucun
/// middleware ne s'installe (mur #7 exact).
#[test]
fn array_subclass_ctor_args() {
    let mut e = Engine::new();
    let v = eval_or_panic(
        &mut e,
        r"var out=[];
        class MA extends Array {}
        var a = new MA(5,6,7);
        out.push('3args len=' + a.length);
        var b = new MA('x');
        out.push('str len=' + b.length + ' [0]=' + b[0]);
        var c = new Array('x');
        out.push('plain len=' + c.length + ' [0]=' + c[0]);
        class MB extends Array { constructor(...args){ super(...args); } }
        var d = new MB('y');
        out.push('explicit len=' + d.length + ' [0]=' + d[0]);
        class ME extends Array { constructor(){ super(); } }
        var f = new ME('z');
        out.push('noargs len=' + f.length + ' [0]=' + f[0]);
        out.join('\n');",
    );
    eprintln!("{}", e.describe(v));
}

/// §B-22 — bissecte le transport middleware : `MiddlewareArray`
/// (sous-classe Array) → spread `...` → `.map` → `applyMiddleware`.
#[test]
fn rtk_middleware_array_transport() {
    let read = |n: &str| {
        std::fs::read_to_string(format!("../../_scratch/{n}"))
            .unwrap_or_else(|_| panic!("bundle {n} absent"))
    };
    let deps: Vec<(String, String)> = [("redux", read("redux.mjs"))]
        .into_iter()
        .map(|(a, b)| (a.to_string(), b))
        .collect();
    let main = std::fs::read_to_string("../../_scratch/rtk_chain.mjs").expect("sonde absente");
    let mut e = Engine::new();
    let map: std::collections::BTreeMap<String, String> = deps.into_iter().collect();
    e.set_module_loader(std::rc::Rc::new(std::cell::RefCell::new(MapLoader(map))));
    match e.eval_module(&main, "main.js") {
        Ok(_) => {}
        Err(x) => panic!("module eval → {x}"),
    }
    let v = e.eval("globalThis.__r").unwrap_or_else(|x| panic!("{x}"));
    eprintln!("{}", e.describe(v));
}

/// Bissecte immer `produce` + `configureStore({reducer: slice.reducer})`.
#[test]
fn rtk_immer_slice_bisect() {
    let read = |n: &str| {
        std::fs::read_to_string(format!("../../_scratch/{n}"))
            .unwrap_or_else(|_| panic!("bundle {n} absent"))
    };
    let deps: Vec<(String, String)> = [
        ("redux", read("redux.mjs")),
        ("immer", read("immer.mjs")),
        ("reselect", read("reselect.mjs")),
        ("redux-thunk", read("redux-thunk.mjs")),
        ("rtk", read("rtk.mjs")),
    ]
    .into_iter()
    .map(|(a, b)| (a.to_string(), b))
    .collect();
    let main = std::fs::read_to_string("../../_scratch/rtk_bisect3.mjs").expect("bisect absent");
    let mut e = Engine::new();
    eval_or_panic(&mut e, "var process={env:{NODE_ENV:'production'}}");
    let map: std::collections::BTreeMap<String, String> = deps.into_iter().collect();
    e.set_module_loader(std::rc::Rc::new(std::cell::RefCell::new(MapLoader(map))));
    match e.eval_module(&main, "main.js") {
        Ok(_) => {}
        Err(x) => panic!("module eval → {x}"),
    }
    let v = e.eval("globalThis.__r").unwrap_or_else(|x| panic!("{x}"));
    eprintln!("{}", e.describe(v));
}
