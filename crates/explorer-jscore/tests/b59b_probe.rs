// This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Sonde §B-59 second bras — rejoue `wrap.js` confiant
//! (actugaming, 378 Ko) : RSS +2,9 Go pour `heap_b` +15 Mo,
//! timers/microtasks/DOM plats — la croissance est hors
//! compteur cellules. `EXPLORER_JS_RSS_WATCH=100` arme le
//! dump périodique (rss/heap/cells/kinds) dans le fil même.
//! `cargo test --test b59b_probe -- --ignored --nocapture`.

use explorer_jscore::{Dom, Engine};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

/// Discriminant §B-59 bras 2 — la forme tueuse réduite :
///   `s += fromCharCode` en boucle (le decoder confiant).
/// Mesure RSS du process vs `heap.bytes` : si RSS décolle
///   pendant que heap reste bas → churn transitoire retenu par
/// l'allocateur (fragmentation/arena), pas de la rétention
///   de cellules. Borne : ~40 s d'exécution max.
#[test]
fn concat_churn_rss_vs_heap() {
    let rss = || memory_stats::memory_stats().map_or(0, |m| m.physical_mem / (1024 * 1024));
    let mut e = Engine::new();
    e.set_mem_limit(0); // bornes heap hors du chemin — on mesure
    let base = rss();
    eprintln!("[churn] rss base {base} Mo");
    // 8 × 200 k tours — la courbe RSS parle.
    for round in 0..8 {
        let r =
            e.eval("var s='';for(var i=0;i<200000;i++){s+=String.fromCharCode(97+i%26)}s.length;");
        let st = e.heap_stats();
        eprintln!(
            "[churn] round {round} rss={} Mo heap={} o cells={} gc={} -> {:?}",
            rss(),
            st.estimated_bytes,
            st.live_cells,
            st.gc_runs,
            r.as_ref().map(|v| e.describe(*v))
        );
        if r.is_err() {
            break;
        }
    }
}

/// Repro fidèle §B-59 bras 2 — le decoder confiant : par appel,
///   atob `+=` char + échappement `%xx` `+=` + `decodeURIComponent`
/// + écriture cache sur objet retenu — ×10⁵ appels. Chaque decode
///   émet ~2-3 intermédiaires natifs ; le cache retient le résultat.
///   Mesure RSS vs heap : le churn entrelacé de tailles variées est
///   le candidat fragmentation-arena (RSS +2,9 Go / heap +15 Mo).
#[test]
fn decoder_churn_rss_vs_heap() {
    let rss = || memory_stats::memory_stats().map_or(0, |m| m.physical_mem / (1024 * 1024));
    let mut e = Engine::new();
    e.set_mem_limit(0);
    let base = rss();
    eprintln!("[dec] rss base {base} Mo");
    // Le decoder obfuscator : str-table globale + decodeur
    // atob+%+decodeURI + cache sur `arguments` équivalent.
    let setup = r"
var TAB=[];for(var i=0;i<400;i++){TAB.push(
 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789+/='+i+
 'ZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZ');}
var CACHE={};
function dec(idx){ if(CACHE[idx]!==undefined) return CACHE[idx];
  var src=TAB[idx%400], out='', hex='';
  for(var c=0;c<src.length;c++){out+=String.fromCharCode(src.charCodeAt(c)^29);}
  for(var c=0;c<out.length;c++){hex+='%'+('00'+out.charCodeAt(c).toString(16)).slice(-2);}
  var r; try{r=decodeURIComponent(hex)}catch(e){r=out}
  CACHE[idx]=r; return r; }
";
    e.eval(setup).expect("setup decodeur");
    for round in 0..6 {
        let r = e.eval("var acc=0;for(var i=0;i<20000;i++){acc+=dec(i%400+(i>>8)).length}acc;");
        let st = e.heap_stats();
        eprintln!(
            "[dec] round {round} rss={} Mo heap={} o slab={} o cells={} gc={} -> {:?}",
            rss(),
            st.estimated_bytes,
            e.slab_overhead_bytes(),
            st.live_cells,
            st.gc_runs,
            r.as_ref().map(|v| e.describe(*v))
        );
        if r.is_err() {
            break;
        }
    }
}

