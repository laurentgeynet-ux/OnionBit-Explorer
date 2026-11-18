// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Gate JS-C603 — Iterator helpers **paresseux**
//! (`ObjKind::IterHelper`, spec `%IteratorHelperPrototype%`).
//!
//! Avant : snapshot eager — `map(f)` drainait tout le
//! sous-jacent via `get_prop` data-only à la construction.
//! Un `get done() { throw }` n'était jamais observé → drain
//! infini jusqu'au cap heap (cluster test262 #1, ~178
//! fichiers — profil §B-114), et la paresse spec était
//! observable manquée : `assert.throws` entoure
//! `helper.next()`, pas `iter.map(f)`.
//!
//! Mécanisme : record `(source, next)` capturé à la
//! construction + `op` (`Map`/`Filter`/`Take`/`Drop`/
//! `FlatMap`/`Wrap`/`Chunks`/`Windows`) — `next()` tire UNE
//! étape, propage les getters (`IteratorStep`/`IteratorValue`),
//! ferme le sous-jacent sur complétion abrupte
//! (`IteratorClose`/`IfAbruptCloseIterator`), valide
//! `IsCallable` à la construction et les limites
//! (`take(-1)` → `RangeError`).

use explorer_jscore::Engine;

/// Évalue `src` et décrit la valeur produite.
#[track_caller]
fn run(src: &str) -> String {
    let mut e = Engine::new();
    match e.eval(src) {
        Ok(v) => e.describe(v),
        Err(err) => panic!("eval -> {err}"),
    }
}

/// `map` ne tire **rien** du sous-jacent avant `next()` —
/// la paresse est observable (spec).
#[test]
fn map_ne_tire_pas_avant_next() {
    let out = run("let n=0;\
         let src={next(){n++;return{value:n,done:n>2}}};\
         let m=Iterator.from(src).map(x=>x*10);\
         let r='before:'+n;\
         let a=m.next();r+='|a:'+n+','+a.value+','+a.done;\
         r+='|b:'+m.next().value;\
         r+='|c:'+m.next().done;r");
    assert_eq!(out, "before:0|a:1,10,false|b:20|c:true");
}

/// `get done() { throw }` propage à `.next()` (une fois), pas
/// à `.map()` — l'eager bouclait jusqu'au cap heap.
#[test]
fn getter_done_qui_jette_propage_au_pull() {
    let out = run(
        "class T extends Iterator{next(){return{get done(){throw 42}}}};\
         let m=new T().map(x=>x);\
         try{m.next()}catch(e){e}",
    );
    assert_eq!(out, "42");
}

/// `get value() { throw }` — même propagation.
#[test]
fn getter_value_qui_jette_propage() {
    let out = run(
        "class T extends Iterator{next(){return{done:false,get value(){throw 7}}}};\
         try{new T().map(x=>x).next()}catch(e){e}",
    );
    assert_eq!(out, "7");
}

/// `take(n)` sur source infinie — impossible en eager.
#[test]
fn take_sur_source_infinie() {
    let out = run("let i=0;\
         let inf={next(){return{value:i++,done:false}}};\
         [...Iterator.from(inf).take(4)].join(',')");
    assert_eq!(out, "0,1,2,3");
}

/// `take` épuisé → `IteratorClose` : `return` du sous-jacent
/// appelé une fois (spec `return-is-forwarded`).
#[test]
fn take_forwarde_return_une_fois() {
    let out = run("let rc=0;\
         let s={next(){return{value:1,done:false}},\
               return(){rc++;return{done:true}}};\
         let t=Iterator.from(s).take(1);\
         t.next();t.next();t.next();rc");
    assert_eq!(out, "1");
}

/// `helper.return()` forwarde au sous-jacent + rend
/// `{value:undefined, done:true}` (spec — arg non propagé).
#[test]
fn helper_return_forwarde() {
    let out = run("let rc=0;\
         let s={next(){return{value:1,done:false}},\
               return(){rc++;return{done:true}}};\
         let m=Iterator.from(s).map(x=>x);\
         let r=m.return(9);\
         r.done+'|'+r.value+'|'+rc+'|'+m.next().done");
    // Spec `%IteratorHelperPrototype%.return` : l'argument
    // n'est PAS propagé — `{value: undefined, done: true}`.
    assert_eq!(out, "true|undefined|1|true");
}

