// JS-C34a — générateurs lazy réels : `next()` reprend
// l'exécution au `yield`, `next(v)` livre `v` dans le site,
// `for..of`/`spread`/`return`/`throw` spec — pas de collecte
// eager.
use explorer_jscore::Engine;

fn ev(src: &str) -> String {
    let mut e = Engine::new();
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => format!("ERR {err}"),
    }
}

#[test]
fn lazy_gen_resumes_at_yield() {
    // Le corps ne tourne qu'à `next()` — pas à l'appel.
    assert_eq!(
        ev(
            "var log='';function* g(){log+='A';yield 1;log+='B';yield 2;log+='C';return 'R'}\
            var it=g();var r=log;\
            r+='|'+it.next().value+'/'+log;\
            r+='|'+it.next().value+'/'+log;\
            r+='|'+it.next().value+'/'+it.next().done+'/'+log;\
            r"
        ),
        "|1/A|2/AB|R/true/ABC"
    );
    // `next(v)` livre `v` dans le site du yield.
    assert_eq!(
        ev("function* g(){var a=yield 1;var b=yield a;return b}\
            var it=g();it.next();it.next('x').value+'/'+it.next('y').value"),
        "x/y"
    );
    // `for..of` + spread sur lazy.
    assert_eq!(
        ev("function* g(){var i=0;while(i<3){yield i++}}\
            var s='';for(var x of g())s+=x;\
            s+'|'+[...g()].join(',')"),
        "012|0,1,2"
    );
    // `return(v)` interrompt proprement ; `throw(e)` jette au site.
    assert_eq!(
        ev("function* g(){try{yield 1}catch(e){yield 'C'+e}yield 9}\
            var it=g();it.next();var a=it.throw('x').value;it.next().value+'|'+a"),
        "9|Cx"
    );
}

/// JS-C35 — TLA de module : `await` top-level suspend le corps
/// sur `GExec` ; la promesse pendante le reprend ; les effets du
/// reste du module courent dans la continuation.
#[test]
fn tla_module_awaits_pending() {
    let mut e = explorer_jscore::Engine::new();
    e.eval("globalThis.__out = 'unset'").unwrap();
    e.eval_module(
        "var x = await new Promise(res => Promise.resolve().then(() => res(7)));\
         globalThis.__out = x;",
        "m",
    )
    .unwrap();
    let v = e.eval("globalThis.__out").unwrap();
    assert_eq!(e.describe(v), "7");
}