/// Discriminant §B-59 bras 3 — `eval()`/`new Function` en boucle
/// (pattern obfuscateur : decode → eval du payload). Chaque éval
///   parse un AST `Rc<FnLit>` retenu par la cellule `Fun` créée —
/// `cell_bytes` compte le `Rc` shallow, pas l'arbre : la masse
///   native serait invisible à `heap_b` — la signature exacte du
/// vol (rss 4 Go / heap 50 Mo / virt 6,7 Go).
#[test]
fn eval_ast_retention_rss_vs_heap() {
    let rss = || memory_stats::memory_stats().map_or(0, |m| m.physical_mem / (1024 * 1024));
    let mut e = Engine::new();
    e.set_mem_limit(0);
    eprintln!("[ast] rss base {} Mo", rss());
    // Corps de fonction ~4 Ko d'expression réelle — `1+1+…`
    // produit de vrais nœuds AST (un commentaire serait ignoré).
    let body = format!("function(){{return {}}}", "1+".repeat(2000) + "1");
    e.eval(&format!("var SRC=\"{body}\";")).expect("SRC");
    for round in 0..6 {
        let r =
            e.eval("var keep=[];for(var i=0;i<200;i++){keep.push(eval('('+SRC+')'))}keep.length;");
        let st = e.heap_stats();
        eprintln!(
            "[ast] round {round} rss={} Mo heap={} o slab={} o cells={} -> {:?}",
            rss(),
            st.estimated_bytes,
            e.slab_overhead_bytes(),
            st.live_cells,
            r.as_ref().map(|v| e.describe(*v))
        );
        if r.is_err() {
            break;
        }
    }
}

/// Verrou §B-59 bras 3 — sous cap `eval`/`Function` bas, la
///   boucle decode→eval meurt en `RangeError` nommé et le moteur
/// survit (l'AST n'est jamais parsé — la rétention est coupée
///   à la source).
#[test]
fn eval_src_cap_kills_eval_loop() {
    let mut e = Engine::new();
    e.set_eval_src_cap(4 * 1024);
    // ~1 Ko source par eval × 400 → dépassement au ~5e appel.
    let r = e.eval(
        "var k=[],pad='xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx'.repeat(20);\
         for(var i=0;i<400;i++){k.push(eval('(function(){var x'+i+'=\"'+pad+'\"})'))}k.length;",
    );
    match r {
        Err(err) => {
            let msg = format!("{err}");
            assert!(
                msg.contains("eval source budget exceeded"),
                "RangeError cap attendu, obtenu : {msg}"
            );
        }
        Ok(_) => panic!("la boucle eval aurait dû être tuée par le cap source"),
    }
    assert_eq!(e.eval("1+1").ok().map(|v| e.describe(v)), Some("2".into()));
}

#[test]
#[ignore = "terrain : rejoue le wrap.js §B-59 capturé"]
fn confiant_wrap_under_mem_limit() {
    let src = std::fs::read_to_string("_confiant_wrap.js").expect("_confiant_wrap.js à la racine");
    eprintln!("[b59b] {} o — eval sous mem_limit 512 Mio", src.len());
    let mut e = Engine::new();
    e.set_mem_limit(512 * 1024 * 1024);
    e.install_timers();
    e.install_dom(&Rc::new(RefCell::new(Dom::from_html(
        "<html><body><div id=x></div></body></html>",
    ))));
    // Préambule : stubs des APIs hôte absentes du mini-DOM de
    // test (XHR, fetch, Image…) — l'éval daemon atteint
    // wrap.js:14/16 ; le stub laisse le script descendre dans
    // la zone chaude. Itératif : chaque ReferenceError nomme
    // le stub suivant.
    let pre = r"
var XMLHttpRequest = function(){ this.readyState=4; this.status=200;
  this.responseText=''; this.open=function(){}; this.send=function(){};
  this.setRequestHeader=function(){}; this.addEventListener=function(){};
  this.onload=null; this.onerror=null; };
var fetch = function(){ return Promise.resolve({ok:true,status:200,
  text:function(){return Promise.resolve('')},
  json:function(){return Promise.resolve({})},
  arrayBuffer:function(){return Promise.resolve(new ArrayBuffer(0))},
  headers:{get:function(){return null}}}); };
";
    e.eval(pre).expect("préambule stubs");
    let t = Instant::now();
    match e.eval_named(&src, "wrap.js") {
        Ok(v) => {
            let d = e.describe(v);
            eprintln!(
                "[b59b] OK en {:?} — {}",
                t.elapsed(),
                d.chars().take(200).collect::<String>()
            );
        }
        Err(err) => eprintln!("[b59b] ERR en {:?} — {err}", t.elapsed()),
    }
    let (p, x) = e.last_eval_split();
    eprintln!("[b59b] split parse={p}us exec={x}us");
    for (k, n, b) in e.heap_kind_histogram() {
        eprintln!("[b59b] kind {k}: {n} cells / {b} o");
    }
}