/// Early-exit des terminaux → `IteratorClose` (`find`, `some`).
#[test]
fn early_exit_ferme_le_sous_jacent() {
    let out = run("let mk=c=>({next(){return{value:1,done:false}},\
               return(){c.v++;return{done:true}}});\
         let h1={v:0},h2={v:0};\
         Iterator.from(mk(h1)).find(x=>true);\
         Iterator.from(mk(h2)).some(x=>true);\
         h1.v+'|'+h2.v");
    assert_eq!(out, "1|1");
}

/// Callback qui jette → `IfAbruptCloseIterator` (`forEach`).
#[test]
fn callback_qui_jette_ferme() {
    let out = run("let rc=0;\
         let s={next(){return{value:1,done:false}},\
               return(){rc++;return{done:true}}};\
         try{Iterator.from(s).forEach(x=>{throw 1})}catch(e){}rc");
    assert_eq!(out, "1");
}

/// `Iterator.from({next:fn})` wrappe — les helpers deviennent
/// accessibles (spec `%WrapForValidIteratorPrototype%`).
#[test]
fn from_wrappe_les_objets_next_bruts() {
    let out = run(
        "let s={next(){let i=(this.i||0)+1;this.i=i;return{value:i,done:i>2}}};\
         let w=Iterator.from(s);\
         typeof w.map+'|'+(w instanceof Iterator)",
    );
    assert_eq!(out, "function|true");
}

/// Un vrai `Iterator` passe tel quel (pas de double wrap).
#[test]
fn from_passe_un_vrai_iterateur_tel_quel() {
    let out = run("let g=function*(){yield 1}();Iterator.from(g)===g");
    assert_eq!(out, "true");
}

/// Résultat `next()` non-objet → `TypeError` (spec
/// `IteratorStep` — l'ancien chemin le lisait `done`).
#[test]
fn resultat_non_objet_jette() {
    let out = run("try{[...Iterator.from({next(){return 5}})]}catch(e){e.constructor.name}");
    assert_eq!(out, "TypeError");
}

/// `next` non-callable → `TypeError`.
#[test]
fn next_non_callable_jette() {
    let out = run("try{Iterator.from({next:5}).toArray()}catch(e){e.constructor.name}");
    assert_eq!(out, "TypeError");
}

/// Validations d'arguments — `IsCallable` et limites à la
/// construction (avant : le callback n'était vérifié qu'au
/// pull ; `-1` était clampé à 0).
#[test]
fn validations_arguments() {
    let out = run("let r=[];\
         try{Iterator.from([]).map(5)}catch(e){r.push(e.constructor.name)}\
         try{Iterator.from([]).filter(null)}catch(e){r.push(e.constructor.name)}\
         try{Iterator.from([]).flatMap(0)}catch(e){r.push(e.constructor.name)}\
         try{Iterator.from([]).take(-1)}catch(e){r.push(e.constructor.name)}\
         try{Iterator.from([]).drop(-2)}catch(e){r.push(e.constructor.name)}\
         r.join('|')");
    assert_eq!(out, "TypeError|TypeError|TypeError|RangeError|RangeError");
}

/// Index des callbacks — `f(v, i)` avec `i` croissant sur la
/// consommation paresseuse (filter ne compte que les tirés).
#[test]
fn index_callbacks() {
    let out = run("let seen=[];\
         [...Iterator.from([10,20,30]).map((v,i)=>(seen.push(i),v))];\
         seen.join(',')");
    assert_eq!(out, "0,1,2");
}

/// Chaînage — `drop` après `filter`, `take` après `map` :
/// chaque maillon ne tire que ce qu'il doit.
#[test]
fn chainage_paresseux() {
    let out = run("let pulled=0;\
         let s={next(){pulled++;return{value:pulled,done:pulled>10}}};\
         let r=Iterator.from(s).filter(x=>x%2).map(x=>x*10).take(2).toArray();\
         r.join(',')+'|pulled='+pulled");
    assert_eq!(out, "10,30|pulled=3");
}

/// `flatMap` : valeurs itérables aplaties paresseusement —
/// l'itérateur interne est re-tiré jusqu'à épuisement avant
/// de reprendre l'externe.
#[test]
fn flat_map_intercale() {
    let out = run("let log=[];\
         let s={vals:[[1,2],[3]],i:0,\
               next(){let v=this.vals[this.i];this.i++;\
                    return{value:v,done:v===undefined}}};\
         [...Iterator.from(s).flatMap(x=>x)].join(',')");
    assert_eq!(out, "1,2,3");
}

