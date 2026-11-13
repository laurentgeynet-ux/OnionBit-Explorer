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

/// JS-C492 — le corps baissé d'un gen/async est **partagé par
/// `lit`** (un seul `Rc<Vec<Stmt>>` réutilisé à chaque appel).
/// Avant : `lower_suspend_fn` réallouait un AST neuf par appel ;
/// `resolved_map` (clé `ptr(Expr)`) gardait les adresses du
/// corps du 1er appel après la mort de son `GExec` → un corps
/// baissé ultérieur pouvait réutiliser ces adresses →
/// profondeur fantôme → mauvaise porte lue (corruption async).
/// Verrou : instanciations répétées de générateurs lisant une
/// var d'une portée englobante — chaque appel rend la bonne
/// valeur, quel que soit l'ordre alloc/libération des corps.
#[test]
fn suspend_lowered_body_shared_scope() {
    // `make` renvoie un générateur capturant `x` (env parent) —
    // chaque appel produit un `GExec` dont le corps baissé
    // vit/meurt ; la lecture de `x` (profondeur 1) doit rester
    // correcte sur tous les appels.
    assert_eq!(
        ev(
            "var out='';function make(x){function* g(){var t=x;return t}return g()}\
             for(var i=0;i<64;i++){out+=make(i).next().value+','}\
             out"
        ),
        "0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31,\
         32,33,34,35,36,37,38,39,40,41,42,43,44,45,46,47,48,49,50,51,52,53,54,55,56,57,58,59,60,\
         61,62,63,"
            .replace(' ', "")
    );
    // Même lecture de portée sur `async` — la promesse rend
    // la valeur de la var capturée.
    let mut e = Engine::new();
    e.eval(
        "globalThis.__r=[];function mk(x){return async function(){var t=x;return t}()}\
         for(var i=0;i<32;i++){(function(k){mk(k).then(function(v){__r[k]=v})})(i)}",
    )
    .unwrap();
    e.run_microtasks().unwrap();
    let v = e.eval("__r.length+'|'+__r[31]").unwrap();
    assert_eq!(e.describe(v), "32|31");
}