/// `chunks`/`windows` paresseux — `windows` garde n-1 de
/// lookahead ; les deux bornent `n` (`RangeError` si < 1).
#[test]
fn chunks_windows_paresseux() {
    let out = run(
        "JSON.stringify([...Iterator.from([1,2,3,4,5]).chunks(2)])+'|'+\
         JSON.stringify([...Iterator.from([1,2,3]).windows(2)])+'|'+\
         (()=>{try{Iterator.from([]).windows(0)}catch(e){return e.constructor.name}})()",
    );
    assert_eq!(out, "[[1,2],[3,4],[5]]|[[1,2],[2,3]]|RangeError");
}

/// `for-of` — un `get done() { throw }` propage (la boucle
/// paresseuse partageait le même trou `get_prop`).
#[test]
fn for_of_propage_le_getter_done() {
    let out = run(
        "try{for(const x of{[Symbol.iterator](){return{next(){return{get done(){throw 3}}}}}}){}}\
         catch(e){e}",
    );
    assert_eq!(out, "3");
}

/// `for-of` — `IteratorClose` sur `break` (verrou acquis
/// JS-C456, conservé sous les nouvelles lectures).
#[test]
fn for_of_close_sur_break() {
    let out = run("let rc=0;\
         for(const x of{[Symbol.iterator](){return{\
            next(){return{value:1,done:false}},\
            return(){rc++;return{done:true}}}}}){break}rc");
    assert_eq!(out, "1");
}

/// Régression `Iter` snapshot — les itérateurs de collections
/// (`map.keys()`…) restent des snapshots consommables par les
/// helpers paresseux.
#[test]
fn snapshot_iter_alimente_les_helpers() {
    let out = run("let m=new Map([[1,'a'],[2,'b']]);\
         [...m.keys()].join(',')+'|'+\
         [...Iterator.from([...m.values()]).map(v=>v+'!')].join(',')");
    assert_eq!(out, "1,2|a!,b!");
}

/// `Iterator.concat` — paresseux : `@@iterator` lu une fois à
/// la construction, items tirés à la demande.
#[test]
fn concat_paresseux() {
    let out = run("let pulls=0;let a={[Symbol.iterator](){return{\
        next(){pulls++;return pulls<3?{value:pulls,done:false}:{value:0,done:true}}}}};\
        let it=Iterator.concat(a,[9]);let b4=pulls;\
        let r=[it.next().value,it.next().value,it.next().value,it.next().done];\
        [b4,r.join('|')].join(';')");
    assert_eq!(out, "0;1|2|9|true");
}

/// `Iterator.concat` — `return` ferme le record **courant**
/// (une fois), pas les suivants ; `next` après → `done`.
#[test]
fn concat_ferme_le_courant() {
    let out = run("let r1=0;let r2=0;let a={[Symbol.iterator](){return{\
        next(){return{value:1,done:false}},return(){r1++;return{}}}}};\
        let b={[Symbol.iterator](){return{\
        next(){return{value:2,done:false}},return(){r2++;return{}}}}};\
        let it=Iterator.concat(a,b);it.next();it.return();it.return();\
        [r1,r2,it.next().done].join('|')");
    assert_eq!(out, "1|0|true");
}

/// `Iterator.concat` — source infinie bornée par `take`, et
/// non-itérable → `TypeError` à la construction.
#[test]
fn concat_borne_et_valide() {
    let out = run("let inf={[Symbol.iterator](){let i=0;return{\
        next(){return{value:i++,done:false}}}}};\
        [...Iterator.concat(inf).take(3)].join(',')+'|'+\
        (()=>{try{Iterator.concat(42)}catch(e){return e.constructor.name}})()");
    assert_eq!(out, "0,1,2|TypeError");
}

/// Ré-entrance — `next()` pendant qu'une étape tourne →
/// `TypeError` (« generator is already running », sentinelle
/// `IterOp::Running`). Idem `return`/`throw` pendant course.
#[test]
fn reentrance_next_pendant_course() {
    let out = run("let h;\
         let s={next(){let r;try{r=h.next()}catch(e){r=e.constructor.name}\
            let r2;try{r2=h.return()}catch(e){r2=e.constructor.name}\
            let r3;try{r3=h.throw(0)}catch(e){r3=e.constructor.name}\
            return{value:[r,r2,r3].join(','),done:false}}};\
         h=Iterator.from(s).map(x=>x);h.next().value");
    assert_eq!(out, "TypeError,TypeError,TypeError");
}

/// `it_close` = `GetMethod` — `return` non-callable non-nullish
/// → `TypeError` (pas « pas de return »).
#[test]
fn return_non_callable_typeerror() {
    let out = run("let s={next(){return{value:1,done:false}},return:5};\
         try{Iterator.from(s).map(x=>x).return()}catch(e){e.constructor.name}");
    assert_eq!(out, "TypeError");
}
